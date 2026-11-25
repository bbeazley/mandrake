pub(crate) use mandrake_view::billboard::*;

use crate::pipeline::RenderPipelineConfig;
use crate::vertex::VertexLayout;
use mandrake_tag::collection::{BitmapFrame, CollectionTag};
use wgpu::util::DeviceExt;

pub const BILLBOARD_INSTANCE_ATTRIBS: [wgpu::VertexAttribute; 10] = wgpu::vertex_attr_array![
    0 => Float32x3,
    1 => Uint32,
    2 => Float32x2,
    3 => Float32x2,
    4 => Float32x2,
    5 => Float32x4,
    6 => Float32x2,
    7 => Float32x4,
    8 => Float32x4,
    9 => Float32x4,
];

impl VertexLayout for BillboardInstance {
    fn desc() -> wgpu::VertexBufferLayout<'static> {
        wgpu::VertexBufferLayout {
            array_stride: core::mem::size_of::<Self>() as wgpu::BufferAddress,
            step_mode: wgpu::VertexStepMode::Instance,
            attributes: &BILLBOARD_INSTANCE_ATTRIBS,
        }
    }
}

pub struct TextureAtlas {
    pub texture: wgpu::Texture,
    pub view: wgpu::TextureView,
    pub sampler: wgpu::Sampler,
    pub bind_group: wgpu::BindGroup,
    pub width: u32,
    pub height: u32,
    pub frames: Vec<AtlasFrame>,
}

impl TextureAtlas {
    pub fn frame(&self, index: usize) -> Option<&AtlasFrame> {
        self.frames.get(index)
    }

    pub fn frame_count(&self) -> usize {
        self.frames.len()
    }

    pub fn create_instance(
        &self,
        frame_index: usize,
        position: [f32; 3],
        world_scale: f32,
    ) -> Option<BillboardInstance> {
        self.create_instance_with_flip(frame_index, position, world_scale, false)
    }

    pub fn create_instance_with_world_size(
        &self,
        frame_index: usize,
        position: [f32; 3],
        size: [f32; 2],
        flip_x: bool,
    ) -> Option<BillboardInstance> {
        let frame = self.frame(frame_index)?;
        Some(BillboardInstance::from_atlas_frame(
            frame, position, size, flip_x,
        ))
    }

    pub fn create_instance_with_flip(
        &self,
        frame_index: usize,
        position: [f32; 3],
        world_scale: f32,
        flip_x: bool,
    ) -> Option<BillboardInstance> {
        let frame = self.frame(frame_index)?;
        let size = [
            (frame.width as f32 / 100.0) * world_scale,
            (frame.height as f32 / 100.0) * world_scale,
        ];
        Some(BillboardInstance::from_atlas_frame(
            frame, position, size, flip_x,
        ))
    }
}

pub fn frame_to_rgba_image(frame: &BitmapFrame, palette: &[[u8; 4]]) -> image::RgbaImage {
    let width = frame.width.max(1) as u32;
    let height = frame.height.max(1) as u32;
    let mut img = image::RgbaImage::new(width, height);

    let pixels = &frame.pixels;
    for y in 0..height {
        for x in 0..width {
            let idx = (y * width + x) as usize;
            let palette_idx = if idx < pixels.len() { pixels[idx] } else { 0 };

            let rgba = if palette_idx == 0 {
                image::Rgba([0, 0, 0, 0])
            } else if let Some(col) = palette.get(palette_idx as usize) {
                image::Rgba([col[0], col[1], col[2], 255])
            } else {
                image::Rgba([palette_idx, palette_idx, palette_idx, 255])
            };

            img.put_pixel(x, y, rgba);
        }
    }

    img
}

#[cfg(target_arch = "wasm32")]
const WEB_UPLOAD_BYTES: usize = 16 << 20;

pub fn write_rgba8_rows(
    queue: &wgpu::Queue,
    texture: &wgpu::Texture,
    pixels: &[u8],
    width: u32,
    height: u32,
) {
    if width == 0 || height == 0 {
        return;
    }
    let row = width as usize * 4;
    #[cfg(target_arch = "wasm32")]
    let strip = (WEB_UPLOAD_BYTES / row.max(1)).max(1) as u32;
    #[cfg(not(target_arch = "wasm32"))]
    let strip = height.max(1);
    let mut y = 0;
    while y < height {
        let rows = strip.min(height - y);
        let start = y as usize * row;
        queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture,
                mip_level: 0,
                origin: wgpu::Origin3d { x: 0, y, z: 0 },
                aspect: wgpu::TextureAspect::All,
            },
            &pixels[start..start + rows as usize * row],
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(width * 4),
                rows_per_image: Some(rows),
            },
            wgpu::Extent3d {
                width,
                height: rows,
                depth_or_array_layers: 1,
            },
        );
        y += rows;
        #[cfg(target_arch = "wasm32")]
        queue.submit(std::iter::empty());
    }
}

pub fn create_texture_from_rgba(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    layout: &wgpu::BindGroupLayout,
    img: &image::RgbaImage,
    label: Option<&str>,
) -> (wgpu::Texture, wgpu::TextureView, wgpu::BindGroup) {
    let width = img.width().max(1);
    let height = img.height().max(1);

    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: label.or(Some("Sprite Texture")),
        size: wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8Unorm,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });

    if img.width() > 0 && img.height() > 0 {
        write_rgba8_rows(queue, &texture, img.as_raw(), img.width(), img.height());
    }

    let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
    let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
        label: Some("Sprite Sampler"),
        address_mode_u: wgpu::AddressMode::ClampToEdge,
        address_mode_v: wgpu::AddressMode::ClampToEdge,
        mag_filter: wgpu::FilterMode::Linear,
        min_filter: wgpu::FilterMode::Linear,
        ..Default::default()
    });

    let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("Sprite Texture Bind Group"),
        layout,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::TextureView(&view),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: wgpu::BindingResource::Sampler(&sampler),
            },
        ],
    });

    (texture, view, bind_group)
}

pub fn create_atlas_from_collection(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    layout: &wgpu::BindGroupLayout,
    collection: &CollectionTag,
) -> TextureAtlas {
    let mut images = Vec::with_capacity(collection.frames.len());
    let mut offsets = Vec::with_capacity(collection.frames.len());

    for frame in &collection.frames {
        let img = frame_to_rgba_image(frame, collection.palette());
        images.push(img);
        offsets.push((frame.offset_x, frame.offset_y));
    }

    create_atlas_from_images_with_offsets(device, queue, layout, &images, &offsets)
}

pub fn create_atlas_from_images(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    layout: &wgpu::BindGroupLayout,
    images: &[image::RgbaImage],
) -> TextureAtlas {
    let dummy_offsets = vec![(0i16, 0i16); images.len()];
    create_atlas_from_images_with_offsets(device, queue, layout, images, &dummy_offsets)
}

pub fn build_atlas_image(
    images: &[image::RgbaImage],
    offsets: &[(i16, i16)],
    max_dim: u32,
) -> (image::RgbaImage, Vec<AtlasFrame>) {
    let layout = atlas_layout(&atlas_cell_sizes(images), max_dim);
    if layout.width > max_dim || layout.height > max_dim {
        tracing::error!(
            "Sprite atlas {}x{} px exceeds the device's {} px texture limit",
            layout.width,
            layout.height,
            max_dim
        );
    }
    let (atlas_w, atlas_h) = (layout.width, layout.height);
    let mut atlas_img = image::RgbaImage::new(atlas_w, atlas_h);
    for (img, &(cx, cy, _, _)) in images.iter().zip(layout.placements.iter()) {
        let cell = atlas_cell_image(img);
        image::imageops::replace(&mut atlas_img, &cell, cx.into(), cy.into());
    }
    let frames = atlas_frames(images, offsets, &layout);
    (atlas_img, frames)
}

fn atlas_cell_sizes(images: &[image::RgbaImage]) -> Vec<(u32, u32)> {
    images
        .iter()
        .map(|i| frame_cell_size(i.width().max(1), i.height().max(1)))
        .collect()
}

fn atlas_cell_image(img: &image::RgbaImage) -> image::RgbaImage {
    let (w, h) = (img.width().max(1), img.height().max(1));
    let (cw, ch) = frame_cell_size(w, h);
    let mut cell = image::RgbaImage::new(cw, ch);
    for (level, &(ox, oy)) in sprite_mip_chain(img)
        .iter()
        .zip(frame_level_offsets(w, h).iter())
    {
        image::imageops::replace(&mut cell, level, (ox + 1).into(), (oy + 1).into());
    }
    cell
}

fn atlas_frames(
    images: &[image::RgbaImage],
    offsets: &[(i16, i16)],
    layout: &AtlasLayout,
) -> Vec<AtlasFrame> {
    let (atlas_w, atlas_h) = (layout.width, layout.height);
    images
        .iter()
        .zip(layout.placements.iter())
        .enumerate()
        .map(|(idx, (img, &(cx, cy, _, _)))| {
            let (x, y) = (cx + 1, cy + 1);
            let (w, h) = (img.width().max(1), img.height().max(1));
            let (off_x, off_y) = offsets.get(idx).copied().unwrap_or((0, 0));
            AtlasFrame {
                index: idx,
                uv_min: [x as f32 / atlas_w as f32, y as f32 / atlas_h as f32],
                uv_max: [
                    (x + w) as f32 / atlas_w as f32,
                    (y + h) as f32 / atlas_h as f32,
                ],
                width: w,
                height: h,
                offset_x: off_x,
                offset_y: off_y,
            }
        })
        .collect()
}

