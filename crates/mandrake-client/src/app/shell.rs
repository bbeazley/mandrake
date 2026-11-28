use crate::app::bar_reservation::BarReservation;
use crate::app::fps_cap::{self, FpsCap, Pacer};
use crate::app::frame_budget;
use crate::app::game_scene::{Ended, FrameScene, GameScene, SceneFrame, SceneSetup};
use crate::app::input::myth_key;
use crate::app::sound::open_audio_device;
use crate::app::view_settings::ViewSettings;
use crate::args::Args;
use crate::audio_out::AudioOutput;
use crate::clock::Instant;
use crate::controls::{self, Controls, Modifiers};
use crate::{settings, ui_scale, window_surface};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;
use tracing::{error, info, warn};
use winit::application::ApplicationHandler;
use winit::dpi::PhysicalSize;
use winit::event::{ElementState, KeyEvent, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow};
use winit::keyboard::{ModifiersState, PhysicalKey};
use winit::monitor::MonitorHandle;
use winit::window::{Fullscreen, Window, WindowId};

pub(super) const MENU_HIDDEN_WAKE: Duration = Duration::from_millis(100);
const MONITOR_CHECK: Duration = Duration::from_secs(1);

const FPS_REPORT_WARM_UP: u64 = 2;

pub(crate) struct ShellGpu {
    pub(super) surface: wgpu::Surface<'static>,
    pub(super) surface_config: wgpu::SurfaceConfiguration,
    surface_formats: Vec<wgpu::TextureFormat>,
    hdr_sdr_white: Option<f32>,
    pub(crate) canvas: Option<mandrake_render::FrameCanvas>,
    pub(super) queue: wgpu::Queue,
    pub(super) device: wgpu::Device,
    pub(super) instance: wgpu::Instance,
    pub(super) budget_surface: String,
    pub(super) renderers: std::cell::OnceCell<crate::app::gpu::MapRenderers>,
    _bar_reservation: Option<BarReservation>,
}

impl ShellGpu {
    async fn open(
        window: Arc<Window>,
        budget: bool,
        vsync: (bool, &str),
        ten_bit: (bool, &str),
        hdr: (crate::args::HdrMode, &str),
    ) -> anyhow::Result<Self> {
        let size = window.inner_size();
        let width = size.width.max(1);
        let height = size.height.max(1);
        let monitor_hdr = if cfg!(not(target_arch = "wasm32")) {
            crate::win_hdr::query_monitor_hdr(&window)
        } else {
            None
        };

        let window_surface::WindowSurface {
            instance,
            surface,
            adapter,
        } = window_surface::create_window_surface(window).await?;

        let info = adapter.get_info();
        let timer_features = budget && adapter.features().contains(frame_budget::timer_features());
        let (
            device,
            queue,
            surface_config,
            surface_formats,
            hdr_sdr_white,
            bar_reservation,
            canvas,
        ) = async {
            let surface_caps = surface.get_capabilities(&adapter);
            let ten_bit = (cfg!(not(target_arch = "wasm32")) && ten_bit.0, ten_bit.1);
            let hdr = if cfg!(target_arch = "wasm32") {
                (crate::args::HdrMode::Off, hdr.1)
            } else {
                hdr
            };
            let choice = window_surface::choose_surface_config(
                &surface_caps.formats,
                ten_bit.0,
                hdr.0,
                monitor_hdr,
            );
            eprintln!(
                "{}",
                window_surface::surface_choice_line(choice, ten_bit, hdr, &surface_caps.formats)
            );

            let (device, queue) = adapter
                .request_device(&wgpu::DeviceDescriptor {
                    label: Some("Window Device"),
                    required_features: if timer_features {
                        frame_budget::timer_features()
                    } else {
                        wgpu::Features::empty()
                    },
                    required_limits: adapter.limits(),
                    experimental_features: wgpu::ExperimentalFeatures::disabled(),
                    memory_hints: wgpu::MemoryHints::default(),
                    trace: wgpu::Trace::Off,
                })
                .await?;
            #[cfg(target_arch = "wasm32")]
            {
                device.on_uncaptured_error(std::sync::Arc::new(|e| {
                    tracing::error!("WebGPU error: {e}");
                }));
                device.set_device_lost_callback(|reason, message| {
                    tracing::error!("WebGPU device lost ({reason:?}): {message}");
                });
            }
            let bar_reservation = BarReservation::take(&device);
            if let Some(bar) = &bar_reservation {
                info!(
                    "staging kept off the PCIe BAR: {} of its {} MB held",
                    bar.size >> 20,
                    bar.heap_size >> 20,
                );
            }

            let formats = window_surface::SurfaceFormats::choose(
                choice.format,
                adapter
                    .get_downlevel_capabilities()
                    .flags
                    .contains(wgpu::DownlevelFlags::SURFACE_VIEW_FORMATS),
            );
            let surface_config = wgpu::SurfaceConfiguration {
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                    | (surface_caps.usages & wgpu::TextureUsages::COPY_SRC),
                format: choice.format,
                width,
                height,
                present_mode: frame_budget::window_present_mode(
                    vsync.0,
                    budget,
                    &surface_caps.present_modes,
                ),
                alpha_mode: surface_caps
                    .alpha_modes
                    .first()
                    .copied()
                    .ok_or_else(|| anyhow::anyhow!("surface has no alpha modes"))?,
                color_space: choice.color_space,
                view_formats: formats.view_formats(),
                desired_maximum_frame_latency: 2,
            };
            surface.configure(&device, &surface_config);
            eprintln!(
                "{}",
                frame_budget::present_mode_line(
                    surface_config.present_mode,
                    vsync.0,
                    budget,
                    vsync.1,
                    &surface_caps.present_modes,
                )
            );

            let canvas = formats.canvas().then(|| {
                mandrake_render::FrameCanvas::new(
                    &device,
                    choice.format,
                    (width, height),
                    choice.hdr_sdr_white,
                )
            });
            Ok::<_, anyhow::Error>((
                device,
                queue,
                surface_config,
                surface_caps.formats,
                choice.hdr_sdr_white,
                bar_reservation,
                canvas,
            ))
        }
        .await?;

        let budget_surface = if budget {
            format!(
                "{width}x{height} {:?} {:?} of {:?}, {} {:?}",
                surface_config.format,
                surface_config.present_mode,
                surface.get_capabilities(&adapter).present_modes,
                info.name,
                info.backend,
            )
        } else {
            String::new()
        };

        Ok(Self {
            surface,
            surface_config,
            surface_formats,
            hdr_sdr_white,
            canvas,
            queue,
            device,
            instance,
            budget_surface,
            renderers: std::cell::OnceCell::new(),
            _bar_reservation: bar_reservation,
        })
    }
}

