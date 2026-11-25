use mandrake_common::FourCC;

use crate::Stop;
use crate::draw_site;

pub const PARAMETER_WIDTHS: [usize; 19] =
    [1, 1, 2, 2, 2, 4, 4, 4, 4, 4, 4, 4, 8, 16, 2, 2, 2, 12, 2];
pub const TYPE_ACTIONS: i16 = 3;

pub const FLAG_INITIALLY_ACTIVE: u32 = 0x1;
pub const FLAG_NO_ACTIVATION_DELAY: u32 = 0x4;
pub const FLAG_NO_RESCHEDULE_DELAY: u32 = 0x8;
pub const FLAG_ACTIVE: u32 = 0x10;
pub const FLAG_SUCCEEDED: u32 = 0x20;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Parameter {
    pub kind: i16,
    pub count: i16,
    pub name: FourCC,
    pub data: Vec<u8>,
}

impl Parameter {
    pub fn i16_at(&self, i: usize) -> i16 {
        self.data
            .get(2 * i..2 * i + 2)
            .map_or(0, |b| i16::from_be_bytes([b[0], b[1]]))
    }
    pub fn i32_at(&self, i: usize) -> i32 {
        self.data
            .get(4 * i..4 * i + 4)
            .map_or(0, |b| i32::from_be_bytes([b[0], b[1], b[2], b[3]]))
    }
    pub fn shorts(&self) -> Vec<i16> {
        (0..self.count.max(0) as usize)
            .map(|i| self.i16_at(i))
            .collect()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MapAction {
    pub identifier: i16,
    pub kind: i16,
    pub tag: u32,
    pub flags: u32,
    pub delay: i32,
    pub delay_range: i32,
    pub time: i32,
    pub parameters: Vec<Parameter>,
}

impl MapAction {
    pub fn tag_name(&self) -> String {
        String::from_utf8_lossy(&self.tag.to_be_bytes()).into_owned()
    }

    pub fn memory(&self) -> (Vec<Option<u8>>, Vec<usize>) {
        let mut out = Vec::new();
        let mut offsets = Vec::with_capacity(self.parameters.len());
        for p in &self.parameters {
            out.extend(p.kind.to_le_bytes().map(Some));
            out.extend(p.count.to_le_bytes().map(Some));
            out.extend(u32::from_be_bytes(p.name.0).to_le_bytes().map(Some));
            offsets.push(out.len());
            let w = PARAMETER_WIDTHS
                .get(p.kind.max(0) as usize)
                .copied()
                .unwrap_or(1);
            let swap = match w {
                1 => 1,
                2 => 2,
                _ => 4,
            };
            for c in p.data.chunks(swap) {
                out.extend(c.iter().rev().copied().map(Some));
            }
            out.resize(out.len().next_multiple_of(4), None);
        }
        (out, offsets)
    }
}

fn be16(d: &[u8], o: usize) -> Option<i16> {
    Some(i16::from_be_bytes(d.get(o..o + 2)?.try_into().ok()?))
}
fn be32(d: &[u8], o: usize) -> Option<u32> {
    Some(u32::from_be_bytes(d.get(o..o + 4)?.try_into().ok()?))
}

pub fn parse(mesh: &[u8]) -> Result<Vec<MapAction>, Stop> {
    let bad = |m: String| Stop::new(draw_site::MAP_ACTIONS_PARSE, format!("map actions: {m}"));
    let short = || bad("truncated".into());
    let count = be32(mesh, 0x80).ok_or_else(short)? as usize;
    let offset = be32(mesh, 0x84).ok_or_else(short)? as usize;
    let size = be32(mesh, 0x88).ok_or_else(short)? as usize;
    if count == 0 {
        return Ok(Vec::new());
    }
    let block = mesh
        .get(1024 + offset..1024 + offset + size)
        .ok_or_else(short)?;
    let buffer = block.get(count * 0x40..).ok_or_else(short)?;
    let mut out = Vec::with_capacity(count);
    for i in 0..count {
        let r = &block[i * 0x40..(i + 1) * 0x40];
        let pcount = be16(r, 0x14).ok_or_else(short)?;
        let mut o = be32(r, 0x18).ok_or_else(short)? as usize;
        let mut parameters = Vec::new();
        for _ in 0..pcount.max(0) {
            let kind = be16(buffer, o).ok_or_else(short)?;
            let n = be16(buffer, o + 2).ok_or_else(short)?;
            let name = FourCC(
                buffer
                    .get(o + 4..o + 8)
                    .ok_or_else(short)?
                    .try_into()
                    .unwrap(),
            );
            let w = *PARAMETER_WIDTHS
                .get(kind.max(0) as usize)
                .ok_or_else(|| bad(format!("action {i}: parameter type {kind}")))?;
            let bytes = w * n.max(0) as usize;
            let data = buffer.get(o + 8..o + 8 + bytes).ok_or_else(short)?.to_vec();
            parameters.push(Parameter {
                kind,
                count: n,
                name,
                data,
            });
            o += 8 + bytes.div_ceil(4) * 4;
        }
        out.push(MapAction {
            identifier: be16(r, 0).ok_or_else(short)?,
            kind: be16(r, 2).ok_or_else(short)?,
            tag: be32(r, 4).ok_or_else(short)?,
            flags: be32(r, 8).ok_or_else(short)?,
            delay: be32(r, 0x0c).ok_or_else(short)? as i32,
            delay_range: be32(r, 0x10).ok_or_else(short)? as i32,
            time: be32(r, 0x3c).ok_or_else(short)? as i32,
            parameters,
        });
    }
    Ok(out)
}

const LINK: u32 = u32::from_be_bytes(*b"link");

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Found {
    pub action: usize,
    pub param: usize,
}

#[derive(Debug, Clone)]
pub struct Walk {
    pub flags: u32,
    pub name: Option<u32>,
    pub kind: i16,
    list: Vec<(usize, usize)>,
    current: usize,
}

impl Walk {
    pub fn new(action: usize, flags: u32, name: Option<&[u8; 4]>, kind: i16) -> Self {
        Self {
            flags,
            name: name.map(|n| u32::from_be_bytes(*n)),
            kind,
            list: vec![(action, 0)],
            current: 0,
        }
    }

    pub fn next(&mut self, actions: &[MapAction]) -> Option<Found> {
        while self.current < self.list.len() {
            let (a, _) = self.list[self.current];
            while self.list[self.current].1 < actions[a].parameters.len() {
                let pi = self.list[self.current].1;
                self.list[self.current].1 += 1;
                let p = &actions[a].parameters[pi];
                let name = u32::from_be_bytes(p.name.0);
                if self.flags & 1 == 0 && p.kind == TYPE_ACTIONS && name == LINK {
                    for id in p.shorts() {
                        let Some(j) = index_of(actions, id) else {
                            continue;
                        };
                        if self.list[..self.current].iter().any(|&(k, _)| k == j) {
                            continue;
                        }
                        if self.list.len() < 0x20 {
                            self.list.push((j, 0));
                        }
                    }
                    continue;
                }
                if (self.kind == -1 || p.kind == self.kind)
                    && self.name.is_none_or(|n| n == name)
                    && (p.count > 0 || self.flags & 2 != 0)
                    && (p.count != 1 || self.flags & 4 == 0)
                {
                    return Some(Found {
                        action: a,
                        param: pi,
                    });
                }
            }
            self.current += 1;
        }
        None
    }
}

pub fn index_of(actions: &[MapAction], id: i16) -> Option<usize> {
    actions
        .iter()
        .position(|a| a.identifier != 0 && a.identifier == id)
}

#[inline(always)]
pub(crate) fn not_read_by_tfl(_names: &[&[u8; 4]]) {}

pub fn find<'a>(
    actions: &'a [MapAction],
    action: usize,
    flags: u32,
    name: &[u8; 4],
    kind: i16,
) -> Option<&'a Parameter> {
    Walk::new(action, flags, Some(name), kind)
        .next(actions)
        .map(|f| &actions[f.action].parameters[f.param])
}

