use crate::controls::{Modifiers, MythKey};
use mandrake_view::canvas_fit::CanvasFit;

pub const ZOOM_LEVELS: [u16; 8] = [100, 125, 150, 175, 200, 250, 300, 400];

pub const ZOOM_REFERENCE: (u32, u32) = (640, 480);

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum UiScaleMode {
    #[default]
    Native,
    Lines768,
    Zoom(u16),
}

impl UiScaleMode {
    pub fn from_zoom(percent: u16) -> Self {
        if percent <= 100 {
            Self::Native
        } else {
            Self::Zoom(percent)
        }
    }

    pub fn filter(self, chosen: Option<UiFilter>) -> UiFilter {
        chosen.unwrap_or(match self {
            Self::Zoom(_) => UiFilter::Lanczos2,
            Self::Native | Self::Lines768 => UiFilter::Sharp,
        })
    }
}

impl std::str::FromStr for UiScaleMode {
    type Err = String;

    fn from_str(text: &str) -> Result<Self, String> {
        let text = text.trim().to_ascii_lowercase();
        match text.as_str() {
            "native" => return Ok(Self::Native),
            "768p" => return Ok(Self::Lines768),
            _ => {}
        }
        let levels = || {
            let levels: Vec<String> = ZOOM_LEVELS
                .iter()
                .map(|&l| format!("{}", f64::from(l) / 100.0))
                .collect();
            format!(
                "expected native, 768p or a level: {} (or in percent: 100, 125, ...)",
                levels.join(", ")
            )
        };
        let number = text
            .strip_suffix('%')
            .map(|n| (n, true))
            .or_else(|| text.strip_suffix('x').map(|n| (n, false)))
            .unwrap_or((&text, false));
        let (value, percent) = number;
        let value: f64 = value.parse().map_err(|_| levels())?;
        let percent = if percent || value > 4.0 {
            value
        } else {
            value * 100.0
        };
        ZOOM_LEVELS
            .iter()
            .find(|&&l| f64::from(l) == percent)
            .map(|&l| Self::from_zoom(l))
            .ok_or_else(levels)
    }
}

fn zoom_limit(screen: (u32, u32), reference: (u32, u32)) -> u32 {
    (screen.0 * 100 / reference.0).min(screen.1 * 100 / reference.1)
}

pub fn effective_zoom(wanted: u16, screen: (u32, u32), reference: (u32, u32)) -> u16 {
    let limit = zoom_limit(screen, reference);
    let wanted = i64::from(wanted).pow(2);
    let mut best: Option<(i64, u16)> = None;
    for &level in ZOOM_LEVELS.iter().rev() {
        let distance = (i64::from(level).pow(2) - wanted).abs();
        if u32::from(level) <= limit && best.is_none_or(|(d, _)| distance < d) {
            best = Some((distance, level));
        }
    }
    best.map_or(100, |(_, level)| level)
}

pub fn step_zoom(current: u16, delta: i32, screen: (u32, u32)) -> u16 {
    let index = ZOOM_LEVELS
        .iter()
        .position(|&l| l >= current)
        .unwrap_or(ZOOM_LEVELS.len() - 1) as i32;
    let next = (index + delta).clamp(0, ZOOM_LEVELS.len() as i32 - 1);
    effective_zoom(ZOOM_LEVELS[next as usize], screen, ZOOM_REFERENCE)
}

pub fn zoom_after_press(mode: UiScaleMode, ui: &UiScale, delta: i32, screen: (u32, u32)) -> u16 {
    if mode == UiScaleMode::Lines768 {
        let nearest = effective_zoom((ui.factor * 100.0).round() as u16, screen, ZOOM_REFERENCE);
        if ui.is_scaled() {
            nearest
        } else {
            step_zoom(nearest, delta, screen)
        }
    } else {
        step_zoom(ui.zoom.unwrap_or(100), delta, screen)
    }
}

pub fn min_window(percent: u16) -> (u32, u32) {
    let dpi = u32::from(percent) * 96 / 100;
    let mul_div = |n: u32| (n * dpi + 48) / 96;
    (mul_div(ZOOM_REFERENCE.0), mul_div(ZOOM_REFERENCE.1))
}

pub fn zoom_key_delta(key: MythKey, held: Modifiers) -> Option<i32> {
    if !held.ctrl || held.alt {
        return None;
    }
    match key.0 {
        0x1c | 0x61 => Some(1),
        0x1b | 0x60 => Some(-1),
        _ => None,
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, clap::ValueEnum)]
pub enum UiFilter {
    #[default]
    Sharp,
    Nearest,
    Linear,
    Lanczos2,
}