impl ShellGpu {
    pub(crate) fn frame_surface(&self) -> window_surface::FrameSurface<'_> {
        window_surface::FrameSurface {
            surface: &self.surface,
            canvas: self.canvas.as_ref(),
            config: &self.surface_config,
        }
    }
}

pub(crate) const fn cursor_grab(wants: bool, fullscreen: bool, focused: bool) -> bool {
    wants && fullscreen && focused
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct WindowFacts {
    pub(super) focused: bool,
    pub(super) occluded: bool,
    pub(super) cursor_in_window: bool,
    pub(super) cursor: (f32, f32),
}

impl Default for WindowFacts {
    fn default() -> Self {
        Self {
            focused: true,
            occluded: false,
            cursor_in_window: false,
            cursor: (0.0, 0.0),
        }
    }
}

impl WindowFacts {
    fn observe(&mut self, event: &WindowEvent) {
        match event {
            WindowEvent::Focused(focused) => self.focused = *focused,
            WindowEvent::Occluded(occluded) => {
                #[cfg(target_arch = "wasm32")]
                if *occluded != self.occluded {
                    info!(
                        "web: the canvas is {}",
                        if *occluded { "hidden" } else { "shown" }
                    );
                }
                self.occluded = *occluded;
            }
            WindowEvent::CursorEntered { .. } => self.cursor_in_window = true,
            WindowEvent::CursorLeft { .. } => self.cursor_in_window = false,
            WindowEvent::CursorMoved { position, .. } => {
                self.cursor_in_window = true;
                self.cursor = (position.x as f32, position.y as f32);
            }
            _ => {}
        }
    }
}

pub(crate) struct ShellHost {
    window: Option<Arc<Window>>,
    pub(super) net_waker: Option<mandrake_net::transport::Waker>,
    pub(super) net_woken: Arc<std::sync::atomic::AtomicBool>,
    pub(super) gpu: Option<ShellGpu>,
    #[cfg(target_arch = "wasm32")]
    web_pending_size: Option<PhysicalSize<u32>>,
    #[cfg(target_arch = "wasm32")]
    gpu_pending: std::rc::Rc<std::cell::RefCell<Option<anyhow::Result<ShellGpu>>>>,
    pub(super) audio: Option<AudioOutput>,
    pub(super) failed: Option<anyhow::Error>,
    pub(super) window_facts: WindowFacts,
    pub(super) modifiers: ModifiersState,
    pub(super) args: Args,
    pub(super) view: ViewSettings,
    pub(super) controls: Controls,
    pub(super) settings_path: Option<PathBuf>,
    pub(super) ui_mode: ui_scale::UiScaleMode,
    pub(super) fullscreen_setting: bool,
    fps_cap: (FpsCap, &'static str),
    pacer: Pacer,
    scene_timeouts: window_surface::SurfaceTimeoutLog,
    cap_monitor: Option<(Option<MonitorHandle>, Option<u32>)>,
    monitor_checked: Option<Instant>,
    cap_line: String,
    scene_title: String,
    cursor_grabbed: bool,
    scene_shot: bool,
    scene_min_window: Option<(u32, u32)>,
    scene_ended: bool,
}

pub(crate) struct Shell {
    pub(super) host: ShellHost,
    scene: Box<dyn GameScene>,
}

pub(crate) fn run_event_loop(
    event_loop: winit::event_loop::EventLoop<()>,
    shell: Shell,
) -> anyhow::Result<Option<Shell>> {
    #[cfg(not(target_arch = "wasm32"))]
    {
        let mut shell = shell;
        shell.host.wake_on_net(&event_loop);
        super::guard::install();
        event_loop.run_app(&mut shell)?;
        Ok(Some(shell))
    }
    #[cfg(target_arch = "wasm32")]
    {
        use winit::platform::web::EventLoopExtWebSys;
        event_loop.spawn_app(shell);
        Ok(None)
    }
}

fn web_canvas(attrs: winit::window::WindowAttributes) -> winit::window::WindowAttributes {
    #[cfg(target_arch = "wasm32")]
    {
        use web_sys::wasm_bindgen::JsCast;
        use winit::platform::web::WindowAttributesExtWebSys;
        let canvas = web_sys::window()
            .and_then(|w| w.document())
            .and_then(|d| d.get_element_by_id("myth2"))
            .and_then(|e| e.dyn_into::<web_sys::HtmlCanvasElement>().ok());
        let append = canvas.is_none();
        attrs.with_canvas(canvas).with_append(append)
    }
    #[cfg(not(target_arch = "wasm32"))]
    attrs
}

fn opens_fullscreen(setting: bool, args: &Args) -> bool {
    opens_fullscreen_on(setting, args, cfg!(target_arch = "wasm32"))
}

fn opens_fullscreen_on(setting: bool, args: &Args, web: bool) -> bool {
    setting && args.frame_budget.is_none() && !web
}

fn opens_hidden() -> bool {
    opens_hidden_on(std::env::consts::OS)
}

// On Windows, winit shows the window at its initial size before switching to fullscreen and
// initializing the GPU, flashing a white box. Other platforms stay visible (Wayland won't
// configure a hidden window).
fn opens_hidden_on(os: &str) -> bool {
    os == "windows"
}

pub(crate) fn acquire_surface(
    surface: &wgpu::Surface<'_>,
    device: &wgpu::Device,
    config: &wgpu::SurfaceConfiguration,
    timeouts: &mut window_surface::SurfaceTimeoutLog,
) -> Option<wgpu::SurfaceTexture> {
    match surface.get_current_texture() {
        wgpu::CurrentSurfaceTexture::Success(frame)
        | wgpu::CurrentSurfaceTexture::Suboptimal(frame) => {
            timeouts.presented();
            Some(frame)
        }
        failed => {
            window_surface::handle_surface_error(&failed, surface, device, config, timeouts);
            None
        }
    }
}

pub(crate) fn clear_target(
    encoder: &mut wgpu::CommandEncoder,
    view: &wgpu::TextureView,
    color: wgpu::Color,
    label: &str,
) {
    encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
        label: Some(label),
        color_attachments: &[Some(wgpu::RenderPassColorAttachment {
            depth_slice: None,
            view,
            resolve_target: None,
            ops: wgpu::Operations {
                load: wgpu::LoadOp::Clear(color),
                store: wgpu::StoreOp::Store,
            },
        })],
        depth_stencil_attachment: None,
        occlusion_query_set: None,
        timestamp_writes: None,
        multiview_mask: None,
    });
}

