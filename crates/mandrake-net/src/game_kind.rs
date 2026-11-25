use crate::game_description::GameDescription;

pub const TFL_GAMEPLAY_VERSION: u32 = u32::from_be_bytes(*b"TFL5");
const TFL_VERSION_AT: std::ops::Range<usize> = 0x24..0x28;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub enum GameKind {
    #[default]
    Myth2,
    Tfl,
}

impl GameKind {
    pub fn of(description: &GameDescription) -> GameKind {
        if description.parameters.version == TFL_GAMEPLAY_VERSION {
            GameKind::Tfl
        } else {
            GameKind::Myth2
        }
    }

    pub const fn commands(self) -> CommandCodes {
        match self {
            GameKind::Myth2 => MYTH2_COMMANDS,
            GameKind::Tfl => TFL_COMMANDS,
        }
    }

    pub const fn join_answer_timeout_ms(self) -> u64 {
        match self {
            GameKind::Myth2 => crate::topology::JOIN_ANSWER_TIMEOUT_MS,
            GameKind::Tfl => crate::topology::TFL_JOIN_ANSWER_TIMEOUT_MS,
        }
    }

    pub const fn chat_shift(self) -> u8 {
        match self {
            GameKind::Myth2 => crate::wire::CHAT_SHIFT,
            GameKind::Tfl => 0,
        }
    }

    pub fn decoded(self, mut m: crate::wire::Message) -> crate::wire::Message {
        if self == GameKind::Tfl
            && let crate::wire::Body::GameDescription(raw) = &mut m.body
            && let Some(version) = raw.get_mut(TFL_VERSION_AT)
            && *version == [0; 4]
        {
            version.copy_from_slice(&TFL_GAMEPLAY_VERSION.to_be_bytes());
        }
        let undo = crate::wire::CHAT_SHIFT.wrapping_sub(self.chat_shift());
        if undo != 0
            && let crate::wire::Body::Chat(c) = &mut m.body
        {
            for b in &mut c.text {
                *b = b.wrapping_add(undo);
            }
        }
        m
    }

    pub fn encode(self, m: &crate::wire::Message) -> Result<Vec<u8>, crate::wire::WireError> {
        let undo = crate::wire::CHAT_SHIFT.wrapping_sub(self.chat_shift());
        match &m.body {
            crate::wire::Body::Chat(_) if undo != 0 => {
                let mut m = m.clone();
                if let crate::wire::Body::Chat(c) = &mut m.body {
                    for b in &mut c.text {
                        *b = b.wrapping_sub(undo);
                    }
                }
                m.encode()
            }
            _ => m.encode(),
        }
    }

    pub const fn name(self) -> &'static str {
        match self {
            GameKind::Myth2 => "Myth II",
            GameKind::Tfl => "TFL",
        }
    }

    pub const fn switch(self) -> &'static str {
        match self {
            GameKind::Myth2 => "myth2",
            GameKind::Tfl => "tfl",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CommandCodes {
    pub null: u8,
    pub sync: u8,
    pub drop_player: u8,
    pub chat: u8,
    pub unit_adjustment: u8,
    pub forced_end: Option<u8>,
    pub pause: Option<u8>,
    pub game_speed: Option<u8>,
    pub highest: u8,
}

impl CommandCodes {
    pub const fn has(&self, kind: u8) -> bool {
        kind <= self.highest
    }
}

pub const MYTH2_COMMANDS: CommandCodes = CommandCodes {
    null: 0,
    sync: 1,
    drop_player: 9,
    chat: 10,
    unit_adjustment: 12,
    forced_end: Some(0x15),
    pause: Some(0x17),
    game_speed: Some(0x18),
    highest: 0x18,
};

pub const TFL_COMMANDS: CommandCodes = CommandCodes {
    null: 0,
    sync: 1,
    drop_player: 8,
    chat: 9,
    unit_adjustment: 11,
    forced_end: None,
    pause: None,
    game_speed: None,
    highest: 13,
};

#[cfg(test)]
mod chat_tests {
    use super::GameKind;
    use crate::wire::{Body, Chat, Message, TO_ALL};

    #[test]
    fn tfl_chat_text_is_not_shifted() {
        let m = Message::new(
            TO_ALL,
            0,
            Body::Chat(Chat {
                name: [0; 32],
                words: [0; 8],
                text: b"Hello. This is".to_vec(),
            }),
        );
        let tfl = GameKind::Tfl.encode(&m).unwrap();
        let myth2 = GameKind::Myth2.encode(&m).unwrap();
        assert_eq!(myth2, m.encode().unwrap());
        let text_at = |b: &[u8]| b[b.len() - 15..b.len() - 1].to_vec();
        assert_eq!(text_at(&tfl), b"Hello. This is");
        let read_as_myth2 = match Message::decode(&tfl).unwrap().body {
            Body::Chat(c) => c.text,
            _ => unreachable!(),
        };
        assert_eq!(read_as_myth2[..6], [59, 88, 95, 95, 98, 33]);
        for (kind, bytes) in [(GameKind::Tfl, tfl), (GameKind::Myth2, myth2)] {
            assert_eq!(
                kind.decoded(Message::decode(&bytes).unwrap()),
                m,
                "{kind:?}"
            );
        }
    }

    #[test]
    fn tfl_chat_preserves_0xf3_and_cuts_at_nul() {
        let make = |text: &[u8]| {
            Message::new(
                TO_ALL,
                0,
                Body::Chat(Chat {
                    name: [0; 32],
                    words: [0; 8],
                    text: text.to_vec(),
                }),
            )
        };
        let tfl_bytes = GameKind::Tfl.encode(&make(b"a\xf3b\x00c")).unwrap();
        let decoded = GameKind::Tfl.decoded(Message::decode(&tfl_bytes).unwrap());
        assert_eq!(decoded, make(b"a\xf3b"));
        let m2_bytes = GameKind::Myth2.encode(&make(b"a\xf3b")).unwrap();
        let decoded = GameKind::Myth2.decoded(Message::decode(&m2_bytes).unwrap());
        assert_eq!(decoded, make(b"a"));
    }
}