pub fn combined(
    actions: &[MapAction],
    action: usize,
    name: &[u8; 4],
    kind: i16,
    max: usize,
) -> Vec<i16> {
    let mut w = Walk::new(action, 0, Some(name), kind);
    let mut out = Vec::new();
    while let Some(f) = w.next(actions) {
        let p = &actions[f.action].parameters[f.param];
        let take = (p.count.max(0) as usize).min(max - out.len());
        out.extend((0..take).map(|i| p.i16_at(i)));
    }
    out
}

pub fn combined_points(
    actions: &[MapAction],
    action: usize,
    name: &[u8; 4],
    max: usize,
) -> Vec<(i32, i32)> {
    let mut w = Walk::new(action, 0, Some(name), 0xc);
    let mut out = Vec::new();
    while let Some(f) = w.next(actions) {
        let p = &actions[f.action].parameters[f.param];
        let take = (p.count.max(0) as usize).min(max - out.len());
        out.extend((0..take).map(|k| (p.i32_at(2 * k), p.i32_at(2 * k + 1))));
    }
    out
}

#[derive(Debug, Clone, Default)]
pub struct MapActions {
    pub actions: Vec<MapAction>,
    pub initialized: bool,
    pub triggering: Vec<usize>,
    pub game: GameState,
    pub pool: Option<crate::objects::DatumPool>,
}

