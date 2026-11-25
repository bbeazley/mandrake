use mandrake_view::water::{WaterColorMode, WaterSettings};

pub static DEFAULT_WATER_TOML: std::sync::LazyLock<String> =
    std::sync::LazyLock::new(|| crate::conf::shipped_table(&["water"]));

fn number(item: &toml_edit::Item) -> Option<f32> {
    crate::conf::layered::number(item)
        .map(|v| v as f32)
        .filter(|v| v.is_finite())
}

fn triple(item: &toml_edit::Item) -> Option<[f32; 3]> {
    let v: Vec<f32> = item
        .as_array()?
        .iter()
        .filter_map(|v| {
            v.as_float()
                .map(|f| f as f32)
                .or_else(|| v.as_integer().map(|i| i as f32))
                .filter(|f| f.is_finite())
        })
        .collect();
    match v.as_slice() {
        &[x, y, z] => Some([x, y, z]),
        _ => None,
    }
}

pub(crate) fn merge(settings: &mut WaterSettings, text: &str, source: &str, mesh: Option<&str>) {
    let doc = match text.parse::<toml_edit::DocumentMut>() {
        Ok(doc) => doc,
        Err(e) => {
            tracing::warn!("{source}: not TOML, ignored: {e}");
            return;
        }
    };
    merge_table(settings, doc.as_table(), source);
    let Some(maps) = doc.get("map") else {
        return;
    };
    let Some(maps) = maps.as_table_like() else {
        tracing::warn!("{source}: `map` must be a table of [map.<mesh tag>] tables, ignored");
        return;
    };
    for (tag, item) in maps.iter() {
        let at = format!("{source} [map.{tag}]");
        let Some(table) = item.as_table() else {
            tracing::warn!("{at}: not a table, ignored");
            continue;
        };
        if tag.len() != 4 {
            tracing::warn!("{at}: a mesh tag has four characters, ignored");
            continue;
        }
        if mesh == Some(tag) {
            merge_table(settings, table, &at);
        } else {
            merge_table(&mut settings.clone(), table, &at);
        }
    }
}

