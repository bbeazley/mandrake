mod common;

use common::{ALTERNATE, BREACH, BROKEN_PATH, DRINKING, LOSS, WIN, entry, level};
use mandrake_common::FourCC;
use mandrake_session::PostgameExits;
use mandrake_session::catalog::{Level, ListEntry};
use mandrake_session::progress::VisitedLevel;
use mandrake_session::rules::{
    ENTRY_EXTRA_ALTERNATE, ENTRY_LISTED, ENTRY_OPEN_HIDDEN, ENTRY_SOLO, LevelList,
    MESH_CAMPAIGN_END, PostgameChoice, PostgameRule, TUTORIAL, has_briefing, level_list, postgame,
};

const A: FourCC = FourCC(*b"aaaa");
const B: FourCC = FourCC(*b"bbbb");
const C: FourCC = FourCC(*b"cccc");
const LISTED: u32 = ENTRY_SOLO | ENTRY_LISTED;

fn seen(meshes: &[FourCC]) -> Vec<VisitedLevel> {
    meshes
        .iter()
        .map(|&mesh| VisitedLevel {
            mesh,
            plugin_checksum: -1,
        })
        .collect()
}

fn rows(list: &LevelList) -> Vec<(FourCC, bool)> {
    list.levels.iter().map(|l| (l.mesh, l.selectable)).collect()
}

fn alternate(mesh: FourCC, order: i16) -> ListEntry {
    ListEntry {
        extra_flags: ENTRY_EXTRA_ALTERNATE,
        ..entry(mesh, order, LISTED)
    }
}

#[test]
fn entry_with_neither_flag_is_listed_only_with_shift() {
    let entries = [entry(B, 201, LISTED), entry(A, 200, ENTRY_SOLO)];
    assert_eq!(rows(&level_list(&entries, &[], None, false)), [(B, true)]);
    assert_eq!(
        rows(&level_list(&entries, &[], None, true)),
        [(A, true), (B, true)]
    );
}

#[test]
fn entry_with_the_hidden_flag_is_listed_only_with_shift_even_with_the_listed_flag() {
    let entries = [
        entry(B, 201, LISTED),
        entry(A, 300, ENTRY_SOLO | ENTRY_OPEN_HIDDEN),
        entry(C, 301, LISTED | ENTRY_OPEN_HIDDEN),
    ];
    assert_eq!(rows(&level_list(&entries, &[], None, false)), [(B, true)]);
    assert_eq!(
        rows(&level_list(&entries, &[], None, true)),
        [(B, true), (A, true), (C, true)]
    );
}

#[test]
fn entry_with_the_listed_flag_is_listed_either_way() {
    let entries = [entry(A, 200, LISTED)];
    assert_eq!(rows(&level_list(&entries, &[], None, false)), [(A, true)]);
    assert_eq!(rows(&level_list(&entries, &[], None, true)), [(A, true)]);
}

#[test]
fn entry_outside_solos_mask_is_never_listed() {
    let entries = [entry(A, 200, ENTRY_LISTED)];
    assert!(level_list(&entries, &[], None, false).levels.is_empty());
    assert!(level_list(&entries, &[], None, true).levels.is_empty());
}

#[test]
fn a_listed_levels_state_without_shift() {
    let entries = [
        entry(A, 200, LISTED),
        entry(B, 202, LISTED),
        entry(C, 203, LISTED),
        entry(FourCC(*b"o101"), 301, LISTED),
        entry(FourCC(*b"curr"), 205, LISTED),
    ];
    let list = level_list(&entries, &seen(&[B]), Some(FourCC(*b"curr")), false);
    assert_eq!(
        rows(&list),
        [
            (A, true),
            (B, true),
            (C, false),
            (FourCC(*b"curr"), true),
            (FourCC(*b"o101"), true),
        ]
    );
}

#[test]
fn a_tie_for_the_lowest_order_opens_both() {
    let entries = [entry(A, 200, LISTED), entry(B, 200, LISTED)];
    let list = level_list(&entries, &[], None, false);
    assert!(list.levels.iter().all(|l| l.selectable));
}

