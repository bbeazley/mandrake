use mandrake_common::FourCC;
use mandrake_common::game_options;
use mandrake_session::net::{
    FLAG_PASSWORD, GameDescription, GameOptions, NetMap, NetMapset, NetSavedGame, NetScreen,
    choose_map,
};
use mandrake_session::{Action, Screen};
use mandrake_tag::Rect;

use super::{OpenScreen, ScreenFrame};
use crate::controls::button::button_flags;
use crate::controls::list::{EntryLook, List, list_flags};
use crate::controls::numb::Format;
use crate::core::{Dialog, DialogStack, dialog_flags};
use crate::resources::Resources;
use mandrake_tag::font::{STYLE_PLAIN, text_flags};

pub const NGP: FourCC = FourCC(*b"!ngp");

pub const OK: i16 = 1;
pub const CANCEL: i16 = 2;
pub const COOPERATIVE: i16 = 3;
pub const LEVELS: i16 = 4;
pub const GAME: i16 = 5;
pub const TAGSETS: i16 = 6;
pub const TFL: i16 = 7;
pub const ANTI_CLUMP: i16 = 8;
pub const GAME_TYPE_CAPTION: i16 = 9;
pub const GAME_TYPES: i16 = 10;
const NAME_CAPTION: i16 = 11;
pub const NAME: i16 = 12;
const PASSWORD_CAPTION: i16 = 13;
pub const PASSWORD: i16 = 14;
pub const DIFFICULTIES: i16 = 16;
const PLAYER_LIMIT_CAPTION: i16 = 17;
pub const PLAYER_LIMIT: i16 = 18;
const TIME_LIMIT_CAPTION: i16 = 19;
pub const TIME_LIMIT: i16 = 20;
pub const PLANNING: i16 = 21;
pub const PLANNING_TIME: i16 = 22;
pub const UNIT_TRADING: i16 = 23;
pub const OLD_CLIENTS: i16 = 24;
pub const OBSERVER: i16 = 25;
pub const SHOW_ENEMY: i16 = 26;
pub const VETERANS: i16 = 27;
pub const ALLIANCES: i16 = 28;
pub const TEAMS: i16 = 29;
pub const TEAMS_MAXIMUM: i16 = 30;
pub const TEAMS_COUNT: i16 = 31;
pub const DEATHMATCH: i16 = 32;
pub const NARRATION: i16 = 33;
pub const MAPSETS: i16 = 34;
pub const FILTER: i16 = 35;
pub const PREVIEW: i16 = 37;
pub const ACTIVE_TAGSETS: i16 = 40;
const DIFFICULTY_SELECTOR: i16 = 41;
pub const SPLASH: i16 = 43;
pub const LOAD_GAME: i16 = 1000;
pub const CLEAR: i16 = 0x3e9;
pub const SAVE_SNAPSHOT: i16 = 0x3ea;
pub const SAVE_NAME: i16 = 0x3eb;
pub const SAVE_CAPTION: i16 = 0x3ec;
const SAVE_CAPTION_MIN_WIDTH: i32 = 0x1e0;
const DROPS_SAVE: u64 = 0x4_2a01_01d8;

pub fn saved_game_rects(caption: Rect, list: Rect) -> [(i16, Rect); 5] {
    let clear = Rect::new(
        list.bottom - 26,
        list.left + 6,
        list.bottom - 6,
        list.right - 6,
    );
    let load = Rect::new(
        clear.top - 0x1c,
        clear.left,
        clear.bottom - 0x1c,
        clear.right,
    );
    let name = Rect::new(
        load.bottom - 0x28,
        load.left,
        load.bottom - 0x18,
        load.right,
    );
    let snapshot = Rect::new(list.top + 12, name.left + 6, name.top - 4, name.right - 6);
    [
        (SAVE_CAPTION, caption),
        (CLEAR, clear),
        (LOAD_GAME, load),
        (SAVE_NAME, name),
        (SAVE_SNAPSHOT, snapshot),
    ]
}

