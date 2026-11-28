//! Windows monitor HDR state and SDR white-level query via `user32.dll` DisplayConfig FFI.

/// The Windows HDR state and SDR content brightness scale of a window's monitor.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MonitorHdrInfo {
    pub hdr_active: bool,
    /// `SDRWhiteLevel / 1000.0`, where `1.0` = 80 nits in scRGB (e.g. `3.0` = 240 nits).
    pub sdr_white_scale: f32,
}

/// Converts an scRGB SDR white scale (`1.0` = 80 nits) to rounded nits for display logging.
pub fn sdr_white_nits(scale: f32) -> u32 {
    (scale * 80.0).round() as u32
}

/// `DISPLAYCONFIG_ADVANCED_COLOR_MODE_HDR` from `DISPLAYCONFIG_GET_ADVANCED_COLOR_INFO_2`.
#[cfg_attr(not(target_os = "windows"), allow(dead_code))]
const ADVANCED_COLOR_MODE_HDR: i32 = 3;

/// Decodes HDR state from `DISPLAYCONFIG_GET_ADVANCED_COLOR_INFO_2::activeColorMode`.
/// Distinguishes HDR (3) from Windows Automatic Color Management / WCG (2) and SDR (1).
#[cfg_attr(not(target_os = "windows"), allow(dead_code))]
pub fn hdr_active_from_color_mode_v2(active_color_mode: i32) -> bool {
    active_color_mode == ADVANCED_COLOR_MODE_HDR
}

/// Decodes HDR state from legacy `DISPLAYCONFIG_GET_ADVANCED_COLOR_INFO::value` bitfield.
/// Bit 1 is `advancedColorEnabled`, bit 2 is `wideColorEnforced`, bit 3 is
/// `advancedColorForceDisabled`.
#[cfg_attr(not(target_os = "windows"), allow(dead_code))]
pub fn hdr_active_from_color_info_v1(flags: u32) -> bool {
    const ADVANCED_COLOR_ENABLED: u32 = 1 << 1;
    const WIDE_COLOR_ENFORCED: u32 = 1 << 2;
    const ADVANCED_COLOR_FORCE_DISABLED: u32 = 1 << 3;
    (flags & ADVANCED_COLOR_ENABLED) != 0
        && (flags & WIDE_COLOR_ENFORCED) == 0
        && (flags & ADVANCED_COLOR_FORCE_DISABLED) == 0
}

/// Converts Win32 `DISPLAYCONFIG_SDR_WHITE_LEVEL::SDRWhiteLevel` (1000 = 80 nits = 1.0 scRGB)
/// into a clamped scRGB multiplier.
#[cfg_attr(not(target_os = "windows"), allow(dead_code))]
pub fn sdr_white_scale_from_raw(raw_white_level: u32) -> f32 {
    (raw_white_level as f32 / 1000.0).clamp(1.0, 10.0)
}

#[cfg_attr(not(target_os = "windows"), allow(dead_code))]
fn wide_str_eq(a: &[u16], b: &[u16]) -> bool {
    fn trim(s: &[u16]) -> &[u16] {
        let len = s.iter().position(|&c| c == 0).unwrap_or(s.len());
        &s[..len]
    }
    trim(a) == trim(b)
}

#[cfg_attr(not(target_os = "windows"), allow(dead_code))]
mod ffi_types {
    use core::mem::size_of;

    #[repr(C)]
    #[derive(Clone, Copy, Default)]
    pub struct Luid {
        pub low_part: u32,
        pub high_part: i32,
    }

    #[repr(C)]
    #[derive(Clone, Copy, Default)]
    pub struct Rect {
        pub left: i32,
        pub top: i32,
        pub right: i32,
        pub bottom: i32,
    }

    #[repr(C)]
    #[derive(Clone, Copy, Default)]
    pub struct MonitorInfo {
        pub cb_size: u32,
        pub rc_monitor: Rect,
        pub rc_work: Rect,
        pub dw_flags: u32,
    }

    #[repr(C)]
    #[derive(Clone, Copy, Default)]
    pub struct MonitorInfoExW {
        pub monitor_info: MonitorInfo,
        pub sz_device: [u16; 32],
    }