#[derive(Debug, Clone, Default)]
pub struct GameState {
    pub sync_sample: Option<(u32, u32)>,
    pub end_reason: i16,
    pub ended_at: Option<u32>,
    pub end_suspended: bool,
    pub overtime: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MungerField {
    Insert,
    Replace,
    Delete,
    Append,
    Deactivate,
    Activate,
    DeleteAction,
    Succeed,
}

impl MungerField {
    fn from_name(name: u32) -> Option<Self> {
        Some(match &name.to_be_bytes() {
            b"pins" => Self::Insert,
            b"prep" => Self::Replace,
            b"pdel" => Self::Delete,
            b"papp" => Self::Append,
            b"adea" => Self::Deactivate,
            b"aact" => Self::Activate,
            b"adel" => Self::DeleteAction,
            b"asuc" => Self::Succeed,
            _ => return None,
        })
    }
}

#[derive(Debug, Clone, Copy)]
struct Record {
    name: u32,
    kind: i16,
    count: i16,
    at: Found,
}

fn record(actions: &[MapAction], f: Found) -> Record {
    let p = &actions[f.action].parameters[f.param];
    Record {
        name: u32::from_be_bytes(p.name.0),
        kind: p.kind,
        count: p.count,
        at: f,
    }
}

impl MapActions {
    fn param(&self, r: &Record) -> Option<&Parameter> {
        self.actions.get(r.at.action)?.parameters.get(r.at.param)
    }

    fn insert_copy(&mut self, action: usize, arg: &Record) {
        let (count, data) = match self.param(arg) {
            Some(p) => {
                let w = PARAMETER_WIDTHS[p.kind.max(0) as usize];
                let n = arg.count.max(0) as usize;
                let mut d = p.data.clone();
                d.resize(w * n, 0);
                (arg.count, d)
            }
            None => return,
        };
        self.actions[action].parameters.push(Parameter {
            kind: arg.kind,
            count,
            name: FourCC(arg.name.to_be_bytes()),
            data,
        });
    }

    fn delete_all(&mut self, action: usize, arg: &Record) {
        loop {
            let mut w = Walk::new(action, 3, Some(&arg.name.to_be_bytes()), arg.kind);
            let Some(f) = w.next(&self.actions) else {
                return;
            };
            self.actions[f.action].parameters.remove(f.param);
        }
    }

