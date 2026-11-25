use crate::draw_site;
use crate::map_actions::{combined, find, index_of};
use crate::model_effects::Request;
use crate::{Stop, TflSim};

const SIZE: usize = 0x108;
const QUEUE: usize = 16;

type Entry = [i16; 3];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KnotState {
    pub action: i16,
    pub centre: (i32, i32),
    pub radius: i32,
    pub state: i16,
    pub out: i32,
    pub time: i32,
    pub duration: i32,
    pub model_count: i32,
    pub models: [i16; 5],
    pub permutations: [i16; 5],
    pub start: i16,
    pub incoming: [Entry; QUEUE],
    pub incoming_count: i16,
    pub free: [Entry; QUEUE],
    pub free_count: i16,
    pub next: i16,
    pub box_centre: i32,
    pub wave_box_centre: i32,
}

impl KnotState {
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut b = vec![0u8; SIZE];
        let mut w16 = |o: usize, v: i16| b[o..o + 2].copy_from_slice(&v.to_le_bytes());
        w16(0, self.action);
        w16(0x10, self.state);
        for k in 0..5 {
            w16(0x24 + 2 * k, self.models[k]);
            w16(0x2e + 2 * k, self.permutations[k]);
        }
        w16(0x38, self.start);
        for k in 0..QUEUE {
            for j in 0..3 {
                w16(0x3a + 6 * k + 2 * j, self.incoming[k][j]);
                w16(0x9c + 6 * k + 2 * j, self.free[k][j]);
            }
        }
        w16(0x9a, self.incoming_count);
        w16(0xfc, self.free_count);
        w16(0xfe, self.next);
        let mut w32 = |o: usize, v: i32| b[o..o + 4].copy_from_slice(&v.to_le_bytes());
        w32(4, self.centre.0);
        w32(8, self.centre.1);
        w32(0xc, self.radius);
        w32(0x14, self.out);
        w32(0x18, self.time);
        w32(0x1c, self.duration);
        w32(0x20, self.model_count);
        w32(0x100, self.box_centre);
        w32(0x104, self.wave_box_centre);
        b
    }

    pub fn from_bytes(b: &[u8]) -> Self {
        let r16 = |o: usize| i16::from_le_bytes([b[o], b[o + 1]]);
        let r32 = |o: usize| i32::from_le_bytes(b[o..o + 4].try_into().unwrap());
        let entry = |o: usize| [r16(o), r16(o + 2), r16(o + 4)];
        Self {
            action: r16(0),
            centre: (r32(4), r32(8)),
            radius: r32(0xc),
            state: r16(0x10),
            out: r32(0x14),
            time: r32(0x18),
            duration: r32(0x1c),
            model_count: r32(0x20),
            models: std::array::from_fn(|k| r16(0x24 + 2 * k)),
            permutations: std::array::from_fn(|k| r16(0x2e + 2 * k)),
            start: r16(0x38),
            incoming: std::array::from_fn(|k| entry(0x3a + 6 * k)),
            incoming_count: r16(0x9a),
            free: std::array::from_fn(|k| entry(0x9c + 6 * k)),
            free_count: r16(0xfc),
            next: r16(0xfe),
            box_centre: r32(0x100),
            wave_box_centre: r32(0x104),
        }
    }

    fn push_incoming(&mut self, e: Entry) -> Result<(), Stop> {
        let n = self.incoming_count as usize;
        if n >= QUEUE {
            return Err(Stop::new(
                draw_site::KNOT_INCOMING_OVERFLOW,
                "a knot's incoming queue overflows",
            ));
        }
        self.incoming[n] = e;
        self.incoming_count += 1;
        Ok(())
    }

    fn push_free(&mut self, e: Entry) -> Result<(), Stop> {
        let n = self.free_count as usize;
        if n >= QUEUE {
            return Err(Stop::new(
                draw_site::KNOT_FREE_OVERFLOW,
                "a knot's free queue overflows",
            ));
        }
        self.free[n] = e;
        self.free_count += 1;
        Ok(())
    }

    fn pop_incoming(&mut self) {
        let n = (self.incoming_count as usize).clamp(1, QUEUE);
        self.incoming.copy_within(1..n, 0);
        self.incoming_count -= 1;
    }

    fn pop_free(&mut self) {
        let n = (self.free_count as usize).clamp(1, QUEUE);
        self.free.copy_within(1..n, 0);
        self.free_count -= 1;
    }
}