    #[repr(C)]
    #[derive(Clone, Copy, Default)]
    pub struct DisplayConfigRational {
        pub numerator: u32,
        pub denominator: u32,
    }

    #[repr(C)]
    #[derive(Clone, Copy, Default)]
    pub struct DisplayConfig2DRegion {
        pub cx: u32,
        pub cy: u32,
    }

    #[repr(C)]
    #[derive(Clone, Copy, Default)]
    pub struct DisplayConfigPathSourceInfo {
        pub adapter_id: Luid,
        pub id: u32,
        pub mode_info_idx: u32,
        pub status_flags: u32,
    }

    #[repr(C)]
    #[derive(Clone, Copy, Default)]
    pub struct DisplayConfigPathTargetInfo {
        pub adapter_id: Luid,
        pub id: u32,
        pub mode_info_idx: u32,
        pub output_technology: i32,
        pub rotation: i32,
        pub scaling: i32,
        pub refresh_rate: DisplayConfigRational,
        pub scan_line_ordering: i32,
        pub target_available: i32,
        pub status_flags: u32,
    }

    #[repr(C)]
    #[derive(Clone, Copy, Default)]
    pub struct DisplayConfigPathInfo {
        pub source_info: DisplayConfigPathSourceInfo,
        pub target_info: DisplayConfigPathTargetInfo,
        pub flags: u32,
    }

    #[repr(C)]
    #[derive(Clone, Copy, Default)]
    pub struct DisplayConfigVideoSignalInfo {
        pub pixel_rate: u64,
        pub h_sync_freq: DisplayConfigRational,
        pub v_sync_freq: DisplayConfigRational,
        pub active_size: DisplayConfig2DRegion,
        pub total_size: DisplayConfig2DRegion,
        pub video_standard: u32,
        pub scan_line_ordering: i32,
    }

    #[repr(C)]
    #[derive(Clone, Copy, Default)]
    pub struct DisplayConfigModeInfo {
        pub info_type: i32,
        pub id: u32,
        pub adapter_id: Luid,
        pub target_mode: DisplayConfigVideoSignalInfo,
    }

    #[repr(C)]
    #[derive(Clone, Copy, Default)]
    pub struct DisplayConfigDeviceInfoHeader {
        pub info_type: i32,
        pub size: u32,
        pub adapter_id: Luid,
        pub id: u32,
    }

    #[repr(C)]
    #[derive(Clone, Copy, Default)]
    pub struct DisplayConfigSourceDeviceName {
        pub header: DisplayConfigDeviceInfoHeader,
        pub view_gdi_device_name: [u16; 32],
    }

    #[repr(C)]
    #[derive(Clone, Copy, Default)]
    pub struct DisplayConfigGetAdvancedColorInfo {
        pub header: DisplayConfigDeviceInfoHeader,
        pub value: u32,
        pub color_encoding: i32,
        pub bits_per_color_channel: u32,
    }

    #[repr(C)]
    #[derive(Clone, Copy, Default)]
    pub struct DisplayConfigGetAdvancedColorInfo2 {
        pub header: DisplayConfigDeviceInfoHeader,
        pub value: u32,
        pub color_encoding: i32,
        pub bits_per_color_channel: u32,
        pub active_color_mode: i32,
    }

    #[repr(C)]
    #[derive(Clone, Copy, Default)]
    pub struct DisplayConfigSdrWhiteLevel {
        pub header: DisplayConfigDeviceInfoHeader,
        pub sdr_white_level: u32,
    }

    // Verified against Windows SDK / windows-sys 0.61.2 layouts.
    const _: () = assert!(size_of::<MonitorInfoExW>() == 104);
    const _: () = assert!(size_of::<DisplayConfigPathInfo>() == 72);
    const _: () = assert!(size_of::<DisplayConfigModeInfo>() == 64);
    const _: () = assert!(size_of::<DisplayConfigDeviceInfoHeader>() == 20);
    const _: () = assert!(size_of::<DisplayConfigSourceDeviceName>() == 84);
    const _: () = assert!(size_of::<DisplayConfigGetAdvancedColorInfo>() == 32);
    const _: () = assert!(size_of::<DisplayConfigGetAdvancedColorInfo2>() == 36);
    const _: () = assert!(size_of::<DisplayConfigSdrWhiteLevel>() == 24);
}

