use std::io;
use std::net::{Ipv4Addr, SocketAddrV4, UdpSocket};

use crate::wire::DEFAULT_PORT;

pub const QUERY_SIZE: usize = 0x200;
pub const QUERY_WORD: [u8; 4] = [0xfa, 0xfb, 0xfc, 0xfd];
pub const ANSWER_SIZE: usize = 0x38;
pub const ANSWER_MAGIC: u32 = 0xafbf_cfdf;
pub const GAME_ID: u32 = u32::from_be_bytes(*b"myth");
pub const VERSION: u32 = 0x100;
pub const NAME_SIZE: usize = 0x20;
pub const QUERY_INTERVAL_MS: u64 = 500;
pub const FORGET_MS: u64 = 2000;
pub const PENDING_MAX: usize = 10;
pub const BROADCAST: SocketAddrV4 = SocketAddrV4::new(Ipv4Addr::BROADCAST, DEFAULT_PORT);

pub fn query() -> [u8; QUERY_SIZE] {
    let mut q = [0u8; QUERY_SIZE];
    for w in q.as_chunks_mut::<4>().0 {
        w.copy_from_slice(&QUERY_WORD);
    }
    q
}

pub fn is_query(bytes: &[u8]) -> bool {
    bytes.len() == QUERY_SIZE && bytes.as_chunks::<4>().0.iter().all(|w| *w == QUERY_WORD)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Answer {
    pub game_id: u32,
    pub version: u32,
    pub ip: Ipv4Addr,
    pub port: u16,
    pub name: Vec<u8>,
}

impl Answer {
    pub fn new(ip: Ipv4Addr, port: u16, name: &[u8]) -> Answer {
        Answer {
            game_id: GAME_ID,
            version: VERSION,
            ip,
            port,
            name: name_field(name),
        }
    }

    pub fn encode(&self) -> [u8; ANSWER_SIZE] {
        let mut b = [0u8; ANSWER_SIZE];
        b[0..4].copy_from_slice(&ANSWER_MAGIC.to_be_bytes());
        b[4..8].copy_from_slice(&self.game_id.to_be_bytes());
        b[8..12].copy_from_slice(&self.version.to_be_bytes());
        b[12..16].copy_from_slice(&self.ip.octets());
        b[16..18].copy_from_slice(&self.port.to_be_bytes());
        let name = name_field(&self.name);
        b[0x14..0x14 + name.len()].copy_from_slice(&name);
        b
    }

    pub fn decode(bytes: &[u8]) -> Option<Answer> {
        let mut b = [0u8; ANSWER_SIZE];
        let n = bytes.len().min(ANSWER_SIZE);
        b[..n].copy_from_slice(&bytes[..n]);
        let word = |at: usize| u32::from_be_bytes([b[at], b[at + 1], b[at + 2], b[at + 3]]);
        if n < 4 || word(0) != ANSWER_MAGIC {
            return None;
        }
        Some(Answer {
            game_id: word(4),
            version: word(8),
            ip: Ipv4Addr::from(word(12)),
            port: u16::from_be_bytes([b[16], b[17]]),
            name: name_field(&b[0x14..0x14 + NAME_SIZE]),
        })
    }
}

fn name_field(name: &[u8]) -> Vec<u8> {
    let end = name.iter().position(|&c| c == 0).unwrap_or(name.len());
    name[..end.min(NAME_SIZE - 1)].to_vec()
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LanGame {
    pub address: SocketAddrV4,
    pub name: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LanEvent {
    Added(LanGame),
    Removed(SocketAddrV4),
}

#[derive(Debug, Clone)]
struct Entry {
    game: LanGame,
    new: bool,
    heard_ms: u64,
}

#[derive(Debug, Clone, Default)]
pub struct LanList {
    games: Vec<Entry>,
    pending: Vec<LanGame>,
    last_query_ms: Option<u64>,
}

impl LanList {
    pub fn new() -> LanList {
        LanList::default()
    }

    pub fn receive(&mut self, from: SocketAddrV4, bytes: &[u8]) {
        if let Some(a) = Answer::decode(bytes)
            && a.game_id == GAME_ID
            && self.pending.len() < PENDING_MAX
            && !self.pending.iter().any(|p| p.address == from)
        {
            self.pending.push(LanGame {
                address: from,
                name: a.name,
            });
        }
    }

    pub fn update(&mut self, now_ms: u64) -> (Vec<LanEvent>, bool) {
        for game in std::mem::take(&mut self.pending) {
            match self
                .games
                .iter_mut()
                .find(|e| e.game.address == game.address)
            {
                Some(e) => e.heard_ms = now_ms,
                None => self.games.push(Entry {
                    game,
                    new: true,
                    heard_ms: now_ms,
                }),
            }
        }
        let mut events = Vec::new();
        self.games.retain_mut(|e| {
            if e.new {
                e.new = false;
                events.push(LanEvent::Added(e.game.clone()));
                true
            } else if now_ms.saturating_sub(e.heard_ms) > FORGET_MS {
                events.push(LanEvent::Removed(e.game.address));
                false
            } else {
                true
            }
        });
        let due = self
            .last_query_ms
            .is_none_or(|t| now_ms.saturating_sub(t) > QUERY_INTERVAL_MS);
        if due {
            self.last_query_ms = Some(now_ms);
        }
        (events, due)
    }

    pub fn games(&self) -> impl Iterator<Item = &LanGame> {
        self.games.iter().filter(|e| !e.new).map(|e| &e.game)
    }
}

#[derive(Debug)]
pub struct Browser {
    socket: UdpSocket,
    target: SocketAddrV4,
    list: LanList,
}

const ANSWERS_PER_POLL: usize = 32;

impl Browser {
    pub fn open(target: SocketAddrV4) -> io::Result<Browser> {
        let socket = UdpSocket::bind(SocketAddrV4::new(Ipv4Addr::UNSPECIFIED, 0))?;
        socket.set_broadcast(true)?;
        socket.set_nonblocking(true)?;
        Ok(Browser {
            socket,
            target,
            list: LanList::new(),
        })
    }

    pub fn poll(&mut self, now_ms: u64) -> Vec<LanEvent> {
        let mut buf = [0u8; 0x400];
        for _ in 0..ANSWERS_PER_POLL {
            match self.socket.recv_from(&mut buf) {
                Ok((n, std::net::SocketAddr::V4(from))) => self.list.receive(from, &buf[..n]),
                Ok(_) => {}
                Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
                Err(_) => break,
            }
        }
        let (events, due) = self.list.update(now_ms);
        if due {
            let _ = self.socket.send_to(&query(), self.target);
        }
        events
    }

    pub fn games(&self) -> impl Iterator<Item = &LanGame> {
        self.list.games()
    }
}

pub fn answer_for(datagram: &[u8], answer: Option<&Answer>) -> Option<[u8; ANSWER_SIZE]> {
    let head = &datagram[..datagram.len().min(QUERY_SIZE)];
    if is_query(head) {
        answer.map(Answer::encode)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(port: u16) -> SocketAddrV4 {
        SocketAddrV4::new(Ipv4Addr::new(192, 168, 1, 7), port)
    }

    #[test]
    fn the_query_is_the_word_128_times() {
        let q = query();
        assert_eq!(&q[..8], &[0xfa, 0xfb, 0xfc, 0xfd, 0xfa, 0xfb, 0xfc, 0xfd]);
        assert!(is_query(&q));
        assert!(!is_query(&q[..0x1fc]));
        let mut bad = q;
        bad[0x1ff] = 0;
        assert!(!is_query(&bad));
    }

    #[test]
    fn the_answer_is_0x38_bytes_big_endian() {
        let a = Answer::new(Ipv4Addr::UNSPECIFIED, 3453, b"Untitled Game");
        let b = a.encode();
        assert_eq!(
            &b[..12],
            &[0xaf, 0xbf, 0xcf, 0xdf, b'm', b'y', b't', b'h', 0, 0, 1, 0]
        );
        assert_eq!(&b[12..20], &[0, 0, 0, 0, 0x0d, 0x7d, 0, 0]);
        assert_eq!(&b[0x14..0x21], b"Untitled Game");
        assert_eq!(&b[0x21..], &[0; ANSWER_SIZE - 0x21]);
        assert_eq!(Answer::decode(&b), Some(a));
    }

    #[test]
    fn a_long_name_keeps_31_bytes_and_its_nul() {
        let a = Answer::new(Ipv4Addr::LOCALHOST, 1, &[b'x'; 40]);
        let b = a.encode();
        assert_eq!(&b[0x14..0x33], &[b'x'; 31]);
        assert_eq!(b[0x33], 0);
        assert_eq!(Answer::decode(&b).unwrap().name.len(), 31);
    }

    #[test]
    fn only_a_query_is_answered() {
        let a = Answer::new(Ipv4Addr::UNSPECIFIED, 3453, b"G");
        assert_eq!(answer_for(&query(), Some(&a)), Some(a.encode()));
        assert_eq!(answer_for(&query(), None), None);
        assert_eq!(answer_for(&[0xfa; 512], Some(&a)), None);
        let mut long = query().to_vec();
        long.extend_from_slice(&[1, 2, 3]);
        assert_eq!(answer_for(&long, Some(&a)), Some(a.encode()));
    }

    #[test]
    fn the_list_adds_refreshes_and_forgets() {
        let mut l = LanList::new();
        let (e, due) = l.update(0);
        assert!(e.is_empty() && due);
        let answer = Answer::new(Ipv4Addr::UNSPECIFIED, 3453, b"One").encode();
        l.receive(at(3453), &answer);
        let (e, due) = l.update(100);
        assert!(!due);
        assert_eq!(
            e,
            vec![LanEvent::Added(LanGame {
                address: at(3453),
                name: b"One".to_vec()
            })]
        );
        l.receive(
            at(3453),
            &Answer::new(Ipv4Addr::UNSPECIFIED, 3453, b"Two").encode(),
        );
        assert!(l.update(1500).0.is_empty());
        assert_eq!(l.games().next().unwrap().name, b"One");
        assert!(l.update(3500).0.is_empty(), "2000 ms is still heard");
        assert_eq!(l.update(3501).0, vec![LanEvent::Removed(at(3453))]);
        assert_eq!(l.games().count(), 0);
    }

    #[test]
    fn another_game_or_garbage_is_not_listed() {
        let mut l = LanList::new();
        let mut other = Answer::new(Ipv4Addr::UNSPECIFIED, 3453, b"x");
        other.game_id = u32::from_be_bytes(*b"bngi");
        l.receive(at(1), &other.encode());
        l.receive(at(2), &query());
        l.receive(at(3), &[0xaf, 0xbf]);
        assert!(l.update(0).0.is_empty());
    }

    #[test]
    fn ten_new_answers_a_poll() {
        let mut l = LanList::new();
        let answer = Answer::new(Ipv4Addr::UNSPECIFIED, 3453, b"G").encode();
        for p in 0..12 {
            l.receive(at(1000 + p), &answer);
        }
        assert_eq!(l.update(0).0.len(), PENDING_MAX);
    }

    #[test]
    fn queries_go_every_500_ms() {
        let mut l = LanList::new();
        let sent: Vec<u64> = (0..=1600).step_by(100).filter(|&t| l.update(t).1).collect();
        assert_eq!(sent, [0, 600, 1200]);
    }

    #[test]
    fn a_browser_poll_reads_a_bounded_number_of_datagrams() {
        let sink = UdpSocket::bind("127.0.0.1:0").unwrap();
        let target = match sink.local_addr().unwrap() {
            std::net::SocketAddr::V4(v4) => v4,
            _ => unreachable!(),
        };
        let mut browser = Browser::open(target).unwrap();
        let browser_addr = browser.socket.local_addr().unwrap();
        let sender = UdpSocket::bind("127.0.0.1:0").unwrap();
        for _ in 0..ANSWERS_PER_POLL {
            sender.send_to(b"noise", browser_addr).unwrap();
        }
        let answer = Answer::new(Ipv4Addr::LOCALHOST, 3453, b"After Flood").encode();
        sender.send_to(&answer, browser_addr).unwrap();
        std::thread::sleep(std::time::Duration::from_millis(20));
        assert!(
            browser.poll(100).is_empty(),
            "first poll stops at ANSWERS_PER_POLL"
        );
        let second = browser.poll(200);
        assert_eq!(second.len(), 1);
        assert!(matches!(&second[0], LanEvent::Added(g) if g.name == b"After Flood"));
    }
}
