use std::io::{self, Read, Write};
use std::net::{Shutdown, SocketAddr, TcpListener, TcpStream, UdpSocket};
use std::sync::atomic::{AtomicBool, AtomicI32, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use crate::wire::{Body, Frame, MAX_QUEUE_SIZE, Message, Splitter, TO_HOST};

pub const CONNECT_TIMEOUT: Duration = Duration::from_secs(30);

pub trait ByteStream: std::fmt::Debug + Send {
    fn read(&mut self, out: &mut Vec<u8>) -> io::Result<()>;
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize>;
    fn close(&mut self);
}

const DRAIN_WAIT: Duration = Duration::from_millis(250);

const INBOX_LIMIT: usize = 64 * MAX_QUEUE_SIZE;

const ACCEPTS_PER_POLL: usize = 16;
const DATAGRAMS_PER_POLL: usize = 32;

pub type Waker = Arc<dyn Fn() + Send + Sync>;

#[derive(Default)]
pub struct Notify {
    arrived: Mutex<bool>,
    cond: Condvar,
    waker: Mutex<Option<Waker>>,
}

impl std::fmt::Debug for Notify {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Notify")
            .field("arrived", &*lock(&self.arrived))
            .field("waker", &lock(&self.waker).is_some())
            .finish()
    }
}

impl Notify {
    pub fn notify(&self) {
        *lock(&self.arrived) = true;
        self.cond.notify_all();
        let waker = lock(&self.waker).clone();
        if let Some(w) = waker {
            w();
        }
    }

    pub fn set_waker(&self, waker: Option<Waker>) {
        *lock(&self.waker) = waker;
    }

    pub fn wait(&self, timeout: Duration) -> bool {
        let until = Instant::now().checked_add(timeout);
        let mut arrived = lock(&self.arrived);
        while !*arrived {
            let rest = match until {
                Some(until) => match until.checked_duration_since(Instant::now()) {
                    Some(rest) => rest,
                    None => break,
                },
                None => timeout,
            };
            arrived = self
                .cond
                .wait_timeout(arrived, rest)
                .map(|(g, _)| g)
                .unwrap_or_else(|e| e.into_inner().0);
        }
        std::mem::take(&mut *arrived)
    }
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PongTimes {
    pub answered_us: Vec<u32>,
    pub seen_us: Vec<u32>,
}

#[derive(Debug, Default)]
struct Io {
    inbox: Vec<u8>,
    arrived: Option<Instant>,
    out: Vec<u8>,
    writing: usize,
    closing: bool,
    answered: Vec<(u32, Instant)>,
    pongs_queued: Vec<Instant>,
    times: PongTimes,
}

#[derive(Debug)]
struct Shared {
    io: Mutex<Io>,
    out_ready: Condvar,
    drained: Condvar,
    inbox_room: Condvar,
    lost: AtomicBool,
    pong_from: AtomicI32,
    notify: Mutex<Option<Arc<Notify>>>,
}

impl Shared {
    fn notify(&self) {
        let notify = lock(&self.notify).clone();
        if let Some(n) = notify {
            n.notify();
        }
    }

    fn lose(&self) {
        self.lost.store(true, Ordering::SeqCst);
        drop(lock(&self.io));
        self.drained.notify_all();
        self.out_ready.notify_all();
        self.inbox_room.notify_all();
        self.notify();
    }
}

fn reader(mut stream: TcpStream, shared: Arc<Shared>, pongs: bool) {
    let mut splitter = Splitter::with_greeting();
    let mut following = pongs;
    let mut buf = [0u8; 0x1000];
    loop {
        {
            let mut io = lock(&shared.io);
            while io.inbox.len() >= INBOX_LIMIT
                && !io.closing
                && !shared.lost.load(Ordering::SeqCst)
            {
                io = shared
                    .inbox_room
                    .wait(io)
                    .unwrap_or_else(|e| e.into_inner());
            }
            if shared.lost.load(Ordering::SeqCst) {
                break;
            }
        }
        let n = match stream.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => n,
            Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
            Err(_) => break,
        };
        let at = Instant::now();
        let mut queued = false;
        {
            let mut io = lock(&shared.io);
            io.inbox.extend_from_slice(&buf[..n]);
            io.arrived = Some(at);
            if following {
                splitter.push(&buf[..n]);
                loop {
                    match splitter.next_frame() {
                        Ok(Some(Frame::Message(bytes))) => {
                            queued |= answer_ping(&bytes, at, &shared, &mut io);
                        }
                        Ok(Some(Frame::Greeting(_))) => {}
                        Ok(None) => break,
                        Err(_) => {
                            following = false;
                            break;
                        }
                    }
                }
            }
        }
        if queued {
            shared.out_ready.notify_all();
        }
        shared.notify();
    }
    shared.lose();
}

