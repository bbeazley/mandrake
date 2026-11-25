pub fn compute_gameplay_version(options: u32, version_id: i32, build_number: i16) -> i16 {
    let after = |v: i32| v != -1;
    match options >> 18 & 3 {
        0 if version_id == 0 => return 0x4b0,
        1 if version_id == 0 => return 0x514,
        2 if version_id == 0 || version_id == 0x85c => return 0x578,
        2 if (0x85d..0x861).contains(&version_id) => return 0x582,
        3 if version_id == 0 || version_id == 0x866 => return 0x5dc,
        _ => {}
    }
    match version_id {
        0x867 => return 0x5e6,
        0x86d => return 0x5e5,
        _ => {}
    }
    if options & mandrake_common::game_options::GAMEPLAY_1_5_1_GAME != 0 {
        return 0x5e6;
    }
    let b = i32::from(build_number);
    let flag = options & mandrake_common::game_options::BUILD_VERSIONS != 0;
    match version_id {
        0x868 | 0x86f | 0x870 => 0x640,
        0x87a => 0x6a4,
        0x87b => 0x6ad + i16::from(flag),
        0x87c if flag && (!after(b) || b > 0x173) => 0x6b8,
        0x87c => 0x6b3,
        0x882 => 0x69f,
        0x883 => 0x69a,
        0x884 if flag && (b > 0x1a4 || !after(b)) && (b > 0x1a6 || !after(b)) => 0x70d,
        0x884 => 0x708,
        0x885 => 0x713 - i16::from(b < 0x1af && after(b)),
        0x886 if b > 0x1bb || !after(b) => 0x71e - i16::from(b < 0x1be && after(b)),
        0x886 => 0x71c,
        0x887 => 0x726,
        0x888 => 0x731 - i16::from(b < 0x1cc && after(b)),
        _ => 0x73c,
    }
}

const VERSIONS: [(i16, i32, i32); 22] = [
    (1200, 2120, -1),
    (1300, 2130, -1),
    (1500, 2150, -1),
    (1510, 2151, -1),
    (1600, 2160, -1),
    (1700, 2170, -1),
    (1710, 2171, -1),
    (1720, 2172, -1),
    (1800, 2180, 417),
    (1801, 2180, 420),
    (1805, 2180, -1),
    (1810, 2181, 430),
    (1811, 2181, -1),
    (1820, 2182, 443),
    (1821, 2182, 445),
    (1822, 2182, -1),
    (1830, 2183, -1),
    (1840, 2184, 461),
    (1841, 2184, -1),
    (1850, 2185, 465),
    (1851, 2185, 468),
    (1852, 2185, -1),
];

pub fn gameplay_version_from_version_id(version_id: i32, build_number: i16) -> i16 {
    let id = if version_id == 0 { 0x852 } else { version_id };
    VERSIONS
        .iter()
        .find(|&&(_, row_id, last)| row_id == id && (i32::from(build_number) <= last || last == -1))
        .map_or(-1, |&(version, _, _)| version)
}

pub fn gameplay_version_to_string(version: i16) -> String {
    let named = match version {
        0x4b0 => "1.2",
        0x514 => "1.3",
        0x582 => "1.4.x",
        0x5e5 | 0x5e6 => "1.5.1",
        0x69a | 0x69f => "1.7.0 BETA",
        0x6ad => "1.7.1 BETA",
        0x6b3 | 0x6b5 => "1.7.2 BETA",
        0x708 => "1.8.0 BETA",
        0x709 => "1.8.0 BETA 2",
        0x70a => "1.8.0 B422",
        0x712 => "1.8.1 BETA",
        0x71c | 0x71d => "1.8.2 BETA",
        0x730 => "1.8.4 BETA 1 (unsupported)",
        0x73a | 0x73b => "1.8.5 BETA",
        _ => "",
    };
    if !named.is_empty() {
        return named.to_owned();
    }
    let v = i32::from(version);
    let major = v / 1000;
    let rest = v - major * 1000;
    let mut s = format!("{major}.{}.{}", rest / 100, (rest % 100) / 10);
    s.truncate(7);
    s
}
