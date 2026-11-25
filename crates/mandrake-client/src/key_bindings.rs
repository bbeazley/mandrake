use mandrake_render::overlay::{OverlayDraw, OverlayImage, OverlayLayer, OverlayRenderer};
use mandrake_tag::Rect;
use mandrake_tag::font::{
    Bitmap32, FontFamily, RgbColor, TextClip, TextLocale, draw_string, string_width, text_flags,
};

use crate::controls::{Binding, ControlAction, Controls, MythKey};

pub(crate) const ROW_WIDTH: i32 = 0x136;
pub(crate) const ROW_HEIGHT: i32 = 0x19;
pub(crate) const COLUMN_GAP: i32 = 10;
pub(crate) const MARGIN: i32 = 3;
pub(crate) const BAND_TRANSPARENCY: u32 = 0x8000;
pub(crate) const FONT: usize = 1;
pub(crate) const PRIORITY: i16 = -500;
pub(crate) const ACTION_NAMES_ID: [u8; 4] = *b"inan";
pub(crate) const KEY_NAMES_ID: [u8; 4] = *b"inkn";

fn client_label(action: ControlAction) -> &'static str {
    use ControlAction as A;
    match action {
        A::Quit => "Quit",
        A::GameSpeedDown => "Game Speed Down",
        A::GameSpeedUp => "Game Speed Up",
        A::Screenshot => "Screenshot",
        A::ToggleFpsDisplay => "Frame Rate",
        A::ToggleFullscreen => "Full Screen",
        A::Quicksave => "Quicksave",
        A::ToggleControlBar => "Control Bar",
        A::ShowScores => "Show Scores",
        A::ToggleScores => "Toggle Scores",
        A::ShowKeyBindings => "Show Key Bindings",
        other => other.config_name(),
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct KeyNames {
    pub(crate) actions: Vec<Vec<u8>>,
    pub(crate) keys: Vec<Vec<u8>>,
}

impl KeyNames {
    fn action(&self, action: ControlAction) -> Vec<u8> {
        action
            .engine_id()
            .and_then(|id| self.actions.get(usize::from(id)))
            .filter(|s| !s.is_empty())
            .cloned()
            .unwrap_or_else(|| client_label(action).as_bytes().to_vec())
    }

    fn key(&self, key: MythKey) -> Vec<u8> {
        self.keys
            .get(usize::from(key.0))
            .filter(|s| !s.is_empty())
            .cloned()
            .unwrap_or_else(|| key.name().as_bytes().to_vec())
    }

    fn binding(&self, b: &Binding) -> Vec<u8> {
        let mut out = Vec::new();
        for (on, key) in [
            (b.modifiers.ctrl, MythKey::CONTROL),
            (b.modifiers.shift, MythKey::SHIFT),
            (b.modifiers.alt, MythKey::ALT_COMMAND),
        ] {
            if on {
                out.extend(self.key(key));
                out.push(b'+');
            }
        }
        out.extend(self.key(b.key));
        out
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(crate) struct KeyBindingRow {
    pub(crate) action: Vec<u8>,
    pub(crate) key: Vec<u8>,
}

pub(crate) fn myth2_order() -> Vec<ControlAction> {
    ControlAction::ALL
        .iter()
        .copied()
        .filter(|a| a.engine_id().is_some() && a.preset_index().is_none())
        .collect()
}

pub(crate) fn rows(
    controls: &Controls,
    order: &[ControlAction],
    names: &KeyNames,
) -> Vec<KeyBindingRow> {
    let client = ControlAction::ALL
        .iter()
        .copied()
        .filter(|a| a.engine_id().is_none() && !order.contains(a));
    order
        .iter()
        .copied()
        .chain(client)
        .flat_map(|a| {
            controls.bindings(a).iter().map(move |b| KeyBindingRow {
                action: names.action(a),
                key: names.binding(b),
            })
        })
        .collect()
}

pub(crate) fn rows_per_column(count: usize, (width, height): (i32, i32)) -> i32 {
    let n = count as i32;
    let half = (n + 1) >> 1;
    let fit = ((height - MARGIN) / ROW_HEIGHT).max(1);
    let per_column = half.min(fit).max(1);
    let columns_fit = ((width - MARGIN + COLUMN_GAP) / (ROW_WIDTH + COLUMN_GAP)).max(2);
    if (n + per_column - 1) / per_column <= columns_fit {
        per_column
    } else {
        ((n + columns_fit - 1) / columns_fit).max(per_column)
    }
}

pub(crate) fn row_rect(index: usize, per_column: i32) -> Rect {
    let (column, row) = (index as i32 / per_column, index as i32 % per_column);
    let left = MARGIN + column * (ROW_WIDTH + COLUMN_GAP);
    let top = MARGIN + row * ROW_HEIGHT;
    Rect::new(top, left, top + ROW_HEIGHT, left + ROW_WIDTH)
}

pub(crate) struct KeyBindingsStyle<'a> {
    pub(crate) family: &'a FontFamily,
    pub(crate) locale: &'a TextLocale,
    pub(crate) color: RgbColor,
}

pub(crate) fn image(
    rows: &[KeyBindingRow],
    frame: Rect,
    style: &KeyBindingsStyle<'_>,
) -> Option<(Rect, Bitmap32)> {
    if rows.is_empty() {
        return None;
    }
    let per_column = rows_per_column(rows.len(), (frame.width(), frame.height()));
    let rects: Vec<Rect> = (0..rows.len())
        .map(|i| row_rect(i, per_column).offset(frame.left, frame.top))
        .collect();
    let bounds = rects.iter().fold(rects[0], |a, b| {
        Rect::new(
            a.top.min(b.top),
            a.left.min(b.left),
            a.bottom.max(b.bottom),
            a.right.max(b.right),
        )
    });
    let text = |background: u32| {
        let mut bmp = Bitmap32::new(bounds.width(), bounds.height(), background);
        for (row, r) in rows.iter().zip(&rects) {
            let mut rect = Rect::new(
                r.top - bounds.top,
                r.left - bounds.left,
                r.bottom - bounds.top,
                r.right - bounds.left,
            );
            let mut draw = |rect: Rect, s: &[u8], flags: u32| {
                draw_string(
                    &mut bmp,
                    rect,
                    TextClip::None,
                    style.family,
                    -1,
                    s,
                    style.color,
                    flags,
                    0,
                    style.locale,
                );
            };
            draw(rect, &row.key, text_flags::MIDDLE | text_flags::RIGHT);
            rect.right -= string_width(style.family, -1, &row.key, style.locale);
            draw(rect, &row.action, text_flags::MIDDLE);
        }
        bmp
    };
    let (black, white) = (text(0xff00_0000), text(0xffff_ffff));
    let coverage = crate::command_bar::coverage_rgba(&black, &white);
    let band_alpha = 0xffff - BAND_TRANSPARENCY;
    let w = bounds.width();
    let h = bounds.height();
    let mut banded = vec![false; (w * h) as usize];
    for r in &rects {
        let y0 = (r.top - bounds.top).clamp(0, h);
        let y1 = (r.bottom - bounds.top).clamp(0, h);
        let x0 = (r.left - bounds.left).clamp(0, w);
        let x1 = (r.right - bounds.left).clamp(0, w);
        for y in y0..y1 {
            let row = (y * w) as usize;
            banded[row + x0 as usize..row + x1 as usize].fill(true);
        }
    }
    let mut out = Bitmap32::new(w, h, 0);
    for (i, px) in coverage.as_chunks::<4>().0.iter().enumerate() {
        let cov = u32::from(px[3]) * 0x101;
        let under = if banded[i] { band_alpha } else { 0 };
        let a = cov + (0xffff - cov) * under / 0xffff;
        let ch = |t: u8| {
            (u32::from(t) * cov)
                .checked_div(a)
                .map_or(0, |v| v.min(255))
        };
        out.pixels[i] = (a >> 8) << 24 | ch(px[0]) << 16 | ch(px[1]) << 8 | ch(px[2]);
    }
    let (full_w, full_h) = (
        bounds.right - frame.left + MARGIN,
        bounds.bottom - frame.top + MARGIN,
    );
    let scale = f64::min(
        f64::from(frame.width()) / f64::from(full_w),
        f64::from(frame.height()) / f64::from(full_h),
    );
    if scale < 1.0 {
        let w = ((f64::from(out.width) * scale).floor() as i32).max(1);
        let h = ((f64::from(out.height) * scale).floor() as i32).max(1);
        let left = frame.left + (f64::from(MARGIN) * scale).round() as i32;
        let top = frame.top + (f64::from(MARGIN) * scale).round() as i32;
        return Some((Rect::new(top, left, top + h, left + w), shrink(&out, w, h)));
    }
    Some((bounds, out))
}

fn shrink(src: &Bitmap32, w: i32, h: i32) -> Bitmap32 {
    let mut out = Bitmap32::new(w, h, 0);
    let (sx, sy) = (
        f64::from(src.width) / f64::from(w),
        f64::from(src.height) / f64::from(h),
    );
    for y in 0..h {
        let (y0, y1) = (
            (f64::from(y) * sy) as i32,
            ((f64::from(y + 1) * sy).ceil() as i32),
        );
        for x in 0..w {
            let (x0, x1) = (
                (f64::from(x) * sx) as i32,
                ((f64::from(x + 1) * sx).ceil() as i32),
            );
            let mut acc = [0u64; 4];
            let mut n = 0u64;
            for v in y0..y1.min(src.height) {
                for u in x0..x1.min(src.width) {
                    let p = src.get(u, v);
                    let a = u64::from(p >> 24);
                    acc[0] += a;
                    acc[1] += a * u64::from(p >> 16 & 0xff);
                    acc[2] += a * u64::from(p >> 8 & 0xff);
                    acc[3] += a * u64::from(p & 0xff);
                    n += 1;
                }
            }
            if n == 0 || acc[0] == 0 {
                continue;
            }
            let a = (acc[0] / n) as u32;
            let c = |k: usize| (acc[k] / acc[0]) as u32;
            out.set(x, y, a << 24 | c(1) << 16 | c(2) << 8 | c(3));
        }
    }
    out
}

pub(crate) fn rgba(bmp: &Bitmap32) -> Vec<u8> {
    bmp.pixels
        .iter()
        .flat_map(|&p| [(p >> 16) as u8, (p >> 8) as u8, p as u8, (p >> 24) as u8])
        .collect()
}

#[derive(Default)]
pub(crate) struct KeyBindingsImage {
    key: Option<(Vec<KeyBindingRow>, Rect)>,
    image: Option<(Rect, OverlayImage)>,
}

impl KeyBindingsImage {
    pub(crate) fn update(
        &mut self,
        renderer: &OverlayRenderer,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        rows: Option<&[KeyBindingRow]>,
        frame: Rect,
        style: &KeyBindingsStyle<'_>,
    ) {
        let Some(rows) = rows else {
            self.key = None;
            self.image = None;
            return;
        };
        if self
            .key
            .as_ref()
            .is_some_and(|(r, f)| r.as_slice() == rows && *f == frame)
        {
            return;
        }
        self.key = Some((rows.to_vec(), frame));
        self.image = image(rows, frame, style).map(|(r, bmp)| {
            let image = renderer.create_image(
                device,
                queue,
                bmp.width as u32,
                bmp.height as u32,
                &rgba(&bmp),
            );
            (r, image)
        });
    }

    pub(crate) fn draws(&self) -> Option<OverlayDraw<'_>> {
        let (r, image) = self.image.as_ref()?;
        Some(OverlayDraw::at(
            OverlayLayer::PostRasterization(PRIORITY),
            image,
            r.left,
            r.top,
            0xff,
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names() -> KeyNames {
        let mut actions = vec![Vec::new(); 54];
        actions[2] = b"Move Forward".to_vec();
        actions[12] = b"Stop".to_vec();
        let mut keys = vec![Vec::new(); 100];
        keys[0x20] = b"W key".to_vec();
        KeyNames { actions, keys }
    }

    #[test]
    fn rows_are_the_live_bindings_with_the_games_names() {
        let (c, d) = crate::controls::Controls::with_override(
            "[bindings]\nmove_forward = [\"W\", \"Ctrl+Up\"]\nstop_units = []\n",
            "user.toml",
        );
        assert!(d.is_empty(), "{d:?}");
        let rows = rows(&c, &myth2_order(), &names());
        assert_eq!(rows[0].action, b"Move Forward".to_vec());
        assert_eq!(rows[0].key, b"W key".to_vec());
        assert_eq!(rows[1].key, b"Control+Up Arrow".to_vec());
        assert!(!rows.iter().any(|r| r.action == b"Stop".to_vec()));
        let f8 = rows
            .iter()
            .position(|r| r.action == b"Show Key Bindings".to_vec());
        assert_eq!(rows[f8.unwrap()].key, b"F8".to_vec());
        assert!(f8.unwrap() > rows.iter().position(|r| r.key == b"Tab".to_vec()).unwrap());
    }

    #[test]
    fn the_layout_is_tfls_two_columns() {
        assert_eq!(rows_per_column(28, (640, 480)), 14);
        assert_eq!(rows_per_column(27, (640, 480)), 14);
        assert_eq!(row_rect(0, 14), Rect::new(3, 3, 28, 313));
        assert_eq!(row_rect(13, 14), Rect::new(328, 3, 353, 313));
        assert_eq!(row_rect(14, 14), Rect::new(3, 323, 28, 633));
        assert_eq!(rows_per_column(44, (1024, 480)), 19);
        assert_eq!(row_rect(38, 19), Rect::new(3, 643, 28, 953));
        assert_eq!(rows_per_column(44, (640, 480)), 22);
    }
}