fn merge_table(settings: &mut WaterSettings, table: &toml_edit::Table, source: &str) {
    for (key, item) in table.iter() {
        let ok = match key {
            "map" => continue,
            "color_mode" => match item.as_str() {
                Some("color") => {
                    settings.color_mode = WaterColorMode::Color;
                    true
                }
                Some("map") => {
                    settings.color_mode = WaterColorMode::Map;
                    true
                }
                _ => false,
            },
            "light_fallback" => match (item.as_str(), item.as_array()) {
                (Some("off"), _) => {
                    settings.light_fallback = None;
                    true
                }
                (_, Some(a)) if a.len() == 2 => {
                    let v: Vec<f32> = a
                        .iter()
                        .filter_map(|v| {
                            v.as_float()
                                .map(|f| f as f32)
                                .or_else(|| v.as_integer().map(|i| i as f32))
                                .filter(|f| f.is_finite())
                        })
                        .collect();
                    if let [e, az] = v[..] {
                        settings.light_fallback = Some((e, az));
                        true
                    } else {
                        false
                    }
                }
                _ => false,
            },
            "physical" | "scatter_tint" | "refraction" | "ripples" | "wet_sand"
            | "wind_ripples" | "gusts" | "shelter" | "river_bed" => match item.as_bool() {
                Some(v) => {
                    *match key {
                        "wet_sand" => &mut settings.wet_sand,
                        "river_bed" => &mut settings.river_bed,
                        "wind_ripples" => &mut settings.wind_ripples,
                        "gusts" => &mut settings.gusts,
                        "shelter" => &mut settings.shelter,
                        "physical" => &mut settings.physical,
                        "refraction" => &mut settings.refraction,
                        "ripples" => &mut settings.ripples,
                        _ => &mut settings.scatter_tint,
                    } = v;
                    true
                }
                None => false,
            },
            "pure_absorption" => triple(item).map(|v| settings.pure_absorption = v).is_some(),
            "dissolved_absorption" => triple(item)
                .map(|v| settings.dissolved_absorption = v)
                .is_some(),
            "water_color" => triple(item).map(|v| settings.water_color = v).is_some(),
            "absorption" => triple(item).map(|v| settings.absorption = v).is_some(),
            "light_direction" => triple(item)
                .map(|v| settings.light_direction = Some(v))
                .is_some(),
            _ => {
                let slot = match key {
                    "color_scale" => &mut settings.color_scale,
                    "color_radius" => &mut settings.color_radius,
                    "surface_opacity" => &mut settings.surface_opacity,
                    "shore_fade" => &mut settings.shore_fade,
                    "surface_fade" => &mut settings.surface_fade,
                    "shallow_tint" => &mut settings.shallow_tint,
                    "deep_max" => &mut settings.deep_max,
                    "wave_depth" => &mut settings.wave_depth,
                    "normal_strength" => &mut settings.normal_strength,
                    "swell" => &mut settings.swell,
                    "reflection_min" => &mut settings.reflection_min,
                    "fresnel_power" => &mut settings.fresnel_power,
                    "specular" => &mut settings.specular,
                    "shininess" => &mut settings.shininess,
                    "sheen_tint" => &mut settings.sheen_tint,
                    "light_fit_min_r2" => &mut settings.light_fit_min_r2,
                    "meters_per_cell" => &mut settings.meters_per_cell,
                    "turbidity" => &mut settings.turbidity,
                    "turbidity_absorption" => &mut settings.turbidity_absorption,
                    "backscatter_ratio" => &mut settings.backscatter_ratio,
                    "refraction_strength" => &mut settings.refraction_strength,
                    "ripple_strength" => &mut settings.ripple_strength,
                    "ripple_damping" => &mut settings.ripple_damping,
                    "wet_height" => &mut settings.wet_height,
                    "ripple_wind_scale" => &mut settings.ripple_wind_scale,
                    "ripple_calm" => &mut settings.ripple_calm,
                    "ripple_min_wavelength" => &mut settings.ripple_min_wavelength,
                    "ripple_max_wavelength" => &mut settings.ripple_max_wavelength,
                    "gust_scale" => &mut settings.gust_scale,
                    "gust_strength" => &mut settings.gust_strength,
                    "gust_speed" => &mut settings.gust_speed,
                    "glint_sparkle" => &mut settings.glint_sparkle,
                    "reflection_strength" => &mut settings.reflection_strength,
                    "reflection_distortion" => &mut settings.reflection_distortion,
                    "reflection_blur" => &mut settings.reflection_blur,
                    "glint_facet" => &mut settings.glint_facet,
                    "glint_lifetime" => &mut settings.glint_lifetime,
                    "glint_size" => &mut settings.glint_size,
                    "bed_blur" => &mut settings.bed_blur,
                    "shore_swash" => &mut settings.shore_swash,
                    "min_depth" => &mut settings.min_depth,
                    "wet_darken" => &mut settings.wet_darken,
                    "wet_specular" => &mut settings.wet_specular,
                    "wet_distance" => &mut settings.wet_distance,
                    _ => {
                        tracing::warn!("{source}: unknown key {key}, ignored");
                        continue;
                    }
                };
                number(item).map(|v| *slot = v).is_some()
            }
        };
        if !ok {
            tracing::warn!("{source}: bad value for {key}, ignored");
        }
    }
}

