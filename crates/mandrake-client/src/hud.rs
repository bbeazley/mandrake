use std::path::Path;

use mandrake_common::FourCC;
use mandrake_common::game_options;
use mandrake_render::{OverlayDraw, OverlayImage, OverlayLayer, OverlayRenderer};
use mandrake_tag::font::{self, Bitmap32, FontFamily, RgbColor, TextClip, TextLocale, text_flags};
use mandrake_tag::hud::{self, ControlBarEntry, status_item, status_sequence};
use mandrake_tag::interface::{
    FONT_SCREEN_LOW_RES, FONT_SYSTEM, InterfaceDefinition, STANDARD_INTERFACE_DEFINITION,
};
use mandrake_tag::{DialogDefinition, Rect, TagLookup};
use mandrake_view::interface_bitmap::{self, placement};

use crate::command_bar::{self, ConsolePrompts};
use crate::console::{self, Console, SoundSubtitles};
use crate::game_speed::{self, SpeedPopup};
use crate::scores::{self, ScoreLine, ScoreStrings};
use crate::unit_info::{self, UnitInfo, UnitTags, string_list};

pub const DIALOG_COLLECTION_ID: FourCC = FourCC(*b"baie");
pub const MESH_FLAG_MAP_HIDDEN_AT_START: u32 = 0x40_0000;
pub const MAP_BUTTON_TRANSPARENCY: u32 = 0xaaaa;
pub const GAME_OPTION_OPAQUE_MAP_BUTTON: u32 = game_options::SHOW_ENEMY;

pub const CAPTION_FLAGS: u32 = text_flags::CENTER | text_flags::MIDDLE | text_flags::SHADOW;

pub use interface_bitmap::InterfaceArt as Art;

fn load_art(tags: &TagLookup, id: FourCC) -> Option<Art> {
    Art::from_bytes(tags.get(FourCC(*b".256"), id)?)
}

fn art_color(art: &Art, sequence: usize, entry: i32) -> RgbColor {
    hud::rgb_color_from_sequence(&art.raw, &art.coll, sequence as i32, entry)
}

const FPS_BUCKET_MS: u32 = 125;
const FPS_BUCKETS: usize = 8;
pub(crate) const FPS_LAYER: OverlayLayer = OverlayLayer::PostRasterization(-100);
const FPS_TEXT_FLAGS: u32 = text_flags::WRAP | text_flags::RIGHT;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DisplayChat {
    display_chat: bool,
    clear_display_chat: bool,
}

impl Default for DisplayChat {
    fn default() -> Self {
        Self {
            display_chat: false,
            clear_display_chat: true,
        }
    }
}

impl DisplayChat {
    pub fn action(&mut self, held: bool, shift_up: bool, caps: bool) -> bool {
        if shift_up && held {
            self.clear_display_chat = !self.clear_display_chat;
        }
        if held {
            self.display_chat = true;
        } else if self.clear_display_chat {
            self.display_chat = false;
        }
        self.display_chat || caps
    }
}

#[derive(Debug, Default, Clone, PartialEq)]
pub struct FpsDisplay {
    shown: bool,
    counts: [u32; FPS_BUCKETS],
    starts: [u32; FPS_BUCKETS],
    rate: f32,
}

impl FpsDisplay {
    pub fn toggle(&mut self) {
        self.shown = !self.shown;
    }

    fn reset(&mut self) {
        *self = Self {
            shown: self.shown,
            ..Self::default()
        };
    }

    pub fn frame_presented(&mut self, now: u32) {
        let k = (now / FPS_BUCKET_MS) as usize % FPS_BUCKETS;
        if now.wrapping_sub(self.starts[k]) < FPS_BUCKET_MS {
            self.counts[k] += 1;
        } else {
            self.counts[k] = 1;
            self.starts[k] = now;
        }
        let (mut first, mut end, mut frames) = (u32::MAX, 0, 0u32);
        for (&start, &count) in self.starts.iter().zip(&self.counts) {
            if now.wrapping_sub(start).wrapping_sub(FPS_BUCKET_MS) <= 0x36a {
                first = first.min(start);
                end = end.max(start.wrapping_add(FPS_BUCKET_MS));
                frames = frames.wrapping_add(count);
            }
        }
        let seconds = end.wrapping_sub(first) as f32 / 1000.0;
        self.rate = frames as f32 / seconds;
    }

    pub fn text(&self) -> Option<Vec<u8>> {
        let mut buf = [0u8; 32];
        self.write_text(&mut buf).map(<[u8]>::to_vec)
    }

    pub fn write_text<'a>(&self, buf: &'a mut [u8; 32]) -> Option<&'a [u8]> {
        if !self.shown {
            return None;
        }
        if self.rate < 1.0 {
            let s = b"fps: --\r";
            buf[..s.len()].copy_from_slice(s);
            return Some(&buf[..s.len()]);
        }
        let mut cursor = std::io::Cursor::new(&mut buf[..]);
        let _ = std::io::Write::write_fmt(&mut cursor, format_args!("fps: {:3.1}\r", self.rate));
        let len = cursor.position() as usize;
        Some(&buf[..len])
    }
}

#[derive(Default)]
pub(crate) struct OverlayImageSlot {
    revision: Option<u64>,
    image: Option<OverlayImage>,
}

impl OverlayImageSlot {
    pub(crate) fn sync(
        &mut self,
        renderer: &OverlayRenderer,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        revision: u64,
        dims: (u32, u32),
        rgba: impl FnOnce() -> Vec<u8>,
    ) {
        let (w, h) = dims;
        match &mut self.image {
            Some(img) if img.width == w && img.height == h => {
                if self.revision != Some(revision) {
                    OverlayRenderer::update_image(queue, img, &rgba());
                    self.revision = Some(revision);
                }
            }
            slot => {
                *slot = Some(renderer.create_image(device, queue, w, h, &rgba()));
                self.revision = Some(revision);
            }
        }
    }

    pub(crate) fn clear(&mut self) {
        self.revision = None;
        self.image = None;
    }

    pub(crate) fn image(&self) -> Option<&OverlayImage> {
        self.image.as_ref()
    }
}

#[derive(Default)]
pub(crate) struct FpsOverlay {
    key: Option<(Rect, [u8; 32], usize)>,
    band: Rect,
    image: Option<OverlayImage>,
}

impl FpsOverlay {
    pub(crate) fn update(
        &mut self,
        renderer: &OverlayRenderer,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        text: Option<&[u8]>,
        frame: Rect,
        render: impl FnOnce(&[u8]) -> Option<(Rect, Vec<u8>)>,
    ) {
        let Some(text) = text else {
            self.image = None;
            self.key = None;
            return;
        };
        let mut buf = [0u8; 32];
        let len = text.len().min(buf.len());
        buf[..len].copy_from_slice(&text[..len]);
        let key = (frame, buf, len);
        if self.key.as_ref() == Some(&key) {
            return;
        }
        self.key = Some(key);
        let Some((band, rgba)) = render(&buf[..len]) else {
            self.image = None;
            return;
        };
        let (w, h) = (band.width() as u32, band.height() as u32);
        match &mut self.image {
            Some(img) if img.width == w && img.height == h => {
                OverlayRenderer::update_image(queue, img, &rgba);
                self.band = band;
            }
            slot => {
                *slot = Some(renderer.create_image(device, queue, w, h, &rgba));
                self.band = band;
            }
        }
    }

    pub(crate) fn draw(&self) -> Option<OverlayDraw<'_>> {
        let img = self.image.as_ref()?;
        Some(OverlayDraw::at(
            FPS_LAYER,
            img,
            self.band.left,
            self.band.top,
            0xff,
        ))
    }
}

pub fn fps_text_image(
    family: &FontFamily,
    style: i16,
    locale: &TextLocale,
    text: &[u8],
    frame: Rect,
) -> (Rect, Bitmap32, Bitmap32) {
    let base = &family.base;
    let width = font::string_width(family, style, text, locale) + 2 * base.margin();
    let height = base.ascending as i32 + base.descending as i32;
    let draw = |background| {
        let mut bmp = Bitmap32::new(width, height, background);
        font::draw_string(
            &mut bmp,
            Rect::new(0, 0, height, width),
            TextClip::None,
            family,
            style,
            text,
            [0xffff; 3],
            FPS_TEXT_FLAGS,
            0,
            locale,
        );
        bmp
    };
    let band = Rect::new(0, 0, height, width).offset(frame.right - width, frame.top);
    (band, draw(0xff00_0000), draw(0xffff_ffff))
}

const SOLO_GAME_TYPE: i16 = mandrake_sim::engine_rng::SOLO_GAME_TYPE;

