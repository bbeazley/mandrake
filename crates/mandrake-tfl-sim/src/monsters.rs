use mandrake_common::FourCC;

use crate::Stop;
use crate::TflSim;
use crate::draw_site;
use crate::objects::TflObject;
use crate::sounds::SoundKind;

pub(crate) struct ActionTransition {
    pub action: u8,
    pub substate: u8,
    pub seq: i16,
    pub speed: i32,
    pub shape_flags: u8,
    pub interest_site: u32,
    pub interest: u8,
    pub reset_order: bool,
    pub reset_target: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TflMonster {
    pub identifier: i16,
    pub flags: u32,
    pub definition: FourCC,
    pub unit: FourCC,
    pub palette: i16,
    pub object: usize,
    pub team: i16,
    pub action: u8,
    pub substate: u8,
    pub facing: u16,
    pub path: u8,
    pub waypoint_count: i8,
    pub waypoint_index: i8,
    pub waypoints: [(i32, i32); 4],
    pub flags_44: u8,
    pub final_facing: u16,
    pub target: i16,
    pub field_4a: i16,
    pub leg_distance: i16,
    pub leg: (i32, i32),
    pub field_70: i16,
    pub commander: i16,
    pub field_16: u8,
    pub player: i8,
    pub vitality: i16,
    pub max_vitality: i16,
    pub ammunition: i8,
    pub field_13: u8,
    pub attack_ticks: i16,
    pub attack_elevation: u16,
    pub attack_speed: i16,
    pub carried: i16,
    pub experience: i8,
    pub artifact: Option<FourCC>,
    pub name_index: u8,
    pub field_77: u8,
    pub field_02: u8,
    pub attack_point: (i32, i32, i16),
    pub guard: i16,
    pub pick_up: (i16, i16),
    pub name: Vec<u8>,
}

pub const MAX_MONSTERS: usize = 300;

impl TflMonster {
    pub fn new(identifier: i16, definition: FourCC, object: usize, team: i16) -> Self {
        Self {
            identifier,
            flags: 0,
            definition,
            unit: definition,
            palette: -1,
            object,
            team,
            action: 0,
            substate: 0,
            facing: 0,
            path: 0xff,
            waypoint_count: 0,
            waypoint_index: 0,
            waypoints: [(0, 0); 4],
            flags_44: 0,
            final_facing: 0,
            target: -1,
            field_4a: 0,
            leg_distance: 0,
            leg: (0, 0),
            field_70: 0,
            commander: 0,
            field_16: 0,
            player: -1,
            vitality: 0,
            max_vitality: 0,
            ammunition: 0,
            field_13: 0xff,
            attack_ticks: 0,
            attack_elevation: 0,
            attack_speed: 0,
            carried: -1,
            experience: 0,
            artifact: None,
            name_index: 0,
            field_77: 0,
            field_02: 0,
            attack_point: (0, 0, 0),
            guard: 0,
            pick_up: (0, 0),
            name: Vec::new(),
        }
    }
}

impl TflSim {
    pub(crate) fn next_waypoint(&mut self, i: usize) -> Result<(), Stop> {
        let m = &mut self.monsters[i];
        let f = m.flags;
        m.flags = f & !0x10;
        if f & 8 == 0 {
            m.flags = f & !0x11;
        }
        let mut f = m.flags;
        if f & 8 != 0 {
            self.set_monster_action(i, 6, 0)?;
            let m = &mut self.monsters[i];
            m.flags = (m.flags & !0x80) | 1;
            return Ok(());
        }
        if f & 0x800 != 0 {
            m.waypoint_index = m.waypoint_index.wrapping_sub(1);
            if m.waypoint_index < 0 {
                m.waypoint_index = 1;
                m.flags = f & !0x800;
            }
            m.flags = (m.flags & !0x80) | 0x20;
            return Ok(());
        }
        m.waypoint_index = m.waypoint_index.wrapping_add(1);
        if m.waypoint_count <= m.waypoint_index {
            if f & 0x200 == 0 {
                m.waypoint_count = 0;
                let commander = m.commander;
                if f & 2 != 0
                    && commander != -1
                    && let Some(a) =
                        crate::map_actions::index_of(&self.map_actions.actions, commander)
                {
                    self.succeed(a)?;
                }
                if self.monsters[i].flags & 0x1000 != 0 {
                    let m = &mut self.monsters[i];
                    m.facing = m.final_facing;
                    self.set_monster_action(i, 4, 0)?;
                } else {
                    self.stop_moving(i)?;
                }
                self.monsters[i].flags &= 0xffff_effd;
                return Ok(());
            }
            if f & 0x400 == 0 {
                m.waypoint_index = 0;
            } else {
                m.flags = f | 0x800;
                m.waypoint_index = m.waypoint_count.wrapping_sub(2);
            }
            f = m.flags;
        }
        self.monsters[i].flags = (f & !0x80) | 0x20;
        Ok(())
    }

