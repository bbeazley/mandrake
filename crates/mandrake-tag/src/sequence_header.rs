use mandrake_common::M2Error;

const COLLECTION_HEADER_SIZE: usize = 320;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
pub struct SequenceHeader {
    pub flags: u32,
    pub number_of_views: i16,
    pub frames_per_view: i16,
    pub ticks_per_frame: i16,
    pub key_frame: i16,
    pub loop_frame: i16,
    pub field_24: i16,
    pub field_26: i16,
    #[serde(default)]
    pub world_bounds: crate::SequenceWorldBounds,
}

impl SequenceHeader {
    pub fn from_bytes(h: &[u8]) -> Result<Self, M2Error> {
        if h.len() < 0x28 {
            return Err(M2Error::TagParse(format!(
                "sequence header too short: {} bytes",
                h.len()
            )));
        }
        let i16_at = |o: usize| i16::from_be_bytes([h[o], h[o + 1]]);
        Ok(Self {
            flags: u32::from_be_bytes([h[0], h[1], h[2], h[3]]),
            number_of_views: i16_at(0x08),
            frames_per_view: i16_at(0x0a),
            ticks_per_frame: i16_at(0x0c),
            key_frame: i16_at(0x0e),
            loop_frame: i16_at(0x10),
            field_24: i16_at(0x24),
            field_26: i16_at(0x26),
            world_bounds: if h.len() >= 0x34 {
                crate::SequenceBounds::from_header(h).world()
            } else {
                crate::SequenceWorldBounds::default()
            },
        })
    }

    pub fn effective_loop_frame(&self) -> i16 {
        if self.loop_frame < 0 || self.loop_frame >= self.frames_per_view {
            0
        } else {
            self.loop_frame
        }
    }
}

pub fn sequence_headers(collection: &[u8]) -> Result<Vec<SequenceHeader>, M2Error> {
    use crate::parse::{checked_table_range, nonneg_i32};

    let err = || M2Error::TagParse("collection header truncated".into());
    let count = nonneg_i32(collection, 0x80).ok_or_else(err)?;
    let refs = nonneg_i32(collection, 0x84).ok_or_else(err)?;
    let refs_range =
        checked_table_range(COLLECTION_HEADER_SIZE, refs, count, 128, collection.len())
            .ok_or_else(|| {
                M2Error::TagParse("collection sequence references out of range".into())
            })?;
    let mut out = Vec::with_capacity(count);
    for i in 0..count {
        let sref = refs_range.start + i * 128;
        let rel = nonneg_i32(collection, sref + 64)
            .ok_or_else(|| M2Error::TagParse(format!("sequence {i} header offset invalid")))?;
        let at = COLLECTION_HEADER_SIZE
            .checked_add(rel)
            .ok_or_else(|| M2Error::TagParse(format!("sequence {i} header offset overflow")))?;
        let end = at
            .checked_add(64)
            .ok_or_else(|| M2Error::TagParse(format!("sequence {i} header end overflow")))?;
        let h = collection
            .get(at..end)
            .ok_or_else(|| M2Error::TagParse(format!("sequence {i} header out of range")))?;
        out.push(SequenceHeader::from_bytes(h)?);
    }
    Ok(out)
}

pub(crate) fn sequence_summary(collection: &[u8], sequence: usize) -> Option<(String, u16, i16)> {
    use crate::parse::{be_i16, checked_table_range, nonneg_i32};

    let count = nonneg_i32(collection, 0x80)?;
    if sequence >= count {
        return None;
    }
    let refs = nonneg_i32(collection, 0x84)?;
    let refs_range =
        checked_table_range(COLLECTION_HEADER_SIZE, refs, count, 128, collection.len())?;
    let sref = refs_range.start + sequence * 128;
    let name = crate::header::decode_c_string(&collection[sref..sref + 32]);
    let rel = nonneg_i32(collection, sref + 64)?;
    let at = COLLECTION_HEADER_SIZE.checked_add(rel)?;
    let end = at.checked_add(64)?;
    if end > collection.len() {
        return None;
    }
    let frames_per_view = be_i16(collection, at + 10)?.max(0) as u16;
    let ticks_per_frame = be_i16(collection, at + 12)?.max(1);
    Some((name, frames_per_view, ticks_per_frame))
}

