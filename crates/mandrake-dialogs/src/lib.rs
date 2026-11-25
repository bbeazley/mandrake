pub mod canvas;
pub mod controls;
pub mod core;
pub mod fade;
pub mod resources;
pub mod screens;
pub mod slices;
#[cfg(test)]
#[path = "../../mandrake-common/tests/support/require_data.rs"]
pub(crate) mod test_data;
pub mod tfl;

pub use mandrake_view::interface_picture::{
    InterfaceFilter, InterfaceLayer as Layer, InterfacePicture, MenuCanvas,
};

use std::cell::OnceCell;

use mandrake_session::{FrameInput, FrameOutput, FrontEnd, Screen, UiEvent};
use mandrake_tag::font::Bitmap32;
use mandrake_tag::{Rect, TagLookup};

use crate::canvas::CanvasFit;
use crate::core::DialogStack;
use crate::resources::Resources;
use crate::screens::{OpenScreen, ScreenFrame, ScreenKind};

const TICKS_PER_SECOND: u64 = 30;
const CANVAS_BLACK: u32 = 0xff00_0000;

pub struct EngineDialogs {
    res: Resources,
    stack: DialogStack,
    kind: Option<ScreenKind>,
    open: Option<Box<dyn OpenScreen>>,
    canvas_size: (u32, u32),
    canvas: OnceCell<Bitmap32>,
    static_canvas: Bitmap32,
    window: (u32, u32),
    tick: Option<u64>,
    revision: u64,
    static_revision: u64,
    static_composed: Option<u64>,
    prologue_scroll: Option<(Rect, f32, i32, bool)>,
    hints_scale: f64,
    menu_filter: InterfaceFilter,
    last_full_art: Option<(Bitmap32, (u32, u32))>,
}

impl EngineDialogs {
    pub fn new(tags: TagLookup) -> Self {
        Self::with_resources(Resources::new(tags))
    }

    pub fn with_resources(res: Resources) -> Self {
        let canvas_size = res
            .dialog(screens::main_menu::IMM6)
            .map_or((640, 480), |d| (d.width() as u32, d.height() as u32));
        Self {
            res,
            stack: DialogStack::new(),
            kind: None,
            open: None,
            canvas_size,
            canvas: OnceCell::new(),
            static_canvas: Bitmap32::new(canvas_size.0 as i32, canvas_size.1 as i32, CANVAS_BLACK),
            window: canvas_size,
            tick: None,
            revision: 0,
            static_revision: 0,
            static_composed: None,
            prologue_scroll: None,
            hints_scale: 1.5,
            menu_filter: InterfaceFilter::default(),
            last_full_art: None,
        }
    }

    #[must_use]
    pub fn with_hints_scale(mut self, scale: f64) -> Self {
        self.hints_scale = scale.max(1.0);
        self
    }

    #[must_use]
    pub fn with_menu_filter(mut self, filter: InterfaceFilter) -> Self {
        self.menu_filter = filter;
        self
    }

    #[must_use]
    pub fn with_random_seed(mut self, seed: u32) -> Self {
        self.res.set_random_seed(seed);
        self
    }

    #[must_use]
    pub fn with_campaign_art(mut self, source: resources::CampaignArtSource) -> Self {
        self.res.set_campaign_art(source);
        self
    }

    #[must_use]
    pub fn with_wall_clock(mut self, clock: resources::WallClockSource) -> Self {
        self.res.set_wall_clock(clock);
        self
    }

    pub fn stack(&self) -> &DialogStack {
        &self.stack
    }

    pub fn canvas(&self) -> &Bitmap32 {
        self.canvas.get_or_init(|| self.render_canvas())
    }

    pub fn canvas_size(&self) -> (u32, u32) {
        self.canvas_size
    }

    pub fn revision(&self) -> u64 {
        self.revision
    }

    pub fn menu_filter(&self) -> InterfaceFilter {
        self.menu_filter
    }

    fn full_art_dialog(&self) -> Option<&core::Dialog> {
        let top = self.stack.top()?;
        if matches!(
            top.id,
            screens::briefing::PRDI | screens::postgame::EPDI | screens::campaign_end::INGE
        ) && ((top.bounds().width() as u32) < self.canvas_size.0
            || (top.bounds().height() as u32) < self.canvas_size.1)
        {
            Some(top)
        } else {
            None
        }
    }

