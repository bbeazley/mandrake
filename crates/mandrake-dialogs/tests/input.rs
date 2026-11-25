use mandrake_common::FourCC;
use mandrake_dialogs::EngineDialogs;
use mandrake_dialogs::core::{Dialog, DialogStack, Hit, dialog_flags};
use mandrake_dialogs::resources::Resources;
use mandrake_session::{
    Action, FrameInput, FrameOutput, FrontEnd, MainMenu, MenuTrack, Screen, SoundCue, UiEvent,
    UiKey,
};
use mandrake_tag::{DialogDefinition, Rect};

fn bina(items: &[(&[u8; 4], u16, Rect)]) -> Vec<u8> {
    let mut d = vec![
        0,
        1,
        0,
        items.len() as u8,
        0,
        0,
        0,
        0,
        0x01,
        0xe0,
        0x02,
        0x80,
    ];
    for (kind, id, r) in items {
        d.extend_from_slice(*kind);
        d.extend_from_slice(&[0, 0]);
        d.extend_from_slice(&id.to_be_bytes());
        d.extend_from_slice(&[0xff, 0xff, 0xff, 0xff, 0, 0, 0, 0]);
        for v in [r.top, r.left, r.bottom, r.right] {
            d.extend_from_slice(&(v as i16).to_be_bytes());
        }
    }
    d.extend(std::iter::repeat_n(0, items.len()));
    d
}

fn imm6() -> Vec<u8> {
    bina(&[
        (b"bitm", 1, Rect::new(0, 16384, 480, 17024)),
        (b"butt", 2, Rect::new(224, 153, 252, 280)),
        (b"butt", 3, Rect::new(252, 111, 280, 324)),
        (b"butt", 4, Rect::new(280, 141, 307, 293)),
        (b"butt", 5, Rect::new(335, 146, 362, 288)),
        (b"butt", 6, Rect::new(307, 162, 335, 271)),
        (b"butt", 7, Rect::new(390, 140, 418, 294)),
        (b"butt", 8, Rect::new(362, 144, 390, 290)),
        (b"butt", 9, Rect::new(418, 185, 450, 248)),
    ])
}

fn resources(tag: Vec<u8>) -> Resources {
    Resources::from_source(move |group, _| (group == FourCC(*b"bina")).then(|| tag.clone()))
}

const NEW_GAME: (i32, i32) = (200, 238);
const MULTIPLAYER: (i32, i32) = (200, 266);
const PREFERENCES: (i32, i32) = (200, 376);
const INTRO: (i32, i32) = (200, 404);
const QUIT: (i32, i32) = (200, 434);
const OUTSIDE: (i32, i32) = (600, 100);

fn menu() -> MainMenu {
    MainMenu {
        new_game: true,
        tutorial: true,
        preferences: true,
        ..MainMenu::default()
    }
}

struct Rig {
    front_end: EngineDialogs,
    menu: MainMenu,
    now_ms: u64,
    shift_held: bool,
}

impl Rig {
    fn new() -> Self {
        let mut rig = Self {
            front_end: EngineDialogs::with_resources(resources(imm6())),
            menu: menu(),
            now_ms: 0,
            shift_held: false,
        };
        rig.frame(&[]);
        rig
    }

    fn frame(&mut self, events: &[UiEvent]) -> FrameOutput {
        let input = FrameInput {
            now_ms: self.now_ms,
            size: (640, 480),
            events,
            sound_busy: false,
            shift_held: self.shift_held,
        };
        self.front_end.frame(&Screen::MainMenu(&self.menu), &input)
    }

    fn run(&mut self, events: &[UiEvent]) -> Vec<Action> {
        let mut actions = self.frame(events).actions;
        for _ in 0..5 {
            self.now_ms += 34;
            actions.extend(self.frame(&[]).actions);
        }
        actions
    }
}

fn to((x, y): (i32, i32)) -> UiEvent {
    UiEvent::PointerMoved { x, y }
}

const DOWN: UiEvent = UiEvent::Button {
    left: true,
    pressed: true,
};
const UP: UiEvent = UiEvent::Button {
    left: true,
    pressed: false,
};

