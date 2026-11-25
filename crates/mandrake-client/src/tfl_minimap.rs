use mandrake_tag::Rect;
use mandrake_tag::font::Bitmap32;
use mandrake_view::interface_bitmap::InterfaceArt;

use crate::minimap::OverheadMap;
use crate::minimap::{BlipMonster, BlipTable, MapSight, MapState, MinimapFrame, OutlineView};
use crate::world_view::{ViewSight, WorldView};

const MAX_MAP_SIZE: i32 = 0x180;
pub(crate) const MESH_FLAG_LARGE_MAP: u32 = 0x8_0000;
const REBUILD_TICKS: u32 = 15;
const BAR_CLEARANCE: i32 = 8;
pub(crate) const OUTLINE_COLOR: usize = 6;
const NET_BLIP_COLOR_BASE: usize = 6;
const SOLO_EXTRA_COLOR: usize = 10;
const NET_BLIP_COLORS: usize = 6;
const WINDOW_SCREEN: (i32, i32) = (640, 480);

const MONSTER_FLAG_10000000: u32 = 0x1000_0000;
const OBJECT_HIDDEN: u32 = 0x800_0000;
const OBJECT_FLAG_80: u32 = 0x80;

pub(crate) const SKETCH_POINTS: usize = 0xc8;
const SKETCH_COLOR: usize = 5;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct TflBlipRules {
    pub(crate) solo: bool,
    pub(crate) local_team: i16,
    pub(crate) local_player: Option<i8>,
    pub(crate) fog: bool,
    pub(crate) observer: bool,
}

impl TflBlipRules {
    pub(crate) fn as_viewer(self, player: i16, team: Option<i16>) -> Self {
        Self {
            local_team: team.unwrap_or(-1),
            local_player: Some(player as i8),
            observer: false,
            ..self
        }
    }
}

pub(crate) fn blip_color(m: &BlipMonster, rules: &TflBlipRules) -> Option<usize> {
    let hidden = m.object_flags & (OBJECT_FLAG_80 | OBJECT_HIDDEN) != 0;
    if rules.observer {
        let player = usize::try_from(m.owner as i8).ok();
        return player.filter(|_| !hidden).map(|p| NET_BLIP_COLORS + p);
    }
    if rules.solo {
        let team = m.team as i8;
        return (team >= 0).then_some(team as usize);
    }
    if m.team as i8 != rules.local_team as i8 {
        return (!hidden).then_some(2);
    }
    let own = Some(m.owner as i8) == rules.local_player;
    Some(if !own || m.flag_7_10 { 3 } else { 1 })
}

pub(crate) fn tfl_blip(
    index: usize,
    (x, y): (i32, i32),
    team: i16,
    player: i8,
    flags: u32,
    object_flags: u32,
) -> BlipMonster {
    BlipMonster {
        used: true,
        flag_7_10: flags & MONSTER_FLAG_10000000 != 0,
        team,
        owner: i16::from(player),
        object: index as i32,
        object_flags,
        position: (x, y),
        ..BlipMonster::default()
    }
}

pub(crate) fn blip_colors(
    rules: &TflBlipRules,
    team_colors: &[[u8; 3]],
    player_colors: &[[u8; 3]],
    interface: &dyn Fn(usize) -> [u8; 3],
) -> Vec<[u8; 3]> {
    if rules.solo {
        let mut c = team_colors.to_vec();
        c.push(interface(SOLO_EXTRA_COLOR));
        c
    } else {
        let mut c: Vec<_> = (0..NET_BLIP_COLORS)
            .map(|k| interface(NET_BLIP_COLOR_BASE + k))
            .collect();
        if rules.observer {
            c.extend_from_slice(player_colors);
        }
        c
    }
}

pub(crate) fn view_outline(
    camera: &mandrake_view::camera::Camera,
    size: (u32, u32),
    cells: (u32, u32),
    ground: &[mandrake_tag::tfl::TflCell],
) -> OutlineView {
    let view = crate::region_view::region_view_in(
        camera,
        crate::tfl_view::world_frame(cells),
        size,
        (0, 0),
    );
    let region = crate::tfl_view::tfl_region(camera, size, cells, ground);
    let (mut sum, mut n) = (0i64, 0i64);
    for row in region.first_row..=region.last_row {
        let Some(&(first, last)) = region.spans.get((row - region.first_row) as usize) else {
            continue;
        };
        if first >= last || row < 0 || row >= cells.1 as i32 {
            continue;
        }
        for col in i32::from(first).max(0)..=i32::from(last).min(cells.0 as i32 - 1) {
            let Some(c) = ground.get(row as usize * cells.0 as usize + col as usize) else {
                continue;
            };
            let z = i32::from(c.physical_height);
            sum += i64::from(if c.flags & 1 != 0 { z.max(0) } else { z });
            n += 1;
        }
    }
    let h = if n == 0 { 0 } else { (sum / n) as i32 };
    let corners: Vec<[i32; 2]> = (0..4)
        .map(|i| view.ground_point((i & 1) as f32, (i >> 1) as f32, h))
        .collect();
    let to = view.ground_point(0.5, 0.25, h);
    OutlineView {
        from: (view.eye[0] as i32, view.eye[1] as i32),
        to: (to[0], to[1]),
        polygon: mandrake_view::view_region::convex_hull(&corners),
    }
}