pub fn load(conf: Option<&std::path::Path>, headless: bool, mesh: &str) -> WaterSettings {
    let mut shipped = WaterSettings::default();
    merge(
        &mut shipped,
        &DEFAULT_WATER_TOML,
        "mandrake.default.toml",
        Some(mesh),
    );
    let (settings, _, diagnostics) = crate::conf::load_table(
        crate::conf::to_read(conf, headless),
        &["water"],
        || shipped,
        |text, source| {
            let mut settings = shipped;
            merge(&mut settings, text, source, Some(mesh));
            (settings, Vec::new())
        },
    );
    for d in &diagnostics {
        tracing::warn!("{d}");
    }
    settings
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shipped_file_is_the_render_default() {
        let mut s = WaterSettings::default();
        merge(&mut s, &DEFAULT_WATER_TOML, "shipped", None);
        assert_eq!(s, WaterSettings::default());
        for mesh in ["85tk", "01bp", "85tn0"] {
            let mut s = WaterSettings::default();
            merge(&mut s, &DEFAULT_WATER_TOML, "shipped", Some(mesh));
            assert_eq!(s, WaterSettings::default(), "{mesh}");
        }
    }

    #[test]
    fn tarn_is_tea_coloured() {
        for mesh in ["85tl", "85tn"] {
            let mut s = WaterSettings::default();
            merge(&mut s, &DEFAULT_WATER_TOML, "shipped", Some(mesh));
            let d = s.dissolved_absorption;
            assert!(d[2] > d[1] && d[1] > d[0] && d[0] >= 0.0, "{mesh}: {d:?}");
            assert_eq!(s.color_mode, WaterColorMode::Color, "{mesh}");
            let c = s.water_color;
            assert!(c[0] > c[1] && c[1] > c[2], "{mesh}: brown {c:?}");
        }
    }

    #[test]
    fn map_tables_override_in_order() {
        let shipped = "specular = 0.5\n[map.aaaa]\nspecular = 0.6\nturbidity = 2\n";
        let user = "turbidity = 3\n[map.aaaa]\nswell = 0.2\n[map.bbbb]\nswell = 0.9\n";
        let mut s = WaterSettings::default();
        merge(&mut s, shipped, "shipped", Some("aaaa"));
        merge(&mut s, user, "user", Some("aaaa"));
        assert_eq!((s.specular, s.turbidity, s.swell), (0.6, 3.0, 0.2));
        let mut s = WaterSettings::default();
        merge(&mut s, shipped, "shipped", Some("bbbb"));
        merge(&mut s, user, "user", Some("bbbb"));
        assert_eq!((s.specular, s.turbidity, s.swell), (0.5, 3.0, 0.9));
    }

    #[test]
    fn user_keys_override() {
        let mut s = WaterSettings::default();
        merge(
            &mut s,
            "specular = 0\nlight_direction = [0, 1, 0]\ncolor_mode = \"color\"\nphysical = false\nwind_ripples = false\ngusts = false\nshelter = false\nlight_fallback = \"off\"\n",
            "user",
            None,
        );
        assert_eq!(s.specular, 0.0);
        assert_eq!(s.light_direction, Some([0.0, 1.0, 0.0]));
        assert_eq!(s.color_mode, WaterColorMode::Color);
        assert!(!s.physical);
        assert!(!s.wind_ripples);
        assert!(!s.gusts);
        assert!(!s.shelter);
        assert_eq!(s.light_fallback, None);
    }
}

#[cfg(test)]
mod water_measurements {
    fn water_game() -> std::path::PathBuf {
        std::env::var_os("M2_WATER_GAME")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(crate::test_data::game_dir)
    }

