use mandrake_common::game_options;
use mandrake_rec::writer::NewGameData;

use crate::endpoint::Event;
use crate::game_description::{GameDescription, NewGameError, new_game_data};
use crate::game_kind::GameKind;
use crate::player_data::{NONE, PlayerData};
use crate::topology::{HOST_NET_ID, Player};
use crate::wire::{Body, Chat, Message, TO_ALL};

pub mod code {
    pub const CANCEL: u16 = 4;
    pub const SEAT: u16 = 0xb;
    pub const START: u16 = 100;
    pub const BEGIN_MIDGAME: u16 = 0x65;
}

pub const VERSION_RANGE: std::ops::RangeInclusive<u32> =
    game_options::VERSION_ID_1_5_1 as u32..=0x897;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InvalidGame {
    Version(u32),
    OtherGame(GameKind),
}

pub fn check(description: &GameDescription) -> Result<(), InvalidGame> {
    let v = description.parameters.version;
    if VERSION_RANGE.contains(&v) {
        Ok(())
    } else {
        Err(InvalidGame::Version(v))
    }
}

pub fn check_as(kind: GameKind, description: &GameDescription) -> Result<(), InvalidGame> {
    let theirs = GameKind::of(description);
    if theirs != kind {
        return Err(InvalidGame::OtherGame(theirs));
    }
    match kind {
        GameKind::Myth2 => check(description),
        GameKind::Tfl => Ok(()),
    }
}

pub fn configuration_request(local_id: u8, code: u16, value: u32) -> Message {
    Message::new(
        crate::topology::HOST_NET_ID,
        local_id,
        Body::Configuration {
            flags: 1,
            code,
            value,
        },
    )
}

pub mod chat {
    pub const PASS_ON: u16 = 1;
    pub const TEAM: u16 = 0;
    pub const PLAYER: u16 = 1;
    pub const ALL: u16 = 2;
    pub const NO_TARGET: u16 = 0xffff;
    pub const TEXT_MAX: usize = 0xff;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChatTo {
    All,
    Player(u16),
    Team(u16),
}

impl ChatTo {
    pub fn words(self) -> (u16, u16) {
        match self {
            ChatTo::All => (chat::ALL, chat::NO_TARGET),
            ChatTo::Player(id) => (chat::PLAYER, id),
            ChatTo::Team(leader) => (chat::TEAM, leader),
        }
    }
}

pub type ListColor = [u16; 4];

const SUBSTITUTE_COLORS: [ListColor; 20] = [
    [0xe146, 0, 0, 0],
    [0, 0xbd6f, 0, 0],
    [0x7fff, 0x2b84, 0, 0],
    [0xf0a2, 0xf850, 0, 0],
    [0xbae0, 0xbae0, 0xffff, 0],
    [0x547a, 0x51eb, 0x6666, 0],
    [0, 0xd47a, 0xd47a, 0],
    [0, 0, 0xee13, 0],
    [0xae13, 0xffff, 0xffff, 0],
    [0xd47a, 0x5709, 0xffff, 0],
    [0xdeb7, 0xb851, 0, 0],
    [0x6666, 0x6666, 0, 0],
    [0, 0xa665, 0xca3c, 0],
    [0xffff, 0xa3d6, 0xa3d6, 0],
    [0xca3c, 0x9709, 0xffff, 0],
    [0, 0x3851, 0x8a3c, 0],
    [0x6b84, 0, 0x6147, 0],
    [0xffff, 0xffff, 0xcf5b, 0],
    [0x35c2, 0x63d6, 0, 0],
    [0xbfff, 0xc28e, 0xa665, 0],
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ListedPlayer {
    pub id: u16,
    pub team: i32,
    pub colors: [ListColor; 2],
}

pub fn list_players(players: &[Player]) -> Vec<ListedPlayer> {
    let mut rows: Vec<(i16, ListedPlayer)> = players
        .iter()
        .map(|p| {
            let c = p.data.appearance.colors;
            (
                p.data.appearance.rank,
                ListedPlayer {
                    id: p.id,
                    team: p.data.team,
                    colors: [[c[0], c[1], c[2], c[3]], [c[4], c[5], c[6], c[7]]],
                },
            )
        })
        .collect();
    rows.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.id.cmp(&b.1.id)));
    let rows: Vec<ListedPlayer> = rows.into_iter().map(|(_, r)| r).collect();
    let mut taken = vec![false; rows.len()];
    let mut out = Vec::with_capacity(rows.len());
    for i in 0..rows.len() {
        let r = &rows[i];
        if taken[i] || !(i32::from(r.id) == r.team || r.team == -1) {
            continue;
        }
        taken[i] = true;
        out.push(r.clone());
        for j in 0..rows.len() {
            if !taken[j] && rows[j].team == i32::from(r.id) {
                taken[j] = true;
                out.push(rows[j].clone());
            }
        }
    }
    for (i, r) in rows.iter().enumerate() {
        if !taken[i] {
            out.push(r.clone());
        }
    }
    arbitrate(&mut out);
    out
}

