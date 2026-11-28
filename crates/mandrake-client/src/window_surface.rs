use std::sync::Arc;

use tracing::{debug, error, warn};
use winit::window::Window;

#[cfg_attr(target_arch = "wasm32", allow(dead_code))]
pub fn backend_order(os: &str, env: Option<wgpu::Backends>) -> Vec<wgpu::Backends> {
    let mut order = match env {
        Some(backends) => vec![backends],
        None if os == "windows" => vec![wgpu::Backends::DX12, wgpu::Backends::VULKAN],
        None => vec![wgpu::Backends::PRIMARY],
    };
    if !order.iter().any(|b| b.contains(wgpu::Backends::GL)) {
        order.push(wgpu::Backends::GL);
    }
    order
}

pub struct WindowSurface {
    pub instance: wgpu::Instance,
    pub surface: wgpu::Surface<'static>,
    pub adapter: wgpu::Adapter,
}

pub async fn create_window_surface(window: Arc<Window>) -> anyhow::Result<WindowSurface> {
    #[cfg(not(target_arch = "wasm32"))]
    let order = backend_order(std::env::consts::OS, wgpu::Backends::from_env());
    #[cfg(target_arch = "wasm32")]
    let order = [wgpu::Backends::BROWSER_WEBGPU];
    let mut failures: Vec<String> = Vec::new();
    for (i, &backends) in order.iter().enumerate() {
        match try_backends(backends, window.clone()).await {
            Ok(found) => return Ok(found),
            Err(e) => {
                if let Some(next) = order.get(i + 1) {
                    warn!(
                        "No {backends:?} adapter for the window ({e:#}); falling back to {next:?}"
                    );
                }
                failures.push(format!("{backends:?}: {e:#}"));
            }
        }
    }
    Err(anyhow::anyhow!(
        "no backend can present to the window ({})",
        failures.join("; ")
    ))
}

async fn try_backends(
    backends: wgpu::Backends,
    window: Arc<Window>,
) -> anyhow::Result<WindowSurface> {
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
        backends,
        display: Some(Box::new(window.clone())),
        ..wgpu::InstanceDescriptor::new_without_display_handle()
    });
    let surface = instance.create_surface(window)?;
    let adapter = instance
        .request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            compatible_surface: Some(&surface),
            force_fallback_adapter: false,
            apply_limit_buckets: false,
        })
        .await
        .map_err(|e| anyhow::anyhow!("no compatible {backends:?} adapter for the surface: {e}"))?;
    let adapter_info = adapter.get_info();
    let (line, software) = adapter_line(&adapter_info);
    eprintln!("{line}");
    if software {
        warn!(
            "The window draws on a CPU adapter ({}): expect a low frame rate. Is the GPU's \
             driver loaded (a reboot after a driver update, or the Vulkan ICDs in the shell)?",
            adapter_info.name
        );
    }
    Ok(WindowSurface {
        instance,
        surface,
        adapter,
    })
}

