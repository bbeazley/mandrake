use std::sync::Arc;

use mandrake_common::FourCC;
use mandrake_session::{UiEvent, UiKey};
use mandrake_tag::font::Bitmap32;
use mandrake_tag::{DialogDefinition, DialogItem, Rect};
use mandrake_view::interface_bitmap::{BACKGROUND_ORDER, InterfaceArt, tile_and_bevel_rectangle};

use crate::controls::{Behaviour, Click, Control, DrawContext, Key, item_flags};
use crate::resources::{BAIE, Resources};
use crate::slices::{ThreeSlice, draw_three_slice};

pub mod dialog_flags {
    pub const HIDDEN: u16 = 0x02;
    pub const NO_BACKGROUND: u16 = 0x04;
    pub const CENTERED: u16 = 0x10;
    pub const NO_TAB: u16 = 0x20;
    pub const CENTERED_ACROSS: u16 = 0x100;
    pub const CENTERED_DOWN: u16 = 0x200;
    pub const MODAL: u16 = 0x18;
}

const DEFAULT_BACKGROUND: i32 = 5;
const PRESS_TICKS: i32 = 3;
const DOUBLE_CLICK_TICKS: u64 = 15;
const DOUBLE_CLICK_PIXELS: i32 = 2;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Hit {
    pub dialog: FourCC,
    pub item: i16,
}

#[derive(Debug, Clone, Copy)]
struct Pending {
    item: i16,
    countdown: i32,
    just_pressed: bool,
}

pub struct Dialog {
    pub id: FourCC,
    flags: u16,
    bounds: Rect,
    items: Vec<Control>,
    focus: i16,
    default_item: i16,
    cancel_item: i16,
    tracking: i16,
    pending: Option<Pending>,
    background: Option<(Arc<InterfaceArt>, i32)>,
    slices: Option<ThreeSlice>,
    background_drawn: Option<Vec<u32>>,
    background_draws: u32,
    visible: bool,
    dirty: bool,
    bitmap: Bitmap32,
    newest_item_first: bool,
}

#[derive(Debug, Clone, Copy)]
struct MouseInput {
    shift: bool,
    double: bool,
    now_tick: u64,
}

#[derive(Debug, Clone, Copy, Default)]
struct Pointer {
    x: i32,
    y: i32,
    held: bool,
}

impl Dialog {
    pub fn new(def: &DialogDefinition, id: FourCC, flags: u16, res: &mut Resources) -> Self {
        let (w, h) = (def.width().max(0), def.height().max(0));
        let background = if flags & dialog_flags::NO_BACKGROUND == 0 {
            res.art(BAIE).map(|a| (a, DEFAULT_BACKGROUND))
        } else {
            None
        };
        let items = def.items.iter().map(|i| Control::new(i, res)).collect();
        let mut dialog = Self {
            id,
            flags,
            bounds: def.rect,
            items,
            focus: -1,
            default_item: -1,
            cancel_item: -1,
            tracking: -1,
            pending: None,
            background,
            slices: None,
            background_drawn: None,
            background_draws: 0,
            visible: flags & dialog_flags::HIDDEN == 0,
            dirty: true,
            bitmap: Bitmap32::new(w, h, 0xff00_0000),
            newest_item_first: false,
        };
        dialog.handle_tab(true);
        dialog
    }

    pub fn add_control(
        &mut self,
        kind: FourCC,
        id: i16,
        rect: Rect,
        name: &[u8],
        res: &mut Resources,
    ) {
        self.items.retain(|c| c.item.id != id);
        let item = DialogItem {
            kind,
            field_04: 0,
            id: id as u32,
            fields_08: [0; 4],
            rect,
            name: name.to_vec(),
        };
        self.items.push(Control::new(&item, res));
        self.dirty = true;
    }

    pub fn bounds(&self) -> Rect {
        self.bounds
    }

    pub fn move_to(&mut self, left: i32, top: i32) {
        let (w, h) = (self.bounds.width(), self.bounds.height());
        self.bounds = Rect::new(top, left, top + h, left + w);
    }

    pub fn bitmap(&self) -> &Bitmap32 {
        &self.bitmap
    }

    pub fn control(&self, id: i16) -> Option<&Control> {
        self.items.iter().find(|c| c.item.id == id)
    }

