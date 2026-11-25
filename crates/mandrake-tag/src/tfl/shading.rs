use crate::collection::{CollectionTag, LandscapeShadingTable};

pub const TFL_SHADING_LEVELS: u32 = 256;

pub fn landscape_shading_table(
    palette16: &[[u16; 3]; 256],
    color_count: usize,
    levels: u32,
) -> LandscapeShadingTable {
    let n1 = levels.saturating_sub(1);
    let mut channels = vec![[0u16; 3]; levels as usize * 256];
    for l in 0..levels {
        for (j, p) in palette16.iter().enumerate().take(color_count) {
            let ch = |c: u16| -> u16 {
                if n1 == 0 {
                    return c;
                }
                ((u32::from(c) * l + n1 / 2) / n1).min(0xffff) as u16
            };
            channels[(l as usize) << 8 | j] = [ch(p[0]), ch(p[1]), ch(p[2])];
        }
    }
    LandscapeShadingTable { levels, channels }
}

pub fn tfl_team_colors(
    coll: &CollectionTag,
    base: &[[u16; 4]],
    primary: [u16; 4],
    secondary: [u16; 4],
) -> Vec<[u16; 4]> {
    let on = |c: &[u16; 4]| c[3] & 4 == 0;
    coll.hue_changes
        .iter()
        .zip(base)
        .map(|(hc, b)| {
            if hc.flags & 1 != 0 && on(&primary) {
                primary
            } else if hc.flags & 2 != 0 && on(&secondary) {
                secondary
            } else {
                *b
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_landscape_table_runs_from_black_to_the_palette() {
        let mut pal = [[0u16; 3]; 256];
        pal[1] = [0xffff, 0x8000, 0x0101];
        let t = landscape_shading_table(&pal, 2, 256);
        assert_eq!(t.channels.len(), 256 * 256);
        assert_eq!(t.channels[1], [0; 3]);
        assert_eq!(t.channels[255 << 8 | 1], pal[1]);
        assert_eq!(t.channels[128 << 8 | 1], [0x8080, 0x4040, 0x0081]);
        assert_eq!(t.rgb8()[255 << 8 | 1], [0xff, 0x80, 0x01]);
    }

    #[test]
    fn team_colors_take_primary_then_secondary_then_base() {
        let hc = |flags| crate::collection::HueChange {
            name: String::new(),
            hue_range: (0, 0),
            mask: [0; 32],
            min_saturation: 0,
            base_hue: 0,
            flags,
        };
        let coll = CollectionTag {
            bitmap_count: 0,
            pixels_to_world: 0,
            palette: Vec::new(),
            frames: Vec::new(),
            sequences: Vec::new(),
            shadow_descriptors: Vec::new(),
            color_table: Vec::new(),
            hue_changes: vec![hc(1), hc(2), hc(3), hc(0)],
            sequence_instance_flags: Vec::new(),
            sequence_missing_bitmaps: Vec::new(),
        };
        let (b, p, s) = ([1, 1, 1, 0], [2, 2, 2, 0], [3, 3, 3, 0]);
        assert_eq!(tfl_team_colors(&coll, &[b; 4], p, s), vec![p, s, p, b]);
        let p_off = [2, 2, 2, 4];
        assert_eq!(tfl_team_colors(&coll, &[b; 4], p_off, s), vec![b, s, s, b]);
        let b_off = [1, 1, 1, 4];
        assert_eq!(
            tfl_team_colors(&coll, &[b_off; 4], p, s),
            vec![p, s, p, b_off]
        );
    }
}