pub(crate) fn binding_modifiers(state: ModifiersState) -> Modifiers {
    Modifiers {
        ctrl: state.control_key(),
        shift: state.shift_key(),
        alt: state.alt_key() || state.super_key(),
    }
}

impl Shell {
    pub(crate) fn new(host: ShellHost, scene: Box<dyn GameScene>) -> Self {
        Self { host, scene }
    }

    pub(crate) fn take_failure(&mut self) -> Option<anyhow::Error> {
        self.host.failed.take()
    }

    fn drop_ended_scene(&mut self) {
        if std::mem::take(&mut self.host.scene_ended) {
            self.scene = Box::new(Ended);
        }
    }
}

impl ShellHost {
    pub(crate) fn new(args: Args) -> Self {
        let (controls, controls_path) = controls::load(args.conf.as_deref(), false);
        match &controls_path {
            Some(path) => info!(
                "Controls: {} merged over the Myth II defaults",
                path.display()
            ),
            None => info!("Controls: Myth II 1.8.5 defaults (Right Handed set)"),
        }
        let settings_path = settings::user_settings_path();
        let saved_zoom = match (&args.ui_scale, &settings_path) {
            (None, Some(path)) => settings::load_ui_zoom(path).unwrap_or_else(|e| {
                warn!("settings: {}: {e}; ignoring it", path.display());
                None
            }),
            _ => None,
        };
        if let Some(percent) = saved_zoom {
            info!("UI zoom: {percent}% saved in the client settings");
        }
        let ui_mode = args.ui_scale.unwrap_or_else(|| {
            saved_zoom.map_or_else(Default::default, ui_scale::UiScaleMode::from_zoom)
        });
        let audio = open_audio_device(&args);
        Self {
            window: None,
            gpu: None,
            #[cfg(target_arch = "wasm32")]
            web_pending_size: None,
            #[cfg(target_arch = "wasm32")]
            gpu_pending: Default::default(),
            audio,
            failed: None,
            window_facts: WindowFacts::default(),
            net_waker: None,
            net_woken: Arc::default(),
            modifiers: ModifiersState::empty(),
            view: ViewSettings::of(&args),
            args,
            controls,
            settings_path,
            ui_mode,
            fullscreen_setting: false,
            fps_cap: (FpsCap::Off, "unread"),
            pacer: Pacer::default(),
            scene_timeouts: window_surface::SurfaceTimeoutLog::default(),
            cap_monitor: None,
            monitor_checked: None,
            cap_line: String::new(),
            scene_title: String::new(),
            cursor_grabbed: false,
            scene_shot: false,
            scene_min_window: None,
            scene_ended: false,
        }
    }

    pub(crate) fn view(&self) -> &ViewSettings {
        &self.view
    }

    pub(crate) fn make_scene(
        &mut self,
        make: impl FnOnce(&Args, SceneSetup) -> anyhow::Result<Box<dyn GameScene>>,
        controls: Controls,
    ) -> anyhow::Result<Box<dyn GameScene>> {
        let interface = crate::interface_config::load(self.args.conf.as_deref(), false);
        let setup = SceneSetup {
            audio: self.audio.as_ref().map(AudioOutput::link),
            view: self.view.clone(),
            menu_filter: self
                .args
                .ui_filter
                .map_or_else(|| interface.menu_filter.into(), Into::into),
            cutscene_filter: self
                .args
                .cutscene_filter
                .unwrap_or(interface.cutscene_filter)
                .into(),
        };
        let scene = make(&self.args, setup)?;
        if self.window.is_none() {
            self.fullscreen_setting = interface.fullscreen;
        }
        self.controls = controls;
        Ok(scene)
    }

