use std::collections::BTreeMap;
use std::path::Path;

use mandrake_common::game_options::HOST_OBSERVER;
use mandrake_common::{FourCC, NativeVfs};
use mandrake_net::endpoint::Host;
use mandrake_net::game_description::{FLAG_PASSWORD, GameDescription};
use mandrake_net::game_kind::GameKind;
use mandrake_net::lobby::ChatTo;
use mandrake_net::ours::Ours;
use mandrake_tag::{TagArchive, engine_tag_files};
use tracing::{info, warn};

pub const SERVER_NAME: &[u8] = b"Server";

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Policy {
    pub password: Vec<u8>,
    pub max_players: i16,
    pub tfl: Option<mandrake_net::tfl::DataChecksums>,
}

#[derive(Debug, Clone, Default)]
pub struct Owner {
    games: BTreeMap<u8, GameKind>,
    owner: Option<u8>,
    password: Option<Vec<u8>>,
}

impl Owner {
    pub fn id(&self) -> Option<u8> {
        self.owner
    }

    pub fn is(&self, id: u8) -> bool {
        self.owner == Some(id)
    }

    pub fn game(&mut self, from: u8, kind: GameKind) {
        info!("lobby: player {from} plays {}", kind.name());
        self.games.insert(from, kind);
    }

    pub fn hello(
        &mut self,
        host: &mut Host,
        from: u8,
        version: u16,
        in_lobby: bool,
        ours: GameKind,
    ) {
        info!("lobby: player {from} is our client (extension version {version})");
        let theirs = self.games.get(&from).copied();
        if let Some(theirs) = theirs.filter(|&k| k != ours) {
            info!(
                "lobby: player {from} plays {}, not this server's {}: it owns nothing",
                theirs.name(),
                ours.name()
            );
            return;
        }
        if self.owner.is_none() && in_lobby {
            self.owner = Some(from);
            host.send_ours(
                from,
                Ours::Owner {
                    id: u16::from(from),
                },
            );
            info!("lobby: player {from} owns the game");
        }
    }

    pub fn left(&mut self, id: u8) -> bool {
        self.games.remove(&id);
        self.owner == Some(id)
    }

    pub fn request(
        &mut self,
        host: &mut Host,
        game_dir: &Path,
        policy: &Policy,
        from: u8,
        ours: Ours,
    ) {
        if !self.is(from) {
            warn!("lobby: player {from} isn't the owner: {ours:?} ignored");
            return;
        }
        match ours {
            Ours::Description(raw) => match GameDescription::read(&raw)
                .map_err(|e| e.to_string())
                .and_then(|mut d| match policy.tfl {
                    Some(words) => {
                        let name =
                            crate::tfl_game::check_description_with_name(game_dir, words, &d)?;
                        d.map_name = name;
                        Ok(d)
                    }
                    None => {
                        check_description(game_dir, &d)?;
                        d.parameters.options |= HOST_OBSERVER;
                        Ok(d)
                    }
                }) {
                Ok(mut d) => {
                    if policy.max_players > 0 && d.parameters.max_players > policy.max_players {
                        warn!(
                            "lobby: the owner's {} players held to the server's {}",
                            d.parameters.max_players, policy.max_players
                        );
                        d.parameters.max_players = policy.max_players;
                        let line =
                            format!("This server takes at most {} players.", policy.max_players);
                        host.chat_as_to(
                            SERVER_NAME,
                            ChatTo::Player(u16::from(from)),
                            line.as_bytes(),
                        );
                    }
                    if self.password(policy).is_empty() {
                        d.flags &= !FLAG_PASSWORD;
                    } else {
                        d.flags |= FLAG_PASSWORD;
                    }
                    info!(
                        "lobby: the owner's description: \"{}\", map {:?}, type {}, time limit {}, \
                         at most {} players",
                        String::from_utf8_lossy(&d.game_name),
                        String::from_utf8_lossy(&d.parameters.mesh),
                        d.parameters.game_type,
                        d.parameters.time_limit,
                        d.parameters.max_players
                    );
                    host.set_options(d);
                }
                Err(e) => {
                    warn!("lobby: the owner's description refused: {e}");
                    host.send_description_to(from);
                }
            },
            Ours::Boot { id } => match u8::try_from(id) {
                Ok(id) if id != 0 && id != from && host.topology().player(id).is_some() => {
                    info!("lobby: the owner boots player {id}");
                    host.boot(id);
                }
                _ => warn!("lobby: the owner can't boot player {id}"),
            },
            Ours::Lock { locked } => {
                info!(
                    "lobby: the owner {} the game",
                    if locked { "closes" } else { "opens" }
                );
                host.set_accepting(!locked);
            }
            Ours::Password(p) => {
                let p = if p.is_empty() {
                    policy.password.clone()
                } else {
                    p
                };
                info!(
                    "lobby: the owner sets {}",
                    match (p.is_empty(), p == policy.password) {
                        (true, _) => "no password",
                        (false, true) => "the server's password",
                        (false, false) => "a password",
                    }
                );
                self.password = Some(p.clone());
                host.set_password(p);
            }
            other => warn!("lobby: the owner's {other:?} ignored"),
        }
    }

