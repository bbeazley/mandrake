mod support;

use support::gpu;

use mandrake_render::interface_scale::{InterfaceScaler, OffscreenTarget};
use mandrake_render::water::WaterShading;
use mandrake_render::{
    BillboardRenderer, InterfaceFrame, InterfacePresenter, ModelMesh, ModelRenderer, ModelSubMesh,
    OverlayDraw, OverlayLayer, OverlayRenderer, PointLight, ReflectionBatch, RenderPipelineConfig,
    SceneUniforms, TerrainMesh, TerrainRenderer, TranslucentBatch, TranslucentTriangle,
    create_texture_from_rgba, sort_translucent_triangles_on_screen,
};
use mandrake_tag::font::Bitmap32;
use mandrake_view::billboard::{
    BILLBOARD_FLAG_MEDIA_EFFECT, BILLBOARD_FLAG_SUBMERGED, BillboardInstance,
};
use mandrake_view::camera::{Camera, Mat4};
use mandrake_view::interface_picture::{InterfaceFilter, InterfaceLayer, InterfacePicture};
use mandrake_view::landscape::PixelRect;
use mandrake_view::media::{MediaSurface, MediaWobble};
use mandrake_view::model::ModelVertex;
use mandrake_view::model_surfaces::ReflectionTriangle;
use mandrake_view::water::settings::WaterSettings;
use mandrake_view::water::synthetic_mesh;

fn pixel_at(rgba: &[u8], width: u32, x: u32, y: u32) -> [u8; 4] {
    let i = ((y * width + x) * 4) as usize;
    [rgba[i], rgba[i + 1], rgba[i + 2], rgba[i + 3]]
}

fn assert_pixel_near(got: [u8; 4], want: [u8; 4], tol: i32, msg: &str) {
    for ch in 0..4 {
        assert!(
            (got[ch] as i32 - want[ch] as i32).abs() <= tol,
            "{msg}: got {got:?}, want {want:?} (tol {tol})"
        );
    }
}

#[test]
fn interface_presenter_preserves_multi_pass_uniforms_and_z_order() {
    let Some((device, queue)) = gpu("b1 test", wgpu::Features::empty()) else {
        return;
    };
    let size = (64u32, 64u32);
    let format = wgpu::TextureFormat::Rgba8Unorm;
    let red = Bitmap32::new(16, 16, 0x00ff_0000);
    let green = Bitmap32::new(8, 8, 0x0000_ff00);
    let blue = Bitmap32::new(8, 8, 0x0000_00ff);
    let yellow = Bitmap32::new(16, 16, 0x00ff_ff00);

    for filter in [InterfaceFilter::Nearest, InterfaceFilter::Fsr1] {
        let target = OffscreenTarget::new(&device, format, size);
        let mut encoder = device.create_command_encoder(&Default::default());
        target.clear(&mut encoder);
        let mut frame = InterfaceFrame {
            device: &device,
            queue: &queue,
            encoder: &mut encoder,
            target: &target.view,
            format,
            size,
        };
        let mut presenter = InterfacePresenter::new(&frame);
        presenter.draw(
            &mut frame,
            InterfacePicture::Layers {
                layers: vec![
                    InterfaceLayer {
                        bitmap: &red,
                        dst: [0, 0, 16, 16],
                    },
                    InterfaceLayer {
                        bitmap: &green,
                        dst: [8, 8, 32, 32],
                    },
                    InterfaceLayer {
                        bitmap: &blue,
                        dst: [24, 24, 32, 32],
                    },
                    InterfaceLayer {
                        bitmap: &yellow,
                        dst: [40, 40, 16, 16],
                    },
                ],
                revision: 1,
            },
            filter,
        );
        queue.submit([encoder.finish()]);
        let px = target.read(&device, &queue);
        let tol = if filter == InterfaceFilter::Fsr1 {
            1
        } else {
            0
        };

        assert_pixel_near(pixel_at(&px, 64, 4, 4), [255, 0, 0, 255], 0, "red");
        assert_pixel_near(
            pixel_at(&px, 64, 12, 12),
            [0, 255, 0, 255],
            tol,
            "green over red",
        );
        assert_pixel_near(
            pixel_at(&px, 64, 30, 30),
            [0, 0, 255, 255],
            tol,
            "blue over green",
        );
        assert_pixel_near(
            pixel_at(&px, 64, 48, 48),
            [255, 255, 0, 255],
            0,
            "yellow over blue",
        );
        assert_pixel_near(pixel_at(&px, 64, 60, 4), [0, 0, 0, 255], 0, "background");
    }
}

