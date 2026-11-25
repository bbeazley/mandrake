use mandrake_session::Snapshot;
use mandrake_tag::font::Bitmap32;

use super::{ControlProc, DrawContext, ItemState};

#[derive(Default)]
pub struct SnapshotItem {
    snapshot: Option<Snapshot>,
}

impl SnapshotItem {
    pub fn set(&mut self, item: &mut ItemState, snapshot: Option<Snapshot>) {
        self.snapshot = snapshot;
        item.dirty = true;
    }
}

impl ControlProc for SnapshotItem {
    fn draw(&self, item: &ItemState, _ctx: &DrawContext<'_>, bitmap: &mut Bitmap32) {
        let Some(s) = &self.snapshot else {
            return;
        };
        let r = item.rect;
        let (w, h) = (s.width as i32, s.height as i32);
        if w <= 0 || h <= 0 {
            return;
        }
        let needed = (w as usize).saturating_mul(h as usize).saturating_mul(4);
        if s.rgba.len() < needed {
            return;
        }
        let left = r.left + (r.right - r.left - w) / 2;
        let top = r.top + (r.bottom - r.top - h) / 2;
        for y in 0..h {
            let dy = top + y;
            if !(0..bitmap.height).contains(&dy) {
                continue;
            }
            for x in 0..w {
                let dx = left + x;
                if !(0..bitmap.width).contains(&dx) {
                    continue;
                }
                let p = &s.rgba[((y * w + x) * 4) as usize..][..3];
                let v =
                    0xff00_0000 | u32::from(p[0]) << 16 | u32::from(p[1]) << 8 | u32::from(p[2]);
                bitmap.set(dx, dy, v);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mandrake_common::FourCC;
    use mandrake_tag::Rect;

    #[test]
    fn malformed_short_rgba_snapshot_does_not_panic() {
        let mut item = ItemState {
            id: 5,
            kind: FourCC(*b"snap"),
            rect: Rect::new(0, 0, 32, 32),
            flags: 0,
            pressed: false,
            dirty: false,
        };
        let mut snap = SnapshotItem::default();
        snap.set(
            &mut item,
            Some(Snapshot {
                width: 16,
                height: 16,
                rgba: vec![0; 10],
            }),
        );
        let res = crate::resources::Resources::from_source(|_, _| None);
        let ctx = DrawContext {
            res: &res,
            now_tick: 0,
            focused: false,
        };
        let mut bmp = Bitmap32::new(32, 32, 0);
        snap.draw(&item, &ctx, &mut bmp);
    }
}
