use mandrake_common::FourCC;

use crate::catalog::{Level, ListEntry};
use crate::progress::VisitedLevel;
use crate::screen::{LevelEntry, PostgameExits};

pub const TUTORIAL: FourCC = FourCC(*b"00tm");
pub const FALLBACK_POST_ART: FourCC = FourCC(*b"inge");
const AMBIENT_WIN: FourCC = FourCC(*b"wisc");
const AMBIENT_LOSS: FourCC = FourCC(*b"losc");

pub const ENTRY_SOLO: u32 = 0x10000;
pub const ENTRY_LISTED: u32 = 0x100000;
pub const ENTRY_OPEN_HIDDEN: u32 = 0x800000;
pub const ENTRY_FORCES_ANTI_CLUMP: u32 = 0x2000_0000;
pub const ENTRY_FORCES_TFL: u32 = 0x4000_0000;
pub const ENTRY_EXTRA_ALTERNATE: u32 = 8;
const ALTERNATE_MESHES: [FourCC; 2] = [FourCC(*b"21md"), FourCC(*b"03hp")];

pub const MESH_CAMPAIGN_END: u32 = 1;
pub const MESH_FILM_BEFORE_BRIEFING: u32 = 2;
pub const MESH_NARRATION_BEFORE_FILM: u32 = 4;
pub const PREGAME_FILM: usize = 0;
pub const WIN_FILM: usize = 1;
pub const LOSS_FILM: usize = 2;
const CAMPAIGN_END_FILE_TYPE: i16 = 3;

#[derive(Debug, Clone, Default, PartialEq)]
pub struct LevelList {
    pub levels: Vec<LevelEntry>,
    pub selected: Option<FourCC>,
}

pub fn level_list(
    entries: &[ListEntry],
    visited: &[VisitedLevel],
    current: Option<FourCC>,
    all_levels: bool,
) -> LevelList {
    let is_open = |e: &ListEntry| {
        Some(e.mesh) == current
            || visited.contains(&VisitedLevel {
                mesh: e.mesh,
                plugin_checksum: e.plugin_checksum,
            })
    };
    let mut listed: Vec<&ListEntry> = entries
        .iter()
        .rev()
        .filter(|e| is_listed(e, is_open(e), all_levels))
        .collect();
    listed.sort_by_key(|e| e.order);

    let lowest = listed
        .iter()
        .filter(|e| e.flags & ENTRY_OPEN_HIDDEN == 0)
        .map(|e| e.order)
        .min();
    let levels = listed
        .iter()
        .map(|e| LevelEntry {
            mesh: e.mesh,
            name: e.name.clone(),
            selectable: all_levels
                || is_open(e)
                || e.flags & ENTRY_OPEN_HIDDEN != 0
                || lowest.is_some_and(|lowest| e.order <= lowest)
                || e.order % 100 == 1,
            listed: e.flags & ENTRY_LISTED != 0,
            order: e.order,
        })
        .collect::<Vec<_>>();

    let selected = current
        .filter(|&mesh| listed.iter().any(|e| e.mesh == mesh))
        .or_else(|| {
            let lowest = lowest?;
            let first = listed
                .iter()
                .find(|e| e.flags & ENTRY_OPEN_HIDDEN == 0 && e.order == lowest)?;
            Some(first.mesh)
        })
        .or_else(|| listed.first().map(|e| e.mesh));
    LevelList { levels, selected }
}

fn is_listed(e: &ListEntry, is_open: bool, all_levels: bool) -> bool {
    if e.flags & ENTRY_SOLO == 0 {
        return false;
    }
    if !all_levels && (e.flags & ENTRY_LISTED == 0 || e.flags & ENTRY_OPEN_HIDDEN != 0) {
        return false;
    }
    let alternate =
        e.extra_flags & ENTRY_EXTRA_ALTERNATE != 0 || ALTERNATE_MESHES.contains(&e.mesh);
    !alternate || is_open
}

