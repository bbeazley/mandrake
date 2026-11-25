use crate::draw_site;
use crate::trig::{arctangent, index, trig};
use crate::{Stop, TflSim};

pub type Places = (Vec<Vec<(i32, i32)>>, Vec<u16>);

pub type FormationPlaces = (Vec<(i32, i32)>, Vec<u16>);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MovementCommand {
    pub flags: u16,
    pub formation: u16,
    pub facing: u16,
}

pub fn heapsort<T: Copy>(items: &mut [T], cmp: impl Fn(&T, &T) -> i32) {
    crate::statistics::qsort(items, cmp);
}

fn formation_reversed(formation: u16, count: usize) -> bool {
    let n = count as i16;
    match formation {
        0 | 2 | 3 => n <= 4,
        1 => n <= 8,
        6 | 7 => true,
        _ => false,
    }
}

pub(crate) fn full_turn() -> f64 {
    f64::from((8.0 * 1f64.atan()) as f32)
}

fn ftol(v: f64) -> i32 {
    v.trunc() as i64 as i32
}

pub fn formation_places(
    formation: u16,
    point: (i32, i32),
    angle: u16,
    count: usize,
) -> Result<FormationPlaces, Stop> {
    let n = count as i32;
    let a = i32::from(angle);
    let same = |p: Vec<(i32, i32)>| {
        let f = vec![angle; p.len()];
        (p, f)
    };
    Ok(match formation {
        0 => same(line_places(point, a, n, 4, 4, 0x200)),
        1 => same(line_places(point, a, n, 8, 8, 0x200)),
        2 => same(line_places(point, a, n, 4, 4, 0x400)),
        3 => same(line_places(point, a, n, 4, 5, 0x400)),
        4 => {
            let mut side = crate::groups::isqrt(count as u32) as i32;
            if side * side < n {
                side += 1;
            }
            same(line_places(point, a, n, side, side, 0x200))
        }
        5 => same(scattered_places(point, count, 0x300)),
        6 => arc_places(point, a, count, 0x4000),
        7 => arc_places(point, a, count, 0x5555),
        8 => same(wedge_places(point, a, count)),
        9 => circle_places(point, a, count),
        f => {
            return Err(Stop::new(
                draw_site::FORMATION_WITHOUT_BUILDER,
                format!("formation {f} has no builder"),
            ));
        }
    })
}

fn scattered_places(point: (i32, i32), count: usize, spacing: i32) -> Vec<(i32, i32)> {
    let mut seed = (point.0 ^ point.1) as u32;
    let range = (crate::groups::isqrt(count as u32) + 2).wrapping_mul(spacing as u32);
    let half = (range as i32) >> 1;
    let mut out: Vec<(i32, i32)> = Vec::with_capacity(count);
    while out.len() < count {
        let mut tries = 0i16;
        let mut ok = false;
        let mut c = point;
        while !ok {
            c = if out.is_empty() {
                point
            } else {
                let u = seed.wrapping_mul(0x19660d).wrapping_add(0x3c6ef35f);
                seed = u.wrapping_mul(0x19660d).wrapping_add(0x3c6ef35f);
                (
                    ((u % range) as i32)
                        .wrapping_sub(half)
                        .wrapping_add(point.0),
                    ((seed % range) as i32)
                        .wrapping_sub(half)
                        .wrapping_add(point.1),
                )
            };
            ok = out.iter().all(|p| {
                let (dx, dy) = (c.0.wrapping_sub(p.0), c.1.wrapping_sub(p.1));
                dy.wrapping_mul(dy).wrapping_add(dx.wrapping_mul(dx)) >= spacing * spacing
            });
            tries += 1;
            if tries >= 12 {
                break;
            }
        }
        out.push(c);
    }
    out
}

fn arc_places(point: (i32, i32), angle: i32, count: usize, arc: i32) -> FormationPlaces {
    let n = count as i32;
    let t = f64::from((3 * n).wrapping_shl(24).wrapping_sub(0x300_0000));
    let r = ftol(t / (f64::from(arc) * full_turn())).max(0x800);
    let step = ftol(t / (f64::from(r) * full_turn()));
    let k = index(angle);
    let tr = trig();
    let cx = ((i32::from(tr.cosine[k]) * r) >> 14) + point.0;
    let cy = ((i32::from(tr.sine[k]) * r) >> 14) + point.1;
    let (mut places, mut facings) = (Vec::new(), Vec::new());
    let mut acc = 0i32;
    for _ in 0..count {
        let a = ((acc / (n - 1)) - (step >> 1) + angle) & 0xffff;
        let j = index(a);
        places.push((
            cx - ((i32::from(tr.cosine[j]) * r) >> 14),
            cy - ((i32::from(tr.sine[j]) * r) >> 14),
        ));
        facings.push(a as u16);
        acc = acc.wrapping_add(step);
    }
    (places, facings)
}

