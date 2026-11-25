use crate::clock::Instant;
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::ptr::NonNull;
use std::sync::mpsc::{Receiver, Sender, SyncSender, TryRecvError, channel, sync_channel};
use std::time::Duration;
use tracing::{info, warn};

use rav1d::include::dav1d::data::Dav1dData;
use rav1d::include::dav1d::dav1d::{Dav1dContext, Dav1dSettings};
use rav1d::include::dav1d::headers::DAV1D_PIXEL_LAYOUT_I420;
use rav1d::include::dav1d::picture::Dav1dPicture;
use rav1d::src::lib as dav1d;

pub(crate) const FOLDER: &str = "cutscenes-hd";
const AHEAD_FRAMES: usize = 2;
const QUEUED_FRAMES: usize = 30;

pub(crate) fn data_base_from(
    env: impl Fn(&str) -> Option<OsString>,
    windows: bool,
) -> Option<PathBuf> {
    let set = |name: &str| env(name).filter(|v| !v.is_empty());
    set("XDG_DATA_HOME")
        .map(PathBuf::from)
        .or_else(|| set("HOME").map(|h| PathBuf::from(h).join(".local").join("share")))
        .or_else(|| {
            windows
                .then(|| set("LOCALAPPDATA"))
                .flatten()
                .map(PathBuf::from)
        })
}

pub(crate) const TFL_FOLDER: &str = "tfl";

pub(crate) fn hd_path(smk: &Path) -> Option<PathBuf> {
    let stem = smk.file_stem()?;
    let base = data_base_from(|name| std::env::var_os(name), cfg!(windows))?;
    Some(hd_file(
        &base.join(crate::user_dirs::APP_DIR).join(FOLDER),
        stem,
    ))
}

pub(crate) fn tfl_hd_path(id: mandrake_common::FourCC) -> Option<PathBuf> {
    let base = data_base_from(|name| std::env::var_os(name), cfg!(windows))?;
    let dir = base
        .join(crate::user_dirs::APP_DIR)
        .join(FOLDER)
        .join(TFL_FOLDER);
    Some(hd_file(&dir, tfl_file_stem(id).as_ref()))
}

fn hd_file(dir: &Path, stem: &std::ffi::OsStr) -> PathBuf {
    dir.join(stem).with_extension("ivf")
}

