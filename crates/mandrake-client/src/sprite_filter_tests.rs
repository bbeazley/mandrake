use crate::args::Args;
use crate::camera_control;
use crate::render_passes::create_depth_view;
use crate::tests::test_gpu;
use crate::ui_scale;
use clap::Parser;
use mandrake_render::{
    BillboardRenderer, ModelRenderer, RenderPipelineConfig, SceneUniforms, TerrainRenderer,
    TextureSampling,
};
use mandrake_view::billboard::BILLBOARD_FLAG_CONNECTOR;
use mandrake_view::interface_picture::InterfaceFilter;
use mandrake_view::{billboard::BillboardInstance, camera::Camera};
use ui_scale::UiFilter;
use wgpu::util::DeviceExt;

const MODES: [InterfaceFilter; 4] = [
    InterfaceFilter::Linear,
    InterfaceFilter::Sharp,
    InterfaceFilter::Nearest,
    InterfaceFilter::Lanczos2,
];
const RED: [u8; 4] = [255, 0, 0, 255];
const GREEN: [u8; 4] = [0, 255, 0, 255];
const BLUE: [u8; 4] = [0, 0, 255, 255];

#[test]
fn option_parses_and_leaves_the_default_to_sampling() {
    let parse = |extra: &[&str]| {
        let mut argv = vec!["mandrake"];
        argv.extend_from_slice(extra);
        Args::try_parse_from(argv).map(|a| a.sprite_filter)
    };
    assert_eq!(parse(&[]).unwrap(), None);
    for (value, filter) in [
        ("linear", UiFilter::Linear),
        ("sharp", UiFilter::Sharp),
        ("nearest", UiFilter::Nearest),
        ("lanczos2", UiFilter::Lanczos2),
    ] {
        assert_eq!(parse(&["--sprite-filter", value]).unwrap(), Some(filter));
    }
    assert!(parse(&["--sprite-filter", "lanczos3"]).is_err());
}

#[test]
fn sampling_defaults_to_software_and_the_flag_selects_linear() {
    let uniforms = |extra: &[&str]| {
        let mut argv = vec!["mandrake"];
        argv.extend_from_slice(extra);
        let a = Args::try_parse_from(argv).unwrap();
        ui_scale::with_world_sampling(SceneUniforms::default(), a.sampling, a.sprite_filter)
            .render_params
    };
    assert_eq!(uniforms(&[])[0], 1.0, "software landscape and models");
    assert_eq!(uniforms(&[])[1], 4.0, "SPRITE_FILTER_SOFTWARE");
    assert_eq!(
        uniforms(&["--sampling", "linear"]),
        SceneUniforms::default().render_params
    );
    assert_eq!(uniforms(&["--sprite-filter", "linear"])[..2], [1.0, 0.0]);
    assert_eq!(
        uniforms(&["--sampling", "linear", "--sprite-filter", "nearest"])[..2],
        [0.0, 2.0]
    );
}

#[test]
fn sampling_bicubic_and_sharp_select_their_slots() {
    let slots = |extra: &[&str]| {
        let mut argv = vec!["mandrake"];
        argv.extend_from_slice(extra);
        let a = Args::try_parse_from(argv).unwrap();
        let p =
            ui_scale::with_world_sampling(SceneUniforms::default(), a.sampling, a.sprite_filter)
                .render_params;
        [p[0], p[1]]
    };
    assert_eq!(slots(&["--sampling", "bicubic"]), [2.0, 3.0], "Lanczos-2");
    assert_eq!(slots(&["--sampling", "sharp"]), [3.0, 1.0], "sharp");
    assert_eq!(
        slots(&["--sampling", "bicubic", "--sprite-filter", "linear"]),
        [2.0, 0.0]
    );
    assert_eq!(
        slots(&["--sampling", "sharp", "--sprite-filter", "nearest"]),
        [3.0, 2.0]
    );
    assert!(Args::try_parse_from(["mandrake", "--sampling", "cubic"]).is_err());
}

