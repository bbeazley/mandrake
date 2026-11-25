use mandrake_common::FourCC;

use crate::data::{Def, TflData};
use crate::draw_site;
use crate::{Stop, TflSim};

impl TflSim {
    fn object_projectile(&self, o: usize) -> Option<FourCC> {
        let ob = &self.objects[o];
        match ob.kind {
            3 => self
                .projectiles
                .list
                .get(ob.owner as usize)
                .and_then(|p| p.definition),
            4 => ob.sleeping,
            _ => None,
        }
    }

    pub(crate) fn monster_could_take_object_id(
        &mut self,
        m: usize,
        o: i16,
        identifier: i16,
    ) -> Result<u8, Stop> {
        if !self.object_with_identifier(o, identifier) {
            return Ok(0);
        }
        self.monster_could_take_object(m, o as usize)
    }

    pub(crate) fn object_with_identifier(&self, o: i16, identifier: i16) -> bool {
        usize::try_from(o)
            .ok()
            .and_then(|o| self.objects.get(o))
            .is_some_and(|ob| ob.identifier != 0 && ob.identifier == identifier)
    }

    pub(crate) fn within_pick_up_reach(&self, m: usize, o: usize) -> bool {
        let ob = &self.objects[o];
        if ob.flags & 0x10_0000 != 0 {
            return false;
        }
        let mo = &self.objects[self.monsters[m].object];
        crate::groups::distance((ob.x, ob.y), (mo.x, mo.y)) <= 0xcc
    }

    pub(crate) fn monster_could_take_object(&mut self, m: usize, o: usize) -> Result<u8, Stop> {
        let question = self.take_question(m, o);
        take_flags(&question, &mut Loading(&mut self.data))
    }

    pub fn could_take_object(&self, m: usize, o: usize) -> bool {
        let (Some(ob), Some(_)) = (self.objects.get(o), self.monsters.get(m)) else {
            return false;
        };
        if ob.flags & 0x10_0000 != 0 {
            return false;
        }
        let question = self.take_question(m, o);
        let mut defs = Peeking {
            data: &self.data,
            read: Vec::new(),
        };
        take_flags(&question, &mut defs).is_ok_and(|r| r != 0)
    }

    fn take_question(&self, m: usize, o: usize) -> TakeQuestion {
        let (ob, mo) = (&self.objects[o], &self.monsters[m]);
        TakeQuestion {
            identifier: ob.identifier,
            flags: ob.flags,
            render: ob.render_flags & 7,
            projectile: self.object_projectile(o),
            definition: mo.definition,
            ammunition: i16::from(mo.ammunition),
            carrying: mo.carried != -1,
        }
    }

    pub(crate) fn give_object_to_monster(&mut self, m: usize, o: usize) -> Result<u8, Stop> {
        let bits = self.monster_could_take_object(m, o)?;
        if bits == 0 {
            return Ok(0);
        }
        if self.objects[o].kind == 3 {
            let k = self.objects[o].owner as usize;
            self.sleep_projectile(k)?;
        }
        if bits & 8 != 0 {
            let from = self.objects[o].sleeping.ok_or_else(|| {
                Stop::new(
                    draw_site::GIVE_OBJECT_NOT_PROJECTILE,
                    "a promotion of an object that is no projectile",
                )
            })?;
            self.objects[o].sleeping = self.data.def(b"proj", from)?.tag(0x60);
        }
        if bits & 4 != 0 {
            let mo = &mut self.monsters[m];
            mo.ammunition = mo.ammunition.wrapping_add(1);
            mo.field_13 |= 0x80;
            self.delete_object(o)?;
            return Ok(bits);
        }
        let mo = &mut self.monsters[m];
        mo.carried = o as i16;
        mo.field_13 |= 8;
        if self.objects[o].kind == 4 {
            let proj = self.objects[o].sleeping.ok_or_else(|| {
                Stop::new(
                    draw_site::GIVE_OBJECT_NO_DEFINITION,
                    "an object asleep with no projectile definition",
                )
            })?;
            let arti = self.data.def(b"proj", proj)?.tag(0x7c);
            self.monsters[m].artifact = arti;
            if let Some(a) = arti
                && self.objects[o].charges == -1
            {
                let def = self.data.def(b"arti", a)?;
                let (base, range) = (def.i16(0x0a), def.i16(0x0c));
                let extra = if range > 0 {
                    (i32::from(self.rng.draw(draw_site::GIVE_OBJECT_CHARGES)) * i32::from(range))
                        >> 16
                } else {
                    0
                };
                self.objects[o].charges = (base as i8).wrapping_add(extra as i8);
            }
        } else {
            self.monsters[m].artifact = None;
        }
        let ob = &mut self.objects[o];
        ob.flags = (ob.flags & !0x1_0000) | 0x200;
        Ok(bits)
    }