pub(crate) fn tfl_file_stem(id: mandrake_common::FourCC) -> String {
    id.0.iter()
        .map(|&b| {
            if b.is_ascii_alphanumeric() || b == b'_' || b == b'-' {
                char::from(b).to_string()
            } else {
                format!("%{b:02X}")
            }
        })
        .collect()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct IvfHeader {
    pub fourcc: [u8; 4],
    pub width: u16,
    pub height: u16,
    pub rate: u32,
    pub scale: u32,
    pub frames: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Ivf {
    pub header: IvfHeader,
    pub frames: Vec<std::ops::Range<usize>>,
}

fn le16(b: &[u8], at: usize) -> u16 {
    u16::from_le_bytes([b[at], b[at + 1]])
}

fn le32(b: &[u8], at: usize) -> u32 {
    u32::from_le_bytes(b[at..at + 4].try_into().expect("four bytes"))
}

pub(crate) fn parse_ivf(data: &[u8]) -> Result<Ivf, String> {
    if data.len() < 32 || &data[0..4] != b"DKIF" {
        return Err("not an IVF file".into());
    }
    let version = le16(data, 4);
    let header_len = usize::from(le16(data, 6));
    if version != 0 || header_len < 32 || header_len > data.len() {
        return Err(format!(
            "IVF version {version}, header of {header_len} bytes"
        ));
    }
    let header = IvfHeader {
        fourcc: data[8..12].try_into().expect("four bytes"),
        width: le16(data, 12),
        height: le16(data, 14),
        rate: le32(data, 16),
        scale: le32(data, 20),
        frames: le32(data, 24),
    };
    let mut frames = Vec::with_capacity(header.frames.min(1 << 16) as usize);
    let mut at = header_len;
    while at < data.len() {
        if data.len() - at < 12 {
            return Err(format!("frame {} header cut short", frames.len()));
        }
        let size = le32(data, at) as usize;
        let start = at + 12;
        if size == 0 || data.len() - start < size {
            return Err(format!(
                "frame {} of {size} bytes cut short or empty",
                frames.len()
            ));
        }
        frames.push(start..start + size);
        at = start + size;
    }
    Ok(Ivf { header, frames })
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Choice {
    Hd { data: Vec<u8>, ivf: Ivf },
    Smk(Option<String>),
}

pub(crate) fn choose(path: Option<&Path>, frame_count: u32) -> Choice {
    let Some(path) = path else {
        return Choice::Smk(None);
    };
    let data = match std::fs::read(path) {
        Ok(data) => data,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Choice::Smk(None),
        Err(e) => return Choice::Smk(Some(format!("{}: {e}", path.display()))),
    };
    let ivf = match parse_ivf(&data) {
        Ok(ivf) => ivf,
        Err(e) => return Choice::Smk(Some(format!("{}: {e}", path.display()))),
    };
    if &ivf.header.fourcc != b"AV01" {
        return Choice::Smk(Some(format!(
            "{}: {:?} is not AV1",
            path.display(),
            String::from_utf8_lossy(&ivf.header.fourcc)
        )));
    }
    if ivf.frames.len() != frame_count as usize {
        return Choice::Smk(Some(format!(
            "{}: {} frames, the .smk has {frame_count}",
            path.display(),
            ivf.frames.len()
        )));
    }
    Choice::Hd { data, ivf }
}

pub(crate) struct HdFrame {
    pub index: u32,
    pub size: (u32, u32),
    pub y: Vec<u16>,
    pub u: Vec<u16>,
    pub v: Vec<u16>,
}

impl HdFrame {
    pub(crate) fn chroma_size(&self) -> (u32, u32) {
        (self.size.0.div_ceil(2), self.size.1.div_ceil(2))
    }
}

pub(crate) enum HdMessage {
    Frame(HdFrame),
    Failed(String),
}

struct Decoder {
    context: Option<Dav1dContext>,
}

const EAGAIN: i32 = -11;

impl Decoder {
    fn open() -> Result<Self, String> {
        let mut settings = std::mem::MaybeUninit::<Dav1dSettings>::uninit();
        let mut settings = unsafe {
            dav1d::dav1d_default_settings(NonNull::new_unchecked(settings.as_mut_ptr()));
            settings.assume_init()
        };
        let threads = std::thread::available_parallelism().map_or(2, |n| n.get());
        settings.n_threads = threads.clamp(1, 16) as i32;
        let mut context = None;
        let result = unsafe {
            dav1d::dav1d_open(
                Some(NonNull::from(&mut context)),
                Some(NonNull::from(&mut settings)),
            )
        };
        if result.0 < 0 || context.is_none() {
            return Err(format!("dav1d_open: {}", result.0));
        }
        Ok(Self { context })
    }

    fn decode(&mut self, bytes: &[u8], out: &mut Vec<Dav1dPicture>) -> Result<(), String> {
        let mut data = Dav1dData::default();
        unsafe {
            let ptr = dav1d::dav1d_data_create(Some(NonNull::from(&mut data)), bytes.len());
            if ptr.is_null() {
                return Err("dav1d_data_create failed".into());
            }
            std::ptr::copy_nonoverlapping(bytes.as_ptr(), ptr, bytes.len());
        }
        let result = loop {
            let sent =
                unsafe { dav1d::dav1d_send_data(self.context, Some(NonNull::from(&mut data))) };
            if sent.0 < 0 && sent.0 != EAGAIN {
                break Err(format!("dav1d_send_data: {}", sent.0));
            }
            if let Err(e) = self.pictures(out) {
                break Err(e);
            }
            if data.sz == 0 {
                break Ok(());
            }
        };
        if data.sz > 0 {
            unsafe { dav1d::dav1d_data_unref(Some(NonNull::from(&mut data))) };
        }
        result
    }

    fn pictures(&mut self, out: &mut Vec<Dav1dPicture>) -> Result<(), String> {
        loop {
            let mut picture = Dav1dPicture::default();
            let got = unsafe {
                dav1d::dav1d_get_picture(self.context, Some(NonNull::from(&mut picture)))
            };
            match got.0 {
                0 => out.push(picture),
                EAGAIN => return Ok(()),
                e => return Err(format!("dav1d_get_picture: {e}")),
            }
        }
    }

    fn drain(&mut self, out: &mut Vec<Dav1dPicture>) -> Result<(), String> {
        for _ in 0..64 {
            let before = out.len();
            self.pictures(out)?;
            if out.len() == before {
                return Ok(());
            }
        }
        Ok(())
    }
}

impl Drop for Decoder {
    fn drop(&mut self) {
        unsafe { dav1d::dav1d_close(Some(NonNull::from(&mut self.context))) };
    }
}

fn take_picture(mut picture: Dav1dPicture, index: u32) -> Result<HdFrame, String> {
    let (w, h) = (picture.p.w, picture.p.h);
    let check = if picture.p.bpc != 10 {
        Err(format!("{}-bit, not 10-bit", picture.p.bpc))
    } else if picture.p.layout != DAV1D_PIXEL_LAYOUT_I420 {
        Err("not 4:2:0".to_owned())
    } else if w <= 0 || h <= 0 || picture.data.iter().any(Option::is_none) {
        Err("no picture".to_owned())
    } else {
        Ok(())
    };
    let frame = check.map(|()| {
        let (w, h) = (w as usize, h as usize);
        let (cw, ch) = (w.div_ceil(2), h.div_ceil(2));
        let plane = |data: NonNull<std::ffi::c_void>, stride: isize, pw: usize, ph: usize| {
            let mut plane = Vec::with_capacity(pw * ph);
            for row in 0..ph {
                let samples = unsafe {
                    let start = data.as_ptr().cast::<u8>().offset(row as isize * stride);
                    std::slice::from_raw_parts(start.cast::<u16>(), pw)
                };
                plane.extend_from_slice(samples);
            }
            plane
        };
        let [y, u, v] = picture.data.map(|p| p.expect("checked"));
        HdFrame {
            index,
            size: (w as u32, h as u32),
            y: plane(y, picture.stride[0], w, h),
            u: plane(u, picture.stride[1], cw, ch),
            v: plane(v, picture.stride[1], cw, ch),
        }
    });
    unsafe { dav1d::dav1d_picture_unref(Some(NonNull::from(&mut picture))) };
    frame
}

fn decode_thread(data: Vec<u8>, ivf: Ivf, out: SyncSender<HdMessage>) {
    let started = Instant::now();
    let mut busy = Duration::ZERO;
    let mut decoder = match Decoder::open() {
        Ok(decoder) => decoder,
        Err(e) => {
            let _ = out.send(HdMessage::Failed(e));
            return;
        }
    };
    let mut pictures = Vec::new();
    let mut index = 0u32;
    let total = ivf.frames.len();
    let send = |pictures: &mut Vec<Dav1dPicture>, index: &mut u32| -> bool {
        for picture in pictures.drain(..) {
            match take_picture(picture, *index) {
                Ok(frame) => {
                    *index += 1;
                    if out.send(HdMessage::Frame(frame)).is_err() {
                        return false;
                    }
                }
                Err(e) => {
                    let _ = out.send(HdMessage::Failed(format!("frame {index}: {e}")));
                    return false;
                }
            }
        }
        true
    };
    for (n, range) in ivf.frames.iter().enumerate() {
        let t = Instant::now();
        let result = decoder.decode(&data[range.clone()], &mut pictures);
        busy += t.elapsed();
        if let Err(e) = result {
            let _ = out.send(HdMessage::Failed(format!("frame {n}: {e}")));
            return;
        }
        if !send(&mut pictures, &mut index) {
            return;
        }
    }
    let t = Instant::now();
    let result = decoder.drain(&mut pictures);
    busy += t.elapsed();
    if let Err(e) = result {
        let _ = out.send(HdMessage::Failed(format!("at the end: {e}")));
        return;
    }
    if !send(&mut pictures, &mut index) {
        return;
    }
    if index as usize != total {
        let _ = out.send(HdMessage::Failed(format!(
            "{index} pictures from {total} frames"
        )));
        return;
    }
    info!(
        "cutscene-hd: decoded {total} frames in {:.1} s ({:.2} ms a frame decoding, {:.0} fps)",
        started.elapsed().as_secs_f64(),
        busy.as_secs_f64() * 1e3 / total.max(1) as f64,
        total as f64 / busy.as_secs_f64().max(1e-9),
    );
}

pub(crate) struct HdShown {
    pub index: u32,
    pub size: (u32, u32),
    slot: usize,
    bind_group: wgpu::BindGroup,
}

enum Uploaded {
    Frame(HdShown),
    Failed(String),
}

struct GpuLink {
    device: wgpu::Device,
    queue: wgpu::Queue,
    layout: wgpu::BindGroupLayout,
}

const SLOTS: usize = AHEAD_FRAMES + 3;

struct Slot {
    size: (u32, u32),
    textures: [wgpu::Texture; 3],
    bind_group: wgpu::BindGroup,
}

impl Slot {
    fn new(gpu: &GpuLink, size: (u32, u32)) -> Self {
        let (cw, ch) = (size.0.div_ceil(2), size.1.div_ceil(2));
        let texture = |label, w, h| {
            gpu.device.create_texture(&wgpu::TextureDescriptor {
                label: Some(label),
                size: wgpu::Extent3d {
                    width: w,
                    height: h,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::R16Uint,
                usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
                view_formats: &[],
            })
        };
        let textures = [
            texture("Cutscene HD Y", size.0, size.1),
            texture("Cutscene HD U", cw, ch),
            texture("Cutscene HD V", cw, ch),
        ];
        let views: Vec<_> = textures
            .iter()
            .map(|t| t.create_view(&wgpu::TextureViewDescriptor::default()))
            .collect();
        let bind_group = gpu.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Cutscene HD Bind Group"),
            layout: &gpu.layout,
            entries: &[0, 1, 2].map(|i| wgpu::BindGroupEntry {
                binding: i,
                resource: wgpu::BindingResource::TextureView(&views[i as usize]),
            }),
        });
        Self {
            size,
            textures,
            bind_group,
        }
    }

    fn write(&self, gpu: &GpuLink, frame: &HdFrame) -> Result<(), String> {
        let (cw, ch) = frame.chroma_size();
        let planes = [
            (&frame.y, frame.size),
            (&frame.u, (cw, ch)),
            (&frame.v, (cw, ch)),
        ];
        let pitch = |w: u32| (2 * w).next_multiple_of(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT);
        let offsets: Vec<u64> = planes
            .iter()
            .scan(0u64, |at, (_, (w, h))| {
                let start = *at;
                *at += u64::from(pitch(*w)) * u64::from(*h);
                Some(start)
            })
            .collect();
        let total = offsets[2] + u64::from(pitch(cw)) * u64::from(ch);
        let staging = gpu.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Cutscene HD Staging"),
            size: total,
            usage: wgpu::BufferUsages::COPY_SRC,
            mapped_at_creation: true,
        });
        {
            let mut mapped = staging
                .slice(..)
                .get_mapped_range_mut()
                .map_err(|e| format!("staging buffer: {e}"))?;
            for ((samples, (w, _)), offset) in planes.iter().zip(&offsets) {
                let (row, pitch) = (2 * *w as usize, pitch(*w) as usize);
                let bytes = u16_bytes(samples);
                let start = *offset as usize;
                if row == pitch {
                    mapped
                        .slice(start..start + bytes.len())
                        .copy_from_slice(bytes);
                } else {
                    for (n, src) in bytes.chunks_exact(row).enumerate() {
                        let at = start + n * pitch;
                        mapped.slice(at..at + row).copy_from_slice(src);
                    }
                }
            }
        }
        staging.unmap();
        let mut encoder = gpu
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("Cutscene HD Upload"),
            });
        for ((texture, (_, (w, h))), offset) in self.textures.iter().zip(planes).zip(offsets) {
            encoder.copy_buffer_to_texture(
                wgpu::TexelCopyBufferInfo {
                    buffer: &staging,
                    layout: wgpu::TexelCopyBufferLayout {
                        offset,
                        bytes_per_row: Some(pitch(w)),
                        rows_per_image: Some(h),
                    },
                },
                wgpu::TexelCopyTextureInfo {
                    texture,
                    mip_level: 0,
                    origin: wgpu::Origin3d::ZERO,
                    aspect: wgpu::TextureAspect::All,
                },
                wgpu::Extent3d {
                    width: w,
                    height: h,
                    depth_or_array_layers: 1,
                },
            );
        }
        gpu.queue.submit(std::iter::once(encoder.finish()));
        Ok(())
    }
}

