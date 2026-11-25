use std::collections::BTreeSet;

use mandrake_rec::writer::{NewGameData, Recorder, film_file, subgroup_for_name};

use crate::endpoint::Event;
use crate::game_kind::GameKind;
use crate::topology::Topology;
use crate::wire::{Body, Command, Message, TO_HOST};

pub const HOST_ID: u8 = crate::topology::HOST_NET_ID;
pub const COMMAND_NULL: u8 = 0;
pub const COMMAND_SYNC: u8 = 1;
pub const COMMAND_SYNC_FORMATION: u8 = 16;
pub const UNSTAMPED_PLAYER: u8 = 0xff;
pub const END_GAME_COUNTDOWN: u32 = 300;

pub const DEFAULT_FORMATION: [u8; 40] = [
    0x00, 0x04, 0x02, 0x80, 0x00, 0x08, 0x02, 0x80, 0x00, 0x04, 0x04, 0x00, 0x00, 0x04, 0x04, 0x00,
    0x02, 0x80, 0x03, 0x00, 0x01, 0x80, 0x06, 0x00, 0x03, 0x00, 0x40, 0x00, 0x55, 0x55, 0x01, 0x00,
    0x02, 0x00, 0x02, 0x00, 0x03, 0x00, 0x00, 0x00,
];

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum Barrier {
    #[default]
    Idle,
    Waiting {
        answered: bool,
        synced: BTreeSet<u16>,
    },
}

#[derive(Debug, Clone, Default)]
pub struct Client {
    horizon: i32,
    barrier: Barrier,
    barriers: u32,
    arrived: BTreeSet<u16>,
    in_game: bool,
    answered: Vec<u32>,
}

impl Client {
    pub fn horizon(&self) -> i32 {
        self.horizon
    }

    pub fn barrier(&self) -> &Barrier {
        &self.barrier
    }

    pub fn barriers_begun(&self) -> u32 {
        self.barriers
    }

    pub fn arrived(&self) -> &BTreeSet<u16> {
        &self.arrived
    }

    pub fn in_game(&self) -> bool {
        self.in_game
    }

    pub fn pongs_sent(&mut self, ticks: &[u32]) {
        self.answered.extend_from_slice(ticks);
    }

    pub fn reset_horizon(&mut self) {
        self.horizon = 0;
    }

    pub fn handle(
        &mut self,
        message: &Message,
        topology: &Topology,
        stamped: &mut Vec<Command>,
        out: &mut Vec<Message>,
        events: &mut Vec<Event>,
    ) -> bool {
        match &message.body {
            Body::Command(c) => {
                self.horizon = self.horizon.max(c.tick);
                if c.kind != COMMAND_NULL {
                    stamped.push(c.clone());
                }
                true
            }
            Body::Ping { sdl_ticks, .. } => {
                if let Some(at) = self.answered.iter().position(|t| t == sdl_ticks) {
                    self.answered.remove(at);
                } else if let Some(id) = topology.local_id() {
                    out.push(Message::new(
                        TO_HOST,
                        id,
                        Body::Pong {
                            sdl_ticks: *sdl_ticks,
                        },
                    ));
                }
                true
            }
            Body::BeginSyncData => {
                self.barrier = Barrier::Waiting {
                    answered: false,
                    synced: BTreeSet::new(),
                };
                self.barriers += 1;
                self.arrived.clear();
                if !self.barriers.is_multiple_of(2) {
                    self.in_game = true;
                }
                events.push(Event::BeginSync {
                    barrier: self.barriers,
                });
                true
            }
            Body::PlayerSyncData { id, .. } => {
                self.arrived.insert(*id);
                if let Barrier::Waiting { synced, .. } = &mut self.barrier {
                    synced.insert(*id);
                    if topology.players().all(|p| synced.contains(&p.id)) {
                        self.barrier = Barrier::Idle;
                        if self.barriers.is_multiple_of(2) {
                            self.in_game = false;
                        }
                        events.push(Event::AllSynced {
                            barrier: self.barriers,
                        });
                    }
                }
                true
            }
            Body::ServerCommand(_) | Body::Pong { .. } => true,
            _ => false,
        }
    }

    pub fn answer_barrier(&mut self, local_id: u8) -> Option<Message> {
        let Barrier::Waiting { answered, .. } = &mut self.barrier else {
            return None;
        };
        if std::mem::replace(answered, true) {
            return None;
        }
        Some(Message::new(
            HOST_ID,
            local_id,
            Body::PlayerSyncData {
                id: u16::from(local_id),
                value: 1,
            },
        ))
    }
}

pub fn drop_player_message(local_id: u8, dropped: u8, tick: i32) -> Message {
    drop_player_message_for(GameKind::Myth2, local_id, dropped, tick)
}

pub fn drop_player_message_for(kind: GameKind, local_id: u8, dropped: u8, tick: i32) -> Message {
    Message::new(
        TO_HOST,
        local_id,
        Body::Command(Command {
            kind: kind.commands().drop_player,
            player: dropped,
            tick,
            payload: Vec::new(),
        }),
    )
}

pub fn command_message(local_id: u8, kind: u8, payload: Vec<u8>) -> Message {
    Message::new(
        TO_HOST,
        local_id,
        Body::Command(Command {
            kind,
            player: UNSTAMPED_PLAYER,
            tick: 0,
            payload,
        }),
    )
}

pub fn command_bytes(c: &Command) -> Vec<u8> {
    let mut b = Vec::with_capacity(crate::wire::COMMAND_HEADER_SIZE + c.payload.len());
    crate::wire::write_command(c, &mut b);
    b
}

pub fn film(game: NewGameData, commands: &[Command], end_tick: u32, name: &[u8]) -> Vec<u8> {
    let mut r = Recorder::new(game, &[]);
    for c in commands {
        r.record(&command_bytes(c));
    }
    film_file(name, subgroup_for_name(name, &[]), &r.finish(end_tick))
}
