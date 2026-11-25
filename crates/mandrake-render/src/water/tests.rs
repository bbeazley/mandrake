use super::*;
use mandrake_view::water::synthetic_mesh;

#[test]
fn water_shader_parses_and_validates() {
    let module = naga::front::wgsl::parse_str(WaterShading::SHADER_SRC)
        .unwrap_or_else(|e| panic!("{}", e.emit_to_string(WaterShading::SHADER_SRC)));
    naga::valid::Validator::new(
        naga::valid::ValidationFlags::all(),
        naga::valid::Capabilities::default(),
    )
    .validate(&module)
    .expect("water.wgsl validates");
}

#[test]
fn phase_steps_wrap_to_the_nearest_alias() {
    assert_eq!(wrapped_step(0x179 * 10), 3770.0);
    assert_eq!(wrapped_step(0x8000), -32768.0);
    assert_eq!(wrapped_step(0x10000 + 5), 5.0);
    assert_eq!(wrapped_step(0xffff), -1.0);
}

#[test]
fn light_fit_recovers_a_lambert_light() {
    let mesh = synthetic_mesh(32, 32, |c, r| {
        (300.0 * ((c as f32) * 0.5).sin() + 250.0 * ((r as f32) * 0.37).cos()) as i16
    });
    let light = normalized([0.5, 0.6, -0.4]);
    let (w, h) = (128u32, 128u32);
    let mut shade = vec![0u8; (w * h) as usize];
    let height = |c: usize, r: usize| {
        f32::from(mesh.cell_at(c.min(31), r.min(31)).unwrap().physical_height) / 512.0
    };
    for r in 0..32 {
        for c in 0..32 {
            let dcol =
                (height(c + 1, r) + height(c + 1, r + 1) - height(c, r) - height(c, r + 1)) * 0.5;
            let drow =
                (height(c, r + 1) + height(c + 1, r + 1) - height(c, r) - height(c + 1, r)) * 0.5;
            let len = (dcol * dcol + drow * drow + 1.0).sqrt();
            let n = [dcol / len, 1.0 / len, -drow / len];
            let lambert = n[0] * light[0] + n[1] * light[1] + n[2] * light[2];
            let v = (60.0 + 150.0 * lambert).round().clamp(0.0, 255.0) as u8;
            for y in r * 4..r * 4 + 4 {
                for x in c * 4..c * 4 + 4 {
                    shade[y * w as usize + x] = v;
                }
            }
        }
    }
    let fit = fit_light(&mesh, &shade, (w, h)).expect("a fit");
    assert!(fit.r2 > 0.95, "{fit:?}");
    let dot: f32 = (0..3).map(|i| fit.direction[i] * light[i]).sum();
    assert!(dot > 0.98, "{fit:?} vs {light:?}");
}
fn physical_bed_mirror(bed: [f32; 3], light: f32, tint: [f32; 3], depth: f32, mu: f32) -> [f32; 3] {
    let s = WaterSettings::default();
    let per = s.meters_per_cell;
    let lin = |c: f32| {
        if c <= 0.04045 {
            c / 12.92
        } else {
            ((c + 0.055) / 1.055).powf(2.4)
        }
    };
    let enc = |c: f32| {
        let c = c.clamp(0.0, 1.0);
        if c <= 0.0031308 {
            c * 12.92
        } else {
            1.055 * c.powf(1.0 / 2.4) - 0.055
        }
    };
    let path = depth / mu.max(0.05);
    std::array::from_fn(|k| {
        let a = (s.pure_absorption[k]
            + s.dissolved_absorption[k]
            + s.turbidity * s.turbidity_absorption)
            * per;
        let b = (PURE_BACKSCATTER[k] + s.turbidity * s.backscatter_ratio * tint[k]) * per;
        let kk = (a + b) * (1.0 + mu);
        let t = (-kk * path).exp();
        enc(lin(bed[k]) * t + light * b * 0.5 / kk * (1.0 - t))
    })
}