fn wedge_places(point: (i32, i32), angle: i32, count: usize) -> Vec<(i32, i32)> {
    let k = index(angle);
    let c = i32::from(trig().cosine[k]);
    let s = i32::from(trig().sine[k]);
    let n = count as i32;
    let odd = n & 1 != 0;
    let half = if odd { n / 2 + 1 } else { n / 2 };
    let lead = (point.0, (((half / 2) * s) << 9 >> 14) + point.1);
    let extra = if odd { 0 } else { 0x100 };
    let mut out = Vec::with_capacity(count);
    for r in 0..half {
        if r == 0 {
            if odd {
                out.push(lead);
            } else {
                out.push((
                    lead.0 + ((-(extra * s)) >> 14),
                    lead.1 + ((extra * c) >> 14),
                ));
                out.push((
                    lead.0 + ((extra * s) >> 14),
                    lead.1 + ((-(extra * c)) >> 14),
                ));
            }
        } else {
            let d = (r << 9) + extra;
            let e = -(r << 9);
            out.push((
                lead.0 + ((e * c - d * s) >> 14),
                lead.1 + ((d * c + e * s) >> 14),
            ));
            out.push((
                lead.0 + ((e * c + d * s) >> 14),
                lead.1 + ((e * s - d * c) >> 14),
            ));
        }
    }
    out
}

fn circle_places(point: (i32, i32), angle: i32, count: usize) -> FormationPlaces {
    let n = count as i32;
    let r = ftol(f64::from(n * 0x300) / full_turn());
    let tr = trig();
    let (mut places, mut facings) = (Vec::new(), Vec::new());
    for i in 0..n {
        let a = ((i.wrapping_mul(0x10000) / n).wrapping_add(angle)) & 0xffff;
        let k = index(a);
        places.push((
            ((i32::from(tr.cosine[k]) * r) >> 14) + point.0,
            ((i32::from(tr.sine[k]) * r) >> 14) + point.1,
        ));
        facings.push(a as u16);
    }
    (places, facings)
}

fn line_places(
    point: (i32, i32),
    facing: i32,
    count: i32,
    first: i32,
    second: i32,
    spacing: i32,
) -> Vec<(i32, i32)> {
    let k = index(facing);
    let c = i32::from(trig().cosine[k]);
    let s = i32::from(trig().sine[k]);
    let mut out = Vec::new();
    let mut left = count;
    let mut odd = false;
    let (mut row_x, mut row_y) = (0i32, 0i32);
    while left > 0 {
        let size = if odd { second } else { first };
        let n = size.min(left);
        let start = -(((n - 1) * spacing) / 2);
        let mut ax = row_x - start * s;
        let mut ay = start * c + row_y;
        for _ in 0..n {
            out.push(((ax >> 14) + point.0, (ay >> 14) + point.1));
            ay += c * spacing;
            ax -= s * spacing;
        }
        left -= size;
        odd = !odd;
        row_y -= s * spacing;
        row_x -= c * spacing;
    }
    out
}

const FRAME_ORDER: usize = 0x64;
const FRAME_SLOTS: usize = 0xa0;
const FRAME_FACINGS: usize = 0xdc;
const FRAME_KEYS: usize = 0x118;
const FRAME_PLACES: usize = 0x190;
const FRAME_END: usize = 0x550;
const FRAME_ROW: usize = 0xf0;

fn frame_stop() -> Stop {
    Stop::new(
        draw_site::MOVEMENT_COMMAND_FRAME,
        "a movement command's arrays run out of the movement command's stack frame",
    )
}

fn key_of(keys: &[i32], k: i16) -> i32 {
    usize::try_from(k)
        .ok()
        .and_then(|k| keys.get(k))
        .copied()
        .unwrap_or(0)
}

struct CommandFrame {
    bytes: Vec<u8>,
    written: Vec<bool>,
}

impl CommandFrame {
    fn new(waypoints: &[(i32, i32)], n: usize) -> Result<Self, Stop> {
        if n > 90 {
            return Err(frame_stop());
        }
        let mut f = Self {
            bytes: vec![0; FRAME_END],
            written: vec![false; FRAME_END],
        };
        for (w, p) in waypoints.iter().take(4).enumerate() {
            f.put32(FRAME_ORDER + 8 * w, p.0)?;
            f.put32(FRAME_ORDER + 8 * w + 4, p.1)?;
        }
        Ok(f)
    }

    fn put(&mut self, at: usize, b: &[u8]) -> Result<(), Stop> {
        if at + b.len() > FRAME_END {
            return Err(frame_stop());
        }
        self.bytes[at..at + b.len()].copy_from_slice(b);
        self.written[at..at + b.len()].fill(true);
        Ok(())
    }

    fn get<const N: usize>(&self, at: usize) -> Result<[u8; N], Stop> {
        if at + N > FRAME_END || !self.written[at..at + N].iter().all(|&w| w) {
            return Err(frame_stop());
        }
        Ok(self.bytes[at..at + N].try_into().expect("N bytes"))
    }