fn key(key: UiKey, alt: bool) -> UiEvent {
    UiEvent::Key {
        key,
        pressed: true,
        repeat: false,
        shift: false,
        alt,
    }
}

#[test]
fn a_press_and_release_inside_is_the_action_after_a_tick() {
    let mut rig = Rig::new();
    let out = rig.frame(&[to(NEW_GAME), DOWN, UP]);
    assert!(out.actions.is_empty(), "the press runs out first");
    rig.now_ms += 34;
    assert_eq!(
        rig.frame(&[]).actions,
        [Action::NewGame { all_levels: false }]
    );
    rig.now_ms += 34;
    assert!(rig.frame(&[]).actions.is_empty(), "only once");
}

#[test]
fn shift_at_that_moment_asks_for_all_levels() {
    let mut rig = Rig::new();
    rig.shift_held = true;
    assert_eq!(
        rig.run(&[to(NEW_GAME), DOWN, UP]),
        [Action::NewGame { all_levels: true }]
    );
}

#[test]
fn a_release_outside_is_nothing() {
    let mut rig = Rig::new();
    assert!(rig.run(&[to(NEW_GAME), DOWN, to(OUTSIDE), UP]).is_empty());
}

#[test]
fn it_follows_the_mouse_out_and_back_in() {
    let mut rig = Rig::new();
    rig.frame(&[to(NEW_GAME), DOWN]);
    let pressed = |rig: &Rig| {
        rig.front_end
            .stack()
            .top()
            .unwrap()
            .control(2)
            .unwrap()
            .item
            .pressed
    };
    assert!(pressed(&rig));
    rig.frame(&[to(OUTSIDE)]);
    assert!(!pressed(&rig));
    rig.frame(&[to(NEW_GAME)]);
    assert!(pressed(&rig));
    assert_eq!(rig.run(&[UP]), [Action::NewGame { all_levels: false }]);
}

#[test]
fn a_press_that_begins_outside_never_takes() {
    let mut rig = Rig::new();
    assert!(rig.run(&[to(OUTSIDE), DOWN, to(NEW_GAME), UP]).is_empty());
    assert!(rig.run(&[to(QUIT), DOWN, to(NEW_GAME), UP]).is_empty());
}

#[test]
fn a_disabled_button_takes_nothing() {
    let mut rig = Rig::new();
    assert!(rig.run(&[to(MULTIPLAYER), DOWN, UP]).is_empty());
    assert!(rig.run(&[key(UiKey::Char('g'), true)]).is_empty());
    let d = rig.front_end.stack().top().unwrap();
    assert!(!d.control(3).unwrap().item.enabled());
    assert!(!d.control(3).unwrap().item.pressed);
    assert!(!d.is_tracking());
}

#[test]
fn enabled_follows_the_session_and_drops_a_press() {
    let mut rig = Rig::new();
    rig.frame(&[to(NEW_GAME), DOWN]);
    rig.menu.new_game = false;
    rig.frame(&[]);
    let d = rig.front_end.stack().top().unwrap();
    assert!(!d.is_tracking());
    assert!(!d.control(2).unwrap().item.pressed);
    assert!(rig.run(&[UP]).is_empty());
    rig.menu.new_game = true;
    rig.frame(&[]);
    assert_eq!(
        rig.run(&[DOWN, UP]),
        [Action::NewGame { all_levels: false }]
    );
}

#[test]
fn a_working_button_with_no_action_yet_is_only_logged() {
    let mut rig = Rig::new();
    assert!(rig.run(&[to(PREFERENCES), DOWN, UP]).is_empty());
    assert!(rig.run(&[key(UiKey::Char('p'), true)]).is_empty());
}

#[test]
fn replay_intro_follows_the_film_and_asks_for_it() {
    let mut rig = Rig::new();
    assert!(rig.run(&[to(INTRO), DOWN, UP]).is_empty(), "no film yet");
    assert!(rig.run(&[key(UiKey::Char('i'), true)]).is_empty());
    rig.menu.intro = true;
    rig.frame(&[]);
    assert_eq!(rig.run(&[DOWN, UP]), [Action::ReplayIntro]);
    assert_eq!(
        rig.run(&[key(UiKey::Char('i'), true)]),
        [Action::ReplayIntro]
    );
}

