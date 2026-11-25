//! `--water enhanced` ripples (an enhancement, not parity): a GPU wave-equation
//! heightfield over the map's water, which adds to the surface's normal in `water.wgsl` (the
//! reflections' Fresnel share, the sheen and the refraction). See `water_ripples.wgsl`.
//!
//! The field covers the grid vertices that have a water level (`water::grid_info`) with a
//! margin, at up to [`MAX_TEXELS_PER_CELL`] texels per cell and at most [`MAX_SIDE`] texels a
//! side. It steps at [`STEP_HZ`] (real time, at most [`MAX_STEPS`] per frame), the shallow-water
//! speed `sqrt(g depth)` from the floor heights. The disturbances are the client's: objects in
//! the water this frame. Client side only: nothing reads it back.

/// Field steps per second.
pub const STEP_HZ: f32 = 60.0;
/// At most this many steps a frame (after a stall the field skips ahead).
const MAX_STEPS: u32 = 4;
const MAX_TEXELS_PER_CELL: u32 = 8;
const MAX_SIDE: u32 = 1024;
/// Disturbances per frame (`water_ripples.wgsl`).
pub const MAX_DISTURBANCES: usize = 32;
/// Courant number squared held below this (the 2D scheme is stable to 0.5).
const MAX_COURANT2: f32 = 0.45;
const GRAVITY: f32 = 9.81;
const FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rg16Float;

/// An object in the water this frame.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Disturbance {
    /// Render space.
    pub position: [f32; 3],
    /// Height added at the centre this frame (render units, before `ripple_strength`).
    pub amount: f32,
    /// Radius (cells).
    pub radius: f32,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, bytemuck::Pod, bytemuck::Zeroable)]
struct RippleUniforms {
    region: [f32; 4],
    wave: [f32; 4],
    count: [u32; 4],
    disturbances: [[f32; 4]; MAX_DISTURBANCES],
}

/// The heightfield, its two states and the step's pipeline.
pub struct RippleField {
    /// Kept alive for `views`.
    _textures: [wgpu::Texture; 2],
    views: [wgpu::TextureView; 2],
    pipeline: wgpu::RenderPipeline,
    /// `[state read][0: with the frame's disturbances, 1: without]`.
    bind_groups: [[wgpu::BindGroup; 2]; 2],
    uniform_buffers: [wgpu::Buffer; 2],
    uniforms: RippleUniforms,
    /// The state the last step wrote.
    current: usize,
    accumulator: f32,
    /// Grid placement: half width, half depth, cell size (render units), as `water.wgsl`'s
    /// `grid`.
    grid: [f32; 3],
    /// The grid's `(w, h)` vertices' water level and physical height, 1 where there is a level.
    levels: Vec<[f32; 3]>,
    grid_size: (u32, u32),
    strength: f32,
    /// The disturbances of frames since the last step, merged by place.
    pending: Vec<Disturbance>,
}

/// Pending disturbances kept between steps.
const MAX_PENDING: usize = 4 * MAX_DISTURBANCES;

