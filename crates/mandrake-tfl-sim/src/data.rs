use std::collections::HashMap;
use std::hash::{BuildHasherDefault, Hasher};
use std::path::Path;
use std::sync::Arc;

use mandrake_common::FourCC;
use mandrake_rec::tfl::TflRecording;
use mandrake_tag::tfl::TflMesh;
use mandrake_tag::{SequenceHeader, TagArchive, sequence_headers};

use crate::Stop;

#[derive(Default)]
pub(crate) struct FxHasher(u64);

impl FxHasher {
    const K: u64 = 0x517c_c1b7_2722_0a95;

    #[inline]
    fn add(&mut self, v: u64) {
        self.0 = (self.0.rotate_left(5) ^ v).wrapping_mul(Self::K);
    }
}

impl Hasher for FxHasher {
    #[inline]
    fn finish(&self) -> u64 {
        self.0
    }

    #[inline]
    fn write(&mut self, mut bytes: &[u8]) {
        while bytes.len() >= 8 {
            let (chunk, rest) = bytes.split_at(8);
            self.add(u64::from_ne_bytes(chunk.try_into().unwrap()));
            bytes = rest;
        }
        if bytes.len() >= 4 {
            let (chunk, rest) = bytes.split_at(4);
            self.add(u64::from(u32::from_ne_bytes(chunk.try_into().unwrap())));
            bytes = rest;
        }
        for &b in bytes {
            self.add(u64::from(b));
        }
    }

    #[inline]
    fn write_u16(&mut self, i: u16) {
        self.add(u64::from(i));
    }

    #[inline]
    fn write_i16(&mut self, i: i16) {
        self.add(u64::from(i as u16));
    }

    #[inline]
    fn write_u32(&mut self, i: u32) {
        self.add(u64::from(i));
    }

    #[inline]
    fn write_i32(&mut self, i: i32) {
        self.add(u64::from(i as u32));
    }

    #[inline]
    fn write_usize(&mut self, i: usize) {
        self.add(i as u64);
    }
}

pub(crate) type FxMap<K, V> = HashMap<K, V, BuildHasherDefault<FxHasher>>;

#[derive(Debug, Clone)]
pub struct Def(pub Vec<u8>);

impl Def {
    pub fn i16(&self, o: usize) -> i16 {
        self.0
            .get(o..o + 2)
            .map_or(0, |b| i16::from_be_bytes([b[0], b[1]]))
    }
    pub fn u16(&self, o: usize) -> u16 {
        self.i16(o) as u16
    }
    pub fn u32(&self, o: usize) -> u32 {
        self.0
            .get(o..o + 4)
            .map_or(0, |b| u32::from_be_bytes([b[0], b[1], b[2], b[3]]))
    }
    pub fn closed_terrain(&self) -> u16 {
        (0..16).fold(0u16, |m, k| {
            if (self.0.get(0x48 + k).copied().unwrap_or(0) as i8) < 0 {
                m | 1 << k
            } else {
                m
            }
        })
    }
    pub fn tag(&self, o: usize) -> Option<FourCC> {
        let v = self.u32(o);
        (v != 0xffff_ffff && v != 0).then(|| FourCC(v.to_be_bytes()))
    }
}

#[derive(Debug, Clone)]
pub struct Core {
    pub collection: FourCC,
    pub permutation_count: i16,
    pub permutations: Vec<i16>,
}

pub struct TflData {
    pub archive: TagArchive,
    pub film: TflRecording,
    pub mesh: TflMesh,
    pub mesh_bytes: Vec<u8>,
    defs: FxMap<(FourCC, FourCC), Arc<Def>>,
    closed_terrains: FxMap<FourCC, u16>,
    sequences: FxMap<FourCC, Vec<SequenceHeader>>,
    sequence_sounds: FxMap<FourCC, Vec<[Option<FourCC>; 3]>>,
    key_points: FxMap<FourCC, Option<Vec<Vec<[i16; 3]>>>>,
}

fn load_err(what: String) -> Stop {
    Stop::new(0, format!("data: {what}"))
}