pub fn has_briefing(level: &Level) -> bool {
    level.pre_art.is_some() && level.briefing_text.is_some()
}

#[derive(Debug, Clone, PartialEq)]
pub enum PostgameRule {
    ToMenu,
    RestartFresh,
    Dialog(PostgameChoice),
    Undrawn(PostgameChoice),
}

#[derive(Debug, Clone, PartialEq)]
pub struct PostgameChoice {
    pub exits: PostgameExits,
    pub art: FourCC,
    pub art_sequence: i16,
    pub ambient: FourCC,
    pub next_mesh: Option<FourCC>,
    pub skip_mesh: Option<FourCC>,
    pub campaign_end: bool,
    pub mark_visited: Option<FourCC>,
    pub won: bool,
}

pub fn level_film(level: &Level, index: usize) -> Option<&str> {
    level.cutscenes.get(index)?.as_deref()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EpilogueRules {
    pub tutorial: Option<FourCC>,
    pub fallback_art: Option<FourCC>,
    pub ambient_win: FourCC,
    pub ambient_loss: FourCC,
    pub quit_on_win: bool,
}

pub const MYTH2_EPILOGUE: EpilogueRules = EpilogueRules {
    tutorial: Some(TUTORIAL),
    fallback_art: Some(FALLBACK_POST_ART),
    ambient_win: AMBIENT_WIN,
    ambient_loss: AMBIENT_LOSS,
    quit_on_win: true,
};

pub const TFL_EPILOGUE: EpilogueRules = EpilogueRules {
    tutorial: None,
    fallback_art: None,
    ambient_win: FourCC(*b"scwi"),
    ambient_loss: FourCC(*b"sclo"),
    quit_on_win: true,
};

pub fn postgame(level: Option<&Level>, end_code: i16, can_revert: bool) -> PostgameRule {
    postgame_with(&MYTH2_EPILOGUE, level, end_code, can_revert)
}

pub fn postgame_with(
    rules: &EpilogueRules,
    level: Option<&Level>,
    end_code: i16,
    can_revert: bool,
) -> PostgameRule {
    let Some(level) = level.filter(|l| Some(l.mesh) != rules.tutorial) else {
        return PostgameRule::ToMenu;
    };
    if !(1..=3).contains(&end_code) {
        return PostgameRule::RestartFresh;
    }
    let won = end_code == 1 || (end_code == 2 && level.alternate.is_some());

    let last_map_win =
        level.mesh == FourCC(*b"25so") && level.post_art == Some(FourCC(*b"25po")) && end_code == 1;
    let own_art = level.post_art.filter(|_| !last_map_win);
    let (art, art_sequence) = match (own_art, rules.fallback_art) {
        (Some(art), _) => (art, i16::from(!won)),
        (None, Some(fallback)) => (fallback, 0),
        (None, None) => (FourCC([0xff; 4]), i16::from(!won)),
    };

    let choice = PostgameChoice {
        exits: PostgameExits {
            next: won,
            restart: !won,
            last_saved: !won && can_revert,
            skip: end_code == 2 && !won,
            save_film: true,
            quit: !won || rules.quit_on_win,
        },
        art,
        art_sequence,
        ambient: if won {
            level.ambient_win.unwrap_or(rules.ambient_win)
        } else {
            level.ambient_loss.unwrap_or(rules.ambient_loss)
        },
        next_mesh: if end_code == 2 {
            level.alternate
        } else {
            level.next
        },
        skip_mesh: level.next,
        campaign_end: matches!(end_code, 1 | 2)
            && (level.flags & MESH_CAMPAIGN_END != 0
                || (level.next.is_none() && level.file_type == CAMPAIGN_END_FILE_TYPE)),
        mark_visited: level.next.filter(|_| end_code == 1),
        won,
    };
    if own_art.is_none() && rules.fallback_art.is_none() {
        PostgameRule::Undrawn(choice)
    } else {
        PostgameRule::Dialog(choice)
    }
}
