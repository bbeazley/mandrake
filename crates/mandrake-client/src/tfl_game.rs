use crate::args::Args;
use crate::object_history::{
    FRACTION_ONE, ObjectMotion, Stride, TFL_STRIDE_JUMP, shows_history_shape,
};
use crate::tfl_map::{TflMap, default_tfl_dir, resolve_tfl_film};
use crate::world_view::tfl::ObjectHistory;
use mandrake_common::FourCC;
use mandrake_tag::tfl::Game;
use mandrake_tfl_sim::{Stop, TflSim};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use tracing::info;

pub(crate) fn chosen_game(args: &Args) -> (Game, Option<PathBuf>) {
    let tfl_dir = args.tfl_data.clone().unwrap_or_else(default_tfl_dir);
    let game = args.game.unwrap_or_else(|| {
        let tfl_film = args
            .replay
            .as_deref()
            .is_some_and(|f| is_tfl_film(&resolve_tfl_film(f, &tfl_dir)));
        let tfl_folder = args
            .tfl_data
            .as_deref()
            .is_some_and(|d| Game::detect(d) == Some(Game::Tfl));
        if tfl_film || tfl_folder {
            Game::Tfl
        } else {
            Game::Myth2
        }
    });
    match game {
        Game::Tfl => (game, Some(tfl_dir)),
        Game::Myth2 => (game, None),
    }
}

fn is_tfl_film(path: &Path) -> bool {
    use std::io::Read;
    let mut head = [0u8; 64];
    std::fs::File::open(path)
        .and_then(|mut f| f.read_exact(&mut head))
        .is_ok_and(|()| mandrake_rec::tfl::TflFilmHeader::is_tfl_film(&head))
}