    fn scene_frame<S: FrameScene + ?Sized>(&mut self, scene: &mut S) {
        let Some(gpu) = self.gpu.as_ref() else {
            return;
        };
        let title = scene.title();
        if title != self.scene_title {
            if let Some(window) = &self.window {
                set_window_title(window, &title);
            }
            self.scene_title = title;
        }
        let Some(frame) = acquire_surface(
            &gpu.surface,
            &gpu.device,
            &gpu.surface_config,
            &mut self.scene_timeouts,
        ) else {
            return;
        };
        let formats = window_surface::SurfaceFormats::of(&gpu.surface_config);
        let target = window_surface::frame_texture(gpu.canvas.as_ref(), &frame);
        let view = |format| {
            target.create_view(&wgpu::TextureViewDescriptor {
                format: Some(format),
                ..Default::default()
            })
        };
        let (color, interface) = (view(formats.scene), view(formats.interface));
        let mut encoder = gpu
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("Scene Encoder"),
            });
        let size = (gpu.surface_config.width, gpu.surface_config.height);
        scene.frame(SceneFrame {
            device: &gpu.device,
            queue: &gpu.queue,
            encoder: &mut encoder,
            color: &color,
            interface: &interface,
            size,
            formats,
            ui: ui_scale::UiScale::new(self.ui_mode, size.0, size.1),
        });
        window_surface::present_canvas(gpu.canvas.as_ref(), &mut encoder, &frame);
        let shot_source = window_surface::screenshot_texture(
            gpu.canvas.as_ref(),
            &frame,
            gpu.surface_config.color_space,
        );
        let readback = std::mem::take(&mut self.scene_shot)
            .then(|| crate::screenshot::Readback::encode(&gpu.device, &mut encoder, shot_source));
        let timed = self.args.frame_budget.is_some().then(Instant::now);
        gpu.queue.submit(std::iter::once(encoder.finish()));
        let submitted = timed.map(|t| (t, Instant::now()));
        gpu.queue.present(frame);
        if let Some((started, submitted)) = submitted {
            let ms = |d: Duration| d.as_secs_f32() * 1e3;
            scene.submitted(ms(submitted - started), ms(submitted.elapsed()));
        }
        let pixels = readback.map(|copy| {
            copy.ok_or_else(|| anyhow::anyhow!("this window's surface can't be read"))
                .and_then(|copy| copy.read(&gpu.device))
        });
        let shot = pixels.map(|pixels| {
            pixels.and_then(|(size, rgba)| {
                let dir = crate::screenshot::user_screenshot_dir();
                let info = scene.shot_info(size);
                let time = crate::screenshot::time_of_day();
                crate::screenshot::write_shot(&dir, &info, &time, size, &rgba)
            })
        });
        match shot {
            Some(Ok(path)) => {
                let line = crate::screenshot::saved_line(&path);
                println!("{line}");
                scene.print_line(&line);
            }
            Some(Err(e)) => {
                warn!("Screenshot failed: {e:#}");
                scene.print_line(&crate::screenshot::failed_line(&e));
            }
            None => {}
        }
    }

    pub(crate) fn scene_event<S: FrameScene + ?Sized>(
        &mut self,
        scene: &mut S,
        event_loop: &ActiveEventLoop,
        event: WindowEvent,
    ) {
        self.scene_event_inner(scene, event_loop, event);
        self.sync_cursor_grab(scene.wants_cursor_grab());
    }

    fn sync_cursor_grab(&mut self, wants: bool) {
        let Some(window) = &self.window else {
            return;
        };
        let want = !cfg!(target_arch = "wasm32")
            && cursor_grab(
                wants,
                window.fullscreen().is_some(),
                self.window_facts.focused,
            );
        if want == self.cursor_grabbed {
            return;
        }
        self.cursor_grabbed = want;
        use winit::window::CursorGrabMode;
        let result = if want {
            window
                .set_cursor_grab(CursorGrabMode::Confined)
                .or_else(|_| window.set_cursor_grab(CursorGrabMode::Locked))
        } else {
            window.set_cursor_grab(CursorGrabMode::None)
        };
        match result {
            Ok(()) => info!(
                "Cursor grab: {}",
                if want {
                    "held in the window"
                } else {
                    "released"
                }
            ),
            Err(e) => warn!("Cursor grab ({want}): {e}"),
        }
    }

    fn scene_event_inner<S: FrameScene + ?Sized>(
        &mut self,
        scene: &mut S,
        event_loop: &ActiveEventLoop,
        event: WindowEvent,
    ) {
        match event {
            WindowEvent::CloseRequested => {
                self.quit(scene, event_loop);
                return;
            }
            WindowEvent::Resized(new_size) => {
                self.resized(scene, new_size);
                self.follow_monitor();
                self.sync_scene_min_window();
                return;
            }
            WindowEvent::RedrawRequested => {
                #[cfg(target_arch = "wasm32")]
                if self.gpu.is_some() {
                    self.apply_pending_size(scene);
                }
                self.pacer.frame_drawn(Instant::now());
                self.scene_frame(scene);
            }
            event => {
                let pressed = match &event {
                    WindowEvent::KeyboardInput {
                        event:
                            KeyEvent {
                                physical_key: PhysicalKey::Code(code),
                                state: ElementState::Pressed,
                                repeat: false,
                                ..
                            },
                        ..
                    } => myth_key(*code),
                    _ => None,
                };
                let held = binding_modifiers(self.modifiers);
                if let Some(delta) = pressed.and_then(|key| ui_scale::zoom_key_delta(key, held)) {
                    self.scene_ui_zoom(delta);
                    return;
                }
                if pressed.is_some_and(|key| {
                    self.controls
                        .actions_for_press_with(key, held)
                        .contains(&controls::ControlAction::Screenshot)
                }) {
                    self.scene_shot = true;
                }
                let size = self.map_size();
                scene.window_event(&event, size);
            }
        }
        if scene.wants_exit() {
            self.quit(scene, event_loop);
        }
    }

    pub(crate) fn scene_wait<S: GameScene + ?Sized>(
        &mut self,
        scene: &mut S,
        event_loop: &ActiveEventLoop,
    ) {
        if self.fps_report_done(scene, event_loop) {
            return;
        }
        if self.window_facts.occluded {
            event_loop.set_control_flow(ControlFlow::WaitUntil(Instant::now() + MENU_HIDDEN_WAKE));
        } else if self.gpu.is_some() {
            self.pace_frame(event_loop);
        }
    }

    fn scene_ui_zoom(&mut self, delta: i32) {
        let size = self.map_size();
        let ui = ui_scale::UiScale::new(self.ui_mode, size.0, size.1);
        let window = self.window.as_deref();
        let Some(level) = super::window::ui_zoom_press(self.ui_mode, &ui, delta, window) else {
            return;
        };
        self.ui_mode = ui_scale::UiScaleMode::from_zoom(level);
        super::window::ui_zoom_applied(level, size, window, self.settings_path.as_deref());
        self.sync_scene_min_window();
    }

    fn sync_scene_min_window(&mut self) {
        let size = self.map_size();
        let min = ui_scale::UiScale::new(self.ui_mode, size.0, size.1)
            .zoom
            .map(ui_scale::min_window);
        if min != self.scene_min_window {
            self.scene_min_window = min;
            if let Some(window) = &self.window {
                window.set_min_inner_size(min.map(|(w, h)| PhysicalSize::new(w, h)));
            }
        }
    }

    pub(super) fn map_size(&self) -> (u32, u32) {
        self.gpu
            .as_ref()
            .map_or((self.args.width, self.args.height), |gpu| {
                (gpu.surface_config.width, gpu.surface_config.height)
            })
    }

    pub(super) fn window(&self) -> Option<&Arc<Window>> {
        self.window.as_ref()
    }

    pub(super) fn gpu(&self) -> Option<&ShellGpu> {
        self.gpu.as_ref()
    }

    fn present_initial_black(&mut self) {
        let Some(gpu) = self.gpu.as_ref() else {
            return;
        };
        let Some(frame) = acquire_surface(
            &gpu.surface,
            &gpu.device,
            &gpu.surface_config,
            &mut self.scene_timeouts,
        ) else {
            return;
        };
        let target = window_surface::frame_texture(gpu.canvas.as_ref(), &frame);
        let view = target.create_view(&Default::default());
        let mut encoder = gpu
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("Initial Clear Encoder"),
            });
        clear_target(&mut encoder, &view, wgpu::Color::BLACK, "Initial Clear");
        window_surface::present_canvas(gpu.canvas.as_ref(), &mut encoder, &frame);
        gpu.queue.submit(std::iter::once(encoder.finish()));
        gpu.queue.present(frame);
    }

    fn init_gpu<S: GameScene + ?Sized>(
        &mut self,
        scene: &mut S,
        window: Arc<Window>,
    ) -> anyhow::Result<()> {
        let budget = self.args.frame_budget.is_some();
        let vsync = crate::startup::vsync_wanted(&self.args);
        let ten_bit = crate::startup::ten_bit_wanted(&self.args);
        let hdr = crate::startup::hdr_wanted(&self.args);
        #[cfg(not(target_arch = "wasm32"))]
        {
            let gpu = pollster::block_on(ShellGpu::open(window, budget, vsync, ten_bit, hdr))?;
            self.gpu_ready(scene, gpu, false);
        }
        #[cfg(target_arch = "wasm32")]
        {
            let _ = scene;
            let slot = self.gpu_pending.clone();
            wasm_bindgen_futures::spawn_local(async move {
                let wake = window.clone();
                let opened = ShellGpu::open(window, budget, vsync, ten_bit, hdr).await;
                *slot.borrow_mut() = Some(opened);
                wake.request_redraw();
            });
        }
        Ok(())
    }

    #[cfg(target_arch = "wasm32")]
    fn gpu_arrived<S: GameScene + ?Sized>(&mut self, scene: &mut S) {
        let Some(opened) = self.gpu_pending.borrow_mut().take() else {
            return;
        };
        match opened {
            Ok(gpu) => {
                self.gpu_ready(scene, gpu, true);
                let size = self.window.as_ref().map(|window| window.inner_size());
                let configured = self
                    .gpu
                    .as_ref()
                    .map(|gpu| (gpu.surface_config.width, gpu.surface_config.height));
                if let (Some(size), Some(configured)) = (size, configured)
                    && size.width > 0
                    && size.height > 0
                    && (size.width, size.height) != configured
                {
                    info!("web: the canvas is {}x{}", size.width, size.height);
                    self.resized(scene, size);
                }
            }
            Err(e) => error!("GPU initialization failure: {:?}", e),
        }
    }

    fn gpu_ready<S: GameScene + ?Sized>(&mut self, scene: &mut S, gpu: ShellGpu, late: bool) {
        self.gpu = Some(gpu);
        scene.on_gpu(self, late);
        self.fps_cap = crate::startup::fps_cap_wanted(&self.args);
        self.follow_monitor();
        if self.args.fps_report.is_some() {
            self.pacer
                .report_from(Instant::now() + Duration::from_secs(FPS_REPORT_WARM_UP));
        }
    }

    pub(super) fn follow_monitor(&mut self) {
        let (Some(window), Some(gpu)) = (&self.window, &self.gpu) else {
            return;
        };
        self.monitor_checked = Some(Instant::now());
        let monitor = window.current_monitor();
        let refresh = monitor
            .as_ref()
            .and_then(MonitorHandle::refresh_rate_millihertz);
        let seen = (monitor, refresh);
        if self.cap_monitor.as_ref() != Some(&seen) {
            self.cap_monitor = Some(seen);
            let budget = self.args.frame_budget.is_some();
            let vsync = fps_cap::vsync_paces(gpu.surface_config.present_mode);
            let (cap, source) = self.fps_cap;
            let choice = fps_cap::choose(cap, refresh, vsync, budget);
            let source = if budget { "--frame-budget" } else { source };
            let line = fps_cap::cap_line(cap, source, refresh, choice);
            if line != self.cap_line {
                eprintln!("{line}");
                self.cap_line = line;
            }
            self.pacer.set(choice);
        }
        self.follow_monitor_hdr();
    }

    fn follow_monitor_hdr(&mut self) {
        let (Some(window), Some(gpu)) = (&self.window, &mut self.gpu) else {
            return;
        };
        let Some(monitor_hdr) = crate::win_hdr::query_monitor_hdr(window) else {
            return;
        };
        let ten_bit = crate::startup::ten_bit_wanted(&self.args);
        let hdr = crate::startup::hdr_wanted(&self.args);
        let desired = window_surface::choose_surface_config(
            &gpu.surface_formats,
            ten_bit.0,
            hdr.0,
            Some(monitor_hdr),
        );
        if desired.format == gpu.surface_config.format
            && desired.color_space == gpu.surface_config.color_space
        {
            if desired.hdr_sdr_white != gpu.hdr_sdr_white {
                gpu.hdr_sdr_white = desired.hdr_sdr_white;
                if let Some(canvas) = &gpu.canvas {
                    canvas.set_hdr(&gpu.queue, desired.hdr_sdr_white);
                }
                eprintln!(
                    "{}",
                    window_surface::surface_choice_line(
                        desired,
                        ten_bit,
                        hdr,
                        &gpu.surface_formats
                    )
                );
            }
            return;
        }
        // Only switch surface format live when both old and new surfaces are canvas-backed,
        // keeping all scene and interface pipelines valid on CANVAS_FORMAT.
        if gpu.canvas.is_some()
            && window_surface::SurfaceFormats::choose(desired.format, false).canvas()
        {
            gpu.surface_config.format = desired.format;
            gpu.surface_config.color_space = desired.color_space;
            gpu.hdr_sdr_white = desired.hdr_sdr_white;
            gpu.surface.configure(&gpu.device, &gpu.surface_config);
            gpu.canvas = Some(mandrake_render::FrameCanvas::new(
                &gpu.device,
                desired.format,
                (gpu.surface_config.width, gpu.surface_config.height),
                desired.hdr_sdr_white,
            ));
            eprintln!(
                "{}",
                window_surface::surface_choice_line(desired, ten_bit, hdr, &gpu.surface_formats)
            );
        }
    }

    pub(super) fn frame_drawn(&mut self) {
        self.pacer.frame_drawn(Instant::now());
    }

    pub(super) fn pace_frame(&mut self, event_loop: &ActiveEventLoop) {
        if self
            .monitor_checked
            .is_some_and(|at| at.elapsed() >= MONITOR_CHECK)
        {
            self.follow_monitor();
        }
        let Some(window) = &self.window else {
            return;
        };
        let mut now = Instant::now();
        let (mut due, mut next) = self.pacer.poll(now);
        if let Some(deadline) = next.filter(|&d| !due && now >= Pacer::wake_at(d)) {
            Pacer::finish_wait(deadline);
            now = Instant::now();
            (due, next) = self.pacer.poll(now);
        }
        if due {
            window.request_redraw();
        }
        event_loop.set_control_flow(match next {
            Some(deadline) => ControlFlow::WaitUntil(Pacer::wake_at(deadline)),
            None => ControlFlow::Poll,
        });
    }

    pub(super) fn fps_report_done<S: GameScene + ?Sized>(
        &mut self,
        scene: &mut S,
        event_loop: &ActiveEventLoop,
    ) -> bool {
        let Some(secs) = self.args.fps_report else {
            return false;
        };
        let now = Instant::now();
        let done = self
            .pacer
            .report_since(now)
            .is_some_and(|since| (now - since).as_secs_f32() >= secs);
        if done {
            eprintln!("{}", self.pacer.report_line(now));
            self.quit(scene, event_loop);
        }
        done
    }

    fn signalled<S: GameScene + ?Sized>(
        &mut self,
        scene: &mut S,
        event_loop: &ActiveEventLoop,
    ) -> bool {
        let Some(sig) = super::guard::take_signal() else {
            return false;
        };
        error!(
            "mandrake: signal {sig}: the run ends here{}",
            match scene.game_tick() {
                Some(tick) => format!(", at game tick {tick}"),
                None => String::new(),
            }
        );
        self.quit(scene, event_loop);
        true
    }

    pub(super) fn quit<S: GameScene + ?Sized>(
        &mut self,
        scene: &mut S,
        event_loop: &ActiveEventLoop,
    ) {
        scene.finish();
        self.release_window(scene);
        event_loop.exit();
        #[cfg(target_arch = "wasm32")]
        crate::web::quit_to_page();
    }

    fn release_window<S: GameScene + ?Sized>(&mut self, scene: &mut S) {
        let gpu = self.gpu.take();
        let rest = gpu.map(|gpu| {
            let _ = gpu.device.poll(wgpu::PollType::wait_indefinitely());
            let ShellGpu {
                surface,
                queue,
                device,
                instance,
                ..
            } = gpu;
            drop(surface);
            (queue, device, instance)
        });
        if !scene.release_gpu() {
            self.scene_ended = true;
        }
        drop(rest);
        self.window = None;
    }

    pub(super) fn resized<S: GameScene + ?Sized>(
        &mut self,
        scene: &mut S,
        size: PhysicalSize<u32>,
    ) {
        #[cfg(target_arch = "wasm32")]
        if self.gpu.is_some() {
            self.web_pending_size = Some(size);
            return;
        }
        scene.on_resize(self, size);
    }

    #[cfg(target_arch = "wasm32")]
    pub(super) fn apply_pending_size<S: GameScene + ?Sized>(&mut self, scene: &mut S) {
        let Some(size) = self.web_pending_size.take() else {
            return;
        };
        if size.width == 0 || size.height == 0 {
            return;
        }
        let configured = self
            .gpu
            .as_ref()
            .map(|gpu| (gpu.surface_config.width, gpu.surface_config.height));
        if configured != Some((size.width, size.height)) {
            info!("web: the canvas is now {}x{}", size.width, size.height);
            scene.on_resize(self, size);
        }
    }

    pub(crate) fn configure_surface(&mut self, size: PhysicalSize<u32>) {
        if let Some(gpu) = &mut self.gpu {
            gpu.surface_config.width = size.width.max(1);
            gpu.surface_config.height = size.height.max(1);
            gpu.surface.configure(&gpu.device, &gpu.surface_config);
            if let Some(canvas) = &mut gpu.canvas {
                canvas.resize(
                    &gpu.device,
                    (gpu.surface_config.width, gpu.surface_config.height),
                );
            }
        }
    }

    fn toggle_fullscreen(&mut self) {
        let Some(window) = &self.window else {
            return;
        };
        if self.args.frame_budget.is_some() {
            info!("Fullscreen: not in a --frame-budget run");
            return;
        }
        let next = window
            .fullscreen()
            .is_none()
            .then_some(Fullscreen::Borderless(None));
        info!("Fullscreen: {}", if next.is_some() { "on" } else { "off" });
        window.set_fullscreen(next);
    }

    fn fullscreen_key(&mut self, event: &WindowEvent) -> bool {
        let WindowEvent::KeyboardInput {
            event:
                KeyEvent {
                    physical_key: PhysicalKey::Code(code),
                    state,
                    repeat,
                    ..
                },
            ..
        } = event
        else {
            return false;
        };
        let Some(key) = myth_key(*code) else {
            return false;
        };
        if !self
            .controls
            .toggles_fullscreen(key, binding_modifiers(self.modifiers))
        {
            return false;
        }
        if *state == ElementState::Pressed && !*repeat {
            self.toggle_fullscreen();
        }
        true
    }

    pub(crate) fn hidden(&self) -> bool {
        self.window_facts.occluded
    }

    #[cfg(not(target_arch = "wasm32"))]
    pub(crate) fn wake_on_net(&mut self, event_loop: &winit::event_loop::EventLoop<()>) {
        use std::sync::atomic::Ordering;
        let proxy = std::sync::Mutex::new(event_loop.create_proxy());
        let woken = self.net_woken.clone();
        self.net_waker = Some(std::sync::Arc::new(move || {
            if !woken.swap(true, Ordering::SeqCst)
                && let Ok(p) = proxy.lock()
            {
                let _ = p.send_event(());
            }
        }));
    }
}

