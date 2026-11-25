use mandrake_common::FourCC;

use crate::map_actions::{Parameter, Walk, combined, find, index_of};
use crate::{Stop, TflSim};

#[derive(Debug, Clone, Copy)]
struct Entry {
    identifier: i16,
    point: (i32, i32),
    inside: bool,
}

fn word(s: &TflSim, i: usize, name: &[u8; 4], kind: i16) -> Option<i32> {
    find(&s.map_actions.actions, i, 0, name, kind)
        .filter(|p| p.count > 0)
        .map(|p| p.i32_at(0))
}

fn inside_polygon(p: (i32, i32), poly: &[(i32, i32)], closed: bool) -> bool {
    crate::actions::point_in_polygon(p, poly, closed)
}

impl TflSim {
    pub(crate) fn geometry_filter(&mut self, i: usize) -> Result<u32, Stop> {
        let a = &self.map_actions.actions;
        let subj = find(a, i, 0, b"subj", 2).filter(|p| p.count > 0).is_some();
        let kind_tag = word(self, i, b"type", 7);
        let obje = find(&self.map_actions.actions, i, 0, b"obje", 0xe)
            .filter(|p| p.count > 0)
            .is_some();
        if (subj && kind_tag.is_some()) || ((subj || kind_tag.is_some()) && obje) {
            return Ok(0);
        }
        let mut list: Vec<Entry> = Vec::new();
        if subj {
            let ids = combined(&self.map_actions.actions, i, b"subj", 2, 300);
            if find(&self.map_actions.actions, i, 2, b"mark", 0).is_none() {
                let slots = self.monster_slots(&ids);
                let slots: Vec<usize> = slots
                    .into_iter()
                    .filter(|&s| self.monsters[s].flags & 0x4000_0000 == 0)
                    .collect();
                let hold = word(self, i, b"hold", 7);
                for s in slots {
                    let m = &self.monsters[s];
                    if let Some(h) = hold {
                        let c = m.carried;
                        let held = c != -1 && {
                            let ob = &self.objects[c as usize];
                            ob.kind == 4
                                && ob
                                    .sleeping
                                    .is_some_and(|d| u32::from_be_bytes(d.0) == h as u32)
                        };
                        if !held {
                            continue;
                        }
                    }
                    let o = &self.objects[m.object];
                    list.push(Entry {
                        identifier: m.identifier,
                        point: (o.x, o.y),
                        inside: false,
                    });
                }
            } else {
                for id in ids {
                    if let Some(mk) = self
                        .data
                        .mesh
                        .markers
                        .iter()
                        .find(|m| m.marker_type == 3 && m.marker_id == id)
                    {
                        list.push(Entry {
                            identifier: id,
                            point: (mk.x, mk.y),
                            inside: false,
                        });
                    }
                }
            }
        }
        if let Some(t) = kind_tag {
            for k in 0..self.object_pool.high {
                if list.len() > 299 {
                    break;
                }
                let ob = &self.objects[k];
                if ob.identifier == 0 || ob.flags & 0x200 != 0 || !(3..5).contains(&ob.kind) {
                    continue;
                }
                let def = if ob.kind == 4 {
                    ob.sleeping
                } else {
                    self.projectiles.list[ob.owner as usize].definition
                };
                if def.is_some_and(|d| u32::from_be_bytes(d.0) == t as u32) {
                    list.push(Entry {
                        identifier: ob.identifier,
                        point: (ob.x, ob.y),
                        inside: false,
                    });
                }
            }
        }
        if obje && let Some(p) = find(&self.map_actions.actions, i, 0, b"obje", 0xe).cloned() {
            for id in p.shorts() {
                #[allow(clippy::needless_range_loop)]
                for k in 0..self.object_pool.high {
                    if list.len() > 299 {
                        break;
                    }
                    let ob = &self.objects[k];
                    if ob.identifier != 0 && ob.identifier == id {
                        list.push(Entry {
                            identifier: id,
                            point: (ob.x, ob.y),
                            inside: false,
                        });
                    }
                }
            }
        }
        if list.is_empty() {
            return Ok(0);
        }
        let mut chosen = false;
        if let Some(poly) = find(&self.map_actions.actions, i, 0, b"poly", 12).cloned() {
            let closed = find(&self.map_actions.actions, i, 2, b"clpo", 0).is_some();
            let pts: Vec<(i32, i32)> = (0..poly.count.max(0) as usize)
                .map(|k| (poly.i32_at(2 * k), poly.i32_at(2 * k + 1)))
                .collect();
            for e in &mut list {
                e.inside = inside_polygon(e.point, &pts, closed);
            }
            chosen = true;
        }
        let centre = find(&self.map_actions.actions, i, 0, b"cent", 12)
            .filter(|p| p.count > 0)
            .map(|p| (p.i32_at(0), p.i32_at(1)))
            .or_else(|| {
                find(&self.map_actions.actions, i, 0, b"cemo", 2)
                    .filter(|p| p.count > 0)
                    .and_then(|p| self.monster_slots(&[p.i16_at(0)]).first().copied())
                    .map(|s| {
                        let o = &self.objects[self.monsters[s].object];
                        (o.x, o.y)
                    })
            });
        if let Some(c) = centre
            && let Some(r) = word(self, i, b"radi", 6)
        {
            for e in &mut list {
                let dx = (e.point.0 - c.0).abs();
                let dy = (e.point.1 - c.1).abs();
                e.inside = dx < r
                    && dy < r
                    && dy.wrapping_mul(dy).wrapping_add(dx.wrapping_mul(dx)) < r.wrapping_mul(r);
            }
            chosen = true;
        }
        if find(&self.map_actions.actions, i, 2, b"_map", 0).is_some() {
            for e in &mut list {
                e.inside = true;
            }
            chosen = true;
        }
        match word(self, i, b"coun", 5) {
            Some(n) => {
                for (k, e) in list.iter_mut().enumerate() {
                    e.inside = (k as i32) < n;
                }
            }
            None if !chosen => return Ok(0),
            None => {}
        }
        let kind: i16 = if subj { 2 } else { 0xe };
        let resu = find(&self.map_actions.actions, i, 0, b"resu", 3)
            .filter(|p| p.count > 0)
            .map(|p| p.i16_at(0));
        let j = match resu {
            Some(id) => match index_of(&self.map_actions.actions, id) {
                Some(j) => j,
                None => return Ok(0),
            },
            None => i,
        };
        let most = word(self, i, b"_max", 5).map_or(list.len() as i16, |m| m as i16);
        let mut result = 0;
        for (name, inside) in [(b"insi", true), (b"outs", false)] {
            if let Some(n) = word(self, i, name, 7)
                && self.write_selection(j, n as u32, kind, &list, most, inside) != 0
            {
                result |= 2;
            }
        }
        for (name, inside) in [(b"inlo", true), (b"oulo", false)] {
            if let Some(n) = word(self, i, name, 7)
                && self.write_selection(j, n as u32, 0xc, &list, most, inside) != 0
            {
                result |= 2;
            }
        }
        Ok(result)
    }

    fn write_selection(
        &mut self,
        j: usize,
        name: u32,
        kind: i16,
        list: &[Entry],
        most: i16,
        inside: bool,
    ) -> i16 {
        let n = name.to_be_bytes();
        loop {
            let mut w = Walk::new(j, 3, Some(&n), kind);
            let Some(f) = w.next(&self.map_actions.actions) else {
                break;
            };
            self.map_actions.actions[f.action]
                .parameters
                .remove(f.param);
        }
        let mut p = Parameter {
            kind,
            count: 0,
            name: FourCC(n),
            data: Vec::new(),
        };
        let mut left = most;
        let mut written = 0i16;
        for e in list.iter().filter(|e| e.inside == inside) {
            let more = left > 0;
            left = left.wrapping_sub(1);
            if !more {
                continue;
            }
            if kind == 0xc {
                p.data.extend_from_slice(&e.point.0.to_be_bytes());
                p.data.extend_from_slice(&e.point.1.to_be_bytes());
            } else {
                p.data.extend_from_slice(&e.identifier.to_be_bytes());
            }
            p.count += 1;
            written += 1;
        }
        self.map_actions.actions[j].parameters.push(p);
        written
    }
}
