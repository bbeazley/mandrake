/// What group 3 binds ([`WaterBindings::groups`]).
pub(super) struct WaterBindings {
    pub(super) layout: wgpu::BindGroupLayout,
    pub(super) uniform: wgpu::Buffer,
    pub(super) floor: wgpu::TextureView,
    pub(super) map_colors: wgpu::TextureView,
    pub(super) cell_light: wgpu::TextureView,
    /// The ripple field's two states.
    pub(super) ripple: [wgpu::TextureView; 2],
    pub(super) fft: wgpu::TextureView,
    pub(super) sampler: wgpu::Sampler,
    pub(super) fetch: wgpu::TextureView,
    pub(super) layer: wgpu::TextureView,
    /// The submerged layer: colour and height.
    pub(super) submerged: wgpu::TextureView,
    pub(super) submerged_height: wgpu::TextureView,
}

impl WaterBindings {
    /// One bind group per ripple state.
    pub(super) fn groups(&self, device: &wgpu::Device) -> [wgpu::BindGroup; 2] {
        let view = wgpu::BindingResource::TextureView;
        [0, 1].map(|state| {
            device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("Water Bind Group"),
                layout: &self.layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: self.uniform.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: view(&self.floor),
                    },
                    wgpu::BindGroupEntry {
                        binding: 2,
                        resource: view(&self.map_colors),
                    },
                    wgpu::BindGroupEntry {
                        binding: 3,
                        resource: view(&self.cell_light),
                    },
                    wgpu::BindGroupEntry {
                        binding: 4,
                        resource: view(&self.ripple[state]),
                    },
                    wgpu::BindGroupEntry {
                        binding: 5,
                        resource: view(&self.fft),
                    },
                    wgpu::BindGroupEntry {
                        binding: 6,
                        resource: wgpu::BindingResource::Sampler(&self.sampler),
                    },
                    wgpu::BindGroupEntry {
                        binding: 7,
                        resource: view(&self.fetch),
                    },
                    wgpu::BindGroupEntry {
                        binding: 8,
                        resource: view(&self.layer),
                    },
                    wgpu::BindGroupEntry {
                        binding: 9,
                        resource: view(&self.submerged),
                    },
                    wgpu::BindGroupEntry {
                        binding: 10,
                        resource: view(&self.submerged_height),
                    },
                ],
            })
        })
    }
}

/// The reflection layer: display values (the format the reflections are drawn in), drawn and
/// sampled.
pub(super) fn reflection_layer_texture(
    device: &wgpu::Device,
    format: wgpu::TextureFormat,
    (width, height): (u32, u32),
) -> wgpu::Texture {
    device.create_texture(&wgpu::TextureDescriptor {
        label: Some("Water Layer"),
        size: wgpu::Extent3d {
            width: width.max(1),
            height: height.max(1),
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
        view_formats: &[],
    })
}

/// The water's bind group (group 3 of `water.wgsl`): the uniforms and the textures [`WaterBindings`]
/// binds, in its order.
pub(super) fn layout(device: &wgpu::Device) -> wgpu::BindGroupLayout {
    let vertex_fragment = wgpu::ShaderStages::VERTEX | wgpu::ShaderStages::FRAGMENT;
    let fragment = wgpu::ShaderStages::FRAGMENT;
    device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("Water Bind Group Layout"),
        entries: &[
            wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: vertex_fragment,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            },
            wgpu::BindGroupLayoutEntry {
                binding: 1,
                visibility: vertex_fragment,
                ty: wgpu::BindingType::Texture {
                    sample_type: wgpu::TextureSampleType::Float { filterable: false },
                    view_dimension: wgpu::TextureViewDimension::D2,
                    multisampled: false,
                },
                count: None,
            },
            wgpu::BindGroupLayoutEntry {
                binding: 2,
                visibility: fragment,
                ty: wgpu::BindingType::Texture {
                    sample_type: wgpu::TextureSampleType::Float { filterable: true },
                    view_dimension: wgpu::TextureViewDimension::D2,
                    multisampled: false,
                },
                count: None,
            },
            wgpu::BindGroupLayoutEntry {
                binding: 3,
                visibility: fragment,
                ty: wgpu::BindingType::Texture {
                    sample_type: wgpu::TextureSampleType::Float { filterable: true },
                    view_dimension: wgpu::TextureViewDimension::D2,
                    multisampled: false,
                },
                count: None,
            },
            wgpu::BindGroupLayoutEntry {
                binding: 4,
                visibility: fragment,
                ty: wgpu::BindingType::Texture {
                    sample_type: wgpu::TextureSampleType::Float { filterable: true },
                    view_dimension: wgpu::TextureViewDimension::D2,
                    multisampled: false,
                },
                count: None,
            },
            wgpu::BindGroupLayoutEntry {
                binding: 5,
                visibility: fragment,
                ty: wgpu::BindingType::Texture {
                    sample_type: wgpu::TextureSampleType::Float { filterable: true },
                    view_dimension: wgpu::TextureViewDimension::D2Array,
                    multisampled: false,
                },
                count: None,
            },
            wgpu::BindGroupLayoutEntry {
                binding: 6,
                visibility: fragment,
                ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                count: None,
            },
            wgpu::BindGroupLayoutEntry {
                binding: 7,
                visibility: fragment,
                ty: wgpu::BindingType::Texture {
                    sample_type: wgpu::TextureSampleType::Float { filterable: true },
                    view_dimension: wgpu::TextureViewDimension::D2,
                    multisampled: false,
                },
                count: None,
            },
            wgpu::BindGroupLayoutEntry {
                binding: 8,
                visibility: fragment,
                ty: wgpu::BindingType::Texture {
                    sample_type: wgpu::TextureSampleType::Float { filterable: true },
                    view_dimension: wgpu::TextureViewDimension::D2,
                    multisampled: false,
                },
                count: None,
            },
            wgpu::BindGroupLayoutEntry {
                binding: 9,
                visibility: fragment,
                ty: wgpu::BindingType::Texture {
                    sample_type: wgpu::TextureSampleType::Float { filterable: true },
                    view_dimension: wgpu::TextureViewDimension::D2,
                    multisampled: false,
                },
                count: None,
            },
            wgpu::BindGroupLayoutEntry {
                binding: 10,
                visibility: fragment,
                ty: wgpu::BindingType::Texture {
                    sample_type: wgpu::TextureSampleType::Float { filterable: true },
                    view_dimension: wgpu::TextureViewDimension::D2,
                    multisampled: false,
                },
                count: None,
            },
        ],
    })
}