fn fill(format: &[u8], values: &[&[u8]]) -> Vec<u8> {
    let mut out = Vec::with_capacity(format.len());
    let mut rest = format;
    for &v in values {
        let Some(at) = rest.windows(2).position(|w| w == b"%s") else {
            break;
        };
        out.extend_from_slice(&rest[..at]);
        out.extend_from_slice(v);
        rest = &rest[at + 2..];
    }
    out.extend_from_slice(rest);
    out
}

pub fn gameplay_version_string(version: i16) -> String {
    match version {
        0x4b0 => "1.2".into(),
        0x514 => "1.3".into(),
        0x582 => "1.4.x".into(),
        0x5e5 | 0x5e6 => "1.5.1".into(),
        0x69a | 0x69f => "1.7.0 BETA".into(),
        0x6ad => "1.7.1 BETA".into(),
        0x6b3 | 0x6b5 => "1.7.2 BETA".into(),
        0x708 => "1.8.0 BETA".into(),
        0x709 => "1.8.0 BETA 2".into(),
        0x70a => "1.8.0 B422".into(),
        0x712 => "1.8.1 BETA".into(),
        0x71c | 0x71d => "1.8.2 BETA".into(),
        0x730 => "1.8.4 BETA 1 (unsupported)".into(),
        0x73a | 0x73b => "1.8.5 BETA".into(),
        v => {
            let minor = v % 1000;
            format!("{}.{}.{}", v / 1000, minor / 100, minor % 100 / 10)
        }
    }
}

fn tfl_version_string(plugins: &[&str]) -> Option<&'static str> {
    plugins.iter().find_map(|&p| match p {
        "Patch 1.5 Unity" => Some("TFL"),
        "Patch 1.8.5 Unity" => Some("TFLv2"),
        _ => None,
    })
}

pub fn version_text(gameplay_version: i16, game_options: u32, plugins: &[&str]) -> Vec<u8> {
    let tfl = (game_options & game_options::TFL != 0)
        .then(|| tfl_version_string(plugins))
        .flatten()
        .map_or(String::new(), |t| format!(" |i({t})|p"));
    format!("v{}{tfl}", gameplay_version_string(gameplay_version)).into_bytes()
}

pub const GAME_RULES_ID: FourCC = FourCC(*b"innr");

pub const CHAT_CAPTIONS_ID: FourCC = FourCC(*b"incb");

