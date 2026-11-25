use crate::app::ClientApp;
use crate::app::shell_menu::Menu;
use crate::args::Args;
use crate::clock::Instant;
use mandrake_render::DrawInterface;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

pub(crate) const BUDGET_MS: f32 = 4.0;
const WARM_UP: Duration = Duration::from_secs(2);
const READBACK_SLOTS: usize = 8;

#[derive(Clone, Copy)]
pub(crate) enum Mark {
    Start,
    Category2,
    Media,
    SoftEdgeMask,
    Category4,
    OpaqueModels,
    DryLandscape,
    SpriteOccluders,
    Sprites,
    Interface,
}
const MARKS: usize = Mark::Interface as usize + 1;

const GPU_STAGES: [&str; MARKS - 1] = [
    "gpu category-2 models",
    "gpu media",
    "gpu category 4: soft edge mask",
    "gpu category 4: reflections",
    "gpu opaque models",
    "gpu landscape (dry)",
    "gpu sprite occluders (depth)",
    "gpu sprites",
    "gpu interface",
];

#[derive(Clone, Copy)]
pub(crate) enum Lap {
    Sim,
    Acquire,
    Billboards,
    LandscapeUpload,
    Prepare,
    Encode,
    Submit,
    GpuWait,
    Present,
}
const LAPS: usize = Lap::Present as usize + 1;
const LAP_NAMES: [&str; LAPS] = [
    "cpu sim and camera",
    "cpu acquire",
    "cpu build: billboards",
    "cpu build: landscape upload",
    "cpu build: region, models",
    "cpu build: encode",
    "cpu submit",
    "cpu wait for the GPU",
    "cpu present",
];

pub(crate) fn timer_features() -> wgpu::Features {
    wgpu::Features::TIMESTAMP_QUERY
        | wgpu::Features::TIMESTAMP_QUERY_INSIDE_ENCODERS
        | wgpu::Features::TIMESTAMP_QUERY_INSIDE_PASSES
}

pub(crate) fn uncapped_present_mode(modes: &[wgpu::PresentMode]) -> wgpu::PresentMode {
    [wgpu::PresentMode::Immediate, wgpu::PresentMode::Mailbox]
        .into_iter()
        .find(|mode| modes.contains(mode))
        .unwrap_or(wgpu::PresentMode::Fifo)
}

pub(crate) fn window_present_mode(
    vsync: bool,
    budget: bool,
    modes: &[wgpu::PresentMode],
) -> wgpu::PresentMode {
    if vsync && !budget {
        wgpu::PresentMode::AutoVsync
    } else {
        uncapped_present_mode(modes)
    }
}

