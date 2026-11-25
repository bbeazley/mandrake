use mandrake_sim::lightning::lightning_midpoints;

#[test]
fn test_new_lightning_zero_distance() {
    let pt = [1000, 2000, 300];
    let mut draw_count = 0;
    let (mid1, mid2) = lightning_midpoints(pt, pt, |_site| {
        draw_count += 1;
        0
    });
    assert_eq!(draw_count, 0);
    assert_eq!(mid1, pt);
    assert_eq!(mid2, pt);
}

#[test]
fn test_new_lightning_draw_sites_and_jitter() {
    let pt1 = [1000, 2000, 300];
    let pt2 = [1400, 2800, 500];

    let mut sites = Vec::new();
    let (mid1, mid2) = lightning_midpoints(pt1, pt2, |site| {
        sites.push(site);
        0x8000
    });

    let expected_sites = [
        mandrake_sim::draw_site::LIGHTNING_MID1_X,
        mandrake_sim::draw_site::LIGHTNING_MID1_Y,
        mandrake_sim::draw_site::LIGHTNING_MID1_Z,
        mandrake_sim::draw_site::LIGHTNING_MID2_X,
        mandrake_sim::draw_site::LIGHTNING_MID2_Y,
        mandrake_sim::draw_site::LIGHTNING_MID2_Z,
    ];
    assert_eq!(sites, expected_sites);

    assert_eq!((mid1, mid2), ([1100, 2200, 464], [1300, 2600, 564]));
}

#[test]
fn test_new_lightning_small_distance() {
    let pt1 = [0, 0, 0];
    let pt2 = [2, 0, 0];
    let mut sites = Vec::new();
    let _ = lightning_midpoints(pt1, pt2, |site| {
        sites.push(site);
        0
    });
    assert_eq!(
        sites,
        [
            mandrake_sim::draw_site::LIGHTNING_MID1_X,
            mandrake_sim::draw_site::LIGHTNING_MID1_Y,
            mandrake_sim::draw_site::LIGHTNING_MID2_X,
            mandrake_sim::draw_site::LIGHTNING_MID2_Y
        ]
    );
}