#[test]
fn a_hidden_entry_doesnt_count_for_the_lowest_order() {
    let entries = [
        entry(A, 100, ENTRY_SOLO | ENTRY_OPEN_HIDDEN),
        entry(B, 202, LISTED),
        entry(C, 203, LISTED),
    ];
    let list = level_list(&entries, &[], None, false);
    assert_eq!(rows(&list), [(B, true), (C, false)]);
}

#[test]
fn shift_opens_every_listed_level() {
    let entries = [
        entry(A, 200, LISTED),
        entry(B, 202, LISTED),
        entry(C, 203, ENTRY_SOLO),
    ];
    let list = level_list(&entries, &[], None, true);
    assert_eq!(rows(&list), [(A, true), (B, true), (C, true)]);
}

#[test]
fn alternates_need_a_visit_or_the_current_choice_and_shift_doesnt_reveal_them() {
    for alt in [
        alternate(C, 205),
        entry(FourCC(*b"03hp"), 205, LISTED),
        entry(FourCC(*b"21md"), 205, LISTED),
    ] {
        let mesh = alt.mesh;
        let entries = [entry(A, 200, LISTED), alt];
        for all_levels in [false, true] {
            assert_eq!(
                rows(&level_list(&entries, &[], None, all_levels)),
                [(A, true)],
                "{mesh:?} unvisited, shift {all_levels}"
            );
            assert_eq!(
                rows(&level_list(&entries, &seen(&[mesh]), None, all_levels)),
                [(A, true), (mesh, true)],
                "{mesh:?} visited, shift {all_levels}"
            );
            assert_eq!(
                rows(&level_list(&entries, &[], Some(mesh), all_levels)),
                [(A, true), (mesh, true)],
                "{mesh:?} current, shift {all_levels}"
            );
        }
    }
}

#[test]
fn a_visit_counts_only_with_the_entrys_own_plugin() {
    let entries = [
        entry(A, 200, LISTED),
        ListEntry {
            plugin_checksum: 77,
            ..entry(B, 202, LISTED)
        },
    ];
    let other_plugin = seen(&[B]);
    assert_eq!(
        rows(&level_list(&entries, &other_plugin, None, false)),
        [(A, true), (B, false)]
    );
    let same_plugin = [VisitedLevel {
        mesh: B,
        plugin_checksum: 77,
    }];
    assert_eq!(
        rows(&level_list(&entries, &same_plugin, None, false)),
        [(A, true), (B, true)]
    );
}

#[test]
fn the_list_is_in_order_whatever_the_catalogs() {
    let entries = [
        entry(C, 203, LISTED),
        entry(A, 200, LISTED),
        entry(B, 202, LISTED),
    ];
    let meshes: Vec<_> = level_list(&entries, &[], None, false)
        .levels
        .iter()
        .map(|l| l.mesh)
        .collect();
    assert_eq!(meshes, [A, B, C]);
}

#[test]
fn the_selection_is_the_current_choice_or_else_the_lowest_order_or_else_the_first_row() {
    let entries = [entry(A, 200, LISTED), entry(B, 202, LISTED)];
    assert_eq!(level_list(&entries, &[], Some(B), false).selected, Some(B));
    assert_eq!(level_list(&entries, &[], None, false).selected, Some(A));
    assert_eq!(
        level_list(&entries, &[], Some(FourCC(*b"gone")), false).selected,
        Some(A)
    );

    let all_hidden = [
        entry(B, 301, ENTRY_SOLO | ENTRY_OPEN_HIDDEN),
        entry(A, 300, ENTRY_SOLO | ENTRY_OPEN_HIDDEN),
    ];
    assert_eq!(level_list(&all_hidden, &[], None, true).selected, Some(A));
    assert_eq!(level_list(&all_hidden, &[], None, false).selected, None);
}

const POST_ART: FourCC = FourCC(*b"post");
const INGE: FourCC = FourCC(*b"inge");

fn exits(next: bool, restart: bool, last_saved: bool, skip: bool) -> PostgameExits {
    PostgameExits {
        next,
        restart,
        last_saved,
        skip,
        save_film: true,
        quit: true,
    }
}

fn dialog(level: &Level, end_code: i16, can_revert: bool) -> PostgameChoice {
    match postgame(Some(level), end_code, can_revert) {
        PostgameRule::Dialog(choice) => choice,
        other => panic!("no dialog: {other:?}"),
    }
}

