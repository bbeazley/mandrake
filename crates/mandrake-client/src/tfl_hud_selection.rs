use std::collections::HashMap;
use std::rc::Rc;

use mandrake_common::FourCC;
use mandrake_tag::mesh::MarkerPaletteEntry;
use mandrake_tfl_sim::TflSim;

use crate::tfl_hud::{TflPortrait, TflSelection, TflStatusIcon, TflStatusRecord};
use crate::unit_info::{MonsterTexts, string_from_list, string_list};

const MONS_PORTRAIT_SEQUENCE: usize = 0x18;
const MONS_AMMUNITION_SEQUENCE: usize = 0x24;
const MONS_EXPERIENCE_VIEW: usize = 0x148;
const MONS_SPELLING: usize = 0x138;
const MONS_NAMES: usize = 0x13c;
const MONS_QUOTES: usize = 0x140;
const ARTI_COLLECTION: usize = 0x10;
const ARTI_SEQUENCE: usize = 0x14;
const OPTION_80: u32 = mandrake_common::game_options::COOPERATIVE;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Viewer {
    pub(crate) player: i8,
    pub(crate) team: i16,
}

const FILM_TEAM: i16 = i16::MIN;

impl Viewer {
    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) const FILM: Viewer = Viewer {
        player: -1,
        team: FILM_TEAM,
    };

    fn sees_items_of(self, team: i16) -> bool {
        team == self.team || self.team == FILM_TEAM
    }
}

#[derive(Default)]
pub(crate) struct SelectionTexts {
    texts: HashMap<FourCC, Option<Rc<MonsterTexts>>>,
}

impl SelectionTexts {
    fn get(&mut self, sim: &TflSim, mons: FourCC) -> Option<Rc<MonsterTexts>> {
        self.texts
            .entry(mons)
            .or_insert_with(|| {
                let d = sim.data.peek_def(b"mons", mons)?;
                let list = |o| {
                    let id = d.tag(o)?;
                    let raw = match sim.data.peek_def(b"stli", id) {
                        Some(s) => s.0.clone(),
                        None => crate::tfl_map::tag_data(&sim.data.archive, b"stli", id)?,
                    };
                    Some(string_list(&raw))
                };
                Some(Rc::new(MonsterTexts {
                    portrait_sequence: d.i16(MONS_PORTRAIT_SEQUENCE),
                    ammunition_sequence: d.i16(MONS_AMMUNITION_SEQUENCE),
                    experience_view: d.i16(MONS_EXPERIENCE_VIEW),
                    flags_224: 0,
                    spelling: list(MONS_SPELLING),
                    names: list(MONS_NAMES),
                    quotes: list(MONS_QUOTES),
                }))
            })
            .clone()
    }
}

pub(crate) fn selection_of(
    sim: &TflSim,
    selected: &[usize],
    viewer: Viewer,
    cache: &mut SelectionTexts,
) -> Option<TflSelection> {
    let selected: Vec<usize> = selected
        .iter()
        .copied()
        .filter(|&s| s < sim.monsters.len())
        .collect();
    let portrait = portrait(sim, &selected, cache)?;
    let texts: Vec<Option<Rc<MonsterTexts>>> = selected
        .iter()
        .map(|&s| cache.get(sim, sim.monsters[s].definition))
        .collect();
    let one = (selected.len() == 1).then(|| selected[0]);
    let status = TflStatusRecord {
        name: one.and_then(|s| sim.monster_name(s)).unwrap_or_default(),
        kind: type_string(sim, &selected, &texts, viewer, cache),
        experience: one.map_or([0, 0, -1], |s| {
            let m = &sim.monsters[s];
            [
                i16::from(m.experience),
                i16::from(m.field_02 as i8),
                texts[0].as_ref().map_or(-1, |t| t.experience_view),
            ]
        }),
        ammunition: one.and_then(|s| ammunition(sim, s, texts[0].as_deref(), viewer)),
        item: one.and_then(|s| item(sim, s, texts[0].as_deref(), viewer)),
        seed: selected
            .first()
            .map_or(-1, |&s| i16::from(sim.monsters[s].name_index as i8)),
    };
    Some(TflSelection {
        status,
        text: description(sim, &selected, &texts),
        portrait,
    })
}