pub const OPTION_ITEMS: [(i16, u32); 8] = [
    (TEAMS, game_options::TEAMS),
    (UNIT_TRADING, game_options::UNIT_TRADING),
    (VETERANS, game_options::VETERANS),
    (ALLIANCES, game_options::ALLIANCES),
    (SHOW_ENEMY, game_options::SHOW_ENEMY),
    (OBSERVER, game_options::HOST_OBSERVER),
    (DEATHMATCH, game_options::DEATHMATCH),
    (NARRATION, game_options::NARRATION),
];
const MAP_TRADING: u32 = 0x2_0000;
const MAP_VETERANS: u32 = 0x4_0000;
const COOPERATIVE_GAME_TYPE: i16 = 0xd;
const PLANNING_DELTAS: [(i32, i32); 3] = [(1800, 450), (9000, 900), (18000, 1800)];
const TIME_LIMIT_DELTAS: [(i32, i32); 5] = [
    (18000, 1800),
    (36000, 3600),
    (162_000, 9000),
    (270_000, 18000),
    (810_000, 90000),
];
const TIME_LIMIT_MIN: i32 = 1800;
const TIME_LIMIT_MAX: i32 = 810_000;
const PLANNING_MIN: i32 = 450;
const PLANNING_MAX: i32 = 18000;
const PLAYERS_MIN: i32 = 1;
const PLAYERS_MAX: i32 = 16;
const TEAMS_MIN: i32 = 2;
const TEAMS_MAX: i16 = 16;
const GAME_TYPE_NAMES: FourCC = FourCC(*b"innn");
const SPLASH_ART: FourCC = FourCC(*b"##s#");
const PREVIEW_ART: FourCC = FourCC(*b"##p#");
const NO_FILTER: &[u8] = b"(search)";
const CHECKBOX_FLAGS: u32 = button_flags::SHAPE_LEFT | button_flags::STATE_WHEN_DISABLED;
const NAME_MAX: usize = 0x1f;

fn list_of(dialog: &mut Dialog, id: i16) -> Option<&mut List> {
    dialog.list_mut(id)
}

fn selected(dialog: &Dialog, id: i16) -> Option<i32> {
    dialog.selected_list_value(id)
}

fn checked(dialog: &Dialog, id: i16) -> bool {
    dialog.is_checked(id)
}

fn set_check(dialog: &mut Dialog, id: i16, on: bool) {
    dialog.set_checked(id, on);
}

fn enabled(dialog: &Dialog, id: i16) -> bool {
    dialog.is_enabled(id)
}

fn set_enabled(dialog: &mut Dialog, id: i16, on: bool) {
    dialog.set_enabled_if(id, on);
}

fn set_visible(dialog: &mut Dialog, id: i16, on: bool) {
    dialog.set_visible_if(id, on);
}

fn arrow_value(dialog: &Dialog, id: i16) -> i32 {
    dialog.arrow_value(id)
}

fn set_arrows(dialog: &mut Dialog, id: i16, min: i32, max: i32, value: i32) {
    dialog.set_arrows(id, min, max, value);
}

fn edit_text(dialog: &Dialog, id: i16) -> Vec<u8> {
    dialog.edit_text_slice(id).to_vec()
}

fn password(dialog: &Dialog) -> Vec<u8> {
    let p = dialog.edit_text_slice(PASSWORD);
    p[..p.len().min(NAME_MAX)].to_vec()
}

fn set_edit_text(dialog: &mut Dialog, id: i16, text: &[u8]) {
    dialog.set_edit_text(id, text);
}

fn valid_name(name: &[u8]) -> bool {
    super::net_join::valid_game_name(name)
}

pub fn game_type_mask(map_flags: u32) -> u32 {
    let mut mask = map_flags;
    if mask & 0x200 != 0 {
        mask |= 0x4000;
    }
    if mask & 0x80 != 0 {
        mask |= 0x8000;
    }
    mask
}

pub fn mapset_of(mapsets: &[NetMapset], description: &GameDescription) -> usize {
    let mesh = FourCC(description.parameters.mesh);
    let has = |m: &NetMapset| m.maps.iter().any(|map| map.mesh == mesh);
    let listed = |m: &NetMapset| {
        m.plugins
            .iter()
            .all(|p| description.plugins.0.iter().any(|d| d.name == p.name))
    };
    mapsets
        .iter()
        .position(|m| has(m) && listed(m))
        .or_else(|| mapsets.iter().position(has))
        .unwrap_or(0)
}