    fn password<'a>(&'a self, policy: &'a Policy) -> &'a [u8] {
        self.password.as_deref().unwrap_or(&policy.password)
    }
}

pub fn check_description(game_dir: &Path, d: &GameDescription) -> Result<(), String> {
    mandrake_net::lobby::check(d).map_err(|e| format!("{e:?}"))?;
    let mut plugins = d.plugins.clone();
    plugins.remove_gameplay_versions();
    let names: Vec<String> = plugins
        .0
        .iter()
        .map(|p| String::from_utf8_lossy(&p.name).into_owned())
        .collect();
    let refs: Vec<&str> = names.iter().map(String::as_str).collect();
    let files = engine_tag_files(game_dir, &refs);
    if !files.missing_plugins.is_empty() {
        return Err(format!(
            "the plugins {:?} aren't on the server",
            files.missing_plugins
        ));
    }
    let vfs = NativeVfs::new(".");
    let archives: Vec<Option<TagArchive>> = files
        .paths
        .iter()
        .map(|p| TagArchive::open(&vfs, &p.to_string_lossy()).ok())
        .collect();
    let plugin_archives = &archives[archives.len() - plugins.0.len()..];
    let other: Vec<String> = plugins
        .0
        .iter()
        .zip(&names)
        .zip(plugin_archives)
        .filter(|((p, _), arch)| {
            arch.as_ref()
                .map(|a| u32::from_be_bytes(a.header().checksum))
                != Some(p.checksum)
        })
        .map(|((p, name), _)| format!("{name} ({:#010x})", p.checksum))
        .collect();
    if !other.is_empty() {
        return Err(format!(
            "the server's plugins {other:?} have other checksums"
        ));
    }
    let mesh = FourCC::new(d.parameters.mesh);
    let found = archives
        .iter()
        .flatten()
        .any(|a| a.find_tag_by_id(FourCC::new(*b"mesh"), mesh).is_some());
    if found {
        Ok(())
    } else {
        Err(format!("the map {mesh} isn't on the server"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mandrake_net::endpoint::HostOptions;
    use mandrake_net::player_data::{JoinRequest, PlayerData};

    fn host() -> Host {
        let description = mandrake_session::net::host_description(
            &mandrake_session::Text(b"Dedicated".to_vec()),
            &mandrake_session::Text::default(),
            0,
        );
        Host::new(
            HostOptions {
                player: JoinRequest {
                    name: b"Dedicated".to_vec(),
                    password: Vec::new(),
                    player: PlayerData::new(b"Dedicated", b"Dedicated", [0; 8]),
                },
                password: Vec::new(),
                description,
            },
            1,
            0,
        )
    }

    #[test]
    fn the_first_of_ours_owns_the_game_and_its_leaving_ends_it() {
        let mut h = host();
        let mut o = Owner::default();
        o.hello(&mut h, 2, 1, true, GameKind::Myth2);
        o.hello(&mut h, 3, 1, true, GameKind::Myth2);
        assert_eq!(o.id(), Some(2));
        assert!(!o.left(3), "another of ours leaves: the game goes on");
        assert_eq!(o.id(), Some(2));
        assert!(o.left(2));
    }

    #[test]
    fn a_client_of_the_other_game_owns_no_lobby() {
        let mut h = host();
        let mut o = Owner::default();
        o.game(1, GameKind::Myth2);
        o.hello(&mut h, 1, 1, true, GameKind::Tfl);
        assert_eq!(o.id(), None, "a Myth II client in a TFL lobby");
        assert!(!o.left(1));
        o.game(2, GameKind::Tfl);
        o.hello(&mut h, 2, 1, true, GameKind::Tfl);
        assert_eq!(o.id(), Some(2));

        let mut o = Owner::default();
        o.game(1, GameKind::Tfl);
        o.hello(&mut h, 1, 1, true, GameKind::Myth2);
        assert_eq!(o.id(), None, "a TFL client in a Myth II lobby");
        o.hello(&mut h, 2, 1, true, GameKind::Myth2);
        assert_eq!(o.id(), Some(2), "no game named: believed");
    }

    #[test]
    fn no_one_takes_a_started_game() {
        let mut h = host();
        h.start(1);
        let mut o = Owner::default();
        o.hello(&mut h, 1, 1, false, GameKind::Myth2);
        assert_eq!(o.id(), None);
        o.hello(&mut h, 2, 1, true, GameKind::Myth2);
        assert_eq!(o.id(), Some(2));
    }

    fn policy() -> Policy {
        Policy {
            password: b"config".to_vec(),
            max_players: 4,
            tfl: None,
        }
    }

    #[test]
    fn an_owners_empty_password_keeps_the_configs() {
        let mut h = host();
        let mut o = Owner::default();
        o.hello(&mut h, 1, 1, true, GameKind::Myth2);
        let dir = Path::new("/nonexistent");
        assert_eq!(o.password(&policy()), b"config", "none set: the config's");
        o.request(&mut h, dir, &policy(), 1, Ours::Password(Vec::new()));
        assert_eq!(o.password(&policy()), b"config");
        o.request(&mut h, dir, &policy(), 1, Ours::Password(b"mine".to_vec()));
        assert_eq!(o.password(&policy()), b"mine");
        o.request(
            &mut h,
            dir,
            &policy(),
            2,
            Ours::Password(b"theirs".to_vec()),
        );
        assert_eq!(o.password(&policy()), b"mine");
        let open = Policy {
            password: Vec::new(),
            ..policy()
        };
        let mut o = Owner::default();
        o.hello(&mut h, 1, 1, true, GameKind::Myth2);
        o.request(&mut h, dir, &open, 1, Ours::Password(Vec::new()));
        assert_eq!(o.password(&open), b"");
    }

    #[test]
    fn the_owners_boot_lowers_the_teammates_ready() {
        use mandrake_common::game_options::TEAMS;
        use mandrake_net::endpoint::Joiner;
        let mut h = host();
        let mut d = h.lobby().description().clone();
        d.parameters.options |= TEAMS;
        d.parameters.max_players = 8;
        h.set_description(d);
        let mut joiners: Vec<(u8, Joiner)> = Vec::new();
        let pump = |h: &mut Host, joiners: &mut Vec<(u8, Joiner)>, now: u64| {
            for (id, j) in joiners.iter_mut() {
                h.receive(*id, &j.take_outgoing());
            }
            h.poll(now);
            for (id, j) in joiners.iter_mut() {
                j.receive(&h.take_outgoing(*id));
                j.poll(now);
            }
        };
        pump(&mut h, &mut joiners, 0);
        for (n, name) in [b"Owner", b"Alpha", b"Gamma"].iter().enumerate() {
            let id = h.accept().unwrap();
            let request = JoinRequest {
                name: name.to_vec(),
                password: Vec::new(),
                player: PlayerData::new(*name, b"Team", [0; 8]),
            };
            joiners.push((id, Joiner::new(request, 0x1000 + u16::from(id), n as u64)));
            pump(&mut h, &mut joiners, 10 * n as u64 + 5);
        }
        let ids: Vec<u8> = joiners.iter().map(|(id, _)| *id).collect();
        let (owner, alpha, gamma) = (ids[0], ids[1], ids[2]);
        let mut o = Owner::default();
        o.hello(&mut h, owner, 1, true, GameKind::Myth2);
        assert!(joiners[2].1.team_button(Some(i32::from(alpha))));
        pump(&mut h, &mut joiners, 40);
        pump(&mut h, &mut joiners, 50);
        for j in joiners.iter_mut() {
            assert!(j.1.set_ready(true));
        }
        pump(&mut h, &mut joiners, 60);
        pump(&mut h, &mut joiners, 70);
        let ready = |h: &Host, id: u8| {
            h.local()
                .players()
                .iter()
                .find(|p| p.id == u16::from(id))
                .map(|p| p.data.ready)
        };
        assert_eq!(
            h.topology().player(gamma).unwrap().data.team,
            i32::from(alpha)
        );
        assert_eq!((ready(&h, owner), ready(&h, alpha)), (Some(1), Some(1)));
        o.request(
            &mut h,
            Path::new("/nonexistent"),
            &policy(),
            owner,
            Ours::Boot {
                id: u16::from(gamma),
            },
        );
        pump(&mut h, &mut joiners, 80);
        assert_eq!(ready(&h, gamma), None, "booted");
        assert_eq!(ready(&h, alpha), Some(0), "the booted player's teammate");
        assert_eq!(ready(&h, owner), Some(1), "in no team");
        assert_eq!(
            joiners[1]
                .1
                .players()
                .iter()
                .find(|p| p.id == u16::from(alpha))
                .unwrap()
                .data
                .ready,
            1,
            "nothing sent: Alpha keeps its own"
        );
    }
}