fn atlas_max_dim(device: &wgpu::Device) -> u32 {
    device
        .limits()
        .max_texture_dimension_2d
        .min(wgpu::Limits::default().max_texture_dimension_2d)
}

pub fn grow_atlas_from_images_with_offsets(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    layout: &wgpu::BindGroupLayout,
    old: &TextureAtlas,
    images: &[image::RgbaImage],
    offsets: &[(i16, i16)],
) -> TextureAtlas {
    let rebuild = || create_atlas_from_images_with_offsets(device, queue, layout, images, offsets);
    let kept = old.frames.len();
    let prefix = kept > 0
        && kept <= images.len()
        && old
            .frames
            .iter()
            .zip(images)
            .all(|(f, i)| (f.width, f.height) == (i.width().max(1), i.height().max(1)))
        && old.texture.usage().contains(wgpu::TextureUsages::COPY_SRC);
    if !prefix {
        return rebuild();
    }
    let sizes = atlas_cell_sizes(images);
    let Some(after) = grown_layout((old.width, old.height), &sizes, kept, atlas_max_dim(device))
    else {
        return rebuild();
    };
    let texture = create_atlas_texture(device, after.width, after.height);
    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("Atlas Growth"),
    });
    encoder.copy_texture_to_texture(
        old.texture.as_image_copy(),
        texture.as_image_copy(),
        wgpu::Extent3d {
            width: old.width,
            height: old.height,
            depth_or_array_layers: 1,
        },
    );
    queue.submit(std::iter::once(encoder.finish()));
    for (img, &(cx, cy, _, _)) in images[kept..].iter().zip(&after.placements[kept..]) {
        let cell = atlas_cell_image(img);
        queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &texture,
                mip_level: 0,
                origin: wgpu::Origin3d { x: cx, y: cy, z: 0 },
                aspect: wgpu::TextureAspect::All,
            },
            cell.as_raw(),
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(cell.width() * 4),
                rows_per_image: Some(cell.height()),
            },
            wgpu::Extent3d {
                width: cell.width(),
                height: cell.height(),
                depth_or_array_layers: 1,
            },
        );
    }
    let frames = atlas_frames(images, offsets, &after);
    atlas_from_texture(device, layout, texture, after.width, after.height, frames)
}

fn grown_layout(
    old_size: (u32, u32),
    sizes: &[(u32, u32)],
    kept: usize,
    max_dim: u32,
) -> Option<AtlasLayout> {
    let before = atlas_layout(&sizes[..kept], max_dim);
    let after = atlas_layout(sizes, max_dim);
    let same = (before.width, before.height) == old_size
        && after.width == before.width
        && after.height >= before.height
        && after.width <= max_dim
        && after.height <= max_dim
        && after.placements[..kept] == before.placements[..];
    same.then_some(after)
}

fn create_atlas_texture(device: &wgpu::Device, width: u32, height: u32) -> wgpu::Texture {
    device.create_texture(&wgpu::TextureDescriptor {
        label: Some("Atlas Sprite Texture"),
        size: wgpu::Extent3d {
            width: width.max(1),
            height: height.max(1),
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8Unorm,
        usage: wgpu::TextureUsages::TEXTURE_BINDING
            | wgpu::TextureUsages::COPY_DST
            | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    })
}

fn atlas_from_texture(
    device: &wgpu::Device,
    layout: &wgpu::BindGroupLayout,
    texture: wgpu::Texture,
    width: u32,
    height: u32,
    frames: Vec<AtlasFrame>,
) -> TextureAtlas {
    let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
    let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
        label: Some("Atlas Sampler"),
        address_mode_u: wgpu::AddressMode::ClampToEdge,
        address_mode_v: wgpu::AddressMode::ClampToEdge,
        mag_filter: wgpu::FilterMode::Linear,
        min_filter: wgpu::FilterMode::Linear,
        ..Default::default()
    });
    let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("Atlas Bind Group"),
        layout,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::TextureView(&view),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: wgpu::BindingResource::Sampler(&sampler),
            },
        ],
    });
    TextureAtlas {
        texture,
        view,
        sampler,
        bind_group,
        width,
        height,
        frames,
    }
}

pub fn create_atlas_from_images_with_offsets(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    layout: &wgpu::BindGroupLayout,
    images: &[image::RgbaImage],
    offsets: &[(i16, i16)],
) -> TextureAtlas {
    if images.is_empty() {
        let img = image::RgbaImage::new(1, 1);
        let (texture, view, bind_group) =
            create_texture_from_rgba(device, queue, layout, &img, Some("Empty Atlas"));
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor::default());
        return TextureAtlas {
            texture,
            view,
            sampler,
            bind_group,
            width: 1,
            height: 1,
            frames: Vec::new(),
        };
    }

    let (atlas_img, frames) = build_atlas_image(images, offsets, atlas_max_dim(device));
    let (atlas_w, atlas_h) = atlas_img.dimensions();
    let texture = create_atlas_texture(device, atlas_w, atlas_h);
    write_rgba8_rows(queue, &texture, atlas_img.as_raw(), atlas_w, atlas_h);
    atlas_from_texture(device, layout, texture, atlas_w, atlas_h, frames)
}

pub fn downsample_mip_level(src: &image::RgbaImage, w: u32, h: u32) -> image::RgbaImage {
    let (sw, sh) = (src.width().max(1), src.height().max(1));
    let (dw, dh) = (w.max(1), h.max(1));
    let mut dst = image::RgbaImage::new(dw, dh);
    let axis = |i: u32, dn: u32, sn: u32| -> (usize, usize, f32) {
        let s = ((i as f32 + 0.5) * sn as f32 / dn as f32 - 0.5).clamp(0.0, (sn - 1) as f32);
        let i0 = s.floor() as u32;
        (i0 as usize, (i0 + 1).min(sn - 1) as usize, s - i0 as f32)
    };
    let columns: Vec<_> = (0..dw).map(|x| axis(x, dw, sw)).collect();
    let (texels, row) = (src.as_raw(), sw as usize * 4);
    let out = dst.as_mut();
    for y in 0..dh {
        let (y0, y1, fy) = axis(y, dh, sh);
        let (r0, r1) = (&texels[y0 * row..][..row], &texels[y1 * row..][..row]);
        let out_row = &mut out[y as usize * dw as usize * 4..][..dw as usize * 4];
        for (px, &(x0, x1, fx)) in out_row.as_chunks_mut::<4>().0.iter_mut().zip(&columns) {
            for ch in 0..4 {
                let (a, b) = (r0[x0 * 4 + ch] as f32, r0[x1 * 4 + ch] as f32);
                let (c, d) = (r1[x0 * 4 + ch] as f32, r1[x1 * 4 + ch] as f32);
                let top = a + (b - a) * fx;
                let bottom = c + (d - c) * fx;
                px[ch] = (top + (bottom - top) * fy).round().clamp(0.0, 255.0) as u8;
            }
        }
    }
    dst
}

pub fn sprite_mip_chain(img: &image::RgbaImage) -> Vec<image::RgbaImage> {
    let sizes = sprite_mip_sizes(img.width(), img.height());
    let mut chain = vec![img.clone()];
    for &(w, h) in &sizes[1..] {
        let next = downsample_mip_level(chain.last().unwrap(), w, h);
        chain.push(next);
    }
    chain
}

struct BillboardPipelines {
    pipeline: wgpu::RenderPipeline,
    reflection_pipeline: wgpu::RenderPipeline,
    soft_reflection_pipeline: wgpu::RenderPipeline,
    interface_pipeline: wgpu::RenderPipeline,
    interface_format: wgpu::TextureFormat,
    uniform_bind_group_layout: wgpu::BindGroupLayout,
    texture_bind_group_layout: wgpu::BindGroupLayout,
    default_texture_bind_group: wgpu::BindGroup,
}

pub struct BillboardRenderer {
    pub pipeline: wgpu::RenderPipeline,
    pub reflection_pipeline: wgpu::RenderPipeline,
    pub soft_reflection_pipeline: wgpu::RenderPipeline,
    pub interface_pipeline: wgpu::RenderPipeline,
    pub interface_format: wgpu::TextureFormat,
    pub uniform_bind_group_layout: wgpu::BindGroupLayout,
    pub texture_bind_group_layout: wgpu::BindGroupLayout,
    pub default_texture_bind_group: wgpu::BindGroup,
    instance_buffer: Option<(wgpu::Buffer, usize)>,
}