pub(crate) fn present_mode_line(
    mode: wgpu::PresentMode,
    vsync: bool,
    budget: bool,
    source: &str,
    modes: &[wgpu::PresentMode],
) -> String {
    let asked = if budget {
        "vsync off, --frame-budget".to_owned()
    } else {
        format!("vsync {}, {source}", if vsync { "on" } else { "off" })
    };
    let fallback = if (budget || !vsync) && mode == wgpu::PresentMode::Fifo {
        "; the surface has no Immediate or Mailbox, so vsync stays on"
    } else {
        ""
    };
    format!("present mode: {mode:?} ({asked}) of {modes:?}{fallback}")
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct BudgetOptions {
    pub(crate) seconds: f32,
    pub(crate) from_tick: Option<u32>,
    pub(crate) scan_ticks: Option<u32>,
    pub(crate) wait: bool,
    pub(crate) follow: Option<u32>,
}

pub(crate) const DEFAULT_FOLLOW_UNIT: u32 = 10084;

pub(crate) struct Follow {
    pub(crate) unit: u32,
    archetype: Option<mandrake_common::FourCC>,
    point: Option<(i32, i32)>,
    fell: Option<u32>,
}

impl Follow {
    pub(crate) fn new(unit: u32) -> Self {
        Self {
            unit,
            archetype: None,
            point: None,
            fell: None,
        }
    }

    pub(crate) fn step(&mut self, sim: &mandrake_sim::Simulation) -> Option<(i32, i32)> {
        match sim.units.get(&self.unit) {
            Some(u) if u.is_alive() && self.fell.is_none() => {
                self.archetype.get_or_insert(u.archetype);
                self.point = Some(sim.world_to_command_point(u.position));
            }
            _ if self.point.is_some() && self.fell.is_none() => {
                self.fell = Some(sim.current_tick);
            }
            _ => {}
        }
        self.point
    }

    fn describe(&self) -> String {
        let kind = self
            .archetype
            .map_or_else(|| "never seen".to_string(), |a| a.to_string());
        let kind = if self.unit == DEFAULT_FOLLOW_UNIT {
            format!("{kind}, salvation_vic_new's benchmark dwarf")
        } else {
            kind
        };
        match self.fell {
            Some(tick) => format!("follow unit {} ({kind}), fell at tick {tick}", self.unit),
            None => format!("follow unit {} ({kind})", self.unit),
        }
    }
}

struct Readback {
    buffer: wgpu::Buffer,
    in_flight: bool,
    mapped: Arc<AtomicBool>,
}

pub(crate) struct GpuTimer {
    set: wgpu::QuerySet,
    resolve: wgpu::Buffer,
    slots: Vec<Readback>,
    next: usize,
    oldest: usize,
    period: f32,
}

const QUERY_BYTES: u64 = MARKS as u64 * wgpu::QUERY_SIZE as u64;

impl GpuTimer {
    pub(crate) fn new(device: &wgpu::Device, queue: &wgpu::Queue) -> Self {
        let set = device.create_query_set(&wgpu::QuerySetDescriptor {
            label: Some("Frame Budget Timestamps"),
            ty: wgpu::QueryType::Timestamp,
            count: MARKS as u32,
        });
        let resolve = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Frame Budget Resolve"),
            size: QUERY_BYTES,
            usage: wgpu::BufferUsages::QUERY_RESOLVE | wgpu::BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        });
        let slots = (0..READBACK_SLOTS)
            .map(|_| Readback {
                buffer: device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some("Frame Budget Readback"),
                    size: QUERY_BYTES,
                    usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
                    mapped_at_creation: false,
                }),
                in_flight: false,
                mapped: Arc::new(AtomicBool::new(false)),
            })
            .collect();
        Self {
            set,
            resolve,
            slots,
            next: 0,
            oldest: 0,
            period: queue.get_timestamp_period(),
        }
    }

    pub(crate) fn mark(&self, encoder: &mut wgpu::CommandEncoder, mark: Mark) {
        encoder.write_timestamp(&self.set, mark as u32);
    }

    pub(crate) fn mark_in_pass(&self, pass: &mut wgpu::RenderPass<'_>, mark: Mark) {
        pass.write_timestamp(&self.set, mark as u32);
    }

    pub(crate) fn encode_resolve(
        &mut self,
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
    ) {
        if self.slots[self.next].in_flight {
            let _ = device.poll(wgpu::PollType::wait_indefinitely());
            let slot = &mut self.slots[self.next];
            if slot.mapped.swap(false, Ordering::AcqRel) {
                slot.buffer.unmap();
            }
            slot.in_flight = false;
            if self.oldest == self.next {
                self.oldest = (self.oldest + 1) % self.slots.len();
            }
        }
        encoder.resolve_query_set(&self.set, 0..MARKS as u32, &self.resolve, 0);
        encoder.copy_buffer_to_buffer(
            &self.resolve,
            0,
            &self.slots[self.next].buffer,
            0,
            QUERY_BYTES,
        );
    }

    fn after_submit(&mut self) {
        let slot = &mut self.slots[self.next];
        slot.in_flight = true;
        slot.mapped.store(false, Ordering::Release);
        let mapped = slot.mapped.clone();
        slot.buffer
            .slice(..)
            .map_async(wgpu::MapMode::Read, move |result| {
                mapped.store(result.is_ok(), Ordering::Release);
            });
        self.next = (self.next + 1) % self.slots.len();
    }

    fn collect(&mut self, device: &wgpu::Device, mut each: impl FnMut([f32; MARKS - 1])) {
        let _ = device.poll(wgpu::PollType::Poll);
        loop {
            let slot = &mut self.slots[self.oldest];
            if !slot.in_flight || !slot.mapped.load(Ordering::Acquire) {
                return;
            }
            if let Ok(bytes) = slot.buffer.slice(..).get_mapped_range() {
                let tick = |i: usize| {
                    let at = i * wgpu::QUERY_SIZE as usize;
                    u64::from_le_bytes(bytes[at..at + 8].try_into().unwrap_or_default())
                };
                let mut spans = [0.0; MARKS - 1];
                for (i, span) in spans.iter_mut().enumerate() {
                    *span = tick(i + 1).saturating_sub(tick(i)) as f32 * self.period / 1e6;
                }
                each(spans);
            }
            slot.buffer.unmap();
            slot.in_flight = false;
            self.oldest = (self.oldest + 1) % self.slots.len();
        }
    }
}

