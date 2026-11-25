use super::*;

pub(super) const SURFACE_DEPTH_BIAS: i32 = 64;

pub(super) const BED_DEPTH_BIAS: i32 = 16;

pub(super) struct WaterPipelines {
    pub(super) bed_media: wgpu::RenderPipeline,
    pub(super) bed_dry: wgpu::RenderPipeline,
    pub(super) keep: wgpu::RenderPipeline,
    pub(super) surface_under: wgpu::RenderPipeline,
    pub(super) surface: wgpu::RenderPipeline,
    pub(super) bed_under: wgpu::RenderPipeline,
    pub(super) surface_depth: wgpu::RenderPipeline,
    pub(super) wet: Option<wgpu::RenderPipeline>,
    pub(super) empty_group: wgpu::BindGroup,
}

impl WaterPipelines {
    pub(super) fn new(
        device: &wgpu::Device,
        terrain: &TerrainRenderer,
        layout: &wgpu::BindGroupLayout,
        settings: &WaterSettings,
        display_format: wgpu::TextureFormat,
    ) -> Self {
        let empty = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("Water Empty Group"),
            entries: &[],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("Water Pipeline Layout"),
            bind_group_layouts: &[
                Some(&terrain.uniform_bind_group_layout),
                Some(&terrain.texture_bind_group_layout),
                Some(&empty),
                Some(layout),
            ],
            immediate_size: 0,
        });
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Water WGSL Shader"),
            source: wgpu::ShaderSource::Wgsl(WaterShading::SHADER_SRC.into()),
        });
        let pipeline = |label: &str,
                        format: wgpu::TextureFormat,
                        vertex: &str,
                        fragment: Option<&str>,
                        blend: wgpu::BlendState,
                        write: bool,
                        compare: wgpu::CompareFunction| {
            let mut constants = crate::scene_constants(format).to_vec();
            if crate::is_ten_bit(display_format) {
                constants.push(("WATER_DITHER_LEVELS", 1023.0));
            }
            let targets = [Some(wgpu::ColorTargetState {
                format,
                blend: Some(blend),
                write_mask: if fragment.is_some() {
                    wgpu::ColorWrites::ALL
                } else {
                    wgpu::ColorWrites::empty()
                },
            })];
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(label),
                layout: Some(&pipeline_layout),
                vertex: wgpu::VertexState {
                    module: &shader,
                    entry_point: Some(vertex),
                    buffers: &[Some(TerrainVertex::desc())],
                    compilation_options: Default::default(),
                },
                fragment: Some(wgpu::FragmentState {
                    module: &shader,
                    entry_point: Some(fragment.unwrap_or("fs_depth_only")),
                    targets: &targets,
                    compilation_options: wgpu::PipelineCompilationOptions {
                        constants: &constants,
                        ..Default::default()
                    },
                }),
                primitive: wgpu::PrimitiveState {
                    topology: wgpu::PrimitiveTopology::TriangleList,
                    front_face: wgpu::FrontFace::Ccw,
                    cull_mode: Some(wgpu::Face::Back),
                    ..Default::default()
                },
                depth_stencil: terrain
                    .config
                    .depth_format
                    .map(|format| wgpu::DepthStencilState {
                        format,
                        depth_write_enabled: Some(write),
                        depth_compare: Some(compare),
                        stencil: wgpu::StencilState::default(),
                        bias: if vertex == "vs_surface" {
                            wgpu::DepthBiasState {
                                constant: SURFACE_DEPTH_BIAS,
                                slope_scale: 2.0,
                                clamp: 0.0,
                            }
                        } else {
                            wgpu::DepthBiasState {
                                constant: -BED_DEPTH_BIAS,
                                slope_scale: 0.0,
                                clamp: 0.0,
                            }
                        },
                    }),
                multisample: wgpu::MultisampleState::default(),
                multiview_mask: None,
                cache: None,
            })
        };
        let under = wgpu::BlendComponent {
            src_factor: wgpu::BlendFactor::OneMinusDstAlpha,
            dst_factor: wgpu::BlendFactor::One,
            operation: wgpu::BlendOperation::Add,
        };
        let under = wgpu::BlendState {
            color: under,
            alpha: under,
        };
        let scale = wgpu::BlendComponent {
            src_factor: wgpu::BlendFactor::Zero,
            dst_factor: wgpu::BlendFactor::Src,
            operation: wgpu::BlendOperation::Add,
        };
        let scale = wgpu::BlendState {
            color: scale,
            alpha: scale,
        };
        use wgpu::CompareFunction::{Less, LessEqual};
        let bed_media_pipeline = pipeline(
            "Water Bed Media Pipeline",
            terrain.config.target_format,
            "vs_bed",
            Some("fs_bed_media"),
            wgpu::BlendState::REPLACE,
            true,
            Less,
        );
        let bed_dry_pipeline = pipeline(
            "Water Bed Dry Pipeline",
            terrain.config.target_format,
            "vs_bed",
            Some("fs_bed_dry"),
            wgpu::BlendState::REPLACE,
            true,
            Less,
        );
        let keep_pipeline = pipeline(
            "Water Reflection Keep Pipeline",
            display_format,
            "vs_surface",
            Some("fs_reflection_keep"),
            scale,
            false,
            Less,
        );
        let surface_under_pipeline = pipeline(
            "Water Surface Under Pipeline",
            display_format,
            "vs_surface",
            Some("fs_surface_under"),
            under,
            false,
            Less,
        );
        let surface_pipeline = pipeline(
            "Water Surface Pipeline",
            display_format,
            "vs_surface",
            Some("fs_surface"),
            wgpu::BlendState {
                color: wgpu::BlendComponent {
                    src_factor: wgpu::BlendFactor::One,
                    dst_factor: wgpu::BlendFactor::One,
                    operation: wgpu::BlendOperation::Add,
                },
                alpha: wgpu::BlendComponent {
                    src_factor: wgpu::BlendFactor::One,
                    dst_factor: wgpu::BlendFactor::One,
                    operation: wgpu::BlendOperation::Add,
                },
            },
            false,
            Less,
        );
        let bed_under_pipeline = pipeline(
            "Water Bed Under Pipeline",
            display_format,
            "vs_bed",
            Some("fs_bed_under"),
            under,
            false,
            LessEqual,
        );
        let surface_depth_pipeline = pipeline(
            "Water Surface Depth Pipeline",
            terrain.config.target_format,
            "vs_surface",
            None,
            wgpu::BlendState::REPLACE,
            true,
            Less,
        );
        let wet_pipeline = (settings.wet_sand || settings.physical).then(|| {
            pipeline(
                "Water Wet Sand Pipeline",
                terrain.config.target_format,
                "vs_bed",
                Some("fs_wet"),
                wgpu::BlendState {
                    color: wgpu::BlendComponent {
                        src_factor: wgpu::BlendFactor::One,
                        dst_factor: wgpu::BlendFactor::SrcAlpha,
                        operation: wgpu::BlendOperation::Add,
                    },
                    alpha: wgpu::BlendComponent {
                        src_factor: wgpu::BlendFactor::Zero,
                        dst_factor: wgpu::BlendFactor::One,
                        operation: wgpu::BlendOperation::Add,
                    },
                },
                false,
                LessEqual,
            )
        });
        let empty_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Water Empty Bind Group"),
            layout: &empty,
            entries: &[],
        });
        Self {
            bed_media: bed_media_pipeline,
            bed_dry: bed_dry_pipeline,
            keep: keep_pipeline,
            surface_under: surface_under_pipeline,
            surface: surface_pipeline,
            bed_under: bed_under_pipeline,
            surface_depth: surface_depth_pipeline,
            wet: wet_pipeline,
            empty_group,
        }
    }
}
