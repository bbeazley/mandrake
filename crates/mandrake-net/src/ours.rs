use crate::game_kind::GameKind;
use crate::wire::{Body, WireError};

pub const SERVER_MARK: [u8; 6] = *b"m2sv\x00\x01";
pub const CLIENT_MARK: [u8; 6] = *b"m2cl\x00\x01";
pub const VERSION: u16 = 1;

pub mod kind {
    pub const HELLO: u16 = 0x40;
    pub const OWNER: u16 = 0x41;
    pub const DESCRIPTION: u16 = 0x42;
    pub const BOOT: u16 = 0x43;
    pub const LOCK: u16 = 0x44;
    pub const PASSWORD: u16 = 0x45;
    pub const FORCE_END: u16 = 0x46;
    pub const DOWNLOADS: u16 = 0x47;
    pub const GAME: u16 = 0x48;
}

pub fn is_ours(kind: u16) -> bool {
    (kind::HELLO..=kind::GAME).contains(&kind)
}

pub const DOWNLOADS_URL_MAX: usize = 0x200;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Ours {
    Hello { version: u16 },
    Owner { id: u16 },
    Description(Vec<u8>),
    Boot { id: u16 },
    Lock { locked: bool },
    Password(Vec<u8>),
    ForceEnd { kind: i16 },
    Downloads(Vec<u8>),
    Game { kind: GameKind },
}

fn game_word(kind: GameKind) -> u16 {
    match kind {
        GameKind::Myth2 => 0,
        GameKind::Tfl => 1,
    }
}

impl Ours {
    pub fn kind(&self) -> u16 {
        match self {
            Ours::Hello { .. } => kind::HELLO,
            Ours::Owner { .. } => kind::OWNER,
            Ours::Description(_) => kind::DESCRIPTION,
            Ours::Boot { .. } => kind::BOOT,
            Ours::Lock { .. } => kind::LOCK,
            Ours::Password(_) => kind::PASSWORD,
            Ours::ForceEnd { .. } => kind::FORCE_END,
            Ours::Downloads(_) => kind::DOWNLOADS,
            Ours::Game { .. } => kind::GAME,
        }
    }

    pub(crate) fn write(&self, out: &mut Vec<u8>) {
        match self {
            Ours::Hello { version: v } => out.extend_from_slice(&v.to_be_bytes()),
            Ours::Owner { id } | Ours::Boot { id } => out.extend_from_slice(&id.to_be_bytes()),
            Ours::Lock { locked } => out.extend_from_slice(&u16::from(*locked).to_be_bytes()),
            Ours::ForceEnd { kind } => out.extend_from_slice(&kind.to_be_bytes()),
            Ours::Game { kind } => out.extend_from_slice(&game_word(*kind).to_be_bytes()),
            Ours::Description(raw) => out.extend_from_slice(raw),
            Ours::Password(p) | Ours::Downloads(p) => {
                out.extend_from_slice(p);
                out.push(0);
            }
        }
    }

    pub(crate) fn decode(kind: u16, body: &[u8]) -> Result<Body, WireError> {
        let bad = |reason| WireError::BadBody { kind, reason };
        let word = || -> Result<u16, WireError> {
            match body {
                [a, b] => Ok(u16::from_be_bytes([*a, *b])),
                _ => Err(bad("not one u16")),
            }
        };
        let o = match kind {
            kind::HELLO => Ours::Hello { version: word()? },
            kind::OWNER => Ours::Owner { id: word()? },
            kind::DESCRIPTION => Ours::Description(body.to_vec()),
            kind::FORCE_END => Ours::ForceEnd {
                kind: word()? as i16,
            },
            kind::BOOT => Ours::Boot { id: word()? },
            kind::LOCK => Ours::Lock {
                locked: word()? != 0,
            },
            kind::PASSWORD => match body.split_last() {
                Some((0, p)) if !p.contains(&0) => Ours::Password(p.to_vec()),
                _ => return Err(bad("string without its terminator")),
            },
            kind::DOWNLOADS => match body.split_last() {
                Some((0, u)) if !u.contains(&0) && u.len() <= DOWNLOADS_URL_MAX => {
                    Ours::Downloads(u.to_vec())
                }
                _ => return Err(bad("a URL without its terminator, or too long")),
            },
            kind::GAME => Ours::Game {
                kind: match word()? {
                    0 => GameKind::Myth2,
                    1 => GameKind::Tfl,
                    _ => return Err(bad("not a game we know")),
                },
            },
            other => return Err(WireError::UnknownType(other)),
        };
        Ok(Body::Ours(o))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::wire::{Message, TO_HOST};

    #[test]
    fn each_extension_message_round_trips() {
        for o in [
            Ours::Hello { version: VERSION },
            Ours::Owner { id: 3 },
            Ours::Description(vec![1, 2, 3]),
            Ours::Boot { id: 2 },
            Ours::Lock { locked: true },
            Ours::Lock { locked: false },
            Ours::Password(b"secret".to_vec()),
            Ours::Password(Vec::new()),
            Ours::ForceEnd { kind: 2 },
            Ours::ForceEnd { kind: 3 },
            Ours::Downloads(b"http://{host}:8080/plugins/".to_vec()),
            Ours::Game {
                kind: GameKind::Myth2,
            },
            Ours::Game {
                kind: GameKind::Tfl,
            },
        ] {
            let m = Message::new(TO_HOST, 1, Body::Ours(o.clone()));
            let bytes = m.encode().unwrap();
            assert_eq!(u16::from_be_bytes([bytes[0], bytes[1]]), o.kind());
            assert!(is_ours(o.kind()) && o.kind() >= 0xe);
            assert_eq!(Message::decode(&bytes).unwrap(), m, "{o:?}");
        }
    }

    #[test]
    fn a_short_body_is_refused() {
        assert!(Ours::decode(kind::OWNER, &[1]).is_err());
        assert!(Ours::decode(kind::PASSWORD, b"x").is_err());
        assert!(Ours::decode(kind::DOWNLOADS, b"http://x/").is_err());
        let long = [vec![b'a'; DOWNLOADS_URL_MAX + 1], vec![0]].concat();
        assert!(Ours::decode(kind::DOWNLOADS, &long).is_err());
    }
}
