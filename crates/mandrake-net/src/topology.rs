use std::collections::BTreeMap;

use crate::endpoint::Event;
use crate::player_data::{JoinRequest, PlayerData};
use crate::wire::{Body, Message, TO_HOST};

pub const JOIN_ANSWER_TIMEOUT_MS: u64 = 5000;
pub const TFL_JOIN_ANSWER_TIMEOUT_MS: u64 = 15_000;
pub const NEW_PLAYER_IS_YOU: u16 = 1;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Player {
    pub id: u16,
    pub name: Vec<u8>,
    pub data: PlayerData,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JoinState {
    Waiting { deadline_ms: u64 },
    Joined { id: u8 },
    Failed,
}

#[derive(Debug, Clone)]
pub struct Topology {
    state: JoinState,
    request: JoinRequest,
    players: BTreeMap<u16, Player>,
    order: Vec<u16>,
    messages: Vec<Vec<u8>>,
}

pub const MAX_INTERNAL_MESSAGE: u32 = 10;

pub fn denial_code(error: &[u8]) -> Option<u32> {
    let text = std::str::from_utf8(crate::player_data::until_nul(error)).ok()?;
    let digits = text.trim_start().strip_prefix("error=")?;
    let end = digits
        .char_indices()
        .find(|&(i, c)| !(c.is_ascii_digit() || (i == 0 && (c == '-' || c == '+'))))
        .map_or(digits.len(), |(i, _)| i);
    digits[..end].parse::<i64>().ok().map(|n| n as u32)
}

fn internal_message(messages: &[Vec<u8>], n: u32) -> Option<&[u8]> {
    messages
        .get(n as usize)
        .map(Vec::as_slice)
        .filter(|m| !m.is_empty())
}

impl Topology {
    pub fn new(request: JoinRequest, now_ms: u64) -> (Topology, Message) {
        Topology::new_waiting(request, now_ms, JOIN_ANSWER_TIMEOUT_MS)
    }

    pub fn new_waiting(request: JoinRequest, now_ms: u64, wait_ms: u64) -> (Topology, Message) {
        let message = Message::new(TO_HOST, 0, request.to_body());
        let t = Topology {
            state: JoinState::Waiting {
                deadline_ms: now_ms + wait_ms,
            },
            request,
            players: BTreeMap::new(),
            order: Vec::new(),
            messages: Vec::new(),
        };
        (t, message)
    }

    pub fn state(&self) -> JoinState {
        self.state
    }

    pub fn set_internal_messages(&mut self, messages: Vec<Vec<u8>>) {
        self.messages = messages;
    }

    pub fn local_id(&self) -> Option<u8> {
        match self.state {
            JoinState::Joined { id } => Some(id),
            _ => None,
        }
    }

    pub fn players(&self) -> impl Iterator<Item = &Player> {
        self.players.values()
    }

    pub fn list_order(&self) -> &[u16] {
        &self.order
    }

    pub fn sort_for_game(&mut self) {
        let mut records: Vec<PlayerData> = self
            .order
            .iter()
            .filter_map(|id| self.players.get(id))
            .map(|p| {
                let mut r = p.data.clone();
                r.net_id = i32::from(p.id);
                r
            })
            .collect();
        crate::player_data::sort_players(&mut records);
        self.order = records.iter().map(|r| r.net_id as u16).collect();
    }

    pub(crate) fn lower_ready(&mut self, ids: &[u16]) {
        for id in ids {
            if let Some(p) = self.players.get_mut(id) {
                p.data.ready = 0;
            }
        }
    }

    pub fn local_record(&self) -> &PlayerData {
        &self.request.player
    }

    pub fn handle(&mut self, message: &Message, events: &mut Vec<Event>) -> bool {
        match &message.body {
            Body::NewPlayer {
                id,
                flags,
                name,
                player_data,
            } => {
                let Ok(data) = PlayerData::read(player_data) else {
                    events.push(Event::Malformed(message.body.kind()));
                    return true;
                };
                let Some(net_id) = u8::try_from(*id).ok().filter(|&n| n <= MAX_NET_ID) else {
                    events.push(Event::Malformed(message.body.kind()));
                    return true;
                };
                if flags & NEW_PLAYER_IS_YOU != 0
                    && let JoinState::Waiting { .. } = self.state
                {
                    self.state = JoinState::Joined { id: net_id };
                    events.push(Event::Joined { id: net_id });
                }
                let player = Player {
                    id: *id,
                    name: name.clone(),
                    data,
                };
                let new = self.players.insert(*id, player.clone()).is_none();
                if new {
                    self.order.push(*id);
                }
                events.push(Event::Player { player, new });
                true
            }
            Body::JoinDenied { .. } if !matches!(self.state, JoinState::Waiting { .. }) => {
                events.push(Event::Ignored(message.body.kind()));
                true
            }
            Body::JoinDenied { reason, error } => {
                self.state = JoinState::Failed;
                let code = error.as_deref().and_then(denial_code);
                let reason = code
                    .filter(|&n| n <= MAX_INTERNAL_MESSAGE)
                    .and_then(|n| internal_message(&self.messages, n))
                    .map_or_else(|| reason.clone(), <[u8]>::to_vec);
                events.push(Event::Denied { reason });
                true
            }
            Body::PlayerDeleted { id } => {
                if self.players.remove(id).is_some() {
                    self.order.retain(|o| o != id);
                    events.push(Event::PlayerLeft { id: *id });
                }
                true
            }
            Body::JoinRequest { .. }
            | Body::PlayerDataChanged { .. }
            | Body::GameDataChanged(_)
            | Body::GameData(_)
            | Body::ConnectionClosed(_) => true,
            _ => false,
        }
    }

    pub fn poll(&mut self, now_ms: u64, events: &mut Vec<Event>) {
        if let JoinState::Waiting { deadline_ms } = self.state
            && now_ms >= deadline_ms
        {
            self.state = JoinState::Failed;
            events.push(Event::JoinTimedOut);
        }
    }

    pub fn lost(&mut self) {
        self.state = JoinState::Failed;
    }

    pub fn change_local(&mut self, change: impl FnOnce(&mut PlayerData)) -> Option<Message> {
        let id = self.local_id()?;
        if let Some(listed) = self.players.get(&u16::from(id)) {
            self.request.player = listed.data.clone();
        }
        let p = &mut self.request.player;
        p.net_id = i32::from(id);
        change(p);
        Some(Message::new(
            TO_HOST,
            id,
            Body::PlayerDataChanged {
                id: u16::from(id),
                player_data: p.to_bytes().to_vec(),
            },
        ))
    }

    pub fn record_message(&self, record: &PlayerData) -> Option<Message> {
        let id = self.local_id()?;
        Some(Message::new(
            TO_HOST,
            id,
            Body::PlayerDataChanged {
                id: record.net_id as u16,
                player_data: record.to_bytes().to_vec(),
            },
        ))
    }
}

pub const HOST_NET_ID: u8 = 0;
pub const NET_LAYER_SENDER: u8 = TO_HOST;
pub const MAX_NET_ID: u8 = 0x7f;
const RECORD_SIZE: usize = 0x7c;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Denial {
    Malformed,
    PasswordMissing,
    PasswordWrong,
    RecordSize,
    NotOnMetaserver,
    Midgame(Vec<u8>),
}

impl Denial {
    pub fn body(&self) -> Body {
        self.body_with(&[])
    }

    pub fn code(&self) -> Option<u32> {
        match self {
            Denial::PasswordMissing => Some(9),
            Denial::PasswordWrong => Some(10),
            _ => None,
        }
    }

    pub fn body_with(&self, messages: &[Vec<u8>]) -> Body {
        let code = self.code();
        let reason: &[u8] = match self {
            Denial::Malformed | Denial::RecordSize => b"The join request could not be read.",
            Denial::PasswordMissing => b"This game requires a password.",
            Denial::PasswordWrong => b"The password is incorrect.",
            Denial::NotOnMetaserver => b"Game is not hosted on a metaserver.",
            Denial::Midgame(reason) => reason,
        };
        let reason = code
            .and_then(|n| internal_message(messages, n))
            .unwrap_or(reason);
        Body::JoinDenied {
            reason: reason.to_vec(),
            error: code.map(|c| format!("error={c}").into_bytes()),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HostPlayer {
    pub id: u8,
    pub joined: Option<Player>,
}

#[derive(Debug, Clone)]
pub struct HostTopology {
    players: BTreeMap<u8, HostPlayer>,
    next_id: u8,
    max_players: usize,
    password: Vec<u8>,
    accepting: bool,
}

pub type Outgoing = (u8, Message);

impl HostTopology {
    pub fn new(max_players: usize, password: Vec<u8>) -> HostTopology {
        let mut t = HostTopology {
            players: BTreeMap::new(),
            next_id: 1,
            max_players: max_players.clamp(1, crate::game_description::MAX_PLAYERS),
            password,
            accepting: true,
        };
        t.players.insert(
            HOST_NET_ID,
            HostPlayer {
                id: HOST_NET_ID,
                joined: None,
            },
        );
        t
    }

    pub fn set_accepting(&mut self, accepting: bool) {
        self.accepting = accepting;
    }

    pub fn accepting(&self) -> bool {
        self.accepting
    }

    pub fn max_players(&self) -> usize {
        self.max_players
    }

    pub fn set_max_players(&mut self, max_players: usize) {
        self.max_players = max_players.clamp(1, crate::game_description::MAX_PLAYERS);
    }

    pub fn joined(&self) -> impl Iterator<Item = &Player> {
        self.players.values().filter_map(|p| p.joined.as_ref())
    }

    pub fn set_password(&mut self, password: Vec<u8>) {
        self.password = password;
    }

    pub fn joined_ids(&self) -> Vec<u8> {
        self.joined().map(|p| p.id as u8).collect()
    }

    pub fn player(&self, id: u8) -> Option<&Player> {
        self.players.get(&id)?.joined.as_ref()
    }

    pub fn has(&self, id: u8) -> bool {
        self.players.contains_key(&id)
    }

    pub fn accept(&mut self) -> Option<u8> {
        if !self.accepting || self.players.len() + 1 > self.max_players {
            return None;
        }
        let id = self.next_id()?;
        self.players.insert(id, HostPlayer { id, joined: None });
        Some(id)
    }

    fn next_id(&mut self) -> Option<u8> {
        for _ in 0..MAX_NET_ID {
            let id = self.next_id;
            self.next_id = if id >= MAX_NET_ID { 1 } else { id + 1 };
            if !self.players.contains_key(&id) {
                return Some(id);
            }
        }
        None
    }

    pub fn join_request(&mut self, id: u8, body: &Body) -> Result<Vec<Outgoing>, Denial> {
        self.join_request_with(id, body, |_| Ok(()))
    }

    pub fn join_request_with(
        &mut self,
        id: u8,
        body: &Body,
        check: impl FnOnce(&PlayerData) -> Result<(), Denial>,
    ) -> Result<Vec<Outgoing>, Denial> {
        let deny = Err;
        if !self.players.contains_key(&id) {
            return deny(Denial::Malformed);
        }
        let Body::JoinRequest {
            name,
            password,
            player_data,
        } = body
        else {
            return deny(Denial::Malformed);
        };
        if !self.password.is_empty() && *password != self.password {
            return deny(if password.is_empty() {
                Denial::PasswordMissing
            } else {
                Denial::PasswordWrong
            });
        }
        if player_data.len() != RECORD_SIZE {
            return deny(Denial::RecordSize);
        }
        let Ok(data) = PlayerData::read(player_data) else {
            return deny(Denial::RecordSize);
        };
        if data.metaserver != 0 {
            return deny(Denial::NotOnMetaserver);
        }
        check(&data)?;
        let mut name = name.clone();
        name.truncate(0x1f);
        self.players.get_mut(&id).unwrap().joined = Some(Player {
            id: u16::from(id),
            name,
            data,
        });
        let mut out = Vec::new();
        for other in self.joined_ids() {
            out.push(self.new_player(id, other));
            out.push(self.new_player(other, id));
        }
        Ok(out)
    }

    fn new_player(&self, to: u8, about: u8) -> Outgoing {
        let p = self.player(about).expect("a joined player");
        (
            to,
            Message::new(
                to,
                NET_LAYER_SENDER,
                Body::NewPlayer {
                    id: p.id,
                    flags: if to == about { NEW_PLAYER_IS_YOU } else { 0 },
                    name: p.name.clone(),
                    player_data: p.data.to_bytes().to_vec(),
                },
            ),
        )
    }

    pub fn player_changed(&mut self, id: u8, player_data: &[u8]) -> Option<Vec<Outgoing>> {
        let data = PlayerData::read(player_data).ok()?;
        self.players.get_mut(&id)?.joined.as_mut()?.data = data;
        Some(
            self.joined_ids()
                .into_iter()
                .map(|to| self.new_player(to, id))
                .collect(),
        )
    }

    pub fn team_ended(&mut self, by: u8, id: u8, player_data: &[u8]) -> Option<Vec<Outgoing>> {
        const NO_TEAM: i32 = -1;
        let sent = PlayerData::read(player_data).ok()?;
        self.player(by)?;
        let data = &mut self.players.get_mut(&id)?.joined.as_mut()?.data;
        let theirs = data.team == i32::from(by) || data.team == NO_TEAM;
        if id == HOST_NET_ID || id == by || sent.team != NO_TEAM || !theirs {
            return None;
        }
        data.team = NO_TEAM;
        data.team_locked = sent.team_locked;
        Some(
            self.joined_ids()
                .into_iter()
                .map(|to| self.new_player(to, id))
                .collect(),
        )
    }

    pub fn remove(&mut self, id: u8) -> (Option<Player>, Vec<Outgoing>) {
        let Some(p) = self.players.remove(&id) else {
            return (None, Vec::new());
        };
        let Some(player) = p.joined else {
            return (None, Vec::new());
        };
        let out = self
            .joined_ids()
            .into_iter()
            .map(|to| {
                (
                    to,
                    Message::new(
                        to,
                        NET_LAYER_SENDER,
                        Body::PlayerDeleted { id: u16::from(id) },
                    ),
                )
            })
            .collect();
        (Some(player), out)
    }
}

#[cfg(test)]
mod tests {
    use crate::endpoint::{Event, Joiner};
    use crate::game_kind::GameKind;
    use crate::player_data::{JoinRequest, PlayerData};
    use crate::topology::JoinState;

    #[test]
    fn a_tfl_joiner_waits_15_s_for_the_answer() {
        let request = || JoinRequest {
            name: b"Guest".to_vec(),
            password: Vec::new(),
            player: PlayerData::new(b"Guest", b"Team", [0; 8]),
        };
        let mut j = Joiner::new_as(GameKind::Tfl, request(), 1, 1000);
        assert!(j.poll(6000).is_empty());
        assert!(j.poll(15_999).is_empty());
        assert_eq!(j.poll(16_000), [Event::JoinTimedOut]);
        assert_eq!(j.join_state(), JoinState::Failed);
        let mut m = Joiner::new(request(), 1, 1000);
        assert!(m.poll(5999).is_empty());
        assert_eq!(m.poll(6000), [Event::JoinTimedOut]);
    }
}