impl TflData {
    pub fn load(game_dir: &Path, film: &[u8]) -> Result<Self, Stop> {
        let archive = mandrake_tag::tag_files::open_tfl_tags(game_dir)
            .map_err(|e| load_err(format!("tags in {}: {e}", game_dir.display())))?;
        let film = TflRecording::from_bytes(film).map_err(|e| load_err(format!("film: {e}")))?;
        let mesh_id = film.header.mesh_tag_id;
        let entry = archive
            .find_tag_by_id(FourCC(*b"mesh"), mesh_id)
            .ok_or_else(|| load_err(format!("mesh {mesh_id} not found")))?;
        let bytes = archive
            .get_tag_data(entry)
            .map_err(|e| load_err(format!("mesh {mesh_id}: {e}")))?;
        let mesh = TflMesh::from_bytes(&bytes).map_err(|e| load_err(format!("mesh: {e}")))?;
        Ok(Self {
            archive,
            film,
            mesh,
            mesh_bytes: bytes,
            defs: FxMap::default(),
            closed_terrains: FxMap::default(),
            sequences: FxMap::default(),
            sequence_sounds: FxMap::default(),
            key_points: FxMap::default(),
        })
    }

    pub fn peek_def(&self, group: &[u8; 4], id: FourCC) -> Option<&Def> {
        self.defs.get(&(FourCC(*group), id)).map(Arc::as_ref)
    }

    pub fn read_def(&self, group: &[u8; 4], id: FourCC) -> Option<Def> {
        let e = self.archive.find_tag_by_id(FourCC(*group), id)?;
        self.archive.get_tag_data(e).ok().map(Def)
    }

    fn load_def(&mut self, group: &[u8; 4], id: FourCC) -> Result<(), Stop> {
        let key = (FourCC(*group), id);
        let e = self
            .archive
            .find_tag_by_id(key.0, id)
            .ok_or_else(|| load_err(format!("{} {id} not found", key.0)))?;
        let d = self
            .archive
            .get_tag_data(e)
            .map_err(|e| load_err(format!("{} {id}: {e}", key.0)))?;
        let def = Def(d);
        if group == b"mons" {
            self.closed_terrains.insert(id, def.closed_terrain());
        }
        self.defs.insert(key, Arc::new(def));
        if group == b"mons" {
            self.equalize_monster_sequences(id)?;
        }
        Ok(())
    }

    pub fn def(&mut self, group: &[u8; 4], id: FourCC) -> Result<&Def, Stop> {
        let key = (FourCC(*group), id);
        if !self.defs.contains_key(&key) {
            self.load_def(group, id)?;
        }
        Ok(self.defs[&key].as_ref())
    }

    #[inline]
    pub(crate) fn def_arc(&mut self, group: &[u8; 4], id: FourCC) -> Result<Arc<Def>, Stop> {
        let key = (FourCC(*group), id);
        if let Some(d) = self.defs.get(&key) {
            return Ok(Arc::clone(d));
        }
        self.load_def(group, id)?;
        Ok(Arc::clone(&self.defs[&key]))
    }

    #[inline]
    pub(crate) fn mons_closed_terrain(&mut self, id: FourCC) -> Result<u16, Stop> {
        if let Some(&mask) = self.closed_terrains.get(&id) {
            return Ok(mask);
        }
        self.load_def(b"mons", id)?;
        Ok(self.closed_terrains[&id])
    }

    pub(crate) fn set_def_tag(
        &mut self,
        group: &[u8; 4],
        id: FourCC,
        o: usize,
        tag: Option<FourCC>,
    ) {
        if let Some(d) = self.defs.get_mut(&(FourCC(*group), id))
            && let Some(b) = Arc::make_mut(d).0.get_mut(o..o + 4)
        {
            b.copy_from_slice(&tag.map_or([0xff; 4], |t| t.0));
        }
    }