impl RippleField {
    /// The field over `info`'s water (`water::grid_info`: `w x h` vertices), or `None` without
    /// water. `floor` is the view of `water.wgsl`'s `t_floor`.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        floor: &wgpu::TextureView,
        (w, h): (u32, u32),
        info: &[[f32; 4]],
        grid: [f32; 3],
        meters_per_cell: f32,
        strength: f32,
        damping: f32,
    ) -> Option<Self> {
        let wet = |i: usize| info[i][2] > 0.5 && info[i][1] > info[i][0];
        let (mut c0, mut r0, mut c1, mut r1) = (u32::MAX, u32::MAX, 0u32, 0u32);
        for i in (0..(w * h) as usize).filter(|&i| wet(i)) {
            let (c, r) = (i as u32 % w, i as u32 / w);
            c0 = c0.min(c);
            r0 = r0.min(r);
            c1 = c1.max(c);
            r1 = r1.max(r);
        }
        if c0 > c1 {
            return None;
        }
        let (c0, r0) = (c0.saturating_sub(1), r0.saturating_sub(1));
        let (c1, r1) = ((c1 + 1).min(w - 1), (r1 + 1).min(h - 1));
        let (cells_w, cells_h) = ((c1 - c0).max(1), (r1 - r0).max(1));
        let tpc = (MAX_SIDE / cells_w.max(cells_h)).clamp(1, MAX_TEXELS_PER_CELL);
        let size = wgpu::Extent3d {
            width: cells_w * tpc,
            height: cells_h * tpc,
            depth_or_array_layers: 1,
        };
        tracing::info!(
            "--water enhanced ripples: cells {c0}..{c1} x {r0}..{r1}, {tpc} texels per cell, {} x {} texels",
            size.width,
            size.height
        );
        let make = |label: &str| {
            device.create_texture(&wgpu::TextureDescriptor {
                label: Some(label),
                size,
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: FORMAT,
                usage: wgpu::TextureUsages::TEXTURE_BINDING
                    | wgpu::TextureUsages::RENDER_ATTACHMENT,
                view_formats: &[],
            })
        };
        let textures = [make("Water Ripples A"), make("Water Ripples B")];
        let views = [
            textures[0].create_view(&Default::default()),
            textures[1].create_view(&Default::default()),
        ];
        let dt = 1.0 / STEP_HZ;
        // Courant number squared `(c dt / dx)^2` with `c^2 = g depth`, depth in render units
        // times metres per render unit, `dx` a texel in metres.
        let per_render = meters_per_cell / grid[2];
        let dx = per_render / tpc as f32 * grid[2];
        let uniforms = RippleUniforms {
            region: [c0 as f32, r0 as f32, tpc as f32, 0.0],
            wave: [
                GRAVITY * per_render * dt * dt / (dx * dx),
                MAX_COURANT2,
                (1.0 - damping.clamp(0.0, 1.0)).powf(dt),
                0.0,
            ],
            count: [0; 4],
            disturbances: [[0.0; 4]; MAX_DISTURBANCES],
        };
        let uniform_buffers = [0, 1].map(|_| {
            device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("Water Ripple Uniforms"),
                size: std::mem::size_of::<RippleUniforms>() as u64,
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            })
        });
        for b in &uniform_buffers {
            queue.write_buffer(b, 0, bytemuck::bytes_of(&uniforms));
        }
        let texture_entry = |binding| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Texture {
                sample_type: wgpu::TextureSampleType::Float { filterable: false },
                view_dimension: wgpu::TextureViewDimension::D2,
                multisampled: false,
            },
            count: None,
        };
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("Water Ripple Layout"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                texture_entry(1),
                texture_entry(2),
            ],
        });
        let group = |state: usize, uniform: usize| {
            device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("Water Ripple Bind Group"),
                layout: &layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: uniform_buffers[uniform].as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::TextureView(&views[state]),
                    },
                    wgpu::BindGroupEntry {
                        binding: 2,
                        resource: wgpu::BindingResource::TextureView(floor),
                    },
                ],
            })
        };
        let bind_groups = [[group(0, 0), group(0, 1)], [group(1, 0), group(1, 1)]];
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Water Ripples WGSL"),
            source: wgpu::ShaderSource::Wgsl(SHADER_SRC.into()),
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("Water Ripple Pipeline Layout"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("Water Ripple Pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_ripple"),
                buffers: &[],
                compilation_options: Default::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_ripple"),
                targets: &[Some(wgpu::ColorTargetState {
                    format: FORMAT,
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
        let levels = info.iter().map(|v| [v[1], v[0], v[2]]).collect::<Vec<_>>();
        Some(Self {
            _textures: textures,
            views,
            pipeline,
            bind_groups,
            uniform_buffers,
            uniforms,
            current: 0,
            accumulator: 0.0,
            grid,
            levels,
            grid_size: (w, h),
            strength,
            pending: Vec::new(),
        })
    }

    /// The state the last step wrote, which the water's draws read.
    pub fn current(&self) -> usize {
        self.current
    }

    pub fn views(&self) -> &[wgpu::TextureView; 2] {
        &self.views
    }

    /// Grid column and row of texel (0, 0) and texels per cell.
    pub fn region(&self) -> [f32; 3] {
        [
            self.uniforms.region[0],
            self.uniforms.region[1],
            self.uniforms.region[2],
        ]
    }

    /// Grid column and row of a render-space point.
    fn grid_point(&self, p: [f32; 3]) -> (f32, f32) {
        (
            (self.grid[0] - p[0]) / self.grid[2],
            (p[2] + self.grid[1]) / self.grid[2],
        )
    }

    /// Whether `p` is in the water: the vertex nearest it has a level above its floor, and `p`
    /// is not above the level by more than a quarter cell (flyers and arrows in the air).
    fn in_water(&self, p: [f32; 3]) -> bool {
        let (c, r) = self.grid_point(p);
        let (w, h) = self.grid_size;
        if !(c >= 0.0 && r >= 0.0 && c <= (w - 1) as f32 && r <= (h - 1) as f32) {
            return false;
        }
        let v = self.levels[r.round() as usize * w as usize + c.round() as usize];
        v[2] > 0.5 && v[0] > v[1] && p[1] <= v[0] + 0.25 * self.grid[2]
    }

    /// Advance the field by `dt` seconds of real time. The frame's `disturbances` (those in the
    /// water) wait with the frames' before them, merged where they overlap, for the next step,
    /// which adds them all.
    pub fn step(
        &mut self,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        dt: f32,
        disturbances: &[Disturbance],
    ) {
        if !disturbances.is_empty() {
            tracing::trace!(
                "water ripples: {} offered, {} in the water, e.g. {:?}",
                disturbances.len(),
                disturbances
                    .iter()
                    .filter(|d| self.in_water(d.position))
                    .count(),
                disturbances[0].position
            );
        }
        for d in disturbances {
            if !self.in_water(d.position) {
                continue;
            }
            let room = self.pending.len() < MAX_PENDING;
            let cell = self.grid[2];
            let near = self.pending.iter_mut().find(|p| {
                let (dx, dz) = (p.position[0] - d.position[0], p.position[2] - d.position[2]);
                (dx * dx + dz * dz).sqrt() < 0.5 * d.radius.min(p.radius) * cell
            });
            match near {
                Some(p) => {
                    p.amount += d.amount;
                    p.position = d.position;
                }
                None if room => self.pending.push(*d),
                None => {}
            }
        }
        self.accumulator = (self.accumulator + dt.max(0.0)).min(MAX_STEPS as f32 / STEP_HZ);
        let steps = (self.accumulator * STEP_HZ).floor() as u32;
        if steps == 0 {
            return;
        }
        self.accumulator -= steps as f32 / STEP_HZ;
        // The strongest first.
        self.pending
            .sort_by(|a, b| b.amount.abs().total_cmp(&a.amount.abs()));
        let mut n = 0usize;
        for d in self.pending.drain(..).take(MAX_DISTURBANCES) {
            let (c, r) = (
                (self.grid[0] - d.position[0]) / self.grid[2],
                (d.position[2] + self.grid[1]) / self.grid[2],
            );
            self.uniforms.disturbances[n] = [c, r, d.amount * self.strength, d.radius];
            n += 1;
        }
        if n > 0 {
            tracing::debug!("water ripples: {n} disturbances, {steps} steps");
        }
        self.uniforms.count[0] = n as u32;
        queue.write_buffer(
            &self.uniform_buffers[0],
            0,
            bytemuck::bytes_of(&self.uniforms),
        );
        for i in 0..steps {
            let read = self.current;
            let write = 1 - read;
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("Water Ripple Step"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &self.views[write],
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_pipeline(&self.pipeline);
            pass.set_bind_group(0, &self.bind_groups[read][usize::from(i > 0)], &[]);
            pass.draw(0..3, 0..1);
            drop(pass);
            self.current = write;
        }
    }
}

/// The step's shader.
pub const SHADER_SRC: &str = include_str!("../../shaders/water_ripples.wgsl");

#[cfg(test)]
mod tests {
    #[test]
    fn ripple_shader_parses_and_validates() {
        let module = naga::front::wgsl::parse_str(super::SHADER_SRC)
            .unwrap_or_else(|e| panic!("{}", e.emit_to_string(super::SHADER_SRC)));
        naga::valid::Validator::new(
            naga::valid::ValidationFlags::all(),
            naga::valid::Capabilities::default(),
        )
        .validate(&module)
        .expect("water_ripples.wgsl validates");
    }
}