fn answer_ping(bytes: &[u8], at: Instant, shared: &Shared, io: &mut Io) -> bool {
    let Ok(from) = u8::try_from(shared.pong_from.load(Ordering::SeqCst)) else {
        return false;
    };
    if bytes.get(..2) != Some(&crate::wire::kind::PING.to_be_bytes()[..]) {
        return false;
    }
    let Ok(Message {
        body: Body::Ping { sdl_ticks, .. },
        ..
    }) = Message::decode(bytes)
    else {
        return false;
    };
    let Ok(pong) = Message::new(TO_HOST, from, Body::Pong { sdl_ticks }).encode() else {
        return false;
    };
    if io.out.len() + io.writing + pong.len() > MAX_QUEUE_SIZE {
        return false;
    }
    io.out.extend_from_slice(&pong);
    io.answered.push((sdl_ticks, at));
    io.pongs_queued.push(at);
    true
}

fn writer(mut stream: TcpStream, shared: Arc<Shared>) {
    loop {
        let (bytes, pongs) = {
            let mut io = lock(&shared.io);
            loop {
                if shared.lost.load(Ordering::SeqCst) {
                    return;
                }
                if !io.out.is_empty() {
                    break;
                }
                if io.closing {
                    return;
                }
                io = shared.out_ready.wait(io).unwrap_or_else(|e| e.into_inner());
            }
            let bytes = std::mem::take(&mut io.out);
            io.writing = bytes.len();
            (bytes, std::mem::take(&mut io.pongs_queued))
        };
        let sent = stream.write_all(&bytes).is_ok();
        let done = Instant::now();
        {
            let mut io = lock(&shared.io);
            io.writing = 0;
            if sent {
                io.times
                    .answered_us
                    .extend(pongs.iter().map(|at| micros(done - *at)));
            }
        }
        shared.drained.notify_all();
        if !sent {
            shared.lose();
            return;
        }
    }
}

fn micros(d: Duration) -> u32 {
    u32::try_from(d.as_micros()).unwrap_or(u32::MAX)
}

#[derive(Debug)]
enum Stream {
    Tcp(TcpStream, Arc<Shared>),
    Other(Box<dyn ByteStream>),
}

#[derive(Debug)]
pub struct Connection {
    stream: Stream,
    udp: Option<UdpSocket>,
    udp_port: u16,
    send: Vec<u8>,
    lost: bool,
    closed: bool,
}

impl Connection {
    pub fn connect(host: SocketAddr, timeout: Duration) -> io::Result<Connection> {
        let local = match host {
            SocketAddr::V4(_) => SocketAddr::from(([0, 0, 0, 0], 0)),
            SocketAddr::V6(_) => SocketAddr::from(([0u16; 8], 0)),
        };
        let udp = UdpSocket::bind(local)?;
        udp.set_nonblocking(true)?;
        let stream = TcpStream::connect_timeout(&host, timeout)?;
        let udp_port = udp.local_addr()?.port();
        Ok(Connection {
            stream: Connection::threads(stream, true)?,
            udp: Some(udp),
            udp_port,
            send: Vec::new(),
            lost: false,
            closed: false,
        })
    }

    pub fn accepted(stream: TcpStream, udp_port: u16) -> io::Result<Connection> {
        Ok(Connection {
            stream: Connection::threads(stream, false)?,
            udp: None,
            udp_port,
            send: Vec::new(),
            lost: false,
            closed: false,
        })
    }

