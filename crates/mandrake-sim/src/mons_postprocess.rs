use crate::engine_rng::TagSource;
use mandrake_common::FourCC;
use mandrake_common::game_options;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MonsPostprocess {
    pub mode: i16,
    pub options: u32,
    pub gameplay_version: i32,
}

const MONS_LEN: usize = 0x224;

fn be_i16(m: &[u8], o: usize) -> i16 {
    i16::from_be_bytes([m[o], m[o + 1]])
}

fn put_i16(m: &mut [u8], o: usize, v: i16) {
    m[o..o + 2].copy_from_slice(&v.to_be_bytes());
}

impl Default for MonsPostprocess {
    fn default() -> Self {
        Self::COMMAND_LINE
    }
}

impl MonsPostprocess {
    pub const COMMAND_LINE: Self = Self {
        mode: crate::engine_rng::GameParameters::COMMAND_LINE.mode,
        options: crate::players::COMMAND_LINE_GAME_OPTIONS,
        gameplay_version: crate::engine_rng::GameParameters::COMMAND_LINE.gameplay_version,
    };

    pub fn apply(&self, raw: &[u8]) -> Vec<u8> {
        let mut m = raw.to_vec();
        if m.len() < MONS_LEN {
            return m;
        }
        let mut flags = u32::from_be_bytes([m[0], m[1], m[2], m[3]]);
        let class = be_i16(&m, 0x1b6);
        let mut w198: u16 = 0;
        let mut b1b5: u8 = 0;

        if self.mode == 1
            && self.options & game_options::COOPERATIVE == 0
            && (class.wrapping_sub(4) as u16) > 1
        {
            flags &= !0x2000;
        }
        if flags & 0x20_0000 != 0 {
            let v = be_i16(&m, 0x21c).min(9);
            put_i16(&mut m, 0x21c, v);
            if v <= 0 {
                flags &= !0x20_0000;
            }
        }
        let (lo, hi) = (be_i16(&m, 0x190), be_i16(&m, 0x192));
        if m[0x1b4] != 0 && lo <= 0 && hi <= 0 {
            m[0x1b4] = 0;
        } else {
            let max = lo.max(hi);
            put_i16(&mut m, 0x192, max);
            put_i16(&mut m, 0x190, lo);
        }

        let options = self.options;
        let option_2 = options & game_options::TFL != 0;
        let mut dl: u8;
        let big;
        if option_2 && class == 1 {
            b1b5 = 0xa;
            dl = 0xa;
            big = options & (game_options::TFL | game_options::ANTI_CLUMP) != 0;
        } else if options & (game_options::TFL | game_options::ANTI_CLUMP) == 0 {
            dl = 0;
            big = false;
        } else {
            dl = 0;
            big = true;
        }
        if big {
            let r = be_i16(&m, 0x5a);
            if class == 0 {
                if r <= 0x63 {
                    put_i16(&mut m, 0x5a, 0xa2);
                } else if r <= 0x9a {
                    put_i16(&mut m, 0x5a, 0xc8);
                }
                dl = 0x28;
            } else {
                if r <= 0x63 {
                    let r32 = i32::from(r);
                    let third = ((r32 * 0x5556) >> 16) - (r32 >> 15);
                    put_i16(&mut m, 0x5a, (r32 + third) as i16);
                    dl = b1b5;
                }
                dl |= 0x28;
            }
            if option_2 {
                let x = i32::from(be_i16(&m, 0x7e));
                let tenth = (x * 0x6667) >> 18;
                let sign = i32::from((x as i16) >> 15);
                put_i16(&mut m, 0x7e, (x + (sign - tenth)) as i16);
            }
            b1b5 = dl;
            w198 = 0x800;
        }

        if flags & 0x10_0000 != 0 {
            let mut ax = w198;
            let force = flags & 0x4000_0000 != 0;
            let (with_4f, with_4c) = if force {
                (true, true)
            } else {
                (m[0x4f] != 0xff, m[0x4c] != 0xff)
            };
            let b4e = m[0x4e];
            if force || b4e != 0xff {
                for bit in [1u8, 2, 4, 8, 0x10, 0x20, 0x40] {
                    if b4e & bit != 0 && (bit != 4 || be_i16(&m, 0x22) != -1) {
                        ax |= u16::from(bit);
                    }
                }
                if b4e & 0x80 != 0 {
                    put_i16(&mut m, 0x1f0, 3);
                    ax |= 0x80;
                }
            }
            if with_4f {
                ax |= u16::from(m[0x4f]) << 8;
            }
            if with_4c {
                let c = m[0x4c];
                for (from, to) in [(1u8, 1u8), (2, 2), (0x80, 4), (4, 8), (8, 0x10)] {
                    if c & from != 0 {
                        dl |= to;
                    }
                }
            }
            b1b5 = dl;
            w198 = ax;
            if ax != 0 {
                m[0x4e] = 0;
                m[0x4f] = 0;
            }
            if dl != 0
                && (self.gameplay_version <= game_options::GAMEPLAY_1_5_1 || (m[0x4c] as i8) > 0)
            {
                m[0x4c] = 0;
            }
        }

        m[0..4].copy_from_slice(&flags.to_be_bytes());
        m[0x198] = w198 as u8;
        m[0x199] = (w198 >> 8) as u8;
        m[0x1b5] = b1b5;
        if let Ok(t) = mandrake_tag::unit::MonsterTerrain::from_mons_bytes(&m) {
            let mask = t.impassable_mask(self.gameplay_version);
            m[0x58..0x5a].copy_from_slice(&mask.to_be_bytes());
        }
        m
    }
}

