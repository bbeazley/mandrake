use mandrake_common::FourCC;

use crate::TflSim;
use crate::draw_site;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Stain {
    pub tick: u32,
    pub order: u32,
    pub site: u32,
    pub collection: Option<FourCC>,
    pub sequence: i16,
    pub world: [i32; 3],
    pub random: u16,
}

impl Stain {
    pub fn pixel(&self) -> (i16, i16) {
        ((self.world[0] >> 6) as i16, (self.world[1] >> 6) as i16)
    }
}

impl TflSim {
    pub fn take_stains(&mut self) -> Vec<Stain> {
        std::mem::take(&mut self.stains)
    }

    pub(crate) fn stain_mesh(
        &mut self,
        site: u32,
        collection: Option<FourCC>,
        sequence: i16,
        world: [i32; 3],
    ) {
        let random = self.rng.draw(draw_site::MESH_STAIN);
        let order = self.next_mark();
        self.stains.push(Stain {
            tick: self.tick,
            order,
            site,
            collection,
            sequence: i16::from(sequence as i8),
            world,
            random,
        });
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Blood {
    pub tick: u32,
    pub order: u32,
    pub world: [i32; 3],
}

impl Blood {
    pub fn pixel(&self) -> (i32, i32) {
        (self.world[0] >> 6, self.world[1] >> 6)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Footprint {
    pub tick: u32,
    pub order: u32,
    pub object: usize,
    pub terrain: u8,
    pub world: (i32, i32),
}

impl Footprint {
    pub fn pixel(&self) -> (i32, i32) {
        (self.world.0 >> 6, self.world.1 >> 6)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Mark {
    Blood(Blood),
    Footprint(Footprint),
}

impl Mark {
    pub fn tick(&self) -> u32 {
        match self {
            Mark::Blood(b) => b.tick,
            Mark::Footprint(f) => f.tick,
        }
    }

    pub fn order(&self) -> u32 {
        match self {
            Mark::Blood(b) => b.order,
            Mark::Footprint(f) => f.order,
        }
    }
}

impl TflSim {
    pub fn take_marks(&mut self) -> Vec<Mark> {
        std::mem::take(&mut self.marks)
    }

    fn next_mark(&mut self) -> u32 {
        let n = self.mark_count;
        self.mark_count += 1;
        n
    }

    pub(crate) fn bloody_mesh(&mut self, world: [i32; 3]) {
        let order = self.next_mark();
        self.marks.push(Mark::Blood(Blood {
            tick: self.tick,
            order,
            world,
        }));
    }

    pub(crate) fn footprint(&mut self, object: usize, terrain: u8, world: (i32, i32)) {
        let order = self.next_mark();
        self.marks.push(Mark::Footprint(Footprint {
            tick: self.tick,
            order,
            object,
            terrain,
            world,
        }));
    }
}