#[test]
fn the_physical_bed_is_the_dry_terrain_at_the_waterline_and_never_lighter() {
    let sand = [0.76, 0.66, 0.48];
    let tint = {
        let lin = |c: f32| {
            if c <= 0.04045 {
                c / 12.92
            } else {
                ((c + 0.055) / 1.055).powf(2.4)
            }
        };
        let w = [0.06f32, 0.12, 0.14].map(lin);
        let m = w[0].max(w[1]).max(w[2]);
        w.map(|v| v / m)
    };
    let at = |d: f32| physical_bed_mirror(sand, 0.71, tint, d, 0.7);
    let zero = at(0.0);
    for k in 0..3 {
        assert!((zero[k] - sand[k]).abs() < 1e-5, "{zero:?}");
    }
    let mut last = zero;
    for d in [0.01, 0.05, 0.1, 0.3, 1.0, 5.0] {
        let c = at(d);
        for k in 0..3 {
            assert!(
                c[k] <= last[k] + 1e-6,
                "lighter at depth {d}: {c:?} after {last:?}"
            );
        }
        last = c;
    }
}
#[test]
fn the_fetch_is_short_in_the_lee_and_long_downwind() {
    let (w, h) = (25u32, 5u32);
    let info: Vec<[f32; 4]> = (0..w * h)
        .map(|i| {
            if i % w == 0 {
                [0.0, 0.0, 0.0, 0.0]
            } else {
                [-1.0, 0.0, 1.0, 1.0]
            }
        })
        .collect();
    let map = ShelterMap::new(&info, (w, h), 1.589);
    let fetch_at = |f: &[u8], c: u32| ShelterMap::decode(f[((w + c) * 4) as usize]);
    let f = map.fetch(std::f64::consts::PI, 1.0);
    assert_eq!(f[(w * 4) as usize], 0, "dry");
    let (near, mid, far) = (fetch_at(&f, 1), fetch_at(&f, 4), fetch_at(&f, 12));
    assert!((near - 0.5 * 1.589).abs() < 0.2, "{near}");
    assert!(near < mid && mid < far, "{near} {mid} {far}");
    let g = map.fetch(0.0, 1.0);
    assert!(fetch_at(&g, 1) > 200.0);
    let n = map.fetch(0.0, 0.0);
    assert!(
        (fetch_at(&n, 3) - 3.0 * 1.589).abs() < 0.5,
        "{}",
        fetch_at(&n, 3)
    );
}
#[test]
fn the_fetch_sweep_is_the_march() {
    let (w, h) = (70u32, 50u32);
    let info: Vec<[f32; 4]> = (0..w * h)
        .map(|i| {
            let (c, r) = ((i % w) as f32, (i / w) as f32);
            let lake = ((c - 35.0) / 30.0).powi(2) + ((r - 25.0) / 20.0).powi(2) < 1.0;
            let channel = (r - 25.0).abs() < 3.0 && c > 50.0;
            let island = ((c - 30.0).powi(2) + (r - 20.0).powi(2)) < 16.0
                || ((c * 7.0 + r * 13.0) as u32).is_multiple_of(97);
            let spit = (c - 45.0).abs() < 1.0 && r > 30.0;
            if (lake || channel) && !island && !spit {
                [-1.0, 0.0, 1.0, 1.0]
            } else {
                [0.0, 0.0, 0.0, 0.0]
            }
        })
        .collect();
    let map = ShelterMap::new(&info, (w, h), 1.589);
    let march = |direction: f64, windiness: f64| {
        let reach = FETCH_MAX / map.meters_per_cell;
        let rays = ShelterMap::rays(direction);
        let total: f32 = rays.iter().map(|r| r.2).sum();
        let mut cells = vec![0.0f32; (w * h) as usize];
        for (i, cell) in cells.iter_mut().enumerate() {
            if !map.wet[i] {
                continue;
            }
            let (c, r) = ((i as u32 % w) as f32, (i as u32 / w) as f32);
            for &(dc, dr, weight) in &rays {
                let (mut k, mut n) = (0.0f32, 0.0f32);
                while n < reach {
                    k += 1.0;
                    let x = c + (k * dc + 0.5).floor();
                    let y = r + (k * dr + 0.5).floor();
                    if x < 0.0 || y < 0.0 || x >= w as f32 || y >= h as f32 {
                        n = reach;
                        break;
                    }
                    if !map.wet[(y as u32 * w + x as u32) as usize] {
                        break;
                    }
                    n += 1.0;
                }
                *cell += weight * n / total;
            }
        }
        map.encode(&cells, windiness)
    };
    let (mut off, mut wet) = (0usize, 0usize);
    for step in 0..72 {
        let direction = f64::from(step) * 5.0_f64.to_radians() + 0.01;
        for windiness in [1.0, 0.4] {
            let sweep = map.fetch(direction, windiness);
            assert!(
                sweep == march(direction, windiness),
                "{} deg, windiness {windiness}",
                step * 5
            );
            let old = map.fetch_march(direction, windiness);
            off += (0..sweep.len() / 4)
                .filter(|&i| sweep[i * 4] != old[i * 4])
                .count();
            wet += map.wet.iter().filter(|&&x| x).count();
        }
    }
    assert!(off * 200 < wet, "{off} of {wet}");
}
#[test]
fn squared_distance_is_exact() {
    let (w, h) = (23usize, 17usize);
    let seed: Vec<bool> = (0..w * h).map(|i| i % 37 == 5 || i == 200).collect();
    let d = squared_distance(&seed, (w, h));
    for (i, &got) in d.iter().enumerate() {
        let (c, r) = ((i % w) as f32, (i / w) as f32);
        let want = (0..w * h)
            .filter(|&j| seed[j])
            .map(|j| ((j % w) as f32 - c).powi(2) + ((j / w) as f32 - r).powi(2))
            .fold(f32::MAX, f32::min);
        assert_eq!(got, want, "{i}");
    }
}
#[test]
fn the_shared_light_is_near_every_fit() {
    let shared = WaterSettings::default().light_fallback.expect("a fallback");
    let d = light_from_angles(shared.0, shared.1);
    for fit in [
        [0.573f32, 0.553, -0.604],
        [0.621, 0.520, -0.587],
        [0.652, 0.264, -0.711],
    ] {
        let f = normalized(fit);
        let dot: f32 = (0..3).map(|i| d[i] * f[i]).sum();
        assert!(dot > 0.95, "{d:?} vs {f:?}: {dot}");
    }
    let g = light_from_angles(31.3, -136.6);
    assert!(
        (g[0] - 0.621).abs() < 0.01 && (g[2] + 0.587).abs() < 0.01,
        "{g:?}"
    );
}
#[test]
fn coverage_is_the_signed_distance_to_the_drawn_edge() {
    let info: Vec<[f32; 4]> = (0..9)
        .map(|i| {
            if i < 5 {
                [-1.0, 0.0, 1.0, 0.0]
            } else {
                [0.0, 0.0, 0.0, 0.0]
            }
        })
        .collect();
    let mesh = synthetic_mesh(8, 1, |_, _| 0);
    let sd: Vec<f32> = coverage(&info, (9, 1), &mesh)
        .iter()
        .map(|&g| f32::from(g) / 255.0 * 16.0 - 8.0)
        .collect();
    for (i, want) in [5.0, 4.0, 3.0, 2.0, 1.0, -1.0, -2.0, -3.0, -4.0]
        .iter()
        .enumerate()
    {
        assert!((sd[i] - want).abs() < 0.05, "{i}: {} vs {want}", sd[i]);
    }
}
#[test]
fn water_category4_shader_parses_and_validates() {
    let src = WaterCategory4::SHADER_SRC;
    let module =
        naga::front::wgsl::parse_str(src).unwrap_or_else(|e| panic!("{}", e.emit_to_string(src)));
    naga::valid::Validator::new(
        naga::valid::ValidationFlags::all(),
        naga::valid::Capabilities::default(),
    )
    .validate(&module)
    .expect("water_billboard.wgsl validates");
}

