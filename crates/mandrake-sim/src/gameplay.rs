pub use mandrake_common::game_options::{GAMEPLAY_1_8_3, GAMEPLAY_1_8_5};

pub fn compute_minimum_build_number(block: &[u8]) -> i16 {
    let be16 = |o: usize| -> Option<i16> {
        block
            .get(o..o + 2)
            .map(|b| i16::from_be_bytes([b[0], b[1]]))
    };
    let count = be16(0x22c).unwrap_or(0);
    let mut min = 0x7fffi16;
    if count <= 0 {
        return min;
    }
    for i in 0..count as usize {
        let e = 0x230 + i * 0x7c;
        if be16(e + 0x02) == Some(0)
            && let Some(build) = be16(e + 0x20)
            && min > build
        {
            min = build;
        }
    }
    if min == 0 { -1 } else { min }
}

pub fn compute_gameplay_version(options: u32, build: i32, player_build: i16) -> i32 {
    let known = player_build != -1;
    let mode = (options >> 18) & 3;
    let early = match mode {
        0 if build == 0 => Some(0x4b0),
        1 if build == 0 => Some(0x514),
        2 if build == 0 || build == 0x85c => Some(0x578),
        2 if (0x85d..=0x860).contains(&build) => Some(0x582),
        3 if build == 0 || build == 0x866 => Some(0x5dc),
        _ => None,
    };
    if let Some(v) = early {
        return v;
    }
    match build {
        0x867 => return 0x5e6,
        0x86d => return 0x5e5,
        _ => {}
    }
    if options & mandrake_common::game_options::GAMEPLAY_1_5_1_GAME != 0 {
        return 0x5e6;
    }
    let extended = options & mandrake_common::game_options::BUILD_VERSIONS != 0;
    match build {
        0x868 | 0x86f | 0x870 => 0x640,
        0x87a => 0x6a4,
        0x87b => 0x6ad + i32::from(extended),
        0x87c if !extended => 0x6b3,
        0x87c if known && player_build <= 0x173 => 0x6b5,
        0x87c => 0x6b8,
        0x882 => 0x69f,
        0x883 => 0x69a,
        0x884 if !extended => 0x708,
        0x884 if known && player_build <= 0x1a4 => 0x709,
        0x884 if known && player_build <= 0x1a6 => 0x70a,
        0x884 => 0x70d,
        0x885 if known && player_build <= 0x1ae => 0x712,
        0x885 => 0x713,
        0x886 if known && player_build <= 0x1bb => 0x71c,
        0x886 if known && player_build <= 0x1bd => 0x71d,
        0x886 => 0x71e,
        0x887 => GAMEPLAY_1_8_3,
        0x888 if known && player_build <= 0x1cb => 0x730,
        0x888 => 0x731,
        _ => GAMEPLAY_1_8_5,
    }
}

pub fn gameplay_version_of_block(block: &[u8]) -> Option<i32> {
    let options = block.get(0x04..0x08)?;
    let build = block.get(0x24..0x28)?;
    Some(compute_gameplay_version(
        u32::from_be_bytes([options[0], options[1], options[2], options[3]]),
        i32::from_be_bytes([build[0], build[1], build[2], build[3]]),
        compute_minimum_build_number(block),
    ))
}

pub fn replay_clamp(version: i32) -> i32 {
    version.min(GAMEPLAY_1_8_5)
}

#[cfg(test)]
mod tests {
    use super::*;

    const FILM_OPTIONS: u32 = 0x200c_40bc;

    #[test]
    fn the_films_builds() {
        assert_eq!(compute_gameplay_version(FILM_OPTIONS, 0x887, 0x1c0), 0x726);
        assert_eq!(compute_gameplay_version(FILM_OPTIONS, 0x889, 0x1d7), 0x73c);
    }

    #[test]
    fn the_jump_table() {
        let v = |b, s| compute_gameplay_version(0, b, s);
        for b in [0x868, 0x86f, 0x870] {
            assert_eq!(v(b, -1), 0x640);
        }
        for b in [
            0x869, 0x86e, 0x871, 0x879, 0x87d, 0x881, 0x889, 0x900, 0x800,
        ] {
            assert_eq!(v(b, -1), 0x73c, "{b:#x}");
        }
        assert_eq!(v(0x87a, -1), 0x6a4);
        assert_eq!(v(0x882, -1), 0x69f);
        assert_eq!(v(0x883, -1), 0x69a);
        assert_eq!(v(0x887, 3), 0x726);
        assert_eq!(v(0x867, -1), 0x5e6);
        assert_eq!(v(0x86d, -1), 0x5e5);
    }