    fn equalize_monster_sequences(&mut self, id: FourCC) -> Result<(), Stop> {
        let mons = Arc::clone(&self.defs[&(FourCC(*b"mons"), id)]);
        let Some(collection) = mons.tag(4) else {
            return Ok(());
        };
        let from = mons.i16(0x08);
        let mut to = Vec::new();
        for k in 0..mons.i16(0x94).max(0) as usize {
            let a = 0x9c + 0x20 * k;
            to.extend([mons.i16(a + 8), mons.i16(a + 0x14), mons.i16(a + 0x16)]);
        }
        to.extend((1..16).map(|k| mons.i16(0x08 + 2 * k)));
        self.ensure_sequences(collection)?;
        let list = self.sequences.get_mut(&collection).expect("loaded");
        let len = list.len();
        let pick = |s: i16| {
            if s >= 0 && (s as usize) < len {
                s as usize
            } else {
                0
            }
        };
        let bounds = list.get(pick(from)).map(|h| h.world_bounds);
        if let Some(b) = bounds.filter(|_| from != -1) {
            for t in to.into_iter().filter(|&t| t != -1) {
                if let Some(h) = list.get_mut(pick(t)) {
                    h.world_bounds = b;
                }
            }
        }
        Ok(())
    }

    pub fn string_count(&mut self, id: FourCC) -> Result<i16, Stop> {
        let d = &self.def(b"stli", id)?.0;
        let mut count = 0i16;
        let mut start = true;
        for &b in d {
            let sep = b == 0 || b == b'\n' || b == b'\r';
            if !sep && start {
                count = count.wrapping_add(1);
            }
            start = sep;
        }
        Ok(count)
    }

    pub fn core(&mut self, id: FourCC) -> Result<Core, Stop> {
        let d = self.def(b"core", id)?;
        let count = d.i16(4);
        Ok(Core {
            collection: d
                .tag(0)
                .ok_or_else(|| load_err(format!("core {id}: no collection")))?,
            permutation_count: count,
            permutations: (0..count.max(0) as usize)
                .map(|i| d.i16(0x176 + 2 * i))
                .collect(),
        })
    }

    fn ensure_sequences(&mut self, id: FourCC) -> Result<(), Stop> {
        if self.sequences.contains_key(&id) {
            return Ok(());
        }
        let e = self
            .archive
            .find_tag_by_id(FourCC(*b".256"), id)
            .ok_or_else(|| load_err(format!(".256 {id} not found")))?;
        let d = self
            .archive
            .get_tag_data(e)
            .map_err(|e| load_err(format!(".256 {id}: {e}")))?;
        let mut h = sequence_headers(&d).map_err(|e| load_err(format!(".256 {id}: {e}")))?;
        let be = |o: usize| {
            d.get(o..o + 4).map_or(0, |b| {
                i32::from_be_bytes([b[0], b[1], b[2], b[3]]).max(0) as usize
            })
        };
        let refs = be(0x84);
        let tag = |o: usize| {
            d.get(o..o + 4)
                .map(|b| FourCC([b[0], b[1], b[2], b[3]]))
                .filter(|t| t.0 != [0xff; 4] && t.0 != [0; 4])
        };
        let mut sounds = Vec::with_capacity(h.len());
        for (i, seq) in h.iter_mut().enumerate() {
            let at = 320 + be(320 + refs + i * 128 + 64);
            sounds.push([tag(at + 0x18), tag(at + 0x1c), tag(at + 0x20)]);
            let Some(raw) = d.get(at..at + 64) else {
                continue;
            };
            let b = mandrake_tag::SequenceBounds::from_header(raw);
            let f = b.pixels_to_world.wrapping_mul(7);
            let conv = |v: i32| ((((v >> 16) as i16) as i32).wrapping_mul(f) >> 16) as i16;
            seq.world_bounds = mandrake_tag::SequenceWorldBounds {
                half_width: conv(b.half_width),
                bottom: conv(b.bottom),
                top: conv(b.top),
            };
        }
        let kp = collection_key_points(&d);
        self.sequences.insert(id, h);
        self.sequence_sounds.insert(id, sounds);
        self.key_points.insert(id, kp);
        Ok(())
    }