#[test]
fn the_cancel_item_takes_escape_and_alt_period() {
    let mut rig = Rig::new();
    assert_eq!(rig.run(&[key(UiKey::Escape, false)]), [Action::Quit]);
    assert_eq!(rig.run(&[key(UiKey::Char('.'), true)]), [Action::Quit]);
    assert!(rig.run(&[key(UiKey::Char('.'), false)]).is_empty());
}

#[test]
fn alt_and_a_letter_press_a_button() {
    let mut rig = Rig::new();
    assert_eq!(
        rig.run(&[key(UiKey::Char('n'), true)]),
        [Action::NewGame { all_levels: false }]
    );
    assert_eq!(rig.run(&[key(UiKey::Char('t'), true)]), [Action::Tutorial]);
    assert_eq!(rig.run(&[key(UiKey::Char('q'), true)]), [Action::Quit]);
    assert!(
        rig.run(&[key(UiKey::Char('q'), false)]).is_empty(),
        "no Alt"
    );
    assert!(rig.run(&[key(UiKey::Char('x'), true)]).is_empty());
    assert!(
        rig.run(&[key(UiKey::Return, false)]).is_empty(),
        "no default"
    );
}

#[test]
fn a_key_release_or_a_repeat_presses_nothing() {
    let mut rig = Rig::new();
    let release = UiEvent::Key {
        key: UiKey::Escape,
        pressed: false,
        repeat: false,
        shift: false,
        alt: false,
    };
    let repeat = UiEvent::Key {
        key: UiKey::Escape,
        pressed: true,
        repeat: true,
        shift: false,
        alt: false,
    };
    assert!(rig.run(&[release, repeat]).is_empty());
}

#[test]
fn input_waits_while_a_press_runs_out() {
    let mut rig = Rig::new();
    assert_eq!(
        rig.run(&[to(NEW_GAME), DOWN, UP, key(UiKey::Escape, false), DOWN, UP]),
        [Action::NewGame { all_levels: false }]
    );
}

#[test]
fn a_click_on_the_version_text_does_nothing() {
    let mut rig = Rig::new();
    assert!(rig.run(&[to((100, 8)), DOWN, UP]).is_empty());
}

#[test]
fn the_pointer_maps_through_the_fit() {
    let mut rig = Rig::new();
    let out = {
        let events = [UiEvent::PointerMoved { x: 400, y: 476 }, DOWN, UP];
        let input = FrameInput {
            now_ms: 0,
            size: (1280, 960),
            events: &events,
            sound_busy: false,
            shift_held: false,
        };
        rig.front_end.frame(&Screen::MainMenu(&rig.menu), &input);
        rig.now_ms = 34;
        let input = FrameInput {
            now_ms: 34,
            size: (1280, 960),
            events: &[],
            sound_busy: false,
            shift_held: false,
        };
        rig.front_end.frame(&Screen::MainMenu(&rig.menu), &input)
    };
    assert_eq!(out.actions, [Action::NewGame { all_levels: false }]);
}

#[test]
fn music_begins_when_nothing_is_playing() {
    let track = MenuTrack {
        tag: FourCC(*b"mamm"),
        weight: 20,
    };
    let mut rig = Rig::new();
    rig.menu.music = vec![track];
    let mut frame = |busy: bool| {
        let input = FrameInput {
            now_ms: rig.now_ms,
            size: (640, 480),
            events: &[],
            sound_busy: busy,
            shift_held: false,
        };
        rig.front_end.frame(&Screen::MainMenu(&rig.menu), &input)
    };
    assert_eq!(frame(false).sounds, [SoundCue::Music(track.tag)]);
    assert!(frame(true).sounds.is_empty());
    assert!(frame(true).sounds.is_empty());
    assert_eq!(
        frame(false).sounds,
        [SoundCue::Music(track.tag)],
        "the only track, again, when it ends"
    );
}

