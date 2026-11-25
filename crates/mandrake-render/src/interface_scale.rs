use std::num::NonZeroU64;
use std::sync::atomic::{AtomicU32, Ordering};

pub(crate) use mandrake_view::interface_picture::InterfaceFilter;

const UNIFORM_RING_SLOTS: u32 = 32;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct ClippedDst {
    origin: (i32, i32),
    size: (u32, u32),
    viewport: (u32, u32, u32, u32),
}

fn clip_dst_rect(dst: Option<[i32; 4]>, target: (u32, u32)) -> Option<ClippedDst> {
    if target.0 == 0 || target.1 == 0 {
        return None;
    }
    let [x, y, w, h] = match dst {
        Some(rect) => rect,
        None => [0, 0, target.0 as i32, target.1 as i32],
    };
    if w <= 0 || h <= 0 {
        return None;
    }
    let x0 = i64::from(x).max(0);
    let y0 = i64::from(y).max(0);
    let x1 = (i64::from(x) + i64::from(w)).min(i64::from(target.0));
    let y1 = (i64::from(y) + i64::from(h)).min(i64::from(target.1));
    if x1 <= x0 || y1 <= y0 {
        return None;
    }
    Some(ClippedDst {
        origin: (x, y),
        size: (w as u32, h as u32),
        viewport: (x0 as u32, y0 as u32, (x1 - x0) as u32, (y1 - y0) as u32),
    })
}

pub(crate) trait FilterModes {
    fn mode(self) -> u32;
    fn sprite_mode(self) -> u32;
}

impl FilterModes for InterfaceFilter {
    fn mode(self) -> u32 {
        match self {
            Self::Sharp => 0,
            Self::Nearest => 1,
            Self::Linear => 2,
            Self::Lanczos2 | Self::Fsr1 => 3,
        }
    }

