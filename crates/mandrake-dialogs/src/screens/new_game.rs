use mandrake_common::FourCC;
use mandrake_session::screen::ForcedGameplay;
use mandrake_session::{Action, NewGame, Screen};

use super::{OpenScreen, ScreenFrame, main_menu};
use crate::controls::button::button_flags;
use crate::controls::list::{EntryLook, List, list_flags};
use crate::core::{Dialog, DialogStack, dialog_flags};
use crate::resources::Resources;

pub const NEW_GAME: FourCC = FourCC(*b"!new");
pub const TFL_WARNING: FourCC = FourCC(*b"vtfl");

pub const OK: i16 = 1;
pub const CANCEL: i16 = 2;
pub const TAGSETS: i16 = 3;
pub const LEVELS: i16 = 6;
pub const DIFFICULTIES: i16 = 8;
pub const DESCRIPTION: i16 = 9;
pub const ACTIVE_TAGSETS: i16 = 11;
pub const ANTI_CLUMP: i16 = 12;
pub const TFL: i16 = 13;
pub const SPLASH: i16 = 14;
pub const CAMPAIGN_CAPTION: i16 = 15;
pub const CAMPAIGNS: i16 = 16;
pub const PREVIEW: i16 = 17;
pub const FILTER: i16 = 18;

const WARNING_TEXT: i16 = 1;
const WARNING_FULL_TFL: i16 = 2;
const WARNING_ANTI_CLUMP: i16 = 3;
const WARNING_CANCEL: i16 = 4;

const NO_FILTER: &[u8] = b"(search)";
const SPLASH_ART: bool = true;
const PREVIEW_ART: bool = false;

const CHECKBOX_FLAGS: u32 = button_flags::SHAPE_LEFT | button_flags::STATE_WHEN_DISABLED;

fn list_of(dialog: &mut Dialog, id: i16) -> Option<&mut List> {
    dialog.control_mut(id)?.list_mut()
}

pub fn open(
    data: &NewGame,
    res: &mut Resources,
    stack: &mut DialogStack,
) -> Option<Box<dyn OpenScreen>> {
    res.dialog(NEW_GAME)?;
    if !data.no_menu {
        main_menu::push_behind(res, stack);
    }
    open_over_game(data, res, stack)
}

pub fn open_over_game(
    data: &NewGame,
    res: &mut Resources,
    stack: &mut DialogStack,
) -> Option<Box<dyn OpenScreen>> {
    let def = res.dialog(NEW_GAME)?;
    let art = res.art(NEW_GAME);
    let mut dialog = Dialog::new(&def, NEW_GAME, dialog_flags::MODAL, res);
    dialog.set_background(art, 0);
    if let Some(list) = list_of(&mut dialog, DIFFICULTIES) {
        list.set_flag(list_flags::KEEP_SELECTION, true);
        list.set_flag(list_flags::NO_SCROLLBAR, true);
        list.set_flag(list_flags::FRAME, false);
    }
    if let Some(list) = list_of(&mut dialog, LEVELS) {
        list.set_double_click_item(OK);
        list.set_flag(list_flags::FOCUS_RING, false);
        list.set_flag(list_flags::FRAME, true);
    }
    if let Some(list) = list_of(&mut dialog, CAMPAIGNS) {
        list.set_flag(list_flags::FRAME, false);
    }
    dialog.set_list_unfocusable(ACTIVE_TAGSETS);
    if let Some(list) = list_of(&mut dialog, ACTIVE_TAGSETS) {
        list.set_flag(list_flags::FOCUS_RING, false);
        list.set_flag(list_flags::FRAME, true);
        list.set_flag(list_flags::NO_ROW_CLICKS, true);
    }
    for id in [ANTI_CLUMP, TFL] {
        if let Some((_, button)) = dialog.control_mut(id).and_then(|c| c.button_mut()) {
            button.set_flags(CHECKBOX_FLAGS);
        }
    }
    if let Some((item, edit)) = dialog.control_mut(FILTER).and_then(|c| c.edit_mut()) {
        edit.set_text(item, NO_FILTER);
    }
    if let Some(list) = list_of(&mut dialog, CAMPAIGNS) {
        for (index, name) in data.campaigns.iter().enumerate() {
            list.add_row(&name.0, index as i32, true);
        }
    }
    if let Some(list) = list_of(&mut dialog, DIFFICULTIES) {
        for (index, difficulty) in data.difficulties.iter().enumerate() {
            list.add_row(&difficulty.name.0, index as i32, true);
        }
    }
    dialog.set_focus(LEVELS);
    dialog.set_default_item(OK);
    dialog.set_cancel_item(CANCEL);
    let mut screen = Open {
        shown: Shown::default(),
        filter: NO_FILTER.to_vec(),
        campaigns_by_name: false,
        warning_open: false,
    };
    screen.shown.update(&mut dialog, data, res, 0);
    stack.push(dialog);
    Some(Box::new(screen))
}

