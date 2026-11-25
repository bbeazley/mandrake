use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use crate::console::ConsoleLine;
use anyhow::Context;
use mandrake_view::camera::Camera;

pub(crate) const COMMIT: &str = env!("M2_CLIENT_COMMIT");
const SLUG_LIMIT: usize = 40;

static VIEW_CAMERA: OnceLock<[f32; 6]> = OnceLock::new();

pub(crate) fn set_view_camera(values: &[f32]) -> anyhow::Result<()> {
    let values: [f32; 6] = values.try_into().map_err(|_| {
        anyhow::anyhow!(
            "--view-camera takes TX,TY,TZ,DISTANCE,YAW,PITCH, got {} values",
            values.len()
        )
    })?;
    let _ = VIEW_CAMERA.set(values);
    Ok(())
}

pub(crate) fn frame_camera(camera: &Camera) -> Camera {
    let mut camera = camera.clone();
    if let Some(&[x, y, z, distance, yaw, pitch]) = VIEW_CAMERA.get() {
        camera.target = [x, y, z];
        camera.distance = distance;
        camera.yaw = yaw;
        camera.pitch = pitch;
    }
    camera
}

pub(crate) fn newest_subtitle(lines: &[ConsoleLine]) -> Option<Vec<u8>> {
    let mut subtitles = lines
        .iter()
        .filter(|line| line.player == -1 && !is_client_notice(&line.text));
    let newest = subtitles.next()?;
    let mut parts: Vec<&[u8]> = vec![&newest.text];
    parts.extend(
        subtitles
            .take_while(|line| line.stamp == newest.stamp)
            .map(|line| line.text.as_slice()),
    );
    parts.reverse();
    Some(parts.join(&b' '))
}

fn is_client_notice(text: &[u8]) -> bool {
    [
        SAVED_PREFIX.as_bytes(),
        FAILED_PREFIX.as_bytes(),
        b"Autosave created.",
        b"Quicksave created.",
    ]
    .iter()
    .any(|notice| text.starts_with(notice))
}

const SAVED_PREFIX: &str = "Screenshot saved: ";
const FAILED_PREFIX: &str = "Screenshot failed: ";

pub(crate) fn saved_line(path: &Path) -> String {
    let name = path.file_name().map_or_else(
        || path.display().to_string(),
        |n| n.to_string_lossy().into_owned(),
    );
    format!("{SAVED_PREFIX}{name}")
}

pub(crate) fn failed_line(error: &anyhow::Error) -> String {
    format!("{FAILED_PREFIX}{error}")
}

pub(crate) fn subtitle_slug(text: &[u8]) -> String {
    let cleaned: String = text
        .iter()
        .filter(|&&b| b != b'\'')
        .map(|&b| {
            if b.is_ascii_alphanumeric() {
                b.to_ascii_lowercase() as char
            } else {
                ' '
            }
        })
        .collect();
    let mut slug = String::new();
    for word in cleaned.split_whitespace() {
        if !slug.is_empty() && slug.len() + 1 + word.len() > SLUG_LIMIT {
            break;
        }
        if !slug.is_empty() {
            slug.push('-');
        }
        slug.push_str(word);
    }
    slug.truncate(SLUG_LIMIT);
    slug
}

pub(crate) fn file_stem(name: &str, tick: u32, subtitle: Option<&[u8]>, time: &str) -> String {
    let name: String = name
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '_' || c == '-' {
                c
            } else {
                '_'
            }
        })
        .collect();
    match subtitle.map(subtitle_slug).filter(|s| !s.is_empty()) {
        Some(slug) => format!("{name}_t{tick:05}_{slug}_{time}"),
        None => format!("{name}_t{tick:05}_{time}"),
    }
}

pub(crate) fn screenshot_dir(
    env_pictures: Option<&str>,
    user_dirs: Option<&str>,
    home: Option<&str>,
) -> PathBuf {
    let from_file = user_dirs.and_then(|text| {
        let line = text
            .lines()
            .map(str::trim)
            .find(|l| l.starts_with("XDG_PICTURES_DIR="))?;
        let value = line["XDG_PICTURES_DIR=".len()..].trim_matches('"');
        let expanded = match (value.strip_prefix("$HOME"), home) {
            (Some(rest), Some(home)) => format!("{home}{rest}"),
            (Some(_), None) => return None,
            (None, _) => value.to_owned(),
        };
        (Path::new(&expanded) != Path::new(home.unwrap_or(""))).then_some(expanded)
    });
    let pictures = env_pictures
        .filter(|p| !p.is_empty())
        .map(PathBuf::from)
        .or_else(|| from_file.map(PathBuf::from))
        .or_else(|| home.map(|h| Path::new(h).join("Pictures")))
        .unwrap_or_else(std::env::temp_dir);
    pictures.join("Screenshots")
}

