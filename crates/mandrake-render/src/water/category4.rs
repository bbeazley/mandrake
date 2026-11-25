pub(super) struct WaterCategory4 {
    pub(super) reflection: wgpu::RenderPipeline,
    pub(super) soft_reflection: wgpu::RenderPipeline,
    pub(super) submerged: wgpu::RenderPipeline,
}

pub(super) const SUBMERGED_HEIGHT_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rg16Float;

impl WaterCategory4 {
    pub(super) const SHADER_SRC: &'static str = concat!(
        include_str!("../../shaders/scene_uniforms.wgsl"),
        include_str!("../../shaders/filter_helpers.wgsl"),
        include_str!("../../shaders/billboard.wgsl"),
        include_str!("../../shaders/reflection_edge_fade.wgsl"),
        include_str!("../../shaders/fog.wgsl"),
        include_str!("../../shaders/water_billboard.wgsl")
    );

    pub(super) fn new(
        device: &wgpu::Device,
        billboards: &crate::billboard::BillboardRenderer,
        depth_format: Option<wgpu::TextureFormat>,
    ) -> Self {
        let format = billboards.interface_format;
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Water Category 4 WGSL"),
            source: wgpu::ShaderSource::Wgsl(Self::SHADER_SRC.into()),
        });
        let plain = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("Water Category 4 Layout"),
            bind_group_layouts: &[
                Some(&billboards.uniform_bind_group_layout),
                Some(&billboards.texture_bind_group_layout),
            ],
            immediate_size: 0,
        });
        let edge = crate::reflection_edge::ReflectionEdge::bind_group_layout(device);
        let soft = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("Water Category 4 Soft Layout"),
            bind_group_layouts: &[
                Some(&billboards.uniform_bind_group_layout),
                Some(&billboards.texture_bind_group_layout),
                Some(&edge),
            ],
            immediate_size: 0,
        });
        let srgb = if format.is_srgb() { 1.0 } else { 0.0 };
        let constants = [("WATER_SRGB", srgb)];
        let submerged_constants = [("WATER_SRGB", srgb), ("SPRITE_HEIGHT_FOG_AFTER", 1.0)];
        let color = |format| {
            Some(wgpu::ColorTargetState {
                format,
                blend: Some(wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING),
                write_mask: wgpu::ColorWrites::ALL,
            })
        };
        let pipeline = |label: &str,
                        layout: &wgpu::PipelineLayout,
                        vertex_entry: &str,
                        entry: &str,
                        targets: &[Option<wgpu::ColorTargetState>],
                        compare: wgpu::CompareFunction,
                        constants: &[(&str, f64)]| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(label),
                layout: Some(layout),
                vertex: wgpu::VertexState {
                    module: &shader,
                    entry_point: Some(vertex_entry),
                    buffers: &[Some(
                        <crate::billboard::BillboardInstance as crate::vertex::VertexLayout>::desc(
                        ),
                    )],
                    compilation_options: Default::default(),
                },
                fragment: Some(wgpu::FragmentState {
                    module: &shader,
                    entry_point: Some(entry),
                    targets,
                    compilation_options: wgpu::PipelineCompilationOptions {
                        constants,
                        ..Default::default()
                    },
                }),
                primitive: wgpu::PrimitiveState::default(),
                depth_stencil: depth_format.map(|format| wgpu::DepthStencilState {
                    format,
                    depth_write_enabled: Some(false),
                    depth_compare: Some(compare),
                    stencil: wgpu::StencilState::default(),
                    bias: wgpu::DepthBiasState::default(),
                }),
                multisample: wgpu::MultisampleState::default(),
                multiview_mask: None,
                cache: None,
            })
        };
        let reflection = pipeline(
            "Water Reflections Pipeline",
            &plain,
            "vs_water_reflection",
            "fs_water_reflection",
            &[color(format)],
            wgpu::CompareFunction::Always,
            &constants,
        );
        let soft_reflection = pipeline(
            "Water Soft Reflections Pipeline",
            &soft,
            "vs_water_reflection",
            "fs_water_reflection_soft",
            &[color(format)],
            wgpu::CompareFunction::Always,
            &constants,
        );
        let submerged = pipeline(
            "Water Submerged Pipeline",
            &plain,
            "vs_water_submerged",
            "fs_water_submerged",
            &[color(format), color(SUBMERGED_HEIGHT_FORMAT)],
            wgpu::CompareFunction::LessEqual,
            &submerged_constants,
        );
        Self {
            reflection,
            soft_reflection,
            submerged,
        }
    }
}