fn arbitrate(rows: &mut [ListedPlayer]) {
    let id = |r: &ListedPlayer| i32::from(r.id);
    let leaders: Vec<usize> = (0..rows.len())
        .filter(|&i| id(&rows[i]) == rows[i].team || rows[i].team == -1)
        .collect();
    for &i in leaders.iter().rev() {
        arbitrate_one(rows, i, &leaders);
    }
    for i in 0..rows.len() {
        let me = id(&rows[i]);
        let same = (0..rows.len()).filter(|&j| id(&rows[j]) == me);
        let joined = (0..rows.len()).filter(|&j| rows[j].team == me && id(&rows[j]) != me);
        let list: Vec<usize> = same.chain(joined).collect();
        for &j in list.iter().rev() {
            arbitrate_one(rows, j, &list);
        }
    }
    for i in 0..rows.len() {
        if rows[i].team == -1 {
            continue;
        }
        if let Some(j) = (0..rows.len()).find(|&j| id(&rows[j]) == rows[i].team) {
            rows[i].colors[1] = rows[i].colors[0];
            rows[i].colors[0] = rows[j].colors[0];
        }
    }
}

fn arbitrate_one(rows: &mut [ListedPlayer], who: usize, among: &[usize]) {
    let mut at = i32::from(rows[who].id).wrapping_mul(0x2e);
    let end = at.wrapping_add(0x14);
    loop {
        let own = rows[who].colors[0];
        let clash = among.iter().rev().any(|&o| {
            let other = rows[o].colors[0];
            o != who && (0..3).all(|c| own[c].abs_diff(other[c]) < 0x3f00)
        });
        if !clash || at == end {
            return;
        }
        rows[who].colors[0] = SUBSTITUTE_COLORS[(i64::from(at) as u64 % 20) as usize];
        at = at.wrapping_add(1);
    }
}

pub fn chat_body(
    name: &[u8; 32],
    color: ListColor,
    flags: u16,
    mode: u16,
    target: u16,
    text: &[u8],
) -> Chat {
    let mut name = *name;
    name[31] = 0;
    let c = color;
    Chat {
        name,
        words: [c[0], c[1], c[2], 0, flags, mode, target, 0],
        text: chat_text(text),
    }
}

fn chat_text(text: &[u8]) -> Vec<u8> {
    let end = text.iter().position(|&b| b == 0).unwrap_or(text.len());
    text[..end.min(chat::TEXT_MAX)].to_vec()
}

pub fn chat_message(
    local_id: u8,
    name: &[u8; 32],
    color: ListColor,
    to: ChatTo,
    text: &[u8],
) -> Message {
    let (mode, target) = to.words();
    Message::new(
        crate::topology::HOST_NET_ID,
        local_id,
        Body::Chat(chat_body(name, color, chat::PASS_ON, mode, target, text)),
    )
}

pub fn pass_on_chat(local_id: u8, from: u8, chat: &Chat, players: &[Player]) -> Vec<Message> {
    let mode = chat.words[5];
    let target = chat.words[6];
    let copy = |to: u8, mode: u16, target: u16| {
        let mut c = chat.clone();
        c.words[4] = 0;
        c.words[5] = mode;
        c.words[6] = target;
        c.words[7] = 0;
        c.text = chat_text(&c.text);
        Message::new(to, local_id, Body::Chat(c))
    };
    let mut out = Vec::new();
    match mode {
        chat::ALL => out.push(copy(TO_ALL, chat::ALL, chat::NO_TARGET)),
        chat::PLAYER => {
            out.push(copy(target as u8, chat::PLAYER, target));
            if i32::from(from) != i32::from(target as i16) {
                out.push(copy(from, chat::PLAYER, target));
            }
        }
        chat::TEAM => {
            let mut sender_in = false;
            for p in players
                .iter()
                .filter(|p| p.data.team == i32::from(target as i16))
            {
                let id = p.id as u8;
                out.push(copy(id, chat::TEAM, target));
                if id == from {
                    sender_in = true;
                }
            }
            if !sender_in {
                out.push(copy(from, chat::TEAM, target));
            }
        }
        _ => out.push(copy(from, mode, target)),
    }
    out
}