pub(crate) fn user_screenshot_dir() -> PathBuf {
    let var = |name: &str| std::env::var(name).ok().filter(|v| !v.is_empty());
    let home = crate::user_dirs::pictures_home();
    let user_dirs = crate::user_dirs::config_base()
        .and_then(|c| std::fs::read_to_string(c.join("user-dirs.dirs")).ok());
    screenshot_dir(
        var("XDG_PICTURES_DIR").as_deref(),
        user_dirs.as_deref(),
        home.as_deref(),
    )
}

pub(crate) fn time_of_day() -> String {
    let local = std::process::Command::new("date")
        .arg("+%H%M%S")
        .output()
        .ok()
        .filter(|o| o.status.success())
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|s| s.trim().to_owned())
        .filter(|s| s.len() == 6);
    local.unwrap_or_else(|| {
        let now = crate::clock::SystemTime::now()
            .duration_since(crate::clock::UNIX_EPOCH)
            .map_or(0, |d| d.as_secs());
        format!("{:02}{:02}{:02}", now / 3600 % 24, now / 60 % 60, now % 60)
    })
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct ShotInfo {
    pub film: Option<PathBuf>,
    pub mesh: String,
    pub mesh_id: String,
    pub difficulty: Option<i16>,
    pub tick: u32,
    pub subtitle: Option<Vec<u8>>,
    pub size: (u32, u32),
    pub observer: [f32; 6],
    pub view_camera: [f32; 6],
    pub game_speed: i16,
    pub paused: bool,
    pub selection: Vec<u32>,
    pub fraction: i32,
    pub sampling: String,
    pub fog: crate::args::Fog,
    pub fog_seconds: f32,
    pub reflection_soft_edge: bool,
    pub hover: Option<(f32, f32)>,
    pub caps_lock: bool,
    pub overhead_map: bool,
    pub ui_scale: Option<String>,
    pub argv: Vec<String>,
    pub cwd: Option<PathBuf>,
}

fn speed_text(speed: i16) -> String {
    match speed {
        0 => "0".to_owned(),
        s if s < 256 => format!("1/{}", 256 / s),
        s => (s / 256).to_string(),
    }
}

const REPLACED_ARGS: [(&str, bool); 23] = [
    ("--ticks", true),
    ("--frame-fraction", true),
    ("--screenshot", true),
    ("--observer", true),
    ("--view-camera", true),
    ("--select-ids", true),
    ("--width", true),
    ("--height", true),
    ("--game-speed", true),
    ("--sampling", true),
    ("--fog", true),
    ("--fog-time", true),
    ("--reflection-soft-edge", true),
    ("--ui-scale", true),
    ("--film-report", true),
    ("--hover", true),
    ("--hud", false),
    ("--caps-lock", false),
    ("--headless", false),
    ("--session", false),
    ("--session-script", true),
    ("--force-end", true),
    ("--rebuild-first", true),
];

const SOLO_MAP_ARGS: [(&str, bool); 5] = [
    ("--mesh", true),
    ("-m", true),
    ("--play", true),
    ("--difficulty", true),
    ("--input-film", true),
];

fn difficulty_name(difficulty: i16) -> &'static str {
    match difficulty {
        0 => "Timid",
        1 => "Simple",
        2 => "Normal",
        3 => "Heroic",
        4 => "Legendary",
        _ => "?",
    }
}

impl ShotInfo {
    pub(crate) fn name(&self) -> String {
        self.film
            .as_deref()
            .and_then(Path::file_stem)
            .map_or_else(|| self.mesh.clone(), |s| s.to_string_lossy().into_owned())
    }

    fn floats(values: &[f32]) -> String {
        let text: Vec<String> = values.iter().map(|v| format!("{v:?}")).collect();
        text.join(",")
    }