pub(crate) struct FrameClock {
    last: Instant,
    laps: [f32; LAPS],
}

#[inline]
pub(crate) fn lap(clock: &mut Option<FrameClock>, which: Lap) {
    if let Some(clock) = clock {
        let now = Instant::now();
        clock.laps[which as usize] = now.duration_since(clock.last).as_secs_f32() * 1e3;
        clock.last = now;
    }
}

#[derive(Clone, Copy, Default)]
pub(crate) struct FrameScene {
    pub(crate) tick: u32,
    pub(crate) billboards: usize,
    pub(crate) models: usize,
    pub(crate) living: usize,
    pub(crate) living_middle: [f32; 2],
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Phase {
    Seeking,
    WarmingUp,
    Measuring,
    Done,
}

pub(crate) struct FrameBudget {
    options: BudgetOptions,
    phase: Phase,
    phase_start: Instant,
    last_frame_start: Option<Instant>,
    frame_ms: Vec<f32>,
    laps: Vec<[f32; LAPS]>,
    gpu: Vec<[f32; MARKS - 1]>,
    per_tick_ms: Vec<f32>,
    first_tick: Option<u32>,
    last_scene: FrameScene,
    billboards: Vec<usize>,
    scan_next_tick: u32,
    pub(crate) surface: String,
    current_size: (u32, u32),
    measured_sizes: Vec<(u32, u32)>,
    ticks_before: u32,
    dialog: Option<(f32, bool)>,
    dialog_still_ms: Vec<f32>,
    dialog_redrawn_ms: Vec<f32>,
    session_ms: Vec<f32>,
    pub(crate) follow: Option<Follow>,
}

impl FrameBudget {
    pub(crate) fn new(options: BudgetOptions) -> Self {
        Self {
            options,
            phase: if options.from_tick.is_some() {
                Phase::Seeking
            } else {
                Phase::WarmingUp
            },
            phase_start: Instant::now(),
            last_frame_start: None,
            frame_ms: Vec::new(),
            laps: Vec::new(),
            gpu: Vec::new(),
            per_tick_ms: Vec::new(),
            first_tick: None,
            last_scene: FrameScene::default(),
            billboards: Vec::new(),
            scan_next_tick: 0,
            surface: String::new(),
            current_size: (0, 0),
            measured_sizes: Vec::new(),
            ticks_before: 0,
            dialog: None,
            dialog_still_ms: Vec::new(),
            dialog_redrawn_ms: Vec::new(),
            session_ms: Vec::new(),
            follow: options.follow.map(Follow::new),
        }
    }

    fn at_measuring_tick(&self) -> bool {
        self.phase != Phase::Seeking
    }

    pub(crate) fn wait_for_gpu(&self) -> bool {
        self.options.wait
    }

    pub(crate) fn finished(&self) -> bool {
        self.phase == Phase::Done
    }

    pub(crate) fn begin_play(&mut self) {
        if self.phase == Phase::WarmingUp {
            self.phase_start = Instant::now();
        }
    }