#[derive(Default)]
struct Shown {
    levels: Vec<mandrake_session::LevelEntry>,
    level: Option<FourCC>,
    campaign: Option<usize>,
    difficulty: Option<i16>,
    art: Option<(Vec<String>, Option<FourCC>, Option<FourCC>)>,
    enables: Option<(bool, bool, Option<ForcedGameplay>)>,
}

impl Shown {
    fn update(&mut self, dialog: &mut Dialog, data: &NewGame, res: &mut Resources, tick: u64) {
        if self.levels != data.levels {
            if let Some(list) = list_of(dialog, LEVELS) {
                list.clear();
                for (index, level) in data.levels.iter().enumerate() {
                    let look = EntryLook {
                        listed: level.listed,
                        order: level.order,
                    };
                    list.add_entry(&level.name.0, index as i32, level.selectable, look);
                }
            }
            self.levels.clone_from(&data.levels);
            self.level = None;
        }
        if self.level != data.level {
            if let Some(list) = list_of(dialog, LEVELS) {
                match data.levels.iter().position(|l| Some(l.mesh) == data.level) {
                    Some(row) => {
                        list.select_value(row as i32);
                        list.show_selected();
                    }
                    None => list.clear_selection(),
                }
            }
            self.level = data.level;
        }
        if self.campaign != Some(data.campaign) {
            if let Some(list) = list_of(dialog, CAMPAIGNS) {
                list.select_value(data.campaign as i32);
                list.show_selected();
            }
            self.campaign = Some(data.campaign);
        }
        if self.difficulty != Some(data.difficulty) {
            if let Some(list) = list_of(dialog, DIFFICULTIES) {
                list.select_value(i32::from(data.difficulty));
                list.show_selected();
            }
            let description = usize::try_from(data.difficulty)
                .ok()
                .and_then(|d| data.difficulties.get(d))
                .map(|d| d.description.0.as_slice())
                .unwrap_or_default();
            if let Some((item, text)) = dialog
                .control_mut(DESCRIPTION)
                .and_then(|c| c.static_text_mut())
            {
                text.set_text(item, description);
            }
            self.difficulty = Some(data.difficulty);
        }
        let art_changed = self.art.as_ref().is_none_or(|(plugins, preview, splash)| {
            *plugins != data.art_plugins || *preview != data.preview || *splash != data.splash
        });
        if art_changed {
            set_art(dialog, SPLASH, data, SPLASH_ART, res, tick);
            set_art(dialog, PREVIEW, data, PREVIEW_ART, res, tick);
            self.art = Some((data.art_plugins.clone(), data.preview, data.splash));
        }
        let enables = (data.tagsets, data.gameplay_options, data.forced_gameplay);
        if self.enables != Some(enables) {
            set_enables(dialog, data);
            self.enables = Some(enables);
        }
    }
}