pub fn disabled_transparency(t: u32) -> u32 {
    (t as i32).wrapping_mul(-0x7fff).wrapping_add(0x8000).max(0) as u32
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct BarTeam {
    pub flags: u16,
    pub players: u8,
    pub player: i16,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BarTeams {
    pub mode: i16,
    pub local_player: i16,
    pub local_team: i16,
    pub teams: Vec<BarTeam>,
}

impl Default for BarTeams {
    fn default() -> Self {
        Self {
            mode: 2,
            local_player: 0,
            local_team: -1,
            teams: Vec::new(),
        }
    }
}

impl BarTeams {
    pub fn of(world: &mandrake_sim::engine_rng::EngineWorld) -> Self {
        Self {
            mode: world.mode,
            local_player: world.local_player,
            local_team: world.local_team(),
            teams: world
                .players
                .teams
                .iter()
                .map(|t| BarTeam {
                    flags: t.flags,
                    players: t.players,
                    player: t.player,
                })
                .collect(),
        }
    }

    fn local(&self) -> Option<&BarTeam> {
        usize::try_from(self.local_team)
            .ok()
            .and_then(|t| self.teams.get(t))
    }

    pub fn chat_enabled(&self) -> bool {
        self.mode == 1 && self.local_team != -1
    }

    pub fn chat_yells(&self) -> bool {
        self.local_team == -1 || self.local().is_some_and(|t| t.players == 1)
    }

    pub fn chat_mode(&self, key_0x39: bool) -> Option<i16> {
        let team = self.local()?;
        Some(if team.players == 1 || key_0x39 { 6 } else { 5 })
    }

    pub fn ally_enabled(&self, game_options: u32) -> bool {
        self.mode == 1
            && game_options & game_options::COOPERATIVE == 0
            && self.local().is_some_and(|t| t.flags & 1 == 0)
            && self
                .teams
                .iter()
                .enumerate()
                .filter(|&(i, t)| i as i16 != self.local_team && t.players != 0)
                .count()
                > 1
    }

    pub fn ally_selects(&self, game_options: u32) -> bool {
        self.mode == 1
            && game_options & game_options::COOPERATIVE == 0
            && game_options & game_options::ALLIANCES != 0
            && self
                .local()
                .is_some_and(|t| t.player == self.local_player && t.flags & 1 == 0)
            && self
                .teams
                .iter()
                .filter(|t| t.flags & 1 == 0 && t.players != 0)
                .count()
                >= 3
    }
}

pub struct HudTags {
    pub status_dialog: DialogDefinition,
    pub control_dialog: DialogDefinition,
    pub control_captions: Vec<Vec<u8>>,
    pub status_art: Art,
    pub control_art: Art,
    pub family: FontFamily,
    pub locale: TextLocale,
    pub difficulty_names: Option<Vec<Vec<u8>>>,
    pub experience_art: Option<Art>,
    pub prompts: Option<command_bar::PromptTags>,
    pub fps_family: Option<FontFamily>,
    pub chat_captions: Option<Vec<Vec<u8>>>,
    pub alliance_strings: Vec<Vec<u8>>,
    pub key_names: crate::key_bindings::KeyNames,
    status_caption_color: RgbColor,
    tags: TagLookup,
}

impl HudTags {
    pub fn load(game_dir: &Path, plugins: &[&str]) -> anyhow::Result<Self> {
        let tags = TagLookup::open(&mandrake_tag::engine_interface_tag_files(game_dir, plugins));
        let get = |g: &[u8; 4], id: FourCC| {
            tags.get(FourCC(*g), id)
                .ok_or_else(|| anyhow::anyhow!("tag {} {id} missing", String::from_utf8_lossy(g)))
        };
        let inte = InterfaceDefinition::from_bytes(&get(b"inte", STANDARD_INTERFACE_DEFINITION)?)?;
        let font_id = inte
            .font(FONT_SYSTEM)
            .ok_or_else(|| anyhow::anyhow!("inte: no font 0"))?;
        let mut family = FontFamily::load(font_id, |id| tags.get(FourCC(*b"font"), id))?;
        family.adjust_line_height(-2);
        let locale = TextLocale::from_stli_bytes(&get(b"stli", font::TEXT_LOCALE_ID)?);
        let art = |id| load_art(&tags, id).ok_or_else(|| anyhow::anyhow!(".256 {id} missing"));
        let prompts = command_bar::PromptTags::load(&tags, &inte, locale.clone());
        let fps_family = inte
            .font(FONT_SCREEN_LOW_RES)
            .and_then(|id| FontFamily::load(id, |id| tags.get(FourCC(*b"font"), id)).ok());
        Ok(Self {
            status_dialog: DialogDefinition::from_bytes(&get(b"bina", hud::STATUS_BAR_ID)?)?,
            control_dialog: DialogDefinition::from_bytes(&get(b"bina", hud::CONTROL_BAR_ID)?)?,
            control_captions: tags
                .get(FourCC(*b"ditl"), hud::CONTROL_BAR_ID)
                .map(|d| mandrake_tag::parse_ditl(&d))
                .unwrap_or_default(),
            status_art: art(hud::STATUS_BAR_ID)?,
            control_art: art(hud::CONTROL_BAR_ID)?,
            family,
            locale,
            difficulty_names: tags
                .get(FourCC(*b"stli"), hud::DIFFICULTY_NAMES_ID)
                .map(|d| string_list(&d)),
            experience_art: load_art(&tags, hud::EXPERIENCE_ICONS_ID),
            prompts,
            fps_family,
            chat_captions: tags
                .get(FourCC(*b"stli"), CHAT_CAPTIONS_ID)
                .map(|d| string_list(&d)),
            alliance_strings: tags
                .get(FourCC(*b"stli"), crate::console::ALLIANCE_STRINGS_ID)
                .map(|d| crate::unit_info::string_list(&d))
                .unwrap_or_default(),
            key_names: {
                let list = |id: [u8; 4]| {
                    tags.get(FourCC(*b"stli"), FourCC(id))
                        .map(|d| crate::unit_info::string_list(&d))
                        .unwrap_or_default()
                };
                crate::key_bindings::KeyNames {
                    actions: list(crate::key_bindings::ACTION_NAMES_ID),
                    keys: list(crate::key_bindings::KEY_NAMES_ID),
                }
            },
            status_caption_color: load_art(&tags, DIALOG_COLLECTION_ID)
                .map_or(hud::DEFAULT_SEQUENCE_COLOR, |a| art_color(&a, 13, 2)),
            tags,
        })
    }

    pub fn lookup(&self) -> &TagLookup {
        &self.tags
    }

    pub fn control_caption_color(&self) -> RgbColor {
        hud::DEFAULT_SEQUENCE_COLOR
    }

    pub fn title(&self, mesh_strings: Option<FourCC>, game_type: i16, difficulty: i16) -> Vec<u8> {
        let name = self.map_name_bytes(mesh_strings).unwrap_or_default();
        let d = self
            .difficulty_names
            .as_deref()
            .and_then(|s| s.get(difficulty.max(0) as usize))
            .map_or(&[][..], Vec::as_slice);
        if game_type == SOLO_GAME_TYPE {
            return fill(hud::STATUS_BAR_TITLE_FORMAT.as_bytes(), &[&name, d]);
        }
        let scoring = self
            .tags
            .get(FourCC(*b"stli"), hud::GAME_SCORING_NAMES_ID)
            .and_then(|d| string_list(&d).into_iter().nth(game_type.max(0) as usize))
            .unwrap_or_default();
        fill(
            hud::STATUS_BAR_SCORED_TITLE_FORMAT.as_bytes(),
            &[&name, &scoring, d],
        )
    }

    pub fn rules(&self, mesh_strings: Option<FourCC>, game_type: i16) -> Vec<u8> {
        let (list, index) = if game_type == SOLO_GAME_TYPE {
            (mesh_strings, 1)
        } else {
            (Some(GAME_RULES_ID), game_type.max(0) as usize)
        };
        list.and_then(|id| self.tags.get(FourCC(*b"stli"), id))
            .and_then(|d| string_list(&d).into_iter().nth(index))
            .unwrap_or_default()
    }

    fn map_name_bytes(&self, mesh_strings: Option<FourCC>) -> Option<Vec<u8>> {
        mesh_strings
            .and_then(|id| self.tags.get(FourCC(*b"stli"), id))
            .and_then(|d| string_list(&d).into_iter().next())
    }

    pub fn map_name(&self, mesh_strings: Option<FourCC>) -> Option<String> {
        self.map_name_bytes(mesh_strings)
            .map(|b| mandrake_common::mac_roman::decode(&b))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BarId {
    Status,
    Control,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Button {
    pub item: u32,
    pub kind: u8,
    pub rect: Rect,
    pub sequence: i32,
    pub state: i32,
    pub views: i32,
    pub transparency: u32,
    pub caption: Vec<u8>,
    pub visible: bool,
    pub enabled: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HudEvent {
    Pause,
    Help { open: bool },
    OverheadMap { shown: bool },
    Command { kind: u8 },
}

pub struct Bar {
    pub rect: Rect,
    pub buttons: Vec<Button>,
    pub title: Option<(Rect, Vec<u8>)>,
}

#[derive(Debug, Clone)]
pub struct HudSetup {
    pub title: Vec<u8>,
    pub mesh_flags: u32,
    pub network_game: bool,
    pub game_options: u32,
}

pub struct Hud {
    tags: HudTags,
    setup: HudSetup,
    pub status: Bar,
    pub control: Bar,
    tracking: Option<(BarId, usize)>,
    pointer: (i32, i32),
    status_revision: u64,
    control_revision: u64,
    has_selection: bool,
    unit: Option<UnitInfo>,
    unit_tags: UnitTags,
    show_all_experience: bool,
    names: Vec<i8>,
    pub console: ConsolePrompts,
    pub console_lines: Console,
    pub stop_line: Option<Vec<u8>>,
    subtitles: SoundSubtitles,
    speed_popup: Option<SpeedPopup>,
    speed_popup_revision: u64,
    speed_popup_fade: Option<(u16, u16)>,
    pub fps: FpsDisplay,
    key_rows: Option<Vec<crate::key_bindings::KeyBindingRow>>,
    control_bar_shown: bool,
    view_drop: i32,
    local_time: Option<i32>,
    control_bar_fixed: bool,
    version: Vec<u8>,
    score_strings: ScoreStrings,
    score_lines: Vec<ScoreLine>,
    score_width: i32,
    bar_teams: BarTeams,
    chat_players: Vec<(Vec<u8>, [u16; 3])>,
    console_now: i32,
    rules: Vec<u8>,
    network_replay: bool,
    display_chat: DisplayChat,
    back_log: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConsoleRow {
    pub band: Rect,
    pub row: console::ShownLine,
    pub chat: Option<(Vec<u8>, [u16; 3], u32)>,
}

fn next_state(state: i32, views: i32) -> i32 {
    if state == views - 1 { 0 } else { state + 1 }
}

impl Hud {
    pub fn map_name(&self, mesh_strings: Option<FourCC>) -> Option<String> {
        self.tags.map_name(mesh_strings)
    }

    pub fn new(tags: HudTags, setup: HudSetup, width: u32, height: u32) -> Self {
        let mut hud = Self {
            status: Bar {
                rect: Rect::default(),
                buttons: Vec::new(),
                title: None,
            },
            control: Bar {
                rect: Rect::default(),
                buttons: Vec::new(),
                title: None,
            },
            tags,
            setup,
            tracking: None,
            pointer: (-1, -1),
            status_revision: 0,
            control_revision: 0,
            has_selection: false,
            unit: None,
            unit_tags: UnitTags::default(),
            show_all_experience: false,
            names: Vec::new(),
            console: ConsolePrompts::default(),
            console_lines: Console::for_mode(command_bar::CONSOLE_MODE),
            stop_line: None,
            subtitles: SoundSubtitles::default(),
            speed_popup: None,
            speed_popup_revision: 0,
            speed_popup_fade: None,
            fps: FpsDisplay::default(),
            key_rows: None,
            control_bar_shown: true,
            view_drop: 0,
            local_time: None,
            control_bar_fixed: false,
            version: Vec::new(),
            score_strings: ScoreStrings::default(),
            score_lines: Vec::new(),
            score_width: 0,
            bar_teams: BarTeams::default(),
            chat_players: Vec::new(),
            console_now: 0,
            rules: Vec::new(),
            network_replay: false,
            display_chat: DisplayChat::default(),
            back_log: false,
        };
        hud.layout(width, height);
        hud.map_start();
        hud
    }

    fn bar_revision(&self, id: BarId) -> u64 {
        match id {
            BarId::Status => self.status_revision,
            BarId::Control => self.control_revision,
        }
    }

    fn bump_bar(&mut self, id: BarId) {
        match id {
            BarId::Status => self.status_revision += 1,
            BarId::Control => self.control_revision += 1,
        }
    }

    pub fn layout(&mut self, width: u32, height: u32) {
        let (w, h) = (width as i32, height as i32);
        let old_frame = self.screen_frame();
        let keep: Vec<(BarId, u32, i32)> = self
            .status
            .buttons
            .iter()
            .map(|b| (BarId::Status, b.item, b.state))
            .chain(
                self.control
                    .buttons
                    .iter()
                    .map(|b| (BarId::Control, b.item, b.state)),
            )
            .collect();
        self.status = self.status_bar(w);
        self.control = self.control_bar(w, h);
        for (bar, item, state) in keep {
            let bar = match bar {
                BarId::Status => &mut self.status,
                BarId::Control => &mut self.control,
            };
            if let Some(b) = bar.buttons.iter_mut().find(|b| b.item == item) {
                b.state = state;
            }
        }
        self.tracking = None;
        self.status_revision += 1;
        self.control_revision += 1;
        if self.screen_frame() != old_frame {
            self.fps.reset();
        }
    }

    fn status_bar(&self, w: i32) -> Bar {
        let t = &self.tags;
        let art = &t.status_art;
        let network = self.setup.network_game;
        let layout = hud::status_bar_layout(&t.status_dialog, w);
        let rect_of = |id| layout.iter().find(|(i, _)| *i == id).map(|(_, r)| *r);
        let button = |item: u32, sequence: usize, transparency: u32| {
            rect_of(item).map(|rect| Button {
                item,
                kind: 0,
                rect,
                sequence: sequence as i32,
                state: 0,
                views: art.views(sequence as i32),
                transparency,
                caption: Vec::new(),
                visible: hud::status_bar_item_visible(item, self.has_selection, network),
                enabled: true,
            })
        };
        let map_transparency = if self.setup.game_options & GAME_OPTION_OPAQUE_MAP_BUTTON != 0 {
            0
        } else {
            MAP_BUTTON_TRANSPARENCY
        };
        let buttons = [
            button(status_item::PAUSE, status_sequence::PAUSE, 0),
            button(status_item::HELP, status_sequence::HELP, 0),
            button(
                status_item::OVERHEAD_MAP,
                status_sequence::MAP_ICON,
                map_transparency,
            ),
            button(status_item::ERASER, status_sequence::ERASER, 0),
            button(status_item::SKETCH, status_sequence::SKETCH, 0),
        ]
        .into_iter()
        .flatten()
        .collect();
        let title = rect_of(status_item::TITLE).map(|r| (r, self.setup.title.clone()));
        Bar {
            rect: hud::status_bar_rect(&t.status_dialog, w),
            buttons,
            title,
        }
    }

    fn control_bar(&self, w: i32, h: i32) -> Bar {
        let t = &self.tags;
        let art = &t.control_art;
        let entries: [ControlBarEntry; 17] = hud::control_bar_entries(self.setup.network_game);
        let sizes: Vec<(i32, i32)> = entries
            .iter()
            .map(|e| {
                art.bitmap(e.sequence as i32, 0, 0)
                    .map_or((0, 0), |(b, _)| (b.width, b.height))
            })
            .collect();
        let bar_h = t.control_dialog.height();
        let buttons = hud::control_bar_layout(&entries, &sizes, w, bar_h)
            .into_iter()
            .filter_map(|(item, rect)| {
                let e = entries.iter().find(|e| e.item == item)?;
                let mut caption = t
                    .control_captions
                    .get((item as usize).wrapping_sub(1))
                    .cloned()
                    .unwrap_or_default();
                if e.kind == 1
                    && self.bar_teams.chat_yells()
                    && let Some(yell) = t.chat_captions.as_ref().and_then(|l| l.get(1))
                {
                    caption = yell.clone();
                }
                let enabled = match e.kind {
                    1 => self.bar_teams.chat_enabled(),
                    2 => self.bar_teams.ally_enabled(self.setup.game_options),
                    _ => true,
                };
                Some(Button {
                    item: item as u32,
                    kind: e.kind,
                    rect,
                    sequence: e.sequence as i32,
                    state: 0,
                    views: art.views(e.sequence as i32),
                    transparency: 0,
                    caption,
                    visible: true,
                    enabled,
                })
            })
            .collect();
        Bar {
            rect: hud::control_bar_rect(&t.control_dialog, w, h),
            buttons,
            title: None,
        }
    }

    fn map_start(&mut self) {
        if self.setup.mesh_flags & MESH_FLAG_MAP_HIDDEN_AT_START == 0 {
            self.push(BarId::Status, status_item::OVERHEAD_MAP);
        }
        self.push(BarId::Status, status_item::HELP);
    }

    pub fn set_bar_teams(&mut self, teams: BarTeams) {
        if teams != self.bar_teams {
            self.bar_teams = teams;
            let (w, h) = (self.status.rect.width(), self.control.rect.bottom);
            self.layout(w as u32, h as u32);
        }
    }

    pub fn set_rules(&mut self, rules: Vec<u8>) {
        if rules != self.rules {
            self.rules = rules;
            self.status_revision += 1;
        }
    }

    pub fn rules_shown(&self) -> bool {
        !self.has_selection
            && (self.setup.network_game || self.network_replay)
            && self.setup.game_options & game_options::COOPERATIVE == 0
    }

    pub fn set_network_replay(&mut self, on: bool) {
        if on != self.network_replay {
            self.network_replay = on;
            self.status_revision += 1;
        }
    }

    pub fn display_chat_action(&mut self, held: bool, shift_up: bool, caps: bool) {
        self.back_log = self.display_chat.action(held, shift_up, caps);
    }

    pub fn set_chat_players(&mut self, players: &mandrake_sim::players::Players) {
        self.chat_players = (0..players.names.len().max(players.colors.len()))
            .map(|p| {
                let name = players.names.get(p).map_or(&[][..], |n| {
                    &n[..n.iter().position(|&b| b == 0).unwrap_or(n.len())]
                });
                let c = players.colors.get(p).map_or([0; 4], |c| c[0]);
                (name.to_vec(), [c[0], c[1], c[2]])
            })
            .collect();
    }

    pub fn start_chat(&mut self, mode: i16, now: i32) {
        let (left, right) = self
            .prompt_tags()
            .map(|t| t.selection_prompts((mode as usize * 2, mode as usize * 2 + 1)))
            .unwrap_or_default();
        self.console.start_chat(&left, &right, mode, now);
    }

    pub fn chat_input_fits(&self, text: &[u8]) -> bool {
        self.prompt_tags()
            .is_none_or(|t| command_bar::input_fits(t, &self.console, self.prompt_band(), text))
    }

    pub fn append_chat(&mut self, player: i16, text: &[u8], team: bool, now: i32) {
        let width = self.status.rect.width();
        if let Some(tags) = self.tags.prompts.as_ref() {
            console::append_chat(
                &mut self.console_lines,
                tags,
                player,
                text,
                team,
                width,
                now,
            );
        }
    }

    pub fn alliance_strings(&self) -> &[Vec<u8>] {
        &self.tags.alliance_strings
    }

    pub fn set_key_bindings(&mut self, controls: Option<&crate::controls::Controls>) {
        self.key_rows = controls.map(|c| {
            crate::key_bindings::rows(c, &crate::key_bindings::myth2_order(), &self.tags.key_names)
        });
    }

    pub fn key_rows(&self) -> Option<&[crate::key_bindings::KeyBindingRow]> {
        self.key_rows.as_deref()
    }

    pub fn key_bindings_style(&self) -> Option<crate::key_bindings::KeyBindingsStyle<'_>> {
        let p = self.tags.prompts.as_ref()?;
        Some(crate::key_bindings::KeyBindingsStyle {
            family: &p.family,
            locale: &p.locale,
            color: p.color,
        })
    }

    pub fn prompt_tags(&self) -> Option<&command_bar::PromptTags> {
        self.tags.prompts.as_ref()
    }

    pub fn prompt_band(&self) -> Rect {
        command_bar::prompt_band(self.view_frame())
    }

    pub fn set_local_time(&mut self, local: Option<i32>) {
        self.local_time = local;
    }

    pub fn set_view_drop(&mut self, drop: i32) {
        self.view_drop = drop;
    }

    pub fn view_frame(&self) -> Rect {
        Rect::new(
            self.status.rect.bottom + self.view_drop,
            self.status.rect.left,
            if self.control_bar_shown {
                self.control.rect.top
            } else {
                self.control.rect.bottom
            },
            self.control.rect.right,
        )
    }

    pub fn set_score_strings(&mut self, mesh_strings: Option<FourCC>, alternate_level: bool) {
        self.score_strings = ScoreStrings::load(&self.tags.tags, mesh_strings, alternate_level);
    }

    pub fn update_scores(
        &mut self,
        sim: &mandrake_sim::Simulation,
        names: &[(mandrake_session::Text, mandrake_session::Text)],
        keyed: bool,
    ) {
        let (Some(mut clock), Some(view)) = (scores::ScoreClock::of(sim), sim.statistics()) else {
            self.score_lines.clear();
            return;
        };
        if let Some(local) = self.local_time {
            clock.blink = local;
        }
        let shown = scores::shown(&clock, keyed);
        self.score_width = scores::score_width(clock.game_type);
        if shown == scores::Shown::Nothing {
            self.score_lines.clear();
            return;
        }
        let alliances = sim.engine.alliance_view();
        self.score_lines =
            scores::score_lines(&clock, shown, &view, &self.score_strings, names, &alliances);
    }

    pub fn score_lines(&self) -> &[ScoreLine] {
        &self.score_lines
    }

    pub fn set_version(&mut self, text: Vec<u8>) {
        self.version = text;
        self.status_revision += 1;
    }

    pub fn set_control_bar_shown(&mut self, shown: bool) {
        self.control_bar_shown = shown;
        if !shown && matches!(self.tracking, Some((BarId::Control, _))) {
            self.tracking = None;
        }
        self.control_revision += 1;
    }

    pub fn set_control_bar_fixed(&mut self, fixed: bool) {
        self.control_bar_fixed = fixed;
    }

    pub fn toggle_control_bar(&mut self) -> Option<bool> {
        if self.control_bar_fixed {
            return None;
        }
        let shown = !self.control_bar_shown;
        self.set_control_bar_shown(shown);
        Some(shown)
    }

    fn screen_frame(&self) -> Rect {
        Rect::new(
            self.status.rect.top,
            self.status.rect.left,
            self.control.rect.bottom,
            self.control.rect.right,
        )
    }

    pub fn show_speed_popup(&mut self, index: i16, now: i32) {
        let list = self
            .tags
            .lookup()
            .get(FourCC(*b"stli"), game_speed::STRING_LIST)
            .map(|d| string_list(&d));
        self.speed_popup = Some(SpeedPopup::new(list.as_deref(), index, now));
        self.speed_popup_revision += 1;
    }

    pub fn show_preset_popup(
        &mut self,
        world: &mandrake_sim::engine_rng::EngineWorld,
        unit_ids: &[u32],
        preset: usize,
        now: i32,
    ) {
        let selection = unit_info::selected_monsters(world, unit_ids);
        let ctx = unit_info::SelectionContext::of(world, self.setup.game_options);
        let info = UnitInfo::from_selection(
            world,
            &selection,
            &self.names,
            ctx,
            &self.tags.tags,
            &mut self.unit_tags,
        );
        let list = self
            .tags
            .lookup()
            .get(FourCC(*b"stli"), PRESET_STRING_LIST)
            .map(|d| string_list(&d));
        self.speed_popup = Some(preset_popup(
            unit_info::string_from_list(list.as_deref(), 0),
            preset,
            unit_ids.len(),
            &info,
            now,
        ));
        self.speed_popup_revision += 1;
    }

    pub fn update_speed_popup(&mut self, now: i32) {
        self.speed_popup_fade = self.speed_popup.as_ref().and_then(|p| p.transparency(now));
        if self.speed_popup_fade.is_none() && self.speed_popup.take().is_some() {
            self.speed_popup_revision += 1;
        }
    }

    fn fps_image(&self) -> Option<(Rect, Bitmap32, Bitmap32)> {
        let text = self.fps.text()?;
        let family = self.tags.fps_family.as_ref()?;
        Some(fps_text_image(
            family,
            font::STYLE_PLAIN,
            &self.tags.locale,
            &text,
            self.view_frame(),
        ))
    }

    pub fn print_sound_plays(
        &mut self,
        plays: &[mandrake_sim::map_actions::SoundActionPlay],
    ) -> Vec<Vec<u8>> {
        self.subtitles
            .print(self.tags.lookup(), plays, &mut self.console_lines)
    }

    pub fn tags_lookup(&self) -> &mandrake_tag::TagLookup {
        self.tags.lookup()
    }

    pub fn print_game_sound_line(&mut self, line: &str, now: i32) {
        let encoded = mandrake_common::mac_roman::encode(line);
        self.console_lines.cputs(-1, &encoded, now);
    }

    pub fn update_console(&mut self, now: i32) {
        self.console_lines.expire(now);
        self.console.update_cursor(now);
        self.console_now = now;
    }

    pub fn console_rows(&self) -> Vec<ConsoleRow> {
        let Some(tags) = self.prompt_tags() else {
            return Vec::new();
        };
        let frame = self.view_frame();
        let width = frame.right - frame.left;
        let rows = if self.back_log {
            console::back_log_rows(&self.console_lines, tags, width)
        } else {
            console::displayed_rows(&self.console_lines, tags, width)
        };
        rows.into_iter()
            .enumerate()
            .map(|(k, row)| {
                let band = console::line_band(frame, k, self.console.shown());
                let chat = usize::try_from(row.player).ok().map(|p| {
                    let (name, color) = self.chat_players.get(p).cloned().unwrap_or_default();
                    let fade = if self.back_log {
                        0
                    } else {
                        console::chat_fade(&row, self.console_lines.life, self.console_now)
                    };
                    (name, color, fade)
                });
                ConsoleRow { band, row, chat }
            })
            .collect()
    }

    pub fn formation_mode(&mut self, formation: i16) {
        if let Some(t) = &self.tags.prompts {
            let (left, right) = t.formation_prompts(formation);
            self.console
                .set(&left, &right, command_bar::SELECTION_MODE_FORMATION);
        }
    }

    pub fn special_ability_mode(&mut self, list: Option<mandrake_common::FourCC>) {
        let own = list
            .and_then(|id| {
                self.tags
                    .lookup()
                    .get(mandrake_common::FourCC(*b"stli"), id)
            })
            .map(|d| {
                let l = string_list(&d);
                let get = |i| l.get(i).cloned().unwrap_or_default();
                (get(0), get(1))
            });
        let pair = own.or_else(|| {
            self.tags
                .prompts
                .as_ref()
                .map(|t| t.selection_prompts(command_bar::SPECIAL_ABILITY_STRINGS))
        });
        let (left, right) = pair.unwrap_or_default();
        self.console
            .set(&left, &right, command_bar::SELECTION_MODE_SPECIAL);
    }

    pub fn attack_ground_mode(&mut self) {
        let (left, right) = self
            .tags
            .prompts
            .as_ref()
            .map(|t| t.selection_prompts(command_bar::ATTACK_GROUND_STRINGS))
            .unwrap_or_default();
        self.console
            .set(&left, &right, command_bar::SELECTION_MODE_ATTACK_GROUND);
    }

    pub fn set_paused(&mut self, paused: bool) {
        if let Some(b) = self
            .status
            .buttons
            .iter_mut()
            .find(|b| b.item == status_item::PAUSE)
            && b.state != paused as i32
        {
            b.state = paused as i32;
            self.status_revision += 1;
        }
    }

    pub fn overhead_map_shown(&self) -> bool {
        self.button(BarId::Status, status_item::OVERHEAD_MAP)
            .is_some_and(|b| b.state == 1)
    }

    pub fn button(&self, bar: BarId, item: u32) -> Option<&Button> {
        self.bar(bar).buttons.iter().find(|b| b.item == item)
    }

    pub fn bar(&self, bar: BarId) -> &Bar {
        match bar {
            BarId::Status => &self.status,
            BarId::Control => &self.control,
        }
    }

    fn bar_mut(&mut self, bar: BarId) -> &mut Bar {
        match bar {
            BarId::Status => &mut self.status,
            BarId::Control => &mut self.control,
        }
    }

    pub fn push(&mut self, bar: BarId, item: u32) -> Option<HudEvent> {
        let b = self
            .bar_mut(bar)
            .buttons
            .iter_mut()
            .find(|b| b.item == item)?;
        if b.views > 1 && !(bar == BarId::Status && item == status_item::PAUSE) {
            b.state = next_state(b.state, b.views);
        }
        let (state, kind) = (b.state, b.kind);
        self.bump_bar(bar);
        Some(match (bar, item) {
            (BarId::Status, status_item::PAUSE) => HudEvent::Pause,
            (BarId::Status, status_item::HELP) => HudEvent::Help { open: state == 1 },
            (BarId::Status, status_item::OVERHEAD_MAP) => {
                HudEvent::OverheadMap { shown: state == 1 }
            }
            (BarId::Status, _) => return None,
            (BarId::Control, _) => HudEvent::Command { kind },
        })
    }

    pub fn bar_at(&self, x: i32, y: i32) -> Option<BarId> {
        [BarId::Status, BarId::Control]
            .into_iter()
            .filter(|&b| b != BarId::Control || self.control_bar_shown)
            .find(|&b| self.bar(b).rect.contains(x, y))
    }

    fn button_at(&self, x: i32, y: i32) -> Option<(BarId, usize)> {
        let id = self.bar_at(x, y)?;
        let bar = self.bar(id);
        let (lx, ly) = (x - bar.rect.left, y - bar.rect.top);
        bar.buttons
            .iter()
            .position(|b| b.visible && b.enabled && b.rect.contains(lx, ly))
            .map(|i| (id, i))
    }

    pub fn press(&mut self, x: i32, y: i32) -> bool {
        self.pointer = (x, y);
        let Some(bar) = self.bar_at(x, y) else {
            return false;
        };
        self.tracking = self.button_at(x, y);
        if self.tracking.is_none()
            && self.has_selection
            && self
                .status_item_rect(status_item::MONSTER_STATUS)
                .is_some_and(|r| r.contains(x, y))
        {
            self.show_all_experience = !self.show_all_experience;
        }
        self.bump_bar(bar);
        true
    }

    pub fn moved(&mut self, x: i32, y: i32) {
        let before = self.held();
        self.pointer = (x, y);
        let after = self.held();
        if after != before
            && let Some((bar, _)) = before.or(after)
        {
            self.bump_bar(bar);
        }
    }

    pub fn tracking(&self) -> bool {
        self.tracking.is_some()
    }

    pub fn release(&mut self, x: i32, y: i32) -> Option<HudEvent> {
        self.pointer = (x, y);
        let t = self.tracking.take()?;
        self.bump_bar(t.0);
        if self.button_at(x, y) != Some(t) {
            return None;
        }
        let item = self.bar(t.0).buttons[t.1].item;
        self.push(t.0, item)
    }

    fn held(&self) -> Option<(BarId, usize)> {
        self.tracking
            .filter(|&t| self.button_at(self.pointer.0, self.pointer.1) == Some(t))
    }

    pub fn status_item_rect(&self, item: u32) -> Option<Rect> {
        hud::status_bar_layout(&self.tags.status_dialog, self.status.rect.width())
            .into_iter()
            .find(|(i, _)| *i == item)
            .map(|(_, r)| r)
    }

    pub fn set_map_load(
        &mut self,
        world: &mandrake_sim::engine_rng::EngineWorld,
        rng_at_start: mandrake_math::BungieRng,
    ) {
        self.names = unit_info::name_indices(world, rng_at_start, &self.tags.tags);
    }

    pub fn has_selection(&self) -> bool {
        self.has_selection
    }

    pub fn unit_info(&self) -> Option<&UnitInfo> {
        self.unit.as_ref()
    }

    pub fn set_selection(
        &mut self,
        world: &mandrake_sim::engine_rng::EngineWorld,
        unit_ids: &[u32],
    ) {
        let selection = unit_info::selected_monsters(world, unit_ids);
        let ctx = unit_info::SelectionContext::of(world, self.setup.game_options);
        let info = UnitInfo::from_selection(
            world,
            &selection,
            &self.names,
            ctx,
            &self.tags.tags,
            &mut self.unit_tags,
        );
        if self.unit.as_ref() != Some(&info) {
            self.has_selection = info.portrait.is_some();
            self.unit = Some(info);
            self.status_revision += 1;
        }
    }

    fn draw_unit_info(&self, bmp: &mut Bitmap32, status_layout: &[(u32, Rect)]) {
        let Some(info) = self.unit.as_ref().filter(|_| self.has_selection) else {
            return;
        };
        let rect_of = |item: u32| {
            status_layout
                .iter()
                .find(|(i, _)| *i == item)
                .map(|(_, r)| *r)
        };
        let t = &self.tags;
        if let Some(b) = rect_of(status_item::MONSTER_STATUS) {
            let cx = unit_info::DrawContext {
                family: &t.family,
                locale: &t.locale,
                cache: &self.unit_tags,
                experience: t.experience_art.as_ref(),
                color: art_color(&t.status_art, status_sequence::COLOR_SWATCH, 0),
            };
            unit_info::draw_status_control(bmp, b, &info.record, self.show_all_experience, &cx);
        }
        if let Some(r) = rect_of(status_item::DESCRIPTION) {
            font::draw_string(
                bmp,
                r,
                TextClip::SameAsRect,
                &t.family,
                font::STYLE_PLAIN,
                &info.description,
                art_color(&t.status_art, status_sequence::COLOR_SWATCH, 1),
                text_flags::WRAP | text_flags::SHADOW | text_flags::CLIP_LINES,
                0,
                &t.locale,
            );
        }
        if let (Some(r), Some(p)) = (rect_of(status_item::PORTRAIT), info.portrait.as_ref()) {
            unit_info::draw_portrait(
                bmp,
                r,
                p,
                &self.unit_tags,
                &t.status_art,
                status_sequence::PORTRAIT_FRAME as i32,
            );
        }
    }

    pub fn draw(&self, id: BarId) -> Bitmap32 {
        let bar = self.bar(id);
        let (w, h) = (bar.rect.width(), bar.rect.height());
        let mut bmp = Bitmap32::new(w.max(0), h.max(0), 0xff00_0000);
        let local = Rect::new(0, 0, h, w);
        let t = &self.tags;
        let (art, background, caption_color) = match id {
            BarId::Status => (
                &t.status_art,
                status_sequence::BACKGROUND as i32,
                t.status_caption_color,
            ),
            BarId::Control => (
                &t.control_art,
                hud::CONTROL_BAR_BACKGROUND_SEQUENCE as i32,
                t.control_caption_color(),
            ),
        };
        interface_bitmap::tile_and_bevel_rectangle(
            &mut bmp,
            art,
            background,
            0,
            local,
            None,
            Some(&interface_bitmap::BACKGROUND_ORDER),
        );
        let held = self.held();
        for (i, b) in bar.buttons.iter().enumerate() {
            if !b.visible {
                continue;
            }
            let pressed = held == Some((id, i));
            let (state, transparency) = if b.enabled {
                (b.state, b.transparency)
            } else {
                (0, disabled_transparency(b.transparency))
            };
            interface_bitmap::draw_bitmap_from_sequence(
                &mut bmp,
                art,
                b.sequence,
                state,
                pressed as i32,
                b.rect,
                placement::REGISTRATION,
                0,
                transparency,
            );
            if !b.caption.is_empty() {
                font::draw_string(
                    &mut bmp,
                    b.rect,
                    TextClip::None,
                    &t.family,
                    font::STYLE_PLAIN,
                    &b.caption,
                    caption_color,
                    CAPTION_FLAGS,
                    transparency,
                    &t.locale,
                );
            }
        }
        if id == BarId::Status {
            let status_layout = hud::status_bar_layout(&t.status_dialog, self.status.rect.width());
            self.draw_unit_info(&mut bmp, &status_layout);
            if self.rules_shown()
                && let Some((_, r)) = status_layout.iter().find(|(i, _)| *i == status_item::RULES)
            {
                font::draw_string(
                    &mut bmp,
                    *r,
                    TextClip::SameAsRect,
                    &t.family,
                    font::STYLE_PLAIN,
                    &self.rules,
                    art_color(&t.status_art, status_sequence::COLOR_SWATCH, 3),
                    text_flags::WRAP | text_flags::SHADOW | text_flags::CLIP_LINES,
                    0,
                    &t.locale,
                );
            }
        }
        if let Some((rect, text)) = bar.title.as_ref().filter(|_| {
            id == BarId::Control
                || hud::status_bar_item_visible(
                    status_item::TITLE,
                    self.has_selection,
                    self.setup.network_game,
                )
        }) {
            font::draw_string(
                &mut bmp,
                *rect,
                TextClip::SameAsRect,
                &t.family,
                font::STYLE_BOLD,
                text,
                art_color(&t.status_art, status_sequence::COLOR_SWATCH, 2),
                text_flags::SHADOW,
                0,
                &t.locale,
            );
            if id == BarId::Status && !self.version.is_empty() {
                font::draw_string(
                    &mut bmp,
                    *rect,
                    TextClip::SameAsRect,
                    &t.family,
                    font::STYLE_BOLD,
                    &self.version,
                    art_color(&t.status_art, status_sequence::COLOR_SWATCH, 2),
                    text_flags::RIGHT | text_flags::SHADOW,
                    0,
                    &t.locale,
                );
            }
        }
        bmp
    }
}

type LinesKey = (u64, bool, Rect, bool, Option<Vec<u8>>);

struct ChatTagImage {
    rect: Rect,
    color: [u8; 3],
    tag_transparency: u16,
    name: OverlayImage,
    name_transparency: u16,
    stamp: i32,
}

struct LineImage {
    band: Rect,
    band_transparency: u16,
    text_at: (i32, i32),
    text: OverlayImage,
    tag: Option<ChatTagImage>,
}

#[derive(Default)]
pub struct HudImages {
    status: OverlayImageSlot,
    control: OverlayImageSlot,
    prompt: Option<(Rect, OverlayImage)>,
    white: Option<OverlayImage>,
    prompt_revision: Option<(u64, Rect)>,
    lines: Vec<LineImage>,
    lines_revision: Option<LinesKey>,
    speed_popup: Option<(Rect, OverlayImage)>,
    speed_popup_revision: Option<(u64, Rect)>,
    speed_popup_fade: Option<(u16, u16)>,
    fps: FpsOverlay,
    scores: Vec<(Rect, Option<[u16; 3]>, bool, OverlayImage)>,
    scores_key: Option<(Vec<ScoreLine>, i32, Rect)>,
    key_bindings: crate::key_bindings::KeyBindingsImage,
}

impl HudImages {
    pub fn update(
        &mut self,
        renderer: &OverlayRenderer,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        hud: &Hud,
    ) {
        self.update_prompts(renderer, device, queue, hud);
        self.update_lines(renderer, device, queue, hud);
        self.update_speed_popup(renderer, device, queue, hud);
        self.update_fps(renderer, device, queue, hud);
        self.update_scores(renderer, device, queue, hud);
        if let Some(style) = hud.key_bindings_style() {
            self.key_bindings.update(
                renderer,
                device,
                queue,
                hud.key_rows(),
                hud.view_frame(),
                &style,
            );
        }
        for (id, slot) in [
            (BarId::Status, &mut self.status),
            (BarId::Control, &mut self.control),
        ] {
            if id == BarId::Control && !hud.control_bar_shown {
                slot.clear();
                continue;
            }
            let rev = hud.bar_revision(id);
            if slot.revision == Some(rev) {
                continue;
            }
            let bmp = hud.draw(id);
            let dims = (bmp.width.max(0) as u32, bmp.height.max(0) as u32);
            slot.sync(renderer, device, queue, rev, dims, || bmp.to_rgba());
        }
    }

    fn update_fps(
        &mut self,
        renderer: &OverlayRenderer,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        hud: &Hud,
    ) {
        let mut buf = [0u8; 32];
        let text = hud.fps.write_text(&mut buf);
        self.fps
            .update(renderer, device, queue, text, hud.view_frame(), |_| {
                let (band, black, white) = hud.fps_image()?;
                Some((band, command_bar::coverage_rgba(&black, &white)))
            });
    }

    fn update_speed_popup(
        &mut self,
        renderer: &OverlayRenderer,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        hud: &Hud,
    ) {
        self.speed_popup_fade = hud.speed_popup_fade;
        let key = (hud.speed_popup_revision, hud.screen_frame());
        if self.speed_popup_revision == Some(key) {
            return;
        }
        self.speed_popup_revision = Some(key);
        self.speed_popup = None;
        let (Some(popup), Some(tags)) = (&hud.speed_popup, hud.prompt_tags()) else {
            return;
        };
        let band = popup.band(tags, hud.screen_frame());
        let (w, h, rgba) = popup.rgba(tags, band);
        self.speed_popup = Some((band, renderer.create_image(device, queue, w, h, &rgba)));
        if self.white.is_none() {
            self.white = Some(renderer.create_image(device, queue, 1, 1, &[0xff; 4]));
        }
    }

    fn update_scores(
        &mut self,
        renderer: &OverlayRenderer,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        hud: &Hud,
    ) {
        let frame = hud.view_frame();
        if self.scores_key.as_ref().is_some_and(|(lines, w, f)| {
            *w == hud.score_width && *f == frame && lines.as_slice() == hud.score_lines.as_slice()
        }) {
            return;
        }
        self.scores_key = Some((hud.score_lines.clone(), hud.score_width, frame));
        self.scores.clear();
        let Some(tags) = hud.prompt_tags() else {
            return;
        };
        for line in &hud.score_lines {
            let band = scores::line_rect(frame, line, hud.score_width);
            let (w, h, rgba) = scores::line_rgba(tags, line, hud.score_width);
            self.scores.push((
                band,
                line.fill,
                line.active,
                renderer.create_image(device, queue, w, h, &rgba),
            ));
        }
        if self.white.is_none() {
            self.white = Some(renderer.create_image(device, queue, 1, 1, &[0xff; 4]));
        }
    }

    pub fn score_draws(&self) -> Vec<OverlayDraw<'_>> {
        let layer = OverlayLayer::PostRasterization(1);
        let Some(white) = &self.white else {
            return Vec::new();
        };
        let mut draws = Vec::new();
        for (band, fill, active, text) in &self.scores {
            if let Some(c) = fill {
                draws.push(OverlayDraw::fill(
                    layer,
                    white,
                    [
                        band.left,
                        band.top,
                        band.right - band.left,
                        band.bottom - band.top,
                    ],
                    c.map(|v| (v >> 8) as u8),
                    mandrake_render::overlay::transparency_alpha(if *active {
                        scores::FILL_TRANSPARENCY
                    } else {
                        scores::FADED_FILL_TRANSPARENCY
                    }),
                ));
            }
            let alpha = if *active {
                0xff
            } else {
                mandrake_render::overlay::transparency_alpha(scores::FADED_TEXT_TRANSPARENCY)
            };
            draws.push(OverlayDraw::at(layer, text, band.left, band.top, alpha));
        }
        draws
    }

    fn update_prompts(
        &mut self,
        renderer: &OverlayRenderer,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        hud: &Hud,
    ) {
        let band = hud.prompt_band();
        if self.prompt_revision == Some((hud.console.revision(), band)) {
            return;
        }
        self.prompt_revision = Some((hud.console.revision(), band));
        self.prompt = None;
        let Some(tags) = hud.prompt_tags().filter(|_| hud.console.shown()) else {
            return;
        };
        if band.right <= band.left || band.bottom <= band.top {
            return;
        }
        let (w, h, rgba) = command_bar::prompt_rgba(tags, &hud.console, band);
        self.prompt = Some((band, renderer.create_image(device, queue, w, h, &rgba)));
        if self.white.is_none() {
            self.white = Some(renderer.create_image(device, queue, 1, 1, &[0xff; 4]));
        }
    }

    fn update_lines(
        &mut self,
        renderer: &OverlayRenderer,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        hud: &Hud,
    ) {
        let frame = hud.view_frame();
        let rev = hud.console_lines.revision();
        let shown = hud.console.shown();
        if self
            .lines_revision
            .as_ref()
            .is_some_and(|(r, back_log, f, s, stop)| {
                *r == rev
                    && *back_log == hud.back_log
                    && *f == frame
                    && *s == shown
                    && stop.as_deref() == hud.stop_line.as_deref()
            })
        {
            for line in &mut self.lines {
                if let Some(tag) = &mut line.tag {
                    let fade = if hud.back_log {
                        0
                    } else {
                        console::chat_fade_stamp(tag.stamp, hud.console_lines.life, hud.console_now)
                    };
                    tag.tag_transparency = console::chat_tag_transparency(fade);
                    tag.name_transparency = fade.min(0xffff) as u16;
                }
            }
            return;
        }
        self.lines_revision = Some((rev, hud.back_log, frame, shown, hud.stop_line.clone()));
        self.lines.clear();
        let Some(tags) = hud.prompt_tags() else {
            return;
        };
        let rows = hud.console_rows();
        let k = rows.len();
        for row in rows {
            let band = row.band;
            if band.right <= band.left || band.bottom <= band.top {
                continue;
            }
            let Some((name, color, fade)) = row.chat else {
                let (w, h, rgba) = console::line_rgba(tags, &row.row.text, band);
                self.lines.push(LineImage {
                    band,
                    band_transparency: console::LINE_BAND_TRANSPARENCY,
                    text_at: (band.left, band.top),
                    text: renderer.create_image(device, queue, w, h, &rgba),
                    tag: None,
                });
                continue;
            };
            let (tag, rest) = console::chat_rects(band);
            let (w, h, rgba) =
                console::text_rgba(tags, &row.row.text, rest, console::LINE_TEXT_FLAGS);
            let text = renderer.create_image(device, queue, w, h, &rgba);
            let (nw, nh, name_rgba) =
                console::text_rgba(tags, &name, tag, console::CHAT_NAME_FLAGS);
            let name_image = renderer.create_image(device, queue, nw, nh, &name_rgba);
            self.lines.push(LineImage {
                band,
                band_transparency: if row.row.team {
                    console::TEAM_LINE_BAND_TRANSPARENCY
                } else {
                    console::LINE_BAND_TRANSPARENCY
                },
                text_at: (rest.left, rest.top),
                text,
                tag: Some(ChatTagImage {
                    rect: tag,
                    color: color.map(|c| (c >> 8) as u8),
                    tag_transparency: console::chat_tag_transparency(fade),
                    name: name_image,
                    name_transparency: fade.min(0xffff) as u16,
                    stamp: row.row.stamp,
                }),
            });
        }
        if let Some(text) = &hud.stop_line {
            let band = console::line_band(frame, k, shown);
            if band.right > band.left && band.bottom > band.top {
                let (w, h, rgba) = console::line_rgba(tags, text, band);
                self.lines.push(LineImage {
                    band,
                    band_transparency: console::LINE_BAND_TRANSPARENCY,
                    text_at: (band.left, band.top),
                    text: renderer.create_image(device, queue, w, h, &rgba),
                    tag: None,
                });
            }
        }
        if self.white.is_none() {
            self.white = Some(renderer.create_image(device, queue, 1, 1, &[0xff; 4]));
        }
    }

    pub fn console_draws(&self) -> Vec<OverlayDraw<'_>> {
        let layer = OverlayLayer::PostRasterization(1);
        let Some(white) = &self.white else {
            return Vec::new();
        };
        let fill_in = |band: &Rect, color: [u8; 3], transparency| {
            OverlayDraw::fill(
                layer,
                white,
                [
                    band.left,
                    band.top,
                    band.right - band.left,
                    band.bottom - band.top,
                ],
                color,
                mandrake_render::overlay::transparency_alpha(transparency),
            )
        };
        let fill = |band: &Rect, transparency| fill_in(band, [0; 3], transparency);
        let mut draws = Vec::new();
        for line in &self.lines {
            draws.push(fill(&line.band, line.band_transparency));
            if let Some(tag) = &line.tag {
                draws.push(fill_in(&tag.rect, tag.color, tag.tag_transparency));
                draws.push(OverlayDraw::at(
                    layer,
                    &tag.name,
                    tag.rect.left,
                    tag.rect.top,
                    mandrake_render::overlay::transparency_alpha(tag.name_transparency),
                ));
            }
            draws.push(OverlayDraw::at(
                layer,
                &line.text,
                line.text_at.0,
                line.text_at.1,
                0xff,
            ));
        }
        if let (Some((band, text)), Some((band_t, text_t))) =
            (&self.speed_popup, self.speed_popup_fade)
        {
            draws.push(fill(band, band_t));
            draws.push(OverlayDraw::at(
                layer,
                text,
                band.left,
                band.top,
                mandrake_render::overlay::transparency_alpha(text_t),
            ));
        }
        let Some((band, text)) = &self.prompt else {
            return draws;
        };
        draws.extend([
            fill(band, command_bar::PROMPT_BAND_TRANSPARENCY),
            OverlayDraw::at(layer, text, band.left, band.top, 0xff),
        ]);
        draws
    }

    pub fn key_binding_draws(&self) -> Option<OverlayDraw<'_>> {
        self.key_bindings.draws()
    }

    pub fn fps_draws(&self) -> Option<OverlayDraw<'_>> {
        self.fps.draw()
    }

    pub fn draws<'a>(&'a self, hud: &Hud, translucent: bool) -> Vec<OverlayDraw<'a>> {
        let alpha = mandrake_render::overlay::transparency_alpha(if translucent {
            mandrake_render::overlay::TRANSLUCENT_BAR_TRANSPARENCY
        } else {
            0
        });
        [(&self.status, &hud.status), (&self.control, &hud.control)]
            .into_iter()
            .filter_map(|(slot, bar)| {
                let img = slot.image()?;
                Some(OverlayDraw::at(
                    OverlayLayer::Dialog,
                    img,
                    bar.rect.left,
                    bar.rect.top,
                    alpha,
                ))
            })
            .collect()
    }
}