pub fn open(
    screen: &NetScreen<'_>,
    res: &mut Resources,
    stack: &mut DialogStack,
) -> Option<Box<dyn OpenScreen>> {
    let NetScreen::GameOptions(options) = screen else {
        return None;
    };
    let def = res.dialog(NGP)?;
    let mut dialog = Dialog::new(&def, NGP, dialog_flags::MODAL, res);
    let mut description = options.description.clone();
    if description.parameters.max_players < 2 {
        description.parameters.max_players = 2;
    }
    set_edit_text(&mut dialog, NAME, &description.game_name);
    if let Some((item, e)) = dialog.control_mut(NAME).and_then(|c| c.edit_mut()) {
        e.select_range(item, 0, usize::MAX);
    }
    set_edit_text(&mut dialog, PASSWORD, &options.password.0);
    if let Some(list) = list_of(&mut dialog, LEVELS) {
        list.set_flag(list_flags::FRAME, true);
        list.set_double_click_item(OK);
        list.set_flag(list_flags::FOCUS_RING, false);
    }
    if let Some(list) = list_of(&mut dialog, GAME_TYPES) {
        list.set_flag(list_flags::KEEP_SELECTION, true);
        list.set_flag(list_flags::FRAME, true);
    }
    if let Some(list) = list_of(&mut dialog, DIFFICULTIES) {
        list.set_flag(list_flags::KEEP_SELECTION, true);
        list.set_flag(list_flags::NO_SCROLLBAR, true);
        list.set_flag(list_flags::FRAME, false);
    }
    if let Some(list) = list_of(&mut dialog, MAPSETS) {
        list.set_flag(list_flags::FRAME, false);
    }
    dialog.set_list_unfocusable(ACTIVE_TAGSETS);
    if let Some(list) = list_of(&mut dialog, ACTIVE_TAGSETS) {
        list.set_flag(list_flags::FOCUS_RING, false);
        list.set_flag(list_flags::FRAME, true);
        list.set_flag(list_flags::NO_ROW_CLICKS, true);
    }
    for (id, color) in [
        (NAME_CAPTION, None),
        (PASSWORD_CAPTION, None),
        (TIME_LIMIT_CAPTION, Some(2)),
        (PLAYER_LIMIT_CAPTION, Some(2)),
    ] {
        let color = color.map(|c| res.dialog_color(c));
        let flags = if color.is_some() {
            text_flags::SHADOW
        } else {
            text_flags::RIGHT | text_flags::SHADOW
        };
        if let Some((item, t)) = dialog.control_mut(id).and_then(|c| c.static_text_mut()) {
            t.set_appearance(item, None, STYLE_PLAIN, color, Some(flags));
        }
    }
    for id in [TFL, ANTI_CLUMP] {
        if let Some((_, b)) = dialog.control_mut(id).and_then(|c| c.button_mut()) {
            b.set_flags(CHECKBOX_FLAGS);
        }
    }
    if let Some((item, e)) = dialog.control_mut(FILTER).and_then(|c| c.edit_mut()) {
        e.set_text(item, NO_FILTER);
    }
    if let Some((_, a)) = dialog
        .control_mut(DIFFICULTY_SELECTOR)
        .and_then(|c| c.double_arrow_mut())
    {
        a.set_reversed(true);
    }
    dialog.set_visible(COOPERATIVE, true);
    dialog.set_visible(DEATHMATCH, true);
    if let Some(list) = list_of(&mut dialog, DIFFICULTIES) {
        for (i, d) in options.difficulties.iter().enumerate() {
            list.add_row(&d.name.0, i as i32, true);
        }
    }
    if let (Some(caption), Some(list)) = (
        dialog.control(GAME_TYPE_CAPTION).map(|c| c.item.rect),
        dialog.control(GAME_TYPES).map(|c| c.item.rect),
    ) {
        for (id, rect) in saved_game_rects(caption, list) {
            let (kind, name): (&[u8; 4], &[u8]) = match id {
                SAVE_CAPTION => (b"stat", b"Saved Game:"),
                CLEAR => (b"butt", b"Clear"),
                LOAD_GAME => (b"butt", b"Load Game"),
                SAVE_NAME => (b"stat", b""),
                _ => (b"snap", b""),
            };
            dialog.add_control(FourCC(*kind), id, rect, name, res);
        }
        for (id, flags) in [(SAVE_CAPTION, 0), (SAVE_NAME, text_flags::CENTER)] {
            if let Some((item, t)) = dialog.control_mut(id).and_then(|c| c.static_text_mut()) {
                t.set_appearance(item, None, STYLE_PLAIN, None, Some(flags));
            }
        }
    }
    set_items(&mut dialog, &description);
    dialog.set_background(res.art(NGP), 0);
    dialog.set_focus(LEVELS);
    dialog.set_default_item(OK);
    dialog.set_cancel_item(CANCEL);
    let mut screen = Open {
        description,
        cooperative: false,
        mapset: 0,
        game_types: res.string_list(GAME_TYPE_NAMES).to_vec(),
        details: None,
        sent: options.description.clone(),
        sent_password: options.password.0.clone(),
        saved_game: options.saved_game.clone(),
        picker: None,
    };
    screen.fill_mapsets(&mut dialog, options, res);
    screen.cooperative_enables(&mut dialog, options);
    screen.game_types(&mut dialog, options);
    screen.build(&dialog, options);
    screen.show_details(&mut dialog, options, res);
    stack.push(dialog);
    Some(Box::new(screen))
}