fn with_both_links() -> Level {
    Level {
        alternate: Some(DRINKING),
        ..level(BROKEN_PATH, Some(BREACH))
    }
}

fn with_next_link() -> Level {
    level(BROKEN_PATH, Some(BREACH))
}

#[test]
fn a_win_offers_next_and_quit_and_goes_to_the_next_link() {
    let choice = dialog(&with_both_links(), WIN, false);
    assert_eq!(choice.exits, exits(true, false, false, false));
    assert_eq!(choice.next_mesh, Some(BREACH));
    assert_eq!(choice.art, POST_ART);
    assert_eq!(choice.art_sequence, 0);
    assert_eq!(choice.mark_visited, Some(BREACH));
    assert!(!choice.campaign_end);
}

#[test]
fn result_2_with_an_alternate_link_goes_to_it() {
    let choice = dialog(&with_both_links(), ALTERNATE, false);
    assert_eq!(choice.exits, exits(true, false, false, false));
    assert_eq!(choice.next_mesh, Some(DRINKING));
    assert_eq!(choice.art_sequence, 0);
    assert_eq!(choice.mark_visited, None);
}

#[test]
fn result_2_without_one_offers_restart_and_skip() {
    let choice = dialog(&with_next_link(), ALTERNATE, false);
    assert_eq!(choice.exits, exits(false, true, false, true));
    assert_eq!(choice.skip_mesh, Some(BREACH));
    assert_eq!(choice.art_sequence, 1);
    assert_eq!(choice.mark_visited, None);
}

#[test]
fn a_loss_offers_restart_only() {
    let choice = dialog(&with_both_links(), LOSS, false);
    assert_eq!(choice.exits, exits(false, true, false, false));
    assert_eq!(choice.art_sequence, 1);
    assert_eq!(choice.mark_visited, None);
    assert!(!choice.campaign_end);
}

#[test]
fn a_save_adds_last_saved_game_beside_restart() {
    assert_eq!(
        dialog(&with_next_link(), ALTERNATE, true).exits,
        exits(false, true, true, true)
    );
    assert_eq!(
        dialog(&with_next_link(), LOSS, true).exits,
        exits(false, true, true, false)
    );
    assert_eq!(
        dialog(&with_next_link(), WIN, true).exits,
        exits(true, false, false, false)
    );
}

#[test]
fn save_film_is_always_offered() {
    for end_code in [WIN, ALTERNATE, LOSS] {
        assert!(dialog(&with_both_links(), end_code, true).exits.save_film);
    }
}

#[test]
fn the_tutorial_and_a_mesh_that_wont_read_have_no_dialog() {
    let tutorial = level(TUTORIAL, None);
    for end_code in [WIN, LOSS] {
        assert_eq!(
            postgame(Some(&tutorial), end_code, false),
            PostgameRule::ToMenu
        );
        assert_eq!(postgame(None, end_code, false), PostgameRule::ToMenu);
    }
}

#[test]
fn an_end_code_outside_the_table_starts_the_map_again() {
    assert_eq!(
        postgame(Some(&with_next_link()), 0, false),
        PostgameRule::RestartFresh
    );
    assert_eq!(
        postgame(Some(&with_next_link()), 4, false),
        PostgameRule::RestartFresh
    );
}

#[test]
fn next_level_with_no_link_has_nowhere_to_go() {
    let choice = dialog(&level(BREACH, None), WIN, false);
    assert!(choice.exits.next);
    assert_eq!(choice.next_mesh, None);
    assert_eq!(choice.mark_visited, None);
}

#[test]
fn a_map_with_no_post_art_shows_the_fallback_at_sequence_0() {
    let no_art = Level {
        post_art: None,
        ..with_next_link()
    };
    for end_code in [WIN, ALTERNATE, LOSS] {
        let choice = dialog(&no_art, end_code, false);
        assert_eq!((choice.art, choice.art_sequence), (INGE, 0));
    }
}

#[test]
fn the_last_maps_own_art_gives_way_only_on_a_win() {
    let last = Level {
        post_art: Some(FourCC(*b"25po")),
        ..level(FourCC(*b"25so"), None)
    };
    let win = dialog(&last, WIN, false);
    assert_eq!((win.art, win.art_sequence), (INGE, 0));
    let loss = dialog(&last, LOSS, false);
    assert_eq!((loss.art, loss.art_sequence), (FourCC(*b"25po"), 1));
}

