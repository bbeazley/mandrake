use super::*;

#[test]
fn unit_control_masks_are_the_engines_tables() {
    let masks = |gameplay: i32, set: &[(&[u8; 4], bool)]| {
        let m = UnitControlMasks::new(gameplay, |n| {
            set.iter().find(|(k, _)| *k == n).map(|(_, v)| *v)
        });
        (m.and_8, m.or_8, m.and_4, m.or_4)
    };
    let none = (0xffff_ffff, 0, 0xffff_ffff, 0);
    assert_eq!(masks(0x726, &[]), none);
    let dnat = |v| [(b"dnat", v)];
    assert_eq!(masks(0x726, &dnat(true)), (0xffff_fff7, 8, !0, 0));
    assert_eq!(masks(0x726, &dnat(false)), (0xffff_fff7, 0, !0, 0));
    let both: &[(&[u8; 4], bool)] = &[(b"dnat", true), (b"cnba", true)];
    assert_eq!(masks(0x726, both), (0xffff_ffe7, 0x18, !0, 0));
    assert_eq!(masks(0x726, &[(b"cnba", true)]), (0xffff_ffef, 0x10, !0, 0));
    assert_eq!(masks(0x726, &[(b"cnba", false)]), (0xffff_ffef, 0, !0, 0));
    let three: &[(&[u8; 4], bool)] = &[
        (b"dnat", true),
        (b"ngtf", true),
        (b"hdcf", true),
        (b"mamb", true),
    ];
    assert_eq!(masks(0x708, three), (0xffff_fff7, 8, !0, 0));
    assert_eq!(masks(0x709, three), (0xffff_c7f7, 0x3808, !0, 0));
    let igno0 = [(b"igno", false)];
    assert_eq!(masks(0x514, &igno0).0, 0xffff_fffd);
    assert_eq!(masks(0x515, &igno0).0, 0xffff_fff5);
    let with: &[(&[u8; 4], bool)] = &[(b"igno", false), (b"dnat", true)];
    assert_eq!(masks(0x515, with), (0xffff_fff5, 8, !0, 0));
    assert_eq!(masks(0x726, &[(b"igno", true)]), none);
    assert_eq!(
        masks(0x726, &[(b"user", true)]),
        (!0, 0, !0x800_0000, 0x800_0000)
    );
    assert_eq!(masks(0x726, &[(b"user", false)]), (!0, 0, !0x800_0000, 0));
}