impl ApplicationHandler for Shell {
    fn user_event(&mut self, _event_loop: &ActiveEventLoop, (): ()) {
        self.host
            .net_woken
            .store(false, std::sync::atomic::Ordering::SeqCst);
        self.scene.on_wake(&mut self.host);
    }

    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        let Self { host, scene } = self;
        if host.window.is_some() {
            return;
        }
        let Some(title) = scene.opening_title() else {
            return;
        };

        let window_attrs =
            web_canvas(Window::default_attributes()).with_title(printable_title(&title));
        #[cfg(not(target_arch = "wasm32"))]
        let window_attrs =
            window_attrs.with_inner_size(PhysicalSize::new(host.args.width, host.args.height));
        let window_attrs = window_attrs.with_fullscreen(
            opens_fullscreen(host.fullscreen_setting, &host.args)
                .then_some(Fullscreen::Borderless(None)),
        );
        let open_hidden = opens_hidden();
        let window_attrs = if open_hidden {
            window_attrs.with_visible(false)
        } else {
            window_attrs
        };

        match event_loop.create_window(window_attrs) {
            Ok(window) => {
                let window = Arc::new(window);
                host.window = Some(window.clone());
                scene.on_window(host, &window);
                if let Err(e) = host.init_gpu(scene.as_mut(), window.clone()) {
                    error!("GPU initialization failure: {:?}", e);
                }
                if open_hidden {
                    host.present_initial_black();
                    window.set_visible(true);
                }
            }
            Err(e) => {
                error!("Window creation failure: {:?}", e);
            }
        }
    }

    fn window_event(
        &mut self,
        event_loop: &ActiveEventLoop,
        _window_id: WindowId,
        event: WindowEvent,
    ) {
        let Self { host, scene } = self;
        let scene = scene.as_mut();
        #[cfg(target_arch = "wasm32")]
        host.gpu_arrived(scene);
        if !host.signalled(scene, event_loop) {
            if let WindowEvent::ModifiersChanged(modifiers) = &event {
                host.modifiers = modifiers.state();
            }
            host.window_facts.observe(&event);
            if !host.fullscreen_key(&event) {
                if matches!(
                    event,
                    WindowEvent::Moved(_) | WindowEvent::ScaleFactorChanged { .. }
                ) {
                    host.follow_monitor();
                }
                scene.on_event(host, event_loop, event);
            }
        }
        self.drop_ended_scene();
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        let Self { host, scene } = self;
        let scene = scene.as_mut();
        #[cfg(target_arch = "wasm32")]
        host.gpu_arrived(scene);
        super::guard::frame(scene.game_tick());
        if !host.signalled(scene, event_loop) {
            scene.on_wait(host, event_loop);
        }
        self.drop_ended_scene();
    }

    fn exiting(&mut self, _event_loop: &ActiveEventLoop) {
        self.host.release_window(self.scene.as_mut());
        self.drop_ended_scene();
    }
}

