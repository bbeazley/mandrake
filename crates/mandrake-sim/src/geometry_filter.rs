use mandrake_common::FourCC;
use mandrake_common::game_options;
use mandrake_tag::MapActionParameter;
use mandrake_tag::map_action::PARAMETER_TYPE_WIDTHS;

use crate::draw_site;
use crate::engine_rng::{EngineWorld, Unmodelled};
use crate::map_actions::{
    MapActionContext, MapActions, TYPE_ACTIONS, TYPE_INTEGER, TYPE_MONSTERS, TYPE_POINT,
};

pub(crate) const PARAMETER_POOL_SIZE: i32 = 0x18000;
const TYPE_TAG_NAME: i16 = 7;
const TYPE_OBJECTS: i16 = 15;
const TYPE_DISTANCE: i16 = 6;
const TYPE_FIXED: i16 = 8;

const RECORDS: usize = 0x190;

#[derive(Debug, Clone, Copy)]
struct Record {
    id: i16,
    point: Option<(i32, i32)>,
    inside: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UnitMarker {
    pub identifier: i16,
    pub difficulty: i16,
    pub point: (i32, i32),
    pub created: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FilterObject {
    pub index: i32,
    pub identifier: i16,
    pub kind: u8,
    pub projectile: Option<FourCC>,
    pub point: (i32, i32),
    pub z: i32,
    pub no_shape: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GeomRun {
    pub action: usize,
    pub counter: u16,
    pub objects: Vec<FilterObject>,
}

thread_local! {
    pub static GEOM_LOG: std::cell::RefCell<Option<Vec<GeomRun>>> =
        const { std::cell::RefCell::new(None) };
}

fn tag_value(t: FourCC) -> i32 {
    i32::from_be_bytes(t.0)
}

impl EngineWorld {
    pub(crate) fn iter_filter_objects(&self) -> impl Iterator<Item = FilterObject> + '_ {
        let monsters = self
            .monsters
            .iter()
            .filter(|m| m.identifier != 0)
            .map(|m| FilterObject {
                index: m.object.index,
                identifier: m.object.identifier,
                kind: m.object.kind,
                projectile: None,
                point: m.position,
                z: m.object.z,
                no_shape: m.object.no_shape,
            });
        let scenery = self
            .scenery
            .iter()
            .filter(|s| !s.deleted)
            .map(|s| FilterObject {
                index: s.object.index,
                identifier: s.object.identifier,
                kind: s.object.kind,
                projectile: s.projectile,
                point: s.position,
                z: s.object.z,
                no_shape: s.object.no_shape,
            });
        let projectiles = self.projectiles.iter().map(|p| FilterObject {
            index: p.object.index,
            identifier: p.object.identifier,
            kind: p.object.kind,
            projectile: Some(p.definition),
            point: p.position,
            z: p.object.z,
            no_shape: p.object.no_shape,
        });
        let records = self.projectile_records.iter().filter_map(|p| {
            let o = p.shape.as_ref()?;
            Some(FilterObject {
                index: o.index,
                identifier: o.identifier,
                kind: o.kind,
                projectile: Some(p.definition),
                point: (p.object.x, p.object.y),
                z: p.object.z,
                no_shape: o.no_shape,
            })
        });
        let dormant = self.dormant_objects.iter().map(|d| FilterObject {
            index: d.object.index,
            identifier: d.object.identifier,
            kind: d.object.kind,
            projectile: Some(d.definition),
            point: (d.position[0], d.position[1]),
            z: d.position[2],
            no_shape: d.object.no_shape,
        });
        monsters
            .chain(scenery)
            .chain(projectiles)
            .chain(records)
            .chain(dormant)
    }

    pub fn filter_objects(&self) -> Vec<FilterObject> {
        let mut v: Vec<FilterObject> = self.iter_filter_objects().collect();
        v.sort_by_key(|o| o.index);
        v
    }

    pub(crate) fn fade_with_identifier(&self, id: i16) -> bool {
        self.fade_records.iter().any(|r| r.object.identifier == id)
    }

    fn unit_marker(&self, id: i16) -> Option<UnitMarker> {
        self.unit_markers
            .iter()
            .find(|m| m.identifier == id)
            .copied()
    }

    fn marker_flag_8(&self, m: usize) -> bool {
        self.combat(m).marker_flags & 8 != 0
    }

    fn filter_team(&self, m: usize) -> i16 {
        let o = &self.monsters[m];
        if o.flags_8 & 0x1000 != 0 { -1 } else { o.team }
    }
}

fn add_record(records: &mut Vec<Record>, id: i16, point: (i32, i32)) {
    if records.iter().any(|r| r.id == id) || records.len() > RECORDS {
        return;
    }
    records.push(Record {
        id,
        point: Some(point),
        inside: false,
    });
}

fn set_slot(records: &mut Vec<Record>, n: usize, id: i16) {
    if n < records.len() {
        records[n].id = id;
        records[n].inside = true;
    } else {
        records.push(Record {
            id,
            point: None,
            inside: true,
        });
    }
}

struct SubjectFilters {
    hold_list: Option<Vec<i32>>,
    noho: bool,
    holi_list: Option<Vec<i32>>,
    nhoi: bool,
    vitality_below: Option<i32>,
    vitality_above: Option<i32>,
    taun: bool,
    subt: bool,
    atta: bool,
    moving: bool,
    running: bool,
    stan: bool,
    mana: (Option<i32>, Option<i32>),
    power: (Option<i32>, Option<i32>),
    ammunition: (Option<i32>, Option<i32>),
    experience: (Option<i32>, Option<i32>),
    charges: (Option<i32>, Option<i32>),
}

impl SubjectFilters {
    fn read(a: &MapActions, i: usize) -> Self {
        let name = |n: &[u8; 4]| a.indexed(i, n, TYPE_TAG_NAME, 0).is_some();
        let int = |n: &[u8; 4]| a.indexed(i, n, TYPE_INTEGER, 0);
        let list = |first: &[u8; 4], second: &[u8; 4]| {
            a.find(i, 0, first, Some(TYPE_TAG_NAME))
                .or_else(|| a.find(i, 0, second, Some(TYPE_TAG_NAME)))
                .map(|p| p.values.clone())
        };
        Self {
            hold_list: list(b"hold", b"noho"),
            noho: name(b"noho"),
            holi_list: list(b"holi", b"nhoi"),
            nhoi: name(b"nhoi"),
            vitality_below: a.indexed(i, b"hef<", TYPE_FIXED, 0),
            vitality_above: a.indexed(i, b"hef>", TYPE_FIXED, 0),
            taun: a.flag(i, b"taun"),
            subt: a.flag(i, b"subt"),
            atta: a.flag(i, b"atta"),
            moving: a.flag(i, b"move"),
            running: a.flag(i, b"_run"),
            stan: a.flag(i, b"stan"),
            mana: (int(b"man>"), int(b"man<")),
            power: (int(b"pow>"), int(b"pow<")),
            ammunition: (int(b"amm>"), int(b"amm<")),
            experience: (int(b"exp>"), int(b"exp<")),
            charges: (int(b"chr>"), int(b"chr<")),
        }
    }
}

pub(crate) fn parameter_bytes(p: &MapActionParameter) -> i32 {
    let width = PARAMETER_TYPE_WIDTHS
        .get(p.kind as usize)
        .copied()
        .unwrap_or(1) as i32;
    8 + ((i32::from(p.count.max(0)) * width + 3) & !3)
}

impl MapActions {
    pub(crate) fn parameter_pool_used(&self) -> i32 {
        self.actions
            .iter()
            .filter(|a| a.identifier != 0)
            .flat_map(|a| &a.parameters)
            .map(parameter_bytes)
            .sum()
    }

    pub(crate) fn action_bytes(&self, action: usize) -> i32 {
        self.actions[action]
            .parameters
            .iter()
            .map(parameter_bytes)
            .sum()
    }

    pub(crate) fn replace_parameter(&mut self, action: usize, name: u32, kind: i16) -> bool {
        self.actions[action]
            .parameters
            .retain(|p| !(u32::from_be_bytes(p.name.0) == name && p.kind == kind));
        let needed = self.action_bytes(action) + 8 + self.parameter_pool_used();
        if needed >= PARAMETER_POOL_SIZE {
            return false;
        }
        self.actions[action].parameters.push(MapActionParameter {
            kind,
            count: 0,
            name: FourCC::new(name.to_be_bytes()),
            values: Vec::new(),
        });
        true
    }

    pub(crate) fn append_parameter(&mut self, action: usize, element: &[i32]) {
        let Some(last) = self.actions[action].parameters.last() else {
            return;
        };
        let mut grown = last.clone();
        grown.count += 1;
        let grown_bytes = parameter_bytes(&grown) - parameter_bytes(last);
        if grown_bytes > 0
            && self.parameter_pool_used() + self.action_bytes(action) + grown_bytes
                >= PARAMETER_POOL_SIZE
        {
            return;
        }
        let p = self.actions[action].parameters.last_mut().expect("checked");
        p.count += 1;
        p.values.extend_from_slice(element);
    }

    #[allow(clippy::too_many_arguments)]
    fn write_selection(
        &mut self,
        rand: bool,
        target: usize,
        name: u32,
        kind: i16,
        records: &[Record],
        max: i16,
        want: bool,
        ctx: &mut MapActionContext,
    ) -> Result<i32, Unmodelled> {
        if !self.replace_parameter(target, name, kind) {
            return Ok(0);
        }
        let element = |r: &Record| -> Result<Vec<i32>, Unmodelled> {
            if kind == TYPE_POINT {
                let Some(p) = r.point else {
                    return Err(Unmodelled::new(
                        draw_site::GEOM_RECORD_POINT,
                        "geom: the point of a bmon / team / mons record (stack garbage)",
                    ));
                };
                Ok(vec![p.0, p.1])
            } else {
                Ok(vec![i32::from(r.id)])
            }
        };
        if !rand {
            let (mut left, mut written) = (max, 0);
            for r in records.iter().filter(|r| r.inside == want) {
                if left > 0 {
                    self.append_parameter(target, &element(r)?);
                    written += 1;
                }
                left = left.wrapping_sub(1);
            }
            return Ok(written);
        }
        let mut list: Vec<usize> = (0..records.len()).filter(|&k| records[k].inside).collect();
        let picks = i32::from(max).min(list.len() as i32);
        if picks <= 0 {
            return Ok(picks);
        }
        let mut left = list.len() as u32;
        for k in 0..picks as usize {
            let a = (u32::from(ctx.rng.draw(draw_site::GEOM_RANDOM_PICK)) * left) >> 16;
            list.swap(k, k + a as usize);
            left -= 1;
        }
        for &k in &list[..picks as usize] {
            self.append_parameter(target, &element(&records[k])?);
        }
        Ok(picks)
    }

    fn subject_passes(
        f: &SubjectFilters,
        m: usize,
        excluded: &[usize],
        ctx: &MapActionContext,
    ) -> Result<bool, Unmodelled> {
        let w = &*ctx.world;
        let o = &w.monsters[m];
        let c = w.combat(m);
        let def = &w.monster_definitions[o.definition];
        let carrying = |list: &[i32]| {
            w.carried_projectile(m)
                .is_some_and(|p| list.contains(&i32::from_be_bytes(p.0)))
        };
        if let Some(list) = &f.hold_list
            && carrying(list) == f.noho
        {
            return Ok(false);
        }
        if let Some(list) = &f.holi_list {
            let flags = def
                .ai
                .raw
                .get(..4)
                .map_or(0, |b| u32::from_be_bytes([b[0], b[1], b[2], b[3]]));
            let matched = if flags & 0x20_0000 != 0 && c.inventory.len > 0 {
                let mut matched = false;
                for &object in c.inventory.objects() {
                    let Some(p) = w
                        .projectiles
                        .iter()
                        .find(|p| p.object.index == i32::from(object))
                    else {
                        return Err(Unmodelled::new(
                            draw_site::GEOM_HOLDING_OBJECT_NOT_PROJECTILE,
                            format!(
                                "geometry filter 'holi': monster {}: inventory object {object} \
                                 isn't a projectile's at rest",
                                o.identifier
                            ),
                        ));
                    };
                    if p.object.kind == 4 && list.contains(&i32::from_be_bytes(p.definition.0)) {
                        matched = true;
                        break;
                    }
                }
                matched
            } else {
                carrying(list)
            };
            if matched == f.nhoi {
                return Ok(false);
            }
        }
        let fraction = || {
            let (cur, max) = (o.object.vitality as i16, o.object.max_vitality as i16);
            if max == 0 {
                0
            } else {
                (i32::from(cur) << 16) / i32::from(max)
            }
        };
        if let Some(v) = f.vitality_below
            && fraction() >= v
        {
            return Ok(false);
        }
        if let Some(v) = f.vitality_above
            && fraction() <= v
        {
            return Ok(false);
        }
        if f.taun && o.mode != 3 {
            return Ok(false);
        }
        if f.subt && excluded.contains(&m) {
            return Ok(false);
        }
        if f.atta && !(7..=9).contains(&o.mode) {
            return Ok(false);
        }
        if f.moving && !(4..=5).contains(&o.mode) {
            return Ok(false);
        }
        if f.running && o.anim_state != 0x13 {
            return Ok(false);
        }
        if f.stan && o.mode != 1 {
            return Ok(false);
        }
        let power = i32::from(def.spatial_group_strength);
        let ammunition = i32::from(c.ammunition as i8);
        let experience = i32::from(c.experience);
        let charges = 0;
        let mana = i32::from(c.mana);
        let mana_tests = if w.formation_globals.gameplay_version > 0x73a {
            f.mana
        } else {
            (None, None)
        };
        let tests = [
            (f.power.0.is_none_or(|v| power > v)),
            (f.power.1.is_none_or(|v| power < v)),
            (f.ammunition.0.is_none_or(|v| ammunition > v)),
            (f.ammunition.1.is_none_or(|v| ammunition < v)),
            (f.experience.0.is_none_or(|v| experience > v)),
            (f.experience.1.is_none_or(|v| experience < v)),
            (f.charges.0.is_none_or(|v| charges > v)),
            (f.charges.1.is_none_or(|v| charges < v)),
            (mana_tests.0.is_none_or(|v| mana > v)),
            (mana_tests.1.is_none_or(|v| mana < v)),
        ];
        Ok(tests.into_iter().all(|t| t))
    }

    fn subject_records(
        &self,
        i: usize,
        ids: &[i32],
        ctx: &MapActionContext,
    ) -> Result<Option<Vec<Record>>, Unmodelled> {
        let w = &*ctx.world;
        let indexes = |ids: &[i32]| -> Vec<usize> {
            ids.iter()
                .filter_map(|&id| w.monster_index_by_identifier(id as u16))
                .filter(|&m| w.monsters[m].flags & 0x4000_0000 == 0)
                .collect()
        };
        let mut subjects = indexes(ids);
        let excluded = if self.indexed(i, b"obje", TYPE_MONSTERS, 0).is_some() {
            indexes(&self.combined(i, b"obje", TYPE_MONSTERS, 0x190))
        } else {
            Vec::new()
        };
        let (mown, mtea, mtyp) = (
            self.flag(i, b"mown"),
            self.flag(i, b"mtea"),
            self.flag(i, b"mtyp"),
        );
        if mown || mtea || mtyp {
            let Some(&first) = subjects.first() else {
                return Ok(None);
            };
            let one = &w.monsters[first];
            let unit = w.combat(first).unit.0;
            let mut like = Vec::new();
            for (k, o) in w.monsters.iter().enumerate() {
                if o.identifier == 0 || w.marker_flag_8(k) {
                    continue;
                }
                let same_unit = w.combat(k).unit.0 == unit;
                let by_owner =
                    mown && o.owner == one.owner && (o.flags_8 ^ one.flags_8) & 0x1000 == 0;
                let by_team = mtea && w.filter_team(k) == w.filter_team(first);
                let keep = if by_owner || by_team {
                    !mtyp || same_unit
                } else {
                    mtyp && !mown && !mtea && same_unit
                };
                if keep {
                    like.push(k);
                }
            }
            subjects = like;
        }
        if let Some(team) = self.indexed(i, b"mont", TYPE_INTEGER, 0) {
            subjects.retain(|&m| {
                i32::from(w.filter_team(m)) == team
                    && w.unit_marker(w.monsters[m].identifier)
                        .is_none_or(|_| !w.marker_flag_8(m))
            });
        }
        let mut records = Vec::new();
        if subjects.is_empty() {
            return Ok(Some(records));
        }
        let filters = SubjectFilters::read(self, i);
        for &m in &subjects {
            if Self::subject_passes(&filters, m, &excluded, ctx)? {
                let o = &w.monsters[m];
                add_record(&mut records, o.identifier, o.position);
            }
        }
        Ok(Some(records))
    }

    pub(crate) fn geometry_filter(
        &mut self,
        i: usize,
        ctx: &mut MapActionContext,
    ) -> Result<u32, Unmodelled> {
        if ctx.world.formation_globals.gameplay_version <= game_options::GAMEPLAY_1_7_2 {
            return Err(Unmodelled::new(
                draw_site::GEOM_OLD_GAMEPLAY,
                "geom at gameplay <= game_options::GAMEPLAY_1_7_2 (the older branches are not ported)",
            ));
        }
        GEOM_LOG.with(|l| {
            if let Some(log) = l.borrow_mut().as_mut() {
                log.push(GeomRun {
                    action: i,
                    counter: ctx.world.object_counter(),
                    objects: ctx
                        .world
                        .filter_objects()
                        .into_iter()
                        .filter(|o| matches!(o.kind, 3 | 4))
                        .collect(),
                });
            }
        });
        let has_subj = self.indexed(i, b"subj", TYPE_MONSTERS, 0).is_some();
        let has_type = self.indexed(i, b"type", TYPE_TAG_NAME, 0).is_some();
        let has_objects = self.indexed(i, b"obje", TYPE_OBJECTS, 0).is_some();
        if ((has_type || has_objects) && has_subj) || (has_type && has_objects) {
            return Ok(0);
        }
        let mut records: Vec<Record> = Vec::new();
        if has_subj {
            let ids = self.combined(i, b"subj", TYPE_MONSTERS, 0x190);
            if self.flag(i, b"mark") {
                let uncr = self.flag(i, b"uncr");
                for &id in &ids {
                    let id = id as i16;
                    let Some(m) = ctx.world.unit_marker(id) else {
                        continue;
                    };
                    if uncr && (m.created || m.difficulty > ctx.world.difficulty) {
                        continue;
                    }
                    add_record(&mut records, id, m.point);
                }
            } else {
                match self.subject_records(i, &ids, ctx)? {
                    Some(r) => records = r,
                    None => return Ok(0),
                }
            }
        }
        if has_type {
            let tags = self
                .find(i, 0, b"type", Some(TYPE_TAG_NAME))
                .map(|p| p.values.clone())
                .unwrap_or_default();
            for o in ctx.world.filter_objects() {
                if o.no_shape || !matches!(o.kind, 3 | 4) {
                    continue;
                }
                let Some(p) = o.projectile else { continue };
                if !tags.contains(&tag_value(p)) {
                    continue;
                }
                add_record(&mut records, o.identifier, o.point);
                if records.len() > 0x18f {
                    break;
                }
            }
        }
        if has_objects {
            let ids = self
                .find(i, 0, b"obje", Some(TYPE_OBJECTS))
                .map(|p| p.values.clone())
                .unwrap_or_default();
            let objects = ctx.world.filter_objects();
            for &id in &ids {
                if records.len() > 0x18f {
                    break;
                }
                let id = id as i16;
                if id != 0 && ctx.world.fade_with_identifier(id) {
                    return Err(Unmodelled::new(
                        draw_site::GEOM_FADE_RECORD_OBJECT,
                        format!("geom: 'obje' names {id}, a fade record's object"),
                    ));
                }
                for o in &objects {
                    if id != 0 && o.identifier == id && !o.no_shape && records.len() <= 0x18f {
                        add_record(&mut records, id, o.point);
                    }
                }
            }
        }
        if records.is_empty()
            && self.indexed(i, b"team", TYPE_INTEGER, 0).is_none()
            && self.indexed(i, b"mons", TYPE_TAG_NAME, 0).is_none()
            && !self.flag(i, b"bmon")
        {
            return Ok(0);
        }
        let mut area = false;
        if let Some(p) = self.find(i, 0, b"poly", Some(TYPE_POINT)) {
            let v: Vec<(i32, i32)> = p
                .values
                .as_chunks::<2>()
                .0
                .iter()
                .map(|&[x, y]| (x, y))
                .collect();
            let closed = self.flag(i, b"clpo");
            for r in &mut records {
                let at = r.point.unwrap_or_default();
                r.inside = crate::map_actions::point_in_polygon(at, &v, closed);
            }
            area = true;
        }
        let centre = self.indexed_point(i, b"cent", 0).or_else(|| {
            let id = self.indexed(i, b"cemo", TYPE_MONSTERS, 0)?;
            let m = ctx.world.monster_index_by_identifier(id as u16)?;
            Some(ctx.world.monsters[m].position)
        });
        if let Some(c) = centre
            && let Some(radius) = self.indexed(i, b"radi", TYPE_DISTANCE, 0)
        {
            for r in &mut records {
                let at = r.point.unwrap_or_default();
                let dx = at.0.wrapping_sub(c.0).wrapping_abs();
                let dy = at.1.wrapping_sub(c.1).wrapping_abs();
                r.inside = dx < radius
                    && dy < radius
                    && dy.wrapping_mul(dy).wrapping_add(dx.wrapping_mul(dx))
                        < radius.wrapping_mul(radius);
            }
            area = true;
        }
        if self.flag(i, b"_map") {
            for r in &mut records {
                r.inside = true;
            }
            area = true;
        }
        let w = &*ctx.world;
        let alive = || {
            w.monsters
                .iter()
                .enumerate()
                .filter(|(_, m)| m.identifier != 0)
        };
        let mut scan = |picked: Vec<i16>| {
            for (n, &id) in picked.iter().enumerate() {
                set_slot(&mut records, n, id);
            }
            records.truncate(picked.len());
        };
        if self.flag(i, b"bmon") {
            scan(
                alive()
                    .filter(|(_, m)| m.flags_8 & 0x4000 != 0)
                    .map(|(_, m)| m.identifier)
                    .collect(),
            );
            area = true;
        }
        if let Some(team) = self.indexed(i, b"team", TYPE_INTEGER, 0) {
            scan(
                alive()
                    .filter(|&(k, _)| i32::from(w.filter_team(k)) == team && !w.marker_flag_8(k))
                    .map(|(_, m)| m.identifier)
                    .collect(),
            );
            area = true;
        }
        if let Some(unit) = self.indexed(i, b"mons", TYPE_TAG_NAME, 0) {
            scan(
                alive()
                    .filter(|&(k, m)| {
                        m.flags_8 & 0x1000 == 0
                            && !w.marker_flag_8(k)
                            && w.combat(k).unit.0 as i32 == unit
                    })
                    .map(|(_, m)| m.identifier)
                    .collect(),
            );
            area = true;
        }
        let n = records.len() as i32;
        let coun = self.indexed(i, b"coun", TYPE_INTEGER, 0);
        if let Some(c) = coun {
            for (j, r) in records.iter_mut().enumerate() {
                r.inside = c > j as i32;
            }
        }
        match self.indexed(i, b"_min", TYPE_INTEGER, 0) {
            Some(min) if min > n => return Ok(0),
            Some(_) => {}
            None if coun.is_none() && !area => return Ok(0),
            None => {}
        }
        let kind = if has_subj {
            TYPE_MONSTERS
        } else {
            TYPE_OBJECTS
        };
        let names: [(&[u8; 4], bool, i16); 4] = [
            (b"insi", true, kind),
            (b"outs", false, kind),
            (b"inlo", true, TYPE_POINT),
            (b"oulo", false, TYPE_POINT),
        ];
        let resu: Option<Vec<i32>> = self
            .find(i, 0, b"resu", Some(TYPE_ACTIONS))
            .map(|p| p.values.clone());
        let mut targets = 1usize;
        if let Some(r) = &resu {
            targets = r.len();
            for (name, _, _) in names {
                if let Some(q) = self.find(i, 0, name, Some(TYPE_TAG_NAME)) {
                    targets = targets.min(q.count.max(0) as usize);
                }
            }
        }
        let rand = self.flag(i, b"rand");
        let mut selected = false;
        for j in 0..targets.max(1) {
            let target = match &resu {
                Some(r) => self.index_of(r[j]),
                None => Some(i),
            };
            let Some(target) = target else { continue };
            let max = self.indexed(i, b"_max", TYPE_INTEGER, 0).unwrap_or(n) as i16;
            let mut any = false;
            for (name, want, element_kind) in names {
                let Some(field) = self.indexed(i, name, TYPE_TAG_NAME, j) else {
                    continue;
                };
                let written = self.write_selection(
                    rand,
                    target,
                    field as u32,
                    element_kind,
                    &records,
                    max,
                    want,
                    ctx,
                )?;
                any |= written as i16 != 0;
            }
            selected |= any;
        }
        Ok(if selected { 2 } else { 0 })
    }
}

pub fn unit_markers(mesh: &mandrake_tag::MeshTag) -> Vec<UnitMarker> {
    mesh.markers
        .iter()
        .filter(|m| m.marker_type == 3)
        .map(|m| UnitMarker {
            identifier: m.marker_id,
            difficulty: m.difficulty_flags,
            point: (m.x, m.y),
            created: false,
        })
        .collect()
}
