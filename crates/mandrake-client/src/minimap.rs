use mandrake_view::interface_bitmap::InterfaceArt;

use crate::world_view::{MinimapStart, MinimapView, WorldView};

pub const VISIBILITY_STENCILS: [(i32, i32); 4] = [(20, 22), (26, 28), (10, 12), (36, 38)];

#[derive(Debug, Clone, PartialEq, Eq)]
struct StencilRow {
    x_offset: i32,
    count: i32,
    words: Vec<u32>,
}

fn build_stencil(inner: i32, outer: i32) -> Vec<StencilRow> {
    (-outer..=outer)
        .map(|dy| {
            let w_out = isqrt((outer + 1) * (outer + 1) - dy * dy).min(outer);
            let w_in = if dy < inner && -inner < dy {
                isqrt((inner + 1) * (inner + 1) - dy * dy).min(inner)
            } else {
                0
            };
            let count = 2 * w_out + 1;
            let mut words = Vec::new();
            let mut shift = 0;
            for dx in -w_out..=w_out {
                let v = if w_in != 0 && -w_in <= dx && dx <= w_in {
                    3
                } else {
                    1
                };
                if shift == 0 {
                    words.push(v);
                    shift = 2;
                } else {
                    *words.last_mut().unwrap() |= v << shift;
                    shift += 2;
                    if shift >= 32 {
                        shift = 0;
                    }
                }
            }
            StencilRow {
                x_offset: outer - w_out,
                count: if w_out < -w_out { 0 } else { count },
                words,
            }
        })
        .collect()
}