fn upload_thread(
    frames: Receiver<HdMessage>,
    attach: Receiver<GpuLink>,
    free: Receiver<usize>,
    out: SyncSender<Uploaded>,
) {
    let Ok(gpu) = attach.recv() else {
        return;
    };
    let mut slots: Vec<Slot> = Vec::with_capacity(SLOTS);
    let (mut count, mut busy, mut max) = (0u32, Duration::ZERO, Duration::ZERO);
    for message in frames {
        let frame = match message {
            HdMessage::Frame(frame) => frame,
            HdMessage::Failed(e) => {
                let _ = out.send(Uploaded::Failed(e));
                return;
            }
        };
        let slot = match free.try_recv() {
            Ok(slot) => slot,
            Err(_) if slots.len() < SLOTS => {
                slots.push(Slot::new(&gpu, frame.size));
                slots.len() - 1
            }
            Err(_) => match free.recv() {
                Ok(slot) => slot,
                Err(_) => return,
            },
        };
        let t = Instant::now();
        if slots[slot].size != frame.size {
            slots[slot] = Slot::new(&gpu, frame.size);
        }
        if let Err(e) = slots[slot].write(&gpu, &frame) {
            let _ = out.send(Uploaded::Failed(format!("frame {}: {e}", frame.index)));
            return;
        }
        let spent = t.elapsed();
        count += 1;
        busy += spent;
        max = max.max(spent);
        let shown = HdShown {
            index: frame.index,
            size: frame.size,
            slot,
            bind_group: slots[slot].bind_group.clone(),
        };
        if out.send(Uploaded::Frame(shown)).is_err() {
            return;
        }
    }
    info!(
        "cutscene-hd: {count} frames uploaded on the uploader's thread, {:.3} ms mean, {:.3} ms max",
        busy.as_secs_f64() * 1e3 / f64::from(count.max(1)),
        max.as_secs_f64() * 1e3,
    );
}