    pub(crate) fn reproduce_args(&self, output: &str) -> Vec<String> {
        let mut out = Vec::new();
        let solo: &[(&str, bool)] = if self.film.is_none() {
            &SOLO_MAP_ARGS
        } else {
            &[]
        };
        let mut args = self.argv.iter();
        while let Some(arg) = args.next() {
            let flag = arg.split('=').next().unwrap_or(arg);
            match REPLACED_ARGS
                .iter()
                .chain(solo)
                .find(|(name, _)| *name == flag)
            {
                Some(&(_, true)) if !arg.contains('=') => {
                    args.next();
                }
                Some(_) => {}
                None => out.push(arg.clone()),
            }
        }
        if self.film.is_none() {
            out.extend(["--play".to_owned(), self.mesh_id.clone()]);
            out.extend([
                "--difficulty".to_owned(),
                self.difficulty.unwrap_or(0).to_string(),
            ]);
        }
        let [x, y, _, facing, _, zoom] = self.observer;
        for (flag, value) in [
            ("--ticks", self.tick.to_string()),
            ("--frame-fraction", format!("{:?}", self.fraction_ratio())),
            ("--width", self.size.0.to_string()),
            ("--height", self.size.1.to_string()),
            ("--game-speed", speed_text(self.game_speed)),
            ("--sampling", self.sampling.clone()),
            ("--fog", self.fog.name().to_owned()),
            ("--fog-time", format!("{:?}", self.fog_seconds)),
            (
                "--reflection-soft-edge",
                if self.reflection_soft_edge {
                    "on"
                } else {
                    "off"
                }
                .to_owned(),
            ),
            ("--observer", Self::floats(&[x, y, facing, zoom])),
            ("--view-camera", Self::floats(&self.view_camera)),
        ] {
            out.extend([flag.to_owned(), value]);
        }
        out.push("--hud".to_owned());
        if !self.selection.is_empty() {
            let ids: Vec<String> = self.selection.iter().map(u32::to_string).collect();
            out.extend(["--select-ids".to_owned(), ids.join(",")]);
        }
        if let Some((x, y)) = self.hover {
            out.extend(["--hover".to_owned(), Self::floats(&[x, y])]);
        }
        if self.caps_lock {
            out.push("--caps-lock".to_owned());
        }
        if !self.overhead_map && self.tick > 0 {
            let toggle = self.tick.saturating_sub(60);
            out.extend(["--minimap-toggle".to_owned(), toggle.to_string()]);
        }
        if let Some(scale) = &self.ui_scale {
            out.extend(["--ui-scale".to_owned(), scale.clone()]);
        }
        out.extend(["--screenshot".to_owned(), output.to_owned()]);
        out
    }

    fn fraction_ratio(&self) -> f32 {
        self.fraction as f32 / 65536.0
    }