fn draw(
    (device, queue): &(wgpu::Device, wgpu::Queue),
    texture: impl FnOnce(&wgpu::BindGroupLayout) -> (wgpu::BindGroup, [f32; 2], [f32; 2]),
    size: u32,
    filter: InterfaceFilter,
) -> Vec<u8> {
    let format = wgpu::TextureFormat::Rgba8Unorm;
    let config = RenderPipelineConfig {
        width: size,
        height: size,
        target_format: format,
        depth_format: Some(wgpu::TextureFormat::Depth32Float),
    };
    let terrain = TerrainRenderer::new(device, queue, &config);
    terrain.update_uniforms(
        queue,
        &SceneUniforms::default()
            .with_viewport(size, size)
            .with_sprite_filter(filter),
    );
    let renderer = BillboardRenderer::new(device, queue, &config);
    let (bind_group, uv_min, uv_max) = texture(&renderer.texture_bind_group_layout);
    let instance = BillboardInstance {
        position: [-1.0, -1.0, 0.5],
        flags: BILLBOARD_FLAG_CONNECTOR,
        size: [0.0, 2.0],
        anchor: [2.0, 0.0],
        uv_min,
        uv_max,
        ..BillboardInstance::default()
    };
    draw_with(
        device,
        queue,
        &terrain,
        &renderer,
        &bind_group,
        instance,
        size,
    )
}

fn draw_with(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    terrain: &TerrainRenderer,
    renderer: &BillboardRenderer,
    bind_group: &wgpu::BindGroup,
    instance: BillboardInstance,
    size: u32,
) -> Vec<u8> {
    let buffer = BillboardRenderer::create_instance_buffer(device, &[instance]);
    let target = Target::new(device, wgpu::TextureFormat::Rgba8Unorm, size);
    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
    {
        let mut pass = target.pass(&mut encoder);
        renderer.render(
            &mut pass,
            &terrain.uniform_bind_group,
            Some(bind_group),
            &buffer,
            1,
        );
    }
    target.read(device, queue, encoder)
}

struct Target {
    texture: wgpu::Texture,
    view: wgpu::TextureView,
    depth: wgpu::TextureView,
    size: u32,
}

impl Target {
    fn new(device: &wgpu::Device, format: wgpu::TextureFormat, size: u32) -> Self {
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: None,
            size: wgpu::Extent3d {
                width: size,
                height: size,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        let depth = create_depth_view(device, size, size);
        Self {
            texture,
            view,
            depth,
            size,
        }
    }

    fn pass<'e>(&self, encoder: &'e mut wgpu::CommandEncoder) -> wgpu::RenderPass<'e> {
        encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: None,
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                depth_slice: None,
                view: &self.view,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                view: &self.depth,
                depth_ops: Some(wgpu::Operations {
                    load: wgpu::LoadOp::Clear(1.0),
                    store: wgpu::StoreOp::Store,
                }),
                stencil_ops: None,
            }),
            occlusion_query_set: None,
            timestamp_writes: None,
            multiview_mask: None,
        })
    }

    fn read(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        mut encoder: wgpu::CommandEncoder,
    ) -> Vec<u8> {
        let size = self.size;
        let row = (size * 4).next_multiple_of(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT);
        let readback = device.create_buffer(&wgpu::BufferDescriptor {
            label: None,
            size: u64::from(row * size),
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        encoder.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo {
                texture: &self.texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::TexelCopyBufferInfo {
                buffer: &readback,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(row),
                    rows_per_image: Some(size),
                },
            },
            wgpu::Extent3d {
                width: size,
                height: size,
                depth_or_array_layers: 1,
            },
        );
        queue.submit(std::iter::once(encoder.finish()));
        let slice = readback.slice(..);
        slice.map_async(wgpu::MapMode::Read, |_| {});
        let _ = device.poll(wgpu::PollType::wait_indefinitely());
        let data = slice.get_mapped_range().unwrap();
        data.chunks(row as usize)
            .flat_map(|r| r[..(size * 4) as usize].to_vec())
            .collect()
    }
}