fn start_decoder(data: Vec<u8>, ivf: Ivf) -> Result<Receiver<HdMessage>, String> {
    let (send, frames) = sync_channel(QUEUED_FRAMES);
    std::thread::Builder::new()
        .name("cutscene-hd".to_owned())
        .spawn(move || decode_thread(data, ivf, send))
        .map_err(|e| e.to_string())?;
    Ok(frames)
}

pub(crate) struct HdStream {
    frames: Receiver<Uploaded>,
    attach: Option<SyncSender<GpuLink>>,
    layout: Option<wgpu::BindGroupLayout>,
    free: Sender<usize>,
    ahead: std::collections::VecDeque<HdShown>,
    done: bool,
    uploader: Option<std::thread::JoinHandle<()>>,
}

impl Drop for HdStream {
    fn drop(&mut self) {
        self.close();
    }
}

impl HdStream {
    pub(crate) fn start(data: Vec<u8>, ivf: Ivf) -> Result<Self, String> {
        let decoded = start_decoder(data, ivf)?;
        let (attach, attached) = sync_channel(1);
        let (free, freed) = channel();
        let (send, frames) = sync_channel(1);
        let uploader = std::thread::Builder::new()
            .name("cutscene-hd-upload".to_owned())
            .spawn(move || upload_thread(decoded, attached, freed, send))
            .map_err(|e| e.to_string())?;
        Ok(Self {
            uploader: Some(uploader),
            frames,
            attach: Some(attach),
            layout: None,
            free,
            ahead: std::collections::VecDeque::new(),
            done: false,
        })
    }