    fn put16(&mut self, at: usize, v: i16) -> Result<(), Stop> {
        self.put(at, &v.to_le_bytes())
    }

    fn put32(&mut self, at: usize, v: i32) -> Result<(), Stop> {
        self.put(at, &v.to_le_bytes())
    }

    fn get16(&self, at: usize) -> Result<i16, Stop> {
        self.get::<2>(at).map(i16::from_le_bytes)
    }

    fn get32(&self, at: usize) -> Result<i32, Stop> {
        self.get::<4>(at).map(i32::from_le_bytes)
    }

    fn place_at(w: usize, k: i32) -> Result<usize, Stop> {
        usize::try_from(k)
            .ok()
            .map(|k| FRAME_PLACES + FRAME_ROW * w + 8 * k)
            .ok_or_else(frame_stop)
    }

    fn put_place(&mut self, w: usize, k: usize, p: (i32, i32)) -> Result<(), Stop> {
        let at = Self::place_at(w, k as i32)?;
        self.put32(at, p.0)?;
        self.put32(at + 4, p.1)
    }

    fn place(&self, w: usize, k: i32) -> Result<(i32, i32), Stop> {
        let at = Self::place_at(w, k)?;
        Ok((self.get32(at)?, self.get32(at + 4)?))
    }
}

impl TflSim {
    pub(crate) fn monsters_centre_spread(&self, slots: &[usize]) -> ((i32, i32), i32) {
        if slots.is_empty() {
            return ((-1, -1), 0);
        }
        let (mut lo_x, mut lo_y, mut hi_x, mut hi_y) = (i32::MAX, i32::MAX, i32::MIN, i32::MIN);
        let (mut sx, mut sy) = (0i32, 0i32);
        for &s in slots {
            let o = &self.objects[self.monsters[s].object];
            lo_x = lo_x.min(o.x);
            lo_y = lo_y.min(o.y);
            hi_x = hi_x.max(o.x);
            hi_y = hi_y.max(o.y);
            sx = sx.wrapping_add(o.x);
            sy = sy.wrapping_add(o.y);
        }
        let n = slots.len() as i32;
        let (cx, cy) = (sx / n, sy / n);
        let mut r = if hi_x - cx <= cx - lo_x {
            cx - lo_x
        } else {
            hi_x - cx
        };
        r = r.max(cy - lo_y);
        r = r.max(hi_y - cy);
        ((cx, cy), r)
    }

    pub fn command_places(
        &self,
        command: &MovementCommand,
        slots: &[usize],
        waypoints: &[(i32, i32)],
    ) -> Result<Places, Stop> {
        let (centre, _) = self.monsters_centre_spread(slots);
        let mut wp = waypoints.to_vec();
        if command.flags & 0x40 != 0 {
            if wp.len() == 4 {
                wp.truncate(3);
            }
            wp.push(centre);
        }
        let n = wp.len();
        let last = if n == 0 { centre } else { wp[n - 1] };
        let facing = if command.flags & 0x10 == 0 {
            arctangent(last.0 - centre.0, last.1 - centre.1)
        } else {
            command.facing
        };
        let mut places = vec![vec![(0, 0); slots.len()]; n];
        let mut facings = vec![facing; slots.len()];
        if command.flags & 8 == 0 {
            for (k, &s) in slots.iter().enumerate() {
                let o = &self.objects[self.monsters[s].object];
                for (w, p) in wp.iter().enumerate() {
                    places[w][k] = (p.0 + (o.x - centre.0), p.1 + (o.y - centre.1));
                }
            }
        } else {
            for w in 0..n {
                let angle = if n < 2 {
                    facing
                } else {
                    let (to, from) = if command.flags & 1 == 0 {
                        if w == n - 1 {
                            (usize::MAX, usize::MAX)
                        } else {
                            (w + 1, w)
                        }
                    } else if w == 0 {
                        (n - 1, 0)
                    } else {
                        (w, w - 1)
                    };
                    if to == usize::MAX {
                        facing
                    } else {
                        arctangent(wp[to].0 - wp[from].0, wp[to].1 - wp[from].1)
                    }
                };
                match slots.len() {
                    0 => {}
                    1 => {
                        places[w][0] = wp[w];
                        facings[0] = angle;
                    }
                    count => {
                        let (p, f) = formation_places(command.formation, wp[w], angle, count)?;
                        places[w].copy_from_slice(&p);
                        facings.copy_from_slice(&f);
                    }
                }
            }
        }
        let max_x = self.data.mesh.cell_width as i32 * 0x400 - 1;
        let max_y = self.data.mesh.cell_height as i32 * 0x400 - 1;
        for row in &mut places {
            for p in row.iter_mut() {
                p.0 = p.0.max(0).min(max_x);
                p.1 = p.1.max(0).min(max_y);
            }
        }
        Ok((places, facings))
    }

