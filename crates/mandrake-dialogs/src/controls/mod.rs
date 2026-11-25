pub mod bitmap;
pub mod button;
pub mod carnage;
pub mod color_picker;
pub mod edit;
pub mod game;
pub mod icon_popup;
pub mod list;
pub mod lobby_rows;
pub mod mpll;
pub mod numb;
pub mod progress;
pub mod prol;
pub mod repo;
pub mod snapshot;
pub mod static_text;
pub mod tab;

use mandrake_common::FourCC;
use mandrake_session::UiKey;
use mandrake_tag::font::Bitmap32;
use mandrake_tag::{DialogItem, Rect};

use crate::resources::Resources;

pub mod item_flags {
    pub const DISABLED: u16 = 0x01;
    pub const HIDDEN: u16 = 0x02;
    pub const WANTS_IDLE: u16 = 0x10;
    pub const SINGLE_STATE: u16 = 0x20;
    pub const REPORT_AT_PRESS: u16 = 0x80;
    pub const HOLD_PRESS: u16 = 0x100;
}

#[derive(Debug, Clone)]
pub struct ItemState {
    pub id: i16,
    pub kind: FourCC,
    pub rect: Rect,
    pub flags: u16,
    pub pressed: bool,
    pub dirty: bool,
}

impl ItemState {
    pub const fn enabled(&self) -> bool {
        self.flags & item_flags::DISABLED == 0
    }

    pub const fn visible(&self) -> bool {
        self.flags & item_flags::HIDDEN == 0
    }

    pub const fn live(&self) -> bool {
        self.flags & (item_flags::DISABLED | item_flags::HIDDEN) == 0
    }