pub(crate) struct TflOverheadMap {
    map: OverheadMap,
    slots: Vec<u8>,
    cells: (i32, i32),
    sight: ViewSight,
    tick: u32,
    drawn_at: Option<u32>,
    revision: u64,
    held: bool,
    sketch: Vec<u16>,
    local: LocalSketch,
}

#[derive(Debug, Clone, Default)]
pub(crate) struct LocalSketch {
    pub(crate) mode: bool,
    pending: Vec<u16>,
    taken_at: i32,
    pub(crate) outgoing: Vec<(u16, Vec<u16>)>,
}

const PENDING_POINTS: usize = 0x13;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum MapPoll {
    Centre((i32, i32)),
    Sketch,
}

fn to_map((x, y): (i32, i32)) -> (i32, i32) {
    (x >> 1, y >> 1)
}

impl TflOverheadMap {
    pub(crate) fn new(
        art: &InterfaceArt,
        cells: (i32, i32),
        mesh_flags: u32,
        blips: &[[u8; 3]],
    ) -> Option<Self> {
        let (bitmap, _) = art.bitmap(0, 0, 0)?;
        let (width, height) = (bitmap.width, bitmap.height);
        if width <= 0 || height <= 0 || width > MAX_MAP_SIZE || height > MAX_MAP_SIZE {
            return None;
        }
        let pixels: Vec<u8> = bitmap
            .pixels
            .iter()
            .map(|p| p.map_or(0, |(i, _)| i))
            .collect();
        let mut shading = vec![0u32; 256];
        for (i, c) in art.palette().iter().enumerate().take(256) {
            shading[i] = u32::from(c[0]) << 16 | u32::from(c[1]) << 8 | u32::from(c[2]);
        }
        let mut used = [false; 256];
        for &p in &pixels {
            used[p as usize] = true;
        }
        let mut slots = Vec::new();
        let mut next = 255i32;
        for c in blips {
            while next >= 1 && used[next as usize] {
                next -= 1;
            }
            if next < 1 {
                break;
            }
            slots.push(next as u8);
            shading[next as usize] = u32::from(c[0]) << 16 | u32::from(c[1]) << 8 | u32::from(c[2]);
            next -= 1;
        }
        Some(Self::from_pixels(
            pixels,
            (width, height),
            shading,
            slots,
            cells,
            mesh_flags,
        ))
    }

    fn from_pixels(
        pixels: Vec<u8>,
        (width, height): (i32, i32),
        shading: Vec<u32>,
        slots: Vec<u8>,
        cells: (i32, i32),
        mesh_flags: u32,
    ) -> Self {
        let mut index = [0xff; 20];
        for (i, &s) in index.iter_mut().zip(&slots) {
            *i = s;
        }
        let table = BlipTable { shading, index };
        let flags = if mesh_flags & MESH_FLAG_LARGE_MAP != 0 {
            crate::minimap::MESH_FLAG_EXPLORED_MAP
        } else {
            0
        };
        let map = OverheadMap::from_map(
            pixels,
            width,
            height,
            [table.clone(), table],
            cells,
            flags,
            WINDOW_SCREEN,
            false,
        );
        Self {
            map,
            slots,
            cells,
            sight: ViewSight::all_seen(cells, &[]),
            tick: 0,
            drawn_at: None,
            revision: 0,
            held: false,
            sketch: Vec::new(),
            local: LocalSketch::default(),
        }
    }

    pub(crate) fn state(&self) -> u8 {
        match self.map.state {
            MapState::Hidden => 0,
            MapState::FadingIn => 1,
            MapState::Shown => 2,
            MapState::FadingOut => 3,
        }
    }

    pub(crate) fn showing(&self) -> bool {
        matches!(self.map.state, MapState::FadingIn | MapState::Shown)
    }

    pub(crate) fn drawn(&self) -> bool {
        self.map.drawn()
    }

    pub(crate) fn set_shown(&mut self, show: bool) {
        self.map.set_visible(show);
    }

    pub(crate) fn tick(&mut self) {
        if self.map.drawn() {
            self.map.step_fade(1);
        }
    }