    fn threads(stream: TcpStream, pongs: bool) -> io::Result<Stream> {
        stream.set_nodelay(true)?;
        stream.set_nonblocking(false)?;
        let shared = Arc::new(Shared {
            io: Mutex::new(Io::default()),
            out_ready: Condvar::new(),
            drained: Condvar::new(),
            inbox_room: Condvar::new(),
            lost: AtomicBool::new(false),
            pong_from: AtomicI32::new(-1),
            notify: Mutex::new(None),
        });
        let (r, s) = (stream.try_clone()?, shared.clone());
        std::thread::Builder::new()
            .name("mandrake-net read".into())
            .spawn(move || reader(r, s, pongs))?;
        let (w, s) = (stream.try_clone()?, shared.clone());
        let spawned = std::thread::Builder::new()
            .name("mandrake-net write".into())
            .spawn(move || writer(w, s));
        if let Err(e) = spawned {
            let _ = stream.shutdown(Shutdown::Both);
            return Err(e);
        }
        Ok(Stream::Tcp(stream, shared))
    }

    pub fn over(stream: Box<dyn ByteStream>) -> Connection {
        Connection {
            stream: Stream::Other(stream),
            udp: None,
            udp_port: 0,
            send: Vec::new(),
            lost: false,
            closed: false,
        }
    }

    pub fn udp_port(&self) -> u16 {
        self.udp_port
    }

    pub fn peer_addr(&self) -> Option<SocketAddr> {
        match &self.stream {
            Stream::Tcp(s, _) => s.peer_addr().ok(),
            Stream::Other(_) => None,
        }
    }

    pub fn set_peer_udp_port(&mut self, port: u16) {
        if let (Some(udp), Some(mut peer)) = (&self.udp, self.peer_addr()) {
            peer.set_port(port);
            let _ = udp.connect(peer);
        }
    }

    pub fn set_notify(&mut self, notify: Arc<Notify>) {
        if let Stream::Tcp(_, shared) = &self.stream {
            let io = lock(&shared.io);
            *lock(&shared.notify) = Some(notify.clone());
            let early = !io.inbox.is_empty() || shared.lost.load(Ordering::SeqCst);
            drop(io);
            if early {
                notify.notify();
            }
        }
    }

    pub fn set_pong_from(&mut self, id: Option<u8>) {
        if let Stream::Tcp(_, shared) = &self.stream {
            shared
                .pong_from
                .store(id.map_or(-1, i32::from), Ordering::SeqCst);
        }
    }

    pub fn take_pong_times(&mut self) -> PongTimes {
        match &self.stream {
            Stream::Tcp(_, shared) => std::mem::take(&mut lock(&shared.io).times),
            Stream::Other(_) => PongTimes::default(),
        }
    }

    pub fn is_lost(&self) -> bool {
        match &self.stream {
            Stream::Tcp(_, shared) => self.lost || shared.lost.load(Ordering::SeqCst),
            Stream::Other(_) => self.lost,
        }
    }

    pub fn read(&mut self, out: &mut Vec<u8>) -> (Option<Instant>, Vec<u32>) {
        match &mut self.stream {
            Stream::Other(s) => {
                if !self.lost && s.read(out).is_err() {
                    self.lost = true;
                }
                (None, Vec::new())
            }
            Stream::Tcp(_, shared) => {
                let mut io = lock(&shared.io);
                if io.inbox.is_empty() {
                    return (None, Vec::new());
                }
                out.append(&mut io.inbox);
                shared.inbox_room.notify_all();
                let now = Instant::now();
                let answered = std::mem::take(&mut io.answered);
                io.times
                    .seen_us
                    .extend(answered.iter().map(|(_, at)| micros(now - *at)));
                (
                    io.arrived.take(),
                    answered.into_iter().map(|(ticks, _)| ticks).collect(),
                )
            }
        }
    }

    pub fn queue(&mut self, bytes: &[u8]) {
        match &self.stream {
            Stream::Other(_) => {
                if self.send.len() + bytes.len() > MAX_QUEUE_SIZE {
                    self.lost = true;
                    return;
                }
                self.send.extend_from_slice(bytes);
            }
            Stream::Tcp(_, shared) => {
                let mut io = lock(&shared.io);
                if io.out.len() + io.writing + bytes.len() > MAX_QUEUE_SIZE {
                    self.lost = true;
                    return;
                }
                io.out.extend_from_slice(bytes);
            }
        }
    }