fn isqrt(v: i32) -> i32 {
    mandrake_math::engine::isqrt(v.max(0) as u32) as i32
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct VisibilityMonster {
    pub used: bool,
    pub flag_8_2: bool,
    pub flag_7_40: bool,
    pub class: i16,
    pub stencil: i16,
    pub marker_flag_8: bool,
    pub team: i16,
    pub object_flag_80: bool,
    pub position: (i32, i32),
}

#[derive(Debug, Clone)]
pub struct Visibility {
    width: i32,
    height: i32,
    stencils: Vec<Vec<StencilRow>>,
    accumulating: Vec<u32>,
    pub visible: Vec<u32>,
    pub explored: Vec<u32>,
    pub center: (i16, i16),
    sum: (i32, i32),
    count: u16,
}

impl Visibility {
    pub fn new(width: i32, height: i32) -> Self {
        let words = (width * height / 4 / 4).max(0) as usize;
        Self {
            width,
            height,
            stencils: VISIBILITY_STENCILS
                .iter()
                .map(|&(i, o)| build_stencil(i, o))
                .collect(),
            accumulating: vec![0; words],
            visible: vec![0; words],
            explored: vec![0; words],
            center: (0, 0),
            sum: (0, 0),
            count: 0,
        }
    }

    fn word_index(&self, x: i32, y: i32) -> usize {
        (((self.width as i16) >> 4) as i32 * y + (x >> 4)) as usize
    }

    pub fn is_visible(&self, x: i32, y: i32) -> bool {
        visible_cell_bits(&self.visible, (self.width, self.height), x, y) & 1 != 0
    }

    pub fn is_explored(&self, x: i32, y: i32) -> bool {
        visible_cell_bits(&self.explored, (self.width, self.height), x, y) & 1 != 0
    }

    pub fn view_on_explored_cell(&self, x: i32, y: i32, facing: u16) -> bool {
        let trig = mandrake_math::engine::trig_tables();
        let along = ((i32::from(facing) + 0x20) >> 6 & 0x3ff) as usize;
        let across = ((i32::from(facing) + 0x4020) >> 6 & 0x3ff) as usize;
        let (c0, s0) = (i32::from(trig.cosine[along]), i32::from(trig.sine[along]));
        let (c1, s1) = (i32::from(trig.cosine[across]), i32::from(trig.sine[across]));
        [
            (0x3c00, -0x1e00),
            (0x3c00, 0x1e00),
            (0xc00, 0xe00),
            (0xc00, -0xe00),
        ]
        .into_iter()
        .any(|(ahead, side)| {
            let cx = ((c0 * ahead + c1 * side) >> 14) + x;
            let cy = ((s0 * ahead + s1 * side) >> 14) + y;
            self.explored_inner((cx >> 9) as i16, (cy >> 9) as i16)
        })
    }

    fn explored_inner(&self, x: i16, y: i16) -> bool {
        let (x, y) = (i32::from(x), i32::from(y));
        if x < 0 || y < 0 || x >= self.width || y >= self.height {
            return false;
        }
        self.explored
            .get(self.word_index(x, y))
            .is_some_and(|w| w >> (2 * (x & 15)) & 2 != 0)
    }

    fn rasterize(&mut self, stencil: usize, x: i32, y: i32) {
        self.sum.0 += x;
        self.sum.1 += y;
        self.count = self.count.wrapping_add(1);
        let Some(rows) = self.stencils.get(stencil) else {
            return;
        };
        let radius = VISIBILITY_STENCILS[stencil].1;
        let (x0, mut cy) = (x - radius, y - radius);
        for k in 0..rows.len() {
            let row = &self.stencils[stencil][k];
            if cy >= 0 && cy < self.height && row.count > 0 {
                let mut left = row.count;
                let mut src_pos = 0i32;
                let mut src_word = 0usize;
                let mut cx = row.x_offset + x0;
                let mut dst = self.word_index(cx, cy) as isize;
                while left > 0 {
                    let dbit = cx & 0xf;
                    let sbit = src_pos & 0xf;
                    let n = left.min(16).min(16 - dbit);
                    let w0 = row.words.get(src_word).copied().unwrap_or(0);
                    let mut bits = (w0 >> (2 * sbit as u32)).wrapping_shl(2 * dbit as u32);
                    if 16 - sbit <= n {
                        src_word += 1;
                        if 16 - sbit != n {
                            let w1 = row.words.get(src_word).copied().unwrap_or(0);
                            bits |= w1.wrapping_shl((2 * dbit + 2 * (16 - sbit)) as u32);
                        }
                    }
                    if cx >= 0 && cx < self.width {
                        let mask = (u32::MAX >> (32 - 2 * n) as u32).wrapping_shl(2 * dbit as u32);
                        if let Some(d) = self.accumulating.get_mut(dst as usize) {
                            *d |= bits & mask;
                        }
                    }
                    left -= n;
                    src_pos += n;
                    cx += n;
                    dst += 1;
                }
            }
            cy += 1;
        }
    }

    fn swap(&mut self, game_options: u32) {
        let option_4 = game_options & mandrake_common::game_options::LIMITED_VISIBILITY != 0;
        if self.count == 0 && option_4 {
            self.reset_counters();
            return;
        }
        if self.count != 0 {
            let n = self.count as i16 as i32;
            self.center = ((self.sum.0 / n) as i16, (self.sum.1 / n) as i16);
        }
        for (e, a) in self.explored.iter_mut().zip(&self.accumulating) {
            *e |= *a;
        }
        std::mem::swap(&mut self.accumulating, &mut self.visible);
        let fill = if option_4 { 0 } else { u32::MAX };
        self.accumulating.fill(fill);
        self.reset_counters();
    }

    fn reset_counters(&mut self) {
        self.sum = (0, 0);
        self.count = 0;
    }

    fn rasterize_monster(&mut self, m: &VisibilityMonster, local_team: i16) {
        if !m.used || m.flag_8_2 || m.class == 5 || m.stencil == -1 {
            return;
        }
        if !m.marker_flag_8 {
            if m.flag_7_40 {
                return;
            }
            if m.team != local_team && local_team != -1 {
                return;
            }
        }
        let stencil = if m.object_flag_80 { 2 } else { m.stencil };
        if !(0..4).contains(&stencil) {
            return;
        }
        self.rasterize(stencil as usize, m.position.0 >> 9, m.position.1 >> 9);
    }

    pub fn build_initial(
        &mut self,
        monsters: &[VisibilityMonster],
        local_team: i16,
        game_options: u32,
    ) {
        *self = Self::new(self.width, self.height);
        for _ in 0..2 {
            self.swap(game_options);
            for m in monsters {
                self.rasterize_monster(m, local_team);
            }
        }
    }

    pub fn update(
        &mut self,
        time: i32,
        monsters: &[VisibilityMonster],
        local_team: i16,
        game_options: u32,
    ) {
        let first = (time & 0x3f) as usize;
        if first == 0 {
            self.swap(game_options);
        }
        for m in monsters.iter().skip(first).step_by(64) {
            self.rasterize_monster(m, local_team);
        }
    }
}

pub fn visible_cell_bits(visible: &[u32], mesh: (i32, i32), col: i32, row: i32) -> u32 {
    if col < 0 || row < 0 || col >= mesh.0 || row >= mesh.1 {
        return 0;
    }
    let word = ((mesh.0 as i16) >> 4) as i32 * row + (col >> 4);
    visible
        .get(word as usize)
        .map_or(0, |w| w >> ((col * 2) & 0x1e))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ObjectVisibility {
    pub drawn: bool,
    pub full: bool,
}

pub fn local_object_visibility(
    flags: u32,
    kind: u8,
    bits: u32,
    game_type: i16,
    gameplay_version: i32,
) -> ObjectVisibility {
    if flags & 0x2000_0200 != 0 {
        return ObjectVisibility {
            drawn: false,
            full: false,
        };
    }
    let (mut drawn, mut full) = (true, true);
    if flags & 0x400_0000 != 0 {
        drawn = bits & 1 != 0;
        full = false;
        if bits & 2 != 0 && flags & 0x1000_0000 == 0 {
            drawn = true;
            full = true;
        }
    }
    if kind == 6
        && (game_type != 3 || gameplay_version <= mandrake_common::game_options::GAMEPLAY_1_5_1)
    {
        drawn = true;
        full = true;
    }
    ObjectVisibility { drawn, full }
}

const UNCLAIMED_TEAM_COLORS: [[u16; 3]; 8] = [
    [0, 0xffff, 0],
    [0xffff, 0, 0],
    [0, 0, 0],
    [0, 0, 0],
    [0xffff, 0xffff, 0],
    [0, 0, 0xffff],
    [0xffff, 0xffff, 0],
    [0xffff, 0, 0],
];

pub fn team_blip_colors(claimed: &[Option<[u16; 3]>]) -> Vec<[u16; 3]> {
    claimed
        .iter()
        .enumerate()
        .map(|(team, c)| match c {
            Some(c) => *c,
            None => UNCLAIMED_TEAM_COLORS
                .get(team)
                .copied()
                .unwrap_or([0, 0, 0]),
        })
        .collect()
}

pub const BLIP_COLORS: [[u8; 3]; 6] = [
    [0xff, 0xff, 0],
    [0, 0, 0xff],
    [0xff, 0xff, 0],
    [0xff, 0, 0],
    [0, 0xff, 0],
    [0xff, 0, 0],
];

pub const OUTLINE_COLOR: [u8; 3] = BLIP_COLORS[0];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BlipTable {
    pub shading: Vec<u32>,
    pub index: [u8; 20],
}

impl BlipTable {
    pub fn color(&self, k: usize) -> Option<[u8; 3]> {
        let i = *self.index.get(k)?;
        let c = *self.shading.get(i as usize)?;
        Some([(c >> 16) as u8, (c >> 8) as u8, c as u8])
    }
}

pub fn free_blip_colors(
    pixels: &mut [u8],
    color_table: &mut [[u16; 4]],
    team_colors: Option<&[[u16; 3]]>,
) -> BlipTable {
    let mut shading: Vec<u32> = (0..256)
        .map(|i| {
            color_table.get(i).map_or(0, |c| {
                (u32::from(c[0] >> 8) << 16) | (u32::from(c[1] >> 8) << 8) | u32::from(c[2] >> 8)
            })
        })
        .collect();
    shading[0] = 0;
    let n = team_colors.map_or(6, |t| t.len() + 4);
    let mut index = [0xffu8; 20];
    let mut counts = [0i32; 256];
    for &p in pixels.iter() {
        counts[p as usize] += 1;
    }
    for slot in index.iter_mut().take(n.min(20)) {
        let (mut best, mut low) = (-1i32, i32::MAX);
        for (i, &c) in counts.iter().enumerate() {
            if c < low {
                low = c;
                best = i as i32;
            }
        }
        if best < 0 {
            break;
        }
        let freed = best as usize;
        counts[freed] = i32::MAX;
        *slot = freed as u8;
        let Some(old) = color_table.get(freed).copied() else {
            continue;
        };
        color_table[freed] = [0, 0, 0, 0];
        let mut nearest = -1i32;
        let mut dist = 0i32;
        for (i, c) in color_table.iter().enumerate() {
            let dr = (old[0] as i32 - c[0] as i32) >> 4;
            let dg = (old[1] as i32 - c[1] as i32) >> 4;
            let db = (old[2] as i32 - c[2] as i32) >> 4;
            let d = 2 * dr * dr + 5 * dg * dg + db * db;
            if nearest == -1 || d < dist {
                nearest = i as i32;
                dist = d;
            }
        }
        for p in pixels.iter_mut().filter(|p| **p as usize == freed) {
            *p = nearest as u8;
        }
    }
    for (k, &i) in index.iter().enumerate().take(n.min(20)) {
        let c = match team_colors {
            Some(t) if k >= 4 => {
                let c = t[k - 4];
                [(c[0] >> 8) as u8, (c[1] >> 8) as u8, (c[2] >> 8) as u8]
            }
            _ => BLIP_COLORS[k.min(5)],
        };
        shading[i as usize] = (u32::from(c[0]) << 16) | (u32::from(c[1]) << 8) | u32::from(c[2]);
    }
    BlipTable { shading, index }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct BlipMonster {
    pub used: bool,
    pub flag_7_40: bool,
    pub flag_7_10: bool,
    pub flag_9_10: bool,
    pub flag_9_20: bool,
    pub team: i16,
    pub owner: i16,
    pub definition_flags: u32,
    pub class: i16,
    pub object: i32,
    pub object_flags: u32,
    pub object_33: u8,
    pub position: (i32, i32),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BlipRules {
    pub mode: i16,
    pub game_type: i16,
    pub game_options: u32,
    pub local_team: i16,
    pub local_player: i16,
    pub team_mode: bool,
    pub solo_replay: bool,
    pub allied_selectable: bool,
    pub alliances: mandrake_sim::ally::AllianceView,
}

fn evaluate_for_selection(m: &BlipMonster, rules: &BlipRules) -> i32 {
    let mut e = 0;
    if rules.alliances.teams_are_allied(rules.local_team, m.team) {
        let owned = (m.owner == rules.local_player) as i32;
        e = if rules.allied_selectable {
            owned * 4 + 1
        } else {
            owned * 5
        };
        if m.flag_7_10 || rules.mode == 0 {
            e &= !1;
        }
        e |= 2;
    }
    if m.flag_9_10 || m.class == 5 {
        e = if m.definition_flags & 0x200_0000 == 0 {
            e & !1
        } else {
            e & !3
        };
    }
    e
}

pub fn blip_type(m: &BlipMonster, rules: &BlipRules, visible: &dyn Fn(i32, i32) -> bool) -> i16 {
    if !m.used || m.flag_7_40 {
        return -1;
    }
    let mut t: i16 = -1;
    if m.class != 5 {
        let e = evaluate_for_selection(m, rules);
        let ambient = m.flag_9_10;
        if ambient {
            t = if m.definition_flags & 0x200_0000 != 0 {
                5
            } else {
                2
            };
            if m.definition_flags & 0x1_0000 != 0 && !(rules.game_type == 12 && m.flag_9_20) {
                t = -1;
            }
        } else if e & 2 != 0 || rules.alliances.teams_are_allied(m.team, rules.local_team) {
            t = if e & 4 != 0 && !m.flag_7_10 { 4 } else { 1 };
        } else {
            let red = rules.game_options & mandrake_common::game_options::SHOW_ENEMY != 0
                || (rules.mode == 0 && !rules.solo_replay);
            if red && m.object_flags & 0x800_0080 == 0 && m.object_33 != 6 {
                t = 5;
            }
        }
        if t != -1 && rules.team_mode {
            t = m.team + 4;
        }
        if rules.game_options & mandrake_common::game_options::LIMITED_VISIBILITY != 0
            && !(rules.alliances.teams_are_allied(m.team, rules.local_team)
                && m.team != rules.local_team)
        {
            let (cx, cy) = (m.position.0 >> 9, m.position.1 >> 9);
            if !visible(cx, cy) {
                t = -1;
            }
        }
    }
    if rules.mode == 1 && rules.local_team == -1 {
        t = m.team + 4;
    }
    if rules.game_type == 12 && m.flag_9_20 {
        t = 2;
    }
    t
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MapState {
    Hidden,
    FadingIn,
    Shown,
    FadingOut,
}

pub fn fade_transparency(state: MapState, count: i32) -> (u16, u16) {
    let map = |c: i32| (c.wrapping_mul(0xbfff) / 15) as u16;
    let line = |c: i32| c.wrapping_mul(0x1111) as u16;
    match state {
        MapState::Hidden => (0xffff, 0xffff),
        MapState::Shown => (0x4000, 0),
        MapState::FadingIn => (!map(count), !line(count)),
        MapState::FadingOut => (map(count).wrapping_add(0x4000), line(count)),
    }
}

pub fn centring_offset(w: i32, h: i32, x: (i16, i16), y: (i16, i16)) -> (i16, i16) {
    let dx = x.1 as i32 - x.0 as i32;
    let dy = y.1 as i32 - y.0 as i32;
    (
        (((w - dx) >> 1) - (x.0 as u16) as i32) as i16,
        (((h - dy) >> 1) - y.0 as i32) as i16,
    )
}

const CORNER_ROWS: [usize; 4] = [4, 2, 1, 1];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Fill {
    pub rect: [i32; 4],
    pub color: [u8; 3],
}

#[derive(Debug, Clone, PartialEq)]
pub struct MinimapFrame {
    pub dst: [i32; 4],
    pub map_alpha: u8,
    pub fills: Vec<Fill>,
    pub fill_alpha: u8,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct OutlineView {
    pub from: (i32, i32),
    pub to: (i32, i32),
    pub polygon: Vec<[i32; 2]>,
}

#[derive(Clone, Copy)]
pub struct MapSight<'a> {
    pub center: (i16, i16),
    pub explored: &'a dyn Fn(i32, i32) -> bool,
}

pub struct OverheadMap {
    pub map: Vec<u8>,
    pub map_w: i32,
    pub map_h: i32,
    pub window: (i32, i32),
    pub explored_only: bool,
    pub tables: [BlipTable; 2],
    pub team_mode: bool,
    pub overlay: Vec<u8>,
    pub overlay_w: i32,
    pub overlay_h: i32,
    pub mesh: (i32, i32),
    pub state: MapState,
    pub count: i32,
    pub transparency: (u16, u16),
    pub zoom: f32,
    pub screen: (i32, i32),
    pub relative_zoom: f32,
    pub offset: (i16, i16),
    pub scroll: (i16, i16),
    stamp_local: i32,
    stamp_tick: i32,
    pub rect: [i32; 4],
    pub view_drop: i32,
    scaled: (i32, i32),
    pub blips: Vec<(i32, i16, (i32, i32))>,
    blip_tick: i32,
    press: u8,
    overlay_rev: u64,
}

pub fn map_window(screen_w: i32, screen_h: i32) -> (i32, i32) {
    let w = match screen_w {
        ..=0x320 => 0x80,
        0x321..=0x500 => 0x100,
        0x501..=0x690 => 0x180,
        _ => 0x200,
    };
    let h = match screen_h {
        ..=0x280 => 0x80,
        0x281..=0x438 => 0x100,
        0x439..=0x578 => 0x180,
        _ => 0x200,
    };
    (w, h)
}

pub const MESH_FLAG_EXPLORED_MAP: u32 = 0x8_0000;
pub const MESH_FLAG_UNCLAMPED_WINDOW: u32 = 0x800_0000;

impl OverheadMap {
    pub fn new(
        art: &InterfaceArt,
        mesh: (i32, i32),
        mesh_flags: u32,
        screen: (i32, i32),
        team_colors: &[[u16; 3]],
        team_mode: bool,
    ) -> Option<Self> {
        let (bitmap, _) = art.bitmap(0, 0, 0)?;
        let (map_w, map_h) = (bitmap.width, bitmap.height);
        if map_w < 8 || map_h < 8 || map_w > 0x1000 || map_h > 0x1000 || mesh.0 <= 0 || mesh.1 <= 0
        {
            return None;
        }
        let mut map: Vec<u8> = bitmap
            .pixels
            .iter()
            .map(|p| p.map_or(0, |(i, _)| i))
            .collect();
        let mut colors = art.coll.color_table.clone();
        let tables = [
            free_blip_colors(&mut map, &mut colors, None),
            free_blip_colors(&mut map, &mut colors, Some(team_colors)),
        ];
        Some(Self::from_map(
            map, map_w, map_h, tables, mesh, mesh_flags, screen, team_mode,
        ))
    }

    #[allow(clippy::too_many_arguments)]
    pub fn from_map(
        map: Vec<u8>,
        map_w: i32,
        map_h: i32,
        tables: [BlipTable; 2],
        mesh: (i32, i32),
        mesh_flags: u32,
        screen: (i32, i32),
        team_mode: bool,
    ) -> Self {
        let explored_only = mesh_flags & MESH_FLAG_EXPLORED_MAP != 0;
        let mut window = map_window(screen.0, screen.1);
        let (mut ow, mut oh) = (map_w, map_h);
        if explored_only {
            if mesh_flags & MESH_FLAG_UNCLAMPED_WINDOW == 0 {
                ow = ow.min(window.0);
                oh = oh.min(window.1);
            }
            window = (ow, oh);
        }
        let mut m = Self {
            map,
            map_w,
            map_h,
            window,
            explored_only,
            tables,
            team_mode,
            overlay: vec![0; (ow * oh) as usize],
            overlay_w: ow,
            overlay_h: oh,
            mesh,
            state: MapState::Hidden,
            count: 0,
            transparency: (0xffff, 0xffff),
            zoom: 1.0,
            screen,
            relative_zoom: 1.0,
            offset: (0, 0),
            scroll: (0, 0),
            stamp_local: 0,
            stamp_tick: 0,
            rect: [0; 4],
            view_drop: 0,
            scaled: (0, 0),
            blips: Vec::new(),
            blip_tick: -1,
            press: 0,
            overlay_rev: 0,
        };
        m.set_zoom(m.base_scale() * m.relative_zoom);
        m
    }

    pub fn table(&self) -> &BlipTable {
        &self.tables[self.team_mode as usize]
    }

    pub fn base_scale(&self) -> f32 {
        let (w, h) = if self.explored_only {
            self.window
        } else {
            (self.map_w, self.map_h)
        };
        mandrake_tag::hud::minimap_base_scale(w, h, self.screen.0, self.screen.1)
    }

    fn set_zoom(&mut self, zoom: f32) {
        self.zoom = zoom;
        self.scaled = (
            (self.map_w as f32 * zoom) as i32,
            (self.map_h as f32 * zoom) as i32,
        );
        self.relative_zoom = zoom / self.base_scale();
    }

    pub fn rezoom(&mut self, screen: (i32, i32)) {
        self.screen = screen;
        self.set_zoom(self.base_scale() * self.relative_zoom);
    }

    pub fn set_visible(&mut self, show: bool) {
        match (show, self.state) {
            (true, MapState::Hidden) => {
                self.state = MapState::FadingIn;
                self.count = 0;
                self.stamp_local = 0;
            }
            (true, MapState::FadingOut) => {
                self.state = MapState::FadingIn;
                self.count = 14 - self.count;
            }
            (false, MapState::Shown) => {
                self.state = MapState::FadingOut;
                self.count = 0;
            }
            (false, MapState::FadingIn) => {
                self.state = MapState::FadingOut;
                self.count = 14 - self.count;
            }
            _ => {}
        }
        self.transparency = fade_transparency(self.state, self.count);
        self.rect = [0; 4];
    }

    pub fn step_fade(&mut self, elapsed: i32) {
        self.count += elapsed;
        match self.state {
            MapState::FadingIn if self.count > 14 => self.state = MapState::Shown,
            MapState::FadingOut if self.count > 14 => self.state = MapState::Hidden,
            _ => {}
        }
        self.transparency = fade_transparency(self.state, self.count);
    }

    pub fn drawn(&self) -> bool {
        self.state != MapState::Hidden
    }

    fn rebuild_with(&mut self, center: (i16, i16), explored: &dyn Fn(i32, i32) -> bool) {
        if !self.explored_only {
            if self.overlay_rev == 0 {
                self.overlay.copy_from_slice(&self.map);
                self.overlay_rev = 1;
            }
            return;
        }
        let (mw, mh) = self.mesh;
        let (map_w, map_h) = (self.map_w, self.map_h);
        if mw <= 0 || mh <= 0 || map_w <= 0 || map_h <= 0 {
            return;
        }
        let (win_w, win_h) = self.window;
        let step_x = (mw << 16) / map_w;
        let step_y = (mh << 16) / map_h;
        let (cx, cy) = (center.0 as i32, center.1 as i32);
        let map_cx = (mw - cx) * map_w / mw;
        let map_cy = cy * map_h / mh;
        let half =
            |v: i32| ((v as i16 as i32 + ((v as i16 as u16 >> 15) as i32)) as i16 >> 1) as i32;
        let mut sx = ((map_cx - half(win_w)) as i16).max(0) as i32;
        let mut sy_raw = (map_cy - half(win_h)) as i16 as i32;
        let (mut copy_w, mut copy_h) = (win_w, win_h);
        if map_w < sx + win_w {
            sx = map_w - win_w;
            sy_raw = sy_raw.max(0);
            if map_h < sy_raw + win_h {
                sy_raw = map_h - win_h;
            }
            sy_raw = sy_raw.max(0);
            if map_w < sx + win_w {
                copy_w = map_w - sx;
            }
        } else if map_h < sy_raw + win_h {
            sy_raw = map_h - win_h;
        }
        let sy = (sy_raw as i16).max(0) as i32;
        if map_h < sy + copy_h {
            copy_h = map_h - sy;
        }
        self.scroll = (sx as i16, sy as i16);
        let (mut x0, mut x1, mut y0, mut y1) = (0x7fffi16, -0x8000i16, 0x7fffi16, -0x8000i16);
        let ow = self.overlay_w;
        let mut changed = self.overlay_rev == 0;
        if copy_h > 0 {
            let mut acc_y = step_y * sy;
            for row in 0..copy_h {
                let cell_y = acc_y >> 16;
                let mut acc_x = (mw << 16) - 1 - sx * step_x;
                for x in 0..copy_w.max(0) {
                    let cell_x = acc_x >> 16;
                    let src = ((row + sy) * map_w + sx + x) as usize;
                    let dst = (row * ow + x) as usize;
                    let explored = explored(cell_x, cell_y);
                    if let (Some(d), true) = (self.overlay.get_mut(dst), explored) {
                        let px = self.map.get(src).copied().unwrap_or(0);
                        if *d != px {
                            *d = px;
                            changed = true;
                        }
                        let (xi, ri) = (x as i16, row as i16);
                        x0 = x0.min(xi);
                        x1 = x1.max(xi);
                        y0 = y0.min(ri);
                        y1 = y1.max(ri);
                    } else if let Some(d) = self.overlay.get_mut(dst)
                        && *d != 0
                    {
                        *d = 0;
                        changed = true;
                    }
                    acc_x -= step_x;
                }
                acc_y += step_y;
            }
        }
        let (w, h) = (self.overlay_w as usize, self.overlay_h as usize);
        if w >= 8 && h >= 8 {
            for (k, &n) in CORNER_ROWS.iter().enumerate() {
                for (row, left) in [(k, 0), (h - 1 - k, 0), (k, w - n), (h - 1 - k, w - n)] {
                    for x in left..(left + n).min(w) {
                        if let Some(p) = self.overlay.get_mut(row * w + x)
                            && *p != 0
                        {
                            *p = 0;
                            changed = true;
                        }
                    }
                }
            }
        }
        if changed {
            self.overlay_rev += 1;
        }
        self.offset = centring_offset(copy_w, copy_h, (x0, x1), (y0, y1));
    }

    #[allow(clippy::too_many_arguments)]
    pub fn update(
        &mut self,
        tick: i32,
        local: i32,
        frame_w: i32,
        frame_h: i32,
        monsters: &[BlipMonster],
        rules: &BlipRules,
        vis: &Visibility,
    ) {
        let explored = |cell_x: i32, cell_y: i32| vis.is_explored(cell_x, cell_y);
        self.update_with(
            tick,
            local,
            (frame_w, frame_h),
            MapSight {
                center: vis.center,
                explored: &explored,
            },
            |m| m.update_blips(monsters, rules, vis),
        );
    }

    pub fn update_with(
        &mut self,
        tick: i32,
        local: i32,
        (frame_w, frame_h): (i32, i32),
        sight: MapSight<'_>,
        blips: impl FnOnce(&mut Self),
    ) -> bool {
        if !self.drawn() {
            return false;
        }
        let fit = ((frame_w - 10) as f32 / self.overlay_w as f32)
            .min((frame_h - 10) as f32 / self.overlay_h as f32);
        let z = if 0.5 > self.zoom { 0.5 } else { self.zoom };
        self.set_zoom(fit.min(z));
        if tick > self.blip_tick {
            blips(self);
            self.blips.sort_by_key(|b| b.1);
            self.blip_tick = tick;
        }
        let pos = (
            (self.offset.0 as f32 * self.zoom) as i32 as i16 as i32,
            (self.offset.1 as f32 * self.zoom) as i32 as i16 as i32,
        );
        let rebuilt =
            self.stamp_local == 0 || tick - self.stamp_tick > 15 || local - self.stamp_local > 15;
        if rebuilt {
            if local - self.stamp_local > 15 {
                self.stamp_local = local;
            }
            self.stamp_tick = tick;
            self.rebuild_with(sight.center, sight.explored);
        }
        let (dw, dh) = if self.explored_only {
            (
                (self.window.0 as f32 * self.zoom) as i32,
                (self.window.1 as f32 * self.zoom) as i32,
            )
        } else {
            self.scaled
        };
        let left = pos.0 + frame_w - 5 - dw;
        let top = pos.1 + 5 + self.view_drop;
        self.rect = [top, left, top + dh, left + dw];
        rebuilt
    }

    pub fn redraw(&mut self) {
        self.stamp_local = 0;
    }

    pub fn draws(&self, view: &OutlineView, frame_origin: (i32, i32)) -> Option<MinimapFrame> {
        self.draws_with(Some(view), frame_origin, OUTLINE_COLOR)
    }

    pub fn draws_with(
        &self,
        view: Option<&OutlineView>,
        frame_origin: (i32, i32),
        outline_color: [u8; 3],
    ) -> Option<MinimapFrame> {
        if !self.drawn() {
            return None;
        }
        let [top, left, bottom, right] = self.rect;
        let (dw, dh) = (right - left, bottom - top);
        let fill_alpha = mandrake_render::overlay::transparency_alpha(self.transparency.1);
        let mut fills = Vec::new();
        for &(_, t, p) in &self.blips {
            let Some((x, y)) = self.blip_point(p) else {
                continue;
            };
            let Some(color) = self.table().color(t as usize) else {
                continue;
            };
            fills.push(Fill {
                rect: [
                    frame_origin.0 + left + x - 1,
                    frame_origin.1 + top + y - 1,
                    2,
                    2,
                ],
                color,
            });
        }
        for (a, b) in view.map_or_else(Vec::new, |v| self.outline_segments(v)) {
            for r in gl_line(a, b) {
                fills.push(Fill {
                    rect: [frame_origin.0 + r[0], frame_origin.1 + r[1], r[2], r[3]],
                    color: outline_color,
                });
            }
        }
        Some(MinimapFrame {
            dst: [frame_origin.0 + left, frame_origin.1 + top, dw, dh],
            map_alpha: mandrake_render::overlay::transparency_alpha(self.transparency.0),
            fills,
            fill_alpha,
        })
    }

    fn update_blips(&mut self, monsters: &[BlipMonster], rules: &BlipRules, vis: &Visibility) {
        let visible = |x, y| vis.is_visible(x, y);
        self.blips = monsters
            .iter()
            .filter_map(|m| {
                let t = blip_type(m, rules, &visible);
                (t != -1).then_some((m.object, t, m.position))
            })
            .collect();
    }

    fn blip_point(&self, p: (i32, i32)) -> Option<(i32, i32)> {
        let (x, y) = self.map_point(p);
        let w = self.rect[3] - self.rect[1];
        let h = self.rect[2] - self.rect[0];
        ((x as u32) < w as u32 && (y as u32) < h as u32).then_some((x, y))
    }

    fn map_point(&self, p: (i32, i32)) -> (i32, i32) {
        let k = 1.0f32 / 512.0;
        let fx = (p.0 as f32 * k) / self.mesh.0 as f32;
        let fx = (1.0f32 - fx) * self.map_w as f32 - self.scroll.0 as f32;
        let fy = (p.1 as f32 * k) / self.mesh.1 as f32;
        let fy = fy * self.map_h as f32 - self.scroll.1 as f32;
        (
            (fx * self.zoom) as i32 as i16 as i32,
            (fy * self.zoom) as i32 as i16 as i32,
        )
    }

    fn frame_point(&self, p: (i32, i32)) -> (i32, i32) {
        let (x, y) = self.map_point(p);
        (
            (x + self.rect[1]) as i16 as i32,
            (y + self.rect[0]) as i16 as i32,
        )
    }

    pub fn outline_segments(&self, view: &OutlineView) -> Vec<((i32, i32), (i32, i32))> {
        let a = self.frame_point(view.from);
        let b = self.frame_point(view.to);
        let mut out = vec![(a, b)];
        let dx = (view.to.0 - view.from.0) as f32;
        let dy = (view.to.1 - view.from.1) as f32;
        let len = ((dx * dx + dy * dy) as f64).sqrt();
        let k = (2560.0f64 / len) as f32;
        let t = 0.577_307_7f32;
        let (bx, by) = (view.to.0 as f32, view.to.1 as f32);
        let p1 = (
            (bx - (dx + -t * dy) * k) as i32,
            (by - (dy + t * dx) * k) as i32,
        );
        let p2 = (
            (bx - (dx - -t * dy) * k) as i32,
            (by - (dy - t * dx) * k) as i32,
        );
        out.push((self.frame_point(p1), b));
        out.push((self.frame_point(p2), b));
        let n = view.polygon.len();
        let pts: Vec<(i32, i32)> = view
            .polygon
            .iter()
            .map(|q| self.frame_point((q[0], q[1])))
            .collect();
        for i in 0..n {
            out.push((pts[i], pts[(i + 1) % n]));
        }
        out
    }

    pub fn click(&mut self, px: i32, py: i32, pressed: bool) -> Option<(i32, i32)> {
        if !self.drawn() {
            return None;
        }
        let [top, left, bottom, right] = self.rect;
        let inside = px >= left && px < right && py >= top && py < bottom;
        if pressed {
            self.press = 0;
        }
        let on_map = inside && {
            if self.explored_only {
                let (mx, my) = self.overlay_pixel(px, py);
                mx >= 0
                    && my >= 0
                    && mx < self.overlay_w
                    && my < self.overlay_h
                    && self.overlay[(my * self.overlay_w + mx) as usize] != 0
            } else {
                true
            }
        };
        match self.press {
            0 => self.press = if on_map { 2 } else { 1 },
            2 if !on_map && self.explored_only => return None,
            1 => return None,
            _ => {}
        }
        if self.press != 2 || !on_map {
            return None;
        }
        Some(self.world_point(px, py))
    }

    pub fn overlay_pixel(&self, px: i32, py: i32) -> (i32, i32) {
        let [top, left, _, _] = self.rect;
        (
            (((px - left) * self.window.0) as f32 / (self.window.0 as f32 * self.zoom)) as i32,
            (((py - top) * self.window.1) as f32 / (self.window.1 as f32 * self.zoom)) as i32,
        )
    }

    pub fn release(&mut self) {
        self.press = 0;
    }

    pub fn takes(&self, px: i32, py: i32) -> bool {
        let [top, left, bottom, right] = self.rect;
        self.drawn() && px >= left && px < right && py >= top && py < bottom
    }

    pub fn world_point(&self, px: i32, py: i32) -> (i32, i32) {
        let sx = (self.scroll.0 as f32 * self.zoom) as i32;
        let sy = (self.scroll.1 as f32 * self.zoom) as i32;
        let (w, h) = (self.mesh.0 << 9, self.mesh.1 << 9);
        let x = (w - 1) - w.wrapping_mul(sx + px - self.rect[1]) / self.scaled.0.max(1);
        let y = h.wrapping_mul(sy + py - self.rect[0]) / self.scaled.1.max(1);
        (x, y)
    }
}

pub fn gl_line(a: (i32, i32), b: (i32, i32)) -> Vec<[i32; 4]> {
    let (a, b) = if a.1 > b.1 { (b, a) } else { (a, b) };
    let (dx, dy) = (b.0 - a.0, b.1 - a.1);
    if dx == 0 && dy == 0 {
        return Vec::new();
    }
    let mut runs: Vec<[i32; 4]> = Vec::new();
    if dx.abs() >= dy.abs() {
        let (lo, hi) = (a.0.min(b.0), a.0.max(b.0));
        for i in lo..hi {
            let y =
                (a.1 as f64 + (i as f64 + 0.5 - a.0 as f64) * dy as f64 / dx as f64).floor() as i32;
            if let Some(last) = runs.last_mut()
                && last[1] == y
                && last[0] + last[2] == i
            {
                last[2] += 1;
            } else {
                runs.push([i, y, 1, 1]);
            }
        }
    } else {
        for j in a.1..b.1 {
            let x =
                (a.0 as f64 + (j as f64 + 0.5 - a.1 as f64) * dx as f64 / dy as f64).floor() as i32;
            if let Some(last) = runs.last_mut()
                && last[0] == x
                && last[1] + last[3] == j
            {
                last[3] += 1;
            } else {
                runs.push([x, j, 1, 1]);
            }
        }
    }
    runs
}

pub struct Minimap {
    pub map: OverheadMap,
    pub vis: Visibility,
    last_local: i32,
    pub dragging: bool,
}

impl Minimap {
    pub fn load(
        game_dir: &std::path::Path,
        plugins: &[&str],
        mesh_tag: &mandrake_tag::MeshTag,
        mesh_flags: u32,
        start: &MinimapStart,
        screen: (i32, i32),
    ) -> Option<Self> {
        let tags = mandrake_tag::TagLookup::open(&mandrake_tag::engine_interface_tag_files(
            game_dir, plugins,
        ));
        let id = mesh_tag.overhead_tag?;
        let raw = tags.get(mandrake_common::FourCC(*b".256"), id)?;
        let art = InterfaceArt::from_bytes(raw)?;
        let mesh = start.mesh_cells;
        let map = OverheadMap::new(
            &art,
            mesh,
            mesh_flags,
            screen,
            &start.team_colors,
            start.team_mode,
        )?;
        let mut mm = Self {
            map,
            vis: Visibility::new(mesh.0, mesh.1),
            last_local: 1,
            dragging: false,
        };
        let view = &start.view;
        mm.vis
            .build_initial(&view.visibility, view.local_team, view.game_options);
        Some(mm)
    }

    pub fn before_tick(&mut self, tick: u32, view: Option<&MinimapView>) {
        let Some(view) = view else {
            return;
        };
        self.vis.update(
            tick as i32,
            &view.visibility,
            view.local_team,
            view.game_options,
        );
    }

    pub fn set_visible(&mut self, shown: bool, local: i32) {
        self.step(local);
        self.map.set_visible(shown);
    }

    fn step(&mut self, local: i32) {
        let elapsed = local - self.last_local;
        if elapsed > 0 {
            self.map.step_fade(elapsed);
            self.last_local = local;
        }
    }

    pub fn update(&mut self, world: &WorldView, local: i32, frame: (i32, i32)) {
        self.step(local);
        let Some(view) = world.minimap.as_ref() else {
            return;
        };
        let rules = BlipRules {
            team_mode: self.map.team_mode,
            ..view.rules
        };
        self.map.update(
            world.tick as i32,
            local,
            frame.0,
            frame.1,
            &world.blips,
            &rules,
            &self.vis,
        );
        static MINIMAP_DUMP_DIR: std::sync::OnceLock<Option<std::path::PathBuf>> =
            std::sync::OnceLock::new();
        if let Some(dir) = MINIMAP_DUMP_DIR
            .get_or_init(|| std::env::var_os("MYTH2_MINIMAP_DUMP").map(std::path::PathBuf::from))
            .as_deref()
        {
            let t = world.tick;
            let words = |v: &[u32]| v.iter().flat_map(|w| w.to_le_bytes()).collect::<Vec<u8>>();
            let _ = std::fs::write(dir.join(format!("overlay.t{t}.bin")), &self.map.overlay);
            let _ = std::fs::write(
                dir.join(format!("visible.t{t}.bin")),
                words(&self.vis.visible),
            );
            let _ = std::fs::write(
                dir.join(format!("explored.t{t}.bin")),
                words(&self.vis.explored),
            );
            let blips: Vec<String> = self
                .map
                .blips
                .iter()
                .map(|(o, ty, p)| format!("[{o} {ty} {},{}]", p.0, p.1))
                .collect();
            let _ = std::fs::write(
                dir.join(format!("state.t{t}.txt")),
                format!(
                    "state {:?} count {} transp {:#x}/{:#x} off {:?} scroll {:?} zoom {} vcentre {:?} rect {:?} blips {} {}\n",
                    self.map.state,
                    self.map.count,
                    self.map.transparency.0,
                    self.map.transparency.1,
                    self.map.offset,
                    self.map.scroll,
                    self.map.zoom,
                    self.vis.center,
                    self.map.rect,
                    blips.len(),
                    blips.join(" ")
                ),
            );
        }
    }
}

impl OverheadMap {
    pub fn overlay_rgba(&self) -> Vec<u8> {
        let table = &self.table().shading;
        let mut out = Vec::with_capacity(self.overlay.len() * 4);
        for &i in &self.overlay {
            if i == 0 {
                out.extend_from_slice(&[0, 0, 0, 0]);
            } else {
                let c = table.get(i as usize).copied().unwrap_or(0);
                out.extend_from_slice(&[(c >> 16) as u8, (c >> 8) as u8, c as u8, 0xff]);
            }
        }
        out
    }
}

#[derive(Default)]
pub struct MinimapImages {
    map: Option<mandrake_render::OverlayImage>,
    white: Option<mandrake_render::OverlayImage>,
    uploaded: Option<(u64, bool, u32, u32)>,
}

impl MinimapImages {
    pub fn update(
        &mut self,
        renderer: &mandrake_render::OverlayRenderer,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        map: &OverheadMap,
    ) {
        if !map.drawn() {
            return;
        }
        let (w, h) = (map.overlay_w as u32, map.overlay_h as u32);
        let key = (map.overlay_rev, map.team_mode, w, h);
        if self.uploaded != Some(key) {
            let rgba = map.overlay_rgba();
            match &self.map {
                Some(img) if img.width == w && img.height == h => {
                    mandrake_render::OverlayRenderer::update_image(queue, img, &rgba)
                }
                _ => {
                    self.map =
                        Some(renderer.create_image_filtered(device, queue, w, h, &rgba, true))
                }
            }
            self.uploaded = Some(key);
        }
        if self.white.is_none() {
            self.white = Some(renderer.create_image(device, queue, 1, 1, &[0xff; 4]));
        }
    }

    pub fn draws<'a>(&'a self, frame: &MinimapFrame) -> Vec<mandrake_render::OverlayDraw<'a>> {
        use mandrake_render::{OverlayDraw, OverlayLayer};
        let layer = OverlayLayer::PostRasterization(3);
        let mut out = Vec::new();
        if let Some(img) = &self.map {
            out.push(OverlayDraw {
                layer,
                image: img,
                src: [0, 0, img.width as i32, img.height as i32],
                dst: frame.dst,
                alpha: frame.map_alpha,
                tint: [0xff; 3],
            });
        }
        if let Some(white) = &self.white {
            for f in &frame.fills {
                out.push(OverlayDraw::fill(
                    layer,
                    white,
                    f.rect,
                    f.color,
                    frame.fill_alpha,
                ));
            }
        }
        out
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::world_view::myth2::game_blip_rules;

    fn pre_myth_951_isqrt(v: i32) -> i32 {
        if v <= 0 {
            return 0;
        }
        let mut r = (v as f64).sqrt() as i32;
        while r * r > v {
            r -= 1;
        }
        while (r + 1) * (r + 1) <= v {
            r += 1;
        }
        let rem = v - r * r;
        if r < rem { r + 1 } else { r }
    }

    #[test]
    fn isqrt_matches_the_old_copy() {
        const TOP: i64 = 46340 * 46340;
        for k in 0i64..46340 {
            for v in [k * k, k * k + k, k * k + k + 1]
                .into_iter()
                .flat_map(|v| [v - 1, v, v + 1])
                .filter(|&v| v < TOP)
            {
                let v = v as i32;
                assert_eq!(isqrt(v), pre_myth_951_isqrt(v), "isqrt({v})");
            }
        }
        for v in [i32::MIN, -1_000_000, -1, 0] {
            assert_eq!(isqrt(v), pre_myth_951_isqrt(v), "isqrt({v})");
        }
    }

    #[test]
    #[ignore = "exhaustive, 2^31 inputs"]
    fn isqrt_matches_the_old_copy_everywhere() {
        const TOP: i32 = 46340 * 46340;
        let threads = std::thread::available_parallelism().map_or(8, |n| n.get()) as i32;
        std::thread::scope(|s| {
            for t in 0..threads {
                s.spawn(move || {
                    for v in (t..TOP).step_by(threads as usize) {
                        assert_eq!(isqrt(v), pre_myth_951_isqrt(v), "isqrt({v})");
                    }
                });
            }
        });
    }

    #[test]
    fn engine_tables_are_the_sims() {
        let t = mandrake_sim::engine_move::trig_tables();
        assert_eq!(t.sine, mandrake_math::engine::SINE);
        assert_eq!(t.cosine, mandrake_math::engine::COSINE);
        assert_eq!(t.tangent, mandrake_math::engine::TANGENT);
        let grid = (-200..=200).flat_map(|x| (-200..=200).map(move |y| (x * 37, y * 41)));
        for (x, y) in grid.chain([(12345, -678), (-40000, -1), (i32::MIN, i32::MAX)]) {
            assert_eq!(
                mandrake_sim::engine_move::arctangent(x, y),
                mandrake_math::engine::arctangent(x, y)
            );
            assert_eq!(
                mandrake_sim::engine_move::distance2d(x, y),
                mandrake_math::engine::distance2d(x, y)
            );
        }
    }

    #[test]
    fn blips_read_the_alliance_matrix() {
        use mandrake_sim::engine_rng::EngineWorld;
        let new_world = || {
            let mut w = EngineWorld::default();
            w.players.teams = vec![Default::default(); 3];
            w.game_options = mandrake_common::game_options::ALLIANCES;
            w
        };
        let m = BlipMonster {
            used: true,
            team: 1,
            owner: -1,
            ..Default::default()
        };
        let all = |_, _| true;
        let blip = |w: &EngineWorld| {
            let rules = BlipRules {
                game_options: 0x4000,
                local_team: 0,
                ..game_blip_rules(w)
            };
            blip_type(&m, &rules, &all)
        };
        let mut w = new_world();
        assert_eq!(blip(&w), 5);
        w.ally_with_team(1, 0, true);
        assert_eq!(blip(&w), 1, "team 1 takes the local team for an ally");
        let mut w = new_world();
        w.ally_with_team(0, 1, true);
        assert_eq!(blip(&w), 1, "the local team takes team 1 for an ally");
        w.ally_with_team(0, 1, false);
        assert_eq!(blip(&w), 5);
    }

    #[test]
    fn stencils_have_the_engine_radii() {
        let s = build_stencil(20, 22);
        assert_eq!(s.len(), 45);
        let mid = &s[22];
        assert_eq!((mid.x_offset, mid.count), (0, 45));
        let cell = |r: &StencilRow, i: i32| (r.words[(i / 16) as usize] >> (2 * (i % 16))) & 3;
        assert_eq!(cell(mid, 0), 1);
        assert_eq!(cell(mid, 1), 1);
        assert_eq!(cell(mid, 2), 3);
        assert_eq!(cell(mid, 22), 3);
        assert_eq!(cell(mid, 44), 1);
        assert_eq!(isqrt(45), 7);
        assert_eq!(isqrt(672), 26);
        assert_eq!(isqrt(42), 6);
        assert_eq!((s[0].x_offset, s[0].count), (15, 15));
        assert!((0..15).all(|i| cell(&s[0], i) == 1));
    }

    #[test]
    fn visibility_rasterises_and_swaps() {
        let mut v = Visibility::new(192, 192);
        let m = VisibilityMonster {
            used: true,
            stencil: 2,
            team: 0,
            position: (100 << 9, 50 << 9),
            ..Default::default()
        };
        v.build_initial(&[m], 0, 4);
        assert!(v.is_visible(100, 50));
        assert!(v.is_visible(112, 50));
        assert!(!v.is_visible(113, 50));
        assert!(v.is_explored(88, 50));
        assert_eq!(v.center, (100, 50));
        let mut w = Visibility::new(192, 192);
        w.build_initial(&[VisibilityMonster { team: 1, ..m }], 0, 4);
        assert!(!w.is_visible(100, 50));
    }

    #[test]
    fn centring_offset_on_the_le3e_box() {
        assert_eq!(centring_offset(154, 154, (105, 153), (36, 89)), (-52, 14));
    }

    #[test]
    fn fade_steps_match_the_engine() {
        assert_eq!(fade_transparency(MapState::FadingIn, 5), (0xc000, 0xaaaa));
        assert_eq!(fade_transparency(MapState::FadingIn, 6), (0xb333, 0x9999));
        assert_eq!(fade_transparency(MapState::Shown, 20), (0x4000, 0));
        assert_eq!(fade_transparency(MapState::FadingIn, 0), (0xffff, 0xffff));
        assert_eq!(mandrake_render::overlay::transparency_alpha(0x4000), 0xbf);
    }

    #[test]
    fn map_window_by_screen_size() {
        assert_eq!(map_window(1024, 768), (256, 256));
        assert_eq!(map_window(3840, 2160), (512, 512));
        assert_eq!(map_window(640, 480), (128, 128));
    }

    #[test]
    fn gl_line_columns() {
        let px = gl_line((0, 0), (4, 2));
        assert_eq!(px, vec![[0, 0, 2, 1], [2, 1, 2, 1]]);
        let v = gl_line((5, 3), (5, 0));
        assert_eq!(v, vec![[5, 0, 1, 3]]);
    }

    #[test]
    fn rebuild_handles_tiny_map_without_underflow_or_div_zero() {
        let tables = [
            BlipTable {
                shading: vec![0; 256],
                index: [0; 20],
            },
            BlipTable {
                shading: vec![0; 256],
                index: [0; 20],
            },
        ];
        let mut tiny = OverheadMap::from_map(
            vec![1; 4],
            2,
            2,
            tables.clone(),
            (0, 0),
            MESH_FLAG_EXPLORED_MAP,
            (800, 600),
            false,
        );
        tiny.rebuild_with((0, 0), &|_, _| true);
        let mut small = OverheadMap::from_map(
            vec![1; 9],
            3,
            3,
            tables,
            (16, 16),
            MESH_FLAG_EXPLORED_MAP,
            (800, 600),
            false,
        );
        small.rebuild_with((8, 8), &|_, _| true);
        assert_eq!(small.overlay_rev, 1);
        small.rebuild_with((8, 8), &|_, _| true);
        assert_eq!(small.overlay_rev, 1);
    }

    fn blip_rules() -> BlipRules {
        BlipRules {
            mode: 2,
            game_type: 13,
            game_options: 0x200c_40fc,
            local_team: 0,
            local_player: 0,
            team_mode: false,
            solo_replay: false,
            allied_selectable: false,
            alliances: Default::default(),
        }
    }

    #[test]
    fn blip_types_of_the_le3e_capture() {
        let rules = blip_rules();
        let own = BlipMonster {
            used: true,
            team: 0,
            owner: 0,
            ..Default::default()
        };
        let none = |_, _| false;
        let all = |_, _| true;
        assert_eq!(blip_type(&own, &rules, &all), 4);
        assert_eq!(blip_type(&own, &rules, &none), -1);
        assert_eq!(blip_type(&BlipMonster { owner: 1, ..own }, &rules, &all), 1);
        let enemy = BlipMonster { team: 1, ..own };
        assert_eq!(blip_type(&enemy, &rules, &none), -1);
        assert_eq!(blip_type(&enemy, &rules, &|_, _| true), 5);
        let deer = BlipMonster {
            team: 2,
            class: 5,
            definition_flags: 0x1_0800,
            ..own
        };
        assert_eq!(blip_type(&deer, &rules, &|_, _| true), -1);
    }

    #[test]
    fn world_point_is_x_mirrored() {
        let mut m = OverheadMap {
            map: vec![1; 150 * 150],
            map_w: 150,
            map_h: 150,
            window: (256, 256),
            explored_only: false,
            tables: [
                BlipTable {
                    shading: vec![0; 256],
                    index: [0xff; 20],
                },
                BlipTable {
                    shading: vec![0; 256],
                    index: [0xff; 20],
                },
            ],
            team_mode: false,
            overlay: vec![1; 150 * 150],
            overlay_w: 150,
            overlay_h: 150,
            mesh: (96, 96),
            state: MapState::Shown,
            count: 20,
            transparency: (0x4000, 0),
            zoom: 1.0,
            screen: (1024, 768),
            relative_zoom: 1.0,
            offset: (0, 0),
            scroll: (0, 0),
            stamp_local: 0,
            stamp_tick: 0,
            rect: [5, 865, 159, 1019],
            view_drop: 0,
            scaled: (0, 0),
            blips: Vec::new(),
            blip_tick: -1,
            press: 0,
            overlay_rev: 0,
        };
        m.set_zoom(1.026_667);
        assert_eq!(m.scaled, (154, 154));
        assert_eq!(m.world_point(865, 5), (96 * 512 - 1, 0));
        let (x, y) = m.world_point(865 + 77, 5 + 77);
        assert_eq!(
            (x, y),
            (96 * 512 - 1 - 96 * 512 * 77 / 154, 96 * 512 * 77 / 154)
        );
        assert_eq!(m.frame_point((26029, 37809)), (937, 123));
        assert!(m.click(940, 100, true).is_some());
    }

    #[test]
    fn local_object_visibility_is_the_engine_rule() {
        let v = |flags, kind, bits| {
            let o = local_object_visibility(flags, kind, bits, 13, 0x726);
            (o.drawn, o.full)
        };
        const LIMITED: u32 = 0x400_0000;
        assert_eq!(v(0x200, 2, 3), (false, false));
        assert_eq!(v(0x2000_0000, 2, 3), (false, false));
        assert_eq!(v(0x8400410, 2, 0), (true, true));
        assert_eq!(v(LIMITED, 2, 0), (false, false));
        assert_eq!(v(LIMITED, 2, 1), (true, false));
        assert_eq!(v(LIMITED, 2, 3), (true, true));
        assert_eq!(v(LIMITED | 0x1000_0000, 2, 3), (true, false));
        assert_eq!(v(LIMITED, 2, 2), (true, true));
        assert_eq!(v(LIMITED | 0x1000_0000, 2, 2), (false, false));
        assert_eq!(v(LIMITED, 6, 0), (true, true));
        let t3 = |version| local_object_visibility(LIMITED, 6, 0, 3, version).drawn;
        assert!(t3(0x5e6) && !t3(0x5e7));
    }
}