    pub fn control_mut(&mut self, id: i16) -> Option<&mut Control> {
        self.items.iter_mut().find(|c| c.item.id == id)
    }

    pub fn focus(&self) -> i16 {
        self.focus
    }

    pub fn is_tracking(&self) -> bool {
        self.tracking != -1
    }

    pub fn is_press_pending(&self) -> bool {
        self.pending.is_some()
    }

    pub fn set_background(&mut self, art: Option<Arc<InterfaceArt>>, sequence: i32) {
        self.background = art.map(|a| (a, sequence));
        self.slices = None;
        self.background_drawn = None;
        self.dirty = true;
    }

    pub fn set_background_three_slice(
        &mut self,
        art: Option<Arc<InterfaceArt>>,
        sequence: i32,
        slice: ThreeSlice,
    ) {
        self.set_background(art, sequence);
        self.slices = Some(slice);
    }

    pub fn background_draws(&self) -> u32 {
        self.background_draws
    }

    pub fn set_default_item(&mut self, id: i16) {
        if self.default_item == id {
            return;
        }
        if self.default_item != -1
            && let Some(c) = self.control_mut(id)
        {
            c.keyboard = None;
        }
        self.default_item = id;
        self.dirty = true;
    }

    pub fn set_cancel_item(&mut self, id: i16) {
        if self.cancel_item == id {
            return;
        }
        self.cancel_item = id;
        self.dirty = true;
    }

    pub fn set_keyboard_equivalent(&mut self, id: i16, ch: char, alt: bool) {
        if let Some(c) = self.control_mut(id) {
            c.keyboard = Some((ch.to_ascii_uppercase(), alt));
        }
    }

    pub fn set_enabled(&mut self, id: i16, enabled: bool) {
        self.set_item_flag(id, item_flags::DISABLED, !enabled);
    }

    pub fn set_visible(&mut self, id: i16, visible: bool) {
        if self.set_item_flag(id, item_flags::HIDDEN, !visible) {
            self.dirty = true;
        }
    }

    fn set_item_flag(&mut self, id: i16, flag: u16, on: bool) -> bool {
        let tracked = self.tracking == id;
        let pending = self.pending.is_some_and(|p| p.item == id);
        let Some(c) = self.control_mut(id) else {
            return false;
        };
        if (c.item.flags & flag != 0) == on {
            return false;
        }
        if on {
            c.item.flags |= flag;
            if tracked || pending {
                c.item.set_pressed(false);
            }
            if let Some((item, popup)) = c.icon_popup_mut() {
                popup.close(item);
            }
        } else {
            c.item.flags &= !flag;
        }
        c.item.dirty = true;
        if on {
            if tracked {
                self.tracking = -1;
            }
            if pending {
                self.pending = None;
            }
        }
        true
    }

    pub fn edit_text_slice(&self, id: i16) -> &[u8] {
        self.control(id)
            .and_then(Control::edit)
            .map_or(&[], crate::controls::edit::Edit::text)
    }

    pub fn set_edit_text(&mut self, id: i16, text: &[u8]) {
        if let Some((item, e)) = self.control_mut(id).and_then(Control::edit_mut) {
            e.set_text(item, text);
        }
    }

    pub fn set_static_text(&mut self, id: i16, text: &[u8]) {
        if let Some((item, t)) = self.control_mut(id).and_then(Control::static_text_mut) {
            t.set_text(item, text);
        }
    }

    pub fn set_button_caption(&mut self, id: i16, name: &[u8]) {
        if let Some((item, b)) = self.control_mut(id).and_then(Control::button_mut) {
            b.set_caption(item, name);
        }
    }

    pub fn set_button_caption_if_changed(&mut self, id: i16, name: &[u8]) {
        if let Some((item, b)) = self.control_mut(id).and_then(Control::button_mut)
            && b.caption() != name
        {
            b.set_caption(item, name);
        }
    }

    pub fn is_checked(&self, id: i16) -> bool {
        self.control(id)
            .and_then(Control::button)
            .is_some_and(|b| b.state() != 0)
    }

    pub fn set_checked(&mut self, id: i16, on: bool) {
        if let Some((item, b)) = self.control_mut(id).and_then(Control::button_mut) {
            b.set_state(item, i32::from(on));
        }
    }

