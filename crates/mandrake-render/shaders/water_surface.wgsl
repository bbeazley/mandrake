fn surface_missing(in: WaterVertexOutput) -> bool {
    return in.level.y < 0.999;
}

fn surface_depth(in: WaterVertexOutput) -> f32 {
    return max(in.level.x - min_depth() - smooth_floor(water_grid(in.world_pos)), 0.0);
}

fn smooth_floor(cr_in: vec2<f32>) -> f32 {
    let dims = vec2<i32>(textureDimensions(t_floor));
    let last = dims - vec2<i32>(1);
    let cr = clamp(cr_in, vec2<f32>(0.0), vec2<f32>(last));
    let base = floor(cr);
    let wx = catmull_rom_weights(cr.x - base.x);
    let wy = catmull_rom_weights(cr.y - base.y);
    let first = vec2<i32>(base) - vec2<i32>(1);
    var floor_height = 0.0;
    for (var j = 0; j < 4; j++) {
        for (var i = 0; i < 4; i++) {
            let at = clamp(first + vec2<i32>(i, j), vec2<i32>(0), last);
            floor_height += wx[i] * wy[j] * textureLoad(t_floor, at, 0).x;
        }
    }
    return floor_height;
}

fn swash(world: vec3<f32>, footprint: f32) -> f32 {
    let amount = water.reflection_extra.z;
    if (amount <= 0.0) {
        return 0.0;
    }
    let cr = water_grid(world);
    let turns = (water.phase.x + cr.x * water.phase.y + cr.y * water.phase.z) / 65536.0;
    var h = water.wobble.x * sin(WATER_TAU * fract(turns)) * water.falloff.z;
    if (water.ripple.x > 0.5) {
        let dims = vec2<f32>(textureDimensions(t_ripple));
        let uv = (cr - water.ripple.yz) * water.ripple.w / dims;
        h += textureSampleLevel(t_ripple, s_diffuse, uv, 0.0).r;
    }
    if (water.fft.x > 0.5) {
        let k_p = fetch_peak(cr);
        var waves = 0.0;
        for (var c = 0; c < 3; c++) {
            let cascade = water.cascades[c];
            let keep = peak_weight(cascade.z, k_p);
            if (keep > 0.02) {
                let g = footprint * cascade.x;
                let t = textureSampleGrad(
                    t_waves,
                    s_waves,
                    world.xz * cascade.x,
                    c,
                    vec2<f32>(g, 0.0),
                    vec2<f32>(0.0, g),
                );
                waves += t.w * keep;
            }
        }
        h += waves / max(water.fetch.z, 1.0e-6);
    }
    return h * amount;
}

fn shore_depth_from_still(world: vec3<f32>, still: f32, footprint: f32) -> f32 {
    if (abs(still) > SHORE_BAND) {
        return still;
    }
    return still + swash(world, footprint);
}

fn shore_depth(world: vec3<f32>, level: f32, footprint: f32) -> f32 {
    let still = level - min_depth() - smooth_floor(water_grid(world));
    return shore_depth_from_still(world, still, footprint);
}

const SHORE_BAND: f32 = 0.15;

fn min_depth() -> f32 {
    return water.reflection_extra.w;
}

fn shore_ramp(in: WaterVertexOutput, width: f32) -> f32 {
    return smoothstep(0.0, max(width, 1.0e-4), surface_depth(in));
}

fn reflection_keep(in: WaterVertexOutput, n: vec3<f32>, v: vec3<f32>) -> f32 {
    let grazing = pow(1.0 - clamp(dot(n, v), 0.0, 1.0), water.surface.z);
    return mix(water.surface.y, 1.0, grazing) * shore_ramp(in, water.wobble.z);
}

@fragment
fn fs_reflection_keep(in: WaterVertexOutput) -> @location(0) vec4<f32> {
    let footprint = water_footprint(in.world_pos);
    if (surface_missing(in)) {
        discard;
    }
    let n = water_normal(in.world_pos, footprint).xyz;
    let v = normalize(scene.camera_pos.xyz - in.world_pos);
    return vec4<f32>(reflection_keep(in, n, v) * coverage_at(water_grid(in.world_pos)));
}