fn centre_of_points(points: &[(i32, i32)], box_centre: bool) -> ((i32, i32), i32) {
    if points.is_empty() {
        return ((-1, -1), 0);
    }
    let (mut x0, mut y0, mut x1, mut y1) = (i32::MAX, i32::MAX, i32::MIN, i32::MIN);
    let (mut sx, mut sy) = (0i32, 0i32);
    for &(x, y) in points {
        x0 = x0.min(x);
        y0 = y0.min(y);
        x1 = x1.max(x);
        y1 = y1.max(y);
        sx = sx.wrapping_add(x);
        sy = sy.wrapping_add(y);
    }
    if box_centre {
        let cx = x1.wrapping_add(x0) / 2;
        let cy = y1.wrapping_add(y0) / 2;
        let (a, b) = (cx.wrapping_sub(x0), cy.wrapping_sub(y0));
        return ((cx, cy), if a < b { b } else { a });
    }
    let n = points.len() as i32;
    let (cx, cy) = (sx / n, sy / n);
    let mut r = x1.wrapping_sub(cx).max(cx.wrapping_sub(x0));
    r = r.max(cy.wrapping_sub(y0));
    r = r.max(y1.wrapping_sub(cy));
    ((cx, cy), r)
}

impl TflSim {
    pub(crate) fn world_knot_action(&mut self, i: usize) -> Result<u32, Stop> {
        let mut s = match self.structure(i, b"#glo", SIZE) {
            Some(b) => KnotState::from_bytes(&b),
            None => match self.knot_initial(i)? {
                Some(s) => s,
                None => return Ok(4),
            },
        };
        let tick = self.tick as i32;
        let mut result = 0;
        match s.state {
            0 => {
                if s.out != 0 {
                    if s.next != -1
                        && let Some(b) = self.structure(s.next as usize, b"#glo", SIZE)
                    {
                        let mut n = KnotState::from_bytes(&b);
                        self.handle_intra_knot(&mut s, &mut n)?;
                        self.set_param(s.next as usize, b"#glo", 1, SIZE as i16, n.to_bytes());
                    }
                } else if s.incoming_count != 0 {
                    if self.monsters_in_radius(s.centre, s.radius).is_empty() {
                        self.set_knot_state(&mut s, 1)?;
                        if s.start != -1 {
                            self.trigger(s.start as usize)?;
                        }
                    }
                } else if s.next == -1 {
                    result = 6;
                }
            }
            1 if tick.wrapping_sub(s.time) > s.duration => {
                let e = s.incoming[0];
                if e[1] != -1 {
                    self.activate(e[1] as usize)?;
                }
                s.push_free(e)?;
                s.pop_incoming();
                self.set_knot_state(&mut s, 0)?;
                if s.next != -1 {
                    s.out = 1;
                }
            }
            2 if tick.wrapping_sub(s.time) > s.duration => {
                let e = s.incoming[0];
                if e[1] != -1 {
                    self.activate(e[1] as usize)?;
                }
                let to = e[2];
                match (to != -1)
                    .then(|| self.structure(to as usize, b"#glo", SIZE))
                    .flatten()
                {
                    Some(b) => {
                        let mut n = KnotState::from_bytes(&b);
                        n.push_incoming(e)?;
                        self.set_param(to as usize, b"#glo", 1, SIZE as i16, n.to_bytes());
                    }
                    None => s.push_free(e)?,
                }
                s.pop_incoming();
                self.set_knot_state(&mut s, 0)?;
                s.out = 0;
            }
            _ => {}
        }
        self.set_param(i, b"#glo", 1, SIZE as i16, s.to_bytes());
        Ok(result)
    }