    #[cfg(test)]
    pub(crate) fn mouse(&mut self, held: bool, cursor: (i32, i32)) -> Option<(i32, i32)> {
        match self.mouse_poll(held, cursor, 0) {
            Some(MapPoll::Centre(p)) => Some(p),
            _ => None,
        }
    }

    pub(crate) fn mouse_poll(
        &mut self,
        held: bool,
        cursor: (i32, i32),
        now: i32,
    ) -> Option<MapPoll> {
        if !self.map.drawn() {
            return None;
        }
        if !held {
            self.held = false;
            self.map.release();
            if !self.local.pending.is_empty() {
                let mut points = std::mem::take(&mut self.local.pending);
                points.push(0);
                self.send_sketch(0, points);
            }
            return None;
        }
        let pressed = !std::mem::replace(&mut self.held, true);
        let p = self.map.click(cursor.0, cursor.1, pressed)?;
        if self.local.mode {
            let (x, y) = self.map.overlay_pixel(cursor.0, cursor.1);
            self.take_point(x, y, now);
            return Some(MapPoll::Sketch);
        }
        Some(MapPoll::Centre((p.0 * 2 + 1, p.1 * 2)))
    }

    fn take_point(&mut self, x: i32, y: i32, now: i32) {
        let l = &mut self.local;
        if let Some(&last) = l.pending.last() {
            let (dx, dy) = (i32::from(last & 0xff) - x, i32::from(last >> 8) - y);
            let d = ((dx * dx + dy * dy) as f64).sqrt() as i32;
            if d < 11 && (d < 3 || now - l.taken_at <= 0) {
                return;
            }
        }
        if l.pending.len() >= PENDING_POINTS {
            return;
        }
        l.taken_at = now;
        l.pending.push((x as u16 & 0xff) | ((y as u16 & 0xff) << 8));
        if let [.., a, b] = l.pending[..] {
            self.sketch_line(a, b);
        }
    }

    fn sketch_line(&mut self, a: u16, b: u16) {
        let Some(&slot) = self.slots.get(SKETCH_COLOR) else {
            return;
        };
        let at = |w: u16| (i32::from(w & 0xff), i32::from(w >> 8));
        let size = (self.map.overlay_w, self.map.overlay_h);
        index_line(&mut self.map.overlay, size, at(a), at(b), slot);
        self.revision += 1;
    }

    fn send_sketch(&mut self, flags: u16, points: Vec<u16>) {
        self.add_sketch(&points);
        self.local.outgoing.push((flags, points));
    }

    fn send_pending(&mut self) {
        if self.local.pending.is_empty() {
            return;
        }
        let mut points = std::mem::take(&mut self.local.pending);
        let last = points[points.len() - 1];
        points.push(0);
        self.send_sketch(0, points);
        self.local.pending = vec![last];
    }

    fn after_empty_send(&mut self) {
        self.local.pending = vec![(self.local.taken_at >> 16) as u16];
    }

    pub(crate) fn set_sketch_mode(&mut self, on: bool) {
        self.local.mode = on;
        self.local.pending.clear();
        self.map.redraw();
        if on {
            self.send_sketch(1, Vec::new());
            self.after_empty_send();
        }
    }

    pub(crate) fn erase_own_sketch(&mut self) {
        self.sketch.clear();
        self.local.pending.clear();
        self.map.redraw();
        self.send_sketch(2, Vec::new());
        self.after_empty_send();
    }

    pub(crate) fn take_sketch_commands(&mut self) -> Vec<(u16, Vec<u16>)> {
        std::mem::take(&mut self.local.outgoing)
    }

    pub(crate) fn update(&mut self, view: &WorldView, rules: &TflBlipRules) -> bool {
        self.update_with(view.tick, &view.blips, view.sight.clone(), rules)
    }

    pub(crate) fn update_owned(&mut self, view: WorldView, rules: &TflBlipRules) -> bool {
        self.update_with(view.tick, &view.blips, view.sight, rules)
    }

    fn update_with(
        &mut self,
        tick: u32,
        monsters: &[crate::minimap::BlipMonster],
        sight: Option<ViewSight>,
        rules: &TflBlipRules,
    ) -> bool {
        self.tick = tick;
        let blips_sight_owned = match sight {
            Some(s) if !rules.observer => {
                self.sight = s;
                true
            }
            _ if rules.observer => {
                let all: Vec<_> = monsters.iter().map(|m| m.position).collect();
                self.sight = ViewSight::all_seen(self.cells, &all);
                false
            }
            _ => {
                let team: Vec<_> = monsters
                    .iter()
                    .filter(|m| m.team as i8 == rules.local_team as i8)
                    .map(|m| m.position)
                    .collect();
                self.sight = ViewSight::all_seen(self.cells, &team);
                false
            }
        };
        let sight = blips_sight_owned.then_some(&self.sight);
        self.map.blips = monsters
            .iter()
            .filter(|m| {
                let seen = match sight {
                    Some(s) => s.bits_at(m.position) & 1 != 0,
                    None => m.team as i8 == rules.local_team as i8,
                };
                !rules.fog || rules.observer || seen
            })
            .filter_map(|m| {
                let k = blip_color(m, rules).filter(|&k| k < self.slots.len())?;
                Some((m.object, k as i16, to_map(m.position)))
            })
            .collect();
        if self
            .drawn_at
            .is_some_and(|t| tick.wrapping_sub(t) <= REBUILD_TICKS && tick >= t)
        {
            return false;
        }
        self.send_pending();
        self.drawn_at = Some(tick);
        true
    }