#[test]
fn interface_scaler_clamps_overhanging_and_zero_viewports() {
    let Some((device, queue)) = gpu("b2 test", wgpu::Features::empty()) else {
        return;
    };
    let format = wgpu::TextureFormat::Rgba8Unorm;
    let size = (64u32, 64u32);
    let mut overlay = OverlayRenderer::new(&device, format);
    // Left half red, right half green.
    let mut rgba = vec![0u8; 16 * 16 * 4];
    for y in 0..16 {
        for x in 0..16 {
            let o = (y * 16 + x) * 4;
            if x < 8 {
                rgba[o..o + 4].copy_from_slice(&[255, 0, 0, 255]);
            } else {
                rgba[o..o + 4].copy_from_slice(&[0, 255, 0, 255]);
            }
        }
    }
    let img = overlay.create_image(&device, &queue, 16, 16, &rgba);

    for filter in [InterfaceFilter::Nearest, InterfaceFilter::Fsr1] {
        let mut scaler = InterfaceScaler::new(&device, format);
        let target = OffscreenTarget::new(&device, format, size);
        let mut encoder = device.create_command_encoder(&Default::default());
        target.clear(&mut encoder);
        let layer = scaler.layer_view(&device, (16, 16));
        let draw = OverlayDraw::at(OverlayLayer::Dialog, &img, 0, 0, 0xff);
        overlay.encode_cleared(&device, &queue, &mut encoder, layer, (16, 16), &[draw]);

        // Zero target, zero/negative dst, completely off-screen dst: all no-ops, no panic.
        scaler.composite_into(&queue, &mut encoder, &target.view, None, (0, 0), filter);
        scaler.composite_into(
            &queue,
            &mut encoder,
            &target.view,
            Some([0, 0, 0, 32]),
            size,
            filter,
        );
        scaler.composite_into(
            &queue,
            &mut encoder,
            &target.view,
            Some([-40, -40, 32, 32]),
            size,
            filter,
        );

        // Overhanging top-left: dst [-16, 0, 32, 32] puts the right (green) half into [0..16, 0..32].
        scaler.composite_into(
            &queue,
            &mut encoder,
            &target.view,
            Some([-16, 0, 32, 32]),
            size,
            filter,
        );
        // Overhanging bottom-right: dst [48, 32, 32, 32] puts the left (red) half into [48..64, 32..64].
        scaler.composite_into(
            &queue,
            &mut encoder,
            &target.view,
            Some([48, 32, 32, 32]),
            size,
            filter,
        );
        queue.submit([encoder.finish()]);
        let px = target.read(&device, &queue);

        let tol = if filter == InterfaceFilter::Fsr1 {
            1
        } else {
            0
        };
        assert_pixel_near(
            pixel_at(&px, 64, 8, 16),
            [0, 255, 0, 255],
            tol,
            "clipped left shows green right half",
        );
        assert_pixel_near(
            pixel_at(&px, 64, 56, 48),
            [255, 0, 0, 255],
            tol,
            "clipped right shows red left half",
        );
        assert_pixel_near(
            pixel_at(&px, 64, 32, 32),
            [0, 0, 0, 255],
            0,
            "untouched center",
        );
    }
}

#[test]
fn zero_sized_texture_uploads_do_not_panic() {
    let Some((device, queue)) = gpu("b7 test", wgpu::Features::empty()) else {
        return;
    };
    let config = RenderPipelineConfig::default();
    let billboards = BillboardRenderer::new(&device, &queue, &config);
    let empty_img = image::RgbaImage::new(0, 0);
    let (_tex, _view, _bg) = create_texture_from_rgba(
        &device,
        &queue,
        &billboards.texture_bind_group_layout,
        &empty_img,
        Some("Empty Sprite"),
    );

    let mut scaler = InterfaceScaler::new(&device, wgpu::TextureFormat::Rgba8Unorm);
    scaler.set_scroll_layer(&device, &queue, 0, 0, &[]);

    let models = ModelRenderer::new(&device, &queue, &config);
    let _bg = models.create_texture_bind_group(&device, &queue, &empty_img, Some("Empty Model"));

    let terrain = TerrainRenderer::new(&device, &queue, &config);
    let landscape = terrain.create_landscape_texture(&device, &queue, 4, 4, &[128u8; 4 * 4 * 4]);
    landscape.write_region(
        &queue,
        PixelRect {
            x: 0,
            y: 0,
            width: 0,
            height: 0,
        },
        &[],
    );
}

