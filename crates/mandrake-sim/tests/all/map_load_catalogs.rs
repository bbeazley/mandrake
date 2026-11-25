#[test]
fn flag_0x1000000_raises_a_low_marker() {
    use mandrake_sim::engine_rng::{EngineRng, SiteCounts, new_monster_marker_height};
    let (mut rng, mut sites) = (mandrake_math::BungieRng::new(7), SiteCounts::default());
    let mut r = EngineRng {
        rng: &mut rng,
        sites: &mut sites,
    };
    assert_eq!(
        new_monster_marker_height(0x301, 0x100, 0x726, &mut r),
        0x301
    );
    assert_eq!(
        new_monster_marker_height(0x300, 0x100, 0x726, &mut r),
        0x900
    );
    assert_eq!(new_monster_marker_height(-5, 0x100, 0x726, &mut r), 0x900);
    assert_eq!(new_monster_marker_height(0, 0x1_0100, 0x726, &mut r), 0x900);
    assert_eq!(r.sites.total(), 0);
    let mut want = mandrake_math::BungieRng::new(7);
    let d = i32::from(want.next_u16()) >> 5;
    assert_eq!(
        new_monster_marker_height(0x300, 0x100, 0x5dc, &mut r),
        0x100 + 0x600 + d
    );
    assert_eq!(r.sites.total(), 1);
}
