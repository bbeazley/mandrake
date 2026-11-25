mod support;

use support::gpu;

use mandrake_render::interface_scale::{
    InterfaceScaler, OffscreenTarget, set_lanczos_sharpen, upscale_offscreen,
};
use mandrake_view::interface_picture::InterfaceFilter;

fn noise(w: u32, h: u32) -> Vec<u8> {
    let mut seed = 0x1234_5678u32;
    let mut out = Vec::with_capacity((w * h * 4) as usize);
    for _ in 0..w * h {
        for _ in 0..3 {
            seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            out.push((seed >> 24) as u8);
        }
        out.push(0xff);
    }
    out
}

#[test]
fn fsr1_at_1x_is_the_identity() {
    let Some((device, queue)) = gpu("fsr1 test device", wgpu::Features::empty()) else {
        eprintln!("Notice: no GPU adapter, skipping");
        return;
    };
    let (w, h) = (64, 40);
    let picture = noise(w, h);
    for format in [
        wgpu::TextureFormat::Rgba8Unorm,
        wgpu::TextureFormat::Rgba8UnormSrgb,
        wgpu::TextureFormat::Bgra8UnormSrgb,
    ] {
        let out = upscale_offscreen(
            &device,
            &queue,
            format,
            &picture,
            (w, h),
            (w, h),
            InterfaceFilter::Fsr1,
        );
        let worst = out
            .iter()
            .zip(&picture)
            .map(|(a, b)| a.abs_diff(*b))
            .max()
            .unwrap();
        assert!(worst <= 1, "{format:?}: off by {worst}");
    }
}

fn edge(w: u32, h: u32, slope: f32, lo: u8, hi: u8) -> Vec<u8> {
    let mut out = Vec::with_capacity((w * h * 4) as usize);
    for y in 0..h {
        for x in 0..w {
            let mut cover = 0.0f32;
            for sy in 0..8 {
                for sx in 0..8 {
                    let px = x as f32 + (sx as f32 + 0.5) / 8.0;
                    let py = y as f32 + (sy as f32 + 0.5) / 8.0;
                    let at = w as f32 / 2.0 + (py - h as f32 / 2.0) * slope;
                    if px >= at {
                        cover += 1.0 / 64.0;
                    }
                }
            }
            let v = (lo as f32 + (hi as f32 - lo as f32) * cover).round() as u8;
            out.extend_from_slice(&[v, v, v, 0xff]);
        }
    }
    out
}

fn gradient_and_ringing(rows: &[u8], w: u32, h: u32, lo: u8, hi: u8) -> (f32, u32) {
    let g = |x: u32, y: u32| f32::from(rows[((y * w + x) * 4 + 1) as usize]);
    let (lo_f, hi_f) = (f32::from(lo), f32::from(hi));
    let (a, b) = (lo_f + 0.1 * (hi_f - lo_f), lo_f + 0.9 * (hi_f - lo_f));
    let crossing = |y: u32, level: f32| {
        (1..w).find_map(|x| {
            let (p, q) = (g(x - 1, y), g(x, y));
            (p < level && q >= level).then(|| (x - 1) as f32 + (level - p) / (q - p))
        })
    };
    let (mut width, mut rows_seen, mut ring) = (0.0f32, 0u32, 0.0f32);
    for y in h / 8..h - h / 8 {
        if let (Some(xa), Some(xb)) = (crossing(y, a), crossing(y, b)) {
            width += xb - xa;
            rows_seen += 1;
        }
        for x in 0..w {
            ring = ring.max(g(x, y) - hi_f).max(lo_f - g(x, y));
        }
    }
    assert!(rows_seen > 0, "no edge");
    (0.8 * (hi_f - lo_f) * rows_seen as f32 / width, ring as u32)
}

#[test]
fn an_upscaled_edge_is_steeper_than_lanczos2_with_no_more_ringing() {
    let Some((device, queue)) = gpu("fsr1 test device", wgpu::Features::empty()) else {
        eprintln!("Notice: no GPU adapter, skipping");
        return;
    };
    let (lo, hi) = (48u8, 200u8);
    let (w, h) = (40, 40);
    for (ow, oh, film_scale) in [
        (w * 3, h * 3, true),
        (w * 6, h * 6, true),
        (w * 5 / 2, h * 5 / 2, false),
    ] {
        for slope in [0.0f32, 0.3, 0.5, 1.0, 2.0] {
            let picture = edge(w, h, slope, lo, hi);
            let run = |filter| {
                let out = upscale_offscreen(
                    &device,
                    &queue,
                    wgpu::TextureFormat::Rgba8Unorm,
                    &picture,
                    (w, h),
                    (ow, oh),
                    filter,
                );
                gradient_and_ringing(&out, ow, oh, lo, hi)
            };
            let (fsr, fsr_ring) = run(InterfaceFilter::Fsr1);
            set_lanczos_sharpen(0.0);
            let (plain, plain_ring) = run(InterfaceFilter::Lanczos2);
            set_lanczos_sharpen(0.4);
            let (sharpened, sharpened_ring) = run(InterfaceFilter::Lanczos2);
            eprintln!(
                "{w}x{h} -> {ow}x{oh}, slope {slope}: gradient fsr1 {fsr:.1}, lanczos2 {plain:.1}, \
                 lanczos2 0.4 {sharpened:.1}; ringing fsr1 {fsr_ring}, lanczos2 {plain_ring}, \
                 lanczos2 0.4 {sharpened_ring}"
            );
            assert!(fsr > plain, "{ow}x{oh} slope {slope}: {fsr} <= {plain}");
            if slope >= 0.5 {
                assert!(
                    fsr > sharpened,
                    "{ow}x{oh} slope {slope}: {fsr} <= {sharpened}"
                );
            }
            assert!(
                fsr_ring <= plain_ring && (!film_scale || fsr_ring <= sharpened_ring),
                "{ow}x{oh} slope {slope}: fsr1 rings {fsr_ring}"
            );
        }
    }
}

