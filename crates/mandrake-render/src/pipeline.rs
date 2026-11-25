use crate::mesh::{OccluderRule, TerrainMesh};
use crate::uniforms::SceneUniforms;
use crate::vertex::{OccluderVertex, TerrainVertex};

#[derive(Clone, Debug)]
pub struct RenderPipelineConfig {
    pub width: u32,
    pub height: u32,
    pub target_format: wgpu::TextureFormat,
    pub depth_format: Option<wgpu::TextureFormat>,
}

impl Default for RenderPipelineConfig {
    fn default() -> Self {
        Self {
            width: 1280,
            height: 720,
            target_format: wgpu::TextureFormat::Rgba8UnormSrgb,
            depth_format: Some(wgpu::TextureFormat::Depth32Float),
        }
    }
}

pub struct LandscapeTexture {
    pub texture: wgpu::Texture,
    pub bind_group: wgpu::BindGroup,
}

impl LandscapeTexture {
    pub fn write_region(
        &self,
        queue: &wgpu::Queue,
        rect: crate::landscape::PixelRect,
        rgba: &[u8],
    ) {
        if rect.width == 0 || rect.height == 0 {
            return;
        }
        queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &self.texture,
                mip_level: 0,
                origin: wgpu::Origin3d {
                    x: rect.x,
                    y: rect.y,
                    z: 0,
                },
                aspect: wgpu::TextureAspect::All,
            },
            rgba,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(rect.width * 4),
                rows_per_image: Some(rect.height),
            },
            wgpu::Extent3d {
                width: rect.width,
                height: rect.height,
                depth_or_array_layers: 1,
            },
        );
    }
}

struct TerrainPipelines {
    terrain_pipeline: wgpu::RenderPipeline,
    occluder_pipeline: wgpu::RenderPipeline,
    tfl_occluder_pipeline: SharedOccluderPipeline,
    uniform_bind_group_layout: wgpu::BindGroupLayout,
    texture_bind_group_layout: wgpu::BindGroupLayout,
    default_texture_bind_group: wgpu::BindGroup,
    detail_pipeline: SharedDetailPipeline,
}

type SharedDetailPipeline = std::sync::Arc<std::sync::OnceLock<DetailPipeline>>;
type SharedOccluderPipeline = std::sync::Arc<std::sync::OnceLock<wgpu::RenderPipeline>>;

#[derive(Clone)]
pub struct DetailPipeline {
    pub pipeline: wgpu::RenderPipeline,
    pub texture_bind_group_layout: wgpu::BindGroupLayout,
}

pub struct TerrainRenderer {
    pub terrain_pipeline: wgpu::RenderPipeline,
    pub occluder_pipeline: wgpu::RenderPipeline,
    tfl_occluder_pipeline: SharedOccluderPipeline,
    pub uniform_buffer: wgpu::Buffer,
    pub uniform_bind_group: wgpu::BindGroup,
    pub uniform_bind_group_layout: wgpu::BindGroupLayout,
    pub texture_bind_group_layout: wgpu::BindGroupLayout,
    pub default_texture_bind_group: wgpu::BindGroup,
    pub config: RenderPipelineConfig,
    detail_pipeline: SharedDetailPipeline,
    pub detail_textures: Option<crate::detail::DetailTextures>,
}