    pub(crate) fn stop_moving(&mut self, i: usize) -> Result<(), Stop> {
        let f = self.monsters[i].flags;
        let mut action: u8;
        if f & 0x100_0000 != 0 {
            let m = &mut self.monsters[i];
            m.flags = f & 0xfeff_fff6;
            m.waypoint_count = 0;
            action = 1;
        } else if self.winning_team == self.monsters[i].team && f & 0x200_0000 == 0 {
            action = 2;
        } else if f & 0x4000 == 0 && f & 1 == 0 {
            let m = &self.monsters[i];
            if m.waypoint_count < 1 || m.waypoint_count <= m.waypoint_index {
                if f & 8 != 0 {
                    return Err(Stop::new(
                        draw_site::STOP_MOVING_WITH_TARGET,
                        format!("monster {i} stops moving with a target"),
                    ));
                }
                let range_7c = self.data.def(b"mons", m.definition)?.i16(0x7c);
                let m = &self.monsters[i];
                let o = &self.objects[m.object];
                let w = self.data.mesh.cell_width as i32;
                let h = self.data.mesh.cell_height as i32;
                let (cx, cy) = crate::targets::window_ahead(o.facing, (o.x, o.y), 2, w, h);
                let (pos, oflags) = ((o.x, o.y), o.flags);
                let mut best: Option<(i16, u32, i32)> = None;
                for row in 0..2 {
                    for col in 0..2 {
                        let mut k = self.cell_heads[((cy + row) * w + cx + col) as usize];
                        while k >= 0 {
                            let ob = &self.objects[k as usize];
                            let next = ob.next_in_cell;
                            if ob.flags & 0x200 == 0
                                && ob.kind == 2
                                && ((oflags ^ ob.flags) as u8 as i8) >= 0
                            {
                                let ally = &self.monsters[ob.owner as usize];
                                let me = &self.monsters[i];
                                if me.team == ally.team
                                    && ally.flags & 4 == 0
                                    && ally.flags & 8 != 0
                                    && ally.flags & 0x8_0000 == 0
                                    && ally.palette == me.palette
                                {
                                    let t = ally.target;
                                    if t != -1 && t != i as i16 {
                                        let tm = &self.monsters[t as usize];
                                        let to = &self.objects[tm.object];
                                        let d = crate::groups::distance((to.x, to.y), pos);
                                        let mut score = 0;
                                        if (d as i32) < i32::from(range_7c) {
                                            score += 1;
                                        }
                                        if tm.target == i as i16 {
                                            score += 1;
                                        }
                                        let better = match best {
                                            None => true,
                                            Some((_, bd, bs)) => {
                                                (bs == score && (d as i32) < bd as i32)
                                                    || bs < score
                                            }
                                        };
                                        if better && tm.flags_44 & 0x20 == 0 {
                                            best = Some((t, d, score));
                                        }
                                    }
                                }
                            }
                            k = next;
                        }
                    }
                }
                action = 1;
                if let Some((t, _, _)) = best
                    && self.try_and_switch_target(i, t as usize, 2)?
                {
                    let m = &mut self.monsters[i];
                    m.flags = (m.flags & !0x80) | 0x20;
                    action = 3;
                }
            } else {
                self.monsters[i].flags = (f & !0x80) | 0x20;
                action = 3;
            }
        } else if f & 0x4000 == 0 && f & 0x2000_0000 != 0 {
            action = 6;
        } else {
            self.monsters[i].flags = (f & !0x80) | 0x20;
            action = 3;
        }
        self.set_monster_action(i, action, 0)
    }

