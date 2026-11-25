use mandrake_common::FourCC;
use mandrake_session::net::{FLAG_PASSWORD, GameDescription, GameOptions, NetMap};
use mandrake_session::{Action, Text};
use mandrake_tag::font::{STYLE_PLAIN, text_flags};

use crate::controls::list::{EntryLook, List, list_flags};
use crate::controls::numb::Format;
use crate::core::{Dialog, DialogStack, dialog_flags};
use crate::resources::Resources;
use crate::screens::ScreenFrame;

pub const NEPD: FourCC = FourCC(*b"nepd");
const INGO: FourCC = FourCC(*b"ingo");
const INNN: FourCC = FourCC(*b"innn");

pub const OK: i16 = 1;
pub const CANCEL: i16 = 2;
pub const GAME: i16 = 3;
const NAME_CAPTION: i16 = 4;
pub const NAME: i16 = 5;
const PASSWORD_CAPTION: i16 = 6;
pub const PASSWORD: i16 = 7;
pub const TEAMS: i16 = 8;
const TIME_LIMIT_CAPTION: i16 = 9;
const TEAMS_STAT: i16 = 10;
pub const TIME_LIMIT: i16 = 11;
const RANDOM_ENDING: i16 = 12;
pub const MAPS: i16 = 14;
pub const GAME_TYPES: i16 = 16;
pub const DIFFICULTIES: i16 = 18;
const LIMITED_VISIBILITY: i16 = 19;
pub const UNIT_TRADING: i16 = 20;
pub const VETERANS: i16 = 21;
pub const SINGLE_PLAYER: i16 = 22;
pub const TEAMS_MAXIMUM: i16 = 24;
pub const TEAMS_COUNT: i16 = 25;
const ADVANCED: i16 = 26;
pub const PLANNING: i16 = 27;
pub const PLANNING_TIME: i16 = 28;
const PLAYER_LIMIT_CAPTION: i16 = 29;
pub const PLAYER_LIMIT: i16 = 30;
const TRADING_CAPTION: i16 = 31;
const VETERANS_CAPTION: i16 = 32;
const PLUGINS: i16 = 33;
const PLUGINS_CAPTION: i16 = 34;

pub const OPTION_ITEMS: [(i16, u32); 4] = [
    (TEAMS, 0x2),
    (UNIT_TRADING, 0x10),
    (VETERANS, 0x20),
    (SINGLE_PLAYER, 0x80),
];
const OPTION_TRADING: u32 = 0x10;
const OPTION_VETERANS: u32 = 0x20;
const OPTION_LAST_GAME: u32 = 0x40;
const OPTION_SINGLE_PLAYER: u32 = 0x80;
const OPTIONS_ON_OK: u32 = 0xe004;
const MAP_TRADING: u32 = 0x2_0000;
const MAP_VETERANS: u32 = 0x4_0000;
const COOPERATIVE_GAME_TYPE: i16 = 11;
const DEFAULT_DIFFICULTY: i16 = 2;
const TIME_LIMIT_DELTAS: [(i32, i32); 3] = [(18000, 1800), (36000, 3600), (162_000, 9000)];
const PLANNING_DELTAS: [(i32, i32); 3] = [(1800, 450), (9000, 900), (18000, 1800)];
const TIME_LIMIT_MIN: i32 = 0x708;
const TIME_LIMIT_MAX: i32 = 0x278d0;
const PLANNING_MIN: i32 = 0x1c2;
const PLANNING_MAX: i32 = 18000;
const PLAYERS_MIN: i32 = 1;
const PLAYERS_MAX: i32 = 0x10;
const TEAMS_MIN: i32 = 2;
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

fn arrow_value(dialog: &Dialog, id: i16) -> i32 {
    dialog.arrow_value(id)
}

fn set_arrows(dialog: &mut Dialog, id: i16, min: i32, max: i32, value: i32) {
    dialog.set_arrows(id, min, max, value);
}