    pub fn is_enabled(&self, id: i16) -> bool {
        self.control(id).is_some_and(|c| c.item.enabled())
    }

    pub fn set_enabled_if(&mut self, id: i16, enabled: bool) {
        if self
            .control(id)
            .is_some_and(|c| c.item.enabled() != enabled)
        {
            self.set_enabled(id, enabled);
        }
    }

    pub fn set_visible_if(&mut self, id: i16, visible: bool) {
        if self
            .control(id)
            .is_some_and(|c| c.item.visible() != visible)
        {
            self.set_visible(id, visible);
        }
    }

    pub fn list_mut(&mut self, id: i16) -> Option<&mut crate::controls::list::List> {
        self.control_mut(id)?.list_mut()
    }

    pub fn selected_list_value(&self, id: i16) -> Option<i32> {
        self.control(id)
            .and_then(Control::list)
            .and_then(crate::controls::list::List::selected_value)
    }

    pub fn arrow_value(&self, id: i16) -> i32 {
        self.control(id)
            .and_then(Control::double_arrow)
            .map_or(0, crate::controls::numb::DoubleArrow::value)
    }

    pub fn set_arrows(&mut self, id: i16, min: i32, max: i32, value: i32) {
        if let Some((item, a)) = self.control_mut(id).and_then(Control::double_arrow_mut) {
            a.set_value(item, min, max, value);
        }
    }

    pub fn set_list_unfocusable(&mut self, id: i16) {
        if let Some(list) = self.control_mut(id).and_then(Control::list_mut) {
            list.set_flag(crate::controls::list::list_flags::FOCUSABLE, false);
        }
        if self.focus == id {
            self.set_focus(-1);
        }
    }

    pub fn set_focus(&mut self, id: i16) {
        if self.focus == id {
            return;
        }
        let old = self.focus;
        if let Some(c) = self.control_mut(old) {
            c.focus(false);
        }
        self.focus = id;
        if let Some(c) = self.control_mut(id) {
            c.focus(true);
        }
        self.dirty = true;
    }

    pub fn handle_tab(&mut self, forward: bool) -> bool {
        let count = self.items.len() as i32;
        if count == 0 {
            return false;
        }
        let focus = self.focus;
        let at = |i: i32| (count - 1 - i) as usize;
        let current = (0..count).find(|&i| self.items[at(i)].item.id == focus);
        let (mut i, step) = match current {
            Some(i) => (i, if forward { -1 } else { 1 }),
            None => (-1, 1),
        };
        for _ in 0..count {
            i = (i + step + count) % count;
            let c = &self.items[at(i)];
            if c.can_focus() && c.item.id != -1 {
                let id = c.item.id;
                self.set_focus(id);
                return true;
            }
        }
        false
    }

    fn hit_test(&self, x: i32, y: i32) -> Option<i16> {
        let under = |c: &&Control| c.item.live() && c.item.rect.contains(x, y);
        let found = if self.newest_item_first {
            self.items.iter().rev().find(under)
        } else {
            self.items.iter().find(under)
        };
        found.map(|c| c.item.id)
    }

    fn press(&mut self, id: i16, click: &Click) {
        let Some(c) = self.control_mut(id) else {
            return;
        };
        let whole = c.click(click);
        let wants_focus = c.take_focus_request();
        let presses = c.take_press_request();
        if wants_focus {
            self.set_focus(id);
        }
        if let Some(other) = presses {
            self.press_item(other, false);
        }
        if !whole {
            return;
        }
        if let Some(p) = self.pending
            && let Some(old) = self.control_mut(p.item)
        {
            old.item.set_pressed(false);
        }
        let Some(c) = self.control_mut(id) else {
            return;
        };
        if c.item.live() {
            let single = c.item.flags & item_flags::SINGLE_STATE != 0;
            c.item.set_pressed(single);
            c.item.dirty = true;
            self.pending = Some(Pending {
                item: id,
                countdown: if single { PRESS_TICKS } else { 0 },
                just_pressed: true,
            });
        }
    }

    pub fn press_item(&mut self, id: i16, shift: bool) {
        let Some(c) = self.control(id) else { return };
        let r = c.item.rect;
        let click = Click {
            point: (r.left + r.width() / 2, r.top + r.height() / 2),
            held: false,
            shift,
            double: false,
            now_tick: 0,
        };
        self.press(id, &click);
    }

