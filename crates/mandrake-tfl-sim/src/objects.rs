use mandrake_common::FourCC;
use mandrake_tag::{SequenceHeader, get_sequence};

use crate::Stop;
use crate::draw_site;
use crate::rng::TflRng;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TflObject {
    pub identifier: i16,
    pub x: i32,
    pub y: i32,
    pub collection: FourCC,
    pub permutation: i16,
    pub permutation_index: i16,
    pub render_flags: u8,
    pub flags: u32,
    pub sequence: i16,
    pub frame: i8,
    pub tick: i8,
    pub ticks_per_frame: i8,
    pub view: u8,
    pub sub_kind: u8,
    pub sub_count: u8,
    pub sub_limit: u8,
    pub facing: u16,
    pub field_1e: u16,
    pub kind: u8,
    pub owner: i16,
    pub sleeping: Option<FourCC>,
    pub scenery: Option<FourCC>,
    pub next_in_cell: i16,
    pub scale: i16,
    pub radius: i16,
    pub z: i32,
    pub velocity: [i16; 3],
    pub above_ground: i16,
    pub bottom: i16,
    pub top: i16,
    pub carried: i16,
    pub ignore: i16,
    pub interest: u8,
    pub charges: i8,
    pub physics: Option<FourCC>,
    pub cell_flags: u8,
    pub terrain_mask: u16,
}

impl TflObject {
    pub fn new(x: i32, y: i32, collection: FourCC, permutation: i16) -> Self {
        Self {
            identifier: 0,
            x,
            y,
            collection,
            permutation,
            permutation_index: 0,
            render_flags: 0,
            flags: 0x10,
            sequence: -1,
            frame: 0,
            tick: 0,
            ticks_per_frame: 1,
            view: 0,
            sub_kind: 0,
            sub_count: 0,
            sub_limit: 1,
            facing: 0,
            field_1e: 0,
            kind: 1,
            owner: -1,
            sleeping: None,
            scenery: None,
            next_in_cell: -1,
            scale: 0,
            radius: 0,
            z: 0,
            velocity: [0; 3],
            above_ground: 0,
            bottom: 0,
            top: 0,
            carried: -1,
            ignore: -1,
            interest: 0,
            charges: -1,
            physics: None,
            cell_flags: 0,
            terrain_mask: 0,
        }
    }
}

pub const MAX_OBJECTS: usize = 0x1000;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DatumPool {
    pub max: usize,
    pub first_free: usize,
    pub high: usize,
    pub count: i16,
    pub salt: i16,
}

impl DatumPool {
    pub fn new(max: usize) -> Self {
        Self {
            max,
            first_free: 0,
            high: 0,
            count: 0,
            salt: i16::MIN,
        }
    }

    pub fn allocate(&mut self, used: impl Fn(usize) -> bool) -> Option<(usize, i16)> {
        let mut k = self.first_free;
        while k < self.max {
            if !used(k) {
                let id = self.salt;
                self.salt = self.salt.wrapping_add(1);
                if self.salt == 0 {
                    self.salt = i16::MIN;
                }
                self.count = self.count.wrapping_add(1);
                self.first_free = k + 1;
                if self.high <= k {
                    self.high = k + 1;
                }
                return Some((k, id));
            }
            k += 1;
        }
        None
    }

    pub fn free(&mut self, k: usize, used: impl Fn(usize) -> bool) {
        if k < self.first_free {
            self.first_free = k;
        }
        if k + 1 == self.high {
            loop {
                self.high -= 1;
                if self.high < 1 || used(self.high - 1) {
                    break;
                }
            }
        }
        self.count = self.count.wrapping_sub(1);
    }

    pub fn set_identifier(&mut self, k: usize, id: i16, ids: &[i16]) -> Option<i16> {
        self.set_identifier_with(k, id, |j| ids.get(j).copied().unwrap_or(0))
    }

    pub fn set_identifier_with(
        &mut self,
        k: usize,
        id: i16,
        id_at: impl Fn(usize) -> i16,
    ) -> Option<i16> {
        if id == 0 {
            return None;
        }
        let mut id = id;
        for j in 0..self.high {
            let other = id_at(j);
            if j != k && other != 0 && other == id {
                id = self.salt;
            }
        }
        if self.salt <= id {
            self.salt = id.wrapping_add(1);
        }
        Some(id)
    }
}

pub fn set_object_shape(
    obj: &mut TflObject,
    headers: &[SequenceHeader],
    sequence: i16,
    speed: i32,
    flags: u8,
    rng: &mut TflRng,
) -> Result<(), Stop> {
    if sequence == -1 {
        obj.flags |= 0x200;
        return Ok(());
    }
    let h = *get_sequence(headers, i32::from(sequence))
        .ok_or_else(|| Stop::new(draw_site::OBJECT_SHAPE, "a collection without sequences"))?;
    let bad = |what: &str| {
        Stop::new(
            draw_site::OBJECT_SHAPE,
            format!("{what} of sequence {sequence} is 0"),
        )
    };
    if obj.sequence != sequence || flags & 0x20 != 0 {
        obj.sequence = sequence;
        obj.view = if flags & 8 != 0 {
            rng.draw_mod(draw_site::OBJECT_SHAPE_VIEW, i32::from(h.number_of_views))
                .ok_or_else(|| bad("views"))? as u8
        } else {
            0
        };
        let tpf = ((i32::from(h.ticks_per_frame) * speed) as u32 >> 8) as i8;
        obj.ticks_per_frame = tpf.max(1);
        obj.frame = if flags & 0x10 != 0 {
            h.loop_frame as i8
        } else if flags & 1 != 0 {
            rng.draw_mod(draw_site::OBJECT_SHAPE_FRAME, i32::from(h.frames_per_view))
                .ok_or_else(|| bad("frames"))? as i8
        } else {
            0
        };
        obj.tick = if flags & 2 != 0 && h.ticks_per_frame != 0 {
            rng.draw_mod(draw_site::OBJECT_SHAPE_TICK, i32::from(h.ticks_per_frame))
                .ok_or_else(|| bad("ticks"))? as i8
        } else {
            0
        };
        obj.flags &= !0xf;
    }
    let scaled = |v: i16| ((i32::from(v) * i32::from(obj.scale)) as u32 >> 8) as i16;
    obj.radius = scaled(h.world_bounds.half_width);
    obj.bottom = scaled(h.world_bounds.bottom);
    obj.top = scaled(h.world_bounds.top);
    let limit = ((i32::from(h.field_26) * speed) >> 8).max(1);
    obj.sub_kind = h.field_24 as u8;
    obj.sub_limit = limit as u8;
    obj.flags &= !0x200;
    obj.sub_count = if flags & 4 != 0 {
        rng.draw_mod(draw_site::OBJECT_SHAPE_LIMIT, limit)
            .unwrap_or(0) as u8
    } else {
        0
    };
    Ok(())
}