#[test]
fn sprite_rows_are_clipped_by_the_depth_plane() {
    let Some((device, queue)) = test_gpu() else {
        eprintln!("no GPU adapter: skipping the sprite depth GPU test");
        return;
    };
    const SIZE: u32 = 128;
    let format = wgpu::TextureFormat::Rgba8Unorm;
    let config = RenderPipelineConfig {
        width: SIZE,
        height: SIZE,
        target_format: format,
        depth_format: Some(wgpu::TextureFormat::Depth32Float),
    };
    let terrain = TerrainRenderer::new(&device, &queue, &config);
    let renderer = BillboardRenderer::new(&device, &queue, &config);
    let image = image::RgbaImage::from_pixel(16, 16, image::Rgba(RED));
    let (_, _, bind_group) = mandrake_render::create_texture_from_rgba(
        &device,
        &queue,
        &renderer.texture_bind_group_layout,
        &image,
        None,
    );
    let frame = mandrake_view::model_surfaces::WorldFrame::new(256, 256);
    let pitch = camera_control::START_PITCH - std::f32::consts::TAU;
    let shot = |eye_y: f32| {
        let eye = frame.from_engine([34570.0, eye_y, 1771.0]);
        let camera = Camera::from_myth_observer(eye, std::f32::consts::FRAC_PI_2, pitch, 1.0);
        terrain.update_uniforms(
            &queue,
            &SceneUniforms::from_camera(&camera, 1.0).with_viewport(SIZE, SIZE),
        );
        let tree = BillboardInstance::new(
            frame.from_engine([34599.0, 19682.0, 626.0]),
            [1.92, 2.72],
            [0.0; 2],
            [1.0; 2],
        )
        .with_anchor([0.5, 1.0])
        .with_screen_aligned(true)
        .in_engine_view(&mandrake_view::billboard::EngineSpriteView::of(&camera));
        let px = draw_with(
            &device,
            &queue,
            &terrain,
            &renderer,
            &bind_group,
            tree,
            SIZE,
        );
        let drawn = |rows: std::ops::Range<u32>| {
            rows.map(|y| {
                px[(y * SIZE * 4) as usize..((y + 1) * SIZE * 4) as usize]
                    .chunks(4)
                    .filter(|p| p[3] != 0)
                    .count()
            })
            .sum::<usize>()
        };
        (drawn(0..SIZE / 2), drawn(SIZE / 2..SIZE))
    };
    assert_eq!(shot(20003.0), (0, 0), "the near tree is not drawn");
    let (top, bottom) = shot(19503.0);
    assert_eq!(
        top, 0,
        "further back the upper rows stay beyond the far plane"
    );
    assert!(bottom > 0, "and the lowest rows are drawn");
    assert!(
        shot(17003.0).0 > bottom,
        "far enough, the sprite is drawn whole"
    );
}

fn hostile_atlas() -> image::RgbaImage {
    image::RgbaImage::from_fn(16, 16, |x, y| {
        image::Rgba(match (x, y) {
            (2..=3, 2..=5) => RED,
            (4..=5, 2..=5) => GREEN,
            (1..=6, 1..=6) => [0; 4],
            _ => BLUE,
        })
    })
}

#[test]
fn magnified_modes_stay_inside_the_frame() {
    let Some(gpu) = test_gpu() else {
        eprintln!("no GPU adapter: skipping sprite filter GPU test");
        return;
    };
    const SIZE: u32 = 61;
    let image = hostile_atlas();
    let shot = |filter| {
        draw(
            &gpu,
            |layout| {
                let (_, _, bg) =
                    mandrake_render::create_texture_from_rgba(&gpu.0, &gpu.1, layout, &image, None);
                (bg, [2.0 / 16.0, 2.0 / 16.0], [6.0 / 16.0, 6.0 / 16.0])
            },
            SIZE,
            filter,
        )
    };
    let [linear, sharp, nearest, lanczos] = MODES.map(shot);
    for (name, px) in [
        ("linear", &linear),
        ("sharp", &sharp),
        ("nearest", &nearest),
        ("lanczos2", &lanczos),
    ] {
        for p in px.chunks(4) {
            assert_eq!(p[2], 0, "{name} read past the ring: {p:?}");
            assert!(
                p[0] <= p[3] && p[1] <= p[3],
                "{name} not premultiplied: {p:?}"
            );
        }
    }
    let row = (SIZE / 2) as usize;
    let mixed = |px: &[u8]| {
        px[row * (SIZE as usize) * 4..(row + 1) * (SIZE as usize) * 4]
            .chunks(4)
            .filter(|p| p[0] != 0 && p[1] != 0)
            .count()
    };
    assert!(mixed(&linear) >= 14, "linear blends a whole texel");
    assert_eq!(mixed(&nearest), 0, "nearest has no blended pixel");
    assert!((1..=2).contains(&mixed(&sharp)), "sharp: {}", mixed(&sharp));
    assert_ne!(lanczos, linear, "lanczos2 is its own filter");
    assert!(nearest.chunks(4).all(|p| p == RED || p == GREEN));
}

