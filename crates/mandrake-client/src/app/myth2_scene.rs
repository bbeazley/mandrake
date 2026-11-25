use crate::app::ClientApp;
use crate::app::frame_budget;
use crate::app::game_scene::GameScene;
use crate::app::recording::FilmFile;
use crate::app::shell::{MENU_HIDDEN_WAKE, ShellHost, set_window_title};
use crate::app::shell_game::{LoadFlow, LoadTimes, MapRun, Rebuilds};
use crate::app::shell_menu::{MENU_TITLE, Menu};
use crate::args;
use crate::audio_out::AudioOutput;
use crate::clock::Instant;
use crate::startup::Startup;
use std::sync::Arc;
use std::time::Duration;
use winit::dpi::PhysicalSize;
use winit::event::WindowEvent;
use winit::event_loop::{ActiveEventLoop, ControlFlow};
use winit::window::Window;

const NET_HIDDEN_WAKE: Duration = Duration::from_millis(5);

pub(crate) struct Myth2Scene {
    pub(crate) pending_net: Option<crate::net_game::NetPlay>,
    #[cfg(not(target_arch = "wasm32"))]
    pub(super) connecting: Option<crate::net_game::Connecting>,
    #[cfg(target_arch = "wasm32")]
    pub(super) connecting: Option<crate::web::Connecting>,
    pub(super) net_clock: Instant,
    pub(super) net_announced: bool,
    #[cfg(target_arch = "wasm32")]
    web_frames: u64,
    pub(crate) app: Option<ClientApp>,
    pub(super) menu: Option<Menu>,
    pub(super) load: LoadFlow,
    pub(super) pending_map: Option<Startup>,
    pub(super) pending_args: Option<crate::args::Args>,
    pub(super) load_times: LoadTimes,
    pub(super) started: Instant,
    pub(crate) rebuilds: Option<Rebuilds>,
    pub(super) front_end_had_input: bool,
    pub(super) dialog_ui: Option<(u32, u32)>,
    pub(super) modeless_press: bool,
    pub(super) budget_dialog: Option<Menu>,
    pub(super) console_cache: Vec<crate::console::ConsoleLine>,
}

impl Myth2Scene {
    pub(crate) fn new() -> Self {
        Self {
            pending_net: None,
            connecting: None,
            net_clock: Instant::now(),
            net_announced: false,
            #[cfg(target_arch = "wasm32")]
            web_frames: 0,
            app: None,
            menu: None,
            load: LoadFlow::default(),
            pending_map: None,
            pending_args: None,
            load_times: LoadTimes::default(),
            started: Instant::now(),
            rebuilds: None,
            front_end_had_input: true,
            dialog_ui: None,
            modeless_press: false,
            budget_dialog: None,
            console_cache: Vec::new(),
        }
    }

    pub(crate) fn start_map(&mut self, host: &mut ShellHost, start: Startup, run: MapRun) {
        if host.window().is_none() {
            host.fullscreen_setting = start.interface.fullscreen;
        }
        let mut app = ClientApp::for_map(host, &host.args, start, run);
        if let Some(net) = self.pending_net.take() {
            app.net_strings = crate::net_game::networking_strings(&host.args.archive);
            app.net = Some(net);
        }
        if run == MapRun::Direct
            && args::records_direct_game(&host.args)
            && let Some(file) = FilmFile::for_archive(&host.args.archive)
        {
            let game_dir = file.game_dir().to_path_buf();
            app.start_recording(&host.args, &game_dir, Some(file));
        }
        self.app = Some(app);
    }

    pub(crate) fn start_menu(&mut self, host: &mut ShellHost) -> anyhow::Result<()> {
        let link = host.audio.as_ref().map(AudioOutput::link);
        let menu = Menu::new(&host.args, link)?;
        if host.window().is_none() {
            host.fullscreen_setting = menu.config.fullscreen;
        }
        self.menu = Some(menu);
        Ok(())
    }

    pub(super) fn attach_map(&mut self, host: &ShellHost) {
        let (Some(app), Some(window), Some(gpu)) = (&mut self.app, host.window(), host.gpu())
        else {
            return;
        };
        app.attach_window(window.clone(), host.window_facts);
        set_window_title(window, &app.title());
        app.attach_gpu(gpu);
    }

    fn after_map_event(&mut self, host: &mut ShellHost, event_loop: &ActiveEventLoop) {
        let Some(app) = self.app.as_mut() else {
            return;
        };
        host.ui_mode = app.ui_mode;
        let granted = app.granted_window_size.take();
        let quit = std::mem::take(&mut app.quit_requested);
        let map_over = app.map_over.take();
        if let Some(size) = granted {
            host.resized(self, size);
        }
        if quit {
            host.quit(self, event_loop);
        } else if let Some(report) = map_over {
            if self.menu.is_some() {
                self.map_over(host, report, event_loop);
            } else {
                host.quit(self, event_loop);
            }
        }
    }