pub struct PostprocessedTags<'a> {
    pub tags: &'a dyn TagSource,
    pub postprocess: MonsPostprocess,
}

impl TagSource for PostprocessedTags<'_> {
    fn tag(&self, group: [u8; 4], id: FourCC) -> Option<Vec<u8>> {
        let data = self.tags.tag(group, id)?;
        Some(if &group == b"mons" {
            self.postprocess.apply(&data)
        } else {
            data
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mons(flags: u32, class: i16) -> Vec<u8> {
        let mut m = vec![0u8; 0x400];
        m[0..4].copy_from_slice(&flags.to_be_bytes());
        m[0x1b6..0x1b8].copy_from_slice(&class.to_be_bytes());
        m[0x5c..0x6c].fill(0x80);
        m
    }

    const PLAIN: MonsPostprocess = MonsPostprocess {
        mode: 2,
        options: 0x200c_40bc,
        gameplay_version: crate::engine_rng::GAMEPLAY_VERSION_1_8_3,
    };

    #[test]
    fn the_tag_bytes_at_0x198_and_0x1b5_are_replaced() {
        let mut m = mons(0xd7, 1);
        m[0x1b5] = 0x32;
        m[0x198] = 0x12;
        m[0x199] = 0x34;
        let p = PLAIN.apply(&m);
        assert_eq!((p[0x198], p[0x199], p[0x1b5]), (0, 0, 0));
    }

    #[test]
    fn the_game_options_set_0x800_and_0x28() {
        let o = |options| MonsPostprocess { options, ..PLAIN };
        let mut m = mons(0, 0);
        m[0x5a..0x5c].copy_from_slice(&0x77i16.to_be_bytes());
        let p = o(0x40_0000).apply(&m);
        assert_eq!((p[0x198], p[0x199], p[0x1b5]), (0, 8, 0x28));
        assert_eq!(be_i16(&p, 0x5a), 0xc8);
        let mut m = mons(0, 1);
        m[0x5a..0x5c].copy_from_slice(&0x51i16.to_be_bytes());
        m[0x7e..0x80].copy_from_slice(&0x15i16.to_be_bytes());
        let p = o(0x20_0000).apply(&m);
        assert_eq!((p[0x198], p[0x199], p[0x1b5]), (0, 8, 0x2a));
        assert_eq!(be_i16(&p, 0x5a), 0x51 + 0x1b);
        assert_eq!(be_i16(&p, 0x7e), 0x15 - 2);
        let mut m = mons(0, 1);
        m[0x7e..0x80].copy_from_slice(&(-0x15i16).to_be_bytes());
        assert_eq!(be_i16(&o(0x20_0000).apply(&m), 0x7e), -0x15 + 2);
    }

    #[test]
    fn flag_0x100000_moves_the_terrain_bytes_into_the_bits() {
        let mut m = mons(0x10_0000, 0);
        m[0x22..0x24].copy_from_slice(&(-1i16).to_be_bytes());
        m[0x4e] = 0x86;
        m[0x4f] = 0x41;
        m[0x4c] = 0x8d;
        let p = PLAIN.apply(&m);
        assert_eq!(u16::from(p[0x198]) | u16::from(p[0x199]) << 8, 0x4182);
        assert_eq!(p[0x1b5], 0x1d);
        assert_eq!(be_i16(&p, 0x1f0), 3);
        assert_eq!((p[0x4c], p[0x4e], p[0x4f]), (0x8d, 0, 0));
        let mut m = mons(0x10_0000, 0);
        m[0x4e] = 0xff;
        m[0x4f] = 0xff;
        m[0x4c] = 0x02;
        let p = PLAIN.apply(&m);
        assert_eq!((p[0x198], p[0x199], p[0x1b5]), (0, 0, 2));
        assert_eq!((p[0x4c], p[0x4e], p[0x4f]), (0, 0xff, 0xff));
    }

    #[test]
    fn the_recovery_bounds_and_the_cap_at_0x21c() {
        let mut m = mons(0x20_0000, 0);
        m[0x190..0x192].copy_from_slice(&30i16.to_be_bytes());
        m[0x192..0x194].copy_from_slice(&0i16.to_be_bytes());
        m[0x21c..0x21e].copy_from_slice(&12i16.to_be_bytes());
        let p = PLAIN.apply(&m);
        assert_eq!((be_i16(&p, 0x190), be_i16(&p, 0x192)), (30, 30));
        assert_eq!(be_i16(&p, 0x21c), 9);
        assert_eq!(p[1] & 0x20, 0x20);
        m[0x21c..0x21e].copy_from_slice(&0i16.to_be_bytes());
        m[0x1b4] = 1;
        m[0x190..0x192].copy_from_slice(&0i16.to_be_bytes());
        let p = PLAIN.apply(&m);
        assert_eq!(p[1] & 0x20, 0);
        assert_eq!(p[0x1b4], 0);
    }

    #[test]
    fn mode_1_clears_flag_0x2000_except_classes_4_and_5() {
        let one = MonsPostprocess {
            mode: 1,
            options: PLAIN.options & !game_options::COOPERATIVE,
            ..PLAIN
        };
        assert_eq!(one.apply(&mons(0x2000, 0))[2], 0);
        assert_eq!(one.apply(&mons(0x2000, 5))[2], 0x20);
        assert_eq!(
            MonsPostprocess { mode: 1, ..PLAIN }.apply(&mons(0x2000, 0))[2],
            0x20
        );
        assert_eq!(PLAIN.apply(&mons(0x2000, 0))[2], 0x20);
    }
}