    fn close(&mut self) {
        self.frames = sync_channel(1).1;
        self.free = channel().0;
        self.attach = None;
        if let Some(uploader) = self.uploader.take() {
            let _ = uploader.join();
        }
    }

    pub(crate) fn attach(&mut self, device: &wgpu::Device, queue: &wgpu::Queue) {
        if let Some(attach) = self.attach.take() {
            let layout = plane_layout(device);
            let _ = attach.send(GpuLink {
                device: device.clone(),
                queue: queue.clone(),
                layout: layout.clone(),
            });
            self.layout = Some(layout);
        }
    }

    pub(crate) fn layout(&self) -> Option<&wgpu::BindGroupLayout> {
        self.layout.as_ref()
    }

    pub(crate) fn fill(&mut self) -> Result<(), String> {
        while !self.done && self.ahead.len() < AHEAD_FRAMES {
            match self.frames.try_recv() {
                Ok(Uploaded::Frame(frame)) => self.ahead.push_back(frame),
                Ok(Uploaded::Failed(e)) => {
                    self.done = true;
                    return Err(e);
                }
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => self.done = true,
            }
        }
        Ok(())
    }

    pub(crate) fn has_first(&self) -> bool {
        !self.ahead.is_empty()
    }

    pub(crate) fn take_due(&mut self, due: u32) -> Option<HdShown> {
        let mut latest = None;
        while self.ahead.front().is_some_and(|f| f.index <= due) {
            if let Some(passed) = latest.replace(self.ahead.pop_front().expect("front was there")) {
                self.release(passed);
            }
        }
        latest
    }

