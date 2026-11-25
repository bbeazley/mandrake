use mandrake_sim::map_actions::{SubjectFacts, SubjectThresholds};

const GAMEPLAY: i32 = 0x889;

fn read(names: &[(&[u8; 4], i16, i32)], mode: i16, gameplay: i32) -> Option<SubjectThresholds> {
    SubjectThresholds::read(
        mode,
        gameplay,
        |n| names.iter().any(|(m, k, _)| *m == n && *k == 0),
        |n, kind| {
            names
                .iter()
                .find(|(m, k, _)| *m == n && *k == kind)
                .map(|(_, _, v)| *v)
        },
    )
}

fn subject() -> SubjectFacts {
    SubjectFacts {
        ammunition: 1,
        experience: 0,
        flying_height: 0,
        mana: 0,
        max_mana: 0,
        charges: 0,
        selected: false,
        mode: 1,
    }
}

#[test]
fn only_the_first_threshold_found_is_read() {
    assert_eq!(read(&[], 2, GAMEPLAY), None);
    assert_eq!(read(&[(b"zzzz", 5, 3)], 2, GAMEPLAY), None);
    assert_eq!(read(&[(b"sele", 0, 0)], 0, GAMEPLAY), None);
    assert_eq!(read(&[(b"sele", 5, 0)], 2, GAMEPLAY), None);
    assert_eq!(read(&[(b"taun", 5, 0)], 2, GAMEPLAY), None);
    assert!(read(&[(b"sele", 0, 0)], 2, GAMEPLAY).unwrap().selected);
    assert!(
        !read(&[(b"sele", 0, 0), (b"ammo", 5, 0)], 1, GAMEPLAY)
            .unwrap()
            .selected
    );
    let t = read(&[(b"taun", 0, 0)], 0, GAMEPLAY).unwrap();
    assert!(t.taunting);
    assert_eq!((t.ammunition, t.mana, t.experience), (-1, -1, -1));
    let all = [
        (b"hei>", 5, 6),
        (b"chr>", 5, 5),
        (b"exp>", 5, 4),
        (b"mana", 8, 3),
        (b"ammo", 5, 2),
    ];
    let t = read(&all, 2, GAMEPLAY).unwrap();
    assert_eq!((t.ammunition, t.mana, t.experience), (2, -1, -1));
    assert_eq!((t.charges, t.height), (None, None));
    let t = read(&all[..4], 2, GAMEPLAY).unwrap();
    assert_eq!((t.ammunition, t.mana, t.experience), (-1, 3, -1));
    let t = read(&all[..3], 2, GAMEPLAY).unwrap();
    assert_eq!((t.mana, t.experience, t.charges), (-1, 4, None));
    let t = read(&all[..2], 2, GAMEPLAY).unwrap();
    assert_eq!((t.experience, t.charges, t.height), (-1, Some(5), None));
    let t = read(&all[..1], 2, GAMEPLAY).unwrap();
    assert_eq!((t.charges, t.height), (None, Some(6)));
    assert_eq!(read(&all[..2], 2, 0x5e6), None);
    assert_eq!(read(&[(b"ammo", 8, 2)], 2, GAMEPLAY), None);
    assert_eq!(read(&[(b"mana", 5, 2)], 2, GAMEPLAY), None);
}

#[test]
fn every_slot_is_compared_at_or_below() {
    let base = read(&[(b"ammo", 5, 0)], 2, GAMEPLAY).unwrap();
    let pass = |t: &SubjectThresholds, s: SubjectFacts| t.pass(&[s], GAMEPLAY);
    for (a, ok) in [
        (-128, false),
        (-1, false),
        (0, false),
        (1, true),
        (127, true),
    ] {
        let s = SubjectFacts {
            ammunition: a,
            ..subject()
        };
        assert_eq!(pass(&base, s), ok, "ammo {a}");
    }
    let low = SubjectFacts {
        ammunition: 0,
        ..subject()
    };
    assert!(!base.pass(&[subject(), low], GAMEPLAY));
    let t = read(&[(b"exp>", 5, 2)], 2, GAMEPLAY).unwrap();
    let neg = SubjectFacts {
        ammunition: -1,
        experience: 9,
        ..subject()
    };
    assert!(!pass(&t, neg));
    for (e, ok) in [(0, false), (2, false), (3, true), (255, true)] {
        let s = SubjectFacts {
            experience: e,
            ..subject()
        };
        assert_eq!(pass(&t, s), ok, "exp {e}");
    }
    let t = read(&[(b"mana", 8, 0x8000)], 2, GAMEPLAY).unwrap();
    for (m, max, ok) in [
        (0x40, 0x80, false),
        (0x41, 0x80, true),
        (-5, 0, true),
        (5, -10, false),
    ] {
        let s = SubjectFacts {
            mana: m,
            max_mana: max,
            ..subject()
        };
        assert_eq!(pass(&t, s), ok, "mana {m}/{max}");
    }
    let s = SubjectFacts {
        mana: -1,
        max_mana: 0x80,
        ..subject()
    };
    assert!(!pass(&base, s));
    let t = read(&[(b"chr>", 5, 0)], 2, GAMEPLAY).unwrap();
    assert!(!pass(&t, subject()));
    let s = SubjectFacts {
        charges: 1,
        ..subject()
    };
    assert!(pass(&t, s));
    let t = read(&[(b"hei>", 5, 10)], 2, GAMEPLAY).unwrap();
    for (h, ok) in [(-5, false), (10, false), (11, true)] {
        let s = SubjectFacts {
            flying_height: h,
            ..subject()
        };
        assert_eq!(pass(&t, s), ok, "hei {h}");
    }
    let s = SubjectFacts {
        flying_height: -5,
        ..subject()
    };
    assert!(pass(&base, s));
    let t = read(&[(b"sele", 0, 0)], 2, GAMEPLAY).unwrap();
    assert!(!pass(&t, subject()));
    let sel = SubjectFacts {
        selected: true,
        ..subject()
    };
    assert!(pass(&t, sel));
    assert!(!t.pass(&[sel, subject()], GAMEPLAY));
    let t = read(&[(b"taun", 0, 0)], 2, GAMEPLAY).unwrap();
    let taunting = SubjectFacts {
        mode: 3,
        ..subject()
    };
    assert!(!pass(&t, subject()));
    assert!(t.pass(&[subject(), taunting], GAMEPLAY));
    assert!(!base.pass(&[], GAMEPLAY));
    assert!(base.pass(&[], 0x6ae));
    assert!(!t.pass(&[], 0x6ae));
}