fn portrait(sim: &TflSim, selected: &[usize], cache: &mut SelectionTexts) -> Option<TflPortrait> {
    let s = most_shared(sim, selected)?;
    let m = &sim.monsters[s];
    let core_id = sim.data.peek_def(b"unit", m.unit)?.tag(4)?;
    let core = sim.data.peek_def(b"core", core_id)?;
    let collection = core.tag(0)?;
    let sequence = cache
        .get(sim, m.definition)
        .map_or(-1, |t| i32::from(t.portrait_sequence));
    let p = if selected.len() == 1 {
        portrait_permutation(sim, s)
    } else {
        0
    };
    let colors = mandrake_tag::CorePermutations::from_bytes(&core.0)
        .ok()
        .map(|c| c.colors(p, PERMUTATION_COLORS));
    Some(TflPortrait {
        collection,
        sequence,
        colors,
    })
}

const PERMUTATION_COLORS: usize = 8;

const MONSTER_SHARED_DEFINITION: u32 = 0x1000_0000;

fn most_shared(sim: &TflSim, selected: &[usize]) -> Option<usize> {
    let mut counts: Vec<((u8, usize), u32, usize)> = Vec::new();
    for &s in selected {
        let p = sim.monsters[s].palette;
        let key = engine_order(sim, p);
        match counts.iter_mut().find(|c| sim.monsters[c.2].palette == p) {
            Some(c) => c.1 += 1,
            None => counts.push((key, 1, s)),
        }
    }
    counts.sort_by_key(|c| c.0);
    let mut best: Option<(u32, usize)> = None;
    for &(_, n, s) in &counts {
        if best.is_none_or(|(b, _)| n > b) {
            best = Some((n, s));
        }
    }
    best.map(|(_, s)| s)
}

fn engine_order(sim: &TflSim, palette: i16) -> (u8, usize) {
    let index = usize::try_from(palette).unwrap_or(usize::MAX);
    let Some(e) = sim.statistics.palette.get(index) else {
        return (3, index);
    };
    if e.player == -1 {
        let at = sim
            .data
            .mesh
            .palettes
            .iter()
            .filter(|p| p.marker_type == MarkerPaletteEntry::TYPE_UNIT)
            .position(|p| p.tag_id == e.unit);
        return at.map_or((3, index), |k| (0, k));
    }
    let made = sim
        .unit_copies()
        .iter()
        .position(|c| c.unit == e.unit && i16::try_from(c.player) == Ok(e.player));
    made.map_or((2, index), |k| (1, k))
}

fn portrait_permutation(sim: &TflSim, s: usize) -> usize {
    let m = &sim.monsters[s];
    let coloured = m.flags & MONSTER_SHARED_DEFINITION == 0
        && sim.unit_copies().iter().any(|c| {
            c.unit == m.unit && i8::try_from(c.player) == Ok(m.player) && c.permutation.is_some()
        });
    if coloured {
        return 0;
    }
    let count = sim
        .data
        .peek_def(b"unit", m.unit)
        .and_then(|u| u.tag(4))
        .and_then(|c| sim.data.peek_def(b"core", c))
        .map_or(0, |c| c.i16(4));
    let p = sim.objects.get(m.object).map_or(0, |o| o.permutation_index);
    if (0..count).contains(&p) {
        p as usize
    } else {
        0
    }
}