    fn append(&mut self, action: usize, arg: &Record) {
        let Some(src) = self.param(arg).cloned() else {
            return;
        };
        let mut w = Walk::new(action, 3, Some(&arg.name.to_be_bytes()), arg.kind);
        let f = match w.next(&self.actions) {
            Some(f) => f,
            None => {
                self.actions[action].parameters.push(Parameter {
                    kind: arg.kind,
                    count: 0,
                    name: FourCC(arg.name.to_be_bytes()),
                    data: Vec::new(),
                });
                Found {
                    action,
                    param: self.actions[action].parameters.len() - 1,
                }
            }
        };
        let p = &mut self.actions[f.action].parameters[f.param];
        let w = PARAMETER_WIDTHS[p.kind.max(0) as usize];
        let n = arg.count.max(0) as usize;
        let mut add = src.data;
        add.resize(w * n, 0);
        p.count = p.count.wrapping_add(arg.count);
        p.data.extend_from_slice(&add);
    }
}

impl crate::TflSim {
    pub(crate) fn munger(&mut self, i: usize) -> Result<u32, Stop> {
        let mut walk = Walk::new(i, 1, None, -1);
        let mut rec = match walk.next(&self.map_actions.actions) {
            Some(f) => record(&self.map_actions.actions, f),
            None => return Ok(0),
        };
        loop {
            if rec.kind == TYPE_ACTIONS {
                let list = rec;
                let mut k = 0;
                while k < rec.count.max(0) as usize {
                    let id = self.map_actions.param(&list).map_or(0, |p| p.i16_at(k));
                    if let Some(j) = index_of(&self.map_actions.actions, id) {
                        match MungerField::from_name(rec.name) {
                            None => {}
                            Some(
                                f @ (MungerField::Insert
                                | MungerField::Replace
                                | MungerField::Delete
                                | MungerField::Append),
                            ) => {
                                if let Some(n) = walk.next(&self.map_actions.actions) {
                                    rec = record(&self.map_actions.actions, n);
                                    let m = &mut self.map_actions;
                                    match f {
                                        MungerField::Insert => m.insert_copy(j, &rec),
                                        MungerField::Replace => {
                                            m.delete_all(j, &rec);
                                            m.insert_copy(j, &rec);
                                        }
                                        MungerField::Delete => m.delete_all(j, &rec),
                                        _ => m.append(j, &rec),
                                    }
                                }
                            }
                            Some(MungerField::Deactivate) => self.deactivate(j)?,
                            Some(MungerField::Activate) => self.activate(j)?,
                            Some(MungerField::DeleteAction) => self.delete_map_action(j),
                            Some(MungerField::Succeed) => self.succeed(j)?,
                        }
                    }
                    k += 1;
                }
            }
            match walk.next(&self.map_actions.actions) {
                Some(f) => rec = record(&self.map_actions.actions, f),
                None => return Ok(0),
            }
        }
    }

    pub(crate) fn action_list(&mut self, i: usize) -> Result<u32, Stop> {
        not_read_by_tfl(&[b"remo"]);
        let a = &self.map_actions.actions;
        let acti = combined(a, i, b"acti", 3, 0x20);
        let only = find(a, i, 2, b"only", 0).is_some();
        let rand = find(a, i, 2, b"rand", 0).is_some();
        let chances = |a: &[MapAction], name: &[u8; 4], n: usize| {
            find(a, i, 0, name, 8)
                .filter(|p| p.count as usize == n)
                .map(|p| (0..n).map(|k| p.i32_at(k)).collect::<Vec<i32>>())
        };
        let mut changed = false;
        let n = acti.len();
        if n > 0 {
            let acch = chances(a, b"acch", n);
            if rand || only {
                let start = if rand {
                    i32::from(self.rng.draw(draw_site::ACTION_LIST_START)) % n as i32
                } else {
                    0
                } as usize;
                let mut k = start;
                loop {
                    if let Some(j) = index_of(&self.map_actions.actions, acti[k]) {
                        let f = self.map_actions.actions[j].flags;
                        if f & 2 == 0 || f & 0x40 == 0 {
                            match &acch {
                                Some(c) => {
                                    let Some(&chance) = c.get(j) else {
                                        return Err(Stop::new(
                                            draw_site::ACTION_LIST_CHANCE_PAST_END,
                                            format!(
                                                "map action {i}: acli reads acch[{j}] past its {n}"
                                            ),
                                        ));
                                    };
                                    if self.chance_draw(draw_site::ACTION_LIST_CHANCE, chance) {
                                        self.activate(j)?;
                                        changed = true;
                                    }
                                }
                                None => {
                                    self.activate(j)?;
                                    changed = true;
                                }
                            }
                        }
                    }
                    k = (k + 1) % n;
                    if changed || k == start {
                        break;
                    }
                }
            } else {
                match acch {
                    Some(c) => {
                        for (k, &chance) in c.iter().enumerate() {
                            if self.chance_draw(draw_site::ACTION_LIST_EACH_CHANCE, chance) {
                                self.each_action(&acti[k..k + 1], false)?;
                                changed = true;
                            }
                        }
                    }
                    None => {
                        self.each_action(&acti, false)?;
                        changed = true;
                    }
                }
            }
        }
        let a = &self.map_actions.actions;
        let deac = combined(a, i, b"deac", 3, 0x20);
        if !deac.is_empty() {
            match chances(a, b"dech", deac.len()) {
                Some(c) => {
                    for (k, &chance) in c.iter().enumerate() {
                        if self.chance_draw(draw_site::ACTION_LIST_SEQUENCE_CHANCE, chance) {
                            self.each_action(&deac[k..k + 1], true)?;
                            changed = true;
                        }
                    }
                }
                None => {
                    self.each_action(&deac, true)?;
                    changed = true;
                }
            }
        }
        Ok(if changed { 2 } else { 0 })
    }