    pub(crate) fn note_session(&mut self, spent: Duration) {
        if self.phase == Phase::Measuring {
            self.session_ms.push(spent.as_secs_f32() * 1e3);
        }
    }

    pub(crate) fn reached_seek_tick(&mut self, tick: u32) -> bool {
        if self.phase == Phase::Seeking && self.options.from_tick.is_some_and(|t| tick >= t) {
            self.phase = Phase::WarmingUp;
            self.phase_start = Instant::now();
            return true;
        }
        false
    }

    pub(crate) fn gpu_resized(&mut self, width: u32, height: u32) {
        let size = (width.max(1), height.max(1));
        self.current_size = size;
        if self.phase == Phase::Measuring && self.measured_sizes.last() != Some(&size) {
            self.measured_sizes.push(size);
        }
    }

    fn effective_size(&self) -> (u32, u32) {
        if self.current_size != (0, 0) {
            self.current_size
        } else if let Some(((w, h), _)) = parse_leading_size(&self.surface) {
            (w, h)
        } else {
            (0, 0)
        }
    }

    pub(crate) fn size_header(&self) -> String {
        let sizes = if !self.measured_sizes.is_empty() {
            &self.measured_sizes[..]
        } else {
            let eff = self.effective_size();
            if eff != (0, 0) {
                return format!("{}x{}", eff.0, eff.1);
            } else {
                return String::new();
            }
        };

        match sizes {
            [] => String::new(),
            [(w, h)] => format!("{w}x{h}"),
            _ => {
                let formatted = sizes
                    .iter()
                    .map(|(w, h)| format!("{w}x{h}"))
                    .collect::<Vec<_>>()
                    .join(" -> ");
                format!("{formatted} (resized)")
            }
        }
    }

    pub(crate) fn surface_header(&self) -> String {
        let size_str = self.size_header();
        if size_str.is_empty() {
            return self.surface.clone();
        }
        if let Some((_size, rest)) = parse_leading_size(&self.surface) {
            if rest.is_empty() {
                size_str
            } else {
                format!("{size_str} {rest}")
            }
        } else if self.surface.is_empty() {
            size_str
        } else {
            format!("{size_str} {}", self.surface)
        }
    }

    pub(crate) fn start_frame(&mut self, tick: u32) -> FrameClock {
        let now = Instant::now();
        if self.phase == Phase::WarmingUp && now.duration_since(self.phase_start) >= WARM_UP {
            self.phase = Phase::Measuring;
            self.phase_start = now;
            self.gpu.clear();
            self.last_frame_start = None;
            self.measured_sizes.clear();
            let size = self.effective_size();
            if size != (0, 0) {
                self.measured_sizes.push(size);
            }
        }
        if self.phase == Phase::Measuring && self.measured_sizes.is_empty() {
            let size = self.effective_size();
            if size != (0, 0) {
                self.measured_sizes.push(size);
            }
        }
        if self.phase == Phase::Measuring
            && let Some(last) = self.last_frame_start
        {
            self.frame_ms
                .push(now.duration_since(last).as_secs_f32() * 1e3);
        }
        self.last_frame_start = Some(now);
        self.ticks_before = tick;
        FrameClock {
            last: now,
            laps: [0.0; LAPS],
        }
    }

    pub(crate) fn end_frame(
        &mut self,
        clock: FrameClock,
        scene: FrameScene,
        timer: Option<&mut GpuTimer>,
        device: &wgpu::Device,
    ) {
        if let Some(timer) = timer {
            timer.after_submit();
            let measuring = self.phase == Phase::Measuring;
            let gpu = &mut self.gpu;
            timer.collect(device, |spans| {
                if measuring {
                    gpu.push(spans);
                }
            });
        }
        self.last_scene = scene;
        if self.phase != Phase::Measuring {
            return;
        }
        let ran = scene.tick.saturating_sub(self.ticks_before);
        if ran > 0 {
            self.per_tick_ms
                .push(clock.laps[Lap::Sim as usize] / ran as f32);
        }
        if let Some((step, redrew)) = self.dialog.take() {
            let ms = step + clock.laps[Lap::Encode as usize];
            if redrew {
                self.dialog_redrawn_ms.push(ms);
            } else {
                self.dialog_still_ms.push(ms);
            }
        }
        self.laps.push(clock.laps);
        self.billboards.push(scene.billboards);
        self.first_tick.get_or_insert(self.ticks_before);
        if let Some(every) = self.options.scan_ticks {
            if scene.tick >= self.scan_next_tick {
                self.scan_next_tick = scene.tick + every.max(1);
                self.print_scan_line(&scene);
            }
        } else if self.phase_start.elapsed().as_secs_f32() >= self.options.seconds {
            self.print_report();
            self.phase = Phase::Done;
        }
    }

