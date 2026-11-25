use std::num::NonZeroU64;

const SCREEN_RING_SLOTS: u32 = 64;
const INITIAL_INSTANCE_CAPACITY: usize = 256;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OverlayLayer {
    PostRasterization(i16),
    Dialog,
}

impl Ord for OverlayLayer {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        use OverlayLayer::{Dialog, PostRasterization};
        match (self, other) {
            (PostRasterization(a), PostRasterization(b)) => b.cmp(a),
            (PostRasterization(_), Dialog) => std::cmp::Ordering::Less,
            (Dialog, PostRasterization(_)) => std::cmp::Ordering::Greater,
            (Dialog, Dialog) => std::cmp::Ordering::Equal,
        }
    }
}

impl PartialOrd for OverlayLayer {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

pub fn transparency_alpha(transparency: u16) -> u8 {
    ((0xffff - transparency as u32) >> 8) as u8
}

pub const TRANSLUCENT_BAR_TRANSPARENCY: u16 = 0xb333;

pub struct OverlayImage {
    texture: wgpu::Texture,
    bind_group: wgpu::BindGroup,
    pub width: u32,
    pub height: u32,
}

#[derive(Clone, Copy)]
pub struct OverlayDraw<'a> {
    pub layer: OverlayLayer,
    pub image: &'a OverlayImage,
    pub src: [i32; 4],
    pub dst: [i32; 4],
    pub alpha: u8,
    pub tint: [u8; 3],
}

impl<'a> OverlayDraw<'a> {
    pub fn at(layer: OverlayLayer, image: &'a OverlayImage, x: i32, y: i32, alpha: u8) -> Self {
        let (w, h) = (image.width as i32, image.height as i32);
        Self {
            layer,
            image,
            src: [0, 0, w, h],
            dst: [x, y, w, h],
            alpha,
            tint: [0xff; 3],
        }
    }

