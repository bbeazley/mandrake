use super::*;

fn parameter(name: &[u8; 4], kind: i16, values: &[i32]) -> MapActionParameter {
    MapActionParameter {
        kind,
        count: (values.len() / MapActionParameter::scalars_per_element(kind)) as i16,
        name: FourCC::new(*name),
        values: values.to_vec(),
    }
}

#[test]
fn the_modes_are_the_procs() {
    let mode = |names: &[&[u8; 4]], gameplay| {
        let a = plmo(names.iter().map(|n| parameter(n, 0, &[])).collect());
        let m = a.visibility_mode(0, gameplay);
        (m.visible, m.invisible)
    };
    assert_eq!(mode(&[], 0x73c), (-1, -1));
    assert_eq!(mode(&[b"moen"], 0x73c), (0, 0));
    assert_eq!(mode(&[b"blam"], 0x73c), (1, 1));
    assert_eq!(mode(&[b"moen", b"blam"], 0x73c), (1, 1));
    assert_eq!(mode(&[b"tain"], 0x73c), (2, 2));
    assert_eq!(mode(&[b"fade"], 0x73c), (3, 3));
    assert_eq!(mode(&[b"tain", b"fade"], 0x5e6), (2, 3));
}

fn plmo(parameters: Vec<MapActionParameter>) -> MapActions {
    MapActions {
        actions: vec![MapAction {
            identifier: 1,
            kind: 3,
            tag: tag(b"plmo"),
            flags: 4,
            delay: 0,
            delay_range: 0,
            time: 0,
            parameters,
        }],
        ..MapActions::default()
    }
}

fn waypoints(n: i32) -> MapActionParameter {
    let points: Vec<i32> = (0..n).flat_map(|k| [1000 * k, 0]).collect();
    parameter(b"wayp", TYPE_POINT, &points)
}

#[test]
fn the_leg_past_the_list_is_the_next_parameters_bytes() {
    let a = plmo(vec![
        waypoints(8),
        parameter(b"wayr", TYPE_DISTANCE, &[1024]),
        parameter(b"radi", TYPE_DISTANCE, &[5120, 12288]),
        parameter(b"loop", 0, &[]),
        parameter(b"ntrp", 0, &[]),
        parameter(b"spac", TYPE_DISTANCE, &[2048]),
    ]);
    let w = a.waypoints(0).unwrap();
    assert_eq!(
        a.interpolation_leg(0, &w, 9).unwrap(),
        ((65542, 0x7761_7972), (1024, 131078))
    );
    assert_eq!(a.interpolation_steps(0, 9, 7).unwrap(), 15);
    assert_eq!(a.interpolation_leg(0, &w, 8).unwrap(), (w[7], w[0]));
}

#[test]
fn wayr_with_six_values_gives_the_engines_read() {
    let a = plmo(vec![
        waypoints(6),
        parameter(b"wayr", TYPE_DISTANCE, &[512, 1024, 1024, 1536, 512, 1024]),
        parameter(b"spac", TYPE_DISTANCE, &[1024]),
    ]);
    let w = a.waypoints(0).unwrap();
    assert_eq!(
        a.interpolation_leg(0, &w, 7).unwrap(),
        ((393222, 0x7761_7972), (512, 1024))
    );
    assert_eq!(a.interpolation_steps(0, 7, 0).unwrap(), 31);
}

#[test]
fn an_empty_wayr_gives_the_engines_read() {
    let a = plmo(vec![
        waypoints(5),
        parameter(b"wayr", TYPE_DISTANCE, &[]),
        parameter(b"radi", TYPE_DISTANCE, &[6656, 7680]),
        parameter(b"ntrp", 0, &[]),
        parameter(b"spac", TYPE_DISTANCE, &[3584]),
    ]);
    let w = a.waypoints(0).unwrap();
    assert_eq!(
        a.interpolation_leg(0, &w, 6).unwrap(),
        ((6, 0x7761_7972), (131078, 0x7261_6469))
    );
    assert_eq!(a.interpolation_steps(0, 6, 0).unwrap(), 9);
}

#[test]
fn a_read_past_the_block_or_into_padding_stays_unmodelled() {
    let a = plmo(vec![waypoints(2)]);
    let w = a.waypoints(0).unwrap();
    let e = a.interpolation_leg(0, &w, 3).unwrap_err();
    assert_eq!(e.site, crate::draw_site::INTERPOLATION_BEFORE_WAYPOINTS);
    assert!(e.reason.contains("past the block"), "{}", e.reason);
    let a = plmo(vec![waypoints(2), parameter(b"wayr", TYPE_DISTANCE, &[9])]);
    let e = a.interpolation_leg(0, &w, 3).unwrap_err();
    assert!(e.reason.contains("past the block"), "{}", e.reason);
    let a = plmo(vec![
        waypoints(2),
        parameter(b"subj", TYPE_MONSTERS, &[7]),
        parameter(b"loop", 0, &[]),
    ]);
    let e = a.interpolation_leg(0, &w, 3).unwrap_err();
    assert!(e.reason.contains("padding"), "{}", e.reason);
    assert_eq!(
        a.interpolation_leg(0, &w, 0).unwrap_err().site,
        crate::draw_site::INTERPOLATION_BEFORE_WAYPOINTS
    );
}

#[test]
fn a_rewritten_parameter_goes_last() {
    let mut a = plmo(vec![
        waypoints(2),
        parameter(b"#wpi", TYPE_INTEGER, &[1]),
        parameter(b"#wpi", TYPE_DISTANCE, &[2]),
        parameter(b"spac", TYPE_DISTANCE, &[2048]),
    ]);
    a.set_param(0, b"#wpi", TYPE_INTEGER, vec![3]);
    let order: Vec<(FourCC, i16)> = a.actions[0]
        .parameters
        .iter()
        .map(|p| (p.name, p.kind))
        .collect();
    assert_eq!(
        order,
        [
            (FourCC::new(*b"wayp"), TYPE_POINT),
            (FourCC::new(*b"#wpi"), TYPE_DISTANCE),
            (FourCC::new(*b"spac"), TYPE_DISTANCE),
            (FourCC::new(*b"#wpi"), TYPE_INTEGER),
        ]
    );
    a.set_param(0, b"wayp", TYPE_POINT, vec![0, 0, 10, 0]);
    a.set_param(0, b"#iwi", TYPE_INTEGER, vec![5]);
    a.set_param(0, b"#iwc", TYPE_INTEGER, vec![6]);
    let w = a.waypoints(0).unwrap();
    assert_eq!(
        a.interpolation_leg(0, &w, 3).unwrap(),
        ((65541, tag(b"#iwi") as i32), (5, 65541))
    );
}