fn adapter_line(info: &wgpu::AdapterInfo) -> (String, bool) {
    let name = info.name.to_ascii_lowercase();
    let software = info.device_type == wgpu::DeviceType::Cpu
        || ["llvmpipe", "lavapipe", "softpipe", "swiftshader"]
            .iter()
            .any(|s| name.contains(s));
    let line = format!(
        "window adapter: {} ({:?}, {:?}, driver {})",
        info.name, info.backend, info.device_type, info.driver_info
    );
    (line, software)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SurfaceErrorAction {
    Reconfigure,
    SkipTimeout,
    Report,
}

impl SurfaceErrorAction {
    pub fn of(status: &wgpu::CurrentSurfaceTexture) -> Option<Self> {
        match status {
            wgpu::CurrentSurfaceTexture::Success(_)
            | wgpu::CurrentSurfaceTexture::Suboptimal(_) => None,
            wgpu::CurrentSurfaceTexture::Lost | wgpu::CurrentSurfaceTexture::Outdated => {
                Some(Self::Reconfigure)
            }
            wgpu::CurrentSurfaceTexture::Timeout | wgpu::CurrentSurfaceTexture::Occluded => {
                Some(Self::SkipTimeout)
            }
            wgpu::CurrentSurfaceTexture::Validation => Some(Self::Report),
        }
    }
}

#[derive(Debug, Default)]
pub struct SurfaceTimeoutLog {
    burst: u32,
}

impl SurfaceTimeoutLog {
    pub fn timeout(&mut self) {
        self.burst += 1;
        if self.burst == 1 {
            debug!("Surface acquire timed out (window hidden or occluded?); skipping frames");
        }
    }

    pub fn presented(&mut self) -> Option<u32> {
        let burst = std::mem::take(&mut self.burst);
        if burst > 0 {
            debug!("Surface acquire recovered after {burst} timed-out frame(s)");
            Some(burst)
        } else {
            None
        }
    }
}

pub fn handle_surface_error(
    status: &wgpu::CurrentSurfaceTexture,
    surface: &wgpu::Surface<'_>,
    device: &wgpu::Device,
    config: &wgpu::SurfaceConfiguration,
    timeouts: &mut SurfaceTimeoutLog,
) {
    match SurfaceErrorAction::of(status) {
        Some(SurfaceErrorAction::Reconfigure) => surface.configure(device, config),
        Some(SurfaceErrorAction::SkipTimeout) => timeouts.timeout(),
        Some(SurfaceErrorAction::Report) => error!("Surface error: {status:?}"),
        None => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_software_adapter_is_told_apart() {
        let info = |name: &str, device_type| wgpu::AdapterInfo {
            name: name.into(),
            device_type,
            driver_info: "590".into(),
            ..wgpu::AdapterInfo::new(wgpu::DeviceType::Other, wgpu::Backend::Vulkan)
        };
        let (line, software) = adapter_line(&info(
            "NVIDIA GeForce RTX 5070 Ti",
            wgpu::DeviceType::DiscreteGpu,
        ));
        assert!(!software);
        assert_eq!(
            line,
            "window adapter: NVIDIA GeForce RTX 5070 Ti (Vulkan, DiscreteGpu, driver 590)"
        );
        assert!(
            adapter_line(&info(
                "llvmpipe (LLVM 20.1.8, 256 bits)",
                wgpu::DeviceType::Cpu
            ))
            .1
        );
        assert!(
            adapter_line(&info(
                "llvmpipe (LLVM 20.1.8, 256 bits)",
                wgpu::DeviceType::Other
            ))
            .1
        );
    }

    #[test]
    fn surface_timeout_is_a_quiet_skipped_frame() {
        use wgpu::CurrentSurfaceTexture as Status;
        for (status, action) in [
            (Status::Timeout, SurfaceErrorAction::SkipTimeout),
            (Status::Occluded, SurfaceErrorAction::SkipTimeout),
            (Status::Lost, SurfaceErrorAction::Reconfigure),
            (Status::Outdated, SurfaceErrorAction::Reconfigure),
            (Status::Validation, SurfaceErrorAction::Report),
        ] {
            assert_eq!(SurfaceErrorAction::of(&status), Some(action));
        }
    }

    #[test]
    fn surface_timeouts_log_once_per_burst() {
        let mut log = SurfaceTimeoutLog::default();
        assert_eq!(log.presented(), None);
        for _ in 0..5 {
            log.timeout();
        }
        assert_eq!(log.burst, 5);
        assert_eq!(log.presented(), Some(5));
        assert_eq!(log.presented(), None);
        log.timeout();
        assert_eq!(log.presented(), Some(1));
    }

    #[test]
    fn this_os_tries_gl_only_last() {
        let order = backend_order(std::env::consts::OS, None);
        assert!(!order[0].contains(wgpu::Backends::GL));
        assert_eq!(order.last(), Some(&wgpu::Backends::GL));
    }

    #[test]
    fn windows_tries_dx12_then_vulkan_then_gl() {
        use wgpu::Backends as B;
        assert_eq!(
            backend_order("windows", None),
            vec![B::DX12, B::VULKAN, B::GL]
        );
    }

    #[test]
    fn other_oses_try_primary_then_gl() {
        use wgpu::Backends as B;
        for os in ["linux", "macos", "freebsd", "android"] {
            assert_eq!(backend_order(os, None), vec![B::PRIMARY, B::GL], "{os}");
        }
    }

    #[test]
    fn wgpu_backend_overrides_the_os_order() {
        use wgpu::Backends as B;
        for os in ["windows", "linux"] {
            assert_eq!(backend_order(os, Some(B::VULKAN)), vec![B::VULKAN, B::GL]);
            assert_eq!(backend_order(os, Some(B::GL)), vec![B::GL]);
            assert_eq!(
                backend_order(os, Some(B::DX12 | B::GL)),
                vec![B::DX12 | B::GL]
            );
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SurfaceFormats {
    pub surface: wgpu::TextureFormat,
    pub scene: wgpu::TextureFormat,
    pub interface: wgpu::TextureFormat,
}

impl SurfaceFormats {
    pub fn choose(preferred: wgpu::TextureFormat, view_formats: bool) -> Self {
        if matches!(
            preferred,
            wgpu::TextureFormat::Rgb10a2Unorm | wgpu::TextureFormat::Rgba16Float
        ) {
            return Self {
                surface: preferred,
                scene: mandrake_render::CANVAS_FORMAT,
                interface: mandrake_render::CANVAS_FORMAT,
            };
        }
        let pick = |other: wgpu::TextureFormat| if view_formats { other } else { preferred };
        Self {
            surface: preferred,
            scene: pick(preferred.add_srgb_suffix()),
            interface: pick(preferred.remove_srgb_suffix()),
        }
    }

    pub fn of(config: &wgpu::SurfaceConfiguration) -> Self {
        Self::choose(
            config.format,
            config.view_formats.iter().any(|&f| {
                f != config.format && f.remove_srgb_suffix() == config.format.remove_srgb_suffix()
            }),
        )
    }

    pub fn view_formats(&self) -> Vec<wgpu::TextureFormat> {
        [self.scene, self.interface]
            .into_iter()
            .filter(|&f| {
                f != self.surface && f.remove_srgb_suffix() == self.surface.remove_srgb_suffix()
            })
            .collect()
    }

    pub fn canvas(&self) -> bool {
        matches!(
            self.surface,
            wgpu::TextureFormat::Rgb10a2Unorm | wgpu::TextureFormat::Rgba16Float
        )
    }
}

#[derive(Clone, Copy)]
pub struct FrameSurface<'a> {
    pub surface: &'a wgpu::Surface<'static>,
    pub canvas: Option<&'a mandrake_render::FrameCanvas>,
    pub config: &'a wgpu::SurfaceConfiguration,
}

pub fn frame_texture<'a>(
    canvas: Option<&'a mandrake_render::FrameCanvas>,
    frame: &'a wgpu::SurfaceTexture,
) -> &'a wgpu::Texture {
    canvas.map_or(&frame.texture, |c| c.texture())
}

pub fn screenshot_texture<'a>(
    canvas: Option<&'a mandrake_render::FrameCanvas>,
    frame: &'a wgpu::SurfaceTexture,
    color_space: wgpu::SurfaceColorSpace,
) -> &'a wgpu::Texture {
    if color_space == wgpu::SurfaceColorSpace::ExtendedSrgbLinear {
        frame_texture(canvas, frame)
    } else {
        &frame.texture
    }
}