    fn knot_initial(&mut self, i: usize) -> Result<Option<KnotState>, Stop> {
        crate::map_actions::not_read_by_tfl(&[b"dist"]);
        let actions = self.map_actions.actions.clone();
        let a = &actions;
        let short = |name: &[u8; 4], k: usize| {
            find(a, i, 0, name, 3)
                .filter(|p| (k as i32) < i32::from(p.count))
                .map(|p| p.i16_at(k))
        };
        let index = |id: i16| index_of(a, id).map_or(-1, |j| j as i16);
        let ids = combined(a, i, b"mode", 0xf, 5);
        let mut s = KnotState {
            action: i as i16,
            centre: (0, 0),
            radius: 0,
            state: 0,
            out: 0,
            time: 0,
            duration: 0,
            model_count: ids.len() as i32,
            models: [0; 5],
            permutations: [0; 5],
            start: -1,
            incoming: [[0; 3]; QUEUE],
            incoming_count: 0,
            free: [[0; 3]; QUEUE],
            free_count: 0,
            next: -1,
            box_centre: 0,
            wave_box_centre: 0,
        };
        let mut ok = !ids.is_empty();
        let mut points = Vec::new();
        for (k, &id) in ids.iter().enumerate() {
            s.models[k] = id;
            match self.model_index(i32::from(id)) {
                Some(m) => {
                    points.push(self.model_centre_point(m)?);
                    s.permutations[k] = self.model_instances[m].permutation;
                }
                None => ok = false,
            }
        }
        s.box_centre = i32::from(find(a, i, 2, b"knce", 0).is_some());
        s.wave_box_centre = i32::from(find(a, i, 2, b"wace", 0).is_some());
        let (centre, radius) = centre_of_points(&points, s.box_centre != 0);
        s.centre = centre;
        s.radius = radius;
        if let Some(r) = find(a, i, 0, b"radi", 6).filter(|p| p.count > 0) {
            s.radius = r.i32_at(0);
        }
        if let Some(c) = find(a, i, 0, b"cent", 12).filter(|p| p.count > 0) {
            s.centre = (c.i32_at(0), c.i32_at(1));
        }
        s.out = i32::from(find(a, i, 2, b"_out", 0).is_some());
        if let Some(n) = short(b"next", 0) {
            s.next = index(n);
        }
        match find(a, i, 0, b"wave", 3) {
            Some(p) => {
                for k in 0..p.count.max(0) as usize {
                    let Some(w) = short(b"wave", k) else {
                        continue;
                    };
                    let wave = index(w);
                    if wave == -1 {
                        continue;
                    }
                    let acow = short(b"acow", k).map_or(-1, index);
                    if s.next == -1 {
                        s.push_incoming([wave, acow, 0])?;
                    } else {
                        s.push_free([wave, acow, 0])?;
                    }
                }
            }
            None if s.out == 0 => ok = false,
            None => {}
        }
        match find(a, i, 0, b"dura", 8).filter(|p| p.count > 0) {
            Some(d) => {
                let v = d.i32_at(0).wrapping_mul(0x1e);
                s.duration = v.wrapping_add((v >> 31) & 0xffff) >> 16;
            }
            None => ok = false,
        }
        if let Some(st) = short(b"strt", 0) {
            s.start = index(st);
        }
        if s.next != -1 {
            s.out = 1;
        }
        Ok(ok.then_some(s))
    }

    fn model_centre_point(&mut self, m: usize) -> Result<(i32, i32), Stop> {
        let id = self.model_instances[m].mode;
        let mode = self.data.def(b"mode", id)?;
        let geom_id = mode.tag(4).ok_or_else(|| {
            Stop::new(
                draw_site::MODEL_CENTRE_NO_GEOMETRY,
                format!("mode {id}: no geom"),
            )
        })?;
        let geom = self.data.def_arc(b"geom", geom_id)?;
        let mi = &self.model_instances[m];
        Ok((
            mi.position[0].wrapping_add(i32::from(geom.i16(0x10))),
            mi.position[1].wrapping_add(i32::from(geom.i16(0x12))),
        ))
    }

    fn monsters_in_radius(&self, c: (i32, i32), r: i32) -> Vec<i16> {
        let r2 = r.wrapping_mul(r);
        let mut out = Vec::new();
        for m in &self.monsters {
            if m.identifier == 0 {
                continue;
            }
            let o = &self.objects[m.object];
            if o.flags & 0x200 != 0 {
                continue;
            }
            let dx = o.x.wrapping_sub(c.0).wrapping_abs();
            let dy = o.y.wrapping_sub(c.1).wrapping_abs();
            if dx >= r || dy >= r {
                continue;
            }
            if dy.wrapping_mul(dy).wrapping_add(dx.wrapping_mul(dx)) >= r2 {
                continue;
            }
            if out.len() < 0x20 {
                out.push(m.identifier);
            }
        }
        out
    }