const SCEN_NET_GAME_ITEM: u32 = 0x8;
const OBJECT_NO_SEQUENCE: u32 = 0x200;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct UnitLook {
    pub(crate) core: FourCC,
    pub(crate) player: i16,
    pub(crate) permutation: usize,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct Placed {
    identifier: i16,
    hidden: bool,
    unit: Option<UnitLook>,
}

const PROJ_KEEPS_SOURCE_PERMUTATION: u32 = 1;

pub(crate) struct TflReplay {
    sim: Option<TflSim>,
    stop: Option<Stop>,
    placed: Vec<Placed>,
    shapes: Vec<(FourCC, usize, Option<UnitLook>)>,
    cells_across: (u32, u32),
    history: Option<ObjectHistory>,
    stains: Vec<mandrake_tfl_sim::stains::Stain>,
    stains_base: usize,
    marks: Vec<mandrake_tfl_sim::stains::Mark>,
    marks_base: usize,
    local_orders: Vec<mandrake_tfl_sim::procs::LocalProjectileGroupOrder>,
    local_orders_base: usize,
    local_group_starts: Vec<mandrake_tfl_sim::local_groups::LocalGroupStart>,
    local_group_starts_base: usize,
    sources_buf: Vec<Source>,
    strides: Vec<Stride>,
    observer: bool,
}

pub(crate) fn watched_as_observer(film: &mandrake_rec::tfl::TflFilmHeader) -> bool {
    film.mode == 1 && film.game_options & mandrake_common::game_options::COOPERATIVE == 0
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct FrameTime {
    pub(crate) before: u32,
    pub(crate) live: u32,
    pub(crate) fraction: i32,
}

impl FrameTime {
    pub(crate) fn whole(time: u32) -> Self {
        Self {
            before: time,
            live: time,
            fraction: FRACTION_ONE,
        }
    }

    pub(crate) fn is_whole(&self) -> bool {
        self.fraction >= FRACTION_ONE || self.before == self.live
    }

    pub(crate) fn shape_time(&self) -> u32 {
        if !self.is_whole() && shows_history_shape(self.fraction) {
            self.before
        } else {
            self.live
        }
    }

    pub(crate) fn interpolate(&self, at: impl Fn(u32) -> [i32; 3]) -> [i32; 3] {
        let live = at(self.live);
        if self.is_whole() {
            return live;
        }
        let motion = |world| ObjectMotion {
            world,
            ..ObjectMotion::default()
        };
        ObjectMotion::interpolated(&motion(at(self.before)), &motion(live), self.fraction).world
    }
}

impl TflReplay {
    pub(crate) fn new(game_dir: &Path, map: &TflMap) -> Option<Self> {
        Self::new_as(game_dir, map, 0)
    }

    pub(crate) fn new_as(game_dir: &Path, map: &TflMap, game_mode: i16) -> Option<Self> {
        let film = map.film_bytes.as_deref()?;
        let cells_across = (map.tfl_mesh.cell_width, map.tfl_mesh.cell_height);
        let observer = game_mode == 0 && map.film.as_ref().is_some_and(watched_as_observer);
        let replay = match TflSim::set_up_as(game_dir, film, game_mode).map_err(|b| b.1) {
            Ok(mut sim) => {
                let local_orders = sim.take_local_projectile_group_orders();
                let local_group_starts = sim.take_local_group_starts();
                info!(
                    "TFL sim: set up with {} objects, {} monsters",
                    sim.objects.len(),
                    sim.monsters.len()
                );
                let mut placed: Vec<Placed> = (0..sim.objects.len())
                    .map(|i| look_of(&sim, i, &[]))
                    .collect();
                let before: Vec<_> = placed.iter().map(|p| (p.identifier, p.unit)).collect();
                let made = sim.take_colour_sources();
                let exact = coloured_by_source(before.iter().map(|(i, u)| (*i, u)), &made);
                for p in &mut placed {
                    if let Some(unit) = exact.get(&p.identifier) {
                        p.unit = *unit;
                    }
                }
                let shapes = monster_shapes(&sim, map, &placed);
                Self {
                    sim: Some(sim),
                    stop: None,
                    placed,
                    shapes,
                    cells_across,
                    history: None,
                    stains: Vec::new(),
                    stains_base: 0,
                    marks: Vec::new(),
                    marks_base: 0,
                    local_orders,
                    local_orders_base: 0,
                    local_group_starts,
                    local_group_starts_base: 0,
                    sources_buf: Vec::new(),
                    strides: Vec::new(),
                    observer,
                }
            }
            Err(stop) => Self {
                sim: None,
                stop: Some(stop),
                placed: Vec::new(),
                shapes: Vec::new(),
                cells_across,
                history: None,
                stains: Vec::new(),
                stains_base: 0,
                marks: Vec::new(),
                marks_base: 0,
                local_orders: Vec::new(),
                local_orders_base: 0,
                local_group_starts: Vec::new(),
                local_group_starts_base: 0,
                sources_buf: Vec::new(),
                strides: Vec::new(),
                observer,
            },
        };
        if replay.stop.is_some() {
            replay.report_stop();
        }
        Some(replay)
    }

    pub(crate) fn tick(&self) -> u32 {
        self.sim.as_ref().map_or(0, |s| s.tick)
    }

    pub(crate) fn cells(&self) -> Option<&[mandrake_tag::tfl::TflCell]> {
        self.sim.as_ref().map(|s| s.cells.as_slice())
    }

    pub(crate) fn stop(&self) -> Option<&Stop> {
        self.stop.as_ref()
    }

    pub(crate) fn step(&mut self) -> bool {
        self.step_with(TflSim::step).is_some()
    }

    pub(crate) fn step_with<T>(
        &mut self,
        run: impl FnOnce(&mut TflSim) -> Result<T, Stop>,
    ) -> Option<T> {
        if self.stop.is_some() {
            return None;
        }
        self.fill_sources();
        let sim = self.sim.as_mut()?;
        let armed = sim.sniffer;
        let stepped = run(sim);
        if sim.sniffer != armed
            && let Some(s) = sim.sniffer
        {
            tracing::debug!(
                "TFL tutorial: tick {}: the sniffer armed: case {} (action {}, flags {:#x}, deadline {})",
                sim.tick,
                s.case,
                s.action,
                s.flags,
                s.deadline
            );
        }
        self.update_looks();
        let sim = self.sim.as_mut()?;
        self.stains.extend(sim.take_stains());
        self.marks.extend(sim.take_marks());
        self.local_orders
            .extend(sim.take_local_projectile_group_orders());
        self.local_group_starts
            .extend(sim.take_local_group_starts());
        let tick = sim.tick;
        self.strides.resize(sim.objects.len(), Stride::default());
        for (s, o) in self.strides.iter_mut().zip(&sim.objects) {
            *s = s.next(
                i64::from(o.identifier),
                [o.x, o.y, o.z],
                o.ticks_per_frame,
                tick,
                TFL_STRIDE_JUMP,
            );
        }
        match stepped {
            Ok(v) => Some(v),
            Err(stop) => {
                self.stop = Some(stop);
                self.report_stop();
                None
            }
        }
    }

    pub(crate) fn queue(&mut self, commands: mandrake_rec::TickCommands) -> bool {
        let Some(sim) = self.sim.as_mut() else {
            return false;
        };
        sim.data.film.tick_commands.push(commands);
        true
    }

    pub(crate) fn set_overhead_map_state(&mut self, state: i16) {
        if let Some(sim) = self.sim.as_mut() {
            sim.overhead_map_state = Some(state);
        }
    }

    pub(crate) fn sniff_action(
        &mut self,
        action: &mandrake_tfl_sim::sniffing::InputAction,
    ) -> bool {
        if self.stop.is_some() {
            return true;
        }
        let Some(sim) = self.sim.as_mut() else {
            return true;
        };
        let before = sim.sniffer;
        match sim.handle_action_sniffing(action) {
            Ok(sniffed) => {
                if sim.sniffer != before {
                    tracing::debug!(
                        "TFL tutorial: tick {}: the sniffer {sniffed:?} the poll: {before:?} -> {:?}",
                        sim.tick,
                        sim.sniffer
                    );
                }
                sniffed.survives()
            }
            Err(stop) => {
                self.stop = Some(stop);
                self.report_stop();
                false
            }
        }
    }

    pub(crate) fn sniff_command(&mut self, command: &mandrake_rec::CommandPacket) -> bool {
        if self.stop.is_some() {
            return true;
        }
        let Some(sim) = self.sim.as_mut() else {
            return true;
        };
        let before = sim.sniffer;
        let ty = command.command_type.to_u8();
        match sim.handle_command_sniffing(ty, &command.raw_payload) {
            Ok(sent) => {
                if sim.sniffer != before || !sent {
                    tracing::debug!(
                        "TFL tutorial: tick {}: command type {ty} {}: {before:?} -> {:?}",
                        sim.tick,
                        if sent { "sent" } else { "dropped" },
                        sim.sniffer
                    );
                }
                sent
            }
            Err(stop) => {
                self.stop = Some(stop);
                self.report_stop();
                false
            }
        }
    }

    pub(crate) fn run(&mut self, ticks: u32) -> u32 {
        (0..ticks).take_while(|_| self.step()).count() as u32
    }

    pub(crate) fn status(&self) -> String {
        match &self.stop {
            None => format!("tick {}", self.tick()),
            Some(s) => format!("stopped at tick {}: {s}", self.tick()),
        }
    }

    fn report_stop(&self) {
        if self.stop.is_some() {
            tracing::error!("TFL film {}", self.status());
        } else {
            println!("TFL film {}", self.status());
        }
    }

    pub(crate) fn stains_from(&self, index: usize) -> &[mandrake_tfl_sim::stains::Stain] {
        let off = index
            .saturating_sub(self.stains_base)
            .min(self.stains.len());
        &self.stains[off..]
    }

    pub(crate) fn marks_from(&self, index: usize) -> &[mandrake_tfl_sim::stains::Mark] {
        let off = index.saturating_sub(self.marks_base).min(self.marks.len());
        &self.marks[off..]
    }

    pub(crate) fn local_orders_from(
        &self,
        index: usize,
    ) -> &[mandrake_tfl_sim::procs::LocalProjectileGroupOrder] {
        let off = index
            .saturating_sub(self.local_orders_base)
            .min(self.local_orders.len());
        &self.local_orders[off..]
    }

    pub(crate) fn has_local_group_starts(&self) -> bool {
        self.local_group_starts_base > 0 || !self.local_group_starts.is_empty()
    }

    pub(crate) fn local_group_starts_from(
        &self,
        index: usize,
    ) -> &[mandrake_tfl_sim::local_groups::LocalGroupStart] {
        let off = index
            .saturating_sub(self.local_group_starts_base)
            .min(self.local_group_starts.len());
        &self.local_group_starts[off..]
    }

    pub(crate) fn trim_consumed(
        &mut self,
        stains: usize,
        marks: usize,
        orders: usize,
        starts: usize,
    ) {
        let drop_stains = stains
            .saturating_sub(self.stains_base)
            .min(self.stains.len());
        if drop_stains > 0 {
            self.stains.drain(..drop_stains);
            self.stains_base += drop_stains;
        }
        let drop_marks = marks.saturating_sub(self.marks_base).min(self.marks.len());
        if drop_marks > 0 {
            self.marks.drain(..drop_marks);
            self.marks_base += drop_marks;
        }
        let drop_orders = orders
            .saturating_sub(self.local_orders_base)
            .min(self.local_orders.len());
        if drop_orders > 0 {
            self.local_orders.drain(..drop_orders);
            self.local_orders_base += drop_orders;
        }
        let drop_starts = starts
            .saturating_sub(self.local_group_starts_base)
            .min(self.local_group_starts.len());
        if drop_starts > 0 {
            self.local_group_starts.drain(..drop_starts);
            self.local_group_starts_base += drop_starts;
        }
    }

    pub(crate) fn take_sound_events(&mut self) -> Vec<mandrake_tfl_sim::SoundEvent> {
        self.sim
            .as_mut()
            .map(TflSim::take_sound_events)
            .unwrap_or_default()
    }

    pub(crate) fn monsters_voices(
        &mut self,
        voices: &[(Vec<usize>, i8)],
    ) -> Vec<mandrake_tfl_sim::SoundEvent> {
        let Some(sim) = self.sim.as_mut() else {
            return Vec::new();
        };
        let list = crate::tfl_sound::mons_list(sim);
        voices
            .iter()
            .flat_map(|(monsters, kind)| {
                crate::tfl_sound::monsters_voice(sim, &list, monsters, *kind)
            })
            .collect()
    }

    pub(crate) fn observer(&self) -> bool {
        self.observer
    }

    pub(crate) fn sim(&self) -> Option<&TflSim> {
        self.sim.as_ref()
    }

    pub(crate) fn sim_mut(&mut self) -> Option<&mut TflSim> {
        self.sim.as_mut()
    }

    pub(crate) fn shapes(&self) -> &[(FourCC, usize, Option<UnitLook>)] {
        &self.shapes
    }

    fn fill_sources(&mut self) {
        self.sources_buf.clear();
        let Some(sim) = self.sim.as_ref() else {
            return;
        };
        self.sources_buf.extend(
            sim.objects
                .iter()
                .zip(&self.placed)
                .filter(|(o, p)| o.identifier != 0 && p.identifier == o.identifier)
                .map(|(o, p)| Source {
                    collection: o.collection,
                    world: (o.x, o.y),
                    placed: *p,
                }),
        );
    }

    fn update_looks(&mut self) {
        let made = self
            .sim
            .as_mut()
            .map_or_else(Vec::new, TflSim::take_colour_sources);
        self.update_looks_for_made(&made);
    }

    fn update_looks_for_made(&mut self, made: &[mandrake_tfl_sim::ColourSource]) {
        let Some(sim) = self.sim.as_ref() else {
            return;
        };
        self.placed.resize(sim.objects.len(), Placed::default());
        let has_new = sim
            .objects
            .iter()
            .zip(&self.placed)
            .any(|(o, p)| o.identifier != 0 && o.identifier != p.identifier);
        if !has_new {
            return;
        }
        let exact = if made.is_empty() {
            HashMap::new()
        } else {
            let before = self
                .sources_buf
                .iter()
                .map(|s| (s.placed.identifier, &s.placed.unit));
            coloured_by_source(before, made)
        };
        for (index, (o, p)) in sim.objects.iter().zip(&mut self.placed).enumerate() {
            if o.identifier != 0 && o.identifier != p.identifier {
                *p = match exact.get(&o.identifier) {
                    Some(&unit) => Placed {
                        identifier: o.identifier,
                        hidden: false,
                        unit,
                    },
                    None => look_of(sim, index, &self.sources_buf),
                };
            }
        }
    }

    pub(crate) fn drawable(
        &self,
    ) -> impl Iterator<
        Item = (
            usize,
            &mandrake_tfl_sim::objects::TflObject,
            Option<UnitLook>,
        ),
    > {
        let objects = self.sim.as_ref().map_or(&[][..], |s| s.objects.as_slice());
        objects
            .iter()
            .zip(&self.placed)
            .enumerate()
            .filter(|(_, (o, p))| {
                o.identifier != 0
                    && p.identifier == o.identifier
                    && !p.hidden
                    && o.flags & OBJECT_NO_SEQUENCE == 0
                    && o.sequence >= 0
            })
            .map(|(index, (o, p))| (index, o, p.unit))
    }

    pub(crate) fn cells_across(&self) -> (u32, u32) {
        self.cells_across
    }

    pub(crate) fn record_history(&mut self, before: ObjectHistory) {
        self.history = (!before.is_empty()).then_some(before);
    }

    pub(crate) fn stride(&self, index: usize) -> Option<&Stride> {
        let o = self.sim.as_ref()?.objects.get(index)?;
        self.strides
            .get(index)
            .filter(|s| s.identifier == i64::from(o.identifier) && o.identifier != 0)
    }

    pub(crate) fn history(&self) -> Option<&ObjectHistory> {
        self.history.as_ref()
    }

    pub(crate) fn time_at(&self, fraction: i32) -> FrameTime {
        match &self.history {
            Some(h) if fraction < FRACTION_ONE => FrameTime {
                before: h.tick,
                live: self.tick(),
                fraction,
            },
            _ => FrameTime::whole(self.tick()),
        }
    }
}

fn coloured_by_source<'a>(
    before: impl IntoIterator<Item = (i16, &'a Option<UnitLook>)>,
    made: &[mandrake_tfl_sim::ColourSource],
) -> HashMap<i16, Option<UnitLook>> {
    let mut known: HashMap<i16, Option<UnitLook>> = HashMap::new();
    for (identifier, unit) in before {
        if identifier != 0 {
            known.entry(identifier).or_insert(*unit);
        }
    }
    let mut exact = HashMap::new();
    for c in made {
        let source = exact
            .get(&c.source_identifier)
            .or_else(|| known.get(&c.source_identifier))
            .copied();
        match source {
            Some(unit) => {
                exact.insert(c.identifier, unit);
            }
            None => {
                exact.remove(&c.identifier);
            }
        }
    }
    exact
}

#[derive(Clone, Copy, Debug)]
struct Source {
    collection: FourCC,
    world: (i32, i32),
    placed: Placed,
}

fn look_of(sim: &TflSim, index: usize, sources: &[Source]) -> Placed {
    let o = &sim.objects[index];
    let identifier = o.identifier;
    let def = |group: &[u8; 4], id: FourCC| sim.data.peek_def(group, id);
    let permutation = usize::try_from(o.permutation_index).unwrap_or(0);
    if let Some(s) = sources
        .iter()
        .find(|s| s.placed.identifier == identifier && s.collection == o.collection)
    {
        return Placed {
            identifier,
            ..s.placed
        };
    }
    if let Some(j) = sim
        .monsters
        .iter()
        .filter(|m| m.identifier != 0 && m.object != index)
        .map(|m| m.object)
        .find(|&j| {
            sim.objects
                .get(j)
                .is_some_and(|s| s.identifier == identifier && s.collection == o.collection)
        })
    {
        return Placed {
            identifier,
            ..look_of(sim, j, &[])
        };
    }
    if let Some(m) = sim
        .monsters
        .iter()
        .find(|m| m.identifier != 0 && m.object == index)
    {
        let core = def(b"unit", m.unit).and_then(|u| u.tag(0x04));
        return Placed {
            identifier,
            hidden: false,
            unit: core.map(|core| UnitLook {
                core,
                player: i16::from(m.player),
                permutation,
            }),
        };
    }
    if let Some(scen) = o.scenery.and_then(|id| def(b"scen", id)) {
        return Placed {
            identifier,
            hidden: scen.u32(0) & SCEN_NET_GAME_ITEM != 0,
            unit: scen.tag(0x08).map(|core| UnitLook {
                core,
                player: -1,
                permutation,
            }),
        };
    }
    let keeps = sim
        .projectiles
        .list
        .iter()
        .find(|p| p.identifier != 0 && p.object == index)
        .and_then(|p| def(b"proj", p.definition?))
        .is_some_and(|d| d.u32(0) & PROJ_KEEPS_SOURCE_PERMUTATION != 0);
    let distance = |s: &Source| {
        let (dx, dy) = (i64::from(s.world.0 - o.x), i64::from(s.world.1 - o.y));
        dx * dx + dy * dy
    };
    let unit = keeps
        .then(|| {
            sources
                .iter()
                .filter(|s| s.collection == o.collection)
                .filter_map(|s| Some((s, s.placed.unit?)))
                .min_by_key(|(s, u)| (u.permutation != permutation, distance(s)))
        })
        .flatten()
        .map(|(_, u)| UnitLook { permutation, ..u });
    Placed {
        identifier,
        hidden: false,
        unit,
    }
}

fn be_i16(d: &[u8], o: usize) -> Option<i16> {
    Some(i16::from_be_bytes(d.get(o..o + 2)?.try_into().ok()?))
}

fn monster_shapes(
    sim: &TflSim,
    map: &TflMap,
    placed: &[Placed],
) -> Vec<(FourCC, usize, Option<UnitLook>)> {
    let mut out = Vec::new();
    for m in &sim.monsters {
        let (Some(o), Some(mons)) = (sim.objects.get(m.object), map.tag(b"mons", m.definition))
        else {
            continue;
        };
        let look = placed.get(m.object).and_then(|p| p.unit);
        for at in [0x08, 0x1c, 0x1e] {
            if let Some(seq) = be_i16(&mons, at).and_then(|s| usize::try_from(s).ok()) {
                let shape = (o.collection, seq, look);
                if !out.contains(&shape) {
                    out.push(shape);
                }
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_frame_time_is_between_its_two_ticks() {
        let at = |t: u32| [t as i32 * 100, -(t as i32) * 40, 7];
        let time = |fraction| FrameTime {
            before: 10,
            live: 11,
            fraction,
        };
        assert_eq!(time(0).interpolate(at), at(10));
        assert_eq!(time(0x4000).interpolate(at), [1025, -410, 7]);
        assert_eq!(time(0x8000).interpolate(at), [1050, -420, 7]);
        assert_eq!(time(0xc000).interpolate(at), [1075, -430, 7]);
        assert_eq!(time(FRACTION_ONE).interpolate(at), at(11));
        assert_eq!(FrameTime::whole(11).interpolate(at), at(11));
        assert_eq!((time(0).shape_time(), time(0x7fff).shape_time()), (10, 10));
        assert_eq!(
            (time(0x8000).shape_time(), time(FRACTION_ONE).shape_time()),
            (11, 11)
        );
        let same = FrameTime {
            before: 11,
            live: 11,
            fraction: 0,
        };
        assert!(same.is_whole() && time(FRACTION_ONE).is_whole() && !time(0xffff).is_whole());
    }

    fn look(core: &[u8; 4], player: i16, permutation: usize) -> Option<UnitLook> {
        Some(UnitLook {
            core: FourCC::new(*core),
            player,
            permutation,
        })
    }

    fn made(
        object: usize,
        identifier: i16,
        source: usize,
        source_identifier: i16,
    ) -> mandrake_tfl_sim::ColourSource {
        mandrake_tfl_sim::ColourSource {
            object,
            identifier,
            source,
            source_identifier,
        }
    }

    #[test]
    fn a_part_takes_its_sources_look() {
        let ghoul = look(b"ghoa", 1, 1);
        let other = look(b"ghoa", 2, 0);
        let before = [(100, &ghoul), (101, &other), (102, &None)];
        let exact = coloured_by_source(
            before,
            &[
                made(9, -5, 7, 100),
                made(7, -6, 9, -5),
                made(11, -7, 3, 102),
                made(12, -8, 4, 555),
            ],
        );
        assert_eq!(exact.get(&-5), Some(&ghoul));
        assert_eq!(exact.get(&-6), Some(&ghoul));
        assert_eq!(exact.get(&-7), Some(&None));
        assert_eq!(exact.get(&-8), None);
    }
}