    pub(crate) fn begin_action(
        &mut self,
        index: usize,
        mons: &crate::data::Def,
        t: ActionTransition,
    ) -> Result<(), Stop> {
        let m = &mut self.monsters[index];
        let old = m.flags;
        let reset_order = t.reset_order || old & 0x100_0000 != 0;
        let reset_target = t.reset_target || old & 0x100_0000 != 0;
        m.flags = old & 0xfffd_ffff;
        if reset_order {
            m.flags_44 &= 0xfc;
            m.flags = old & 0xfffd_ff5f;
            m.waypoint_count = 0;
            if m.path != 0xff {
                let p = m.path;
                m.path = 0xff;
                self.paths.delete(p);
            }
        }
        if reset_target {
            let m = &mut self.monsters[index];
            m.flags_44 &= 0xbf;
            m.flags = m.flags & 0xde7e_ffe0 | 0x8000;
            m.target = -1;
        }
        let m = &mut self.monsters[index];
        m.action = t.action;
        m.substate = t.substate;
        let object = m.object;
        let player = i16::from(m.player);
        self.objects[object].velocity = [0; 3];
        let seq = if t.seq != -1 { t.seq } else { mons.i16(0x08) };
        if seq != -1 {
            self.set_shape(object, seq, t.speed, t.shape_flags)?;
        }
        self.set_interest(t.interest_site, object, t.interest, player);
        if mons.u32(0) & 0x4000 != 0 {
            let obj = &mut self.objects[object];
            if t.action == 1 || (2 < t.action && t.action < 5) {
                obj.flags &= !0x400;
            } else {
                obj.flags |= 0x400;
            }
        }
        Ok(())
    }

    pub(crate) fn set_monster_action(
        &mut self,
        index: usize,
        action: u8,
        substate: u8,
    ) -> Result<(), Stop> {
        let mons = self
            .data
            .def_arc(b"mons", self.monsters[index].definition)?;
        let m = &self.monsters[index];
        let mut action = action;
        if m.flags & 0x8000_0000 != 0 && action == 3 {
            action = if m.flags & 8 != 0 { 6 } else { 1 };
        }
        let object = m.object;
        if self.objects[object].flags & 0x80_0000 != 0 {
            if action == 3 {
                return Ok(());
            }
            self.objects[object].flags &= !0x80_0000;
        }
        let (seq, speed, substate, reset_order, reset_target) = match action {
            1 => {
                let r = i32::from(self.rng.draw(draw_site::MONSTER_ACTION_ROLL));
                (mons.i16(0x08), ((r << 7) >> 16) + 0x300, 0x19, true, true)
            }
            2 => {
                let r = i32::from(self.rng.draw(draw_site::MONSTER_ACTION_SECOND_ROLL));
                self.monsters[index].flags |= 0x200_0000;
                (
                    mons.i16(0x1a),
                    ((r << 8) >> 16) + 0x100,
                    substate,
                    true,
                    true,
                )
            }
            3 | 4 => {
                let at = if substate == 0x13 { 0x22 } else { 0x0a };
                (mons.i16(at), 0x100, substate, false, false)
            }
            6 => (mons.i16(0x26), 0x100, substate, false, false),
            a => {
                return Err(Stop::new(
                    draw_site::MONSTER_ACTION_NOT_MODELLED,
                    format!("set_monster_action {a} is not modelled"),
                ));
            }
        };
        let shape_flags = if action == 2 { 0 } else { 3 };
        let interest = if action == 1 { 0 } else { 0x19 };
        self.begin_action(
            index,
            &mons,
            ActionTransition {
                action,
                substate,
                seq,
                speed,
                shape_flags,
                interest_site: crate::tracking::site::SET_MONSTER_ACTION,
                interest,
                reset_order,
                reset_target,
            },
        )
    }

    pub(crate) fn monster_celebrating(&mut self, index: usize) -> Result<(), Stop> {
        let object = self.monsters[index].object;
        if self.objects[object].flags & 4 == 0 {
            return Ok(());
        }
        let m = &mut self.monsters[index];
        m.substate = m.substate.wrapping_sub(1);
        if (m.substate as i8) < 1 {
            return self.stop_moving(index);
        }
        let r = i32::from(self.rng.draw(draw_site::MONSTER_CELEBRATING));
        let seq = self.objects[object].sequence;
        self.set_shape(object, seq, ((r << 8) >> 16) + 0x100, 0x30)
    }

