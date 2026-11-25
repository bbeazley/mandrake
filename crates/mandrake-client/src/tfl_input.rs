use crate::local_input::PresetWalk;
use crate::move_marker::{LocalSprites, MarkerKind, MaskRule, RemoveMode};
use crate::overlays::MoveGesture;
use crate::picking::{GamePicks, PickScene, PickWorld};
#[cfg(test)]
use crate::selection_input::chebyshev;
use crate::selection_input::{
    self as si, ActionFlags, AttackGroundStep, InputAction, MonsterPick, Outcome, PressTarget,
    SelectionMachine, SelectionRules, State,
};
use crate::tfl_objects::{TFL_CELL_SIZE, WORLD_PER_RENDER_UNIT};
use mandrake_common::FourCC;
use mandrake_tag::MeshTag;
use mandrake_tfl_sim::TflSim;
use mandrake_view::camera::Camera;
use mandrake_view::selection::state::TargetHighlight;
use mandrake_view::selection::{ObjectBounds, PixelRect, projected_frame_rect};

const UNDER_MODEL: u8 = 0x40;
const SELECTABLE: u32 = 1;
const FRIENDLY: u32 = 2;
const OURS: u32 = 4;
const NOT_COMMANDED: u32 = 0x1000_0000;
const MOVE_CYCLE: u16 = 1;
const MOVE_BACK_AND_FORTH: u16 = 2;
const MOVE_FORMATION: u16 = 8;
const MOVE_FACING: u16 = 0x10;
const MOVE_RETURN_TO_CENTRE: u16 = 0x40;
const WAYPOINT_CLOSE: i32 = 0x400;
const MAX_WAYPOINTS: usize = 4;
const GROUP_SPREAD: i32 = 0xc00;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct MouseAction {
    pub(crate) time: u64,
    pub(crate) cursor: (i32, i32),
    pub(crate) left_held: bool,
    pub(crate) left_pressed: bool,
    pub(crate) double: bool,
    pub(crate) right_held: bool,
    pub(crate) right_pressed: bool,
    pub(crate) shift: bool,
    pub(crate) alt: bool,
    pub(crate) option: bool,
    pub(crate) control: bool,
    pub(crate) right_double: bool,
    pub(crate) attack_ground: bool,
    pub(crate) recall_preset: bool,
    pub(crate) number: Option<usize>,
    pub(crate) center_held: bool,
    pub(crate) sniffed_away: bool,
}