    fn print_scan_line(&mut self, scene: &FrameScene) {
        let gpu_total = |spans: &[f32; MARKS - 1]| spans.iter().sum::<f32>();
        let gpu: Vec<f32> = self.gpu.iter().map(gpu_total).collect();
        let build: Vec<f32> = self.laps.iter().map(build_ms).collect();
        if let Some(follow) = &self.follow {
            println!("frame-budget scan: {}", follow.describe());
        }
        println!(
            "frame-budget scan tick {} billboards avg {:.0} max {} models {} living {} middle \
             {:.1},{:.1} gpu avg {:.3} max {:.3} build avg {:.3} max {:.3} sim/tick avg {:.3} \
             max {:.3}",
            scene.tick,
            self.billboards.iter().sum::<usize>() as f32 / self.billboards.len().max(1) as f32,
            self.billboards.iter().copied().max().unwrap_or(0),
            scene.models,
            scene.living,
            scene.living_middle[0],
            scene.living_middle[1],
            average(&gpu),
            gpu.iter().copied().fold(0.0, f32::max),
            average(&build),
            build.iter().copied().fold(0.0, f32::max),
            average(&self.per_tick_ms),
            self.per_tick_ms.iter().copied().fold(0.0, f32::max),
        );
        self.gpu.clear();
        self.laps.clear();
        self.per_tick_ms.clear();
        self.frame_ms.clear();
        self.billboards.clear();
    }

    fn print_report(&self) {
        let seconds = self.phase_start.elapsed().as_secs_f32();
        let frames = self.laps.len();
        println!(
            "frame-budget: {} | ticks {}..{} | {frames} frames in {seconds:.1} s, {:.1} fps | \
             billboards avg {:.0} max {} | models {} | living {} | budget {BUDGET_MS} ms",
            self.surface_header(),
            self.first_tick.unwrap_or(0),
            self.last_scene.tick,
            frames as f32 / seconds.max(0.001),
            self.billboards.iter().sum::<usize>() as f32 / frames.max(1) as f32,
            self.billboards.iter().copied().max().unwrap_or(0),
            self.last_scene.models,
            self.last_scene.living,
        );
        if let Some(follow) = &self.follow {
            println!("frame-budget: {}", follow.describe());
        }
        let row = |name: &str, samples: Vec<f32>| {
            println!("{}", report_row(name, samples));
        };
        println!("{:<34} {:>8} {:>8} {:>8}", "stage, ms", "avg", "p99", "max");
        row("whole frame (interval)", self.frame_ms.clone());
        println!(
            "frames over {BUDGET_MS} ms: {} of {}",
            self.frame_ms.iter().filter(|&&ms| ms > BUDGET_MS).count(),
            self.frame_ms.len(),
        );
        for (i, name) in LAP_NAMES.iter().enumerate() {
            if i == Lap::GpuWait as usize && !self.options.wait {
                continue;
            }
            row(name, self.laps.iter().map(|l| l[i]).collect());
        }
        row("cpu build total", self.laps.iter().map(build_ms).collect());
        row("cpu sim per tick", self.per_tick_ms.clone());
        if !self.session_ms.is_empty() {
            row("cpu session", self.session_ms.clone());
        }
        if !self.dialog_still_ms.is_empty() {
            row("cpu dialog + encode, still", self.dialog_still_ms.clone());
            row(
                "cpu dialog + encode, redrawn",
                self.dialog_redrawn_ms.clone(),
            );
        }
        if self.gpu.is_empty() {
            println!("gpu: no timestamps on this device");
            return;
        }
        for (i, name) in GPU_STAGES.iter().enumerate() {
            row(name, self.gpu.iter().map(|g| g[i]).collect());
        }
        row(
            "gpu category 4 total",
            self.gpu.iter().map(|g| g[2] + g[3]).collect(),
        );
        row(
            "gpu total",
            self.gpu.iter().map(|g| g.iter().sum()).collect(),
        );
    }
}

pub(crate) fn overlay_frame<'a>(
    dialog: &'a mut Option<Menu>,
    args: &Args,
    app: &mut ClientApp,
) -> Option<&'a mut dyn DrawInterface> {
    let which = args.frame_budget_overlay?;
    let screen = app.dialog_ui().logical;
    let budget = app.frame_budget.as_mut()?;
    if !budget.at_measuring_tick() {
        return None;
    }
    if dialog.is_none() {
        match Menu::for_budget(args, app.mesh_tag.mesh_id, which) {
            Ok(menu) => *dialog = Some(menu),
            Err(e) => {
                tracing::error!("--frame-budget-overlay: {e:#}");
                return None;
            }
        }
    }
    let menu = dialog.as_mut()?;
    let started = Instant::now();
    let redrew = menu.budget_step(screen, which);
    budget.dialog = Some((started.elapsed().as_secs_f32() * 1e3, redrew));
    app.set_paused(true);
    Some(menu.front_end())
}