impl From<UiFilter> for mandrake_view::interface_picture::InterfaceFilter {
    fn from(f: UiFilter) -> Self {
        match f {
            UiFilter::Sharp => Self::Sharp,
            UiFilter::Nearest => Self::Nearest,
            UiFilter::Linear => Self::Linear,
            UiFilter::Lanczos2 => Self::Lanczos2,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, clap::ValueEnum)]
pub enum Sampling {
    #[default]
    Software,
    Linear,
    Bicubic,
    Sharp,
}

impl From<Sampling> for mandrake_render::TextureSampling {
    fn from(s: Sampling) -> Self {
        match s {
            Sampling::Software => Self::Software,
            Sampling::Linear => Self::Linear,
            Sampling::Bicubic => Self::Bicubic,
            Sampling::Sharp => Self::Sharp,
        }
    }
}

pub fn with_world_sampling(
    uniforms: mandrake_render::SceneUniforms,
    sampling: Sampling,
    sprite_filter: Option<UiFilter>,
) -> mandrake_render::SceneUniforms {
    let uniforms = uniforms.with_sampling(sampling.into());
    match sprite_filter {
        Some(filter) => uniforms.with_sprite_filter(filter.into()),
        None => uniforms,
    }
}

pub fn dialog_fit(ui: UiScale, canvas: (u32, u32)) -> CanvasFit {
    let max_fit = CanvasFit::new(ui.window, canvas);
    let factor = ui.factor.min(max_fit.factor);
    let size = (
        (f64::from(canvas.0.max(1)) * factor).round() as i32,
        (f64::from(canvas.1.max(1)) * factor).round() as i32,
    );
    CanvasFit {
        origin: (
            (ui.window.0 as i32 - size.0) / 2,
            (ui.window.1 as i32 - size.1) / 2,
        ),
        factor,
        size,
    }
}

pub const LOGICAL_LINES: u32 = 768;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct UiScale {
    pub window: (u32, u32),
    pub logical: (u32, u32),
    pub factor: f64,
    pub zoom: Option<u16>,
}

impl UiScale {
    pub fn new(mode: UiScaleMode, width: u32, height: u32) -> Self {
        let window = (width.max(1), height.max(1));
        match mode {
            UiScaleMode::Native => Self::native(window),
            UiScaleMode::Zoom(choice) => {
                let percent = effective_zoom(choice, window, ZOOM_REFERENCE);
                if percent == 100 {
                    return Self::native(window);
                }
                let dpi = u32::from(percent) * 96 / 100;
                Self {
                    window,
                    logical: (window.0 * 96 / dpi, window.1 * 96 / dpi),
                    factor: f64::from(dpi) / 96.0,
                    zoom: Some(percent),
                }
            }
            UiScaleMode::Lines768 => {
                let s = f64::from(window.1) / f64::from(LOGICAL_LINES);
                let lw = (f64::from(window.0) / s).round().max(1.0) as u32;
                Self {
                    window,
                    logical: (lw, LOGICAL_LINES),
                    factor: s,
                    zoom: None,
                }
            }
        }
    }

    fn native(window: (u32, u32)) -> Self {
        Self {
            window,
            logical: window,
            factor: 1.0,
            zoom: None,
        }
    }

    pub fn filter(&self, chosen: Option<UiFilter>) -> UiFilter {
        match self.zoom {
            Some(percent) => UiScaleMode::Zoom(percent),
            None => UiScaleMode::Lines768,
        }
        .filter(chosen)
    }

    pub fn is_scaled(&self) -> bool {
        self.logical != self.window
    }

    pub fn axis_factors(&self) -> (f64, f64) {
        (
            f64::from(self.window.0) / f64::from(self.logical.0),
            f64::from(self.window.1) / f64::from(self.logical.1),
        )
    }

    pub fn to_logical(self, x: f32, y: f32) -> (i32, i32) {
        if !self.is_scaled() {
            return (x as i32, y as i32);
        }
        if let Some(percent) = self.zoom {
            let dpi = i32::from(percent) * 96 / 100;
            let scale = |v: f32, size: u32| (v as i32).clamp(0, size as i32 - 1) * 96 / dpi;
            return (scale(x, self.window.0), scale(y, self.window.1));
        }
        let (sx, sy) = self.axis_factors();
        (
            (f64::from(x) / sx).floor() as i32,
            (f64::from(y) / sy).floor() as i32,
        )
    }