    fn sprite_mode(self) -> u32 {
        match self {
            Self::Linear => 0,
            Self::Sharp => 1,
            Self::Nearest => 2,
            Self::Lanczos2 | Self::Fsr1 => 3,
        }
    }
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct Params {
    size: [f32; 2],
    scale: [f32; 2],
    uv_offset: [f32; 2],
    view_size: [f32; 2],
    fade_rows: f32,
    alpha: f32,
    mode: u32,
    sharpen: f32,
}

static LANCZOS_SHARPEN: AtomicU32 = AtomicU32::new(0.4_f32.to_bits());

pub fn set_lanczos_sharpen(sharpen: f32) {
    let clamped = if sharpen.is_finite() {
        sharpen.clamp(0.0, 1.0)
    } else {
        0.4
    };
    LANCZOS_SHARPEN.store(clamped.to_bits(), Ordering::Relaxed);
}

fn lanczos_sharpen() -> f32 {
    f32::from_bits(LANCZOS_SHARPEN.load(Ordering::Relaxed))
}

static FSR_SHARPNESS: AtomicU32 = AtomicU32::new(FSR_DEFAULT_SHARPNESS.to_bits());

pub const FSR_DEFAULT_SHARPNESS: f32 = 0.5;

pub fn set_fsr_sharpness(stops: f32) {
    let clamped = if stops.is_finite() {
        stops.clamp(0.0, 2.0)
    } else {
        FSR_DEFAULT_SHARPNESS
    };
    FSR_SHARPNESS.store(clamped.to_bits(), Ordering::Relaxed);
}

fn fsr_rcas_scale() -> f32 {
    (-f32::from_bits(FSR_SHARPNESS.load(Ordering::Relaxed))).exp2()
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct FsrParams {
    in_size: [f32; 2],
    out_size: [f32; 2],
    out_origin: [f32; 2],
    sharpness: f32,
    flags: u32,
}

#[derive(Clone)]
struct FsrPipelines {
    layout: wgpu::BindGroupLayout,
    easu: wgpu::RenderPipeline,
    rcas: wgpu::RenderPipeline,
}

const FSR_INTERMEDIATE_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;

struct FsrTarget {
    size: (u32, u32),
    layer_generation: u64,
    view: wgpu::TextureView,
    easu_bind_group: wgpu::BindGroup,
    rcas_bind_group: wgpu::BindGroup,
}

impl FsrPipelines {
    fn new(device: &wgpu::Device, format: wgpu::TextureFormat) -> Self {
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("FSR1 Layout"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: true,
                        min_binding_size: NonZeroU64::new(std::mem::size_of::<FsrParams>() as u64),
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: false },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
            ],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("FSR1 Pipeline Layout"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("FSR1 WGSL Shader"),
            source: wgpu::ShaderSource::Wgsl(InterfaceScaler::FSR1_SHADER_SRC.into()),
        });
        let pipeline = |label, entry, format, blend| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(label),
                layout: Some(&pipeline_layout),
                vertex: wgpu::VertexState {
                    module: &shader,
                    entry_point: Some("vs_main"),
                    buffers: &[],
                    compilation_options: Default::default(),
                },
                fragment: Some(wgpu::FragmentState {
                    module: &shader,
                    entry_point: Some(entry),
                    targets: &[Some(wgpu::ColorTargetState {
                        format,
                        blend,
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
            })
        };
        Self {
            easu: pipeline(
                "FSR1 EASU Pipeline",
                "fs_easu",
                FSR_INTERMEDIATE_FORMAT,
                None,
            ),
            rcas: pipeline(
                "FSR1 RCAS Pipeline",
                "fs_rcas",
                format,
                Some(wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING),
            ),
            layout,
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct ScrollDraw {
    pub dst: [f32; 4],
    pub view_size: [f32; 2],
    pub uv_offset: [f32; 2],
    pub fade_rows: f32,
    pub alpha: f32,
}

struct Layer {
    view: wgpu::TextureView,
    bind_group: wgpu::BindGroup,
    size: (u32, u32),
    generation: u64,
}

struct ScalerPipelines {
    pipeline: wgpu::RenderPipeline,
    layout: wgpu::BindGroupLayout,
    linear_sampler: wgpu::Sampler,
    nearest_sampler: wgpu::Sampler,
    fsr: FsrPipelines,
    format: wgpu::TextureFormat,
}

pub struct InterfaceScaler {
    pipeline: wgpu::RenderPipeline,
    layout: wgpu::BindGroupLayout,
    params: wgpu::Buffer,
    scroll_params: wgpu::Buffer,
    uniform_stride: u32,
    params_slot: AtomicU32,
    scroll_slot: AtomicU32,
    fsr_slot: AtomicU32,
    linear_sampler: wgpu::Sampler,
    nearest_sampler: wgpu::Sampler,
    fsr: FsrPipelines,
    easu_params: wgpu::Buffer,
    rcas_params: wgpu::Buffer,
    fsr_target: std::sync::Mutex<Option<FsrTarget>>,
    device: wgpu::Device,
    layers_made: u64,
    pub format: wgpu::TextureFormat,
    layer: Option<Layer>,
    scroll_layer: Option<Layer>,
}

impl InterfaceScaler {
    pub const SHADER_SRC: &'static str = include_str!("../shaders/interface_scale.wgsl");
    pub const FSR1_SHADER_SRC: &'static str = include_str!("../shaders/fsr1.wgsl");

    pub fn new(device: &wgpu::Device, format: wgpu::TextureFormat) -> Self {
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("Interface Scale Layout"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: true,
                        min_binding_size: NonZeroU64::new(std::mem::size_of::<Params>() as u64),
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 3,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("Interface Scale Pipeline Layout"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Interface Scale WGSL Shader"),
            source: wgpu::ShaderSource::Wgsl(Self::SHADER_SRC.into()),
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("Interface Scale Pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                buffers: &[],
                compilation_options: Default::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_main"),
                targets: &[Some(wgpu::ColorTargetState {
                    format,
                    blend: Some(wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING),
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
        let linear_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("Interface Scale Linear Sampler"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        let nearest_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("Interface Scale Nearest Sampler"),
            mag_filter: wgpu::FilterMode::Nearest,
            min_filter: wgpu::FilterMode::Nearest,
            ..Default::default()
        });
        Self::with_pipelines(
            device,
            ScalerPipelines {
                pipeline,
                layout,
                linear_sampler,
                nearest_sampler,
                fsr: FsrPipelines::new(device, format),
                format,
            },
        )
    }

    pub fn sharing_pipelines(&self, device: &wgpu::Device) -> Self {
        Self::with_pipelines(
            device,
            ScalerPipelines {
                pipeline: self.pipeline.clone(),
                layout: self.layout.clone(),
                linear_sampler: self.linear_sampler.clone(),
                nearest_sampler: self.nearest_sampler.clone(),
                fsr: self.fsr.clone(),
                format: self.format,
            },
        )
    }

    fn with_pipelines(device: &wgpu::Device, pipelines: ScalerPipelines) -> Self {
        let ScalerPipelines {
            pipeline,
            layout,
            linear_sampler,
            nearest_sampler,
            fsr,
            format,
        } = pipelines;
        let uniform_stride = device
            .limits()
            .min_uniform_buffer_offset_alignment
            .max(std::mem::size_of::<Params>() as u32)
            .max(std::mem::size_of::<FsrParams>() as u32);
        let ring_buffer = |label| {
            device.create_buffer(&wgpu::BufferDescriptor {
                label: Some(label),
                size: u64::from(UNIFORM_RING_SLOTS) * u64::from(uniform_stride),
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            })
        };
        Self {
            pipeline,
            layout,
            params: ring_buffer("Interface Scale Params"),
            scroll_params: ring_buffer("Interface Scale Scroll Params"),
            uniform_stride,
            params_slot: AtomicU32::new(0),
            scroll_slot: AtomicU32::new(0),
            fsr_slot: AtomicU32::new(0),
            linear_sampler,
            nearest_sampler,
            fsr,
            easu_params: ring_buffer("FSR1 EASU Params"),
            rcas_params: ring_buffer("FSR1 RCAS Params"),
            fsr_target: std::sync::Mutex::new(None),
            device: device.clone(),
            layers_made: 0,
            format,
            layer: None,
            scroll_layer: None,
        }
    }

    fn next_offset(&self, counter: &AtomicU32) -> u32 {
        (counter.fetch_add(1, Ordering::Relaxed) % UNIFORM_RING_SLOTS) * self.uniform_stride
    }

    pub fn layer_view(&mut self, device: &wgpu::Device, size: (u32, u32)) -> &wgpu::TextureView {
        let size = (size.0.max(1), size.1.max(1));
        if self.layer.as_ref().is_none_or(|l| l.size != size) {
            let texture = device.create_texture(&wgpu::TextureDescriptor {
                label: Some("Interface Logical Layer"),
                size: wgpu::Extent3d {
                    width: size.0,
                    height: size.1,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: self.format,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                    | wgpu::TextureUsages::TEXTURE_BINDING,
                view_formats: &[],
            });
            let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
            let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("Interface Scale Bind Group"),
                layout: &self.layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                            buffer: &self.params,
                            offset: 0,
                            size: NonZeroU64::new(std::mem::size_of::<Params>() as u64),
                        }),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::TextureView(&view),
                    },
                    wgpu::BindGroupEntry {
                        binding: 2,
                        resource: wgpu::BindingResource::Sampler(&self.linear_sampler),
                    },
                    wgpu::BindGroupEntry {
                        binding: 3,
                        resource: wgpu::BindingResource::Sampler(&self.nearest_sampler),
                    },
                ],
            });
            self.layers_made += 1;
            self.layer = Some(Layer {
                view,
                bind_group,
                size,
                generation: self.layers_made,
            });
        }
        &self.layer.as_ref().expect("layer just made").view
    }

    pub fn composite(
        &self,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        target: &wgpu::TextureView,
        window: (u32, u32),
        filter: InterfaceFilter,
    ) {
        self.composite_into(queue, encoder, target, None, window, filter);
    }

    pub fn composite_into(
        &self,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        target: &wgpu::TextureView,
        dst: Option<[i32; 4]>,
        window: (u32, u32),
        filter: InterfaceFilter,
    ) {
        let Some(layer) = &self.layer else {
            return;
        };
        let Some(rect) = clip_dst_rect(dst, window) else {
            return;
        };
        let filter = match filter {
            InterfaceFilter::Fsr1 if rect.size == layer.size => InterfaceFilter::Nearest,
            InterfaceFilter::Fsr1 => {
                self.composite_fsr1(queue, encoder, target, layer, rect);
                return;
            }
            other => other,
        };
        let offset = self.write_params(queue, layer, rect, filter);
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("Interface Scale Pass"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                depth_slice: None,
                view: target,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Load,
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            occlusion_query_set: None,
            timestamp_writes: None,
            multiview_mask: None,
        });
        let (vx, vy, vw, vh) = rect.viewport;
        if (vx, vy, vw, vh) != (0, 0, window.0, window.1) {
            pass.set_viewport(vx as f32, vy as f32, vw as f32, vh as f32, 0.0, 1.0);
        }
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, &layer.bind_group, &[offset]);
        pass.draw(0..4, 0..1);
    }

    pub fn composite_content(
        &self,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        target: &wgpu::TextureView,
        window: (u32, u32),
        filter: InterfaceFilter,
        content: &[[i32; 4]],
    ) {
        let Some(layer) = &self.layer else {
            return;
        };
        let Some(rect) = clip_dst_rect(None, window) else {
            return;
        };
        let filter = match filter {
            InterfaceFilter::Fsr1 if window == layer.size => InterfaceFilter::Nearest,
            InterfaceFilter::Fsr1 => {
                self.composite(queue, encoder, target, window, filter);
                return;
            }
            other => other,
        };
        let scissors = content_scissors(content, layer.size, window);
        if scissors.is_empty() {
            return;
        }
        let offset = self.write_params(queue, layer, rect, filter);
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("Interface Scale Pass"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                depth_slice: None,
                view: target,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Load,
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            occlusion_query_set: None,
            timestamp_writes: None,
            multiview_mask: None,
        });
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, &layer.bind_group, &[offset]);
        for [x, y, w, h] in scissors {
            pass.set_scissor_rect(x, y, w, h);
            pass.draw(0..4, 0..1);
        }
    }

    fn write_params(
        &self,
        queue: &wgpu::Queue,
        layer: &Layer,
        rect: ClippedDst,
        filter: InterfaceFilter,
    ) -> u32 {
        let (lw, lh) = (layer.size.0 as f32, layer.size.1 as f32);
        let scale = [rect.size.0 as f32 / lw, rect.size.1 as f32 / lh];
        let (vx, vy, vw, vh) = rect.viewport;
        let clipped = vx as i32 != rect.origin.0
            || vy as i32 != rect.origin.1
            || vw != rect.size.0
            || vh != rect.size.1;
        let (uv_offset, view_size) = if clipped {
            (
                [
                    (vx as f32 - rect.origin.0 as f32) / scale[0],
                    (vy as f32 - rect.origin.1 as f32) / scale[1],
                ],
                [vw as f32 / scale[0], vh as f32 / scale[1]],
            )
        } else {
            ([0.0, 0.0], [lw, lh])
        };
        let offset = self.next_offset(&self.params_slot);
        queue.write_buffer(
            &self.params,
            u64::from(offset),
            bytemuck::bytes_of(&Params {
                size: [lw, lh],
                scale,
                uv_offset,
                view_size,
                fade_rows: 0.0,
                alpha: 1.0,
                mode: filter.mode(),
                sharpen: lanczos_sharpen(),
            }),
        );
        offset
    }

    fn composite_fsr1(
        &self,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        target: &wgpu::TextureView,
        layer: &Layer,
        rect: ClippedDst,
    ) {
        let size = rect.size;
        let mut fsr_target = self.fsr_target.lock().unwrap_or_else(|e| e.into_inner());
        if fsr_target
            .as_ref()
            .is_none_or(|t| t.size != size || t.layer_generation != layer.generation)
        {
            *fsr_target = Some(self.make_fsr_target(layer, size));
        }
        let fsr = fsr_target.as_ref().expect("target just made");
        let flags = u32::from(self.format.is_srgb());
        let out_size = [size.0 as f32, size.1 as f32];
        let offset = self.next_offset(&self.fsr_slot);
        queue.write_buffer(
            &self.easu_params,
            u64::from(offset),
            bytemuck::bytes_of(&FsrParams {
                in_size: [layer.size.0 as f32, layer.size.1 as f32],
                out_size,
                out_origin: [0.0, 0.0],
                sharpness: 0.0,
                flags,
            }),
        );
        queue.write_buffer(
            &self.rcas_params,
            u64::from(offset),
            bytemuck::bytes_of(&FsrParams {
                in_size: out_size,
                out_size,
                out_origin: [rect.origin.0 as f32, rect.origin.1 as f32],
                sharpness: fsr_rcas_scale(),
                flags,
            }),
        );
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("FSR1 EASU Pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    depth_slice: None,
                    view: &fsr.view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                occlusion_query_set: None,
                timestamp_writes: None,
                multiview_mask: None,
            });
            pass.set_pipeline(&self.fsr.easu);
            pass.set_bind_group(0, &fsr.easu_bind_group, &[offset]);
            pass.draw(0..4, 0..1);
        }
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("FSR1 RCAS Pass"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                depth_slice: None,
                view: target,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Load,
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            occlusion_query_set: None,
            timestamp_writes: None,
            multiview_mask: None,
        });
        let (vx, vy, vw, vh) = rect.viewport;
        pass.set_viewport(vx as f32, vy as f32, vw as f32, vh as f32, 0.0, 1.0);
        pass.set_pipeline(&self.fsr.rcas);
        pass.set_bind_group(0, &fsr.rcas_bind_group, &[offset]);
        pass.draw(0..4, 0..1);
    }

    fn make_fsr_target(&self, layer: &Layer, size: (u32, u32)) -> FsrTarget {
        let device = &self.device;
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("FSR1 Intermediate"),
            size: wgpu::Extent3d {
                width: size.0.max(1),
                height: size.1.max(1),
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: FSR_INTERMEDIATE_FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        let bind_group = |label, params: &wgpu::Buffer, source: &wgpu::TextureView| {
            device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some(label),
                layout: &self.fsr.layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                            buffer: params,
                            offset: 0,
                            size: NonZeroU64::new(std::mem::size_of::<FsrParams>() as u64),
                        }),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::TextureView(source),
                    },
                ],
            })
        };
        FsrTarget {
            size,
            layer_generation: layer.generation,
            easu_bind_group: bind_group("FSR1 EASU Bind Group", &self.easu_params, &layer.view),
            rcas_bind_group: bind_group("FSR1 RCAS Bind Group", &self.rcas_params, &view),
            view,
        }
    }

    pub fn set_scroll_layer(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        width: u32,
        height: u32,
        rgba: &[u8],
    ) {
        let size = (width.max(1), height.max(1));
        let format = if self.format.is_srgb() {
            wgpu::TextureFormat::Rgba8UnormSrgb
        } else {
            wgpu::TextureFormat::Rgba8Unorm
        };
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("Interface Scroll Layer"),
            size: wgpu::Extent3d {
                width: size.0,
                height: size.1,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        if width > 0 && height > 0 {
            queue.write_texture(
                wgpu::TexelCopyTextureInfo {
                    texture: &texture,
                    mip_level: 0,
                    origin: wgpu::Origin3d::ZERO,
                    aspect: wgpu::TextureAspect::All,
                },
                rgba,
                wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(4 * width),
                    rows_per_image: Some(height),
                },
                wgpu::Extent3d {
                    width,
                    height,
                    depth_or_array_layers: 1,
                },
            );
        }
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Interface Scroll Bind Group"),
            layout: &self.layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                        buffer: &self.scroll_params,
                        offset: 0,
                        size: NonZeroU64::new(std::mem::size_of::<Params>() as u64),
                    }),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&view),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::Sampler(&self.linear_sampler),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: wgpu::BindingResource::Sampler(&self.nearest_sampler),
                },
            ],
        });
        self.scroll_layer = Some(Layer {
            view,
            bind_group,
            size,
            generation: 0,
        });
    }

    pub fn composite_scroll(
        &self,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        target: &wgpu::TextureView,
        target_size: (u32, u32),
        draw: ScrollDraw,
        filter: InterfaceFilter,
    ) {
        let Some(layer) = &self.scroll_layer else {
            return;
        };
        if draw.alpha <= 0.0 || target_size.0 == 0 || target_size.1 == 0 {
            return;
        }
        let x = draw.dst[0].clamp(0.0, target_size.0 as f32);
        let y = draw.dst[1].clamp(0.0, target_size.1 as f32);
        let w = draw.dst[2].min(target_size.0 as f32 - x);
        let h = draw.dst[3].min(target_size.1 as f32 - y);
        if w <= 0.0 || h <= 0.0 {
            return;
        }
        let (lw, lh) = (layer.size.0 as f32, layer.size.1 as f32);
        let (vw, vh) = (draw.view_size[0].max(1.0), draw.view_size[1].max(1.0));
        let offset = self.next_offset(&self.scroll_slot);
        queue.write_buffer(
            &self.scroll_params,
            u64::from(offset),
            bytemuck::bytes_of(&Params {
                size: [lw, lh],
                scale: [w / vw, h / vh],
                uv_offset: draw.uv_offset,
                view_size: [vw, vh],
                fade_rows: draw.fade_rows,
                alpha: draw.alpha,
                mode: filter.mode(),
                sharpen: lanczos_sharpen(),
            }),
        );
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("Interface Scroll Scale Pass"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                depth_slice: None,
                view: target,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Load,
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            occlusion_query_set: None,
            timestamp_writes: None,
            multiview_mask: None,
        });
        pass.set_viewport(x, y, w, h, 0.0, 1.0);
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, &layer.bind_group, &[offset]);
        pass.draw(0..4, 0..1);
    }
}