    pub(crate) fn execute_movement_command(
        &mut self,
        sender: i16,
        command: &MovementCommand,
        slots: &[usize],
        waypoints: &[(i32, i32)],
    ) -> Result<(), Stop> {
        for &s in slots {
            self.monsters[s].commander = sender;
        }
        if slots.is_empty() {
            return Ok(());
        }
        let mut points = waypoints.to_vec();
        if command.flags & 0x40 != 0 {
            let (centre, _) = self.monsters_centre_spread(slots);
            if points.len() == 4 {
                points.truncate(3);
            }
            points.push(centre);
        }
        if points.is_empty() && command.flags & 8 != 0 {
            return Err(Stop::new(
                draw_site::FORMATION_WITHOUT_WAYPOINTS,
                "a formation command without waypoints",
            ));
        }
        let (places, facings) = self.command_places(command, slots, &points)?;
        let wp_count = points.len();
        let n = slots.len();
        let mut f = CommandFrame::new(waypoints, n)?;
        if command.flags & 8 == 0 {
            for k in 0..n {
                for (w, row) in places.iter().enumerate() {
                    f.put_place(w, k, row[k])?;
                }
                f.put16(FRAME_FACINGS + 2 * k, facings[k] as i16)?;
            }
            for (k, &s) in slots.iter().enumerate() {
                f.put16(FRAME_SLOTS + 2 * k, s as i16)?;
            }
        } else {
            for (w, row) in places.iter().enumerate() {
                for (k, &p) in row.iter().enumerate() {
                    f.put_place(w, k, p)?;
                }
            }
            for (k, &a) in facings.iter().enumerate() {
                f.put16(FRAME_FACINGS + 2 * k, a as i16)?;
            }
            let reversed = formation_reversed(command.formation, n);
            let (centre, _) = self.monsters_centre_spread(slots);
            for k in 0..n {
                let p = f.place(0, k as i32)?;
                let d = (p.1 - centre.1)
                    .wrapping_mul(p.1 - centre.1)
                    .wrapping_add((p.0 - centre.0).wrapping_mul(p.0 - centre.0));
                f.put16(FRAME_ORDER + 2 * k, k as i16)?;
                f.put32(
                    FRAME_KEYS + 4 * k,
                    if reversed { d.wrapping_neg() } else { d },
                )?;
            }
            let keys: Vec<i32> = (0..n)
                .map(|k| f.get32(FRAME_KEYS + 4 * k))
                .collect::<Result<_, _>>()?;
            let mut order: Vec<i16> = (0..n)
                .map(|k| f.get16(FRAME_ORDER + 2 * k))
                .collect::<Result<_, _>>()?;
            heapsort(&mut order, |a, b| {
                key_of(&keys, *b).wrapping_sub(key_of(&keys, *a))
            });
            for (k, &o) in order.iter().enumerate() {
                f.put16(FRAME_ORDER + 2 * k, o)?;
            }
            let mut taken = 0u32;
            for j in 0..n {
                let place = i32::from(f.get16(FRAME_ORDER + 2 * j)?);
                let p = f.place(0, place)?;
                let mut best: Option<(usize, i32)> = None;
                for (k, &s) in slots.iter().enumerate() {
                    let o = &self.objects[self.monsters[s].object];
                    let (dx, dy) = (p.0 - o.x, p.1 - o.y);
                    let d = dy.wrapping_mul(dy).wrapping_add(dx.wrapping_mul(dx));
                    if taken & (1 << (k & 0x1f)) == 0 && best.is_none_or(|(_, b)| d < b) {
                        best = Some((k, d));
                    }
                }
                let Some((k, _)) = best else {
                    return Err(Stop::new(
                        draw_site::MOVEMENT_COMMAND_FRAME,
                        "a formation command: no monster left for a place",
                    ));
                };
                let at = usize::try_from(place)
                    .ok()
                    .map(|p| FRAME_SLOTS + 2 * p)
                    .ok_or_else(frame_stop)?;
                f.put16(at, slots[k] as i16)?;
                taken |= 1 << (k & 0x1f);
            }
        }
        for k in 0..n {
            let points: Vec<(i32, i32)> = (0..wp_count)
                .map(|w| f.place(w, k as i32))
                .collect::<Result<_, _>>()?;
            let s = f.get16(FRAME_SLOTS + 2 * k)?;
            let facing = f.get16(FRAME_FACINGS + 2 * k)? as u16;
            let s = usize::try_from(s)
                .ok()
                .filter(|&s| self.monsters.get(s).is_some_and(|m| m.identifier != 0))
                .ok_or_else(frame_stop)?;
            self.set_monster_waypoints(s, &points, command.flags as u8, facing);
        }
        Ok(())
    }

