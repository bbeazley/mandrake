#[test]
fn custom_colors_need_a_hue_change_to_replace() {
    use mandrake_sim::players::colors_replace_core;
    let mut core = vec![0u8; 0x180];
    core[..4].copy_from_slice(b"test");
    core[5] = 1;
    let on = [1u16, 2, 3, 0];
    let off = [1u16, 2, 3, 4];
    assert!(colors_replace_core(&core, &[1], &[on, off]));
    assert!(!colors_replace_core(&core, &[1], &[off, on]));
    assert!(colors_replace_core(&core, &[2], &[off, on]));
    assert!(!colors_replace_core(&core, &[2], &[on, off]));
    assert!(!colors_replace_core(&core, &[0], &[on, on]));
    assert!(!colors_replace_core(&core, &[], &[on, on]));
    core[0x0f] = 4;
    assert!(!colors_replace_core(&core, &[1], &[on, on]));
    core[0x0f] = 0;
    core[0x153] = 1;
    assert!(!colors_replace_core(&core, &[1], &[on, on]));
}
