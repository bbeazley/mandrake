use crate::interface_config::StatusBar;
use crate::tfl_hud_scores::{self as table, TflClock, TflScorePlayer, TflTeamScore};
use crate::tfl_minimap::{TflBlipRules, TflOverheadMap, blip_colors};
use crate::world_view::WorldView;
use mandrake_common::FourCC;
use mandrake_dialogs::core::Dialog;
use mandrake_dialogs::resources::Resources;
use mandrake_dialogs::slices::ThreeSlice;
use mandrake_render::overlay::OverlayLayer;
use mandrake_tag::font::{Bitmap32, RgbColor};
use mandrake_tag::interface::InterfaceColorTable;
use mandrake_tag::tfl::menus::TflStringList;
use mandrake_tag::{DialogDefinition, Rect, TagArchive};
use mandrake_view::interface_bitmap::InterfaceArt;
use std::sync::Arc;

pub(crate) const STATUS_BAR: FourCC = FourCC(*b"stba");
pub(crate) const STATUS_ART: FourCC = FourCC(*b"baic");
const INTERFACE_ART: FourCC = FourCC(*b"stin");
const GAME_TYPE_NAMES: FourCC = FourCC(*b"innn");
const GAME_TYPE_RULES: FourCC = FourCC(*b"innr");
pub(crate) const GAME_TYPE_SOLO: i16 = 0xb;
pub(crate) const GAME_MODE_SOLO: i16 = 2;
pub(crate) const GAME_OPTION_FOG: u32 = 4;
pub(crate) const MESH_FLAG_MAP_HIDDEN_AT_START: u32 = 0x40_0000;
const MESH_FLAGS_OFFSET: usize = 0x4c;

pub(crate) const BAR_LEFT_END: i32 = 16;
pub(crate) const BAR_RIGHT_END: i32 = 612;
const BAR_SPACER: std::ops::Range<i32> = 540..607;
const BAR_OVERLAP: i32 = 32;
const BAR_LEATHER_ROWS: std::ops::Range<i32> = 4..58;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum BarButton {
    Pause,
    OverheadMap,
    Sketch,
    EraseSketch,
}

impl BarButton {
    const fn item(self) -> i16 {
        match self {
            Self::Pause => item::PAUSE,
            Self::OverheadMap => item::OVERHEAD_MAP,
            Self::Sketch => item::SKETCH_9,
            Self::EraseSketch => item::SKETCH_8,
        }
    }
}

pub(crate) mod item {
    pub const HIDDEN_1: i16 = 1;
    pub const HIDDEN_2: i16 = 2;
    pub const PAUSE: i16 = 3;
    pub const MONSTER_STATUS: i16 = 4;
    pub const UNIT_TEXT: i16 = 5;
    pub const PORTRAIT: i16 = 6;
    pub const OVERHEAD_MAP: i16 = 7;
    pub const SKETCH_8: i16 = 8;
    pub const SKETCH_9: i16 = 9;
    pub const TITLE: i16 = 10;
    pub const DESCRIPTION: i16 = 11;
}

mod sequence {
    pub const MAP_BUTTON: i32 = 0;
    pub const EXPERIENCE: i32 = 2;
    pub const PAUSE: i32 = 3;
    pub const EMPTY: i32 = 4;
    pub const SKETCH_9: i32 = 5;
    pub const SKETCH_8: i32 = 6;
    pub const BACKGROUND: i32 = 7;
    pub const PORTRAIT_FRAME: i32 = 8;
    pub const SWATCH: i32 = 9;
}

#[derive(Debug, Clone, Default)]
pub(crate) struct TflHudSetup {
    pub(crate) title: Vec<u8>,
    pub(crate) description: Vec<u8>,
    pub(crate) game_mode: i16,
    pub(crate) film: bool,
    pub(crate) mesh_flags: u32,
    pub(crate) overhead: Option<FourCC>,
    pub(crate) cells: (i32, i32),
    pub(crate) rules: Option<TflBlipRules>,
    pub(crate) scores: Option<TflScoreSetup>,
    pub(crate) team_colors: Vec<[u8; 3]>,
    pub(crate) player_colors: Vec<[u8; 3]>,
}

fn film_player_colors(players: &[mandrake_rec::tfl::TflFilmPlayer]) -> Vec<[u8; 3]> {
    let n = players.iter().map(|p| p.slot + 1).max().unwrap_or(0);
    let mut out = vec![[0; 3]; n];
    for p in players {
        out[p.slot] = [
            (p.primary[0] >> 8) as u8,
            (p.primary[1] >> 8) as u8,
            (p.primary[2] >> 8) as u8,
        ];
    }
    out
}

impl TflHudSetup {
    pub(crate) fn of_map(map: &crate::tfl_map::TflMap) -> Self {
        let game_type = map.film.as_ref().map_or(GAME_TYPE_SOLO, |f| f.game_type);
        let strings = |list: Option<FourCC>, i: usize| {
            list.and_then(|id| map.tag(b"stli", id))
                .and_then(|d| TflStringList::from_tag(FourCC(*b"stli"), 1, &d).ok())
                .map(|l| l.get(i).to_vec())
                .unwrap_or_default()
        };
        let (title, description) = if game_type == GAME_TYPE_SOLO {
            let list = map.tfl_mesh.string_list_tag;
            (
                style_codes(&strings(list, 0)),
                style_codes(&strings(list, 1)),
            )
        } else {
            let i = game_type.max(0) as usize;
            (
                strings(Some(GAME_TYPE_NAMES), i),
                strings(Some(GAME_TYPE_RULES), i),
            )
        };
        let mesh_flags = map
            .tag(b"mesh", map.mesh.mesh_id)
            .and_then(|d| {
                d.get(MESH_FLAGS_OFFSET..MESH_FLAGS_OFFSET + 4)
                    .map(|b| u32::from_be_bytes([b[0], b[1], b[2], b[3]]))
            })
            .unwrap_or(0);
        let game_mode = map.film.as_ref().map_or(GAME_MODE_SOLO, |f| f.mode);
        let game_options = map.film.as_ref().map_or(0, |f| f.game_options);
        Self {
            title,
            description,
            game_mode,
            film: map.film.is_some(),
            scores: map.film.as_ref().map(|f| TflScoreSetup {
                automatic: game_mode == 1 && f.game_options & GAME_OPTION_NO_AUTOMATIC_TABLE == 0,
                ..TflScoreSetup::of_film(f, game_type)
            }),
            mesh_flags,
            overhead: map.tfl_mesh.overhead_tag,
            cells: (
                map.tfl_mesh.cell_width as i32,
                map.tfl_mesh.cell_height as i32,
            ),
            rules: Some(TflBlipRules {
                solo: game_mode == 0,
                local_team: 0,
                local_player: match map.film.as_ref() {
                    Some(_) if game_mode == 1 => None,
                    Some(f) => Some(f.local_player as i8),
                    None => Some(0),
                },
                fog: game_options & GAME_OPTION_FOG != 0,
                observer: map
                    .film
                    .as_ref()
                    .is_some_and(crate::tfl_game::watched_as_observer),
            }),
            team_colors: Vec::new(),
            player_colors: map
                .film
                .as_ref()
                .map(|f| film_player_colors(&f.players))
                .unwrap_or_default(),
        }
    }
}

impl TflHudSetup {
    pub(crate) fn live_network(
        map: &crate::tfl_map::TflMap,
        player: i16,
        team: Option<i16>,
    ) -> Self {
        let mut setup = Self::of_map(map);
        setup.film = false;
        if let Some(rules) = setup.rules.as_mut() {
            *rules = rules.as_viewer(player, team);
        }
        setup
    }
}

