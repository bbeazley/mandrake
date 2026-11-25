use mandrake_sim::engine_attack::{ThrownSolve, thrown_solve};

#[test]
fn the_rule_at_what_the_film_does_not_run() {
    let (own, kp) = ([0, 0, 0x400], [10, 0, 0x400]);
    let solve = |q, flags, gravity| thrown_solve(own, kp, q, flags, 82, 256, gravity);
    assert_eq!(solve([0x7ff, 0, 0], 0x8000, -2), ThrownSolve::Ballistic);
    assert_ne!(solve([0x7ff, 0, 0x100], 0x8000, -2), ThrownSolve::Ballistic);
    assert_ne!(solve([0x800, 0, 0], 0x8000, -2), ThrownSolve::Ballistic);
    assert_ne!(solve([0x7ff, 0, 0], 0, -2), ThrownSolve::Ballistic);
    let level = solve([0x500, 0, 0x400], 0, 0);
    assert!(matches!(level, ThrownSolve::OutOfReach { elevation } if elevation == 0x2000));
    assert!(matches!(
        thrown_solve([0, 0, 0x400], [0, 0, 0x400], [2, 0, 0x400], 0, 82, 256, -2),
        ThrownSolve::OutOfReach { elevation: 0x2000 }
    ));
    assert!(matches!(
        solve([0x500, 0, 0x8000], 0, -2),
        ThrownSolve::OutOfReach { .. }
    ));
}