fn show_saved_game(dialog: &mut Dialog, cooperative: bool, saved: Option<&NetSavedGame>) {
    let wide = dialog.bounds().width() > SAVE_CAPTION_MIN_WIDTH;
    set_visible(dialog, SAVE_CAPTION, cooperative && wide);
    for id in [CLEAR, LOAD_GAME, SAVE_SNAPSHOT, SAVE_NAME] {
        set_visible(dialog, id, cooperative);
    }
    set_enabled(dialog, CLEAR, cooperative && saved.is_some());
    if let Some((item, snap)) = dialog
        .control_mut(SAVE_SNAPSHOT)
        .and_then(|c| c.snapshot_mut())
    {
        snap.set(item, saved.and_then(|s| s.snapshot.clone()));
    }
    let name = saved.map(|s| s.name.0.clone()).unwrap_or_default();
    if let Some((item, text)) = dialog
        .control_mut(SAVE_NAME)
        .and_then(|c| c.static_text_mut())
    {
        text.set_text(item, &name);
    }
}

fn set_items(dialog: &mut Dialog, description: &GameDescription) {
    let p = &description.parameters;
    set_check(
        dialog,
        COOPERATIVE,
        p.options & game_options::COOPERATIVE != 0,
    );
    set_check(dialog, TFL, p.options & game_options::TFL != 0);
    set_check(
        dialog,
        ANTI_CLUMP,
        p.options & game_options::ANTI_CLUMP != 0,
    );
    set_check(dialog, PLANNING, p.planning_time != 0);
    let planning = if p.planning_time == 0 {
        -1
    } else {
        p.planning_time
    };
    set_arrows(dialog, PLANNING_TIME, PLANNING_MIN, PLANNING_MAX, planning);
    let time_limit = p.time_limit.clamp(TIME_LIMIT_MIN, TIME_LIMIT_MAX);
    set_arrows(
        dialog,
        TIME_LIMIT,
        TIME_LIMIT_MIN,
        TIME_LIMIT_MAX,
        time_limit,
    );
    let players = i32::from(p.max_players);
    set_arrows(dialog, PLAYER_LIMIT, PLAYERS_MIN, PLAYERS_MAX, players);
    set_check(dialog, TEAMS_MAXIMUM, p.field_16 != -1);
    if p.field_16 != -1 {
        set_arrows(dialog, TEAMS_COUNT, TEAMS_MIN, i32::from(TEAMS_MAX), -1);
        set_arrows(dialog, TEAMS_COUNT, -1, -1, i32::from(p.field_16));
    }
    for (id, deltas, format) in [
        (PLANNING_TIME, &PLANNING_DELTAS[..], Format::Time),
        (TIME_LIMIT, &TIME_LIMIT_DELTAS[..], Format::Time),
    ] {
        if let Some((item, a)) = dialog.control_mut(id).and_then(|c| c.double_arrow_mut()) {
            a.set_deltas(deltas);
            a.set_format(item, format);
        }
    }
    for (id, bit) in OPTION_ITEMS {
        set_check(dialog, id, p.options & bit == bit);
    }
    if let Some(list) = list_of(dialog, DIFFICULTIES) {
        list.select_value(i32::from(p.difficulty));
    }
}