    pub(crate) fn sidecar(&self, image: &Path) -> String {
        let [ox, oy, oz, facing, pitch, zoom] = self.observer;
        let selection: Vec<String> = self.selection.iter().map(u32::to_string).collect();
        let subtitle = self
            .subtitle
            .as_deref()
            .map_or(String::new(), |s| String::from_utf8_lossy(s).into_owned());
        let launch: Vec<String> = self.argv.iter().map(|a| shell_quote(a)).collect();
        let reproduce: Vec<String> = self
            .reproduce_args("reproduced.png")
            .iter()
            .map(|a| shell_quote(a))
            .collect();
        let mut text = format!(
            "# Myth II client screenshot: what reproduces this view.\n\
             image = {image}\n\
             film = {film}\n\
             mesh = {mesh}\n\
             mesh_id = {mesh_id}\n\
             difficulty = {difficulty}\n\
             tick = {tick}\n\
             subtitle = {subtitle}\n\
             frame_fraction = {fraction}/65536\n\
             window = {w}x{h}\n\
             observer_position = {ox:?} {oy:?} {oz:?}\n\
             facing = {facing:?}\n\
             pitch = {pitch:?}\n\
             zoom = {zoom:?}\n\
             view_camera = {camera}\n\
             game_speed = {speed} (of normal){paused}\n\
             selection = {selection}\n\
             overhead_map = {map}\n\
             sampling = {sampling}\n\
             fog = {fog}\n\
             fog_time = {fog_time:?}\n\
             reflection_soft_edge = {reflection_soft_edge}\n\
             commit = {COMMIT}\n\
             cwd = {cwd}\n\
             launch = {launch}\n",
            image = image.display(),
            film = self
                .film
                .as_deref()
                .map_or_else(String::new, |f| f.display().to_string()),
            mesh = self.mesh,
            mesh_id = self.mesh_id,
            difficulty = self
                .difficulty
                .map_or_else(String::new, |d| format!("{d} ({})", difficulty_name(d))),
            tick = self.tick,
            fraction = self.fraction,
            w = self.size.0,
            h = self.size.1,
            camera = Self::floats(&self.view_camera),
            speed = speed_text(self.game_speed),
            paused = if self.paused { ", paused" } else { "" },
            selection = selection.join(","),
            map = if self.overhead_map { "shown" } else { "hidden" },
            sampling = self.sampling,
            fog = self.fog.name(),
            fog_time = self.fog_seconds,
            reflection_soft_edge = if self.reflection_soft_edge {
                "on"
            } else {
                "off"
            },
            cwd = self
                .cwd
                .as_deref()
                .map_or_else(String::new, |c| c.display().to_string()),
            launch = launch.join(" "),
        );
        if self.film.is_none() {
            text.push_str(
                "# A solo game's input isn't recorded: the line below gives the map and the view, \
                 not the units.\n",
            );
        }
        text.push_str(&format!("reproduce = {}\n", reproduce.join(" ")));
        text
    }
}

fn shell_quote(arg: &str) -> String {
    let plain = !arg.is_empty()
        && arg
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "-_./=,:+%".contains(c));
    if plain {
        arg.to_owned()
    } else {
        format!("'{}'", arg.replace('\'', "'\\''"))
    }
}