    fn chance_draw(&mut self, site: u32, chance: i32) -> bool {
        let r = ((i32::from(self.rng.draw(site)) << 8) >> 16) as i16;
        r < (chance >> 8) as i16
    }

    fn each_action(&mut self, ids: &[i16], deactivate: bool) -> Result<(), Stop> {
        for &id in ids.iter().take(30) {
            if let Some(j) = index_of(&self.map_actions.actions, id) {
                if deactivate {
                    self.deactivate(j)?;
                } else {
                    self.activate(j)?;
                }
            }
        }
        Ok(())
    }

    pub(crate) fn control(&mut self, i: usize) -> Result<u32, Stop> {
        let t = self.tick;
        let stop = |what: &str| {
            Err(Stop::new(
                draw_site::CONTROL_NOT_PORTED,
                format!("tick {t}: map action {i}: ctrl '{what}' is not ported"),
            ))
        };
        let a = &self.map_actions.actions;
        let subjects = combined(a, i, b"subj", 2, 300);
        let moso = find(a, i, 2, b"moso", 0).is_some();
        let byte = |name: &[u8; 4]| {
            find(a, i, 0, name, 0)
                .filter(|p| p.count > 0)
                .map(|p| p.data.first().copied().unwrap_or(0))
        };
        let (mut clear, mut set, mut clear_44, mut set_44) = (0u32, 0u32, 0u32, 0u32);
        if let Some(v) = byte(b"user") {
            clear = 0x800_0000;
            if v != 0 {
                set = 0x800_0000;
            }
        }
        if let Some(v) = byte(b"dnat") {
            clear_44 = 0x10;
            if v != 0 {
                set_44 = 0x10;
            }
        }
        if let Some(v) = byte(b"cnba") {
            clear_44 |= 0x20;
            if v != 0 {
                set_44 |= 0x20;
            }
        }
        if let Some(v) = byte(b"igno") {
            clear |= 0x1000_0000;
            if v != 0 {
                set |= 0x1000_0000;
            } else {
                clear_44 |= 4;
            }
        }
        if let Some(p) = find(a, i, 0, b"heig", 6) {
            let (count, lo, hi) = (p.count, p.i32_at(0), p.i32_at(1));
            for &id in &subjects {
                let Some(k) = self
                    .data
                    .mesh
                    .markers
                    .iter()
                    .position(|m| m.marker_type == crate::setup::UNIT && m.marker_id == id)
                else {
                    continue;
                };
                let range = hi.wrapping_sub(lo);
                let d = if count > 1 && range != 0 {
                    let r = i32::from(self.rng.draw(draw_site::CONTROL_PICK));
                    ((r % range) as i16).wrapping_add(lo as i16)
                } else {
                    lo as i16
                };
                let w = &mut self.marker_motion[k][3];
                *w = w.wrapping_add(d);
            }
        }
        let mode = if find(a, i, 2, b"tain", 0).is_some() {
            2
        } else if find(a, i, 2, b"blam", 0).is_some() {
            1
        } else if find(a, i, 2, b"moen", 0).is_some() {
            0
        } else {
            -1
        };
        let visi = find(a, i, 2, b"visi", 0).is_some() || moso;
        let invi = find(a, i, 2, b"invi", 0).is_some();
        if visi {
            let rein = find(a, i, 2, b"rein", 0).is_some();
            let local = self.local_team()?;
            for &id in &subjects {
                let shown = self.make_monster_visible(id, mode)?;
                if rein
                    && let Some(m) = shown
                    && Some(self.monsters[m].team as i8 as i16) == local
                {
                    self.game_sound(draw_site::CONTROL_SOUND, 0x14);
                }
            }
        }
        if invi {
            for &id in &subjects {
                self.make_monster_invisible(id, mode)?;
            }
        }
        let monsters: Vec<usize> = subjects
            .iter()
            .filter_map(|&id| {
                self.monsters
                    .iter()
                    .position(|m| m.identifier != 0 && m.identifier == id)
            })
            .collect();
        let a = &self.map_actions.actions;
        let objects: Vec<i16> = find(a, i, 0, b"proj", 0xe).map_or(Vec::new(), |p| {
            (0..(p.count.max(0) as usize).min(monsters.len()))
                .map(|k| p.i16_at(k))
                .collect()
        });
        for (k, &id) in objects.iter().enumerate() {
            if let Some(o) = self
                .objects
                .iter()
                .position(|ob| ob.identifier != 0 && ob.identifier == id)
            {
                self.give_object_to_monster(monsters[k], o)?;
            }
        }
        for &m in &monsters {
            let mo = &mut self.monsters[m];
            mo.flags = (mo.flags & !clear) | set;
            mo.flags_44 = ((u32::from(mo.flags_44) & !clear_44) | set_44) as u8;
        }
        let a = &self.map_actions.actions;
        let long = |name: &[u8; 4], kind: i16, k: usize| {
            find(a, i, 0, name, kind)
                .filter(|p| (k as i32) < i32::from(p.count))
                .map(|p| p.i32_at(k))
        };
        let hefr = long(b"hefr", 8, 0).map(|from| {
            let range = long(b"hefr", 8, 1).map_or(0, |to| to.wrapping_sub(from));
            (from >> 8, range >> 8)
        });
        let ammo = long(b"ammo", 5, 0);
        let nena = long(b"nena", 5, 0);
        let expe = long(b"expe", 5, 0);
        let lesu = long(b"lesu", 5, 0);
        let majk = find(a, i, 0, b"majk", 0).map(|p| p.data.first().copied().unwrap_or(0));
        let fade = find(a, i, 2, b"fade", 0).is_some();
        let dele = find(a, i, 2, b"dele", 0).is_some();
        if let Some((from, range)) = hefr
            && !monsters.is_empty()
        {
            for &m in &monsters {
                let extra = if range > 0 {
                    (i32::from(self.rng.draw(draw_site::CONTROL_RANGE)) * range) >> 16
                } else {
                    0
                };
                let mo = &mut self.monsters[m];
                mo.vitality = (extra
                    .wrapping_add(from)
                    .wrapping_mul(i32::from(mo.max_vitality))
                    >> 8) as i16;
            }
        }
        for &m in &monsters {
            let mo = &mut self.monsters[m];
            if let Some(v) = ammo {
                mo.ammunition = v as i8;
            }
        }
        for &m in &monsters {
            if let Some(v) = nena {
                let mo = &mut self.monsters[m];
                mo.name_index = v as u8;
                mo.field_13 |= 1;
            }
        }
        for &m in &monsters {
            if let Some(v) = expe {
                self.monsters[m].experience = v as i8;
            }
        }
        for &m in &monsters {
            if let Some(v) = lesu {
                self.monsters[m].field_77 = v as u8;
            }
        }
        if let Some(on) = majk {
            for &m in &monsters {
                self.set_magical_invisibility(m, on != 0)?;
            }
        }
        for (flag, what) in [(fade, "fade"), (dele, "dele")] {
            if !flag {
                continue;
            }
            for &m in &monsters {
                if self.monsters[m].identifier == 0 {
                    return stop(&format!("{what} of a monster already removed"));
                }
                if what == "fade" {
                    self.fade_monster(m)?;
                } else {
                    self.remove_monster(m)?;
                }
            }
        }
        Ok(2)
    }