#[cfg(target_os = "windows")]
pub fn query_monitor_hdr(window: &winit::window::Window) -> Option<MonitorHdrInfo> {
    use core::ffi::c_void;
    use core::mem::size_of;
    use ffi_types::*;
    use winit::raw_window_handle::{HasWindowHandle, RawWindowHandle};

    const MONITOR_DEFAULTTONEAREST: u32 = 2;
    const QDC_ONLY_ACTIVE_PATHS: u32 = 2;
    const ERROR_SUCCESS: u32 = 0;
    const ERROR_INSUFFICIENT_BUFFER: u32 = 122;
    const GET_SOURCE_NAME: i32 = 1;
    const GET_ADVANCED_COLOR_INFO: i32 = 9;
    const GET_SDR_WHITE_LEVEL: i32 = 11;
    const GET_ADVANCED_COLOR_INFO_2: i32 = 15;

    #[link(name = "user32")]
    unsafe extern "system" {
        fn MonitorFromWindow(hwnd: *mut c_void, dw_flags: u32) -> *mut c_void;
        fn GetMonitorInfoW(hmonitor: *mut c_void, lpmi: *mut MonitorInfoExW) -> i32;
        fn GetDisplayConfigBufferSizes(
            flags: u32,
            num_path_array_elements: *mut u32,
            num_mode_info_array_elements: *mut u32,
        ) -> u32;
        fn QueryDisplayConfig(
            flags: u32,
            num_path_array_elements: *mut u32,
            path_array: *mut DisplayConfigPathInfo,
            num_mode_info_array_elements: *mut u32,
            mode_info_array: *mut DisplayConfigModeInfo,
            current_topology_id: *mut i32,
        ) -> u32;
        fn DisplayConfigGetDeviceInfo(request_packet: *mut DisplayConfigDeviceInfoHeader) -> i32;
    }

    let handle = window.window_handle().ok()?;
    let RawWindowHandle::Win32(win32) = handle.as_raw() else {
        return None;
    };
    let hwnd = win32.hwnd.get() as *mut c_void;

    let hmonitor = unsafe { MonitorFromWindow(hwnd, MONITOR_DEFAULTTONEAREST) };
    if hmonitor.is_null() {
        return None;
    }

    let mut mon_info = MonitorInfoExW::default();
    mon_info.monitor_info.cb_size = size_of::<MonitorInfoExW>() as u32;
    if unsafe { GetMonitorInfoW(hmonitor, &mut mon_info) } == 0 {
        return None;
    }

    let mut paths: Vec<DisplayConfigPathInfo> = Vec::new();
    let mut modes: Vec<DisplayConfigModeInfo> = Vec::new();
    for _ in 0..4 {
        let mut path_count = 0u32;
        let mut mode_count = 0u32;
        let status = unsafe {
            GetDisplayConfigBufferSizes(QDC_ONLY_ACTIVE_PATHS, &mut path_count, &mut mode_count)
        };
        if status != ERROR_SUCCESS || path_count == 0 {
            return None;
        }
        paths.resize(path_count as usize, DisplayConfigPathInfo::default());
        modes.resize(mode_count as usize, DisplayConfigModeInfo::default());
        let status = unsafe {
            QueryDisplayConfig(
                QDC_ONLY_ACTIVE_PATHS,
                &mut path_count,
                paths.as_mut_ptr(),
                &mut mode_count,
                modes.as_mut_ptr(),
                core::ptr::null_mut(),
            )
        };
        if status == ERROR_SUCCESS {
            paths.truncate(path_count as usize);
            break;
        }
        if status != ERROR_INSUFFICIENT_BUFFER {
            return None;
        }
    }

    let target = paths.iter().find(|path| {
        let mut src = DisplayConfigSourceDeviceName::default();
        src.header.info_type = GET_SOURCE_NAME;
        src.header.size = size_of::<DisplayConfigSourceDeviceName>() as u32;
        src.header.adapter_id = path.source_info.adapter_id;
        src.header.id = path.source_info.id;
        let ok = unsafe { DisplayConfigGetDeviceInfo(&mut src.header) } == 0;
        ok && wide_str_eq(&src.view_gdi_device_name, &mon_info.sz_device)
    })?;

    let mut color2 = DisplayConfigGetAdvancedColorInfo2::default();
    color2.header.info_type = GET_ADVANCED_COLOR_INFO_2;
    color2.header.size = size_of::<DisplayConfigGetAdvancedColorInfo2>() as u32;
    color2.header.adapter_id = target.target_info.adapter_id;
    color2.header.id = target.target_info.id;

    let hdr_active = if unsafe { DisplayConfigGetDeviceInfo(&mut color2.header) } == 0 {
        hdr_active_from_color_mode_v2(color2.active_color_mode)
    } else {
        let mut color1 = DisplayConfigGetAdvancedColorInfo::default();
        color1.header.info_type = GET_ADVANCED_COLOR_INFO;
        color1.header.size = size_of::<DisplayConfigGetAdvancedColorInfo>() as u32;
        color1.header.adapter_id = target.target_info.adapter_id;
        color1.header.id = target.target_info.id;
        if unsafe { DisplayConfigGetDeviceInfo(&mut color1.header) } != 0 {
            return None;
        }
        hdr_active_from_color_info_v1(color1.value)
    };

    let mut white = DisplayConfigSdrWhiteLevel::default();
    white.header.info_type = GET_SDR_WHITE_LEVEL;
    white.header.size = size_of::<DisplayConfigSdrWhiteLevel>() as u32;
    white.header.adapter_id = target.target_info.adapter_id;
    white.header.id = target.target_info.id;
    let sdr_white_scale = if unsafe { DisplayConfigGetDeviceInfo(&mut white.header) } == 0 {
        sdr_white_scale_from_raw(white.sdr_white_level)
    } else {
        1.0
    };

    Some(MonitorHdrInfo {
        hdr_active,
        sdr_white_scale,
    })
}