impl BillboardRenderer {
    pub const BILLBOARD_SHADER_SRC: &'static str = concat!(
        include_str!("../shaders/scene_uniforms.wgsl"),
        include_str!("../shaders/filter_helpers.wgsl"),
        include_str!("../shaders/billboard.wgsl"),
        include_str!("../shaders/reflection_edge_fade.wgsl"),
        include_str!("../shaders/fog.wgsl")
    );

    pub fn interface_view_format(target_format: wgpu::TextureFormat) -> wgpu::TextureFormat {
        target_format.remove_srgb_suffix()
    }

    pub fn new(device: &wgpu::Device, queue: &wgpu::Queue, config: &RenderPipelineConfig) -> Self {
        Self::with_interface_format(
            device,
            queue,
            config,
            Self::interface_view_format(config.target_format),
        )
    }

    pub fn with_interface_format(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        config: &RenderPipelineConfig,
        interface_format: wgpu::TextureFormat,
    ) -> Self {
        let uniform_bind_group_layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("Billboard Scene Uniform Bind Group Layout"),
                entries: &[wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::VERTEX | wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                }],
            });

        let texture_bind_group_layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("Billboard Texture Bind Group Layout"),
                entries: &[
                    wgpu::BindGroupLayoutEntry {
                        binding: 0,
                        visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                        ty: wgpu::BindingType::Texture {
                            sample_type: wgpu::TextureSampleType::Float { filterable: true },
                            view_dimension: wgpu::TextureViewDimension::D2,
                            multisampled: false,
                        },
                        count: None,
                    },
                    wgpu::BindGroupLayoutEntry {
                        binding: 1,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                        count: None,
                    },
                ],
            });

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("Billboard Pipeline Layout"),
            bind_group_layouts: &[
                Some(&uniform_bind_group_layout),
                Some(&texture_bind_group_layout),
            ],
            immediate_size: 0,
        });

        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Billboard WGSL Shader"),
            source: wgpu::ShaderSource::Wgsl(Self::BILLBOARD_SHADER_SRC.into()),
        });

        let depth_stencil = config.depth_format.map(|format| wgpu::DepthStencilState {
            format,
            depth_write_enabled: Some(false),
            depth_compare: Some(wgpu::CompareFunction::LessEqual),
            stencil: wgpu::StencilState::default(),
            bias: wgpu::DepthBiasState::default(),
        });

        let sprite_entry = if interface_format.is_srgb() {
            "fs_main_srgb"
        } else {
            "fs_main"
        };
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("Billboard Render Pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                buffers: &[Some(BillboardInstance::desc())],
                compilation_options: Default::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some(sprite_entry),
                targets: &[Some(wgpu::ColorTargetState {
                    format: interface_format,
                    blend: Some(wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: Default::default(),
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                strip_index_format: None,
                front_face: wgpu::FrontFace::Ccw,
                cull_mode: None,
                unclipped_depth: false,
                polygon_mode: wgpu::PolygonMode::Fill,
                conservative: false,
            },
            depth_stencil,
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        });

        let category4_pipeline = |label, layout, entry| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(label),
                layout: Some(layout),
                vertex: wgpu::VertexState {
                    module: &shader,
                    entry_point: Some("vs_main"),
                    buffers: &[Some(BillboardInstance::desc())],
                    compilation_options: Default::default(),
                },
                fragment: Some(wgpu::FragmentState {
                    module: &shader,
                    entry_point: Some(entry),
                    targets: &[Some(wgpu::ColorTargetState {
                        format: interface_format,
                        blend: Some(wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING),
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                    compilation_options: Default::default(),
                }),
                primitive: wgpu::PrimitiveState {
                    topology: wgpu::PrimitiveTopology::TriangleList,
                    strip_index_format: None,
                    front_face: wgpu::FrontFace::Ccw,
                    cull_mode: None,
                    unclipped_depth: false,
                    polygon_mode: wgpu::PolygonMode::Fill,
                    conservative: false,
                },
                depth_stencil: config.depth_format.map(|format| wgpu::DepthStencilState {
                    format,
                    depth_write_enabled: Some(false),
                    depth_compare: Some(wgpu::CompareFunction::Always),
                    stencil: wgpu::StencilState::default(),
                    bias: wgpu::DepthBiasState::default(),
                }),
                multisample: wgpu::MultisampleState::default(),
                multiview_mask: None,
                cache: None,
            })
        };
        let reflection_pipeline = category4_pipeline(
            "Billboard Reflection Pipeline",
            &pipeline_layout,
            sprite_entry,
        );
        let soft_reflection_pipeline = category4_pipeline(
            "Billboard Soft Reflection Pipeline",
            &device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("Billboard Soft Reflection Pipeline Layout"),
                bind_group_layouts: &[
                    Some(&uniform_bind_group_layout),
                    Some(&texture_bind_group_layout),
                    Some(&crate::reflection_edge::ReflectionEdge::bind_group_layout(
                        device,
                    )),
                ],
                immediate_size: 0,
            }),
            if interface_format.is_srgb() {
                "fs_reflection_soft_srgb"
            } else {
                "fs_reflection_soft"
            },
        );

        let interface_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("Billboard Interface Pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_interface"),
                buffers: &[Some(BillboardInstance::desc())],
                compilation_options: Default::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some(if interface_format.is_srgb() {
                    "fs_interface_srgb"
                } else {
                    "fs_interface"
                }),
                targets: &[Some(wgpu::ColorTargetState {
                    format: interface_format,
                    blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: Default::default(),
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                strip_index_format: None,
                front_face: wgpu::FrontFace::Ccw,
                cull_mode: None,
                unclipped_depth: false,
                polygon_mode: wgpu::PolygonMode::Fill,
                conservative: false,
            },
            depth_stencil: config.depth_format.map(|format| wgpu::DepthStencilState {
                format,
                depth_write_enabled: Some(false),
                depth_compare: Some(wgpu::CompareFunction::LessEqual),
                stencil: wgpu::StencilState::default(),
                bias: wgpu::DepthBiasState::default(),
            }),
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        });

        let default_img = image::RgbaImage::from_pixel(1, 1, image::Rgba([255, 255, 255, 255]));
        let (_tex, _view, default_texture_bind_group) = create_texture_from_rgba(
            device,
            queue,
            &texture_bind_group_layout,
            &default_img,
            Some("Default Billboard Texture"),
        );

        Self::with_pipelines(BillboardPipelines {
            pipeline,
            reflection_pipeline,
            soft_reflection_pipeline,
            interface_pipeline,
            interface_format,
            uniform_bind_group_layout,
            texture_bind_group_layout,
            default_texture_bind_group,
        })
    }

    pub fn sharing_pipelines(&self, _device: &wgpu::Device, _queue: &wgpu::Queue) -> Self {
        Self::with_pipelines(BillboardPipelines {
            pipeline: self.pipeline.clone(),
            reflection_pipeline: self.reflection_pipeline.clone(),
            soft_reflection_pipeline: self.soft_reflection_pipeline.clone(),
            interface_pipeline: self.interface_pipeline.clone(),
            interface_format: self.interface_format,
            uniform_bind_group_layout: self.uniform_bind_group_layout.clone(),
            texture_bind_group_layout: self.texture_bind_group_layout.clone(),
            default_texture_bind_group: self.default_texture_bind_group.clone(),
        })
    }

    fn with_pipelines(pipelines: BillboardPipelines) -> Self {
        let BillboardPipelines {
            pipeline,
            reflection_pipeline,
            soft_reflection_pipeline,
            interface_pipeline,
            interface_format,
            uniform_bind_group_layout,
            texture_bind_group_layout,
            default_texture_bind_group,
        } = pipelines;

        Self {
            pipeline,
            reflection_pipeline,
            soft_reflection_pipeline,
            interface_pipeline,
            interface_format,
            uniform_bind_group_layout,
            texture_bind_group_layout,
            default_texture_bind_group,
            instance_buffer: None,
        }
    }

    pub fn create_instance_buffer(
        device: &wgpu::Device,
        instances: &[BillboardInstance],
    ) -> wgpu::Buffer {
        device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("Billboard Instance Buffer"),
            contents: bytemuck::cast_slice(instances),
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
        })
    }

    pub fn upload_instances(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        instances: &[BillboardInstance],
    ) -> wgpu::Buffer {
        let needed = instances.len().max(1);
        if self
            .instance_buffer
            .as_ref()
            .is_none_or(|(_, cap)| *cap < needed)
        {
            let cap = instances.len().max(256).next_power_of_two();
            let buf = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("Billboard Instance Buffer"),
                size: (cap * core::mem::size_of::<BillboardInstance>()) as u64,
                usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
            self.instance_buffer = Some((buf, cap));
        }
        let (buf, _) = self.instance_buffer.as_ref().expect("allocated above");
        if !instances.is_empty() {
            queue.write_buffer(buf, 0, bytemuck::cast_slice(instances));
        }
        buf.clone()
    }

    pub fn render<'a>(
        &'a self,
        render_pass: &mut wgpu::RenderPass<'a>,
        uniform_bind_group: &'a wgpu::BindGroup,
        texture_bind_group: Option<&'a wgpu::BindGroup>,
        instance_buffer: &'a wgpu::Buffer,
        instance_count: u32,
    ) {
        if instance_count == 0 {
            return;
        }

        render_pass.set_pipeline(&self.pipeline);
        render_pass.set_bind_group(0, uniform_bind_group, &[]);
        let tex_bg = texture_bind_group.unwrap_or(&self.default_texture_bind_group);
        render_pass.set_bind_group(1, tex_bg, &[]);
        render_pass.set_vertex_buffer(0, instance_buffer.slice(..));
        render_pass.draw(0..6, 0..instance_count);
    }

    pub fn render_range<'a>(
        &'a self,
        render_pass: &mut wgpu::RenderPass<'a>,
        uniform_bind_group: Option<&'a wgpu::BindGroup>,
        texture_bind_group: Option<&'a wgpu::BindGroup>,
        instance_buffer: &'a wgpu::Buffer,
        instances: core::ops::Range<u32>,
    ) {
        if instances.is_empty() {
            return;
        }
        render_pass.set_pipeline(&self.pipeline);
        if let Some(scene) = uniform_bind_group {
            render_pass.set_bind_group(0, scene, &[]);
        }
        let tex_bg = texture_bind_group.unwrap_or(&self.default_texture_bind_group);
        render_pass.set_bind_group(1, tex_bg, &[]);
        render_pass.set_vertex_buffer(0, instance_buffer.slice(..));
        render_pass.draw(0..6, instances);
    }

    pub fn render_reflections(
        &self,
        render_pass: &mut wgpu::RenderPass<'_>,
        uniform_bind_group: &wgpu::BindGroup,
        texture_bind_group: Option<&wgpu::BindGroup>,
        instance_buffer: &wgpu::Buffer,
        instances: core::ops::Range<u32>,
        soft_edge: Option<&wgpu::BindGroup>,
    ) {
        if instances.is_empty() {
            return;
        }
        match soft_edge {
            Some(edge) => {
                render_pass.set_pipeline(&self.soft_reflection_pipeline);
                render_pass.set_bind_group(2, edge, &[]);
            }
            None => render_pass.set_pipeline(&self.reflection_pipeline),
        }
        render_pass.set_bind_group(0, uniform_bind_group, &[]);
        let tex_bg = texture_bind_group.unwrap_or(&self.default_texture_bind_group);
        render_pass.set_bind_group(1, tex_bg, &[]);
        render_pass.set_vertex_buffer(0, instance_buffer.slice(..));
        render_pass.draw(0..6, instances);
    }

    pub fn render_interface_range<'a>(
        &'a self,
        render_pass: &mut wgpu::RenderPass<'a>,
        uniform_bind_group: Option<&'a wgpu::BindGroup>,
        instance_buffer: &'a wgpu::Buffer,
        instances: core::ops::Range<u32>,
    ) {
        if instances.is_empty() {
            return;
        }
        render_pass.set_pipeline(&self.interface_pipeline);
        if let Some(scene) = uniform_bind_group {
            render_pass.set_bind_group(0, scene, &[]);
        }
        render_pass.set_bind_group(1, &self.default_texture_bind_group, &[]);
        render_pass.set_vertex_buffer(0, instance_buffer.slice(..));
        render_pass.draw(0..6, instances);
    }

    pub fn render_interface<'a>(
        &'a self,
        render_pass: &mut wgpu::RenderPass<'a>,
        uniform_bind_group: &'a wgpu::BindGroup,
        instance_buffer: &'a wgpu::Buffer,
        instance_count: u32,
    ) {
        if instance_count == 0 {
            return;
        }

        render_pass.set_pipeline(&self.interface_pipeline);
        render_pass.set_bind_group(0, uniform_bind_group, &[]);
        render_pass.set_bind_group(1, &self.default_texture_bind_group, &[]);
        render_pass.set_vertex_buffer(0, instance_buffer.slice(..));
        render_pass.draw(0..6, 0..instance_count);
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn a_mip_level_by_rows_is_the_per_texel_sample() {
        fn per_texel(src: &image::RgbaImage, w: u32, h: u32) -> image::RgbaImage {
            let (sw, sh) = (src.width().max(1), src.height().max(1));
            let mut dst = image::RgbaImage::new(w.max(1), h.max(1));
            let axis = |i: u32, dn: u32, sn: u32| -> (u32, u32, f32) {
                let s =
                    ((i as f32 + 0.5) * sn as f32 / dn as f32 - 0.5).clamp(0.0, (sn - 1) as f32);
                let i0 = s.floor() as u32;
                (i0, (i0 + 1).min(sn - 1), s - i0 as f32)
            };
            for y in 0..dst.height() {
                let (y0, y1, fy) = axis(y, dst.height(), sh);
                for x in 0..dst.width() {
                    let (x0, x1, fx) = axis(x, dst.width(), sw);
                    let (a, b) = (src.get_pixel(x0, y0).0, src.get_pixel(x1, y0).0);
                    let (c, d) = (src.get_pixel(x0, y1).0, src.get_pixel(x1, y1).0);
                    let mut out = [0u8; 4];
                    for ch in 0..4 {
                        let top = a[ch] as f32 + (b[ch] as f32 - a[ch] as f32) * fx;
                        let bottom = c[ch] as f32 + (d[ch] as f32 - c[ch] as f32) * fx;
                        out[ch] = (top + (bottom - top) * fy).round().clamp(0.0, 255.0) as u8;
                    }
                    dst.put_pixel(x, y, image::Rgba(out));
                }
            }
            dst
        }
        for (w, h) in [
            (1, 1),
            (2, 1),
            (5, 3),
            (37, 64),
            (64, 37),
            (90, 71),
            (128, 128),
        ] {
            let img = image::RgbaImage::from_fn(w, h, |x, y| {
                let v = (x * 97 + y * 61 + w * 13) as u8;
                image::Rgba([
                    v,
                    v ^ 0x5a,
                    (x * y) as u8,
                    if (x + y) % 3 == 0 { 0 } else { 200 },
                ])
            });
            for &(lw, lh) in &super::sprite_mip_sizes(w, h)[1..] {
                assert_eq!(
                    super::downsample_mip_level(&img, lw, lh),
                    per_texel(&img, lw, lh),
                    "{w}x{h} to {lw}x{lh}"
                );
            }
        }
    }

    #[test]
    fn an_atlas_grows_in_place_until_it_widens() {
        let sizes: Vec<(u32, u32)> = (0..300u32)
            .map(|i| (20 + i % 37 * 7, 30 + i % 11 * 9))
            .collect();
        let old = super::atlas_layout(&sizes[..120], 8192);
        let grown = super::grown_layout((old.width, old.height), &sizes[..200], 120, 8192).unwrap();
        assert_eq!(grown.placements[..120], old.placements[..]);
        assert_eq!(grown, super::atlas_layout(&sizes[..200], 8192));
        assert!(
            super::grown_layout((old.width, old.height + 1), &sizes[..200], 120, 8192).is_none()
        );
        let tall = vec![(1000u32, 300u32); 40];
        let narrow = super::atlas_layout(&tall[..3], 1024);
        assert!(super::grown_layout((narrow.width, narrow.height), &tall[..5], 3, 1024).is_none());
    }

    #[test]
    fn atlas_layout_widens_past_the_texture_limit() {
        let sizes = vec![(256u32, 300u32); 140];
        assert_eq!(super::pack_shelves(&sizes, 1024).height, 35 * 300);
        let l = super::atlas_layout(&sizes, 8192);
        assert_eq!((l.width, l.height), (2048, 18 * 300));
        assert!(
            l.placements
                .iter()
                .all(|&(x, y, w, h)| x + w <= 2048 && y + h <= l.height)
        );
        let small = super::atlas_layout(&sizes[..8], 8192);
        assert_eq!((small.width, small.height), (1024, 600));
        let wide = super::atlas_layout(&[(1500, 10)], 8192);
        assert_eq!(wide.width, 1500);
    }

    #[test]
    fn atlas_layout_packs_densely_past_the_texture_limit() {
        let sizes: Vec<(u32, u32)> = (0..64)
            .map(|i| if i % 2 == 0 { (2000, 1000) } else { (2000, 10) })
            .collect();
        assert_eq!(super::pack_shelves(&sizes, 8192).height, 16 * 1000);
        let l = super::atlas_layout(&sizes, 8192);
        assert_eq!((l.width, l.height), (8000, 8 * 1000 + 8 * 10));
        assert_eq!(l.placements.len(), sizes.len());
        for (i, &(x, y, w, h)) in l.placements.iter().enumerate() {
            assert_eq!((w, h), sizes[i]);
            assert!(x + w <= l.width && y + h <= l.height);
            for &(x2, y2, w2, h2) in &l.placements[..i] {
                assert!(x + w <= x2 || x2 + w2 <= x || y + h <= y2 || y2 + h2 <= y);
            }
        }
        let filled = super::pack_shelves_ffdh(&[(600, 500), (500, 400), (400, 20)], 1024);
        assert_eq!(
            filled.placements,
            vec![(0, 0, 600, 500), (0, 500, 500, 400), (600, 0, 400, 20)]
        );
        assert_eq!(filled.height, 900);
    }

    use super::*;
    use mandrake_view::selection::PixelRect;

    #[test]
    fn sprite_quad_growth_truncates_and_clips() {
        assert_eq!(sprite_quad_growth(266, 10), 13);
        assert_eq!(sprite_quad_growth(-170, 17), -5);
        assert_eq!(sprite_quad_growth(291, 27), 5);
        assert!((sprite_growth_cells(266.0 / 512.0, 10) * 512.0 - 13.0).abs() < 1e-3);
        let q = engine_sprite_quad([0, 170, 200, 0], [120, -32768], (20, 17), 0);
        assert_eq!((q[0].x, q[0].y, q[1].y, q[2].x), (-5, 120, -5, 205));
        let (v0, dv) = sprite_border_uv(17);
        assert!((q[0].v - (v0 + dv * 50.0 / 170.0)).abs() < 1e-6);
        assert!((q[1].v - (v0 + dv)).abs() < 1e-6);
        let q = engine_sprite_quad([0, 170, 200, 0], [172, -32768], (20, 17), 0);
        assert_eq!(q[0].y, 172);
        assert!((q[0].v - v0).abs() < 1e-6);
    }

    #[test]
    fn atlas_frames_have_transparent_rings_and_mip_columns() {
        assert_eq!(
            sprite_mip_sizes(20, 17),
            vec![(20, 17), (10, 8), (5, 4), (2, 2), (1, 1)]
        );
        assert_eq!(sprite_mip_sizes(96, 3).len(), 7);
        assert_eq!(sprite_mip_sizes(1, 1), vec![(1, 1)]);
        assert_eq!(
            frame_level_offsets(20, 17),
            vec![(0, 0), (21, 0), (21, 9), (21, 14), (21, 17)]
        );
        assert_eq!(frame_cell_size(20, 17), (20 + 10 + 3, 17 + 1 + 2));
        let opaque = |w, h, c: u8| image::RgbaImage::from_pixel(w, h, image::Rgba([c, c, c, 255]));
        let images = vec![
            opaque(20, 17, 200),
            opaque(7, 30, 100),
            opaque(1, 1, 50),
            opaque(64, 2, 10),
        ];
        let (atlas, frames) = build_atlas_image(&images, &[], 8192);
        let mut owner = vec![usize::MAX; (atlas.width() * atlas.height()) as usize];
        for (f, img) in frames.iter().zip(images.iter()) {
            let x0 = (f.uv_min[0] * atlas.width() as f32).round() as u32;
            let y0 = (f.uv_min[1] * atlas.height() as f32).round() as u32;
            assert_eq!((f.width, f.height), img.dimensions());
            for (&(lw, lh), &(ox, oy)) in sprite_mip_sizes(f.width, f.height)
                .iter()
                .zip(frame_level_offsets(f.width, f.height).iter())
            {
                let (lx, ly) = (x0 + ox, y0 + oy);
                for y in ly - 1..ly + lh + 1 {
                    for x in lx - 1..lx + lw + 1 {
                        let inside = x >= lx && x < lx + lw && y >= ly && y < ly + lh;
                        let p = atlas.get_pixel(x, y).0;
                        if inside {
                            assert_eq!(p[3], 255, "frame {} level texel {x},{y}", f.index);
                            let o = &mut owner[(y * atlas.width() + x) as usize];
                            assert_eq!(*o, usize::MAX, "overlap at {x},{y}");
                            *o = f.index;
                        } else {
                            assert_eq!(p, [0, 0, 0, 0], "ring texel {x},{y} of frame {}", f.index);
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn mip_levels_follow_the_linear_blit() {
        let mut img = image::RgbaImage::new(4, 2);
        img.put_pixel(0, 0, image::Rgba([200, 100, 0, 255]));
        let l1 = downsample_mip_level(&img, 2, 1);
        assert_eq!(l1.get_pixel(0, 0).0, [50, 25, 0, 64]);
        assert_eq!(l1.get_pixel(1, 0).0, [0, 0, 0, 0]);
        let mut row = image::RgbaImage::new(3, 1);
        row.put_pixel(1, 0, image::Rgba([90, 90, 90, 255]));
        assert_eq!(
            downsample_mip_level(&row, 1, 1).get_pixel(0, 0).0,
            [90, 90, 90, 255]
        );
        let mut five = image::RgbaImage::new(5, 1);
        five.put_pixel(1, 0, image::Rgba([100, 100, 100, 255]));
        assert_eq!(
            downsample_mip_level(&five, 2, 1).get_pixel(0, 0).0,
            [75, 75, 75, 191]
        );
        assert_eq!(sprite_mip_chain(&five).len(), 3);
    }

    #[test]
    fn test_order_for_sprite_pass_groups_and_sorts_back_to_front() {
        let sprite = |z: f32| {
            BillboardInstance::new([0.0, 0.0, z], [1.0, 1.0], [0.0; 2], [1.0; 2])
                .with_screen_aligned(true)
        };
        let band = BillboardInstance::screen_overlay_rect(
            PixelRect::new(0, 0, 1, 1),
            [1.0, 1.0, 0.0, 1.0],
        );
        let mut list = vec![band, sprite(-2.0), sprite(-9.0), sprite(-5.0)];
        let order = order_for_sprite_pass(&mut list, [0.0; 3], [0.0, 0.0, -1.0]);
        assert_eq!(order.sorted, 0..3);
        assert_eq!(order.overlay, 3..4);
        assert_eq!(order.keys, [9.0, 5.0, 2.0]);
        assert_eq!(order.interface, [false; 3]);
        let zs: Vec<f32> = list[0..3].iter().map(|i| i.position[2]).collect();
        assert_eq!(zs, [-9.0, -5.0, -2.0]);
        assert_eq!(
            list[3].flags,
            BILLBOARD_FLAG_INTERFACE_RECT | BILLBOARD_FLAG_SCREEN_OVERLAY
        );
    }

    #[test]
    fn test_order_for_sprite_pass_interleaves_object_overlays() {
        let sprite = |z: f32| {
            BillboardInstance::new([0.0, 0.0, z], [1.0, 1.0], [0.0; 2], [1.0; 2])
                .with_screen_aligned(true)
        };
        let rect = |z: f32, x: i32| {
            BillboardInstance::interface_rect(
                [0.0, 0.0, z],
                PixelRect::new(x, 0, x + 1, 1),
                [1.0, 1.0, 0.0, 1.0],
            )
        };
        let band = BillboardInstance::screen_overlay_rect(
            PixelRect::new(5, 5, 50, 50),
            [1.0, 1.0, 0.0, 0.5],
        );
        let mut list = vec![
            sprite(-2.0),
            rect(-2.0, 1),
            rect(-2.0, 2),
            sprite(-5.0),
            sprite(-8.0),
            rect(-8.0, 3),
            band,
        ];
        let order = order_for_sprite_pass(&mut list, [0.0; 3], [0.0, 0.0, -1.0]);
        assert_eq!(order.sorted, 0..6);
        assert_eq!(order.overlay, 6..7);
        assert_eq!(order.interface, [false, true, false, false, true, true]);
        let seq: Vec<(f32, bool)> = list[..6]
            .iter()
            .map(|i| (i.position[2], i.is_interface_rect()))
            .collect();
        assert_eq!(
            seq,
            [
                (-8.0, false),
                (-8.0, true),
                (-5.0, false),
                (-2.0, false),
                (-2.0, true),
                (-2.0, true)
            ]
        );
        assert_eq!((list[4].uv_min[0], list[5].uv_min[0]), (1.0, 2.0));

        let steps = sprite_category_steps(&list, &order, &[]);
        assert_eq!(
            steps,
            [
                SpriteStep::Billboards(0..1),
                SpriteStep::Interface(1..2),
                SpriteStep::Billboards(2..4),
                SpriteStep::Interface(4..6),
                SpriteStep::Interface(6..7),
            ]
        );
    }

    #[test]
    fn test_sprite_category_steps_keep_overlays_with_their_sprite() {
        let sprite = |z: f32| {
            BillboardInstance::new([0.0, 0.0, z], [1.0, 1.0], [0.0; 2], [1.0; 2])
                .with_screen_aligned(true)
        };
        let rect = |z: f32| {
            BillboardInstance::interface_rect([0.0, 0.0, z], PixelRect::new(0, 0, 1, 1), [1.0; 4])
        };
        let arrow = BillboardInstance::facing_arrow([0.0; 3], 0.0, [1.0; 4]);
        let mut list = vec![sprite(-6.0), rect(-6.0), sprite(-3.0), rect(-3.0), arrow];
        let order = order_for_sprite_pass(&mut list, [0.0; 3], [0.0, 0.0, -1.0]);
        let steps = sprite_category_steps(&list, &order, &[7.0, 6.0, 4.0, 1.0]);
        assert_eq!(
            steps,
            [
                SpriteStep::Triangles(0..2),
                SpriteStep::Billboards(0..1),
                SpriteStep::Interface(1..2),
                SpriteStep::Triangles(2..3),
                SpriteStep::Billboards(2..3),
                SpriteStep::Interface(3..4),
                SpriteStep::Triangles(3..4),
                SpriteStep::Billboards(4..5),
            ]
        );
    }

    #[test]
    fn test_merge_sprite_steps_only_moves_triangles_over_disjoint_billboards() {
        let vp = crate::camera::Mat4::perspective_rh_zo(1.0, 1.0, 0.1, 100.0);
        let sprite = |x: f32, z: f32| {
            BillboardInstance::new([x, 0.0, z], [0.2, 0.2], [0.0; 2], [1.0; 2])
                .with_screen_aligned(true)
        };
        let list = [sprite(-2.0, -6.0), sprite(2.0, -5.0), sprite(-2.0, -4.0)];
        let steps = || {
            vec![
                SpriteStep::Triangles(0..1),
                SpriteStep::Billboards(0..1),
                SpriteStep::Triangles(1..2),
                SpriteStep::Billboards(1..2),
                SpriteStep::Triangles(2..3),
                SpriteStep::Billboards(2..3),
            ]
        };
        let right = [0.5, -0.1, 0.6, 0.1];
        let left = [-0.6, -0.1, -0.5, 0.1];
        let bounds = |r: core::ops::Range<u32>| Some(if r.start == 2 { left } else { right });
        assert!(list[0].screen_bounds(&vp).unwrap()[2] < 0.0);
        assert!(list[1].screen_bounds(&vp).unwrap()[0] > 0.0);
        assert_eq!(
            merge_sprite_steps(steps(), &list, &vp, bounds),
            [
                SpriteStep::Triangles(0..2),
                SpriteStep::Billboards(0..2),
                SpriteStep::Triangles(2..3),
                SpriteStep::Billboards(2..3),
            ]
        );
        let all_right = |_: core::ops::Range<u32>| Some(right);
        assert_eq!(
            merge_sprite_steps(steps(), &list, &vp, all_right),
            [
                SpriteStep::Triangles(0..2),
                SpriteStep::Billboards(0..2),
                SpriteStep::Triangles(2..3),
                SpriteStep::Billboards(2..3),
            ]
        );
        assert_eq!(merge_sprite_steps(steps(), &list, &vp, |_| None), steps());
        let with_interface = vec![
            SpriteStep::Triangles(0..1),
            SpriteStep::Interface(0..1),
            SpriteStep::Triangles(1..2),
        ];
        assert_eq!(
            merge_sprite_steps(with_interface.clone(), &list, &vp, bounds),
            with_interface
        );
    }

    #[test]
    fn test_screen_bounds_hold_the_sprite_quad() {
        let (cam, mut sprite) = reference_sprite();
        let vp = cam.view_proj_matrix(16.0 / 9.0);
        let (right, up, _) = cam.view_axes();
        for flags in [
            BILLBOARD_FLAG_SCREEN_ALIGNED,
            BILLBOARD_FLAG_SCREEN_ALIGNED | BILLBOARD_FLAG_FLIP_X,
            0,
        ] {
            sprite.flags = flags;
            let (r, u) = if flags & BILLBOARD_FLAG_SCREEN_ALIGNED != 0 {
                (right, up)
            } else {
                sprite.sprite_basis(cam.eye_position())
            };
            let b = sprite.screen_bounds(&vp).expect("in front of the eye");
            let ([x0, x1], [y0, y1]) = sprite.effective_quad_bounds();
            let p = sprite.position;
            let corners = [(x0, y0), (x1, y0), (x0, y1), (x1, y1)]
                .map(|(ox, oy)| core::array::from_fn(|k| p[k] + r[k] * ox + u[k] * oy));
            let q = ndc_rect(&vp, corners).unwrap();
            assert!(
                b[0] <= q[0] && b[1] <= q[1] && b[2] >= q[2] && b[3] >= q[3],
                "{b:?} {q:?}"
            );
        }
        sprite.flags = BILLBOARD_FLAG_INTERFACE_RECT;
        assert_eq!(sprite.screen_bounds(&vp), None);
    }

    #[test]
    fn test_band_is_drawn_before_facing_arrows() {
        let sprite = BillboardInstance::new([0.0, 0.0, -4.0], [1.0, 1.0], [0.0; 2], [1.0; 2])
            .with_screen_aligned(true);
        let band = |x: i32| {
            BillboardInstance::screen_overlay_rect(
                PixelRect::new(x, 0, x + 10, 10),
                [1.0, 1.0, 0.0, 0.5],
            )
        };
        let arrow = BillboardInstance::facing_arrow([0.0; 3], 0.0, [1.0; 4]);
        let mut list = vec![band(0), band(20), arrow, sprite];
        let order = order_for_sprite_pass(&mut list, [0.0; 3], [0.0, 0.0, -1.0]);
        assert_eq!(order.sorted, 0..1);
        assert_eq!(order.overlay, 1..4);
        assert!(list[1].flags & BILLBOARD_FLAG_SCREEN_OVERLAY != 0);
        assert!(list[2].flags & BILLBOARD_FLAG_SCREEN_OVERLAY != 0);
        assert!(list[3].flags & BILLBOARD_FLAG_FACING_ARROW != 0);
        let steps = sprite_category_steps(&list, &order, &[]);
        assert_eq!(
            steps,
            [
                SpriteStep::Billboards(0..1),
                SpriteStep::Interface(1..3),
                SpriteStep::Billboards(3..4),
            ]
        );
    }

    fn reference_sprite() -> (crate::camera::Camera, BillboardInstance) {
        use crate::camera::Camera;
        let eye = [64.0 - 11547.0 / 512.0, 8.0, 45198.0 / 512.0 - 64.0];
        let cam = Camera::from_myth_observer(eye, 5.326173, 5.81187 - std::f32::consts::TAU, 0.0);
        let world = [64.0 - 14828.0 / 512.0, 50.0 / 512.0, 42149.0 / 512.0 - 64.0];
        let mut sprite =
            BillboardInstance::new(world, [310.0 / 512.0, 610.0 / 512.0], [0.0; 2], [1.0; 2])
                .with_screen_aligned(true);
        sprite.anchor = [170.0 / 310.0, 570.0 / 610.0];
        (cam, sprite)
    }

    fn view_corners(cam: &crate::camera::Camera, sprite: &BillboardInstance) -> [[f32; 2]; 2] {
        let (r, u, f) = cam.view_axes();
        let eye = cam.eye_position();
        let v = cam.view_matrix();
        [[0.0, 0.0], [1.0, 1.0]].map(|corner| {
            let (q, _) = sprite.screen_aligned_corner(eye, r, u, f, corner);
            let [x, y, _] = v.transform_point3(q);
            [x * 512.0, y * 512.0]
        })
    }

    fn assert_near(a: [[f32; 2]; 2], b: [[f32; 2]; 2]) {
        for (a, b) in a.iter().flatten().zip(b.iter().flatten()) {
            assert!((a - b).abs() < 0.05, "{a} {b}");
        }
    }

    #[test]
    fn test_in_engine_view_gives_the_reference_vertices() {
        let (cam, sprite) = reference_sprite();
        let placed = sprite.in_engine_view(&EngineSpriteView::of(&cam));
        assert_near(
            view_corners(&cam, &placed),
            [[-1097.0, -1656.0], [-787.0, -1046.0]],
        );
    }

    #[test]
    fn test_in_engine_view_rounds_the_scaled_words_down() {
        let (cam, mut sprite) = reference_sprite();
        sprite.size = [311.0 / 512.0, 610.0 / 512.0];
        sprite.anchor = [170.5 / 311.0, 570.0 / 610.0];
        let placed = sprite.in_engine_view(&EngineSpriteView::of(&cam));
        assert_near(
            view_corners(&cam, &placed),
            [[-927.0 - 171.0, -1656.0], [-927.0 + 140.0, -1046.0]],
        );
        let flipped = sprite
            .with_flip_x(true)
            .in_engine_view(&EngineSpriteView::of(&cam));
        assert_near(
            view_corners(&cam, &flipped),
            [[-927.0 - 141.0, -1656.0], [-927.0 + 170.0, -1046.0]],
        );
    }

    #[test]
    fn test_in_engine_view_leaves_other_instances_alone() {
        let (cam, sprite) = reference_sprite();
        let view = EngineSpriteView::of(&cam);
        let centered = sprite.with_flags(sprite.flags | BILLBOARD_FLAG_CENTERED);
        assert_eq!(centered.in_engine_view(&view).position, centered.position);
        let plain = sprite.with_flags(0);
        assert_eq!(plain.in_engine_view(&view).size, plain.size);
    }

    #[test]
    fn test_in_engine_view_clip_height_is_whole_units() {
        let (cam, sprite) = reference_sprite();
        let surface = 100.0 / 512.0;
        let view = EngineSpriteView::of(&cam);
        let placed = sprite.with_media_clip(surface).in_engine_view(&view);
        let (_, up, _) = cam.view_axes();
        let along = (placed.media[0] - placed.position[1]) * up[1] * 512.0;
        let engine = cam.engine_view();
        let expected = engine.view_coords([sprite.position[0], surface, sprite.position[2]])[1]
            - engine.view_coords(sprite.position)[1];
        assert!((along - expected as f32).abs() < 0.05, "{along} {expected}");
    }

    #[test]
    fn sprite_plane_depth_follows_the_sprite_proc() {
        assert_eq!(sprite_plane_depth_units(-321.0, -321.0, 234.0), 234.0);
        assert_eq!(sprite_plane_depth_units(-321.0, -642.0, 235.0), 117.0);
        assert_eq!(sprite_plane_depth_units(-321.0, 311.0, 234.0), 0.0);
        assert_eq!(sprite_plane_depth_units(1.0, 1000.0, 234.0), 0.0);
        assert_eq!(sprite_plane_depth_units(-321.0, 0.0, 234.0), 0.0);
        assert!(sprite_plane_depth_units(321.0, 0.0, 234.0) > 360.0 * 512.0);
    }

    #[test]
    fn test_screen_aligned_corner_view_plane_and_vertical_depth_plane() {
        let mut cam = mandrake_view::camera::Camera::new([3.0, 0.0, -2.0], 10.0, 0.4, 0.6);
        cam.fov_y = mandrake_view::camera::Camera::myth_fov_y();
        let (r, u, f) = cam.view_axes();
        let eye = cam.eye_position();
        let sprite = BillboardInstance::new([4.5, 0.3, -1.0], [0.6, 1.3], [0.0; 2], [1.0; 2])
            .with_anchor([0.5, 0.9])
            .with_screen_aligned(true);
        let p = sprite.position;
        let dot = |a: [f32; 3], b: [f32; 3]| a[0] * b[0] + a[1] * b[1] + a[2] * b[2];
        let sub = |a: [f32; 3], b: [f32; 3]| [a[0] - b[0], a[1] - b[1], a[2] - b[2]];
        let n = {
            let l = (f[0] * f[0] + f[2] * f[2]).sqrt();
            [f[0] / l, 0.0, f[2] / l]
        };
        for corner in [[0.0, 0.0], [1.0, 0.0], [0.0, 1.0], [1.0, 1.0]] {
            let (q, z) = sprite.screen_aligned_corner(eye, r, u, f, corner);
            assert!((dot(sub(q, eye), f) - dot(sub(p, eye), f)).abs() < 1e-4);
            let z = z.expect("plane hit");
            assert!(dot(sub(z, p), n).abs() < 1e-4);
            let a = sub(q, eye);
            let b = sub(z, eye);
            let c = [
                a[1] * b[2] - a[2] * b[1],
                a[2] * b[0] - a[0] * b[2],
                a[0] * b[1] - a[1] * b[0],
            ];
            assert!(dot(c, c).sqrt() < 1e-3);
        }
        let (_, bottom) = sprite.screen_aligned_corner(eye, r, u, f, [0.5, 0.0]);
        let (_, top) = sprite.screen_aligned_corner(eye, r, u, f, [0.5, 1.0]);
        let d0 = dot(sub(p, eye), f);
        assert!(dot(sub(bottom.unwrap(), eye), f) > d0);
        assert!(dot(sub(top.unwrap(), eye), f) < d0);
    }

    #[test]
    fn test_billboard_instance_layout_stride_and_offsets() {
        assert_eq!(core::mem::size_of::<BillboardInstance>(), 112);
        let desc = BillboardInstance::desc();
        assert_eq!(desc.array_stride, 112);
        assert_eq!(desc.step_mode, wgpu::VertexStepMode::Instance);
        assert_eq!(desc.attributes.len(), 10);
        assert_eq!(desc.attributes[0].offset, 0);
        assert_eq!(desc.attributes[1].offset, 12);
        assert_eq!(desc.attributes[2].offset, 16);
        assert_eq!(desc.attributes[3].offset, 24);
        assert_eq!(desc.attributes[4].offset, 32);
        assert_eq!(desc.attributes[5].offset, 40);
        assert_eq!(desc.attributes[6].offset, 56);
        assert_eq!(desc.attributes[7].offset, 64);
        assert_eq!(desc.attributes[8].offset, 80);
        assert_eq!(desc.attributes[9].offset, 96);
    }

    #[test]
    fn submerged_copy_keeps_the_rows_below_the_surface() {
        let base = BillboardInstance::new([0.0, -0.25, 0.0], [0.5, 1.0], [0.0, 0.0], [1.0, 1.0])
            .with_anchor([0.5, 1.0])
            .with_screen_aligned(true);
        let (s, c) = (30f32.to_radians().sin(), 30f32.to_radians().cos());
        let up = [0.0, c, s];
        let own = base.with_media_clip(0.0);
        let copy = own.submerged_copy(crate::media::SUBMERGED_COPY_TRANSPARENCY);
        assert_eq!(
            copy.flags & (BILLBOARD_FLAG_MEDIA_CLIP | BILLBOARD_FLAG_SUBMERGED),
            BILLBOARD_FLAG_MEDIA_CLIP | BILLBOARD_FLAG_SUBMERGED
        );
        assert_eq!(copy.tint[3], 31.0 / 255.0);
        let t0 = own.media_span(up).3.unwrap()[0];
        assert_eq!(copy.media_span(up).3, Some([0.0, t0]));
        let deep = BillboardInstance {
            position: [0.0, -2.0, 0.0],
            ..base
        }
        .with_media_clip(0.0);
        assert_eq!(deep.submerged_copy(0xe000).media_span(up).3, None);
        let dry = BillboardInstance {
            position: [0.0, 0.5, 0.0],
            ..base
        }
        .with_media_clip(0.0);
        assert_eq!(dry.submerged_copy(0xe000).media_span(up).3, None);
        let refl = base.reflection(0.0, 0.0, true, [63, 0, 0, 0, 0, 0, 0, 0]);
        let mut v = vec![copy, own, refl];
        let order = order_for_sprite_pass(&mut v, [0.0, 5.0, 5.0], [0.0, -0.5, -0.8]);
        assert_eq!(order.sorted, 0..1);
        assert_eq!(order.reflections, 1..3);
        assert_eq!(v[1], copy);
        assert_eq!(v[2], refl);
        assert_eq!(crate::media::surface_opacity(0), 255.0 / 255.0);
    }

    #[test]
    fn media_clip_keeps_the_rows_above_the_surface() {
        let base = BillboardInstance::new([0.0, -0.25, 0.0], [0.5, 1.0], [0.0, 0.0], [1.0, 1.0])
            .with_anchor([0.5, 1.0])
            .with_screen_aligned(true);
        let (s, c) = (30f32.to_radians().sin(), 30f32.to_radians().cos());
        let up = [0.0, c, s];
        assert_eq!(base.media_span(up).3, Some([0.0, 1.0]));
        let clipped = base.with_media_clip(0.0);
        assert_eq!(
            clipped.flags & BILLBOARD_FLAG_MEDIA_CLIP,
            BILLBOARD_FLAG_MEDIA_CLIP
        );
        let (origin, lo, hi, kept) = clipped.media_span(up);
        assert_eq!(origin, base.position);
        assert_eq!((lo, hi), (0.0, 1.0));
        let t0 = kept.unwrap()[0];
        assert!((t0 - 0.25 * c).abs() < 1e-6, "t0 {t0}");
        let deep = BillboardInstance {
            position: [0.0, -2.0, 0.0],
            ..base
        }
        .with_media_clip(0.0);
        assert_eq!(deep.media_span(up).3, None);

        let r = base.reflection(0.0, 0.0, true, [63, 0, 0, 0, 0, 0, 0, 0]);
        assert_eq!(
            r.flags
                & (BILLBOARD_FLAG_REFLECTION | BILLBOARD_FLAG_MEDIA_CLIP | BILLBOARD_FLAG_SHADE),
            BILLBOARD_FLAG_REFLECTION | BILLBOARD_FLAG_MEDIA_CLIP | BILLBOARD_FLAG_SHADE
        );
        assert_eq!(
            r.position, base.position,
            "depth from the real origin (z < m)"
        );
        assert!((r.shade_kpqr[0] - 63.0 / 255.0).abs() < 1e-6);
        let (origin, lo, hi, kept) = r.media_span(up);
        let d = 0.5 * c;
        assert!((origin[1] - (-0.25 + up[1] * d)).abs() < 1e-6);
        assert!((origin[2] - up[2] * d).abs() < 1e-6);
        assert_eq!((lo, hi), (-0.0, -1.0));
        assert!((kept.unwrap()[0] - 0.25 * c).abs() < 1e-5);

        let dry = BillboardInstance {
            position: [0.0, 0.5, 0.0],
            ..base
        };
        let r = dry.reflection(0.0, 0.125, false, [63, 0, 0, 0, 0, 0, 0, 0]);
        assert_eq!(r.position[1], -0.5);
        assert_eq!(r.flags & BILLBOARD_FLAG_MEDIA_CLIP, 0);
        let (origin, lo, hi, kept) = r.media_span(up);
        assert_eq!(origin, [0.0, -0.5, 0.0]);
        assert_eq!((lo, hi), (-0.0, -1.125));
        assert_eq!(kept, Some([0.0, 1.0]));
    }

    #[test]
    fn test_billboard_wgsl_syntax_validation() {
        let src = BillboardRenderer::BILLBOARD_SHADER_SRC;
        let module = naga::front::wgsl::parse_str(src);
        assert!(
            module.is_ok(),
            "Billboard WGSL syntax validation failed: {:?}",
            module.err()
        );
    }

    #[test]
    fn shaders_translate_to_glsl_es_300() {
        use naga::back::glsl;
        for (name, src) in [
            ("billboard", BillboardRenderer::BILLBOARD_SHADER_SRC),
            ("model", crate::model::ModelRenderer::MODEL_SHADER_SRC),
            (
                "terrain",
                crate::pipeline::TerrainRenderer::TERRAIN_SHADER_SRC,
            ),
        ] {
            let module = naga::front::wgsl::parse_str(src).unwrap();
            let info = naga::valid::Validator::new(
                naga::valid::ValidationFlags::all(),
                naga::valid::Capabilities::empty(),
            )
            .validate(&module)
            .unwrap_or_else(|e| panic!("{name}: {e:?}"));
            let (module, info) = naga::back::pipeline_constants::process_overrides(
                &module,
                &info,
                None,
                &naga::back::PipelineConstants::default(),
            )
            .unwrap_or_else(|e| panic!("{name}: {e:?}"));
            let options = glsl::Options {
                version: glsl::Version::Embedded {
                    version: 300,
                    is_webgl: true,
                },
                ..Default::default()
            };
            for ep in &module.entry_points {
                let pipeline = glsl::PipelineOptions {
                    shader_stage: ep.stage,
                    entry_point: ep.name.clone(),
                    multiview: None,
                };
                let mut out = String::new();
                glsl::Writer::new(
                    &mut out,
                    &module,
                    &info,
                    &options,
                    &pipeline,
                    naga::proc::BoundsCheckPolicies::default(),
                )
                .and_then(|mut w| w.write().map(|_| ()))
                .unwrap_or_else(|e| panic!("{name} {}: {e:?}", ep.name));
                assert!(out.starts_with("#version 300 es"), "{name} {}", ep.name);
            }
        }
    }

    #[test]
    fn test_sprite_view_selector_8_views() {
        let two_pi = core::f32::consts::PI * 2.0;

        let (view, flip) = select_sprite_view(0.0, 0.0, 8);
        assert_eq!(view, 0);
        assert!(!flip);

        let (view, flip) = select_sprite_view(core::f32::consts::FRAC_PI_2, 0.0, 8);
        assert_eq!(view, 2);
        assert!(!flip);

        let (view, flip) = select_sprite_view(core::f32::consts::PI, 0.0, 8);
        assert_eq!(view, 4);
        assert!(!flip);

        let (view, flip) = select_sprite_view(3.0 * core::f32::consts::FRAC_PI_2, 0.0, 8);
        assert_eq!(view, 6);
        assert!(!flip);

        let (view, flip) = select_sprite_view(0.0, two_pi, 8);
        assert_eq!(view, 0);
        assert!(!flip);
    }

    #[test]
    fn test_sprite_view_selector_symmetrical_5_views() {
        let (view, flip) = select_sprite_view(0.0, 0.0, 5);
        assert_eq!(view, 0);
        assert!(!flip);

        let (view, flip) = select_sprite_view(core::f32::consts::FRAC_PI_4, 0.0, 5);
        assert_eq!(view, 1);
        assert!(!flip);

        let (view, flip) = select_sprite_view(core::f32::consts::FRAC_PI_2, 0.0, 5);
        assert_eq!(view, 2);
        assert!(!flip);

        let (view, flip) = select_sprite_view(core::f32::consts::PI, 0.0, 5);
        assert_eq!(view, 4);
        assert!(!flip);

        let (view, flip) = select_sprite_view(5.0 * core::f32::consts::FRAC_PI_4, 0.0, 5);
        assert_eq!(view, 3);
        assert!(flip);

        let (view, flip) = select_sprite_view(3.0 * core::f32::consts::FRAC_PI_2, 0.0, 5);
        assert_eq!(view, 2);
        assert!(flip);

        let (view, flip) = select_sprite_view(7.0 * core::f32::consts::FRAC_PI_4, 0.0, 5);
        assert_eq!(view, 1);
        assert!(flip);
    }

    #[test]
    fn test_sprite_view_selector_symmetrical_helper() {
        let (view, flip) =
            select_sprite_view_symmetrical(3.0 * core::f32::consts::FRAC_PI_2, 0.0, 8);
        assert_eq!(view, 2);
        assert!(flip);

        let (view, flip) = select_sprite_view(0.0, 0.0, 9);
        assert_eq!(view, 0);
        assert!(!flip);

        let (view, flip) = select_sprite_view(3.0 * core::f32::consts::FRAC_PI_2, 0.0, 9);
        assert_eq!(view, 4);
        assert!(flip);
    }

    #[test]
    fn test_interface_rect_encoding() {
        let rect = PixelRect::new(1747, 950, 1843, 951);
        let inst = BillboardInstance::interface_rect([1.0, 2.0, 3.0], rect, [1.0, 1.0, 0.0, 0.5]);
        assert_eq!(inst.flags, BILLBOARD_FLAG_INTERFACE_RECT);
        assert_eq!(inst.position, [1.0, 2.0, 3.0]);
        assert_eq!(inst.uv_min, [1747.0, 950.0]);
        assert_eq!(inst.uv_max, [1843.0, 951.0]);
        assert_eq!(inst.tint, [1.0, 1.0, 0.0, 0.5]);
    }

    #[test]
    fn test_facing_arrow_builder() {
        let arrow = BillboardInstance::facing_arrow([10.0, 0.5, 20.0], 1.57, [1.0, 0.9, 0.2, 1.0]);
        assert_eq!(arrow.position, [10.0, 0.5, 20.0]);
        assert_eq!(
            arrow.flags & BILLBOARD_FLAG_FACING_ARROW,
            BILLBOARD_FLAG_FACING_ARROW
        );
        assert_eq!(arrow.tint, [1.0, 0.9, 0.2, 1.0]);
        assert_eq!(arrow.anchor, [1.57, 0.0]);
        assert!((arrow.size[0] - 0.382_683).abs() < 1e-4);
        assert!((arrow.size[1] - 1.75).abs() < 1e-6);
    }

    #[test]
    fn test_facing_arrow_decoded_polygons() {
        let arrow = BillboardInstance::facing_arrow([0.0; 3], 0.0, [1.0; 4]);
        assert!(arrow.facing_arrow_contains(0.0, 0.0));
        assert!(arrow.facing_arrow_contains(0.06, 1.0));
        assert!(!arrow.facing_arrow_contains(0.07, 1.0));
        assert!(!arrow.facing_arrow_contains(0.0, -0.01));
        assert!(arrow.facing_arrow_contains(0.0, 1.74));
        assert!(arrow.facing_arrow_contains(0.18, 1.30));
        assert!(!arrow.facing_arrow_contains(0.18, 1.25));
        assert!(!arrow.facing_arrow_contains(0.0, 1.76));
        assert!(!arrow.facing_arrow_contains(0.2, 1.4));
    }

    #[test]
    fn test_frame_to_rgba_palette_transparency() {
        let palette = vec![[0, 0, 0, 255], [255, 0, 0, 255], [0, 255, 0, 255]];
        let frame = BitmapFrame {
            index: 0,
            name: "test".into(),
            width: 2,
            height: 2,
            offset_x: 0,
            offset_y: 0,
            flags: 0,
            pixel_data_offset: 0,
            pixel_data_size: 4,
            pixels: vec![0, 1, 2, 0],
            wide_pixels: Vec::new(),
        };

        let img = frame_to_rgba_image(&frame, &palette);
        assert_eq!(img.width(), 2);
        assert_eq!(img.height(), 2);

        assert_eq!(img.get_pixel(0, 0).0, [0, 0, 0, 0]);
        assert_eq!(img.get_pixel(1, 0).0, [255, 0, 0, 255]);
        assert_eq!(img.get_pixel(0, 1).0, [0, 255, 0, 255]);
        assert_eq!(img.get_pixel(1, 1).0, [0, 0, 0, 0]);
    }

    #[test]
    fn test_negative_and_wrapping_yaw_angles() {
        let two_pi = core::f32::consts::PI * 2.0;

        let (view, flip) = select_sprite_view(-core::f32::consts::FRAC_PI_2, 0.0, 8);
        assert_eq!(view, 6);
        assert!(!flip);

        let (view, flip) = select_sprite_view(-core::f32::consts::FRAC_PI_2, 0.0, 5);
        assert_eq!(view, 2);
        assert!(flip);

        let (view, flip) = select_sprite_view(two_pi * 5.0, 0.0, 8);
        assert_eq!(view, 0);
        assert!(!flip);
    }

    #[test]
    fn test_billboard_instance_builders() {
        let inst = BillboardInstance::new([1.0, 2.0, 3.0], [2.0, 4.0], [0.0, 0.0], [1.0, 1.0])
            .with_flip_x(true)
            .with_cylindrical(true)
            .with_centered(true)
            .with_tint([1.0, 0.5, 0.25, 0.8]);

        assert_eq!(inst.position, [1.0, 2.0, 3.0]);
        assert_eq!(inst.size, [2.0, 4.0]);
        assert_eq!(
            inst.flags,
            BILLBOARD_FLAG_FLIP_X | BILLBOARD_FLAG_CYLINDRICAL | BILLBOARD_FLAG_CENTERED
        );
        assert_eq!(inst.tint, [1.0, 0.5, 0.25, 0.8]);
    }

    #[test]
    fn test_billboard_anchor_registration_point_and_horizontal_flip() {
        let frame = AtlasFrame {
            index: 0,
            uv_min: [0.1, 0.2],
            uv_max: [0.6, 0.8],
            width: 100,
            height: 80,
            offset_x: 25,
            offset_y: 60,
        };

        let unflipped =
            BillboardInstance::from_atlas_frame(&frame, [10.0, 0.0, 20.0], [4.0, 3.2], false);
        assert!((unflipped.anchor[0] - 0.25).abs() < 1e-5);
        assert!((unflipped.anchor[1] - 0.75).abs() < 1e-5);
        assert_eq!(unflipped.flags & BILLBOARD_FLAG_FLIP_X, 0);

        let (x_bounds, y_bounds) = unflipped.effective_quad_bounds();
        assert!((x_bounds[0] - (-1.0)).abs() < 1e-5);
        assert!((x_bounds[1] - 3.0).abs() < 1e-5);
        assert!((y_bounds[0] - (-0.8)).abs() < 1e-5);
        assert!((y_bounds[1] - 2.4).abs() < 1e-5);

        let flipped =
            BillboardInstance::from_atlas_frame(&frame, [10.0, 0.0, 20.0], [4.0, 3.2], true);
        assert_eq!(flipped.flags & BILLBOARD_FLAG_FLIP_X, BILLBOARD_FLAG_FLIP_X);
        let (fx_bounds, fy_bounds) = flipped.effective_quad_bounds();
        assert!((fx_bounds[0] - (-3.0)).abs() < 1e-5);
        assert!((fx_bounds[1] - 1.0).abs() < 1e-5);
        assert!((fy_bounds[0] - (-0.8)).abs() < 1e-5);
        assert!((fy_bounds[1] - 2.4).abs() < 1e-5);
    }

    #[test]
    fn test_billboard_anchor_uses_signed_key_point() {
        let frame = AtlasFrame {
            index: 0,
            uv_min: [0.0, 0.0],
            uv_max: [1.0, 1.0],
            width: 10,
            height: 20,
            offset_x: 5,
            offset_y: 21,
        };
        let inst = BillboardInstance::from_atlas_frame(&frame, [0.0; 3], [1.0, 2.0], false);
        let (_, y) = inst.effective_quad_bounds();
        assert!((y[0] - 0.1).abs() < 1e-5, "bottom {}", y[0]);
        assert!((y[1] - 2.1).abs() < 1e-5, "top {}", y[1]);

        let frame = AtlasFrame {
            offset_x: -2,
            offset_y: 0,
            ..frame
        };
        let inst = BillboardInstance::from_atlas_frame(&frame, [0.0; 3], [1.0, 2.0], false);
        let (x, y) = inst.effective_quad_bounds();
        assert!(
            (x[0] - 0.2).abs() < 1e-5 && (x[1] - 1.2).abs() < 1e-5,
            "{x:?}"
        );
        assert!((y[0] - (-2.0)).abs() < 1e-5 && y[1].abs() < 1e-5, "{y:?}");
    }

    #[test]
    fn billboard_shader_guards_degenerate_edges() {
        let src = BillboardRenderer::BILLBOARD_SHADER_SRC;
        assert!(src.contains("xz_len > 1.0e-4"));
        assert!(src.contains("max(clip.w, 1.0e-4)"));
    }
}
