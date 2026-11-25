use mandrake_rec::saved_game::{
    CONSOLE_LINES, ConsoleLine, SNAPSHOT_FORMAT_32_BIT, SNAPSHOT_HEIGHT, SNAPSHOT_WIDTH,
    SavedGameHeader, Snapshot,
};
use mandrake_rec::writer::Recorder;
use tracing::{info, warn};

use super::ClientApp;

pub(crate) enum SnapshotUse {
    Prompt,
    Save(PendingSave),
}

pub(crate) struct PendingSave {
    pub(crate) name: Vec<u8>,
    line: &'static str,
    recorder: Recorder,
    end_tick: u32,
    header: SavedGameHeader,
}

pub(crate) enum SnapshotTaken {
    Prompt(Snapshot),
    Save {
        name: Vec<u8>,
        data: Vec<u8>,
        line: &'static str,
    },
}

pub(crate) fn snapshot_of_frame(
    size: (u32, u32),
    rgba: &[u8],
    rect: (u32, u32, u32, u32),
) -> Snapshot {
    let (left, top, width, height) = rect;
    let (left, width) = if u64::from(width) * 3 > u64::from(height) * 4 {
        let cut = height * 4 / 3;
        (left + (width - cut) / 2, cut)
    } else {
        (left, width)
    };
    let (sw, sh) = (SNAPSHOT_WIDTH, SNAPSHOT_HEIGHT);
    let mut out = Vec::with_capacity((sw * sh * 4) as usize);
    for y in 0..sh {
        let y0 = top + y * height / sh;
        let y1 = (top + (y + 1) * height / sh).max(y0 + 1).min(size.1);
        for x in 0..sw {
            let x0 = left + x * width / sw;
            let x1 = (left + (x + 1) * width / sw).max(x0 + 1).min(size.0);
            let mut sum = [0u64; 3];
            let mut n = 0u64;
            for py in y0..y1 {
                for px in x0..x1 {
                    let p = &rgba[((py * size.0 + px) * 4) as usize..][..3];
                    for (s, &c) in sum.iter_mut().zip(p) {
                        *s += u64::from(c);
                    }
                    n += 1;
                }
            }
            let n = n.max(1);
            out.extend(sum.iter().map(|&s| ((s + n / 2) / n) as u8));
            out.push(0xff);
        }
    }
    Snapshot {
        width: sw,
        height: sh,
        rgba: out,
    }
}

fn black() -> Snapshot {
    Snapshot {
        width: SNAPSHOT_WIDTH,
        height: SNAPSHOT_HEIGHT,
        rgba: [0, 0, 0, 0xff].repeat((SNAPSHOT_WIDTH * SNAPSHOT_HEIGHT) as usize),
    }
}

impl ClientApp {
    fn recorder_now(&self) -> Option<Recorder> {
        let recording = self.recording.as_ref()?;
        let commands = self.sim.recorded_commands.as_deref().unwrap_or_default();
        Some(recording.recorder_with(commands))
    }

    fn saved_game_header(&self) -> SavedGameHeader {
        let observer = self
            .camera_control
            .observer(&self.observer, self.camera_frame());
        let location = observer.location();
        let cached = self
            .hud
            .as_ref()
            .map(|hud| hud.console_lines.cached().to_vec())
            .unwrap_or_default();
        let mut console: Vec<ConsoleLine> = cached
            .iter()
            .map(|l| ConsoleLine {
                kind: l.player,
                text: l.text.clone(),
            })
            .collect();
        console.resize(
            CONSOLE_LINES,
            ConsoleLine {
                kind: 0,
                text: Vec::new(),
            },
        );
        SavedGameHeader {
            snapshot_width: SNAPSHOT_WIDTH as i16,
            snapshot_height: SNAPSHOT_HEIGHT as i16,
            snapshot_format: SNAPSHOT_FORMAT_32_BIT,
            observer_x: location.point.0,
            observer_y: location.point.1,
            observer_facing: location.facing as i16,
            observer_zoom: observer.zoom_factor(),
            console,
            console_count: cached.len() as i16,
        }
    }

    pub(crate) fn want_snapshot(&mut self) {
        self.snapshot_wanted = Some(SnapshotUse::Prompt);
    }