    pub(crate) fn add_sketch(&mut self, points: &[u16]) {
        let mut last = self.sketch.last().copied().unwrap_or(0);
        for &p in points {
            if last != 0 && p != 0 {
                self.sketch_line(last, p);
            }
            last = p;
        }
        self.sketch.extend_from_slice(points);
        let over = self.sketch.len().saturating_sub(SKETCH_POINTS);
        self.sketch.drain(..over);
    }

    pub(crate) fn erase_sketch(&mut self) {
        self.sketch.clear();
        self.map.redraw();
    }

    pub(crate) fn sketch(&self) -> &[u16] {
        &self.sketch
    }

    fn draw_sketch(&mut self) {
        let points: Vec<u16> = self
            .sketch
            .iter()
            .chain([&0])
            .chain(&self.local.pending)
            .copied()
            .collect();
        let mut last = 0u16;
        for p in points {
            if last != 0 && p != 0 {
                self.sketch_line(last, p);
            }
            last = p;
        }
    }

    pub(crate) fn frame(&mut self, screen: (i32, i32), bar: Option<Rect>) {
        if self.map.screen != screen {
            self.map.rezoom(screen);
        }
        let sight = &self.sight;
        let explored = |col: i32, row: i32| sight.explored((col, row));
        let rebuilt = self.map.update_with(
            self.tick as i32,
            self.tick as i32,
            screen,
            MapSight {
                center: sight.centre,
                explored: &explored,
            },
            |_| {},
        );
        if rebuilt {
            self.revision += 1;
            self.draw_sketch();
        }
        if let Some(b) = bar
            && self.map.drawn()
            && b.right + BAR_CLEARANCE > self.map.rect[1]
        {
            self.map.rect[0] += b.height();
            self.map.rect[2] += b.height();
        }
    }

    pub(crate) fn rect(&self) -> Rect {
        let [top, left, bottom, right] = self.map.rect;
        Rect::new(top, left, bottom, right)
    }

    pub(crate) fn draws(
        &self,
        outline: Option<&OutlineView>,
        color: [u8; 3],
    ) -> Option<MinimapFrame> {
        let outline = outline.map(|o| OutlineView {
            from: to_map(o.from),
            to: to_map(o.to),
            polygon: o.polygon.iter().map(|p| [p[0] >> 1, p[1] >> 1]).collect(),
        });
        self.map.draws_with(outline.as_ref(), (0, 0), color)
    }

    pub(crate) fn overhead_map(&self) -> &OverheadMap {
        &self.map
    }

    pub(crate) fn revision(&self) -> u64 {
        self.revision
    }

    pub(crate) fn draw(&self, dst: &mut Bitmap32, outline: Option<&OutlineView>, color: [u8; 3]) {
        let Some(frame) = self.draws(outline, color) else {
            return;
        };
        let rgba = self.map.overlay_rgba();
        draw_quad(
            dst,
            &rgba,
            (self.map.overlay_w, self.map.overlay_h),
            frame.dst,
            frame.map_alpha,
        );
        for f in &frame.fills {
            let [x, y, w, h] = f.rect;
            let c = [f.color[0], f.color[1], f.color[2], 0xff];
            for py in y..y + h {
                for px in x..x + w {
                    crate::tfl_hud::over(
                        dst,
                        px,
                        py,
                        [c[0], c[1], c[2]],
                        u32::from(c[3]) * u32::from(frame.fill_alpha) / 255,
                    );
                }
            }
        }
    }
}