#[test]
#[ignore = "needs a GPU; a measurement"]
fn fsr1_cost_at_4k() {
    const FRAMES: usize = 300;
    const SKIP: usize = 30;
    let features =
        wgpu::Features::TIMESTAMP_QUERY | wgpu::Features::TIMESTAMP_QUERY_INSIDE_ENCODERS;
    let (device, queue) = gpu("fsr1 test device", features).expect("a GPU");
    assert!(device.features().contains(features), "no timestamps");
    let (w, h) = (640u32, 264u32);
    let picture = noise(w, h);
    let window = (3840u32, 2160u32);
    let fit = (3840u32, 1584u32);
    let dst = [
        0,
        ((window.1 - fit.1) / 2) as i32,
        fit.0 as i32,
        fit.1 as i32,
    ];
    let set = device.create_query_set(&wgpu::QuerySetDescriptor {
        label: None,
        ty: wgpu::QueryType::Timestamp,
        count: 2,
    });
    let resolve = device.create_buffer(&wgpu::BufferDescriptor {
        label: None,
        size: 16,
        usage: wgpu::BufferUsages::QUERY_RESOLVE | wgpu::BufferUsages::COPY_SRC,
        mapped_at_creation: false,
    });
    let read = device.create_buffer(&wgpu::BufferDescriptor {
        label: None,
        size: 16,
        usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    let period = f64::from(queue.get_timestamp_period());
    for format in [
        wgpu::TextureFormat::Bgra8UnormSrgb,
        wgpu::TextureFormat::Bgra8Unorm,
    ] {
        let target = OffscreenTarget::new(&device, format, window);
        for (name, filter) in [
            ("fsr1", InterfaceFilter::Fsr1),
            ("lanczos2", InterfaceFilter::Lanczos2),
            ("sharp", InterfaceFilter::Sharp),
        ] {
            let mut scaler = InterfaceScaler::new(&device, format);
            let mut overlay = mandrake_render::OverlayRenderer::new(&device, format);
            let image = overlay.create_image(&device, &queue, w, h, &picture);
            let mut ms = Vec::new();
            for i in 0..FRAMES {
                let mut encoder = device.create_command_encoder(&Default::default());
                if i == 0 {
                    let draw = mandrake_render::OverlayDraw::at(
                        mandrake_render::OverlayLayer::Dialog,
                        &image,
                        0,
                        0,
                        0xff,
                    );
                    let layer = scaler.layer_view(&device, (w, h));
                    overlay.encode_cleared(&device, &queue, &mut encoder, layer, (w, h), &[draw]);
                }
                target.clear(&mut encoder);
                encoder.write_timestamp(&set, 0);
                scaler.composite_into(
                    &queue,
                    &mut encoder,
                    &target.view,
                    Some(dst),
                    window,
                    filter,
                );
                encoder.write_timestamp(&set, 1);
                encoder.resolve_query_set(&set, 0..2, &resolve, 0);
                encoder.copy_buffer_to_buffer(&resolve, 0, &read, 0, 16);
                queue.submit([encoder.finish()]);
                let slice = read.slice(..);
                slice.map_async(wgpu::MapMode::Read, |_| {});
                device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
                let bytes = slice.get_mapped_range().unwrap().to_vec();
                read.unmap();
                let t0 = u64::from_le_bytes(bytes[0..8].try_into().unwrap());
                let t1 = u64::from_le_bytes(bytes[8..16].try_into().unwrap());
                if i >= SKIP {
                    ms.push(t1.saturating_sub(t0) as f64 * period / 1e6);
                }
            }
            ms.sort_by(f64::total_cmp);
            let median = ms[ms.len() / 2];
            let p99 = ms[ms.len() * 99 / 100];
            println!(
                "{}x{} in {}x{} {format:?} {name}: gpu median {median:.3} ms ({:.1}% of 4 ms), p99 {p99:.3} ms",
                fit.0,
                fit.1,
                window.0,
                window.1,
                median / 4.0 * 100.0
            );
        }
    }
}