    fn run_press(&mut self, ticks: i32, hits: &mut Vec<Hit>) {
        let Some(mut p) = self.pending else { return };
        let dialog = self.id;
        let Some(c) = self.control_mut(p.item) else {
            self.pending = None;
            return;
        };
        if !c.item.live() {
            c.item.set_pressed(false);
            self.pending = None;
            return;
        }
        let flags = c.item.flags;
        let report_now = p.just_pressed && flags & item_flags::REPORT_AT_PRESS != 0;
        p.just_pressed = false;
        let mut report = report_now;
        let mut ended = false;
        if flags & (item_flags::REPORT_AT_PRESS | item_flags::HOLD_PRESS)
            == item_flags::REPORT_AT_PRESS
        {
            c.item.set_pressed(false);
            ended = true;
        } else {
            p.countdown -= ticks;
            if p.countdown >= 2 {
                c.item.set_pressed(true);
            } else if p.countdown == 1 {
                c.item.set_pressed(false);
            } else {
                ended = true;
                if flags & item_flags::REPORT_AT_PRESS == 0 {
                    c.item.set_pressed(false);
                    report = true;
                }
            }
        }
        self.pending = if ended { None } else { Some(p) };
        if report {
            hits.push(Hit {
                dialog,
                item: p.item,
            });
        }
    }

    fn press_by_key(&mut self, key: &Key) -> bool {
        let is_return = key.key == UiKey::Return;
        let is_escape = key.key == UiKey::Escape;
        let ch = key_char(key.key).map(|c| c.to_ascii_uppercase());
        let cancel_chord = ch == Some('.') && key.alt;
        for i in (0..self.items.len()).rev() {
            let c = &self.items[i];
            if !c.item.live() {
                continue;
            }
            let id = c.item.id;
            let by_equivalent = c
                .keyboard
                .is_some_and(|(k, alt)| ch == Some(k) && (!alt || key.alt));
            let by_default = id == self.default_item && is_return;
            let by_cancel = id == self.cancel_item && (cancel_chord || is_escape);
            if by_default || by_cancel || by_equivalent {
                self.press_item(id, key.shift);
                return true;
            }
        }
        false
    }

    fn key_down(&mut self, key: &Key, hits: &mut Vec<Hit>) {
        if !key.repeat && self.press_by_key(key) {
            return;
        }
        if key.key == UiKey::Tab && self.flags & dialog_flags::NO_TAB == 0 {
            self.handle_tab(!key.shift);
            return;
        }
        let (focus, dialog) = (self.focus, self.id);
        if let Some(c) = self.control_mut(focus)
            && c.item.live()
            && c.key(key)
        {
            c.item.dirty = true;
            hits.push(Hit {
                dialog,
                item: focus,
            });
        }
    }

    fn text(&mut self, ch: char, hits: &mut Vec<Hit>) {
        let (focus, dialog) = (self.focus, self.id);
        if let Some(c) = self.control_mut(focus)
            && c.item.live()
            && c.text(ch)
        {
            c.item.dirty = true;
            hits.push(Hit {
                dialog,
                item: focus,
            });
        }
    }

    fn wheel(&mut self, delta: i32, x: i32, y: i32) {
        if !self.bounds_local().contains(x, y) {
            return;
        }
        let taken = self
            .hit_test(x, y)
            .and_then(|under| self.control_mut(under))
            .is_some_and(|c| c.wheel(delta));
        if !taken {
            let focus = self.focus;
            if let Some(c) = self.control_mut(focus)
                && c.item.live()
            {
                c.wheel(delta);
            }
        }
    }

    fn mouse(&mut self, x: i32, y: i32, press: bool, held: bool, input: MouseInput) {
        if press {
            self.tracking = if self.bounds_local().contains(x, y) {
                self.hit_test(x, y).unwrap_or(-1)
            } else {
                -1
            };
        }
        if self.tracking != -1 {
            let id = self.tracking;
            let click = Click {
                point: (x, y),
                held,
                shift: input.shift,
                double: press && input.double,
                now_tick: input.now_tick,
            };
            self.press(id, &click);
            if !held {
                self.tracking = -1;
            }
        }
    }