    pub fn view_bars(&self, bars: (i32, i32)) -> (i32, i32) {
        if !self.is_scaled() {
            return bars;
        }
        let sy = self.axis_factors().1;
        let top = (f64::from(bars.0) * sy).round() as i32;
        let bottom_edge = (f64::from(self.logical.1 as i32 - bars.1) * sy).round() as i32;
        (top, self.window.1 as i32 - bottom_edge)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn logical_screens_of_the_spec() {
        let s = UiScale::new(UiScaleMode::Lines768, 3840, 2160);
        assert_eq!(s.logical, (1365, 768));
        assert_eq!(s.factor, 2.8125);
        assert!(s.is_scaled());
        let s = UiScale::new(UiScaleMode::Lines768, 2560, 1440);
        assert_eq!((s.logical, s.factor), ((1365, 768), 1.875));
        let s = UiScale::new(UiScaleMode::Lines768, 1024, 768);
        assert_eq!((s.logical, s.is_scaled()), ((1024, 768), false));
        let s = UiScale::new(UiScaleMode::Native, 3840, 2160);
        assert_eq!(
            (s.logical, s.factor, s.is_scaled()),
            ((3840, 2160), 1.0, false)
        );
    }

    #[test]
    fn input_and_bars_scale() {
        let s = UiScale::new(UiScaleMode::Lines768, 3840, 2160);
        assert_eq!(s.to_logical(0.0, 0.0), (0, 0));
        assert_eq!(s.to_logical(3839.0, 2159.0), (1364, 767));
        assert_eq!(s.to_logical(28.2, 28.2), (10, 10));
        assert_eq!(s.view_bars((64, 32)), (180, 90));
        let n = UiScale::new(UiScaleMode::Native, 3840, 2160);
        assert_eq!(n.to_logical(10.9, 20.5), (10, 20));
        assert_eq!(n.view_bars((64, 32)), (64, 32));
    }

    #[test]
    fn zoom_logical_screens_of_the_table() {
        for (w, h, pct, lw, lh) in [
            (1024, 768, 125, 819, 614),
            (1024, 768, 150, 682, 512),
            (1920, 1080, 125, 1536, 864),
            (1920, 1080, 150, 1280, 720),
            (1920, 1080, 175, 1097, 617),
            (1920, 1080, 200, 960, 540),
            (3840, 2160, 150, 2560, 1440),
            (3840, 2160, 175, 2194, 1234),
            (3840, 2160, 200, 1920, 1080),
            (3840, 2160, 250, 1536, 864),
            (3840, 2160, 300, 1280, 720),
            (3840, 2160, 400, 960, 540),
        ] {
            let ui = UiScale::new(UiScaleMode::Zoom(pct), w, h);
            assert_eq!(ui.logical, (lw, lh), "{w}x{h} at {pct}");
            assert_eq!(ui.zoom, Some(pct));
            assert_eq!(ui.factor, f64::from(pct) / 100.0);
            assert!(ui.is_scaled());
        }
    }

    #[test]
    fn a_screens_filter_is_its_modes() {
        let lines = UiScale::new(UiScaleMode::Lines768, 3840, 2160);
        assert_eq!(lines.filter(None), UiFilter::Sharp);
        let zoom = UiScale::new(UiScaleMode::Zoom(150), 1920, 1080);
        assert_eq!(zoom.filter(None), UiFilter::Lanczos2);
        assert_eq!(zoom.filter(None), UiScaleMode::Zoom(150).filter(None));
        assert_eq!(lines.filter(Some(UiFilter::Nearest)), UiFilter::Nearest);
        assert_eq!(zoom.filter(Some(UiFilter::Linear)), UiFilter::Linear);
    }

    #[test]
    fn zoom_100_is_native() {
        let native = UiScale::new(UiScaleMode::Native, 3840, 2160);
        assert_eq!(
            UiScale::new(UiScaleMode::from_zoom(100), 3840, 2160),
            native
        );
        assert_eq!(UiScaleMode::from_zoom(100), UiScaleMode::Native);
        let tiny = UiScale::new(UiScaleMode::Zoom(200), 800, 500);
        assert_eq!(tiny, UiScale::new(UiScaleMode::Native, 800, 500));
    }

    #[test]
    fn effective_zoom_limits() {
        for (screen, top) in [
            ((1024, 768), 150),
            ((1920, 1080), 200),
            ((3840, 2160), 400),
            ((640, 480), 100),
            ((639, 480), 100),
        ] {
            assert_eq!(
                effective_zoom(400, screen, ZOOM_REFERENCE),
                top,
                "{screen:?}"
            );
        }
        assert_eq!(effective_zoom(400, (1024, 768), (800, 600)), 125);
        assert_eq!(effective_zoom(400, (1920, 1080), (800, 600)), 175);
        assert_eq!(effective_zoom(400, (3840, 2160), (800, 600)), 300);
        assert_eq!(effective_zoom(130, (3840, 2160), ZOOM_REFERENCE), 125);
        assert_eq!(effective_zoom(281, (3840, 2160), ZOOM_REFERENCE), 300);
        let ui = UiScale::new(UiScaleMode::Zoom(400), 1024, 768);
        assert_eq!(ui.zoom, Some(150));
        assert_eq!(
            UiScale::new(UiScaleMode::Zoom(400), 3840, 2160).zoom,
            Some(400)
        );
    }

    #[test]
    fn step_zoom_stays_at_both_ends() {
        let screen = (3840, 2160);
        assert_eq!(step_zoom(100, -1, screen), 100);
        assert_eq!(step_zoom(400, 1, screen), 400);
        let mut level = 100;
        for next in &ZOOM_LEVELS[1..] {
            level = step_zoom(level, 1, screen);
            assert_eq!(level, *next);
        }
        for next in ZOOM_LEVELS[..7].iter().rev() {
            level = step_zoom(level, -1, screen);
            assert_eq!(level, *next);
        }
        assert_eq!(step_zoom(150, 1, (1024, 768)), 150);
        assert_eq!(step_zoom(200, 1, (1920, 1080)), 200);
        assert_eq!(step_zoom(175, 1, (1920, 1080)), 200);
    }

    #[test]
    fn press_from_each_mode() {
        let screen = (3840, 2160);
        let press =
            |mode, w, h, delta| zoom_after_press(mode, &UiScale::new(mode, w, h), delta, screen);
        assert_eq!(press(UiScaleMode::Native, 3840, 2160, 1), 125);
        assert_eq!(press(UiScaleMode::Native, 3840, 2160, -1), 100);
        assert_eq!(press(UiScaleMode::Lines768, 3840, 2160, 1), 300);
        assert_eq!(press(UiScaleMode::Lines768, 3840, 2160, -1), 300);
        assert_eq!(press(UiScaleMode::Lines768, 2560, 1440, 1), 200);
        assert_eq!(press(UiScaleMode::Lines768, 1920, 1080, 1), 150);
        assert_eq!(press(UiScaleMode::Lines768, 1024, 768, 1), 125);
        assert_eq!(press(UiScaleMode::Zoom(400), 1024, 768, -1), 125);
        assert_eq!(press(UiScaleMode::Zoom(400), 1024, 768, 1), 175);
    }

    #[test]
    fn zoom_input_is_the_engines_integer_form() {
        let ui = UiScale::new(UiScaleMode::Zoom(175), 1920, 1080);
        assert_eq!(ui.to_logical(1919.0, 1079.0), (1096, 616));
        assert_eq!(ui.to_logical(5000.0, 5000.0), (1096, 616));
        assert_eq!(ui.to_logical(-3.0, -3.0), (0, 0));
        assert_eq!(ui.to_logical(175.9, 7.0), (100, 4));
        let ui = UiScale::new(UiScaleMode::Zoom(300), 3840, 2160);
        assert_eq!(ui.to_logical(3839.0, 2159.0), (1279, 719));
    }

    #[test]
    fn zoom_bars_are_the_engines() {
        for (w, h) in [
            (1024, 768),
            (1280, 720),
            (1920, 1080),
            (2560, 1440),
            (3840, 2160),
        ] {
            for pct in ZOOM_LEVELS {
                let ui = UiScale::new(UiScaleMode::Zoom(pct), w, h);
                let dpi = i32::from(ui.zoom.unwrap_or(100)) * 96 / 100;
                assert_eq!(64 * dpi % 96, 0);
                assert_eq!(32 * dpi % 96, 0);
                assert_eq!(
                    ui.view_bars((64, 32)),
                    (64 * dpi / 96, 32 * dpi / 96),
                    "{w}x{h} at {pct}"
                );
            }
        }
    }

    #[test]
    fn dialog_fit_follows_ui_scale_and_clamps_to_window() {
        let canvas = (640, 480);

        let native = UiScale::new(UiScaleMode::Native, 1920, 1080);
        let fit = dialog_fit(native, canvas);
        assert_eq!(
            (fit.factor, fit.size, fit.origin),
            (1.0, (640, 480), (640, 300))
        );
        assert_eq!(fit.to_canvas(960, 540), (320, 240));

        let zoom150 = UiScale::new(UiScaleMode::Zoom(150), 1920, 1080);
        let fit = dialog_fit(zoom150, canvas);
        assert_eq!(
            (fit.factor, fit.size, fit.origin),
            (1.5, (960, 720), (480, 180))
        );
        assert_eq!(fit.to_canvas(960, 540), (320, 240));

        let zoom200 = UiScale::new(UiScaleMode::Zoom(200), 3840, 2160);
        let fit = dialog_fit(zoom200, canvas);
        assert_eq!(
            (fit.factor, fit.size, fit.origin),
            (2.0, (1280, 960), (1280, 600))
        );
        assert_eq!(fit.to_canvas(1920, 1080), (320, 240));

        let lines = UiScale::new(UiScaleMode::Lines768, 3840, 2160);
        let fit = dialog_fit(lines, canvas);
        assert_eq!(
            (fit.factor, fit.size, fit.origin),
            (2.8125, (1800, 1350), (1020, 405))
        );
        assert_eq!(fit.to_canvas(1920, 1080), (320, 240));

        let tiny = UiScale::new(UiScaleMode::Native, 320, 240);
        let fit = dialog_fit(tiny, canvas);
        assert_eq!(
            (fit.factor, fit.size, fit.origin),
            (0.5, (320, 240), (0, 0))
        );

        let narrow_tall = UiScale::new(UiScaleMode::Lines768, 640, 960);
        let fit = dialog_fit(narrow_tall, canvas);
        assert_eq!(
            (fit.factor, fit.size, fit.origin),
            (1.0, (640, 480), (0, 240))
        );
    }

    #[test]
    fn min_windows_of_the_engine() {
        assert_eq!(min_window(175), (1120, 840));
        assert_eq!(min_window(200), (1280, 960));
        assert_eq!(min_window(250), (1600, 1200));
        assert_eq!(min_window(300), (1920, 1440));
    }

    #[test]
    fn zoom_keys() {
        let held = |ctrl, shift, alt| Modifiers { ctrl, shift, alt };
        let key = |name| MythKey::from_name(name).unwrap();
        for (name, delta) in [
            ("Plus", 1),
            ("Keypad Plus", 1),
            ("Minus", -1),
            ("Keypad Minus", -1),
        ] {
            assert_eq!(
                zoom_key_delta(key(name), held(true, false, false)),
                Some(delta)
            );
            assert_eq!(
                zoom_key_delta(key(name), held(true, true, false)),
                Some(delta)
            );
            assert_eq!(zoom_key_delta(key(name), held(true, false, true)), None);
            assert_eq!(zoom_key_delta(key(name), held(false, false, true)), None);
            assert_eq!(zoom_key_delta(key(name), held(false, false, false)), None);
        }
        assert_eq!(zoom_key_delta(key("W"), held(true, false, false)), None);
    }

    #[test]
    fn ui_scale_values() {
        let parse = |v: &str| v.parse::<UiScaleMode>();
        assert_eq!(parse("native"), Ok(UiScaleMode::Native));
        assert_eq!(parse("768p"), Ok(UiScaleMode::Lines768));
        for v in ["1.5", "1.5x", "150", "150%"] {
            assert_eq!(parse(v), Ok(UiScaleMode::Zoom(150)), "{v}");
        }
        assert_eq!(parse("3"), Ok(UiScaleMode::Zoom(300)));
        assert_eq!(parse("400"), Ok(UiScaleMode::Zoom(400)));
        assert_eq!(parse("1"), Ok(UiScaleMode::Native));
        assert_eq!(parse("100"), Ok(UiScaleMode::Native));
        for v in ["1.3", "130", "5", "0", "-2", "abc", "720p", ""] {
            assert!(parse(v).is_err(), "{v}");
        }
    }

    #[test]
    fn ui_filter_values() {
        use clap::ValueEnum;
        let parse = |v| UiFilter::from_str(v, false);
        assert_eq!(parse("sharp"), Ok(UiFilter::Sharp));
        assert_eq!(parse("nearest"), Ok(UiFilter::Nearest));
        assert_eq!(parse("linear"), Ok(UiFilter::Linear));
        assert_eq!(parse("lanczos2"), Ok(UiFilter::Lanczos2));
        assert!(parse("lanczos3").is_err());
        assert_eq!(UiFilter::default(), UiFilter::Sharp);
        assert_eq!(UiScaleMode::Lines768.filter(None), UiFilter::Sharp);
        assert_eq!(UiScaleMode::Zoom(200).filter(None), UiFilter::Lanczos2);
        assert_eq!(
            UiScaleMode::Zoom(200).filter(Some(UiFilter::Nearest)),
            UiFilter::Nearest
        );
        assert_eq!(
            mandrake_view::interface_picture::InterfaceFilter::from(UiFilter::Lanczos2),
            mandrake_view::interface_picture::InterfaceFilter::Lanczos2
        );
    }
}