fn set_art(
    dialog: &mut Dialog,
    id: i16,
    data: &NewGame,
    splash: bool,
    res: &mut Resources,
    tick: u64,
) {
    let tag = if splash { data.splash } else { data.preview };
    let art = tag.and_then(|tag| res.art_of_plugins(&data.art_plugins, tag));
    let visible = art.is_some();
    if let Some((item, bitmap)) = dialog.control_mut(id).and_then(|c| c.bitmap_mut()) {
        bitmap.set_drawing_flags(item, 0, tick);
        bitmap.set_art(item, art, 0, 0);
    }
    dialog.set_visible(id, visible);
}

fn set_enables(dialog: &mut Dialog, data: &NewGame) {
    dialog.set_enabled(TAGSETS, data.tagsets);
    let free = data.gameplay_options && data.forced_gameplay.is_none();
    dialog.set_enabled(ANTI_CLUMP, free);
    dialog.set_enabled(TFL, free);
    match data.forced_gameplay {
        Some(ForcedGameplay::AntiClump) => {
            set_check(dialog, ANTI_CLUMP, true);
            set_check(dialog, TFL, false);
        }
        Some(ForcedGameplay::Tfl) => {
            set_check(dialog, ANTI_CLUMP, false);
            set_check(dialog, TFL, true);
        }
        None => {}
    }
}

fn set_check(dialog: &mut Dialog, id: i16, on: bool) {
    if let Some((item, button)) = dialog.control_mut(id).and_then(|c| c.button_mut()) {
        button.set_state(item, i32::from(on));
    }
}

fn checked(dialog: &mut Dialog, id: i16) -> bool {
    dialog
        .control_mut(id)
        .and_then(|c| c.button_mut())
        .is_some_and(|(_, button)| button.state() != 0)
}

struct Open {
    shown: Shown,
    filter: Vec<u8>,
    campaigns_by_name: bool,
    warning_open: bool,
}

impl Open {
    fn filter_campaigns(&mut self, dialog: &mut Dialog) {
        let Some(text) = dialog
            .control(FILTER)
            .and_then(|c| c.edit())
            .map(|e| e.text())
        else {
            return;
        };
        if text == self.filter.as_slice() {
            return;
        }
        self.filter.clear();
        self.filter.extend_from_slice(text);
        if self.filter == NO_FILTER {
            return;
        }
        let Some(list) = list_of(dialog, CAMPAIGNS) else {
            return;
        };
        for row in 0..list.row_count() {
            let holds = list
                .row_text(row)
                .is_some_and(|name| holds_any_case(name, &self.filter));
            list.set_row_visible(row, holds);
        }
    }

    fn sort_campaigns(&mut self, dialog: &mut Dialog, data: &NewGame) {
        self.campaigns_by_name = !self.campaigns_by_name;
        let mut order: Vec<usize> = (0..data.campaigns.len()).collect();
        if self.campaigns_by_name {
            let first = order.len().min(1);
            order[first..].sort_by_key(|&i| data.campaigns[i].0.to_ascii_lowercase());
        }
        if let Some(list) = list_of(dialog, CAMPAIGNS) {
            list.sort_by(|value| {
                order
                    .iter()
                    .position(|&i| i as i32 == value)
                    .unwrap_or(usize::MAX)
            });
            list.show_selected();
        }
    }

    fn ok(&mut self, frame: &mut ScreenFrame<'_>, data: &NewGame) {
        if !data.can_start {
            return;
        }
        let tfl_on = data.gameplay_options
            && frame
                .stack
                .find_mut(NEW_GAME)
                .is_some_and(|d| checked(d, TFL));
        match data.tfl_conflict.as_ref().filter(|_| tfl_on) {
            Some(plugin) => self.warn(frame, &plugin.0),
            None => frame.out.actions.push(Action::StartLevel),
        }
    }

