use mandrake_common::FourCC;
use mandrake_common::game_options;
use mandrake_tag::{MapActionParameter, MeshMapActions};

use crate::draw_site;
use crate::engine_move::{arctangent, distance2d, trig_tables};
use crate::engine_rng::{
    EngineRng, EngineWorld, MovementHeader, TagSource, Unmodelled, set_object_shape,
};

pub const FLAG_INITIALLY_ACTIVE: u32 = 0x1;
pub const FLAG_NO_ACTIVATION_DELAY: u32 = 0x4;
pub const FLAG_NO_RESCHEDULE_DELAY: u32 = 0x8;
pub const FLAG_DELETE_ON_DEACTIVATION: u32 = 0x10;
pub const FLAG_ACTIVE: u32 = 0x20;
pub const FLAG_SUCCEEDED: u32 = 0x40;

const fn tag(s: &[u8; 4]) -> u32 {
    u32::from_be_bytes(*s)
}

pub const TYPE_MONSTERS: i16 = 2;
pub const TYPE_ACTIONS: i16 = 3;
pub const TYPE_INTEGER: i16 = 5;
pub const TYPE_DISTANCE: i16 = 6;
pub const TYPE_POINT: i16 = 13;
pub const TYPE_POINT3D: i16 = 0x12;
pub const TYPE_MODEL_ANIMATION: i16 = 0x14;
pub const TYPE_TAG: i16 = 11;
pub const TYPE_FIXED: i16 = 8;
const TYPE_FLAG: i16 = 0;
const TAG_NAME: i16 = 7;
const TYPE_OBJECTS: i16 = 15;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MapAction {
    pub identifier: u16,
    pub kind: i16,
    pub tag: u32,
    pub flags: u32,
    pub delay: i32,
    pub delay_range: i32,
    pub time: i32,
    pub parameters: Vec<MapActionParameter>,
}

