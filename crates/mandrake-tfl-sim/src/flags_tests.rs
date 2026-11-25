use crate::flags::{FlagCount, flag_count_mode};

#[test]
fn the_mode_table_is_tfl_15_s() {
    let modes: Vec<u32> = (0..=11).map(flag_count_mode).collect();
    assert_eq!(modes, [8, 0x1a, 0x1a, 8, 8, 0xe, 0xe, 8, 8, 8, 9, 8]);
}

#[test]
fn the_late_outcomes_by_game_type() {
    use crate::statistics::{LateOutcome, Statistics, Teams};
    let teams = |ty| Teams {
        player_teams: vec![0, 1],
        teams: 2,
        game_type: ty,
        options: mandrake_common::game_options::COOPERATIVE,
        claims: vec![Some(0), Some(1)],
        tick: 100,
        ended: false,
    };
    let fresh = || {
        let mut s = Statistics::default();
        s.set_up_teams(&teams(0), &[0, 0], &[]);
        s.countdown = 0;
        s
    };
    let count = |held: Vec<i16>, settled, unnumbered, numbered| FlagCount {
        held,
        settled,
        unnumbered,
        numbered,
    };
    let mut s = fresh();
    let mut w = -1;
    let out = s
        .update(&teams(7), &[], &mut w, Some(&count(vec![0, 3], true, 3, 0)))
        .unwrap();
    assert_eq!(
        out.late,
        [LateOutcome::Suspend(false), LateOutcome::EndGame]
    );
    assert_eq!(s.teams[0].team, 1);
    assert_eq!(s.teams[0].score, 3);
    let mut s = fresh();
    s.team_data[0].field_18 = 4;
    s.team_data[1].field_18 = 5;
    let out = s
        .update(
            &teams(4),
            &[],
            &mut w,
            Some(&count(vec![0, 0], false, 0, 5)),
        )
        .unwrap();
    assert_eq!(
        out.late,
        [
            LateOutcome::Suspend(true),
            LateOutcome::AboutToWin(0),
            LateOutcome::EndGame
        ]
    );
    assert_eq!(s.team_data[0].flags & 2, 2);
    let mut s = fresh();
    let out = s
        .update(&teams(5), &[], &mut w, Some(&count(vec![0, 2], true, 2, 0)))
        .unwrap();
    assert_eq!(
        out.late,
        [LateOutcome::Suspend(false), LateOutcome::Eliminated(0, 9)]
    );
    let mut s = fresh();
    s.team_data[0].last_taken = 50;
    s.team_data[1].last_taken = 70;
    s.update(&teams(1), &[], &mut w, Some(&count(vec![2, 2], true, 4, 0)))
        .unwrap();
    assert_eq!((s.teams[0].team, s.teams[0].score), (1, 2));
}