    const fn bounds_local(&self) -> Rect {
        Rect::new(0, 0, self.bounds.height(), self.bounds.width())
    }

    fn tick(&mut self, ticks: i32, now_tick: u64, hits: &mut Vec<Hit>) {
        self.run_press(ticks, hits);
        for c in &mut self.items {
            if c.item.flags & item_flags::WANTS_IDLE != 0 && c.idle(now_tick) {
                c.item.dirty = true;
            }
        }
    }

    pub fn is_dirty(&self) -> bool {
        self.dirty || self.items.iter().any(|c| c.item.dirty)
    }

    pub fn redraw(&mut self, res: &Resources, now_tick: u64) {
        let _ = self.redraw_static(res, now_tick);
    }

    pub(crate) fn redraw_static(&mut self, res: &Resources, now_tick: u64) -> bool {
        let static_dirty = self.dirty
            || self
                .items
                .iter()
                .any(|c| c.item.dirty && c.prologue().is_none());
        if static_dirty {
            let (w, h) = (self.bitmap.width, self.bitmap.height);
            if let Some(drawn) = &self.background_drawn {
                self.bitmap.pixels.copy_from_slice(drawn);
            } else {
                self.bitmap.pixels.fill(0xff00_0000);
                if let (Some((art, sequence)), Some(slice)) = (&self.background, &self.slices) {
                    draw_three_slice(&mut self.bitmap, art, *sequence, slice);
                } else if let Some((art, sequence)) = &self.background {
                    tile_and_bevel_rectangle(
                        &mut self.bitmap,
                        art,
                        *sequence,
                        -1,
                        Rect::new(0, 0, h, w),
                        None,
                        Some(&BACKGROUND_ORDER),
                    );
                }
                self.background_drawn = Some(self.bitmap.pixels.clone());
                self.background_draws += 1;
            }
            let focus = self.focus;
            let ctx = |focused| DrawContext {
                res,
                now_tick,
                focused,
            };
            for c in self.items.iter().rev() {
                if c.item.visible() && c.item.id != focus && c.prologue().is_none() {
                    c.draw(&ctx(false), &mut self.bitmap);
                }
            }
            if let Some(c) = self
                .items
                .iter()
                .find(|c| c.item.id == focus && c.item.visible() && c.prologue().is_none())
            {
                c.draw(&ctx(true), &mut self.bitmap);
            }
        }
        for c in &mut self.items {
            c.item.dirty = false;
        }
        self.dirty = false;
        static_dirty
    }

    pub fn draw_prologues(&self, res: &Resources, now_tick: u64, canvas: &mut Bitmap32) {
        let focus = self.focus;
        for c in self.items.iter().rev() {
            if c.item.visible()
                && let Behaviour::Prologue(prol) = &c.behaviour
            {
                let mut item = c.item.clone();
                item.rect = Rect::new(
                    item.rect.top + self.bounds.top,
                    item.rect.left + self.bounds.left,
                    item.rect.bottom + self.bounds.top,
                    item.rect.right + self.bounds.left,
                );
                let ctx = DrawContext {
                    res,
                    now_tick,
                    focused: c.item.id == focus,
                };
                use crate::controls::ControlProc;
                prol.draw(&item, &ctx, canvas);
            }
        }
    }

    pub fn gpu_prologue(&self) -> Option<(Rect, crate::controls::prol::PrologueGpuLayer<'_>)> {
        for c in self.items.iter().rev() {
            if c.item.visible()
                && let Some(prol) = c.prologue()
                && let Some(layer) = prol.gpu_layer()
            {
                let rect = Rect::new(
                    c.item.rect.top + self.bounds.top,
                    c.item.rect.left + self.bounds.left,
                    c.item.rect.bottom + self.bounds.top,
                    c.item.rect.right + self.bounds.left,
                );
                return Some((rect, layer));
            }
        }
        None
    }

    fn place(&mut self, size: (i32, i32)) {
        let (w, h) = (self.bounds.width(), self.bounds.height());
        let centre = |screen: i32, own: i32| (screen / 2 - own / 2).max(0);
        let (across, down) = match self.flags & 0x310 {
            0 => return,
            0x100 => (true, false),
            0x200 => (false, true),
            _ => (true, true),
        };
        let (mut left, mut top) = (self.bounds.left, self.bounds.top);
        if across {
            left = centre(size.0, w);
        }
        if down {
            top = centre(size.1, h);
        }
        self.bounds = Rect::new(top, left, top + h, left + w);
    }
}

