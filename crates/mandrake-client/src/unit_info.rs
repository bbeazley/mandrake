use std::collections::BTreeMap;
use std::rc::Rc;

use mandrake_common::FourCC;
use mandrake_math::BungieRng;
use mandrake_sim::engine_rng::EngineWorld;
use mandrake_tag::font::{self, Bitmap32, FontFamily, RgbColor, TextClip, TextLocale, text_flags};
use mandrake_tag::{CorePermutations, Rect, TagLookup};
use mandrake_view::interface_bitmap::{self, InterfaceArt, placement};

fn be16(d: &[u8], o: usize) -> i16 {
    d.get(o..o + 2)
        .map_or(-1, |b| i16::from_be_bytes([b[0], b[1]]))
}

fn tag_ref(d: &[u8], o: usize) -> Option<FourCC> {
    let b = d.get(o..o + 4)?;
    (b != [0xff; 4] && b != [0; 4]).then(|| FourCC([b[0], b[1], b[2], b[3]]))
}

pub fn string_list(raw: &[u8]) -> Vec<Vec<u8>> {
    raw.split(|&b| b == 0 || b == b'\n' || b == b'\r')
        .filter(|s| !s.is_empty())
        .map(<[u8]>::to_vec)
        .collect()
}

pub fn string_from_list(list: Option<&[Vec<u8>]>, i: i32) -> &[u8] {
    match list {
        Some(l) if i >= 0 => l.get(i as usize).map_or(&[][..], Vec::as_slice),
        _ => &[],
    }
}

fn count_strings(list: Option<&[Vec<u8>]>) -> i32 {
    list.map_or(0, |l| l.len() as i32)
}

#[derive(Debug, Clone, Default)]
pub struct MonsterTexts {
    pub portrait_sequence: i16,
    pub ammunition_sequence: i16,
    pub experience_view: i16,
    pub flags_224: u8,
    pub spelling: Option<Vec<Vec<u8>>>,
    pub names: Option<Vec<Vec<u8>>>,
    pub quotes: Option<Vec<Vec<u8>>>,
}

impl MonsterTexts {
    pub fn from_tags(mons: &[u8], tags: &TagLookup) -> Self {
        let list = |o| {
            tag_ref(mons, o)
                .and_then(|id| tags.get(FourCC(*b"stli"), id))
                .map(|d| string_list(&d))
        };
        Self {
            portrait_sequence: be16(mons, 0x18),
            ammunition_sequence: be16(mons, 0x24),
            experience_view: be16(mons, 0x1b8),
            flags_224: mons.get(0x224).copied().unwrap_or(0),
            spelling: list(0x1a8),
            names: list(0x1ac),
            quotes: list(0x1b0),
        }
    }

    pub fn spelling(&self, n: i32) -> &[u8] {
        let l = self.spelling.as_deref();
        let i = if n == 1 {
            0
        } else {
            (count_strings(l) != 1) as i32
        };
        string_from_list(l, i)
    }
}

pub fn name_indices(world: &EngineWorld, rng_at_start: BungieRng, tags: &TagLookup) -> Vec<i8> {
    if !world.combat.is_empty() {
        return (0..world.monsters.len())
            .map(|i| world.combat(i).name_index)
            .collect();
    }
    let mut counters: BTreeMap<FourCC, i32> = BTreeMap::new();
    let mut names: BTreeMap<FourCC, i32> = BTreeMap::new();
    let mut out = Vec::with_capacity(world.monsters.len());
    for m in &world.monsters {
        let Some(def) = world.monster_definitions.get(m.definition) else {
            out.push(0);
            continue;
        };
        let count = *names.entry(def.tag).or_insert_with(|| {
            let t = tags
                .get(FourCC(*b"mons"), def.tag)
                .map(|d| MonsterTexts::from_tags(&d, tags));
            count_strings(t.as_ref().and_then(|t| t.names.as_deref()))
        });
        let counter = counters.entry(def.tag).or_insert_with(|| {
            if m.first_draw == 0 {
                return 0;
            }
            let obje = tags.get(FourCC(*b"obje"), def.object).unwrap_or_default();
            let k = 1
                + (be16(&obje, 0x0e) > 0) as u64
                + (be16(&obje, 0x0a) > 0) as u64
                + (def.field_76 > 0) as u64
                + (((i32::from(def.speed) * 7) >> 8) > 0) as u64;
            let mut rng = rng_at_start;
            for _ in 0..m.first_draw + k {
                rng.next_u16();
            }
            i32::from(rng.next_u16() & 0x7fff)
        });
        let index = if count > 0 {
            *counter % count
        } else {
            *counter & 0x7f
        };
        *counter += 1;
        out.push(index as i8);
    }
    out
}

