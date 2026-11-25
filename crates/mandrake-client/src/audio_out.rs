#[cfg(not(target_arch = "wasm32"))]
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
#[cfg(not(target_arch = "wasm32"))]
use cpal::{FromSample, SampleFormat, SizedSample, Stream};
use mandrake_audio::SharedAudio;
use std::sync::Arc;
#[cfg(not(target_arch = "wasm32"))]
use std::sync::atomic::Ordering;
use std::sync::atomic::{AtomicU32, AtomicU64};

macro_rules! alog {
    ($($t:tt)*) => { eprintln!("[audio] {}", format!($($t)*)) };
}

#[cfg_attr(target_arch = "wasm32", allow(dead_code))]
#[derive(Default)]
pub struct CallbackStats {
    pub callbacks: AtomicU64,
    pub peak_bits: AtomicU32,
}

#[cfg(target_os = "linux")]
pub(crate) fn point_alsa_at_host_plugins() {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| {
        if std::env::var_os("ALSA_PLUGIN_DIR").is_some() {
            return;
        }
        for dir in [
            "/usr/lib/alsa-lib",
            "/usr/lib64/alsa-lib",
            "/usr/lib/x86_64-linux-gnu/alsa-lib",
        ] {
            if std::path::Path::new(dir).is_dir() {
                alog!("ALSA_PLUGIN_DIR unset; using host plugins at {dir}");
                unsafe { std::env::set_var("ALSA_PLUGIN_DIR", dir) };
                return;
            }
        }
    });
}
#[cfg(not(target_os = "linux"))]
pub(crate) fn point_alsa_at_host_plugins() {}

#[cfg(target_os = "linux")]
unsafe extern "C" fn silent_alsa_error_handler(
    _file: *const std::ffi::c_char,
    _line: std::ffi::c_int,
    _function: *const std::ffi::c_char,
    _err: std::ffi::c_int,
    _fmt: *const std::ffi::c_char,
    _: ...
) {
}

#[cfg(target_os = "linux")]
fn silence_alsa_logging() {
    unsafe extern "C" {
        fn snd_lib_error_set_handler(
            handler: Option<
                unsafe extern "C" fn(
                    *const std::ffi::c_char,
                    std::ffi::c_int,
                    *const std::ffi::c_char,
                    std::ffi::c_int,
                    *const std::ffi::c_char,
                    ...
                ),
            >,
        ) -> std::ffi::c_int;
    }
    unsafe {
        snd_lib_error_set_handler(Some(silent_alsa_error_handler));
    }
}
#[cfg(not(any(target_os = "linux", target_arch = "wasm32")))]
fn silence_alsa_logging() {}

#[cfg_attr(target_arch = "wasm32", allow(dead_code))]
#[derive(Clone)]
pub struct AudioLink {
    pub audio: SharedAudio,
    pub stats: Arc<CallbackStats>,
}

pub struct AudioOutput {
    link: AudioLink,
    #[cfg(not(target_arch = "wasm32"))]
    _stream: Stream,
}

#[cfg(not(target_arch = "wasm32"))]
fn build<T>(
    device: &cpal::Device,
    config: cpal::StreamConfig,
    audio: SharedAudio,
    stats: Arc<CallbackStats>,
) -> Result<Stream, cpal::Error>
where
    T: SizedSample + FromSample<f32>,
{
    let channels = config.channels as usize;
    let mut scratch: Vec<f32> = Vec::new();
    device.build_output_stream(
        config,
        move |data: &mut [T], _| {
            let frames = data.len() / channels;
            scratch.resize(frames * 2, 0.0);
            audio.fill_stereo(&mut scratch);
            stats.callbacks.fetch_add(1, Ordering::Relaxed);
            let peak = scratch.iter().fold(0.0f32, |m, s| m.max(s.abs()));
            stats.peak_bits.fetch_max(peak.to_bits(), Ordering::Relaxed);
            for (frame, st) in data.chunks_mut(channels).zip(scratch.chunks(2)) {
                for (c, out) in frame.iter_mut().enumerate() {
                    *out = T::from_sample(if c < 2 { st[c] } else { 0.0 });
                }
            }
        },
        |e| alog!("stream error: {e}"),
        None,
    )
}

#[cfg(not(target_arch = "wasm32"))]
fn first_usable<D, I, O>(
    default: Option<D>,
    others: impl FnOnce() -> I,
    mut try_one: impl FnMut(&D) -> Option<O>,
) -> Option<O>
where
    I: IntoIterator<Item = D>,
{
    if let Some(output) = default.and_then(|device| try_one(&device)) {
        return Some(output);
    }
    others().into_iter().find_map(|device| try_one(&device))
}

impl AudioOutput {
    pub fn link(&self) -> AudioLink {
        self.link.clone()
    }