    pub(crate) fn release(&self, frame: HdShown) {
        let _ = self.free.send(frame.slot);
    }

    pub(crate) fn is_drained(&self) -> bool {
        self.done && self.ahead.is_empty()
    }
}

pub(crate) fn warn_fallback(why: &str) {
    warn!("cutscene-hd: playing the .smk instead ({why})");
}

pub(crate) struct HdGpu {
    pipeline: wgpu::RenderPipeline,
}

fn plane_layout(device: &wgpu::Device) -> wgpu::BindGroupLayout {
    let plane = |binding| wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::FRAGMENT,
        ty: wgpu::BindingType::Texture {
            sample_type: wgpu::TextureSampleType::Uint,
            view_dimension: wgpu::TextureViewDimension::D2,
            multisampled: false,
        },
        count: None,
    };
    device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("Cutscene HD Layout"),
        entries: &[plane(0), plane(1), plane(2)],
    })
}

const SHADER: &str = r"
@group(0) @binding(0) var y_plane: texture_2d<u32>;
@group(0) @binding(1) var u_plane: texture_2d<u32>;
@group(0) @binding(2) var v_plane: texture_2d<u32>;

@vertex
fn vs_main(@builtin(vertex_index) i: u32) -> @builtin(position) vec4<f32> {
    let x = f32((i << 1u) & 2u);
    let y = f32(i & 2u);
    return vec4<f32>(x * 2.0 - 1.0, 1.0 - y * 2.0, 0.0, 1.0);
}

fn sample_at(t: texture_2d<u32>, p: vec2<i32>) -> f32 {
    let size = vec2<i32>(textureDimensions(t));
    return f32(textureLoad(t, clamp(p, vec2<i32>(0), size - 1), 0).r);
}

fn chroma(t: texture_2d<u32>, c: vec2<f32>) -> f32 {
    let base = floor(c);
    let f = c - base;
    let p = vec2<i32>(base);
    let top = mix(sample_at(t, p), sample_at(t, p + vec2<i32>(1, 0)), f.x);
    let bottom = mix(sample_at(t, p + vec2<i32>(0, 1)), sample_at(t, p + vec2<i32>(1, 1)), f.x);
    return mix(top, bottom, f.y);
}

fn rgb_at(pos: vec4<f32>) -> vec3<f32> {
    let p = vec2<i32>(floor(pos.xy));
    let c = vec2<f32>(f32(p.x) * 0.5, (f32(p.y) + 0.5) * 0.5 - 0.5);
    let y = (sample_at(y_plane, p) - 64.0) / 876.0;
    let cb = (chroma(u_plane, c) - 512.0) / 896.0;
    let cr = (chroma(v_plane, c) - 512.0) / 896.0;
    let rgb = vec3<f32>(
        y + 1.5748 * cr,
        y - 0.187324 * cb - 0.468124 * cr,
        y + 1.8556 * cb,
    );
    return clamp(rgb, vec3<f32>(0.0), vec3<f32>(1.0));
}

@fragment
fn fs_main(@builtin(position) pos: vec4<f32>) -> @location(0) vec4<f32> {
    return vec4<f32>(rgb_at(pos), 1.0);
}

fn srgb_to_linear(c: vec3<f32>) -> vec3<f32> {
    let lo = c / 12.92;
    let hi = pow((c + 0.055) / 1.055, vec3<f32>(2.4));
    return select(hi, lo, c <= vec3<f32>(0.04045));
}

// An sRGB view encodes on write: decode first so the film's values land as they are.
@fragment
fn fs_srgb(@builtin(position) pos: vec4<f32>) -> @location(0) vec4<f32> {
    return vec4<f32>(srgb_to_linear(rgb_at(pos)), 1.0);
}
";

impl HdGpu {
    pub(crate) fn new(
        device: &wgpu::Device,
        format: wgpu::TextureFormat,
        layout: &wgpu::BindGroupLayout,
    ) -> Self {
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("Cutscene HD Pipeline Layout"),
            bind_group_layouts: &[Some(layout)],
            immediate_size: 0,
        });
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Cutscene HD Shader"),
            source: wgpu::ShaderSource::Wgsl(SHADER.into()),
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("Cutscene HD Pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                buffers: &[],
                compilation_options: Default::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some(if format.is_srgb() {
                    "fs_srgb"
                } else {
                    "fs_main"
                }),
                targets: &[Some(wgpu::ColorTargetState {
                    format,
                    blend: None,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: Default::default(),
            }),
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        });
        Self { pipeline }
    }

    pub(crate) fn draw(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        target: &wgpu::TextureView,
        frame: &HdShown,
    ) {
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("Cutscene HD Pass"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                depth_slice: None,
                view: target,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            occlusion_query_set: None,
            timestamp_writes: None,
            multiview_mask: None,
        });
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, &frame.bind_group, &[]);
        pass.draw(0..3, 0..1);
    }
}