#[derive(Debug, Clone, Copy)]
pub struct SelectionContext {
    pub local_player: i16,
    pub local_team: i16,
    pub mode: i16,
    pub game_options: u32,
    pub replay_type: i32,
}

impl SelectionContext {
    pub fn of(world: &EngineWorld, game_options: u32) -> Self {
        Self {
            local_player: world.local_player,
            local_team: world.local_team(),
            mode: world.mode,
            game_options,
            replay_type: world.replay.map_or(0, |r| i32::from(r.mode)),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StatusIcon {
    pub count: i16,
    pub collection: Option<FourCC>,
    pub sequence: i16,
    pub extra: i16,
}

impl StatusIcon {
    const NONE: Self = Self {
        count: 0,
        collection: None,
        sequence: -1,
        extra: -1,
    };
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StatusRecord {
    pub name: Vec<u8>,
    pub kind: Vec<u8>,
    pub experience: [i16; 3],
    pub ammunition: StatusIcon,
    pub item: StatusIcon,
    pub seed: i16,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Portrait {
    pub collection: FourCC,
    pub sequence: i16,
    pub core: FourCC,
    pub permutation: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnitInfo {
    pub portrait: Option<Portrait>,
    pub description: Vec<u8>,
    pub record: StatusRecord,
}

#[derive(Default)]
pub struct UnitTags {
    texts: BTreeMap<FourCC, Option<Rc<MonsterTexts>>>,
    units: BTreeMap<FourCC, Option<FourCC>>,
    cores: BTreeMap<FourCC, Option<Rc<CorePermutations>>>,
    arts: BTreeMap<FourCC, Option<Rc<InterfaceArt>>>,
}

impl UnitTags {
    fn texts(&mut self, tags: &TagLookup, mons: FourCC) -> Option<Rc<MonsterTexts>> {
        self.texts
            .entry(mons)
            .or_insert_with(|| {
                tags.get(FourCC(*b"mons"), mons)
                    .map(|d| Rc::new(MonsterTexts::from_tags(&d, tags)))
            })
            .clone()
    }

    fn core_of_unit(&mut self, tags: &TagLookup, unit: FourCC) -> Option<FourCC> {
        *self.units.entry(unit).or_insert_with(|| {
            tags.get(FourCC(*b"unit"), unit)
                .and_then(|d| tag_ref(&d, 4))
        })
    }

    fn core(&mut self, tags: &TagLookup, core: FourCC) -> Option<Rc<CorePermutations>> {
        self.cores
            .entry(core)
            .or_insert_with(|| {
                tags.get(FourCC(*b"core"), core)
                    .and_then(|d| CorePermutations::from_bytes(&d).ok())
                    .map(Rc::new)
            })
            .clone()
    }

    pub fn load_art(&mut self, tags: &TagLookup, id: FourCC) -> Option<Rc<InterfaceArt>> {
        self.arts
            .entry(id)
            .or_insert_with(|| {
                tags.get(FourCC(*b".256"), id)
                    .and_then(InterfaceArt::from_bytes)
                    .map(Rc::new)
            })
            .clone()
    }

    pub fn art(&self, id: FourCC) -> Option<&InterfaceArt> {
        self.arts.get(&id)?.as_deref()
    }

    pub fn core_loaded(&self, id: FourCC) -> Option<&CorePermutations> {
        self.cores.get(&id)?.as_deref()
    }
}

pub fn selected_monsters(world: &EngineWorld, ids: &[u32]) -> Vec<usize> {
    ids.iter()
        .filter_map(|&id| world.monsters.iter().position(|m| m.unit_key() == Some(id)))
        .collect()
}

impl UnitInfo {
    pub fn from_selection(
        world: &EngineWorld,
        selection: &[usize],
        names: &[i8],
        ctx: SelectionContext,
        tags: &TagLookup,
        cache: &mut UnitTags,
    ) -> Self {
        let n = selection.len();
        let name_index = |i: usize| i32::from(names.get(i).copied().unwrap_or(0));
        let def_tag = |i: usize| world.monster_definitions[world.monsters[i].definition].tag;
        let texts: Vec<Option<Rc<MonsterTexts>>> = selection
            .iter()
            .map(|&i| cache.texts(tags, def_tag(i)))
            .collect();
        let name_of = |k: usize| -> Vec<u8> {
            let c = world.combat(selection[k]);
            let custom = mandrake_sim::veterans::custom_name_text(&c.custom_name);
            if !custom.is_empty() {
                return custom.into_bytes();
            }
            let idx = if c.name_index >= 0 {
                i32::from(c.name_index)
            } else {
                name_index(selection[k])
            };
            string_from_list(texts[k].as_ref().and_then(|t| t.names.as_deref()), idx).to_vec()
        };

        let units: Vec<(u32, i16)> = selection.iter().map(|&i| world.combat(i).unit).collect();
        let cores: Vec<Option<FourCC>> = units
            .iter()
            .map(|&(u, _)| cache.core_of_unit(tags, FourCC(u.to_be_bytes())))
            .collect();
        let defs: Vec<usize> = selection
            .iter()
            .map(|&i| world.monsters[i].definition)
            .collect();
        let mut best: Option<usize> = None;
        let mut best_score = 0;
        for k in 0..n {
            let score = (0..n)
                .map(|j| {
                    (units[j] == units[k]) as i32
                        + (defs[j] == defs[k]) as i32
                        + (cores[j] == cores[k]) as i32
                })
                .sum::<i32>();
            if score > best_score {
                best = Some(k);
                best_score = score;
            }
        }
        let portrait = best.and_then(|k| {
            let core_id = cores[k]?;
            let core = cache.core(tags, core_id)?;
            let permutation = if n == 1 {
                let p = world.monsters[selection[k]].object.permutation.max(0) as usize;
                if core.count > 0 { p } else { 0 }
            } else {
                0
            };
            cache.load_art(tags, core.collection_tag)?;
            Some(Portrait {
                collection: core.collection_tag,
                sequence: texts[k].as_ref().map_or(-1, |t| t.portrait_sequence),
                core: core_id,
                permutation,
            })
        });

        let description = if n == 1 {
            let l = texts[0].as_ref().and_then(|t| t.quotes.as_deref());
            match l {
                None => Vec::new(),
                Some(_) => {
                    let c = count_strings(l);
                    let c = if c != 0 { c } else { 1 };
                    let combat = world.combat(selection[0]);
                    let idx = if combat.name_index >= 0 {
                        i32::from(combat.name_index)
                    } else {
                        name_index(selection[0])
                    };
                    string_from_list(l, idx.max(0) % c).to_vec()
                }
            }
        } else {
            let mut out = Vec::new();
            let mut first = true;
            let mut left = 0x400usize;
            for k in 0..n {
                let name = name_of(k);
                if name.is_empty() {
                    continue;
                }
                let mut s: Vec<u8> = if first { Vec::new() } else { b", ".to_vec() };
                s.extend_from_slice(&name);
                let w = s.len().min(left - 1);
                out.extend_from_slice(&s[..w]);
                first = false;
                left -= w;
                if left <= 1 {
                    break;
                }
            }
            if !first && left > 1 {
                out.push(b'.');
            }
            out
        };

        let record = StatusRecord {
            name: if n == 1 { name_of(0) } else { Vec::new() },
            kind: Self::type_string(world, selection, &texts, ctx),
            experience: if n == 1 {
                let c = world.combat(selection[0]);
                [
                    i16::from(c.experience),
                    i16::from(c.battles),
                    texts[0].as_ref().map_or(-1, |t| t.experience_view),
                ]
            } else {
                [0, 0, -1]
            },
            ammunition: if n == 1 {
                Self::ammunition(world, selection[0], texts[0].as_deref(), ctx)
            } else {
                StatusIcon::NONE
            },
            item: if n == 1 {
                Self::item(world, selection[0], texts[0].as_deref(), ctx)
            } else {
                StatusIcon::NONE
            },
            seed: selection.first().map_or(-1, |&i| {
                let c = world.combat(i);
                if c.name_index >= 0 {
                    i16::from(c.name_index)
                } else {
                    name_index(i) as i16
                }
            }),
        };
        for icon in [record.ammunition, record.item] {
            if let Some(c) = icon.collection {
                cache.load_art(tags, c);
            }
        }
        Self {
            portrait,
            description,
            record,
        }
    }

    fn type_string(
        world: &EngineWorld,
        selection: &[usize],
        texts: &[Option<Rc<MonsterTexts>>],
        ctx: SelectionContext,
    ) -> Vec<u8> {
        const LIMIT: usize = 0x100;
        let n = selection.len();
        if n == 1 {
            let m = &world.monsters[selection[0]];
            let spell = texts[0].as_ref().map_or(&[][..], |t| t.spelling(1));
            let mut out = Vec::with_capacity(spell.len() + 2);
            out.push(b'(');
            out.extend_from_slice(spell);
            out.push(b')');
            let owner_suffix = || {
                !(ctx.game_options & mandrake_common::game_options::COOPERATIVE != 0
                    && (m.team as u8).wrapping_sub(1) <= 1)
            };
            let suffix = if m.flags & 0x1000_0000 != 0 {
                false
            } else if ctx.mode == 0 && ctx.replay_type != 2 {
                owner_suffix()
            } else if m.owner == ctx.local_player
                || texts[0].as_ref().is_some_and(|t| t.flags_224 & 1 != 0)
            {
                false
            } else {
                owner_suffix()
            };
            if suffix {
                out.push(b' ');
            }
            out.truncate(LIMIT - 1);
            return out;
        }
        let mut counts: Vec<(usize, i32, usize)> = Vec::new();
        for (k, &i) in selection.iter().enumerate() {
            let d = world.monsters[i].definition;
            match counts.iter_mut().find(|c| c.0 == d) {
                Some(c) => c.1 += 1,
                None => counts.push((d, 1, k)),
            }
        }
        counts.sort_by_key(|c| c.0);
        let mut out: Vec<u8> = Vec::new();
        for (j, &(_, count, k)) in counts.iter().enumerate() {
            let mut prefix: Vec<u8> = if j == 0 { Vec::new() } else { b", ".to_vec() };
            prefix.extend_from_slice(format!("{count} ").as_bytes());
            let spell = texts[k].as_ref().map_or(&[][..], |t| t.spelling(count));
            if spell.len() + prefix.len() + out.len() >= LIMIT {
                if out.len() + 3 < LIMIT {
                    out.extend_from_slice(b"...");
                }
                break;
            }
            out.extend_from_slice(&prefix);
            out.extend_from_slice(spell);
        }
        out
    }

    fn ammunition(
        world: &EngineWorld,
        i: usize,
        texts: Option<&MonsterTexts>,
        ctx: SelectionContext,
    ) -> StatusIcon {
        let m = &world.monsters[i];
        let count = world.combat(i).ammunition as i8;
        if count <= 0 || (m.team != ctx.local_team && ctx.mode != 0) {
            return StatusIcon::NONE;
        }
        StatusIcon {
            count: i16::from(count),
            collection: Some(m.object.collection),
            sequence: texts.map_or(-1, |t| t.ammunition_sequence),
            extra: m.object.permutation,
        }
    }

    fn item(
        world: &EngineWorld,
        i: usize,
        texts: Option<&MonsterTexts>,
        ctx: SelectionContext,
    ) -> StatusIcon {
        let m = &world.monsters[i];
        let c = world.combat(i);
        if c.carried == -1 || (m.team != ctx.local_team && ctx.mode != 0) || c.special != -1 {
            return StatusIcon::NONE;
        }
        StatusIcon {
            count: 0,
            collection: Some(m.object.collection),
            sequence: texts.map_or(-1, |t| t.ammunition_sequence),
            extra: m.object.permutation,
        }
    }
}

pub trait StatusArt {
    fn status_art(&self, id: FourCC) -> Option<&InterfaceArt>;
}

impl StatusArt for UnitTags {
    fn status_art(&self, id: FourCC) -> Option<&InterfaceArt> {
        self.art(id)
    }
}

pub struct DrawContext<'a> {
    pub family: &'a FontFamily,
    pub locale: &'a TextLocale,
    pub cache: &'a dyn StatusArt,
    pub experience: Option<&'a InterfaceArt>,
    pub color: RgbColor,
}

fn count_text(count: i16, percent: bool) -> Vec<u8> {
    let c = i32::from(count);
    if c > 9999 {
        format!("{}k", c / 1000)
    } else if percent && c < 0 {
        format!("{}%", -c)
    } else {
        c.to_string()
    }
    .into_bytes()
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct StatusLayout {
    pub type_offset: i32,
    pub counts: Vec<Rect>,
    pub icons: Vec<(i32, i32)>,
    pub end: i32,
}

pub fn draw_status_control(
    bmp: &mut Bitmap32,
    b: Rect,
    record: &StatusRecord,
    show_all: bool,
    cx: &DrawContext,
) -> StatusLayout {
    let mut out = StatusLayout::default();
    let text = |bmp: &mut Bitmap32, pen: (i32, i32), rect: Rect, style: i16, s: &[u8]| {
        font::draw_string_at(
            bmp,
            pen,
            rect,
            TextClip::None,
            cx.family,
            style,
            s,
            cx.color,
            text_flags::SHADOW,
            0,
            cx.locale,
        )
    };
    let full = Rect::new(0, 0, bmp.height, bmp.width);
    let icon = |bmp: &mut Bitmap32,
                out: &mut StatusLayout,
                art: &InterfaceArt,
                bm: &interface_bitmap::StoredBitmap,
                x| {
        interface_bitmap::copy_bitmap(bmp, bm, art.palette(), x, b.top, full, 0);
        out.icons.push((x, bm.width));
    };
    let mut p = (0, 0);
    if !record.name.is_empty() {
        p = text(bmp, p, b, font::STYLE_BOLD, &record.name);
        p.0 += 10;
    }
    out.type_offset = p.0;
    if !record.kind.is_empty() {
        p = text(bmp, p, b, font::STYLE_ITALIC, &record.kind);
    }
    let mut x = b.right;
    let a = record.ammunition;
    if let Some(c) = a.collection {
        let rect = Rect::new(b.top, b.right - 45, b.bottom, b.right);
        text(
            bmp,
            (0, 0),
            rect,
            font::STYLE_PLAIN,
            &count_text(a.count, false),
        );
        out.counts.push(rect);
        x -= 45;
        if let Some(art) = cx.cache.status_art(c)
            && let Some((bm, _)) = art.bitmap(i32::from(a.sequence), -1, i32::from(record.seed))
        {
            x -= bm.width;
            icon(bmp, &mut out, art, bm, x);
            x -= 10;
        }
    }
    let it = record.item;
    if let Some(c) = it.collection {
        let found = cx.cache.status_art(c).and_then(|art| {
            art.bitmap(i32::from(it.sequence), -1, i32::from(record.seed))
                .map(|(bm, _)| (art, bm))
        });
        if it.count != 0 {
            let rect = Rect::new(b.top, x - 45, b.bottom, x);
            text(
                bmp,
                (0, 0),
                rect,
                font::STYLE_PLAIN,
                &count_text(it.count, true),
            );
            out.counts.push(rect);
            x -= 45;
        }
        if let Some((art, bm)) = found {
            x -= bm.width;
            icon(bmp, &mut out, art, bm, x);
            x -= 10;
        }
    }
    let [xp, xp2, view] = record.experience;
    let Some(exp) = cx.experience.filter(|e| view != -1 && e.frames(2) != 0) else {
        out.end = x;
        return out;
    };
    let seed = i32::from(record.seed);
    for pass in 1..=2 {
        let (n, v) = if pass == 1 {
            (i32::from(xp2), 3 - i32::from(view))
        } else {
            (i32::from(xp), i32::from(view))
        };
        if n == 0 {
            if pass == 2 {
                break;
            }
            continue;
        }
        let first = exp.bitmap(2, v, seed).map(|(bm, _)| bm);
        let mut capacity = first.map_or(0, |bm| (x - (b.left + p.0 + 10)) / (bm.width + 2));
        if !show_all {
            capacity = capacity.min(9);
        }
        if capacity < n {
            let s: Vec<u8> = if n > 249 {
                b"250+".to_vec()
            } else {
                n.to_string().into_bytes()
            };
            let right = x + 1;
            x -= font::string_width(cx.family, font::STYLE_PLAIN, &s, cx.locale) + 5;
            let rect = Rect::new(b.top, x, b.bottom, right);
            text(bmp, (0, 0), rect, font::STYLE_PLAIN, &s);
            out.counts.push(rect);
            if let Some(bm) = first {
                x -= bm.width;
                icon(bmp, &mut out, exp, bm, x);
                x -= 2;
            }
        } else {
            for i in 0..n {
                let Some((bm, _)) = exp.bitmap(2, v, seed + i) else {
                    continue;
                };
                x -= bm.width;
                icon(bmp, &mut out, exp, bm, x);
                x -= 2;
                if b.left + p.0 + 9 >= x {
                    break;
                }
            }
        }
        if pass == 1 {
            x -= 10;
        }
    }
    out.end = x;
    out
}

pub fn draw_portrait(
    bmp: &mut Bitmap32,
    rect: Rect,
    portrait: &Portrait,
    cache: &UnitTags,
    frame_art: &InterfaceArt,
    frame_sequence: i32,
) {
    if let Some(art) = cache.art(portrait.collection) {
        let palette = cache
            .core_loaded(portrait.core)
            .map(|core| art.coll.permutation_palette(core, portrait.permutation));
        interface_bitmap::draw_bitmap_from_sequence_shaded(
            bmp,
            art,
            palette.as_deref().unwrap_or(art.palette()),
            i32::from(portrait.sequence),
            0,
            0,
            rect,
            placement::REGISTRATION,
            0,
            0,
        );
    }
    interface_bitmap::draw_bitmap_from_sequence(
        bmp,
        frame_art,
        frame_sequence,
        0,
        0,
        rect,
        placement::REGISTRATION,
        0,
        0,
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn string_lists_split_like_the_engine() {
        assert_eq!(
            string_list(b"Mauriac\rAdalard\r\r\nAjax\0"),
            vec![b"Mauriac".to_vec(), b"Adalard".to_vec(), b"Ajax".to_vec()]
        );
        let l = string_list(b"a\rb");
        assert_eq!(string_from_list(Some(&l), 1), b"b");
        assert_eq!(string_from_list(Some(&l), 2), b"");
        assert_eq!(string_from_list(Some(&l), -1), b"");
        assert_eq!(string_from_list(None, 0), b"");
    }

    #[test]
    fn count_texts() {
        assert_eq!(count_text(1, false), b"1");
        assert_eq!(count_text(12345, false), b"12k");
        assert_eq!(count_text(-40, true), b"40%");
        assert_eq!(count_text(-40, false), b"-40");
    }

    #[test]
    fn the_replay_game_type_comes_from_the_replay() {
        use mandrake_sim::engine_rng::{EngineMonster, EngineObject, EngineWorld, ReplayGame};
        let mut world = EngineWorld::default();
        world.mode = 0;
        world.replay = Some(ReplayGame {
            mode: 2,
            solo: true,
        });
        world.monsters.push(EngineMonster {
            index: 0,
            identifier: 10000,
            first_draw: 0,
            definition: 0,
            object: EngineObject::new(0, FourCC::new(*b"test")),
            mode: 1,
            anim_state: 0x1a,
            speed: 0,
            position: (0, 0),
            flags: 0,
            flags_8: 0,
            ambient: None,
            unmodelled: None,
            mirrored: false,
            team: 0,
            owner: 0,
            facing: 0,
            impassable: 0,
            neighbours: 0,
            formation: Default::default(),
            cell_link: 0,
            flying_height: 0,
        });
        let ctx = SelectionContext::of(&world, 0);
        assert_eq!((ctx.mode, ctx.replay_type), (0, 2));
        assert_eq!(UnitInfo::type_string(&world, &[0], &[None], ctx), b"()");
        let other = SelectionContext {
            replay_type: 0,
            ..ctx
        };
        assert_eq!(UnitInfo::type_string(&world, &[0], &[None], other), b"() ");
        world.replay = None;
        world.mode = 2;
        assert_eq!(SelectionContext::of(&world, 0).replay_type, 0);
    }

    #[test]
    fn unit_info_retains_veteran_custom_name_battles_and_seed() {
        use mandrake_sim::engine_rng::{EngineMonster, EngineObject, EngineWorld};
        let mut world = EngineWorld::default();
        world.monsters.push(EngineMonster {
            index: 0,
            identifier: 10000,
            first_draw: 0,
            definition: 0,
            object: {
                let mut obj = EngineObject::new(0, FourCC::new(*b"test"));
                obj.permutation = 3;
                obj
            },
            mode: 1,
            anim_state: 0x1a,
            speed: 0,
            position: (0, 0),
            flags: 0,
            flags_8: 0,
            ambient: None,
            unmodelled: None,
            mirrored: false,
            team: 0,
            owner: 0,
            facing: 0,
            impassable: 0,
            neighbours: 0,
            formation: Default::default(),
            cell_link: 0,
            flying_height: 0,
        });
        world.monster_definitions.push(
            mandrake_sim::engine_rng::MonsterDefinition::from_bytes(
                FourCC::new(*b"test"),
                &[0u8; 0x224],
            )
            .unwrap(),
        );
        *world.combat_mut(0) = mandrake_sim::attack_ai::MonsterCombat {
            experience: 7,
            battles: 3,
            name_index: 4,
            custom_name: mandrake_sim::veterans::custom_name("Sir Veteran"),
            unit: (u32::from_be_bytes(*b"test"), 0),
            ..Default::default()
        };
        let ctx = SelectionContext::of(&world, 0);
        let tags = mandrake_tag::TagLookup::empty();
        let mut cache = UnitTags::default();
        let info = UnitInfo::from_selection(&world, &[0], &[4], ctx, &tags, &mut cache);
        assert_eq!(info.record.name, b"Sir Veteran");
        assert_eq!(info.record.experience, [7, 3, -1]);
        assert_eq!(info.record.seed, 4);

        let dummy_rng = mandrake_math::BungieRng::new(0);
        let names = name_indices(&world, dummy_rng, &tags);
        assert_eq!(names, vec![4]);
    }
}