fn draw_quad(dst: &mut Bitmap32, rgba: &[u8], size: (i32, i32), rect: [i32; 4], alpha: u8) {
    let [x0, y0, w, h] = rect;
    let (sw, sh) = size;
    if w <= 0 || h <= 0 || sw <= 0 || sh <= 0 {
        return;
    }
    let texel = |x: i32, y: i32| -> [f32; 4] {
        let i = ((y.clamp(0, sh - 1) * sw + x.clamp(0, sw - 1)) * 4) as usize;
        [0, 1, 2, 3].map(|k| f32::from(rgba[i + k]))
    };
    for py in y0.max(0)..(y0 + h).min(dst.height) {
        let v = (py - y0) as f32 + 0.5;
        let fy = v * sh as f32 / h as f32 - 0.5;
        let (ty, ay) = (fy.floor() as i32, fy - fy.floor());
        for px in x0.max(0)..(x0 + w).min(dst.width) {
            let u = (px - x0) as f32 + 0.5;
            let fx = u * sw as f32 / w as f32 - 0.5;
            let (tx, ax) = (fx.floor() as i32, fx - fx.floor());
            let (a, b, c, d) = (
                texel(tx, ty),
                texel(tx + 1, ty),
                texel(tx, ty + 1),
                texel(tx + 1, ty + 1),
            );
            let s = [0, 1, 2, 3].map(|k| {
                let top = a[k] + (b[k] - a[k]) * ax;
                let bottom = c[k] + (d[k] - c[k]) * ax;
                (top + (bottom - top) * ay).round().clamp(0.0, 255.0) as u8
            });
            crate::tfl_hud::over(
                dst,
                px,
                py,
                [s[0], s[1], s[2]],
                u32::from(s[3]) * u32::from(alpha) / 255,
            );
        }
    }
}