#[test]
fn minified_modes_keep_the_mip_blend_and_meet_it_smoothly() {
    let Some(gpu) = test_gpu() else {
        eprintln!("no GPU adapter: skipping sprite filter GPU test");
        return;
    };
    let stripes = image::RgbaImage::from_fn(64, 64, |x, y| {
        image::Rgba(if (x / 2 + y / 2) % 2 == 0 { RED } else { GREEN })
    });
    let shot = |size, filter| {
        draw(
            &gpu,
            |layout| {
                let atlas = mandrake_render::create_atlas_from_images(
                    &gpu.0,
                    &gpu.1,
                    layout,
                    std::slice::from_ref(&stripes),
                );
                let (uv_min, uv_max) = (atlas.frames[0].uv_min, atlas.frames[0].uv_max);
                (atlas.bind_group, uv_min, uv_max)
            },
            size,
            filter,
        )
    };
    for size in [16, 45] {
        let [linear, sharp, nearest, lanczos] = MODES.map(|f| shot(size, f));
        assert_eq!(sharp, linear, "sharp at {size}");
        assert_eq!(nearest, linear, "nearest at {size}");
        assert_eq!(lanczos, linear, "lanczos2 at {size}");
    }
    let [linear, sharp, nearest, lanczos] = MODES.map(|f| shot(46, f));
    for (name, px, limit) in [
        ("sharp", &sharp, 1),
        ("nearest", &nearest, 15),
        ("lanczos2", &lanczos, 15),
    ] {
        let worst = px
            .iter()
            .zip(&linear)
            .map(|(a, b)| a.abs_diff(*b))
            .max()
            .unwrap();
        assert!(worst <= limit, "{name} is {worst} from bilinear");
    }
}

const SAMPLINGS: [TextureSampling; 3] = [
    TextureSampling::Linear,
    TextureSampling::Bicubic,
    TextureSampling::Sharp,
];

fn quad_buffer(
    device: &wgpu::Device,
    vertex: impl Fn(f32, f32, f32, f32) -> Vec<f32>,
) -> wgpu::Buffer {
    let corners = [
        (-1.0, -1.0),
        (1.0, -1.0),
        (-1.0, 1.0),
        (1.0, -1.0),
        (1.0, 1.0),
        (-1.0, 1.0),
    ];
    let bytes: Vec<u8> = corners
        .iter()
        .flat_map(|&(x, y)| vertex(x, y, (x + 1.0) / 2.0, (y + 1.0) / 2.0))
        .flat_map(f32::to_ne_bytes)
        .collect();
    device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: None,
        contents: &bytes,
        usage: wgpu::BufferUsages::VERTEX,
    })
}

fn draw_landscape(
    (device, queue): &(wgpu::Device, wgpu::Queue),
    img: &image::RgbaImage,
    size: u32,
    sampling: TextureSampling,
) -> Vec<u8> {
    let format = wgpu::TextureFormat::Rgba8UnormSrgb;
    let config = RenderPipelineConfig {
        width: size,
        height: size,
        target_format: format,
        depth_format: Some(wgpu::TextureFormat::Depth32Float),
    };
    let terrain = TerrainRenderer::new(device, queue, &config);
    terrain.update_uniforms(queue, &SceneUniforms::default().with_sampling(sampling));
    let bind_group =
        terrain.create_texture_bind_group(device, queue, img.width(), img.height(), img.as_raw());
    let vertices = quad_buffer(device, |x, y, u, v| vec![x, y, 0.5, u, v, 0.0]);
    let target = Target::new(device, format, size);
    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
    {
        let mut pass = target.pass(&mut encoder);
        pass.set_pipeline(&terrain.terrain_pipeline);
        pass.set_bind_group(0, &terrain.uniform_bind_group, &[]);
        pass.set_bind_group(1, &bind_group, &[]);
        pass.set_vertex_buffer(0, vertices.slice(..));
        pass.draw(0..6, 0..1);
    }
    target.read(device, queue, encoder)
}

