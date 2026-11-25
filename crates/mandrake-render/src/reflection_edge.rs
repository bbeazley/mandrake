use crate::mesh::TerrainMesh;
use crate::pipeline::RenderPipelineConfig;
use crate::vertex::TerrainVertex;

const MASK_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::R8Unorm;

pub struct ReflectionEdge {
    pipeline: wgpu::RenderPipeline,
    layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
    view: wgpu::TextureView,
    bind_group: wgpu::BindGroup,
    size: (u32, u32),
}

impl ReflectionEdge {
    pub const SHADER_SRC: &'static str = concat!(
        include_str!("../shaders/scene_uniforms.wgsl"),
        include_str!("../shaders/reflection_edge.wgsl")
    );

    pub fn bind_group_layout(device: &wgpu::Device) -> wgpu::BindGroupLayout {
        device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("Reflection Edge Bind Group Layout"),
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
        })
    }

    pub fn new(device: &wgpu::Device, config: &RenderPipelineConfig) -> Self {
        let layout = Self::bind_group_layout(device);
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("Reflection Edge Sampler"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        let size = (config.width.max(1), config.height.max(1));
        let (view, bind_group) = Self::target(device, &layout, &sampler, size.0, size.1);
        let scene_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("Reflection Edge Scene Layout"),
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
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("Reflection Edge Pipeline Layout"),
            bind_group_layouts: &[Some(&scene_layout)],
            immediate_size: 0,
        });
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Reflection Edge WGSL Shader"),
            source: wgpu::ShaderSource::Wgsl(Self::SHADER_SRC.into()),
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("Reflection Edge Pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                buffers: &[Some(TerrainVertex::desc())],
                compilation_options: Default::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_main"),
                targets: &[Some(wgpu::ColorTargetState {
                    format: MASK_FORMAT,
                    blend: None,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: Default::default(),
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                cull_mode: Some(wgpu::Face::Back),
                ..Default::default()
            },
            depth_stencil: config.depth_format.map(|format| wgpu::DepthStencilState {
                format,
                depth_write_enabled: Some(false),
                depth_compare: Some(wgpu::CompareFunction::Less),
                stencil: wgpu::StencilState::default(),
                bias: wgpu::DepthBiasState::default(),
            }),
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        });
        Self {
            pipeline,
            layout,
            sampler,
            view,
            bind_group,
            size,
        }
    }

    pub fn sharing_pipelines(&self, device: &wgpu::Device, config: &RenderPipelineConfig) -> Self {
        let size = (config.width.max(1), config.height.max(1));
        let (view, bind_group) = Self::target(device, &self.layout, &self.sampler, size.0, size.1);
        Self {
            pipeline: self.pipeline.clone(),
            layout: self.layout.clone(),
            sampler: self.sampler.clone(),
            view,
            bind_group,
            size,
        }
    }

    fn target(
        device: &wgpu::Device,
        layout: &wgpu::BindGroupLayout,
        sampler: &wgpu::Sampler,
        width: u32,
        height: u32,
    ) -> (wgpu::TextureView, wgpu::BindGroup) {
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("Reflection Edge Mask"),
            size: wgpu::Extent3d {
                width: width.max(1),
                height: height.max(1),
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: MASK_FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Reflection Edge Bind Group"),
            layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(sampler),
                },
            ],
        });
        (view, bind_group)
    }

    pub fn resize(&mut self, device: &wgpu::Device, width: u32, height: u32) {
        let size = (width.max(1), height.max(1));
        if self.size == size {
            return;
        }
        (self.view, self.bind_group) =
            Self::target(device, &self.layout, &self.sampler, size.0, size.1);
        self.size = size;
    }

    pub fn bind_group(&self) -> &wgpu::BindGroup {
        &self.bind_group
    }

    pub fn encode_mask(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        scene_bind_group: &wgpu::BindGroup,
        mesh: &TerrainMesh,
        depth: &wgpu::TextureView,
    ) {
        self.encode_mask_on(encoder, scene_bind_group, mesh, depth, wgpu::LoadOp::Load);
    }

    pub fn encode_mask_depth_cleared(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        scene_bind_group: &wgpu::BindGroup,
        mesh: &TerrainMesh,
        depth: &wgpu::TextureView,
    ) {
        self.encode_mask_on(
            encoder,
            scene_bind_group,
            mesh,
            depth,
            wgpu::LoadOp::Clear(1.0),
        );
    }

    fn encode_mask_on(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        scene_bind_group: &wgpu::BindGroup,
        mesh: &TerrainMesh,
        depth: &wgpu::TextureView,
        depth_load: wgpu::LoadOp<f32>,
    ) {
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("Reflection Edge Mask Pass"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                depth_slice: None,
                view: &self.view,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                view: depth,
                depth_ops: Some(wgpu::Operations {
                    load: depth_load,
                    store: wgpu::StoreOp::Store,
                }),
                stencil_ops: None,
            }),
            occlusion_query_set: None,
            timestamp_writes: None,
            multiview_mask: None,
        });
        let dry = mesh.num_media_indices..mesh.num_indices;
        if dry.is_empty() {
            return;
        }
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, scene_bind_group, &[]);
        pass.set_vertex_buffer(0, mesh.vertex_buffer.slice(..));
        pass.set_index_buffer(mesh.index_buffer.slice(..), wgpu::IndexFormat::Uint32);
        pass.draw_indexed(dry, 0, 0..1);
    }
}