#[test]
fn water_vertex_shaders_only_use_group3_bindings_0_and_1() {
    let module = naga::front::wgsl::parse_str(WaterShading::SHADER_SRC)
        .unwrap_or_else(|e| panic!("{}", e.emit_to_string(WaterShading::SHADER_SRC)));
    let info = naga::valid::Validator::new(
        naga::valid::ValidationFlags::all(),
        naga::valid::Capabilities::default(),
    )
    .validate(&module)
    .expect("water.wgsl validates");
    for (ep_idx, ep) in module.entry_points.iter().enumerate() {
        if ep.stage != naga::ShaderStage::Vertex {
            continue;
        }
        let ep_info = info.get_entry_point(ep_idx);
        for (handle, var) in module.global_variables.iter() {
            if ep_info[handle].is_empty() {
                continue;
            }
            if let Some(binding) = &var.binding
                && binding.group == 3
            {
                assert!(
                    binding.binding <= 1,
                    "vertex entry {} uses group 3 binding {}",
                    ep.name,
                    binding.binding
                );
            }
        }
    }
}

#[test]
fn shelter_map_bounds_allocation_on_tiny_or_nan_meters_per_cell() {
    let (w, h) = (32u32, 24u32);
    let info: Vec<[f32; 4]> = (0..w * h)
        .map(|i| {
            if i % w == 0 {
                [0.0, 0.0, 0.0, 0.0]
            } else {
                [-1.0, 0.0, 1.0, 1.0]
            }
        })
        .collect();
    for mpc in [0.0f32, -1.0, 1e-6, f32::NAN, f32::INFINITY] {
        let map = ShelterMap::new(&info, (w, h), mpc);
        let max_expected = ((w as f64).hypot(h as f64).ceil() as usize) + 1;
        assert!(
            map.samples <= max_expected,
            "mpc={mpc}: samples={}",
            map.samples
        );
        assert!(
            map.padded.len() < 100_000,
            "mpc={mpc}: padded={}",
            map.padded.len()
        );
        let fetch = map.fetch(0.0, 1.0);
        assert_eq!(fetch.len(), (w * h * 4) as usize);
        assert_eq!(fetch[0], 0);
        assert!(fetch[4] > 0);
    }
}