const TITLE_MAX_CHARS: usize = 200;

pub(crate) fn printable_title(title: &str) -> String {
    let mut out = String::with_capacity(title.len().min(TITLE_MAX_CHARS * 4));
    let mut chars = 0;
    for c in title.chars() {
        let c = match c {
            '\t' | '\n' | '\r' => ' ',
            c if c.is_control() => continue,
            c => c,
        };
        if chars == TITLE_MAX_CHARS {
            out.push_str("...");
            break;
        }
        out.push(c);
        chars += 1;
    }
    out
}

pub(crate) fn set_window_title(window: &Window, title: &str) {
    window.set_title(&printable_title(title));
}

#[cfg(test)]
mod tests {
    use super::*;
    use winit::dpi::PhysicalPosition;
    use winit::event::DeviceId;

    #[test]
    fn window_titles_are_printable_and_capped() {
        let stop = "stop at 1234: scen Some(FourCC('\0\0\0\''))\nnext\u{7}";
        let t = printable_title(stop);
        assert_eq!(t, "stop at 1234: scen Some(FourCC(''')) next");
        assert!(!t.chars().any(char::is_control));
        assert!(std::ffi::CString::new(t).is_ok());
        assert_eq!(printable_title("Myth II"), "Myth II");
        let long = "x".repeat(TITLE_MAX_CHARS + 50);
        let capped = printable_title(&long);
        assert_eq!(capped.chars().count(), TITLE_MAX_CHARS + 3);
        assert!(capped.ends_with("..."));
        assert_eq!(
            printable_title(&"y".repeat(TITLE_MAX_CHARS)),
            "y".repeat(TITLE_MAX_CHARS)
        );
    }