#[test]
fn model_reflection_zero_and_negative_water_height_lit_without_nan() {
    let Some((device, queue)) = gpu("b8 test", wgpu::Features::empty()) else {
        return;
    };
    let size = (32u32, 32u32);
    let format = wgpu::TextureFormat::Rgba8Unorm;
    let config = RenderPipelineConfig {
        width: size.0,
        height: size.1,
        target_format: format,
        depth_format: None,
    };
    let terrain = TerrainRenderer::new(&device, &queue, &config);
    let models = ModelRenderer::new(&device, &queue, &config);
    let white = image::RgbaImage::from_pixel(2, 2, image::Rgba([200, 200, 200, 255]));
    let tex_bg = models.create_texture_bind_group(&device, &queue, &white, Some("White"));

    let light = PointLight {
        pos: [0.0, 2.0, 0.5, 10.0],
        color: [1.0, 1.0, 1.0, 2.0],
    };
    let uniforms = SceneUniforms {
        view_proj: Mat4::IDENTITY.cols,
        camera_pos: [0.0, 2.0, 2.0, 1.0],
        ..Default::default()
    }
    .with_lighting([0.0, 0.0, 0.0], true, &[light]);
    terrain.update_uniforms(&queue, &uniforms);

    let (model_buf, model_bg) =
        models.create_model_bind_group(&device, &Mat4::IDENTITY.cols, &[1.0; 4]);
    let _ = model_buf;
    let dummy_mesh = ModelMesh::from_submeshes(
        &device,
        "B8 Dummy",
        &[ModelVertex::new([0.0; 3], [0.0, 1.0, 0.0], [0.0, 0.0])],
        &[0, 0, 0],
        vec![ModelSubMesh {
            index_offset: 0,
            index_count: 3,
            bitmap_index: Some(0),
            texture_bind_group: Some(tex_bg),
            translucent: false,
            two_sided: true,
            category2: false,
            material: Some(0),
        }],
    );

    for water_height in [0.0f32, -1.0f32] {
        let mut batch = ReflectionBatch::new(&device);
        let tri = ReflectionTriangle {
            instance: 0,
            submesh: 0,
            key: 1.0,
            surface: 0,
            vertices: [
                ModelVertex::new([-1.0, -1.0, 0.0], [0.0, water_height, 0.0], [0.0, 0.0]),
                ModelVertex::new([1.0, -1.0, 0.0], [0.0, water_height, 0.0], [1.0, 0.0]),
                ModelVertex::new([0.0, 1.0, 0.0], [0.0, water_height, 0.0], [0.5, 1.0]),
            ],
        };
        batch.prepare(&device, &queue, &[tri]);
        let target = OffscreenTarget::new(&device, format, size);
        let mut encoder = device.create_command_encoder(&Default::default());
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("B8 Reflection Pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &target.view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                occlusion_query_set: None,
                timestamp_writes: None,
                multiview_mask: None,
            });
            models.render_reflection_triangles(
                &mut pass,
                &terrain.uniform_bind_group,
                &batch,
                &[(&model_bg, &dummy_mesh)],
                0..1,
                None,
            );
        }
        queue.submit([encoder.finish()]);
        let px = target.read(&device, &queue);
        let center = pixel_at(&px, 32, 16, 16);
        assert!(
            center[0] > 40 && center[1] > 40 && center[2] > 40,
            "water_height={water_height}: expected lit non-NaN center pixel, got {center:?}"
        );
    }
}