    pub(crate) fn make_monster_visible(
        &mut self,
        identifier: i16,
        mode: i16,
    ) -> Result<Option<usize>, Stop> {
        let shown = self.make_monster_visible_new(identifier)?;
        if let Some((m, _)) = shown {
            if mode == 0 {
                self.appear_in_group(identifier, m)?;
            } else if mode > 0 {
                self.appear_in_copy(m, mode)?;
            }
        }
        Ok(shown.map(|(m, _)| m))
    }

    fn appear_in_group(&mut self, identifier: i16, m: usize) -> Result<(), Stop> {
        let o = self.monsters[m].object;
        let def = self.monsters[m].definition;
        if let Some(group) = self.data.def(b"mons", def)?.tag(0x1ac) {
            let r = self.monster_group_request(identifier, o, group);
            self.new_projectile_group(&r)?;
        }
        self.hide_on_ground(m)
    }

    fn monster_group_request(
        &self,
        identifier: i16,
        o: usize,
        group: FourCC,
    ) -> crate::projectiles::GroupRequest {
        let position = self
            .data
            .mesh
            .markers
            .iter()
            .find(|k| k.marker_type == crate::setup::UNIT && k.marker_id == identifier)
            .map_or([0; 3], |k| [k.x, k.y, k.z]);
        crate::projectiles::GroupRequest {
            flags: 0,
            group: Some(group),
            source_object: o as i16,
            object: o,
            position,
            speed: 0,
            velocity: [0; 3],
        }
    }

