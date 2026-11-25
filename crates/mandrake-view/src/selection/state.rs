use super::{PixelRect, TRANSPARENCY_HIGHLIGHTED, TRANSPARENCY_SELECTED};

pub const OBJECT_SELECTED: u32 = 0x1_0000;
pub const OBJECT_TARGETED: u32 = 0x4000_0000;
pub const OBJECT_HOVERED: u32 = 0x8000_0000;
pub const OBJECT_FLIPPED: u32 = 0x4_0000;

pub const TRANSPARENCY_SUBMERGED: u16 = 0xd554;

pub const CAPS_LOCK_KEY: u16 = 0x2c;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum CapsLock {
    SelectionBoxes = 0,
    Ignored = 1,
    HealthBars = 2,
    PlayerStats = 3,
    ChatBacklog = 4,
}

pub const PREFERENCE_TFL_SELECTION_BOXES: u32 = 0x1_0000;

pub const EVAL_ALLIED: u32 = 2;
pub const EVAL_OWNED: u32 = 4;
pub const EVAL_CONTROLLABLE: u32 = 1;

pub const MONSTER_FLAG_ALLY_COLORED: u32 = 0x1000_0000;
pub const MONSTER_FLAGS8_ALLY_COLORED: u32 = 0x4100;
pub const MONSTER_FLAGS8_DEFINITION_COLORED: u32 = 0x1000;
pub const MONSTER_FLAGS8_HUNTED: u32 = 0x2000;
pub const MONSTER_FLAGS8_OUTER_FRAME: u32 = 0x800;
pub const MONS_FLAG_ENEMY_COLORED: u32 = 0x200_0000;
pub const MONS_CLASS_EXCLUDED: i16 = 5;

pub const GAME_TYPE_SINGLE_PLAYER: u16 = 0xd;
pub const GAME_TYPE_HUNTED: u16 = 0xc;
pub const GAME_TYPES_OUTER_FRAME: [u16; 2] = [0xa, 0xb];

pub const OUTER_FRAME_COLOR: [u8; 3] = [0xff, 0x7f, 0x00];
pub const OUTER_FRAME_TRANSPARENCY: u16 = 0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GameContext {
    pub game_mode: u16,
    pub game_type: u16,
    pub local_team: i16,
    pub local_player: i8,
    pub controllable_flag: bool,
}

