use std::fmt;

pub const HEADER_SIZE: usize = 8;
pub const MAX_MESSAGE_SIZE: usize = 0x1000;
pub const MAX_QUEUE_SIZE: usize = 0x4000;
pub const GREETING_SIZE: usize = 2;
pub const DEFAULT_PORT: u16 = 3453;
pub const TO_HOST: u8 = 0xfe;
pub const TO_ALL: u8 = 0xff;
pub const CHAT_SHIFT: u8 = 13;
pub const CHAT_FIXED_SIZE: usize = 32 + 16;
pub const COMMAND_HEADER_SIZE: usize = 8;

pub mod kind {
    pub const JOIN_REQUEST: u16 = 0x8000;
    pub const PLAYER_DATA_CHANGED: u16 = 0x8001;
    pub const GAME_DATA_CHANGED: u16 = 0x8002;
    pub const NEW_PLAYER: u16 = 0x8003;
    pub const JOIN_DENIED: u16 = 0x8004;
    pub const PLAYER_DELETED: u16 = 0x8005;
    pub const GAME_DATA: u16 = 0x8006;
    pub const CONNECTION_CLOSED: u16 = 0x8007;
    pub const CHAT: u16 = 0;
    pub const COMMAND: u16 = 1;
    pub const CONFIGURATION: u16 = 2;
    pub const PLAYER_SYNC_DATA: u16 = 3;
    pub const BEGIN_SYNC_DATA: u16 = 4;
    pub const SERVER_COMMAND: u16 = 5;
    pub const GAME_DESCRIPTION: u16 = 6;
    pub const MYTH_PLAYER_DATA: u16 = 7;
    pub const VETS: u16 = 8;
    pub const PING: u16 = 9;
    pub const BEGIN_GAME_MIDGAME: u16 = 0xa;
    pub const REQUEST_MORE_COMMANDS: u16 = 0xb;
    pub const SAVED_GAME_HEADER: u16 = 0xc;
    pub const SAVED_GAME_PROGRESS: u16 = 0xd;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WireError {
    BadLength(u16),
    LengthMismatch { header: usize, actual: usize },
    TooLong(usize),
    UnknownType(u16),
    BadBody { kind: u16, reason: &'static str },
}

impl fmt::Display for WireError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            WireError::BadLength(n) => write!(f, "message length {n} outside 8..=0x1000"),
            WireError::LengthMismatch { header, actual } => {
                write!(f, "header says {header} bytes, have {actual}")
            }
            WireError::TooLong(n) => write!(f, "message of {n} bytes is over 0x1000"),
            WireError::UnknownType(k) => write!(f, "unknown message type {k:#06x}"),
            WireError::BadBody { kind, reason } => write!(f, "type {kind:#06x}: {reason}"),
        }
    }
}

impl std::error::Error for WireError {}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Header {
    pub kind: u16,
    pub length: u16,
    pub to: u8,
    pub from: u8,
    pub spare: u16,
}

impl Header {
    pub fn read(bytes: &[u8]) -> Option<Header> {
        let b = bytes.get(..HEADER_SIZE)?;
        Some(Header {
            kind: u16::from_be_bytes([b[0], b[1]]),
            length: u16::from_be_bytes([b[2], b[3]]),
            to: b[4],
            from: b[5],
            spare: u16::from_be_bytes([b[6], b[7]]),
        })
    }