    pub(crate) fn appear_in_copy(&mut self, m: usize, mode: i16) -> Result<(), Stop> {
        let o = self.monsters[m].object;
        let Some(copy) = self.copy_object(o) else {
            return Ok(());
        };
        let (site, limit, group) = if mode == 1 {
            (draw_site::APPEAR_IN_COPY_MODE_1, 0x1e, 9)
        } else {
            (draw_site::APPEAR_IN_COPY, 0x3c, 11)
        };
        let delay = ((u32::from(self.rng.draw(site)) * 0x3c) >> 16) as i16;
        let Some(k) = self.new_carried_record(false, copy, delay, limit) else {
            return self.delete_object(copy);
        };
        self.projectiles.list[k].owner = m as i16;
        self.projectiles.list[k].delay_group = self.global_group(group);
        self.hide_on_ground(m)
    }

    fn hide_on_ground(&mut self, m: usize) -> Result<(), Stop> {
        let o = self.monsters[m].object;
        self.monsters[m].flags |= 0x4000_0000;
        let (x, y) = (self.objects[o].x, self.objects[o].y);
        let ground = self.mesh_height(x, y)? as i16;
        let ob = &mut self.objects[o];
        ob.flags |= 0x210;
        ob.z = i32::from(ground);
        ob.above_ground = 0;
        Ok(())
    }