@fragment
fn fs_surface_under(in: WaterVertexOutput) -> @location(0) vec4<f32> {
    let body = lit_and_fogged(water_color_at(in.world_pos), in.clip_position, in.world_pos).rgb;
    let footprint = water_footprint(in.world_pos);
    if (surface_missing(in)) {
        discard;
    }
    let normal = water_normal(in.world_pos, footprint);
    let v = normalize(scene.camera_pos.xyz - in.world_pos);
    let shore = smoothstep(0.0, max(water.wobble.w, 1.0e-4), surface_depth(in));
    let cover = coverage_at(water_grid(in.world_pos));
    let alpha = select(water.surface.x, 0.0, water.extinction.w > 0.5) * shore * cover;
    let sheen = surface_sheen(in, normal, v, shore, footprint) * cover;
    return vec4<f32>(water_out(body) * alpha + water_out(sheen), alpha);
}

fn surface_sheen(
    in: WaterVertexOutput,
    normal: vec4<f32>,
    v: vec3<f32>,
    shore: f32,
    footprint: f32,
) -> vec3<f32> {
    let n = normal.xyz;
    let h = normalize(water.light.xyz + v);
    let shininess = water.light_color.w / (1.0 + water.light_color.w * normal.w);
    let lobe = pow(clamp(dot(n, h), 0.0, 1.0), shininess);
    let smooth_part = lobe * (shininess + 8.0) / (water.light_color.w + 8.0);
    var spec = water.light_color.rgb * water.light.w * shore
        * (smooth_part + 4.0 * water.glint.x * glint_points(in.world_pos, footprint, lobe));
    if (scene.fog.a != 0.0 || scene.fog_height.x != 0.0) {
        spec *= height_fog_factor(in.clip_position, in.world_pos);
    }
    return srgb_to_linear(clamp(spec, vec3<f32>(0.0), vec3<f32>(1.0)));
}

const GLINT_CELL: f32 = 0.4;

fn glint_points(world: vec3<f32>, footprint: f32, lobe: f32) -> f32 {
    if (water.glint.x <= 0.0 || water.light.w <= 0.0 || lobe <= 1.0e-4) {
        return 0.0;
    }
    let per_facet = max(footprint / max(water.glint.y, 1.0e-5), 1.0);
    let glinting = per_facet * per_facet * lobe;
    let fringe = 1.0 - smoothstep(0.3, 1.5, glinting);
    if (fringe <= 0.0) {
        return 0.0;
    }
    let cell = GLINT_CELL / max(water.fetch.z, 1.0e-6);
    let wanted = water.reflection_extra.x * footprint;
    let radius = min(wanted, 0.3 * cell);
    let base = floor(world.xz / cell);
    let life = water.glint.w / max(water.glint.z, 0.05) + fog_hash(base);
    let cycle = floor(life) % 997.0;
    let age = fract(life);
    let h1 = fog_hash(base + vec2<f32>(cycle * 7.13, cycle * 3.71));
    let h2 = fog_hash(base.yx + vec2<f32>(cycle * 1.97 + 5.3, cycle * 4.41 + 1.1));
    let h3 = fog_hash(base + vec2<f32>(cycle * 2.71 + 9.2, cycle * 0.37 + 3.3));
    let inner = max(1.0 - 2.0 * radius / cell, 0.0);
    let point = (base + vec2<f32>(0.5) + (vec2<f32>(h1, h2) - vec2<f32>(0.5)) * inner) * cell;
    let d = distance(world.xz, point) / max(radius, 1.0e-6);
    let disc = pow(clamp(1.0 - d * d, 0.0, 1.0), 2.0);
    let cell_px = cell / max(footprint, 1.0e-6);
    let chance = 1.0 - exp(-0.003 * glinting * cell_px * cell_px);
    let on = 1.0 - smoothstep(chance - 0.05, chance + 0.05, h3);
    let fade = sin(3.14159265 * age);
    let coverage = min(radius / max(wanted, 1.0e-6), 1.0);
    return disc * on * fade * fade * fringe * coverage * coverage;
}

fn screen_uv(world: vec3<f32>) -> vec2<f32> {
    let clip = scene.view_proj * vec4<f32>(world, 1.0);
    let ndc = clip.xy / max(clip.w, 1.0e-6);
    return vec2<f32>(ndc.x * 0.5 + 0.5, 0.5 - ndc.y * 0.5);
}