    fn warn(&mut self, frame: &mut ScreenFrame<'_>, plugin: &[u8]) {
        let Some(def) = frame.res.dialog(TFL_WARNING) else {
            frame.out.actions.push(Action::StartLevel);
            return;
        };
        let mut dialog = Dialog::new(&def, TFL_WARNING, dialog_flags::MODAL, frame.res);
        let text = [
            b"Warning: \"",
            plugin,
            b"\" possibly conflicts with full TFL Gameplay and may fail to load.\r\r".as_slice(),
            b"Anti-clumping is a safer alternative if you're not sure it will work with TFL.",
        ]
        .concat();
        if let Some((item, static_text)) = dialog
            .control_mut(WARNING_TEXT)
            .and_then(|c| c.static_text_mut())
        {
            static_text.set_text(item, &text);
        }
        dialog.set_default_item(WARNING_ANTI_CLUMP);
        dialog.set_cancel_item(WARNING_CANCEL);
        frame.stack.push(dialog);
        self.warning_open = true;
    }

    fn warning_hit(&mut self, item: i16, frame: &mut ScreenFrame<'_>) {
        if !matches!(item, WARNING_FULL_TFL | WARNING_ANTI_CLUMP | WARNING_CANCEL) {
            return;
        }
        frame.stack.pop();
        self.warning_open = false;
        if item == WARNING_ANTI_CLUMP
            && let Some(dialog) = frame.stack.find_mut(NEW_GAME)
        {
            set_check(dialog, TFL, false);
            set_check(dialog, ANTI_CLUMP, true);
        }
        if item != WARNING_CANCEL {
            frame.out.actions.push(Action::StartLevel);
        }
    }
}

fn holds_any_case(name: &[u8], text: &[u8]) -> bool {
    text.is_empty()
        || name
            .windows(text.len())
            .any(|window| window.eq_ignore_ascii_case(text))
}

impl OpenScreen for Open {
    fn frame(&mut self, screen: &Screen<'_>, frame: &mut ScreenFrame<'_>) {
        let (Screen::NewGame(data)
        | Screen::Playing {
            overlay: Some(mandrake_session::Overlay::NewGame(data)),
        }) = screen
        else {
            return;
        };
        for hit in frame.hits {
            if hit.dialog == TFL_WARNING {
                self.warning_hit(hit.item, frame);
                continue;
            }
            if hit.dialog != NEW_GAME || self.warning_open {
                continue;
            }
            match hit.item {
                OK => {
                    self.ok(frame, data);
                    continue;
                }
                CANCEL => {
                    frame.out.actions.push(Action::Cancel);
                    continue;
                }
                _ => {}
            }
            let Some(dialog) = frame.stack.find_mut(NEW_GAME) else {
                continue;
            };
            match hit.item {
                LEVELS => {
                    let row = list_of(dialog, LEVELS).and_then(|l| l.selected_value());
                    if let Some(level) = row.and_then(|r| data.levels.get(r as usize)) {
                        frame.out.actions.push(Action::ChooseLevel(level.mesh));
                    }
                }
                CAMPAIGNS => {
                    let row = list_of(dialog, CAMPAIGNS).and_then(|l| l.selected_value());
                    if let Some(index) = row {
                        frame.out.actions.push(Action::ChooseCampaign {
                            index: index as usize,
                            all_levels: frame.input.shift_held,
                        });
                    }
                }
                DIFFICULTIES => {
                    if let Some(row) =
                        list_of(dialog, DIFFICULTIES).and_then(|l| l.selected_value())
                    {
                        frame.out.actions.push(Action::ChooseDifficulty(row as i16));
                    }
                }
                ANTI_CLUMP => set_check(dialog, TFL, false),
                TFL => set_check(dialog, ANTI_CLUMP, false),
                CAMPAIGN_CAPTION => self.sort_campaigns(dialog, data),
                _ => {}
            }
        }
        let tick = frame.now_tick;
        if let Some(dialog) = frame.stack.find_mut(NEW_GAME) {
            self.filter_campaigns(dialog);
            self.shown.update(dialog, data, frame.res, tick);
        }
    }
}
