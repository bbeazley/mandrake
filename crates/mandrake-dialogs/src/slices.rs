use std::ops::Range;

use mandrake_tag::Rect;
use mandrake_tag::font::Bitmap32;
use mandrake_view::interface_bitmap::{InterfaceArt, copy_bitmap};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SliceRun {
    pub columns: Range<i32>,
    pub mirrored: bool,
}

impl SliceRun {
    pub fn new(columns: Range<i32>, mirrored: bool) -> Self {
        Self { columns, mirrored }
    }

    pub fn len(&self) -> i32 {
        (self.columns.end - self.columns.start).max(0)
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub fn column(&self, i: i32) -> i32 {
        if self.mirrored {
            self.columns.end - 1 - i
        } else {
            self.columns.start + i
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Piece {
    pub at: i32,
    pub run: SliceRun,
    pub visible: i32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ThreeSlice {
    pub left_end: i32,
    pub right_end: i32,
    pub middle: Vec<SliceRun>,
    pub overlap: i32,
    pub blend_rows: Range<i32>,
}

impl ThreeSlice {
    pub fn layout(&self, width: i32, painted: i32) -> Vec<Piece> {
        if width <= painted {
            return vec![Piece {
                at: 0,
                run: SliceRun::new(0..painted, false),
                visible: width.max(0),
            }];
        }
        let end = width - (painted - self.right_end);
        let mut pieces = vec![Piece {
            at: 0,
            run: SliceRun::new(0..self.left_end, false),
            visible: self.left_end,
        }];
        let mut at = self.left_end;
        let usable = self.middle.iter().any(|r| r.len() > self.overlap);
        let runs = self.middle.iter().filter(|r| r.len() > self.overlap);
        for run in runs.cycle().take_while(|_| usable) {
            if at >= end - self.overlap {
                break;
            }
            pieces.push(Piece {
                at,
                run: run.clone(),
                visible: run.len().min(end - at),
            });
            at += run.len() - self.overlap;
        }
        let lead = if usable { self.overlap } else { 0 };
        let right = SliceRun::new(self.right_end - lead..painted, false);
        pieces.push(Piece {
            at: end - lead,
            visible: right.len(),
            run: right,
        });
        pieces
    }

    pub fn compose(&self, painting: &Bitmap32, width: i32) -> Bitmap32 {
        let h = painting.height;
        let w = width.max(0);
        let rgb = |p: u32| [16, 8, 0].map(|k| ((p >> k) & 0xff) as f32);
        let mut canvas = vec![[0f32; 3]; (w * h.max(0)) as usize];
        let mut filled = vec![false; w as usize];
        for piece in self.layout(width, painting.width) {
            for i in 0..piece.visible {
                let x = piece.at + i;
                if x < 0 || x >= w {
                    continue;
                }
                let sx = piece.run.column(i);
                let blend = filled[x as usize] && self.overlap > 0;
                let f = (i as f32 + 0.5) / self.overlap.max(1) as f32;
                for y in 0..h {
                    let new = rgb(painting.get(sx, y));
                    let c = &mut canvas[(y * w + x) as usize];
                    if !blend || (!self.blend_rows.contains(&y) && f >= 0.5) {
                        *c = new;
                    } else if self.blend_rows.contains(&y) {
                        for (v, n) in c.iter_mut().zip(new) {
                            *v = *v * (1.0 - f) + n * f;
                        }
                    }
                }
                filled[x as usize] = true;
            }
        }
        let mut out = Bitmap32::new(w, h, 0xff00_0000);
        for y in 0..h {
            for x in 0..w {
                let [r, g, b] =
                    canvas[(y * w + x) as usize].map(|v| v.round().clamp(0.0, 255.0) as u32);
                out.set(x, y, 0xff00_0000 | r << 16 | g << 8 | b);
            }
        }
        out
    }
}

pub fn draw_three_slice(dst: &mut Bitmap32, art: &InterfaceArt, sequence: i32, slice: &ThreeSlice) {
    let Some((bitmap, _)) = art.bitmap(sequence, 0, 0) else {
        return;
    };
    let (w, h) = (bitmap.width, bitmap.height);
    if w <= 0 || h <= 0 {
        return;
    }
    let mut painting = Bitmap32::new(w, h, 0xff00_0000);
    copy_bitmap(
        &mut painting,
        bitmap,
        art.palette(),
        0,
        0,
        Rect::new(0, 0, h, w),
        0,
    );
    let wide = slice.compose(&painting, dst.width);
    for y in 0..h.min(dst.height) {
        for x in 0..dst.width {
            dst.set(x, y, wide.get(x, y));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bar() -> ThreeSlice {
        ThreeSlice {
            left_end: 16,
            right_end: 612,
            middle: vec![
                SliceRun::new(16..612, false),
                SliceRun::new(540..607, false),
                SliceRun::new(16..612, true),
                SliceRun::new(540..607, true),
            ],
            overlap: 32,
            blend_rows: 4..58,
        }
    }

    #[test]
    fn the_pieces_overlap_and_stop_with_room_for_the_right_end() {
        let s = bar();
        assert_eq!(
            s.layout(640, 640),
            [Piece {
                at: 0,
                run: SliceRun::new(0..640, false),
                visible: 640
            }]
        );
        let at = |w| -> Vec<(i32, i32, bool, i32)> {
            s.layout(w, 640)
                .iter()
                .map(|p| (p.at, p.run.columns.start, p.run.mirrored, p.visible))
                .collect()
        };
        assert_eq!(
            at(1920),
            [
                (0, 0, false, 16),
                (16, 16, false, 596),
                (580, 540, false, 67),
                (615, 16, true, 596),
                (1179, 540, true, 67),
                (1214, 16, false, 596),
                (1778, 540, false, 67),
                (1813, 16, true, 1892 - 1813),
                (1860, 580, false, 60),
            ]
        );
        for w in [700, 1280, 2560] {
            let l = s.layout(w, 640);
            let last = l.last().unwrap();
            assert_eq!(last.at + last.visible, w);
            assert_eq!(last.run.columns, 580..640);
            let before = &l[l.len() - 2];
            assert_eq!(before.at + before.visible, w - 28, "{w}: {l:?}");
        }
    }

    #[test]
    fn a_join_cross_fades_the_leather_and_cuts_the_beads_at_its_middle() {
        let mut p = Bitmap32::new(10, 8, 0);
        for x in 0..10 {
            for y in 0..8 {
                let v = (x * 20) as u32;
                p.set(x, y, 0xff00_0000 | v << 16 | v << 8 | v);
            }
        }
        let s = ThreeSlice {
            left_end: 1,
            right_end: 9,
            middle: vec![SliceRun::new(1..9, false), SliceRun::new(1..9, true)],
            overlap: 4,
            blend_rows: 2..6,
        };
        let l = s.layout(20, 10);
        let ats: Vec<i32> = l.iter().map(|p| p.at).collect();
        assert_eq!(ats, [0, 1, 5, 9, 13, 15]);
        let out = s.compose(&p, 20);
        let grey = |x, y| out.get(x, y) & 0xff;
        assert_eq!(
            (1..5).map(|x| grey(x, 3)).collect::<Vec<_>>(),
            [20, 40, 60, 80]
        );
        let want = |a: f32, b: f32, f: f32| (a * (1.0 - f) + b * f).round() as u32;
        for (i, x) in (5..9).enumerate() {
            let (a, b) = ((100 + 20 * i) as f32, (160 - 20 * i) as f32);
            let f = (i as f32 + 0.5) / 4.0;
            assert_eq!(grey(x, 3), want(a, b, f), "x {x}");
            let bead = if f >= 0.5 { b } else { a } as u32;
            assert_eq!(grey(x, 0), bead, "x {x}");
            assert_eq!(grey(x, 7), bead, "x {x}");
        }
        assert_eq!(grey(19, 3), 180);
        assert_eq!(grey(19, 0), 180);
    }

    #[test]
    fn no_overlap_tiles_with_hard_joins() {
        let s = ThreeSlice {
            left_end: 2,
            right_end: 8,
            middle: vec![SliceRun::new(2..5, false), SliceRun::new(6..8, true)],
            overlap: 0,
            blend_rows: 0..0,
        };
        let mut p = Bitmap32::new(10, 1, 0);
        for x in 0..10 {
            p.set(x, 0, x as u32);
        }
        let out = s.compose(&p, 16);
        let cols: Vec<u32> = (0..16).map(|x| out.get(x, 0) & 0xff).collect();
        assert_eq!(cols, [0, 1, 2, 3, 4, 7, 6, 2, 3, 4, 7, 6, 2, 3, 8, 9]);
    }
}
