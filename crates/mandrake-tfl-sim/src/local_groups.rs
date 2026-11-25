use mandrake_common::FourCC;

use crate::TflSim;
use crate::draw_site;

pub const GROUP_SITE: u32 = draw_site::LOCAL_PROJECTILE_GROUP;
pub const PROJECTILE_SITE: u32 = draw_site::LOCAL_PROJECTILE;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LocalGroupObject {
    pub slot: usize,
    pub identifier: i16,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LocalGroupTarget {
    pub object: LocalGroupObject,
    pub point: [i32; 3],
    pub radius: i16,
    pub bottom: i16,
    pub top: i16,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LocalGroupSource {
    ProjectileGroup(FourCC),
    Projectile { definition: FourCC, slot: usize },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LocalGroupStart {
    pub tick: u32,
    pub marks_before: u32,
    pub site: u32,
    pub source: LocalGroupSource,
    pub definition: FourCC,
    pub point: [i32; 3],
    pub central: Option<LocalGroupObject>,
    pub target: Option<LocalGroupTarget>,
    pub opacity: u16,
    pub facing: u16,
}

impl TflSim {
    pub fn take_local_group_starts(&mut self) -> Vec<LocalGroupStart> {
        std::mem::take(&mut self.local_group_starts)
    }

    fn local_group_at(&mut self, def: &crate::data::Def, offset: usize) -> Option<FourCC> {
        def.tag(offset)
            .filter(|&t| self.data.def(b"lpgr", t).is_ok())
    }

    pub(crate) fn start_group_local_group(
        &mut self,
        group: FourCC,
        prgr: &crate::data::Def,
        object: usize,
    ) {
        let Some(definition) = self.local_group_at(prgr, 0x10) else {
            return;
        };
        let ob = &self.objects[object];
        let point = [
            ob.x,
            ob.y,
            ob.z + (i32::from(ob.bottom) + i32::from(ob.top)) / 2,
        ];
        let target = (ob.identifier != 0).then_some(LocalGroupTarget {
            object: LocalGroupObject {
                slot: object,
                identifier: ob.identifier,
            },
            point: [ob.x, ob.y, ob.z],
            radius: ob.radius,
            bottom: ob.bottom,
            top: ob.top,
        });
        let facing = ob.facing;
        self.local_group_starts.push(LocalGroupStart {
            tick: self.tick,
            marks_before: self.mark_count,
            site: GROUP_SITE,
            source: LocalGroupSource::ProjectileGroup(group),
            definition,
            point,
            central: None,
            target,
            opacity: 0xffff,
            facing,
        });
    }

    pub(crate) fn start_projectile_local_group(
        &mut self,
        def_id: FourCC,
        proj: &crate::data::Def,
        slot: usize,
        object: usize,
    ) {
        let Some(definition) = self.local_group_at(proj, 0x8c) else {
            return;
        };
        let ob = &self.objects[object];
        let start = LocalGroupStart {
            tick: self.tick,
            marks_before: self.mark_count,
            site: PROJECTILE_SITE,
            source: LocalGroupSource::Projectile {
                definition: def_id,
                slot,
            },
            definition,
            point: [ob.x, ob.y, ob.z],
            central: Some(LocalGroupObject {
                slot: object,
                identifier: ob.identifier,
            }),
            target: None,
            opacity: 0xffff,
            facing: ob.facing,
        };
        self.local_group_starts.push(start);
    }
}
