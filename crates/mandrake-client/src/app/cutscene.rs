#[cfg(not(target_arch = "wasm32"))]
use super::cutscene_hd::{self, HdGpu, HdShown, HdStream};
use crate::audio_out::AudioLink;
use crate::clock::Instant;
use mandrake_audio::SoundBuffer;
use mandrake_dialogs::canvas::CanvasFit;
use mandrake_render::{InterfaceScaler, OverlayDraw, OverlayImage, OverlayLayer, OverlayRenderer};
use mandrake_view::interface_picture::InterfaceFilter;
use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::mpsc::{Receiver, SyncSender, TryRecvError, sync_channel};
use std::time::Duration;
use tracing::{info, warn};
use winit::event::{ElementState, WindowEvent};

pub(crate) const FOLDER: &str = "cutscenes";
pub(crate) const EXTENSIONS: [&str; 3] = ["mov", "smk", "mpg"];
const CHANNEL: usize = 7;
const LEAD_FRAMES: usize = 8;

pub(crate) fn candidates(game_dir: &Path, name: &str) -> Vec<PathBuf> {
    let stem = name.rsplit_once('.').map_or(name, |(stem, _)| stem);
    EXTENSIONS
        .iter()
        .map(|ext| game_dir.join(FOLDER).join(format!("{stem}.{ext}")))
        .filter(|path| crate::game_files::is_file(path))
        .collect()
}

pub(crate) fn found(game_dir: &Path, name: &str) -> bool {
    let found = !candidates(game_dir, name).is_empty();
    if !found {
        warn!("Warning: Unable to find cutscene '{name}'.");
    }
    found
}