const PRESET_STRING_LIST: FourCC = FourCC(*b"insp");

fn preset_popup(
    title: &[u8],
    preset: usize,
    count: usize,
    info: &UnitInfo,
    stamp: i32,
) -> SpeedPopup {
    preset_popup_of(
        title,
        preset,
        count,
        (&info.record.name, &info.record.kind),
        stamp,
    )
}

pub(crate) fn preset_popup_of(
    title: &[u8],
    preset: usize,
    count: usize,
    (name, kind): (&[u8], &[u8]),
    stamp: i32,
) -> SpeedPopup {
    let mut head = title.to_vec();
    head.extend_from_slice(format!(" {}:", preset + 1).as_bytes());
    let body: &[u8] = match count {
        0 => b"(empty)",
        1 => name,
        _ => kind,
    };
    head.truncate(0x7f);
    SpeedPopup {
        title: head,
        text: body[..body.len().min(0x7f)].to_vec(),
        stamp,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fps_after(step: u32, until: u32) -> FpsDisplay {
        let mut fps = FpsDisplay::default();
        fps.toggle();
        for t in (5000..until).step_by(step as usize) {
            fps.frame_presented(t);
        }
        fps
    }

    fn fps_text(fps: &FpsDisplay) -> String {
        String::from_utf8(fps.text().unwrap()).unwrap()
    }

    #[test]
    fn test_fps_display_toggle() {
        let mut fps = FpsDisplay::default();
        assert!(!fps.shown);
        assert_eq!(fps.text(), None);
        fps.toggle();
        assert_eq!(fps.text().as_deref(), Some(&b"fps: --\r"[..]));
        fps.toggle();
        assert_eq!(fps.text(), None);
    }

    #[test]
    fn test_fps_display_reads_presented_frames() {
        let near = |fps: &FpsDisplay, want: f32, slack: f32| {
            assert!((fps.rate - want).abs() <= slack, "{} not {want}", fps.rate);
        };
        near(&fps_after(4, 8000), 250.0, 2.0);
        near(&fps_after(5, 8000), 200.0, 2.0);
        near(&fps_after(25, 8000), 40.0, 1.0);
        let mut t30 = FpsDisplay::default();
        for n in 0..300u32 {
            t30.frame_presented(5000 + n * 1000 / 30);
        }
        near(&t30, 29.5, 1.0);
        assert_eq!(fps_text(&fps_after(2000, 20000)), "fps: --\r");
        let mut fps = fps_after(5, 8000);
        fps.frame_presented(9500);
        assert_eq!(fps_text(&fps), "fps: --\r");
        let mut fps = fps_after(5, 8000);
        for t in (8000..9000).step_by(20) {
            fps.frame_presented(t);
        }
        near(&fps, 50.0, 1.0);
        assert_eq!(format!("{:3.1}", fps_after(5, 8000).rate).len(), 5);
    }

    #[test]
    fn test_fps_display_update_rate_and_reset() {
        let mut fps = FpsDisplay::default();
        fps.toggle();
        let mut changes = 0;
        let mut last = fps.text();
        for t in (5000..7000).step_by(5) {
            fps.frame_presented(t + (t / 125) % 3);
            if fps.text() != last {
                changes += 1;
                last = fps.text();
            }
        }
        assert!(changes <= 16, "{changes} changes in 2 s");
        fps.reset();
        assert!(fps.shown);
        assert_eq!(fps.rate, 0.0);
        assert_eq!(fps.text().as_deref(), Some(&b"fps: --\r"[..]));
    }

    #[test]
    fn gameplay_versions_read_as_the_engines() {
        assert_eq!(gameplay_version_string(0x73c), "1.8.5");
        assert_eq!(gameplay_version_string(0x726), "1.8.3");
        assert_eq!(gameplay_version_string(0x578), "1.4.0");
        assert_eq!(gameplay_version_string(0x582), "1.4.x");
        assert_eq!(gameplay_version_string(0x73b), "1.8.5 BETA");
    }

    #[test]
    fn the_version_text_names_tfl_only_with_its_option() {
        let plugins = ["Patch 1.8.5 Maps", "Patch 1.8.5 Unity"];
        assert_eq!(version_text(0x73c, 0, &plugins), b"v1.8.5");
        assert_eq!(
            version_text(0x73c, game_options::TFL, &plugins),
            b"v1.8.5 |i(TFLv2)|p"
        );
        assert_eq!(version_text(0x73c, game_options::TFL, &[]), b"v1.8.5");
    }

    fn net_teams(local: i16, more: usize) -> BarTeams {
        let one = |player| BarTeam {
            flags: 0,
            players: 1,
            player,
        };
        let mut teams = vec![BarTeam::default(); 6];
        teams[local as usize] = one(0);
        teams[(local as usize + 1) % 6] = one(1);
        for k in 0..more {
            teams[(local as usize + 2 + k) % 6] = one(2 + k as i16);
        }
        BarTeams {
            mode: 1,
            local_player: 0,
            local_team: local,
            teams,
        }
    }

    #[test]
    fn enable_chatting_and_selection_ally_tests() {
        let two = net_teams(2, 0);
        assert_eq!(two.chat_mode(false), Some(6));
        assert!(!two.ally_selects(game_options::ALLIANCES));
        let three = net_teams(2, 1);
        assert!(three.ally_selects(game_options::ALLIANCES));
        assert!(!three.ally_selects(0));
        assert!(!three.ally_selects(game_options::ALLIANCES | game_options::COOPERATIVE));
        let mut led = three.clone();
        led.local_player = 1;
        assert!(!led.ally_selects(game_options::ALLIANCES));
        let mut pair = three.clone();
        pair.teams[2].players = 2;
        assert_eq!(pair.chat_mode(false), Some(5));
        assert_eq!(pair.chat_mode(true), Some(6));
        let watcher = BarTeams {
            local_team: -1,
            ..three
        };
        assert_eq!(watcher.chat_mode(false), None);
        assert!(!watcher.chat_enabled() && watcher.chat_yells());
        assert!(!BarTeams::default().chat_enabled());
    }

    #[test]
    fn the_fps_display_is_the_last_effect_under_the_bars() {
        assert_eq!(FPS_LAYER, OverlayLayer::PostRasterization(-100));
        assert!(
            FPS_LAYER > OverlayLayer::PostRasterization(3),
            "over the map"
        );
        assert!(
            FPS_LAYER > OverlayLayer::PostRasterization(1),
            "over the console"
        );
        assert!(FPS_LAYER < OverlayLayer::Dialog, "under the bars");
    }

    #[test]
    fn fill_preserves_mac_roman_bytes() {
        assert_eq!(
            fill(
                hud::STATUS_BAR_TITLE_FORMAT.as_bytes(),
                &[b"Gonen\xd5s Bridge", b"Heroic"],
            ),
            b"|bGonen\xd5s Bridge|p |i(Heroic)|p"
        );
        assert_eq!(
            fill(
                hud::STATUS_BAR_SCORED_TITLE_FORMAT.as_bytes(),
                &[b"Gonen\xd5s Bridge", b"King\xd5s Hill", b"Heroic"],
            ),
            b"|bGonen\xd5s Bridge|p King\xd5s Hill |i(Heroic)|p"
        );
    }
}