const fn key_char(key: UiKey) -> Option<char> {
    match key {
        UiKey::Char(c) => Some(c),
        UiKey::Space => Some(' '),
        UiKey::Return => Some('\r'),
        UiKey::Tab => Some('\t'),
        UiKey::Escape => Some('\u{1b}'),
        _ => None,
    }
}

#[derive(Default)]
pub struct DialogStack {
    dialogs: Vec<Dialog>,
    pointer: Pointer,
    screen: (i32, i32),
    shift_held: bool,
    now_tick: u64,
    last_press: Option<(u64, i32, i32)>,
    double_clicked: bool,
    newest_item_first: bool,
}

impl DialogStack {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn tfl() -> Self {
        Self {
            newest_item_first: true,
            ..Self::default()
        }
    }

    pub fn push(&mut self, mut dialog: Dialog) {
        dialog.newest_item_first = self.newest_item_first;
        dialog.place(self.screen);
        self.dialogs.push(dialog);
    }

    pub fn pop(&mut self) -> Option<Dialog> {
        self.dialogs.pop()
    }

    pub fn clear(&mut self) {
        self.dialogs.clear();
        self.double_clicked = false;
    }

    pub const fn take_double_click(&mut self) -> bool {
        let double = self.double_clicked;
        self.double_clicked = false;
        double
    }

    pub fn is_empty(&self) -> bool {
        self.dialogs.is_empty()
    }

    pub fn top(&self) -> Option<&Dialog> {
        self.dialogs.last()
    }

    pub fn top_mut(&mut self) -> Option<&mut Dialog> {
        self.dialogs.last_mut()
    }

    pub fn find_mut(&mut self, id: FourCC) -> Option<&mut Dialog> {
        self.dialogs.iter_mut().rev().find(|d| d.id == id)
    }

    pub fn iter(&self) -> impl Iterator<Item = &Dialog> {
        self.dialogs.iter().filter(|d| d.visible)
    }

    pub fn place(&mut self, size: (i32, i32)) {
        self.screen = size;
        for d in &mut self.dialogs {
            d.place(size);
        }
    }

    pub fn set_shift_held(&mut self, shift: bool) {
        self.shift_held = shift;
    }

    pub fn event(&mut self, event: &UiEvent, hits: &mut Vec<Hit>) {
        let pointer = &mut self.pointer;
        let mut press = false;
        let mut mouse = false;
        let mut double = false;
        match *event {
            UiEvent::PointerMoved { x, y } => {
                pointer.x = x;
                pointer.y = y;
                mouse = true;
            }
            UiEvent::Button {
                left: true,
                pressed,
            } => {
                press = pressed;
                pointer.held = pressed;
                mouse = true;
                if pressed {
                    let (now, x, y) = (self.now_tick, pointer.x, pointer.y);
                    double = self.last_press.is_some_and(|(t, px, py)| {
                        now <= t + DOUBLE_CLICK_TICKS
                            && (x - px).abs() <= DOUBLE_CLICK_PIXELS
                            && (y - py).abs() <= DOUBLE_CLICK_PIXELS
                    });
                    self.last_press = (!double).then_some((now, x, y));
                    self.double_clicked |= double;
                }
            }
            _ => {}
        }
        let pointer = *pointer;
        let Some(d) = self.dialogs.last_mut().filter(|d| d.visible) else {
            return;
        };
        if d.pending.is_some() {
            return;
        }
        let (x, y) = (pointer.x - d.bounds.left, pointer.y - d.bounds.top);
        match *event {
            UiEvent::Key {
                key,
                pressed: true,
                repeat,
                shift,
                alt,
            } => d.key_down(
                &Key {
                    key,
                    repeat,
                    shift,
                    alt,
                    now_tick: self.now_tick,
                },
                hits,
            ),
            UiEvent::Text(ch) => d.text(ch, hits),
            UiEvent::Wheel(delta) => d.wheel(delta, x, y),
            _ if mouse => {
                let input = MouseInput {
                    shift: self.shift_held,
                    double,
                    now_tick: self.now_tick,
                };
                d.mouse(x, y, press, pointer.held, input);
            }
            _ => {}
        }
    }