pub fn get_sequence(headers: &[SequenceHeader], index: i32) -> Option<&SequenceHeader> {
    let i = if index >= 0 && (index as usize) < headers.len() {
        index as usize
    } else {
        0
    };
    headers.get(i)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct CollectionReference {
    pub collection_tag: mandrake_common::FourCC,
    pub permutation_count: i16,
}

impl CollectionReference {
    pub fn from_bytes(data: &[u8]) -> Result<Self, M2Error> {
        if data.len() < 6 {
            return Err(M2Error::TagParse(format!(
                "core tag too short: {} bytes",
                data.len()
            )));
        }
        Ok(Self {
            collection_tag: mandrake_common::FourCC::new([data[0], data[1], data[2], data[3]]),
            permutation_count: i16::from_be_bytes([data[4], data[5]]),
        })
    }
}

pub fn sequence_key_points(collection: &[u8]) -> Result<Vec<Vec<[i16; 3]>>, M2Error> {
    use crate::parse::{be_i16, be_i32, checked_table_range, nonneg_i32};

    let err = || M2Error::TagParse("collection truncated".into());
    let h32 = |o: usize| -> Result<i32, M2Error> { be_i32(collection, o).ok_or_else(err) };
    let h16 = |o: usize| -> Result<i16, M2Error> { be_i16(collection, o).ok_or_else(err) };
    let base = COLLECTION_HEADER_SIZE;
    let coll_flag1 = h32(0)? & 1 != 0;
    let bitmaps_off = nonneg_i32(collection, 0x64).ok_or_else(err)?;
    let bitmaps = base.checked_add(bitmaps_off).ok_or_else(err)?;
    let instance_count = nonneg_i32(collection, 0x70).ok_or_else(err)?;
    let instances_off = nonneg_i32(collection, 0x74).ok_or_else(err)?;
    let instances = base.checked_add(instances_off).ok_or_else(err)?;
    let count = nonneg_i32(collection, 0x80).ok_or_else(err)?;
    let refs_off = nonneg_i32(collection, 0x84).ok_or_else(err)?;
    let refs_range =
        checked_table_range(base, refs_off, count, 128, collection.len()).ok_or_else(err)?;
    let mut out = Vec::with_capacity(count);
    for i in 0..count {
        let sref = refs_range.start + i * 128;
        let so_rel = nonneg_i32(collection, sref + 64).ok_or_else(err)?;
        let so = base.checked_add(so_rel).ok_or_else(err)?;
        let flags = h32(so)?;
        let s = i64::from(h32(so.checked_add(4).ok_or_else(err)?)?.wrapping_mul(10));
        let views = h16(so.checked_add(8).ok_or_else(err)?)?;
        let frames = h16(so.checked_add(0x0a).ok_or_else(err)?)?.max(0) as usize;
        let nv = views.max(0) as usize;
        let displayed = crate::collection::displayed_views(nv);
        let size = 2usize
            .checked_mul(nv)
            .and_then(|v| v.checked_add(0x2e))
            .ok_or_else(err)?;
        let frames_start = so.checked_add(0x40).ok_or_else(err)?;
        let frames_end = frames
            .checked_mul(size)
            .and_then(|b| frames_start.checked_add(b))
            .ok_or_else(err)?;
        if views > 0 && frames_end > collection.len() {
            return Err(err());
        }
        let mut seq = Vec::with_capacity(frames);
        for fr in 0..frames {
            let f = frames_start + fr * size;
            if views <= 0 {
                seq.push([0, 0, 0]);
                continue;
            }
            let instance = |v: usize| -> Result<usize, M2Error> {
                let slot = f
                    .checked_add(0x2e)
                    .and_then(|p| p.checked_add(2usize.checked_mul(v)?))
                    .ok_or_else(err)?;
                let k = h16(slot)?;
                let k = if k >= 0 && (k as usize) < instance_count {
                    k as usize
                } else {
                    0
                };
                instances
                    .checked_add(k.checked_mul(64).ok_or_else(err)?)
                    .ok_or_else(err)
            };
            let (mut lo, mut hi, mut sum_dy, mut first) = (i32::MAX, i32::MIN, 0i32, 0i32);
            for v in 0..nv {
                let mirrored = v >= displayed && flags & 1 != 0;
                let io = if mirrored {
                    instance(crate::collection::mirror_view(nv, v))?
                } else {
                    instance(v)?
                };
                let bitmap = if coll_flag1 {
                    h16(io.checked_add(0x0a).ok_or_else(err)?)?
                } else {
                    h16(io.checked_add(0x1c).ok_or_else(err)?)?
                };
                let rx = i32::from(h16(io.checked_add(0x0c).ok_or_else(err)?)?);
                let ry = i32::from(h16(io.checked_add(0x0e).ok_or_else(err)?)?);
                let mut kx = h16(io.checked_add(0x18).ok_or_else(err)?)?;
                if mirrored && bitmap != -1 {
                    let b_idx = usize::try_from(bitmap).map_err(|_| err())?;
                    let w_off = bitmaps
                        .checked_add(b_idx.checked_mul(128).ok_or_else(err)?)
                        .and_then(|p| p.checked_add(0x4c))
                        .ok_or_else(err)?;
                    let width = h16(w_off)?;
                    kx = width.wrapping_sub(kx);
                }
                let dx = i32::from(kx) - rx;
                let dy = i32::from(h16(io.checked_add(0x1a).ok_or_else(err)?)?) - ry;
                sum_dy += dy;
                lo = lo.min(dx);
                hi = hi.max(dx);
                if v == 0 {
                    first = dx;
                }
            }
            let scale = |v: i32| ((i64::from(v) * s) >> 16) as i32;
            let y = scale(first);
            let z = -(((i64::from(sum_dy) * s) / i64::from(views)) >> 16) as i32;
            let w = (scale(hi) - scale(lo)) / 2;
            let r2 = w.wrapping_mul(w).wrapping_sub(y.wrapping_mul(y)) as u32;
            let mut r = (f64::from(r2)).sqrt() as u64;
            while r * r > u64::from(r2) {
                r -= 1;
            }
            while (r + 1) * (r + 1) <= u64::from(r2) {
                r += 1;
            }
            if u64::from(r2) - r * r > r {
                r += 1;
            }
            seq.push([r as i16, y as i16, z as i16]);
        }
        out.push(seq);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn collection_with(headers: &[[u8; 64]]) -> Vec<u8> {
        let mut d = vec![0u8; COLLECTION_HEADER_SIZE];
        d[0x80..0x84].copy_from_slice(&(headers.len() as i32).to_be_bytes());
        d[0x84..0x88].copy_from_slice(&0i32.to_be_bytes());
        let refs_len = headers.len() * 128;
        d.resize(COLLECTION_HEADER_SIZE + refs_len, 0);
        for i in 0..headers.len() {
            let off = (refs_len + i * 64) as i32;
            let r = COLLECTION_HEADER_SIZE + i * 128 + 64;
            d[r..r + 4].copy_from_slice(&off.to_be_bytes());
        }
        for h in headers {
            d.extend_from_slice(h);
        }
        d
    }

    #[test]
    fn parses_sequence_header_fields_and_falls_back_to_sequence_0() {
        let mut a = [0u8; 64];
        a[0x08..0x0a].copy_from_slice(&1i16.to_be_bytes());
        a[0x0a..0x0c].copy_from_slice(&7i16.to_be_bytes());
        a[0x0c..0x0e].copy_from_slice(&4i16.to_be_bytes());
        a[0x10..0x12].copy_from_slice(&9i16.to_be_bytes());
        a[0x26..0x28].copy_from_slice(&28i16.to_be_bytes());
        let mut b = [0u8; 64];
        b[0x0a..0x0c].copy_from_slice(&3i16.to_be_bytes());
        let hs = sequence_headers(&collection_with(&[a, b])).unwrap();
        assert_eq!(hs.len(), 2);
        assert_eq!(hs[0].number_of_views, 1);
        assert_eq!(hs[0].frames_per_view, 7);
        assert_eq!(hs[0].ticks_per_frame, 4);
        assert_eq!(hs[0].field_26, 28);
        assert_eq!(
            hs[0].effective_loop_frame(),
            0,
            "loop frame 9 >= 7 frames resets to 0"
        );
        assert_eq!(get_sequence(&hs, 1).unwrap().frames_per_view, 3);
        assert_eq!(get_sequence(&hs, 5).unwrap().frames_per_view, 7);
    }

    #[test]
    fn parses_collection_reference() {
        let c = CollectionReference::from_bytes(b"mufs\x00\x03rest").unwrap();
        assert_eq!(c.collection_tag, mandrake_common::FourCC::new(*b"mufs"));
        assert_eq!(c.permutation_count, 3);
    }
}