fn draw_model(
    (device, queue): &(wgpu::Device, wgpu::Queue),
    img: &image::RgbaImage,
    size: u32,
    sampling: TextureSampling,
) -> Vec<u8> {
    let format = wgpu::TextureFormat::Rgba8UnormSrgb;
    let config = RenderPipelineConfig {
        width: size,
        height: size,
        target_format: format,
        depth_format: Some(wgpu::TextureFormat::Depth32Float),
    };
    let terrain = TerrainRenderer::new(device, queue, &config);
    terrain.update_uniforms(queue, &SceneUniforms::default().with_sampling(sampling));
    let models = ModelRenderer::new(device, queue, &config);
    let bind_group = models.create_texture_bind_group(device, queue, img, None);
    let vertices = quad_buffer(device, |x, y, u, v| {
        vec![x, y, 0.5, 0.0, 0.0, 1.0, u, v, 1.0]
    });
    let target = Target::new(device, format, size);
    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
    {
        let mut pass = target.pass(&mut encoder);
        pass.set_pipeline(&models.pipeline);
        pass.set_bind_group(0, &terrain.uniform_bind_group, &[]);
        pass.set_bind_group(1, &bind_group, &[]);
        pass.set_bind_group(2, &models.default_model_bind_group, &[]);
        pass.set_vertex_buffer(0, vertices.slice(..));
        pass.draw(0..6, 0..1);
    }
    target.read(device, queue, encoder)
}

#[test]
fn magnified_landscape_keeps_its_edges_and_has_each_look() {
    let Some(gpu) = test_gpu() else {
        eprintln!("no GPU adapter: skipping landscape sampling GPU test");
        return;
    };
    const SIZE: u32 = 61;
    let flat = image::RgbaImage::from_pixel(4, 4, image::Rgba([90, 140, 200, 255]));
    for sampling in SAMPLINGS {
        let px = draw_landscape(&gpu, &flat, SIZE, sampling);
        for p in px.chunks(4) {
            let off = p
                .iter()
                .zip(flat.get_pixel(0, 0).0)
                .map(|(a, b)| a.abs_diff(b));
            assert!(off.max().unwrap() <= 1, "{sampling:?}: {p:?}");
        }
    }
    let halves =
        image::RgbaImage::from_fn(4, 4, |x, _| image::Rgba(if x < 2 { RED } else { GREEN }));
    let [linear, bicubic, sharp] = SAMPLINGS.map(|s| draw_landscape(&gpu, &halves, SIZE, s));
    let row = (SIZE / 2) as usize;
    let mixed = |px: &[u8]| {
        px[row * (SIZE as usize) * 4..(row + 1) * (SIZE as usize) * 4]
            .chunks(4)
            .filter(|p| p[0] != 0 && p[1] != 0)
            .count()
    };
    assert!(mixed(&linear) >= 14, "linear blends a whole texel");
    assert!((1..=2).contains(&mixed(&sharp)), "sharp: {}", mixed(&sharp));
    assert_ne!(bicubic, linear, "bicubic is its own filter");
}

#[test]
fn magnified_models_have_no_fringe_at_the_key() {
    let Some(gpu) = test_gpu() else {
        eprintln!("no GPU adapter: skipping model sampling GPU test");
        return;
    };
    const SIZE: u32 = 61;
    let keyed = image::RgbaImage::from_fn(8, 8, |x, _| {
        image::Rgba(if x < 4 { [200, 60, 30, 255] } else { [0; 4] })
    });
    let kept = |px: &[u8]| px.chunks(4).filter(|p| p[3] != 0).count();
    let linear = draw_model(&gpu, &keyed, SIZE, TextureSampling::Linear);
    assert!(kept(&linear) > 0);
    for sampling in [TextureSampling::Bicubic, TextureSampling::Sharp] {
        let px = draw_model(&gpu, &keyed, SIZE, sampling);
        for p in px.chunks(4).filter(|p| p[3] != 0) {
            let off = p[..3]
                .iter()
                .zip([200u8, 60, 30])
                .map(|(a, b)| a.abs_diff(b));
            assert!(off.max().unwrap() <= 1, "{sampling:?} fringe: {p:?}");
        }
        assert!(
            kept(&px).abs_diff(kept(&linear)) <= 2 * SIZE as usize,
            "{sampling:?} moved the cutout: {} against {}",
            kept(&px),
            kept(&linear)
        );
    }
}