fn type_string(
    sim: &TflSim,
    selected: &[usize],
    texts: &[Option<Rc<MonsterTexts>>],
    viewer: Viewer,
    cache: &mut SelectionTexts,
) -> Vec<u8> {
    let spelling =
        |t: Option<&Rc<MonsterTexts>>, n: i32| t.map_or(Vec::new(), |t| t.spelling(n).to_vec());
    if let [s] = selected[..] {
        let m = &sim.monsters[s];
        let mut out = b"(".to_vec();
        out.extend(spelling(texts[0].as_ref(), 1));
        out.push(b')');
        let other = m.player != viewer.player;
        let shared = sim.game_options & OPTION_80 != 0 && m.team == 1;
        if other && !shared {
            out.push(b' ');
            if let Some(p) = usize::try_from(m.player)
                .ok()
                .and_then(|p| sim.data.film.header.players.get(p))
            {
                out.extend(mandrake_common::mac_roman::encode(&p.name));
            }
        }
        return out;
    }
    let list = crate::tfl_sound::mons_list(sim);
    let rank = |d: FourCC| list.iter().position(|&e| e == d).unwrap_or(usize::MAX);
    let mut counts: Vec<(usize, FourCC, i32)> = Vec::new();
    for &s in selected {
        let d = sim.monsters[s].definition;
        match counts.iter_mut().find(|c| c.1 == d) {
            Some(c) => c.2 += 1,
            None => counts.push((rank(d), d, 1)),
        }
    }
    counts.sort_by_key(|c| c.0);
    let mut out = Vec::new();
    for (j, &(_, d, n)) in counts.iter().enumerate() {
        if j > 0 {
            out.extend_from_slice(b", ");
        }
        out.extend(format!("{n} ").bytes());
        out.extend(spelling(cache.get(sim, d).as_ref(), n));
    }
    out
}

fn description(sim: &TflSim, selected: &[usize], texts: &[Option<Rc<MonsterTexts>>]) -> Vec<u8> {
    if let [s] = selected[..] {
        let Some(quotes) = texts[0].as_ref().and_then(|t| t.quotes.as_deref()) else {
            return Vec::new();
        };
        let count = (quotes.len() as i32).max(1);
        let i = i32::from(sim.monsters[s].name_index as i8) % count;
        return string_from_list(Some(quotes), i).to_vec();
    }
    let mut out = Vec::new();
    for &s in selected {
        let name = sim.monster_name(s).unwrap_or_default();
        if name.is_empty() {
            continue;
        }
        if !out.is_empty() {
            out.extend_from_slice(b", ");
        }
        out.extend(name);
    }
    if !out.is_empty() {
        out.push(b'.');
    }
    out
}

fn ammunition(
    sim: &TflSim,
    s: usize,
    texts: Option<&MonsterTexts>,
    viewer: Viewer,
) -> Option<TflStatusIcon> {
    let m = &sim.monsters[s];
    if m.ammunition <= 0 || !viewer.sees_items_of(m.team) {
        return None;
    }
    Some(TflStatusIcon {
        collection: sim.objects.get(m.object)?.collection,
        sequence: texts.map_or(-1, |t| t.ammunition_sequence),
        count: i16::from(m.ammunition),
    })
}

fn item(
    sim: &TflSim,
    s: usize,
    texts: Option<&MonsterTexts>,
    viewer: Viewer,
) -> Option<TflStatusIcon> {
    let m = &sim.monsters[s];
    if m.carried == -1 || !viewer.sees_items_of(m.team) {
        return None;
    }
    let Some(arti) = m.artifact else {
        return Some(TflStatusIcon {
            collection: sim.objects.get(m.object)?.collection,
            sequence: texts.map_or(-1, |t| t.ammunition_sequence),
            count: 0,
        });
    };
    let a = sim.data.peek_def(b"arti", arti)?;
    let (collection, sequence) = (a.tag(ARTI_COLLECTION)?, a.i16(ARTI_SEQUENCE));
    if sequence == -1 {
        return None;
    }
    let carried = usize::try_from(m.carried).ok()?;
    Some(TflStatusIcon {
        collection,
        sequence,
        count: i16::from(sim.objects.get(carried)?.charges),
    })
}