fn parse_leading_size(s: &str) -> Option<((u32, u32), &str)> {
    let (first, rest) = match s.split_once(' ') {
        Some((first, rest)) => (first, rest),
        None => (s, ""),
    };
    let (w, h) = first.split_once('x')?;
    let w: u32 = w.parse().ok()?;
    let h: u32 = h.parse().ok()?;
    if w == 0 || h == 0 {
        return None;
    }
    Some(((w, h), rest))
}

fn build_ms(laps: &[f32; LAPS]) -> f32 {
    laps[Lap::Billboards as usize]
        + laps[Lap::LandscapeUpload as usize]
        + laps[Lap::Prepare as usize]
        + laps[Lap::Encode as usize]
}

fn average(samples: &[f32]) -> f32 {
    samples.iter().sum::<f32>() / samples.len().max(1) as f32
}

fn percentile_99(sorted: &[f32]) -> f32 {
    if sorted.is_empty() {
        return 0.0;
    }
    let rank = (sorted.len() as f32 * 0.99).ceil() as usize;
    sorted[rank.clamp(1, sorted.len()) - 1]
}

fn report_row(name: &str, mut samples: Vec<f32>) -> String {
    samples.sort_by(f32::total_cmp);
    format!(
        "{name:<34} {:>8.3} {:>8.3} {:>8.3}",
        average(&samples),
        percentile_99(&samples),
        samples.last().copied().unwrap_or(0.0),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn percentile_is_the_nearest_rank() {
        let hundred: Vec<f32> = (1..=100).map(|i| i as f32).collect();
        assert_eq!(percentile_99(&hundred), 99.0);
        assert_eq!(percentile_99(&[5.0]), 5.0);
        assert_eq!(percentile_99(&[]), 0.0);
    }

    #[test]
    fn uncapped_mode_prefers_immediate_and_says_fifo_when_it_cannot() {
        use wgpu::PresentMode::{Fifo, Immediate, Mailbox};
        assert_eq!(
            uncapped_present_mode(&[Fifo, Mailbox, Immediate]),
            Immediate
        );
        assert_eq!(uncapped_present_mode(&[Fifo, Mailbox]), Mailbox);
        assert_eq!(uncapped_present_mode(&[Fifo]), Fifo);
    }

    #[test]
    fn window_mode_is_uncapped_unless_vsync_is_wanted() {
        use wgpu::PresentMode::{AutoVsync, Fifo, Immediate, Mailbox};
        let all = [Fifo, Mailbox, Immediate];
        assert_eq!(window_present_mode(false, false, &all), Immediate);
        assert_eq!(window_present_mode(false, false, &[Fifo, Mailbox]), Mailbox);
        assert_eq!(window_present_mode(true, false, &all), AutoVsync);
        assert_eq!(window_present_mode(true, true, &all), Immediate);
        assert_eq!(
            present_mode_line(Immediate, false, false, "the default", &all),
            "present mode: Immediate (vsync off, the default) of [Fifo, Mailbox, Immediate]"
        );
        assert_eq!(
            present_mode_line(AutoVsync, true, false, "--vsync", &all),
            "present mode: AutoVsync (vsync on, --vsync) of [Fifo, Mailbox, Immediate]"
        );
        assert_eq!(
            present_mode_line(Immediate, true, true, "the settings file", &all),
            "present mode: Immediate (vsync off, --frame-budget) of [Fifo, Mailbox, Immediate]"
        );
        let mode = window_present_mode(false, false, &[Fifo]);
        assert_eq!(mode, Fifo);
        assert_eq!(
            present_mode_line(mode, false, false, "the settings file", &[Fifo]),
            "present mode: Fifo (vsync off, the settings file) of [Fifo]; the surface has no \
             Immediate or Mailbox, so vsync stays on"
        );
        assert_eq!(
            present_mode_line(Fifo, true, false, "--vsync", &[Fifo]),
            "present mode: Fifo (vsync on, --vsync) of [Fifo]"
        );
    }

    #[test]
    fn follow_defaults_to_the_benchmark_dwarf() {
        use clap::Parser;
        let parse = |extra: &[&str]| {
            Args::try_parse_from(std::iter::once("mandrake").chain(extra.iter().copied()))
        };
        let bare = parse(&["--frame-budget", "5", "--frame-budget-follow"]).unwrap();
        assert_eq!(bare.frame_budget_follow, Some(DEFAULT_FOLLOW_UNIT));
        let named = parse(&["--frame-budget", "5", "--frame-budget-follow", "7"]).unwrap();
        assert_eq!(named.frame_budget_follow, Some(7));
        assert_eq!(
            parse(&["--frame-budget", "5"]).unwrap().frame_budget_follow,
            None
        );
        assert!(parse(&["--frame-budget-follow"]).is_err());
    }

    #[test]
    fn a_seek_ends_once_and_leaves_the_measuring_to_the_warm_up() {
        let mut budget = FrameBudget::new(BudgetOptions {
            seconds: 1.0,
            from_tick: Some(100),
            scan_ticks: None,
            wait: false,
            follow: None,
        });
        assert!(!budget.reached_seek_tick(99));
        assert!(budget.reached_seek_tick(100));
        assert!(!budget.reached_seek_tick(101));
        assert!(budget.phase == Phase::WarmingUp);
    }

    #[test]
    fn a_session_level_warms_up_from_begin_play() {
        let mut budget = FrameBudget::new(BudgetOptions {
            seconds: 1.0,
            from_tick: None,
            scan_ticks: None,
            wait: false,
            follow: None,
        });
        let created = budget.phase_start;
        std::thread::sleep(Duration::from_millis(2));
        budget.begin_play();
        assert!(budget.phase == Phase::WarmingUp);
        assert!(budget.phase_start > created);
        budget.phase = Phase::Measuring;
        let measuring = budget.phase_start;
        budget.begin_play();
        assert!(budget.phase_start == measuring);
    }

    #[test]
    fn header_shows_single_measured_size() {
        let mut budget = FrameBudget::new(BudgetOptions {
            seconds: 1.0,
            from_tick: None,
            scan_ticks: None,
            wait: false,
            follow: None,
        });
        budget.surface = "1270x1382 AutoVsync of [AutoVsync, Fifo], GPU Vulkan".into();
        budget.gpu_resized(1920, 1080);
        budget.phase = Phase::Measuring;
        budget.start_frame(0);
        assert_eq!(budget.size_header(), "1920x1080");
        assert_eq!(
            budget.surface_header(),
            "1920x1080 AutoVsync of [AutoVsync, Fifo], GPU Vulkan"
        );
    }

    #[test]
    fn header_shows_size_at_measurement_start_if_resized_before() {
        let mut budget = FrameBudget::new(BudgetOptions {
            seconds: 1.0,
            from_tick: None,
            scan_ticks: None,
            wait: false,
            follow: None,
        });
        budget.surface = "1270x1382 AutoVsync, GPU Vulkan".into();
        budget.gpu_resized(1270, 1382);
        budget.gpu_resized(3840, 2160);
        budget.phase = Phase::Measuring;
        budget.start_frame(0);
        assert_eq!(budget.size_header(), "3840x2160");
        assert_eq!(budget.surface_header(), "3840x2160 AutoVsync, GPU Vulkan");
    }

    #[test]
    fn header_indicates_when_size_changed_during_measurement() {
        let mut budget = FrameBudget::new(BudgetOptions {
            seconds: 1.0,
            from_tick: None,
            scan_ticks: None,
            wait: false,
            follow: None,
        });
        budget.surface = "1270x1382 AutoVsync, GPU Vulkan".into();
        budget.gpu_resized(1270, 1382);
        budget.phase = Phase::Measuring;
        budget.start_frame(0);
        assert_eq!(budget.size_header(), "1270x1382");

        budget.gpu_resized(3840, 2160);
        assert_eq!(budget.size_header(), "1270x1382 -> 3840x2160 (resized)");
        assert_eq!(
            budget.surface_header(),
            "1270x1382 -> 3840x2160 (resized) AutoVsync, GPU Vulkan"
        );
    }

    #[test]
    fn header_shows_multiple_resizes_during_measurement() {
        let mut budget = FrameBudget::new(BudgetOptions {
            seconds: 1.0,
            from_tick: None,
            scan_ticks: None,
            wait: false,
            follow: None,
        });
        budget.gpu_resized(800, 600);
        budget.phase = Phase::Measuring;
        budget.start_frame(0);

        budget.gpu_resized(1024, 768);
        budget.gpu_resized(1920, 1080);
        budget.gpu_resized(1920, 1080);

        assert_eq!(
            budget.size_header(),
            "800x600 -> 1024x768 -> 1920x1080 (resized)"
        );
    }

    #[test]
    fn header_distinguishes_resizes_before_from_resizes_during() {
        let mut budget = FrameBudget::new(BudgetOptions {
            seconds: 1.0,
            from_tick: None,
            scan_ticks: None,
            wait: false,
            follow: None,
        });
        budget.surface = "1270x1382 Fifo, GPU".into();
        budget.gpu_resized(1270, 1382);
        budget.gpu_resized(1920, 1080);

        budget.phase = Phase::Measuring;
        budget.start_frame(0);

        budget.gpu_resized(3840, 2160);

        assert_eq!(budget.size_header(), "1920x1080 -> 3840x2160 (resized)");
        assert_eq!(
            budget.surface_header(),
            "1920x1080 -> 3840x2160 (resized) Fifo, GPU"
        );
    }

    #[test]
    fn gpu_timer_unmaps_busy_readback_slot_before_copy() {
        let Some((device, queue)) = crate::tests::test_gpu() else {
            return;
        };
        if !device.features().contains(timer_features()) {
            return;
        }
        let mut timer = GpuTimer::new(&device, &queue);
        let scope = device.push_error_scope(wgpu::ErrorFilter::Validation);
        for _ in 0..(READBACK_SLOTS + 2) {
            let mut encoder = device.create_command_encoder(&Default::default());
            timer.mark(&mut encoder, Mark::Start);
            timer.mark(&mut encoder, Mark::Interface);
            timer.encode_resolve(&device, &mut encoder);
            queue.submit(std::iter::once(encoder.finish()));
            timer.after_submit();
        }
        let _ = device.poll(wgpu::PollType::wait_indefinitely());
        timer.collect(&device, |_| {});
        let error = pollster::block_on(scope.pop());
        assert!(error.is_none(), "{error:?}");
    }
}