#[test]
fn minified_landscape_and_models_keep_the_linear_path() {
    let Some(gpu) = test_gpu() else {
        eprintln!("no GPU adapter: skipping sampling GPU test");
        return;
    };
    let stripes = image::RgbaImage::from_fn(64, 64, |x, y| {
        image::Rgba(if (x / 2 + y / 2) % 2 == 0 { RED } else { GREEN })
    });
    let worst = |a: &[u8], b: &[u8]| a.iter().zip(b).map(|(a, b)| a.abs_diff(*b)).max().unwrap();
    type Draw =
        fn(&(wgpu::Device, wgpu::Queue), &image::RgbaImage, u32, TextureSampling) -> Vec<u8>;
    for (name, draw) in [
        ("landscape", draw_landscape as Draw),
        ("model", draw_model as Draw),
    ] {
        for size in [16, 45] {
            let [linear, bicubic, sharp] = SAMPLINGS.map(|s| draw(&gpu, &stripes, size, s));
            assert_eq!(bicubic, linear, "{name} bicubic at {size}");
            assert_eq!(sharp, linear, "{name} sharp at {size}");
        }
        let [linear, bicubic, sharp] = SAMPLINGS.map(|s| draw(&gpu, &stripes, 46, s));
        for (mode, px) in [("bicubic", &bicubic), ("sharp", &sharp)] {
            let worst = worst(px, &linear);
            assert!(worst <= 60, "{name} {mode} is {worst} from linear");
        }
    }
}

struct SoftEdgeScene {
    terrain: TerrainRenderer,
    mesh: mandrake_render::TerrainMesh,
    edge: mandrake_render::ReflectionEdge,
    size: u32,
}

impl SoftEdgeScene {
    fn new((device, queue): &(wgpu::Device, wgpu::Queue), size: u32) -> Self {
        let config = RenderPipelineConfig {
            width: size,
            height: size,
            target_format: wgpu::TextureFormat::Rgba8Unorm,
            depth_format: Some(wgpu::TextureFormat::Depth32Float),
        };
        let terrain = TerrainRenderer::new(device, queue, &config);
        terrain.update_uniforms(queue, &SceneUniforms::default().with_viewport(size, size));
        let corner = |x, y| mandrake_render::TerrainVertex::new([x, y, 0.5], [0.0, 0.0], 0);
        let mesh = mandrake_render::TerrainMesh::from_raw(
            device,
            "dry",
            &[
                corner(0.0, -1.0),
                corner(1.0, -1.0),
                corner(1.0, 1.0),
                corner(0.0, 1.0),
            ],
            &[0, 1, 2, 0, 2, 3],
        );
        let edge = mandrake_render::ReflectionEdge::new(device, &config);
        Self {
            terrain,
            mesh,
            edge,
            size,
        }
    }