    fn set_monster_waypoints(&mut self, s: usize, points: &[(i32, i32)], flags: u8, facing: u16) {
        let object = self.monsters[s].object;
        self.objects[object].flags &= 0xff7f_ffff;
        let m = &mut self.monsters[s];
        let old = m.flags;
        m.flags = if flags & 4 == 0 {
            old & 0x7f07_a162
        } else {
            old & 0x7f27_a17e
        };
        m.flags |= 0x1022;
        m.final_facing = facing;
        m.waypoint_index = 0;
        m.waypoint_count = points.len() as i8;
        m.field_70 = -1;
        for (w, p) in points.iter().enumerate() {
            m.waypoints[w] = *p;
        }
        if flags & 1 != 0 && points.len() > 1 {
            m.flags |= 0x200;
            if flags & 2 != 0 {
                m.flags |= 0x600;
            }
        }
    }
}

impl TflSim {
    pub(crate) fn execute_monster_commands(&mut self) -> Result<(), Stop> {
        let t = self.tick;
        if t & 0x1ff == 0 && t != 0 {
            self.map_actions.game.sync_sample = Some((t, self.rng.state()));
        }
        let groups = &self.data.film.tick_commands;
        if groups.is_empty() {
            return Ok(());
        }
        if self.command_cursor >= groups.len() || groups[self.command_cursor].tick > t {
            self.command_cursor = 0;
        }
        while self.command_cursor < groups.len() && groups[self.command_cursor].tick < t {
            self.command_cursor += 1;
        }
        let idx = if self.command_cursor < groups.len() && groups[self.command_cursor].tick == t {
            self.command_cursor
        } else {
            return Ok(());
        };
        let packets = std::mem::take(&mut self.data.film.tick_commands[idx].commands);
        let players = self.data.film.header.players.len();
        let mut result = Ok(());
        for c in &packets {
            let player = c.player_index as i8;
            if !(player == -1 || (player >= 0 && (player as usize) < players)) {
                continue;
            }
            let ty = c.command_type.to_u8();
            if (2..=6).contains(&ty) && self.game_state().0 == 0 {
                continue;
            }
            let r = match ty {
                1 => self.film_sync_command(&c.raw_payload),
                2 => self.film_general_command(player, &c.raw_payload),
                3 => self.film_movement_command(player, &c.raw_payload),
                4 => self.film_target_command(player, &c.raw_payload),
                5 => self.film_attack_location_command(player, &c.raw_payload),
                6 => self.film_pick_up_command(player, &c.raw_payload),
                7 => self.film_rename_command(player, &c.raw_payload),
                8 => self.drop_player(player),
                9 => {
                    self.chat_command(player, &c.raw_payload);
                    Ok(())
                }
                10 => self.film_detach_command(player, &c.raw_payload),
                11 => self.unit_adjustment_command(player, &c.raw_payload),
                12 => self.sketch_command(player, &c.raw_payload),
                13 => self.save_loaded_command(player, &c.raw_payload),
                _ => Ok(()),
            };
            if let Err(e) = r {
                result = Err(e);
                break;
            }
        }
        self.data.film.tick_commands[idx].commands = packets;
        result
    }

    fn film_sync_command(&mut self, p: &[u8]) -> Result<(), Stop> {
        let (Some(state), Some(tick)) = (crate::be::u32_at(p, 4), crate::be::u32_at(p, 8)) else {
            return Err(Stop::new(
                draw_site::SYNC_COMMAND_SHORT,
                "a short sync command",
            ));
        };
        let (game_tick, game_state) = self
            .map_actions
            .game
            .sync_sample
            .unwrap_or((0, self.data.film.header.initial_seed));
        if tick == game_tick && state != game_state {
            return Err(Stop::new(
                draw_site::OUT_OF_SYNC,
                format!(
                    "tick {}: out of sync: the film's sample of tick {tick} is {state:08x}, ours {game_state:08x}",
                    self.tick
                ),
            ));
        }
        Ok(())
    }

    fn film_movement_command(&mut self, player: i8, p: &[u8]) -> Result<(), Stop> {
        let bad = || Stop::new(draw_site::MOVEMENT_COMMAND, "a short movement command");
        let command = MovementCommand {
            flags: crate::be::u16_at(p, 0).ok_or_else(bad)?,
            formation: crate::be::u16_at(p, 2).ok_or_else(bad)?,
            facing: crate::be::u16_at(p, 4).ok_or_else(bad)?,
        };
        let n = crate::be::i16_at(p, 8).ok_or_else(bad)?.max(0) as usize;
        let ids: Vec<i16> = (0..n)
            .map(|k| crate::be::i16_at(p, 10 + 2 * k).ok_or_else(bad))
            .collect::<Result<_, _>>()?;
        let at = 10 + 2 * n;
        let w = crate::be::i16_at(p, at).ok_or_else(bad)?.max(0) as usize;
        let points: Vec<(i32, i32)> = (0..w)
            .map(|k| {
                Ok((
                    crate::be::i32_at(p, at + 2 + 8 * k).ok_or_else(bad)?,
                    crate::be::i32_at(p, at + 6 + 8 * k).ok_or_else(bad)?,
                ))
            })
            .collect::<Result<_, Stop>>()?;
        let team = self.player_team(player)?;
        let mut slots = Vec::new();
        for s in self.monster_slots(&ids) {
            let m = &mut self.monsters[s];
            if m.flags & 0x1000_0000 == 0 && (m.player == player || Some(m.team) == team) {
                m.flags |= 0x800_0000;
                slots.push(s);
            }
            m.commander = -1;
        }
        self.execute_movement_command(-1, &command, &slots, &points)
    }

