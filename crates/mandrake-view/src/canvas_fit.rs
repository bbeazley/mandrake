//! The menus' fit to the window (design section 7): the canvas keeps its shape, scaled by a fraction
//! where it needs to be, centred on black. Not `ui_scale.rs`, which is for play.

/// Where a canvas sits in a window.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CanvasFit {
    /// The canvas's top-left in the window, in window pixels.
    pub origin: (i32, i32),
    /// Window pixels per canvas pixel.
    pub factor: f64,
    /// The canvas's size in the window, in window pixels.
    pub size: (i32, i32),
}

impl CanvasFit {
    pub fn new(window: (u32, u32), canvas: (u32, u32)) -> Self {
        let (w, h) = (window.0.max(1) as f64, window.1.max(1) as f64);
        let (cw, ch) = (canvas.0.max(1) as f64, canvas.1.max(1) as f64);
        let factor = (w / cw).min(h / ch);
        let size = ((cw * factor).round() as i32, (ch * factor).round() as i32);
        let origin = (
            (window.0 as i32 - size.0) / 2,
            (window.1 as i32 - size.1) / 2,
        );
        Self {
            origin,
            factor,
            size,
        }
    }

    /// A window point on the canvas. It can fall outside it, on the black.
    pub fn to_canvas(&self, x: i32, y: i32) -> (i32, i32) {
        (
            (f64::from(x - self.origin.0) / self.factor).floor() as i32,
            (f64::from(y - self.origin.1) / self.factor).floor() as i32,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The design's table.
    #[test]
    fn the_scale_table() {
        for (window, f800, f640) in [
            ((1280, 720), 1.2, 1.5),
            ((1920, 1080), 1.8, 2.25),
            ((2560, 1440), 2.4, 3.0),
            ((3840, 2160), 3.6, 4.5),
        ] {
            assert_eq!(
                CanvasFit::new(window, (800, 600)).factor,
                f800,
                "{window:?}"
            );
            assert_eq!(
                CanvasFit::new(window, (640, 480)).factor,
                f640,
                "{window:?}"
            );
        }
    }

    #[test]
    fn it_fills_one_direction_and_centres_the_other() {
        let fit = CanvasFit::new((3840, 2160), (800, 600));
        assert_eq!(fit.size, (2880, 2160));
        assert_eq!(fit.origin, (480, 0));
        let tall = CanvasFit::new((800, 1000), (800, 600));
        assert_eq!((tall.size, tall.origin), ((800, 600), (0, 200)));
        let small = CanvasFit::new((400, 300), (800, 600));
        assert_eq!((small.factor, small.size), (0.5, (400, 300)));
    }

    #[test]
    fn the_pointer_maps_back_at_a_fractional_factor() {
        let fit = CanvasFit::new((3840, 2160), (800, 600));
        assert_eq!(fit.to_canvas(480, 0), (0, 0));
        assert_eq!(fit.to_canvas(480 + 2879, 2159), (799, 599));
        assert_eq!(fit.to_canvas(479, 0).0, -1, "the black to the left");
        // x3.6: pixel 100 of the canvas covers 360..363.6 of the scaled canvas.
        assert_eq!(fit.to_canvas(480 + 360, 0).0, 100);
        assert_eq!(fit.to_canvas(480 + 363, 0).0, 100);
        assert_eq!(fit.to_canvas(480 + 364, 0).0, 101);
        let one = CanvasFit::new((800, 600), (800, 600));
        assert_eq!(one.to_canvas(270, 298), (270, 298));
    }

    #[test]
    fn dialog_sized_source_fits_window_filling_height_or_entire_screen() {
        // At 1920x1080 (16:9), a 640x480 dialog scales by 2.25 (1440x1080), centred with 240px pillarbox bars.
        let fit_1080p = CanvasFit::new((1920, 1080), (640, 480));
        assert_eq!(fit_1080p.factor, 2.25);
        assert_eq!(fit_1080p.size, (1440, 1080));
        assert_eq!(fit_1080p.origin, (240, 0));
        assert_eq!(fit_1080p.to_canvas(240, 0), (0, 0));
        assert_eq!(fit_1080p.to_canvas(240 + 1439, 1079), (639, 479));
        assert_eq!(fit_1080p.to_canvas(239, 0).0, -1, "pillarbox left");
        assert_eq!(fit_1080p.to_canvas(240 + 1440, 0).0, 640, "pillarbox right");

        // At 1024x768 (4:3), a 640x480 dialog scales by 1.6 (1024x768) and fills the entire screen.
        let fit_4_3 = CanvasFit::new((1024, 768), (640, 480));
        assert_eq!(fit_4_3.factor, 1.6);
        assert_eq!(fit_4_3.size, (1024, 768));
        assert_eq!(fit_4_3.origin, (0, 0));
        assert_eq!(fit_4_3.to_canvas(0, 0), (0, 0));
        assert_eq!(fit_4_3.to_canvas(1023, 767), (639, 479));

        // At 1892x1023 (windowed 1080p), dialog scales to fill height (1023 / 480).
        let fit_win = CanvasFit::new((1892, 1023), (640, 480));
        assert_eq!(fit_win.factor, 1023.0 / 480.0);
        assert_eq!(fit_win.size, (1364, 1023));
        assert_eq!(fit_win.origin, (264, 0));
    }
}