impl GameContext {
    pub fn single_player(local_team: i16, local_player: i8) -> Self {
        Self {
            game_mode: 0,
            game_type: GAME_TYPE_SINGLE_PLAYER,
            local_team,
            local_player,
            controllable_flag: false,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Alliances {
    pub team_count: i16,
    pub table: [[bool; 16]; 16],
}

impl Alliances {
    pub fn new(team_count: i16) -> Self {
        Self {
            team_count,
            table: [[false; 16]; 16],
        }
    }

    pub fn teams_are_allied(&self, a: i16, b: i16) -> bool {
        if a == b {
            return true;
        }
        if a < 0 || b < 0 || self.team_count <= a || self.team_count <= b {
            return false;
        }
        self.table
            .get(a as usize)
            .and_then(|row| row.get(b as usize))
            .copied()
            .unwrap_or(false)
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct MonsterInterface {
    pub team: i8,
    pub player: i8,
    pub flags: u32,
    pub flags8: u32,
    pub definition_flags: u32,
    pub definition_class: i16,
}

pub fn evaluate_monster(ctx: &GameContext, alliances: &Alliances, m: &MonsterInterface) -> u32 {
    if !alliances.teams_are_allied(ctx.local_team, i16::from(m.team)) {
        return 0;
    }
    let owned = m.player == ctx.local_player;
    let mut r = if ctx.controllable_flag {
        u32::from(owned) * 4 + 1
    } else {
        u32::from(owned) * 5
    };
    if m.flags & MONSTER_FLAG_ALLY_COLORED != 0 || ctx.game_mode == 0 {
        r &= !EVAL_CONTROLLABLE;
    }
    r | EVAL_ALLIED
}

pub fn frame_color_index(ctx: &GameContext, monster: Option<(&MonsterInterface, u32)>) -> usize {
    use mandrake_tag::interface::{
        COLOR_SELECTION_ALLY, COLOR_SELECTION_ENEMY, COLOR_SELECTION_OBJECT, COLOR_SELECTION_OWNED,
    };
    let Some((m, eval)) = monster else {
        return COLOR_SELECTION_OBJECT;
    };
    let mut color = if eval & EVAL_ALLIED == 0 {
        COLOR_SELECTION_ENEMY
    } else if eval & EVAL_OWNED == 0
        || m.flags & MONSTER_FLAG_ALLY_COLORED != 0
        || m.flags8 & MONSTER_FLAGS8_ALLY_COLORED != 0
    {
        COLOR_SELECTION_ALLY
    } else {
        COLOR_SELECTION_OWNED
    };
    if m.flags8 & MONSTER_FLAGS8_DEFINITION_COLORED != 0 {
        color = if m.definition_flags & MONS_FLAG_ENEMY_COLORED != 0 {
            COLOR_SELECTION_ENEMY
        } else {
            COLOR_SELECTION_ALLY
        };
    }
    if ctx.game_type == GAME_TYPE_HUNTED && m.flags8 & MONSTER_FLAGS8_HUNTED != 0 {
        color = COLOR_SELECTION_ENEMY;
    }
    color
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct OverlayState {
    pub transparency: u16,
    pub frame: bool,
}

#[derive(Clone, Copy, Debug)]
pub struct OverlayObject<'a> {
    pub flags: u32,
    pub monster: Option<&'a MonsterInterface>,
}

pub fn overlay_state(
    ctx: &GameContext,
    alliances: &Alliances,
    object: OverlayObject<'_>,
    caps_lock: Option<CapsLock>,
) -> Option<OverlayState> {
    let flags = object.flags;
    if flags & (OBJECT_TARGETED | OBJECT_SELECTED) != 0 {
        let hovered_only = flags & OBJECT_HOVERED != 0 && flags & OBJECT_TARGETED == 0;
        return Some(OverlayState {
            transparency: if hovered_only {
                TRANSPARENCY_HIGHLIGHTED
            } else {
                TRANSPARENCY_SELECTED
            },
            frame: true,
        });
    }
    if flags & OBJECT_HOVERED != 0 {
        return Some(OverlayState {
            transparency: TRANSPARENCY_HIGHLIGHTED,
            frame: true,
        });
    }
    let m = object.monster?;
    let mode = caps_lock?;
    if (mode as u8) & 0xfd != 0 {
        return None;
    }
    let check_team = match ctx.game_mode {
        0 => false,
        1 => ctx.local_team != -1,
        _ => true,
    };
    if check_team
        && (i16::from(m.team) != ctx.local_team || m.definition_class == MONS_CLASS_EXCLUDED)
    {
        return None;
    }
    let frame = mode != CapsLock::HealthBars
        || evaluate_monster(ctx, alliances, m) & (EVAL_OWNED | EVAL_CONTROLLABLE)
            != EVAL_OWNED | EVAL_CONTROLLABLE;
    Some(OverlayState {
        transparency: TRANSPARENCY_HIGHLIGHTED,
        frame,
    })
}

pub fn overlay_transparency(state: OverlayState, submerged: bool) -> u16 {
    if submerged {
        TRANSPARENCY_SUBMERGED
    } else {
        state.transparency
    }
}

pub fn draws_frame(state: OverlayState, obje_frame_flag: bool, object_flags: u32) -> bool {
    state.frame && (obje_frame_flag || object_flags & OBJECT_TARGETED != 0)
}

pub fn draws_outer_frame(ctx: &GameContext, monster: Option<&MonsterInterface>) -> bool {
    let Some(m) = monster else {
        return false;
    };
    m.flags8 & MONSTER_FLAGS8_OUTER_FRAME != 0
        && GAME_TYPES_OUTER_FRAME.contains(&ctx.game_type)
        && (i16::from(m.team) == ctx.local_team || ctx.game_mode == 0)
}

pub fn outer_frame_edges(rect: PixelRect, thickness: i32) -> [PixelRect; 4] {
    let t = thickness;
    let PixelRect {
        left,
        top,
        right,
        bottom,
    } = rect;
    [
        PixelRect::new(left - t, top - t, right + t, top),
        PixelRect::new(left - t, bottom, right + t, bottom + t),
        PixelRect::new(left - t, top, left, bottom),
        PixelRect::new(right, top, right + t, bottom),
    ]
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct HoverTracker {
    stamp_ms: u32,
    hovered: Option<u32>,
}

pub const HOVER_REFRESH_MS: u32 = 0xfa;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct HoverPick {
    pub id: u32,
    pub selected: bool,
    pub monster: bool,
}

impl HoverTracker {
    pub fn hovered(&self) -> Option<u32> {
        self.hovered
    }

    pub fn update(
        &mut self,
        now_ms: u32,
        pointer_active: bool,
        pick: impl FnOnce() -> Option<HoverPick>,
    ) {
        if now_ms <= self.stamp_ms.wrapping_add(HOVER_REFRESH_MS) {
            return;
        }
        self.stamp_ms = now_ms;
        let found = if pointer_active { pick() } else { None };
        self.hovered = found.filter(|p| !p.selected && p.monster).map(|p| p.id);
    }

    pub fn clear(&mut self) {
        self.hovered = None;
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TargetHighlight {
    countdown: i16,
    targets: Vec<u32>,
    object: Option<i32>,
}

pub const TARGET_HIGHLIGHT_TICKS: i16 = 7;

impl TargetHighlight {
    pub fn command(&mut self, targets: &[u32], selection: &[u32]) {
        self.countdown = TARGET_HIGHLIGHT_TICKS;
        self.object = None;
        self.targets = targets
            .iter()
            .copied()
            .filter(|t| !selection.contains(t))
            .collect();
    }

    pub fn pick_up(&mut self, object: i32) {
        self.countdown = TARGET_HIGHLIGHT_TICKS;
        self.object = Some(object);
    }

    pub fn update(&mut self, elapsed: i16) {
        if self.countdown == 0 {
            return;
        }
        self.countdown -= elapsed;
        if self.countdown <= 0 {
            self.countdown = 0;
            self.targets.clear();
            self.object = None;
        }
    }

    pub fn is_targeted(&self, id: u32) -> bool {
        self.targets.contains(&id)
    }

    pub fn is_object_targeted(&self, index: i32) -> bool {
        self.object == Some(index)
    }

    pub fn targets(&self) -> &[u32] {
        &self.targets
    }

    pub fn object(&self) -> Option<i32> {
        self.object
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mandrake_tag::interface::{
        COLOR_SELECTION_ALLY, COLOR_SELECTION_ENEMY, COLOR_SELECTION_OBJECT, COLOR_SELECTION_OWNED,
    };

    fn sp() -> GameContext {
        GameContext::single_player(0, 0)
    }

    fn owned() -> MonsterInterface {
        MonsterInterface {
            team: 0,
            player: 0,
            ..Default::default()
        }
    }

    #[test]
    fn test_teams_are_allied() {
        let mut a = Alliances::new(3);
        assert!(a.teams_are_allied(1, 1));
        assert!(a.teams_are_allied(-1, -1));
        assert!(!a.teams_are_allied(0, 1));
        assert!(!a.teams_are_allied(0, -1));
        a.table[0][1] = true;
        assert!(a.teams_are_allied(0, 1));
        assert!(!a.teams_are_allied(1, 0));
        a.table[0][5] = true;
        assert!(!a.teams_are_allied(0, 5), "team 5 is out of range");
    }

    #[test]
    fn test_evaluate_monster() {
        let a = Alliances::new(2);
        let enemy = MonsterInterface { team: 1, ..owned() };
        assert_eq!(evaluate_monster(&sp(), &a, &enemy), 0);
        assert_eq!(
            evaluate_monster(&sp(), &a, &owned()),
            EVAL_ALLIED | EVAL_OWNED
        );
        let other_player = MonsterInterface {
            player: 1,
            ..owned()
        };
        assert_eq!(evaluate_monster(&sp(), &a, &other_player), EVAL_ALLIED);
        let net = GameContext {
            game_mode: 2,
            ..sp()
        };
        assert_eq!(evaluate_monster(&net, &a, &owned()), 7);
        let flagged = MonsterInterface {
            flags: MONSTER_FLAG_ALLY_COLORED,
            ..owned()
        };
        assert_eq!(evaluate_monster(&net, &a, &flagged), 6);
        let controllable = GameContext {
            controllable_flag: true,
            ..net
        };
        assert_eq!(evaluate_monster(&controllable, &a, &other_player), 3);
    }

    #[test]
    fn test_frame_color_index() {
        let a = Alliances::new(2);
        let c = |m: &MonsterInterface| {
            frame_color_index(&sp(), Some((m, evaluate_monster(&sp(), &a, m))))
        };
        assert_eq!(frame_color_index(&sp(), None), COLOR_SELECTION_OBJECT);
        assert_eq!(c(&owned()), COLOR_SELECTION_OWNED);
        assert_eq!(
            c(&MonsterInterface { team: 1, ..owned() }),
            COLOR_SELECTION_ENEMY
        );
        assert_eq!(
            c(&MonsterInterface {
                player: 3,
                ..owned()
            }),
            COLOR_SELECTION_ALLY
        );
        let villager = MonsterInterface {
            flags: MONSTER_FLAG_ALLY_COLORED,
            ..owned()
        };
        assert_eq!(c(&villager), COLOR_SELECTION_ALLY);
        assert_eq!(
            c(&MonsterInterface {
                flags8: 0x100,
                ..owned()
            }),
            COLOR_SELECTION_ALLY
        );
        let ambient = MonsterInterface {
            team: 1,
            flags8: MONSTER_FLAGS8_DEFINITION_COLORED,
            ..owned()
        };
        assert_eq!(c(&ambient), COLOR_SELECTION_ALLY);
        assert_eq!(
            c(&MonsterInterface {
                definition_flags: MONS_FLAG_ENEMY_COLORED,
                ..ambient
            }),
            COLOR_SELECTION_ENEMY
        );
        let hunted = MonsterInterface {
            flags8: MONSTER_FLAGS8_HUNTED,
            ..owned()
        };
        assert_eq!(c(&hunted), COLOR_SELECTION_OWNED);
        let g12 = GameContext {
            game_type: GAME_TYPE_HUNTED,
            ..sp()
        };
        assert_eq!(
            frame_color_index(&g12, Some((&hunted, EVAL_ALLIED | EVAL_OWNED))),
            COLOR_SELECTION_ENEMY
        );
    }

    #[test]
    fn test_overlay_state_flags() {
        let a = Alliances::new(2);
        let m = owned();
        let st = |flags: u32| {
            overlay_state(
                &sp(),
                &a,
                OverlayObject {
                    flags,
                    monster: Some(&m),
                },
                None,
            )
        };
        let sel = OverlayState {
            transparency: TRANSPARENCY_SELECTED,
            frame: true,
        };
        let hov = OverlayState {
            transparency: TRANSPARENCY_HIGHLIGHTED,
            frame: true,
        };
        assert_eq!(st(0), None);
        assert_eq!(st(OBJECT_SELECTED), Some(sel));
        assert_eq!(st(OBJECT_TARGETED), Some(sel));
        assert_eq!(st(OBJECT_HOVERED), Some(hov));
        assert_eq!(st(OBJECT_HOVERED | OBJECT_SELECTED), Some(hov));
        assert_eq!(st(OBJECT_HOVERED | OBJECT_TARGETED), Some(sel));
        let obj = overlay_state(
            &sp(),
            &a,
            OverlayObject {
                flags: OBJECT_TARGETED,
                monster: None,
            },
            Some(CapsLock::SelectionBoxes),
        );
        assert_eq!(obj, Some(sel));
    }

    #[test]
    fn test_overlay_state_caps_lock() {
        let a = Alliances::new(2);
        let enemy = MonsterInterface { team: 1, ..owned() };
        let caps = |ctx: &GameContext, m: &MonsterInterface, mode| {
            overlay_state(
                ctx,
                &a,
                OverlayObject {
                    flags: 0,
                    monster: Some(m),
                },
                mode,
            )
        };
        let all = Some(OverlayState {
            transparency: TRANSPARENCY_HIGHLIGHTED,
            frame: true,
        });
        assert_eq!(caps(&sp(), &owned(), None), None);
        assert_eq!(caps(&sp(), &owned(), Some(CapsLock::SelectionBoxes)), all);
        assert_eq!(caps(&sp(), &enemy, Some(CapsLock::SelectionBoxes)), all);
        assert_eq!(caps(&sp(), &owned(), Some(CapsLock::HealthBars)), all);
        for mode in [
            CapsLock::Ignored,
            CapsLock::PlayerStats,
            CapsLock::ChatBacklog,
        ] {
            assert_eq!(caps(&sp(), &owned(), Some(mode)), None);
        }
        let net = GameContext {
            game_mode: 2,
            ..sp()
        };
        assert_eq!(caps(&net, &enemy, Some(CapsLock::SelectionBoxes)), None);
        assert_eq!(
            caps(&net, &owned(), Some(CapsLock::HealthBars)),
            Some(OverlayState {
                transparency: TRANSPARENCY_HIGHLIGHTED,
                frame: false
            })
        );
        let excluded = MonsterInterface {
            definition_class: MONS_CLASS_EXCLUDED,
            ..owned()
        };
        assert_eq!(caps(&net, &excluded, Some(CapsLock::SelectionBoxes)), None);
        let observer = GameContext {
            game_mode: 1,
            local_team: -1,
            ..sp()
        };
        assert_eq!(caps(&observer, &enemy, Some(CapsLock::SelectionBoxes)), all);
        assert_eq!(
            overlay_state(
                &sp(),
                &a,
                OverlayObject {
                    flags: 0,
                    monster: None
                },
                Some(CapsLock::SelectionBoxes)
            ),
            None
        );
    }

    #[test]
    fn test_frame_and_submerged() {
        let on = OverlayState {
            transparency: TRANSPARENCY_HIGHLIGHTED,
            frame: true,
        };
        let off = OverlayState { frame: false, ..on };
        assert!(draws_frame(on, true, 0));
        assert!(!draws_frame(on, false, OBJECT_SELECTED));
        assert!(draws_frame(on, false, OBJECT_TARGETED));
        assert!(!draws_frame(off, true, OBJECT_TARGETED));
        assert_eq!(overlay_transparency(on, false), TRANSPARENCY_HIGHLIGHTED);
        assert_eq!(overlay_transparency(on, true), TRANSPARENCY_SUBMERGED);
        assert_eq!(super::super::overlay_alpha(TRANSPARENCY_SUBMERGED), 0x2a);
    }

    #[test]
    fn test_outer_frame() {
        let vip = MonsterInterface {
            flags8: MONSTER_FLAGS8_OUTER_FRAME,
            ..owned()
        };
        assert!(!draws_outer_frame(&sp(), Some(&vip)));
        let g10 = GameContext {
            game_type: 10,
            ..sp()
        };
        let g11 = GameContext {
            game_type: 11,
            ..sp()
        };
        assert!(draws_outer_frame(&g10, Some(&vip)));
        assert!(draws_outer_frame(&g11, Some(&vip)));
        assert!(!draws_outer_frame(&g10, Some(&owned())));
        assert!(!draws_outer_frame(&g10, None));
        let net = GameContext {
            game_mode: 2,
            ..g10
        };
        assert!(!draws_outer_frame(
            &net,
            Some(&MonsterInterface { team: 1, ..vip })
        ));
        assert!(draws_outer_frame(&net, Some(&vip)));
        let r = PixelRect::new(10, 20, 50, 120);
        assert_eq!(
            outer_frame_edges(r, 2),
            [
                PixelRect::new(8, 18, 52, 20),
                PixelRect::new(8, 120, 52, 122),
                PixelRect::new(8, 20, 10, 120),
                PixelRect::new(50, 20, 52, 120),
            ]
        );
        assert_eq!(OUTER_FRAME_COLOR, [0xff, 0x7f, 0]);
        assert_eq!(super::super::overlay_alpha(OUTER_FRAME_TRANSPARENCY), 0xff);
    }

    #[test]
    fn test_hover_tracker() {
        let unit = |id, selected| {
            Some(HoverPick {
                id,
                selected,
                monster: true,
            })
        };
        let mut h = HoverTracker::default();
        h.update(250, true, || unit(7, false));
        assert_eq!(h.hovered(), None);
        h.update(251, true, || unit(7, false));
        assert_eq!(h.hovered(), Some(7));
        h.update(500, true, || None);
        assert_eq!(h.hovered(), Some(7));
        h.update(502, true, || unit(8, true));
        assert_eq!(h.hovered(), None, "selected monsters are not hovered");
        h.update(800, true, || {
            Some(HoverPick {
                id: 9,
                selected: false,
                monster: false,
            })
        });
        assert_eq!(h.hovered(), None, "only monsters are hovered");
        h.update(1100, true, || unit(9, false));
        assert_eq!(h.hovered(), Some(9));
        h.update(1400, false, || unit(9, false));
        assert_eq!(h.hovered(), None, "no pointer, no hover");
    }

    #[test]
    fn test_target_highlight() {
        let mut t = TargetHighlight::default();
        t.command(&[5, 6], &[6]);
        assert!(t.is_targeted(5));
        assert!(!t.is_targeted(6), "selected targets are not flagged");
        for _ in 0..6 {
            t.update(1);
            assert!(t.is_targeted(5));
        }
        t.update(1);
        assert!(!t.is_targeted(5));
        t.update(0);
        t.command(&[1], &[]);
        t.update(0);
        assert!(t.is_targeted(1), "a paused game keeps the highlight");
    }

    #[test]
    fn test_pick_up_target_frames_the_item() {
        let mut t = TargetHighlight::default();
        t.command(&[5], &[]);
        t.update(3);
        t.pick_up(960);
        assert!(t.is_object_targeted(960));
        assert!(!t.is_object_targeted(961));
        assert!(t.is_targeted(5), "earlier targets keep their flag");
        let flags = if t.is_object_targeted(960) {
            OBJECT_TARGETED
        } else {
            0
        };
        let a = Alliances::new(2);
        let state = overlay_state(
            &sp(),
            &a,
            OverlayObject {
                flags,
                monster: None,
            },
            None,
        )
        .expect("a targeted object gets the frame path");
        assert_eq!(state.transparency, TRANSPARENCY_SELECTED);
        assert!(
            draws_frame(state, false, flags),
            "no obje frame flag needed"
        );
        assert_eq!(frame_color_index(&sp(), None), COLOR_SELECTION_OBJECT);
        let none = OverlayObject {
            flags: 0,
            monster: None,
        };
        assert_eq!(overlay_state(&sp(), &a, none, None), None);
        for _ in 0..6 {
            t.update(1);
            assert!(t.is_object_targeted(960));
        }
        t.update(1);
        assert!(!t.is_object_targeted(960));
        assert!(!t.is_targeted(5));
        t.pick_up(7);
        t.command(&[1], &[]);
        assert!(!t.is_object_targeted(7));
    }
}