#[derive(Debug, Clone, Default)]
pub struct Lobby {
    description: Option<GameDescription>,
    vets: Option<Vec<u8>>,
    started: bool,
}

impl Lobby {
    pub fn description(&self) -> Option<&GameDescription> {
        self.description.as_ref()
    }

    pub fn vets(&self) -> Option<&[u8]> {
        self.vets.as_deref()
    }

    pub fn started(&self) -> bool {
        self.started
    }

    pub fn handle(&mut self, message: &Message, events: &mut Vec<Event>) -> bool {
        match &message.body {
            Body::GameDescription(_) => {
                match GameDescription::from_body(&message.body) {
                    Ok(d) => {
                        events.push(Event::Description(d.clone()));
                        self.description = Some(d);
                    }
                    Err(_) => events.push(Event::Malformed(message.body.kind())),
                }
                true
            }
            Body::Configuration { flags, code, value } => {
                if *flags == 0 && *code == code::START {
                    self.started = true;
                }
                events.push(Event::Configuration {
                    flags: *flags,
                    code: *code,
                    value: *value,
                });
                true
            }
            Body::Chat(chat) => {
                events.push(Event::Chat {
                    from: message.from,
                    chat: chat.clone(),
                });
                true
            }
            Body::Vets(raw) => {
                self.vets = Some(raw.clone());
                events.push(Event::Vets(raw.clone()));
                true
            }
            _ => false,
        }
    }

    pub fn new_game_data(
        &self,
        players: &[Player],
        local_id: u8,
    ) -> Option<Result<NewGameData, NewGameError>> {
        let d = self.description.as_ref()?;
        let records: Vec<_> = players
            .iter()
            .map(|p| {
                let mut r = p.data.clone();
                r.ready = 1;
                r
            })
            .collect();
        Some(new_game_data(d, &records, i32::from(local_id)))
    }

    pub fn tfl_new_game(
        &self,
        players: &[Player],
        local_id: u8,
    ) -> Option<Result<mandrake_rec::tfl::TflNewGame, NewGameError>> {
        let d = self.description.as_ref()?;
        let records: Vec<_> = players
            .iter()
            .map(|p| {
                let mut r = record(p);
                r.ready = 1;
                r
            })
            .collect();
        Some(crate::tfl::new_game(d, &records, i32::from(local_id)))
    }
}

pub use mandrake_common::game_options::CARRY_VETERANS as OPTION_VETS;
pub use mandrake_common::game_options::VETERANS as OPTION_VETERANS;
pub const VETS_MIN_VERSION: u32 = game_options::VERSION_ID_1_7 as u32;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BarrierKind {
    Start,
    End,
}

#[derive(Debug, Clone)]
pub struct HostLobby {
    description: GameDescription,
    vets: Option<Vec<u8>>,
    kept_vets: Option<Vec<u8>>,
    started: bool,
    barrier: Option<BarrierKind>,
    arrived: std::collections::BTreeSet<u8>,
    barriers: u32,
    kind: GameKind,
    lobby_plugins: Option<crate::game_description::PluginList>,
}

impl HostLobby {
    pub fn new(description: GameDescription) -> HostLobby {
        HostLobby {
            kind: GameKind::of(&description),
            description,
            vets: None,
            kept_vets: None,
            started: false,
            barrier: None,
            arrived: Default::default(),
            barriers: 0,
            lobby_plugins: None,
        }
    }

    pub fn description(&self) -> &GameDescription {
        &self.description
    }

    pub fn kind(&self) -> GameKind {
        self.kind
    }

    pub fn started(&self) -> bool {
        self.started
    }

    pub fn arrived(&self) -> &std::collections::BTreeSet<u8> {
        &self.arrived
    }

    pub fn barrier(&self) -> Option<BarrierKind> {
        self.barrier
    }

    pub fn barriers_begun(&self) -> u32 {
        self.barriers
    }

    pub fn description_message(&self) -> Message {
        Message::new(TO_ALL, HOST_NET_ID, self.description.to_body())
    }

    pub fn set_description(&mut self, description: GameDescription) {
        self.description = description;
    }

    pub fn set_vets(&mut self, initializers: Option<Vec<u8>>) {
        self.vets = initializers.filter(|v| !v.is_empty());
    }

    pub fn kept_vets(&self) -> Option<&[u8]> {
        self.kept_vets.as_deref()
    }