    fn player_monsters(&mut self, player: i8, slots: Vec<usize>) -> Result<Vec<usize>, Stop> {
        let team = self.player_team(player)?;
        let mut out = Vec::new();
        for s in slots {
            let m = &mut self.monsters[s];
            if m.flags & 0x1000_0000 == 0 && (m.player == player || Some(m.team) == team) {
                m.flags |= 0x800_0000;
                out.push(s);
            }
            m.commander = -1;
        }
        Ok(out)
    }

    fn film_target_command(&mut self, player: i8, p: &[u8]) -> Result<(), Stop> {
        let bad = || Stop::new(draw_site::TARGET_COMMAND, "a short target command");
        let command = crate::attack::TargetCommand {
            flags: crate::be::u16_at(p, 0).ok_or_else(bad)?,
            field_2: 0,
        };
        let n = crate::be::i16_at(p, 2).ok_or_else(bad)?.max(0) as usize;
        let ids: Vec<i16> = (0..n)
            .map(|k| crate::be::i16_at(p, 4 + 2 * k).ok_or_else(bad))
            .collect::<Result<_, _>>()?;
        let at = 4 + 2 * n;
        let m = crate::be::i16_at(p, at).ok_or_else(bad)?.max(0) as usize;
        let tids: Vec<i16> = (0..m)
            .map(|k| crate::be::i16_at(p, at + 2 + 2 * k).ok_or_else(bad))
            .collect::<Result<_, _>>()?;
        let slots = self.monster_slots(&ids);
        let attackers = self.player_monsters(player, slots)?;
        let targets = self.monster_slots(&tids);
        self.target_command(-1, command, &attackers, &targets)
    }

    fn film_attack_location_command(&mut self, player: i8, p: &[u8]) -> Result<(), Stop> {
        let bad = || {
            Stop::new(
                draw_site::ATTACK_LOCATION_COMMAND_SHORT,
                "a short attack location command",
            )
        };
        let flags = crate::be::u16_at(p, 0).ok_or_else(bad)?;
        let point = (
            crate::be::i32_at(p, 4).ok_or_else(bad)?,
            crate::be::i32_at(p, 8).ok_or_else(bad)?,
            crate::be::i32_at(p, 12).ok_or_else(bad)? as i16,
        );
        let n = crate::be::i16_at(p, 16).ok_or_else(bad)?.max(0) as usize;
        let ids: Vec<i16> = (0..n)
            .map(|k| crate::be::i16_at(p, 18 + 2 * k).ok_or_else(bad))
            .collect::<Result<_, _>>()?;
        let slots = self.monster_slots(&ids);
        for s in self.player_monsters(player, slots)? {
            self.attack_location(s, flags, point);
        }
        Ok(())
    }

    pub(crate) fn attack_location_command(
        &mut self,
        sender: i16,
        flags: u16,
        point: (i32, i32, i16),
        slots: &[usize],
    ) {
        let ids: Vec<i16> = slots.iter().map(|&s| self.monsters[s].identifier).collect();
        for s in self.monster_slots(&ids) {
            self.monsters[s].commander = sender;
            self.attack_location(s, flags, point);
        }
    }

    fn attack_location(&mut self, s: usize, flags: u16, point: (i32, i32, i16)) {
        let o = self.monsters[s].object;
        self.objects[o].flags &= 0xff7f_ffff;
        let m = &mut self.monsters[s];
        m.attack_point = point;
        let kept = m.flags & 0x7f06_3f40;
        m.flags = kept | 0x8037;
        m.waypoint_count = 0;
        if flags & 1 != 0 {
            m.flags = kept | 0x80_8037;
        }
        crate::tc2_trace::log(|| {
            let m = &self.monsters[s];
            format!(
                "AL {} {s} {} {} {} {:x} {} {} {:x}",
                self.tick,
                point.0,
                point.1,
                point.2,
                m.flags,
                m.waypoint_count,
                m.commander,
                self.objects[o].flags
            )
        });
    }

    pub(crate) fn attack_height(&self, x: i32, y: i32) -> Result<i16, Stop> {
        let mut z = self.mesh_height(x, y)? as i16;
        let cell = self.cell_at(x >> 10, y >> 10);
        if self.cells[cell].flags & 0x18 != 0 && z < 0 {
            z = 0;
        }
        Ok(z)
    }