    pub(crate) fn monster_idle(&mut self, index: usize) -> Result<(), Stop> {
        let (object, substate, def) = {
            let m = &self.monsters[index];
            (m.object, m.substate, m.definition)
        };
        let (flags, mut seq) = {
            let o = &self.objects[object];
            (o.flags, o.sequence)
        };
        let mut speed = 0x100;
        let next = match substate {
            0x19 | 0x1b => {
                if flags & 1 == 0 {
                    return Ok(());
                }
                crate::tc2_trace::log(|| {
                    format!("MI {} {index} {} {flags:x}", self.tick, substate)
                });
                let r = self.rng.draw(draw_site::MONSTER_IDLE_CHANCE);
                let mons = self.data.def(b"mons", def)?;
                if r < 0x4000 && mons.i16(0x1e) != -1 {
                    seq = mons.i16(0x1c);
                    if substate == 0x19 { 0x1c } else { 0x1a }
                } else {
                    let r = i32::from(self.rng.draw(draw_site::MONSTER_IDLE));
                    speed = ((r << 7) >> 16) + 0x300;
                    substate
                }
            }
            0x1a => {
                if flags & 4 == 0 {
                    return Ok(());
                }
                seq = self.data.def(b"mons", def)?.i16(0x08);
                0x19
            }
            0x1c => {
                if flags & 4 == 0 {
                    return Ok(());
                }
                seq = self.data.def(b"mons", def)?.i16(0x1e);
                0x1b
            }
            _ => return Ok(()),
        };
        self.monsters[index].substate = next;
        self.set_shape(object, seq, speed, 0x21)
    }

    pub(crate) fn animate_object(&mut self, object: usize) -> Result<(), Stop> {
        let obj: &TflObject = &self.objects[object];
        if obj.flags & 0x200 != 0 {
            return Ok(());
        }
        let collection = obj.collection;
        let h = *mandrake_tag::get_sequence(
            self.data.sequences(collection)?,
            i32::from(self.objects[object].sequence),
        )
        .ok_or_else(|| {
            Stop::new(
                draw_site::ANIMATE_NO_SEQUENCES,
                "a collection without sequences",
            )
        })?;
        let sequence = i32::from(self.objects[object].sequence);
        let slots = {
            let all = self.data.sequence_sounds(collection)?;
            let k = usize::try_from(sequence)
                .ok()
                .filter(|&k| k < all.len())
                .unwrap_or(0);
            all.get(k).copied().unwrap_or([None; 3])
        };
        let mut sounds: [Option<(u32, SoundKind, Option<FourCC>)>; 3] = [None, None, None];
        let obj = &mut self.objects[object];
        let tpf = i32::from(obj.ticks_per_frame);
        let mut frame = i32::from(obj.frame);
        let mut tick = i32::from(obj.tick);
        let mut out = 0u32;
        if tpf >= 1 {
            if frame == 0 && tick == 0 {
                sounds[0] = Some((
                    draw_site::SEQUENCE_FIRST_SOUND,
                    SoundKind::SequenceFirst,
                    slots[0],
                ));
            }
            if tick >= tpf {
                tick = 0;
            }
            tick += 1;
            if tick >= tpf {
                out = 1;
                if obj.flags & 0x80000 == 0 {
                    frame += 1;
                    if i32::from(h.frames_per_view) <= frame {
                        frame = i32::from(h.loop_frame);
                        out = 5;
                    }
                } else {
                    frame -= 1;
                    if frame < 0 {
                        frame = i32::from(h.frames_per_view) - 1;
                        out = 5;
                    }
                }
                if out == 5 {
                    sounds[1] = Some((
                        draw_site::SEQUENCE_LAST_SOUND,
                        SoundKind::SequenceLast,
                        slots[2],
                    ));
                }
                if frame == i32::from(h.key_frame) {
                    out |= 2;
                    sounds[2] = Some((
                        draw_site::SEQUENCE_KEY_SOUND,
                        SoundKind::SequenceKey,
                        slots[1],
                    ));
                }
            }
        }
        obj.frame = frame as i8;
        obj.tick = tick as i8;
        if obj.sub_kind != 0 {
            obj.sub_count = obj.sub_count.wrapping_add(1);
            if (obj.sub_limit as i8) < (obj.sub_count as i8) {
                out |= 8;
                obj.sub_count = 0;
            }
        }
        obj.flags = (obj.flags & !0xf) | out;
        for (site, kind, sound) in sounds.into_iter().flatten() {
            self.object_sound(site, kind, object, sound);
        }
        Ok(())
    }
}