    #[test]
    #[ignore]
    fn sandy_gimble_fetch() {
        let archive = &water_game().join("tags/small install");
        let map = crate::map_load::load_mesh_and_texture(
            archive,
            None,
            &crate::map_load::MeshQuery::from_arg("85gs"),
            &["Sandy Gimble"],
        )
        .expect("85gs");
        let mesh = &map.mesh_tag;
        let (cw, ch) = (usize::from(mesh.cell_width), usize::from(mesh.cell_height));
        let wet: Vec<bool> = mesh.cells.iter().map(|c| c.flags & 0x1800 != 0).collect();
        let media_flags = map
            .mesh_data
            .get(
                mandrake_view::media::MESH_HEADER_FLAGS
                    ..mandrake_view::media::MESH_HEADER_FLAGS + 4,
            )
            .map_or(0, |b| u32::from_be_bytes([b[0], b[1], b[2], b[3]]));
        let drawn: usize = mesh
            .cells
            .iter()
            .filter(|v| {
                mandrake_view::media::is_media_height_vertex(v.flags, v.model_index, media_flags)
            })
            .count();
        let deep: usize = mesh
            .cells
            .iter()
            .filter(|v| v.media_height > v.physical_height)
            .count();
        eprintln!(
            "MEASURE {cw}x{ch}: 0x1800 cells {}, media-height vertices {drawn}, media above floor {deep}",
            wet.iter().filter(|&&w| w).count()
        );
        let mut seen = vec![false; cw * ch];
        let mut sizes = Vec::new();
        for start in 0..cw * ch {
            if !wet[start] || seen[start] {
                continue;
            }
            let (mut stack, mut n, mut bb) = (vec![start], 0, (usize::MAX, usize::MAX, 0, 0));
            seen[start] = true;
            while let Some(i) = stack.pop() {
                n += 1;
                let (c, r) = (i % cw, i / cw);
                bb = (bb.0.min(c), bb.1.min(r), bb.2.max(c), bb.3.max(r));
                for (dc, dr) in [(1i64, 0i64), (-1, 0), (0, 1), (0, -1)] {
                    let (x, y) = (c as i64 + dc, r as i64 + dr);
                    if x >= 0 && y >= 0 && (x as usize) < cw && (y as usize) < ch {
                        let j = y as usize * cw + x as usize;
                        if wet[j] && !seen[j] {
                            seen[j] = true;
                            stack.push(j);
                        }
                    }
                }
            }
            sizes.push((n, bb));
        }
        sizes.sort_by_key(|s| std::cmp::Reverse(s.0));
        eprintln!(
            "MEASURE {} water bodies, largest {:?}",
            sizes.len(),
            &sizes[..sizes.len().min(8)]
        );
        let info = mandrake_view::water::grid_info_for(mesh, media_flags);
        let (w, h) = (
            u32::from(mesh.cell_width) + 1,
            u32::from(mesh.cell_height) + 1,
        );
        let wet_v = info.iter().filter(|v| v[2] > 0.5 && v[1] > v[0]).count();
        eprintln!("MEASURE drawn water vertices {wet_v}");
        let (wu, hu) = (w as usize, h as usize);
        let (mut deep_edge, mut dry_edge) = (0, 0);
        let mut deepest = 0.0f32;
        for r in 0..hu {
            for c in 0..wu {
                let v = info[r * wu + c];
                if v[2] < 0.5 {
                    continue;
                }
                let edge = [(1i64, 0i64), (-1, 0), (0, 1), (0, -1)]
                    .iter()
                    .any(|&(dc, dr)| {
                        let (x, y) = (c as i64 + dc, r as i64 + dr);
                        x >= 0
                            && y >= 0
                            && (x as usize) < wu
                            && (y as usize) < hu
                            && info[y as usize * wu + x as usize][2] < 0.5
                    });
                if edge {
                    if v[1] > v[0] + 0.0078125 {
                        deep_edge += 1;
                        deepest = deepest.max(v[1] - v[0]);
                    } else {
                        dry_edge += 1;
                    }
                }
            }
        }
        let mut near = Vec::new();
        for r in 113..137usize {
            for c in 119..143usize {
                let v = info[r * wu + c];
                let nb = info[r * wu + c + 1];
                if v[2] > 0.5 && nb[2] < 0.5 {
                    near.push((c, r, v[1] - v[0], nb[1] - nb[0]));
                }
            }
        }
        eprintln!(
            "MEASURE coverage edge vertices: {deep_edge} under the level (deepest {deepest:.3} cells), {dry_edge} at or above it; near the observer (c, r, depth, next vertex's floor below its own level?): {:?}",
            &near[..near.len().min(12)]
        );
        let shelter = mandrake_render::water::ShelterMap::new(&info, (w, h), 1.589);
        for (dir, windy) in [(62f64.to_radians(), 1.0), (0.0, 0.0)] {
            let started = std::time::Instant::now();
            let f = shelter.fetch(dir, windy);
            eprintln!(
                "MEASURE fetch recompute {:.3} ms",
                started.elapsed().as_secs_f64() * 1000.0
            );
            let edges = [1.0f32, 3.0, 10.0, 30.0, 100.0, 300.0];
            let mut hist = [0usize; 6];
            for (i, v) in info.iter().enumerate() {
                if v[2] > 0.5 && v[1] > v[0] {
                    let m = mandrake_render::water::ShelterMap::decode(f[i * 4]);
                    hist[edges.iter().position(|&e| m < e).unwrap_or(5)] += 1;
                }
            }
            eprintln!(
                "MEASURE wind {:.0} deg {windy}: fetch <1,<3,<10,<30,<100,more m {hist:?}",
                dir.to_degrees()
            );
            for (n, bb) in sizes.iter().rev().take(3) {
                let (c, r) = ((bb.0 + bb.2) / 2, (bb.1 + bb.3) / 2);
                let i = r * w as usize + c;
                eprintln!(
                    "MEASURE  body of {n} cells, vertex {:?}: fetch {:.1} m",
                    (c, r),
                    mandrake_render::water::ShelterMap::decode(f[i * 4])
                );
            }
        }
    }

    #[test]
    fn merge_rejects_non_finite_floats() {
        let mut settings = super::WaterSettings::default();
        let orig_abs = settings.pure_absorption;
        let orig_fb = settings.light_fallback;
        let orig_absorption = settings.absorption;
        super::merge(
            &mut settings,
            "absorption = nan\npure_absorption = [inf, 0.5, 0.5]\nlight_fallback = [-inf, 30]\n",
            "test",
            None,
        );
        assert_eq!(settings.absorption, orig_absorption);
        assert_eq!(settings.pure_absorption, orig_abs);
        assert_eq!(settings.light_fallback, orig_fb);
    }
}