    pub(crate) fn general_command(&mut self, kind: u16, slots: &[usize]) -> Result<(), Stop> {
        let (keep, set) = match kind {
            0 => (0x7f3f_ffbf, 0x40_0040),
            3 => (0x7f3f_ffbf, 0x80_0040),
            4 => (0xff3f_ffff, 0x8000_0040),
            2 => {
                self.stop_monsters(slots);
                return Ok(());
            }
            1 => {
                self.scatter_monsters(slots);
                return Ok(());
            }
            _ => return Ok(()),
        };
        for &s in slots {
            let m = &mut self.monsters[s];
            m.flags = m.flags & keep | set;
        }
        Ok(())
    }

    fn scatter_monsters(&mut self, slots: &[usize]) {
        let (cx, cy) = self.monsters_centre_spread(slots).0;
        let t = trig();
        let max_x = (i32::from(self.data.mesh.cell_width as i16) << 10) - 0x400;
        let max_y = (i32::from(self.data.mesh.cell_height as i16) << 10) - 0x400;
        for &s in slots {
            let o = self.monsters[s].object;
            let (ox, oy) = (self.objects[o].x, self.objects[o].y);
            let angle = arctangent(ox.wrapping_sub(cx), oy.wrapping_sub(cy));
            let k = index(i32::from(angle));
            let mut x = ((i32::from(t.cosine[k]) << 0xb) >> 0xe).wrapping_add(ox);
            let mut y = ((i32::from(t.sine[k]) << 0xb) >> 0xe).wrapping_add(oy);
            if x < 0 {
                x = 0;
            } else if x > max_x {
                x = max_x;
            }
            if y < 0 {
                y = 0;
            } else if y > max_y {
                y = max_y;
            }
            self.objects[o].flags &= 0xff7f_ffff;
            let m = &mut self.monsters[s];
            m.flags = m.flags & 0x7f07_a140 | 0x1022;
            m.final_facing = angle;
            m.waypoint_index = 0;
            m.waypoint_count = 1;
            m.field_70 = -1;
            m.waypoints[0] = (x, y);
        }
    }

    fn film_pick_up_command(&mut self, player: i8, p: &[u8]) -> Result<(), Stop> {
        let bad = || Stop::new(draw_site::PICK_UP_COMMAND_SHORT, "a short pick-up command");
        let be16 = |o: usize| p.get(o..o + 2).map(|b| i16::from_be_bytes([b[0], b[1]]));
        let object = be16(4).ok_or_else(bad)?;
        let identifier = be16(6).ok_or_else(bad)?;
        let n = be16(8).ok_or_else(bad)?.max(0) as usize;
        let ids: Vec<i16> = (0..n)
            .map(|k| be16(10 + 2 * k).ok_or_else(bad))
            .collect::<Result<_, _>>()?;
        let slots = self.monster_slots(&ids);
        let slots = self.player_monsters(player, slots)?;
        self.pick_up_command(&slots, object, identifier)
    }

    pub(crate) fn map_pick_up_command(
        &mut self,
        sender: i16,
        slots: &[usize],
        object: i16,
        identifier: i16,
    ) -> Result<(), Stop> {
        for &s in slots {
            self.monsters[s].commander = sender;
        }
        self.pick_up_command(slots, object, identifier)
    }

    fn pick_up_command(
        &mut self,
        slots: &[usize],
        object: i16,
        identifier: i16,
    ) -> Result<(), Stop> {
        for &s in slots {
            if self.monster_could_take_object_id(s, object, identifier)? == 0 {
                continue;
            }
            let o = self.monsters[s].object;
            self.objects[o].flags &= 0xff7f_ffff;
            let m = &mut self.monsters[s];
            m.flags = m.flags & 0x7f07_bf40 | 0x4022;
            m.pick_up = (object, identifier);
            m.waypoint_count = 0;
        }
        Ok(())
    }

    fn film_rename_command(&mut self, player: i8, p: &[u8]) -> Result<(), Stop> {
        let bad = || Stop::new(draw_site::RENAME_COMMAND_SHORT, "a short rename command");
        let be16 = |o: usize| p.get(o..o + 2).map(|b| i16::from_be_bytes([b[0], b[1]]));
        let n = be16(2).ok_or_else(bad)?.max(0) as usize;
        let ids: Vec<i16> = (0..n)
            .map(|k| be16(4 + 2 * k).ok_or_else(bad))
            .collect::<Result<_, _>>()?;
        let name = rename_command_name(p.get(4 + 2 * n..).ok_or_else(bad)?);
        let slots = self.monster_slots(&ids);
        for s in self.player_monsters(player, slots)? {
            let m = &mut self.monsters[s];
            m.name = name.clone();
            m.field_13 |= 1;
        }
        Ok(())
    }

