//! A campaign built by hand, for the tests that walk the session. No tags are needed.
#![allow(dead_code, reason = "each test binary uses its own part of this")]

pub mod net_pipe;

use mandrake_common::FourCC;
use mandrake_rec::header::MonsterInitializer;
use mandrake_session::catalog::{Campaign, Catalog, Level, ListEntry};
use mandrake_session::rules::{ENTRY_LISTED, ENTRY_SOLO, MESH_CAMPAIGN_END};
use mandrake_session::{Difficulty, Narration, PostgameReport, Text};

pub const WILLOW: FourCC = FourCC(*b"l1  ");
pub const SALVATION: FourCC = FourCC(*b"cr01");
pub const BROKEN_PATH: FourCC = FourCC(*b"le3e");
pub const BREACH: FourCC = FourCC(*b"thkm");
/// The alternate after Broken Path, with no briefing of its own.
pub const DRINKING: FourCC = FourCC(*b"03hp");
/// A level whose flag ends the campaign, with no next link.
pub const FINALE: FourCC = FourCC(*b"finl");
pub const BONUS: FourCC = FourCC(*b"bons");
pub const TUTORIAL: FourCC = FourCC(*b"00tm");

pub const WIN: i16 = 1;
pub const ALTERNATE: i16 = 2;
pub const LOSS: i16 = 3;

pub fn text(s: &str) -> Text {
    Text(s.as_bytes().to_vec())
}

pub fn entry(mesh: FourCC, order: i16, flags: u32) -> ListEntry {
    ListEntry {
        mesh,
        name: Text(mesh.0.to_vec()),
        flags,
        order,
        extra_flags: 0,
        plugin_checksum: -1,
    }
}

/// A level with a briefing, its art named after it.
pub fn level(mesh: FourCC, next: Option<FourCC>) -> Level {
    Level {
        title: Text(mesh.0.to_vec()),
        pre_art: Some(mesh),
        briefing_text: Some(text("A briefing.")),
        narration: Some(FourCC(*b"na01")),
        post_art: Some(FourCC(*b"post")),
        next,
        objectives: text("Objectives."),
        hints: vec![text("A level hint.")],
        description: text("A description."),
        ..Level::new(mesh)
    }
}

/// Two campaigns: the game's, and a plugin's.
pub fn catalog() -> Catalog {
    let open = ENTRY_SOLO | ENTRY_LISTED;
    Catalog {
        campaigns: vec![
            Campaign {
                name: text("The Fallen Lords' sequel"),
                checksum: -1,
                plugins: Vec::new(),
                entries: vec![
                    entry(TUTORIAL, 100, ENTRY_SOLO),
                    entry(WILLOW, 201, open),
                    entry(SALVATION, 202, open),
                    entry(BROKEN_PATH, 203, open),
                    entry(BREACH, 204, open),
                    entry(DRINKING, 205, open),
                ],
            },
            Campaign {
                name: text("A plugin"),
                checksum: 77,
                plugins: vec!["A plugin".to_owned()],
                entries: vec![
                    ListEntry {
                        plugin_checksum: 77,
                        ..entry(BONUS, 1, open)
                    },
                    ListEntry {
                        plugin_checksum: 77,
                        ..entry(FINALE, 2, ENTRY_SOLO)
                    },
                ],
            },
        ],
        levels: vec![
            Level {
                pre_art: None,
                briefing_text: None,
                post_art: None,
                ..level(TUTORIAL, None)
            },
            level(WILLOW, Some(SALVATION)),
            level(SALVATION, Some(BROKEN_PATH)),
            Level {
                alternate: Some(DRINKING),
                ..level(BROKEN_PATH, Some(BREACH))
            },
            level(BREACH, None),
            Level {
                pre_art: None,
                briefing_text: None,
                ..level(DRINKING, Some(BREACH))
            },
            Level {
                plugin_checksum: 77,
                ..level(BONUS, Some(FINALE))
            },
            Level {
                plugin_checksum: 77,
                flags: MESH_CAMPAIGN_END,
                ..level(FINALE, None)
            },
        ],
        difficulties: ["Timid", "Simple", "Normal", "Heroic", "Legendary"]
            .iter()
            .map(|name| Difficulty {
                name: text(name),
                description: text("How hard it is."),
            })
            .collect(),
        hint_tabs: vec![text("General"), text("Level")],
        general_hints: vec![text("A general hint.")],
        menu_music: vec![mandrake_session::MenuTrack {
            tag: FourCC(*b"mamm"),
            weight: 20,
        }],
        campaign_end: Narration {
            art: Some(FourCC(*b"inge")),
            text: text("It is over."),
            narration: Some(FourCC(*b"naep")),
        },
        ..Catalog::default()
    }
}

pub fn report(end_code: i16) -> PostgameReport {
    PostgameReport {
        end_code,
        ..PostgameReport::default()
    }
}

/// A veteran the tests can tell apart.
pub fn veteran(experience: u8) -> MonsterInitializer {
    MonsterInitializer {
        player_id: 1,
        unit_tag: FourCC(*b"mysm"),
        monster_tag: FourCC(*b"mysn"),
        experience,
        field_8f_plus_one: 0,
        permutation: 0,
        name_index: 0,
        name: String::new(),
    }
}