fn style_codes(text: &[u8]) -> Vec<u8> {
    let mut out = text.to_vec();
    for i in 0..out.len().saturating_sub(1) {
        if out[i] == b'\\' && out[i + 1].is_ascii_alphabetic() {
            out[i] = b'|';
        }
    }
    out
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct TflScoreSetup {
    pub(crate) game_type: i16,
    pub(crate) time_limit: i32,
    pub(crate) automatic: bool,
    pub(crate) teams: Vec<TflTeamScore>,
    pub(crate) players: Vec<TflScorePlayer>,
    pub(crate) victory_header: bool,
}

const GAME_OPTION_NO_AUTOMATIC_TABLE: u32 = 0x8;

impl TflScoreSetup {
    fn of_film(f: &mandrake_rec::tfl::TflFilmHeader, game_type: i16) -> Self {
        use mandrake_common::mac_roman;
        let mut teams: Vec<TflTeamScore> = Vec::new();
        let mut names: Vec<(i16, &str)> = Vec::new();
        let players = f
            .players
            .iter()
            .map(|p| TflScorePlayer {
                name: mac_roman::encode(&p.name),
                team_name: mac_roman::encode(&p.team_name),
                primary: [p.primary[0], p.primary[1], p.primary[2]],
                secondary: [p.secondary[0], p.secondary[1], p.secondary[2]],
                gone: p.flags & 4 != 0,
            })
            .collect();
        for p in &f.players {
            let primary = [p.primary[0], p.primary[1], p.primary[2]];
            let secondary = [p.secondary[0], p.secondary[1], p.secondary[2]];
            let name = mac_roman::encode(&p.name);
            let key = (p.team, p.team_name.as_str());
            match names.iter().position(|n| *n == key) {
                Some(i) => teams[i].players.push((name, secondary, 1)),
                None => {
                    names.push(key);
                    teams.push(TflTeamScore {
                        captain: name.clone(),
                        team_name: mac_roman::encode(&p.team_name),
                        primary,
                        alive: true,
                        lost: 0,
                        surviving: 1,
                        score: 0,
                        numbers: 0,
                        members: 0,
                        players: vec![(name, secondary, 1)],
                    });
                }
            }
        }
        for t in &mut teams {
            t.surviving = t.players.len() as i32;
            t.members = t.players.len() as i16;
        }
        Self {
            game_type,
            time_limit: f.time_limit as i32,
            automatic: true,
            teams,
            players,
            victory_header: f.game_options & crate::scores::OPTION_VICTORY_HEADER != 0,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct TflSelection {
    pub(crate) status: TflStatusRecord,
    pub(crate) text: Vec<u8>,
    pub(crate) portrait: TflPortrait,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct TflPortrait {
    pub(crate) collection: FourCC,
    pub(crate) sequence: i32,
    pub(crate) colors: Option<Vec<[u16; 4]>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct TflStatusIcon {
    pub(crate) collection: FourCC,
    pub(crate) sequence: i16,
    pub(crate) count: i16,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct TflStatusRecord {
    pub(crate) name: Vec<u8>,
    pub(crate) kind: Vec<u8>,
    pub(crate) experience: [i16; 3],
    pub(crate) ammunition: Option<TflStatusIcon>,
    pub(crate) item: Option<TflStatusIcon>,
    pub(crate) seed: i16,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct TflHudView {
    pub(crate) observer: (i32, i32),
    pub(crate) facing: u16,
}

impl TflHudView {
    pub(crate) fn of_observer(origin: [f32; 3], yaw: f32, cells: (i32, i32)) -> Self {
        let per = crate::tfl_objects::WORLD_PER_RENDER_UNIT;
        let half = |c: i32| c as f32 * crate::tfl_objects::TFL_CELL_SIZE * 0.5;
        Self {
            observer: (
                ((half(cells.0) - origin[0]) * per) as i32,
                ((origin[2] + half(cells.1)) * per) as i32,
            ),
            facing: ((yaw - std::f32::consts::FRAC_PI_2).rem_euclid(std::f32::consts::TAU)
                / std::f32::consts::TAU
                * 65536.0) as i32 as u16,
        }
    }
}

pub(crate) struct TflHud {
    res: Resources,
    bar: Option<Dialog>,
    bar_def: Option<DialogDefinition>,
    bar_mode: StatusBar,
    map: Option<TflOverheadMap>,
    colors: Option<InterfaceColorTable>,
    setup: TflHudSetup,
    selection: Option<TflSelection>,
    selection_texts: crate::tfl_hud_selection::SelectionTexts,
    started: bool,
    tick: u32,
    f7: bool,
    scores_toggled: bool,
    live_scores: Option<(TflClock, Vec<TflTeamScore>)>,
    chat_captains: Vec<Option<usize>>,
    back_log: bool,
    display_chat: crate::hud::DisplayChat,
    console: crate::console::Console,
    prompts: crate::command_bar::ConsolePrompts,
    layers: [Option<(Rect, Bitmap32, u64)>; 6],
    speed_popup: Option<crate::game_speed::SpeedPopup>,
    speed_popup_fade: Option<(u16, u16)>,
    key_names: crate::key_bindings::KeyNames,
    key_rows: Option<Vec<crate::key_bindings::KeyBindingRow>>,
    pub(crate) fps: crate::hud::FpsDisplay,
    bar_revision: u64,
    prompts_revision: u64,
    speed_popup_revision: u64,
    scores_revision: u64,
    key_bindings_revision: u64,
    chat_captains_revision: u64,
    layer_keys: LayerKeys,
    tracking: Option<BarButton>,
    pointer: (i32, i32),
    show_all_experience: bool,
    paused: bool,
    translucent: bool,
    trading: Option<i32>,
}

#[derive(Default)]
struct LayerKeys {
    console: Option<(Rect, u64, bool, bool, u64)>,
    speed: Option<((i32, i32), u64)>,
    prompts: Option<(Rect, u64)>,
    scores: Option<(Rect, u64, Option<TflClock>)>,
    key_bindings: Option<(Rect, u64)>,
    bar: Option<((i32, i32), u64)>,
}

const FPS_FONT: usize = 2;
const FPS_STYLE: i16 = -1;

impl TflHud {
    pub(crate) fn new(archive: impl Into<std::sync::Arc<TagArchive>>, setup: TflHudSetup) -> Self {
        let archive = archive.into();
        let mut res = Resources::from_source(move |group, id| {
            crate::tfl_map::tag_data(&archive, &group.0, id)
        });
        let colors = res
            .art(INTERFACE_ART)
            .and_then(|a| InterfaceColorTable::from_collection(&a.coll));
        let bar_def = res.dialog(STATUS_BAR);
        let bar = bar_def
            .as_ref()
            .and_then(|def| status_bar(&mut res, &setup, def, None));
        let interface = |i: usize| {
            let c = colors.as_ref().map_or([0xff; 4], |t| t.color(i));
            [c[0], c[1], c[2]]
        };
        let list = |id: [u8; 4]| {
            res.tag(FourCC(*b"stli"), FourCC(id))
                .and_then(|d| TflStringList::from_tag(FourCC(*b"stli"), 1, &d).ok())
                .map(|l| l.strings)
                .unwrap_or_default()
        };
        let key_names = crate::key_bindings::KeyNames {
            actions: list(crate::key_bindings::ACTION_NAMES_ID),
            keys: list(crate::key_bindings::KEY_NAMES_ID),
        };
        let map = setup.overhead.and_then(|id| {
            let raw = res.tag(FourCC(*b".256"), id)?;
            let art = InterfaceArt::from_bytes(raw)?;
            let rules = setup.rules?;
            let blips = blip_colors(&rules, &setup.team_colors, &setup.player_colors, &interface);
            TflOverheadMap::new(&art, setup.cells, setup.mesh_flags, &blips)
        });
        Self {
            res,
            bar,
            bar_def,
            bar_mode: StatusBar::Original,
            map,
            colors,
            setup,
            selection: None,
            selection_texts: Default::default(),
            started: false,
            tick: 0,
            f7: false,
            scores_toggled: false,
            live_scores: None,
            chat_captains: Vec::new(),
            back_log: false,
            display_chat: crate::hud::DisplayChat::default(),
            console: crate::console::Console::for_mode(1),
            prompts: crate::command_bar::ConsolePrompts::default(),
            layers: [None, None, None, None, None, None],
            speed_popup: None,
            speed_popup_fade: None,
            key_names,
            key_rows: None,
            fps: crate::hud::FpsDisplay::default(),
            bar_revision: 0,
            prompts_revision: 0,
            speed_popup_revision: 0,
            scores_revision: 0,
            key_bindings_revision: 0,
            chat_captains_revision: 0,
            layer_keys: LayerKeys::default(),
            tracking: None,
            pointer: (0, 0),
            show_all_experience: false,
            paused: false,
            translucent: false,
            trading: None,
        }
    }

    pub(crate) fn set_trading(&mut self, height: Option<i32>) {
        if self.trading != height {
            self.trading = height;
            self.tracking = None;
            self.bar_revision += 1;
        }
    }

    pub(crate) fn set_translucent_bars(&mut self, translucent: bool) {
        self.translucent = translucent;
    }

    pub(crate) fn bar_alpha(&self) -> u8 {
        mandrake_render::overlay::transparency_alpha(if self.translucent {
            mandrake_render::overlay::TRANSLUCENT_BAR_TRANSPARENCY
        } else {
            0
        })
    }

    pub(crate) fn set_paused(&mut self, paused: bool) {
        if self.paused != paused {
            self.paused = paused;
            self.show_pause();
        }
    }

    fn show_pause(&mut self) {
        let paused = self.paused;
        if let Some((it, b)) = self
            .bar
            .as_mut()
            .and_then(|d| d.control_mut(item::PAUSE))
            .and_then(|c| c.button_mut())
            && b.state() != i32::from(paused)
        {
            b.set_state(it, i32::from(paused));
            self.bar_revision += 1;
        }
    }

    pub(crate) fn press(&mut self, size: (i32, i32), x: i32, y: i32) -> bool {
        self.pointer = (x, y);
        if !self.on_bar(size, x, y) {
            return false;
        }
        self.tracking = self.bar_button_at(size, x, y);
        if self.tracking.is_none()
            && self.selection.is_some()
            && let (Some(r), Some(c)) = (
                self.bar_rect(size),
                self.bar
                    .as_ref()
                    .and_then(|d| d.control(item::MONSTER_STATUS)),
            )
            && c.item.rect.offset(r.left, r.top).contains(x, y)
        {
            self.show_all_experience = !self.show_all_experience;
            self.bar_revision += 1;
        }
        self.show_held(size);
        true
    }

    pub(crate) fn moved(&mut self, size: (i32, i32), x: i32, y: i32) {
        self.pointer = (x, y);
        if self.tracking.is_some() {
            self.show_held(size);
        }
    }

    pub(crate) fn tracking(&self) -> bool {
        self.tracking.is_some()
    }

    pub(crate) fn release(&mut self, size: (i32, i32), x: i32, y: i32) -> Option<BarButton> {
        self.pointer = (x, y);
        let t = self.tracking.take()?;
        self.show_held(size);
        self.bar_revision += 1;
        (self.bar_button_at(size, x, y) == Some(t)).then_some(t)
    }

    fn show_held(&mut self, size: (i32, i32)) {
        let (x, y) = self.pointer;
        let held = self
            .tracking
            .filter(|&t| self.bar_button_at(size, x, y) == Some(t));
        let Some(d) = self.bar.as_mut() else { return };
        let mut changed = false;
        for b in [
            BarButton::Pause,
            BarButton::OverheadMap,
            BarButton::Sketch,
            BarButton::EraseSketch,
        ] {
            if let Some(c) = d.control_mut(b.item()) {
                let pressed = held == Some(b);
                if c.item.pressed != pressed {
                    c.item.set_pressed(pressed);
                    changed = true;
                }
            }
        }
        if changed {
            self.bar_revision += 1;
        }
    }

    pub(crate) fn set_status_bar(&mut self, mode: StatusBar) {
        if self.bar_mode != mode {
            self.bar_mode = mode;
            self.bar_revision += 1;
        }
    }

    fn bar_width(&self, screen_width: i32) -> Option<i32> {
        let w = self.bar_def.as_ref()?.width();
        Some(match self.bar_mode {
            StatusBar::Original => w,
            StatusBar::FitWidth => screen_width.max(w),
        })
    }

    fn fit_bar(&mut self, screen_width: i32) {
        let (Some(width), Some(def)) = (self.bar_width(screen_width), self.bar_def.as_ref()) else {
            return;
        };
        if self
            .bar
            .as_ref()
            .is_some_and(|b| b.bounds().width() == width)
        {
            return;
        }
        let (def, slices) = if width == def.width() {
            (def.clone(), None)
        } else {
            (fit_width_definition(def, width), Some(bar_slices()))
        };
        self.bar = status_bar(&mut self.res, &self.setup, &def, slices);
        self.bar_revision += 1;
        if self.map_shown()
            && let Some((it, b)) = self
                .bar
                .as_mut()
                .and_then(|d| d.control_mut(item::OVERHEAD_MAP))
                .and_then(|c| c.button_mut())
        {
            b.set_state(it, 1);
        }
        let selection = self.selection.take();
        self.set_selection(selection);
        if self.paused {
            self.show_pause();
        }
    }

    pub(crate) fn bar_button_at(&mut self, size: (i32, i32), x: i32, y: i32) -> Option<BarButton> {
        self.fit_bar(size.0);
        let r = self.bar_rect(size)?;
        let d = self.bar.as_ref()?;
        [
            (item::PAUSE, BarButton::Pause),
            (item::OVERHEAD_MAP, BarButton::OverheadMap),
            (item::SKETCH_9, BarButton::Sketch),
            (item::SKETCH_8, BarButton::EraseSketch),
        ]
        .into_iter()
        .find(|(id, _)| {
            d.control(*id).is_some_and(|c| {
                c.item.visible() && c.item.rect.offset(r.left, r.top).contains(x, y)
            })
        })
        .map(|(_, b)| b)
    }

    pub(crate) fn on_bar(&mut self, size: (i32, i32), x: i32, y: i32) -> bool {
        self.fit_bar(size.0);
        self.bar_rect(size).is_some_and(|r| r.contains(x, y))
    }

    pub(crate) fn bar_rect(&self, size: (i32, i32)) -> Option<Rect> {
        if self.trading.is_some() {
            return None;
        }
        self.layout_rect(size)
    }

    fn layout_rect(&self, size: (i32, i32)) -> Option<Rect> {
        let b = self.bar.as_ref()?.bounds();
        let left = (size.0 - b.width()) / 2;
        let height = self.trading.unwrap_or(b.height());
        Some(Rect::new(0, left, height, left + b.width()))
    }

    pub(crate) fn map_rect(&mut self, size: (i32, i32)) -> Option<Rect> {
        self.map_layout(size);
        let m = self.map.as_ref().filter(|m| m.drawn())?;
        Some(m.rect())
    }

    fn map_layout(&mut self, size: (i32, i32)) {
        self.fit_bar(size.0);
        let bar = self.layout_rect(size);
        if let Some(m) = self.map.as_mut() {
            m.frame(size, bar);
        }
    }

    pub(crate) fn map_frame(
        &mut self,
        size: (i32, i32),
        outline: Option<&crate::minimap::OutlineView>,
    ) -> Option<(
        crate::minimap::MinimapFrame,
        &crate::minimap::OverheadMap,
        u64,
    )> {
        self.map_layout(size);
        let color = self.interface_color(crate::tfl_minimap::OUTLINE_COLOR);
        let m = self.map.as_ref()?;
        let frame = m.draws(outline, color)?;
        Some((frame, m.overhead_map(), m.revision()))
    }

    pub(crate) fn map_mouse(
        &mut self,
        size: (i32, i32),
        held: bool,
        cursor: (i32, i32),
        now: i32,
    ) -> Option<crate::tfl_minimap::MapPoll> {
        self.map_layout(size);
        let m = self.map.as_mut()?;
        m.mouse_poll(held, cursor, now)
    }

    pub(crate) fn toggle_sketch(&mut self) -> bool {
        let Some((it, b)) = self
            .bar
            .as_mut()
            .and_then(|d| d.control_mut(item::SKETCH_9))
            .and_then(|c| c.button_mut())
        else {
            return false;
        };
        let on = b.state() == 0;
        b.set_state(it, i32::from(on));
        self.bar_revision += 1;
        if let Some(m) = self.map.as_mut() {
            m.set_sketch_mode(on);
        }
        on
    }

    pub(crate) fn erase_sketch(&mut self) {
        if let Some(m) = self.map.as_mut() {
            m.erase_own_sketch();
        }
    }

    pub(crate) fn take_sketch_commands(&mut self) -> Vec<(u16, Vec<u16>)> {
        self.map
            .as_mut()
            .map(crate::tfl_minimap::TflOverheadMap::take_sketch_commands)
            .unwrap_or_default()
    }

    pub(crate) fn map_state(&self) -> i16 {
        self.map.as_ref().map_or(0, |m| i16::from(m.state()))
    }

    pub(crate) fn map_shown(&self) -> bool {
        self.map.as_ref().is_some_and(TflOverheadMap::showing)
    }

    pub(crate) fn toggle_map(&mut self) {
        self.bar_revision += 1;
        let shown = !self.map_shown();
        if let Some(m) = self.map.as_mut() {
            m.set_shown(shown);
        }
        if let Some((it, b)) = self
            .bar
            .as_mut()
            .and_then(|d| d.control_mut(item::OVERHEAD_MAP))
            .and_then(|c| c.button_mut())
        {
            b.set_state(it, i32::from(shown));
        }
    }

    pub(crate) fn sketch(&mut self, flags: u16, points: &[u16]) -> bool {
        if let Some(m) = self.map.as_mut() {
            m.add_sketch(points);
        }
        let shows = flags & 1 != 0;
        if shows && !self.map_shown() {
            self.toggle_map();
        }
        if flags & 2 != 0
            && let Some(m) = self.map.as_mut()
        {
            m.erase_sketch();
        }
        shows
    }

    pub(crate) fn sketch_points(&self) -> &[u16] {
        self.map.as_ref().map_or(&[], |m| m.sketch())
    }

    pub(crate) fn set_selection(&mut self, selection: Option<TflSelection>) {
        if self.selection == selection {
            return;
        }
        self.bar_revision += 1;
        let res = &mut self.res;
        if let Some(d) = self.bar.as_mut() {
            let on = selection.is_some();
            if let Some(s) = &selection {
                if let Some((it, t)) = d
                    .control_mut(item::UNIT_TEXT)
                    .and_then(|c| c.static_text_mut())
                {
                    t.set_text(it, &s.text);
                }
                let p = &s.portrait;
                let art = res.art(p.collection);
                if let Some((it, b)) = d.control_mut(item::PORTRAIT).and_then(|c| c.button_mut()) {
                    b.set_shape(it, art, p.sequence);
                    b.set_replacement_colors(it, p.colors.clone());
                }
            }
            for id in [item::PORTRAIT, item::MONSTER_STATUS, item::UNIT_TEXT] {
                d.set_visible(id, on);
            }
            for id in [item::TITLE, item::DESCRIPTION] {
                d.set_visible(id, !on);
            }
        }
        self.selection = selection;
    }

    pub(crate) fn update_selection(
        &mut self,
        sim: &mandrake_tfl_sim::TflSim,
        selected: &[usize],
        viewer: crate::tfl_hud_selection::Viewer,
    ) {
        let selection = crate::tfl_hud_selection::selection_of(
            sim,
            selected,
            viewer,
            &mut self.selection_texts,
        );
        self.set_selection(selection);
    }

    #[cfg(test)]
    pub(crate) fn selection(&self) -> Option<&TflSelection> {
        self.selection.as_ref()
    }

    pub(crate) fn update(&mut self, view: &WorldView) {
        self.advance_tick(view.tick);
        if let (Some(m), Some(rules)) = (self.map.as_mut(), self.setup.rules) {
            m.update(view, &rules);
        }
    }

    pub(crate) fn update_owned(&mut self, view: WorldView) {
        self.advance_tick(view.tick);
        if let (Some(m), Some(rules)) = (self.map.as_mut(), self.setup.rules) {
            m.update_owned(view, &rules);
        }
    }

    fn advance_tick(&mut self, tick: u32) {
        let ticks = tick.saturating_sub(self.tick).min(15);
        self.tick = tick;
        self.console.expire(tick as i32);
        if let Some(m) = self.map.as_mut() {
            for _ in 0..ticks {
                m.tick();
            }
        }
        if !self.started {
            self.started = true;
            if self.setup.mesh_flags & MESH_FLAG_MAP_HIDDEN_AT_START == 0 && !self.map_shown() {
                self.toggle_map();
            }
        }
    }

    pub(crate) fn print(&mut self, text: &[u8]) {
        self.console.cputs(-1, text, self.tick as i32);
    }

    pub(crate) fn console_log(&self) -> &[crate::console::ConsoleLine] {
        self.console.lines()
    }

    pub(crate) fn chat(&mut self, player: i8, text: &[u8]) {
        self.console
            .cputs(i16::from(player), text, self.tick as i32);
    }

    pub(crate) fn set_prompts(
        &mut self,
        prompts: &(Vec<u8>, Vec<u8>),
        input: &crate::console::ConsoleInput,
        now: i32,
    ) {
        let left = &prompts.0[..prompts.0.len().min(127)];
        let right = &prompts.1[..prompts.1.len().min(127)];
        let cursor = input.cursor(now);
        if self.prompts.left.as_slice() == left
            && self.prompts.right.as_slice() == right
            && self.prompts.input == *input
            && self.prompts.cursor == cursor
        {
            return;
        }
        self.prompts.left.clear();
        self.prompts.left.extend_from_slice(left);
        self.prompts.right.clear();
        self.prompts.right.extend_from_slice(right);
        self.prompts.input = input.clone();
        self.prompts.cursor = cursor;
        self.prompts_revision += 1;
    }

    fn prompt_layer(&self, size: (i32, i32)) -> Option<(Rect, Bitmap32)> {
        if !self.prompts.shown() {
            return None;
        }
        let tags = self.prompt_tags()?;
        let band = crate::command_bar::prompt_band(self.view_frame(size));
        let (w, h, rgba) = crate::command_bar::prompt_rgba(&tags, &self.prompts, band);
        let bmp = text_over_band(
            (w, h),
            &rgba,
            crate::command_bar::PROMPT_BAND_TRANSPARENCY,
            0,
        );
        Some((band, bmp))
    }

    fn prompt_tags(&self) -> Option<crate::command_bar::PromptTags> {
        let family = self.res.font(table::FONT)?;
        let c = self.interface_color(table::TEXT_COLOR);
        Some(crate::command_bar::PromptTags {
            family: (**family).clone(),
            locale: self.res.locale().clone(),
            color: c.map(|v| u16::from(v) << 8 | u16::from(v)),
            selection_strings: None,
            formation_names: None,
        })
    }

    pub(crate) fn show_speed_popup(&mut self, index: i16, now: i32) {
        let list = self
            .res
            .tag(FourCC(*b"stli"), crate::game_speed::STRING_LIST)
            .and_then(|d| TflStringList::from_tag(FourCC(*b"stli"), 1, &d).ok())
            .map(|l| l.strings);
        let popup = crate::game_speed::SpeedPopup::new(list.as_deref(), index, now);
        self.speed_popup_fade = popup.transparency(now);
        self.speed_popup = Some(popup);
        self.speed_popup_revision += 1;
    }

    pub(crate) fn show_preset_popup(
        &mut self,
        title: &[u8],
        sim: &mandrake_tfl_sim::TflSim,
        selected: &[usize],
        preset: usize,
        viewer: crate::tfl_hud_selection::Viewer,
        now: i32,
    ) {
        let record = crate::tfl_hud_selection::selection_of(
            sim,
            selected,
            viewer,
            &mut self.selection_texts,
        )
        .map(|s| s.status);
        let (name, kind) = record
            .as_ref()
            .map_or((&[][..], &[][..]), |r| (&r.name[..], &r.kind[..]));
        let popup = crate::hud::preset_popup_of(title, preset, selected.len(), (name, kind), now);
        self.speed_popup_fade = popup.transparency(now);
        self.speed_popup = Some(popup);
        self.speed_popup_revision += 1;
    }

    pub(crate) fn update_speed_popup(&mut self, now: i32) {
        let fade = self.speed_popup.as_ref().and_then(|p| p.transparency(now));
        if fade != self.speed_popup_fade {
            self.speed_popup_fade = fade;
            self.speed_popup_revision += 1;
        }
        if fade.is_none() {
            self.speed_popup = None;
        }
    }

    pub(crate) fn speed_popup_text(&self) -> Option<(&[u8], &[u8])> {
        self.speed_popup_fade?;
        let p = self.speed_popup.as_ref()?;
        Some((&p.title, &p.text))
    }

    fn speed_popup_layer(&self, size: (i32, i32)) -> Option<(Rect, Bitmap32)> {
        let (band_t, text_t) = self.speed_popup_fade?;
        let popup = self.speed_popup.as_ref()?;
        let tags = self.prompt_tags()?;
        let band = popup.band(&tags, Rect::new(0, 0, size.1, size.0));
        let (w, h, rgba) = popup.rgba(&tags, band);
        Some((band, text_over_band((w, h), &rgba, band_t, text_t)))
    }

    pub(crate) fn set_key_bindings(&mut self, controls: Option<&crate::controls::Controls>) {
        if controls.is_none() && self.key_rows.is_none() {
            return;
        }
        let rows = controls.map(|c| {
            crate::key_bindings::rows(c, &crate::controls::tfl::shown_actions(), &self.key_names)
        });
        if rows != self.key_rows {
            self.key_rows = rows;
            self.key_bindings_revision += 1;
        }
    }

    pub(crate) fn key_rows(&self) -> Option<&[crate::key_bindings::KeyBindingRow]> {
        self.key_rows.as_deref()
    }

    fn key_bindings_layer(&self, size: (i32, i32)) -> Option<(Rect, Bitmap32)> {
        let rows = self.key_rows.as_ref()?;
        let family = self.res.font(crate::key_bindings::FONT)?;
        let c = self.interface_color(table::TEXT_COLOR);
        let style = crate::key_bindings::KeyBindingsStyle {
            family,
            locale: self.res.locale(),
            color: c.map(|v| u16::from(v) << 8 | u16::from(v)),
        };
        crate::key_bindings::image(rows, self.view_frame(size), &style)
    }

    pub(crate) fn view_frame(&self, size: (i32, i32)) -> Rect {
        let top = self.layout_rect(size).map_or(0, |r| r.bottom);
        Rect::new(top, 0, size.1, size.0)
    }

    pub(crate) fn set_f7(&mut self, held: bool) {
        if self.f7 != held {
            self.f7 = held;
            self.scores_revision += 1;
        }
    }

    pub(crate) fn toggle_scores(&mut self) {
        self.scores_toggled = !self.scores_toggled;
        self.scores_revision += 1;
    }

    pub(crate) fn update_scores(&mut self, sim: &mandrake_tfl_sim::TflSim) {
        let Some(sc) = self.setup.scores.as_ref() else {
            return;
        };
        let stats_teams = sim.statistics_teams().ok();
        let captains_match = match &stats_teams {
            Some(t) => {
                t.player_teams.len() == self.chat_captains.len()
                    && t.player_teams
                        .iter()
                        .map(|&team| {
                            usize::try_from(team)
                                .ok()
                                .and_then(|team| t.claims.get(team).copied().flatten())
                        })
                        .eq(self.chat_captains.iter().copied())
            }
            None => self.chat_captains.is_empty(),
        };
        if !captains_match {
            self.chat_captains = stats_teams
                .map(|t| {
                    t.player_teams
                        .iter()
                        .map(|&team| {
                            usize::try_from(team)
                                .ok()
                                .and_then(|team| t.claims.get(team).copied().flatten())
                        })
                        .collect()
                })
                .unwrap_or_default();
            self.chat_captains_revision += 1;
        }
        let live = table::live_teams(sim, &sc.players).map(|t| (TflClock::of_sim(sim), t));
        if live != self.live_scores {
            self.live_scores = live;
            if sc.automatic || self.f7 || self.scores_toggled {
                self.scores_revision += 1;
            }
        }
    }

    pub(crate) fn display_chat_action(&mut self, held: bool, shift_up: bool, caps: bool) {
        let shown = self.display_chat.action(held, shift_up, caps);
        self.set_back_log(shown);
    }

    pub(crate) fn set_back_log(&mut self, shown: bool) {
        self.back_log = shown;
    }

    pub(crate) fn set_held_keys(
        &mut self,
        controls: &crate::controls::Controls,
        keys: &crate::controls::HeldKeys,
        caps_lock: bool,
    ) {
        use crate::controls::{CapsLock, ControlAction};
        let caps = caps_lock.then_some(controls.options.caps_lock);
        let held = |a| controls.is_held(a, keys);
        self.set_key_bindings(held(ControlAction::ShowKeyBindings).then_some(controls));
        self.set_f7(held(ControlAction::ShowScores) || caps == Some(CapsLock::PlayerStats));
        self.display_chat_action(
            held(ControlAction::DisplayChat),
            false,
            caps == Some(CapsLock::ChatBacklog),
        );
    }

    pub(crate) fn console_rows(&self, size: (i32, i32)) -> Vec<crate::console::ShownLine> {
        let Some(tags) = self.prompt_tags() else {
            return Vec::new();
        };
        let width = self.view_frame(size).width();
        if self.back_log {
            crate::console::back_log_rows(&self.console, &tags, width)
        } else {
            crate::console::displayed_rows(&self.console, &tags, width)
        }
    }

    pub(crate) fn fps_image(&mut self, size: (i32, i32), text: &[u8]) -> Option<(Rect, Vec<u8>)> {
        self.fit_bar(size.0);
        let family = self.res.font(FPS_FONT)?;
        let frame = self.view_frame(size);
        let (band, black, white) =
            crate::hud::fps_text_image(family, FPS_STYLE, self.res.locale(), text, frame);
        Some((band, crate::command_bar::coverage_rgba(&black, &white)))
    }

    pub(crate) fn interface_colors(&self) -> Option<&InterfaceColorTable> {
        self.colors.as_ref()
    }

    fn interface_color(&self, i: usize) -> [u8; 3] {
        let c = self.colors.as_ref().map_or([0xff; 4], |t| t.color(i));
        [c[0], c[1], c[2]]
    }

    pub(crate) fn compose(
        &mut self,
        frame: &mut Bitmap32,
        outline: Option<&crate::minimap::OutlineView>,
        now_tick: u64,
    ) {
        let size = (frame.width, frame.height);
        self.map_layout(size);
        let color = self.interface_color(crate::tfl_minimap::OUTLINE_COLOR);
        if let Some(m) = self.map.as_ref() {
            m.draw(frame, outline, color);
        }
        let fps = self.fps.text().and_then(|text| self.fps_image(size, &text));
        let alpha = self.bar_alpha();
        for (k, layer) in self.make_layers(size, now_tick).into_iter().enumerate() {
            if LAYER_ORDER[k] == LayerKind::KeyBindings
                && let Some((band, rgba)) = fps.as_ref()
            {
                blend(
                    frame,
                    &rgba_bitmap(band.width(), band.height(), rgba),
                    band.left,
                    band.top,
                );
            }
            let Some((r, mut image)) = layer else {
                continue;
            };
            if LAYER_ORDER[k] == LayerKind::Bar {
                for p in &mut image.pixels {
                    *p = (*p & 0x00ff_ffff) | u32::from(alpha) << 24;
                }
            }
            blend(frame, &image, r.left, r.top);
        }
    }

    fn console_layer(&self, size: (i32, i32)) -> Option<(Rect, Bitmap32)> {
        let lines = self.console_lines(size);
        lines.first().map(|(first, _)| {
            let r = lines.iter().fold(*first, |a, (b, _)| union(a, *b));
            let mut c = Bitmap32::new(r.width(), r.height(), 0);
            for (band, line) in &lines {
                blit(&mut c, line, band.left - r.left, band.top - r.top);
            }
            (r, c)
        })
    }

    fn bar_layer(&mut self, size: (i32, i32), now_tick: u64) -> Option<(Rect, Bitmap32)> {
        self.bar_rect(size)
            .zip(self.bar_bitmap(now_tick))
            .map(|(r, mut b)| {
                for p in &mut b.pixels {
                    *p |= 0xff00_0000;
                }
                (r, b)
            })
    }

    fn make_layers(&mut self, size: (i32, i32), now_tick: u64) -> [Option<(Rect, Bitmap32)>; 6] {
        self.fit_bar(size.0);
        [
            self.console_layer(size),
            self.speed_popup_layer(size),
            self.prompt_layer(size),
            self.scores_layer(size),
            self.key_bindings_layer(size),
            self.bar_layer(size, now_tick),
        ]
    }

    fn update_layer_slot(slot: &mut Option<(Rect, Bitmap32, u64)>, made: Option<(Rect, Bitmap32)>) {
        match made {
            Some((r, c)) => {
                if slot.as_ref().is_none_or(|(_, old, _)| *old != c) {
                    let revision = slot.as_ref().map_or(0, |s| s.2) + 1;
                    *slot = Some((r, c, revision));
                } else if let Some(s) = slot.as_mut() {
                    s.0 = r;
                }
            }
            None => *slot = None,
        }
    }

    pub(crate) fn layers(&mut self, size: (i32, i32), now_tick: u64) -> Vec<TflHudLayer<'_>> {
        self.fit_bar(size.0);
        let frame = self.view_frame(size);

        let console_key = (
            frame,
            self.console.revision(),
            self.back_log,
            self.prompts.shown(),
            self.chat_captains_revision,
        );
        if self.layer_keys.console != Some(console_key) {
            self.layer_keys.console = Some(console_key);
            let made = self.console_layer(size);
            Self::update_layer_slot(&mut self.layers[0], made);
        }

        let speed_key = (size, self.speed_popup_revision);
        if self.layer_keys.speed != Some(speed_key) {
            self.layer_keys.speed = Some(speed_key);
            let made = self.speed_popup_layer(size);
            Self::update_layer_slot(&mut self.layers[1], made);
        }

        let prompts_key = (frame, self.prompts_revision);
        if self.layer_keys.prompts != Some(prompts_key) {
            self.layer_keys.prompts = Some(prompts_key);
            let made = self.prompt_layer(size);
            Self::update_layer_slot(&mut self.layers[2], made);
        }

        let film_clock = self.setup.scores.as_ref().and_then(|sc| {
            (self.live_scores.is_none() && (sc.automatic || self.f7 || self.scores_toggled))
                .then(|| TflClock::of_film(sc.time_limit, self.tick as i32))
        });
        let scores_key = (frame, self.scores_revision, film_clock);
        if self.layer_keys.scores != Some(scores_key) {
            self.layer_keys.scores = Some(scores_key);
            let made = self.scores_layer(size);
            Self::update_layer_slot(&mut self.layers[3], made);
        }

        let key_bindings_key = (frame, self.key_bindings_revision);
        if self.layer_keys.key_bindings != Some(key_bindings_key) {
            self.layer_keys.key_bindings = Some(key_bindings_key);
            let made = self.key_bindings_layer(size);
            Self::update_layer_slot(&mut self.layers[4], made);
        }

        let bar_key = (size, self.bar_revision);
        if self.layer_keys.bar != Some(bar_key) {
            self.layer_keys.bar = Some(bar_key);
            let made = self.bar_layer(size, now_tick);
            Self::update_layer_slot(&mut self.layers[5], made);
        }

        self.layer_list()
    }

    fn layer_list(&self) -> Vec<TflHudLayer<'_>> {
        let bar_alpha = self.bar_alpha();
        self.layers
            .iter()
            .zip(LAYER_ORDER)
            .filter_map(|(l, kind)| Some((kind, l.as_ref()?)))
            .map(|(kind, (r, image, revision))| TflHudLayer {
                x: r.left,
                y: r.top,
                image,
                revision: *revision,
                alpha: if kind == LayerKind::Bar {
                    bar_alpha
                } else {
                    0xff
                },
                layer: kind.overlay_layer(),
            })
            .collect()
    }

    fn bar_bitmap(&mut self, now_tick: u64) -> Option<Bitmap32> {
        let d = self.bar.as_mut()?;
        d.redraw(&self.res, now_tick);
        let mut bmp = d.bitmap().clone();
        if let Some((rect, record)) = self.selection.as_ref().and_then(|s| {
            let c = d.control(item::MONSTER_STATUS)?;
            Some((c.item.rect, &s.status))
        }) {
            draw_status_control(
                &mut bmp,
                rect,
                record,
                self.show_all_experience,
                &mut self.res,
            );
        }
        Some(bmp)
    }

    fn score_lines(&self) -> Option<(Vec<crate::scores::ScoreLine>, i32)> {
        let sc = self.setup.scores.as_ref()?;
        let (clock, teams) = match &self.live_scores {
            Some((c, t)) => (*c, &t[..]),
            None => (
                TflClock::of_film(sc.time_limit, self.tick as i32),
                &sc.teams[..],
            ),
        };
        let keyed = self.f7 || self.scores_toggled;
        if !sc.automatic && !keyed {
            return None;
        }
        let shown = table::shown(&clock, keyed);
        let title = table::header_name(&clock, sc.victory_header, &self.setup.title);
        let lines = table::lines(sc.game_type, &title, &clock, shown, teams);
        if lines.is_empty() {
            return None;
        }
        Some((lines, table::score_width(sc.game_type)))
    }

    fn scores_layer(&self, size: (i32, i32)) -> Option<(Rect, Bitmap32)> {
        use mandrake_render::overlay::transparency_alpha;
        let (lines, width) = self.score_lines()?;
        let tags = self.prompt_tags()?;
        let frame = self.view_frame(size);
        let rects: Vec<Rect> = lines
            .iter()
            .map(|l| crate::scores::line_rect(frame, l, width))
            .collect();
        let r = rects.iter().copied().reduce(union)?;
        let mut c = Bitmap32::new(r.width(), r.height(), 0);
        for (line, band) in lines.iter().zip(&rects) {
            let (x, y) = (band.left - r.left, band.top - r.top);
            if let Some(fill) = line.fill {
                let a = transparency_alpha(if line.active {
                    crate::scores::FILL_TRANSPARENCY
                } else {
                    crate::scores::FADED_FILL_TRANSPARENCY
                });
                let rgb = fill.map(|v| (v >> 8) as u8);
                for py in y..y + band.height() {
                    for px in x..x + band.width() {
                        over(&mut c, px, py, rgb, u32::from(a));
                    }
                }
            }
            let text_alpha = if line.active {
                0xff
            } else {
                u32::from(transparency_alpha(crate::scores::FADED_TEXT_TRANSPARENCY))
            };
            let (w, h, rgba) = crate::scores::line_rgba(&tags, line, width);
            for (k, p) in rgba.as_chunks::<4>().0.iter().enumerate() {
                let (px, py) = ((k as u32 % w) as i32, (k as u32 / w) as i32);
                if py < h as i32 && p[3] != 0 {
                    over(
                        &mut c,
                        x + px,
                        y + py,
                        [p[0], p[1], p[2]],
                        u32::from(p[3]) * text_alpha / 255,
                    );
                }
            }
        }
        Some((r, c))
    }

    fn console_lines(&self, size: (i32, i32)) -> Vec<(Rect, Bitmap32)> {
        let Some(tags) = self.prompt_tags() else {
            return Vec::new();
        };
        let frame = self.view_frame(size);
        let shown = self.prompts.shown();
        self.console_rows(size)
            .into_iter()
            .enumerate()
            .filter_map(|(k, row)| {
                let band = crate::console::line_band(frame, k, shown);
                if band.right <= band.left || band.bottom <= band.top {
                    return None;
                }
                if let Some(sender) = usize::try_from(row.player)
                    .ok()
                    .and_then(|p| self.chat_sender(p))
                {
                    return Some((band, chat_line(&tags, sender, &row.text, band)));
                }
                let (w, h, rgba) = crate::console::line_rgba(&tags, &row.text, band);
                Some((
                    band,
                    text_over_band((w, h), &rgba, crate::console::LINE_BAND_TRANSPARENCY, 0),
                ))
            })
            .collect()
    }

    fn chat_sender(&self, p: usize) -> Option<(&[u8], RgbColor)> {
        let players = &self.setup.scores.as_ref()?.players;
        let player = players.get(p)?;
        let captain = self
            .chat_captains
            .get(p)
            .copied()
            .flatten()
            .and_then(|c| players.get(c))
            .unwrap_or(player);
        Some((&player.name, captain.primary))
    }
}

const LAYER_ORDER: [LayerKind; 6] = [
    LayerKind::Console,
    LayerKind::SpeedPopup,
    LayerKind::Prompts,
    LayerKind::Scores,
    LayerKind::KeyBindings,
    LayerKind::Bar,
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LayerKind {
    Console,
    SpeedPopup,
    Prompts,
    Scores,
    KeyBindings,
    Bar,
}

impl LayerKind {
    fn overlay_layer(self) -> OverlayLayer {
        match self {
            Self::Console | Self::SpeedPopup | Self::Prompts | Self::Scores => {
                OverlayLayer::PostRasterization(1)
            }
            Self::KeyBindings => OverlayLayer::PostRasterization(crate::key_bindings::PRIORITY),
            Self::Bar => OverlayLayer::Dialog,
        }
    }
}

fn union(a: Rect, b: Rect) -> Rect {
    Rect::new(
        a.top.min(b.top),
        a.left.min(b.left),
        a.bottom.max(b.bottom),
        a.right.max(b.right),
    )
}

pub(crate) fn over(dst: &mut Bitmap32, x: i32, y: i32, rgb: [u8; 3], alpha: u32) {
    if x < 0 || y < 0 || x >= dst.width || y >= dst.height || alpha == 0 {
        return;
    }
    let d = dst.get(x, y);
    let da = d >> 24;
    let a = alpha * 255 + da * (255 - alpha);
    let ch = |s: u8, sh: u32| {
        let dc = (d >> sh) & 0xff;
        (u32::from(s) * alpha * 255 + dc * da * (255 - alpha))
            .checked_div(a)
            .map_or(0, |v| v.min(255))
    };
    dst.set(
        x,
        y,
        (a / 255).min(255) << 24 | ch(rgb[0], 16) << 16 | ch(rgb[1], 8) << 8 | ch(rgb[2], 0),
    );
}

fn rgba_bitmap(w: i32, h: i32, rgba: &[u8]) -> Bitmap32 {
    let mut bmp = Bitmap32::new(w, h, 0);
    for (p, c) in bmp.pixels.iter_mut().zip(rgba.as_chunks::<4>().0) {
        *p = u32::from(c[3]) << 24 | u32::from(c[0]) << 16 | u32::from(c[1]) << 8 | u32::from(c[2]);
    }
    bmp
}

const CHAT_TAG_WIDTH: i32 = 100;
const CHAT_TAG_TRANSPARENCY: u16 = 0x4000;

fn chat_line(
    tags: &crate::command_bar::PromptTags,
    (name, color): (&[u8], RgbColor),
    text: &[u8],
    band: Rect,
) -> Bitmap32 {
    use crate::console::{CHAT_NAME_FLAGS, LINE_BAND_TRANSPARENCY, LINE_TEXT_FLAGS, text_rgba};
    let (bw, bh) = (band.width().max(1), band.height().max(1));
    let split = CHAT_TAG_WIDTH.min(bw);
    let rest = Rect::new(band.top, band.left + split, band.bottom, band.right);
    let mut coverage = vec![0u8; (bw * bh * 4) as usize];
    let mut blit = |rgba: &[u8], w: u32, left: i32| {
        for y in 0..(h_of(rgba, w)).min(bh) {
            for x in 0..(w as i32).min(bw - left) {
                let from = ((y * w as i32 + x) * 4) as usize;
                let to = ((y * bw + left + x) * 4) as usize;
                coverage[to..to + 4].copy_from_slice(&rgba[from..from + 4]);
            }
        }
    };
    if rest.right > rest.left {
        let (w, _, rgba) = text_rgba(tags, text, rest, LINE_TEXT_FLAGS);
        blit(&rgba, w, split);
    }
    let mut bmp = text_over_band((bw as u32, bh as u32), &coverage, LINE_BAND_TRANSPARENCY, 0);
    let fill = (0xffff - u32::from(CHAT_TAG_TRANSPARENCY)) >> 8;
    let rgb = color.map(|c| (c >> 8) as u8);
    for y in 0..bh {
        for x in 0..split {
            over(&mut bmp, x, y, rgb, fill);
        }
    }
    let tag = Rect::new(band.top, band.left, band.bottom, band.left + split);
    let (w, _, rgba) = text_rgba(tags, name, tag, CHAT_NAME_FLAGS);
    for (i, px) in rgba.as_chunks::<4>().0.iter().enumerate() {
        let (x, y) = ((i as u32 % w) as i32, (i as u32 / w) as i32);
        if x < split {
            over(&mut bmp, x, y, [px[0], px[1], px[2]], u32::from(px[3]));
        }
    }
    bmp
}

fn h_of(rgba: &[u8], w: u32) -> i32 {
    (rgba.len() / 4 / w.max(1) as usize) as i32
}

fn text_over_band(
    size: (u32, u32),
    rgba: &[u8],
    band_transparency: u16,
    text_transparency: u16,
) -> Bitmap32 {
    let band_alpha = 0xffff - u32::from(band_transparency);
    let text_alpha = 0xffff - u32::from(text_transparency);
    let mut bmp = Bitmap32::new(size.0 as i32, size.1 as i32, 0);
    for (p, px) in bmp.pixels.iter_mut().zip(rgba.as_chunks::<4>().0) {
        let cov = u32::from(px[3]) * 0x101 * text_alpha / 0xffff;
        let a = cov + (0xffff - cov) * band_alpha / 0xffff;
        let ch = |t: u8| {
            (u32::from(t) * cov)
                .checked_div(a)
                .map_or(0, |v| v.min(255))
        };
        *p = (a >> 8) << 24 | ch(px[0]) << 16 | ch(px[1]) << 8 | ch(px[2]);
    }
    bmp
}

impl TflHudLayer<'_> {
    pub(crate) fn rgba(&self) -> Vec<u8> {
        self.image
            .pixels
            .iter()
            .flat_map(|&p| [(p >> 16) as u8, (p >> 8) as u8, p as u8, (p >> 24) as u8])
            .collect()
    }
}

pub(crate) struct TflHudLayer<'a> {
    pub(crate) x: i32,
    pub(crate) y: i32,
    pub(crate) image: &'a Bitmap32,
    pub(crate) revision: u64,
    pub(crate) alpha: u8,
    pub(crate) layer: OverlayLayer,
}

struct TflStatusArt(Vec<(FourCC, Arc<InterfaceArt>)>);

impl crate::unit_info::StatusArt for TflStatusArt {
    fn status_art(&self, id: FourCC) -> Option<&InterfaceArt> {
        self.0.iter().find(|(c, _)| *c == id).map(|(_, a)| &**a)
    }
}

impl TflStatusIcon {
    fn myth2(icon: Option<Self>) -> crate::unit_info::StatusIcon {
        match icon {
            Some(i) => crate::unit_info::StatusIcon {
                count: i.count,
                collection: Some(i.collection),
                sequence: i.sequence,
                extra: -1,
            },
            None => crate::unit_info::StatusIcon {
                count: 0,
                collection: None,
                sequence: -1,
                extra: -1,
            },
        }
    }
}

impl TflStatusRecord {
    pub(crate) fn myth2(&self) -> crate::unit_info::StatusRecord {
        crate::unit_info::StatusRecord {
            name: self.name.clone(),
            kind: self.kind.clone(),
            experience: self.experience,
            ammunition: TflStatusIcon::myth2(self.ammunition),
            item: TflStatusIcon::myth2(self.item),
            seed: self.seed,
        }
    }
}

pub(crate) fn draw_status_control(
    bmp: &mut Bitmap32,
    rect: Rect,
    r: &TflStatusRecord,
    show_all: bool,
    res: &mut Resources,
) -> crate::unit_info::StatusLayout {
    let Some(family) = res.font(0).cloned() else {
        return crate::unit_info::StatusLayout {
            end: rect.right,
            ..Default::default()
        };
    };
    let locale = res.locale().clone();
    let status = res.art(STATUS_ART);
    let color = swatch(status.as_ref(), 0).unwrap_or(mandrake_tag::hud::DEFAULT_SEQUENCE_COLOR);
    let arts = TflStatusArt(
        [r.ammunition, r.item]
            .into_iter()
            .flatten()
            .filter_map(|i| Some((i.collection, res.art(i.collection)?)))
            .collect(),
    );
    const _: () = assert!(sequence::EXPERIENCE == 2);
    let cx = crate::unit_info::DrawContext {
        family: &family,
        locale: &locale,
        cache: &arts,
        experience: status.as_deref(),
        color,
    };
    crate::unit_info::draw_status_control(bmp, rect, &r.myth2(), show_all, &cx)
}

fn blend(dst: &mut Bitmap32, src: &Bitmap32, x: i32, y: i32) {
    for sy in 0..src.height {
        for sx in 0..src.width {
            let (dx, dy) = (x + sx, y + sy);
            if dx < 0 || dy < 0 || dx >= dst.width || dy >= dst.height {
                continue;
            }
            let s = src.pixels[(sy * src.width + sx) as usize];
            let d = &mut dst.pixels[(dy * dst.width + dx) as usize];
            let a = s >> 24;
            let mix = |sh: u32| (((s >> sh) & 0xff) * a + ((*d >> sh) & 0xff) * (255 - a)) / 255;
            *d = (*d & 0xff00_0000) | mix(16) << 16 | mix(8) << 8 | mix(0);
        }
    }
}

fn blit(dst: &mut Bitmap32, src: &Bitmap32, x: i32, y: i32) {
    for sy in 0..src.height {
        let ty = y + sy;
        if ty < 0 || ty >= dst.height {
            continue;
        }
        for sx in 0..src.width {
            let tx = x + sx;
            if tx >= 0 && tx < dst.width {
                dst.set(tx, ty, src.get(sx, sy));
            }
        }
    }
}

fn swatch(art: Option<&Arc<InterfaceArt>>, entry: i32) -> Option<RgbColor> {
    let a = art?;
    Some(mandrake_tag::hud::rgb_color_from_sequence(
        &a.raw,
        &a.coll,
        sequence::SWATCH,
        entry,
    ))
}

pub(crate) fn fit_width_definition(def: &DialogDefinition, width: i32) -> DialogDefinition {
    let mut def = def.clone();
    let half = def.width() / 2;
    let dw = width - def.width();
    def.rect.right += dw;
    for it in &mut def.items {
        let r = &mut it.rect;
        if r.right <= half {
        } else if r.left >= half {
            r.left += dw;
            r.right += dw;
        } else {
            r.right += dw;
        }
    }
    def
}

pub(crate) fn bar_slices() -> ThreeSlice {
    use mandrake_dialogs::slices::SliceRun;
    let middle = BAR_LEFT_END..BAR_RIGHT_END;
    ThreeSlice {
        left_end: BAR_LEFT_END,
        right_end: BAR_RIGHT_END,
        middle: vec![
            SliceRun::new(middle.clone(), false),
            SliceRun::new(BAR_SPACER, false),
            SliceRun::new(middle, true),
            SliceRun::new(BAR_SPACER, true),
        ],
        overlap: BAR_OVERLAP,
        blend_rows: BAR_LEATHER_ROWS,
    }
}

const TEXT_FLAGS: u32 = 0x70 | mandrake_tag::font::text_flags::CLIP_LINES;

fn status_bar(
    res: &mut Resources,
    setup: &TflHudSetup,
    def: &DialogDefinition,
    slices: Option<ThreeSlice>,
) -> Option<Dialog> {
    let art = res.art(STATUS_ART);
    let mut d = Dialog::new(
        def,
        STATUS_BAR,
        mandrake_dialogs::core::dialog_flags::MODAL,
        res,
    );
    match slices {
        Some(s) => d.set_background_three_slice(art.clone(), sequence::BACKGROUND, s),
        None => d.set_background(art.clone(), sequence::BACKGROUND),
    }
    let shapes = [
        (item::HIDDEN_1, sequence::EMPTY),
        (item::HIDDEN_2, sequence::EMPTY),
        (item::PAUSE, sequence::PAUSE),
        (item::SKETCH_8, sequence::SKETCH_8),
        (item::SKETCH_9, sequence::SKETCH_9),
        (item::OVERHEAD_MAP, sequence::MAP_BUTTON),
    ];
    for (id, seq) in shapes {
        if let Some((it, b)) = d.control_mut(id).and_then(|c| c.button_mut()) {
            b.set_shape(it, art.clone(), seq);
        }
    }
    if let Some((it, b)) = d.control_mut(item::PORTRAIT).and_then(|c| c.button_mut()) {
        b.set_overlay_shape(it, art.clone(), sequence::PORTRAIT_FRAME);
    }
    d.set_visible(item::HIDDEN_1, false);
    d.set_visible(item::HIDDEN_2, false);
    if setup.game_mode != 1 || setup.film {
        d.set_visible(item::SKETCH_8, false);
        d.set_visible(item::SKETCH_9, false);
    }
    let text = [
        (
            item::TITLE,
            &setup.title,
            2,
            mandrake_tag::font::STYLE_BOLD,
            0x20,
        ),
        (item::DESCRIPTION, &setup.description, 3, -1, TEXT_FLAGS),
        (item::UNIT_TEXT, &Vec::new(), 1, -1, TEXT_FLAGS),
    ];
    for (id, s, entry, style, flags) in text {
        let color = swatch(art.as_ref(), entry);
        if let Some((it, t)) = d.control_mut(id).and_then(|c| c.static_text_mut()) {
            t.set_text(it, s);
            t.set_appearance(it, None, style, color, Some(flags));
        }
    }
    for id in [item::PORTRAIT, item::MONSTER_STATUS, item::UNIT_TEXT] {
        d.set_visible(id, false);
    }
    Some(d)
}
