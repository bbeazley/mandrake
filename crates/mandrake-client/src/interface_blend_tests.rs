use crate::render_passes::{create_depth_view, encode_interface_pass};
use crate::tests::test_gpu;
use mandrake_render::{BillboardRenderer, RenderPipelineConfig, SceneUniforms, TerrainRenderer};
use mandrake_view::billboard::BillboardInstance;
use mandrake_view::selection::{
    PixelRect, TRANSPARENCY_SELECTED, blend_display_channel, overlay_alpha,
};

const SIZE: u32 = 4;
/// Ground under the frame in reference_select_2.png (TASK-039-SELBOX, deviations).
const BACKGROUND: [u8; 3] = [80, 69, 34];

/// Renders a `SIZE`x`SIZE` interface rectangle of `color` (8-bit RGB, alpha `alpha`) over
/// `BACKGROUND` into an `Rgba8UnormSrgb` target through `encode_interface_pass`, with the
/// interface pass viewing the target as `interface_format`. Returns the top-left pixel, or
/// `None` when no GPU adapter is available.
fn render_blend(
    color: [u8; 3],
    alpha: u8,
    interface_format: wgpu::TextureFormat,
) -> Option<[u8; 4]> {
    let (device, queue) = test_gpu()?;

    let config = RenderPipelineConfig {
        width: SIZE,
        height: SIZE,
        target_format: wgpu::TextureFormat::Rgba8UnormSrgb,
        depth_format: Some(wgpu::TextureFormat::Depth32Float),
    };
    // The terrain renderer owns the scene uniform bind group the billboard passes use.
    let terrain = TerrainRenderer::new(&device, &queue, &config);
    terrain.update_uniforms(&queue, &SceneUniforms::default().with_viewport(SIZE, SIZE));
    let renderer =
        BillboardRenderer::with_interface_format(&device, &queue, &config, interface_format);

    let target = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("Blend Test Target"),
        size: wgpu::Extent3d {
            width: SIZE,
            height: SIZE,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8UnormSrgb,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[wgpu::TextureFormat::Rgba8Unorm],
    });
    // Clear through the non-sRGB view so the stored background bytes are exact.
    let raw_view = target.create_view(&wgpu::TextureViewDescriptor {
        format: Some(wgpu::TextureFormat::Rgba8Unorm),
        ..Default::default()
    });
    let interface_view = target.create_view(&wgpu::TextureViewDescriptor {
        format: Some(interface_format),
        ..Default::default()
    });
    let depth_view = create_depth_view(&device, SIZE, SIZE);

    let to_unit = |c: u8| c as f32 / 255.0;
    let billboards = [BillboardInstance::interface_rect(
        [0.0, 0.0, 0.5],
        PixelRect {
            left: 0,
            top: 0,
            right: SIZE as i32,
            bottom: SIZE as i32,
        },
        [
            to_unit(color[0]),
            to_unit(color[1]),
            to_unit(color[2]),
            to_unit(alpha),
        ],
    )];
    let billboard_buf = BillboardRenderer::create_instance_buffer(&device, &billboards);

    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
    {
        let _clear = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("Blend Test Clear"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                depth_slice: None,
                view: &raw_view,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color {
                        r: to_unit(BACKGROUND[0]) as f64,
                        g: to_unit(BACKGROUND[1]) as f64,
                        b: to_unit(BACKGROUND[2]) as f64,
                        a: 1.0,
                    }),
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                view: &depth_view,
                depth_ops: Some(wgpu::Operations {
                    load: wgpu::LoadOp::Clear(1.0),
                    store: wgpu::StoreOp::Store,
                }),
                stencil_ops: None,
            }),
            occlusion_query_set: None,
            timestamp_writes: None,
            multiview_mask: None,
        });
    }
    encode_interface_pass(
        &mut encoder,
        &interface_view,
        &depth_view,
        &renderer,
        &terrain.uniform_bind_group,
        &billboard_buf,
        &[core::ops::Range {
            start: 0,
            end: billboards.len() as u32,
        }],
    );

    let bytes_per_row = wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
    let readback = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("Blend Test Readback"),
        size: (bytes_per_row * SIZE) as u64,
        usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    encoder.copy_texture_to_buffer(
        wgpu::TexelCopyTextureInfo {
            texture: &target,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        wgpu::TexelCopyBufferInfo {
            buffer: &readback,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(bytes_per_row),
                rows_per_image: Some(SIZE),
            },
        },
        wgpu::Extent3d {
            width: SIZE,
            height: SIZE,
            depth_or_array_layers: 1,
        },
    );
    queue.submit(std::iter::once(encoder.finish()));

    let slice = readback.slice(..);
    slice.map_async(wgpu::MapMode::Read, |_| {});
    let _ = device.poll(wgpu::PollType::wait_indefinitely());
    let data = slice.get_mapped_range().unwrap();
    Some([data[0], data[1], data[2], data[3]])
}

#[test]
fn test_interface_pass_blends_on_display_values() {
    let alpha = overlay_alpha(TRANSPARENCY_SELECTED);
    assert_eq!(alpha, 0xaa);
    let frame = [0xff, 0xff, 0x00];
    let Some(px) = render_blend(frame, alpha, wgpu::TextureFormat::Rgba8Unorm) else {
        eprintln!("no GPU adapter: skipping interface blend GPU test");
        return;
    };
    let expected: Vec<u8> = (0..3)
        .map(|c| blend_display_channel(frame[c], BACKGROUND[c], alpha))
        .collect();
    // Engine result for #ffff00 @ 0xaa over (80,69,34).
    assert_eq!(expected, vec![197, 193, 11]);
    for c in 0..3 {
        assert!(
            (px[c] as i32 - expected[c] as i32).abs() <= 1,
            "channel {c}: got {px:?}, engine {expected:?}"
        );
    }

    // The old path (sRGB view, linear-space blend) is measurably off, so this test
    // discriminates between the two.
    let srgb = render_blend(frame, alpha, wgpu::TextureFormat::Rgba8UnormSrgb)
        .expect("adapter available above");
    assert!(
        srgb[0] as i32 - expected[0] as i32 >= 10,
        "sRGB blend {srgb:?} vs engine {expected:?}"
    );
}
