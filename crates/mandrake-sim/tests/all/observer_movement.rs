use mandrake_sim::engine_rng::ReplayGame;

#[test]
fn replay_start_solo_flag_options_and_local_player() {
    use mandrake_sim::players::GameSetup;
    assert!(ReplayGame::new(2, 0).solo);
    assert!(ReplayGame::new(1, 0x80).solo);
    let r = ReplayGame::new(1, 0x7f);
    assert_eq!(
        r,
        ReplayGame {
            mode: 1,
            solo: false
        }
    );
    assert_eq!(r.apply_to_options(0xff), 0xfb);
    assert_eq!(ReplayGame::new(2, 0).apply_to_options(0xff), 0xff);
    let mut setup = GameSetup::command_line(0x8c, 0);
    r.apply_to_setup(&mut setup);
    assert_eq!((setup.options, setup.local_player), (0x88, -1));
    let mut setup = GameSetup::command_line(0x8c, 0);
    ReplayGame::new(2, 0x8c).apply_to_setup(&mut setup);
    assert_eq!((setup.options, setup.local_player), (0x8c, 0));
}