#[test]
fn water_submerged_srgb_fallback_matches_non_srgb_target() {
    let Some((device, queue)) = gpu("b3 test", wgpu::Features::empty()) else {
        return;
    };
    let size = (64u32, 64u32);
    let mut mesh = synthetic_mesh(8, 8, |_, _| -1024);
    for cell in &mut mesh.cells {
        cell.flags = 0x1900;
        cell.media_height = 0;
    }
    let surface = MediaSurface {
        mesh_flags: 0,
        wobble: MediaWobble::default(),
    };
    let settings = WaterSettings {
        physical: true,
        wind_ripples: false,
        ripples: false,
        refraction: false,
        bed_blur: 0.0,
        wet_sand: false,
        ..Default::default()
    };

    let camera = Camera {
        target: [0.0, 0.0, 0.0],
        distance: 6.0,
        pitch: 1.2,
        ..Default::default()
    };
    let uniforms = SceneUniforms::from_camera(&camera, 1.0).with_viewport(size.0, size.1);

    let mut results = Vec::new();
    for display_format in [
        wgpu::TextureFormat::Rgba8Unorm,
        wgpu::TextureFormat::Rgba8UnormSrgb,
    ] {
        let config = RenderPipelineConfig {
            width: size.0,
            height: size.1,
            target_format: wgpu::TextureFormat::Rgba8UnormSrgb,
            depth_format: Some(wgpu::TextureFormat::Depth32Float),
        };
        let terrain = TerrainRenderer::new(&device, &queue, &config);
        terrain.update_uniforms(&queue, &uniforms);
        let mut billboards = BillboardRenderer::new(&device, &queue, &config);
        // Override interface_format to match the tested display_format (simulating sRGB fallback).
        billboards.interface_format = display_format;
        let terrain_mesh = TerrainMesh::from_mesh_tag(&device, &mesh, 1.0, Some(surface), None);
        let mut water = WaterShading::new(
            &device,
            &queue,
            &terrain,
            &mesh,
            1.0,
            surface,
            &settings,
            display_format,
            &billboards,
        );
        water.update_terrain(&device, &queue, &mesh, &terrain_mesh);
        water.ensure_reflection_layer(&device, size);

        // A mid-tone orange submerged media-effect quad just under the water at y = -0.25.
        let mut quad = BillboardInstance::world_quad(
            [
                [-3.0, -0.25, -3.0],
                [3.0, -0.25, -3.0],
                [3.0, -0.25, 3.0],
                [-3.0, -0.25, 3.0],
            ],
            [0.0, 0.0],
            [1.0, 1.0],
        )
        .with_tint([0.6, 0.4, 0.25, 1.0]);
        quad.flags |= BILLBOARD_FLAG_MEDIA_EFFECT | BILLBOARD_FLAG_SUBMERGED;
        let inst_buf = billboards.upload_instances(&device, &queue, &[quad]);

        let depth = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("B3 Depth"),
            size: wgpu::Extent3d {
                width: size.0,
                height: size.1,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Depth32Float,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            view_formats: &[],
        });
        let depth_view = depth.create_view(&Default::default());
        let target = OffscreenTarget::new(&device, display_format, size);

        let mut encoder = device.create_command_encoder(&Default::default());
        // Pass 1: bed depth into scene target (using a scratch scene target).
        let scene_target = OffscreenTarget::new(&device, config.target_format, size);
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("B3 Bed Pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &scene_target.view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
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
            water.render_bed(&mut pass, &terrain, &terrain_mesh, None);
        }
        // Pass 2: submerged category-4 billboard into water's submerged layer.
        let (sub_color, sub_height) = water.submerged_layer().expect("physical submerged layer");
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("B3 Submerged Pass"),
                color_attachments: &[
                    Some(wgpu::RenderPassColorAttachment {
                        view: sub_color,
                        depth_slice: None,
                        resolve_target: None,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                            store: wgpu::StoreOp::Store,
                        },
                    }),
                    Some(wgpu::RenderPassColorAttachment {
                        view: sub_height,
                        depth_slice: None,
                        resolve_target: None,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                            store: wgpu::StoreOp::Store,
                        },
                    }),
                ],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &depth_view,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Load,
                        store: wgpu::StoreOp::Store,
                    }),
                    stencil_ops: None,
                }),
                occlusion_query_set: None,
                timestamp_writes: None,
                multiview_mask: None,
            });
            water.render_category4_billboards(
                &mut pass,
                &billboards,
                &terrain.uniform_bind_group,
                None,
                &inst_buf,
                0..1,
                None,
                true,
            );
        }
        // Pass 3: water under-pass into `target` (display_format).
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("B3 Under Pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &target.view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &depth_view,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Load,
                        store: wgpu::StoreOp::Store,
                    }),
                    stencil_ops: None,
                }),
                occlusion_query_set: None,
                timestamp_writes: None,
                multiview_mask: None,
            });
            water.render_under(&mut pass, &terrain, &terrain_mesh, None);
        }
        queue.submit([encoder.finish()]);
        results.push(target.read(&device, &queue));
    }

    let c_unorm = pixel_at(&results[0], 64, 32, 32);
    let c_srgb = pixel_at(&results[1], 64, 32, 32);
    // Sanity check: the orange submerged sprite actually rendered at center.
    assert!(
        c_unorm[0] > 40,
        "expected visible submerged sprite, got {c_unorm:?}"
    );
    for ch in 0..3 {
        let diff = (c_unorm[ch] as i32 - c_srgb[ch] as i32).abs();
        assert!(
            diff <= 3,
            "sRGB fallback double-decoded channel {ch}: unorm={c_unorm:?} vs srgb={c_srgb:?}"
        );
    }
}