    fn handle_intra_knot(&mut self, s: &mut KnotState, n: &mut KnotState) -> Result<(), Stop> {
        if s.state != 0 || n.state != 0 || s.free_count == 0 || n.free_count == 0 {
            return Ok(());
        }
        let ins = self.monsters_in_radius(s.centre, s.radius);
        let inn = self.monsters_in_radius(n.centre, n.radius);
        if ins.is_empty() && inn.is_empty() {
            return Ok(());
        }
        for (k, ids) in [(&mut *s, ins), (&mut *n, inn)] {
            let wave = k.free[0][0] as usize;
            let data: Vec<u8> = ids.iter().flat_map(|v| v.to_be_bytes()).collect();
            self.set_param(wave, b"subj", 2, ids.len() as i16, data);
            k.free[0][2] = k.next;
            let e = k.free[0];
            k.push_incoming(e)?;
            k.pop_free();
        }
        self.set_knot_state(s, 2)?;
        self.set_knot_state(n, 2)
    }

    fn set_knot_state(&mut self, s: &mut KnotState, state: i16) -> Result<(), Stop> {
        let mut models: Option<(i16, i16)> = None;
        match state {
            0 => models = Some((0, 1)),
            1 => {
                let wave = s.incoming[0][0];
                if wave != -1 {
                    let ids = combined(&self.map_actions.actions, wave as usize, b"subj", 2, 300);
                    if !ids.is_empty() {
                        let mut points = Vec::new();
                        for &id in &ids {
                            if let Some(p) = self.unit_position(id) {
                                points.push(p);
                            }
                        }
                        let (c, _) = centre_of_points(&points, s.wave_box_centre != 0);
                        let (dx, dy) = (c.0.wrapping_sub(s.centre.0), c.1.wrapping_sub(s.centre.1));
                        for &id in &ids {
                            let Some(k) = self.unit_marker(id) else {
                                continue;
                            };
                            let mon = i16::from_be_bytes([
                                self.data.mesh.markers[k].type_data[0x1c],
                                self.data.mesh.markers[k].type_data[0x1d],
                            ]);
                            if mon != -1 {
                                let Some(m) = self.marker_monster_slot(k, mon) else {
                                    continue;
                                };
                                let o = self.monsters[m].object;
                                self.let_go_object(o);
                                let ob = &mut self.objects[o];
                                ob.x = ob.x.wrapping_sub(dx);
                                ob.y = ob.y.wrapping_sub(dy);
                                let cell =
                                    self.cell_at(self.objects[o].x >> 10, self.objects[o].y >> 10);
                                self.objects[o].next_in_cell = self.cell_heads[cell];
                                self.cell_heads[cell] = o as i16;
                            } else {
                                let mk = &mut self.data.mesh.markers[k];
                                mk.x = mk.x.wrapping_sub(dx);
                                mk.y = mk.y.wrapping_sub(dy);
                            }
                            self.make_monster_visible(id, 1)?;
                            models = Some((4, s.duration as i16));
                        }
                    }
                }
            }
            2 => {
                let wave = s.incoming[0][0];
                if wave != -1 {
                    let ids = combined(&self.map_actions.actions, wave as usize, b"subj", 2, 300);
                    if !ids.is_empty() {
                        for &id in &ids {
                            self.make_monster_invisible(id, 1)?;
                        }
                        models = Some((4, s.duration as i16));
                    }
                }
            }
            _ => {}
        }
        if let Some((add, duration)) = models {
            for k in 0..s.model_count.max(0) as usize {
                let r = Request {
                    duration,
                    new_permutation: i32::from(s.permutations[k]) + i32::from(add),
                    permutation: 0,
                    material: 0,
                    ..Default::default()
                };
                self.new_model_effect(i32::from(s.models[k]), -1, &r)?;
            }
        }
        s.state = state;
        s.time = self.tick as i32;
        Ok(())
    }

    fn unit_marker(&self, id: i16) -> Option<usize> {
        self.data
            .mesh
            .markers
            .iter()
            .position(|m| m.marker_type == crate::setup::UNIT && m.marker_id == id)
    }

    fn marker_monster_slot(&self, k: usize, mon: i16) -> Option<usize> {
        let id = self.data.mesh.markers[k].marker_id;
        self.monsters
            .get(mon as usize)
            .filter(|m| m.identifier == id)
            .map(|_| mon as usize)
    }

    fn unit_position(&self, id: i16) -> Option<(i32, i32)> {
        let k = self.unit_marker(id)?;
        let mk = &self.data.mesh.markers[k];
        let mon = i16::from_be_bytes([mk.type_data[0x1c], mk.type_data[0x1d]]);
        if mon == -1 {
            return Some((mk.x, mk.y));
        }
        let m = self.marker_monster_slot(k, mon)?;
        let o = &self.objects[self.monsters[m].object];
        Some((o.x, o.y))
    }
}