    pub fn sequences(&mut self, id: FourCC) -> Result<&[SequenceHeader], Stop> {
        self.ensure_sequences(id)?;
        Ok(&self.sequences[&id])
    }

    pub fn sequence_sounds(&mut self, id: FourCC) -> Result<&[[Option<FourCC>; 3]], Stop> {
        self.ensure_sequences(id)?;
        Ok(&self.sequence_sounds[&id])
    }

    pub fn key_points(&mut self, id: FourCC) -> Result<&[Vec<[i16; 3]>], Stop> {
        self.ensure_sequences(id)?;
        self.key_points[&id]
            .as_deref()
            .ok_or_else(|| load_err(format!(".256 {id}: key points")))
    }
}

pub fn collection_key_points(c: &[u8]) -> Option<Vec<Vec<[i16; 3]>>> {
    let h32 = |o: usize| {
        c.get(o..o + 4)
            .map(|b| i32::from_be_bytes([b[0], b[1], b[2], b[3]]))
    };
    let h16 = |o: usize| c.get(o..o + 2).map(|b| i16::from_be_bytes([b[0], b[1]]));
    const BASE: usize = 320;
    let bitmaps = BASE + h32(0x64)?.max(0) as usize;
    let instance_count = h32(0x70)?.max(0) as usize;
    let instances = BASE + h32(0x74)?.max(0) as usize;
    let count = h32(0x80)?.max(0) as usize;
    let refs = BASE + h32(0x84)?.max(0) as usize;
    let mut out = Vec::with_capacity(count);
    for i in 0..count {
        let so = BASE + h32(refs + i * 128 + 64)?.max(0) as usize;
        let flags = h32(so)?;
        let f = h32(so + 4)?.wrapping_mul(7);
        let views = h16(so + 8)?;
        let frames = h16(so + 0x0a)?.max(0) as usize;
        let nv = views.max(0) as usize;
        let displayed = mandrake_tag::collection::displayed_views(nv);
        let mut seq = Vec::with_capacity(frames);
        for fr in 0..frames {
            let fo = so + 0x40 + fr * 2 * (nv + 0x17);
            if views <= 0 {
                seq.push([0, 0, 0]);
                continue;
            }
            let instance = |v: usize| -> Option<usize> {
                let k = h16(fo + 0x2e + 2 * v)?;
                let k = if k >= 0 && (k as usize) < instance_count {
                    k as usize
                } else {
                    0
                };
                Some(instances + k * 64)
            };
            let (mut lo, mut hi, mut sum_dy, mut first) = (i32::MAX, i32::MIN, 0i32, 0i32);
            for v in 0..nv {
                let mirrored = v >= displayed && flags & 1 != 0;
                let io = if mirrored {
                    instance(mandrake_tag::collection::mirror_view(nv, v))?
                } else {
                    instance(v)?
                };
                let bitmap = h16(io + 0x1c)?;
                let mut kx = h16(io + 0x18)?;
                if mirrored && bitmap != -1 {
                    let width = h16(bitmaps + bitmap.max(0) as usize * 128 + 0x4c)?;
                    kx = width.wrapping_sub(kx);
                }
                let dx = i32::from(kx) - i32::from(h16(io + 0x0c)?);
                sum_dy += i32::from(h16(io + 0x1a)?) - i32::from(h16(io + 0x0e)?);
                lo = lo.min(dx);
                hi = hi.max(dx);
                if v == 0 {
                    first = dx;
                }
            }
            let sc = |v: i32| v.wrapping_mul(f) >> 16;
            let y = sc(first);
            let w = (sc(hi) - sc(lo)) / 2;
            let x = crate::groups::isqrt(w.wrapping_mul(w).wrapping_sub(y.wrapping_mul(y)) as u32);
            let z = ((sum_dy.wrapping_mul(f) / i32::from(views)) >> 16) as i16;
            seq.push([x as i16, y as i16, z.wrapping_neg()]);
        }
        out.push(seq);
    }
    Some(out)
}