    pub(crate) fn make_monster_invisible(
        &mut self,
        identifier: i16,
        mode: i16,
    ) -> Result<(), Stop> {
        let Some(k) = self
            .data
            .mesh
            .markers
            .iter()
            .position(|m| m.marker_type == 3 && m.marker_id == identifier)
        else {
            return Ok(());
        };
        let d = &self.data.mesh.markers[k].type_data;
        let existing = i16::from_be_bytes([d[0x1c], d[0x1d]]);
        if existing == -1 {
            return Ok(());
        }
        self.data.mesh.markers[k].flags |= 1;
        let s = existing as usize;
        if !self
            .monsters
            .get(s)
            .is_some_and(|m| m.identifier == identifier)
        {
            return Ok(());
        }
        if mode == 0 {
            let o = self.monsters[s].object;
            let def = self.monsters[s].definition;
            if let Some(group) = self.data.def(b"mons", def)?.tag(0x1b8) {
                let r = self.monster_group_request(identifier, o, group);
                self.new_projectile_group(&r)?;
            }
        }
        if mode == 1 || mode == 2 {
            let o = self.monsters[s].object;
            if let Some(copy) = self.copy_object(o) {
                let (limit, group, site) = if mode == 1 {
                    (0x1e, 10, draw_site::INVISIBLE_COPY_MODE_1)
                } else {
                    (0x3c, 12, draw_site::INVISIBLE_COPY)
                };
                if let Some(k) = self.new_carried_record(true, copy, 0, limit) {
                    self.objects[copy].ticks_per_frame = 0;
                    self.projectiles.list[k].delay_group = self.global_group(group);
                    let r = u32::from(self.rng.draw(site));
                    self.projectiles.list[k].delay = ((r * 0x1e) >> 16) as i16;
                }
            }
        }
        let m = &mut self.monsters[s];
        m.flags = (m.flags & 0xffff_fff6) | 0x4000_0000;
        let o = m.object;
        self.objects[o].flags |= 0x200;
        self.monster_removed(s)
    }

    pub(crate) fn new_object_state(&mut self, o: usize) -> Result<(), Stop> {
        let owner = self
            .monsters
            .iter()
            .position(|m| m.identifier != 0 && m.object == o);
        let (x, y) = (self.objects[o].x, self.objects[o].y);
        let ground = self.mesh_height(x, y)? as i16;
        let (physics, mask) = match owner {
            Some(k) => {
                let mons = self.data.def(b"mons", self.monsters[k].definition)?;
                (mons.tag(0x90), mons.closed_terrain())
            }
            None => (self.objects[o].physics, self.objects[o].terrain_mask),
        };
        let obj = &mut self.objects[o];
        let z = i32::from(ground) + i32::from(obj.above_ground);
        obj.z = z;
        obj.above_ground = (z as i16).wrapping_sub(ground);
        obj.physics = physics;
        obj.terrain_mask = mask;
        let cell = self.cell_at(x >> 10, y >> 10);
        let mut bits = 0;
        if z < 0 && self.cells[cell].flags & 0x18 != 0 && self.media_at(x, y)? {
            bits |= 0xc0;
        }
        self.objects[o].flags |= bits;
        self.objects[o].cell_flags = self.closed_sides(x, y, mask)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn param(name: &[u8; 4], kind: i16, count: i16, data: &[u8]) -> Parameter {
        Parameter {
            kind,
            count,
            name: FourCC(*name),
            data: data.to_vec(),
        }
    }

    #[test]
    fn memory_is_tfl_layout() {
        let a = MapAction {
            identifier: 1,
            kind: 1,
            tag: 0,
            flags: 0,
            delay: 0,
            delay_range: 0,
            time: 0,
            parameters: vec![
                param(b"wayp", 12, 1, &[0, 0, 0xcc, 0xa5, 0, 0, 0xe5, 0x2e]),
                param(b"wayr", 6, 0, &[]),
                param(b"name", 1, 3, b"ab\0"),
                param(b"#own", 3, 1, &[0x27, 0x15]),
            ],
        };
        let (m, offsets) = a.memory();
        assert_eq!(offsets, [8, 24, 32, 44]);
        let m: Vec<u8> = m.iter().map(|b| b.unwrap_or(0xee)).collect();
        assert_eq!(
            m,
            [
                &[12, 0, 1, 0][..],
                b"pyaw",
                &[0xa5, 0xcc, 0, 0, 0x2e, 0xe5, 0, 0],
                &[6, 0, 0, 0],
                b"ryaw",
                &[1, 0, 3, 0],
                b"eman",
                b"ab\0\xee",
                &[3, 0, 1, 0],
                b"nwo#",
                &[0x15, 0x27, 0xee, 0xee],
            ]
            .concat()
        );
    }
}
