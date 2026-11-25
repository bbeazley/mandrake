use super::shell::ShellHost;
use crate::audio_out::AudioLink;
use crate::window_surface::SurfaceFormats;
use std::sync::Arc;
use winit::dpi::PhysicalSize;
use winit::event::WindowEvent;
use winit::event_loop::ActiveEventLoop;
use winit::window::Window;

pub(crate) struct SceneFrame<'a> {
    pub device: &'a wgpu::Device,
    pub queue: &'a wgpu::Queue,
    pub encoder: &'a mut wgpu::CommandEncoder,
    pub color: &'a wgpu::TextureView,
    pub interface: &'a wgpu::TextureView,
    pub size: (u32, u32),
    pub formats: SurfaceFormats,
    pub ui: crate::ui_scale::UiScale,
}

pub(crate) trait GameScene {
    fn opening_title(&self) -> Option<String>;

    fn finish(&mut self);

    fn hides_system_cursor(&self) -> bool {
        false
    }

    fn on_window(&mut self, _host: &ShellHost, window: &Arc<Window>) {
        if self.hides_system_cursor() {
            window.set_cursor_visible(false);
        }
    }

    fn on_gpu(&mut self, _host: &ShellHost, _late: bool) {}

    fn on_resize(&mut self, host: &mut ShellHost, size: PhysicalSize<u32>) {
        host.configure_surface(size);
    }

    fn on_event(&mut self, host: &mut ShellHost, event_loop: &ActiveEventLoop, event: WindowEvent);

    fn on_wait(&mut self, host: &mut ShellHost, event_loop: &ActiveEventLoop);

    fn on_wake(&mut self, _host: &mut ShellHost) {}

    fn game_tick(&self) -> Option<u32> {
        None
    }

    fn release_gpu(&mut self) -> bool {
        false
    }
}

pub(crate) trait FrameScene: GameScene {
    fn title(&self) -> String;

    fn window_event(&mut self, event: &WindowEvent, size: (u32, u32));

    fn frame(&mut self, frame: SceneFrame<'_>);

    fn wants_exit(&self) -> bool;

    fn submitted(&mut self, _submit_ms: f32, _present_ms: f32) {}

    fn wants_cursor_grab(&self) -> bool {
        false
    }

    fn shot_info(&self, size: (u32, u32)) -> crate::screenshot::ShotInfo;

    fn print_line(&mut self, _line: &str) {}
}

pub(crate) struct Ended;

impl GameScene for Ended {
    fn opening_title(&self) -> Option<String> {
        None
    }
    fn finish(&mut self) {}
    fn on_event(&mut self, _: &mut ShellHost, _: &ActiveEventLoop, _: WindowEvent) {}
    fn on_wait(&mut self, _: &mut ShellHost, _: &ActiveEventLoop) {}
    fn release_gpu(&mut self) -> bool {
        true
    }
}

pub(crate) struct SceneSetup {
    pub audio: Option<AudioLink>,
    pub view: super::view_settings::ViewSettings,
    pub menu_filter: mandrake_view::interface_picture::InterfaceFilter,
    pub cutscene_filter: mandrake_view::interface_picture::InterfaceFilter,
}