#[test]
fn ambient_comes_from_the_mesh_or_else_the_defaults() {
    let plain = with_next_link();
    assert_eq!(dialog(&plain, WIN, false).ambient, FourCC(*b"wisc"));
    assert_eq!(dialog(&plain, LOSS, false).ambient, FourCC(*b"losc"));
    assert_eq!(dialog(&plain, ALTERNATE, false).ambient, FourCC(*b"losc"));
    assert_eq!(
        dialog(&with_both_links(), ALTERNATE, false).ambient,
        FourCC(*b"wisc")
    );

    let own = Level {
        ambient_win: Some(A),
        ambient_loss: Some(B),
        ..with_next_link()
    };
    assert_eq!(dialog(&own, WIN, false).ambient, A);
    assert_eq!(dialog(&own, LOSS, false).ambient, B);
}

#[test]
fn the_campaign_ends_on_the_meshs_flag_or_an_empty_link_in_a_file_of_type_3() {
    let flagged = Level {
        flags: MESH_CAMPAIGN_END,
        ..with_next_link()
    };
    let empty_link = Level {
        file_type: 3,
        ..level(BREACH, None)
    };
    let linked_type_3 = Level {
        file_type: 3,
        ..with_next_link()
    };
    let empty_link_other_file = Level {
        file_type: 2,
        ..level(BREACH, None)
    };
    for (level, ends) in [
        (&flagged, true),
        (&empty_link, true),
        (&linked_type_3, false),
        (&empty_link_other_file, false),
    ] {
        assert_eq!(dialog(level, WIN, false).campaign_end, ends);
        assert_eq!(dialog(level, ALTERNATE, false).campaign_end, ends);
        assert!(!dialog(level, LOSS, false).campaign_end);
    }
}

#[test]
fn next_level_still_shows_after_the_campaigns_end() {
    let last = Level {
        file_type: 3,
        ..level(BREACH, None)
    };
    let choice = dialog(&last, WIN, false);
    assert!(choice.campaign_end);
    assert_eq!(choice.exits, exits(true, false, false, false));
    assert_eq!(choice.next_mesh, None);
}

#[test]
fn a_briefing_needs_art_and_text() {
    let both = level(BREACH, None);
    assert!(has_briefing(&both));
    assert!(has_briefing(&Level {
        narration: None,
        ..both.clone()
    }));
    assert!(!has_briefing(&Level {
        pre_art: None,
        ..both.clone()
    }));
    assert!(!has_briefing(&Level {
        briefing_text: None,
        ..both
    }));
}

#[test]
fn tfls_epilogue_is_myth_iis_with_its_differences() {
    use mandrake_session::rules::{TFL_EPILOGUE, postgame_with};
    let tfl = |level: &Level, code| postgame_with(&TFL_EPILOGUE, Some(level), code, true);
    let PostgameRule::Dialog(win) = tfl(&with_next_link(), WIN) else {
        panic!("a level with art shows the dialog");
    };
    assert!(win.exits.next && win.exits.quit && !win.exits.restart);
    assert_eq!(win.next_mesh, Some(BREACH));
    let PostgameRule::Dialog(loss) = tfl(&with_next_link(), LOSS) else {
        panic!("a loss shows the dialog");
    };
    assert!(loss.exits.quit && loss.exits.restart && loss.exits.last_saved);
    let PostgameRule::Dialog(no_save) =
        postgame_with(&TFL_EPILOGUE, Some(&with_next_link()), LOSS, false)
    else {
        panic!("a loss shows the dialog");
    };
    assert!(no_save.exits.restart && !no_save.exits.last_saved);
    let bare = Level {
        post_art: None,
        ambient_win: None,
        ambient_loss: None,
        ..with_next_link()
    };
    let PostgameRule::Undrawn(choice) = tfl(&bare, WIN) else {
        panic!("no art, no dialog");
    };
    assert_eq!(choice.ambient, FourCC(*b"scwi"));
    assert!(matches!(
        tfl(&level(TUTORIAL, None), LOSS),
        PostgameRule::Undrawn(_) | PostgameRule::Dialog(_)
    ));
}