    #[test]
    fn fullscreen_follows_the_setting_but_not_in_a_frame_budget_run() {
        use clap::Parser;
        let args = Args::parse_from(["mandrake"]);
        assert!(opens_fullscreen(true, &args));
        assert!(!opens_fullscreen(false, &args));
        let budget = Args::parse_from(["mandrake", "--frame-budget", "5"]);
        assert!(!opens_fullscreen(true, &budget));
        assert!(!opens_fullscreen_on(true, &args, true));
        assert!(opens_fullscreen_on(true, &args, false));

        let alt = binding_modifiers(ModifiersState::ALT);
        assert_eq!(binding_modifiers(ModifiersState::SUPER), alt);
        assert!(alt.alt && !alt.ctrl && !alt.shift);
        let controls = Controls::defaults();
        let f11 = myth_key(winit::keyboard::KeyCode::F11).unwrap();
        assert!(controls.toggles_fullscreen(f11, alt));
        assert!(!controls.toggles_fullscreen(f11, binding_modifiers(ModifiersState::empty())));
    }

    #[test]
    fn the_cursor_grab_needs_fullscreen_and_the_focus() {
        assert!(cursor_grab(true, true, true));
        assert!(!cursor_grab(false, true, true));
        assert!(!cursor_grab(true, false, true));
        assert!(!cursor_grab(true, true, false));
    }