#[test]
fn a_change_of_screen_closes_the_dialog() {
    let mut rig = Rig::new();
    assert!(!rig.front_end.stack().is_empty());
    let input = FrameInput {
        now_ms: 0,
        size: (640, 480),
        events: &[],
        sound_busy: false,
        shift_held: false,
    };
    rig.front_end.frame(&Screen::Quit, &input);
    assert!(rig.front_end.stack().is_empty());
    assert!(
        rig.front_end
            .layers()
            .iter()
            .all(|l| { l.bitmap.pixels.iter().all(|&p| p == 0xff00_0000) })
    );
}

const ID: FourCC = FourCC(*b"test");

fn dialog(items: &[(&[u8; 4], u16, Rect)], res: &mut Resources) -> Dialog {
    let def = DialogDefinition::from_bytes(&bina(items)).unwrap();
    Dialog::new(
        &def,
        ID,
        dialog_flags::MODAL | dialog_flags::NO_BACKGROUND,
        res,
    )
}

fn stack_of(dialog: Dialog) -> DialogStack {
    let mut stack = DialogStack::new();
    stack.place((640, 480));
    stack.push(dialog);
    stack
}

fn press(stack: &mut DialogStack, event: UiEvent) -> Vec<i16> {
    let mut hits = Vec::new();
    stack.event(&event, &mut hits);
    stack.tick(5, 5, &mut hits);
    hits.iter().map(|h: &Hit| h.item).collect()
}

const A: Rect = Rect::new(10, 10, 30, 70);
const B: Rect = Rect::new(40, 10, 60, 70);
const C: Rect = Rect::new(70, 10, 90, 70);

#[test]
fn default_and_cancel_keys() {
    let mut res = resources(Vec::new());
    let mut d = dialog(&[(b"butt", 1, A), (b"butt", 2, B)], &mut res);
    d.set_default_item(1);
    d.set_cancel_item(2);
    let mut stack = stack_of(d);
    assert_eq!(press(&mut stack, key(UiKey::Return, false)), [1]);
    assert_eq!(press(&mut stack, key(UiKey::Escape, false)), [2]);
    stack.top_mut().unwrap().set_enabled(1, false);
    assert!(press(&mut stack, key(UiKey::Return, false)).is_empty());
    assert_eq!(press(&mut stack, key(UiKey::Char('.'), true)), [2]);
}

#[test]
fn a_new_default_loses_its_key_as_in_the_engine() {
    let mut res = resources(Vec::new());
    let mut d = dialog(&[(b"butt", 1, A), (b"butt", 2, B)], &mut res);
    d.set_keyboard_equivalent(1, 'k', false);
    d.set_keyboard_equivalent(2, 'j', false);
    d.set_default_item(1);
    d.set_default_item(2);
    let mut stack = stack_of(d);
    assert_eq!(press(&mut stack, key(UiKey::Char('k'), false)), [1]);
    assert!(press(&mut stack, key(UiKey::Char('j'), false)).is_empty());
    assert_eq!(press(&mut stack, key(UiKey::Return, false)), [2]);
}

#[test]
fn tab_walks_what_takes_focus_in_creation_order() {
    let mut res = resources(Vec::new());
    let d = dialog(
        &[
            (b"edit", 1, A),
            (b"butt", 2, B),
            (b"lstm", 3, C),
            (b"stat", 4, Rect::new(100, 0, 120, 60)),
            (b"edit", 5, Rect::new(130, 0, 150, 60)),
        ],
        &mut res,
    );
    assert_eq!(d.focus(), 5, "the newest takes the first focus");
    let mut stack = stack_of(d);
    let tab = |shift| UiEvent::Key {
        key: UiKey::Tab,
        pressed: true,
        repeat: false,
        shift,
        alt: false,
    };
    let mut seen = Vec::new();
    for _ in 0..4 {
        press(&mut stack, tab(false));
        seen.push(stack.top().unwrap().focus());
    }
    assert_eq!(seen, [1, 3, 5, 1], "buttons and text don't take focus");
    press(&mut stack, tab(true));
    assert_eq!(stack.top().unwrap().focus(), 5, "Shift goes back");
}