    pub fn fill(
        layer: OverlayLayer,
        image: &'a OverlayImage,
        dst: [i32; 4],
        tint: [u8; 3],
        alpha: u8,
    ) -> Self {
        Self {
            layer,
            image,
            src: [0, 0, 1, 1],
            dst,
            alpha,
            tint,
        }
    }
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct Instance {
    dst: [f32; 4],
    uv: [f32; 4],
    tint: [f32; 4],
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct Screen {
    size: [f32; 2],
    _pad: [f32; 2],
}

struct OverlayPipelines {
    pipeline: wgpu::RenderPipeline,
    image_layout: wgpu::BindGroupLayout,
    screen_layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
    linear_sampler: wgpu::Sampler,
    format: wgpu::TextureFormat,
}

pub struct OverlayRenderer {
    pipeline: wgpu::RenderPipeline,
    image_layout: wgpu::BindGroupLayout,
    screen_layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
    linear_sampler: wgpu::Sampler,
    screen_buffer: wgpu::Buffer,
    screen_bind_group: wgpu::BindGroup,
    screen_stride: u32,
    screen_slot: u32,
    instances: wgpu::Buffer,
    capacity: usize,
    instance_cursor: usize,
    order_scratch: Vec<usize>,
    instances_scratch: Vec<Instance>,
    pub format: wgpu::TextureFormat,
}

impl OverlayRenderer {
    pub const SHADER_SRC: &'static str = include_str!("../shaders/overlay.wgsl");

    pub fn new(device: &wgpu::Device, format: wgpu::TextureFormat) -> Self {
        let screen_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("Overlay Screen Layout"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: true,
                    min_binding_size: NonZeroU64::new(std::mem::size_of::<Screen>() as u64),
                },
                count: None,
            }],
        });
        let image_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("Overlay Image Layout"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
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
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("Overlay Pipeline Layout"),
            bind_group_layouts: &[Some(&screen_layout), Some(&image_layout)],
            immediate_size: 0,
        });
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Overlay WGSL Shader"),
            source: wgpu::ShaderSource::Wgsl(Self::SHADER_SRC.into()),
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("Overlay Pipeline"),
            layout: Some(&layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                buffers: &[Some(wgpu::VertexBufferLayout {
                    array_stride: std::mem::size_of::<Instance>() as u64,
                    step_mode: wgpu::VertexStepMode::Instance,
                    attributes: &wgpu::vertex_attr_array![
                        0 => Float32x4, 1 => Float32x4, 2 => Float32x4
                    ],
                })],
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
                    blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: Default::default(),
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleStrip,
                ..Default::default()
            },
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        });
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("Overlay Sampler"),
            mag_filter: wgpu::FilterMode::Nearest,
            min_filter: wgpu::FilterMode::Nearest,
            ..Default::default()
        });
        let linear_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("Overlay Linear Sampler"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        Self::with_pipelines(
            device,
            OverlayPipelines {
                pipeline,
                image_layout,
                screen_layout,
                sampler,
                linear_sampler,
                format,
            },
        )
    }

    pub fn sharing_pipelines(&self, device: &wgpu::Device) -> Self {
        Self::with_pipelines(
            device,
            OverlayPipelines {
                pipeline: self.pipeline.clone(),
                image_layout: self.image_layout.clone(),
                screen_layout: self.screen_layout.clone(),
                sampler: self.sampler.clone(),
                linear_sampler: self.linear_sampler.clone(),
                format: self.format,
            },
        )
    }

    fn with_pipelines(device: &wgpu::Device, pipelines: OverlayPipelines) -> Self {
        let OverlayPipelines {
            pipeline,
            image_layout,
            screen_layout,
            sampler,
            linear_sampler,
            format,
        } = pipelines;
        let screen_size = std::mem::size_of::<Screen>() as u32;
        let screen_stride = device
            .limits()
            .min_uniform_buffer_offset_alignment
            .max(screen_size);
        let screen_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Overlay Screen"),
            size: u64::from(SCREEN_RING_SLOTS) * u64::from(screen_stride),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let screen_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Overlay Screen Bind Group"),
            layout: &screen_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                    buffer: &screen_buffer,
                    offset: 0,
                    size: NonZeroU64::new(u64::from(screen_size)),
                }),
            }],
        });
        let capacity = INITIAL_INSTANCE_CAPACITY;
        Self {
            pipeline,
            image_layout,
            screen_layout,
            sampler,
            linear_sampler,
            screen_buffer,
            screen_bind_group,
            screen_stride,
            screen_slot: 0,
            instances: Self::instance_buffer(device, capacity),
            capacity,
            instance_cursor: 0,
            order_scratch: Vec::new(),
            instances_scratch: Vec::new(),
            format,
        }
    }

    fn instance_buffer(device: &wgpu::Device, n: usize) -> wgpu::Buffer {
        device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Overlay Instances"),
            size: (n * std::mem::size_of::<Instance>()) as u64,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        })
    }

    pub fn create_image(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        width: u32,
        height: u32,
        rgba: &[u8],
    ) -> OverlayImage {
        self.create_image_filtered(device, queue, width, height, rgba, false)
    }

    pub fn create_image_filtered(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        width: u32,
        height: u32,
        rgba: &[u8],
        linear: bool,
    ) -> OverlayImage {
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("Overlay Image"),
            size: wgpu::Extent3d {
                width: width.max(1),
                height: height.max(1),
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Overlay Image Bind Group"),
            layout: &self.image_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(if linear {
                        &self.linear_sampler
                    } else {
                        &self.sampler
                    }),
                },
            ],
        });
        let image = OverlayImage {
            texture,
            bind_group,
            width,
            height,
        };
        Self::update_image(queue, &image, rgba);
        image
    }

    pub fn update_image(queue: &wgpu::Queue, image: &OverlayImage, rgba: &[u8]) {
        if image.width == 0 || image.height == 0 {
            return;
        }
        queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &image.texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            rgba,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(4 * image.width),
                rows_per_image: Some(image.height),
            },
            wgpu::Extent3d {
                width: image.width,
                height: image.height,
                depth_or_array_layers: 1,
            },
        );
    }

    pub fn encode(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        target: &wgpu::TextureView,
        screen: (u32, u32),
        draws: &[OverlayDraw<'_>],
    ) {
        if draws.is_empty() {
            return;
        }
        self.encode_pass(
            device,
            queue,
            encoder,
            target,
            screen,
            draws,
            wgpu::LoadOp::Load,
        );
    }

    pub fn encode_cleared(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        target: &wgpu::TextureView,
        screen: (u32, u32),
        draws: &[OverlayDraw<'_>],
    ) {
        self.encode_pass(
            device,
            queue,
            encoder,
            target,
            screen,
            draws,
            wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
        );
    }

    #[allow(clippy::too_many_arguments)]
    fn encode_pass(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        target: &wgpu::TextureView,
        screen: (u32, u32),
        draws: &[OverlayDraw<'_>],
        load: wgpu::LoadOp<wgpu::Color>,
    ) {
        self.order_scratch.clear();
        self.order_scratch.extend(0..draws.len());
        self.order_scratch.sort_by_key(|&i| draws[i].layer);
        self.instances_scratch.clear();
        self.instances_scratch.reserve(self.order_scratch.len());
        for &idx in &self.order_scratch {
            let d = &draws[idx];
            let (iw, ih) = (d.image.width.max(1) as f32, d.image.height.max(1) as f32);
            self.instances_scratch.push(Instance {
                dst: d.dst.map(|v| v as f32),
                uv: [
                    d.src[0] as f32 / iw,
                    d.src[1] as f32 / ih,
                    (d.src[0] + d.src[2]) as f32 / iw,
                    (d.src[1] + d.src[3]) as f32 / ih,
                ],
                tint: [
                    d.tint[0] as f32 / 255.0,
                    d.tint[1] as f32 / 255.0,
                    d.tint[2] as f32 / 255.0,
                    d.alpha as f32 / 255.0,
                ],
            });
        }
        let count = self.instances_scratch.len();
        let instance_slice = if count > 0 {
            if count * 4 > self.capacity {
                self.capacity = (count * 4)
                    .max(INITIAL_INSTANCE_CAPACITY)
                    .next_power_of_two();
                self.instances = Self::instance_buffer(device, self.capacity);
                self.instance_cursor = 0;
            } else if self.instance_cursor + count > self.capacity {
                self.instance_cursor = 0;
            }
            let start_bytes = (self.instance_cursor * std::mem::size_of::<Instance>()) as u64;
            let len_bytes = (count * std::mem::size_of::<Instance>()) as u64;
            queue.write_buffer(
                &self.instances,
                start_bytes,
                bytemuck::cast_slice(&self.instances_scratch),
            );
            self.instance_cursor += count;
            Some(start_bytes..start_bytes + len_bytes)
        } else {
            None
        };
        let screen_offset = self.screen_slot * self.screen_stride;
        self.screen_slot = (self.screen_slot + 1) % SCREEN_RING_SLOTS;
        queue.write_buffer(
            &self.screen_buffer,
            u64::from(screen_offset),
            bytemuck::bytes_of(&Screen {
                size: [screen.0.max(1) as f32, screen.1.max(1) as f32],
                _pad: [0.0; 2],
            }),
        );
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("Interface Overlay Pass"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                depth_slice: None,
                view: target,
                resolve_target: None,
                ops: wgpu::Operations {
                    load,
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            occlusion_query_set: None,
            timestamp_writes: None,
            multiview_mask: None,
        });
        let Some(slice) = instance_slice else {
            return;
        };
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, &self.screen_bind_group, &[screen_offset]);
        pass.set_vertex_buffer(0, self.instances.slice(slice));
        let mut start = 0usize;
        while start < self.order_scratch.len() {
            let image = draws[self.order_scratch[start]].image;
            let mut end = start + 1;
            while end < self.order_scratch.len()
                && std::ptr::eq(draws[self.order_scratch[end]].image, image)
            {
                end += 1;
            }
            pass.set_bind_group(1, &image.bind_group, &[]);
            pass.draw(0..4, start as u32..end as u32);
            start = end;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn overlay_wgsl_parses() {
        let module = naga::front::wgsl::parse_str(OverlayRenderer::SHADER_SRC);
        assert!(module.is_ok(), "{:?}", module.err());
    }

    #[test]
    fn transparency_alpha_matches_the_engine() {
        assert_eq!(transparency_alpha(0), 255);
        assert_eq!(transparency_alpha(TRANSLUCENT_BAR_TRANSPARENCY), 0x4c);
        assert_eq!(transparency_alpha(0xaaaa), 0x55);
    }

    #[test]
    fn layers_order_post_rasterization_high_to_low_then_dialogs() {
        let mut v = [
            OverlayLayer::PostRasterization(-100),
            OverlayLayer::Dialog,
            OverlayLayer::PostRasterization(1),
            OverlayLayer::PostRasterization(3),
        ];
        v.sort();
        assert_eq!(
            v,
            [
                OverlayLayer::PostRasterization(3),
                OverlayLayer::PostRasterization(1),
                OverlayLayer::PostRasterization(-100),
                OverlayLayer::Dialog
            ]
        );
    }

    #[test]
    fn a_layer_keeps_submission_order() {
        let mut v = [
            (OverlayLayer::PostRasterization(1), 0),
            (OverlayLayer::PostRasterization(3), 1),
            (OverlayLayer::PostRasterization(1), 2),
        ];
        v.sort_by_key(|d| d.0);
        assert_eq!(v.map(|d| d.1), [1, 0, 2]);
    }
}