#[cfg(not(target_os = "windows"))]
pub fn query_monitor_hdr(_window: &winit::window::Window) -> Option<MonitorHdrInfo> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn color_mode_v2_distinguishes_hdr_from_wcg_and_sdr() {
        assert!(!hdr_active_from_color_mode_v2(0));
        assert!(!hdr_active_from_color_mode_v2(1));
        assert!(!hdr_active_from_color_mode_v2(2));
        assert!(hdr_active_from_color_mode_v2(3));
    }

    #[test]
    fn color_info_v1_fallback_excludes_wide_color_and_force_disabled() {
        assert!(!hdr_active_from_color_info_v1(0));
        assert!(!hdr_active_from_color_info_v1(0x1));
        assert!(hdr_active_from_color_info_v1(0x2));
        assert!(hdr_active_from_color_info_v1(0x3));
        assert!(!hdr_active_from_color_info_v1(0x2 | 0x4));
        assert!(!hdr_active_from_color_info_v1(0x2 | 0x8));
    }

    #[test]
    fn sdr_white_level_converts_and_clamps() {
        assert_eq!(sdr_white_scale_from_raw(0), 1.0);
        assert_eq!(sdr_white_scale_from_raw(500), 1.0);
        assert_eq!(sdr_white_scale_from_raw(1000), 1.0);
        assert_eq!(sdr_white_scale_from_raw(3000), 3.0);
        assert_eq!(sdr_white_scale_from_raw(12000), 10.0);
        assert_eq!(sdr_white_nits(3.0), 240);
    }

    #[test]
    fn wide_str_comparison_stops_at_nul() {
        let a = [b'\\' as u16, b'\\' as u16, b'.' as u16, 0, 99];
        let b = [b'\\' as u16, b'\\' as u16, b'.' as u16, 0, 0];
        let c = [b'\\' as u16, b'\\' as u16, b'1' as u16, 0];
        assert!(wide_str_eq(&a, &b));
        assert!(!wide_str_eq(&a, &c));
    }
}