#[test]
fn tab_with_nothing_focusable_stays_put() {
    let mut res = resources(Vec::new());
    let d = dialog(&[(b"butt", 1, A), (b"chek", 2, B)], &mut res);
    assert_eq!(d.focus(), -1);
    let mut stack = stack_of(d);
    assert!(press(&mut stack, key(UiKey::Tab, false)).is_empty());
    assert_eq!(stack.top().unwrap().focus(), -1);
}

#[test]
fn hidden_items_take_nothing_and_the_earliest_item_wins_a_point() {
    let mut res = resources(Vec::new());
    let mut d = dialog(
        &[(b"butt", 1, A), (b"butt", 2, A), (b"butt", 3, B)],
        &mut res,
    );
    d.set_keyboard_equivalent(3, 'h', true);
    d.set_visible(3, false);
    let mut stack = stack_of(d);
    let click = |stack: &mut DialogStack, at: (i32, i32)| {
        let mut hits = Vec::new();
        stack.event(&to(at), &mut hits);
        stack.event(&DOWN, &mut hits);
        stack.event(&UP, &mut hits);
        stack.tick(5, 5, &mut hits);
        hits.iter().map(|h| h.item).collect::<Vec<_>>()
    };
    assert_eq!(click(&mut stack, (20, 20)), [1]);
    assert!(click(&mut stack, (20, 50)).is_empty());
    assert!(press(&mut stack, key(UiKey::Char('h'), true)).is_empty());
    stack.top_mut().unwrap().set_visible(1, false);
    assert_eq!(click(&mut stack, (20, 20)), [2]);
}

#[test]
fn only_the_newest_dialog_takes_input() {
    let mut res = resources(Vec::new());
    let mut under = dialog(&[(b"butt", 1, A)], &mut res);
    under.set_cancel_item(1);
    let mut stack = stack_of(under);
    let def = DialogDefinition::from_bytes(&bina(&[(b"butt", 7, B)])).unwrap();
    let mut over = Dialog::new(
        &def,
        FourCC(*b"over"),
        dialog_flags::NO_BACKGROUND,
        &mut res,
    );
    over.set_cancel_item(7);
    stack.push(over);
    let mut hits = Vec::new();
    stack.event(&key(UiKey::Escape, false), &mut hits);
    stack.tick(5, 5, &mut hits);
    assert_eq!(
        hits,
        [Hit {
            dialog: FourCC(*b"over"),
            item: 7
        }]
    );
    stack.pop();
    assert_eq!(press(&mut stack, key(UiKey::Escape, false)), [1]);
}

#[test]
fn a_centred_dialog_takes_its_clicks_where_it_sits() {
    let mut res = resources(Vec::new());
    let mut def = DialogDefinition::from_bytes(&bina(&[(b"butt", 1, A)])).unwrap();
    def.rect = Rect::new(0, 0, 100, 200);
    let d = Dialog::new(
        &def,
        ID,
        dialog_flags::MODAL | dialog_flags::NO_BACKGROUND,
        &mut res,
    );
    let mut stack = stack_of(d);
    assert_eq!(stack.top().unwrap().bounds(), Rect::new(190, 220, 290, 420));
    let mut hits = Vec::new();
    stack.event(&to((220 + 20, 190 + 20)), &mut hits);
    stack.event(&DOWN, &mut hits);
    stack.event(&UP, &mut hits);
    stack.tick(5, 5, &mut hits);
    assert_eq!(hits.len(), 1);
}

#[test]
fn stack_clear_and_visibility() {
    let mut res = resources(Vec::new());
    let mut stack = stack_of(dialog(&[(b"butt", 1, A)], &mut res));
    assert_eq!(stack.iter().count(), 1);
    stack.clear();
    assert!(stack.is_empty());
    assert!(press(&mut stack, key(UiKey::Escape, false)).is_empty());
}