impl MapAction {
    pub fn tag_name(&self) -> String {
        if self.tag == u32::MAX {
            "----".into()
        } else {
            FourCC::new(self.tag.to_be_bytes()).to_string()
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AmbientData {
    Chicken {
        timer: i32,
        home: (i32, i32),
    },
    Deer {
        flags: i32,
        timer: i32,
        home: (i32, i32),
        vitality: u16,
    },
    Soaring {
        timer: i32,
        animation_timer: i32,
    },
    Peasant {
        timer: i32,
        home: (i32, i32),
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MovementOrder {
    pub monster: usize,
    pub waypoints: Vec<(i32, i32)>,
    pub flags: u16,
    pub facing: u16,
    pub action: i16,
}

pub struct MapActionContext<'w, 'r, 's> {
    pub world: &'w mut EngineWorld,
    pub rng: &'r mut EngineRng<'s>,
    pub time: i32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TriggerRun {
    pub time: i32,
    pub action: usize,
    pub depth: usize,
    pub result: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PeasantRun {
    pub time: i32,
    pub identifier: i16,
    pub rng: u32,
    pub destination: (i32, i32),
    pub timer: i32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VisibleRun {
    pub time: i32,
    pub identifier: i16,
    pub monster: i32,
    pub object: i32,
    pub flags: u32,
    pub flags_8: u32,
    pub rng: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PastWaypointsRead {
    pub action: usize,
    pub k: i32,
    pub n: i32,
    pub a: (i32, i32),
    pub b: (i32, i32),
    pub steps: i32,
    pub block: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WaypointSave {
    pub action: usize,
    pub wpi: i32,
    pub interpolated: Option<(i32, i32)>,
}

thread_local! {
    pub static TRIGGER_LOG: std::cell::RefCell<Option<Vec<TriggerRun>>> =
        const { std::cell::RefCell::new(None) };
    pub static ACTION_LOG: std::cell::RefCell<Option<Vec<String>>> =
        const { std::cell::RefCell::new(None) };
    pub static VISIBLE_LOG: std::cell::RefCell<Option<Vec<VisibleRun>>> =
        const { std::cell::RefCell::new(None) };
    pub static PAST_WAYPOINTS_LOG: std::cell::RefCell<Option<Vec<PastWaypointsRead>>> =
        const { std::cell::RefCell::new(None) };
    pub static WAYPOINT_SAVE_LOG: std::cell::RefCell<Option<Vec<WaypointSave>>> =
        const { std::cell::RefCell::new(None) };
    pub static PEASANT_LOG: std::cell::RefCell<Option<Vec<PeasantRun>>> =
        const { std::cell::RefCell::new(None) };
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PendingMonsterAction {
    pub monster: usize,
    pub identifier: i16,
    pub action: FourCC,
}

pub type AfterTrigger = fn(usize, &mut MapActionContext) -> Result<(), Unmodelled>;

#[derive(Debug, Clone, Default)]
pub struct MapActions {
    pub actions: Vec<MapAction>,
    pub started: bool,
    pub counter: u16,
    trigger_stack: Vec<usize>,
    glo: std::collections::HashMap<usize, TestUnitsState>,
    meander_glo: std::collections::HashMap<usize, MeanderState>,
    pub(crate) melee_glo: std::collections::HashMap<usize, crate::map_attacks::MeleeState>,
    pub(crate) legion_glo: std::collections::HashMap<usize, crate::legion_action::LegionState>,
    pub(crate) harass_glo: std::collections::HashMap<usize, crate::harass_action::HarassState>,
    pub(crate) suicide_glo: std::collections::HashMap<usize, crate::suicide_action::SuicideState>,
    pub(crate) follow_glo: std::collections::HashMap<usize, crate::follow_action::FollowState>,
    pub(crate) assassin_glo:
        std::collections::HashMap<usize, crate::assassin_action::AssassinState>,
    pub(crate) surround_cst:
        std::collections::HashMap<usize, crate::surround_action::SurroundState>,
    pub sniffer: Option<Sniffer>,
    pub overhead_map_state: i16,
    pub triggers: std::collections::BTreeMap<String, u64>,
    pub skip_unimplemented: bool,
    pub skipped: Vec<(i32, usize, String)>,
    pub bridged: std::collections::HashMap<usize, Vec<u32>>,
    pub after_trigger: Option<AfterTrigger>,
    pub sound_plays: Vec<SoundActionPlay>,
    pub camera_actions: Vec<crate::camera_actions::CameraAction>,
    pub local_group_actions: Vec<LocalGroupAction>,
    pub local_group_actions_total: u64,
    pub mission_advances: u32,
}

pub const LOCAL_GROUP_ACTIONS_KEPT: usize = 64;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LocalGroupAction {
    pub groups: Vec<i16>,
    pub on: bool,
    pub off: bool,
    pub opacity: Option<i32>,
}

pub const SOUND_PLAYS_KEPT: usize = 64;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SoundPlace {
    Flat,
    NearestVisible(Vec<usize>),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SoundActionPlay {
    pub time: i32,
    pub action: usize,
    pub sound: u32,
    pub place: SoundPlace,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Sniffer {
    pub case: i16,
    pub flags: u32,
    pub action: i16,
    pub delay: i32,
    pub range: i32,
    pub deadline: i32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SniffedCommand {
    General {
        command: i16,
    },
    Movement {
        flags: u16,
        formation: i16,
    },
    Target {
        flags: u16,
        units: Vec<u16>,
        targets: Vec<u16>,
    },
    Other(u8),
}

impl SniffedCommand {
    pub fn kind(&self) -> u8 {
        match self {
            Self::General { .. } => 2,
            Self::Movement { .. } => 3,
            Self::Target { .. } => 4,
            Self::Other(k) => *k,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InputAction {
    pub flags: [u32; 4],
    pub key: i16,
}

impl InputAction {
    pub fn flag(&self, f: u32) -> bool {
        self.flags
            .get((f >> 5) as usize)
            .is_some_and(|w| w & (1 << (f & 31)) != 0)
    }
}

pub const LOCAL_PROCS: [u32; 6] = [
    tag(b"soun"),
    tag(b"obmo"),
    tag(b"ambi"),
    tag(b"lpgr"),
    tag(b"fade"),
    tag(b"uind"),
];

pub const REGISTERED_MAP_ACTION_PROCS: [([u8; 4], i16, i16); 62] = [
    (*b"guar", 0, 0),
    (*b"grou", 0, 0),
    (*b"rout", 0, 0),
    (*b"spup", 0, 0),
    (*b"gene", 0, 0),
    (*b"move", 0, 0),
    (*b"atta", 0, 0),
    (*b"plat", 0, 0),
    (*b"plmo", 0, 0),
    (*b"plsc", 0, 0),
    (*b"squa", 0, 0),
    (*b"mung", 0, 0),
    (*b"endg", 0, 0),
    (*b"tuni", 0, 0),
    (*b"ctrl", 0, 0),
    (*b"wand", 0, 0),
    (*b"acli", 0, 0),
    (*b"surr", 0, 0),
    (*b"look", 0, 0),
    (*b"part", 0, 0),
    (*b"snif", 0, 0),
    (*b"goal", 0, 0),
    (*b"plan", 0, 0),
    (*b"ngty", 0, 0),
    (*b"pick", 0, 0),
    (*b"lead", 0, 0),
    (*b"obco", 0, 0),
    (*b"legi", 0, 0),
    (*b"geom", 0, 0),
    (*b"moef", 0, 0),
    (*b"knot", 0, 0),
    (*b"lock", 0, 0),
    (*b"foll", 0, 0),
    (*b"mece", 0, 0),
    (*b"sham", 0, 0),
    (*b"suic", 0, 0),
    (*b"mele", 0, 0),
    (*b"girl", 0, 0),
    (*b"assa", 0, 0),
    (*b"invi", 0, 0),
    (*b"mean", 0, 0),
    (*b"ligh", 0, 0),
    (*b"dela", 0, 0),
    (*b"moma", 0, 0),
    (*b"anim", 0, 0),
    (*b"miss", 0, 0),
    (*b"plyc", 0, 0),
    (*b"math", 0x640, 0),
    (*b"quni", 0x6b8, 0),
    (*b"scor", 0x6b8, 0),
    (*b"end2", 0x6b8, 0),
    (*b"soun", 0, 1),
    (*b"obmo", 0, 1),
    (*b"ambi", 0, 1),
    (*b"lpgr", 0, 1),
    (*b"fade", 0x640, 1),
    (*b"uind", 0x6b8, 1),
    (*b"chic", 0, 0),
    (*b"deer", 0, 0),
    (*b"soar", 0, 0),
    (*b"peas", 0, 0),
    (*b"bear", 0x6b8, 0),
];

pub fn registered_map_action_proc(kind: u32, gameplay_version: i32) -> Option<(i16, i16)> {
    REGISTERED_MAP_ACTION_PROCS
        .iter()
        .find(|(t, _, _)| u32::from_be_bytes(*t) == kind)
        .filter(|(_, min, _)| i32::from(*min) < gameplay_version)
        .map(|&(_, min, flags)| (min, flags))
}

pub fn sound_action_result() -> u32 {
    2
}

pub const BRIDGED_SITE: u32 = 0xffff_fffe;

const ITER_NO_LINKS: u32 = 1;
const ITER_EMPTY: u32 = 2;
const ITER_NOT_SINGLE: u32 = 4;

type ParamRef = (usize, usize);
type Leg = ((i32, i32), (i32, i32));

impl MapActions {
    pub fn from_mesh(mesh: &MeshMapActions) -> Self {
        Self {
            actions: mesh
                .actions
                .iter()
                .map(|r| MapAction {
                    identifier: r.identifier,
                    kind: r.kind,
                    tag: r.tag,
                    flags: r.flags,
                    delay: r.delay,
                    delay_range: r.delay_range,
                    time: r.time,
                    parameters: r.parameters.clone(),
                })
                .collect(),
            counter: 0x8000u16
                .wrapping_add(mesh.actions.len() as u16)
                .max(0x8000),
            ..Self::default()
        }
    }

    pub fn add_monster_action(&mut self, action: FourCC, monster: i16) -> usize {
        let identifier = if self.counter == 0 {
            0x8000
        } else {
            self.counter
        };
        self.counter = match identifier.wrapping_add(1) {
            0 => 0x8000,
            n => n,
        };
        let record = MapAction {
            identifier,
            kind: 2,
            tag: u32::from_be_bytes(action.0),
            flags: FLAG_INITIALLY_ACTIVE,
            delay: 0,
            delay_range: 0,
            time: 0,
            parameters: vec![MapActionParameter {
                kind: TYPE_MONSTERS,
                count: 1,
                name: FourCC::new(*b"subj"),
                values: vec![i32::from(monster)],
            }],
        };
        match self.actions.iter().position(|a| a.identifier == 0) {
            Some(slot) => {
                self.forget_structures(slot);
                self.actions[slot] = record;
                slot
            }
            None => {
                self.actions.push(record);
                self.actions.len() - 1
            }
        }
    }

    fn forget_structures(&mut self, slot: usize) {
        self.glo.remove(&slot);
        self.meander_glo.remove(&slot);
        self.melee_glo.remove(&slot);
        self.legion_glo.remove(&slot);
        self.harass_glo.remove(&slot);
        self.suicide_glo.remove(&slot);
        self.follow_glo.remove(&slot);
        self.assassin_glo.remove(&slot);
        self.surround_cst.remove(&slot);
    }

    pub fn adopt_monster_actions(&mut self, ctx: &mut MapActionContext) -> Result<(), Unmodelled> {
        if ctx.world.pending_map_actions.is_empty() {
            return Ok(());
        }
        for p in std::mem::take(&mut ctx.world.pending_map_actions) {
            let slot = self.add_monster_action(p.action, p.identifier);
            ctx.world.monster_map_actions.insert(p.monster, slot);
            if self.started
                && ctx.world.formation_globals.gameplay_version > game_options::GAMEPLAY_1_7_2
            {
                self.activate(Some(slot), ctx)?;
            }
        }
        Ok(())
    }

    pub fn index_of(&self, identifier: i32) -> Option<usize> {
        let id = identifier as u16;
        if id == 0 {
            return None;
        }
        self.actions.iter().position(|a| a.identifier == id)
    }

    fn iterate(
        &self,
        action: usize,
        flags: u32,
        name: Option<u32>,
        kind: Option<i16>,
    ) -> Vec<ParamRef> {
        let mut out = Vec::new();
        let mut stack: Vec<usize> = vec![action];
        let mut cur = 0;
        while cur < stack.len() {
            let a = stack[cur];
            for (p, param) in self.actions[a].parameters.iter().enumerate() {
                if flags & ITER_NO_LINKS == 0 && param.kind == 3 && param.name.0 == *b"link" {
                    if param.count > 0 {
                        for &id in &param.values {
                            let Some(ai) = self.index_of(id) else {
                                continue;
                            };
                            if cur > 0 && stack[..cur].contains(&ai) {
                                continue;
                            }
                            if stack.len() <= 0x3f {
                                stack.push(ai);
                            }
                        }
                    }
                    continue;
                }
                if kind.is_some_and(|k| k != param.kind) {
                    continue;
                }
                if name.is_some_and(|n| n != u32::from_be_bytes(param.name.0)) {
                    continue;
                }
                if param.count <= 0 {
                    if flags & ITER_EMPTY == 0 {
                        continue;
                    }
                } else if param.count == 1 && flags & ITER_NOT_SINGLE != 0 {
                    continue;
                }
                out.push((a, p));
            }
            cur += 1;
        }
        out
    }

    fn param(&self, r: ParamRef) -> &MapActionParameter {
        &self.actions[r.0].parameters[r.1]
    }

    pub fn find(
        &self,
        action: usize,
        flags: u32,
        name: &[u8; 4],
        kind: Option<i16>,
    ) -> Option<&MapActionParameter> {
        let want_name = tag(name);
        let no_links = flags & ITER_NO_LINKS != 0;
        let allow_empty = flags & 2 != 0;
        let not_single = flags & 4 != 0;
        let mut stack = [0usize; 64];
        stack[0] = action;
        let mut len = 1usize;
        let mut cur = 0usize;
        while cur < len {
            let a = stack[cur];
            for param in &self.actions[a].parameters {
                if !no_links && param.kind == 3 && param.name.0 == *b"link" {
                    if param.count > 0 {
                        for &id in &param.values {
                            let Some(ai) = self.index_of(id) else {
                                continue;
                            };
                            if cur > 0 && stack[..cur].contains(&ai) {
                                continue;
                            }
                            if len < 64 {
                                stack[len] = ai;
                                len += 1;
                            }
                        }
                    }
                    continue;
                }
                if kind.is_some_and(|k| k != param.kind) {
                    continue;
                }
                if want_name != u32::from_be_bytes(param.name.0) {
                    continue;
                }
                let ok = match (allow_empty, not_single) {
                    (true, true) => param.count != 1,
                    (true, false) => true,
                    (false, true) => param.count > 1,
                    (false, false) => param.count > 0,
                };
                if ok {
                    return Some(param);
                }
            }
            cur += 1;
        }
        None
    }

    pub fn has(&self, action: usize, name: &[u8; 4]) -> bool {
        self.find(action, 2, name, None).is_some()
    }

    pub fn indexed(&self, action: usize, name: &[u8; 4], kind: i16, index: usize) -> Option<i32> {
        let p = self.find(action, 0, name, Some(kind))?;
        (index < p.count as usize).then(|| p.values[index])
    }

    pub fn indexed_point3d(&self, action: usize, name: &[u8; 4], index: usize) -> Option<[i32; 3]> {
        let p = self.find(action, 0, name, Some(TYPE_POINT3D))?;
        (index < p.count as usize).then(|| {
            let v = &p.values[3 * index..3 * index + 3];
            [v[0], v[1], v[2]]
        })
    }

    pub fn indexed_point(&self, action: usize, name: &[u8; 4], index: usize) -> Option<(i32, i32)> {
        let p = self.find(action, 0, name, Some(TYPE_POINT))?;
        (index < p.count as usize).then(|| (p.values[2 * index], p.values[2 * index + 1]))
    }

    pub fn combined(&self, action: usize, name: &[u8; 4], kind: i16, max: usize) -> Vec<i32> {
        let mut out = Vec::new();
        for r in self.iterate(action, 0, Some(tag(name)), Some(kind)) {
            let p = self.param(r);
            let per = MapActionParameter::scalars_per_element(p.kind);
            for chunk in p.values.chunks(per) {
                if out.len() / per >= max {
                    return out;
                }
                out.extend_from_slice(chunk);
            }
        }
        out
    }

    fn own_lists(&self, action: usize, names: &[&[u8; 4]]) -> Vec<(u32, Vec<i32>)> {
        self.iterate(action, ITER_NO_LINKS, None, None)
            .into_iter()
            .map(|r| self.param(r))
            .filter(|p| names.iter().any(|n| tag(n) == u32::from_be_bytes(p.name.0)))
            .map(|p| {
                (
                    u32::from_be_bytes(p.name.0),
                    p.values.iter().take(0x1e).copied().collect(),
                )
            })
            .collect()
    }

    fn log_action(&self, kind: char, i: usize, time: i32) {
        ACTION_LOG.with(|l| {
            if let Some(v) = l.borrow_mut().as_mut() {
                let a = &self.actions[i];
                v.push(format!(
                    "{time} {kind} #{i}/{}/{}",
                    a.identifier,
                    a.tag_name()
                ));
            }
        });
    }

    fn run_lists(
        &mut self,
        i: usize,
        activate: &[u8; 4],
        deactivate: &[u8; 4],
        ctx: &mut MapActionContext,
    ) -> Result<(), Unmodelled> {
        let deact = tag(deactivate);
        for (name, ids) in self.own_lists(i, &[activate, deactivate]) {
            for id in ids {
                let j = self.index_of(id);
                if name == deact {
                    self.deactivate(j, ctx)?;
                } else {
                    self.activate(j, ctx)?;
                }
            }
        }
        Ok(())
    }

    pub fn activate(
        &mut self,
        i: Option<usize>,
        ctx: &mut MapActionContext,
    ) -> Result<(), Unmodelled> {
        let Some(i) = i else { return Ok(()) };
        self.log_action('A', i, ctx.time);
        let f = self.actions[i].flags;
        if f & FLAG_ACTIVE != 0 || f & 0x82 == 0x82 {
            return Ok(());
        }
        self.run_lists(i, b"acoa", b"deoa", ctx)?;
        let a = &mut self.actions[i];
        a.flags |= 0x1a0;
        a.time = ctx.time;
        if a.flags & FLAG_NO_ACTIVATION_DELAY == 0 {
            a.time +=
                a.delay + delay_draw(a.delay_range, draw_site::MAP_ACTION_ACTIVATE_DELAY, ctx.rng);
        }
        Ok(())
    }

    pub fn deactivate(
        &mut self,
        i: Option<usize>,
        ctx: &mut MapActionContext,
    ) -> Result<(), Unmodelled> {
        let Some(i) = i else { return Ok(()) };
        self.log_action('D', i, ctx.time);
        if self.actions[i].flags & FLAG_ACTIVE == 0 {
            return Ok(());
        }
        self.run_lists(i, b"acod", b"deod", ctx)?;
        let a = &mut self.actions[i];
        a.flags &= !FLAG_ACTIVE;
        if a.flags & FLAG_DELETE_ON_DEACTIVATION != 0 {
            a.identifier = 0;
        }
        Ok(())
    }

    pub(crate) fn succeed(
        &mut self,
        i: usize,
        ctx: &mut MapActionContext,
    ) -> Result<(), Unmodelled> {
        self.actions[i].flags |= FLAG_SUCCEEDED;
        self.run_lists(i, b"acos", b"deos", ctx)?;
        if self.actions[i].kind == 2 {
            self.deactivate(Some(i), ctx)?;
        }
        Ok(())
    }

    pub(crate) fn error(&mut self, i: usize, ctx: &mut MapActionContext) -> Result<(), Unmodelled> {
        self.run_lists(i, b"acer", b"deer", ctx)
    }

    fn fail(&mut self, i: usize, ctx: &mut MapActionContext) -> Result<(), Unmodelled> {
        self.actions[i].flags &= !FLAG_SUCCEEDED;
        self.run_lists(i, b"acof", b"deof", ctx)
    }

    pub fn trigger(&mut self, i: usize, ctx: &mut MapActionContext) -> Result<u32, Unmodelled> {
        let depth = self.trigger_stack.len();
        let r = if depth > 3 || self.trigger_stack.contains(&i) {
            Ok(0)
        } else {
            self.trigger_stack.push(i);
            let r = self.trigger_inner(i, ctx);
            self.trigger_stack.pop();
            r
        };
        if let Ok(result) = r {
            TRIGGER_LOG.with(|l| {
                if let Some(v) = l.borrow_mut().as_mut() {
                    v.push(TriggerRun {
                        time: ctx.time,
                        action: i,
                        depth,
                        result,
                    });
                }
            });
        }
        r
    }

    fn trigger_inner(&mut self, i: usize, ctx: &mut MapActionContext) -> Result<u32, Unmodelled> {
        let mut refused: Option<(bool, bool)> = None;
        for (name, ids) in self.own_lists(i, &[b"inhi", b"prer"]) {
            let any_succeeded = ids
                .iter()
                .filter_map(|&id| self.index_of(id))
                .any(|j| self.actions[j].flags & FLAG_SUCCEEDED != 0);
            if name == tag(b"prer") && !any_succeeded {
                refused = Some((true, false));
                break;
            }
            if name == tag(b"inhi") && any_succeeded {
                refused = Some((false, true));
                break;
            }
        }
        if let Some((missing, inhibited)) = refused {
            for (name, ids) in self.own_lists(i, &[b"acin", b"acmp"]) {
                if (name == tag(b"acmp") && missing) || (name == tag(b"acin") && inhibited) {
                    for id in ids {
                        let j = self.index_of(id);
                        self.activate(j, ctx)?;
                    }
                }
            }
            return Ok(0);
        }
        for (name, ids) in self.own_lists(i, &[b"acot", b"trsi", b"acoe"]) {
            for id in ids {
                let j = self.index_of(id);
                if name == tag(b"trsi") {
                    if let Some(j) = j {
                        self.trigger(j, ctx)?;
                    }
                } else {
                    self.activate(j, ctx)?;
                }
            }
        }
        let a = &self.actions[i];
        if a.tag == u32::MAX {
            return Ok(1);
        }
        let tag_bytes = a.tag.to_be_bytes();
        if let Ok(s) = std::str::from_utf8(&tag_bytes)
            && let Some(count) = self.triggers.get_mut(s)
        {
            *count += 1;
        } else {
            *self.triggers.entry(a.tag_name()).or_insert(0) += 1;
        }
        let Some((_, proc_flags)) =
            registered_map_action_proc(a.tag, ctx.world.formation_globals.gameplay_version)
        else {
            return Ok(1);
        };
        let local_list = proc_flags & 1 != 0
            && ctx.world.formation_globals.gameplay_version > game_options::GAMEPLAY_1_7_2
            && (self.find(i, 0, b"plyr", Some(TYPE_INTEGER)).is_some()
                || self.find(i, 0, b"team", Some(TYPE_INTEGER)).is_some());
        let result = match self.run_proc(i, ctx) {
            Ok(_) if local_list => 1,
            Ok(r) => r,
            Err(e) if self.bridged.contains_key(&i) => {
                let name = self.actions[i].tag_name();
                let counts = self.bridged.get_mut(&i).unwrap();
                let n = if counts.is_empty() {
                    0
                } else {
                    counts.remove(0)
                };
                for _ in 0..n {
                    ctx.rng.draw(BRIDGED_SITE);
                }
                self.skipped
                    .push((ctx.time, i, format!("{name} (bridged {n}: {e})")));
                0
            }
            Err(_) if self.skip_unimplemented => {
                let name = self.actions[i].tag_name();
                self.skipped.push((ctx.time, i, name));
                0
            }
            Err(e) => return Err(e),
        };
        self.actions[i].flags &= !0x140;
        Ok(result | 1)
    }

    fn run_proc(&mut self, i: usize, ctx: &mut MapActionContext) -> Result<u32, Unmodelled> {
        match &self.actions[i].tag.to_be_bytes() {
            b"chic" => self.ambient(i, ctx, Ambient::Chicken),
            b"deer" => self.ambient(i, ctx, Ambient::Deer),
            b"soar" => self.ambient(i, ctx, Ambient::Soaring),
            b"peas" => self.ambient(i, ctx, Ambient::Peasant),
            b"acli" => self.action_list(i, ctx),
            b"wand" => self.wander(i, ctx),
            b"tuni" => self.test_units(i, ctx),
            b"gene" => self.general_command(i, ctx),
            b"ctrl" => self.unit_control(i, ctx),
            b"plat" => self.platoon(i, ctx),
            b"squa" => self.squad(i, ctx),
            b"lead" => self.lead(i, ctx),
            b"move" => self.movement_command(i, ctx),
            b"pick" => self.pick_up_object(i, ctx),
            b"rout" => self.rout_action(i, ctx),
            b"surr" => self.surround_action(i, ctx),
            b"look" => self.look_action(i, ctx),
            b"foll" => self.follow_action(i, ctx),
            b"assa" => self.assassin_action(i, ctx),
            b"sham" => self.shaman_action(i, ctx),
            b"obco" => self.object_control_action(i, ctx),
            b"legi" => self.legion_action(i, ctx),
            b"girl" => self.harass_action(i, ctx),
            b"suic" => self.suicide_action(i, ctx),
            b"plmo" | b"plsc" => self.platoon_movement(i, ctx),
            b"mece" => self.mesh_cell(i, ctx),
            b"moma" => self.move_marker_action(i, ctx),
            b"dela" => self.delay_action(i, ctx),
            b"plyc" => self.player_count_action(i, ctx),
            b"plan" => self.netgame_time_action(i, ctx),
            b"ligh" => self.lightning_action(i, ctx),
            b"soun" => {
                self.sound_action(i, ctx);
                Ok(sound_action_result())
            }
            b"lpgr" => {
                self.local_group_action(i, ctx);
                Ok(2)
            }
            b"ambi" => Ok(0),
            b"ngty" => self.game_type_action(i, ctx),
            b"snif" => self.sniffer_action(i, ctx),
            b"atta" => self.attack_command(i, ctx),
            b"obmo" => self.observer_movement(i, ctx),
            b"part" => Ok(self.particle_system_action(i, ctx)),
            b"geom" => self.geometry_filter(i, ctx),
            b"mung" => self.munger(i, ctx),
            b"mean" => self.meander(i, ctx),
            b"mele" => self.melee(i, ctx),
            b"miss" => {
                if self.flag(i, b"adva") {
                    self.mission_advances += 1;
                }
                Ok(2)
            }
            b"endg" => self.endgame_action(i, ctx),
            b"end2" => self.custom_endgame_action(i, ctx),
            b"moef" => self.model_effect(i, ctx),
            b"anim" => {
                let mut animations = std::mem::take(&mut ctx.world.model_animations);
                let r = self.model_animation(i, ctx, &mut animations);
                ctx.world.model_animations = animations;
                r
            }
            _ => Err(Unmodelled::new(
                draw_site::MAP_ACTION_PROC_NOT_IMPLEMENTED,
                format!(
                    "map action {} (#{i}, '{}'): proc not implemented",
                    self.actions[i].identifier,
                    self.actions[i].tag_name()
                ),
            )),
        }
    }

    fn model_effect(&mut self, i: usize, ctx: &mut MapActionContext) -> Result<u32, Unmodelled> {
        use crate::model_effects::{self as fx, ModelEffectDescriptor};
        let short = |s: &Self, name: &[u8; 4], kind: i16| s.long(i, name, kind).map(|v| v as i16);
        let anims = &ctx.world.model_animations;
        let animation_model = |id: i16| {
            anims
                .indexes(&[i32::from(id)])
                .first()
                .map(|&k| anims.animations[k].model)
        };
        let model = match short(self, b"mode", 0x10) {
            Some(id) => {
                let m = ctx.world.model_identifier_to_model_index(id);
                short(self, b"anim", TYPE_MODEL_ANIMATION)
                    .and_then(animation_model)
                    .unwrap_or(m)
            }
            None => match short(self, b"anim", TYPE_MODEL_ANIMATION).and_then(animation_model) {
                Some(m) => m,
                None => return Ok(0),
            },
        };
        if model == -1 {
            return Ok(0);
        }
        let mut d = ModelEffectDescriptor::default();
        if let Some(v) = self.long(i, b"dura", TYPE_FIXED) {
            let t = v.wrapping_mul(0x1e);
            let t = if t < 0 { t.wrapping_add(0xffff) } else { t };
            d.duration = ((t as u32) >> 16) as i16;
        }
        if let Some(v) = short(self, b"dyaw", 4) {
            d.yaw = v;
        }
        if let Some(v) = short(self, b"dpit", 4) {
            d.pitch = v;
        }
        if let Some(v) = self.long(i, b"newp", TYPE_INTEGER) {
            d.new_permutation = v;
        }
        if let Some(v) = self.long(i, b"perm", TYPE_INTEGER) {
            d.permutation = v;
        }
        for (name, flag) in [
            (b"fain", fx::FADE_IN),
            (b"faou", fx::FADE_OUT),
            (b"dele", fx::DELETE),
            (b"peon", fx::PERMUTATION_ONLY),
        ] {
            if self.flag(i, name) {
                d.flags |= flag;
            }
        }
        if let Some(material) = self.long(i, b"mate", TYPE_INTEGER)
            && let Some(v) = self.long(i, b"mafi", TYPE_FIXED)
        {
            d.material_final = !(v.clamp(0, 0xffff) as u16);
            d.material = material;
        }
        if let Some(v) = self.long(i, b"insc", TYPE_FIXED) {
            let t = v.wrapping_shl(9);
            let t = if t < 0 { t.wrapping_add(0xffff) } else { t };
            d.flags |= fx::SCALE;
            d.scale = [(t >> 16) as i16; 2];
        }
        if let Some(p) = self.long(i, b"proj", TAG_NAME) {
            d.projectile = p as u32;
            if let Some(origin) = self.indexed_point3d(i, b"orig", 0) {
                d.flags |= fx::PROJECTILE;
                d.origin = origin;
            }
        }
        let identifier = i32::from(self.actions[i].identifier as i16);
        let r = fx::new_model_effect(ctx.world, model, identifier, &d, ctx.rng)?;
        if r.error
            && let Some(j) = self.index_of(identifier)
        {
            self.error(j, ctx)?;
        }
        Ok(0)
    }

    fn endgame_action(&self, i: usize, ctx: &mut MapActionContext) -> Result<u32, Unmodelled> {
        let game = &mut ctx.world.game;
        if game.winner == 0 {
            return Ok(0);
        }
        let light = self.flag(i, b"ligh");
        game.winner = i16::from(!light);
        ctx.world.force_statistics_update(ctx.time, Some(ctx.rng));
        let game = &mut ctx.world.game;
        game.end_game(0, ctx.time, &mut ctx.world.game_options)?;
        if light {
            game.end_code = if self.flag(i, b"fail") { 2 } else { 1 };
        }
        self.no_celebration(i, ctx);
        Ok(0)
    }

    fn custom_endgame_action(
        &self,
        i: usize,
        ctx: &mut MapActionContext,
    ) -> Result<u32, Unmodelled> {
        let game = &mut ctx.world.game;
        if game.end_code != 0 {
            return Ok(0);
        }
        if let Some(team) = self.long(i, b"team", TYPE_INTEGER) {
            game.winner = team as i16;
        }
        if self.flag(i, b"susp") {
            game.suspend_end_game(true);
            return Ok(0);
        }
        game.suspend_end_game(false);
        ctx.world.force_statistics_update(ctx.time, Some(ctx.rng));
        let game = &mut ctx.world.game;
        game.end_game(0, ctx.time, &mut ctx.world.game_options)?;
        self.no_celebration(i, ctx);
        Ok(0)
    }

    fn no_celebration(&self, i: usize, ctx: &mut MapActionContext) {
        if !self.flag(i, b"noce") {
            return;
        }
        let winner = ctx.world.game.winner;
        for m in &mut ctx.world.monsters {
            if m.identifier != 0 && m.team == winner {
                m.flags |= 0x200_0000;
            }
        }
    }

    fn local_group_action(&mut self, i: usize, ctx: &MapActionContext) {
        if !self.local_proc_runs(i, ctx) {
            return;
        }
        let action = LocalGroupAction {
            groups: self
                .combined(i, b"grou", 0x13, 0x180)
                .into_iter()
                .map(|id| id as i16)
                .collect(),
            on: self.flag(i, b"__on"),
            off: self.flag(i, b"_off"),
            opacity: self.indexed(i, b"opac", 8, 0),
        };
        if self.local_group_actions.len() >= LOCAL_GROUP_ACTIONS_KEPT {
            self.local_group_actions.remove(0);
        }
        self.local_group_actions.push(action);
        self.local_group_actions_total += 1;
    }

    pub(crate) fn local_proc_runs(&self, i: usize, ctx: &MapActionContext) -> bool {
        let w = &*ctx.world;
        if w.formation_globals.gameplay_version <= game_options::GAMEPLAY_1_7_2 {
            return true;
        }
        let listed = |name: &[u8; 4], v: i16| {
            self.find(i, 0, name, Some(TYPE_INTEGER))
                .map(|p| p.values.contains(&i32::from(v)))
        };
        listed(b"plyr", w.local_player)
            .or_else(|| listed(b"team", w.local_team()))
            .unwrap_or(true)
    }

    fn sound_action(&mut self, i: usize, ctx: &MapActionContext) {
        if !self.local_proc_runs(i, ctx) {
            return;
        }
        let w = &*ctx.world;
        let Some(sound) = self.indexed(i, b"soun", TYPE_TAG, 0) else {
            return;
        };
        let raw = self.combined(i, b"subj", TYPE_MONSTERS, 0x190);
        let mut subjects = self.subjects(i, ctx, 0x190);
        let place = if subjects.is_empty() {
            raw.is_empty().then_some(SoundPlace::Flat)
        } else if (self.flag(i, b"ownr") || self.flag(i, b"team"))
            && w.formation_globals.gameplay_version > game_options::GAMEPLAY_1_5_1
        {
            let hit = if self.flag(i, b"team") {
                let team = w.local_team();
                team != -1 && subjects.iter().any(|&m| w.monsters[m].team == team)
            } else {
                let player = w.local_player;
                player != -1 && subjects.iter().any(|&m| w.monsters[m].owner == player)
            };
            hit.then_some(SoundPlace::Flat)
        } else {
            if !self.flag(i, b"over") {
                subjects.retain(|&m| w.monsters[m].flags & 0x0800_0000 == 0);
            }
            if subjects.is_empty() {
                raw.is_empty().then_some(SoundPlace::Flat)
            } else {
                Some(SoundPlace::NearestVisible(subjects))
            }
        };
        if let Some(place) = place {
            if self.sound_plays.len() >= SOUND_PLAYS_KEPT {
                self.sound_plays.remove(0);
            }
            self.sound_plays.push(SoundActionPlay {
                time: ctx.time,
                action: i,
                sound: sound as u32,
                place,
            });
        }
    }

    fn particle_system_action(&self, i: usize, ctx: &mut MapActionContext) -> u32 {
        let swit = self.flag(i, b"swit");
        if swit && let Some(ps) = ctx.world.particles.as_mut() {
            ps.switch_states(ctx.rng);
        }
        match self.long(i, b"mult", TYPE_FIXED) {
            Some(m) => {
                if let Some(ps) = ctx.world.particles.as_mut() {
                    ps.set_multiplier((m >> 8) as i16);
                }
                2
            }
            None if swit => 2,
            None => 0,
        }
    }

    fn mesh_cell(&mut self, i: usize, ctx: &mut MapActionContext) -> Result<u32, Unmodelled> {
        let pts = self.combined(i, b"pnts", TYPE_POINT, 0x40);
        if pts.is_empty() {
            return Ok(0);
        }
        let kill = self.indexed(i, b"kill", 0, 0).map(|b| b != 0);
        let terrain = self.indexed(i, b"teco", TYPE_INTEGER, 0);
        for p in pts.chunks(2) {
            let changed = ctx.world.cells.modify_cells(&[(p[0], p[1])], kill, terrain);
            for (x, y) in changed {
                ctx.world.cells_changed = true;
                crate::engine_rng::update_monster_impassability_for_cell(ctx.world, x, y);
            }
        }
        Ok(2)
    }

    pub fn model_animation(
        &mut self,
        i: usize,
        ctx: &mut MapActionContext,
        animations: &mut crate::model_animations::ModelAnimations,
    ) -> Result<u32, Unmodelled> {
        let subjects = self.combined(i, b"subj", TYPE_MODEL_ANIMATION, 0x20);
        let indexes = animations.indexes(&subjects);
        if indexes.is_empty() {
            return Ok(0);
        }
        let want_above = self.long(i, b"___>", TYPE_INTEGER);
        let want_below = self.long(i, b"___<", TYPE_INTEGER);
        let want_equal = self.long(i, b"___=", TYPE_INTEGER);
        let mut all_passed = true;
        for k in indexes {
            let frame = animations.animations[k].playback.frame as i32;
            let above = want_above.is_none_or(|v| frame > v);
            let below = want_below.is_none_or(|v| frame < v);
            let equal = want_equal.is_none_or(|v| frame == v);
            if !(above && below && equal) {
                all_passed = false;
                continue;
            }
            self.model_animation_effects(i, k, ctx, animations)?;
        }
        Ok(if all_passed { 2 } else { 0 })
    }

    fn model_animation_effects(
        &mut self,
        i: usize,
        k: usize,
        ctx: &mut MapActionContext,
        animations: &mut crate::model_animations::ModelAnimations,
    ) -> Result<(), Unmodelled> {
        let last = animations.animations[k].definition.tag.frame_count() as i32 - 1;
        {
            let p = &mut animations.animations[k].playback;
            if self.flag(i, b"back") {
                p.ping_pong = true;
            }
            if self.flag(i, b"cycl") {
                p.looping = true;
            }
        }
        let mut sound = None;
        if self.flag(i, b"stfo") {
            let p = &mut animations.animations[k].playback;
            p.forward = true;
            p.active = true;
            if self.flag(i, b"rese") {
                animations.jump(k, 0, ctx.world)?;
            }
            if animations.animations[k].playback.frame == 0 {
                sound = Some(0);
            }
        }
        if self.flag(i, b"stba") {
            let p = &mut animations.animations[k].playback;
            p.forward = false;
            p.active = true;
            if self.flag(i, b"rese") {
                animations.jump(k, last.max(0) as usize, ctx.world)?;
            }
            if animations.animations[k].playback.frame as i32 == last {
                sound = Some(1);
            }
        }
        if self.flag(i, b"adva") {
            animations.advance(k, ctx.world)?;
        }
        if self.flag(i, b"stop") {
            animations.animations[k].playback.active = false;
        }
        if let Some(slot) = sound {
            animations.play_sound(k, slot);
        }
        Ok(())
    }

    pub fn update(&mut self, ctx: &mut MapActionContext) -> Result<(), Unmodelled> {
        if ctx.time & 0x1f != 0 {
            return Ok(());
        }
        self.adopt_monster_actions(ctx)?;
        if !self.started {
            for i in 0..self.actions.len() {
                if self.actions[i].identifier != 0
                    && self.actions[i].flags & FLAG_INITIALLY_ACTIVE != 0
                {
                    self.activate(Some(i), ctx)?;
                }
            }
            self.started = true;
        }
        ctx.world.spatial_groups = crate::spatial_groups::build_spatial_groups(ctx.world);
        crate::map_attacks::reset_attack_ai(ctx.world);
        if ctx.world.mode == 3 {
            return Err(Unmodelled::new(
                draw_site::MAP_ACTIONS_GAME_MODE_3,
                "update_map_actions in game mode 3 (attack_with_all_monster_indexes)",
            ));
        }
        let mut i = 0;
        while i < self.actions.len() {
            let a = &self.actions[i];
            if a.identifier == 0 || a.flags & FLAG_ACTIVE == 0 || ctx.time < a.time {
                i += 1;
                continue;
            }
            let r = self.trigger(i, ctx)?;
            if let Some(f) = self.after_trigger {
                f(i, ctx)?;
            }
            self.adopt_monster_actions(ctx)?;
            ACTION_LOG.with(|l| {
                if let Some(v) = l.borrow_mut().as_mut() {
                    let a = &self.actions[i];
                    v.push(format!(
                        "{} P #{i}/{}/{} kind={} flags={:#x} time={} result={r:#x} rng={}",
                        ctx.time,
                        a.identifier,
                        a.tag_name(),
                        a.kind,
                        a.flags,
                        a.time,
                        ctx.rng.rng.state
                    ));
                }
            });
            if r & 1 != 0 {
                if r & 2 != 0 {
                    self.succeed(i, ctx)?;
                } else {
                    self.fail(i, ctx)?;
                }
            }
            let a = &self.actions[i];
            let deactivate = if r & 4 != 0 {
                true
            } else {
                match a.kind {
                    1 => r & 1 != 0,
                    2 => r & 3 == 3,
                    3 => false,
                    4 => r & 3 == 1,
                    _ => true,
                }
            };
            if deactivate {
                if a.identifier != 0 {
                    self.deactivate(Some(i), ctx)?;
                }
            } else if a.identifier != 0 {
                let a = &mut self.actions[i];
                a.time = ctx.time;
                if a.flags & FLAG_NO_RESCHEDULE_DELAY == 0 {
                    a.time += a.delay
                        + delay_draw(a.delay_range, draw_site::MAP_ACTION_UPDATE_DELAY, ctx.rng);
                }
            }
            i += 1;
        }
        crate::map_attacks::update_attack_ai(ctx.world)
    }
}

fn delay_draw(range: i32, site: u32, rng: &mut EngineRng) -> i32 {
    if range > 0 {
        (i32::from(rng.draw(site)) * range) >> 16
    } else {
        0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Ambient {
    Chicken,
    Deer,
    Soaring,
    Peasant,
}

fn cell_window(d: i32, k: i32, size: i32) -> (i32, i32) {
    let lo = if d > k - 1 {
        if d - (k - 1) <= size { d - k } else { size - 1 }
    } else {
        0
    };
    let hi = if d < -k {
        0
    } else if d + (k - 1) < size - 1 {
        d + k
    } else {
        size - 1
    };
    (lo, hi)
}

fn polar(base: (i32, i32), angle: u16, d: i32) -> (i32, i32) {
    let t = trig_tables();
    let i = ((u32::from(angle) + 0x20) & 0xffff) as usize >> 6;
    (
        base.0 + ((i32::from(t.cosine[i]) * d) >> 14),
        base.1 + ((i32::from(t.sine[i]) * d) >> 14),
    )
}

fn closest_point_on_circle(c: (i32, i32), r: i32, p: (i32, i32)) -> (i32, i32) {
    let (dx, dy) = (p.0.wrapping_sub(c.0), p.1.wrapping_sub(c.1));
    let d2 = dx.wrapping_mul(dx).wrapping_add(dy.wrapping_mul(dy)) as u32;
    let len = crate::projectiles::isqrt(d2);
    if len == 0 {
        return p;
    }
    (
        dx.wrapping_mul(r) / len + c.0,
        dy.wrapping_mul(r) / len + c.1,
    )
}

fn rotate_point2d(p: (i32, i32), c: (i32, i32), angle: u16) -> (i32, i32) {
    let t = trig_tables();
    let k = (angle.wrapping_add(0x20) >> 6) as usize;
    let (co, si) = (i32::from(t.cosine[k]), i32::from(t.sine[k]));
    let (dx, dy) = (p.0 - c.0, p.1 - c.1);
    (
        ((co * dx + si * dy) >> 14) + c.0,
        ((dy * co - dx * si) >> 14) + c.1,
    )
}

fn clamp_world(v: i32, cells: i32) -> i32 {
    if v < 0 {
        0
    } else if v >= cells << 9 {
        (cells << 9) - 1
    } else {
        v
    }
}

impl MapActions {
    pub(crate) fn subjects(&self, i: usize, ctx: &MapActionContext, max: usize) -> Vec<usize> {
        self.combined(i, b"subj", TYPE_MONSTERS, max)
            .into_iter()
            .filter_map(|id| ctx.world.monster_index_by_identifier(id as u16))
            .collect()
    }

    fn ambient(
        &mut self,
        i: usize,
        ctx: &mut MapActionContext,
        kind: Ambient,
    ) -> Result<u32, Unmodelled> {
        for m in self.subjects(i, ctx, 0x7e) {
            let mon = &ctx.world.monsters[m];
            if mon.ambient.is_none() {
                continue;
            }
            if mon.mode == 0xe || (0xb..=0xd).contains(&mon.mode) || mon.flags & 0x4000_0000 != 0 {
                continue;
            }
            match kind {
                Ambient::Chicken => chicken(m, ctx)?,
                Ambient::Deer => deer(m, ctx)?,
                Ambient::Soaring => soaring(m, ctx)?,
                Ambient::Peasant => peasant(m, ctx)?,
            }
        }
        Ok(0)
    }

    fn action_list(&mut self, i: usize, ctx: &mut MapActionContext) -> Result<u32, Unmodelled> {
        let acti = self.combined(i, b"acti", TYPE_ACTIONS, 0x20);
        let mut acted = false;
        if !acti.is_empty() {
            let n = acti.len() as i32;
            let only = self.find(i, 2, b"only", None).is_some();
            let rand = self.find(i, 2, b"rand", None).is_some();
            let acch = self
                .find(i, 0, b"acch", None)
                .filter(|p| i32::from(p.count) == n)
                .map(|p| p.values.clone());
            if only || rand {
                let start = if rand {
                    i32::from(ctx.rng.draw(draw_site::ACTION_LIST_START)) % n
                } else {
                    0
                };
                let mut k = start;
                loop {
                    let j = self.index_of(acti[k as usize]);
                    let eligible = j.is_some_and(|j| self.actions[j].flags & 0x82 != 0x82);
                    if let (true, Some(slot)) = (eligible, j) {
                        let take = match &acch {
                            Some(c) => {
                                let Some(&chance) = c.get(slot) else {
                                    return Err(Unmodelled::new(
                                        draw_site::ACTION_LIST_CHANCE_PAST_END,
                                        format!(
                                            "acli 'acch' read past its {} chances at slot {slot}",
                                            c.len()
                                        ),
                                    ));
                                };
                                (chance >> 8) as i16
                                    > i16::from(
                                        (ctx.rng.draw(draw_site::ACTION_LIST_CHANCE) >> 8) as u8,
                                    )
                            }
                            None => true,
                        };
                        if take {
                            self.activate(j, ctx)?;
                            acted = true;
                            break;
                        }
                    }
                    k = (k + 1) % n;
                    if k == start {
                        break;
                    }
                }
            } else if let Some(c) = &acch {
                for (k, &id) in acti.iter().enumerate() {
                    let r = (ctx.rng.draw(draw_site::ACTION_LIST_SEQUENCE_CHANCE) >> 8) as u8;
                    if (c[k] >> 8) as i16 > i16::from(r) {
                        let j = self.index_of(id);
                        self.activate(j, ctx)?;
                        acted = true;
                    }
                }
            } else {
                for &id in &acti {
                    let j = self.index_of(id);
                    self.activate(j, ctx)?;
                }
                acted = true;
            }
        }
        let deac = self.combined(i, b"deac", TYPE_ACTIONS, 0x20);
        if !deac.is_empty() {
            let n = deac.len() as i32;
            let dech = self
                .find(i, 0, b"dech", None)
                .filter(|p| i32::from(p.count) == n)
                .map(|p| p.values.clone());
            match dech {
                Some(c) => {
                    for (k, &id) in deac.iter().enumerate() {
                        let r = (ctx.rng.draw(draw_site::ACTION_LIST_EACH_CHANCE) >> 8) as u8;
                        if (c[k] >> 8) as i16 > i16::from(r) {
                            let j = self.index_of(id);
                            self.deactivate(j, ctx)?;
                            acted = true;
                        }
                    }
                }
                None => {
                    for &id in &deac {
                        let j = self.index_of(id);
                        self.deactivate(j, ctx)?;
                    }
                    return Ok(2);
                }
            }
        }
        Ok(if acted { 2 } else { 0 })
    }

    fn wander(&mut self, i: usize, ctx: &mut MapActionContext) -> Result<u32, Unmodelled> {
        let subjects = self.subjects(i, ctx, 0x7e);
        if subjects.is_empty() {
            return Ok(0);
        }
        let Some(center) = self.indexed_point(i, b"cent", 0) else {
            return Ok(0);
        };
        let Some(radius) = self.indexed(i, b"radi", TYPE_DISTANCE, 0) else {
            return Ok(0);
        };
        let player = i32::from(self.actions[i].identifier as i16);
        for m in subjects {
            let angle = ctx.rng.draw(draw_site::WANDER_ANGLE);
            let r = i32::from(ctx.rng.draw(draw_site::WANDER_DISTANCE));
            if radius == 0 {
                return Err(Unmodelled::new(
                    draw_site::WANDER_RADIUS_ZERO,
                    "wander radius 0 (division by zero)",
                ));
            }
            let dest = polar(center, angle, r % radius);
            ctx.world.order_movement(&[m], dest, 0, player)?;
        }
        Ok(2)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MeanderSubject {
    pub monster: usize,
    pub identifier: i16,
    pub timer: i16,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MeanderState {
    pub subjects: Vec<MeanderSubject>,
    pub waypoints: Vec<(i32, i32)>,
    pub spread: i32,
    pub threat_cells: i32,
}

fn meander_window(c: i32, k: i32, size: i32) -> (i32, i32) {
    let clamp = |v: i32| {
        if v < 0 {
            0
        } else if v < size {
            v
        } else {
            size - 1
        }
    };
    (clamp(c - k), clamp(c + k))
}

impl MapActions {
    fn meander(&mut self, i: usize, ctx: &mut MapActionContext) -> Result<u32, Unmodelled> {
        let mut state = match self.meander_glo.get(&i) {
            Some(s) => s.clone(),
            None => {
                let mut s = MeanderState::default();
                for monster in self.subjects(i, ctx, 0x40) {
                    let identifier = ctx.world.monsters[monster].identifier;
                    let timer = (ctx.rng.draw(draw_site::MEANDER_TIMER) % 12) as i16;
                    s.subjects.push(MeanderSubject {
                        monster,
                        identifier,
                        timer,
                    });
                }
                if let Some(p) = self.find(i, 2, b"wayp", Some(TYPE_POINT)) {
                    if p.count > 0x20 {
                        return Ok(0);
                    }
                    s.waypoints = p
                        .values
                        .as_chunks::<2>()
                        .0
                        .iter()
                        .map(|&[x, y]| (x, y))
                        .collect();
                    s.spread = 0xa00;
                }
                s.threat_cells = self.long(i, b"tscw", TYPE_INTEGER).unwrap_or(3);
                s
            }
        };
        let mut k = 0;
        while k < state.subjects.len() {
            let s = state.subjects[k];
            if s.identifier != 0 && ctx.world.monsters[s.monster].identifier == s.identifier {
                k += 1;
                continue;
            }
            state.subjects.remove(k);
            let casualty: Vec<i32> = self
                .find(i, 0, b"casu", Some(TYPE_ACTIONS))
                .map(|p| p.values.iter().take(0x1e).copied().collect())
                .unwrap_or_default();
            for id in casualty {
                let j = self.index_of(id);
                self.activate(j, ctx)?;
            }
        }
        if state.subjects.is_empty() {
            return Ok(4);
        }
        let thre = self.flag(i, b"thre");
        let source = i32::from(self.actions[i].identifier as i16);
        for s in &mut state.subjects {
            let m = s.monster;
            if thre && let Some(threat) = meander_threat(ctx.world, m, state.threat_cells) {
                let (p, q) = (
                    ctx.world.monsters[m].position,
                    ctx.world.monsters[threat].position,
                );
                let angle = arctangent(p.0 - q.0, p.1 - q.1);
                let dest = polar(p, angle, 0x1000);
                ctx.world.order_movement(&[m], dest, 0, -2)?;
                s.timer = 0;
                continue;
            }
            s.timer = s.timer.wrapping_sub(1);
            if s.timer >= 0 {
                continue;
            }
            let mut dest = if state.waypoints.is_empty() {
                ctx.rng.draw(draw_site::MEANDER_NO_WAYPOINTS);
                (0, 0)
            } else {
                let r = i32::from(ctx.rng.draw(draw_site::MEANDER_WAYPOINT));
                state.waypoints[r as usize % state.waypoints.len()]
            };
            if state.spread == 0 {
                ctx.rng.draw(draw_site::MEANDER_SPREAD_X);
                ctx.rng.draw(draw_site::MEANDER_SPREAD_Y);
            } else {
                let w = 2 * state.spread;
                let r = i32::from(ctx.rng.draw(draw_site::MEANDER_OFFSET_X));
                dest.0 = dest.0.wrapping_add(r % w - state.spread);
                let r = i32::from(ctx.rng.draw(draw_site::MEANDER_OFFSET_Y));
                dest.1 = dest.1.wrapping_add(r % w - state.spread);
            }
            ctx.world.order_movement(&[m], dest, 0, source)?;
            let mon = &ctx.world.monsters[m];
            let d = distance2d(
                dest.0.wrapping_sub(mon.position.0),
                dest.1.wrapping_sub(mon.position.1),
            );
            let speed = i32::from(mon.speed);
            let n = if speed == 0 {
                1
            } else {
                i32::from(((d / (speed << 5) + 2) as i16) >> 1)
            };
            let r = ctx.rng.draw_mod(draw_site::MEANDER_WAIT, n)?;
            s.timer = (n + r) as i16;
        }
        self.meander_glo.insert(i, state);
        Ok(0)
    }
}

fn meander_threat(w: &EngineWorld, m: usize, cells: i32) -> Option<usize> {
    let me = &w.monsters[m];
    let (cw, ch) = w.mesh_cells;
    let (x0, x1) = meander_window(me.position.0 >> 9, cells, cw);
    let (y0, y1) = meander_window(me.position.1 >> 9, cells, ch);
    let mut best: Option<(i32, usize)> = None;
    for j in w.monsters_in_cell_box(x0, y0, x1 - x0 + 1, y1 - y0 + 1) {
        let o = &w.monsters[j];
        if o.object.no_shape || o.team == me.team || o.flags_8 & 4 != 0 {
            continue;
        }
        let d = distance2d(me.position.0 - o.position.0, me.position.1 - o.position.1);
        if best.is_none_or(|(b, _)| d < b) {
            best = Some((d, j));
        }
    }
    best.map(|(_, j)| j)
}

fn chicken(m: usize, ctx: &mut MapActionContext) -> Result<(), Unmodelled> {
    let w = &ctx.world;
    let me = &w.monsters[m];
    let (x, y) = me.position;
    let (cw, ch) = w.mesh_cells;
    let (x0, x1) = cell_window(x >> 9, 3, cw);
    let (y0, y1) = cell_window(y >> 9, 3, ch);
    let threat = w.nearest_monster(m, (x0, x1), (y0, y1), |o| {
        o.definition != me.definition && o.flags_8 & 4 == 0
    });
    let Some(AmbientData::Chicken { timer, home }) = me.ambient.clone() else {
        return Err(Unmodelled::new(
            draw_site::CHICKEN_WITHOUT_DATA,
            "chicken without chicken data",
        ));
    };
    let speed = i32::from(me.speed);
    let mut dest = None;
    if let Some(o) = threat {
        let (ox, oy) = ctx.world.monsters[o].position;
        let angle = arctangent(x - ox, y - oy);
        let r = u32::from(ctx.rng.draw(draw_site::CHICKEN_ALARM));
        let d = ((r * 3) >> 6) as i32 + 0x600;
        let p = polar(home, angle, d);
        ctx.world.order_movement(&[m], p, 0, -2)?;
        dest = Some(p);
        if timer > 0 {
            set_chicken_timer(ctx, m, timer - 30);
        }
    } else if timer > 0 {
        set_chicken_timer(ctx, m, timer - 30);
        return Ok(());
    }
    if timer <= 0 {
        let angle = ctx.rng.draw(draw_site::CHICKEN_WANDER_ANGLE);
        let r = u32::from(ctx.rng.draw(draw_site::CHICKEN_WANDER_DISTANCE));
        let d = ((r * 5) >> 7) as i32;
        let (cw, ch) = ctx.world.mesh_cells;
        let p = polar(home, angle, d);
        let p = (clamp_world(p.0, cw), clamp_world(p.1, ch));
        ctx.world.order_movement(&[m], p, 0, -2)?;
        dest = Some(p);
    }
    let p = dest.expect("a chicken order was issued");
    let mut t = -60;
    if speed != 0 {
        let (x, y) = ctx.world.monsters[m].position;
        t += distance2d(x - p.0, y - p.1) / speed;
    }
    t += (i32::from(ctx.rng.draw(draw_site::CHICKEN_TIMER)) * 150) >> 16;
    set_chicken_timer(ctx, m, t);
    Ok(())
}

fn set_chicken_timer(ctx: &mut MapActionContext, m: usize, t: i32) {
    if let Some(AmbientData::Chicken { timer, .. }) = ctx.world.monsters[m].ambient.as_mut() {
        *timer = t;
    }
}

fn peasant(m: usize, ctx: &mut MapActionContext) -> Result<(), Unmodelled> {
    let me = &ctx.world.monsters[m];
    let Some(AmbientData::Peasant { timer, home }) = me.ambient.clone() else {
        return Err(Unmodelled::new(
            draw_site::PEASANT_WITHOUT_DATA,
            "peasant without peasant data",
        ));
    };
    let speed = i32::from(me.speed);
    if timer > 0 {
        set_peasant_timer(ctx, m, timer - 30);
        return Ok(());
    }
    let rng = ctx.rng.rng.state;
    let angle = ctx.rng.draw(draw_site::PEASANT_WANDER_ANGLE);
    let r = u32::from(ctx.rng.draw(draw_site::PEASANT_WANDER_DISTANCE));
    let d = ((r * 5) >> 6) as i32 + 0x1400;
    let (cw, ch) = ctx.world.mesh_cells;
    let p = polar(home, angle, d);
    let p = (clamp_world(p.0, cw), clamp_world(p.1, ch));
    ctx.world.order_movement(&[m], p, 0, -2)?;
    let mut t = -60;
    if speed != 0 {
        let (x, y) = ctx.world.monsters[m].position;
        t += distance2d(x - p.0, y - p.1) / speed;
    }
    t += (i32::from(ctx.rng.draw(draw_site::PEASANT_TIMER)) * 300) >> 16;
    set_peasant_timer(ctx, m, t);
    PEASANT_LOG.with(|l| {
        if let Some(log) = l.borrow_mut().as_mut() {
            log.push(PeasantRun {
                time: ctx.time,
                identifier: ctx.world.monsters[m].identifier,
                rng,
                destination: p,
                timer: t,
            });
        }
    });
    Ok(())
}

fn set_peasant_timer(ctx: &mut MapActionContext, m: usize, t: i32) {
    if let Some(AmbientData::Peasant { timer, .. }) = ctx.world.monsters[m].ambient.as_mut() {
        *timer = t;
    }
}

fn deer(m: usize, ctx: &mut MapActionContext) -> Result<(), Unmodelled> {
    let w = &ctx.world;
    let me = &w.monsters[m];
    let (x, y) = me.position;
    let (cw, ch) = w.mesh_cells;
    let (x0, x1) = cell_window(x >> 9, 6, cw);
    let (y0, y1) = cell_window(y >> 9, 6, ch);
    let defs = &w.monster_definitions;
    let alarm = w.first_monster(m, (x0, x1), (y0, y1), |o| {
        o.definition != me.definition && o.flags_8 & 4 == 0 && defs[o.definition].field_1b6 != 5
    });
    if alarm.is_some() {
        ctx.world.retreat_command(&[m], -2)?;
        let t = 0x96 + ((i32::from(ctx.rng.draw(draw_site::DEER_ALARM_TIMER)) * 60) >> 16);
        set_monster_movement_animation(m, 0x13, ctx)?;
        set_deer(ctx, m, |d| {
            if let AmbientData::Deer { flags, timer, .. } = d {
                *flags |= 1;
                *timer = t;
            }
        });
    }
    let me = &ctx.world.monsters[m];
    let vitality = me.object.vitality as u16;
    let Some(AmbientData::Deer { vitality: seen, .. }) = me.ambient.clone() else {
        return Err(Unmodelled::new(
            draw_site::DEER_WITHOUT_DATA,
            "deer without deer data",
        ));
    };
    if (vitality as i16) < (seen as i16) {
        let attacker = usize::try_from(ctx.world.combat(m).attacker)
            .ok()
            .filter(|&a| a < ctx.world.monsters.len() && ctx.world.monsters[a].identifier != 0);
        let (mx, my) = me.position;
        if let (Some(a), Some(AmbientData::Deer { home, .. })) = (attacker, me.ambient.clone()) {
            let (ax, ay) = ctx.world.monsters[a].position;
            let angle = arctangent(mx - ax, my - ay);
            let (cw, ch) = ctx.world.mesh_cells;
            let p = polar(home, angle, 0x1e00);
            let p = (clamp_world(p.0, cw), clamp_world(p.1, ch));
            ctx.world.order_movement(&[m], p, 0x20, -2)?;
            let t = 0x96 + ((i32::from(ctx.rng.draw(draw_site::DEER_HURT_TIMER)) * 60) >> 16);
            set_deer(ctx, m, |d| {
                if let AmbientData::Deer { flags, timer, .. } = d {
                    *flags |= 1;
                    *timer = t;
                }
            });
        }
        set_deer(ctx, m, |d| {
            if let AmbientData::Deer { vitality: seen, .. } = d {
                *seen = vitality;
            }
        });
    }
    let Some(AmbientData::Deer {
        flags, timer, home, ..
    }) = ctx.world.monsters[m].ambient.clone()
    else {
        unreachable!()
    };
    if timer > 0 {
        set_deer(ctx, m, |d| {
            if let AmbientData::Deer { timer, .. } = d {
                *timer -= 30;
            }
        });
        return Ok(());
    }
    let angle = ctx.rng.draw(draw_site::DEER_WANDER_ANGLE);
    let r = u32::from(ctx.rng.draw(draw_site::DEER_WANDER_DISTANCE));
    let d = ((r * 5) >> 7) as i32 + 0x200;
    let home = if flags & 1 != 0 {
        ctx.world.monsters[m].position
    } else {
        home
    };
    let (cw, ch) = ctx.world.mesh_cells;
    let p = polar(home, angle, d);
    let p = (clamp_world(p.0, cw), clamp_world(p.1, ch));
    ctx.world.order_movement(&[m], p, 0, -2)?;
    set_monster_movement_animation(m, 0x12, ctx)?;
    let speed = i32::from(ctx.world.monsters[m].speed);
    let mut t = -60;
    if speed != 0 {
        let (x, y) = ctx.world.monsters[m].position;
        t += distance2d(x - p.0, y - p.1) / speed;
    }
    t += 0x258 + ((i32::from(ctx.rng.draw(draw_site::DEER_TIMER)) * 600) >> 16);
    set_deer(ctx, m, |d| {
        if let AmbientData::Deer {
            flags: f,
            timer,
            home: h,
            ..
        } = d
        {
            *f &= !1;
            *h = home;
            *timer = t;
        }
    });
    Ok(())
}

fn set_deer(ctx: &mut MapActionContext, m: usize, f: impl FnOnce(&mut AmbientData)) {
    if let Some(d) = ctx.world.monsters[m].ambient.as_mut() {
        f(d);
    }
}

fn soaring(m: usize, ctx: &mut MapActionContext) -> Result<(), Unmodelled> {
    let Some(AmbientData::Soaring {
        timer,
        animation_timer,
    }) = ctx.world.monsters[m].ambient.clone()
    else {
        return Err(Unmodelled::new(
            draw_site::SOARING_WITHOUT_DATA,
            "soaring monster without soaring data",
        ));
    };
    if animation_timer > 0 {
        set_soaring(ctx, m, None, Some(animation_timer - 30));
    } else {
        let (site, state) = if ctx.world.monsters[m].anim_state == 0x14 {
            (draw_site::SOARING_ANIMATION_TO_12, 0x12)
        } else {
            (draw_site::SOARING_ANIMATION_TO_14, 0x14)
        };
        let t = 0x5a + ((i32::from(ctx.rng.draw(site)) * 0x1e) >> 16);
        set_soaring(ctx, m, None, Some(t));
        set_monster_movement_animation(m, state, ctx)?;
    }
    let me = &ctx.world.monsters[m];
    if me.mode == 4 && me.has_velocity() {
        if timer > 0 {
            set_soaring(ctx, m, Some(timer - 30), None);
            return Ok(());
        }
    } else {
        set_soaring(ctx, m, Some(0), None);
    }
    let (cw, ch) = ctx.world.mesh_cells;
    let mut coord = |cells: i32, cell_site: u32, frac_site: u32| {
        let mut v = 0;
        if cells as i16 > 1 {
            v = ((i32::from(ctx.rng.draw(cell_site)) * (cells - 1)) >> 16) << 9;
        }
        let r = i32::from(ctx.rng.draw(frac_site));
        v + ((r * 511) >> 16)
    };
    let px = coord(
        cw,
        draw_site::SOARING_TARGET_X,
        draw_site::SOARING_TARGET_X_FINE,
    );
    let py = coord(
        ch,
        draw_site::SOARING_TARGET_Y,
        draw_site::SOARING_TARGET_Y_FINE,
    );
    ctx.world.order_movement(&[m], (px, py), 0, -2)?;
    let speed = i32::from(ctx.world.monsters[m].speed);
    let t = if speed != 0 {
        let (x, y) = ctx.world.monsters[m].position;
        distance2d(x - px, y - py) / speed - 0x3c
    } else {
        -0x3c
    };
    set_soaring(ctx, m, Some(t), None);
    Ok(())
}

fn set_soaring(ctx: &mut MapActionContext, m: usize, t: Option<i32>, a: Option<i32>) {
    if let Some(AmbientData::Soaring {
        timer,
        animation_timer,
    }) = ctx.world.monsters[m].ambient.as_mut()
    {
        if let Some(t) = t {
            *timer = t;
        }
        if let Some(a) = a {
            *animation_timer = a;
        }
    }
}

pub fn set_monster_movement_animation(
    m: usize,
    state: u8,
    ctx: &mut MapActionContext,
) -> Result<(), Unmodelled> {
    let w = &mut *ctx.world;
    let mon = &w.monsters[m];
    if (mon.flags as i32) < 0 {
        return Err(Unmodelled::new(
            draw_site::MOVEMENT_ANIMATION_FLAG_80000000,
            "set_monster_movement_animation for monster flag 0x80000000",
        ));
    }
    if mon.object.flags & 0x80_0000 != 0 {
        return Ok(());
    }
    if mon.mode == 0xb {
        return Err(Unmodelled::new(
            draw_site::MOVEMENT_ANIMATION_MODE_B,
            "set_monster_movement_animation in mode 0xb",
        ));
    }
    if mon.flags & 0x0100_0000 != 0 {
        return Err(Unmodelled::new(
            draw_site::MOVEMENT_ANIMATION_FLAG_1000000,
            "set_monster_movement_animation with monster flag 0x1000000",
        ));
    }
    let def = &w.monster_definitions[mon.definition];
    let sequence = match state {
        0x13 if def.sequence_22 != -1 => def.sequence_22,
        0x14 if def.sequence_2a != -1 => def.sequence_2a,
        _ => def.walk_sequence,
    };
    let collection = mon.object.collection;
    let crate::engine_rng::EngineWorld {
        monsters,
        collections,
        ..
    } = &mut *w;
    let mon = &mut monsters[m];
    mon.flags_8 &= !0x0100_0000;
    mon.flags &= !0x20000;
    mon.anim_state = state;
    mon.mode = 4;
    mon.object.velocity = [0; 3];
    if sequence != -1 {
        let headers = collections
            .get(&collection)
            .map(Vec::as_slice)
            .unwrap_or(&[]);
        set_object_shape(
            &mut mon.object,
            headers,
            i32::from(sequence),
            0x100,
            3,
            ctx.rng,
        )?;
    }
    Ok(())
}

pub fn point_in_polygon(p: (i32, i32), v: &[(i32, i32)], closed: bool) -> bool {
    let n = v.len();
    let m = if closed { n + 1 } else { n };
    let fits = |d: i32| (d.wrapping_add(0x7fff) as u32) <= 0xfffd;
    for k in 1..m {
        let a = v[k - 1];
        let b = if k == n { v[0] } else { v[k] };
        let (mut px, mut py) = (p.0.wrapping_sub(a.0), p.1.wrapping_sub(a.1));
        if !fits(py) || !fits(px) {
            px >>= 9;
            py >>= 9;
        }
        let (mut ex, mut ey) = (b.0.wrapping_sub(a.0), b.1.wrapping_sub(a.1));
        if !fits(ey) || !fits(ex) {
            ex >>= 9;
            ey >>= 9;
        }
        if ey.wrapping_mul(px).wrapping_sub(ex.wrapping_mul(py)) < 0 {
            return false;
        }
    }
    true
}

const FRAMEWORK_PARAMETERS: &[&[u8; 4]] = &[
    b"acoa", b"deoa", b"acod", b"deod", b"acos", b"deos", b"acof", b"deof", b"acer", b"deer",
    b"inhi", b"prer", b"acin", b"acmp", b"acot", b"acoe", b"trsi",
];

fn is_framework_parameter(name: [u8; 4]) -> bool {
    FRAMEWORK_PARAMETERS.iter().any(|n| **n == name)
}

const TEST_UNITS_PARAMETERS: &[&[u8; 4]] = &[
    b"name", b"subj", b"enem", b"___<", b"___=", b"___>", b"poly", b"clpo", b"radi", b"crad",
    b"clos", b"<vit", b">vit", b"devi", b"=dif", b"<dif", b">dif", b"repo", b"igin", b"_map",
    b"team", b"insi", b"hold", b"holi", b"_all", b"sele", b"taun", b"ammo", b"mana", b"exp>",
    b"chr>", b"hei>", b"clmp", b"near", b"obje",
];

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct TestUnitsState {
    pub most_subjects: i16,
    pub vitality_at_start: i32,
}

fn clos_band_draw(rng: &mut EngineRng, site: u32, inner: i32, outer: i32, dist: i32) -> bool {
    let r = i32::from((rng.draw(site) >> 8) as u8);
    let q = ((i32::from((outer - dist) as i16) << 8) / (outer - inner)) as i16;
    r < i32::from(q)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SubjectThresholds {
    pub ammunition: i32,
    pub mana: i32,
    pub experience: i32,
    pub charges: Option<i32>,
    pub height: Option<i32>,
    pub selected: bool,
    pub taunting: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct SubjectFacts {
    pub ammunition: i8,
    pub experience: u8,
    pub flying_height: i16,
    pub mana: i16,
    pub max_mana: i16,
    pub charges: i16,
    pub selected: bool,
    pub mode: u8,
}

impl SubjectThresholds {
    pub fn read(
        mode: i16,
        gameplay: i32,
        has: impl Fn(&[u8; 4]) -> bool,
        long: impl Fn(&[u8; 4], i16) -> Option<i32>,
    ) -> Option<Self> {
        let selected = has(b"sele") && mode == 2;
        let taunting = has(b"taun");
        let mut t = SubjectThresholds {
            ammunition: -1,
            mana: -1,
            experience: -1,
            charges: None,
            height: None,
            selected,
            taunting,
        };
        if let Some(v) = long(b"ammo", TYPE_INTEGER) {
            t.ammunition = v;
        } else if let Some(v) = long(b"mana", 8) {
            t.mana = v;
        } else if let Some(v) = long(b"exp>", TYPE_INTEGER) {
            t.experience = v;
        } else if let Some(v) = (gameplay > game_options::GAMEPLAY_1_5_1)
            .then(|| long(b"chr>", TYPE_INTEGER))
            .flatten()
        {
            t.charges = Some(v);
        } else if let Some(v) = (gameplay > game_options::GAMEPLAY_1_5_1)
            .then(|| long(b"hei>", TYPE_INTEGER))
            .flatten()
        {
            t.height = Some(v);
        } else if !selected && !taunting {
            return None;
        }
        Some(t)
    }

    pub fn pass(&self, subjects: &[SubjectFacts], gameplay: i32) -> bool {
        let mut ok = true;
        if subjects.is_empty() {
            return gameplay < 0x6af && !self.taunting;
        }
        let mut taunting = false;
        for s in subjects {
            if let Some(v) = self.charges
                && i32::from(s.charges) <= v
            {
                ok = false;
            }
            if i32::from(s.ammunition) <= self.ammunition {
                ok = false;
            }
            if i32::from(s.experience) <= self.experience {
                ok = false;
            }
            if let Some(v) = self.height
                && i32::from(s.flying_height) <= v
            {
                ok = false;
            }
            if s.max_mana != 0
                && (i32::from(s.mana) << 16).wrapping_div(i32::from(s.max_mana)) <= self.mana
            {
                ok = false;
            }
            if self.selected && !s.selected {
                ok = false;
            }
            if s.mode == 3 {
                taunting = true;
            }
        }
        ok && !(self.taunting && !taunting)
    }
}

pub fn devi_moved(vitality_at_start: i32, mean: i32, threshold: i32) -> bool {
    vitality_at_start.wrapping_sub(mean).wrapping_abs() > threshold
}

impl MapActions {
    pub fn test_units_state(&self, i: usize) -> Option<TestUnitsState> {
        self.glo.get(&i).copied()
    }

    pub fn test_units(&mut self, i: usize, ctx: &mut MapActionContext) -> Result<u32, Unmodelled> {
        for (a, p) in self.iterate(i, ITER_EMPTY, None, None) {
            let name = self.actions[a].parameters[p].name;
            if !TEST_UNITS_PARAMETERS.iter().any(|n| **n == name.0)
                && !is_framework_parameter(name.0)
            {
                return Err(Unmodelled::new(
                    draw_site::TEST_UNITS_PARAMETER_NOT_IMPLEMENTED,
                    format!(
                        "map action {} (#{i}, 'tuni'): parameter '{name}' not implemented",
                        self.actions[i].identifier
                    ),
                ));
            }
        }
        let w = &*ctx.world;
        let repo = self.has(i, b"repo");
        let igin = self.has(i, b"igin");
        let mut state = self.glo.get(&i).copied().unwrap_or_default();
        let mut glo = state.most_subjects;
        let mut subjects: Vec<usize> = match self.long(i, b"team", TYPE_INTEGER) {
            Some(team) => w
                .monsters
                .iter()
                .enumerate()
                .filter(|(_, m)| m.identifier != 0 && i32::from(m.team as i8) == team)
                .map(|(k, _)| k)
                .take(0x190)
                .collect(),
            None => self.subjects(i, ctx, 0x190),
        };
        if subjects.len() as i16 > glo {
            glo = subjects.len() as i16;
        }
        let mut ok = true;
        if self.flag(i, b"clmp") && w.game_options & mandrake_common::game_options::ANTI_CLUMP == 0
        {
            ok = false;
        }
        let mon = |m: usize| &w.monsters[m];
        if let Some(poly) = self.find(i, 0, b"poly", Some(TYPE_POINT)) {
            let v: Vec<(i32, i32)> = poly
                .values
                .as_chunks::<2>()
                .0
                .iter()
                .map(|&[x, y]| (x, y))
                .collect();
            let closed = self.has(i, b"clpo");
            subjects.retain(|&m| {
                (!igin || mon(m).flags_8 & 4 == 0)
                    && point_in_polygon(mon(m).position, &v, closed) != repo
            });
        }
        if let Some(radi) = self.find(i, 0, b"radi", Some(TYPE_POINT)) {
            let p0 = (radi.values[0], radi.values[1]);
            let radius = match radi.count {
                1 => self.indexed(i, b"crad", TYPE_DISTANCE, 0).unwrap_or(0),
                2 => distance2d(p0.0 - radi.values[2], p0.1 - radi.values[3]),
                _ => 0,
            };
            subjects.retain(|&m| {
                let o = mon(m);
                if o.object.no_shape || (igin && o.flags_8 & 4 != 0) {
                    return false;
                }
                let d = distance2d(o.position.0 - p0.0, o.position.1 - p0.1);
                (d >= radius) == repo
            });
        }
        subjects.retain(|&m| mon(m).flags & 0x4000_0000 == 0);
        let count = subjects.len() as i32;
        if let Some(v) = self.indexed(i, b"___>", TYPE_INTEGER, 0)
            && count <= v
        {
            ok = false;
        }
        if let Some(v) = self.indexed(i, b"___<", TYPE_INTEGER, 0)
            && count >= v
        {
            ok = false;
        }
        if let Some(v) = self.indexed(i, b"___=", TYPE_INTEGER, 0)
            && count != v
        {
            ok = false;
        }
        let thresholds = SubjectThresholds::read(
            w.mode,
            w.formation_globals.gameplay_version,
            |name| self.flag(i, name),
            |name, kind| self.indexed(i, name, kind, 0),
        );
        if let Some(t) = thresholds {
            let mut facts = Vec::with_capacity(subjects.len());
            for &m in &subjects {
                let o = mon(m);
                let c = w.combat(m);
                let charges = if t.charges.is_some() {
                    w.monster_artifact_charges(m)?
                } else {
                    0
                };
                facts.push(SubjectFacts {
                    ammunition: c.ammunition as i8,
                    experience: c.experience,
                    flying_height: o.flying_height,
                    mana: c.mana,
                    max_mana: w.monster_definitions[o.definition].ai.word(0x214),
                    charges,
                    selected: w.local_selection.contains(&o.identifier),
                    mode: o.mode,
                });
            }
            if !t.pass(&facts, w.formation_globals.gameplay_version) {
                ok = false;
            }
        }
        let gt = self.indexed(i, b">vit", 8, 0);
        let lt = self.indexed(i, b"<vit", 8, 0);
        let devi = self.indexed(i, b"devi", 8, 0);
        if gt.is_some() || lt.is_some() || devi.is_some() {
            let mut sum = 0i32;
            for &m in &subjects {
                let o = mon(m);
                let max = w.monster_definitions[o.definition].object_vitality;
                if max != 0 {
                    sum = sum
                        .wrapping_add((i32::from(o.object.vitality as i16) << 16) / i32::from(max));
                }
            }
            let mean = if glo == 0 {
                0
            } else {
                ((sum as u32) / (glo as i32 as u32)) as i32
            };
            if gt.is_some_and(|v| v >= mean) {
                ok = false;
            }
            if lt.is_some_and(|v| v <= mean) {
                ok = false;
            }
            if let Some(threshold) = devi {
                if self.actions[i].flags & 0x100 != 0 {
                    state.vitality_at_start = mean;
                    ok = false;
                } else if !devi_moved(state.vitality_at_start, mean, threshold) {
                    ok = false;
                }
            }
        }
        let hold = self.long(i, b"hold", TAG_NAME);
        let holi = self.long(i, b"holi", TAG_NAME);
        if let Some(first) = hold.or(holi) {
            let gameplay = w.formation_globals.gameplay_version;
            let mut value = first;
            let mut carrying = 0;
            for &m in &subjects {
                let counted = if w.combat(m).carried != -1 {
                    w.carried_projectile(m)
                        .is_some_and(|p| i32::from_be_bytes(p.0) == value)
                } else {
                    hold.is_some() && gameplay > game_options::GAMEPLAY_1_7_2 && value == -1
                };
                if counted {
                    carrying += 1;
                    continue;
                }
                if let Some(v) = holi {
                    value = v;
                    let inventory = &w.combat(m).inventory;
                    if gameplay > game_options::GAMEPLAY_1_7_2 && value == -1 {
                        if inventory.len == 0 {
                            carrying += 1;
                        }
                    } else {
                        for &o in inventory.objects() {
                            let Some(p) = w
                                .projectiles
                                .iter()
                                .find(|p| p.object.index == i32::from(o))
                            else {
                                return Err(Unmodelled::new(
                                    draw_site::TEST_UNITS_HOLDING_OBJECT_NOT_PROJECTILE,
                                    format!(
                                        "test_units 'holi': monster {}: inventory object {o} \
                                         isn't a projectile's at rest",
                                        w.monsters[m].identifier
                                    ),
                                ));
                            };
                            if p.object.kind == 4 && i32::from_be_bytes(p.definition.0) == value {
                                carrying += 1;
                                break;
                            }
                        }
                    }
                }
            }
            if carrying == 0 || (self.flag(i, b"_all") && carrying != subjects.len()) {
                ok = false;
            }
        }
        let d = i32::from(ctx.world.difficulty);
        if self
            .indexed(i, b">dif", TYPE_INTEGER, 0)
            .is_some_and(|v| d < v)
        {
            ok = false;
        }
        if self
            .indexed(i, b"<dif", TYPE_INTEGER, 0)
            .is_some_and(|v| d > v)
        {
            ok = false;
        }
        if self
            .indexed(i, b"=dif", TYPE_INTEGER, 0)
            .is_some_and(|v| d != v)
        {
            ok = false;
        }
        if let Some(clos) = self.find(i, 0, b"clos", Some(TYPE_DISTANCE)) {
            let inner = clos.values[0];
            let outer = if clos.count > 1 {
                clos.values[1]
            } else {
                inner + 1
            };
            let listed = self.combined(i, b"enem", TYPE_MONSTERS, 0x190);
            let scan_all = listed.is_empty();
            let enemies: Vec<usize> = listed
                .into_iter()
                .filter_map(|id| w.monster_index_by_identifier(id as u16))
                .collect();
            ok = false;
            'subjects: for &s in &subjects {
                let sp = mon(s).position;
                if scan_all {
                    for o in &w.monsters {
                        if o.identifier == 0
                            || o.flags & 0x4000_0000 != 0
                            || o.team == mon(s).team
                            || (igin && o.flags_8 & 4 != 0)
                        {
                            continue;
                        }
                        let dist = distance2d(o.position.0 - sp.0, o.position.1 - sp.1);
                        if outer <= dist {
                            continue;
                        }
                        if inner < dist
                            && !clos_band_draw(
                                ctx.rng,
                                draw_site::TEST_UNITS_CLOSE_BAND,
                                inner,
                                outer,
                                dist,
                            )
                        {
                            continue;
                        }
                        if w.monster_definitions[o.definition].ai.class != 5 {
                            ok = true;
                            break 'subjects;
                        }
                    }
                    continue;
                }
                for &e in &enemies {
                    if igin && mon(e).flags_8 & 4 != 0 {
                        continue;
                    }
                    let ep = mon(e).position;
                    let dist = distance2d(ep.0 - sp.0, ep.1 - sp.1);
                    if outer <= dist {
                        continue;
                    }
                    let site = if igin {
                        draw_site::TEST_UNITS_ENEMY_IGNORING_INVISIBLE
                    } else {
                        draw_site::TEST_UNITS_ENEMY
                    };
                    if inner >= dist || clos_band_draw(ctx.rng, site, inner, outer, dist) {
                        ok = true;
                        break 'subjects;
                    }
                }
            }
        }
        state.most_subjects = glo;
        self.glo.insert(i, state);
        Ok(if ok { 2 } else { 0 })
    }
}

const UNIT_CONTROL_PARAMETERS: &[&[u8; 4]] = &[
    b"name", b"subj", b"cnba", b"visi", b"ammo", b"user", b"rein", b"igno", b"dnat", b"invi",
    b"ngtf", b"hdcf", b"mamb", b"moen", b"blam", b"tain", b"fade", b"dele", b"elim", b"proj",
    b"nena", b"majk", b"hefr", b"remo", b"deta", b"plyr", b"kill", b"swit", b"expe", b"amm+",
    b"tran", b"debg", b"_all", b"_map", b"team", b"insi", b"onii", b"unco", b"heig",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct VisibilityMode {
    visible: i16,
    invisible: i16,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct UnitControlMasks {
    and_8: u32,
    or_8: u32,
    and_4: u32,
    or_4: u32,
}

impl UnitControlMasks {
    fn new(gameplay: i32, flag: impl Fn(&[u8; 4]) -> Option<bool>) -> Self {
        let mut m = UnitControlMasks {
            and_8: !0,
            or_8: 0,
            and_4: !0,
            or_4: 0,
        };
        if let Some(on) = flag(b"user") {
            m.and_4 &= !0x800_0000;
            m.or_4 |= if on { 0x800_0000 } else { 0 };
        }
        let set_8 = |m: &mut Self, name: &[u8; 4], bit: u32| {
            if let Some(on) = flag(name) {
                m.and_8 &= !bit;
                m.or_8 |= if on { bit } else { 0 };
            }
        };
        set_8(&mut m, b"dnat", 8);
        set_8(&mut m, b"cnba", 0x10);
        if gameplay > 0x708 {
            set_8(&mut m, b"ngtf", 0x800);
            set_8(&mut m, b"hdcf", 0x2000);
            set_8(&mut m, b"mamb", 0x1000);
        }
        if flag(b"igno") == Some(false) {
            m.and_8 &= !2;
            if gameplay > game_options::GAMEPLAY_1_3 && m.or_8 & 8 == 0 {
                m.and_8 &= !8;
            }
        }
        m
    }
}

impl MapActions {
    fn general_command(&mut self, i: usize, ctx: &mut MapActionContext) -> Result<u32, Unmodelled> {
        let subjects = self.subjects(i, ctx, 0x190);
        if subjects.is_empty() {
            return Ok(0);
        }
        let Some(kind) = self.indexed(i, b"type", TYPE_INTEGER, 0) else {
            return Ok(0);
        };
        let id = self.id_of(i);
        for chunk in subjects.chunks(0x7e) {
            ctx.world
                .general_command(chunk, kind as u16, id as i16, ctx.rng, ctx.time)?;
        }
        Ok(2)
    }

    fn visibility_mode(&self, i: usize, gameplay: i32) -> VisibilityMode {
        let mut mode = if self.has(i, b"blam") {
            1
        } else if self.has(i, b"moen") {
            0
        } else {
            -1
        };
        if self.has(i, b"tain") {
            mode = 2;
        }
        let invisible = if self.has(i, b"fade") { 3 } else { mode };
        let visible = if self.has(i, b"fade") && gameplay > game_options::GAMEPLAY_1_5_1 {
            3
        } else {
            mode
        };
        VisibilityMode { visible, invisible }
    }

    fn raise_subject_markers(
        &self,
        i: usize,
        subjects: &[i32],
        ctx: &mut MapActionContext,
    ) -> Result<Option<i16>, Unmodelled> {
        let Some(p) = self.find(i, 0, b"heig", Some(TYPE_DISTANCE)) else {
            return Ok(None);
        };
        let (from, to) = (p.values[0], p.values.get(1).copied());
        let range = to.filter(|&to| p.count > 1 && to != from);
        let mut height = -1i16;
        for &id in subjects {
            if !ctx.world.has_unit_marker(id as i16)? {
                continue;
            }
            let amount = match range {
                Some(to) => from.wrapping_add(
                    i32::from(ctx.rng.draw(draw_site::UNIT_CONTROL_RAISE_MARKERS))
                        % to.wrapping_sub(from),
                ),
                None => from,
            };
            let (x, y, z) = ctx.world.raise_unit_marker(id as i16, amount);
            if ctx.world.formation_globals.gameplay_version > game_options::GAMEPLAY_1_3 {
                let ground = ctx
                    .world
                    .mesh_height(x, y, mandrake_tag::MeshHeightMode::Surface);
                height = z.wrapping_sub(ground) as i16;
            }
        }
        Ok((height != -1).then_some(height))
    }

    pub fn unit_control_action(
        &mut self,
        i: usize,
        ctx: &mut MapActionContext,
    ) -> Result<u32, Unmodelled> {
        self.unit_control(i, ctx)
    }

    fn unit_control(&mut self, i: usize, ctx: &mut MapActionContext) -> Result<u32, Unmodelled> {
        for (a, p) in self.iterate(i, ITER_EMPTY, None, None) {
            let name = self.actions[a].parameters[p].name;
            if !UNIT_CONTROL_PARAMETERS.iter().any(|n| **n == name.0)
                && !is_framework_parameter(name.0)
            {
                return Err(Unmodelled::new(
                    draw_site::UNIT_CONTROL_PARAMETER_NOT_IMPLEMENTED,
                    format!(
                        "map action {} (#{i}, 'ctrl'): parameter '{name}' not implemented",
                        self.actions[i].identifier
                    ),
                ));
            }
        }
        let subjects = self.combined(i, b"subj", TYPE_MONSTERS, 0x190);
        let height = self.raise_subject_markers(i, &subjects, ctx)?;
        let mode = self.visibility_mode(i, ctx.world.formation_globals.gameplay_version);
        if self.has(i, b"visi") {
            let reinforcements = self.has(i, b"rein");
            for &id in &subjects {
                let shown =
                    ctx.world
                        .make_monster_visible(id as i16, mode.visible, ctx.rng, ctx.time)?;
                if let (Some(m), Some(h)) = (shown, height) {
                    ctx.world.monsters[m].flying_height = h;
                }
                if reinforcements
                    && let Some(m) = shown
                    && ctx.world.monsters[m].team == ctx.world.local_team()
                {
                    ctx.world
                        .game
                        .game_sound_requests
                        .push(crate::game_state::GAME_SOUND_UNIT_GAINED);
                }
            }
        }
        if self.has(i, b"invi") {
            for &id in &subjects {
                ctx.world.make_monster_invisible_with_mode(
                    id as i16,
                    mode.invisible,
                    ctx.rng,
                    ctx.time,
                )?;
            }
        }
        let monsters: Vec<usize> = subjects
            .iter()
            .filter_map(|&id| ctx.world.monster_index_by_identifier(id as u16))
            .collect();
        if self.has(i, b"elim")
            && let Some(&first) = monsters.first()
        {
            let team = ctx.world.monsters[first].team;
            ctx.world.eliminate_team(team, Some(ctx.rng))?;
            return Ok(2);
        }
        if let Some(p) = self.find(i, 0, b"proj", Some(TYPE_OBJECTS)) {
            for (&m, &object) in monsters.iter().zip(&p.values) {
                ctx.world
                    .give_object_to_monster(m, object as i16, ctx.rng)?;
            }
        }
        let flag = |name: &[u8; 4]| {
            self.find(i, 0, name, None)
                .map(|p| p.values.first().copied().unwrap_or(0) & 0xff != 0)
        };
        let gameplay = ctx.world.formation_globals.gameplay_version;
        let masks = UnitControlMasks::new(gameplay, flag);
        let igno = flag(b"igno");
        for &m in &monsters {
            let mon = &mut ctx.world.monsters[m];
            mon.flags_8 = mon.flags_8 & masks.and_8 | masks.or_8;
            mon.flags = mon.flags & masks.and_4 | masks.or_4;
            if let Some(on) = igno {
                ctx.world.set_monster_uncontrollable(m, on)?;
            }
        }
        if let Some(on) = flag(b"unco") {
            for &m in &monsters {
                let mon = &mut ctx.world.monsters[m];
                if on {
                    mon.flags_8 |= 0x8_0002;
                } else {
                    mon.flags_8 &= !0x8_0002;
                }
                if gameplay > game_options::GAMEPLAY_1_6 {
                    ctx.world.set_monster_uncontrollable(m, on)?;
                } else if on {
                    ctx.world.monsters[m].flags |= 0x1000_0000;
                } else {
                    ctx.world.monsters[m].flags &= !0x1000_0000;
                }
            }
        }
        if self.find(i, ITER_EMPTY, b"remo", Some(TYPE_FLAG)).is_some() {
            for (k, &m) in monsters.iter().enumerate() {
                ctx.world.delete_monster_carried_object_unit_control(m, k)?;
            }
        }
        if let Some(from) = self.indexed(i, b"hefr", 8, 0) {
            let range = self
                .indexed(i, b"hefr", 8, 1)
                .map_or(0, |to| to.wrapping_sub(from))
                >> 8;
            let from = from >> 8;
            for &m in &monsters {
                let share = if range > 0 {
                    (i32::from(ctx.rng.draw(draw_site::UNIT_CONTROL_RANGE)).wrapping_mul(range)
                        >> 16)
                        .wrapping_add(from)
                } else {
                    from
                };
                let object = &mut ctx.world.monsters[m].object;
                object.vitality = i32::from((share.wrapping_mul(object.max_vitality) >> 8) as i16);
            }
        }
        if let Some(ammo) = self.long(i, b"ammo", TYPE_INTEGER) {
            for &m in &monsters {
                let c = ctx.world.combat_mut(m);
                c.byte_8d |= 0x80;
                c.ammunition = ammo as u8;
            }
        }
        if gameplay > game_options::GAMEPLAY_1_3
            && let Some(tag) = self.indexed(i, b"swit", 7, 0)
        {
            for &m in &monsters {
                ctx.world.unit_control_switch(m, tag, ctx.rng)?;
            }
        }
        if gameplay > game_options::GAMEPLAY_1_3
            && let Some(add) = self.long(i, b"amm+", TYPE_INTEGER)
        {
            for &m in &monsters {
                let c = ctx.world.combat_mut(m);
                let sum = (c.ammunition as i8).wrapping_add(add as i8);
                c.ammunition = sum.max(0) as u8;
                c.byte_8d |= 0x80;
            }
        }
        if let Some(name) = self.long(i, b"nena", TYPE_INTEGER) {
            for &m in &monsters {
                let c = ctx.world.combat_mut(m);
                c.byte_8d |= 1;
                c.name_index = name as i8;
            }
        }
        if let Some(xp) = self.long(i, b"expe", TYPE_INTEGER) {
            for &m in &monsters {
                let c = ctx.world.combat_mut(m);
                c.byte_8d |= 2;
                c.experience = xp as u8;
            }
        }
        if let Some(on) = flag(b"majk") {
            for &m in &monsters {
                ctx.world.set_monster_magical_invisibility(m, on);
            }
        }
        if let Some(on) = flag(b"tran") {
            for &m in &monsters {
                let object = &mut ctx.world.monsters[m].object;
                if on {
                    object.flags |= 0x1000_0000;
                } else {
                    object.flags &= !0x1000_0000;
                }
            }
        }
        if let Some(on) = flag(b"onii") {
            let (kind, other) = if on { (5u8, 6u8) } else { (6, 5) };
            for &m in &monsters {
                let object = &mut ctx.world.monsters[m].object;
                if object.sub_kind == kind {
                    continue;
                }
                object.sub_count = if object.sub_kind == other {
                    30u8.wrapping_sub(object.sub_count)
                } else {
                    0
                };
                object.sub_limit = 30;
                object.sub_kind = kind;
            }
        }
        let kill = self.long(i, b"kill", TYPE_INTEGER);
        if self.has(i, b"dele") && !(gameplay > game_options::GAMEPLAY_1_5_1 && kill.is_some()) {
            for &m in &monsters {
                if ctx.world.monsters[m].identifier == 0 {
                    continue;
                }
                ctx.world.deleted_by_action_statistics(m);
                crate::engine_rng::WorldHooks {
                    world: &mut *ctx.world,
                    actions: &mut None,
                    rng: &mut *ctx.rng,
                    monster: m,
                    time: ctx.time,
                    host: &mut crate::engine_attack::NoHost,
                }
                .delete_monster()?;
            }
        }
        self.give_subjects_to_player(i, ctx, &monsters)?;
        if gameplay > game_options::GAMEPLAY_1_3
            && let Some(kill) = kill
        {
            self.kill_subjects(i, ctx, &monsters, kill)?;
        }
        Ok(2)
    }

    fn kill_subjects(
        &mut self,
        i: usize,
        ctx: &mut MapActionContext,
        monsters: &[usize],
        kill: i32,
    ) -> Result<(), Unmodelled> {
        let code = if kill != 0 { 0x27 } else { 0x26 };
        let gameplay = ctx.world.formation_globals.gameplay_version;
        let dele = gameplay > game_options::GAMEPLAY_1_5_1 && self.has(i, b"dele");
        let identifier = self.actions[i].identifier;
        let mut actions = Some(std::mem::take(self));
        let r = kill_monsters(ctx, &mut actions, monsters, code, dele, (i, identifier));
        *self = actions
            .take()
            .expect("map actions restored after kill_monsters");
        r
    }
}

fn change_owner(
    ctx: &mut MapActionContext,
    m: usize,
    player: i16,
    rename: bool,
) -> Result<(), Unmodelled> {
    let mut mover = ctx.world.movers.get_mut(m).and_then(Option::take);
    let r = crate::engine_rng::WorldHooks {
        world: &mut *ctx.world,
        actions: &mut None,
        rng: &mut *ctx.rng,
        monster: m,
        time: ctx.time,
        host: &mut crate::engine_attack::NoHost,
    }
    .change_monster_owner(mover.as_mut(), player, rename);
    if let Some(slot) = ctx.world.movers.get_mut(m) {
        *slot = mover;
    }
    r
}

fn kill_monsters(
    ctx: &mut MapActionContext,
    actions: &mut Option<MapActions>,
    monsters: &[usize],
    code: u8,
    dele: bool,
    (i, identifier): (usize, u16),
) -> Result<(), Unmodelled> {
    for &m in monsters {
        let mon = &ctx.world.monsters[m];
        if mon.identifier == 0 {
            return Err(Unmodelled::new(
                draw_site::UNIT_CONTROL_KILL_DEAD_MONSTER,
                format!(
                    "map action {identifier} (#{i}, 'ctrl'): 'kill' of monster {m}, which is dead"
                ),
            ));
        }
        if dele {
            let f8 = mon.flags_8;
            let hit_points = mon.object.vitality as i16;
            ctx.world.adjust_damage_statistics(
                crate::statistics::UnitRef::NONE,
                m,
                hit_points,
                true,
                f8 & 0x2800,
                f8 & 0x1000,
                true,
            );
        }
        let mon = &ctx.world.monsters[m];
        let point = [mon.position.0, mon.position.1, mon.object.z];
        crate::engine_rng::WorldHooks {
            world: &mut *ctx.world,
            actions,
            rng: &mut *ctx.rng,
            monster: m,
            time: ctx.time,
            host: &mut crate::engine_attack::NoHost,
        }
        .kill_monster_without_damage(code, point, 0x19)?;
    }
    Ok(())
}

impl MapActions {
    fn give_subjects_to_player(
        &self,
        i: usize,
        ctx: &mut MapActionContext,
        monsters: &[usize],
    ) -> Result<(), Unmodelled> {
        let count = ctx.world.players.players.len() as i32;
        let mut player = -1;
        if ctx.world.formation_globals.gameplay_version > game_options::GAMEPLAY_1_7_2
            && let Some(p) = self.long(i, b"plyr", TYPE_INTEGER)
            && (0..count).contains(&p)
        {
            player = p;
        }
        let deta = if player == -1 {
            match self.long(i, b"deta", TYPE_INTEGER) {
                Some(n) => n,
                None => return Ok(()),
            }
        } else {
            0
        };
        let stop = |at: u32, what: String| {
            Err(Unmodelled::new(
                at,
                format!(
                    "map action {} (#{i}, 'ctrl'): {what}",
                    self.actions[i].identifier
                ),
            ))
        };
        for &m in monsters {
            let team = ctx.world.monsters[m].team as i8;
            let mut target = player;
            if player == -1 {
                let Some(record) = usize::try_from(team)
                    .ok()
                    .and_then(|t| ctx.world.players.teams.get(t))
                else {
                    return stop(
                        draw_site::UNIT_CONTROL_DETACH_TEAM,
                        format!("'deta' on a subject of team {team}"),
                    );
                };
                let captain = i32::from(record.player);
                if deta == 0 {
                    target = captain;
                } else {
                    let found = (0..count)
                        .filter(|&j| {
                            let pl = &ctx.world.players.players[j as usize];
                            j != captain && pl.team == team && pl.flags & 4 == 0
                        })
                        .nth(usize::try_from(deta - 1).unwrap_or(usize::MAX));
                    match found {
                        Some(j) => {
                            ctx.world.detach_monster(m, j as i16)?;
                            continue;
                        }
                        None => continue,
                    }
                }
            }
            if target >= count {
                continue;
            }
            let Some(record) = usize::try_from(target)
                .ok()
                .and_then(|p| ctx.world.players.players.get(p))
            else {
                return stop(
                    draw_site::UNIT_CONTROL_DETACH_PLAYER,
                    format!("'deta' to player {target}"),
                );
            };
            if record.team == team {
                ctx.world.detach_monster(m, target as i16)?;
            } else {
                change_owner(ctx, m, target as i16, false)?;
            }
        }
        Ok(())
    }
}

impl MapActions {
    fn set_param(&mut self, i: usize, name: &[u8; 4], kind: i16, values: Vec<i32>) {
        let per = MapActionParameter::scalars_per_element(kind);
        let p = MapActionParameter {
            kind,
            count: (values.len() / per) as i16,
            name: FourCC::new(*name),
            values,
        };
        let params = &mut self.actions[i].parameters;
        params.retain(|q| !(q.name.0 == *name && q.kind == kind));
        params.push(p);
    }

    pub(crate) fn find_ref(
        &self,
        i: usize,
        flags: u32,
        name: &[u8; 4],
        kind: Option<i16>,
    ) -> Option<ParamRef> {
        self.iterate(
            i,
            (flags & ITER_NO_LINKS) | ITER_EMPTY,
            Some(tag(name)),
            kind,
        )
        .into_iter()
        .find(|&r| {
            let c = self.param(r).count;
            match (flags & 2 != 0, flags & 4 != 0) {
                (true, true) => c != 1,
                (true, false) => true,
                (false, true) => c > 1,
                (false, false) => c > 0,
            }
        })
    }

    fn id_of(&self, i: usize) -> i32 {
        i32::from(self.actions[i].identifier as i16)
    }

    fn platoon(&mut self, i: usize, ctx: &mut MapActionContext) -> Result<u32, Unmodelled> {
        if let Some(r) = self.find_ref(i, 0, b"stat", Some(TYPE_ACTIONS)) {
            let stat = self.param(r).values.clone();
            let k = i32::from(ctx.rng.draw(draw_site::PLATOON_STATE)) % stat.len() as i32;
            let t = self.index_of(stat[k as usize]);
            self.actions[r.0].parameters.remove(r.1);
            if self.actions[i].tag == tag(b"plat") {
                match t {
                    None => self.set_param(i, b"#cst", TYPE_ACTIONS, vec![]),
                    Some(t) => {
                        self.set_param(t, b"#sub", TYPE_INTEGER, vec![-1]);
                        let id = self.id_of(t);
                        self.set_param(i, b"#cst", TYPE_ACTIONS, vec![id]);
                        self.reset_platoon(i, ctx)?;
                    }
                }
            }
        }
        let Some(cst) = self.indexed(i, b"#cst", TYPE_ACTIONS, 0) else {
            return Ok(4);
        };
        let Some(a) = self.index_of(cst) else {
            return Ok(4);
        };
        let id = self.id_of(i);
        self.set_param(a, b"#own", TYPE_ACTIONS, vec![id]);
        self.trigger(a, ctx)?;
        Ok(0)
    }

    fn reset_platoon(&mut self, i: usize, ctx: &mut MapActionContext) -> Result<(), Unmodelled> {
        if self.actions[i].tag != tag(b"plat") {
            return Ok(());
        }
        if let Some(r) = self.find_ref(i, 3, b"#squ", Some(TYPE_ACTIONS)) {
            self.actions[r.0].parameters.remove(r.1);
        }
        self.set_param(i, b"#squ", TYPE_ACTIONS, vec![]);
        let Some(init) = self
            .find(i, 0, b"init", Some(TYPE_ACTIONS))
            .map(|p| p.values.clone())
        else {
            return Ok(());
        };
        let id = self.id_of(i);
        for s in init {
            if let Some(a) = self.index_of(s) {
                self.set_param(a, b"#owp", TYPE_ACTIONS, vec![id]);
                self.trigger(a, ctx)?;
            }
        }
        Ok(())
    }

    fn squad(&mut self, i: usize, _ctx: &mut MapActionContext) -> Result<u32, Unmodelled> {
        let Some(owner) = self
            .indexed(i, b"#owp", TYPE_ACTIONS, 0)
            .and_then(|id| self.index_of(id))
        else {
            return Ok(0);
        };
        let Some(r) = self.find_ref(owner, 3, b"#squ", Some(TYPE_ACTIONS)) else {
            return Ok(0);
        };
        let id = self.id_of(i);
        let p = &mut self.actions[r.0].parameters[r.1];
        p.values.push(id);
        p.count += 1;
        Ok(0)
    }

    fn lead(&mut self, i: usize, ctx: &mut MapActionContext) -> Result<u32, Unmodelled> {
        let subjects = self.subjects(i, ctx, 0x190);
        let code = self.lead_inner(i, &subjects, ctx)?;
        let list: Option<&[u8; 4]> = match code {
            -1 => return Ok(0),
            0 | 1 => None,
            2 => Some(b"died"),
            3 => Some(b"gone"),
            4 => Some(b"succ"),
            _ => return Ok(0),
        };
        if let Some(name) = list {
            self.activate_listed(i, name, 5, ctx)?;
        }
        Ok(4)
    }

    fn activate_listed(
        &mut self,
        i: usize,
        name: &[u8; 4],
        max: usize,
        ctx: &mut MapActionContext,
    ) -> Result<(), Unmodelled> {
        for id in self.combined(i, name, TYPE_ACTIONS, max) {
            let j = self.index_of(id);
            self.activate(j, ctx)?;
        }
        Ok(())
    }

    fn lead_inner(
        &mut self,
        i: usize,
        subjects: &[usize],
        ctx: &mut MapActionContext,
    ) -> Result<i32, Unmodelled> {
        if subjects.is_empty() {
            return Ok(1);
        }
        let Some(leader_id) = self.indexed(i, b"lead", TYPE_MONSTERS, 0) else {
            return Ok(0);
        };
        let (Some(mira), Some(dera), Some(mara)) = (
            self.long(i, b"mira", TYPE_DISTANCE),
            self.long(i, b"dera", TYPE_DISTANCE),
            self.long(i, b"mara", TYPE_DISTANCE),
        ) else {
            return Ok(0);
        };
        if mira > dera || dera > mara {
            return Ok(0);
        }
        let Some(kind) = self.long(i, b"type", TYPE_INTEGER) else {
            return Ok(0);
        };
        let w = &*ctx.world;
        let Some(leader) = w.monster_index_by_identifier(leader_id as u16) else {
            return Ok(2);
        };
        let lp = w.monsters[leader].position;
        let n = subjects.len() as i32;
        let mut near = (0, 0);
        let mut best = -1;
        let mut center = (0i32, 0i32);
        for &m in subjects {
            let q = w.monsters[m].position;
            let d = distance2d(q.0 - lp.0, q.1 - lp.1);
            if best == -1 || d < best {
                best = d;
                near = q;
            }
            center.0 = center.0.wrapping_add(q.0 / n);
            center.1 = center.1.wrapping_add(q.1 / n);
        }
        let rata = self.long(i, b"rata", TYPE_DISTANCE).unwrap_or(0x400);
        let pair = |s: &Self, name: &[u8; 4]| match s.find(i, 0, name, Some(TYPE_INTEGER)) {
            Some(p) if p.count == 2 => (p.values[0], p.values[1]),
            _ => (0x78, 0),
        };
        let deby = pair(self, b"deby");
        let debe = pair(self, b"debe");
        let mrbf = self.long(i, b"mrbf", TYPE_DISTANCE).unwrap_or(0xc00);
        let flra = self.long(i, b"flra", TYPE_DISTANCE).unwrap_or(0x400);
        let yect = self.long(i, b"yect", TYPE_INTEGER).unwrap_or(-1);
        let bect = self.long(i, b"bect", TYPE_INTEGER).unwrap_or(-1);
        let route: Vec<(i32, i32)> = match kind {
            0 => match self.waypoints(i) {
                Some(r) if r.len() <= 15 => r,
                _ => return Ok(0),
            },
            1 => {
                let Some(id) = self.indexed(i, b"mons", TYPE_MONSTERS, 0) else {
                    return Ok(0);
                };
                match w.monster_index_by_identifier(id as u16) {
                    Some(m) => vec![w.monsters[m].position],
                    None => return Ok(3),
                }
            }
            2 => {
                let Some(id) = self.indexed(i, b"obje", TYPE_OBJECTS, 0) else {
                    return Ok(0);
                };
                let id = id as i16;
                if id != 0 && w.fade_with_identifier(id) {
                    return Err(Unmodelled::new(
                        draw_site::LEAD_FADE_RECORD_OBJECT,
                        format!("lead: 'obje' names {id}, a fade record's object"),
                    ));
                }
                let mut point = None;
                for o in w.filter_objects() {
                    if id == 0 || o.identifier != id {
                        continue;
                    }
                    if !o.no_shape {
                        point = Some(o.point);
                        break;
                    }
                    let carrier = (0..w.monsters.len()).find(|&m| {
                        let mon = &w.monsters[m];
                        mon.identifier != 0
                            && mon.flags & 0x4000_0000 == 0
                            && i32::from(w.combat(m).carried) == o.index
                    });
                    if let Some(m) = carrier {
                        point = Some(w.monsters[m].position);
                        break;
                    }
                }
                match point {
                    Some(p) => vec![p],
                    None => return Ok(3),
                }
            }
            _ => Vec::new(),
        };
        let mut code = -1;
        let mut stt = self.long(i, b"#stt", TYPE_INTEGER).unwrap_or(1);
        let mut tye = self.long(i, b"#tye", TYPE_INTEGER).unwrap_or(0);
        let mut yel = self.long(i, b"#yel", TYPE_INTEGER).unwrap_or(yect);
        let mut tbe = self.long(i, b"#tbe", TYPE_INTEGER).unwrap_or(0);
        let mut bec = self.long(i, b"#bec", TYPE_INTEGER).unwrap_or(bect);
        let mut way = self.long(i, b"#way", TYPE_INTEGER).unwrap_or(0);
        let count = route.len() as i32;
        let target = usize::try_from(way)
            .ok()
            .and_then(|k| route.get(k).copied())
            .ok_or_else(|| {
                Unmodelled::new(
                    draw_site::LEAD_ROUTE_INDEX_OUTSIDE,
                    "lead route index outside the route",
                )
            })?;
        if distance2d(target.0 - lp.0, target.1 - lp.1) < rata {
            way += 1;
            if way >= count {
                code = 4;
            }
        } else {
            let succeeded = self.actions[i].flags & FLAG_SUCCEEDED != 0;
            let held = if succeeded { 0 } else { stt as i16 as i32 };
            let d = distance2d(near.0 - lp.0, near.1 - lp.1);
            let new = if d < mira || d > mara {
                if way < count {
                    let mut k = way;
                    let mut b =
                        distance2d(near.0 - route[k as usize].0, near.1 - route[k as usize].1);
                    for j in way + 1..count {
                        let e =
                            distance2d(near.0 - route[j as usize].0, near.1 - route[j as usize].1);
                        if e < b {
                            b = e;
                            k = j;
                        }
                    }
                    way = k;
                }
                1
            } else {
                held
            };
            let changed = stt != new;
            stt = new;
            let dest = match new {
                0 => lp,
                1 => closest_point_on_circle(near, dera, target),
                _ => {
                    return Err(Unmodelled::new(
                        draw_site::LEAD_STATE_UNKNOWN,
                        format!("lead state {new}"),
                    ));
                }
            };
            let w = &*ctx.world;
            let team = w.monsters[leader].team;
            let mut enemy: Option<(i32, usize)> = None;
            for (k, m) in w.monsters.iter().enumerate() {
                if m.identifier == 0 || m.flags & 0x4000_0000 != 0 || m.team == team || m.team == -1
                {
                    continue;
                }
                let e = distance2d(dest.0 - m.position.0, dest.1 - m.position.1);
                if enemy.is_none_or(|(b, _)| e < b) {
                    enemy = Some((e, k));
                }
            }
            if let Some((e, k)) = enemy
                && mrbf > e
            {
                let from = w.monsters[k].position;
                let away = closest_point_on_circle(center, -flra, from);
                let id = self.id_of(i);
                ctx.world.order_waypoints(&[leader], &[away], 0, id)?;
                if ctx.time > tye && !(yel != -1 && yel <= 0) {
                    self.activate_listed(i, b"flee", 5, ctx)?;
                    let mut t = 0;
                    if deby.1 > 0 {
                        t = (i32::from(ctx.rng.draw(draw_site::LEAD_DELAY_BY)) * deby.1) >> 16;
                    }
                    tye = t + deby.0 + ctx.time;
                    if yel > 0 {
                        yel -= 1;
                    }
                }
            } else {
                if changed {
                    yel = yect;
                    bec = bect;
                }
                match stt {
                    0 => {
                        if ctx.time > tbe && !(bec != -1 && bec <= 0) {
                            self.activate_listed(i, b"beck", 5, ctx)?;
                            let mut t = 0;
                            if debe.1 > 0 {
                                t = (i32::from(ctx.rng.draw(draw_site::LEAD_DELAY_BETWEEN))
                                    * debe.1)
                                    >> 16;
                            }
                            tbe = t + debe.0 + ctx.time;
                            if bec > 0 {
                                bec -= 1;
                            }
                        }
                    }
                    1 => {
                        let id = self.id_of(i);
                        ctx.world.order_waypoints(&[leader], &[dest], 0, id)?;
                    }
                    _ => {}
                }
            }
        }
        for (name, v) in [
            (b"#stt", stt),
            (b"#tye", tye),
            (b"#yel", yel),
            (b"#tbe", tbe),
            (b"#bec", bec),
            (b"#way", way),
        ] {
            self.set_param(i, name, TYPE_INTEGER, vec![v]);
        }
        Ok(code)
    }

    fn movement_command(
        &mut self,
        i: usize,
        ctx: &mut MapActionContext,
    ) -> Result<u32, Unmodelled> {
        let subjects: Vec<usize> = self
            .subjects(i, ctx, 0x7e)
            .into_iter()
            .filter(|&m| ctx.world.monsters[m].flags & 0x0800_0000 == 0)
            .collect();
        if self.combined(i, b"subj", TYPE_MONSTERS, 0x7e).is_empty() {
            return Ok(0);
        }
        let wayp_ref = self.find_ref(i, 2, b"wayp", Some(TYPE_POINT));
        if let Some(r) = wayp_ref
            && self.has(i, b"clos")
        {
            let c = ctx.world.centerpoint(&subjects);
            let pts: Vec<(i32, i32)> = self
                .param(r)
                .values
                .chunks(2)
                .map(|c| (c[0], c[1]))
                .collect();
            let mut best = 0usize;
            if !pts.is_empty() {
                let mut d = distance2d(c.0 - pts[0].0, c.1 - pts[0].1);
                for (k, p) in pts.iter().enumerate().skip(1) {
                    let e = distance2d(c.0 - p.0, c.1 - p.1);
                    if d > e {
                        d = e;
                        best = k;
                    }
                }
                let p = pts[best];
                let param = &mut self.actions[r.0].parameters[r.1];
                param.values = vec![p.0, p.1];
                param.count = 1;
            }
        }
        let wayp: Vec<(i32, i32)> = wayp_ref
            .map(|r| {
                self.param(r)
                    .values
                    .chunks(2)
                    .map(|c| (c[0], c[1]))
                    .collect()
            })
            .unwrap_or_default();
        if wayp.len() > 4 {
            return Ok(0);
        }
        let mut header = MovementHeader::default();
        if self.has(i, b"cycl") {
            header.flags |= 1;
        } else if self.has(i, b"back") {
            header.flags |= 3;
        }
        if self.has(i, b"prta") {
            header.flags |= 4;
        }
        if let Some(f) = self.indexed(i, b"form", TYPE_INTEGER, 0) {
            header.flags |= 8;
            header.formation = f as i16;
        }
        let obfa: Vec<usize> = self
            .combined(i, b"obfa", TYPE_MONSTERS, 0x7e)
            .into_iter()
            .filter_map(|id| ctx.world.monster_index_by_identifier(id as u16))
            .collect();
        if !obfa.is_empty() {
            let a = ctx.world.centerpoint(&subjects);
            let b = ctx.world.centerpoint(&obfa);
            header.flags |= 0x10;
            header.facing = arctangent(b.0.wrapping_sub(a.0), b.1.wrapping_sub(a.1));
        }
        if let Some(f) = self.indexed(i, b"fifa", 4, 0) {
            header.flags |= 0x10;
            header.facing = f as u16;
        }
        if subjects.is_empty() {
            return Ok(0);
        }
        let id = self.id_of(i);
        ctx.world.movement_command(&subjects, &wayp, header, id)?;
        Ok(0)
    }

    fn sniffer_action(&mut self, i: usize, ctx: &mut MapActionContext) -> Result<u32, Unmodelled> {
        let Some(case) = self.indexed(i, b"case", TYPE_INTEGER, 0) else {
            return Ok(0);
        };
        let mut flags = 0;
        if !self.has(i, b"nclr") {
            flags |= 1;
        }
        if self.has(i, b"keep") {
            flags |= 2;
        }
        let (mut base, mut range) = (0x96, 0);
        if let Some(t) = self.find(i, 0, b"time", Some(TYPE_INTEGER))
            && t.count == 2
        {
            (base, range) = (t.values[0], t.values[1]);
        }
        let timeout = self.indexed(i, b"inde", TYPE_INTEGER, 0).unwrap_or(0x96);
        let case = case as i16;
        if case == 1 {
            return Ok(if self.overhead_map_state == 2 { 2 } else { 0 });
        }
        self.sniffer = Some(Sniffer {
            case,
            flags: flags | 8,
            action: self.actions[i].identifier as i16,
            delay: base,
            range,
            deadline: ctx.time + timeout,
        });
        Ok(0)
    }

    fn sniffer_matched(&mut self, ctx: &mut MapActionContext) -> Result<(), Unmodelled> {
        let Some(s) = self.sniffer else {
            return Ok(());
        };
        if s.flags & 4 == 0
            && let Some(j) = self.index_of(i32::from(s.action))
        {
            self.succeed(j, ctx)?;
        }
        let Some(s) = self.sniffer.as_mut() else {
            return Ok(());
        };
        if s.flags & 1 != 0 {
            s.case = 0;
        }
        if s.flags & 2 == 0 {
            s.flags &= !8;
        }
        s.flags |= 4;
        Ok(())
    }

    fn sniffer_time_out(
        &mut self,
        site: u32,
        ctx: &mut MapActionContext,
    ) -> Result<(), Unmodelled> {
        let Some(s) = self.sniffer else {
            return Ok(());
        };
        if s.flags & 8 == 0 || (s.deadline != -1 && ctx.time <= s.deadline) {
            return Ok(());
        }
        if let Some(j) = self.index_of(i32::from(s.action)) {
            self.error(j, ctx)?;
        }
        let r = delay_draw(s.range, site, ctx.rng);
        if let Some(s) = self.sniffer.as_mut() {
            s.deadline = ctx.time + s.delay + r;
        }
        Ok(())
    }

    pub fn handle_command_sniffing(
        &mut self,
        command: &SniffedCommand,
        ctx: &mut MapActionContext,
    ) -> Result<bool, Unmodelled> {
        let case = self.sniffer.map_or(0, |s| s.case);
        if !(0x64..=0xa2).contains(&case) || command.kind() <= 1 {
            return Ok(true);
        }
        let matched = match (case, command) {
            (100..=104, SniffedCommand::General { command }) => *command == case - 100,
            (105..=108, c) => i32::from(c.kind()) == i32::from(case) - 102,
            (109, SniffedCommand::Movement { .. } | SniffedCommand::Target { .. }) => true,
            (109, SniffedCommand::General { command }) => *command == 0,
            (
                150 | 151,
                SniffedCommand::Target {
                    flags,
                    units,
                    targets,
                },
            ) if flags & 2 != 0 => {
                let n = units.len().min(targets.len());
                (units[..n] == targets[..n]) == (case == 150)
            }
            (152, SniffedCommand::Target { flags, .. }) => flags & 2 != 0,
            (160, SniffedCommand::Movement { flags, formation }) => {
                flags & 8 != 0 && *formation == 2
            }
            (161, SniffedCommand::Movement { flags, formation }) => {
                flags & 8 != 0 && *formation == 8
            }
            (162, SniffedCommand::Movement { flags, .. }) => flags & 8 != 0,
            _ => false,
        };
        if matched {
            self.sniffer_matched(ctx)?;
            return Ok(true);
        }
        self.sniffer_time_out(draw_site::SNIFFING_COMMAND_TIME_OUT, ctx)?;
        Ok(false)
    }

    pub fn handle_action_sniffing(
        &mut self,
        action: &InputAction,
        ctx: &mut MapActionContext,
    ) -> Result<bool, Unmodelled> {
        let case = self.sniffer.map_or(0, |s| s.case);
        if !(0x32..=0x37).contains(&case) {
            return Ok(true);
        }
        const WATCHED: [u32; 12] = [0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 0x1e, 0x1f];
        let any = WATCHED.iter().any(|&f| action.flag(f));
        let plain_key = action.key == -1 || (0..=15).contains(&action.key);
        if plain_key && !any {
            return Ok(true);
        }
        let own = match case {
            0x32 => vec![0x1e],
            c => {
                let k = (c - 0x33) as u32 * 2;
                vec![k, k + 1]
            }
        };
        if own.iter().any(|&f| action.flag(f)) {
            self.sniffer_matched(ctx)?;
            return Ok(true);
        }
        self.sniffer_time_out(draw_site::SNIFFING_ACTION_TIME_OUT, ctx)?;
        Ok(false)
    }

    fn scan_attack_targets(
        w: &EngineWorld,
        max: usize,
        wanted: impl Fn(&crate::engine_rng::EngineMonster) -> bool,
    ) -> Vec<usize> {
        let mut out = Vec::new();
        for (j, m) in w.monsters.iter().enumerate() {
            if out.len() >= max {
                break;
            }
            if m.identifier == 0 || !wanted(m) {
                continue;
            }
            if m.flags_8 & 0x1004 != 0 || w.combat(j).marker_flags & 8 != 0 {
                continue;
            }
            out.push(j);
        }
        out
    }

    pub fn attack_command(
        &mut self,
        i: usize,
        ctx: &mut MapActionContext,
    ) -> Result<u32, Unmodelled> {
        let id = self.actions[i].identifier;
        let chrm = self.flag(i, b"chrm");
        let mut subjects = self.subjects(i, ctx, 0x190);
        if !chrm {
            subjects.retain(|&m| ctx.world.monsters[m].flags & 0x0800_0000 == 0);
        }
        if subjects.is_empty() {
            return Ok(4);
        }
        if self.flag(i, b"_all") {
            crate::map_attacks::attack_with_monster_indexes(
                ctx.world,
                &subjects,
                crate::map_attacks::AttackConfig::new(),
            );
            return Ok(2);
        }
        if let Some(q) = self.find(i, 2, b"wayp", Some(TYPE_POINT)) {
            if q.count == 0 {
                return Ok(0);
            }
            let mut flags = 4u16;
            if self.flag(i, b"spec") {
                flags |= 1;
            }
            let n = i32::from(q.count);
            let k = (i32::from(ctx.rng.draw(draw_site::ATTACK_TARGET)) % n) as usize;
            let Some(&[x, y]) = k.checked_mul(2).and_then(|j| q.values.get(j..j + 2)) else {
                return Err(Unmodelled::new(
                    draw_site::ATTACK_TARGET,
                    format!(
                        "map action {id} (#{i}, 'atta'): 'wayp' index {k} out of range (count {})",
                        q.count
                    ),
                ));
            };
            let (_, z) = ctx.world.local_attack_location(x, y, false);
            ctx.world
                .handle_attack_location_command(i32::from(id), &subjects, flags, (x, y, z))?;
            return Ok(2);
        }
        if let Some(q) = self.find(i, 2, b"proj", Some(TYPE_OBJECTS)) {
            if q.count == 0 {
                return Ok(0);
            }
            let flags = u16::from(self.flag(i, b"spec"));
            let n = i32::from(q.count);
            let values = q.values.clone();
            let k = ctx.rng.draw_mod(draw_site::ATTACK_OBJECT_TARGET, n)? as usize;
            let wanted = values.get(k).copied().unwrap_or(0) as i16;
            if wanted != 0 && ctx.world.fade_with_identifier(wanted) {
                return Err(Unmodelled::new(
                    draw_site::ATTACK_FADE_RECORD_OBJECT,
                    format!(
                        "map action {id} (#{i}, 'atta'): 'proj' names {wanted}, a fade record's object"
                    ),
                ));
            }
            let Some(o) = ctx
                .world
                .filter_objects()
                .into_iter()
                .find(|o| wanted != 0 && o.identifier == wanted)
            else {
                return Ok(0);
            };
            ctx.world.handle_attack_location_command(
                i32::from(id),
                &subjects,
                flags,
                (o.point.0, o.point.1, o.z),
            )?;
            return Ok(2);
        }
        let repo = self.flag(i, b"repo");
        let mut targets: Vec<usize> = self
            .combined(i, b"obje", TYPE_MONSTERS, 0x7e)
            .into_iter()
            .filter_map(|t| ctx.world.monster_index_by_identifier(t as u16))
            .collect();
        let max = if ctx.world.formation_globals.gameplay_version < 0x515 {
            0x1e
        } else {
            0x7e
        };
        if let Some(team) = self.long(i, b"team", TYPE_INTEGER) {
            if targets.is_empty() {
                targets = Self::scan_attack_targets(ctx.world, max, |m| i32::from(m.team) == team);
            } else {
                targets.retain(|&t| {
                    let m = &ctx.world.monsters[t];
                    m.flags_8 & 4 == 0 && i32::from(m.team) == team
                });
            }
        }
        if let Some(kind) = self.long(i, b"type", TYPE_INTEGER) {
            if targets.is_empty() {
                let own = subjects.first().map(|&m| ctx.world.monsters[m].team);
                let defs = &ctx.world.monster_definitions;
                targets = Self::scan_attack_targets(ctx.world, max, |m| {
                    i32::from(defs[m.definition].field_1b6) == kind && Some(m.team) != own
                });
            } else {
                targets.retain(|&t| {
                    let m = &ctx.world.monsters[t];
                    let class = ctx.world.monster_definitions[m.definition].field_1b6;
                    m.flags_8 & 4 == 0 && ((kind == 0) == (class == 1))
                });
            }
        }
        if let Some(poly) = self
            .find(i, 0, b"poly", Some(TYPE_POINT))
            .map(|p| p.values.chunks(2).map(|v| (v[0], v[1])).collect::<Vec<_>>())
        {
            let closed = self.flag(i, b"clpo");
            targets.retain(|&t| {
                let m = &ctx.world.monsters[t];
                m.flags_8 & 4 == 0 && point_in_polygon(m.position, &poly, closed) != repo
            });
        }
        if targets.is_empty() {
            return Ok(0);
        }
        if chrm && ctx.world.formation_globals.gameplay_version > game_options::GAMEPLAY_1_5_1 {
            let first = subjects[0];
            let (team, owner) = {
                let m = &ctx.world.monsters[first];
                (m.team, m.owner)
            };
            let mut result = 0;
            for &t in &targets {
                let m = &ctx.world.monsters[t];
                if m.team == team && m.owner == owner {
                    continue;
                }
                let definition = ctx.world.own_definition(t) as i16;
                if !ctx
                    .world
                    .statistics
                    .units
                    .iter()
                    .any(|u| u.owner == owner && u.mons == definition)
                {
                    continue;
                }
                change_owner(ctx, t, owner, true)?;
                result = 2;
            }
            return Ok(result);
        }
        let mut flags = 0u16;
        if self.flag(i, b"dore") {
            flags |= 0x10;
        }
        if self.flag(i, b"clos") {
            flags |= 1;
        }
        if self.flag(i, b"near") {
            flags |= 8;
        } else if self.flag(i, b"spec") {
            flags |= 2;
        }
        for chunk in subjects.chunks(0x7e) {
            crate::map_attacks::handle_target_command(ctx.world, i as i32, flags, chunk, &targets)?;
        }
        Ok(2)
    }

    fn game_type_action(
        &mut self,
        i: usize,
        ctx: &mut MapActionContext,
    ) -> Result<u32, Unmodelled> {
        let types = self.combined(i, b"type", TYPE_INTEGER, 10);
        let mut ok = types.is_empty() || types.contains(&i32::from(ctx.world.game_type));
        let score = self.indexed(i, b"scor", TYPE_INTEGER, 0).unwrap_or(0);
        if let Some(team) = self.indexed(i, b"team", TYPE_INTEGER, 0) {
            ok = false;
            let ties = self.flag(i, b"ties");
            for r in &ctx.world.statistics.team_rankings {
                if r.rank != 0 {
                    continue;
                }
                if i32::from(r.team) == team {
                    if r.score >= score {
                        ok = true;
                    }
                } else if !ties {
                    return Ok(0);
                }
            }
        }
        if let Some(patch) = self.indexed(i, b"patc", TYPE_INTEGER, 0) {
            let gameplay = ctx.world.formation_globals.gameplay_version;
            let mut level = (gameplay - 0x3e8) / 100;
            if gameplay <= 0x709 {
                level = level.max(5);
            }
            if patch > level {
                return Ok(0);
            }
        }
        Ok(if ok { 2 } else { 0 })
    }

    fn pick_up_object(&mut self, i: usize, ctx: &mut MapActionContext) -> Result<u32, Unmodelled> {
        let subjects = self.subjects(i, ctx, 0x190);
        if subjects.is_empty() {
            return Ok(0);
        }
        let ids = self.combined(i, b"obje", TYPE_OBJECTS, 0x14);
        let objects = ctx.world.filter_objects();
        let mut targets: Vec<(i16, i16)> = Vec::new();
        for &id in &ids {
            let id = id as i16;
            if id != 0 && ctx.world.fade_with_identifier(id) {
                return Err(Unmodelled::new(
                    draw_site::PICK_FADE_RECORD_OBJECT,
                    format!("pick: 'obje' names {id}, a fade record's object"),
                ));
            }
            if let Some(o) = objects
                .iter()
                .find(|o| id != 0 && o.identifier == id && !o.no_shape)
            {
                targets.push((o.index as i16, id));
            }
        }
        if targets.is_empty() {
            return Ok(0);
        }
        let source = i32::from(self.actions[i].identifier as i16);
        for (k, &m) in subjects.iter().enumerate() {
            let (index, identifier) = targets[k % targets.len()];
            ctx.world
                .handle_pick_up_command(source, &[m], index, identifier)?;
        }
        Ok(0)
    }
}

#[derive(Debug, Clone, Copy)]
struct PlatoonTarget {
    point: (i32, i32),
    facing: u16,
}

impl MapActions {
    pub(crate) fn long(&self, i: usize, name: &[u8; 4], kind: i16) -> Option<i32> {
        self.indexed(i, name, kind, 0)
    }

    pub(crate) fn flag(&self, i: usize, name: &[u8; 4]) -> bool {
        self.find(i, 2, name, Some(0)).is_some()
    }

    fn waypoints(&self, i: usize) -> Option<Vec<(i32, i32)>> {
        self.find(i, 0, b"wayp", Some(TYPE_POINT))
            .map(|p| p.values.chunks(2).map(|c| (c[0], c[1])).collect())
    }

    fn platoon_movement(
        &mut self,
        i: usize,
        ctx: &mut MapActionContext,
    ) -> Result<u32, Unmodelled> {
        let Some(p) = self
            .indexed(i, b"#own", TYPE_ACTIONS, 0)
            .and_then(|id| self.index_of(id))
        else {
            return Ok(0);
        };
        let Some(squads) = self
            .find(p, 3, b"#squ", Some(TYPE_ACTIONS))
            .map(|q| q.values.clone())
        else {
            return Ok(0);
        };
        let mut ids = Vec::new();
        for s in squads {
            if let Some(a) = self.index_of(s) {
                let left = 0x190 - ids.len();
                ids.extend(self.combined(a, b"subj", TYPE_MONSTERS, left));
            }
        }
        let subjects: Vec<usize> = ids
            .into_iter()
            .filter_map(|id| ctx.world.monster_index_by_identifier(id as u16))
            .collect();
        let center = ctx.world.centerpoint(&subjects);
        let old = self.long(i, b"#sub", TYPE_INTEGER).unwrap_or(-1);
        let mut state = old;
        if let Some(r) = self.find_ref(p, 2, b"#att", Some(0)) {
            state = 3;
            self.actions[r.0].parameters.remove(r.1);
        }
        let mut target_group: Option<usize> = None;
        match &self.actions[i].tag.to_be_bytes() {
            b"plmo" => {
                state = match subjects.first() {
                    Some(&m) => {
                        let team = ctx.world.monsters[m].team;
                        match self.find(i, 0, b"radi", Some(TYPE_DISTANCE)) {
                            Some(r) if r.count == 2 => {
                                let (inner, outer) = (r.values[0], r.values[1]);
                                Self::platoon_engagement(ctx, center, team, inner, outer, state)?
                            }
                            _ => 0,
                        }
                    }
                    None => 0,
                };
            }
            b"plsc" => {
                state = match subjects.first() {
                    Some(&m) => {
                        let team = ctx.world.monsters[m].team;
                        match self.find(i, 0, b"radi", Some(TYPE_DISTANCE)) {
                            Some(r) if r.count == 2 => {
                                let outer = r.values[1];
                                let charge = self.flag(i, b"chrg");
                                let (s, g) = Self::platoon_charge(
                                    ctx, &subjects, center, team, outer, state, charge,
                                )?;
                                target_group = g;
                                s
                            }
                            _ => 0,
                        }
                    }
                    None => 0,
                };
            }
            _ => {}
        }
        match state {
            0 => self.platoon_walk(i, p, old, center, ctx)?,
            1 | 2 => {
                let Some(g) = target_group else {
                    return Err(Unmodelled::new(
                        draw_site::PLATOON_FALL_BACK_UNCHARGED,
                        "platoon fall-back without a charged group",
                    ));
                };
                let facing = if state == 2 {
                    let wpi = self.long(i, b"#wpi", TYPE_INTEGER);
                    match (self.waypoints(i), wpi) {
                        (Some(w), Some(k)) if k >= 0 && (k as usize) < w.len() => {
                            let q = w[k as usize];
                            arctangent(center.0 - q.0, center.1 - q.1)
                        }
                        _ => 0,
                    }
                } else {
                    match subjects.first() {
                        Some(&m) => {
                            let team = ctx.world.monsters[m].team;
                            let (mut sx, mut sy) = (0i32, 0i32);
                            for g in &ctx.world.spatial_groups {
                                if !ctx.world.teams_are_allied(team, g.team) {
                                    continue;
                                }
                                let dx = g.center.0.wrapping_sub(center.0);
                                let dy = g.center.1.wrapping_sub(center.1);
                                let d2 = dx.wrapping_mul(dx).wrapping_add(dy.wrapping_mul(dy));
                                if d2 == 0 || d2 > 0x9c3_ffff {
                                    continue;
                                }
                                let d = crate::projectiles::isqrt(d2 as u32);
                                sx = sx.wrapping_add((dx << 16) / d);
                                sy = sy.wrapping_add((dy << 16) / d);
                            }
                            arctangent(sx, sy)
                        }
                        None => 0,
                    }
                };
                let g = ctx.world.spatial_groups[g].clone();
                let reach = i32::from(g.radius) + i32::from(g.range);
                let a = i32::from(arctangent(
                    g.center.0.wrapping_sub(center.0),
                    g.center.1.wrapping_sub(center.1),
                ));
                let d = a - i32::from(facing);
                let turn = if ((d + 0x8000) as u32) <= 0x10000 {
                    let t = -d;
                    if t > 0x1000 { 0x1000 } else { t.max(-0x1000) }
                } else if d >= -0x7fff {
                    let t = d - 0x8000;
                    if t > 0x1000 { 0x1000 } else { t.max(-0x1000) }
                } else {
                    (d + 0x8000).max(-0x1000)
                };
                let angle = (turn + a) as u16;
                let t = trig_tables();
                let k = (angle.wrapping_add(0x20) >> 6) as usize;
                let lde = (
                    g.center.0 - ((i32::from(t.cosine[k]) * reach) >> 14),
                    g.center.1 - ((i32::from(t.sine[k]) * reach) >> 14),
                );
                let go = match self.indexed_point(i, b"#lde", 0) {
                    Some(old) => distance2d(old.0 - lde.0, old.1 - lde.1) > 0x100,
                    None => true,
                };
                if go {
                    self.platoon_orders(
                        p,
                        PlatoonTarget {
                            point: lde,
                            facing: angle,
                        },
                        ctx,
                    )?;
                    self.set_param(i, b"#lde", TYPE_POINT, vec![lde.0, lde.1]);
                }
            }
            3 => {
                if let Some(alli) = self.find(p, 0, b"alli", Some(TYPE_ACTIONS)) {
                    for id in alli.values.clone() {
                        if let Some(j) = self.index_of(id) {
                            self.set_param(j, b"#att", 0, vec![]);
                        }
                    }
                }
                let mut config = crate::map_attacks::AttackConfig::new();
                if let Some(r) = self.find(i, 0, b"radi", Some(TYPE_DISTANCE))
                    && r.count == 2
                {
                    config.flags &= !1;
                    config.radius = r.values[1];
                }
                config.point = center;
                crate::map_attacks::attack_with_monster_indexes(ctx.world, &subjects, config);
            }
            _ => {}
        }
        if old != state {
            self.set_param(i, b"#sub", TYPE_INTEGER, vec![state]);
        }
        Ok(0)
    }

    fn platoon_walk(
        &mut self,
        i: usize,
        p: usize,
        old: i32,
        center: (i32, i32),
        ctx: &mut MapActionContext,
    ) -> Result<(), Unmodelled> {
        let mut ntrp = self.flag(i, b"ntrp");
        if self.flag(i, b"cong") {
            match self.waypoints(i) {
                None => self.set_param(i, b"wayp", TYPE_POINT, vec![center.0, center.1]),
                Some(w) if w.len() <= 0x1e => {
                    let mut v = vec![center.0, center.1];
                    v.extend(w.iter().flat_map(|&(x, y)| [x, y]));
                    self.set_param(i, b"wayp", TYPE_POINT, v);
                }
                Some(_) => {}
            }
            while let Some(r) = self.find_ref(i, 3, b"cong", Some(0)) {
                self.actions[r.0].parameters.remove(r.1);
            }
        }
        let Some(wayp) = self.waypoints(i) else {
            return Ok(());
        };
        let n = wayp.len() as i32;
        let mut wpi;
        let (mut iwi, mut iwc) = (0i32, 0i32);
        let found = self.long(i, b"#wpi", TYPE_INTEGER);
        if n == 1 {
            match found {
                Some(v) => wpi = v,
                None => {
                    ntrp = false;
                    wpi = 0;
                }
            }
        } else {
            wpi = found.unwrap_or(i32::from(ntrp));
        }
        let mut check_squads = false;
        if ntrp && !(n == 1 && found.is_some()) {
            iwi = self.long(i, b"#iwi", TYPE_INTEGER).unwrap_or(0);
            iwc = self.long(i, b"#iwc", TYPE_INTEGER).unwrap_or(0);
            if old == 0 {
                check_squads = true;
            } else {
                iwi = 0;
                iwc = self.interpolation_steps(i, wpi, iwc)?;
            }
        } else if old == 0 {
            ntrp = false;
            check_squads = true;
        } else {
            wpi = -1;
            if n > 0 {
                let mut best = distance2d(center.0 - wayp[0].0, center.1 - wayp[0].1);
                wpi = 0;
                for (k, w) in wayp.iter().enumerate().skip(1) {
                    let d = distance2d(center.0 - w.0, center.1 - w.1);
                    if d < best {
                        best = d;
                        wpi = k as i32;
                    }
                }
            }
            ntrp = false;
        }
        if check_squads {
            let Some(squads) = self
                .find(p, 0, b"#squ", Some(TYPE_ACTIONS))
                .map(|q| q.values.clone())
            else {
                return Ok(());
            };
            for &s in squads.iter().take(0x1e) {
                if let Some(a) = self.index_of(s) {
                    self.test_squad_for_elimination(a, ctx)?;
                }
            }
            if !self.test_actions_for_success(&squads, true) {
                return Ok(());
            }
            self.reset_platoon(p, ctx)?;
            if ntrp {
                iwi += 1;
                if iwi >= iwc {
                    iwi = 0;
                    wpi += 1;
                    iwc = self.interpolation_steps(i, wpi, iwc)?;
                }
            } else {
                wpi += 1;
            }
        }
        let looped = self.flag(i, b"loop");
        let target = if ntrp {
            if looped && n < wpi {
                wpi = 1;
            }
            if wpi <= 0 || n < wpi {
                None
            } else {
                Some(self.interpolated_target(i, wpi, iwi, iwc, n, looped)?)
            }
        } else {
            if looped && n == wpi {
                wpi = 0;
            }
            if wpi < 0 || n <= wpi {
                None
            } else {
                let k = wpi as usize;
                let facing = if n <= 1 {
                    0
                } else if k != 0 {
                    arctangent(wayp[k].0 - wayp[k - 1].0, wayp[k].1 - wayp[k - 1].1)
                } else {
                    arctangent(wayp[1].0 - wayp[0].0, wayp[1].1 - wayp[0].1)
                };
                Some(PlatoonTarget {
                    point: wayp[k],
                    facing,
                })
            }
        };
        match target {
            Some(mut t) => {
                if let Some(r) = self.find(i, 0, b"wayr", Some(TYPE_DISTANCE))
                    && wpi >= 0
                    && wpi < i32::from(r.count)
                {
                    let radius = r.values[wpi as usize];
                    if radius != 0 {
                        let angle = ctx.rng.draw(draw_site::PLATOON_WALK_ANGLE);
                        let d = i32::from(ctx.rng.draw(draw_site::PLATOON_WALK_DISTANCE)) % radius;
                        t.point = polar(t.point, angle, d);
                    }
                }
                self.platoon_orders(p, t, ctx)?;
            }
            None => {
                self.succeed(i, ctx)?;
                self.set_platoon_state(p, None, ctx)?;
            }
        }
        WAYPOINT_SAVE_LOG.with(|l| {
            if let Some(l) = l.borrow_mut().as_mut() {
                l.push(WaypointSave {
                    action: i,
                    wpi,
                    interpolated: ntrp.then_some((iwi, iwc)),
                });
            }
        });
        self.set_param(i, b"#wpi", TYPE_INTEGER, vec![wpi]);
        if ntrp {
            self.set_param(i, b"#iwi", TYPE_INTEGER, vec![iwi]);
            self.set_param(i, b"#iwc", TYPE_INTEGER, vec![iwc]);
        }
        Ok(())
    }

    fn interpolation_steps(&self, i: usize, k: i32, count: i32) -> Result<i32, Unmodelled> {
        let Some(w) = self.waypoints(i) else {
            return Ok(count);
        };
        let (a, b) = self.interpolation_leg(i, &w, k)?;
        let d = distance2d(a.0.wrapping_sub(b.0), a.1.wrapping_sub(b.1));
        let steps = if d == 0 {
            1
        } else {
            match self.long(i, b"spac", TYPE_DISTANCE) {
                Some(0) => 0,
                Some(s) => d / s,
                None => d / 0x200,
            }
        };
        if k > w.len() as i32 {
            PAST_WAYPOINTS_LOG.with(|l| {
                if let Some(l) = l.borrow_mut().as_mut() {
                    l.push(PastWaypointsRead {
                        action: i,
                        k,
                        n: w.len() as i32,
                        a,
                        b,
                        steps,
                        block: self.actions[i]
                            .parameters
                            .iter()
                            .map(|p| {
                                let name = String::from_utf8_lossy(&p.name.0).into_owned();
                                format!("{name}/{}/{}", p.kind, p.count)
                            })
                            .collect(),
                    });
                }
            });
        }
        Ok(steps)
    }

    fn interpolation_leg(&self, i: usize, w: &[(i32, i32)], k: i32) -> Result<Leg, Unmodelled> {
        let n = w.len() as i32;
        if k < 1 {
            return Err(Unmodelled::new(
                draw_site::INTERPOLATION_BEFORE_WAYPOINTS,
                format!("interpolated waypoint {k} of {n} (reads before 'wayp')"),
            ));
        }
        let point = |j: i32| match w.get(j as usize) {
            Some(&p) => Ok(p),
            None => self.past_waypoints(i, j - n),
        };
        let a = point(k - 1)?;
        let b = if k == n {
            if self.flag(i, b"loop") { w[0] } else { a }
        } else {
            point(k)?
        };
        Ok((a, b))
    }

    fn past_waypoints(&self, i: usize, j: i32) -> Result<(i32, i32), Unmodelled> {
        let past = |why: &str| {
            Unmodelled::new(
                draw_site::INTERPOLATION_BEFORE_WAYPOINTS,
                format!(
                    "map action {} (#{i}): waypoint {j} past the end of 'wayp' reads {why}",
                    self.actions[i].identifier
                ),
            )
        };
        let (a, p) = self
            .iterate(i, 0, Some(tag(b"wayp")), Some(TYPE_POINT))
            .into_iter()
            .next()
            .ok_or_else(|| past("without 'wayp'"))?;
        let want = 8 * (j as usize + 1);
        let mut bytes: Vec<Option<u8>> = Vec::with_capacity(want + 16);
        for q in &self.actions[a].parameters[p + 1..] {
            if bytes.len() >= want {
                break;
            }
            let header = i32::from(q.kind as u16) | (i32::from(q.count) << 16);
            bytes.extend(header.to_le_bytes().map(Some));
            bytes.extend(tag(&q.name.0).to_le_bytes().map(Some));
            let width = mandrake_tag::map_action::PARAMETER_TYPE_WIDTHS[q.kind as usize];
            let scalar = width / MapActionParameter::scalars_per_element(q.kind);
            let start = bytes.len();
            for v in &q.values {
                bytes.extend(v.to_le_bytes()[..scalar].iter().map(|&b| Some(b)));
            }
            while !(bytes.len() - start).is_multiple_of(4) {
                bytes.push(None);
            }
        }
        let word = |at: usize| -> Result<i32, Unmodelled> {
            let w = bytes
                .get(at..at + 4)
                .ok_or_else(|| past("past the block"))?;
            let mut b = [0u8; 4];
            for (to, from) in b.iter_mut().zip(w) {
                *to = from.ok_or_else(|| past("a parameter's padding"))?;
            }
            Ok(i32::from_le_bytes(b))
        };
        Ok((word(want - 8)?, word(want - 4)?))
    }

    fn interpolated_point(
        &self,
        i: usize,
        k: i32,
        j: i32,
        count: i32,
    ) -> Result<(i32, i32), Unmodelled> {
        let t = if count != 0 { (j << 8) / count } else { 0 };
        let Some(w) = self.waypoints(i) else {
            return Err(Unmodelled::new(
                draw_site::INTERPOLATION_WITHOUT_WAYPOINTS,
                "interpolation without 'wayp'",
            ));
        };
        let (a, b) = self.interpolation_leg(i, &w, k)?;
        let t = i32::from(t as i16);
        Ok((
            a.0 + (((b.0 - a.0) * t) >> 8),
            a.1 + (((b.1 - a.1) * t) >> 8),
        ))
    }

    fn interpolated_target(
        &self,
        i: usize,
        wpi: i32,
        iwi: i32,
        iwc: i32,
        n: i32,
        looped: bool,
    ) -> Result<PlatoonTarget, Unmodelled> {
        let point = self.interpolated_point(i, wpi, iwi, iwc)?;
        let facing = if wpi == 1 && iwi == 0 {
            let next = if iwc > 1 {
                self.interpolated_point(i, 1, 1, iwc)?
            } else if n > 1 {
                self.interpolated_point(i, 2, 0, iwc)?
            } else if looped {
                self.interpolated_point(i, 1, 0, iwc)?
            } else {
                self.interpolated_point(i, n, 0, iwc)?
            };
            arctangent(next.0 - point.0, next.1 - point.1)
        } else {
            let (k, j) = if iwi - 1 < 0 {
                (wpi - 1, iwc - 1)
            } else {
                (wpi, iwi - 1)
            };
            let prev = self.interpolated_point(i, k, j, iwc)?;
            arctangent(point.0 - prev.0, point.1 - prev.1)
        };
        Ok(PlatoonTarget { point, facing })
    }

    fn test_squad_for_elimination(
        &mut self,
        s: usize,
        ctx: &mut MapActionContext,
    ) -> Result<(), Unmodelled> {
        if self.subjects(s, ctx, 0x7e).is_empty() {
            self.succeed(s, ctx)?;
        }
        Ok(())
    }

    fn test_actions_for_success(&self, ids: &[i32], all: bool) -> bool {
        for &id in ids {
            let Some(j) = self.index_of(id) else {
                continue;
            };
            let ok = self.actions[j].flags & FLAG_SUCCEEDED != 0;
            if ok && !all {
                return true;
            }
            if !ok && all {
                return false;
            }
        }
        all
    }

    fn set_platoon_state(
        &mut self,
        p: usize,
        state: Option<usize>,
        ctx: &mut MapActionContext,
    ) -> Result<(), Unmodelled> {
        if self.actions[p].tag != tag(b"plat") {
            return Ok(());
        }
        match state {
            None => self.set_param(p, b"#cst", TYPE_ACTIONS, vec![]),
            Some(t) => {
                self.set_param(t, b"#sub", TYPE_INTEGER, vec![-1]);
                let id = self.id_of(t);
                self.set_param(p, b"#cst", TYPE_ACTIONS, vec![id]);
                self.reset_platoon(p, ctx)?;
            }
        }
        Ok(())
    }

    fn platoon_orders(
        &mut self,
        p: usize,
        t: PlatoonTarget,
        ctx: &mut MapActionContext,
    ) -> Result<(), Unmodelled> {
        if self.actions[p].tag != tag(b"plat") {
            return Ok(());
        }
        let Some(pc) = self.indexed_point(p, b"cent", 0) else {
            return Ok(());
        };
        let Some(pf) = self.indexed(p, b"faci", 4, 0) else {
            return Ok(());
        };
        let Some(squads) = self
            .find(p, 3, b"#squ", Some(TYPE_ACTIONS))
            .map(|q| q.values.clone())
        else {
            return Ok(());
        };
        for s in squads {
            let Some(s) = self.index_of(s) else {
                continue;
            };
            if self.actions[s].tag != tag(b"squa") {
                self.succeed(s, ctx)?;
                continue;
            }
            let subjects = self.subjects(s, ctx, 0x7e);
            if subjects.is_empty() {
                self.succeed(s, ctx)?;
                continue;
            }
            let sf = self.indexed(s, b"faci", 4, 0).unwrap_or(0);
            let (Some(sc), Some(form)) = (
                self.indexed_point(s, b"cent", 0),
                self.indexed(s, b"form", TYPE_INTEGER, 0),
            ) else {
                self.succeed(s, ctx)?;
                continue;
            };
            let r = rotate_point2d(sc, pc, (pf as u16).wrapping_sub(t.facing));
            let dest = (r.0 - pc.0 + t.point.0, r.1 - pc.1 + t.point.1);
            let header = MovementHeader {
                flags: 0x18,
                formation: form as i16,
                facing: t.facing.wrapping_add(sf as u16),
            };
            let id = self.id_of(s);
            ctx.world.movement_command(&subjects, &[dest], header, id)?;
        }
        Ok(())
    }

    fn platoon_engagement(
        ctx: &mut MapActionContext,
        center: (i32, i32),
        team: i16,
        inner: i32,
        outer: i32,
        state: i32,
    ) -> Result<i32, Unmodelled> {
        if ctx.world.spatial_groups.is_empty() {
            return Ok(0);
        }
        let mut best = i32::MAX;
        let mut found = false;
        for g in &ctx.world.spatial_groups {
            if g.team == team || ctx.world.teams_are_allied(team, g.team) {
                continue;
            }
            let d = distance2d(center.0 - g.center.0, center.1 - g.center.1) - i32::from(g.radius);
            if !found || best > d {
                best = d;
                found = true;
            }
        }
        if outer <= best {
            return Ok(0);
        }
        let d = best - inner;
        if d < 0 {
            return Ok(3);
        }
        if d > 0x1fff {
            return Ok(0);
        }
        if state == 3 {
            return Ok(3);
        }
        let r = i32::from(ctx.rng.draw(draw_site::PLATOON_ENGAGEMENT_DISTANCE));
        if outer - inner == 0 {
            return Err(Unmodelled::new(
                draw_site::PLATOON_ENGAGEMENT_BAND_ZERO,
                "'radi' band 0 (division by zero)",
            ));
        }
        Ok(if 0x4000 - ((d << 14) / (outer - inner)) > r {
            3
        } else {
            0
        })
    }

    #[allow(clippy::too_many_arguments)]
    fn platoon_charge(
        ctx: &mut MapActionContext,
        subjects: &[usize],
        center: (i32, i32),
        team: i16,
        outer: i32,
        state: i32,
        charge: bool,
    ) -> Result<(i32, Option<usize>), Unmodelled> {
        let w = &ctx.world;
        if w.spatial_groups.is_empty() {
            return Ok((0, None));
        }
        let charging = state == 3 && charge;
        let mut best: Option<(usize, i32, i32, bool)> = None;
        for (k, g) in w.spatial_groups.iter().enumerate() {
            if g.team == team || w.teams_are_allied(team, g.team) {
                continue;
            }
            let d = distance2d(center.0 - g.center.0, center.1 - g.center.1) - i32::from(g.radius);
            let e = d - i32::from(g.range);
            if charging {
                if best.is_none_or(|b| e < b.2) {
                    best = Some((k, d, e, true));
                }
                continue;
            }
            let flag = w.monster_definitions[g.definition].flags & 0x80 != 0;
            if !flag && e >= 0 {
                continue;
            }
            if best.is_none_or(|b| b.2 > e) {
                best = Some((k, d, e, flag));
            }
        }
        let Some((k, d, _, flag)) = best else {
            return Ok((0, None));
        };
        if outer <= d {
            return Ok((0, None));
        }
        let g = &w.spatial_groups[k];
        let ours = subjects.iter().fold(0i32, |a, &m| {
            a.wrapping_add(i32::from(
                w.monster_definitions[w.monsters[m].definition].spatial_group_strength,
            ))
        });
        let a = arctangent(center.0 - g.center.0, center.1 - g.center.1);
        let mut theirs = i32::from(g.strength);
        if crate::engine_rng::angular_difference(g.facing, a) >= 0x4001 {
            theirs >>= 1;
        }
        let ratio = if theirs != 0 {
            (ours << 16) / theirs
        } else {
            i32::MAX
        };
        let fallback = if flag { 1 } else { 2 };
        let reach = i32::from(g.range >> 1) + 2 * i32::from(g.radius);
        let dist = distance2d(g.center.0 - center.0, g.center.1 - center.1);
        if dist >= reach && !charging && obstructing_group(w, center, k, team) {
            return Ok((fallback, Some(k)));
        }
        if state != 3 {
            let r = i32::from(ctx.rng.draw(draw_site::PLATOON_CHARGE_DISTANCE));
            if r >= ratio - 0xc000 {
                return Ok((fallback, Some(k)));
            }
        }
        if ratio > 0xc000 || flag {
            return Ok((3, Some(k)));
        }
        Ok((fallback, Some(k)))
    }
}

struct FadeCopy {
    sub_kind: u8,
    ticks: u8,
    countdown: u16,
    group: Option<FourCC>,
    monster: Option<usize>,
    hide_on_countdown: bool,
    zero_tpf: bool,
}

fn obstructing_group(w: &EngineWorld, p: (i32, i32), group: usize, team: i16) -> bool {
    let groups = &w.spatial_groups;
    let g = &groups[group];
    let (dx, dy) = (p.0.wrapping_sub(g.center.0), p.1.wrapping_sub(g.center.1));
    let a0 = arctangent(dx, dy);
    let d0 = dx.wrapping_mul(dx).wrapping_add(dy.wrapping_mul(dy));
    groups.iter().enumerate().any(|(k, h)| {
        if w.monster_definitions[h.definition].field_1b6 != 0
            || k == group
            || h.team == team
            || w.teams_are_allied(team, h.team)
            || h.team != g.team
        {
            return false;
        }
        let (ex, ey) = (p.0.wrapping_sub(h.center.0), p.1.wrapping_sub(h.center.1));
        let d = ex
            .wrapping_mul(ex)
            .wrapping_add(ey.wrapping_mul(ey))
            .wrapping_sub(i32::from(h.radius));
        d0 > d && crate::engine_rng::angular_difference(a0, arctangent(ex, ey)) <= 0x1fff
    })
}

#[cfg(test)]
mod meander_tests;

#[cfg(test)]
mod unit_control_tests;

#[cfg(test)]
mod clos_tests;

#[cfg(test)]
mod end_game_tests;

#[cfg(test)]
mod past_waypoints_tests;

impl EngineWorld {
    pub fn make_monster_visible(
        &mut self,
        identifier: i16,
        mode: i16,
        rng: &mut EngineRng,
        time: i32,
    ) -> Result<Option<usize>, Unmodelled> {
        let Some(creation) = self.marker_creation.clone() else {
            return Err(Unmodelled::new(
                crate::draw_site::SHOW_MONSTER_NO_MARKERS,
                format!("make_monster_visible({identifier}) without the load's markers"),
            ));
        };
        let Some(marker) = creation
            .mesh
            .markers
            .iter()
            .find(|m| m.marker_type == 3 && m.marker_id == identifier)
        else {
            return Ok(None);
        };
        let created = self
            .unit_markers
            .iter()
            .any(|u| u.identifier == identifier && u.created);
        let monster = if created {
            self.monster_index_by_identifier(identifier as u16)
        } else {
            let mut m = marker.clone();
            m.flags &= !1;
            self.new_monster_from_marker(&creation, &m, rng)?
        };
        if let Some(i) = monster {
            if created {
                let mon = &mut self.monsters[i];
                mon.flags &= !0x4000_0000;
                mon.object.no_shape = false;
            }
            if mode != -1 && marker.flags & 8 == 0 {
                match mode {
                    0 => self.visible_mode_0(i, marker, &creation, rng)?,
                    1..=3 => self.visible_fade(i, mode, rng)?,
                    _ => {}
                }
            }
            self.visible_group_flag(i, marker, &creation);
            self.shown_monsters.push(i);
        }
        self.log_visible(time, identifier, monster, rng);
        Ok(monster)
    }

    fn visible_group_flag(
        &mut self,
        i: usize,
        marker: &mandrake_tag::MarkerEntry,
        creation: &crate::engine_rng::MarkerCreation,
    ) {
        let def = &self.monster_definitions[self.monsters[i].definition];
        if self.formation_globals.gameplay_version > game_options::GAMEPLAY_1_5_1
            && marker.flags & 8 == 0
            && def.flags & 0x1000_0000 != 0
            && creation
                .tag(*b"mons", def.tag)
                .and_then(|d| crate::engine_rng::resolved_mons_group(&d, &self.catalog))
                .is_some()
        {
            self.monsters[i].flags_8 |= 0x40_0000;
        }
    }

    pub fn has_unit_marker(&self, id: i16) -> Result<bool, Unmodelled> {
        let Some(creation) = self.marker_creation.as_ref() else {
            return Err(Unmodelled::new(
                crate::draw_site::MARKER_WITHOUT_LOAD_MARKERS,
                format!("get_marker_by_identifier(3, {id}) without the load's markers"),
            ));
        };
        Ok(creation
            .mesh
            .markers
            .iter()
            .any(|m| m.marker_type == 3 && m.marker_id == id))
    }

    pub fn raise_unit_marker(&mut self, id: i16, dz: i32) -> (i32, i32, i32) {
        let creation = std::sync::Arc::make_mut(
            self.marker_creation
                .as_mut()
                .expect("raise_unit_marker after has_unit_marker"),
        );
        let m = creation
            .mesh
            .markers
            .iter_mut()
            .find(|m| m.marker_type == 3 && m.marker_id == id)
            .expect("raise_unit_marker after has_unit_marker");
        m.z = m.z.wrapping_add(dz);
        (m.x, m.y, m.z)
    }

    fn log_visible(&self, time: i32, identifier: i16, monster: Option<usize>, rng: &EngineRng) {
        VISIBLE_LOG.with(|l| {
            if let Some(log) = l.borrow_mut().as_mut() {
                let m = monster.map(|i| &self.monsters[i]);
                log.push(VisibleRun {
                    time,
                    identifier,
                    monster: monster.map_or(-1, |i| i as i32),
                    object: m.map_or(-1, |m| m.object.index),
                    flags: m.map_or(0, |m| m.flags),
                    flags_8: m.map_or(0, |m| m.flags_8),
                    rng: rng.rng.state,
                });
            }
        });
    }

    fn visible_mode_0(
        &mut self,
        i: usize,
        marker: &mandrake_tag::MarkerEntry,
        creation: &crate::engine_rng::MarkerCreation,
        rng: &mut EngineRng,
    ) -> Result<(), Unmodelled> {
        let def = &self.monster_definitions[self.monsters[i].definition];
        let group = creation
            .tag(*b"mons", def.tag)
            .and_then(|d| crate::engine_rng::resolved_mons_group(&d, &self.catalog));
        if let Some(group) = group {
            let m = &self.monsters[i];
            let source = crate::projectile_groups::EngineSpawn::monster_object(
                self, i, m.position, m.facing,
            );
            let desc = crate::projectile_groups::GroupDescriptor {
                flags: 0,
                group,
                shading_object: source.index,
                object: source.index,
                owner: i as i16,
                point: [marker.x, marker.y, marker.z],
                magnitude: 0,
                velocity: [0; 3],
            };
            let mut w = crate::projectile_groups::EngineSpawn {
                world: &mut *self,
                rng: &mut *rng,
                source: Some(source),
                host: None,
            };
            crate::projectile_groups::new_projectile_group(&mut w, &desc)?;
        }
        self.visible_hide(i);
        Ok(())
    }

    fn visible_hide(&mut self, i: usize) {
        let gameplay = self.formation_globals.gameplay_version;
        let (x, y) = self.monsters[i].position;
        let height =
            i32::from(self.mesh_height(x, y, mandrake_tag::MeshHeightMode::Physical) as i16);
        let mon = &mut self.monsters[i];
        mon.object.flags |= 0x10;
        mon.object.no_shape = true;
        mon.flags |= 0x4200_0000;
        if gameplay > game_options::GAMEPLAY_1_7 {
            mon.flags &= !0x19;
            let c = self.combat_mut(i);
            c.target = -1;
            c.attacker = -1;
            c.pending = -1;
            c.held_unit = -1;
        }
        self.monsters[i].object.z = height;
        if gameplay > game_options::GAMEPLAY_1_6 {
            self.monsters[i].object.z = self.monster_location_height(i, (x, y));
        }
    }

    fn push_fade_copy(&mut self, m: usize, index: i32, identifier: i16, copy: FadeCopy) {
        let record = self.new_fade_record_index();
        let mut object = self.monsters[m].object.clone();
        object.index = index;
        object.identifier = identifier;
        object.flags = self.duplicate_flags(m);
        object.sub_kind = copy.sub_kind;
        object.sub_count = 0;
        object.sub_limit = copy.ticks;
        object.kind = 5;
        object.owner = record as i16;
        if copy.hide_on_countdown && copy.countdown != 0 {
            object.no_shape = true;
        }
        if copy.zero_tpf {
            object.ticks_per_frame = 0;
        }
        let cell_link = self.copy_cell_link(self.monsters[m].cell_link);
        self.fade_records.push(crate::engine_rng::FadeRecord {
            index: record,
            host: crate::engine_rng::FadeRecord::NO_HOST,
            object,
            position: self.monsters[m].position,
            facing: self.monsters[m].facing,
            cell_link,
            countdown: copy.countdown,
            group: copy.group,
            monster: copy.monster,
        });
    }

    fn visible_fade(&mut self, i: usize, mode: i16, rng: &mut EngineRng) -> Result<(), Unmodelled> {
        let gameplay = self.formation_globals.gameplay_version;
        if gameplay <= 0x6b5 {
            return Err(Unmodelled::new(
                crate::draw_site::FADE_PARASITE_OLD_GAMEPLAY,
                format!("fade_object of gameplay {gameplay:#x}"),
            ));
        }
        let (group, ticks, site) = match mode {
            1 => (Some(*b"inti"), 30, crate::draw_site::VISIBLE_FADE_MODE_1),
            2 => (Some(*b"inte"), 60, crate::draw_site::VISIBLE_FADE_MODE_2),
            _ => (None, 30, crate::draw_site::VISIBLE_FADE),
        };
        let group = group
            .map(FourCC::new)
            .filter(|g| self.catalog.groups.contains_key(g));
        let (index, identifier) = self.new_object_datum();
        let r = i32::from(rng.draw(site));
        let countdown = ((r * 60) >> 16) as u16;
        self.push_fade_copy(
            i,
            index,
            identifier,
            FadeCopy {
                sub_kind: 2,
                ticks,
                countdown,
                group,
                monster: Some(i),
                hide_on_countdown: true,
                zero_tpf: false,
            },
        );
        self.visible_hide(i);
        Ok(())
    }

    fn invisible_fade(
        &mut self,
        m: usize,
        group: [u8; 4],
        ticks: u8,
        rng: &mut EngineRng,
    ) -> Result<(), Unmodelled> {
        let gameplay = self.formation_globals.gameplay_version;
        if gameplay <= 0x6b5 {
            return Err(Unmodelled::new(
                crate::draw_site::FADE_PARASITE_OLD_GAMEPLAY,
                format!("fade_object of gameplay {gameplay:#x}"),
            ));
        }
        let group = Some(FourCC::new(group)).filter(|g| self.catalog.groups.contains_key(g));
        let (index, identifier) = self.new_object_datum();
        if self.monsters[m].object.no_shape {
            self.free_objects.insert(index);
            return Ok(());
        }
        let r = i32::from(rng.draw(crate::draw_site::FADE_COPY_PAUSE));
        let countdown = ((r * 30) >> 16) as u16;
        self.push_fade_copy(
            m,
            index,
            identifier,
            FadeCopy {
                sub_kind: 1,
                ticks,
                countdown,
                group,
                monster: None,
                hide_on_countdown: false,
                zero_tpf: true,
            },
        );
        Ok(())
    }

    pub fn make_monster_invisible_with_mode(
        &mut self,
        identifier: i16,
        mode: i16,
        rng: &mut EngineRng,
        time: i32,
    ) -> Result<(), Unmodelled> {
        let Some(m) = self.monster_index_by_identifier(identifier as u16) else {
            return Ok(());
        };
        match mode {
            3 => {
                let gameplay = self.formation_globals.gameplay_version;
                if gameplay <= 0x6b5 {
                    return Err(Unmodelled::new(
                        crate::draw_site::FADE_PARASITE_OLD_GAMEPLAY,
                        format!("fade_object of gameplay {gameplay:#x}"),
                    ));
                }
                let (index, identifier) = self.new_object_datum();
                let r = i32::from(rng.draw(crate::draw_site::INVISIBLE_FADE_TICKS));
                let ticks = (30 + ((r * 30) >> 16)) as u8;
                if self.monsters[m].object.no_shape {
                    self.free_objects.insert(index);
                } else {
                    self.push_fade_copy(
                        m,
                        index,
                        identifier,
                        FadeCopy {
                            sub_kind: 1,
                            ticks,
                            countdown: 0,
                            group: None,
                            monster: None,
                            hide_on_countdown: false,
                            zero_tpf: false,
                        },
                    );
                }
            }
            0 => {
                let def = &self.monster_definitions[self.monsters[m].definition];
                let group = self
                    .marker_creation
                    .as_ref()
                    .and_then(|c| c.tag(*b"mons", def.tag))
                    .and_then(|d| crate::engine_rng::tag_ref(&d, 0x210));
                if let Some(group) = group {
                    let marker = self.marker_creation.as_ref().and_then(|c| {
                        c.mesh
                            .markers
                            .iter()
                            .find(|k| k.marker_type == 3 && k.marker_id == identifier)
                            .map(|k| [k.x, k.y, k.z])
                    });
                    let Some(point) = marker else {
                        return Err(Unmodelled::new(
                            crate::draw_site::HIDE_MONSTER_NO_MARKERS,
                            format!(
                                "make_monster_invisible({identifier}, 0) without the load's markers"
                            ),
                        ));
                    };
                    let object = self.monsters[m].object.index;
                    let desc = crate::projectile_groups::GroupDescriptor {
                        flags: 0,
                        group,
                        shading_object: object,
                        object,
                        owner: m as i16,
                        point,
                        magnitude: 0,
                        velocity: [0; 3],
                    };
                    let mut w = crate::projectile_groups::EngineSpawn {
                        world: self,
                        rng,
                        source: None,
                        host: None,
                    };
                    crate::projectile_groups::new_projectile_group(&mut w, &desc)?;
                }
            }
            1 => self.invisible_fade(m, *b"into", 30, rng)?,
            2 => self.invisible_fade(m, *b"inte", 60, rng)?,
            _ => {}
        }
        self.make_monster_invisible(m, rng, time)
    }
}

#[cfg(test)]
mod action_list_tests;