const TAP_REACH: f64 = 3.0;

pub fn content_scissors(
    content: &[[i32; 4]],
    layer: (u32, u32),
    window: (u32, u32),
) -> Vec<[u32; 4]> {
    let scale = (
        f64::from(window.0) / f64::from(layer.0.max(1)),
        f64::from(window.1) / f64::from(layer.1.max(1)),
    );
    let span = |at: i32, len: i32, scale: f64, size: u32| {
        let (lo, hi) = (at.min(at + len), at.max(at + len));
        if lo == hi {
            return None;
        }
        let p0 = ((f64::from(lo) - TAP_REACH) * scale - 0.5).floor().max(0.0);
        let p1 = ((f64::from(hi) + TAP_REACH) * scale)
            .ceil()
            .min(f64::from(size));
        (p1 > p0).then_some((p0 as u32, p1 as u32))
    };
    let boxes: Vec<((u32, u32), (u32, u32))> = content
        .iter()
        .filter_map(|r| {
            Some((
                span(r[0], r[2], scale.0, window.0)?,
                span(r[1], r[3], scale.1, window.1)?,
            ))
        })
        .collect();
    let mut edges: Vec<u32> = boxes.iter().flat_map(|(_, y)| [y.0, y.1]).collect();
    edges.sort_unstable();
    edges.dedup();
    let mut out: Vec<[u32; 4]> = Vec::new();
    let mut previous: Vec<(u32, u32)> = Vec::new();
    let mut previous_at = 0;
    for band in edges.windows(2) {
        let (y0, y1) = (band[0], band[1]);
        let mut spans: Vec<(u32, u32)> = boxes
            .iter()
            .filter(|(_, y)| y.0 <= y0 && y.1 >= y1)
            .map(|&(x, _)| x)
            .collect();
        spans.sort_unstable();
        let mut merged: Vec<(u32, u32)> = Vec::with_capacity(spans.len());
        for (a, b) in spans {
            match merged.last_mut() {
                Some(last) if a <= last.1 => last.1 = last.1.max(b),
                _ => merged.push((a, b)),
            }
        }
        if !merged.is_empty() && merged == previous {
            for rect in &mut out[previous_at..] {
                rect[3] += y1 - y0;
            }
        } else {
            previous_at = out.len();
            out.extend(merged.iter().map(|&(a, b)| [a, y0, b - a, y1 - y0]));
        }
        previous = merged;
    }
    out
}

