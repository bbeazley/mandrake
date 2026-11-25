use mandrake_tag::SequenceWorldBounds;

pub mod state;

pub const TRANSPARENCY_SELECTED: u16 = 0x5555;

pub const TRANSPARENCY_HIGHLIGHTED: u16 = 0xaaaa;

pub const WORLD_UNITS_PER_CELL: f32 = crate::WORLD_UNITS_PER_CELL;

pub const OBJECT_SCALE_ONE: i16 = 0x100;

pub const VITALITY_BAR_LEFT: i32 = 2;
pub const VITALITY_BAR_RIGHT: i32 = 8;
pub const VITALITY_BAR_HEIGHT_INSET: i32 = 6;
pub const VITALITY_BAR_CAP: i32 = 2;

pub const VITALITY_COLOR_STEPS: i32 = 0x15;

pub fn overlay_alpha(transparency: u16) -> u8 {
    ((0xffff - transparency as i32) >> 8) as u8
}

pub fn blend_display_channel(src: u8, dst: u8, alpha: u8) -> u8 {
    let a = alpha as u32;
    let v = a * src as u32 + (255 - a) * dst as u32;
    ((v + 127) / 255) as u8
}

pub fn frame_thickness(thick_preference: bool) -> i32 {
    if thick_preference { 2 } else { 1 }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PixelRect {
    pub left: i32,
    pub top: i32,
    pub right: i32,
    pub bottom: i32,
}

impl PixelRect {
    pub const fn new(left: i32, top: i32, right: i32, bottom: i32) -> Self {
        Self {
            left,
            top,
            right,
            bottom,
        }
    }

    pub fn width(&self) -> i32 {
        self.right - self.left
    }

    pub fn height(&self) -> i32 {
        self.bottom - self.top
    }

    pub fn is_empty(&self) -> bool {
        self.left >= self.right || self.top >= self.bottom
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ObjectBounds {
    pub half_width: i16,
    pub bottom: i16,
    pub top: i16,
}

impl ObjectBounds {
    pub fn from_sequence(world: SequenceWorldBounds, scale: i16) -> Self {
        let s = scale as i32;
        let f = |v: i16| ((v as i32).wrapping_mul(s) >> 8) as i16;
        Self {
            half_width: f(world.half_width),
            bottom: f(world.bottom),
            top: f(world.top),
        }
    }

    pub fn flipped(self) -> Self {
        Self {
            half_width: self.half_width,
            bottom: self.top.wrapping_neg(),
            top: self.bottom.wrapping_neg(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FrameProjection {
    pub origin: [f32; 2],
    pub pixels_per_unit: f32,
    pub vertical_pixels_per_unit: f32,
    pub viewport: [i32; 2],
}

pub fn projected_frame_rect(bounds: ObjectBounds, proj: &FrameProjection) -> PixelRect {
    let (k, ky) = (proj.pixels_per_unit, proj.vertical_pixels_per_unit);
    let [ox, oy] = proj.origin;
    let hw = bounds.half_width as f32;
    PixelRect {
        left: (ox - hw * k) as i32,
        right: (ox + hw * k) as i32,
        top: (oy - bounds.top as f32 * ky) as i32,
        bottom: (oy - bounds.bottom as f32 * ky) as i32,
    }
}

pub fn selection_frame_rect(bounds: ObjectBounds, proj: &FrameProjection) -> Option<PixelRect> {
    let rect = projected_frame_rect(bounds, proj);
    let visible = rect.left < rect.right
        && rect.right > 0
        && rect.top < rect.bottom
        && rect.left < proj.viewport[0]
        && rect.bottom > 0
        && rect.top < proj.viewport[1];
    visible.then_some(rect)
}

pub fn frame_edges(rect: PixelRect, thickness: i32) -> [PixelRect; 4] {
    let t = thickness;
    let PixelRect {
        left,
        top,
        right,
        bottom,
    } = rect;
    [
        PixelRect::new(left, top, right, top + t),
        PixelRect::new(left, bottom - t, right, bottom),
        PixelRect::new(left, top + t, left + t, bottom - t),
        PixelRect::new(right - t, top + t, right, bottom - t),
    ]
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ObjectVitality {
    pub current: i16,
    pub maximum: i16,
    pub original: i16,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BarColor {
    Fill,
    Background,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VitalityBar {
    pub step: usize,
    pub segments: [(PixelRect, BarColor); 3],
}

pub fn vitality_bar(rect: PixelRect, bounds: ObjectBounds, v: ObjectVitality) -> VitalityBar {
    let max = v.maximum as i32;
    let orig = v.original as i32;
    let f: i32 = if v.maximum != 0 {
        ((v.current as i32) << 16).wrapping_div(max)
    } else {
        0
    };
    let step = (f.wrapping_mul(VITALITY_COLOR_STEPS) >> 16).max(0);
    let d = ((bounds.top as i32 - bounds.bottom as i32) * 2) as i16 as i32;
    let a: i32 = if d > 0 {
        (orig << 16).wrapping_div(d)
    } else {
        0
    };

    let (g, r8) = if d > 0 && a < 0 {
        if orig > 0 {
            scaled_fill(f, 1)
        } else {
            degenerate_fill(f)
        }
    } else if orig <= 0 {
        degenerate_fill(f)
    } else {
        let a = a.min(0x10000);
        let e = a.wrapping_mul(max).wrapping_div(orig);
        let g = if e <= 0 { 1 } else { e.min(0x10000) };
        scaled_fill(f, g)
    };

    bar_segments(rect, step, g, r8)
}

fn bar_segments(rect: PixelRect, step: i32, g: i32, fill: i32) -> VitalityBar {
    let height = (rect.bottom - rect.top) as i16 as i32;
    let avail = height - VITALITY_BAR_HEIGHT_INSET;
    let fill_px = fill.wrapping_mul(avail) >> 16;
    let mut total_px = g.wrapping_mul(avail) >> 16;
    if fill_px as i16 >= total_px as i16 {
        total_px = fill_px;
    }
    let empty_px = total_px - fill_px;
    let (left, right) = (
        rect.right + VITALITY_BAR_LEFT,
        rect.right + VITALITY_BAR_RIGHT,
    );

    let mut bottom = rect.bottom;
    let mut seg = |h: i32, color: BarColor| {
        let top = bottom - h;
        let r = (PixelRect::new(left, top, right, bottom), color);
        bottom = top;
        r
    };
    let segments = [
        seg(fill_px + VITALITY_BAR_CAP, BarColor::Fill),
        seg(empty_px, BarColor::Background),
        seg(VITALITY_BAR_CAP, BarColor::Fill),
    ];
    VitalityBar {
        step: step as usize,
        segments,
    }
}

fn scaled_fill(f: i32, g: i32) -> (i32, i32) {
    let fill = (f >> 8).wrapping_mul(g) >> 8;
    (g, if fill <= 0 { 1 } else { fill.min(0x10000) })
}

fn degenerate_fill(f: i32) -> (i32, i32) {
    let fill = f >> 16;
    (1, if fill > 0 { fill.min(0x10000) } else { 1 })
}

pub mod tfl {
    use super::{ObjectBounds, PixelRect, VITALITY_COLOR_STEPS, VitalityBar, bar_segments};

    pub const FRAME_THICKNESS: i32 = 2;

    pub const TRANSPARENCY: u16 = 0x8000;

    pub const OBJECT_FRAMED: u32 = 0x1_0000;

    pub const COMMAND_POLLS: i16 = 7;

    pub fn transparency(hovered_only: bool) -> u16 {
        if hovered_only {
            super::TRANSPARENCY_HIGHLIGHTED
        } else {
            TRANSPARENCY
        }
    }

    pub const EVAL_FRIENDLY: u32 = 2;
    pub const EVAL_OWNED: u32 = 4;
    pub const MONSTER_FLAG_NOT_COMMANDED: u32 = 0x1000_0000;

    pub const COLOR_OBJECT: usize = 5;
    pub const COLOR_OWNED: usize = 2;
    pub const COLOR_FRIENDLY: usize = 4;
    pub const COLOR_OTHER: usize = 3;

    pub fn frame_color_index(monster: Option<(u32, u32)>) -> usize {
        match monster {
            None => COLOR_OBJECT,
            Some((eval, _)) if eval & EVAL_FRIENDLY == 0 => COLOR_OTHER,
            Some((eval, flags))
                if eval & EVAL_OWNED != 0 && flags & MONSTER_FLAG_NOT_COMMANDED == 0 =>
            {
                COLOR_OWNED
            }
            Some(_) => COLOR_FRIENDLY,
        }
    }

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub struct MonsterVitality {
        pub current: i16,
        pub maximum: i16,
        pub definition: i16,
    }

    fn fraction(v: i16, definition: i32) -> i32 {
        (i32::from(v) << 16)
            .wrapping_div(definition)
            .clamp(1, 0x10000)
    }

    pub fn vitality_bar(
        rect: PixelRect,
        bounds: ObjectBounds,
        v: MonsterVitality,
    ) -> Option<VitalityBar> {
        let definition = i32::from(v.definition);
        if definition == 0 {
            return None;
        }
        let mut f = fraction(v.current, definition);
        let mut g = fraction(v.maximum, definition);
        let step = (f.wrapping_mul(VITALITY_COLOR_STEPS) >> 16) as i16;
        let d = (((i32::from(bounds.top) - i32::from(bounds.bottom)) << 9) >> 8) as i16;
        if v.definition <= d {
            let d = i32::from(d);
            if d == 0 {
                return None;
            }
            f = definition.wrapping_mul(f).wrapping_div(d);
            g = definition.wrapping_mul(g).wrapping_div(d);
        }
        (f != 0).then(|| bar_segments(rect, i32::from(step).max(0), g, f))
    }
}

pub mod mana {
    use super::PixelRect;
    use super::state::GameContext;

    pub const OBJECT_TYPE_MONSTER: u8 = 2;

    pub const DEFINITION_FLAG_NO_MANA_BAR: u8 = 0x20;
    pub const DEFINITION_FLAG_MANA_BAR_OWN_TEAM: u8 = 0x10;

    pub const BAR_TOP: i32 = -8;
    pub const BAR_BOTTOM: i32 = -2;

    pub const BAR_COLOR: [u8; 3] = [0, 0, 0xff];

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub struct MonsterMana {
        pub current: i16,
        pub maximum: i16,
        pub definition_flags: u8,
        pub team: i8,
    }

    pub fn draws_bar(ctx: &GameContext, m: &MonsterMana) -> bool {
        if m.definition_flags & DEFINITION_FLAG_NO_MANA_BAR != 0 {
            return false;
        }
        if ctx.game_mode != 0
            && m.definition_flags & DEFINITION_FLAG_MANA_BAR_OWN_TEAM != 0
            && i16::from(m.team) != ctx.local_team
        {
            return false;
        }
        m.maximum != 0
    }

    pub fn fraction(m: &MonsterMana) -> i32 {
        if m.maximum == 0 {
            return 1;
        }
        let f = (i32::from(m.current) << 16).wrapping_div(i32::from(m.maximum));
        let f = f.min(0x10000);
        if f <= 0 { 1 } else { f }
    }

    pub fn bar(frame: PixelRect, m: &MonsterMana) -> PixelRect {
        let f = fraction(m);
        let width = (frame.right - frame.left) as i16 as i32;
        let right = (width.wrapping_mul(f) >> 16) + frame.left;
        let w = |v: i32| v as i16 as i32;
        PixelRect::new(
            w(frame.left),
            w(frame.top + BAR_TOP),
            w(right),
            w(frame.top + BAR_BOTTOM),
        )
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        fn warlock(current: i16) -> MonsterMana {
            MonsterMana {
                current,
                maximum: 25600,
                definition_flags: 0,
                team: 0,
            }
        }

        #[test]
        fn test_mana_bar_full_empty_partial() {
            let frame = PixelRect::new(100, 50, 140, 130);
            assert_eq!(fraction(&warlock(25600)), 0x10000);
            assert_eq!(
                bar(frame, &warlock(25600)),
                PixelRect::new(100, 42, 140, 48)
            );
            assert_eq!(
                bar(frame, &warlock(30000)),
                PixelRect::new(100, 42, 140, 48)
            );
            assert_eq!(fraction(&warlock(0)), 1);
            let empty = bar(frame, &warlock(0));
            assert_eq!(empty, PixelRect::new(100, 42, 100, 48));
            assert!(empty.is_empty());
            assert_eq!(fraction(&warlock(100)), 0x100);
            assert!(bar(frame, &warlock(100)).is_empty());
            assert_eq!(fraction(&warlock(10667)), 0x6aab);
            assert_eq!(
                bar(frame, &warlock(10667)),
                PixelRect::new(100, 42, 116, 48)
            );
            assert_eq!(bar(frame, &warlock(12800)).right, 120);
            assert!(bar(frame, &warlock(-5)).is_empty());
            assert_eq!(BAR_COLOR, [0, 0, 0xff]);
        }

        #[test]
        fn test_mana_bar_gate() {
            let sp = GameContext::single_player(0, 0);
            let net = GameContext { game_mode: 2, ..sp };
            assert!(draws_bar(&sp, &warlock(10)));
            let none = MonsterMana {
                maximum: 0,
                ..warlock(0)
            };
            assert!(!draws_bar(&sp, &none));
            let hidden = MonsterMana {
                definition_flags: DEFINITION_FLAG_NO_MANA_BAR,
                ..warlock(10)
            };
            assert!(!draws_bar(&sp, &hidden));
            let own = MonsterMana {
                definition_flags: DEFINITION_FLAG_MANA_BAR_OWN_TEAM,
                team: 1,
                ..warlock(10)
            };
            assert!(draws_bar(&sp, &own));
            assert!(!draws_bar(&net, &own));
            assert!(draws_bar(&net, &MonsterMana { team: 0, ..own }));
            assert!(draws_bar(
                &net,
                &MonsterMana {
                    team: 1,
                    ..warlock(10)
                }
            ));
        }
    }
}

pub mod band {
    use super::PixelRect;

    pub const EDGE_COLOR_INDEX: usize = 1;

    pub const EDGE_TRANSPARENCY: u16 = 0x8000;

    pub const FILL_TRANSPARENCY: u16 = 0xd555;

    pub const FILL_COLOR: [u8; 3] = [0, 0, 0];

    pub const EDGE_THICKNESS: i32 = 2;

    pub const SMALL_BAND_LIMIT: i32 = 4;

    pub fn bounds(anchor: (i32, i32), cursor: (i32, i32)) -> PixelRect {
        PixelRect::new(
            anchor.0.min(cursor.0),
            anchor.1.min(cursor.1),
            anchor.0.max(cursor.0),
            anchor.1.max(cursor.1),
        )
    }

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub enum Part {
        Edge,
        Fill,
    }

    impl Part {
        pub fn transparency(self) -> u16 {
            match self {
                Part::Edge => EDGE_TRANSPARENCY,
                Part::Fill => FILL_TRANSPARENCY,
            }
        }
    }

    pub fn rectangles(b: PixelRect, viewport: [i32; 2], hardware: bool) -> Vec<(PixelRect, Part)> {
        let clamped = PixelRect::new(
            b.left.max(0),
            b.top.max(0),
            b.right.min(viewport[0]),
            b.bottom.min(viewport[1]),
        );
        if b.width() > SMALL_BAND_LIMIT && b.height() > SMALL_BAND_LIMIT {
            let mut out: Vec<(PixelRect, Part)> = super::frame_edges(b, EDGE_THICKNESS)
                .into_iter()
                .map(|r| (r, Part::Edge))
                .collect();
            if hardware {
                let t = EDGE_THICKNESS;
                out.push((
                    PixelRect::new(
                        clamped.left + t,
                        clamped.top + t,
                        clamped.right - t,
                        clamped.bottom - t,
                    ),
                    Part::Fill,
                ));
            }
            out
        } else {
            let mut r = clamped;
            if r.right - r.left <= 1 {
                if r.right == viewport[0] {
                    r.left = r.right - EDGE_THICKNESS;
                } else {
                    r.right = r.left + EDGE_THICKNESS;
                }
            }
            if r.bottom - r.top <= 1 {
                if r.bottom == viewport[1] {
                    r.top = r.bottom - EDGE_THICKNESS;
                } else {
                    r.bottom = r.top + EDGE_THICKNESS;
                }
            }
            vec![(r, Part::Edge)]
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn warrior() -> ObjectBounds {
        ObjectBounds::from_sequence(
            SequenceWorldBounds {
                half_width: 120,
                bottom: -70,
                top: 580,
            },
            OBJECT_SCALE_ONE,
        )
    }

    fn proj(origin: [f32; 2], k: f32) -> FrameProjection {
        FrameProjection {
            origin,
            pixels_per_unit: k,
            vertical_pixels_per_unit: k,
            viewport: [3840, 2160],
        }
    }

    #[test]
    fn test_object_bounds_scale() {
        assert_eq!(
            warrior(),
            ObjectBounds {
                half_width: 120,
                bottom: -70,
                top: 580
            }
        );
        let big = ObjectBounds::from_sequence(
            SequenceWorldBounds {
                half_width: 120,
                bottom: -70,
                top: 580,
            },
            0x180,
        );
        assert_eq!((big.half_width, big.bottom, big.top), (180, -105, 870));
    }

    #[test]
    fn test_frame_rect_reference_select_2() {
        let p = proj([1795.49, 1180.0], 0.396);
        let r = selection_frame_rect(warrior(), &p).unwrap();
        assert_eq!(r, PixelRect::new(1747, 950, 1843, 1207));
        assert_eq!((r.width(), r.height()), (96, 257));
    }

    #[test]
    fn test_frame_rect_truncates() {
        let r = selection_frame_rect(warrior(), &proj([100.5, 200.75], 0.3)).unwrap();
        assert_eq!(r, PixelRect::new(64, 26, 136, 221));
        let r = selection_frame_rect(warrior(), &proj([-10.5, 100.0], 0.3)).unwrap();
        assert_eq!((r.left, r.right), (-46, 25));
    }

    #[test]
    fn test_frame_rect_culling() {
        let w = warrior();
        assert!(selection_frame_rect(w, &proj([-36.0, 500.0], 0.3)).is_none());
        assert!(selection_frame_rect(w, &proj([3876.0, 500.0], 0.3)).is_none());
        assert!(selection_frame_rect(w, &proj([500.0, -21.0], 0.3)).is_none());
        assert!(selection_frame_rect(w, &proj([500.0, 2160.0 + 174.0], 0.3)).is_none());
        assert!(selection_frame_rect(w, &proj([500.0, 500.0], 0.0)).is_none());
        assert!(selection_frame_rect(w, &proj([0.0, 500.0], 0.3)).is_some());
    }

    #[test]
    fn test_frame_rect_engine_walking_warrior() {
        let k = 0.191_220_f32;
        let origin = [512.0 - 927.0 * k, 64.0 + 211.0 + 1616.0 * k];
        let r = selection_frame_rect(warrior(), &proj(origin, k)).unwrap();
        assert_eq!(r, PixelRect::new(311, 473, 357, 597));
    }

    #[test]
    fn test_frame_rect_vertical_scale() {
        let p = FrameProjection {
            vertical_pixels_per_unit: 0.15,
            ..proj([100.5, 200.75], 0.3)
        };
        let r = selection_frame_rect(warrior(), &p).unwrap();
        assert_eq!(r, PixelRect::new(64, 113, 136, 211));
    }

    #[test]
    fn test_flipped_bounds() {
        let f = warrior().flipped();
        assert_eq!((f.half_width, f.bottom, f.top), (120, -580, 70));
        let r = selection_frame_rect(f, &proj([100.5, 200.75], 0.3)).unwrap();
        assert_eq!((r.top, r.bottom), (179, 374));
    }

    #[test]
    fn test_frame_edges() {
        let r = PixelRect::new(10, 20, 50, 120);
        assert_eq!(
            frame_edges(r, 1),
            [
                PixelRect::new(10, 20, 50, 21),
                PixelRect::new(10, 119, 50, 120),
                PixelRect::new(10, 21, 11, 119),
                PixelRect::new(49, 21, 50, 119),
            ]
        );
        let e = frame_edges(r, frame_thickness(true));
        assert_eq!(e[0], PixelRect::new(10, 20, 50, 22));
        assert_eq!(e[3], PixelRect::new(48, 22, 50, 118));
    }

    #[test]
    fn test_blend_display_channel_matches_reference() {
        let a = overlay_alpha(TRANSPARENCY_SELECTED);
        let got = [
            blend_display_channel(0xff, 72, a),
            blend_display_channel(0xff, 42, a),
            blend_display_channel(0x00, 128, a),
        ];
        assert_eq!(got, [194, 184, 43]);
        assert_eq!(blend_display_channel(0xff, 0, 0xff), 0xff);
        assert_eq!(blend_display_channel(0x10, 0x80, 0), 0x80);
    }

    #[test]
    fn test_overlay_alpha_and_thickness() {
        assert_eq!(overlay_alpha(TRANSPARENCY_SELECTED), 0xaa);
        assert_eq!(overlay_alpha(TRANSPARENCY_HIGHLIGHTED), 0x55);
        assert_eq!(overlay_alpha(0), 0xff);
        assert_eq!(frame_thickness(false), 1);
        assert_eq!(frame_thickness(true), 2);
    }

    #[test]
    fn test_vitality_bar_full_health() {
        let rect = PixelRect::new(1747, 950, 1843, 1207);
        let v = ObjectVitality {
            current: 0x580,
            maximum: 0x580,
            original: 0x580,
        };
        let bar = vitality_bar(rect, warrior(), v);
        assert_eq!(bar.step, 21);
        let h = 257 - 6;
        assert_eq!(
            bar.segments,
            [
                (
                    PixelRect::new(1845, 1207 - h - 2, 1851, 1207),
                    BarColor::Fill
                ),
                (
                    PixelRect::new(1845, 1207 - h - 2, 1851, 1207 - h - 2),
                    BarColor::Background
                ),
                (
                    PixelRect::new(1845, 952, 1851, 1207 - h - 2),
                    BarColor::Fill
                ),
            ]
        );
        assert!(bar.segments[1].0.is_empty());
    }

    #[test]
    fn test_vitality_bar_half_health() {
        let rect = PixelRect::new(0, 0, 40, 106);
        let v = ObjectVitality {
            current: 0x2c0,
            maximum: 0x580,
            original: 0x580,
        };
        let bar = vitality_bar(rect, warrior(), v);
        assert_eq!(bar.step, 10);
        let [(fill, c0), (bg, c1), (cap, c2)] = bar.segments;
        assert_eq!(
            (c0, c1, c2),
            (BarColor::Fill, BarColor::Background, BarColor::Fill)
        );
        assert_eq!(fill, PixelRect::new(42, 106 - 52, 48, 106));
        assert_eq!(bg, PixelRect::new(42, 54 - 50, 48, 54));
        assert_eq!(cap, PixelRect::new(42, 2, 48, 4));
    }

    #[test]
    fn test_vitality_bar_permanent_damage_and_low_vitality() {
        let rect = PixelRect::new(0, 0, 40, 106);
        let v = ObjectVitality {
            current: 0x2c0,
            maximum: 0x2c0,
            original: 0x580,
        };
        let bar = vitality_bar(rect, warrior(), v);
        assert_eq!(bar.step, 21);
        assert_eq!(bar.segments[0].0.top, 106 - 52);
        assert!(bar.segments[1].0.is_empty());
        assert_eq!(bar.segments[2].0, PixelRect::new(42, 52, 48, 54));

        let v = ObjectVitality {
            current: 325,
            maximum: 325,
            original: 325,
        };
        let bar = vitality_bar(rect, warrior(), v);
        assert_eq!(bar.segments[0].0, PixelRect::new(42, 106 - 27, 48, 106));
        assert_eq!(bar.segments[2].0, PixelRect::new(42, 77, 48, 79));
    }

    #[test]
    fn test_vitality_bar_dead_and_degenerate() {
        let rect = PixelRect::new(0, 0, 40, 106);
        let v = ObjectVitality {
            current: 0,
            maximum: 0x580,
            original: 0x580,
        };
        let bar = vitality_bar(rect, warrior(), v);
        assert_eq!(bar.step, 0);
        assert_eq!(bar.segments[0].0, PixelRect::new(42, 104, 48, 106));
        assert_eq!(bar.segments[1].0, PixelRect::new(42, 4, 48, 104));
        let v = ObjectVitality {
            current: 0,
            maximum: 0,
            original: 0,
        };
        let bar = vitality_bar(rect, warrior(), v);
        assert_eq!(bar.segments[0].0, PixelRect::new(42, 104, 48, 106));
        assert!(bar.segments[1].0.is_empty());
        assert_eq!(bar.segments[2].0, PixelRect::new(42, 102, 48, 104));
    }
    #[test]
    fn test_tfl_vitality_bar_sturdy_monster() {
        let rect = PixelRect::new(100, 100, 140, 200);
        let v = |current| tfl::MonsterVitality {
            current,
            maximum: 3000,
            definition: 3000,
        };
        let bar = tfl::vitality_bar(rect, warrior(), v(1500)).unwrap();
        assert_eq!(bar.step, 10);
        assert_eq!(
            bar.segments,
            [
                (PixelRect::new(142, 151, 148, 200), BarColor::Fill),
                (PixelRect::new(142, 104, 148, 151), BarColor::Background),
                (PixelRect::new(142, 102, 148, 104), BarColor::Fill),
            ]
        );
        let bar = tfl::vitality_bar(rect, warrior(), v(3000)).unwrap();
        assert_eq!(bar.step, 21);
        assert_eq!(bar.segments[0].0, PixelRect::new(142, 104, 148, 200));
        assert!(bar.segments[1].0.is_empty());
        let bar = tfl::vitality_bar(rect, warrior(), v(0)).unwrap();
        assert_eq!(bar.step, 0);
        assert_eq!(bar.segments[0].0, PixelRect::new(142, 198, 148, 200));
        assert_eq!(bar.segments[1].0, PixelRect::new(142, 104, 148, 198));
    }

    #[test]
    fn test_tfl_vitality_bar_frail_monster() {
        let rect = PixelRect::new(100, 100, 140, 200);
        let v = |current, definition| tfl::MonsterVitality {
            current,
            maximum: definition,
            definition,
        };
        let bar = tfl::vitality_bar(rect, warrior(), v(1000, 1000)).unwrap();
        assert_eq!(bar.step, 21);
        assert_eq!(bar.segments[0].0, PixelRect::new(142, 126, 148, 200));
        assert!(bar.segments[1].0.is_empty());
        assert_eq!(bar.segments[2].0, PixelRect::new(142, 124, 148, 126));
        assert_eq!(tfl::vitality_bar(rect, warrior(), v(0, 1)), None);
        assert_eq!(tfl::vitality_bar(rect, warrior(), v(5, 0)), None);
    }

    #[test]
    fn test_tfl_frame_color_index() {
        use tfl::*;
        assert_eq!(frame_color_index(None), 5);
        assert_eq!(frame_color_index(Some((0, 0))), 3);
        assert_eq!(frame_color_index(Some((1, 0))), 3);
        assert_eq!(frame_color_index(Some((7, 0))), 2);
        assert_eq!(frame_color_index(Some((6, MONSTER_FLAG_NOT_COMMANDED))), 4);
        assert_eq!(frame_color_index(Some((EVAL_FRIENDLY, 0))), 4);
        assert_eq!(super::overlay_alpha(TRANSPARENCY), 0x7f);
    }

    #[test]
    fn test_band_bounds_min_max() {
        assert_eq!(
            band::bounds((300, 400), (100, 150)),
            PixelRect::new(100, 150, 300, 400)
        );
        assert_eq!(
            band::bounds((10, 20), (50, 5)),
            PixelRect::new(10, 5, 50, 20)
        );
    }

    #[test]
    fn test_band_large_edges_and_fill() {
        let b = PixelRect::new(100, 200, 300, 260);
        let rects = band::rectangles(b, [1280, 720], true);
        assert_eq!(
            rects,
            vec![
                (PixelRect::new(100, 200, 300, 202), band::Part::Edge),
                (PixelRect::new(100, 258, 300, 260), band::Part::Edge),
                (PixelRect::new(100, 202, 102, 258), band::Part::Edge),
                (PixelRect::new(298, 202, 300, 258), band::Part::Edge),
                (PixelRect::new(102, 202, 298, 258), band::Part::Fill),
            ]
        );
        let soft = band::rectangles(b, [1280, 720], false);
        assert_eq!(soft.len(), 4);
        assert!(soft.iter().all(|(_, p)| *p == band::Part::Edge));
    }

    #[test]
    fn test_band_fill_uses_clamped_bounds_edges_do_not() {
        let b = PixelRect::new(-20, 400, 100, 500);
        let rects = band::rectangles(b, [640, 480], true);
        assert_eq!(rects[0].0, PixelRect::new(-20, 400, 100, 402));
        assert_eq!(rects[1].0, PixelRect::new(-20, 498, 100, 500));
        assert_eq!(
            rects[4],
            (PixelRect::new(2, 402, 98, 478), band::Part::Fill)
        );
    }

    #[test]
    fn test_band_small_single_rectangle() {
        let rects = band::rectangles(PixelRect::new(10, 10, 14, 90), [640, 480], true);
        assert_eq!(
            rects,
            vec![(PixelRect::new(10, 10, 14, 90), band::Part::Edge)]
        );
        let rects = band::rectangles(PixelRect::new(50, 10, 50, 90), [640, 480], true);
        assert_eq!(rects[0].0, PixelRect::new(50, 10, 52, 90));
        let rects = band::rectangles(PixelRect::new(640, 10, 700, 12), [640, 480], true);
        assert_eq!(rects[0].0, PixelRect::new(638, 10, 640, 12));
        let rects = band::rectangles(PixelRect::new(10, 479, 20, 480), [640, 480], true);
        assert_eq!(rects[0].0, PixelRect::new(10, 478, 20, 480));
    }

    #[test]
    fn test_band_colors_and_alpha() {
        assert_eq!(band::EDGE_COLOR_INDEX, 1);
        assert_eq!(overlay_alpha(band::Part::Edge.transparency()), 0x7f);
        assert_eq!(overlay_alpha(band::Part::Fill.transparency()), 0x2a);
        assert_eq!(band::FILL_COLOR, [0, 0, 0]);
        let a = overlay_alpha(band::FILL_TRANSPARENCY);
        assert_eq!(blend_display_channel(0, 120, a), 100);
    }
}