    pub fn set_pressed(&mut self, pressed: bool) {
        if self.pressed != pressed {
            self.pressed = pressed;
            self.dirty = true;
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct Click {
    pub point: (i32, i32),
    pub held: bool,
    pub shift: bool,
    pub double: bool,
    pub now_tick: u64,
}

#[derive(Debug, Clone, Copy)]
pub struct Key {
    pub key: UiKey,
    pub repeat: bool,
    pub shift: bool,
    pub alt: bool,
    pub now_tick: u64,
}

pub struct DrawContext<'a> {
    pub res: &'a Resources,
    pub now_tick: u64,
    pub focused: bool,
}

pub fn default_click(item: &mut ItemState, click: &Click) -> bool {
    let inside = item.rect.contains(click.point.0, click.point.1);
    item.set_pressed(inside && click.held);
    inside && !click.held
}

pub trait ControlProc {
    fn draw(&self, _item: &ItemState, _ctx: &DrawContext<'_>, _bitmap: &mut Bitmap32) {}

    fn click(&mut self, item: &mut ItemState, click: &Click) -> bool {
        default_click(item, click)
    }

    fn key(&mut self, _item: &mut ItemState, _key: &Key) -> bool {
        false
    }

    fn text(&mut self, _item: &mut ItemState, _ch: char) -> bool {
        false
    }

    fn can_focus(&self, _item: &ItemState) -> bool {
        false
    }

    fn focus(&mut self, _item: &mut ItemState, _focused: bool) -> bool {
        false
    }

    fn idle(&mut self, _item: &mut ItemState, _now_tick: u64) -> bool {
        false
    }

    fn wheel(&mut self, _item: &mut ItemState, _delta: i32) -> bool {
        false
    }

    fn take_focus_request(&mut self) -> bool {
        false
    }

    fn take_press_request(&mut self) -> Option<i16> {
        None
    }
}

pub enum Behaviour {
    Button(button::Button),
    StaticText(static_text::StaticText),
    Bitmap(bitmap::BitmapItem),
    List(list::List),
    Edit(edit::Edit),
    Prologue(prol::Prologue),
    Tabs(tab::Tabs),
    Report(repo::Report),
    Game(game::GameGraphic),
    CarnageHeader(carnage::CarnageHeader),
    CarnageGraph(carnage::CarnageGraph),
    Snapshot(snapshot::SnapshotItem),
    Progress(progress::ProgressBar),
    ColorPicker(color_picker::ColorPicker),
    IconPopup(icon_popup::IconPopup),
    DoubleArrow(numb::DoubleArrow),
    Other,
}

const fn is_list(kind: &[u8; 4]) -> bool {
    matches!(
        kind,
        b"list" | b"slst" | b"lstm" | b"lstp" | b"lstg" | b"plli" | b"chat" | b"cstt" | b"mpll"
    )
}

pub struct Control {
    pub item: ItemState,
    pub keyboard: Option<(char, bool)>,
    pub behaviour: Behaviour,
}

impl Control {
    pub fn new(item: &DialogItem, res: &mut Resources) -> Self {
        let state = ItemState {
            id: item.id as i16,
            kind: item.kind,
            rect: item.rect,
            flags: 0,
            pressed: false,
            dirty: true,
        };
        let mut control = Self {
            item: state,
            keyboard: None,
            behaviour: Behaviour::Other,
        };
        let name = item.name.as_slice();
        control.behaviour = match &item.kind.0 {
            b"butt" => Behaviour::Button(button::Button::new(&mut control.item, name, res)),
            b"radi" => Behaviour::Button(button::Button::new_radio(&mut control.item, name, res)),
            b"chek" => {
                Behaviour::Button(button::Button::new_checkbox(&mut control.item, name, res))
            }
            b"stat" => Behaviour::StaticText(static_text::StaticText::new(name, res)),
            b"bitm" => Behaviour::Bitmap(bitmap::BitmapItem::new()),
            k if is_list(k) => Behaviour::List(list::List::new(item, res)),
            b"edit" => {
                edit::Edit::prepare(&mut control.item);
                Behaviour::Edit(edit::Edit::new(item, res))
            }
            b"pass" => {
                edit::Edit::prepare(&mut control.item);
                Behaviour::Edit(edit::Edit::new_password(item, res))
            }
            b"colr" => Behaviour::ColorPicker(color_picker::ColorPicker::new(&mut control.item)),
            b"ipop" => Behaviour::IconPopup(icon_popup::IconPopup::new(&control.item, res)),
            b"prol" => Behaviour::Prologue(prol::Prologue::new(item, res)),
            b"tabc" => Behaviour::Tabs(tab::Tabs::new(item, res)),
            b"repo" => Behaviour::Report(repo::Report::new(item, res)),
            b"game" => Behaviour::Game(game::GameGraphic::new(res)),
            b"ched" => Behaviour::CarnageHeader(carnage::CarnageHeader::new(res)),
            b"cgrf" => Behaviour::CarnageGraph(carnage::CarnageGraph::default()),
            b"snap" => Behaviour::Snapshot(snapshot::SnapshotItem::default()),
            b"prog" => Behaviour::Progress(progress::ProgressBar::new(res)),
            b"numb" => Behaviour::DoubleArrow(numb::DoubleArrow::new(&mut control.item, res)),
            _ => Behaviour::Other,
        };
        control
    }

    fn proc(&self) -> Option<&dyn ControlProc> {
        Some(match &self.behaviour {
            Behaviour::Button(c) => c,
            Behaviour::StaticText(c) => c,
            Behaviour::Bitmap(c) => c,
            Behaviour::List(c) => c,
            Behaviour::Edit(c) => c,
            Behaviour::Prologue(c) => c,
            Behaviour::Tabs(c) => c,
            Behaviour::Report(c) => c,
            Behaviour::Game(c) => c,
            Behaviour::CarnageHeader(c) => c,
            Behaviour::CarnageGraph(c) => c,
            Behaviour::Snapshot(c) => c,
            Behaviour::Progress(c) => c,
            Behaviour::ColorPicker(c) => c,
            Behaviour::IconPopup(c) => c,
            Behaviour::DoubleArrow(c) => c,
            Behaviour::Other => return None,
        })
    }

    fn proc_mut(&mut self) -> (&mut ItemState, Option<&mut dyn ControlProc>) {
        let proc: Option<&mut dyn ControlProc> = match &mut self.behaviour {
            Behaviour::Button(c) => Some(c),
            Behaviour::StaticText(c) => Some(c),
            Behaviour::Bitmap(c) => Some(c),
            Behaviour::List(c) => Some(c),
            Behaviour::Edit(c) => Some(c),
            Behaviour::Prologue(c) => Some(c),
            Behaviour::Tabs(c) => Some(c),
            Behaviour::Report(c) => Some(c),
            Behaviour::Game(c) => Some(c),
            Behaviour::CarnageHeader(c) => Some(c),
            Behaviour::CarnageGraph(c) => Some(c),
            Behaviour::Snapshot(c) => Some(c),
            Behaviour::Progress(c) => Some(c),
            Behaviour::ColorPicker(c) => Some(c),
            Behaviour::IconPopup(c) => Some(c),
            Behaviour::DoubleArrow(c) => Some(c),
            Behaviour::Other => None,
        };
        (&mut self.item, proc)
    }

    pub fn button(&self) -> Option<&button::Button> {
        match &self.behaviour {
            Behaviour::Button(b) => Some(b),
            _ => None,
        }
    }

    pub fn button_mut(&mut self) -> Option<(&mut ItemState, &mut button::Button)> {
        match &mut self.behaviour {
            Behaviour::Button(b) => Some((&mut self.item, b)),
            _ => None,
        }
    }

    pub fn static_text(&self) -> Option<&static_text::StaticText> {
        match &self.behaviour {
            Behaviour::StaticText(s) => Some(s),
            _ => None,
        }
    }

    pub fn static_text_mut(&mut self) -> Option<(&mut ItemState, &mut static_text::StaticText)> {
        match &mut self.behaviour {
            Behaviour::StaticText(s) => Some((&mut self.item, s)),
            _ => None,
        }
    }

    pub fn list(&self) -> Option<&list::List> {
        match &self.behaviour {
            Behaviour::List(l) => Some(l),
            _ => None,
        }
    }

    pub fn list_mut(&mut self) -> Option<&mut list::List> {
        match &mut self.behaviour {
            Behaviour::List(l) => Some(l),
            _ => None,
        }
    }

    pub fn edit(&self) -> Option<&edit::Edit> {
        match &self.behaviour {
            Behaviour::Edit(e) => Some(e),
            _ => None,
        }
    }

    pub fn edit_mut(&mut self) -> Option<(&mut ItemState, &mut edit::Edit)> {
        match &mut self.behaviour {
            Behaviour::Edit(e) => Some((&mut self.item, e)),
            _ => None,
        }
    }

    pub fn prologue(&self) -> Option<&prol::Prologue> {
        match &self.behaviour {
            Behaviour::Prologue(p) => Some(p),
            _ => None,
        }
    }

    pub fn prologue_mut(&mut self) -> Option<(&mut ItemState, &mut prol::Prologue)> {
        match &mut self.behaviour {
            Behaviour::Prologue(p) => Some((&mut self.item, p)),
            _ => None,
        }
    }

    pub fn tabs(&self) -> Option<&tab::Tabs> {
        match &self.behaviour {
            Behaviour::Tabs(t) => Some(t),
            _ => None,
        }
    }

    pub fn tabs_mut(&mut self) -> Option<(&mut ItemState, &mut tab::Tabs)> {
        match &mut self.behaviour {
            Behaviour::Tabs(t) => Some((&mut self.item, t)),
            _ => None,
        }
    }

    pub fn report_mut(&mut self) -> Option<(&mut ItemState, &mut repo::Report)> {
        match &mut self.behaviour {
            Behaviour::Report(r) => Some((&mut self.item, r)),
            _ => None,
        }
    }

    pub fn carnage_header_mut(&mut self) -> Option<(&mut ItemState, &mut carnage::CarnageHeader)> {
        match &mut self.behaviour {
            Behaviour::CarnageHeader(c) => Some((&mut self.item, c)),
            _ => None,
        }
    }

    pub fn carnage_graph_mut(&mut self) -> Option<(&mut ItemState, &mut carnage::CarnageGraph)> {
        match &mut self.behaviour {
            Behaviour::CarnageGraph(c) => Some((&mut self.item, c)),
            _ => None,
        }
    }

    pub fn game_mut(&mut self) -> Option<(&mut ItemState, &mut game::GameGraphic)> {
        match &mut self.behaviour {
            Behaviour::Game(g) => Some((&mut self.item, g)),
            _ => None,
        }
    }

    pub fn snapshot_mut(&mut self) -> Option<(&mut ItemState, &mut snapshot::SnapshotItem)> {
        match &mut self.behaviour {
            Behaviour::Snapshot(c) => Some((&mut self.item, c)),
            _ => None,
        }
    }

    pub fn progress_mut(&mut self) -> Option<(&mut ItemState, &mut progress::ProgressBar)> {
        match &mut self.behaviour {
            Behaviour::Progress(c) => Some((&mut self.item, c)),
            _ => None,
        }
    }

    pub fn bitmap_mut(&mut self) -> Option<(&mut ItemState, &mut bitmap::BitmapItem)> {
        match &mut self.behaviour {
            Behaviour::Bitmap(b) => Some((&mut self.item, b)),
            _ => None,
        }
    }

    pub fn color_picker_mut(&mut self) -> Option<(&mut ItemState, &mut color_picker::ColorPicker)> {
        match &mut self.behaviour {
            Behaviour::ColorPicker(c) => Some((&mut self.item, c)),
            _ => None,
        }
    }

    pub fn color_picker(&self) -> Option<&color_picker::ColorPicker> {
        match &self.behaviour {
            Behaviour::ColorPicker(c) => Some(c),
            _ => None,
        }
    }

    pub fn icon_popup_mut(&mut self) -> Option<(&mut ItemState, &mut icon_popup::IconPopup)> {
        match &mut self.behaviour {
            Behaviour::IconPopup(c) => Some((&mut self.item, c)),
            _ => None,
        }
    }

    pub fn icon_popup(&self) -> Option<&icon_popup::IconPopup> {
        match &self.behaviour {
            Behaviour::IconPopup(c) => Some(c),
            _ => None,
        }
    }

    pub fn double_arrow_mut(&mut self) -> Option<(&mut ItemState, &mut numb::DoubleArrow)> {
        match &mut self.behaviour {
            Behaviour::DoubleArrow(c) => Some((&mut self.item, c)),
            _ => None,
        }
    }

    pub fn double_arrow(&self) -> Option<&numb::DoubleArrow> {
        match &self.behaviour {
            Behaviour::DoubleArrow(c) => Some(c),
            _ => None,
        }
    }

    pub(crate) fn can_focus(&self) -> bool {
        self.proc().is_some_and(|proc| proc.can_focus(&self.item))
    }

    pub(crate) fn draw(&self, ctx: &DrawContext<'_>, bitmap: &mut Bitmap32) {
        if let Some(proc) = self.proc() {
            proc.draw(&self.item, ctx, bitmap);
        }
    }

    pub(crate) fn click(&mut self, click: &Click) -> bool {
        match self.proc_mut() {
            (item, Some(proc)) => proc.click(item, click),
            (item, None) => default_click(item, click),
        }
    }

    pub(crate) fn key(&mut self, key: &Key) -> bool {
        match self.proc_mut() {
            (item, Some(proc)) => proc.key(item, key),
            _ => false,
        }
    }

    pub(crate) fn text(&mut self, ch: char) -> bool {
        match self.proc_mut() {
            (item, Some(proc)) => proc.text(item, ch),
            _ => false,
        }
    }

    pub(crate) fn focus(&mut self, focused: bool) -> bool {
        match self.proc_mut() {
            (item, Some(proc)) => proc.focus(item, focused),
            _ => false,
        }
    }

    pub(crate) fn idle(&mut self, now_tick: u64) -> bool {
        match self.proc_mut() {
            (item, Some(proc)) => proc.idle(item, now_tick),
            _ => false,
        }
    }

    pub(crate) fn wheel(&mut self, delta: i32) -> bool {
        match self.proc_mut() {
            (item, Some(proc)) => proc.wheel(item, delta),
            _ => false,
        }
    }

    pub(crate) fn take_focus_request(&mut self) -> bool {
        match self.proc_mut() {
            (_, Some(proc)) => proc.take_focus_request(),
            _ => false,
        }
    }

    pub(crate) fn take_press_request(&mut self) -> Option<i16> {
        match self.proc_mut() {
            (_, Some(proc)) => proc.take_press_request(),
            _ => None,
        }
    }
}