struct Open {
    description: GameDescription,
    cooperative: bool,
    mapset: usize,
    game_types: Vec<Vec<u8>>,
    details: Option<mandrake_session::FilmDetails>,
    sent: GameDescription,
    sent_password: Vec<u8>,
    saved_game: Option<NetSavedGame>,
    picker: Option<super::saved_games::Picker>,
}

impl Open {
    fn mapsets<'a>(&self, options: &'a GameOptions) -> &'a [NetMapset] {
        if self.cooperative {
            &options.cooperative_mapsets
        } else {
            &options.mapsets
        }
    }

    fn map<'a>(&self, dialog: &Dialog, options: &'a GameOptions) -> Option<&'a NetMap> {
        let mapset = self.mapsets(options).get(self.mapset)?;
        let row = selected(dialog, LEVELS)?;
        mapset.maps.get(usize::try_from(row).ok()?)
    }

    fn fill_mapsets(&mut self, dialog: &mut Dialog, options: &GameOptions, res: &mut Resources) {
        self.cooperative = self.description.parameters.options & game_options::COOPERATIVE != 0;
        let mapsets = self.mapsets(options);
        self.mapset = mapset_of(mapsets, &self.description);
        if let Some(list) = list_of(dialog, MAPSETS) {
            list.clear();
            for (i, m) in mapsets.iter().enumerate() {
                list.add_row(&m.name.0, i as i32, true);
            }
            list.select_value(self.mapset as i32);
            list.show_selected();
        }
        self.fill_maps(dialog, options, res);
    }

    fn fill_maps(&mut self, dialog: &mut Dialog, options: &GameOptions, res: &mut Resources) {
        let mesh = FourCC(self.description.parameters.mesh);
        let Some(mapset) = self.mapsets(options).get(self.mapset) else {
            return;
        };
        if let Some(list) = list_of(dialog, LEVELS) {
            list.clear();
            for (i, map) in mapset.maps.iter().enumerate() {
                let look = EntryLook {
                    listed: map.flags & mandrake_session::rules::ENTRY_LISTED != 0,
                    order: map.order,
                };
                list.add_unmarked_entry(&map.name.0, i as i32, look);
            }
            match mapset.maps.iter().position(|m| m.mesh == mesh) {
                Some(row) => list.select_value(row as i32),
                None => list.select(0),
            }
            list.show_selected();
        }
        for (id, tag) in [(SPLASH, SPLASH_ART), (PREVIEW, PREVIEW_ART)] {
            let art = res.art_of_plugins(&mapset.plugin_names, tag);
            let visible = art.is_some();
            if let Some((item, bitmap)) = dialog.control_mut(id).and_then(|c| c.bitmap_mut()) {
                bitmap.set_drawing_flags(item, 0, 0);
                bitmap.set_art(item, art, 0, 0);
            }
            dialog.set_visible(id, visible);
        }
    }

    fn game_types(&mut self, dialog: &mut Dialog, options: &GameOptions) {
        let (flags, most) = self
            .map(dialog, options)
            .map_or((0, 0), |m| (m.flags, i32::from(m.max_teams)));
        let mask = game_type_mask(flags);
        let chosen = selected(dialog, GAME_TYPES)
            .unwrap_or(i32::from(self.description.parameters.game_type));
        let names = self.game_types.clone();
        if let Some(list) = list_of(dialog, GAME_TYPES) {
            list.clear();
            for (i, name) in names.iter().enumerate().take(32) {
                if mask & (1 << i) != 0 {
                    list.add_row(name, i as i32, true);
                }
            }
            let key = |value: i32| -> Vec<u8> {
                names
                    .get(value as usize)
                    .map(|n| n.to_ascii_lowercase())
                    .unwrap_or_default()
            };
            let mut order: Vec<i32> = (0..list.row_count())
                .filter_map(|r| list.row_value(r))
                .collect();
            order.sort_by_key(|&v| key(v));
            list.sort_by(|v| order.iter().position(|&o| o == v).unwrap_or(usize::MAX));
            if (0..list.row_count()).any(|r| list.row_value(r) == Some(chosen)) {
                list.select_value(chosen);
            } else if list.row_count() > 0 {
                list.select(0);
            }
            list.show_selected();
        }
        let trading = !self.cooperative && mask & MAP_TRADING != 0 && checked(dialog, PLANNING);
        set_enabled(dialog, UNIT_TRADING, trading);
        set_enabled(dialog, VETERANS, mask & MAP_VETERANS != 0);
        set_arrows(dialog, TEAMS_COUNT, TEAMS_MIN, most, -1);
    }

    fn cooperative_enables(&self, dialog: &mut Dialog, options: &GameOptions) {
        let coop = self.cooperative;
        set_visible(dialog, GAME_TYPE_CAPTION, !coop);
        set_visible(dialog, GAME_TYPES, !coop);
        set_visible(dialog, NARRATION, coop);
        set_enabled(dialog, TIME_LIMIT_CAPTION, !coop);
        set_enabled(dialog, TIME_LIMIT, !coop);
        set_enabled(dialog, ALLIANCES, !coop);
        let planning = checked(dialog, PLANNING);
        let map_trading = self
            .map(dialog, options)
            .is_some_and(|m| game_type_mask(m.flags) & MAP_TRADING != 0);
        set_enabled(dialog, UNIT_TRADING, !coop && planning && map_trading);
        set_enabled(dialog, PLANNING, !coop);
        set_enabled(dialog, PLANNING_TIME, !coop && planning);
        set_enabled(dialog, TEAMS, true);
        let teams = checked(dialog, TEAMS);
        set_enabled(dialog, TEAMS_MAXIMUM, !coop && teams);
        let most = !coop && teams && checked(dialog, TEAMS_MAXIMUM);
        set_enabled(dialog, TEAMS_COUNT, most);
        set_enabled(dialog, DEATHMATCH, !coop);
        show_saved_game(dialog, coop, self.saved_game.as_ref());
    }

    fn build(&mut self, dialog: &Dialog, options: &GameOptions) {
        let mut d = self.description.clone();
        let mut name = edit_text(dialog, NAME);
        name.truncate(NAME_MAX);
        d.game_name = name;
        if password(dialog).is_empty() {
            d.flags &= !FLAG_PASSWORD;
        } else {
            d.flags |= FLAG_PASSWORD;
        }
        let p = &mut d.parameters;
        let mut o = p.options;
        for (id, bit) in OPTION_ITEMS {
            if checked(dialog, id) {
                o |= bit;
            } else {
                o &= !bit;
            }
        }
        if !enabled(dialog, VETERANS) {
            o &= !game_options::VETERANS;
        }
        if !enabled(dialog, UNIT_TRADING) {
            o &= !game_options::UNIT_TRADING;
        }
        if !enabled(dialog, DEATHMATCH) {
            o &= !game_options::DEATHMATCH;
        }
        o = (o & !game_options::COOPERATIVE)
            | if checked(dialog, COOPERATIVE) {
                game_options::COOPERATIVE
            } else {
                0
            };
        o = (o & !game_options::TFL)
            | if checked(dialog, TFL) {
                game_options::TFL
            } else {
                0
            };
        o = (o & !game_options::ANTI_CLUMP)
            | if checked(dialog, ANTI_CLUMP) {
                game_options::ANTI_CLUMP
            } else {
                0
            };
        p.options = o;
        p.field_16 = if checked(dialog, TEAMS) && checked(dialog, TEAMS_MAXIMUM) {
            arrow_value(dialog, TEAMS_COUNT) as i16
        } else {
            -1
        };
        p.max_players = arrow_value(dialog, PLAYER_LIMIT) as i16;
        if let Some(difficulty) = selected(dialog, DIFFICULTIES) {
            p.difficulty = difficulty as i16;
        }
        p.planning_time = 0;
        p.time_limit = -1;
        if o & game_options::COOPERATIVE == 0 {
            if let Some(game_type) = selected(dialog, GAME_TYPES) {
                p.game_type = game_type as i16;
            }
            p.time_limit = arrow_value(dialog, TIME_LIMIT);
            if checked(dialog, PLANNING) {
                p.planning_time = arrow_value(dialog, PLANNING_TIME);
            }
        } else {
            p.game_type = COOPERATIVE_GAME_TYPE;
        }
        if let (Some(mapset), Some(map)) = (
            self.mapsets(options).get(self.mapset),
            self.map(dialog, options),
        ) {
            choose_map(&mut d, mapset, map);
        }
        self.description = d;
    }

    fn show_details(&mut self, dialog: &mut Dialog, options: &GameOptions, res: &mut Resources) {
        if options.details == self.details {
            return;
        }
        self.details = options.details.clone();
        super::lobby::show_game_details(dialog, GAME, self.details.clone(), res);
    }

    fn hit(&mut self, item: i16, dialog: &mut Dialog, options: &GameOptions, res: &mut Resources) {
        match item {
            COOPERATIVE => {
                let on = checked(dialog, COOPERATIVE);
                let o = &mut self.description.parameters.options;
                *o = (*o & !game_options::COOPERATIVE)
                    | if on { game_options::COOPERATIVE } else { 0 };
                self.fill_mapsets(dialog, options, res);
                self.cooperative_enables(dialog, options);
                self.game_types(dialog, options);
            }
            MAPSETS => {
                if let Some(row) = selected(dialog, MAPSETS) {
                    self.mapset = row as usize;
                }
                self.fill_maps(dialog, options, res);
                self.game_types(dialog, options);
            }
            LEVELS | TAGSETS => self.game_types(dialog, options),
            NAME => {
                let ok = valid_name(&edit_text(dialog, NAME));
                set_enabled(dialog, OK, ok);
            }
            PLANNING => {
                self.game_types(dialog, options);
                let on = checked(dialog, PLANNING);
                set_enabled(dialog, PLANNING_TIME, on);
            }
            TEAMS if !self.cooperative => {
                let teams = checked(dialog, TEAMS);
                set_enabled(dialog, TEAMS_MAXIMUM, teams);
                let most = teams && checked(dialog, TEAMS_MAXIMUM);
                set_enabled(dialog, TEAMS_COUNT, most);
            }
            TEAMS_MAXIMUM => {
                let most = checked(dialog, TEAMS_MAXIMUM) && checked(dialog, TEAMS);
                set_enabled(dialog, TEAMS_COUNT, most);
            }
            TFL => set_check(dialog, ANTI_CLUMP, false),
            ANTI_CLUMP => set_check(dialog, TFL, false),
            _ => {}
        }
    }
}