pub(crate) fn write_shot(
    dir: &Path,
    info: &ShotInfo,
    time: &str,
    (width, height): (u32, u32),
    pixels: &Pixels,
) -> anyhow::Result<PathBuf> {
    std::fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
    let stem = file_stem(&info.name(), info.tick, info.subtitle.as_deref(), time);
    let mut path = dir.join(format!("{stem}.png"));
    let mut number = 2;
    while path.exists() {
        path = dir.join(format!("{stem}_{number}.png"));
        number += 1;
    }
    match pixels {
        Pixels::Rgba8(rgba) => {
            image::save_buffer(&path, rgba, width, height, image::ExtendedColorType::Rgba8)
        }
        Pixels::Rgba16(rgba) => {
            image::ImageBuffer::<image::Rgba<u16>, _>::from_raw(width, height, rgba.as_slice())
                .ok_or_else(|| {
                    image::ImageError::Parameter(image::error::ParameterError::from_kind(
                        image::error::ParameterErrorKind::DimensionMismatch,
                    ))
                })
                .and_then(|buffer| buffer.save(&path))
        }
    }
    .with_context(|| format!("writing {}", path.display()))?;
    let sidecar = path.with_extension("txt");
    std::fs::write(&sidecar, info.sidecar(&path))
        .with_context(|| format!("writing {}", sidecar.display()))?;
    Ok(path)
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Pixels {
    Rgba8(Vec<u8>),
    Rgba16(Vec<u16>),
}

impl Pixels {
    pub(crate) fn to_rgba8(&self) -> std::borrow::Cow<'_, [u8]> {
        match self {
            Self::Rgba8(rgba) => std::borrow::Cow::Borrowed(rgba),
            Self::Rgba16(rgba) => std::borrow::Cow::Owned(
                rgba.iter()
                    .map(|&c| ((u32::from(c) * 255 + 32767) / 65535) as u8)
                    .collect(),
            ),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Layout {
    Rgba8,
    Bgra8,
    Rgb10a2,
    Rgba16Float,
}

pub(crate) struct Readback {
    buffer: wgpu::Buffer,
    size: (u32, u32),
    bytes_per_row: u32,
    layout: Layout,
}

impl Readback {
    pub(crate) fn encode(
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        texture: &wgpu::Texture,
    ) -> Option<Self> {
        use wgpu::TextureFormat::*;
        let layout = match texture.format() {
            Bgra8Unorm | Bgra8UnormSrgb => Layout::Bgra8,
            Rgba8Unorm | Rgba8UnormSrgb => Layout::Rgba8,
            Rgb10a2Unorm => Layout::Rgb10a2,
            Rgba16Float => Layout::Rgba16Float,
            _ => return None,
        };
        let pixel = if layout == Layout::Rgba16Float { 8 } else { 4 };
        if !texture.usage().contains(wgpu::TextureUsages::COPY_SRC) {
            return None;
        }
        let (width, height) = (texture.width(), texture.height());
        let bytes_per_row = (width * pixel).next_multiple_of(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT);
        let buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Screenshot"),
            size: u64::from(bytes_per_row) * u64::from(height),
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        encoder.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo {
                texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::TexelCopyBufferInfo {
                buffer: &buffer,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(bytes_per_row),
                    rows_per_image: Some(height),
                },
            },
            wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
        );
        Some(Self {
            buffer,
            size: (width, height),
            bytes_per_row,
            layout,
        })
    }

    pub(crate) fn read(self, device: &wgpu::Device) -> anyhow::Result<((u32, u32), Pixels)> {
        let slice = self.buffer.slice(..);
        let (tx, rx) = std::sync::mpsc::channel();
        slice.map_async(wgpu::MapMode::Read, move |result| {
            let _ = tx.send(result);
        });
        let _ = device.poll(wgpu::PollType::wait_indefinitely());
        rx.recv()??;
        let data = slice.get_mapped_range()?;
        let pixels = match self.layout {
            Layout::Rgb10a2 => Pixels::Rgba16(tight_rgb10a2(&data, self.size, self.bytes_per_row)),
            Layout::Rgba16Float => {
                let (width, height) = self.size;
                let rows: Vec<u8> = data
                    .chunks_exact(self.bytes_per_row as usize)
                    .take(height as usize)
                    .flat_map(|row| &row[..(width * 8) as usize])
                    .copied()
                    .collect();
                Pixels::Rgba16(canvas_to_rgba16(&rows))
            }
            layout => Pixels::Rgba8(tight_rgba(
                &data,
                self.size,
                self.bytes_per_row,
                layout == Layout::Bgra8,
            )),
        };
        drop(data);
        self.buffer.unmap();
        Ok((self.size, pixels))
    }
}

fn tight_rgba(data: &[u8], (width, height): (u32, u32), bytes_per_row: u32, bgra: bool) -> Vec<u8> {
    let mut out = Vec::with_capacity((width * height * 4) as usize);
    for row in data
        .chunks_exact(bytes_per_row as usize)
        .take(height as usize)
    {
        for pixel in row[..(width * 4) as usize].as_chunks::<4>().0 {
            let (r, b) = if bgra {
                (pixel[2], pixel[0])
            } else {
                (pixel[0], pixel[2])
            };
            out.extend_from_slice(&[r, pixel[1], b, 255]);
        }
    }
    out
}

fn half_to_f32(bits: u16) -> f32 {
    let sign = if bits & 0x8000 != 0 { -1.0 } else { 1.0 };
    let exponent = i32::from((bits >> 10) & 0x1f);
    let fraction = f32::from(bits & 0x3ff);
    sign * match exponent {
        0 => fraction * 2f32.powi(-24),
        31 => f32::INFINITY,
        e => (1.0 + fraction / 1024.0) * 2f32.powi(e - 15),
    }
}

pub(crate) fn canvas_to_ten_bit_rgba16(data: &[u8]) -> Vec<u16> {
    let ten = |bits: u16| {
        let v = (half_to_f32(bits).clamp(0.0, 1.0) * 1023.0).round() as u16;
        (v << 6) | (v >> 4)
    };
    data.as_chunks::<8>()
        .0
        .iter()
        .flat_map(|p| {
            let c = |i: usize| ten(u16::from_le_bytes([p[i], p[i + 1]]));
            [c(0), c(2), c(4), u16::MAX]
        })
        .collect()
}

fn canvas_to_rgba16(data: &[u8]) -> Vec<u16> {
    let full = |bits: u16| (half_to_f32(bits).clamp(0.0, 1.0) * 65535.0).round() as u16;
    data.as_chunks::<8>()
        .0
        .iter()
        .flat_map(|p| {
            let c = |i: usize| full(u16::from_le_bytes([p[i], p[i + 1]]));
            [c(0), c(2), c(4), u16::MAX]
        })
        .collect()
}

fn tight_rgb10a2(data: &[u8], (width, height): (u32, u32), bytes_per_row: u32) -> Vec<u16> {
    let widen = |v: u32| {
        let v = (v & 0x3ff) as u16;
        (v << 6) | (v >> 4)
    };
    let mut out = Vec::with_capacity((width * height * 4) as usize);
    for row in data
        .chunks_exact(bytes_per_row as usize)
        .take(height as usize)
    {
        for pixel in row[..(width * 4) as usize].as_chunks::<4>().0 {
            let word = u32::from_le_bytes(*pixel);
            out.extend_from_slice(&[widen(word), widen(word >> 10), widen(word >> 20), u16::MAX]);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::args::Args;
    use clap::Parser;

    fn shot() -> ShotInfo {
        ShotInfo {
            film: Some(PathBuf::from("films/salvation_vic_new.m2rec")),
            mesh: "Salvation".to_owned(),
            mesh_id: "cr01".to_owned(),
            difficulty: Some(2),
            tick: 2500,
            subtitle: Some(b"Since the end of the Great War...".to_vec()),
            size: (1280, 720),
            observer: [10.5, 20.25, 8.0, 1.5, 5.81187, 0.0],
            view_camera: [1.0, -2.0, 3.0, 4.5, 0.1, 0.47],
            game_speed: 128,
            paused: false,
            selection: vec![7, 9],
            fraction: 0x8000,
            sampling: "linear".to_owned(),
            fog: crate::args::Fog::Engine,
            fog_seconds: 12.5,
            reflection_soft_edge: false,
            hover: None,
            caps_lock: false,
            overhead_map: true,
            ui_scale: None,
            argv: [
                "mandrake",
                "--replay",
                "films/salvation_vic_new.m2rec",
                "--film-report",
                "draws.txt",
                "--game-speed=2",
                "--width",
                "800",
                "--master-volume",
                "0.1",
            ]
            .map(str::to_owned)
            .to_vec(),
            cwd: Some(PathBuf::from("/game")),
        }
    }

    #[test]
    fn the_name_has_the_film_tick_subtitle_and_time() {
        let info = shot();
        assert_eq!(
            file_stem(&info.name(), 2500, info.subtitle.as_deref(), "184326"),
            "salvation_vic_new_t02500_since-the-end-of-the-great-war_184326"
        );
        assert_eq!(file_stem("cr01", 7, None, "000102"), "cr01_t00007_000102");
        assert_eq!(
            file_stem("cr 01", 7, Some(b"..."), "000102"),
            "cr_01_t00007_000102"
        );
    }

    #[test]
    fn a_wrapped_subtitle_is_one_text() {
        let line = |player, stamp, text: &str| ConsoleLine {
            player,
            stamp,
            text: text.as_bytes().to_vec(),
        };
        let lines = [
            line(0, 90, "chat"),
            line(-1, 70, "cemetery."),
            line(-1, 70, "Sir, they are holding the villagers captive in the"),
            line(-1, 10, "Older."),
        ];
        assert_eq!(
            newest_subtitle(&lines).as_deref(),
            Some(b"Sir, they are holding the villagers captive in the cemetery.".as_slice())
        );
        assert_eq!(newest_subtitle(&lines[..1]), None);
        assert_eq!(newest_subtitle(&[]), None);
    }

    #[test]
    fn a_slug_is_short_ascii_words() {
        assert_eq!(subtitle_slug(b"Don't  stop, ALAN!"), "dont-stop-alan");
        assert_eq!(subtitle_slug(b"caf\xe9 au lait"), "caf-au-lait");
        let long = subtitle_slug(b"one two three four five six seven eight nine ten eleven");
        assert!(long.len() <= SLUG_LIMIT, "{long}");
        assert!(long.ends_with("eight"), "{long}");
        assert_eq!(subtitle_slug(&[b'x'; 100]).len(), SLUG_LIMIT);
    }

    #[test]
    fn the_sidecar_says_what_reproduces_the_view() {
        let text = shot().sidecar(Path::new("/p/a.png"));
        for line in [
            "image = /p/a.png",
            "film = films/salvation_vic_new.m2rec",
            "mesh = Salvation",
            "mesh_id = cr01",
            "difficulty = 2 (Normal)",
            "tick = 2500",
            "subtitle = Since the end of the Great War...",
            "frame_fraction = 32768/65536",
            "window = 1280x720",
            "observer_position = 10.5 20.25 8.0",
            "facing = 1.5",
            "zoom = 0.0",
            "view_camera = 1.0,-2.0,3.0,4.5,0.1,0.47",
            "game_speed = 1/2 (of normal)",
            "selection = 7,9",
            "overhead_map = shown",
            "sampling = linear",
            "fog = engine",
            "fog_time = 12.5",
            "cwd = /game",
        ] {
            assert!(text.lines().any(|l| l == line), "{line:?} in\n{text}");
        }
        let commit = format!("commit = {COMMIT}");
        assert!(text.contains(&commit), "{text}");
        assert!(text.contains("launch = mandrake --replay"), "{text}");
        assert!(text.contains("reproduce = mandrake --replay"), "{text}");
    }

    #[test]
    fn the_reproducing_line_parses_to_the_same_frame() {
        let args = Args::try_parse_from(shot().reproduce_args("out.png")).unwrap();
        assert_eq!(args.ticks, 2500);
        assert_eq!(args.frame_fraction, 0.5);
        assert_eq!((args.width, args.height), (1280, 720));
        assert_eq!(args.game_speed, 4);
        assert_eq!(args.fog, Some(crate::args::Fog::Engine));
        assert_eq!(args.fog_time, Some(12.5));
        assert_eq!(
            args.reflection_soft_edge,
            Some(crate::args::ReflectionSoftEdge::Off)
        );
        assert_eq!(args.sampling, crate::ui_scale::Sampling::Linear);
        assert_eq!(args.observer, Some(vec![10.5, 20.25, 1.5, 0.0]));
        assert_eq!(args.view_camera, Some(vec![1.0, -2.0, 3.0, 4.5, 0.1, 0.47]));
        assert_eq!(args.select_ids, Some(vec![7, 9]));
        assert!(args.hud);
        assert_eq!(args.screenshot, Some(PathBuf::from("out.png")));
        assert_eq!(args.film_report, None);
        assert_eq!(args.master_volume, Some(0.1));
        assert!(args.minimap_toggle.is_empty());
        let hidden = ShotInfo {
            overhead_map: false,
            ..shot()
        };
        let args = Args::try_parse_from(hidden.reproduce_args("out.png")).unwrap();
        assert_eq!(args.minimap_toggle, [2440]);
    }

    fn solo_shot(argv: &[&str]) -> ShotInfo {
        ShotInfo {
            film: None,
            mesh: "Down a Broken Path".to_owned(),
            mesh_id: "le3e".to_owned(),
            difficulty: Some(3),
            argv: argv.iter().map(|a| (*a).to_owned()).collect(),
            ..shot()
        }
    }

    #[test]
    fn a_solo_games_reproducing_line_plays_its_mesh_at_its_difficulty() {
        let menu = solo_shot(&["mandrake", "--master-volume", "0.1"]);
        let line = menu.reproduce_args("out.png");
        let at = line.iter().position(|a| a == "--play").unwrap();
        assert_eq!(line[at..at + 4], ["--play", "le3e", "--difficulty", "3"]);
        assert!(!line.contains(&"--mesh".to_owned()), "{line:?}");
        let args = Args::try_parse_from(&line).unwrap();
        assert_eq!(args.play.as_deref(), Some("le3e"));
        assert_eq!(args.difficulty, 3);
        assert_eq!(args.mesh, None);
        assert_eq!(args.replay, None);
        assert_eq!(args.ticks, 2500);
        assert_eq!(args.master_volume, Some(0.1));
        let text = menu.sidecar(Path::new("/p/a.png"));
        assert!(
            text.contains("reproduce = mandrake --master-volume 0.1 --play le3e --difficulty 3 "),
            "{text}"
        );
        assert!(text.lines().any(|l| l == "mesh_id = le3e"), "{text}");
        assert!(
            text.lines().any(|l| l == "difficulty = 3 (Heroic)"),
            "{text}"
        );

        for argv in [
            &[
                "mandrake",
                "--play",
                "le3e",
                "--session",
                "--difficulty",
                "1",
            ][..],
            &["mandrake", "-m", "le3e", "--difficulty=4"][..],
            &["mandrake", "--mesh=le3", "--input-film", "a.m2rec"][..],
        ] {
            let line = solo_shot(argv).reproduce_args("out.png");
            for flag in ["--play", "--difficulty"] {
                assert_eq!(line.iter().filter(|a| *a == flag).count(), 1, "{line:?}");
            }
            let args = Args::try_parse_from(&line).unwrap_or_else(|e| panic!("{line:?}: {e}"));
            assert_eq!(args.play.as_deref(), Some("le3e"), "{line:?}");
            assert_eq!(args.difficulty, 3, "{line:?}");
            assert_eq!((args.mesh, args.session), (None, false), "{line:?}");
        }

        let film = shot().reproduce_args("out.png");
        for flag in ["--mesh", "--play", "--difficulty"] {
            assert!(!film.contains(&flag.to_owned()), "{film:?}");
        }
    }

    #[test]
    fn every_speed_of_the_table_parses_back() {
        for (index, &speed) in crate::game_speed::GAME_SPEEDS.iter().enumerate() {
            assert_eq!(
                crate::game_speed::parse_speed(&speed_text(speed)),
                Ok(index as i16),
                "{speed}"
            );
        }
    }

    #[test]
    fn a_blue_first_surface_comes_out_red_first_and_opaque() {
        let data = [
            1, 2, 3, 0, 4, 5, 6, 0, 9, 9, 9, 9, 7, 8, 9, 0, 10, 11, 12, 0, 9, 9, 9, 9,
        ];
        assert_eq!(
            tight_rgba(&data, (2, 2), 12, true),
            [3, 2, 1, 255, 6, 5, 4, 255, 9, 8, 7, 255, 12, 11, 10, 255]
        );
        assert_eq!(tight_rgba(&data, (1, 1), 12, false), [1, 2, 3, 255]);
    }

    #[test]
    fn a_second_shot_in_one_second_keeps_the_first() {
        let dir = std::env::temp_dir().join(format!("m2-shot-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let info = shot();
        let pixels = [10u8, 20, 30, 255];
        let first = write_shot(
            &dir,
            &info,
            "120000",
            (1, 1),
            &Pixels::Rgba8(pixels.to_vec()),
        )
        .unwrap();
        let second = write_shot(
            &dir,
            &info,
            "120000",
            (1, 1),
            &Pixels::Rgba8(pixels.to_vec()),
        )
        .unwrap();
        assert_ne!(first, second);
        assert!(first.exists() && second.exists());
        assert!(second.with_extension("txt").exists());
        assert_eq!(
            image::open(&first).unwrap().to_rgba8().as_raw().as_slice(),
            pixels
        );
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_ten_bit_frame_is_a_sixteen_bit_png() {
        let word = |r: u32, g: u32, b: u32| (r | g << 10 | b << 20).to_le_bytes();
        let data = [word(1023, 512, 1), word(0, 0, 1023), [9; 4]].concat();
        let pixels = tight_rgb10a2(&data, (2, 1), 12);
        assert_eq!(pixels, [65535, 32800, 64, 65535, 0, 0, 65535, 65535]);
        let pixels = Pixels::Rgba16(pixels);
        assert_eq!(&pixels.to_rgba8()[..4], &[255, 128, 0, 255]);
        let canvas = [0x3c00u16, 0x3800, 0x1400, 0x0000]
            .iter()
            .flat_map(|h| h.to_le_bytes())
            .collect::<Vec<u8>>();
        assert_eq!(canvas_to_ten_bit_rgba16(&canvas), [65535, 32800, 64, 65535]);
        assert_eq!(canvas_to_rgba16(&canvas), [65535, 32768, 64, 65535]);
        let dir = std::env::temp_dir().join(format!("m2-shot16-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let path = write_shot(&dir, &shot(), "120000", (2, 1), &pixels).unwrap();
        let back = image::open(&path).unwrap();
        assert_eq!(back.color(), image::ColorType::Rgba16);
        assert_eq!(
            back.to_rgba16().as_raw().as_slice(),
            [65535, 32800, 64, 65535, 0, 0, 65535, 65535]
        );
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