fn reflected(in: WaterVertexOutput, normal: vec4<f32>) -> vec4<f32> {
    let dims = vec2<f32>(textureDimensions(t_reflections));
    let here = in.clip_position.xy / dims;
    let shift = vec3<f32>(normal.x, 0.0, normal.z) / max(normal.y, 0.2) * water.reflection.z;
    let moved = in.world_pos + shift;
    let info = grid_info(water_grid(moved));
    let over_water = info.z > 0.999 && info.y > info.x;
    let uv = select(here, here + screen_uv(moved) - screen_uv(in.world_pos), over_water);
    let radius = water.reflection.w * sqrt(max(normal.w, 0.0));
    let r = vec2<f32>(radius * dims.y / dims.x, radius);
    var sum = textureSampleLevel(t_reflections, s_diffuse, uv, 0.0);
    if (radius * dims.y > 0.75) {
        sum += textureSampleLevel(t_reflections, s_diffuse, uv + vec2<f32>(r.x, 0.3 * r.y), 0.0);
        sum += textureSampleLevel(t_reflections, s_diffuse, uv - vec2<f32>(0.3 * r.x, r.y), 0.0);
        sum += textureSampleLevel(t_reflections, s_diffuse, uv - vec2<f32>(r.x, 0.3 * r.y), 0.0);
        sum += textureSampleLevel(t_reflections, s_diffuse, uv + vec2<f32>(0.3 * r.x, r.y), 0.0);
        sum *= 0.2;
    }
    return sum;
}

@fragment
fn fs_surface(in: WaterVertexOutput) -> @location(0) vec4<f32> {
    let footprint = water_footprint(in.world_pos);
    if (surface_missing(in)) {
        discard;
    }
    let normal = water_normal(in.world_pos, footprint);
    let v = normalize(scene.camera_pos.xyz - in.world_pos);
    let cos_v = clamp(dot(normal.xyz, v), 0.0, 1.0);
    let fresnel = 0.02 + 0.98 * pow(1.0 - cos_v, 5.0);
    let keep = clamp(fresnel * water.reflection.y, 0.0, 1.0);
    let layer = reflected(in, normal);
    let cr = water_grid(in.world_pos);
    let still = in.level.x - min_depth() - smooth_floor(cr);
    let shore = smoothstep(0.0, max(water.wobble.w, 1.0e-4), max(still, 0.0));
    let sheen = water_out(surface_sheen(in, normal, v, shore, footprint));
    let wetted = smoothstep(0.0, 0.01, shore_depth_from_still(in.world_pos, still, footprint));
    let cover = coverage_at(cr) * wetted;
    return vec4<f32>(layer.rgb * keep + sheen, layer.a * keep) * cover;
}

@fragment
fn fs_wet(in: WaterVertexOutput) -> @location(0) vec4<f32> {
    let n = normalize(cross(dpdy(in.world_pos), dpdx(in.world_pos)));
    let footprint = water_footprint(in.world_pos);
    if (under_water(in) || in.level.y <= 0.0) {
        discard;
    }
    let depth = shore_depth(in.world_pos, in.level.x, footprint);
    let terms = wet_sand_terms(in, n, shore_wetness(in, depth));
    var add = terms.rgb;
    var keep = terms.a;
    var fog = 1.0;
    if (scene.fog.a != 0.0 || scene.fog_height.x != 0.0) {
        fog = height_fog_factor(in.clip_position, in.world_pos);
    }
    if (water.extinction.w > 0.5 && depth > 0.0) {
        let cover = coverage_at(water_grid(in.world_pos));
        let ray = normalize(in.world_pos - scene.camera_pos.xyz);
        let mu = max(-ray.y, 0.05);
        let path = depth * cover / mu;
        let column = water_column(in.world_pos - ray * path, path, mu);
        let t = dot(column[0], vec3<f32>(1.0 / 3.0));
        let water_share = smoothstep(0.0, 0.01, depth);
        add = mix(terms.rgb, column[1] * fog, water_share);
        keep = keep * mix(1.0, t, fog * water_share);
    }
    if (SCENE_ENCODE_SRGB) {
        return vec4<f32>(scene_encode_srgb(add), pow(clamp(keep, 0.0, 1.0), 1.0 / 2.2));
    }
    return vec4<f32>(add, keep);
}

@fragment
fn fs_depth_only(in: WaterVertexOutput) -> @location(0) vec4<f32> {
    if (surface_missing(in)) {
        discard;
    }
    return vec4<f32>(0.0);
}
