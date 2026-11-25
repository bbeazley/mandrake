use crate::collection::CollectionSequence;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct SequenceEvents {
    pub frame_changed: bool,
    pub key_frame: bool,
    pub wrapped: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SequencePlayback {
    pub sequence: usize,
    pub frame: usize,
    pub counter: i32,
    pub ticks_per_frame: i32,
    pub reverse: bool,
}

pub const NORMAL_SPEED: i32 = 0x100;

impl SequencePlayback {
    pub fn start(seq: &CollectionSequence, speed: i32, start_at_loop_frame: bool) -> Self {
        let ticks = ((seq.ticks_per_frame as i32 * speed) >> 8) as i8 as i32;
        let frame = if start_at_loop_frame {
            seq.loop_frame.max(0) as usize
        } else {
            0
        };
        Self {
            sequence: seq.index,
            frame,
            counter: 0,
            ticks_per_frame: if ticks <= 0 { 1 } else { ticks },
            reverse: false,
        }
    }

    pub fn switch_to(&mut self, seq: &CollectionSequence) {
        if self.sequence != seq.index {
            *self = Self::start(seq, NORMAL_SPEED, false);
        }
    }

    pub fn tick(&mut self, seq: &CollectionSequence) -> SequenceEvents {
        let mut events = SequenceEvents::default();
        let frames = seq.frames_per_view as i32;
        if self.ticks_per_frame <= 0 || frames <= 0 {
            return events;
        }
        let mut counter = self.counter;
        if counter >= self.ticks_per_frame {
            counter = 1;
        } else {
            counter += 1;
        }
        let advance = self.ticks_per_frame <= counter;
        self.counter = counter;
        if !advance {
            return events;
        }
        let mut frame = self.frame as i32;
        if self.reverse {
            frame -= 1;
            if frame < 0 {
                frame = frames - 1;
                events.wrapped = true;
            }
        } else {
            frame += 1;
            if frame >= frames {
                frame = seq.loop_frame as i32;
                events.wrapped = true;
            }
        }
        events.frame_changed = true;
        events.key_frame = frame == seq.key_frame as i32;
        self.frame = frame.clamp(0, frames - 1) as usize;
        events
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn seq(frames: u16, ticks: i16, key: i16, loop_frame: i16) -> CollectionSequence {
        CollectionSequence {
            index: 3,
            name: "walk".into(),
            flags: 0,
            pixels_to_world: 1,
            num_views: 8,
            frames_per_view: frames,
            ticks_per_frame: ticks,
            key_frame: key,
            loop_frame,
            bitmap_indices: vec![0; frames as usize * 8],
            shadow_indices: vec![-1; frames as usize],
            bounds: Default::default(),
        }
    }

    #[test]
    fn frame_lasts_ticks_per_frame_ticks() {
        let s = seq(4, 2, -1, 0);
        let mut p = SequencePlayback::start(&s, NORMAL_SPEED, false);
        let mut frames = vec![p.frame];
        for _ in 0..8 {
            p.tick(&s);
            frames.push(p.frame);
        }
        assert_eq!(frames, vec![0, 0, 1, 1, 2, 2, 3, 3, 0]);
    }

    #[test]
    fn one_tick_per_frame_advances_every_tick() {
        let s = seq(3, 1, -1, 0);
        let mut p = SequencePlayback::start(&s, NORMAL_SPEED, false);
        let seen: Vec<usize> = (0..4)
            .map(|_| {
                p.tick(&s);
                p.frame
            })
            .collect();
        assert_eq!(seen, vec![1, 2, 0, 1]);
    }

    #[test]
    fn wrap_jumps_to_loop_frame_and_reports_it() {
        let s = seq(3, 1, -1, 1);
        let mut p = SequencePlayback::start(&s, NORMAL_SPEED, false);
        p.tick(&s);
        p.tick(&s);
        let e = p.tick(&s);
        assert!(e.wrapped && e.frame_changed);
        assert_eq!(p.frame, 1);
    }

    #[test]
    fn key_frame_event_fires_on_arrival() {
        let s = seq(6, 1, 5, 0);
        let mut p = SequencePlayback::start(&s, NORMAL_SPEED, false);
        let hits: Vec<bool> = (0..6)
            .map(|_| {
                let e = p.tick(&s);
                e.key_frame
            })
            .collect();
        assert_eq!(hits, vec![false, false, false, false, true, false]);
    }

    #[test]
    fn reverse_wraps_to_last_frame() {
        let s = seq(3, 1, -1, 0);
        let mut p = SequencePlayback::start(&s, NORMAL_SPEED, false);
        p.reverse = true;
        let e = p.tick(&s);
        assert!(e.wrapped);
        assert_eq!(p.frame, 2);
    }

    #[test]
    fn speed_scales_ticks_per_frame_with_floor_of_one() {
        let s = seq(4, 4, -1, 0);
        assert_eq!(SequencePlayback::start(&s, 0x80, false).ticks_per_frame, 2);
        assert_eq!(SequencePlayback::start(&s, 0, false).ticks_per_frame, 1);
    }

    #[test]
    fn switch_to_same_sequence_keeps_state() {
        let s = seq(4, 2, -1, 0);
        let mut p = SequencePlayback::start(&s, NORMAL_SPEED, false);
        p.tick(&s);
        p.tick(&s);
        let before = p;
        p.switch_to(&s);
        assert_eq!(p, before);
    }
}