impl MouseAction {
    pub(crate) fn polled(builder: &mut si::ActionBuilder, time: u64, poll: si::PollState) -> Self {
        let f = builder.poll(poll).flags;
        Self {
            time,
            cursor: (i32::from(poll.cursor.0), i32::from(poll.cursor.1)),
            left_held: f.test(si::LEFT_HELD),
            left_pressed: f.test(si::LEFT_PRESSED),
            double: f.test(si::LEFT_DOUBLE),
            right_held: f.test(si::RIGHT_HELD),
            right_pressed: f.test(si::RIGHT_PRESSED),
            right_double: f.test(si::RIGHT_DOUBLE),
            shift: poll.shift,
            alt: poll.alt,
            option: poll.option,
            control: poll.control,
            attack_ground: poll.attack_ground,
            ..Self::default()
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) enum Mode {
    #[default]
    None,
    Presets,
    Formation,
    Attack,
    Special,
    Rename,
    ChatAll,
    ChatTeam,
    FormationKey,
    FormationKeyInPlace,
}

impl Mode {
    fn number(self) -> i16 {
        use crate::command_bar as bar;
        match self {
            Mode::None => bar::SELECTION_MODE_NORMAL,
            Mode::Presets => bar::SELECTION_MODE_PRESET,
            Mode::Formation => bar::SELECTION_MODE_FORMATION,
            Mode::Attack => bar::SELECTION_MODE_ATTACK_GROUND,
            Mode::Special => bar::SELECTION_MODE_SPECIAL,
            Mode::Rename => MODE_RENAME,
            Mode::ChatAll => MODE_CHAT_ALL,
            Mode::ChatTeam => MODE_CHAT_TEAM,
            Mode::FormationKey => bar::SELECTION_MODE_FORMATION_KEY,
            Mode::FormationKeyInPlace => bar::SELECTION_MODE_FORMATION_KEY_IN_PLACE,
        }
    }

    fn prompt_strings(self) -> Option<(usize, usize)> {
        match self {
            Mode::None | Mode::Formation | Mode::FormationKey | Mode::FormationKeyInPlace => None,
            m => {
                let n = m.number() as usize;
                Some((2 * n, 2 * n + 1))
            }
        }
    }
}

const MODE_RENAME: i16 = 4;
pub(crate) const RENAME_LENGTH: usize = 0x11 - 2;
const MODE_CHAT_TEAM: i16 = 5;
const MODE_CHAT_ALL: i16 = 6;
pub(crate) const CHAT_LENGTH: usize = 0x31 - 2;

fn chat_mode(shift: bool, alone: bool) -> Mode {
    if shift || alone {
        Mode::ChatAll
    } else {
        Mode::ChatTeam
    }
}

pub(crate) fn chat_payload(team_only: bool, text: &[u8]) -> Vec<u8> {
    let mut p = vec![0, u8::from(team_only)];
    let mut line = text[..text.len().min(0x30)].to_vec();
    line.resize(0x31, 0);
    p.extend(line);
    p
}

#[derive(Debug, Clone, Default)]
pub(crate) struct SelectionStrings {
    pub(crate) insm: Vec<Vec<u8>>,
    pub(crate) insp: Vec<Vec<u8>>,
    pub(crate) infn: Vec<Vec<u8>>,
}

impl SelectionStrings {
    pub(crate) fn of_map(map: &crate::tfl_map::TflMap) -> Self {
        let list = |id: &[u8; 4]| {
            map.tag(b"stli", FourCC(*id))
                .and_then(|d| {
                    mandrake_tag::tfl::TflStringList::from_tag(FourCC(*b"stli"), 1, &d).ok()
                })
                .map(|l| l.strings)
                .unwrap_or_default()
        };
        Self {
            insm: list(b"insm"),
            insp: list(b"insp"),
            infn: list(b"infn"),
        }
    }

    fn get(list: &[Vec<u8>], i: usize) -> Vec<u8> {
        list.get(i).cloned().unwrap_or_default()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum DetachStep {
    Nothing,
    To(i16),
    PickPlayer,
}

pub(crate) fn detach_step(
    all_ours: bool,
    team: i16,
    team_players: &[i16],
    local_player: i16,
) -> DetachStep {
    if !all_ours {
        return DetachStep::To(local_player);
    }
    if team == -1 {
        return DetachStep::Nothing;
    }
    match team_players.len() {
        1 => DetachStep::Nothing,
        2 => DetachStep::To(
            team_players
                .iter()
                .copied()
                .rfind(|&p| p != local_player)
                .unwrap_or(-1),
        ),
        _ => DetachStep::PickPlayer,
    }
}

const FORMATIONS: i16 = 10;

const TARGET_DOUBLE: u16 = 1;
const TARGET_SPECIAL: u16 = 2;
const ATTACK_LOCATION_SPECIAL: u16 = 1;
const MAX_SELECTED: usize = 0x1e;
const MAX_TARGETS: usize = 30;

pub(crate) struct View<'a> {
    pub(crate) camera: &'a Camera,
    pub(crate) size: (u32, u32),
    pub(crate) mesh: &'a MeshTag,
    pub(crate) world: &'a crate::world_view::WorldView,
    pub(crate) models: Option<&'a crate::tfl_models::ModelPick>,
}

impl View<'_> {
    fn half(&self) -> (f32, f32) {
        (
            self.mesh.cell_width as f32 * TFL_CELL_SIZE * 0.5,
            self.mesh.cell_height as f32 * TFL_CELL_SIZE * 0.5,
        )
    }

    fn ground(&self, x: f32, z: f32) -> f32 {
        let (hw, hd) = self.half();
        self.mesh
            .terrain_height_at((hw - x) / TFL_CELL_SIZE, (z + hd) / TFL_CELL_SIZE)
    }

    pub(crate) fn ground_point(&self, cursor: (i32, i32)) -> Option<(i32, i32)> {
        let (o, d) =
            self.camera
                .screen_ray(cursor.0 as f32, cursor.1 as f32, self.size.0, self.size.1);
        let hit = crate::picking::ray_terrain_intersection_scaled(o, d, self.mesh, TFL_CELL_SIZE)?;
        let (hw, hd) = self.half();
        let wx = (hw - hit[0]) * WORLD_PER_RENDER_UNIT;
        let wy = (hit[2] + hd) * WORLD_PER_RENDER_UNIT;
        Some((wx as i32, wy as i32))
    }

    fn surface_pick(&self, sim: &TflSim, cursor: (i32, i32)) -> Pick {
        let ground = self.ground_point(cursor);
        let model = self
            .models
            .and_then(|m| m.pick(self.camera, self.size, cursor, sim.tick));
        let land_key = ground.and_then(|g| self.landscape_key(sim, g));
        match (model, ground) {
            (Some(m), _) if land_key.is_none_or(|k| m.key < k) => Pick::Model(m.point),
            (_, Some(g)) => Pick::Ground(g),
            _ => Pick::Nothing,
        }
    }

    fn landscape_key(&self, sim: &TflSim, (x, y): (i32, i32)) -> Option<f32> {
        let (w, h) = (self.mesh.cell_width as i32, self.mesh.cell_height as i32);
        let (cx, cy) = (x >> 10, y >> 10);
        if cx < 0 || cy < 0 || cx >= w || cy >= h {
            return None;
        }
        if sim
            .cells
            .get((w * cy + cx) as usize)
            .is_some_and(|c| c.flags & UNDER_MODEL != 0)
        {
            return None;
        }
        let (tx, ty) = ((x & 0x3ff) as f32, (y & 0x3ff) as f32);
        let corners: [(i32, i32); 3] = if (cx ^ cy) & 1 == 0 {
            if tx >= ty {
                [(0, 0), (1, 0), (1, 1)]
            } else {
                [(0, 0), (0, 1), (1, 1)]
            }
        } else if tx + ty <= 1024.0 {
            [(0, 0), (1, 0), (0, 1)]
        } else {
            [(1, 1), (0, 1), (1, 0)]
        };
        let (hw, hd) = self.half();
        let view = self.camera.view_matrix().cols;
        corners
            .into_iter()
            .map(|(dx, dy)| {
                let rx = hw - (cx + dx) as f32 * TFL_CELL_SIZE;
                let rz = (cy + dy) as f32 * TFL_CELL_SIZE - hd;
                let q = [rx, self.ground(rx, rz), rz];
                -(view[0][2] * q[0] + view[1][2] * q[1] + view[2][2] * q[2] + view[3][2])
            })
            .reduce(f32::max)
    }

    pub(crate) fn frame(&self, sim: &TflSim, object: usize) -> Option<PixelRect> {
        let o = sim.objects.get(object)?;
        let drawn = i32::try_from(object).ok().and_then(|i| {
            let objects = &self.world.objects;
            let k = objects.binary_search_by_key(&i, |d| d.object).ok()?;
            Some(objects[k].world)
        });
        let bounds = ObjectBounds {
            half_width: o.radius.max(1),
            bottom: o.bottom,
            top: o.top.max(o.bottom + 1),
        };
        let at = drawn.map_or((o.x, o.y), |w| (w[0], w[1]));
        self.frame_at(at, drawn.map(|w| w[2]), bounds)
    }

    fn unit_frame(&self, pick: &crate::world_view::UnitPick) -> Option<PixelRect> {
        let b = pick.bounds;
        let bounds = ObjectBounds {
            half_width: b.half_width.max(1),
            bottom: b.bottom,
            top: b.top.max(b.bottom + 1),
        };
        self.frame_at((pick.world[0], pick.world[1]), Some(pick.world[2]), bounds)
    }

    fn frame_at(
        &self,
        (wx, wy): (i32, i32),
        height: Option<i32>,
        bounds: ObjectBounds,
    ) -> Option<PixelRect> {
        let (hw, hd) = self.half();
        let x = hw - wx as f32 / WORLD_PER_RENDER_UNIT;
        let z = wy as f32 / WORLD_PER_RENDER_UNIT - hd;
        let height = height.map_or_else(|| self.ground(x, z), |h| h as f32 / WORLD_PER_RENDER_UNIT);
        let at = [x, height, z];
        let proj = self.camera.frame_projection(at, self.size.0, self.size.1)?;
        let r = projected_frame_rect(bounds, &proj);
        (!r.is_empty()).then_some(r)
    }

    fn picks(&self) -> Vec<&crate::world_view::UnitPick> {
        let mut picks: Vec<_> = self
            .world
            .units
            .iter()
            .filter_map(|u| u.pick.as_ref())
            .filter(|p| !p.dying)
            .collect();
        picks.sort_by_key(|p| p.monster);
        picks
    }

    fn facing(&self) -> u16 {
        let f = (self.camera.yaw - std::f32::consts::FRAC_PI_2).rem_euclid(std::f32::consts::TAU);
        (f / std::f32::consts::TAU * 65536.0) as i64 as u16
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Pick {
    Nothing,
    Ground((i32, i32)),
    Model([i32; 3]),
    Monster(usize),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Order {
    pub(crate) flags: u16,
    pub(crate) formation: u16,
    pub(crate) facing: u16,
    pub(crate) monsters: Vec<i16>,
    pub(crate) waypoints: Vec<(i32, i32)>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct MarkerRequest {
    pub(crate) command: mandrake_tfl_sim::commands::MovementCommand,
    pub(crate) slots: Vec<usize>,
    pub(crate) waypoints: Vec<(i32, i32)>,
    pub(crate) kind: MarkerKind,
}

#[derive(Debug)]
pub(crate) struct TflSelection {
    pub(crate) machine: SelectionMachine,
    pub(crate) selected: Vec<usize>,
    pub(crate) waypoints: Vec<(i32, i32)>,
    pub(crate) band: Option<PixelRect>,
    pub(crate) gesture: Option<Flick>,
    known: Vec<(usize, i16)>,
    pub(crate) orders: Vec<mandrake_rec::CommandPacket>,
    pub(crate) markers: Option<MarkerRequest>,
    pub(crate) attack_marker: Option<[i32; 3]>,
    pub(crate) mode: Mode,
    attack_ground_was: bool,
    pub(crate) voices: Vec<(Vec<usize>, i8)>,
    unframed: crate::interface_state::OrderFlagHide,
    targets: TargetHighlight,
    drawn: Option<Vec<usize>>,
    pub(crate) formation: u16,
    pub(crate) clear_markers: bool,
    pub(crate) center: Option<bool>,
    pub(crate) alternate_recall: bool,
    pub(crate) preset_popup: Option<usize>,
    number_pressed: Option<usize>,
    pub(crate) prompts: (Vec<u8>, Vec<u8>),
    pub(crate) input: crate::console::ConsoleInput,
    pub(crate) messages: Vec<Vec<u8>>,
    pub(crate) strings: SelectionStrings,
    now: i32,
    pub(crate) chat_allowed: bool,
}

impl Default for TflSelection {
    fn default() -> Self {
        Self::with_rules(SelectionRules::TFL_MYTH2)
    }
}

impl TflSelection {
    pub(crate) fn with_rules(rules: SelectionRules) -> Self {
        Self {
            machine: SelectionMachine::with_rules(rules),
            selected: Vec::new(),
            waypoints: Vec::new(),
            band: None,
            gesture: None,
            known: Vec::new(),
            orders: Vec::new(),
            markers: None,
            attack_marker: None,
            mode: Mode::None,
            attack_ground_was: false,
            voices: Vec::new(),
            unframed: Default::default(),
            targets: TargetHighlight::default(),
            drawn: None,
            formation: 0,
            clear_markers: false,
            center: None,
            alternate_recall: false,
            preset_popup: None,
            number_pressed: None,
            prompts: (Vec::new(), Vec::new()),
            input: crate::console::ConsoleInput::default(),
            messages: Vec::new(),
            strings: SelectionStrings::default(),
            now: 0,
            chat_allowed: false,
        }
    }
}

#[derive(Debug, Default)]
pub(crate) struct FilmInterface {
    pub(crate) selection: TflSelection,
}

impl FilmInterface {
    pub(crate) fn new(rules: SelectionRules, strings: SelectionStrings) -> Self {
        let mut selection = TflSelection::with_rules(rules);
        selection.strings = strings;
        Self { selection }
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn poll(
        &mut self,
        sim: &TflSim,
        view: &View,
        mut hud: Option<&mut crate::tfl_hud::TflHud>,
        a: MouseAction,
        map_size: (i32, i32),
        map_cursor: (i32, i32),
        now: i32,
    ) -> Option<(i32, i32)> {
        let map = hud
            .as_deref_mut()
            .and_then(|h| h.map_mouse(map_size, a.left_held || a.left_pressed, map_cursor, now));
        if let Some(h) = hud.as_deref_mut() {
            h.take_sketch_commands();
        }
        let centre = match map {
            Some(crate::tfl_minimap::MapPoll::Centre(p)) => Some(p),
            Some(crate::tfl_minimap::MapPoll::Sketch) => None,
            None => {
                let s = &mut self.selection;
                let _ = s.update(sim, view, a);
                s.orders.clear();
                s.voices.clear();
                s.markers = None;
                s.attack_marker = None;
                s.clear_markers = false;
                s.preset_popup = None;
                s.messages.clear();
                None
            }
        };
        if let Some(h) = hud {
            h.update_selection(
                sim,
                &self.selection.selected,
                crate::tfl_hud_selection::Viewer::FILM,
            );
        }
        centre
    }
}

pub(crate) const VOICE_ATTACK: i8 = 0;
pub(crate) const VOICE_MOVE: i8 = 3;
pub(crate) const VOICE_SELECTED: i8 = 5;

pub(crate) fn monster_flags(sim: &TflSim, slot: usize) -> u32 {
    let m = &sim.monsters[slot];
    let local_team = sim.local_team().ok().flatten().unwrap_or(-1);
    let mut f = 0;
    if i16::from(m.team as i8) == local_team {
        if i16::from(m.player) == sim.local_player() {
            f = 5;
        }
        if sim.local_player_leads().unwrap_or(false) {
            f |= SELECTABLE;
        }
        if m.flags & NOT_COMMANDED != 0 || sim.game_mode == 0 {
            f &= !SELECTABLE;
        }
        f |= FRIENDLY;
    }
    f
}

fn preset_member(sim: &TflSim, id: u32) -> bool {
    (0..sim.monsters.len())
        .any(|s| live(sim, s) && u32::from(sim.monsters[s].identifier as u16) == id)
}

pub(crate) type ScreenView<'a> = (&'a Camera, (u32, u32), (u32, u32));

pub(crate) fn selection_focus(
    sim: &TflSim,
    selected: &[usize],
    zoom_factor: i32,
    view: Option<ScreenView<'_>>,
) -> Option<(i32, i32, i32)> {
    let local = sim.local_player();
    let units = selected.iter().filter(|&&s| live(sim, s)).filter_map(|&s| {
        let m = &sim.monsters[s];
        let o = sim.objects.get(m.object)?;
        Some(crate::local_input::FocusUnit {
            position: (o.x, o.y, o.z),
            facing: m.facing,
            ahead: i16::from(m.player) == local,
        })
    });
    let on_screen = view.map(|(camera, size, cells)| {
        let m = camera.view_proj_matrix(size.0 as f32 / size.1.max(1) as f32);
        move |p: (i32, i32, i32)| {
            let c = crate::tfl_objects::world_render_position([p.0, p.1, p.2], cells);
            let row = |r: usize| (0..3).map(|k| m.cols[k][r] * c[k]).sum::<f32>() + m.cols[3][r];
            let (x, y, w) = (row(0), row(1), row(3));
            w > 0.0 && x.abs() <= w && y.abs() <= w
        }
    });
    let test: Option<&dyn Fn((i32, i32, i32)) -> bool> = on_screen
        .as_ref()
        .map(|f| f as &dyn Fn((i32, i32, i32)) -> bool);
    crate::local_input::focus_of(units, zoom_factor, test)
}

pub(crate) fn live(sim: &TflSim, slot: usize) -> bool {
    sim.monsters
        .get(slot)
        .is_some_and(|m| m.identifier != 0 && m.flags & 0x4000_0000 == 0)
}

pub(crate) fn monster_under(view: &View, cursor: (i32, i32)) -> Option<usize> {
    let mut best: Option<(i32, usize)> = None;
    for p in view.picks() {
        let Some(r) = view.unit_frame(p) else {
            continue;
        };
        if cursor.0 >= r.left
            && cursor.0 < r.right
            && cursor.1 >= r.top
            && cursor.1 < r.bottom
            && best.is_none_or(|(b, _)| r.bottom > b)
        {
            best = Some((r.bottom, p.monster as usize));
        }
    }
    best.map(|(_, slot)| slot)
}

fn pick(sim: &TflSim, view: &View, cursor: (i32, i32)) -> Pick {
    match monster_under(view, cursor) {
        Some(slot) => Pick::Monster(slot),
        None => view.surface_pick(sim, cursor),
    }
}

pub(crate) struct TflPicks<'a> {
    sim: &'a TflSim,
    world: &'a crate::world_view::WorldView,
    models: Option<&'a crate::tfl_models::ModelPick>,
}

impl<'a> PickWorld<'a> {
    pub(crate) fn tfl(sim: &'a TflSim, view: &View<'a>) -> Self {
        Self {
            scene: PickScene {
                camera: view.camera,
                viewport: view.size,
                mesh_tag: view.mesh,
            },
            selection: Vec::new(),
            game: GamePicks::Tfl(TflPicks {
                sim,
                world: view.world,
                models: view.models,
            }),
        }
    }
}

fn on_screen(view: &View) -> Vec<usize> {
    view.picks()
        .into_iter()
        .filter(|p| {
            view.unit_frame(p).is_some_and(|r| {
                r.right > 0
                    && r.bottom > 0
                    && r.left < view.size.0 as i32
                    && r.top < view.size.1 as i32
            })
        })
        .map(|p| p.monster as usize)
        .collect()
}

fn cursor_of(c: (i16, i16)) -> (i32, i32) {
    (i32::from(c.0), i32::from(c.1))
}

impl<'a> TflPicks<'a> {
    fn view(&self, scene: &PickScene<'a>) -> View<'a> {
        View {
            camera: scene.camera,
            size: scene.viewport,
            mesh: scene.mesh_tag,
            world: self.world,
            models: self.models,
        }
    }

    pub(crate) fn surface(&self, scene: &PickScene<'a>, cursor: (i16, i16)) -> PressTarget {
        match self.view(scene).surface_pick(self.sim, cursor_of(cursor)) {
            Pick::Ground(point) => PressTarget::Ground { point },
            Pick::Model(p) => PressTarget::Model {
                point: (p[0], p[1], p[2]),
            },
            Pick::Nothing | Pick::Monster(_) => PressTarget::Nothing,
        }
    }

    pub(crate) fn press_target(&self, scene: &PickScene<'a>, cursor: (i16, i16)) -> PressTarget {
        let view = self.view(scene);
        match pick(self.sim, &view, cursor_of(cursor)) {
            Pick::Monster(slot) => PressTarget::Unit(MonsterPick {
                id: slot as u32,
                evaluation: view
                    .picks()
                    .into_iter()
                    .find(|p| p.monster as usize == slot)
                    .map_or(0, |p| p.evaluation),
                ..MonsterPick::default()
            }),
            Pick::Ground(point) => PressTarget::Ground { point },
            Pick::Model(p) => PressTarget::Model {
                point: (p[0], p[1], p[2]),
            },
            Pick::Nothing => PressTarget::Nothing,
        }
    }

    pub(crate) fn band_units(&self, scene: &PickScene<'a>, band: PixelRect) -> Vec<u32> {
        let view = self.view(scene);
        view.picks()
            .into_iter()
            .filter(|p| {
                u32::from(p.evaluation) & SELECTABLE != 0
                    && view.unit_frame(p).is_some_and(|r| {
                        r.left < band.right
                            && r.right > band.left
                            && r.top < band.bottom
                            && r.bottom > band.top
                    })
            })
            .map(|p| p.monster)
            .collect()
    }

    pub(crate) fn pick_up_target(
        &self,
        scene: &PickScene<'a>,
        cursor: (i16, i16),
        selection: &[u32],
    ) -> Option<(i16, i16)> {
        use crate::world_view::pick::PickSim;
        let (sim, cursor, view) = (self.sim, cursor_of(cursor), self.view(scene));
        let mut best: Option<(i32, usize, i16)> = None;
        for (o, ob) in sim.objects.iter().enumerate() {
            if ob.identifier == 0 || ob.kind == 2 {
                continue;
            }
            let Some(r) = view.frame(sim, o) else {
                continue;
            };
            if cursor.0 >= r.left
                && cursor.0 < r.right
                && cursor.1 >= r.top
                && cursor.1 < r.bottom
                && best.is_none_or(|(b, _, _)| r.bottom > b)
                && let Some(identifier) = sim.could_pick_up(selection, o as i32)
            {
                best = Some((r.bottom, o, identifier));
            }
        }
        best.map(|(_, o, identifier)| (o as i16, identifier))
    }

    pub(crate) fn same_type_flood(&self, scene: &PickScene<'a>, base: u32) -> Vec<u32> {
        let view = self.view(scene);
        let def = self.sim.monsters[base as usize].definition;
        view.picks()
            .into_iter()
            .filter(|p| {
                p.definition == def
                    && u32::from(p.evaluation) & SELECTABLE != 0
                    && view.unit_frame(p).is_some_and(|r| {
                        r.right > 0
                            && r.bottom > 0
                            && r.left < view.size.0 as i32
                            && r.top < view.size.1 as i32
                    })
            })
            .take(30)
            .map(|p| p.monster)
            .collect()
    }

    pub(crate) fn gesture_point(
        &self,
        scene: &PickScene<'a>,
        cursor: (i16, i16),
    ) -> Option<(i32, i32)> {
        match self.surface(scene, cursor) {
            PressTarget::Ground { point } => Some(point),
            PressTarget::Model { point } => Some((point.0, point.1)),
            _ => None,
        }
    }

    pub(crate) fn unit_location(&self, slot: u32) -> Option<(i32, i32)> {
        let m = self.sim.monsters.get(slot as usize)?;
        let o = self.sim.objects.get(m.object)?;
        Some((o.x, o.y))
    }

    pub(crate) fn gesture_facing(
        &self,
        scene: &PickScene<'a>,
        from: (i16, i16),
        to: (i16, i16),
    ) -> u16 {
        let (from, to) = (cursor_of(from), cursor_of(to));
        let drag = mandrake_tfl_sim::trig::arctangent(-(to.1 - from.1), to.0 - from.0);
        self.view(scene).facing().wrapping_sub(drag)
    }
}

fn input_action(a: &MouseAction, attack_ground_was: bool, modifiers: bool) -> InputAction {
    let mut flags = ActionFlags::default();
    for (on, flag) in [
        (modifiers && a.option, si::OPTION),
        (modifiers && a.control, si::CONTROL),
        (modifiers && a.right_double, si::RIGHT_DOUBLE),
        (a.left_held, si::LEFT_HELD),
        (a.left_pressed, si::LEFT_PRESSED),
        (a.double, si::LEFT_DOUBLE),
        (a.right_held, si::RIGHT_HELD),
        (a.right_pressed, si::RIGHT_PRESSED),
        (a.shift, si::SHIFT),
        (a.alt, si::ALT),
        (a.attack_ground, si::ATTACK_GROUND_HELD),
        (
            a.attack_ground && !attack_ground_was,
            si::ATTACK_GROUND_PRESSED,
        ),
    ] {
        if on {
            flags.set(flag);
        }
    }
    let clamp = |v: i32| v.clamp(i32::from(i16::MIN), i32::from(i16::MAX)) as i16;
    InputAction {
        time: a.time as i32,
        elapsed: 1,
        cursor: (clamp(a.cursor.0), clamp(a.cursor.1)),
        flags,
    }
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Flick {
    pub(crate) gesture: MoveGesture,
    pub(crate) units: Vec<[f32; 3]>,
}

pub(crate) fn render_yaw(facing: u16) -> f32 {
    std::f32::consts::PI - f32::from(facing) * std::f32::consts::TAU / 65536.0
}

impl TflSelection {
    fn flick(&self, sim: &TflSim, view: &View) -> Option<Flick> {
        let (_, cursor) = self.machine.held_gesture()?;
        let facing = self
            .machine
            .gesture_facing(&mut PickWorld::tfl(sim, view), cursor);
        let place = |slot: usize| {
            view.world
                .units
                .iter()
                .find(|u| u.pick.as_ref().is_some_and(|p| p.monster as usize == slot))
                .map(|u| u.position)
        };
        let (origin, on_selection) = match self.machine.press_pick() {
            si::PressPick::Ground((x, y)) => {
                let (hw, hd) = view.half();
                let (rx, rz) = (
                    hw - x as f32 / WORLD_PER_RENDER_UNIT,
                    y as f32 / WORLD_PER_RENDER_UNIT - hd,
                );
                ([rx, view.ground(rx, rz), rz], false)
            }
            si::PressPick::Unit(slot) => (place(slot as usize)?, true),
            si::PressPick::Other => return None,
        };
        let units = if on_selection {
            self.selected.iter().filter_map(|&s| place(s)).collect()
        } else {
            Vec::new()
        };
        Some(Flick {
            gesture: MoveGesture {
                origin,
                on_selection,
                facing: Some(render_yaw(facing)),
            },
            units,
        })
    }

    fn viewing(&self) -> bool {
        self.machine.looking()
    }

    fn commandable(&self) -> bool {
        !self.selected.is_empty() && !self.viewing()
    }

    fn clear(&mut self) {
        self.selected.clear();
        self.machine.stop_looking();
    }

    fn identifiers(&self, sim: &TflSim) -> Vec<i16> {
        self.selected
            .iter()
            .map(|&s| sim.monsters[s].identifier)
            .collect()
    }

    pub(crate) fn key(&mut self, sim: &TflSim, order: keys::KeyOrder) {
        if self.input.typing() {
            return;
        }
        if self.preset_mode_skips_keys() && order.skipped_in_mode_0() {
            return;
        }
        self.selected.retain(|&s| live(sim, s));
        let given = self.orders.len();
        self.key_order(sim, order);
        if self.orders.len() > given {
            self.after_command();
        }
    }

    fn key_order(&mut self, sim: &TflSim, order: keys::KeyOrder) {
        match order {
            keys::KeyOrder::SelectNothing => {
                let mut selection = self.selected_ids();
                self.machine.clear_selection(&mut selection);
                self.selected = selection.into_iter().map(|id| id as usize).collect();
                self.selection_changed();
            }
            keys::KeyOrder::SelectAll => self.select_all(sim),
            keys::KeyOrder::CenterSelection => self.center = Some(false),
            keys::KeyOrder::RotatePreset { backward } => self.rotate_preset(sim, backward),
            keys::KeyOrder::ClearPreset => self.clear_preset(sim),
            keys::KeyOrder::Detach => {
                self.clear_orders();
                if self.commandable() {
                    self.detach(sim);
                }
            }
            keys::KeyOrder::Rename => {
                self.clear_orders();
                if self.commandable() {
                    self.input.start(RENAME_LENGTH, self.now);
                    self.set_mode(Mode::Rename);
                }
            }
            keys::KeyOrder::Chat { shift } => {
                if self.chat_allowed {
                    self.clear_orders();
                    let team = sim.local_team().ok().flatten().unwrap_or(-1);
                    let alone = team < 0 || sim.team_players(team).len() == 1;
                    self.input.start(CHAT_LENGTH, self.now);
                    self.set_mode(chat_mode(shift, alone));
                }
            }
            _ if !self.commandable() => {}
            keys::KeyOrder::General(keys::GENERAL_SPECIAL) => self.special(sim),
            keys::KeyOrder::General(code) => self.orders.push(crate::tfl_live::film::command(
                crate::tfl_live::film::GENERAL_COMMAND,
                &crate::tfl_live::film::general_payload(code, &self.identifiers(sim)),
            )),
        }
    }

    fn detach(&mut self, sim: &TflSim) {
        let local = sim.local_player();
        let all_ours = self
            .selected
            .iter()
            .all(|&s| monster_flags(sim, s) & OURS != 0);
        let team = sim.local_team().ok().flatten().unwrap_or(-1);
        let players = sim.team_players(team);
        match detach_step(all_ours, team, &players, local) {
            DetachStep::To(player) => {
                self.orders.push(crate::tfl_live::film::command(
                    crate::tfl_live::film::DETACH_COMMAND,
                    &crate::tfl_live::film::detach_payload(player, &self.identifiers(sim)),
                ));
                if player != local {
                    self.select_nothing();
                }
            }
            DetachStep::Nothing => {
                println!("TFL game: detach: every monster is ours already")
            }
            DetachStep::PickPlayer => {
                tracing::error!("TFL game: detach: the team's player pick (mode 7) is not ported")
            }
        }
    }

    fn clear_orders(&mut self) {
        self.clear_markers = true;
        self.waypoints.clear();
    }

    fn set_mode(&mut self, mode: Mode) {
        self.mode = mode;
        self.prompts = match mode.prompt_strings() {
            Some((a, b)) => (
                SelectionStrings::get(&self.strings.insm, a),
                SelectionStrings::get(&self.strings.insm, b),
            ),
            None if mode == Mode::Formation => (
                SelectionStrings::get(&self.strings.insm, 2),
                SelectionStrings::get(&self.strings.infn, usize::from(self.formation)),
            ),
            None => (Vec::new(), Vec::new()),
        };
    }

    fn selection_changed(&mut self) {
        let c = self.machine.take_changes();
        if !self.machine.rules.selection_clears_orders {
            return;
        }
        if c.nothing {
            self.end_mode();
        } else if c.adjusted {
            self.clear_orders();
        }
    }

    pub(crate) fn text_key(&mut self, key: u8, ch: Option<u8>) -> bool {
        self.input.key(key, ch, |_| true, self.now)
    }

    fn select_all(&mut self, sim: &TflSim) {
        let eligible = |s: usize| {
            live(sim, s)
                && i16::from(sim.monsters[s].player) == sim.local_player()
                && sim.monsters[s].flags & NOT_COMMANDED == 0
        };
        let drawn = self
            .drawn
            .as_ref()
            .filter(|_| self.machine.rules.select_all_drawn);
        let ours: Vec<usize> = match drawn {
            Some(d) => d
                .iter()
                .copied()
                .filter(|&s| eligible(s))
                .take(MAX_SELECTED)
                .collect(),
            None => (0..sim.monsters.len())
                .filter(|&s| eligible(s))
                .take(MAX_SELECTED)
                .collect(),
        };
        let all_in = ours.iter().all(|s| self.selected.contains(s));
        self.select_nothing();
        if !all_in {
            self.selected = ours;
        }
    }

    pub(crate) fn forget_removed(&mut self, sim: &TflSim) {
        self.forget_removed_by(|s| {
            sim.monsters
                .get(s)
                .filter(|_| live(sim, s))
                .map(|m| m.identifier)
        });
    }

    fn forget_removed_by(&mut self, monster: impl Fn(usize) -> Option<i16>) {
        let known = std::mem::take(&mut self.known);
        let same = |s: usize| {
            monster(s).is_some_and(|now| {
                known
                    .iter()
                    .find(|&&(k, _)| k == s)
                    .is_none_or(|&(_, id)| now == id)
            })
        };
        self.selected.retain(|&s| same(s));
        self.machine.forget_units(|id| same(id as usize));
        self.remember_by(monster);
    }

    fn remember(&mut self, sim: &TflSim) {
        self.remember_by(|s| sim.monsters.get(s).map(|m| m.identifier));
    }

    fn remember_by(&mut self, monster: impl Fn(usize) -> Option<i16>) {
        self.known = self
            .selected
            .iter()
            .copied()
            .chain(self.machine.held_units().map(|id| id as usize))
            .filter_map(|s| monster(s).map(|id| (s, id)))
            .collect();
    }

    pub(crate) fn update(&mut self, sim: &TflSim, view: &View, a: MouseAction) -> Option<Order> {
        self.now = a.time as i32;
        if self.machine.rules.select_all_drawn {
            self.drawn = Some(on_screen(view));
        }
        self.unframed.update(1);
        self.forget_removed(sim);
        let given = self.orders.len();
        let typing = self.input.typing() && !(a.left_pressed || a.right_pressed);
        if a.left_pressed || a.right_pressed {
            self.input.click();
        }
        if typing {
            self.machine.presets.drop_wheel();
        }
        let order = if typing {
            None
        } else {
            self.update_mouse(sim, view, a)
        };
        if order.is_some() || self.orders.len() > given {
            self.after_command();
        }
        self.remember(sim);
        self.flash_poll();
        self.console_poll(sim);
        order
    }

    fn console_poll(&mut self, sim: &TflSim) {
        use crate::console::InputState;
        let state = self.input.state;
        if !matches!(state, InputState::Entered | InputState::Cancelled) {
            return;
        }
        let chat = match self.mode {
            Mode::ChatAll => Some(false),
            Mode::ChatTeam => Some(true),
            _ => None,
        };
        if state == InputState::Entered
            && let Some(team_only) = chat
            && !self.input.text.is_empty()
        {
            self.orders.push(crate::tfl_live::film::command(
                mandrake_tfl_sim::net_game::CHAT,
                &chat_payload(team_only, &self.input.text),
            ));
        }
        if state == InputState::Entered
            && self.mode == Mode::Rename
            && self.commandable()
            && !self.input.text.is_empty()
        {
            self.orders.push(crate::tfl_live::film::command(
                crate::tfl_live::film::RENAME_COMMAND,
                &crate::tfl_live::film::rename_payload(&self.identifiers(sim), &self.input.text),
            ));
        }
        self.end_mode();
    }

    fn flash_poll(&mut self) {
        self.targets.update(1);
    }

    fn after_command(&mut self) {
        let slots: Vec<u32> = self.selected.iter().map(|&s| s as u32).collect();
        self.unframed.order(&slots);
        if self.mode == Mode::Formation {
            self.set_mode(Mode::None);
        }
    }

    fn preset_poll(&self, mouse_busy: bool) -> crate::local_input::PresetPoll {
        crate::local_input::PresetPoll {
            elapsed: 0,
            recall_held: false,
            number: None,
            mode: self.mode.number(),
            alternate: self.alternate_recall,
            mouse_busy,
            selection_empty: self.selected.is_empty(),
            look: self.viewing(),
            replay: false,
        }
    }

    pub(crate) fn number_key(
        &mut self,
        sim: &TflSim,
        number: usize,
        shift: bool,
        mouse_busy: bool,
    ) -> bool {
        if mouse_busy {
            return false;
        }
        if self.mode == Mode::Presets && !self.alternate_recall {
            self.number_pressed = Some(number);
            return false;
        }
        self.selected.retain(|&s| live(sim, s));
        let p = self.preset_poll(mouse_busy);
        for effect in self.machine.presets.number_press(number, shift, p) {
            self.preset_effect(sim, effect);
        }
        std::mem::take(&mut self.clear_markers)
    }

    pub(crate) fn preset_mode_skips_keys(&self) -> bool {
        self.mode == Mode::Presets && !self.alternate_recall
    }

    fn formation_key(&mut self, formation: i16) -> bool {
        if !(0..FORMATIONS).contains(&formation) {
            return false;
        }
        self.formation = formation as u16;
        self.waypoints.clear();
        if self.commandable() {
            self.set_mode(Mode::Formation);
        }
        true
    }

    fn highlight_targets(&mut self, object: Option<usize>, targets: &[usize]) {
        match object {
            Some(o) => self.targets.pick_up(o as i32),
            None => {
                let targets: Vec<u32> = targets.iter().map(|&t| t as u32).collect();
                self.targets.command(&targets, &self.selected_ids());
            }
        }
    }

    pub(crate) fn framed_monsters(&self) -> Vec<usize> {
        let mut out: Vec<usize> = self
            .selected
            .iter()
            .copied()
            .filter(|&s| !self.unframed.hides(s as u32))
            .collect();
        for &t in self.targets.targets() {
            if !out.contains(&(t as usize)) {
                out.push(t as usize);
            }
        }
        out
    }

    pub(crate) fn framed_object(&self) -> Option<usize> {
        self.targets.object().map(|o| o as usize)
    }

    fn update_mouse(&mut self, sim: &TflSim, view: &View, a: MouseAction) -> Option<Order> {
        let action = input_action(
            &a,
            self.attack_ground_was,
            self.machine.rules.modifier_flags,
        );
        let double = self.machine.double_click(&action);
        let right = if double {
            AttackGroundStep::None
        } else {
            self.machine
                .attack_ground_right_press(&action, &self.selected_ids(), false)
        };
        if right != AttackGroundStep::None {
            self.clear_orders();
        }
        if right == AttackGroundStep::Enter && self.commandable() {
            self.set_mode(Mode::Attack);
        }
        let (held, held_right) = if self.machine.state() == State::Idle {
            (a.left_pressed, a.right_pressed)
        } else {
            (a.left_held, a.right_held)
        };
        let buttonless = !held && !held_right;
        if !double && !buttonless && self.mode == Mode::Presets {
            self.machine.consume();
            self.end_mode();
            self.attack_ground_was = a.attack_ground;
            return None;
        }
        self.machine.context.selection_mode = self.mode.number();
        let mut selection = self.selected_ids();
        let outcome = self
            .machine
            .update(&action, &mut PickWorld::tfl(sim, view), &mut selection);
        self.selected = selection.into_iter().map(|id| id as usize).collect();
        self.band = self.machine.band_bounds();
        self.gesture = self.flick(sim, view);
        self.selection_changed();
        let order = self.outcome(sim, outcome, &a);
        let keys = !double && buttonless;
        if keys {
            if !self.presets_poll(sim, &a) {
                self.attack_mode(&a);
            }
        } else {
            self.machine.presets.double_tap.poll(1);
        }
        self.keys_poll(sim, &a, keys && !a.sniffed_away);
        self.attack_ground_was = a.attack_ground;
        order
    }

    fn keys_poll(&mut self, sim: &TflSim, a: &MouseAction, reached: bool) {
        if !reached || self.preset_mode_skips_keys() {
            self.machine.presets.drop_wheel();
            return;
        }
        let selection = self.selected_ids_by_identifier(sim);
        let look = self.viewing();
        let member = |id| preset_member(sim, id);
        let effects =
            self.machine
                .presets
                .wheel_poll_by(PresetWalk::Members(&member), &selection, look);
        for effect in effects {
            self.preset_effect(sim, effect);
        }
        if a.center_held {
            self.center = Some(false);
        }
    }

    fn end_mode(&mut self) {
        self.clear_orders();
        self.input.stop();
        self.set_mode(Mode::None);
    }

    fn presets_poll(&mut self, sim: &TflSim, a: &MouseAction) -> bool {
        let p = crate::local_input::PresetPoll {
            elapsed: 1,
            recall_held: a.recall_preset,
            number: a.number.or(self.number_pressed.take()),
            ..self.preset_poll(false)
        };
        for effect in self.machine.presets.poll(p) {
            self.preset_effect(sim, effect);
        }
        self.machine.presets.took_poll()
    }

    fn preset_effect(&mut self, sim: &TflSim, effect: crate::local_input::PresetEffect) {
        use crate::local_input::PresetEffect as E;
        match effect {
            E::Recall { preset, toggle } => {
                self.recall_preset(sim, preset, toggle);
                println!(
                    "TFL game: preset {preset} recalled{}: {:?}",
                    if toggle { " (toggled)" } else { "" },
                    self.identifiers(sim)
                );
            }
            E::Create { preset } => {
                let ids = self.selected_ids_by_identifier(sim);
                let c = self.machine.presets.create(preset, &ids);
                println!("TFL game: preset {} set: {:?}", c.index, c.identifiers);
                self.preset_popup = Some(preset);
            }
            E::SelectNothing => self.select_nothing(),
            E::Center { in_view_test } => self.center = Some(in_view_test),
            E::ClearSprites => self.clear_orders(),
            E::EnterMode(mode) => {
                use crate::command_bar as bar;
                let mode = match mode {
                    bar::SELECTION_MODE_FORMATION_KEY => Mode::FormationKey,
                    bar::SELECTION_MODE_FORMATION_KEY_IN_PLACE => Mode::FormationKeyInPlace,
                    _ => Mode::Presets,
                };
                self.set_mode(mode);
                println!("TFL game: {mode:?}");
            }
            E::ExitMode => self.end_mode(),
            E::Formation { formation, .. } => {
                let keyed = matches!(self.mode, Mode::FormationKey | Mode::FormationKeyInPlace);
                if self.formation_key(formation) {
                    self.clear_markers = true;
                }
                if keyed && self.mode != Mode::Formation {
                    self.set_mode(Mode::None);
                }
            }
            E::AllyPick { .. } => {}
        }
    }

    fn select_nothing(&mut self) {
        self.clear();
        self.end_mode();
    }

    fn recall_preset(&mut self, sim: &TflSim, preset: usize, toggle: bool) {
        if !toggle {
            self.select_nothing();
        }
        let before = self.selected.clone();
        let presets = &self.machine.presets;
        presets.recall_with(preset, toggle, MAX_SELECTED, &mut self.selected, |id| {
            (0..sim.monsters.len())
                .find(|&s| live(sim, s) && u32::from(sim.monsters[s].identifier as u16) == id)
                .map(|s| (s, true))
        });
        if toggle && self.selected != before {
            self.clear_orders();
        }
    }

    fn rotate_preset(&mut self, sim: &TflSim, backward: bool) {
        let selection = self.selected_ids_by_identifier(sim);
        let p = self.preset_poll(false);
        let member = |id| preset_member(sim, id);
        let effects = self.machine.presets.rotate_press_by(
            PresetWalk::Members(&member),
            backward,
            &selection,
            p,
        );
        for effect in effects {
            self.preset_effect(sim, effect);
        }
    }

    fn clear_preset(&mut self, sim: &TflSim) {
        let selection = self.selected_ids_by_identifier(sim);
        let look = self.viewing();
        let member = |id| preset_member(sim, id);
        let effects =
            self.machine
                .presets
                .clear_press_by(PresetWalk::Members(&member), &selection, look);
        if !effects.is_empty() {
            println!("TFL game: preset cleared");
        }
        for effect in effects {
            self.preset_effect(sim, effect);
        }
    }

    fn selected_ids_by_identifier(&self, sim: &TflSim) -> Vec<u32> {
        self.identifiers(sim)
            .into_iter()
            .map(|id| u32::from(id as u16))
            .collect()
    }

    fn selected_ids(&self) -> Vec<u32> {
        self.selected.iter().map(|&s| s as u32).collect()
    }

    fn outcome(&mut self, sim: &TflSim, outcome: Outcome, a: &MouseAction) -> Option<Order> {
        match outcome {
            Outcome::SelectOnly(id) | Outcome::Added(id) => {
                self.voices.push((vec![id as usize], VOICE_SELECTED));
            }
            Outcome::Attack { target } => {
                let flags = if self.mode == Mode::Special {
                    TARGET_SPECIAL
                } else {
                    0
                };
                let attackers = self.selected.clone();
                self.target_command(sim, target as usize, &attackers, flags, a.shift);
                self.voices.push((self.selected.clone(), VOICE_ATTACK));
                self.set_mode(Mode::None);
            }
            Outcome::AttackAgain {
                attackers,
                target,
                shift,
            } => {
                let target = target as usize;
                let attackers: Vec<usize> = attackers
                    .into_iter()
                    .map(|s| s as usize)
                    .filter(|&s| live(sim, s))
                    .collect();
                if live(sim, target) && !attackers.is_empty() {
                    self.target_command(sim, target, &attackers, TARGET_DOUBLE, shift);
                }
            }
            Outcome::AttackGround { point, special } => {
                let flags = if special { ATTACK_LOCATION_SPECIAL } else { 0 };
                let z = attack_height(sim, point);
                self.orders.push(crate::tfl_live::film::command(
                    crate::tfl_live::film::ATTACK_LOCATION_COMMAND,
                    &crate::tfl_live::film::attack_location_payload(
                        flags,
                        (point.0, point.1, z),
                        &self.identifiers(sim),
                    ),
                ));
                self.attack_marker = Some([point.0, point.1, z]);
                self.set_mode(Mode::Attack);
            }
            Outcome::AttackModel { point, special } => {
                let flags = if special { ATTACK_LOCATION_SPECIAL } else { 0 };
                self.orders.push(crate::tfl_live::film::command(
                    crate::tfl_live::film::ATTACK_LOCATION_COMMAND,
                    &crate::tfl_live::film::attack_location_payload(
                        flags,
                        point,
                        &self.identifiers(sim),
                    ),
                ));
                self.set_mode(if self.machine.rules.model_attack_keeps_mode {
                    Mode::Attack
                } else {
                    Mode::None
                });
            }
            Outcome::PickUp {
                index, identifier, ..
            } => {
                self.highlight_targets(Some(index as usize), &[]);
                self.orders.push(crate::tfl_live::film::command(
                    crate::tfl_live::film::PICK_UP_COMMAND,
                    &crate::tfl_live::film::pick_up_payload(
                        index,
                        identifier,
                        &self.identifiers(sim),
                    ),
                ));
            }
            Outcome::Move {
                point,
                facing,
                add_waypoint,
            } => return self.movement(sim, point, facing, add_waypoint),
            Outcome::UnitOrder { facing } => return self.unit_order(sim, facing),
            Outcome::Ignored | Outcome::None | Outcome::Released(_) => {}
        }
        None
    }

    fn unit_order(&mut self, sim: &TflSim, facing: Option<u16>) -> Option<Order> {
        let (flags, facing, waypoints) = match facing {
            Some(f) => {
                self.waypoints.clear();
                (MOVE_FACING, f, Vec::new())
            }
            None if self.waypoints.is_empty() => return None,
            None => (
                MOVE_RETURN_TO_CENTRE | MOVE_CYCLE,
                0,
                std::mem::take(&mut self.waypoints),
            ),
        };
        self.clear_markers = true;
        Some(Order {
            flags,
            formation: 0,
            facing,
            monsters: self.identifiers(sim),
            waypoints,
        })
    }

    fn movement(
        &mut self,
        sim: &TflSim,
        point: (i32, i32),
        gesture: Option<u16>,
        add_waypoint: bool,
    ) -> Option<Order> {
        let mut flags = 0u16;
        let mut facing = 0u16;
        if let Some(f) = gesture {
            flags |= MOVE_FACING;
            facing = f;
        }
        let near = self
            .waypoints
            .iter()
            .position(|w| distance(*w, point) < WAYPOINT_CLOSE);
        let send = match near {
            Some(0) => {
                flags |= MOVE_CYCLE;
                true
            }
            Some(_) => {
                flags |= MOVE_CYCLE | MOVE_BACK_AND_FORTH;
                true
            }
            None => {
                if self.waypoints.len() < MAX_WAYPOINTS {
                    self.waypoints.push(point);
                }
                !add_waypoint
            }
        };
        let word = self.formation_word(sim);
        if word.is_some() {
            flags |= MOVE_FORMATION;
        }
        let formation = word.unwrap_or(0);
        self.markers = Some(MarkerRequest {
            command: mandrake_tfl_sim::commands::MovementCommand {
                flags,
                formation,
                facing,
            },
            slots: self.selected.clone(),
            waypoints: self.waypoints.clone(),
            kind: if send {
                MarkerKind::Order
            } else {
                MarkerKind::Preview
            },
        });
        if !send {
            return None;
        }
        let order = Order {
            flags,
            formation,
            facing,
            monsters: self.identifiers(sim),
            waypoints: std::mem::take(&mut self.waypoints),
        };
        self.voices.push((self.selected.clone(), VOICE_MOVE));
        Some(order)
    }

    fn target_command(
        &mut self,
        sim: &TflSim,
        target: usize,
        attackers: &[usize],
        flags: u16,
        shift: bool,
    ) {
        let kind = |s: usize| {
            sim.data
                .peek_def(b"mons", sim.monsters[s].definition)
                .map(|d| (d.i16(0x146), d.i16(0x1fc)))
        };
        let mut many = attackers.len() > 2;
        if !shift && attackers.iter().any(|&s| kind(s).is_some_and(|k| k.0 == 1)) {
            many = false;
        }
        let targets = if many {
            let rule = if shift {
                Near::NotPlayer(sim.monsters[attackers[0]].player)
            } else {
                Near::Same(sim.monsters[target].player, kind(target).map_or(0, |k| k.1))
            };
            near_targets(sim, target, rule)
        } else {
            vec![target]
        };
        let ids = |slots: &[usize]| -> Vec<i16> {
            slots.iter().map(|&s| sim.monsters[s].identifier).collect()
        };
        self.orders.push(crate::tfl_live::film::command(
            crate::tfl_live::film::TARGET_COMMAND,
            &crate::tfl_live::film::target_payload(flags, &ids(attackers), &ids(&targets)),
        ));
        self.highlight_targets(None, &targets);
    }

    fn special(&mut self, sim: &TflSim) {
        let mut kind = -1;
        for &s in &self.selected {
            let k = special_kind(sim, s);
            if kind != -1 && k != -1 && k != kind {
                kind = -1;
                break;
            }
            if k != -1 {
                kind = k;
            }
        }
        match kind {
            0 => self.orders.push(crate::tfl_live::film::command(
                crate::tfl_live::film::GENERAL_COMMAND,
                &crate::tfl_live::film::general_payload(
                    keys::GENERAL_SPECIAL,
                    &self.identifiers(sim),
                ),
            )),
            1 => self.set_mode(Mode::Special),
            _ => {}
        }
    }

    fn attack_mode(&mut self, a: &MouseAction) {
        let action = input_action(a, self.attack_ground_was, self.machine.rules.modifier_flags);
        if self.mode == Mode::Attack
            && self.machine.attack_ground_countdown(&action) == AttackGroundStep::Exit
        {
            self.end_mode();
        }
        let step = self
            .machine
            .attack_ground_key(&action, &self.selected_ids(), false);
        if action.flags.test(si::ATTACK_GROUND_PRESSED) {
            self.clear_orders();
        }
        if step == AttackGroundStep::Enter && self.commandable() {
            self.set_mode(Mode::Attack);
        }
    }

    fn formation_word(&self, sim: &TflSim) -> Option<u16> {
        if self.mode == Mode::Formation {
            Some(self.formation)
        } else if !one_group(sim, &self.selected) {
            Some(0)
        } else {
            None
        }
    }
}

#[derive(Debug, Clone, Copy)]
enum Near {
    Same(i8, i16),
    NotPlayer(i8),
}

fn near_targets(sim: &TflSim, target: usize, rule: Near) -> Vec<usize> {
    const RADIUS: i32 = 1;
    let side = 2 * RADIUS + 1;
    let (w, h) = (
        sim.data.mesh.cell_width as i32,
        sim.data.mesh.cell_height as i32,
    );
    let mut list = vec![target];
    let mut k = 0;
    while k < list.len() {
        let Some(o) = sim.objects.get(sim.monsters[list[k]].object) else {
            break;
        };
        k += 1;
        let corner = |v: i32, max: i32| {
            let c = (v >> 10) as i16 as i32 - RADIUS;
            if c < 0 { 0 } else { c.min(max - side) }
        };
        let (cx, cy) = (corner(o.x, w), corner(o.y, h));
        let base = if cx >= 0 && cx < w && cy >= 0 && cy < h {
            w * cy + cx
        } else {
            0
        };
        for row in 0..side {
            for col in 0..side {
                let cell = base + w * row + col;
                let mut next = usize::try_from(cell)
                    .ok()
                    .and_then(|c| sim.cell_heads.get(c))
                    .copied()
                    .unwrap_or(-1);
                while next >= 0 {
                    let Some(obj) = sim.objects.get(next as usize) else {
                        break;
                    };
                    next = obj.next_in_cell;
                    if obj.flags & 0x200 != 0 || obj.kind != 2 || obj.owner < 0 {
                        continue;
                    }
                    let s = obj.owner as usize;
                    let Some(m) = sim.monsters.get(s) else {
                        continue;
                    };
                    let ok = match rule {
                        Near::Same(player, kind) => {
                            m.player == player
                                && sim
                                    .data
                                    .peek_def(b"mons", m.definition)
                                    .is_some_and(|d| d.i16(0x1fc) == kind)
                        }
                        Near::NotPlayer(player) => m.player != player,
                    };
                    if ok && !list.contains(&s) && list.len() < MAX_TARGETS {
                        list.push(s);
                    }
                }
            }
        }
    }
    list
}

fn special_kind(sim: &TflSim, slot: usize) -> i16 {
    let m = &sim.monsters[slot];
    let Some(mons) = sim.data.peek_def(b"mons", m.definition) else {
        return -1;
    };
    if let Some(a) = m.artifact
        && sim
            .data
            .peek_def(b"arti", a)
            .is_none_or(|d| d.u32(0) & 1 != 0)
    {
        return -1;
    }
    for k in 0..mons.i16(0x94).max(0) as usize {
        let a = mandrake_tfl_sim::combat::Attack::read(mons, k);
        let af = a.flags;
        if af & 0x80 == 0
            || (af & 0x20 != 0 && m.carried == -1)
            || (af & 0x10 != 0 && m.ammunition < 1)
        {
            continue;
        }
        if m.flags & 1 != 0 && m.flags & 0x10 != 0 {
            if af & 0x40 != 0 {
                continue;
            }
            if let Some(p) = a.projectile
                && sim
                    .data
                    .peek_def(b"proj", p)
                    .is_some_and(|d| d.u32(0) & 0x2000 != 0 && d.u32(0) & 0x2000_0000 != 0)
            {
                continue;
            }
        }
        return i16::from(af & 0x40 == 0);
    }
    -1
}

fn attack_height(sim: &TflSim, (x, y): (i32, i32)) -> i32 {
    let z = sim.mesh_height(x, y).map_or(0, |h| i32::from(h as i16));
    let (w, h) = (
        sim.data.mesh.cell_width as i32,
        sim.data.mesh.cell_height as i32,
    );
    let (cx, cy) = (x >> 10, y >> 10);
    let cell = if cx >= 0 && cx < w && cy >= 0 && cy < h {
        (w * cy + cx) as usize
    } else {
        0
    };
    if z < 0 && sim.cells.get(cell).is_some_and(|c| c.flags & 0x18 != 0) {
        0
    } else {
        z
    }
}

pub(crate) fn ground_markers(
    sprites: &mut LocalSprites,
    sim: &TflSim,
    collection: FourCC,
    request: &MarkerRequest,
    now: i32,
) -> usize {
    let made = |sprites: &mut LocalSprites| match highlight_movement_destinations(
        sprites, sim, collection, request,
    ) {
        Ok(n) => n,
        Err(e) => {
            tracing::warn!("TFL: order markers not made ({e:?})");
            0
        }
    };
    match request.kind {
        MarkerKind::Order => {
            let n = made(sprites);
            sprites.remove_local_sprites(RemoveMode::StartTimer, now);
            n
        }
        MarkerKind::Preview => {
            sprites.remove_local_sprites(RemoveMode::ClearUnlessTimed, now);
            made(sprites)
        }
    }
}

fn highlight_movement_destinations(
    sprites: &mut LocalSprites,
    sim: &TflSim,
    collection: FourCC,
    request: &MarkerRequest,
) -> Result<usize, mandrake_tfl_sim::Stop> {
    let (places, _facings) =
        sim.command_places(&request.command, &request.slots, &request.waypoints)?;
    let k = request.waypoints.len().min(places.len());
    let masks: Vec<u16> = request
        .slots
        .iter()
        .map(|&s| {
            sim.monsters
                .get(s)
                .and_then(|m| sim.objects.get(m.object))
                .map_or(0, |o| o.terrain_mask)
        })
        .collect();
    Ok(crate::move_marker::place_markers(
        sprites,
        collection,
        &places[..k],
        &masks,
        MaskRule::AllMonsters,
        request.kind,
        |x, y| Some(usize::from(sim.terrain_at((x, y)))),
        |x, y| marker_height(sim, x, y),
    ))
}

fn marker_height(sim: &TflSim, x: i32, y: i32) -> i32 {
    if sim.media_pixel(x >> 6, y >> 6).unwrap_or(false) {
        0
    } else {
        sim.mesh_height(x, y).map_or(0, |h| i32::from(h as i16))
    }
}

fn distance(a: (i32, i32), b: (i32, i32)) -> i32 {
    let (dx, dy) = ((a.0 - b.0) as i64, (a.1 - b.1) as i64);
    ((dx * dx + dy * dy) as f64).sqrt() as i32
}

fn one_group(sim: &TflSim, slots: &[usize]) -> bool {
    let Some(&first) = slots.first() else {
        return true;
    };
    let at = |s: usize| {
        let o = &sim.objects[sim.monsters[s].object];
        (o.x as i64, o.y as i64)
    };
    let mut reached = vec![first];
    let mut mask = 1u64;
    let mut k = 0;
    while k < reached.len() {
        let p = at(reached[k]);
        for (i, &s) in slots.iter().enumerate() {
            let bit = 1u64 << (i & 63);
            if mask & bit != 0 {
                continue;
            }
            let q = at(s);
            let (dx, dy) = (p.0 - q.0, p.1 - q.1);
            if dx * dx + dy * dy < i64::from(GROUP_SPREAD) * i64::from(GROUP_SPREAD) {
                reached.push(s);
                mask |= bit;
            }
        }
        k += 1;
    }
    reached.len() == slots.len()
}

#[derive(Debug, Clone)]
pub(crate) struct TypedKeys {
    ring: [i16; 16],
    read: usize,
    write: usize,
    down: [bool; Self::KEYS],
    latched: [bool; Self::KEYS],
    last: i16,
    count: i32,
}

impl Default for TypedKeys {
    fn default() -> Self {
        Self {
            ring: [-1; 16],
            read: 0,
            write: 0,
            down: [false; Self::KEYS],
            latched: [false; Self::KEYS],
            last: -1,
            count: 0,
        }
    }
}

impl TypedKeys {
    const KEYS: usize = 0x64;
    const NO_REPEAT: [u8; 7] = [0x00, 0x2c, 0x38, 0x39, 0x44, 0x45, 0x46];

    pub(crate) fn latch(&mut self, key: crate::controls::MythKey) {
        if let Some(l) = self.latched.get_mut(usize::from(key.0)) {
            *l = true;
        }
    }

    pub(crate) fn poll(&mut self, held: impl Fn(crate::controls::MythKey) -> bool) -> i16 {
        for k in 0..Self::KEYS {
            let now = held(crate::controls::MythKey(k as u8));
            if (now || self.latched[k]) && !self.down[k] {
                self.ring[self.write] = k as i16;
                self.write = (self.write + 1) % self.ring.len();
            }
            self.down[k] = now;
            self.latched[k] = false;
        }
        if self.read != self.write {
            let key = self.ring[self.read];
            self.read = (self.read + 1) % self.ring.len();
            self.last = key;
            self.count = 0;
            return key;
        }
        if self.last != -1
            && held(crate::controls::MythKey(self.last as u8))
            && !Self::NO_REPEAT.contains(&(self.last as u8))
        {
            let repeat = self.count > 10 && (self.count - 10) % 3 == 0;
            self.count += 1;
            return if repeat { self.last } else { -1 };
        }
        self.last = -1;
        self.count = 0;
        -1
    }
}

pub(crate) mod keys {
    use crate::controls::ControlAction;

    #[derive(Debug, Clone, PartialEq, Eq)]
    pub(crate) enum KeyOrder {
        SelectNothing,
        SelectAll,
        General(u16),
        Detach,
        CenterSelection,
        RotatePreset { backward: bool },
        ClearPreset,
        Rename,
        Chat { shift: bool },
    }

    pub(crate) fn key_order(a: ControlAction, shift: bool) -> Option<KeyOrder> {
        Some(match a {
            ControlAction::RotatePreset => KeyOrder::RotatePreset { backward: shift },
            ControlAction::ClearPreset => KeyOrder::ClearPreset,
            ControlAction::ClearSelection => KeyOrder::SelectNothing,
            ControlAction::SelectAll => KeyOrder::SelectAll,
            ControlAction::CenterSelection => KeyOrder::CenterSelection,
            ControlAction::StopUnits => KeyOrder::General(GENERAL_STOP),
            ControlAction::ScatterUnits => KeyOrder::General(GENERAL_SCATTER),
            ControlAction::RetreatUnits => KeyOrder::General(GENERAL_RETREAT),
            ControlAction::SpecialAbility => KeyOrder::General(GENERAL_SPECIAL),
            ControlAction::GuardUnits => KeyOrder::General(GENERAL_GUARD),
            ControlAction::DetachUnits => KeyOrder::Detach,
            ControlAction::RenameUnits => KeyOrder::Rename,
            ControlAction::ChatMessage => KeyOrder::Chat { shift },
            _ => return None,
        })
    }

    impl KeyOrder {
        pub(crate) fn skipped_in_mode_0(&self) -> bool {
            matches!(
                self,
                KeyOrder::RotatePreset { .. }
                    | KeyOrder::ClearPreset
                    | KeyOrder::SelectAll
                    | KeyOrder::CenterSelection
                    | KeyOrder::General(
                        GENERAL_STOP | GENERAL_SCATTER | GENERAL_RETREAT | GENERAL_SPECIAL
                    )
                    | KeyOrder::Chat { .. }
                    | KeyOrder::Detach
            )
        }
    }

    pub(crate) const GENERAL_STOP: u16 = 0;
    pub(crate) const GENERAL_SCATTER: u16 = 1;
    pub(crate) const GENERAL_RETREAT: u16 = 2;
    pub(crate) const GENERAL_SPECIAL: u16 = 3;
    pub(crate) const GENERAL_GUARD: u16 = 4;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_typed_key() {
        use crate::controls::MythKey;
        use std::collections::BTreeSet;
        let mut t = TypedKeys::default();
        let mut held = BTreeSet::new();
        let poll = |t: &mut TypedKeys, held: &BTreeSet<u8>| t.poll(|k| held.contains(&k.0));
        assert_eq!(poll(&mut t, &held), -1);
        held.insert(0x2e);
        held.insert(0x20);
        assert_eq!(poll(&mut t, &held), 0x20);
        assert_eq!(poll(&mut t, &held), 0x2e);
        held.remove(&0x20);
        let keys: Vec<i16> = (0..14).map(|_| poll(&mut t, &held)).collect();
        assert_eq!(keys[..13], [-1; 13]);
        assert_eq!(keys[13], 0x2e);
        assert_eq!(poll(&mut t, &held), -1);
        assert_eq!(poll(&mut t, &held), -1);
        assert_eq!(poll(&mut t, &held), 0x2e);
        held.clear();
        assert_eq!(poll(&mut t, &held), -1);
        t.latch(MythKey(0x47));
        assert_eq!(poll(&mut t, &held), 0x47);
        assert_eq!(poll(&mut t, &held), -1);
        held.insert(MythKey::SHIFT.0);
        assert_eq!(poll(&mut t, &held), MythKey::SHIFT.0 as i16);
        assert!((0..20).all(|_| poll(&mut t, &held) == -1));
    }

    #[test]
    fn the_waypoint_and_group_rules() {
        assert_eq!(distance((0, 0), (3, 4)), 5);
        assert_eq!(chebyshev((0, 0), (3, -9)), 9);
    }

    #[test]
    fn a_removed_or_replaced_monster_leaves_the_selection() {
        let mut sel = TflSelection {
            selected: vec![1, 2, 3],
            ..TflSelection::default()
        };
        let ids = |s: usize| Some(100 + s as i16);
        sel.forget_removed_by(ids);
        assert_eq!(sel.selected, [1, 2, 3]);
        sel.forget_removed_by(|s| match s {
            2 => None,
            3 => Some(7),
            _ => ids(s),
        });
        assert_eq!(sel.selected, [1]);
        sel.selected.push(3);
        sel.forget_removed_by(|s| if s == 3 { Some(7) } else { ids(s) });
        assert_eq!(sel.selected, [1, 3]);
    }
}

#[cfg(test)]
mod attack_tests {
    use super::*;

    #[test]
    fn a_command_unframes_the_selection_and_frames_its_targets() {
        let mut sel = TflSelection {
            selected: vec![1, 2],
            ..TflSelection::default()
        };
        assert_eq!(sel.framed_monsters(), [1, 2]);
        sel.highlight_targets(None, &[2, 7, 8]);
        sel.after_command();
        assert_eq!(
            sel.framed_monsters(),
            [7, 8],
            "the selection unframed, not target 2"
        );
        for poll in 1..=7 {
            sel.unframed.update(1);
            sel.flash_poll();
            let want: &[usize] = if poll < 7 { &[7, 8] } else { &[1, 2] };
            assert_eq!(sel.framed_monsters(), want, "poll {poll}");
        }
        sel.highlight_targets(Some(40), &[]);
        assert_eq!(sel.framed_object(), Some(40));
        for _ in 0..7 {
            sel.flash_poll();
        }
        assert_eq!(sel.framed_object(), None);
    }

    #[test]
    fn select_nothing_ends_the_mode_and_an_add_clears_the_orders() {
        let mut sel = TflSelection {
            strings: SelectionStrings {
                insm: (0..20).map(|i| format!("insm {i}").into_bytes()).collect(),
                insp: Vec::new(),
                infn: vec![b"Short Line".to_vec()],
            },
            ..TflSelection::with_rules(SelectionRules::TFL)
        };
        sel.set_mode(Mode::Formation);
        assert_eq!(sel.prompts, (b"insm 2".to_vec(), b"Short Line".to_vec()));
        sel.waypoints = vec![(1, 2)];
        let mut ids = vec![];
        sel.machine.adjust(&mut ids, 4, si::Adjust::Add);
        sel.selection_changed();
        assert_eq!(sel.mode, Mode::Formation, "an add keeps the mode");
        assert!(std::mem::take(&mut sel.clear_markers) && sel.waypoints.is_empty());
        sel.machine.adjust(&mut ids, 4, si::Adjust::Add);
        sel.selection_changed();
        assert!(!sel.clear_markers, "mode 0 with the monster in: no change");
        sel.waypoints = vec![(1, 2)];
        sel.input.start(RENAME_LENGTH, 0);
        sel.machine.clear_selection(&mut ids);
        sel.selection_changed();
        assert_eq!(sel.mode, Mode::None);
        assert_eq!(sel.prompts, (Vec::new(), Vec::new()));
        assert!(sel.clear_markers && sel.waypoints.is_empty() && !sel.input.typing());
        for (mode, pair) in [
            (Mode::Presets, (0, 1)),
            (Mode::Attack, (4, 5)),
            (Mode::Special, (6, 7)),
            (Mode::Rename, (8, 9)),
        ] {
            sel.set_mode(mode);
            let want = |i: usize| format!("insm {i}").into_bytes();
            assert_eq!(sel.prompts, (want(pair.0), want(pair.1)), "{mode:?}");
        }
    }

    #[test]
    fn ctrl_s_press_clears_the_markers_and_the_waypoints() {
        let mut sel = TflSelection {
            waypoints: vec![(5, 5)],
            ..TflSelection::default()
        };
        let ctrl = MouseAction {
            attack_ground: true,
            ..MouseAction::default()
        };
        sel.attack_mode(&ctrl);
        assert!(sel.clear_markers && sel.waypoints.is_empty());
        assert_eq!(sel.mode, Mode::None, "no selection, no mode");
        sel.clear_markers = false;
        sel.attack_ground_was = true;
        sel.attack_mode(&ctrl);
        assert!(!sel.clear_markers, "held, not pressed");
    }

    #[test]
    fn detach_is_tfl_15_s_rule() {
        assert_eq!(detach_step(false, 0, &[0], 0), DetachStep::To(0));
        assert_eq!(detach_step(true, 0, &[0], 0), DetachStep::Nothing);
        assert_eq!(detach_step(true, -1, &[], 0), DetachStep::Nothing);
        assert_eq!(detach_step(true, 0, &[0, 3], 0), DetachStep::To(3));
        assert_eq!(detach_step(true, 1, &[2, 1], 1), DetachStep::To(2));
        assert_eq!(detach_step(true, 0, &[0, 1, 2], 0), DetachStep::PickPlayer);
    }

    #[test]
    fn ctrl_and_the_right_button_latch_the_attack_mode() {
        let mut sel = TflSelection::with_rules(SelectionRules::TFL);
        let ctrl = |held: bool| MouseAction {
            attack_ground: held,
            ..MouseAction::default()
        };
        sel.attack_mode(&ctrl(true));
        assert_eq!(sel.mode, Mode::None, "no selection");
        sel.attack_ground_was = false;
        sel.selected = vec![0];
        sel.attack_mode(&ctrl(true));
        sel.attack_ground_was = true;
        assert_eq!(sel.mode, Mode::Attack);
        for _ in 0..5 {
            sel.attack_mode(&ctrl(true));
        }
        let mut polls = 0;
        while sel.mode == Mode::Attack {
            sel.attack_mode(&ctrl(false));
            polls += 1;
        }
        assert_eq!(polls, 9, "7..0 counted, then below 0");
    }
}

#[cfg(test)]
mod rules_tests {
    use super::*;

    struct Polled {
        builder: si::ActionBuilder,
        t: u64,
    }

    impl Polled {
        fn new() -> Self {
            Self {
                builder: si::ActionBuilder::default(),
                t: 100,
            }
        }

        fn action(&mut self, cursor: (i32, i32), left: bool, right: bool) -> MouseAction {
            self.t += 1;
            MouseAction::polled(
                &mut self.builder,
                self.t,
                si::PollState {
                    cursor: (cursor.0 as i16, cursor.1 as i16),
                    left,
                    right,
                    ..si::PollState::default()
                },
            )
        }
    }

    #[test]
    fn the_poll_s_double_click_is_myth2_s() {
        for (gap, double) in [(1, true), (5, true), (9, true), (10, false), (20, false)] {
            let mut p = Polled::new();
            let a = p.action((10, 10), true, false);
            assert!(a.left_held && a.left_pressed && !a.double);
            let a = p.action((10, 10), true, false);
            assert!(a.left_held && !a.left_pressed, "held, not pressed again");
            for _ in 0..gap {
                p.action((10, 10), false, false);
            }
            let a = p.action((10, 10), true, false);
            assert!(a.left_pressed);
            assert_eq!(a.double, double, "a press {gap} polls after the release");
        }
        let mut p = Polled::new();
        p.action((0, 0), false, true);
        p.action((0, 0), false, false);
        let a = p.action((0, 0), false, true);
        assert!(a.right_pressed && a.right_double);
        let a = MouseAction {
            option: true,
            control: true,
            ..MouseAction::default()
        };
        let f = input_action(&a, false, true).flags;
        assert!(f.test(si::OPTION) && f.test(si::CONTROL));
        let f = input_action(&a, false, false).flags;
        assert!(!f.test(si::OPTION) && !f.test(si::CONTROL));
    }
}

#[cfg(test)]
mod film_tests {
    use super::*;

    #[test]
    fn chat_message_is_the_teams_unless_shift_or_alone() {
        assert_eq!(chat_mode(false, false), Mode::ChatTeam);
        assert_eq!(Mode::ChatTeam.number(), 5);
        assert_eq!(chat_mode(true, false), Mode::ChatAll);
        assert_eq!(chat_mode(false, true), Mode::ChatAll);
        assert_eq!(Mode::ChatAll.number(), 6);
        assert_eq!(Mode::ChatTeam.prompt_strings(), Some((10, 11)));
        assert_eq!(CHAT_LENGTH, 47);
        let p = chat_payload(true, &[b'x'; 60]);
        assert_eq!(p.len(), 2 + 0x31);
        assert_eq!((p[1], p[2 + 0x2f], p[2 + 0x30]), (1, b'x', 0));
    }
}