    pub fn picture(&mut self, size: (u32, u32)) -> InterfacePicture<'_> {
        self.window = size;
        if self.kind.is_some_and(|kind| !kind.over_the_game()) {
            self.compose_static();
            let prologue = self.stack.iter().find_map(|d| d.gpu_prologue());
            if let Some(d) = self.full_art_dialog()
                && size != self.canvas_size
            {
                let bounds = d.bounds();
                let dialog_size = (bounds.width() as u32, bounds.height() as u32);
                let scroll = self.prologue_scroll.map(|(rect, s, str, in_view)| {
                    (
                        Rect::new(
                            rect.top - bounds.top,
                            rect.left - bounds.left,
                            rect.bottom - bounds.top,
                            rect.right - bounds.left,
                        ),
                        s,
                        str,
                        in_view,
                    )
                });
                InterfacePicture::Menu(MenuCanvas {
                    canvas: d.bitmap(),
                    revision: self.static_revision,
                    size: dialog_size,
                    prologue: prologue.map(|(_, p)| p),
                    scroll,
                })
            } else if self.kind == Some(ScreenKind::Loading)
                && size != self.canvas_size
                && let Some((bitmap, dialog_size)) = &self.last_full_art
            {
                InterfacePicture::Menu(MenuCanvas {
                    canvas: bitmap,
                    revision: self.static_revision,
                    size: *dialog_size,
                    prologue: None,
                    scroll: None,
                })
            } else {
                InterfacePicture::Menu(MenuCanvas {
                    canvas: &self.static_canvas,
                    revision: self.static_revision,
                    size: self.canvas_size,
                    prologue: prologue.map(|(_, p)| p),
                    scroll: self.prologue_scroll,
                })
            }
        } else {
            self.stack.place((size.0 as i32, size.1 as i32));
            InterfacePicture::Layers {
                layers: self.layers(),
                revision: self.revision,
            }
        }
    }

    fn overlay_dst(&self, d: &core::Dialog) -> ([i32; 4], f64) {
        let b = d.bounds();
        let (w, h) = (b.width().max(1), b.height().max(1));
        let wanted = if self.kind == Some(ScreenKind::Hints) {
            self.hints_scale
        } else {
            1.0
        };
        let factor = (self.window.0.max(1) as f64 / w as f64)
            .min(self.window.1.max(1) as f64 / h as f64)
            .min(wanted)
            .max(1.0);
        if (factor - 1.0).abs() < 1e-6 {
            return ([b.left, b.top, b.width(), b.height()], 1.0);
        }
        let dw = (w as f64 * factor).round() as i32;
        let dh = (h as f64 * factor).round() as i32;
        let left = (self.window.0 as i32 / 2 - dw / 2).max(0);
        let top = (self.window.1 as i32 / 2 - dh / 2).max(0);
        ([left, top, dw, dh], factor)
    }

    pub fn layers(&self) -> Vec<Layer<'_>> {
        match self.kind {
            Some(kind) if kind.over_the_game() => self
                .stack
                .iter()
                .map(|d| {
                    let (dst, _) = self.overlay_dst(d);
                    Layer {
                        bitmap: d.bitmap(),
                        dst,
                    }
                })
                .collect(),
            Some(_) => {
                if let Some(d) = self.full_art_dialog()
                    && self.window != self.canvas_size
                {
                    let b = d.bounds();
                    let fit = CanvasFit::new(self.window, (b.width() as u32, b.height() as u32));
                    vec![Layer {
                        bitmap: d.bitmap(),
                        dst: [fit.origin.0, fit.origin.1, fit.size.0, fit.size.1],
                    }]
                } else if self.kind == Some(ScreenKind::Loading)
                    && self.window != self.canvas_size
                    && let Some((bitmap, size)) = &self.last_full_art
                {
                    let fit = CanvasFit::new(self.window, *size);
                    vec![Layer {
                        bitmap,
                        dst: [fit.origin.0, fit.origin.1, fit.size.0, fit.size.1],
                    }]
                } else {
                    let fit = CanvasFit::new(self.window, self.canvas_size);
                    vec![Layer {
                        bitmap: self.canvas(),
                        dst: [fit.origin.0, fit.origin.1, fit.size.0, fit.size.1],
                    }]
                }
            }
            None => Vec::new(),
        }
    }

    fn compose_static(&mut self) {
        if self.static_composed == Some(self.static_revision) {
            return;
        }
        self.static_canvas.pixels.fill(CANVAS_BLACK);
        for d in self.stack.iter() {
            core::blit(&mut self.static_canvas, d.bounds(), d.bitmap());
        }
        self.static_composed = Some(self.static_revision);
    }

    fn render_canvas(&self) -> Bitmap32 {
        let mut bitmap = if self.static_composed == Some(self.static_revision) {
            self.static_canvas.clone()
        } else {
            let mut b = Bitmap32::new(
                self.canvas_size.0 as i32,
                self.canvas_size.1 as i32,
                CANVAS_BLACK,
            );
            for d in self.stack.iter() {
                core::blit(&mut b, d.bounds(), d.bitmap());
            }
            b
        };
        let now_tick = self.tick.unwrap_or(0);
        for d in self.stack.iter() {
            d.draw_prologues(&self.res, now_tick, &mut bitmap);
        }
        bitmap
    }
}