fn index_line(dst: &mut [u8], size: (i32, i32), a: (i32, i32), b: (i32, i32), index: u8) {
    let (dx, dy) = ((b.0 - a.0).abs(), -(b.1 - a.1).abs());
    let (sx, sy) = (
        if a.0 < b.0 { 1 } else { -1 },
        if a.1 < b.1 { 1 } else { -1 },
    );
    let (mut x, mut y, mut err) = (a.0, a.1, dx + dy);
    loop {
        if x >= 0 && y >= 0 && x < size.0 && y < size.1 {
            dst[(y * size.0 + x) as usize] = index;
        }
        if x == b.0 && y == b.1 {
            break;
        }
        let e2 = 2 * err;
        if e2 >= dy {
            err += dy;
            x += sx;
        }
        if e2 <= dx {
            err += dx;
            y += sy;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn large_map() -> TflOverheadMap {
        let mut shading = vec![0x0010_2030; 256];
        shading[200] = 0x00ff_0000;
        let mut m = TflOverheadMap::from_pixels(
            vec![7u8; 256 * 200],
            (256, 200),
            shading,
            vec![200],
            (64, 50),
            MESH_FLAG_LARGE_MAP,
        );
        m.set_shown(true);
        for _ in 0..15 {
            m.tick();
        }
        assert_eq!(m.state(), 2);
        m.frame((1024, 768), None);
        m
    }

    fn screen_at(m: &TflOverheadMap, (x, y): (i32, i32)) -> (i32, i32) {
        let r = m.rect();
        let z = m.map.zoom;
        (
            r.left + (x as f32 * z).ceil() as i32,
            r.top + (y as f32 * z).ceil() as i32,
        )
    }

    #[test]
    fn a_press_on_the_map_is_the_map_s() {
        let mut m = large_map();
        let at = screen_at(&m, (64, 40));
        let p = m.mouse(true, at).expect("the map's");
        let q = m.map.world_point(at.0, at.1);
        assert_eq!(p, (q.0 * 2 + 1, q.1 * 2));
        let (sx, sy) = (m.map.scroll.0 as i32, m.map.scroll.1 as i32);
        assert_eq!((sx, sy), (64, 36));
        let px = |w: i32| 256 - w * 256 / (64 << 10);
        assert!((px(p.0) - (64 + sx)).abs() <= 1, "{p:?}");
        assert!((p.1 * 200 / (50 << 10) - (40 + sy)).abs() <= 1, "{p:?}");
        assert!(m.mouse(true, (at.0 + 3, at.1)).is_some());
        assert_eq!(m.mouse(false, at), None);
        assert_eq!(m.mouse(true, (100, 100)), None);
        assert_eq!(m.mouse(true, at), None);
        assert_eq!(m.mouse(false, at), None);
        let corner = screen_at(&m, (0, 0));
        assert_eq!(m.mouse(true, corner), None);
        assert_eq!(m.mouse(false, at), None);
        m.set_shown(false);
        for _ in 0..15 {
            m.tick();
        }
        assert_eq!(m.mouse(true, at), None);
    }

    #[test]
    fn the_map_is_myth2s_quad_sized_to_the_screen() {
        let mut m = large_map();
        assert_eq!(m.rect(), Rect::new(5, 1024 - 5 - 154, 5 + 154, 1024 - 5));
        m.frame((2048, 1536), None);
        assert_eq!(m.rect(), Rect::new(5, 2048 - 5 - 308, 5 + 308, 2048 - 5));
        let bar = Rect::new(0, 300, 64, 1900);
        m.frame((2048, 1536), Some(bar));
        assert_eq!(m.rect().top, 64 + 5);
        m.frame((2048, 1536), Some(Rect::new(0, 0, 64, 640)));
        assert_eq!(m.rect().top, 5);
        let mut whole = TflOverheadMap::from_pixels(
            vec![7u8; 154 * 128],
            (154, 128),
            vec![0; 256],
            vec![200],
            (64, 50),
            0,
        );
        whole.set_shown(true);
        whole.frame((1024, 768), None);
        assert_eq!(whole.rect(), Rect::new(5, 865, 133, 1019));
    }

    #[test]
    fn a_large_map_is_moved_to_centre_its_explored_pixels() {
        let mut m = large_map();
        assert_eq!(m.map.offset, (0, 0));
        let mut sight = ViewSight::all_seen((64, 50), &[]);
        for row in 0..50 {
            for col in 0..64 {
                sight.explored[(row * 64 + col) as usize] = (24..32).contains(&col) && row >= 30;
            }
        }
        let view = WorldView {
            tick: 20,
            sight: Some(sight.clone()),
            ..WorldView::default()
        };
        m.update(&view, &solo_rules());
        m.frame((1024, 768), None);
        m.frame((1024, 768), None);
        let seen: Vec<(i32, i32)> = (0..128 * 128)
            .filter(|i| {
                let (c, r) = (i % 128, i / 128);
                let col = ((64 << 16) - (64 + c) * ((64 << 16) / 256) - 1) >> 16;
                let row = ((36 + r) * ((50 << 16) / 200)) >> 16;
                sight.explored((col, row))
            })
            .map(|i| (i % 128, i / 128))
            .collect();
        let lo = seen
            .iter()
            .fold((i32::MAX, i32::MAX), |a, p| (a.0.min(p.0), a.1.min(p.1)));
        let hi = seen
            .iter()
            .fold((i32::MIN, i32::MIN), |a, p| (a.0.max(p.0), a.1.max(p.1)));
        let expect = (
            ((128 - hi.0 + lo.0) >> 1) - lo.0,
            ((128 - hi.1 + lo.1) >> 1) - lo.1,
        );
        assert_eq!(expect, (-16, -42));
        assert_eq!(m.map.offset, (expect.0 as i16, expect.1 as i16));
        let z = m.map.zoom;
        let r = m.rect();
        assert_eq!(
            (r.left, r.top),
            (
                (expect.0 as f32 * z) as i32 + 1024 - 5 - 154,
                (expect.1 as f32 * z) as i32 + 5
            )
        );
    }

    fn solo_rules() -> TflBlipRules {
        TflBlipRules {
            solo: true,
            local_team: 0,
            local_player: None,
            fog: false,
            observer: false,
        }
    }

    #[test]
    fn a_large_map_is_a_window_around_the_team_with_its_corners_cut() {
        let mut m = large_map();
        assert_eq!(m.map.scroll, (64, 36));
        let row = |m: &TflOverheadMap, y: usize| m.map.overlay[y * 128..][..128].to_vec();
        assert_eq!(&row(&m, 0)[..5], &[0, 0, 0, 0, 7]);
        assert_eq!(&row(&m, 0)[123..], &[7, 0, 0, 0, 0]);
        assert_eq!(&row(&m, 1)[..3], &[0, 0, 7]);
        assert_eq!(&row(&m, 127)[..5], &[0, 0, 0, 0, 7]);
        assert_eq!(row(&m, 64)[0], 7);
        let monster = tfl_blip(3, ((10 << 10) + 5, (30 << 10) + 5), 0, 0, 0, 0);
        let near = tfl_blip(4, (100, 40 << 10), 0, 0, 0, 0);
        let view = |tick, sight| WorldView {
            tick,
            blips: vec![near],
            sight,
            ..WorldView::default()
        };
        m.update(&view(20, None), &solo_rules());
        m.frame((1024, 768), None);
        assert_eq!(m.map.scroll, (128, 72));
        let mut sight = ViewSight::all_seen((64, 50), &[(100, 40 << 10)]);
        sight.explored.fill(false);
        m.update(&view(40, Some(sight.clone())), &solo_rules());
        m.frame((1024, 768), None);
        assert!(m.map.overlay.iter().all(|&p| p == 0));
        sight.explored.fill(true);
        let v = WorldView {
            tick: 41,
            blips: vec![monster],
            sight: Some(sight),
            ..WorldView::default()
        };
        m.update(&v, &solo_rules());
        assert_eq!(
            m.map.blips,
            vec![(3, 0, (monster.position.0 >> 1, monster.position.1 >> 1))]
        );
        m.frame((1024, 768), None);
        let f = m.draws(None, [0; 3]).unwrap();
        assert_eq!(f.fills.len(), 1);
        assert_eq!(f.fills[0].color, [0xff, 0, 0]);
        assert_eq!(f.fills[0].rect[2..], [2, 2]);
    }

    #[test]
    fn the_outline_is_myth2s_from_the_view() {
        let m = large_map();
        let outline = OutlineView {
            from: (32 << 10, 30 << 10),
            to: (32 << 10, 24 << 10),
            polygon: vec![
                [28 << 10, 20 << 10],
                [36 << 10, 20 << 10],
                [34 << 10, 28 << 10],
                [30 << 10, 28 << 10],
            ],
        };
        let f = m.draws(Some(&outline), [1, 2, 3]).unwrap();
        let halved = OutlineView {
            from: (16 << 10, 15 << 10),
            to: (16 << 10, 12 << 10),
            polygon: outline
                .polygon
                .iter()
                .map(|p| [p[0] >> 1, p[1] >> 1])
                .collect(),
        };
        let lines: usize = m
            .map
            .outline_segments(&halved)
            .into_iter()
            .map(|(a, b)| crate::minimap::gl_line(a, b).len())
            .sum();
        assert_eq!(f.fills.len(), lines);
        assert!(f.fills.iter().all(|x| x.color == [1, 2, 3]));
        assert_eq!(f.fill_alpha, 0xff, "shown: opaque");
        assert!(m.draws(None, [1, 2, 3]).unwrap().fills.is_empty());
    }

    #[test]
    fn the_map_fades_in_and_out_over_15_ticks() {
        let mut m =
            TflOverheadMap::from_pixels(vec![1], (1, 1), vec![0; 256], Vec::new(), (1, 1), 0);
        m.set_shown(true);
        assert_eq!(m.map.transparency.0, 0xffff);
        for _ in 0..14 {
            m.tick();
        }
        assert_eq!(m.state(), 1);
        m.tick();
        assert_eq!((m.state(), m.map.transparency.0), (2, 0x4000));
        m.set_shown(false);
        for _ in 0..7 {
            m.tick();
        }
        m.set_shown(true);
        assert_eq!((m.state(), m.map.count), (1, 7));
        for _ in 0..8 {
            m.tick();
        }
        assert_eq!(m.state(), 2);
        m.set_shown(false);
        for _ in 0..15 {
            m.tick();
        }
        assert!(!m.drawn());
    }

    #[test]
    fn a_net_game_colours_by_relation() {
        let rules = TflBlipRules {
            solo: false,
            local_team: 0,
            local_player: Some(0),
            fog: false,
            observer: false,
        };
        let m = |team, player, object_flags| tfl_blip(0, (0, 0), team, player, 0, object_flags);
        assert_eq!(blip_color(&m(0, 0, 0), &rules), Some(1));
        assert_eq!(blip_color(&m(0, 1, 0), &rules), Some(3));
        assert_eq!(
            blip_color(&tfl_blip(0, (0, 0), 0, 0, MONSTER_FLAG_10000000, 0), &rules),
            Some(3)
        );
        assert_eq!(blip_color(&m(1, 1, 0), &rules), Some(2));
        assert_eq!(blip_color(&m(1, 1, 0x80), &rules), None);
        let solo = TflBlipRules {
            solo: true,
            ..rules
        };
        assert_eq!(blip_color(&m(1, 1, 0x80), &solo), Some(1));
        assert_eq!(blip_color(&m(-1, 1, 0), &solo), None);
    }

    #[test]
    fn a_live_net_game_colours_by_the_sims_local_team() {
        let film = TflBlipRules {
            solo: false,
            local_team: 0,
            local_player: None,
            fog: false,
            observer: false,
        };
        let m = |team, player| tfl_blip(0, (0, 0), team, player, 0, 0);
        assert_eq!(blip_color(&m(0, 0), &film), Some(3));
        assert_eq!(blip_color(&m(1, 2), &film), Some(2));
        let live = film.as_viewer(2, Some(1));
        assert_eq!(live.local_player, Some(2));
        assert_eq!(blip_color(&m(1, 2), &live), Some(1));
        assert_eq!(blip_color(&m(1, 3), &live), Some(3));
        assert_eq!(blip_color(&m(0, 0), &live), Some(2));
        let teamless = film.as_viewer(2, None);
        assert_eq!(teamless.local_team, -1);
        assert_eq!(blip_color(&m(1, 2), &teamless), Some(2));
    }

    #[test]
    fn a_net_films_observer_colours_by_player() {
        let film = TflBlipRules {
            solo: false,
            local_team: 0,
            local_player: None,
            fog: true,
            observer: true,
        };
        let m = |team, player, object_flags| tfl_blip(0, (0, 0), team, player, 0, object_flags);
        assert_eq!(blip_color(&m(0, 0, 0), &film), Some(6));
        assert_eq!(blip_color(&m(1, 1, 0), &film), Some(7));
        assert_eq!(blip_color(&m(0, 3, 0), &film), Some(9));
        assert_eq!(blip_color(&m(1, 1, 0x80), &film), None);
        assert_eq!(blip_color(&m(0, -1, 0), &film), None);
        let live = film.as_viewer(1, Some(1));
        assert!(!live.observer);
        assert_eq!(blip_color(&m(1, 1, 0), &live), Some(1));
        let interface = |i: usize| [i as u8; 3];
        let colors = blip_colors(&film, &[], &[[1, 2, 3], [4, 5, 6]], &interface);
        assert_eq!(colors.len(), 8);
        assert_eq!(colors[5], [11; 3]);
        assert_eq!(colors[6..], [[1, 2, 3], [4, 5, 6]]);
        assert_eq!(blip_colors(&live, &[], &[[1, 2, 3]], &interface).len(), 6);
    }

    #[test]
    fn a_sketch_is_drawn_on_the_overhead_map() {
        let mut m = large_map();
        m.slots = vec![250, 249, 248, 247, 246, 245];
        let p = |x: u16, y: u16| x | y << 8;
        m.add_sketch(&[p(10, 20), p(13, 20), 0, p(40, 40), p(40, 42)]);
        let at =
            |m: &TflOverheadMap, x: i32, y: i32| m.map.overlay[(y * m.map.overlay_w + x) as usize];
        for x in 10..=13 {
            assert_eq!(at(&m, x, 20), 245, "the stroke at x {x}");
        }
        assert_eq!(at(&m, 40, 41), 245);
        assert_eq!(at(&m, 20, 30), 7, "no line across the pen lift");
        m.map.redraw();
        m.frame((1024, 768), None);
        assert_eq!(at(&m, 11, 20), 245);
        let many: Vec<u16> = (1..=250).collect();
        m.add_sketch(&many);
        assert_eq!(m.sketch().len(), SKETCH_POINTS);
        assert_eq!(m.sketch()[0], 51);
        m.erase_sketch();
        assert!(m.sketch().is_empty());
        m.frame((1024, 768), None);
        assert_eq!(at(&m, 11, 20), 7, "erased, the map is drawn again");
    }

    #[test]
    fn the_local_sketch_sends_command_12s() {
        let mut m = large_map();
        m.slots = vec![250, 249, 248, 247, 246, 245];
        let ww = m.map.overlay_w;
        let p = |x: i32, y: i32| (x as u16) | (y as u16) << 8;
        m.set_sketch_mode(true);
        assert_eq!(m.take_sketch_commands(), [(1, vec![])]);
        assert_eq!(m.local.pending, [0]);
        m.local.pending.clear();
        m.frame((1024, 768), None);
        let x = ww / 2;
        let at = |m: &TflOverheadMap, dx: i32| screen_at(m, (x + dx, 40));
        let c = at(&m, 0);
        assert_eq!(m.mouse_poll(true, c, 1), Some(MapPoll::Sketch));
        let c = at(&m, 2);
        m.mouse_poll(true, c, 2);
        let c = at(&m, 5);
        m.mouse_poll(true, c, 1);
        assert_eq!(m.local.pending, [p(x, 40)]);
        m.mouse_poll(true, c, 2);
        let c = at(&m, 17);
        m.mouse_poll(true, c, 2);
        assert_eq!(m.local.pending, [p(x, 40), p(x + 5, 40), p(x + 17, 40)]);
        assert_eq!(
            m.map.overlay[(40 * ww + x + 10) as usize],
            245,
            "drawn as it goes"
        );
        assert_eq!(m.mouse_poll(false, c, 3), None);
        let stroke = vec![p(x, 40), p(x + 5, 40), p(x + 17, 40), 0];
        assert_eq!(m.take_sketch_commands(), [(0, stroke.clone())]);
        assert_eq!(m.sketch(), &stroke[..]);
        let c = at(&m, 0);
        m.mouse_poll(true, c, 10);
        let c = at(&m, 20);
        m.mouse_poll(true, c, 11);
        let view = WorldView::default();
        let rules = TflBlipRules {
            solo: false,
            local_team: 0,
            local_player: Some(0),
            fog: false,
            observer: false,
        };
        m.drawn_at = None;
        assert!(m.update(&view, &rules));
        assert_eq!(
            m.take_sketch_commands(),
            [(0, vec![p(x, 40), p(x + 20, 40), 0])]
        );
        assert_eq!(m.local.pending, [p(x + 20, 40)]);
        m.erase_own_sketch();
        assert!(m.sketch().is_empty());
        assert_eq!(m.take_sketch_commands(), [(2, vec![])]);
        m.set_sketch_mode(false);
        let c = at(&m, 0);
        m.mouse_poll(false, c, 20);
        assert!(matches!(
            m.mouse_poll(true, c, 21),
            Some(MapPoll::Centre(_))
        ));
    }
}