impl TerrainRenderer {
    pub const TERRAIN_SHADER_SRC: &'static str = concat!(
        include_str!("../shaders/scene_uniforms.wgsl"),
        include_str!("../shaders/filter_helpers.wgsl"),
        include_str!("../shaders/terrain.wgsl"),
        include_str!("../shaders/fog.wgsl")
    );

    pub const OCCLUDER_SHADER_SRC: &'static str = concat!(
        include_str!("../shaders/scene_uniforms.wgsl"),
        include_str!("../shaders/occluder.wgsl")
    );

    pub fn new(device: &wgpu::Device, queue: &wgpu::Queue, config: &RenderPipelineConfig) -> Self {
        let uniform_bind_group_layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("Scene Uniform Bind Group Layout"),
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
                label: Some("Terrain Texture Bind Group Layout"),
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

        let terrain_pipeline_layout =
            device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("Terrain Scene Pipeline Layout"),
                bind_group_layouts: &[
                    Some(&uniform_bind_group_layout),
                    Some(&texture_bind_group_layout),
                ],
                immediate_size: 0,
            });

        let occluder_pipeline_layout =
            device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("Occluder Pipeline Layout"),
                bind_group_layouts: &[Some(&uniform_bind_group_layout)],
                immediate_size: 0,
            });

        let terrain_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Terrain WGSL Shader"),
            source: wgpu::ShaderSource::Wgsl(Self::TERRAIN_SHADER_SRC.into()),
        });

        let depth_stencil = config.depth_format.map(|format| wgpu::DepthStencilState {
            format,
            depth_write_enabled: Some(true),
            depth_compare: Some(wgpu::CompareFunction::Less),
            stencil: wgpu::StencilState::default(),
            bias: wgpu::DepthBiasState::default(),
        });

        let terrain_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("Terrain Render Pipeline"),
            layout: Some(&terrain_pipeline_layout),
            vertex: wgpu::VertexState {
                module: &terrain_shader,
                entry_point: Some("vs_main"),
                buffers: &[Some(TerrainVertex::desc())],
                compilation_options: Default::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &terrain_shader,
                entry_point: Some("fs_main"),
                targets: &[Some(wgpu::ColorTargetState {
                    format: config.target_format,
                    blend: Some(wgpu::BlendState::REPLACE),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: wgpu::PipelineCompilationOptions {
                    constants: crate::scene_constants(config.target_format),
                    ..Default::default()
                },
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                strip_index_format: None,
                front_face: wgpu::FrontFace::Ccw,
                cull_mode: Some(wgpu::Face::Back),
                unclipped_depth: false,
                polygon_mode: wgpu::PolygonMode::Fill,
                conservative: false,
            },
            depth_stencil: depth_stencil.clone(),
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        });

        let occluder_pipeline = Self::occluder_pipeline(
            device,
            config,
            &occluder_pipeline_layout,
            "Sprite Occluder Depth Pipeline",
            "vs_main",
        );

        let default_texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("Default 1x1 White Texture"),
            size: wgpu::Extent3d {
                width: 1,
                height: 1,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8UnormSrgb,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &default_texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            &[255, 255, 255, 255],
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(4),
                rows_per_image: Some(1),
            },
            wgpu::Extent3d {
                width: 1,
                height: 1,
                depth_or_array_layers: 1,
            },
        );
        let default_view = default_texture.create_view(&wgpu::TextureViewDescriptor::default());
        let default_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("Default Sampler"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        let default_texture_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Default Texture Bind Group"),
            layout: &texture_bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&default_view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&default_sampler),
                },
            ],
        });

        Self::with_pipelines(
            device,
            queue,
            config,
            TerrainPipelines {
                terrain_pipeline,
                occluder_pipeline,
                tfl_occluder_pipeline: SharedOccluderPipeline::default(),
                uniform_bind_group_layout,
                texture_bind_group_layout,
                default_texture_bind_group,
                detail_pipeline: SharedDetailPipeline::default(),
            },
        )
    }

    fn occluder_pipeline(
        device: &wgpu::Device,
        config: &RenderPipelineConfig,
        layout: &wgpu::PipelineLayout,
        label: &str,
        vertex_entry: &str,
    ) -> wgpu::RenderPipeline {
        let occluder_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Occluder WGSL Shader"),
            source: wgpu::ShaderSource::Wgsl(Self::OCCLUDER_SHADER_SRC.into()),
        });
        let occluder_depth_stencil = config.depth_format.map(|format| wgpu::DepthStencilState {
            format,
            depth_write_enabled: Some(true),
            depth_compare: Some(wgpu::CompareFunction::LessEqual),
            stencil: wgpu::StencilState::default(),
            bias: wgpu::DepthBiasState::default(),
        });
        device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some(label),
            layout: Some(layout),
            vertex: wgpu::VertexState {
                module: &occluder_shader,
                entry_point: Some(vertex_entry),
                buffers: &[Some(OccluderVertex::desc())],
                compilation_options: Default::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &occluder_shader,
                entry_point: Some("fs_main"),
                targets: &[Some(wgpu::ColorTargetState {
                    format: config.target_format,
                    blend: None,
                    write_mask: wgpu::ColorWrites::empty(),
                })],
                compilation_options: Default::default(),
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                strip_index_format: None,
                front_face: wgpu::FrontFace::Ccw,
                cull_mode: Some(wgpu::Face::Back),
                unclipped_depth: false,
                polygon_mode: wgpu::PolygonMode::Fill,
                conservative: false,
            },
            depth_stencil: occluder_depth_stencil,
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        })
    }

    pub fn enable_tfl_buckets(&mut self, device: &wgpu::Device) {
        if self.tfl_occluder_pipeline.get().is_some() {
            return;
        }
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("TFL Occluder Pipeline Layout"),
            bind_group_layouts: &[Some(&self.uniform_bind_group_layout)],
            immediate_size: 0,
        });
        let pipeline = Self::occluder_pipeline(
            device,
            &self.config,
            &layout,
            "TFL Bucket Occluder Pipeline",
            "vs_tfl_bucket",
        );
        let _ = self.tfl_occluder_pipeline.set(pipeline);
    }

    pub fn enable_detail(&mut self, device: &wgpu::Device) {
        if self.detail_pipeline.get().is_some() {
            return;
        }
        let texture_bind_group_layout = crate::detail::DetailTextures::bind_group_layout(device);
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("Terrain Detail Pipeline Layout"),
            bind_group_layouts: &[
                Some(&self.uniform_bind_group_layout),
                Some(&self.texture_bind_group_layout),
                Some(&texture_bind_group_layout),
            ],
            immediate_size: 0,
        });
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Terrain Detail WGSL Shader"),
            source: wgpu::ShaderSource::Wgsl(Self::TERRAIN_SHADER_SRC.into()),
        });
        let depth_stencil = self
            .config
            .depth_format
            .map(|format| wgpu::DepthStencilState {
                format,
                depth_write_enabled: Some(true),
                depth_compare: Some(wgpu::CompareFunction::Less),
                stencil: wgpu::StencilState::default(),
                bias: wgpu::DepthBiasState::default(),
            });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("Terrain Detail Render Pipeline"),
            layout: Some(&layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_detail"),
                buffers: &[
                    Some(TerrainVertex::desc()),
                    Some(crate::detail::DetailVertex::desc()),
                ],
                compilation_options: Default::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_detail"),
                targets: &[Some(wgpu::ColorTargetState {
                    format: self.config.target_format,
                    blend: Some(wgpu::BlendState::REPLACE),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: wgpu::PipelineCompilationOptions {
                    constants: crate::scene_constants(self.config.target_format),
                    ..Default::default()
                },
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                strip_index_format: None,
                front_face: wgpu::FrontFace::Ccw,
                cull_mode: Some(wgpu::Face::Back),
                unclipped_depth: false,
                polygon_mode: wgpu::PolygonMode::Fill,
                conservative: false,
            },
            depth_stencil,
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        });
        let _ = self.detail_pipeline.set(DetailPipeline {
            pipeline,
            texture_bind_group_layout,
        });
    }

    pub fn set_detail(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        detail: Option<&crate::detail::LandscapeDetail>,
    ) {
        self.detail_textures = detail.map(|d| {
            self.enable_detail(device);
            let layout = &self
                .detail_pipeline
                .get()
                .expect("enabled")
                .texture_bind_group_layout;
            crate::detail::DetailTextures::new(device, queue, layout, d)
        });
    }

    pub fn sharing_pipelines(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        config: &RenderPipelineConfig,
    ) -> Self {
        debug_assert_eq!(
            (config.target_format, config.depth_format),
            (self.config.target_format, self.config.depth_format)
        );
        Self::with_pipelines(
            device,
            queue,
            config,
            TerrainPipelines {
                terrain_pipeline: self.terrain_pipeline.clone(),
                occluder_pipeline: self.occluder_pipeline.clone(),
                tfl_occluder_pipeline: self.tfl_occluder_pipeline.clone(),
                uniform_bind_group_layout: self.uniform_bind_group_layout.clone(),
                texture_bind_group_layout: self.texture_bind_group_layout.clone(),
                default_texture_bind_group: self.default_texture_bind_group.clone(),
                detail_pipeline: self.detail_pipeline.clone(),
            },
        )
    }

    fn with_pipelines(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        config: &RenderPipelineConfig,
        pipelines: TerrainPipelines,
    ) -> Self {
        let TerrainPipelines {
            terrain_pipeline,
            occluder_pipeline,
            tfl_occluder_pipeline,
            uniform_bind_group_layout,
            texture_bind_group_layout,
            default_texture_bind_group,
            detail_pipeline,
        } = pipelines;
        let default_uniforms = SceneUniforms::default();
        let uniform_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Scene Uniform Buffer"),
            size: core::mem::size_of::<SceneUniforms>() as wgpu::BufferAddress,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let uniform_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Scene Uniform Bind Group"),
            layout: &uniform_bind_group_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: uniform_buffer.as_entire_binding(),
            }],
        });

        let renderer = Self {
            terrain_pipeline,
            occluder_pipeline,
            tfl_occluder_pipeline,
            uniform_buffer,
            uniform_bind_group,
            uniform_bind_group_layout,
            texture_bind_group_layout,
            default_texture_bind_group,
            config: config.clone(),
            detail_pipeline,
            detail_textures: None,
        };

        queue.write_buffer(
            &renderer.uniform_buffer,
            0,
            bytemuck::bytes_of(&default_uniforms),
        );

        renderer
    }

    pub fn create_texture_bind_group(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        width: u32,
        height: u32,
        rgba: &[u8],
    ) -> wgpu::BindGroup {
        self.create_landscape_texture(device, queue, width, height, rgba)
            .bind_group
    }

    pub fn create_landscape_texture(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        width: u32,
        height: u32,
        rgba: &[u8],
    ) -> LandscapeTexture {
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("Landscape Texture"),
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
        let landscape = LandscapeTexture {
            bind_group: device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("Landscape Bind Group"),
                layout: &self.texture_bind_group_layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: wgpu::BindingResource::TextureView(
                            &texture.create_view(&wgpu::TextureViewDescriptor::default()),
                        ),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::Sampler(&device.create_sampler(
                            &wgpu::SamplerDescriptor {
                                label: Some("Landscape Linear Sampler"),
                                address_mode_u: wgpu::AddressMode::ClampToEdge,
                                address_mode_v: wgpu::AddressMode::ClampToEdge,
                                mag_filter: wgpu::FilterMode::Linear,
                                min_filter: wgpu::FilterMode::Linear,
                                mipmap_filter: wgpu::MipmapFilterMode::Linear,
                                anisotropy_clamp: crate::model::MODEL_TEXTURE_ANISOTROPY,
                                ..Default::default()
                            },
                        )),
                    },
                ],
            }),
            texture,
        };
        landscape.write_region(
            queue,
            crate::landscape::PixelRect {
                x: 0,
                y: 0,
                width,
                height,
            },
            rgba,
        );
        landscape
    }

    pub fn update_uniforms(&self, queue: &wgpu::Queue, uniforms: &SceneUniforms) {
        queue.write_buffer(&self.uniform_buffer, 0, bytemuck::bytes_of(uniforms));
    }

    pub fn render_terrain<'a>(
        &'a self,
        render_pass: &mut wgpu::RenderPass<'a>,
        mesh: &'a TerrainMesh,
        texture_bind_group: Option<&'a wgpu::BindGroup>,
    ) {
        self.render_terrain_indices(render_pass, mesh, texture_bind_group, 0..mesh.num_indices);
    }

    pub fn render_terrain_media<'a>(
        &'a self,
        render_pass: &mut wgpu::RenderPass<'a>,
        mesh: &'a TerrainMesh,
        texture_bind_group: Option<&'a wgpu::BindGroup>,
    ) {
        self.render_terrain_indices(
            render_pass,
            mesh,
            texture_bind_group,
            0..mesh.num_media_indices,
        );
    }

    pub fn render_terrain_dry<'a>(
        &'a self,
        render_pass: &mut wgpu::RenderPass<'a>,
        mesh: &'a TerrainMesh,
        texture_bind_group: Option<&'a wgpu::BindGroup>,
    ) {
        self.render_terrain_indices(
            render_pass,
            mesh,
            texture_bind_group,
            mesh.num_media_indices..mesh.num_indices,
        );
    }

    pub fn render_terrain_dry_with<'a>(
        &'a self,
        render_pass: &mut wgpu::RenderPass<'a>,
        mesh: &'a TerrainMesh,
        texture_bind_group: Option<&'a wgpu::BindGroup>,
        vertex_buffer: &'a wgpu::Buffer,
    ) {
        self.render_terrain_indices_from(
            render_pass,
            mesh,
            texture_bind_group,
            mesh.num_media_indices..mesh.num_indices,
            vertex_buffer,
        );
    }

    fn render_terrain_indices<'a>(
        &'a self,
        render_pass: &mut wgpu::RenderPass<'a>,
        mesh: &'a TerrainMesh,
        texture_bind_group: Option<&'a wgpu::BindGroup>,
        indices: core::ops::Range<u32>,
    ) {
        self.render_terrain_indices_from(
            render_pass,
            mesh,
            texture_bind_group,
            indices,
            &mesh.vertex_buffer,
        );
    }

    pub fn render_terrain_indexed<'a>(
        &'a self,
        render_pass: &mut wgpu::RenderPass<'a>,
        texture_bind_group: Option<&'a wgpu::BindGroup>,
        vertex_buffer: &'a wgpu::Buffer,
        index_buffer: &'a wgpu::Buffer,
        indices: core::ops::Range<u32>,
    ) {
        if indices.is_empty() {
            return;
        }
        render_pass.set_pipeline(&self.terrain_pipeline);
        render_pass.set_bind_group(0, &self.uniform_bind_group, &[]);
        let tex_bg = texture_bind_group.unwrap_or(&self.default_texture_bind_group);
        render_pass.set_bind_group(1, tex_bg, &[]);
        render_pass.set_vertex_buffer(0, vertex_buffer.slice(..));
        render_pass.set_index_buffer(index_buffer.slice(..), wgpu::IndexFormat::Uint32);
        render_pass.draw_indexed(indices, 0, 0..1);
    }

    fn render_terrain_indices_from<'a>(
        &'a self,
        render_pass: &mut wgpu::RenderPass<'a>,
        mesh: &'a TerrainMesh,
        texture_bind_group: Option<&'a wgpu::BindGroup>,
        indices: core::ops::Range<u32>,
        vertex_buffer: &'a wgpu::Buffer,
    ) {
        if indices.is_empty() {
            return;
        }
        let detail = match (
            self.detail_pipeline.get(),
            &self.detail_textures,
            &mesh.detail_buffer,
        ) {
            (Some(p), Some(t), Some(b)) => Some((p, t, b)),
            _ => None,
        };
        match detail {
            Some((pipeline, textures, buffer)) => {
                render_pass.set_pipeline(&pipeline.pipeline);
                render_pass.set_bind_group(2, &textures.bind_group, &[]);
                render_pass.set_vertex_buffer(1, buffer.slice(..));
            }
            None => render_pass.set_pipeline(&self.terrain_pipeline),
        }
        render_pass.set_bind_group(0, &self.uniform_bind_group, &[]);
        let tex_bg = texture_bind_group.unwrap_or(&self.default_texture_bind_group);
        render_pass.set_bind_group(1, tex_bg, &[]);
        render_pass.set_vertex_buffer(0, vertex_buffer.slice(..));
        render_pass.set_index_buffer(mesh.index_buffer.slice(..), wgpu::IndexFormat::Uint32);
        render_pass.draw_indexed(indices, 0, 0..1);
    }

    pub fn render_terrain_occluders<'a>(
        &'a self,
        render_pass: &mut wgpu::RenderPass<'a>,
        mesh: &'a TerrainMesh,
    ) {
        if mesh.num_occluder_vertices == 0 {
            return;
        }
        let pipeline = match (mesh.occluder_rule, self.tfl_occluder_pipeline.get()) {
            (OccluderRule::TflBuckets, Some(tfl)) => tfl,
            _ => &self.occluder_pipeline,
        };
        render_pass.set_pipeline(pipeline);
        render_pass.set_bind_group(0, &self.uniform_bind_group, &[]);
        render_pass.set_vertex_buffer(0, mesh.occluder_buffer.slice(..));
        render_pass.draw(0..mesh.num_occluder_vertices, 0..1);
    }
}