    pub fn to_bytes(&self) -> [u8; HEADER_SIZE] {
        let [k0, k1] = self.kind.to_be_bytes();
        let [l0, l1] = self.length.to_be_bytes();
        let [s0, s1] = self.spare.to_be_bytes();
        [k0, k1, l0, l1, self.to, self.from, s0, s1]
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Command {
    pub kind: u8,
    pub player: u8,
    pub tick: i32,
    pub payload: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Chat {
    pub name: [u8; 32],
    pub words: [u16; 8],
    pub text: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Body {
    JoinRequest {
        name: Vec<u8>,
        password: Vec<u8>,
        player_data: Vec<u8>,
    },
    PlayerDataChanged {
        id: u16,
        player_data: Vec<u8>,
    },
    GameDataChanged(Vec<u8>),
    NewPlayer {
        id: u16,
        flags: u16,
        name: Vec<u8>,
        player_data: Vec<u8>,
    },
    JoinDenied {
        reason: Vec<u8>,
        error: Option<Vec<u8>>,
    },
    PlayerDeleted {
        id: u16,
    },
    GameData(Vec<u8>),
    ConnectionClosed(Vec<u8>),
    Chat(Chat),
    Command(Command),
    Configuration {
        flags: u16,
        code: u16,
        value: u32,
    },
    PlayerSyncData {
        id: u16,
        value: u16,
    },
    BeginSyncData,
    ServerCommand(u16),
    GameDescription(Vec<u8>),
    MythPlayerData(Vec<u8>),
    Vets(Vec<u8>),
    Ping {
        sdl_ticks: u32,
        latencies: Vec<i16>,
    },
    Pong {
        sdl_ticks: u32,
    },
    BeginGameMidgame(Vec<u8>),
    RequestMoreCommands,
    SavedGameHeader(Vec<u8>),
    SavedGameProgress {
        tick: u32,
    },
    Ours(crate::ours::Ours),
}

impl Body {
    pub fn kind(&self) -> u16 {
        use kind::*;
        match self {
            Body::JoinRequest { .. } => JOIN_REQUEST,
            Body::PlayerDataChanged { .. } => PLAYER_DATA_CHANGED,
            Body::GameDataChanged(_) => GAME_DATA_CHANGED,
            Body::NewPlayer { .. } => NEW_PLAYER,
            Body::JoinDenied { .. } => JOIN_DENIED,
            Body::PlayerDeleted { .. } => PLAYER_DELETED,
            Body::GameData(_) => GAME_DATA,
            Body::ConnectionClosed(_) => CONNECTION_CLOSED,
            Body::Chat(_) => CHAT,
            Body::Command(_) => COMMAND,
            Body::Configuration { .. } => CONFIGURATION,
            Body::PlayerSyncData { .. } => PLAYER_SYNC_DATA,
            Body::BeginSyncData => BEGIN_SYNC_DATA,
            Body::ServerCommand(_) => SERVER_COMMAND,
            Body::GameDescription(_) => GAME_DESCRIPTION,
            Body::MythPlayerData(_) => MYTH_PLAYER_DATA,
            Body::Vets(_) => VETS,
            Body::Ping { .. } | Body::Pong { .. } => PING,
            Body::BeginGameMidgame(_) => BEGIN_GAME_MIDGAME,
            Body::RequestMoreCommands => REQUEST_MORE_COMMANDS,
            Body::SavedGameHeader(_) => SAVED_GAME_HEADER,
            Body::SavedGameProgress { .. } => SAVED_GAME_PROGRESS,
            Body::Ours(o) => o.kind(),
        }
    }

    fn write(&self, out: &mut Vec<u8>) {
        match self {
            Body::JoinRequest {
                name,
                password,
                player_data,
            } => {
                put_cstr(out, name);
                put_cstr(out, password);
                out.extend_from_slice(player_data);
            }
            Body::PlayerDataChanged { id, player_data } => {
                out.extend_from_slice(&id.to_be_bytes());
                out.extend_from_slice(player_data);
            }
            Body::NewPlayer {
                id,
                flags,
                name,
                player_data,
            } => {
                out.extend_from_slice(&id.to_be_bytes());
                out.extend_from_slice(&flags.to_be_bytes());
                put_cstr(out, name);
                out.extend_from_slice(player_data);
            }
            Body::JoinDenied { reason, error } => {
                put_cstr(out, reason);
                if let Some(error) = error {
                    put_cstr(out, error);
                }
            }
            Body::PlayerDeleted { id } => out.extend_from_slice(&id.to_be_bytes()),
            Body::Chat(chat) => {
                out.extend_from_slice(&chat.name);
                for w in chat.words {
                    out.extend_from_slice(&w.to_be_bytes());
                }
                out.extend(
                    chat.text
                        .iter()
                        .map(|b| b.wrapping_add(CHAT_SHIFT))
                        .take_while(|&b| b != 0),
                );
                out.push(0);
            }
            Body::Command(c) => write_command(c, out),
            Body::Configuration { flags, code, value } => {
                out.extend_from_slice(&flags.to_be_bytes());
                out.extend_from_slice(&code.to_be_bytes());
                out.extend_from_slice(&value.to_be_bytes());
            }
            Body::PlayerSyncData { id, value } => {
                out.extend_from_slice(&id.to_be_bytes());
                out.extend_from_slice(&value.to_be_bytes());
            }
            Body::BeginSyncData | Body::RequestMoreCommands => {}
            Body::ServerCommand(op) => out.extend_from_slice(&op.to_be_bytes()),
            Body::Ping {
                sdl_ticks,
                latencies,
            } => {
                out.extend_from_slice(&sdl_ticks.to_le_bytes());
                for l in latencies {
                    out.extend_from_slice(&l.to_be_bytes());
                }
            }
            Body::Pong { sdl_ticks } => out.extend_from_slice(&sdl_ticks.to_le_bytes()),
            Body::SavedGameProgress { tick } => out.extend_from_slice(&tick.to_be_bytes()),
            Body::Ours(o) => o.write(out),
            Body::GameDataChanged(raw)
            | Body::GameData(raw)
            | Body::ConnectionClosed(raw)
            | Body::GameDescription(raw)
            | Body::MythPlayerData(raw)
            | Body::Vets(raw)
            | Body::BeginGameMidgame(raw)
            | Body::SavedGameHeader(raw) => out.extend_from_slice(raw),
        }
    }

    pub fn decode(kind: u16, body: &[u8]) -> Result<Body, WireError> {
        use kind::*;
        let mut r = Reader {
            kind,
            bytes: body,
            at: 0,
        };
        let parsed = match kind {
            JOIN_REQUEST => Body::JoinRequest {
                name: r.cstr()?,
                password: r.cstr()?,
                player_data: r.rest(),
            },
            PLAYER_DATA_CHANGED => Body::PlayerDataChanged {
                id: r.u16()?,
                player_data: r.rest(),
            },
            GAME_DATA_CHANGED => Body::GameDataChanged(r.rest()),
            NEW_PLAYER => Body::NewPlayer {
                id: r.u16()?,
                flags: r.u16()?,
                name: r.cstr()?,
                player_data: r.rest(),
            },
            JOIN_DENIED => {
                let reason = r.cstr()?;
                let error = if r.done() { None } else { Some(r.cstr()?) };
                Body::JoinDenied { reason, error }
            }
            PLAYER_DELETED => Body::PlayerDeleted { id: r.u16()? },
            GAME_DATA => Body::GameData(r.rest()),
            CONNECTION_CLOSED => Body::ConnectionClosed(r.rest()),
            CHAT => {
                let mut name = [0; 32];
                name.copy_from_slice(r.take(32)?);
                let mut words = [0; 8];
                for w in &mut words {
                    *w = r.u16()?;
                }
                let text = r
                    .cstr()?
                    .iter()
                    .map(|b| b.wrapping_sub(CHAT_SHIFT))
                    .collect();
                r.at = body.len();
                Body::Chat(Chat { name, words, text })
            }
            COMMAND => {
                let length = r.u16()? as usize;
                if length != body.len() {
                    return Err(WireError::BadBody {
                        kind,
                        reason: "command length is not the body's",
                    });
                }
                Body::Command(Command {
                    kind: r.u8()?,
                    player: r.u8()?,
                    tick: r.u32()? as i32,
                    payload: r.rest(),
                })
            }
            CONFIGURATION => Body::Configuration {
                flags: r.u16()?,
                code: r.u16()?,
                value: r.u32()?,
            },
            PLAYER_SYNC_DATA => Body::PlayerSyncData {
                id: r.u16()?,
                value: r.u16()?,
            },
            BEGIN_SYNC_DATA => Body::BeginSyncData,
            SERVER_COMMAND => Body::ServerCommand(r.u16()?),
            GAME_DESCRIPTION => Body::GameDescription(r.rest()),
            MYTH_PLAYER_DATA => Body::MythPlayerData(r.rest()),
            VETS => Body::Vets(r.rest()),
            PING => {
                let sdl_ticks = u32::from_le_bytes(r.take(4)?.try_into().expect("4 bytes"));
                if r.done() {
                    Body::Pong { sdl_ticks }
                } else {
                    let mut latencies = Vec::new();
                    while !r.done() {
                        latencies.push(r.u16()? as i16);
                    }
                    Body::Ping {
                        sdl_ticks,
                        latencies,
                    }
                }
            }
            BEGIN_GAME_MIDGAME => Body::BeginGameMidgame(r.rest()),
            REQUEST_MORE_COMMANDS => Body::RequestMoreCommands,
            SAVED_GAME_HEADER => Body::SavedGameHeader(r.rest()),
            SAVED_GAME_PROGRESS => Body::SavedGameProgress { tick: r.u32()? },
            other if crate::ours::is_ours(other) => return crate::ours::Ours::decode(other, body),
            other => return Err(WireError::UnknownType(other)),
        };
        if !r.done() {
            return Err(WireError::BadBody {
                kind,
                reason: "bytes after the body",
            });
        }
        Ok(parsed)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Message {
    pub to: u8,
    pub from: u8,
    pub spare: u16,
    pub body: Body,
}

impl Message {
    pub fn new(to: u8, from: u8, body: Body) -> Message {
        Message {
            to,
            from,
            spare: 0,
            body,
        }
    }

    pub fn decode(bytes: &[u8]) -> Result<Message, WireError> {
        let header = Header::read(bytes).ok_or(WireError::LengthMismatch {
            header: HEADER_SIZE,
            actual: bytes.len(),
        })?;
        check_length(header.length)?;
        if header.length as usize != bytes.len() {
            return Err(WireError::LengthMismatch {
                header: header.length as usize,
                actual: bytes.len(),
            });
        }
        let body = Body::decode(header.kind, &bytes[HEADER_SIZE..])?;
        Ok(Message {
            to: header.to,
            from: header.from,
            spare: header.spare,
            body,
        })
    }

    pub fn encode(&self) -> Result<Vec<u8>, WireError> {
        let mut out = vec![0; HEADER_SIZE];
        self.body.write(&mut out);
        if out.len() > MAX_MESSAGE_SIZE {
            return Err(WireError::TooLong(out.len()));
        }
        let header = Header {
            kind: self.body.kind(),
            length: out.len() as u16,
            to: self.to,
            from: self.from,
            spare: self.spare,
        };
        out[..HEADER_SIZE].copy_from_slice(&header.to_bytes());
        Ok(out)
    }
}

pub fn encode_greeting(udp_port: u16) -> [u8; GREETING_SIZE] {
    udp_port.to_be_bytes()
}

pub fn decode_greeting(bytes: [u8; GREETING_SIZE]) -> u16 {
    u16::from_be_bytes(bytes)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Frame {
    Greeting(u16),
    Message(Vec<u8>),
}

#[derive(Debug, Clone, Default)]
pub struct Splitter {
    buf: Vec<u8>,
    at: usize,
    greeting_pending: bool,
    failed: Option<WireError>,
}

impl Splitter {
    pub fn with_greeting() -> Splitter {
        Splitter {
            greeting_pending: true,
            ..Splitter::default()
        }
    }

    pub fn without_greeting() -> Splitter {
        Splitter::default()
    }

    pub fn push(&mut self, bytes: &[u8]) {
        if self.at > 0 && self.at * 2 >= self.buf.len() {
            self.buf.drain(..self.at);
            self.at = 0;
        }
        self.buf.extend_from_slice(bytes);
    }

    pub fn pending(&self) -> usize {
        self.buf.len() - self.at
    }

    pub fn next_frame(&mut self) -> Result<Option<Frame>, WireError> {
        if let Some(e) = &self.failed {
            return Err(e.clone());
        }
        let rest = &self.buf[self.at..];
        if self.greeting_pending {
            if rest.len() < GREETING_SIZE {
                return Ok(None);
            }
            self.greeting_pending = false;
            self.at += GREETING_SIZE;
            return Ok(Some(Frame::Greeting(decode_greeting([rest[0], rest[1]]))));
        }
        let Some(header) = Header::read(rest) else {
            return Ok(None);
        };
        if let Err(e) = check_length(header.length) {
            self.failed = Some(e.clone());
            return Err(e);
        }
        let length = header.length as usize;
        if rest.len() < length {
            return Ok(None);
        }
        let message = rest[..length].to_vec();
        self.at += length;
        Ok(Some(Frame::Message(message)))
    }
}

fn check_length(length: u16) -> Result<(), WireError> {
    if (HEADER_SIZE..=MAX_MESSAGE_SIZE).contains(&(length as usize)) {
        Ok(())
    } else {
        Err(WireError::BadLength(length))
    }
}

fn put_cstr(out: &mut Vec<u8>, s: &[u8]) {
    out.extend_from_slice(s);
    out.push(0);
}

struct Reader<'a> {
    kind: u16,
    bytes: &'a [u8],
    at: usize,
}

impl<'a> Reader<'a> {
    fn bad(&self, reason: &'static str) -> WireError {
        WireError::BadBody {
            kind: self.kind,
            reason,
        }
    }

    fn take(&mut self, n: usize) -> Result<&'a [u8], WireError> {
        let s = self
            .bytes
            .get(self.at..self.at + n)
            .ok_or_else(|| self.bad("body too short"))?;
        self.at += n;
        Ok(s)
    }

    fn u8(&mut self) -> Result<u8, WireError> {
        Ok(self.take(1)?[0])
    }

    fn u16(&mut self) -> Result<u16, WireError> {
        let b = self.take(2)?;
        Ok(u16::from_be_bytes([b[0], b[1]]))
    }

    fn u32(&mut self) -> Result<u32, WireError> {
        let b = self.take(4)?;
        Ok(u32::from_be_bytes([b[0], b[1], b[2], b[3]]))
    }

    fn cstr(&mut self) -> Result<Vec<u8>, WireError> {
        let rest = &self.bytes[self.at..];
        let end = rest
            .iter()
            .position(|&b| b == 0)
            .ok_or_else(|| self.bad("string without its terminator"))?;
        self.at += end + 1;
        Ok(rest[..end].to_vec())
    }

    fn rest(&mut self) -> Vec<u8> {
        let r = self.bytes[self.at..].to_vec();
        self.at = self.bytes.len();
        r
    }

    fn done(&self) -> bool {
        self.at == self.bytes.len()
    }
}

pub(crate) fn write_command(c: &Command, out: &mut Vec<u8>) {
    let length = (COMMAND_HEADER_SIZE + c.payload.len()) as i16;
    out.extend_from_slice(&length.to_be_bytes());
    out.push(c.kind);
    out.push(c.player);
    out.extend_from_slice(&c.tick.to_be_bytes());
    out.extend_from_slice(&c.payload);
}

pub(crate) fn be16(b: &[u8], at: usize) -> u16 {
    u16::from_be_bytes([b[at], b[at + 1]])
}

pub(crate) fn be32(b: &[u8], at: usize) -> u32 {
    u32::from_be_bytes([b[at], b[at + 1], b[at + 2], b[at + 3]])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_command_whose_length_disagrees_is_refused() {
        let mut bytes = Message::new(
            TO_HOST,
            1,
            Body::Command(Command {
                kind: 0,
                player: 0xff,
                tick: 0,
                payload: vec![],
            }),
        )
        .encode()
        .unwrap();
        bytes[9] = 9;
        assert!(matches!(
            Message::decode(&bytes),
            Err(WireError::BadBody { kind: 1, .. })
        ));
    }

    #[test]
    fn a_message_over_0x1000_bytes_does_not_encode() {
        let m = Message::new(
            TO_ALL,
            TO_HOST,
            Body::Vets(vec![0; MAX_MESSAGE_SIZE - HEADER_SIZE + 1]),
        );
        assert_eq!(m.encode(), Err(WireError::TooLong(MAX_MESSAGE_SIZE + 1)));
        let m = Message::new(
            TO_ALL,
            TO_HOST,
            Body::Vets(vec![0; MAX_MESSAGE_SIZE - HEADER_SIZE]),
        );
        assert_eq!(m.encode().unwrap().len(), MAX_MESSAGE_SIZE);
    }
}
