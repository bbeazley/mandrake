mod support;

use support::gpu;

use mandrake_render::interface_scale::{InterfaceScaler, OffscreenTarget};
use mandrake_render::{OverlayDraw, OverlayLayer, OverlayRenderer};
use mandrake_view::interface_picture::InterfaceFilter;

fn noise(w: u32, h: u32) -> Vec<u8> {
    let mut seed = 0x1234_5678u32;
    let mut out = Vec::with_capacity((w * h * 4) as usize);
    for _ in 0..w * h {
        seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        let a = (seed >> 24) as u8 | 0x40;
        for _ in 0..3 {
            seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            out.push(((u32::from((seed >> 24) as u8) * u32::from(a)) / 255) as u8);
        }
        out.push(a);
    }
    out
}

#[test]
fn composite_content_matches_the_whole_composite() {
    let Some((device, queue)) = gpu("composite test device", wgpu::Features::empty()) else {
        eprintln!("Notice: no GPU adapter, skipping");
        return;
    };
    let (logical, window) = ((640, 360), (1600, 900));
    let format = wgpu::TextureFormat::Rgba8UnormSrgb;
    let mut overlay = OverlayRenderer::new(&device, format);
    let image = overlay.create_image(&device, &queue, 37, 23, &noise(37, 23));
    let draws = [
        OverlayDraw {
            dst: [0, 0, 640, 30],
            ..OverlayDraw::at(OverlayLayer::Dialog, &image, 0, 0, 0xff)
        },
        OverlayDraw::at(OverlayLayer::Dialog, &image, 610, 300, 0xff),
        OverlayDraw {
            dst: [200, 250, -37, 23],
            ..OverlayDraw::at(OverlayLayer::Dialog, &image, 0, 0, 0xc0)
        },
        OverlayDraw::at(OverlayLayer::Dialog, &image, 300, 120, 0xff),
        OverlayDraw::at(OverlayLayer::Dialog, &image, 700, 100, 0xff),
    ];
    let content: Vec<[i32; 4]> = draws.iter().map(|d| d.dst).collect();
    for filter in [
        InterfaceFilter::Sharp,
        InterfaceFilter::Nearest,
        InterfaceFilter::Linear,
        InterfaceFilter::Lanczos2,
    ] {
        let mut frames = Vec::new();
        for partial in [false, true] {
            let mut scaler = InterfaceScaler::new(&device, format);
            let target = OffscreenTarget::new(&device, format, window);
            let mut encoder = device.create_command_encoder(&Default::default());
            let world = overlay.create_image(&device, &queue, 61, 41, &noise(61, 41));
            let back = OverlayDraw {
                dst: [0, 0, window.0 as i32, window.1 as i32],
                ..OverlayDraw::at(OverlayLayer::Dialog, &world, 0, 0, 0xff)
            };
            target.clear(&mut encoder);
            overlay.encode(&device, &queue, &mut encoder, &target.view, window, &[back]);
            queue.submit([encoder.finish()]);
            let mut encoder = device.create_command_encoder(&Default::default());
            let layer = scaler.layer_view(&device, logical);
            overlay.encode_cleared(&device, &queue, &mut encoder, layer, logical, &draws);
            if partial {
                scaler.composite_content(
                    &queue,
                    &mut encoder,
                    &target.view,
                    window,
                    filter,
                    &content,
                );
            } else {
                scaler.composite(&queue, &mut encoder, &target.view, window, filter);
            }
            queue.submit([encoder.finish()]);
            frames.push(target.read(&device, &queue));
        }
        assert!(frames[0] == frames[1], "{filter:?}: the frames differ");
    }
}