    pub(crate) fn saved_game_now(&self) -> Option<Vec<u8>> {
        let recorder = self.recorder_now()?;
        let snapshot = self.snapshot.clone().unwrap_or_else(black);
        let header = self.saved_game_header().to_bytes();
        Some(recorder.saved_game(self.sim.current_tick, &header, &snapshot.to_bytes()))
    }

    pub(crate) fn save_named(&mut self, name: Vec<u8>, line: &'static str) {
        let Some(recorder) = self.recorder_now() else {
            warn!("{line:?} not made: this map isn't recorded");
            return;
        };
        let save = PendingSave {
            name,
            line,
            recorder,
            end_tick: self.sim.current_tick,
            header: self.saved_game_header(),
        };
        self.snapshot_wanted = Some(SnapshotUse::Save(save));
    }

    pub(crate) fn snapshot_drawn(&mut self, snapshot: Option<Snapshot>) {
        let Some(wanted) = self.snapshot_wanted.take() else {
            return;
        };
        let snapshot = snapshot.unwrap_or_else(|| {
            warn!("snapshot: the frame couldn't be read: a black one");
            black()
        });
        info!("snapshot: drawn at tick {}", self.sim.current_tick);
        self.snapshot_taken = Some(match wanted {
            SnapshotUse::Prompt => {
                self.snapshot = Some(snapshot.clone());
                SnapshotTaken::Prompt(snapshot)
            }
            SnapshotUse::Save(save) => SnapshotTaken::Save {
                data: save.recorder.saved_game(
                    save.end_tick,
                    &save.header.to_bytes(),
                    &snapshot.to_bytes(),
                ),
                name: save.name,
                line: save.line,
            },
        });
    }

    pub(crate) fn take_snapshot(&mut self) -> Option<SnapshotTaken> {
        self.snapshot_taken.take()
    }

    pub(crate) fn world_rect(&self, window: (u32, u32)) -> (u32, u32, u32, u32) {
        let (logical_w, logical_h) = self.ui.logical;
        let ((_, height), (_, top)) = crate::interface_setup::world_frame(logical_w, logical_h);
        let scale = |v: i32| -> u32 {
            (i64::from(v.max(0)) * i64::from(window.1) / i64::from(logical_h.max(1))) as u32
        };
        let (top, height) = (scale(top), scale(height));
        (
            0,
            top.min(window.1),
            window.0,
            height.min(window.1 - top.min(window.1)).max(1),
        )
    }

    pub(crate) fn print_line(&mut self, line: &str) {
        let now = crate::media::local_time(self.sim.current_tick);
        if let Some(hud) = self.hud.as_mut() {
            hud.print_game_sound_line(line, now);
        }
    }

    pub(crate) fn set_console_cache(&mut self, lines: Vec<crate::console::ConsoleLine>) {
        if let Some(hud) = self.hud.as_mut() {
            hud.console_lines.set_cached(lines);
        }
    }

    pub(crate) fn console_cache(&self) -> Vec<crate::console::ConsoleLine> {
        self.hud
            .as_ref()
            .map(|hud| hud.console_lines.cached().to_vec())
            .unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_snapshot_is_the_views_middle_four_by_three() {
        let (w, h) = (1280u32, 720u32);
        let mut rgba = vec![0u8; (w * h * 4) as usize];
        for y in 0..h {
            for x in 0..w {
                let p = ((y * w + x) * 4) as usize;
                let c = if x < 160 {
                    [255, 0, 0]
                } else if x < 1120 {
                    [0, 255, 0]
                } else {
                    [0, 0, 255]
                };
                rgba[p..p + 3].copy_from_slice(&c);
                rgba[p + 3] = 255;
            }
        }
        let s = snapshot_of_frame((w, h), &rgba, (0, 0, w, h));
        assert_eq!((s.width, s.height), (160, 120));
        assert!(s.rgba.chunks(4).all(|p| p == [0, 255, 0, 255]));
        let (top, height) = (40, 640);
        let s = snapshot_of_frame((w, h), &rgba, (0, top, w, height));
        assert!(s.rgba.chunks(4).all(|p| p == [0, 255, 0, 255]));
    }

    #[test]
    fn a_black_snapshot_is_whole() {
        let b = black();
        assert_eq!(b.rgba.len(), 160 * 120 * 4);
        assert_eq!(b.to_bytes()[..4], [0xff, 0, 0, 0]);
    }
}