fn edit_text(dialog: &Dialog, id: i16) -> Vec<u8> {
    let text = dialog.edit_text_slice(id);
    text[..text.len().min(NAME_MAX)].to_vec()
}

fn set_edit_text(dialog: &mut Dialog, id: i16, text: &[u8]) {
    dialog.set_edit_text(id, text);
}

fn valid_name(name: &[u8]) -> bool {
    crate::screens::net_join::valid_game_name(name)
}

fn update_teams_enables(dialog: &mut Dialog) {
    let solo = checked(dialog, SINGLE_PLAYER);
    let teams = !solo && checked(dialog, TEAMS);
    let most = teams && checked(dialog, TEAMS_MAXIMUM);
    set_enabled(dialog, TEAMS_MAXIMUM, teams);
    set_enabled(dialog, TEAMS_COUNT, most);
    set_enabled(dialog, TEAMS_STAT, most);
}

pub(super) struct Open {
    description: GameDescription,
    last_game: u32,
    sent: GameDescription,
    sent_password: Vec<u8>,
    details: Option<mandrake_session::FilmDetails>,
    single_player: bool,
}

impl Open {
    pub(super) fn push(
        options: &GameOptions,
        res: &mut Resources,
        stack: &mut DialogStack,
    ) -> Option<Self> {
        let def = res.dialog(NEPD)?;
        let mut dialog = Dialog::new(&def, NEPD, dialog_flags::MODAL, res);
        let mut description = options.description.clone();
        if description.parameters.max_players < 2 {
            description.parameters.max_players = 2;
        }
        set_edit_text(&mut dialog, NAME, &description.game_name);
        dialog.set_focus(NAME);
        if let Some((item, e)) = dialog.control_mut(NAME).and_then(|c| c.edit_mut()) {
            e.select_range(item, 0, usize::MAX);
        }
        set_edit_text(&mut dialog, PASSWORD, &options.password.0);
        dialog.set_background(res.art(INGO), 0);
        for id in [
            ADVANCED,
            LIMITED_VISIBILITY,
            RANDOM_ENDING,
            PLUGINS,
            PLUGINS_CAPTION,
        ] {
            dialog.set_visible(id, false);
        }
        if let Some(list) = list_of(&mut dialog, MAPS) {
            list.set_flag(list_flags::FRAME, false);
        }
        for id in [GAME_TYPES, DIFFICULTIES] {
            if let Some(list) = list_of(&mut dialog, id) {
                list.set_flag(list_flags::KEEP_SELECTION, true);
                list.set_flag(list_flags::FRAME, false);
            }
        }
        let color = Some(res.dialog_color(2));
        for (id, color, flags) in [
            (NAME_CAPTION, None, text_flags::RIGHT | text_flags::SHADOW),
            (
                PASSWORD_CAPTION,
                None,
                text_flags::RIGHT | text_flags::SHADOW,
            ),
            (TIME_LIMIT_CAPTION, color, text_flags::SHADOW),
            (PLAYER_LIMIT_CAPTION, color, text_flags::SHADOW),
            (TRADING_CAPTION, color, text_flags::SHADOW),
            (VETERANS_CAPTION, color, text_flags::SHADOW),
        ] {
            if let Some((item, t)) = dialog.control_mut(id).and_then(|c| c.static_text_mut()) {
                t.set_appearance(item, None, STYLE_PLAIN, color, Some(flags));
            }
        }
        for (id, deltas) in [
            (TIME_LIMIT, &TIME_LIMIT_DELTAS[..]),
            (PLANNING_TIME, &PLANNING_DELTAS[..]),
        ] {
            if let Some((item, a)) = dialog.control_mut(id).and_then(|c| c.double_arrow_mut()) {
                a.set_deltas(deltas);
                a.set_format(item, Format::Time);
            }
        }
        if let Some(list) = list_of(&mut dialog, DIFFICULTIES) {
            for (i, d) in options.difficulties.iter().enumerate() {
                list.add_row(&d.name.0, i as i32, true);
            }
        }
        let mut open = Self {
            last_game: description.parameters.options & OPTION_LAST_GAME,
            sent: options.description.clone(),
            sent_password: options.password.0.clone(),
            details: None,
            single_player: false,
            description,
        };
        open.set_items(&mut dialog, options, res);
        open.build(&dialog, options);
        dialog.set_default_item(OK);
        dialog.set_cancel_item(CANCEL);
        open.show_details(&mut dialog, options, res);
        stack.push(dialog);
        Some(open)
    }