    fn redraw(&mut self, host: &mut ShellHost, event_loop: &ActiveEventLoop) {
        #[cfg(target_arch = "wasm32")]
        {
            self.web_frames += 1;
            if self.web_frames.is_power_of_two() || self.web_frames.is_multiple_of(1800) {
                tracing::info!(
                    "web: frame {}, gpu {}, tick {:?}",
                    self.web_frames,
                    host.gpu().is_some(),
                    self.app.as_ref().map(|app| app.sim.current_tick)
                );
            }
        }
        #[cfg(target_arch = "wasm32")]
        if host.gpu().is_none() {
            if let Some(app) = self.app.as_mut() {
                app.hold_clock();
            }
            return;
        }
        #[cfg(target_arch = "wasm32")]
        host.apply_pending_size(self);
        host.frame_drawn();
        if self.menu.is_some() {
            self.session_frame(host, event_loop, true);
        } else if let Some(app) = self.app.as_mut() {
            let dialog = frame_budget::overlay_frame(&mut self.budget_dialog, &host.args, app);
            app.redraw(host.gpu().map(|gpu| gpu.frame_surface()), dialog);
        }
    }

    fn input(&mut self, host: &mut ShellHost, event_loop: &ActiveEventLoop, event: WindowEvent) {
        if let Some(rect) = self.menu.as_ref().and_then(|menu| menu.modeless_rect()) {
            self.modeless_event(host, event, rect);
            self.after_map_event(host, event_loop);
            return;
        }
        match self.menu.as_mut().filter(|menu| menu.has_input()) {
            Some(menu) => {
                let dialog = menu.overlay_open();
                let over_map = self.app.as_mut().filter(|_| dialog);
                let mapped = over_map.as_ref().and_then(|a| a.event_for_dialog(&event));
                menu.window_event(mapped.as_ref().unwrap_or(&event), host.modifiers);
                if let Some(app) = over_map {
                    app.key_under_dialog(&event, host.modifiers);
                }
                if let (WindowEvent::Occluded(_), Some(app)) = (&event, self.app.as_mut()) {
                    app.window_event(event);
                }
            }
            None => {
                if let Some(app) = self.app.as_mut() {
                    app.window_event(event);
                }
            }
        }
        self.after_map_event(host, event_loop);
    }
}

impl GameScene for Myth2Scene {
    fn opening_title(&self) -> Option<String> {
        match &self.app {
            Some(app) => Some(app.title()),
            None if self.menu.is_some() || self.rebuilds.is_some() => Some(MENU_TITLE.to_owned()),
            None => None,
        }
    }

    fn finish(&mut self) {
        if let Some(app) = self.app.as_mut() {
            app.finish_recording();
        }
    }

    fn on_window(&mut self, host: &ShellHost, window: &Arc<Window>) {
        if let Some(app) = self.app.as_mut() {
            app.attach_window(window.clone(), host.window_facts);
        }
    }

    fn on_gpu(&mut self, host: &ShellHost, late: bool) {
        if let (Some(app), Some(gpu)) = (self.app.as_mut(), host.gpu()) {
            app.attach_gpu(gpu);
            if late {
                app.hold_clock();
            }
        }
    }

    fn on_resize(&mut self, host: &mut ShellHost, size: PhysicalSize<u32>) {
        if let Some(app) = self.app.as_mut() {
            app.set_screen_size(size.width, size.height);
        }
        host.configure_surface(size);
        if let Some(app) = self.app.as_mut() {
            app.gpu_resized(size);
        }
    }

    fn on_event(&mut self, host: &mut ShellHost, event_loop: &ActiveEventLoop, event: WindowEvent) {
        match event {
            WindowEvent::CloseRequested => host.quit(self, event_loop),
            WindowEvent::Resized(new_size) => {
                host.resized(self, new_size);
                host.follow_monitor();
            }
            WindowEvent::RedrawRequested => self.redraw(host, event_loop),
            event => self.input(host, event_loop, event),
        }
    }

    fn on_wait(&mut self, host: &mut ShellHost, event_loop: &ActiveEventLoop) {
        self.wire_net_waker(host);
        if self.rebuilds.is_some() {
            self.rebuild_step(host, event_loop);
        }
        if host.fps_report_done(self, event_loop) {
            return;
        }
        let map_drives = self.app.is_some() && self.load != LoadFlow::AttachGpu;
        let map_frame = self
            .app
            .as_mut()
            .filter(|_| map_drives)
            .is_some_and(|app| app.about_to_wait(event_loop));
        if map_frame {
            host.pace_frame(event_loop);
        }
        if self.menu.is_some() {
            if host.window_facts.occluded {
                if !map_drives {
                    let woken = host.net_waker.is_some();
                    let wake = if self
                        .pending_net
                        .as_ref()
                        .is_some_and(|net| net.is_host() || !woken)
                    {
                        NET_HIDDEN_WAKE
                    } else {
                        MENU_HIDDEN_WAKE
                    };
                    event_loop.set_control_flow(ControlFlow::WaitUntil(Instant::now() + wake));
                }
                self.session_frame(host, event_loop, false);
            } else if !map_drives {
                host.pace_frame(event_loop);
            }
        }
        self.after_map_event(host, event_loop);
    }

    fn on_wake(&mut self, host: &mut ShellHost) {
        self.net_wake(host);
    }

    fn game_tick(&self) -> Option<u32> {
        self.app.as_ref().map(|app| app.sim.current_tick)
    }

    fn release_gpu(&mut self) -> bool {
        if let Some(app) = self.app.as_mut() {
            app.release_window();
        }
        self.menu = None;
        self.budget_dialog = None;
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn release_gpu_clears_menu_and_budget_dialog() {
        let mut scene = Myth2Scene::new();
        assert!(scene.release_gpu());
        assert!(scene.menu.is_none());
        assert!(scene.budget_dialog.is_none());
    }
}