    #[test]
    fn the_windows_facts_follow_its_events() {
        let device_id = DeviceId::dummy();
        let mut facts = WindowFacts::default();
        assert_eq!(facts.cursor, (0.0, 0.0));
        assert!(facts.focused && !facts.occluded && !facts.cursor_in_window);
        for event in [
            WindowEvent::Focused(false),
            WindowEvent::Occluded(true),
            WindowEvent::CursorMoved {
                device_id,
                position: PhysicalPosition::new(300.5, 200.0),
            },
        ] {
            facts.observe(&event);
        }
        assert_eq!(
            facts,
            WindowFacts {
                focused: false,
                occluded: true,
                cursor_in_window: true,
                cursor: (300.5, 200.0),
            }
        );
        facts.observe(&WindowEvent::CursorLeft { device_id });
        assert!(!facts.cursor_in_window);
        assert_eq!(facts.cursor, (300.5, 200.0), "the last place it was seen");
        facts.observe(&WindowEvent::RedrawRequested);
        assert!(!facts.focused);
    }

    #[test]
    fn windows_opens_hidden_until_first_clear() {
        assert!(opens_hidden_on("windows"));
        for os in ["linux", "macos", "freebsd", "unknown"] {
            assert!(!opens_hidden_on(os), "{os} should open visible");
        }
    }
}