    pub fn start_vets(&mut self) -> Option<Message> {
        if self.kind == GameKind::Tfl {
            self.kept_vets = None;
            return None;
        }
        let p = &mut self.description.parameters;
        let wanted = p.version >= VETS_MIN_VERSION
            && p.options & game_options::VETERANS != 0
            && p.options & game_options::CARRY_VETERANS != 0;
        self.kept_vets = None;
        let raw = match (&self.vets, wanted) {
            (Some(raw), true) => raw.clone(),
            _ => {
                p.options &= !game_options::CARRY_VETERANS;
                return None;
            }
        };
        self.kept_vets = Some(raw.clone());
        (p.options & game_options::COOPERATIVE != 0)
            .then(|| Message::new(TO_ALL, HOST_NET_ID, Body::Vets(raw)))
    }

    pub fn start(&mut self, machine_ticks: u32) -> [Message; 2] {
        let d = &mut self.description;
        match self.kind {
            GameKind::Myth2 => d.parameters.reset_seeds(machine_ticks),
            GameKind::Tfl => d.parameters.seed = machine_ticks,
        }
        self.lobby_plugins = Some(d.plugins.clone());
        d.plugins.remove_gameplay_versions();
        d.parameters.plugin_count = d.plugins.0.len() as u16;
        self.started = true;
        [
            self.description_message(),
            configuration_broadcast(code::START, 0),
        ]
    }

    pub fn reopen(&mut self) -> Message {
        let d = &mut self.description;
        d.parameters.options |= game_options::CARRY_VETERANS;
        if let Some(plugins) = self.lobby_plugins.take() {
            d.plugins = plugins;
            d.parameters.plugin_count = d.plugins.0.len() as u16;
        }
        self.description_message()
    }

    pub fn begin_barrier(&mut self, kind: BarrierKind) -> Message {
        self.barrier = Some(kind);
        self.barriers += 1;
        self.arrived.clear();
        Message::new(TO_ALL, HOST_NET_ID, Body::BeginSyncData)
    }

    pub fn sync_data(&mut self, from: u8, players: &[u8]) -> Option<(BarrierKind, Vec<Message>)> {
        self.arrived.insert(from);
        self.barrier_done(players)
    }

    pub fn barrier_done(&mut self, players: &[u8]) -> Option<(BarrierKind, Vec<Message>)> {
        let kind = self.barrier?;
        if !players.iter().all(|p| self.arrived.contains(p)) {
            return None;
        }
        self.barrier = None;
        let entries = players
            .iter()
            .map(|&id| {
                Message::new(
                    TO_ALL,
                    HOST_NET_ID,
                    Body::PlayerSyncData {
                        id: u16::from(id),
                        value: 0,
                    },
                )
            })
            .collect();
        Some((kind, entries))
    }

    pub fn mark_arrived(&mut self, id: u8) {
        self.arrived.insert(id);
    }

    pub fn remove(&mut self, id: u8) {
        self.arrived.remove(&id);
    }
}

pub fn configuration_broadcast(code: u16, value: u32) -> Message {
    Message::new(
        TO_ALL,
        HOST_NET_ID,
        Body::Configuration {
            flags: 0,
            code,
            value,
        },
    )
}

pub use mandrake_common::game_options::TEAMS as OPTION_TEAMS;
pub const FLAG_NO_TEAM_CHANGE: u16 = 0x40;
pub use mandrake_common::game_options::HOST_OBSERVER as OPTION_HOST_NO_TEAM;

pub(crate) fn record(p: &Player) -> PlayerData {
    let mut r = p.data.clone();
    r.net_id = i32::from(p.id);
    r
}

fn find(players: &[Player], id: i32) -> Option<&Player> {
    players.iter().find(|p| i32::from(p.id) == id)
}

pub fn disband_team(players: &[Player], team: i32) -> Vec<PlayerData> {
    players
        .iter()
        .filter(|p| p.data.team == team)
        .map(|p| {
            let mut r = record(p);
            r.team = NONE;
            r.team_locked = 0;
            r
        })
        .collect()
}

pub fn restart_records(players: &[Player]) -> Vec<PlayerData> {
    players
        .iter()
        .map(|p| {
            let mut r = record(p);
            if r.field_1d == 0 {
                r.ready = 0;
            }
            r
        })
        .collect()
}

fn members(players: &[Player], team: i32) -> usize {
    players.iter().filter(|p| p.data.team == team).count()
}