#[test]
fn map_water_colors_fills_out_of_radius_cells_from_nearest_computed() {
    let mut mesh = synthetic_mesh(8, 6, |_, _| -512);
    let (cw, ch) = (8usize, 6usize);
    mesh.cells[cw + 1].flags = 0x0800;
    mesh.cells[cw + 1].media_height = 0;
    mesh.cells[cw + 6].flags = 0x1000;
    mesh.cells[cw + 6].media_height = 0;
    let mut rgba = vec![0u8; cw * ch * 4];
    for c in 0..cw {
        for r in 0..ch {
            let o = (r * cw + c) * 4;
            if (c, r) == (1, 1) {
                rgba[o..o + 4].copy_from_slice(&[20, 80, 160, 255]);
            } else if (c, r) == (6, 1) {
                rgba[o..o + 4].copy_from_slice(&[40, 140, 220, 255]);
            } else {
                rgba[o..o + 4].copy_from_slice(&[200, 180, 120, 255]);
            }
        }
    }
    let out = map_water_colors(&mesh, 0, &rgba, (cw as u32, ch as u32), 1.0).expect("colors");
    for r in 0..ch {
        for c in 0..cw {
            let o = (r * cw + c) * 4;
            assert_eq!(out[o + 3], 255, "unfilled cell ({c}, {r})");
            assert!(out[o..o + 3] != [0, 0, 0], "black cell ({c}, {r})");
        }
    }
    assert_eq!(&out[(cw + 3) * 4..(cw + 3) * 4 + 3], &[20, 80, 160]);
    assert_eq!(&out[(cw + 4) * 4..(cw + 4) * 4 + 3], &[40, 140, 220]);
    assert_eq!(&out[(5 * cw + 1) * 4..(5 * cw + 1) * 4 + 3], &[20, 80, 160]);
    assert_eq!(
        &out[(5 * cw + 6) * 4..(5 * cw + 6) * 4 + 3],
        &[40, 140, 220]
    );

    let dry = synthetic_mesh(4, 4, |_, _| 0);
    let dry_rgba = vec![128u8; 4 * 4 * 4];
    let dry_out = map_water_colors(&dry, 0, &dry_rgba, (4, 4), 1.0).expect("dry colors");
    assert!(dry_out.iter().all(|&b| b == 0));
}

#[test]
fn triangle_near_water_excludes_far_dry_triangles() {
    let (w, h) = (10u32, 10u32);
    let mut info = vec![[0.0f32; 4]; (w * h) as usize];
    info[w as usize + 1] = [-1.0, 0.0, 1.0, 1.0];
    let v = |c: u32, r: u32| TerrainVertex {
        position: [0.0; 3],
        uv: [c as f32 / (w - 1) as f32, r as f32 / (h - 1) as f32],
        cell_flags: 0,
    };
    let verts = [v(0, 0), v(1, 1), v(2, 2), v(7, 7), v(8, 7), v(8, 8)];
    assert!(triangle_near_water([0, 1, 2], &verts, (w, h), &info));
    assert!(!triangle_near_water([3, 4, 5], &verts, (w, h), &info));
}

#[test]
#[ignore = "needs the game data; run with --ignored --nocapture"]
fn stock_maps_report() {
    let ids: Vec<[u8; 4]> = match std::env::var("M2_WATER_BED_IDS") {
        Ok(v) => v
            .split(',')
            .filter_map(|s| s.as_bytes().try_into().ok())
            .collect(),
        Err(_) => vec![*b"neme", *b"85gi", *b"10la", *b"89sn"],
    };
    for id in &ids {
        let Some((mesh, tex, cov)) = mandrake_view::water::bed::stock_map(id) else {
            eprintln!("{}: not found", String::from_utf8_lossy(id));
            continue;
        };
        let t = std::time::Instant::now();
        let r = mandrake_view::water::bed::repaint_water_paint(
            &tex,
            &mesh,
            &cov,
            mandrake_view::water::bed::BED_SEED,
        );
        let ms = t.elapsed().as_secs_f64() * 1e3;
        eprintln!(
            "{} {}x{}: {:?} in {ms:.0} ms",
            String::from_utf8_lossy(id),
            tex.width,
            tex.height,
            r.as_ref().map(|r| &r.1)
        );
        if let (Ok(dir), Ok((new, _))) = (std::env::var("M2_WATER_BED_OUT"), &r) {
            for (name, ix) in [("before", &tex.color_index), ("after", new)] {
                let px: Vec<u8> = ix
                    .iter()
                    .flat_map(|&i| tex.palette16[i as usize].map(|v| (v >> 8) as u8))
                    .collect();
                image::RgbImage::from_raw(tex.width, tex.height, px)
                    .unwrap()
                    .save(format!("{dir}/{}-{name}.png", String::from_utf8_lossy(id)))
                    .unwrap();
            }
        }
    }
}