    fn maps<'a>(&self, options: &'a GameOptions) -> &'a [NetMap] {
        let mapsets = if self.single_player {
            &options.cooperative_mapsets
        } else {
            &options.mapsets
        };
        mapsets.first().map_or(&[], |m| m.maps.as_slice())
    }

    fn map<'a>(&self, dialog: &Dialog, options: &'a GameOptions) -> Option<&'a NetMap> {
        let row = selected(dialog, MAPS)?;
        self.maps(options).get(usize::try_from(row).ok()?)
    }

    fn set_items(&mut self, dialog: &mut Dialog, options: &GameOptions, res: &mut Resources) {
        let p = self.description.parameters.clone();
        set_check(dialog, PLANNING, p.planning_time != 0);
        set_arrows(
            dialog,
            PLANNING_TIME,
            PLANNING_MIN,
            PLANNING_MAX,
            p.planning_time,
        );
        set_arrows(
            dialog,
            TIME_LIMIT,
            TIME_LIMIT_MIN,
            TIME_LIMIT_MAX,
            p.time_limit,
        );
        set_arrows(
            dialog,
            PLAYER_LIMIT,
            PLAYERS_MIN,
            PLAYERS_MAX,
            i32::from(p.max_players),
        );
        set_check(dialog, TEAMS_MAXIMUM, p.field_16 != -1);
        if p.field_16 != -1 {
            set_arrows(
                dialog,
                TEAMS_COUNT,
                TEAMS_MIN,
                PLAYERS_MAX,
                i32::from(p.field_16),
            );
        }
        for (id, bit) in OPTION_ITEMS {
            set_check(dialog, id, p.options & bit == bit);
        }
        self.single_player = p.options & OPTION_SINGLE_PLAYER != 0;
        self.fill_maps(dialog, options);
        self.game_types(dialog, options, res, Some(i32::from(p.game_type)));
        if let Some(list) = list_of(dialog, DIFFICULTIES) {
            list.select_value(i32::from(p.difficulty));
            list.show_selected();
        }
        let planning = checked(dialog, PLANNING);
        set_enabled(dialog, PLANNING_TIME, planning);
        update_teams_enables(dialog);
        self.single_player_enables(dialog, options, res);
    }

    fn fill_maps(&self, dialog: &mut Dialog, options: &GameOptions) {
        let mesh = FourCC(self.description.parameters.mesh);
        let maps = self.maps(options);
        if let Some(list) = list_of(dialog, MAPS) {
            list.clear();
            for (i, map) in maps.iter().enumerate() {
                let look = EntryLook {
                    listed: true,
                    order: map.order,
                };
                list.add_unmarked_entry(&map.name.0, i as i32, look);
            }
            match maps.iter().position(|m| m.mesh == mesh) {
                Some(row) => list.select(row),
                None => list.select(0),
            }
            list.show_selected();
        }
        if let Some(c) = dialog.control_mut(MAPS) {
            c.item.dirty = true;
        }
    }

    fn game_types(
        &self,
        dialog: &mut Dialog,
        options: &GameOptions,
        res: &mut Resources,
        chosen: Option<i32>,
    ) {
        let (flags, most) = self
            .map(dialog, options)
            .map_or((0, 0), |m| (m.flags, i32::from(m.max_teams)));
        let chosen = chosen.or_else(|| selected(dialog, GAME_TYPES));
        let names = res.string_list(INNN);
        if let Some(list) = list_of(dialog, GAME_TYPES) {
            list.clear();
            for (i, name) in names.iter().enumerate() {
                let allowed = i < 32 && flags & (1 << i) != 0;
                list.add_row(name, i as i32, allowed);
            }
            match chosen
                .and_then(|v| (0..list.row_count()).find(|&r| list.row_value(r) == Some(v)))
                .filter(|&r| list.row_enabled(r))
            {
                Some(row) => list.select(row),
                None => {
                    list.clear_selection();
                    list.select_first_enabled();
                }
            }
            list.show_selected();
        }
        if let Some(c) = dialog.control_mut(GAME_TYPES) {
            c.item.dirty = true;
        }
        let trading = !self.single_player && flags & MAP_TRADING != 0 && checked(dialog, PLANNING);
        set_enabled(dialog, UNIT_TRADING, trading);
        set_enabled(dialog, TRADING_CAPTION, trading);
        let veterans = flags & MAP_VETERANS != 0;
        set_enabled(dialog, VETERANS, veterans);
        set_enabled(dialog, VETERANS_CAPTION, veterans);
        set_arrows(dialog, TEAMS_COUNT, TEAMS_MIN, most, -1);
    }

    fn single_player_enables(
        &mut self,
        dialog: &mut Dialog,
        options: &GameOptions,
        res: &mut Resources,
    ) {
        let solo = checked(dialog, SINGLE_PLAYER);
        set_enabled(dialog, PLANNING, !solo);
        let planning = checked(dialog, PLANNING) && !solo;
        set_enabled(dialog, PLANNING_TIME, planning);
        set_enabled(dialog, TIME_LIMIT_CAPTION, !solo);
        set_enabled(dialog, TIME_LIMIT, !solo);
        update_teams_enables(dialog);
        if solo != self.single_player {
            self.single_player = solo;
            self.fill_maps(dialog, options);
        }
        self.game_types(dialog, options, res, None);
    }

    fn build(&mut self, dialog: &Dialog, options: &GameOptions) {
        let mut d = self.description.clone();
        d.game_name = edit_text(dialog, NAME);
        if edit_text(dialog, PASSWORD).is_empty() {
            d.flags &= !FLAG_PASSWORD;
        } else {
            d.flags |= FLAG_PASSWORD;
        }
        let p = &mut d.parameters;
        let mut o = 0;
        for (id, bit) in OPTION_ITEMS {
            if checked(dialog, id) {
                o |= bit;
            }
        }
        if !enabled(dialog, VETERANS) {
            o &= !OPTION_VETERANS;
        }
        if !enabled(dialog, UNIT_TRADING) {
            o &= !OPTION_TRADING;
        }
        p.options = o;
        p.field_16 = if checked(dialog, TEAMS) && checked(dialog, TEAMS_MAXIMUM) {
            arrow_value(dialog, TEAMS_COUNT) as i16
        } else {
            -1
        };
        p.max_players = arrow_value(dialog, PLAYER_LIMIT) as i16;
        p.difficulty = selected(dialog, DIFFICULTIES).map_or(DEFAULT_DIFFICULTY, |v| v as i16);
        if let Some(map) = self.map(dialog, options) {
            p.mesh = map.mesh.0;
            d.map_name = map.name.0.clone();
        }
        let p = &mut d.parameters;
        p.planning_time = 0;
        p.time_limit = -1;
        if o & OPTION_SINGLE_PLAYER != 0 {
            p.game_type = COOPERATIVE_GAME_TYPE;
        } else {
            p.game_type = selected(dialog, GAME_TYPES).map_or(-1, |v| v as i16);
            p.time_limit = arrow_value(dialog, TIME_LIMIT);
            if checked(dialog, PLANNING) {
                p.planning_time = arrow_value(dialog, PLANNING_TIME);
            }
        }
        self.description = d;
    }

    fn show_details(&mut self, dialog: &mut Dialog, options: &GameOptions, res: &mut Resources) {
        if options.details == self.details {
            return;
        }
        self.details = options.details.clone();
        super::lobby::show_details(dialog, GAME, self.details.clone(), res);
    }

    fn hit(&mut self, item: i16, dialog: &mut Dialog, options: &GameOptions, res: &mut Resources) {
        match item {
            NAME => {
                let ok = valid_name(&edit_text(dialog, NAME));
                set_enabled(dialog, OK, ok);
            }
            TEAMS | TEAMS_MAXIMUM => update_teams_enables(dialog),
            MAPS => self.game_types(dialog, options, res, None),
            SINGLE_PLAYER => self.single_player_enables(dialog, options, res),
            PLANNING => {
                self.game_types(dialog, options, res, None);
                let on = checked(dialog, PLANNING);
                set_enabled(dialog, PLANNING_TIME, on);
            }
            _ => {}
        }
    }

    pub(super) fn frame(&mut self, options: &GameOptions, frame: &mut ScreenFrame<'_>) {
        let hits: Vec<i16> = frame
            .hits
            .iter()
            .filter(|h| h.dialog == NEPD)
            .map(|h| h.item)
            .collect();
        let Some(dialog) = frame.stack.find_mut(NEPD) else {
            return;
        };
        let mut done = None;
        for item in hits {
            match item {
                OK => {
                    let players = arrow_value(dialog, PLAYER_LIMIT).clamp(2, PLAYERS_MAX);
                    set_arrows(dialog, PLAYER_LIMIT, -1, -1, players);
                    self.build(dialog, options);
                    self.description.parameters.options |= self.last_game | OPTIONS_ON_OK;
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
        let typed = edit_text(dialog, PASSWORD);
        if typed != self.sent_password {
            frame
                .out
                .actions
                .push(Action::SetGamePassword(Text(typed.clone())));
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
    fn names_need_a_visible_character() {
        assert!(valid_name(b"Rolly Polly Dwarf Heads"));
        assert!(!valid_name(b"   "));
        assert!(!valid_name(b""));
    }

    #[test]
    fn single_player_keeps_teams_maximum_count_and_stat_disabled() {
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
                make(b"chek", SINGLE_PLAYER),
                make(b"chek", TEAMS),
                make(b"chek", TEAMS_MAXIMUM),
                make(b"numb", TEAMS_COUNT),
                make(b"stat", TEAMS_STAT),
            ],
        };
        let mut res = Resources::from_source(|_, _| None);
        let mut dialog = Dialog::new(&def, NEPD, dialog_flags::MODAL, &mut res);
        dialog.set_checked(SINGLE_PLAYER, false);
        dialog.set_checked(TEAMS, false);
        dialog.set_checked(TEAMS_MAXIMUM, true);
        update_teams_enables(&mut dialog);
        assert!(!dialog.is_enabled(TEAMS_MAXIMUM));
        assert!(!dialog.is_enabled(TEAMS_COUNT));
        assert!(!dialog.is_enabled(TEAMS_STAT));
        dialog.set_checked(SINGLE_PLAYER, true);
        dialog.set_checked(TEAMS, true);
        update_teams_enables(&mut dialog);
        assert!(!dialog.is_enabled(TEAMS_MAXIMUM));
        assert!(!dialog.is_enabled(TEAMS_COUNT));
        assert!(!dialog.is_enabled(TEAMS_STAT));
        dialog.set_checked(SINGLE_PLAYER, false);
        update_teams_enables(&mut dialog);
        assert!(dialog.is_enabled(TEAMS_MAXIMUM));
        assert!(dialog.is_enabled(TEAMS_COUNT));
        assert!(dialog.is_enabled(TEAMS_STAT));
    }
}