    pub fn flush(&mut self) {
        match &mut self.stream {
            Stream::Tcp(_, shared) => shared.out_ready.notify_all(),
            Stream::Other(s) => {
                while !self.lost && !self.send.is_empty() {
                    match s.write(&self.send) {
                        Ok(0) => self.lost = true,
                        Ok(n) => {
                            self.send.drain(..n);
                        }
                        Err(e) if e.kind() == io::ErrorKind::WouldBlock => break,
                        Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
                        Err(_) => self.lost = true,
                    }
                }
            }
        }
    }

    pub fn pending(&self) -> usize {
        match &self.stream {
            Stream::Tcp(_, shared) => {
                let io = lock(&shared.io);
                io.out.len() + io.writing
            }
            Stream::Other(_) => self.send.len(),
        }
    }

    pub fn close(&mut self) {
        if self.closed {
            return;
        }
        self.closed = true;
        let overflowed = std::mem::replace(&mut self.lost, true);
        match &mut self.stream {
            Stream::Tcp(s, shared) => {
                let has_pending = {
                    let mut io = lock(&shared.io);
                    io.closing = true;
                    !io.out.is_empty() || io.writing > 0
                };
                shared.out_ready.notify_all();
                shared.inbox_room.notify_all();
                let drain = (has_pending && !overflowed && !shared.lost.load(Ordering::SeqCst))
                    .then(|| s.try_clone().ok())
                    .flatten()
                    .and_then(|stream| {
                        let shared = shared.clone();
                        std::thread::Builder::new()
                            .name("mandrake-net close".into())
                            .spawn(move || drain_and_shut(&stream, &shared))
                            .ok()
                    });
                if drain.is_none() {
                    let _ = s.shutdown(Shutdown::Both);
                }
            }
            Stream::Other(s) => s.close(),
        }
    }
}

fn drain_and_shut(stream: &TcpStream, shared: &Shared) {
    let until = Instant::now() + DRAIN_WAIT;
    let mut io = lock(&shared.io);
    while io.out.len() + io.writing > 0 && !shared.lost.load(Ordering::SeqCst) {
        let Some(rest) = until.checked_duration_since(Instant::now()) else {
            break;
        };
        io = shared
            .drained
            .wait_timeout(io, rest)
            .map(|(g, _)| g)
            .unwrap_or_else(|e| e.into_inner().0);
    }
    drop(io);
    let _ = stream.shutdown(Shutdown::Both);
}

impl Drop for Connection {
    fn drop(&mut self) {
        if matches!(self.stream, Stream::Tcp(..)) {
            self.close();
        }
    }
}

#[derive(Debug)]
pub struct Listener {
    tcp: TcpListener,
    udp: UdpSocket,
}

impl Listener {
    pub fn bind(addr: SocketAddr) -> io::Result<Listener> {
        let tcp = TcpListener::bind(addr)?;
        tcp.set_nonblocking(true)?;
        let mut at = addr;
        at.set_port(tcp.local_addr()?.port());
        let udp = UdpSocket::bind(at)?;
        udp.set_nonblocking(true)?;
        Ok(Listener { tcp, udp })
    }

    pub fn local_addr(&self) -> io::Result<SocketAddr> {
        self.tcp.local_addr()
    }

    pub fn udp_port(&self) -> u16 {
        self.udp.local_addr().map(|a| a.port()).unwrap_or(0)
    }

    pub fn accept_all(&self) -> Vec<TcpStream> {
        let mut out = Vec::new();
        while out.len() < ACCEPTS_PER_POLL {
            match self.tcp.accept() {
                Ok((s, _)) => out.push(s),
                Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
                Err(_) => break,
            }
        }
        out
    }

    pub fn udp_addr(&self) -> Option<std::net::SocketAddrV4> {
        match self.udp.local_addr() {
            Ok(SocketAddr::V4(a)) => Some(a),
            _ => None,
        }
    }

