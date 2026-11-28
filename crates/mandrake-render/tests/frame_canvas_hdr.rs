mod support;

use mandrake_render::FrameCanvas;
use support::gpu;

fn f16_to_f32(bits: u16) -> f32 {
    let sign = u32::from((bits >> 15) & 1);
    let exp = u32::from((bits >> 10) & 0x1f);
    let mant = u32::from(bits & 0x3ff);
    let f_bits = match exp {
        0 => {
            if mant == 0 {
                sign << 31
            } else {
                let mut m = mant;
                let mut e = 127 - 14;
                while (m & 0x400) == 0 {
                    m <<= 1;
                    e -= 1;
                }
                (sign << 31) | (e << 23) | ((m & 0x3ff) << 13)
            }
        }
        0x1f => (sign << 31) | (0xff << 23) | (mant << 13),
        _ => (sign << 31) | ((exp + 127 - 15) << 23) | (mant << 13),
    };
    f32::from_bits(f_bits)
}

fn read_rgba16f_pixel(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    texture: &wgpu::Texture,
) -> [f32; 4] {
    let bytes_per_row = (texture.width() * 8).next_multiple_of(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT);
    let buffer = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("FrameCanvas HDR Readback"),
        size: u64::from(bytes_per_row) * u64::from(texture.height()),
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let mut encoder = device.create_command_encoder(&Default::default());
    encoder.copy_texture_to_buffer(
        wgpu::TexelCopyTextureInfo {
            texture,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        wgpu::TexelCopyBufferInfo {
            buffer: &buffer,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(bytes_per_row),
                rows_per_image: Some(texture.height()),
            },
        },
        wgpu::Extent3d {
            width: texture.width(),
            height: texture.height(),
            depth_or_array_layers: 1,
        },
    );
    queue.submit([encoder.finish()]);
    let slice = buffer.slice(..);
    let (tx, rx) = std::sync::mpsc::channel();
    slice.map_async(wgpu::MapMode::Read, move |res| {
        let _ = tx.send(res);
    });
    let _ = device.poll(wgpu::PollType::wait_indefinitely());
    rx.recv().unwrap().unwrap();
    let data = slice.get_mapped_range().unwrap();
    let mut px = [0.0f32; 4];
    for (i, ch) in px.iter_mut().enumerate() {
        let lo = data[i * 2];
        let hi = data[i * 2 + 1];
        *ch = f16_to_f32(u16::from_le_bytes([lo, hi]));
    }
    drop(data);
    buffer.unmap();
    px
}

#[test]
fn frame_canvas_hdr_decodes_gamma_22_and_sdr_passes_through() {
    let Some((device, queue)) = gpu("frame canvas hdr test", wgpu::Features::empty()) else {
        eprintln!("Notice: no GPU adapter, skipping");
        return;
    };

    let format = wgpu::TextureFormat::Rgba16Float;
    let size = (4, 4);
    let canvas = FrameCanvas::new(&device, format, size, Some(3.0));
    let surface = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("HDR Surface Target"),
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
    let canvas_view = canvas.texture().create_view(&Default::default());
    let surface_view = surface.create_view(&Default::default());

    let present = |canvas: &FrameCanvas| {
        let mut encoder = device.create_command_encoder(&Default::default());
        {
            let _clear = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("Fill Canvas"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    depth_slice: None,
                    view: &canvas_view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color {
                            r: 0.5,
                            g: 0.0,
                            b: 1.0,
                            a: 1.0,
                        }),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                occlusion_query_set: None,
                timestamp_writes: None,
                multiview_mask: None,
            });
        }
        canvas.encode_present(&mut encoder, &surface_view);
        queue.submit([encoder.finish()]);
        read_rgba16f_pixel(&device, &queue, &surface)
    };

    let hdr_3 = present(&canvas);
    let expected_mid = 0.5f32.powf(2.2) * 3.0;
    assert!(
        (hdr_3[0] - expected_mid).abs() < 0.005,
        "expected mid-gray ~{expected_mid}, got {}",
        hdr_3[0]
    );
    assert_eq!(hdr_3[1], 0.0);
    assert!(
        (hdr_3[2] - 3.0).abs() < 0.01,
        "expected 3.0, got {}",
        hdr_3[2]
    );
    assert_eq!(hdr_3[3], 1.0);

    canvas.set_hdr(&queue, Some(2.0));
    let hdr_2 = present(&canvas);
    let expected_mid_2 = 0.5f32.powf(2.2) * 2.0;
    assert!(
        (hdr_2[0] - expected_mid_2).abs() < 0.005,
        "expected mid-gray ~{expected_mid_2}, got {}",
        hdr_2[0]
    );
    assert!((hdr_2[2] - 2.0).abs() < 0.01);

    canvas.set_hdr(&queue, None);
    let sdr = present(&canvas);
    assert!((sdr[0] - 0.5).abs() < 0.005, "expected 0.5, got {}", sdr[0]);
    assert_eq!(sdr[1], 0.0);
    assert!((sdr[2] - 1.0).abs() < 0.005);
    assert_eq!(sdr[3], 1.0);
}