#[test]
fn overlay_batching_and_reusable_translucent_batch() {
    let Some((device, queue)) = gpu("p2/p5 test", wgpu::Features::empty()) else {
        return;
    };
    let size = (32u32, 32u32);
    let format = wgpu::TextureFormat::Rgba8Unorm;
    let mut overlay = OverlayRenderer::new(&device, format);
    let white = overlay.create_image(&device, &queue, 1, 1, &[255, 255, 255, 255]);
    let draws = [
        OverlayDraw::fill(OverlayLayer::Dialog, &white, [2, 2, 4, 4], [255, 0, 0], 255),
        OverlayDraw::fill(
            OverlayLayer::Dialog,
            &white,
            [10, 2, 4, 4],
            [0, 255, 0],
            255,
        ),
        OverlayDraw::fill(
            OverlayLayer::Dialog,
            &white,
            [18, 2, 4, 4],
            [0, 0, 255],
            255,
        ),
    ];
    let target = OffscreenTarget::new(&device, format, size);
    let mut encoder = device.create_command_encoder(&Default::default());
    target.clear(&mut encoder);
    overlay.encode(&device, &queue, &mut encoder, &target.view, size, &draws);
    queue.submit([encoder.finish()]);
    let px = target.read(&device, &queue);
    assert_eq!(pixel_at(&px, 32, 4, 4), [255, 0, 0, 255]);
    assert_eq!(pixel_at(&px, 32, 12, 4), [0, 255, 0, 255]);
    assert_eq!(pixel_at(&px, 32, 20, 4), [0, 0, 255, 255]);

    // P5: TranslucentBatch::prepare_sorted_on_screen matches sort_translucent_triangles_on_screen.
    let tris = [
        TranslucentTriangle {
            submesh: 0,
            indices: [0, 1, 2],
            positions: [[0.0, 0.0, -5.0], [1.0, 0.0, -5.0], [0.0, 1.0, -5.0]],
        },
        TranslucentTriangle {
            submesh: 1,
            indices: [3, 4, 5],
            positions: [[0.0, 0.0, -2.0], [1.0, 0.0, -2.0], [0.0, 1.0, -2.0]],
        },
    ];
    let vp = Mat4::IDENTITY;
    let list = sort_translucent_triangles_on_screen([(Mat4::IDENTITY.cols, &tris[..])], &vp, &vp);
    let mut batch_a = TranslucentBatch::new(&device);
    batch_a.prepare(&device, &queue, list);
    let mut batch_b = TranslucentBatch::new(&device);
    for _ in 0..3 {
        batch_b.prepare_sorted_on_screen(
            &device,
            &queue,
            [(Mat4::IDENTITY.cols, &tris[..])],
            &vp,
            &vp,
        );
    }
    assert_eq!(batch_a.runs(), batch_b.runs());
    assert_eq!(batch_a.keys(), batch_b.keys());
    assert_eq!(batch_a.screen_bounds(0..2), batch_b.screen_bounds(0..2));
}