    #[test]
    fn the_player_build_rules() {
        let x = 0x2000_0000;
        let v = |o, b, s| compute_gameplay_version(o, b, s);
        assert_eq!(v(0, 0x87b, 0), 0x6ad);
        assert_eq!(v(x, 0x87b, 0), 0x6ae);
        assert_eq!(v(0, 0x87c, 0), 0x6b3);
        assert_eq!(v(x, 0x87c, 0x173), 0x6b5);
        assert_eq!(v(x, 0x87c, 0x174), 0x6b8);
        assert_eq!(v(x, 0x87c, -1), 0x6b8);
        assert_eq!(v(0, 0x884, 0), 0x708);
        assert_eq!(v(x, 0x884, 0x1a4), 0x709);
        assert_eq!(v(x, 0x884, 0x1a6), 0x70a);
        assert_eq!(v(x, 0x884, 0x1a7), 0x70d);
        assert_eq!(v(x, 0x884, -1), 0x70d);
        assert_eq!(v(0, 0x885, 0x1ae), 0x712);
        assert_eq!(v(0, 0x885, 0x1af), 0x713);
        assert_eq!(v(0, 0x885, -1), 0x713);
        assert_eq!(v(0, 0x886, 0x1bb), 0x71c);
        assert_eq!(v(0, 0x886, 0x1bd), 0x71d);
        assert_eq!(v(0, 0x886, 0x1be), 0x71e);
        assert_eq!(v(0, 0x886, -1), 0x71e);
        assert_eq!(v(0, 0x888, 0x1cb), 0x730);
        assert_eq!(v(0, 0x888, 0x1cc), 0x731);
        assert_eq!(v(0, 0x888, -1), 0x731);
    }

    #[test]
    fn the_older_game_fields() {
        let field = |f: u32| f << 18;
        assert_eq!(compute_gameplay_version(field(0), 0, -1), 0x4b0);
        assert_eq!(compute_gameplay_version(field(1), 0, -1), 0x514);
        assert_eq!(compute_gameplay_version(field(2), 0, -1), 0x578);
        assert_eq!(compute_gameplay_version(field(2), 0x85c, -1), 0x578);
        assert_eq!(compute_gameplay_version(field(2), 0x85d, -1), 0x582);
        assert_eq!(compute_gameplay_version(field(2), 0x860, -1), 0x582);
        assert_eq!(compute_gameplay_version(field(3), 0, -1), 0x5dc);
        assert_eq!(compute_gameplay_version(field(3), 0x866, -1), 0x5dc);
        assert_eq!(compute_gameplay_version(0x100_0000, 0x889, -1), 0x5e6);
    }

    fn block(build: i32, entries: &[(i16, i16)]) -> Vec<u8> {
        let mut b = vec![0u8; 0x230 + 16 * 0x7c];
        b[0x04..0x08].copy_from_slice(&FILM_OPTIONS.to_be_bytes());
        b[0x24..0x28].copy_from_slice(&build.to_be_bytes());
        b[0x22c..0x22e].copy_from_slice(&(entries.len() as i16).to_be_bytes());
        for (i, &(field_02, player_build)) in entries.iter().enumerate() {
            let e = 0x230 + i * 0x7c;
            b[e + 0x02..e + 0x04].copy_from_slice(&field_02.to_be_bytes());
            b[e + 0x20..e + 0x22].copy_from_slice(&player_build.to_be_bytes());
        }
        b
    }

    #[test]
    fn the_minimum_build() {
        assert_eq!(compute_minimum_build_number(&block(0, &[])), 0x7fff);
        assert_eq!(
            compute_minimum_build_number(&block(0, &[(0, 0x1d7), (0, 0x1c0), (1, 3)])),
            0x1c0
        );
        assert_eq!(compute_minimum_build_number(&block(0, &[(1, 3)])), 0x7fff);
        assert_eq!(compute_minimum_build_number(&block(0, &[(0, 0)])), -1);
    }

    #[test]
    fn a_block_decides() {
        assert_eq!(gameplay_version_of_block(&block(0x887, &[])), Some(0x726));
        assert_eq!(gameplay_version_of_block(&block(0x889, &[])), Some(0x73c));
        assert_eq!(
            gameplay_version_of_block(&block(0x888, &[(0, 0x1cb), (0, 0x1d0)])),
            Some(0x730)
        );
        assert_eq!(gameplay_version_of_block(&[0u8; 0x12]), None);
    }

    #[test]
    fn the_replay_clamp() {
        assert_eq!(replay_clamp(0x726), 0x726);
        assert_eq!(replay_clamp(0x73c), 0x73c);
        assert_eq!(replay_clamp(0x800), 0x73c);
    }
}