impl OpenScreen for Open {
    fn frame(&mut self, screen: &Screen<'_>, frame: &mut ScreenFrame<'_>) {
        let Screen::Net(NetScreen::GameOptions(options)) = screen else {
            return;
        };
        let hits: Vec<i16> = frame
            .hits
            .iter()
            .filter(|h| h.dialog == NGP)
            .map(|h| h.item)
            .collect();
        match (&options.picker, self.picker.as_mut()) {
            (Some(data), Some(picker)) => {
                picker.frame(data, frame);
                return;
            }
            (Some(data), None) => {
                self.picker = super::saved_games::Picker::open(data, frame.res, frame.stack);
                return;
            }
            (None, Some(_)) => {
                if let Some(picker) = self.picker.take() {
                    picker.close(frame.stack);
                }
            }
            (None, None) => {}
        }
        let Some(dialog) = frame.stack.find_mut(NGP) else {
            return;
        };
        if options.saved_game != self.saved_game {
            let loaded = options.saved_game.is_some();
            self.saved_game = options.saved_game.clone();
            if loaded {
                self.description = options.description.clone();
                self.sent = options.description.clone();
                set_items(dialog, &self.description);
                self.fill_mapsets(dialog, options, frame.res);
                self.cooperative_enables(dialog, options);
                self.game_types(dialog, options);
            } else {
                self.cooperative_enables(dialog, options);
            }
            self.build(dialog, options);
        }
        let mut done = None;
        for item in hits {
            let drops = u16::try_from(item).is_ok_and(|i| i < 64 && DROPS_SAVE & (1 << i) != 0);
            if (drops || item == CLEAR) && self.saved_game.is_some() {
                self.saved_game = None;
                self.description.parameters.flags &= !1;
                frame.out.actions.push(Action::ClearNetSave);
                show_saved_game(dialog, self.cooperative, None);
            }
            match item {
                LOAD_GAME => {
                    frame.out.actions.push(Action::LoadNetSave);
                    continue;
                }
                CLEAR => continue,
                _ => {}
            }
            match item {
                OK => {
                    let players = arrow_value(dialog, PLAYER_LIMIT).clamp(2, PLAYERS_MAX);
                    set_arrows(dialog, PLAYER_LIMIT, -1, -1, players);
                    self.build(dialog, options);
                    done = Some(Action::Ok);
                    break;
                }
                CANCEL => {
                    done = Some(Action::Cancel);
                    break;
                }
                item => {
                    self.hit(item, dialog, options, frame.res);
                    self.build(dialog, options);
                }
            }
        }
        if self.description != self.sent {
            frame
                .out
                .actions
                .push(Action::SetGameOptions(Box::new(self.description.clone())));
            self.sent = self.description.clone();
        }
        let typed = password(dialog);
        if typed != self.sent_password {
            frame
                .out
                .actions
                .push(Action::SetGamePassword(mandrake_session::Text(
                    typed.clone(),
                )));
            self.sent_password = typed;
        }
        if let Some(action) = done {
            frame.out.actions.push(action);
            return;
        }
        self.show_details(dialog, options, frame.res);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flag_rally_maps_allow_captures_and_balls_on_parade_stampede() {
        assert_eq!(game_type_mask(0x200), 0x4200);
        assert_eq!(game_type_mask(0x80), 0x8080);
        assert_eq!(game_type_mask(0x5), 0x5);
    }

    #[test]
    fn names_need_something_printable() {
        assert!(valid_name(b"Myrkridian Rhapsody"));
        assert!(!valid_name(b"   "));
        assert!(!valid_name(b""));
    }

    #[test]
    fn cooperative_enables_respects_map_trading_flag() {
        use mandrake_session::Text;
        use mandrake_tag::{DialogDefinition, DialogItem, Rect};
        let make = |kind: &[u8; 4], id: i16| DialogItem {
            kind: FourCC(*kind),
            field_04: 0,
            id: id as u32,
            fields_08: [0; 4],
            rect: Rect::new(0, 0, 20, 20),
            name: Vec::new(),
        };
        let def = DialogDefinition {
            version: 1,
            rect: Rect::new(0, 0, 200, 200),
            items: vec![
                make(b"list", LEVELS),
                make(b"chek", PLANNING),
                make(b"chek", UNIT_TRADING),
                make(b"chek", TEAMS),
                make(b"chek", TEAMS_MAXIMUM),
                make(b"numb", TEAMS_COUNT),
            ],
        };
        let mut res = Resources::from_source(|_, _| None);
        let mut dialog = Dialog::new(&def, NGP, dialog_flags::MODAL, &mut res);
        dialog.set_checked(PLANNING, true);
        if let Some(list) = dialog.list_mut(LEVELS) {
            list.add_row(b"NoTrade", 0, true);
            list.select(0);
        }
        let description = GameDescription::read(&[0u8; 0x23e]).unwrap();
        let options = GameOptions {
            description: description.clone(),
            password: Text::default(),
            mapsets: vec![NetMapset {
                name: Text(b"Base".to_vec()),
                plugins: Vec::new(),
                plugin_names: Vec::new(),
                maps: vec![NetMap {
                    name: Text(b"NoTrade".to_vec()),
                    mesh: FourCC(*b"m001"),
                    flags: 0x1,
                    max_teams: 4,
                    order: 0,
                }],
            }],
            cooperative_mapsets: Vec::new(),
            difficulties: Vec::new(),
            details: None,
            saved_game: None,
            picker: None,
        };
        let open = Open {
            description: description.clone(),
            cooperative: false,
            mapset: 0,
            game_types: Vec::new(),
            details: None,
            sent: description,
            sent_password: Vec::new(),
            saved_game: None,
            picker: None,
        };
        open.cooperative_enables(&mut dialog, &options);
        assert!(
            !dialog.is_enabled(UNIT_TRADING),
            "UNIT_TRADING must remain disabled when map lacks MAP_TRADING"
        );
    }
}