    pub fn tick(&mut self, ticks: i32, now_tick: u64, hits: &mut Vec<Hit>) {
        self.now_tick = now_tick;
        if ticks <= 0 {
            return;
        }
        for d in self.dialogs.iter_mut().rev() {
            d.tick(ticks, now_tick, hits);
        }
        let (pointer, shift) = (self.pointer, self.shift_held);
        if let Some(d) = self.dialogs.last_mut().filter(|d| d.visible)
            && d.pending.is_none()
            && d.tracking != -1
        {
            let (x, y) = (pointer.x - d.bounds.left, pointer.y - d.bounds.top);
            let input = MouseInput {
                shift,
                double: false,
                now_tick,
            };
            d.mouse(x, y, false, pointer.held, input);
        }
    }

    pub fn redraw_with_static(&mut self, res: &Resources, now_tick: u64) -> (bool, bool) {
        let mut any = false;
        let mut static_any = false;
        for d in &mut self.dialogs {
            if d.is_dirty() {
                static_any |= d.redraw_static(res, now_tick);
                any = true;
            }
        }
        (any, static_any)
    }

    pub fn redraw(&mut self, res: &Resources, now_tick: u64) -> bool {
        self.redraw_with_static(res, now_tick).0
    }
}

pub(crate) fn blit(canvas: &mut Bitmap32, at: Rect, src: &Bitmap32) {
    if src.width <= 0 || src.height <= 0 || canvas.width <= 0 || canvas.height <= 0 {
        return;
    }
    let x0 = at.left.max(0);
    let x1 = (at.left + src.width).min(canvas.width);
    if x1 <= x0 {
        return;
    }
    let w = (x1 - x0) as usize;
    let src_x = x0 - at.left;
    let y0 = (-at.top).max(0);
    let y1 = src.height.min(canvas.height - at.top);
    for y in y0..y1 {
        let cy = y + at.top;
        let from = (y * src.width + src_x) as usize;
        let to = (cy * canvas.width + x0) as usize;
        canvas.pixels[to..to + w].copy_from_slice(&src.pixels[from..from + w]);
    }
}

pub(crate) fn format_one(format: &[u8], arg: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(format.len() + arg.len());
    let mut i = 0;
    let mut used = false;
    while i < format.len() {
        if format[i] == b'%' && i + 1 < format.len() {
            match format[i + 1] {
                b'%' => {
                    out.push(b'%');
                    i += 2;
                    continue;
                }
                b's' | b'd' if !used => {
                    out.extend_from_slice(arg);
                    used = true;
                    i += 2;
                    continue;
                }
                _ => {}
            }
        }
        out.push(format[i]);
        i += 1;
    }
    out
}

#[cfg(test)]
mod hit_tests {
    use super::*;
    use mandrake_tag::TagLookup;

    fn item(kind: &[u8; 4], id: u32, rect: Rect) -> DialogItem {
        DialogItem {
            kind: FourCC(*kind),
            field_04: 0,
            id,
            fields_08: [0; 4],
            rect,
            name: Vec::new(),
        }
    }

    #[test]
    fn in_tfl_a_later_item_takes_a_click_over_an_earlier_one_it_overlaps() {
        let mut res = Resources::new(TagLookup::empty());
        let def = DialogDefinition {
            version: 1,
            rect: Rect::new(0, 0, 480, 640),
            items: vec![
                item(b"bitm", 1, Rect::new(0, 0, 480, 640)),
                item(b"butt", 3, Rect::new(280, 160, 300, 380)),
            ],
        };
        let click = |mut stack: DialogStack, res: &mut Resources| {
            stack.push(Dialog::new(&def, FourCC(*b"main"), 0, res));
            let mut hits = Vec::new();
            for event in [
                UiEvent::PointerMoved { x: 256, y: 288 },
                UiEvent::Button {
                    left: true,
                    pressed: true,
                },
                UiEvent::Button {
                    left: true,
                    pressed: false,
                },
            ] {
                stack.event(&event, &mut hits);
            }
            for t in 1..=10 {
                stack.tick(1, t, &mut hits);
            }
            hits.iter().map(|h| h.item).collect::<Vec<i16>>()
        };
        assert_eq!(
            click(DialogStack::tfl(), &mut res),
            [3],
            "TFL: the button, not the background under it"
        );
        assert_eq!(click(DialogStack::new(), &mut res), [1], "Myth II");
    }