    pub fn answer_queries(&self, answer: Option<&crate::lan::Answer>) -> usize {
        let mut buf = [0u8; crate::lan::QUERY_SIZE];
        let mut sent = 0;
        for _ in 0..DATAGRAMS_PER_POLL {
            match self.udp.recv_from(&mut buf) {
                Ok((n, from)) => {
                    if let Some(a) = crate::lan::answer_for(&buf[..n], answer)
                        && self.udp.send_to(&a, from).is_ok()
                    {
                        sent += 1;
                    }
                }
                Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
                Err(_) => break,
            }
        }
        sent
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::TcpListener;

    #[test]
    fn a_connection_sends_reads_and_sees_eof() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let mut c = Connection::connect(addr, Duration::from_secs(2)).unwrap();
        let (mut host, _) = listener.accept().unwrap();
        assert_ne!(c.udp_port(), 0);
        c.queue(&[1, 2, 3]);
        c.flush();
        let mut got = [0u8; 3];
        host.read_exact(&mut got).unwrap();
        assert_eq!(got, [1, 2, 3]);
        for _ in 0..200 {
            if c.pending() == 0 {
                break;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        assert_eq!(c.pending(), 0);
        host.write_all(&[9, 8]).unwrap();
        let mut seen = Vec::new();
        for _ in 0..200 {
            c.read(&mut seen);
            if seen.len() == 2 {
                break;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        assert_eq!(seen, [9, 8]);
        drop(host);
        for _ in 0..200 {
            c.read(&mut seen);
            if c.is_lost() {
                break;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(c.is_lost());
    }

    #[test]
    fn a_joiners_socket_answers_pings_itself() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let mut c = Connection::connect(addr, Duration::from_secs(2)).unwrap();
        let notify = Arc::new(Notify::default());
        c.set_notify(notify.clone());
        let (mut host, _) = listener.accept().unwrap();
        host.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
        let ping = |ticks: u32| {
            let body = Body::Ping {
                sdl_ticks: ticks,
                latencies: vec![0, 7],
            };
            Message::new(1, 0, body).encode().unwrap()
        };
        let mut stream = crate::wire::encode_greeting(3453).to_vec();
        stream.extend(ping(100));
        host.write_all(&stream).unwrap();
        assert!(notify.wait(Duration::from_secs(5)));
        let mut seen = Vec::new();
        for _ in 0..200 {
            let (at, answered) = c.read(&mut seen);
            assert!(answered.is_empty());
            assert_eq!(at.is_some(), !seen.is_empty());
            if seen.len() == stream.len() {
                break;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        assert_eq!(seen, stream);
        c.set_pong_from(Some(1));
        let second = ping(200);
        host.write_all(&second[..5]).unwrap();
        std::thread::sleep(Duration::from_millis(20));
        host.write_all(&second[5..]).unwrap();
        let pong = Message::new(TO_HOST, 1, Body::Pong { sdl_ticks: 200 })
            .encode()
            .unwrap();
        let mut got = vec![0u8; pong.len()];
        host.read_exact(&mut got).unwrap();
        assert_eq!(got, pong);
        let mut seen = Vec::new();
        let mut answered = Vec::new();
        for _ in 0..200 {
            answered.extend(c.read(&mut seen).1);
            if seen.len() == second.len() {
                break;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        assert_eq!((seen, answered), (second, vec![200]));
        let mut times = c.take_pong_times();
        for _ in 0..1000 {
            if !times.answered_us.is_empty() {
                break;
            }
            std::thread::sleep(Duration::from_millis(5));
            let more = c.take_pong_times();
            times.answered_us.extend(more.answered_us);
            times.seen_us.extend(more.seen_us);
        }
        assert_eq!((times.answered_us.len(), times.seen_us.len()), (1, 1));
        assert_eq!(c.take_pong_times(), PongTimes::default());
        let idle = Notify::default();
        let start = Instant::now();
        assert!(!idle.wait(Duration::from_millis(10)));
        assert!(start.elapsed() >= Duration::from_millis(10));
        idle.notify();
        assert!(idle.wait(Duration::from_secs(5)));
        assert!(!idle.wait(Duration::ZERO));
    }

    #[test]
    fn a_dropped_connection_sends_what_is_queued_and_closes() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let mut c = Connection::connect(addr, Duration::from_secs(2)).unwrap();
        let (mut host, _) = listener.accept().unwrap();
        host.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
        c.queue(&[5; 3000]);
        c.flush();
        drop(c);
        let mut got = Vec::new();
        host.read_to_end(&mut got).unwrap();
        assert_eq!(got, vec![5; 3000]);
    }

    #[derive(Debug, Default)]
    struct Memory {
        inbox: Vec<u8>,
        sent: Vec<u8>,
        room: usize,
        closed: bool,
    }

    #[derive(Debug, Clone, Default)]
    struct Shared(std::sync::Arc<std::sync::Mutex<Memory>>);

    impl ByteStream for Shared {
        fn read(&mut self, out: &mut Vec<u8>) -> io::Result<()> {
            let mut m = self.0.lock().unwrap();
            if m.closed {
                return Err(io::ErrorKind::UnexpectedEof.into());
            }
            out.append(&mut m.inbox);
            Ok(())
        }

        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            let mut m = self.0.lock().unwrap();
            let n = bytes.len().min(m.room);
            if n == 0 {
                return Err(io::ErrorKind::WouldBlock.into());
            }
            m.sent.extend_from_slice(&bytes[..n]);
            Ok(n)
        }

        fn close(&mut self) {
            self.0.lock().unwrap().closed = true;
        }
    }

    #[test]
    fn a_connection_over_another_stream_sends_reads_and_closes() {
        let shared = Shared::default();
        shared.0.lock().unwrap().room = 2;
        let mut c = Connection::over(Box::new(shared.clone()));
        assert_eq!(c.udp_port(), 0);
        assert_eq!(c.peer_addr(), None);
        c.queue(&[1, 2, 3]);
        c.flush();
        assert_eq!(c.pending(), 0);
        assert_eq!(shared.0.lock().unwrap().sent, [1, 2, 3]);
        shared.0.lock().unwrap().room = 0;
        c.queue(&[4]);
        c.flush();
        assert_eq!(c.pending(), 1);
        assert!(!c.is_lost());
        shared.0.lock().unwrap().inbox = vec![9, 8];
        let mut seen = Vec::new();
        c.read(&mut seen);
        assert_eq!(seen, [9, 8]);
        c.close();
        assert!(c.is_lost());
        assert!(shared.0.lock().unwrap().closed);
    }

    #[test]
    fn an_overflowing_queue_loses_the_connection() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let mut c =
            Connection::connect(listener.local_addr().unwrap(), Duration::from_secs(2)).unwrap();
        c.queue(&vec![0; MAX_QUEUE_SIZE]);
        assert!(!c.is_lost());
        c.queue(&[0]);
        assert!(c.is_lost());
    }

    #[test]
    fn the_listener_accepts_with_udp_on_its_port() {
        let l = Listener::bind("127.0.0.1:0".parse().unwrap()).unwrap();
        let addr = l.local_addr().unwrap();
        assert_eq!(l.udp_port(), addr.port());
        assert!(l.accept_all().is_empty());
        let mut c = Connection::connect(addr, Duration::from_secs(2)).unwrap();
        let mut accepted = Vec::new();
        for _ in 0..200 {
            accepted.extend(l.accept_all());
            if !accepted.is_empty() {
                break;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        let mut h = Connection::accepted(accepted.pop().unwrap(), l.udp_port()).unwrap();
        h.queue(&[7]);
        h.flush();
        let mut seen = Vec::new();
        for _ in 0..200 {
            c.read(&mut seen);
            if !seen.is_empty() {
                break;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        assert_eq!(seen, [7]);
    }

    #[test]
    fn an_arrival_before_the_notify_is_set_notifies() {
        use std::sync::atomic::AtomicUsize;
        let l = Listener::bind("127.0.0.1:0".parse().unwrap()).unwrap();
        let mut c = Connection::connect(l.local_addr().unwrap(), Duration::from_secs(2)).unwrap();
        let mut accepted = Vec::new();
        for _ in 0..200 {
            accepted.extend(l.accept_all());
            if !accepted.is_empty() {
                break;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        let mut h = Connection::accepted(accepted.pop().unwrap(), l.udp_port()).unwrap();
        c.queue(&[7]);
        c.flush();
        let Stream::Tcp(_, shared) = &h.stream else {
            unreachable!()
        };
        let until = Instant::now() + Duration::from_secs(5);
        while lock(&shared.io).inbox.is_empty() && Instant::now() < until {
            std::thread::yield_now();
        }
        assert!(!lock(&shared.io).inbox.is_empty());
        let notify = Arc::new(Notify::default());
        let wakes = Arc::new(AtomicUsize::new(0));
        let w = wakes.clone();
        notify.set_waker(Some(Arc::new(move || {
            w.fetch_add(1, Ordering::SeqCst);
        })));
        h.set_notify(notify.clone());
        assert_eq!(wakes.load(Ordering::SeqCst), 1);
        assert!(notify.wait(Duration::ZERO));
    }

    #[test]
    fn a_wait_with_no_end_does_not_overflow_the_clock() {
        let n = Notify::default();
        n.notify();
        assert!(n.wait(Duration::MAX));
    }

    #[test]
    fn the_reader_stops_at_its_limit_until_the_caller_reads() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let mut c = Connection::connect(addr, Duration::from_secs(2)).unwrap();
        let (mut peer, _) = listener.accept().unwrap();
        let total = 3 * INBOX_LIMIT;
        let sent: Vec<u8> = (0..total).map(|i| (i % 251) as u8).collect();
        let to_send = sent.clone();
        let writer = std::thread::spawn(move || peer.write_all(&to_send).map(|()| peer));
        std::thread::sleep(Duration::from_millis(300));
        let Stream::Tcp(_, shared) = &c.stream else {
            unreachable!()
        };
        let held = lock(&shared.io).inbox.len();
        assert!(held <= INBOX_LIMIT + 0x1000, "{held}");
        let mut got = Vec::new();
        let until = Instant::now() + Duration::from_secs(10);
        while got.len() < total && Instant::now() < until {
            c.read(&mut got);
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(got == sent, "every byte, in order");
        let _peer = writer.join().unwrap().unwrap();
    }

    #[test]
    fn closing_does_not_wait_for_a_peer_that_stopped_reading() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let mut c = Connection::connect(addr, Duration::from_secs(2)).unwrap();
        let (_peer, _) = listener.accept().unwrap();
        let chunk = vec![0u8; MAX_QUEUE_SIZE];
        let until = Instant::now() + Duration::from_secs(20);
        let mut stuck_since = None;
        while Instant::now() < until {
            if c.pending() == 0 {
                stuck_since = None;
                c.queue(&chunk);
                c.flush();
            } else if stuck_since.get_or_insert_with(Instant::now).elapsed()
                > Duration::from_millis(100)
            {
                break;
            }
            std::thread::sleep(Duration::from_millis(1));
        }
        assert!(c.pending() > 0, "the peer's buffers are full");
        let at = Instant::now();
        c.close();
        assert!(
            at.elapsed() < Duration::from_millis(50),
            "{:?}",
            at.elapsed()
        );
        assert!(c.is_lost());
    }

    #[test]
    fn a_poll_accepts_a_bounded_number_of_connections() {
        let listener = Listener::bind("127.0.0.1:0".parse().unwrap()).unwrap();
        let addr = listener.local_addr().unwrap();
        let streams: Vec<TcpStream> = (0..ACCEPTS_PER_POLL + 4)
            .map(|_| TcpStream::connect(addr).unwrap())
            .collect();
        std::thread::sleep(Duration::from_millis(50));
        assert_eq!(listener.accept_all().len(), ACCEPTS_PER_POLL);
        assert_eq!(listener.accept_all().len(), 4);
        drop(streams);
    }

    #[test]
    fn a_poll_answers_a_bounded_number_of_udp_queries() {
        let listener = Listener::bind("127.0.0.1:0".parse().unwrap()).unwrap();
        let udp_addr = listener.udp_addr().unwrap();
        let answer = crate::lan::Answer::new(*udp_addr.ip(), udp_addr.port(), b"Bounded");
        let sender = UdpSocket::bind("127.0.0.1:0").unwrap();
        let q = crate::lan::query();
        for _ in 0..DATAGRAMS_PER_POLL + 5 {
            sender.send_to(&q, udp_addr).unwrap();
        }
        std::thread::sleep(Duration::from_millis(50));
        assert_eq!(listener.answer_queries(Some(&answer)), DATAGRAMS_PER_POLL);
        assert_eq!(listener.answer_queries(Some(&answer)), 5);
    }
}