pub(crate) fn is_skip(event: &WindowEvent) -> bool {
    match event {
        WindowEvent::KeyboardInput { event, .. } => event.state == ElementState::Pressed,
        WindowEvent::MouseInput { state, .. } => *state == ElementState::Pressed,
        _ => false,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Schedule {
    pub frame_micros: u32,
    pub frame_count: u32,
}

impl Schedule {
    pub(crate) fn frame_at(&self, elapsed: Duration) -> Option<u32> {
        let frame = elapsed.as_micros() / u128::from(self.frame_micros.max(1));
        (frame < u128::from(self.frame_count)).then_some(frame as u32)
    }
}

pub(crate) fn display_size(header: &mandrake_smacker::Header) -> (u32, u32) {
    let doubled = header.y_doubled || header.y_interlaced;
    (header.width, header.height * if doubled { 2 } else { 1 })
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum HdLookup {
    Off,
    File(Option<PathBuf>),
}

impl HdLookup {
    #[cfg(not(target_arch = "wasm32"))]
    pub(crate) fn of_smk(smk: &Path) -> Self {
        Self::File(cutscene_hd::hd_path(smk))
    }

    #[cfg(not(target_arch = "wasm32"))]
    pub(crate) fn tfl_movie(id: mandrake_common::FourCC, on: bool) -> Self {
        if on {
            Self::File(cutscene_hd::tfl_hd_path(id))
        } else {
            Self::Off
        }
    }

    #[cfg(target_arch = "wasm32")]
    pub(crate) fn of_smk(_smk: &Path) -> Self {
        Self::File(None)
    }

    #[cfg(target_arch = "wasm32")]
    pub(crate) fn tfl_movie(_id: mandrake_common::FourCC, _on: bool) -> Self {
        Self::Off
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn hd_display_size(hd: (u32, u32), stored: (u32, u32), display: (u32, u32)) -> (u32, u32) {
    let scale = |h: u32, s: u32, d: u32| {
        if s == 0 {
            h
        } else {
            (u64::from(h) * u64::from(d) / u64::from(s)) as u32
        }
    };
    (
        scale(hd.0, stored.0, display.0),
        scale(hd.1, stored.1, display.1),
    )
}

struct Decoded {
    index: u32,
    rgba: Vec<u8>,
    sound: Vec<Arc<SoundBuffer>>,
}

fn sound_of(chunk: &mandrake_smacker::AudioChunk, device_rate: u32) -> Option<Arc<SoundBuffer>> {
    let samples = chunk.samples.to_i16();
    let buffer =
        SoundBuffer::from_pcm16_interleaved(u16::from(chunk.channels), chunk.sample_rate, samples)?;
    let buffer = if chunk.sample_rate == device_rate || device_rate == 0 {
        buffer
    } else {
        mandrake_audio::resample_buffer(&buffer, device_rate)
    };
    Some(Arc::new(buffer))
}

fn decode(data: Vec<u8>, device_rate: Option<u32>, out: SyncSender<Decoded>) {
    let Ok(smk) = mandrake_smacker::Smacker::open(&data) else {
        return;
    };
    let mut decoder = smk.decoder();
    loop {
        let frame = match decoder.next_frame() {
            Ok(Some(frame)) => frame,
            Ok(None) => return,
            Err(e) => {
                warn!("cutscene: frame {}: {e}", decoder.position());
                return;
            }
        };
        let sound = device_rate.map_or_else(Vec::new, |rate| {
            frame
                .audio
                .iter()
                .take(1)
                .filter_map(|chunk| sound_of(chunk, rate))
                .collect()
        });
        let decoded = Decoded {
            index: frame.index,
            rgba: decoder.rgba(),
            sound,
        };
        if out.send(decoded).is_err() {
            return;
        }
    }
}

#[derive(Debug, Default, Clone, Copy)]
struct Cost {
    frames: u32,
    total: Duration,
    max: Duration,
    last: Duration,
}

struct FilmGpu {
    overlay: OverlayRenderer,
    scaler: InterfaceScaler,
    image: OverlayImage,
    drawn: Option<u32>,
}

#[cfg(not(target_arch = "wasm32"))]
struct Hd {
    stream: HdStream,
    frame: Option<HdShown>,
    fresh: bool,
    gpu: Option<HdGpu>,
    late: u32,
    first_late: Option<u32>,
    uploads: u32,
    upload_total: Duration,
    upload_max: Duration,
    opened: Instant,
}

#[cfg(not(target_arch = "wasm32"))]
const HD_FIRST_FRAME_WAIT: Duration = Duration::from_secs(3);

pub(crate) struct Cutscene {
    path: PathBuf,
    schedule: Schedule,
    size: (u32, u32),
    stored: (u32, u32),
    frames: Receiver<Decoded>,
    ahead: VecDeque<Decoded>,
    showing: Option<(u32, Vec<u8>)>,
    started: Option<Instant>,
    decoder_done: bool,
    over: bool,
    link: Option<AudioLink>,
    gpu: Option<FilmGpu>,
    cost: Cost,
    #[cfg(not(target_arch = "wasm32"))]
    hd: Option<Hd>,
}

impl Cutscene {
    pub(crate) fn open(
        game_dir: &Path,
        name: &str,
        link: Option<AudioLink>,
        hd: bool,
    ) -> Option<Self> {
        let mut warned = false;
        for path in candidates(game_dir, name) {
            match Self::start(&path, link.clone(), hd) {
                Ok(cutscene) => return Some(cutscene),
                Err(e) => {
                    if !warned {
                        let file = path.file_name().map(|f| f.to_string_lossy().into_owned());
                        warn!(
                            "Warning: Failed to play cutscene '{}' using mandrake-smacker ({e}).",
                            file.unwrap_or_default()
                        );
                        warned = true;
                    }
                }
            }
        }
        None
    }

    pub(crate) fn from_data(
        name: &str,
        data: Vec<u8>,
        link: Option<AudioLink>,
        hd: HdLookup,
    ) -> Option<Self> {
        match Self::start_data(Path::new(name), data, link, hd) {
            Ok(cutscene) => Some(cutscene),
            Err(e) => {
                warn!("Warning: Failed to play cutscene '{name}' using mandrake-smacker ({e}).");
                None
            }
        }
    }

    fn start(path: &Path, link: Option<AudioLink>, hd: bool) -> Result<Self, String> {
        let data = crate::game_files::read(path).map_err(|e| e.to_string())?;
        let hd = if hd {
            HdLookup::of_smk(path)
        } else {
            HdLookup::Off
        };
        Self::start_data(path, data, link, hd)
    }

    fn start_data(
        path: &Path,
        data: Vec<u8>,
        link: Option<AudioLink>,
        hd: HdLookup,
    ) -> Result<Self, String> {
        let header = mandrake_smacker::Smacker::open(&data)
            .map_err(|e| e.to_string())?
            .header()
            .clone();
        let device_rate = link.as_ref().map(|link| link.audio.device_rate());
        let (send, frames) = sync_channel(LEAD_FRAMES);
        std::thread::Builder::new()
            .name("cutscene".to_owned())
            .spawn(move || decode(data, device_rate, send))
            .map_err(|e| e.to_string())?;
        if let Some(link) = &link {
            link.audio.channel_stop(CHANNEL);
            link.audio.channel_set_gains(CHANNEL, 1.0, 1.0);
        }
        info!(
            "cutscene: {} ({}x{}, {} frames at {:.2} fps{})",
            path.display(),
            header.width,
            header.height,
            header.frame_count,
            header.fps(),
            if header.y_doubled || header.y_interlaced {
                ", rows doubled"
            } else {
                ""
            }
        );
        #[cfg(target_arch = "wasm32")]
        let _ = hd;
        Ok(Self {
            #[cfg(not(target_arch = "wasm32"))]
            hd: match hd {
                HdLookup::Off => None,
                HdLookup::File(hd_path) => Self::start_hd(hd_path, header.frame_count),
            },
            path: path.to_path_buf(),
            schedule: Schedule {
                frame_micros: header.frame_micros,
                frame_count: header.frame_count,
            },
            size: display_size(&header),
            stored: (header.width, header.height),
            frames,
            ahead: VecDeque::new(),
            showing: None,
            started: None,
            decoder_done: false,
            over: false,
            link,
            gpu: None,
            cost: Cost::default(),
        })
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn start_hd(hd_path: Option<PathBuf>, frame_count: u32) -> Option<Hd> {
        let (data, ivf) = match cutscene_hd::choose(hd_path.as_deref(), frame_count) {
            cutscene_hd::Choice::Hd { data, ivf } => (data, ivf),
            cutscene_hd::Choice::Smk(why) => {
                match why {
                    Some(why) => cutscene_hd::warn_fallback(&why),
                    None => info!(
                        "cutscene-hd: no HD film at {}",
                        hd_path
                            .map_or_else(|| "(no data folder)".into(), |p| p.display().to_string())
                    ),
                }
                return None;
            }
        };
        let (w, h) = (ivf.header.width, ivf.header.height);
        let stream = match HdStream::start(data, ivf) {
            Ok(stream) => stream,
            Err(e) => {
                cutscene_hd::warn_fallback(&e);
                return None;
            }
        };
        info!(
            "cutscene-hd: {} ({w}x{h}, {frame_count} frames) in place of the .smk's pictures",
            hd_path.map(|p| p.display().to_string()).unwrap_or_default()
        );
        Some(Hd {
            stream,
            frame: None,
            fresh: false,
            gpu: None,
            late: 0,
            first_late: None,
            uploads: 0,
            upload_total: Duration::ZERO,
            upload_max: Duration::ZERO,
            opened: Instant::now(),
        })
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn drop_hd(&mut self, why: &str) {
        cutscene_hd::warn_fallback(why);
        self.hd = None;
        if let Some(gpu) = &mut self.gpu {
            gpu.drawn = None;
        }
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn fill_hd(&mut self) -> bool {
        let Some(hd) = &mut self.hd else {
            return true;
        };
        if let Err(e) = hd.stream.fill() {
            self.drop_hd(&e);
            return true;
        }
        if self.started.is_some() || hd.stream.has_first() {
            return true;
        }
        if hd.stream.is_drained() {
            self.drop_hd("the decoder gave no frames");
            return true;
        }
        if hd.opened.elapsed() > HD_FIRST_FRAME_WAIT {
            self.drop_hd("its first frame took too long");
            return true;
        }
        false
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn show_hd(&mut self, due: u32) {
        let Some(hd) = &mut self.hd else {
            return;
        };
        if let Some(frame) = hd.stream.take_due(due) {
            if frame.index < due {
                hd.late += 1;
                hd.first_late.get_or_insert(frame.index);
            }
            if let Some(old) = hd.frame.replace(frame) {
                hd.stream.release(old);
            }
            hd.fresh = true;
        } else if hd.stream.is_drained() && hd.frame.as_ref().is_none_or(|f| f.index < due) {
            self.drop_hd("the decoder stopped before the film's end");
        }
    }

    pub(crate) fn is_over(&self) -> bool {
        self.over
    }

    pub(crate) fn skip(&mut self) {
        if !self.over {
            info!(
                "cutscene: skipped at frame {:?}",
                self.showing.as_ref().map(|s| s.0)
            );
        }
        self.finish();
    }

    fn finish(&mut self) {
        if self.over {
            return;
        }
        self.over = true;
        if let Some(link) = &self.link {
            link.audio.channel_stop(CHANNEL);
        }
        if self.cost.frames > 0 {
            let mean = self.cost.total / self.cost.frames;
            info!(
                "cutscene: {} over; main-thread cost per frame {:.3} ms mean, {:.3} ms max ({:.1}% / {:.1}% of 4 ms)",
                self.path.display(),
                mean.as_secs_f64() * 1e3,
                self.cost.max.as_secs_f64() * 1e3,
                mean.as_secs_f64() / 0.004 * 100.0,
                self.cost.max.as_secs_f64() / 0.004 * 100.0,
            );
        }
        #[cfg(not(target_arch = "wasm32"))]
        if let Some(hd) = &self.hd
            && hd.uploads > 0
        {
            info!(
                "cutscene-hd: {} frames converted, {:.3} ms mean, {:.3} ms max on the main thread; {} shown late (the first: {:?})",
                hd.uploads,
                hd.upload_total.as_secs_f64() * 1e3 / f64::from(hd.uploads),
                hd.upload_max.as_secs_f64() * 1e3,
                hd.late,
                hd.first_late,
            );
        }
    }

    fn fill(&mut self) {
        while !self.decoder_done && self.ahead.len() < LEAD_FRAMES {
            match self.frames.try_recv() {
                Ok(frame) => {
                    if let Some(link) = &self.link {
                        for sound in &frame.sound {
                            link.audio.channel_queue_resampled(CHANNEL, sound.clone());
                        }
                    }
                    self.ahead.push_back(frame);
                }
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => self.decoder_done = true,
            }
        }
    }

    pub(crate) fn update(&mut self, now: Instant) {
        if self.over {
            return;
        }
        let started = Instant::now();
        #[cfg(not(target_arch = "wasm32"))]
        if !self.fill_hd() {
            return;
        }
        self.fill();
        let start = match self.started {
            Some(start) => start,
            None if !self.ahead.is_empty() => *self.started.insert(now),
            None if self.decoder_done => {
                warn!("cutscene: {} has no frames", self.path.display());
                self.finish();
                return;
            }
            None => return,
        };
        let Some(due) = self.schedule.frame_at(now.saturating_duration_since(start)) else {
            self.finish();
            return;
        };
        while let Some(next) = self.ahead.front() {
            if next.index > due {
                break;
            }
            let frame = self.ahead.pop_front().expect("front was there");
            self.showing = Some((frame.index, frame.rgba));
        }
        if self.decoder_done
            && self.ahead.is_empty()
            && self
                .showing
                .as_ref()
                .is_some_and(|s| s.0 + 1 < self.schedule.frame_count && due > s.0)
        {
            warn!(
                "cutscene: {} ended early after frame {} of {}",
                self.path.display(),
                self.showing.as_ref().map_or(0, |s| s.0 + 1),
                self.schedule.frame_count,
            );
            self.finish();
            return;
        }
        #[cfg(not(target_arch = "wasm32"))]
        self.show_hd(due);
        self.fill();
        self.note_cost(started.elapsed());
    }

    fn note_cost(&mut self, spent: Duration) {
        self.cost.frames += 1;
        self.cost.total += spent;
        self.cost.max = self.cost.max.max(spent);
        self.cost.last = spent;
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn draw(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        target: &wgpu::TextureView,
        format: wgpu::TextureFormat,
        window: (u32, u32),
        filter: InterfaceFilter,
    ) {
        #[cfg(not(target_arch = "wasm32"))]
        if let Some(hd) = &mut self.hd {
            hd.stream.attach(device, queue);
        }
        let Some((index, rgba)) = &self.showing else {
            return;
        };
        let started = Instant::now();
        let (w, h) = self.stored;
        let gpu = self.gpu.get_or_insert_with(|| {
            let overlay = OverlayRenderer::new(device, format);
            let image = overlay.create_image(device, queue, w, h, rgba);
            FilmGpu {
                overlay,
                scaler: InterfaceScaler::new(device, format),
                image,
                drawn: None,
            }
        });
        #[cfg_attr(target_arch = "wasm32", allow(unused_mut))]
        let mut size = self.size;
        #[cfg_attr(target_arch = "wasm32", allow(unused_mut))]
        let mut hd_shows = false;
        #[cfg(not(target_arch = "wasm32"))]
        if let Some(hd) = &mut self.hd
            && let Some(frame) = &hd.frame
        {
            size = hd_display_size(frame.size, self.stored, self.size);
            hd_shows = true;
            if hd.fresh
                && let Some(layout) = hd.stream.layout()
            {
                let t = Instant::now();
                let hd_gpu = hd
                    .gpu
                    .get_or_insert_with(|| HdGpu::new(device, gpu.scaler.format, layout));
                let layer = gpu.scaler.layer_view(device, frame.size);
                hd_gpu.draw(encoder, layer, frame);
                hd.fresh = false;
                gpu.drawn = None;
                let spent = t.elapsed();
                hd.uploads += 1;
                hd.upload_total += spent;
                hd.upload_max = hd.upload_max.max(spent);
            }
        }
        if !hd_shows && gpu.drawn != Some(*index) {
            OverlayRenderer::update_image(queue, &gpu.image, rgba);
            let mut draw = OverlayDraw::at(OverlayLayer::Dialog, &gpu.image, 0, 0, 0xff);
            draw.dst = [0, 0, self.size.0 as i32, self.size.1 as i32];
            let layer = gpu.scaler.layer_view(device, self.size);
            gpu.overlay
                .encode_cleared(device, queue, encoder, layer, self.size, &[draw]);
            gpu.drawn = Some(*index);
        }
        let fit = CanvasFit::new(window, size);
        gpu.scaler.composite_into(
            queue,
            encoder,
            target,
            Some([fit.origin.0, fit.origin.1, fit.size.0, fit.size.1]),
            window,
            filter,
        );
        let spent = started.elapsed();
        self.cost.total += spent;
        self.cost.max = self.cost.max.max(self.cost.last + spent);
    }
}

impl Drop for Cutscene {
    fn drop(&mut self) {
        self.finish();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use winit::event::{DeviceId, MouseButton};

    const FILM_FPS15: Schedule = Schedule {
        frame_micros: 66_660,
        frame_count: 892,
    };

    #[test]
    fn frames_show_at_their_time_and_the_film_ends_after_the_last() {
        let s = FILM_FPS15;
        let at = |ms: u64| s.frame_at(Duration::from_millis(ms));
        assert_eq!(at(0), Some(0));
        assert_eq!(at(66), Some(0));
        assert_eq!(at(67), Some(1));
        assert_eq!(at(1000), Some(15));
        assert_eq!(at(59_390), Some(890));
        assert_eq!(at(59_400), Some(891));
        assert_eq!(at(59_460), Some(891));
        assert_eq!(at(59_461), None);
        let ten = Schedule {
            frame_micros: 100_000,
            frame_count: 3,
        };
        assert_eq!(ten.frame_at(Duration::from_millis(299)), Some(2));
        assert_eq!(ten.frame_at(Duration::from_millis(300)), None);
    }

    #[test]
    fn a_press_of_a_button_skips_and_a_release_or_a_move_doesnt() {
        let device_id = DeviceId::dummy();
        let mouse = |state| WindowEvent::MouseInput {
            device_id,
            state,
            button: MouseButton::Right,
        };
        assert!(is_skip(&mouse(ElementState::Pressed)));
        assert!(!is_skip(&mouse(ElementState::Released)));
        assert!(!is_skip(&WindowEvent::CursorMoved {
            device_id,
            position: winit::dpi::PhysicalPosition::new(1.0, 2.0),
        }));
        assert!(!is_skip(&WindowEvent::Focused(true)));
    }

    #[test]
    fn an_hd_frame_shows_in_the_films_shape() {
        assert_eq!(
            hd_display_size((2560, 1056), (640, 264), (640, 264)),
            (2560, 1056)
        );
        assert_eq!(
            hd_display_size((2560, 768), (640, 192), (640, 384)),
            (2560, 1536)
        );
        assert_eq!(hd_display_size((8, 8), (0, 0), (0, 0)), (8, 8));
    }

    #[test]
    fn the_name_takes_each_extension_in_the_engines_order() {
        let dir = std::env::temp_dir().join(format!("m2-cutscene-{}", std::process::id()));
        let folder = dir.join(FOLDER);
        std::fs::create_dir_all(&folder).unwrap();
        assert!(candidates(&dir, "prologue.smk").is_empty());
        assert!(!found(&dir, "prologue.smk"));
        std::fs::write(folder.join("prologue.mpg"), b"").unwrap();
        std::fs::write(folder.join("prologue.smk"), b"").unwrap();
        std::fs::write(folder.join("prologue.mov"), b"").unwrap();
        let names: Vec<_> = candidates(&dir, "prologue.smk")
            .iter()
            .map(|p| p.file_name().unwrap().to_string_lossy().into_owned())
            .collect();
        assert_eq!(names, ["prologue.mov", "prologue.smk", "prologue.mpg"]);
        assert!(found(&dir, "prologue"));
        assert!(Cutscene::open(&dir, "prologue.smk", None, false).is_none());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn early_decoder_stop_ends_after_last_decoded_frame() {
        let (send, frames) = sync_channel(4);
        send.send(Decoded {
            index: 0,
            rgba: vec![0; 4],
            sound: Vec::new(),
        })
        .unwrap();
        send.send(Decoded {
            index: 1,
            rgba: vec![0; 4],
            sound: Vec::new(),
        })
        .unwrap();
        drop(send);

        let mut film = Cutscene {
            path: PathBuf::from("damaged.smk"),
            schedule: Schedule {
                frame_count: 100,
                ..FILM_FPS15
            },
            size: (1, 1),
            stored: (1, 1),
            frames,
            ahead: VecDeque::new(),
            showing: None,
            started: None,
            decoder_done: false,
            over: false,
            link: None,
            gpu: None,
            cost: Cost::default(),
            #[cfg(not(target_arch = "wasm32"))]
            hd: None,
        };
        let t0 = Instant::now();
        film.update(t0);
        assert!(!film.is_over());
        assert_eq!(film.showing.as_ref().map(|s| s.0), Some(0));

        film.update(t0 + Duration::from_millis(70));
        assert!(!film.is_over());
        assert_eq!(film.showing.as_ref().map(|s| s.0), Some(1));

        film.update(t0 + Duration::from_millis(140));
        assert!(film.is_over());
    }
}