    #[cfg(not(target_arch = "wasm32"))]
    pub fn open(volume: f32) -> Option<Self> {
        silence_alsa_logging();
        let host = cpal::default_host();
        alog!("host: {:?}", host.id());
        let output = first_usable(
            host.default_output_device(),
            || host.output_devices().into_iter().flatten(),
            |device| {
                let name = device
                    .id()
                    .map(|id| id.id().to_owned())
                    .unwrap_or_else(|_| "<unnamed>".into());
                match Self::try_device(device, volume) {
                    Ok(out) => {
                        alog!("playing through device '{name}'");
                        Some(out)
                    }
                    Err(e) => {
                        alog!("device '{name}' unusable: {e}");
                        None
                    }
                }
            },
        );
        if output.is_none() {
            alog!("WARNING: no usable audio output device; running silent");
        }
        output
    }

    #[cfg(target_arch = "wasm32")]
    pub fn open(volume: f32) -> Option<Self> {
        let rate = WEB_RATE.with(std::cell::Cell::get);
        if rate == 0 {
            alog!("the page has no audio output; running silent");
            return None;
        }
        let audio = SharedAudio::new(rate, volume);
        WEB_AUDIO.with(|a| *a.borrow_mut() = Some(audio.clone()));
        alog!("the page's audio worklet: {rate} Hz, 2 ch, master volume {volume}");
        Some(Self {
            link: AudioLink {
                audio,
                stats: Arc::new(CallbackStats::default()),
            },
        })
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn try_device(device: &cpal::Device, volume: f32) -> Result<Self, String> {
        let supported = device
            .default_output_config()
            .map_err(|e| format!("config error: {e}"))?;
        let format = supported.sample_format();
        let config: cpal::StreamConfig = supported.into();
        let audio = SharedAudio::new(config.sample_rate, volume);
        let stats = Arc::new(CallbackStats::default());
        let (a, st) = (audio.clone(), stats.clone());
        let stream = match format {
            SampleFormat::F32 => build::<f32>(device, config, a, st),
            SampleFormat::I16 => build::<i16>(device, config, a, st),
            SampleFormat::U16 => build::<u16>(device, config, a, st),
            other => return Err(format!("unsupported sample format {other:?}")),
        }
        .map_err(|e| format!("build stream: {e}"))?;
        stream.play().map_err(|e| format!("play: {e}"))?;
        alog!(
            "stream open: {} Hz, {} ch, {:?}, master volume {volume}",
            config.sample_rate,
            config.channels,
            format
        );
        Ok(Self {
            link: AudioLink { audio, stats },
            _stream: stream,
        })
    }
}

#[cfg(target_arch = "wasm32")]
thread_local! {
    static WEB_RATE: std::cell::Cell<u32> = const { std::cell::Cell::new(0) };
    static WEB_AUDIO: std::cell::RefCell<Option<SharedAudio>> = const { std::cell::RefCell::new(None) };
}

#[cfg(target_arch = "wasm32")]
pub fn set_web_rate(rate: u32) {
    WEB_RATE.with(|r| r.set(rate.clamp(8_000, 192_000)));
}

#[cfg(target_arch = "wasm32")]
pub fn web_mix(out: &mut [f32]) -> bool {
    WEB_AUDIO.with(|a| match a.borrow().as_ref() {
        Some(audio) => {
            audio.fill_stereo(out);
            true
        }
        None => {
            out.fill(0.0);
            false
        }
    })
}

#[cfg(test)]
mod tests {
    use super::first_usable;
    use std::cell::Cell;

    struct TestDevice<'a> {
        id: u32,
        open: &'a Cell<u32>,
    }

    impl<'a> TestDevice<'a> {
        fn new(id: u32, open: &'a Cell<u32>) -> Self {
            open.set(open.get() + 1);
            Self { id, open }
        }
    }

    impl Drop for TestDevice<'_> {
        fn drop(&mut self) {
            self.open.set(self.open.get() - 1);
        }
    }

    #[test]
    fn a_working_default_never_opens_the_others() {
        let open = Cell::new(0);
        let found = first_usable(
            Some(TestDevice::new(0, &open)),
            || -> Vec<TestDevice> { panic!("enumerated with a working default") },
            |device| Some(device.id),
        );
        assert_eq!(found, Some(0));
    }

    #[test]
    fn a_failed_default_falls_back_with_one_device_open_at_a_time() {
        let open = Cell::new(0);
        let mut tried = Vec::new();
        let found = first_usable(
            Some(TestDevice::new(0, &open)),
            || (1..=3).map(|id| TestDevice::new(id, &open)),
            |device| {
                assert_eq!(open.get(), 1, "device {} opened beside another", device.id);
                tried.push(device.id);
                (device.id == 2).then_some(device.id)
            },
        );
        assert_eq!(found, Some(2));
        assert_eq!(tried, [0, 1, 2]);
    }

    #[test]
    fn no_usable_device_finds_nothing() {
        let open = Cell::new(0);
        let found: Option<u32> = first_usable(
            None,
            || (1..=2).map(|id| TestDevice::new(id, &open)),
            |_| None,
        );
        assert_eq!(found, None);
        assert_eq!(open.get(), 0);
        let found: Option<u32> = first_usable(None, Vec::<TestDevice>::new, |_| None);
        assert_eq!(found, None);
    }

    #[test]
    fn alsa_logging_can_be_silenced_without_error() {
        super::silence_alsa_logging();
    }
}