pub fn upscale_offscreen(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    format: wgpu::TextureFormat,
    rgba: &[u8],
    size: (u32, u32),
    out: (u32, u32),
    filter: InterfaceFilter,
) -> Vec<u8> {
    let mut scaler = InterfaceScaler::new(device, format);
    let target = OffscreenTarget::new(device, format, out);
    let mut overlay = crate::OverlayRenderer::new(device, format);
    let image = overlay.create_image(device, queue, size.0, size.1, rgba);
    let mut encoder = device.create_command_encoder(&Default::default());
    let draw = crate::OverlayDraw::at(crate::OverlayLayer::Dialog, &image, 0, 0, 0xff);
    let layer = scaler.layer_view(device, size);
    overlay.encode_cleared(device, queue, &mut encoder, layer, size, &[draw]);
    target.clear(&mut encoder);
    scaler.composite(queue, &mut encoder, &target.view, out, filter);
    queue.submit([encoder.finish()]);
    target.read(device, queue)
}

pub struct OffscreenTarget {
    pub texture: wgpu::Texture,
    pub view: wgpu::TextureView,
    pub size: (u32, u32),
}

impl OffscreenTarget {
    pub fn new(device: &wgpu::Device, format: wgpu::TextureFormat, size: (u32, u32)) -> Self {
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("Offscreen Target"),
            size: wgpu::Extent3d {
                width: size.0,
                height: size.1,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let view = texture.create_view(&Default::default());
        Self {
            texture,
            view,
            size,
        }
    }

    pub fn clear(&self, encoder: &mut wgpu::CommandEncoder) {
        encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("Offscreen Clear"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                depth_slice: None,
                view: &self.view,
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
    }

    pub fn read(&self, device: &wgpu::Device, queue: &wgpu::Queue) -> Vec<u8> {
        let (w, h) = self.size;
        let row = (w * 4).div_ceil(256) * 256;
        let buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Offscreen Readback"),
            size: u64::from(row * h),
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut encoder = device.create_command_encoder(&Default::default());
        encoder.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo {
                texture: &self.texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::TexelCopyBufferInfo {
                buffer: &buffer,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(row),
                    rows_per_image: Some(h),
                },
            },
            self.texture.size(),
        );
        queue.submit([encoder.finish()]);
        let slice = buffer.slice(..);
        slice.map_async(wgpu::MapMode::Read, |r| r.expect("readback map"));
        device
            .poll(wgpu::PollType::wait_indefinitely())
            .expect("readback poll");
        let mapped = slice.get_mapped_range().expect("readback range");
        let bgra = matches!(
            self.texture.format(),
            wgpu::TextureFormat::Bgra8Unorm | wgpu::TextureFormat::Bgra8UnormSrgb
        );
        let mut out = Vec::with_capacity((w * h * 4) as usize);
        for y in 0..h {
            let start = (y * row) as usize;
            for p in mapped[start..start + (w * 4) as usize].as_chunks::<4>().0 {
                if bgra {
                    out.extend_from_slice(&[p[2], p[1], p[0], p[3]]);
                } else {
                    out.extend_from_slice(p);
                }
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn content_scissors_are_disjoint_and_cover_the_taps() {
        let layer = (154, 86);
        let window = (384, 216);
        let content = [
            [0, 0, 154, 10],
            [140, 15, 10, 12],
            [145, 20, 3, 3],
            [5, 70, 20, -6],
            [60, 80, 30, 6],
            [200, 30, 5, 5],
            [10, 40, 0, 4],
        ];
        let scissors = content_scissors(&content, layer, window);
        let mut hits = vec![0u8; (window.0 * window.1) as usize];
        for [x, y, w, h] in &scissors {
            assert!(x + w <= window.0 && y + h <= window.1, "{scissors:?}");
            for py in *y..y + h {
                for px in *x..x + w {
                    hits[(py * window.0 + px) as usize] += 1;
                }
            }
        }
        assert!(hits.iter().all(|&n| n <= 1), "overlapping scissors");
        let scale = (
            f64::from(window.0) / f64::from(layer.0),
            f64::from(window.1) / f64::from(layer.1),
        );
        let reach = |p: u32, scale: f64, size: u32| {
            let t = (f64::from(p) + 0.5) / scale;
            let lo = (t.floor() as i32 - 2).clamp(0, size as i32 - 1);
            let hi = (t.floor() as i32 + 2).clamp(0, size as i32 - 1);
            (lo, hi)
        };
        let mut drawn = 0;
        for py in 0..window.1 {
            for px in 0..window.0 {
                let (x0, x1) = reach(px, scale.0, layer.0);
                let (y0, y1) = reach(py, scale.1, layer.1);
                let touches = content.iter().any(|r| {
                    let (rx0, rx1) = (r[0].min(r[0] + r[2]), r[0].max(r[0] + r[2]));
                    let (ry0, ry1) = (r[1].min(r[1] + r[3]), r[1].max(r[1] + r[3]));
                    rx0 < rx1 && ry0 < ry1 && x0 < rx1 && x1 >= rx0 && y0 < ry1 && y1 >= ry0
                });
                if touches {
                    assert_eq!(hits[(py * window.0 + px) as usize], 1, "({px}, {py})");
                    drawn += 1;
                }
            }
        }
        let covered: u32 = scissors.iter().map(|r| r[2] * r[3]).sum();
        assert!(drawn > 0 && covered < window.0 * window.1 / 2, "{covered}");
        assert!(content_scissors(&[], layer, window).is_empty());
    }

    #[test]
    fn interface_scale_wgsl_parses_and_validates() {
        let module = naga::front::wgsl::parse_str(InterfaceScaler::SHADER_SRC)
            .unwrap_or_else(|e| panic!("{e:?}"));
        naga::valid::Validator::new(
            naga::valid::ValidationFlags::all(),
            naga::valid::Capabilities::empty(),
        )
        .validate(&module)
        .unwrap_or_else(|e| panic!("{e:?}"));
    }

    fn lanczos2(x: f32) -> f32 {
        let ax = x.abs();
        if ax < 1e-5 {
            return 1.0;
        }
        if ax >= 2.0 {
            return 0.0;
        }
        let px = std::f32::consts::PI * ax;
        2.0 * px.sin() * (px * 0.5).sin() / (px * px)
    }

    #[test]
    fn lanczos2_weights() {
        assert_eq!(lanczos2(0.0), 1.0);
        for x in [1.0f32, -1.0, 2.0, -2.0, 2.5] {
            assert!(lanczos2(x).abs() < 1e-6, "w({x}) = {}", lanczos2(x));
        }
        assert!(lanczos2(1.5) < 0.0);
        for f in [0.0f32, 0.1, 0.25, 0.3555, 0.5, 0.75, 0.99] {
            let w: Vec<f32> = (0..4).map(|k| lanczos2(k as f32 - 1.0 - f)).collect();
            let sum: f32 = w.iter().sum();
            assert!((sum - 1.0).abs() < 0.05, "raw sum {sum} at {f}");
            let n: f32 = w.iter().map(|w| w / sum).sum();
            assert!((n - 1.0).abs() < 1e-6);
        }
        let w: Vec<f32> = (0..4).map(|k| lanczos2(k as f32 - 1.0)).collect();
        assert!(w[1] == 1.0 && w[0].abs() < 1e-6 && w[2].abs() < 1e-6 && w[3].abs() < 1e-6);
        assert_eq!(InterfaceFilter::Lanczos2.mode(), 3);
        assert!(InterfaceScaler::SHADER_SRC.contains("params.mode == 3u"));
    }

    #[test]
    fn params_match_the_wgsl_layout() {
        assert_eq!(std::mem::size_of::<Params>(), 48);
        assert_eq!(std::mem::size_of::<FsrParams>(), 32);
    }

    #[test]
    fn fsr1_wgsl_parses_validates_and_keeps_amds_notice() {
        let module = naga::front::wgsl::parse_str(InterfaceScaler::FSR1_SHADER_SRC)
            .unwrap_or_else(|e| panic!("{e:?}"));
        naga::valid::Validator::new(
            naga::valid::ValidationFlags::all(),
            naga::valid::Capabilities::empty(),
        )
        .validate(&module)
        .unwrap_or_else(|e| panic!("{e:?}"));
        assert!(
            InterfaceScaler::FSR1_SHADER_SRC
                .contains("Copyright (c) 2021 Advanced Micro Devices, Inc. All rights reserved.")
        );
        assert!(InterfaceScaler::FSR1_SHADER_SRC.contains("Permission is hereby granted"));
    }

    #[test]
    fn fsr_sharpness_is_stops_clamped() {
        assert_eq!(FSR_DEFAULT_SHARPNESS, 0.5);
        set_fsr_sharpness(0.0);
        assert_eq!(fsr_rcas_scale(), 1.0);
        set_fsr_sharpness(1.0);
        assert_eq!(fsr_rcas_scale(), 0.5);
        set_fsr_sharpness(9.0);
        assert_eq!(fsr_rcas_scale(), 0.25);
        set_fsr_sharpness(f32::NAN);
        assert!((fsr_rcas_scale() - std::f32::consts::FRAC_1_SQRT_2).abs() < 1e-6);
        assert_eq!(InterfaceFilter::Fsr1.mode(), 3);
    }

    #[test]
    fn clip_dst_rect_handles_zero_and_overhanging_rects() {
        assert_eq!(clip_dst_rect(None, (0, 100)), None);
        assert_eq!(clip_dst_rect(None, (100, 0)), None);
        assert_eq!(clip_dst_rect(Some([0, 0, 0, 10]), (100, 100)), None);
        assert_eq!(clip_dst_rect(Some([0, 0, 10, -5]), (100, 100)), None);
        assert_eq!(clip_dst_rect(Some([-50, 0, 50, 20]), (100, 100)), None);
        assert_eq!(clip_dst_rect(Some([100, 0, 20, 20]), (100, 100)), None);
        assert_eq!(
            clip_dst_rect(Some([-10, -5, 30, 20]), (100, 100)),
            Some(ClippedDst {
                origin: (-10, -5),
                size: (30, 20),
                viewport: (0, 0, 20, 15),
            })
        );
        assert_eq!(
            clip_dst_rect(Some([90, 95, 30, 20]), (100, 100)),
            Some(ClippedDst {
                origin: (90, 95),
                size: (30, 20),
                viewport: (90, 95, 10, 5),
            })
        );
        assert_eq!(
            clip_dst_rect(None, (640, 480)),
            Some(ClippedDst {
                origin: (0, 0),
                size: (640, 480),
                viewport: (0, 0, 640, 480),
            })
        );
    }
}