pub fn present_canvas(
    canvas: Option<&mandrake_render::FrameCanvas>,
    encoder: &mut wgpu::CommandEncoder,
    frame: &wgpu::SurfaceTexture,
) {
    if let Some(canvas) = canvas {
        let view = frame.texture.create_view(&Default::default());
        canvas.encode_present(encoder, &view);
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SurfaceChoice {
    pub format: wgpu::TextureFormat,
    pub color_space: wgpu::SurfaceColorSpace,
    pub hdr_sdr_white: Option<f32>,
}

pub fn hdr_sdr_white_wanted(
    mode: crate::args::HdrMode,
    monitor_hdr: Option<crate::win_hdr::MonitorHdrInfo>,
) -> Option<f32> {
    match mode {
        crate::args::HdrMode::Off => None,
        crate::args::HdrMode::Auto => monitor_hdr
            .filter(|info| info.hdr_active)
            .map(|info| info.sdr_white_scale),
        crate::args::HdrMode::On => Some(monitor_hdr.map_or(1.0, |info| info.sdr_white_scale)),
    }
}

pub fn choose_surface_config(
    listed: &[wgpu::TextureFormat],
    ten_bit: bool,
    hdr_mode: crate::args::HdrMode,
    monitor_hdr: Option<crate::win_hdr::MonitorHdrInfo>,
) -> SurfaceChoice {
    if let Some(sdr_white) = hdr_sdr_white_wanted(hdr_mode, monitor_hdr)
        && listed.contains(&wgpu::TextureFormat::Rgba16Float)
    {
        return SurfaceChoice {
            format: wgpu::TextureFormat::Rgba16Float,
            color_space: wgpu::SurfaceColorSpace::ExtendedSrgbLinear,
            hdr_sdr_white: Some(sdr_white),
        };
    }
    SurfaceChoice {
        format: choose_surface_format(listed, ten_bit),
        color_space: wgpu::SurfaceColorSpace::Auto,
        hdr_sdr_white: None,
    }
}

pub fn choose_surface_format(listed: &[wgpu::TextureFormat], ten_bit: bool) -> wgpu::TextureFormat {
    let ten = wgpu::TextureFormat::Rgb10a2Unorm;
    if ten_bit && listed.contains(&ten) {
        return ten;
    }
    listed
        .iter()
        .copied()
        .find(|f| f.is_srgb())
        .unwrap_or(listed[0])
}

pub fn surface_choice_line(
    choice: SurfaceChoice,
    ten_bit: (bool, &str),
    hdr: (crate::args::HdrMode, &str),
    listed: &[wgpu::TextureFormat],
) -> String {
    if let Some(scale) = choice.hdr_sdr_white {
        let nits = crate::win_hdr::sdr_white_nits(scale);
        let why = match hdr.0 {
            crate::args::HdrMode::Auto => format!("HDR auto by {}", hdr.1),
            crate::args::HdrMode::On => format!("HDR on by {}", hdr.1),
            crate::args::HdrMode::Off => format!("HDR by {}", hdr.1),
        };
        return format!(
            "surface format: {:?} (HDR scRGB, {nits} nits white; {why}) of {listed:?}",
            choice.format
        );
    }
    surface_format_line(choice.format, ten_bit, listed)
}

pub fn surface_format_line(
    chosen: wgpu::TextureFormat,
    (ten_bit, why): (bool, &str),
    listed: &[wgpu::TextureFormat],
) -> String {
    let bits = if mandrake_render::is_ten_bit(chosen) {
        "10-bit"
    } else {
        "8-bit"
    };
    let wanted = match (ten_bit, mandrake_render::is_ten_bit(chosen)) {
        (true, true) => format!("10-bit wanted by {why}"),
        (true, false) => format!("10-bit wanted by {why}, not offered"),
        (false, _) => format!("8-bit wanted by {why}"),
    };
    format!("surface format: {chosen:?} ({bits}; {wanted}) of {listed:?}")
}

#[cfg(test)]
mod surface_format_tests {
    use super::{SurfaceFormats, choose_surface_format, surface_format_line};
    use wgpu::TextureFormat as F;

    #[test]
    fn ten_bit_is_taken_where_listed_and_wanted() {
        let nvidia = [
            F::Bgra8UnormSrgb,
            F::Bgra8Unorm,
            F::Rgb10a2Unorm,
            F::Rgba16Float,
        ];
        assert_eq!(choose_surface_format(&nvidia, true), F::Rgb10a2Unorm);
        assert_eq!(choose_surface_format(&nvidia, false), F::Bgra8UnormSrgb);
        let eight = [F::Bgra8Unorm, F::Bgra8UnormSrgb];
        assert_eq!(choose_surface_format(&eight, true), F::Bgra8UnormSrgb);
        let web = [F::Bgra8Unorm];
        assert_eq!(choose_surface_format(&web, true), F::Bgra8Unorm);
        assert_eq!(
            surface_format_line(F::Rgb10a2Unorm, (true, "the default"), &[F::Rgb10a2Unorm]),
            "surface format: Rgb10a2Unorm (10-bit; 10-bit wanted by the default) of [Rgb10a2Unorm]"
        );
        assert!(
            surface_format_line(F::Bgra8UnormSrgb, (true, "the default"), &eight)
                .contains("8-bit; 10-bit wanted by the default, not offered")
        );
        assert!(
            surface_format_line(F::Bgra8UnormSrgb, (false, "--ten-bit"), &nvidia)
                .contains("8-bit wanted by --ten-bit")
        );
    }

    #[test]
    fn a_ten_bit_surface_is_drawn_through_its_canvas() {
        for views in [true, false] {
            let f = SurfaceFormats::choose(F::Rgb10a2Unorm, views);
            assert_eq!((f.scene, f.interface), (F::Rgba16Float, F::Rgba16Float));
            assert!(f.view_formats().is_empty());
            assert!(f.canvas());
        }
        assert!(!SurfaceFormats::choose(F::Bgra8UnormSrgb, true).canvas());
    }

    #[test]
    fn native_srgb_surface_keeps_its_linear_interface_view() {
        let f = SurfaceFormats::choose(F::Bgra8UnormSrgb, true);
        assert_eq!((f.scene, f.interface), (F::Bgra8UnormSrgb, F::Bgra8Unorm));
        assert_eq!(f.view_formats(), vec![F::Bgra8Unorm]);
    }

    #[test]
    fn web_linear_surface_renders_the_world_into_an_srgb_view() {
        let f = SurfaceFormats::choose(F::Bgra8Unorm, true);
        assert_eq!((f.scene, f.interface), (F::Bgra8UnormSrgb, F::Bgra8Unorm));
        assert_eq!(f.view_formats(), vec![F::Bgra8UnormSrgb]);
    }

    #[test]
    fn without_view_formats_everything_is_the_surface() {
        let f = SurfaceFormats::choose(F::Bgra8Unorm, false);
        assert_eq!((f.scene, f.interface), (F::Bgra8Unorm, F::Bgra8Unorm));
        assert!(f.view_formats().is_empty());
    }

    #[test]
    fn a_configured_surface_gives_back_its_formats() {
        for (preferred, views) in [
            (F::Bgra8Unorm, true),
            (F::Rgba8UnormSrgb, true),
            (F::Bgra8Unorm, false),
        ] {
            let f = SurfaceFormats::choose(preferred, views);
            let config = wgpu::SurfaceConfiguration {
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
                format: f.surface,
                width: 1,
                height: 1,
                present_mode: wgpu::PresentMode::Fifo,
                alpha_mode: wgpu::CompositeAlphaMode::Auto,
                color_space: wgpu::SurfaceColorSpace::Auto,
                view_formats: f.view_formats(),
                desired_maximum_frame_latency: 2,
            };
            assert_eq!(SurfaceFormats::of(&config), f);
        }
    }

    #[test]
    fn hdr_surface_selection_and_canvas_backing() {
        use super::{SurfaceChoice, choose_surface_config, surface_choice_line};
        use crate::args::HdrMode;
        use crate::win_hdr::MonitorHdrInfo;

        let listed = [
            F::Bgra8UnormSrgb,
            F::Bgra8Unorm,
            F::Rgb10a2Unorm,
            F::Rgba16Float,
        ];
        let hdr_on = Some(MonitorHdrInfo {
            hdr_active: true,
            sdr_white_scale: 3.0,
        });
        let hdr_off = Some(MonitorHdrInfo {
            hdr_active: false,
            sdr_white_scale: 1.0,
        });

        assert_eq!(
            choose_surface_config(&listed, true, HdrMode::Auto, hdr_on),
            SurfaceChoice {
                format: F::Rgba16Float,
                color_space: wgpu::SurfaceColorSpace::ExtendedSrgbLinear,
                hdr_sdr_white: Some(3.0),
            }
        );
        assert_eq!(
            choose_surface_config(&listed, true, HdrMode::Auto, hdr_off),
            SurfaceChoice {
                format: F::Rgb10a2Unorm,
                color_space: wgpu::SurfaceColorSpace::Auto,
                hdr_sdr_white: None,
            }
        );
        assert_eq!(
            choose_surface_config(&listed, true, HdrMode::Auto, None),
            SurfaceChoice {
                format: F::Rgb10a2Unorm,
                color_space: wgpu::SurfaceColorSpace::Auto,
                hdr_sdr_white: None,
            }
        );
        assert_eq!(
            choose_surface_config(&listed, true, HdrMode::Off, hdr_on),
            SurfaceChoice {
                format: F::Rgb10a2Unorm,
                color_space: wgpu::SurfaceColorSpace::Auto,
                hdr_sdr_white: None,
            }
        );
        assert_eq!(
            choose_surface_config(&listed, true, HdrMode::On, None),
            SurfaceChoice {
                format: F::Rgba16Float,
                color_space: wgpu::SurfaceColorSpace::ExtendedSrgbLinear,
                hdr_sdr_white: Some(1.0),
            }
        );
        assert_eq!(
            choose_surface_config(
                &[F::Bgra8UnormSrgb, F::Rgb10a2Unorm],
                true,
                HdrMode::On,
                hdr_on
            ),
            SurfaceChoice {
                format: F::Rgb10a2Unorm,
                color_space: wgpu::SurfaceColorSpace::Auto,
                hdr_sdr_white: None,
            }
        );

        let f = SurfaceFormats::choose(F::Rgba16Float, true);
        assert_eq!((f.scene, f.interface), (F::Rgba16Float, F::Rgba16Float));
        assert!(f.view_formats().is_empty());
        assert!(f.canvas());

        let line = surface_choice_line(
            choose_surface_config(&listed, true, HdrMode::Auto, hdr_on),
            (true, "the default"),
            (HdrMode::Auto, "the default"),
            &listed,
        );
        assert!(
            line.contains(
                "surface format: Rgba16Float (HDR scRGB, 240 nits white; HDR auto by the default)"
            ),
            "{line}"
        );
    }
}