pub fn team_request(players: &[Player], from: u16, target: i32) -> Vec<PlayerData> {
    let Some(me) = find(players, i32::from(from)) else {
        return Vec::new();
    };
    let mut me = record(me);
    let mut out = Vec::new();
    if target == NONE {
        if me.team == NONE {
            return out;
        }
        if me.team == me.net_id || members(players, me.team) == 2 {
            return disband_team(players, me.team);
        }
        me.team = NONE;
        out.push(me);
        return out;
    }
    let mut found = false;
    for p in players {
        let r = record(p);
        if r.net_id != target || r.team_locked != 0 {
            continue;
        }
        if r.team == target {
            found = true;
        } else if r.team == NONE {
            let mut r = r;
            r.team = target;
            out.push(r);
            found = true;
        }
    }
    if found {
        me.team = target;
        out.push(me);
    }
    out
}

pub fn team_kick(players: &[Player], local: u16, target: u16) -> Vec<PlayerData> {
    let Some(t) = find(players, i32::from(target)) else {
        return Vec::new();
    };
    let mut t = record(t);
    if t.team != i32::from(local) {
        return Vec::new();
    }
    let mut out = Vec::new();
    if t.team == t.net_id || members(players, t.team) == 2 {
        out.extend(disband_team(players, t.team));
    }
    t.team = NONE;
    out.push(t);
    out
}

pub fn boot_unready(players: &[Player], local: u16, booted: i32) -> Vec<u16> {
    let Some(b) = find(players, booted) else {
        return Vec::new();
    };
    let team = i32::from(b.data.team as i16);
    if team == NONE {
        return Vec::new();
    }
    players
        .iter()
        .filter(|p| p.data.team == team && p.id != local && p.data.field_1d == 0)
        .map(|p| p.id)
        .collect()
}

pub fn host_options_records(
    players: &[Player],
    host: u16,
    old_options: u32,
    new_options: u32,
) -> Vec<PlayerData> {
    if (old_options ^ new_options) & game_options::HOST_OBSERVER == 0 {
        return Vec::new();
    }
    let Some(me) = find(players, i32::from(host)) else {
        return Vec::new();
    };
    let mut me = record(me);
    if new_options & game_options::HOST_OBSERVER != 0 {
        me.flags |= FLAG_NO_TEAM_CHANGE;
    } else {
        me.flags &= !FLAG_NO_TEAM_CHANGE;
    }
    let team = me.team;
    let mut out = vec![me.clone()];
    if team != NONE && (team == me.net_id || members(players, team) == 2) {
        let mut table = players.to_vec();
        apply(&mut table, &out);
        out.extend(disband_team(&table, i32::from(team as i16)));
    }
    out
}

pub fn leave_every_team(players: &[Player]) -> Vec<PlayerData> {
    let mut table = players.to_vec();
    let mut out = Vec::new();
    for id in players.iter().map(|p| p.id).collect::<Vec<_>>() {
        let records = team_request(&table, id, NONE);
        apply(&mut table, &records);
        out.extend(records);
    }
    out
}

fn apply(table: &mut [Player], records: &[PlayerData]) {
    for r in records {
        if let Some(p) = table.iter_mut().find(|p| i32::from(p.id) == r.net_id) {
            p.data = r.clone();
        }
    }
}

pub fn team_lock(players: &[Player], local: u16) -> Option<PlayerData> {
    let mut me = record(find(players, i32::from(local))?);
    me.team_locked = u32::from(me.team_locked == 0);
    Some(me)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TeamButton {
    pub enabled: bool,
    pub name: Option<usize>,
}

pub fn team_button(players: &[Player], local: u16, chosen: Option<i32>, teams: bool) -> TeamButton {
    let off = TeamButton {
        enabled: false,
        name: None,
    };
    if !teams {
        return off;
    }
    let Some(me) = find(players, i32::from(local)).map(record) else {
        return off;
    };
    let fixed = me.flags & FLAG_NO_TEAM_CHANGE != 0;
    if me.team == NONE {
        let open = chosen.and_then(|id| find(players, id)).is_some_and(|p| {
            let r = record(p);
            (r.team == NONE || r.team == r.net_id) && r.team_locked == 0
        });
        return TeamButton {
            enabled: open && !fixed,
            name: Some(0),
        };
    }
    TeamButton {
        enabled: !fixed,
        name: Some(1 + usize::from(me.net_id != me.team)),
    }
}

pub fn team_button_value(players: &[Player], local: u16, chosen: Option<i32>) -> Option<i32> {
    let me = find(players, i32::from(local))?;
    if me.data.team == NONE {
        chosen
    } else {
        Some(NONE)
    }
}

pub fn team_kick_enabled(players: &[Player], local: u16, chosen: Option<i32>) -> bool {
    chosen.is_some_and(|id| {
        id != i32::from(local) && find(players, id).is_some_and(|p| p.data.team == i32::from(local))
    })
}