    pub fn monster_name(&self, slot: usize) -> Option<Vec<u8>> {
        let m = self.monsters.get(slot)?;
        if !m.name.is_empty() {
            return Some(m.name.clone());
        }
        let list = self.data.peek_def(b"mons", m.definition)?.tag(0x13c)?;
        let d = &self.data.peek_def(b"stli", list)?.0;
        let sep = |b: u8| b == 0 || b == b'\n' || b == b'\r';
        let mut k = 0usize;
        let mut i = 0usize;
        while i < d.len() {
            if sep(d[i]) {
                i += 1;
                continue;
            }
            let end = d[i..]
                .iter()
                .position(|&b| sep(b))
                .map_or(d.len(), |e| i + e);
            if k == usize::from(m.name_index) {
                return Some(d[i..end].to_vec());
            }
            k += 1;
            i = end;
        }
        None
    }

    pub fn team_players(&self, team: i16) -> Vec<i16> {
        if team < 0 {
            return Vec::new();
        }
        self.team_claims().map_or(Vec::new(), |t| {
            (0..t.player_teams.len())
                .filter(|&p| t.player_teams[p] == team)
                .map(|p| p as i16)
                .collect()
        })
    }

    fn film_detach_command(&mut self, player: i8, p: &[u8]) -> Result<(), Stop> {
        let bad = || Stop::new(draw_site::DETACH_COMMAND_SHORT, "a short detach command");
        let be16 = |o: usize| p.get(o..o + 2).map(|b| i16::from_be_bytes([b[0], b[1]]));
        let to = be16(2).ok_or_else(bad)?;
        let n = be16(4).ok_or_else(bad)?.max(0) as usize;
        let ids: Vec<i16> = (0..n)
            .map(|k| be16(6 + 2 * k).ok_or_else(bad))
            .collect::<Result<_, _>>()?;
        let slots = self.monster_slots(&ids);
        for s in self.player_monsters(player, slots)? {
            self.detach_monster(s, to)?;
        }
        Ok(())
    }

    fn stop_monsters(&mut self, slots: &[usize]) {
        for &s in slots {
            let (found, point, facing) = self.retreat(s, None);
            if !found {
                continue;
            }
            let o = self.monsters[s].object;
            self.objects[o].flags &= 0xff7f_ffff;
            let m = &mut self.monsters[s];
            m.flags = m.flags & 0x7f07_a140 | 0x1022;
            m.waypoint_index = 0;
            m.waypoint_count = 1;
            m.final_facing = facing;
            m.field_70 = -1;
            m.waypoints[0] = point;
        }
    }

    fn film_general_command(&mut self, player: i8, p: &[u8]) -> Result<(), Stop> {
        let bad = || Stop::new(draw_site::GENERAL_COMMAND_SHORT, "a short general command");
        let be16 = |o: usize| p.get(o..o + 2).map(|b| i16::from_be_bytes([b[0], b[1]]));
        let kind = be16(2).ok_or_else(bad)? as u16;
        let n = be16(4).ok_or_else(bad)?.max(0) as usize;
        let ids: Vec<i16> = (0..n)
            .map(|k| be16(6 + 2 * k).ok_or_else(bad))
            .collect::<Result<_, _>>()?;
        let slots = self.monster_slots(&ids);
        let slots = self.player_monsters(player, slots)?;
        self.general_command(kind, &slots)
    }

    pub(crate) fn player_team(&self, player: i8) -> Result<Option<i16>, Stop> {
        if player < 0 {
            return Ok(None);
        }
        Ok(team_of(&self.team_claims()?, player))
    }
}

pub(crate) fn team_of(teams: &crate::setup::TeamClaims, player: i8) -> Option<i16> {
    if player < 0 {
        return None;
    }
    let p = player as usize;
    teams
        .player_teams
        .get(p)
        .copied()
        .filter(|&t| t >= 0 && teams.claims.get(t as usize) == Some(&Some(p)))
}

pub(crate) fn rename_command_name(p: &[u8]) -> Vec<u8> {
    let end = p.iter().position(|&b| b == 0).unwrap_or(p.len()).min(0x11);
    p[..end].to_vec()
}

#[cfg(test)]
mod tests {
    use super::{heapsort, rename_command_name};

    #[test]
    fn the_rename_name_is_cut_at_its_nul_or_17_bytes() {
        assert_eq!(rename_command_name(b"Ulf\0junk"), b"Ulf");
        assert_eq!(rename_command_name(b""), b"");
        assert_eq!(rename_command_name(&[b'a'; 20]), [b'a'; 17]);
    }

    #[test]
    fn heapsort_sorts() {
        let mut v = [5, 3, 9, 1, 1, 7, 2];
        heapsort(&mut v, |a, b| a - b);
        assert_eq!(v, [1, 1, 2, 3, 5, 7, 9]);
    }
}