impl FrontEnd for EngineDialogs {
    fn frame(&mut self, screen: &Screen<'_>, input: &FrameInput<'_>) -> FrameOutput {
        let mut out = FrameOutput::default();
        self.window = input.size;
        let kind = ScreenKind::of(screen);
        let over_the_game = kind.over_the_game();
        let screen_size = if over_the_game {
            input.size
        } else {
            self.canvas_size
        };
        self.stack
            .place((screen_size.0 as i32, screen_size.1 as i32));
        if self.kind != Some(kind) {
            if kind == ScreenKind::Loading {
                if let Some(d) = self.full_art_dialog() {
                    let bounds = d.bounds();
                    let dialog_size = (bounds.width() as u32, bounds.height() as u32);
                    self.last_full_art = Some((d.bitmap().clone(), dialog_size));
                }
                self.compose_static();
            }
            self.stack.clear();
            self.open = screens::open(screen, &mut self.res, &mut self.stack);
            self.kind = Some(kind);
            if kind != ScreenKind::Loading {
                self.revision += 1;
                self.static_revision = self.revision;
                self.last_full_art = None;
                self.canvas.take();
            }
            self.stack
                .place((screen_size.0 as i32, screen_size.1 as i32));
        }
        let is_full_art = self.full_art_dialog().is_some() && input.size != self.canvas_size;
        let fit = if is_full_art {
            let b = self.full_art_dialog().unwrap().bounds();
            CanvasFit::new(input.size, (b.width() as u32, b.height() as u32))
        } else {
            CanvasFit::new(input.size, self.canvas_size)
        };
        let mut hits = Vec::new();
        self.stack.set_shift_held(input.shift_held);
        for event in input.events {
            let event = match *event {
                UiEvent::PointerMoved { x, y } if !over_the_game => {
                    let (x, y) = if is_full_art {
                        let (dx, dy) = fit.to_canvas(x, y);
                        let b = self.full_art_dialog().unwrap().bounds();
                        (b.left + dx, b.top + dy)
                    } else {
                        fit.to_canvas(x, y)
                    };
                    UiEvent::PointerMoved { x, y }
                }
                UiEvent::PointerMoved { x, y } => {
                    if let Some(top) = self.stack.top() {
                        let (dst, factor) = self.overlay_dst(top);
                        if (factor - 1.0).abs() >= 1e-6 {
                            let b = top.bounds();
                            let lx = (f64::from(x - dst[0]) / factor).floor() as i32;
                            let ly = (f64::from(y - dst[1]) / factor).floor() as i32;
                            UiEvent::PointerMoved {
                                x: b.left + lx,
                                y: b.top + ly,
                            }
                        } else {
                            UiEvent::PointerMoved { x, y }
                        }
                    } else {
                        UiEvent::PointerMoved { x, y }
                    }
                }
                e => e,
            };
            self.stack.event(&event, &mut hits);
        }
        let now_tick = input.now_ms * TICKS_PER_SECOND / 1000;
        let ticks = now_tick.saturating_sub(self.tick.unwrap_or(now_tick)) as i32;
        self.tick = Some(now_tick);
        self.stack.tick(ticks, now_tick, &mut hits);
        if let Some(open) = &mut self.open {
            open.frame(
                screen,
                &mut ScreenFrame {
                    stack: &mut self.stack,
                    res: &mut self.res,
                    input,
                    now_tick,
                    ticks,
                    hits: &hits,
                    out: &mut out,
                },
            );
        }
        let (any_dirty, static_dirty) = self.stack.redraw_with_static(&self.res, now_tick);
        if any_dirty {
            self.revision += 1;
            self.canvas.take();
        }
        if static_dirty {
            self.static_revision = self.revision;
        }
        if kind != ScreenKind::Loading {
            self.prologue_scroll = self.stack.iter().find_map(|d| {
                d.gpu_prologue()
                    .map(|(rect, p)| (rect, p.scroll, p.strength, p.is_in_view(rect.height())))
            });
        }
        out
    }

    fn modeless_rect(&self) -> Option<[i32; 4]> {
        let top = self.stack.top()?;
        (self.kind == Some(ScreenKind::UnitTrading) && top.id == screens::unit_trading::UNTR)
            .then(|| self.overlay_dst(top).0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn it_fits_the_clients_box_with_no_tags() {
        let mut front_end: Box<dyn FrontEnd> = Box::new(EngineDialogs::new(TagLookup::empty()));
        let menu = mandrake_session::MainMenu::default();
        let input = FrameInput {
            now_ms: 0,
            size: (800, 600),
            events: &[],
            sound_busy: false,
            shift_held: false,
        };
        let out = front_end.frame(&Screen::MainMenu(&menu), &input);
        assert_eq!(out, FrameOutput::default());
    }

    #[test]
    fn gpu_picture_defers_software_canvas_compose() {
        let mut dialogs = EngineDialogs::new(TagLookup::empty());
        let menu = mandrake_session::MainMenu::default();
        let input = FrameInput {
            now_ms: 0,
            size: (800, 600),
            events: &[],
            sound_busy: false,
            shift_held: false,
        };
        dialogs.frame(&Screen::MainMenu(&menu), &input);
        let _ = dialogs.picture((800, 600));
        assert!(dialogs.canvas.get().is_none());
        let _ = dialogs.canvas();
        assert!(dialogs.canvas.get().is_some());
    }
}