    #[test]
    fn blit_clips_negative_and_overflowing_rects() {
        let mut dst = Bitmap32::new(4, 4, 0);
        let src = Bitmap32::new(4, 4, 0xffff_0000);
        blit(&mut dst, Rect::new(-2, -2, 2, 2), &src);
        assert_eq!(dst.get(0, 0), 0xffff_0000);
        assert_eq!(dst.get(1, 1), 0xffff_0000);
        assert_eq!(dst.get(2, 2), 0);
        let mut dst2 = Bitmap32::new(4, 4, 0);
        blit(&mut dst2, Rect::new(3, 3, 7, 7), &src);
        assert_eq!(dst2.get(3, 3), 0xffff_0000);
        assert_eq!(dst2.get(2, 2), 0);
        blit(&mut dst2, Rect::new(10, 10, 14, 14), &src);
        blit(&mut dst2, Rect::new(-10, -10, -6, -6), &src);
    }

    #[test]
    fn disabling_item_clears_pending_press_and_blocks_focused_input() {
        let mut res = Resources::new(TagLookup::empty());
        let def = DialogDefinition {
            version: 1,
            rect: Rect::new(0, 0, 200, 200),
            items: vec![
                item(b"butt", 1, Rect::new(10, 10, 30, 80)),
                item(b"edit", 2, Rect::new(40, 10, 60, 120)),
            ],
        };
        let mut dialog = Dialog::new(&def, FourCC(*b"test"), 0, &mut res);
        dialog.control_mut(1).unwrap().item.flags |= item_flags::SINGLE_STATE;
        dialog.press_item(1, false);
        assert!(dialog.is_press_pending());
        dialog.set_enabled(1, false);
        assert!(!dialog.is_press_pending());
        let mut hits = Vec::new();
        for t in 1..=5 {
            dialog.tick(1, t, &mut hits);
        }
        assert!(hits.is_empty());

        dialog.set_focus(2);
        dialog.set_enabled(2, false);
        dialog.text('a', &mut hits);
        dialog.key_down(
            &Key {
                key: UiKey::Backspace,
                repeat: false,
                shift: false,
                alt: false,
                now_tick: 1,
            },
            &mut hits,
        );
        assert!(dialog.edit_text_slice(2).is_empty());
        assert!(hits.is_empty());
    }

    #[test]
    fn unchanged_default_cancel_and_visibility_do_not_dirty_dialog() {
        let mut res = Resources::new(TagLookup::empty());
        let def = DialogDefinition {
            version: 1,
            rect: Rect::new(0, 0, 100, 100),
            items: vec![
                item(b"butt", 1, Rect::new(10, 10, 30, 80)),
                item(b"butt", 2, Rect::new(40, 10, 60, 80)),
            ],
        };
        let mut dialog = Dialog::new(&def, FourCC(*b"test"), 0, &mut res);
        dialog.set_default_item(1);
        dialog.set_cancel_item(2);
        dialog.redraw(&res, 0);
        assert!(!dialog.is_dirty());
        dialog.set_default_item(1);
        dialog.set_cancel_item(2);
        dialog.set_visible(1, true);
        assert!(!dialog.is_dirty());
    }

    #[test]
    fn wheel_over_dialog_background_falls_back_to_focus() {
        let mut res = Resources::new(TagLookup::empty());
        let def = DialogDefinition {
            version: 1,
            rect: Rect::new(0, 0, 200, 200),
            items: vec![item(b"list", 1, Rect::new(10, 10, 50, 100))],
        };
        let mut dialog = Dialog::new(&def, FourCC(*b"test"), 0, &mut res);
        if let Some(list) = dialog.list_mut(1) {
            for i in 0..20 {
                list.add_row(b"row", i, true);
            }
        }
        dialog.set_focus(1);
        dialog.redraw(&res, 0);
        assert!(!dialog.is_dirty());
        dialog.wheel(-1, 150, 150);
        assert!(dialog.is_dirty());
    }
}