fn u16_bytes(samples: &[u16]) -> &[u8] {
    unsafe { std::slice::from_raw_parts(samples.as_ptr().cast::<u8>(), samples.len() * 2) }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ivf_bytes(fourcc: &[u8; 4], frames: &[&[u8]]) -> Vec<u8> {
        let mut out = Vec::new();
        out.extend_from_slice(b"DKIF");
        out.extend_from_slice(&0u16.to_le_bytes());
        out.extend_from_slice(&32u16.to_le_bytes());
        out.extend_from_slice(fourcc);
        out.extend_from_slice(&2560u16.to_le_bytes());
        out.extend_from_slice(&1056u16.to_le_bytes());
        out.extend_from_slice(&50_000u32.to_le_bytes());
        out.extend_from_slice(&3_333u32.to_le_bytes());
        out.extend_from_slice(&(frames.len() as u32).to_le_bytes());
        out.extend_from_slice(&0u32.to_le_bytes());
        for (pts, frame) in frames.iter().enumerate() {
            out.extend_from_slice(&(frame.len() as u32).to_le_bytes());
            out.extend_from_slice(&(pts as u64).to_le_bytes());
            out.extend_from_slice(frame);
        }
        out
    }

    #[test]
    fn the_ivf_reader_takes_the_header_and_each_frame() {
        let data = ivf_bytes(b"AV01", &[b"abc", b"de", b"f"]);
        let ivf = parse_ivf(&data).unwrap();
        assert_eq!(
            ivf.header,
            IvfHeader {
                fourcc: *b"AV01",
                width: 2560,
                height: 1056,
                rate: 50_000,
                scale: 3_333,
                frames: 3,
            }
        );
        let frames: Vec<&[u8]> = ivf.frames.iter().map(|r| &data[r.clone()]).collect();
        assert_eq!(frames, [&b"abc"[..], b"de", b"f"]);
        assert!(
            parse_ivf(&ivf_bytes(b"AV01", &[]))
                .unwrap()
                .frames
                .is_empty()
        );
    }

    #[test]
    fn the_ivf_reader_refuses_a_cut_or_foreign_file() {
        let data = ivf_bytes(b"AV01", &[b"abc", b"de"]);
        assert!(parse_ivf(&data[..data.len() - 1]).is_err());
        assert!(parse_ivf(&data[..32 + 15 + 5]).is_err());
        assert!(parse_ivf(&data[..20]).is_err());
        let mut riff = data.clone();
        riff[0..4].copy_from_slice(b"RIFF");
        assert!(parse_ivf(&riff).is_err());
        let mut version = data;
        version[4] = 1;
        assert!(parse_ivf(&version).is_err());
    }

    #[test]
    fn the_data_folder_is_xdgs_then_homes() {
        let env = |vars: &'static [(&'static str, &'static str)]| {
            move |name: &str| {
                vars.iter()
                    .find(|(k, _)| *k == name)
                    .map(|(_, v)| OsString::from(v))
            }
        };
        let all = &[
            ("XDG_DATA_HOME", "/x"),
            ("HOME", "/h"),
            ("LOCALAPPDATA", "C:\\L"),
        ];
        assert_eq!(data_base_from(env(all), false), Some(PathBuf::from("/x")));
        let no_xdg = &[("XDG_DATA_HOME", ""), ("HOME", "/h")];
        assert_eq!(
            data_base_from(env(no_xdg), false),
            Some(PathBuf::from("/h/.local/share"))
        );
        let windows = &[("LOCALAPPDATA", "C:\\L")];
        assert_eq!(data_base_from(env(windows), false), None);
        assert_eq!(
            data_base_from(env(windows), true),
            Some(PathBuf::from("C:\\L"))
        );
    }

    #[test]
    fn a_tfl_movies_hd_file_is_its_id_in_the_tfl_folder() {
        use mandrake_common::FourCC;
        assert_eq!(tfl_file_stem(FourCC(*b"01cr")), "01cr");
        assert_eq!(tfl_file_stem(FourCC(*b"a/ .")), "a%2F%20%2E");
        assert_eq!(
            hd_file(Path::new("/x").join(TFL_FOLDER).as_path(), "00pr".as_ref()),
            PathBuf::from("/x/tfl/00pr.ivf")
        );
    }

    #[test]
    fn the_hd_file_plays_only_when_whole_and_as_long_as_the_smk() {
        let dir = std::env::temp_dir().join(format!("m2-cutscene-hd-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("prologue.ivf");
        assert_eq!(choose(Some(&path), 2), Choice::Smk(None));
        assert_eq!(choose(None, 2), Choice::Smk(None));
        let whole = ivf_bytes(b"AV01", &[b"abc", b"de"]);
        std::fs::write(&path, &whole).unwrap();
        assert!(
            matches!(choose(Some(&path), 2), Choice::Hd { ref ivf, .. } if ivf.frames.len() == 2)
        );
        let Choice::Smk(Some(why)) = choose(Some(&path), 3) else {
            panic!("a short file played");
        };
        assert!(why.contains("2 frames, the .smk has 3"), "{why}");
        std::fs::write(&path, &whole[..whole.len() - 1]).unwrap();
        assert!(matches!(choose(Some(&path), 2), Choice::Smk(Some(_))));
        std::fs::write(&path, b"not a film").unwrap();
        assert!(matches!(choose(Some(&path), 2), Choice::Smk(Some(_))));
        std::fs::write(&path, ivf_bytes(b"VP90", &[b"abc", b"de"])).unwrap();
        let Choice::Smk(Some(why)) = choose(Some(&path), 2) else {
            panic!("a VP9 file played");
        };
        assert!(why.contains("not AV1"), "{why}");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_corrupt_frame_stops_the_stream_with_its_error() {
        let data = ivf_bytes(b"AV01", &[&[0xffu8; 64][..], &[0x12, 0x00, 0x0a]]);
        let ivf = parse_ivf(&data).unwrap();
        let frames = start_decoder(data, ivf).unwrap();
        let failed = frames.iter().find_map(|message| match message {
            HdMessage::Failed(e) => Some(e),
            HdMessage::Frame(_) => None,
        });
        let e = failed.expect("the garbage decoded");
        assert!(e.starts_with("frame "), "{e}");
    }

    #[test]
    fn the_hd_prologue_decodes_faster_than_it_plays() {
        let Some(path) = std::env::var_os("MYTH2_HD_PROLOGUE") else {
            eprintln!("skipped: MYTH2_HD_PROLOGUE names no file");
            return;
        };
        let env_num = |name: &str, default: u32| {
            std::env::var(name)
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(default)
        };
        let (count, rate) = (
            env_num("MYTH2_HD_FRAMES", 1704),
            env_num("MYTH2_HD_FPS", 15),
        );
        let Choice::Hd { data, ivf } = choose(Some(Path::new(&path)), count) else {
            panic!("{path:?} isn't a {count}-frame HD film");
        };
        let (w, h) = (u32::from(ivf.header.width), u32::from(ivf.header.height));
        let frames = 240.min(ivf.frames.len());
        let started = Instant::now();
        let decoded = start_decoder(data, ivf).unwrap();
        let mut taken = 0u32;
        let mut first = None;
        while taken < frames as u32 {
            match decoded.recv().unwrap() {
                HdMessage::Failed(e) => panic!("{e}"),
                HdMessage::Frame(frame) => {
                    assert_eq!(frame.index, taken);
                    assert_eq!(frame.size, (w, h));
                    assert_eq!(frame.y.len(), (w * h) as usize);
                    assert_eq!(frame.u.len(), (w / 2 * h / 2) as usize);
                    assert!(frame.y.iter().all(|s| *s < 1024));
                    if taken == 0 {
                        first = Some(started.elapsed());
                    }
                    taken += 1;
                }
            }
        }
        let spent = started.elapsed();
        let fps = f64::from(taken) / spent.as_secs_f64();
        eprintln!(
            "hd film ({w}x{h}): {taken} frames in {:.2} s ({fps:.1} fps), first after {:.0} ms",
            spent.as_secs_f64(),
            first.unwrap().as_secs_f64() * 1e3
        );
        assert!(fps > f64::from(rate) * 1.5, "{fps:.1} fps");
    }
}