    fn row(
        &self,
        (device, queue): &(wgpu::Device, wgpu::Queue),
        soft: bool,
        draw: impl FnOnce(&mut wgpu::RenderPass<'_>, &wgpu::BindGroup, Option<&wgpu::BindGroup>),
    ) -> Vec<u8> {
        let target = Target::new(device, wgpu::TextureFormat::Rgba8Unorm, self.size);
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
        drop(target.pass(&mut encoder));
        self.edge.encode_mask(
            &mut encoder,
            &self.terrain.uniform_bind_group,
            &self.mesh,
            &target.depth,
        );
        {
            let mut pass = crate::render_passes::begin_opaque_continue(
                &mut encoder,
                &target.view,
                &target.depth,
            );
            draw(
                &mut pass,
                &self.terrain.uniform_bind_group,
                soft.then(|| self.edge.bind_group()),
            );
        }
        let pixels = target.read(device, queue, encoder);
        let y = self.size as usize / 2;
        (0..self.size as usize)
            .map(|x| pixels[(y * self.size as usize + x) * 4])
            .collect()
    }
}

fn assert_fades_to_the_outline(row: &[u8], full: u8) {
    let half = row.len() / 2;
    for (name, at) in [("left edge", 0), ("beyond the radius", half - 26)] {
        assert!(
            row[at].abs_diff(full) <= 1,
            "{name}: {} should be {full} +-1: {row:?}",
            row[at]
        );
    }
    assert!(row[half - 1] < full / 4, "faded at the outline: {row:?}");
    assert!(
        row[..half].windows(2).all(|w| w[1] <= w[0]),
        "falls towards the outline: {row:?}"
    );
    let steps: Vec<u8> = row[half - 26..half].to_vec();
    assert!(
        steps.windows(2).all(|w| w[0] - w[1] < full / 2),
        "smooth over the radius: {steps:?}"
    );
}

#[test]
fn sprite_reflection_fades_up_to_the_dry_outline() {
    let Some(gpu) = test_gpu() else {
        return;
    };
    let (device, queue) = &gpu;
    let scene = SoftEdgeScene::new(&gpu, 64);
    let renderer = BillboardRenderer::new(
        device,
        queue,
        &RenderPipelineConfig {
            width: 64,
            height: 64,
            target_format: wgpu::TextureFormat::Rgba8Unorm,
            depth_format: Some(wgpu::TextureFormat::Depth32Float),
        },
    );
    let frame_row = |flags: u32, opacity: f32, soft: bool| {
        let instance = BillboardInstance {
            position: [-1.0, -1.0, 0.5],
            flags: BILLBOARD_FLAG_CONNECTOR | flags,
            size: [0.0, 2.0],
            anchor: [2.0, 0.0],
            tint: [1.0, 1.0, 1.0, opacity],
            ..BillboardInstance::default()
        };
        let buffer = BillboardRenderer::create_instance_buffer(device, &[instance]);
        scene.row(&gpu, soft, |pass, scene_group, edge| {
            renderer.render_reflections(pass, scene_group, None, &buffer, 0..1, edge);
        })
    };
    let reflection = mandrake_view::billboard::BILLBOARD_FLAG_REFLECTION;
    assert_fades_to_the_outline(&frame_row(reflection, 1.0, true), 255);
    assert_fades_to_the_outline(&frame_row(reflection, 0.5, true), 127);
    assert!(frame_row(reflection, 1.0, false).iter().all(|&v| v == 255));
    assert!(frame_row(0, 1.0, true).iter().all(|&v| v == 255));
}

#[test]
fn model_reflection_fades_up_to_the_dry_outline() {
    let Some(gpu) = test_gpu() else {
        return;
    };
    let (device, queue) = &gpu;
    let scene = SoftEdgeScene::new(&gpu, 64);
    let models = ModelRenderer::new(
        device,
        queue,
        &RenderPipelineConfig {
            width: 64,
            height: 64,
            target_format: wgpu::TextureFormat::Rgba8Unorm,
            depth_format: Some(wgpu::TextureFormat::Depth32Float),
        },
    );
    let white = image::RgbaImage::from_pixel(1, 1, image::Rgba([255, 255, 255, 255]));
    let bind_group = models.create_texture_bind_group(device, queue, &white, None);
    let vertices = quad_buffer(device, |x, y, u, v| {
        vec![x, y, 0.5, 0.0, 0.0, 1.0, u, v, 1.0]
    });
    let frame_row = |soft: bool| {
        scene.row(&gpu, soft, |pass, scene_group, edge| {
            match edge {
                Some(edge) => {
                    pass.set_pipeline(&models.soft_reflection_pipeline);
                    pass.set_bind_group(3, edge, &[]);
                }
                None => pass.set_pipeline(&models.reflection_pipeline),
            }
            pass.set_bind_group(0, scene_group, &[]);
            pass.set_bind_group(1, &bind_group, &[]);
            pass.set_bind_group(2, &models.default_model_bind_group, &[]);
            pass.set_vertex_buffer(0, vertices.slice(..));
            pass.draw(0..6, 0..1);
        })
    };
    assert_fades_to_the_outline(&frame_row(true), 255);
    assert!(frame_row(false).iter().all(|&v| v == 255));
}

#[test]
fn reflection_edge_mask_is_tested_against_the_media() {
    let Some(gpu) = test_gpu() else {
        return;
    };
    let (device, queue) = &gpu;
    let scene = SoftEdgeScene::new(&gpu, 64);
    let target = Target::new(device, wgpu::TextureFormat::Rgba8Unorm, 64);
    let vertices = quad_buffer(device, |x, y, u, v| vec![x, y, 0.25, u, v, 0.0]);
    let white = image::RgbaImage::from_pixel(1, 1, image::Rgba([255, 255, 255, 255]));
    let bind_group = scene
        .terrain
        .create_texture_bind_group(device, queue, 1, 1, white.as_raw());
    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
    {
        let mut pass = target.pass(&mut encoder);
        pass.set_pipeline(&scene.terrain.terrain_pipeline);
        pass.set_bind_group(0, &scene.terrain.uniform_bind_group, &[]);
        pass.set_bind_group(1, &bind_group, &[]);
        pass.set_vertex_buffer(0, vertices.slice(..));
        pass.draw(0..6, 0..1);
    }
    scene.edge.encode_mask(
        &mut encoder,
        &scene.terrain.uniform_bind_group,
        &scene.mesh,
        &target.depth,
    );
    queue.submit([encoder.finish()]);
    let renderer = BillboardRenderer::new(
        device,
        queue,
        &RenderPipelineConfig {
            width: 64,
            height: 64,
            target_format: wgpu::TextureFormat::Rgba8Unorm,
            depth_format: Some(wgpu::TextureFormat::Depth32Float),
        },
    );
    let instance = BillboardInstance {
        position: [-1.0, -1.0, 0.5],
        flags: BILLBOARD_FLAG_CONNECTOR | mandrake_view::billboard::BILLBOARD_FLAG_REFLECTION,
        size: [0.0, 2.0],
        anchor: [2.0, 0.0],
        ..BillboardInstance::default()
    };
    let buffer = BillboardRenderer::create_instance_buffer(device, &[instance]);
    let black = Target::new(device, wgpu::TextureFormat::Rgba8Unorm, 64);
    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
    {
        let mut pass = black.pass(&mut encoder);
        renderer.render_reflections(
            &mut pass,
            &scene.terrain.uniform_bind_group,
            None,
            &buffer,
            0..1,
            Some(scene.edge.bind_group()),
        );
    }
    let pixels = black.read(device, queue, encoder);
    assert!(pixels.chunks(4).all(|p| p[0] == 255), "nothing faded");
}

fn read_texture(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    texture: &wgpu::Texture,
    (width, height): (u32, u32),
) -> Vec<u8> {
    let row = (width * 4).next_multiple_of(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT);
    let readback = device.create_buffer(&wgpu::BufferDescriptor {
        label: None,
        size: u64::from(row * height),
        usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
    encoder.copy_texture_to_buffer(
        texture.as_image_copy(),
        wgpu::TexelCopyBufferInfo {
            buffer: &readback,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(row),
                rows_per_image: Some(height),
            },
        },
        wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
    );
    queue.submit(std::iter::once(encoder.finish()));
    let slice = readback.slice(..);
    slice.map_async(wgpu::MapMode::Read, |_| {});
    let _ = device.poll(wgpu::PollType::wait_indefinitely());
    let data = slice.get_mapped_range().unwrap();
    data.chunks(row as usize)
        .flat_map(|r| r[..(width * 4) as usize].to_vec())
        .collect()
}

#[test]
fn a_grown_atlas_is_the_atlas_made_again() {
    let Some((device, queue)) = test_gpu() else {
        eprintln!("no GPU adapter: skipping the atlas growth GPU test");
        return;
    };
    let config = RenderPipelineConfig {
        width: 16,
        height: 16,
        target_format: wgpu::TextureFormat::Rgba8Unorm,
        depth_format: Some(wgpu::TextureFormat::Depth32Float),
    };
    let renderer = BillboardRenderer::new(&device, &queue, &config);
    let layout = &renderer.texture_bind_group_layout;
    let images: Vec<image::RgbaImage> = (0..400u32)
        .map(|i| {
            let (w, h) = (3 + i * 37 % 90, 2 + i * 53 % 70);
            image::RgbaImage::from_fn(w, h, |x, y| {
                let v = (x * 31 + y * 17 + i * 7) as u8;
                image::Rgba([
                    v,
                    v.wrapping_mul(3),
                    i as u8,
                    if (x + y + i) % 5 == 0 { 0 } else { 255 },
                ])
            })
        })
        .collect();
    let offsets: Vec<(i16, i16)> = (0..400).map(|i| (i as i16 - 7, 3 - i as i16)).collect();
    let mut atlas = mandrake_render::create_atlas_from_images_with_offsets(
        &device,
        &queue,
        layout,
        &images[..150],
        &offsets[..150],
    );
    for n in [151, 230, 400] {
        atlas = mandrake_render::grow_atlas_from_images_with_offsets(
            &device,
            &queue,
            layout,
            &atlas,
            &images[..n],
            &offsets[..n],
        );
        let made = mandrake_render::create_atlas_from_images_with_offsets(
            &device,
            &queue,
            layout,
            &images[..n],
            &offsets[..n],
        );
        assert_eq!(
            (atlas.width, atlas.height),
            (made.width, made.height),
            "{n} bitmaps"
        );
        assert_eq!(atlas.frames, made.frames, "{n} bitmaps");
        let size = (made.width, made.height);
        assert!(
            read_texture(&device, &queue, &atlas.texture, size)
                == read_texture(&device, &queue, &made.texture, size),
            "{n} bitmaps: texels"
        );
    }
}