    pub(crate) fn set_magical_invisibility(&mut self, m: usize, on: bool) -> Result<(), Stop> {
        let local = self.local_team()?;
        let mo = &mut self.monsters[m];
        let o = mo.object;
        self.objects[o].flags &= 0xcfff_ffff;
        if on {
            mo.flags_44 |= 0x38;
            let theirs = Some(i16::from(mo.team as i8)) != local;
            self.objects[o].flags |= if theirs { 0x2000_0000 } else { 0x1000_0000 };
        } else {
            mo.flags_44 &= 0xc7;
        }
        Ok(())
    }
}

struct TakeQuestion {
    identifier: i16,
    flags: u32,
    render: u8,
    projectile: Option<FourCC>,
    definition: FourCC,
    ammunition: i16,
    carrying: bool,
}

trait TakeDefs {
    fn def(&mut self, group: &[u8; 4], id: FourCC) -> Result<Option<&Def>, Stop>;
}

struct Loading<'a>(&'a mut TflData);

impl TakeDefs for Loading<'_> {
    fn def(&mut self, group: &[u8; 4], id: FourCC) -> Result<Option<&Def>, Stop> {
        self.0.def(group, id).map(Some)
    }
}

struct Peeking<'a> {
    data: &'a TflData,
    read: Vec<((FourCC, FourCC), Def)>,
}

impl TakeDefs for Peeking<'_> {
    fn def(&mut self, group: &[u8; 4], id: FourCC) -> Result<Option<&Def>, Stop> {
        let data = self.data;
        if let Some(d) = data.peek_def(group, id) {
            return Ok(Some(d));
        }
        let key = (FourCC(*group), id);
        let k = match self.read.iter().position(|(k, _)| *k == key) {
            Some(k) => k,
            None => {
                let Some(d) = data.read_def(group, id) else {
                    return Ok(None);
                };
                self.read.push((key, d));
                self.read.len() - 1
            }
        };
        Ok(Some(&self.read[k].1))
    }
}

fn take_flags(q: &TakeQuestion, defs: &mut impl TakeDefs) -> Result<u8, Stop> {
    if q.identifier == 0 || q.flags & 0x200 != 0 {
        return Ok(0);
    }
    if q.flags & 0x10_0000 != 0 {
        return Ok(3);
    }
    let Some(mut proj) = q.projectile else {
        return Ok(0);
    };
    let Some(mons) = defs.def(b"mons", q.definition)? else {
        return Ok(0);
    };
    let (ammunition, most, class) = (mons.tag(0x17c), mons.i16(0x12a), mons.i16(0x96));
    let mut r = 0u8;
    let mut second = false;
    loop {
        let Some(def) = defs.def(b"proj", proj)? else {
            return Ok(0);
        };
        let (own_class, artifact, promotion) = (def.i16(0x26), def.tag(0x7c), def.tag(0x60));
        if ammunition == Some(proj) && q.render != 1 && q.ammunition < most {
            r |= 5;
            break;
        }
        if !q.carrying {
            if class != -1 && class == own_class {
                r |= 1;
                break;
            }
            if let Some(arti) = artifact
                && let Some(a) = defs.def(b"arti", arti)?
            {
                let only = a.u32(4);
                if only == 0xffff_ffff || only == u32::from_be_bytes(q.definition.0) {
                    r |= 1;
                    break;
                }
            }
        }
        if second {
            break;
        }
        r |= 8;
        second = true;
        match promotion {
            Some(p) => proj = p,
            None => break,
        }
    }
    Ok(if r & 1 != 0 { r } else { 0 })
}
