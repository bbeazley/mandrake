fn water_color_at(surface_point: vec3<f32>) -> vec3<f32> {
    if (water.absorb.w < 0.5) {
        return water.water_color.rgb;
    }
    let dims = vec2<f32>(textureDimensions(t_floor)) - vec2<f32>(1.0);
    let uv = water_grid(surface_point) / dims;
    return textureSampleLevel(t_water_color, s_diffuse, uv, 0.0).rgb * water.water_color.a;
}

fn under_water(in: WaterVertexOutput) -> bool {
    return in.level.y > 0.999 && in.level.x > in.world_pos.y + WATER_EPSILON;
}

const WATER_EPSILON: f32 = 0.0078125;

fn bed_color(
    in: WaterVertexOutput,
    footprint: f32,
    terrain_normal: vec3<f32>,
    uv_dx: vec2<f32>,
    uv_dy: vec2<f32>,
    affine_uv: vec2<f32>,
    affine_dx: vec2<f32>,
    affine_dy: vec2<f32>,
) -> vec3<f32> {
    let drawn_depth = in.level.x - min_depth() - in.world_pos.y;
    var shore = drawn_depth;
    if (drawn_depth < SHORE_BAND) {
        shore = mix(
            shore_depth(in.world_pos, in.level.x, footprint),
            drawn_depth,
            smoothstep(0.5 * SHORE_BAND, SHORE_BAND, drawn_depth),
        );
    }
    let smooth_depth = select(0.0, max(shore, 0.0), in.level.y > 0.999);
    let depth = smooth_depth * coverage_at(water_grid(in.world_pos));
    let ray = normalize(in.world_pos - scene.camera_pos.xyz);
    let mu = max(-ray.y, 0.05);
    let path = depth / mu;
    let surface_point = in.world_pos - ray * path;
    let shift = refraction_shift(in, ray, surface_point, depth, footprint);
    let texel = landscape_color_grad(
        in.clip_position,
        in.uv + shift,
        uv_dx,
        uv_dy,
        affine_uv + shift,
        affine_dx,
        affine_dy,
    );
    let blurred = blurred_bed(in.uv + shift, path * water.reflection_extra.y * water.grid.z);
    let bed = mix(texel, blurred.rgb, blurred.a);
    var wet = 1.0;
    if (shore < 0.01 || in.level.y <= 0.999) {
        wet = shore_wetness(in, select(-1.0, shore, in.level.y > 0.999));
    }
    let wet_bed_linear = srgb_to_linear(bed) * (1.0 - water.wet.y * wet);
    var color: vec3<f32>;
    if (water.extinction.w > 0.5) {
        color = physical_bed_linear(wet_bed_linear, surface_point, path, mu);
    } else {
        color = tuned_bed(scene_encode_srgb(wet_bed_linear), surface_point, path, depth);
    }
    let under = lit_and_fogged_at(color, in.clip_position, in.world_pos, surface_point).rgb;
    let water_share = smoothstep(0.0, 0.01, depth);
    if (water_share >= 1.0) {
        return under;
    }
    let flats = wet_sand_on(in, terrain_normal, lit_and_fogged(bed, in.clip_position, in.world_pos).rgb, wet);
    return mix(flats, under, water_share);
}

fn shore_wetness(in: WaterVertexOutput, depth: f32) -> f32 {
    if (water.wet.y <= 0.0 && water.wet.z <= 0.0) {
        return 0.0;
    }
    let near = min(grid_info(water_grid(in.world_pos)).w / 0.9, 1.0);
    let side = 1.0 - smoothstep(0.0, max(water.wet.w, 1.0e-4), 2.0 * (1.0 - near));
    return (1.0 - smoothstep(0.0, max(water.wet.x, 1.0e-4), -depth)) * in.level.y * side;
}

fn wet_sand_terms(in: WaterVertexOutput, n: vec3<f32>, wet: f32) -> vec4<f32> {
    var fog = 1.0;
    if (scene.fog.a != 0.0 || scene.fog_height.x != 0.0) {
        fog = height_fog_factor(in.clip_position, in.world_pos);
    }
    let v = normalize(scene.camera_pos.xyz - in.world_pos);
    let h = normalize(water.light.xyz + v);
    let up = select(-n, n, n.y >= 0.0);
    let spec = water.light_color.rgb * water.light.w * water.wet.z
        * pow(clamp(dot(up, h), 0.0, 1.0), water.light_color.w) * wet * fog;
    return vec4<f32>(
        srgb_to_linear(clamp(spec, vec3<f32>(0.0), vec3<f32>(1.0))),
        1.0 - water.wet.y * wet * fog,
    );
}

fn wet_sand_on(in: WaterVertexOutput, n: vec3<f32>, lit: vec3<f32>, wet: f32) -> vec3<f32> {
    let terms = wet_sand_terms(in, n, wet);
    return lit * terms.a + terms.rgb;
}

fn blurred_bed(uv: vec2<f32>, radius: f32) -> vec4<f32> {
    let texels = vec2<f32>(textureDimensions(t_diffuse));
    let cells = vec2<f32>(textureDimensions(t_floor)) - vec2<f32>(1.0);
    let size = radius * texels.x / cells.x;
    if (size <= 0.5) {
        return vec4<f32>(0.0);
    }
    let r = vec2<f32>(radius) / cells;
    var sum = textureSampleLevel(t_diffuse, s_diffuse, uv, 0.0).rgb;
    sum += textureSampleLevel(t_diffuse, s_diffuse, uv + vec2<f32>(r.x, 0.4 * r.y), 0.0).rgb;
    sum += textureSampleLevel(t_diffuse, s_diffuse, uv - vec2<f32>(0.4 * r.x, r.y), 0.0).rgb;
    sum += textureSampleLevel(t_diffuse, s_diffuse, uv - vec2<f32>(r.x, 0.4 * r.y), 0.0).rgb;
    sum += textureSampleLevel(t_diffuse, s_diffuse, uv + vec2<f32>(0.4 * r.x, r.y), 0.0).rgb;
    return vec4<f32>(sum * 0.2, smoothstep(0.5, 2.0, size));
}

fn refraction_shift(
    in: WaterVertexOutput,
    ray: vec3<f32>,
    surface_point: vec3<f32>,
    depth: f32,
    footprint: f32,
) -> vec2<f32> {
    if (water.refraction.x < 0.5 || depth <= 0.0) {
        return vec2<f32>(0.0);
    }
    let n = water_normal(surface_point, footprint).xyz;
    var r = refract(ray, n, water.refraction.z);
    if (dot(r, r) < 1.0e-6) {
        r = ray;
    }
    let down = max(-r.y, 0.05);
    var t = depth / down;
    for (var i = 0; i < 2; i++) {
        let p = surface_point + r * t;
        let floor_height = grid_info(water_grid(p)).x;
        t = max(surface_point.y - floor_height, 0.0) / down;
    }
    let hit = surface_point + r * t;
    let dims = vec2<f32>(textureDimensions(t_floor)) - vec2<f32>(1.0);
    var cells = water_grid(hit) - water_grid(in.world_pos);
    cells = clamp(cells, vec2<f32>(-2.0), vec2<f32>(2.0));
    let fade = smoothstep(0.0, max(water.wobble.w, 1.0e-4), depth);
    return cells / dims * water.refraction.y * fade;
}

fn tuned_bed(bed: vec3<f32>, surface_point: vec3<f32>, path: f32, depth: f32) -> vec3<f32> {
    let t = vec3<f32>(1.0) - exp(-water.absorb.rgb * path);
    let floor_share = water.falloff.x * smoothstep(0.0, max(water.wobble.w, 1.0e-4), depth);
    let absorbed = mix(vec3<f32>(floor_share), vec3<f32>(water.falloff.y), t);
    return mix(bed, water_color_at(surface_point), absorbed);
}

fn cell_light_at(surface_point: vec3<f32>) -> vec3<f32> {
    let dims = vec2<f32>(textureDimensions(t_cell_light));
    let uv = water_grid(surface_point) / dims;
    return textureSampleLevel(t_cell_light, s_diffuse, uv, 0.0).rgb * 2.0;
}

fn physical_bed_linear(
    linear_bed: vec3<f32>,
    surface_point: vec3<f32>,
    path: f32,
    mu: f32,
) -> vec3<f32> {
    let water_part = water_column(surface_point, path, mu);
    return scene_encode_srgb(linear_bed * water_part[0] + water_part[1]);
}

fn physical_bed(bed: vec3<f32>, surface_point: vec3<f32>, path: f32, mu: f32) -> vec3<f32> {
    return physical_bed_linear(srgb_to_linear(bed), surface_point, path, mu);
}

fn water_column(surface_point: vec3<f32>, path: f32, mu: f32) -> array<vec3<f32>, 2> {
    let light = cell_light_at(surface_point);
    var tint = vec3<f32>(1.0);
    if (water.backscatter.w > 0.5) {
        let w = srgb_to_linear(water_color_at(surface_point));
        tint = w / max(max(w.r, max(w.g, w.b)), 1.0e-4);
    }
    let b = water.backscatter.rgb + water.particle.x * tint;
    let k = (water.extinction.rgb + b) * (1.0 + mu);
    let t = exp(-k * path);
    return array<vec3<f32>, 2>(t, light * b * 0.5 / k * (vec3<f32>(1.0) - t));
}

@fragment
fn fs_bed_media(in: WaterVertexOutput) -> @location(0) vec4<f32> {
    let uv_dx = dpdx(in.uv);
    let uv_dy = dpdy(in.uv);
    let affine_uv = in.affine.xy / in.affine.z;
    let affine_dx = dpdx(affine_uv);
    let affine_dy = dpdy(affine_uv);
    if (under_water(in)) {
        return vec4<f32>(0.0);
    }
    let texel = landscape_color_grad(
        in.clip_position,
        in.uv,
        uv_dx,
        uv_dy,
        affine_uv,
        affine_dx,
        affine_dy,
    );
    let c = lit_and_fogged(texel, in.clip_position, in.world_pos).rgb;
    return scene_out(vec4<f32>(c, 1.0));
}

@fragment
fn fs_bed_dry(in: WaterVertexOutput) -> @location(0) vec4<f32> {
    if (!under_water(in)) {
        discard;
    }
    return vec4<f32>(0.0);
}

fn water_out(linear: vec3<f32>) -> vec3<f32> {
    if (water.surface.w < 0.5) {
        return linear;
    }
    return scene_encode_srgb(linear);
}

@fragment
fn fs_bed_under(in: WaterVertexOutput) -> @location(0) vec4<f32> {
    let world_dx = dpdx(in.world_pos);
    let world_dy = dpdy(in.world_pos);
    let uv_dx = dpdx(in.uv);
    let uv_dy = dpdy(in.uv);
    let affine_uv = in.affine.xy / in.affine.z;
    let affine_dx = dpdx(affine_uv);
    let affine_dy = dpdy(affine_uv);
    if (!under_water(in)) {
        discard;
    }
    let footprint = max(length(world_dx), length(world_dy));
    let terrain_normal = normalize(cross(world_dy, world_dx));
    let c = bed_color(
        in,
        footprint,
        terrain_normal,
        uv_dx,
        uv_dy,
        affine_uv,
        affine_dx,
        affine_dy,
    );
    return vec4<f32>(water_dithered(over_submerged(in, c), in.clip_position.xy), 1.0);
}

override WATER_DITHER_LEVELS: f32 = 255.0;

fn water_dither(frag: vec2<f32>) -> f32 {
    let p = floor(frag);
    let n = fract(52.9829189 * fract(dot(p, vec2<f32>(0.06711056, 0.00583715))));
    return (n - 0.5) / WATER_DITHER_LEVELS;
}

fn water_dithered(linear: vec3<f32>, frag: vec2<f32>) -> vec3<f32> {
    let d = vec3<f32>(water_dither(frag));
    if (water.surface.w < 0.5) {
        let code = clamp(scene_encode_srgb(linear) + d, vec3<f32>(0.0), vec3<f32>(1.0));
        return srgb_to_linear(code);
    }
    return water_out(linear) + d;
}

fn over_submerged(in: WaterVertexOutput, bed: vec3<f32>) -> vec3<f32> {
    if (water.extinction.w < 0.5) {
        return bed;
    }
    let dims = vec2<i32>(textureDimensions(t_submerged));
    let px = clamp(vec2<i32>(in.clip_position.xy), vec2<i32>(0), dims - vec2<i32>(1));
    let s = textureLoad(t_submerged, px, 0);
    if (s.a <= 0.004) {
        return bed;
    }
    let hy = textureLoad(t_submerged_height, px, 0);
    let height = hy.r / max(hy.g, 1.0e-4);
    let ray = normalize(in.world_pos - scene.camera_pos.xyz);
    let mu = max(-ray.y, 0.05);
    let depth = max(in.level.x - min_depth() - height, 0.0) * coverage_at(water_grid(in.world_pos));
    let path = depth / mu;
    let eye = scene.camera_pos.xyz;
    let point = eye + ray * ((height - eye.y) / min(ray.y, -1.0e-4));
    let surface_point = point - ray * path;
    let colour = clamp(s.rgb / s.a, vec3<f32>(0.0), vec3<f32>(1.0));
    let linear_colour = select(colour, srgb_to_linear(colour), water.surface.w >= 0.5);
    var seen = physical_bed_linear(linear_colour, surface_point, path, mu);
    if (scene.fog_height.x != 0.0) {
        seen = mix(scene.fog.rgb, seen, height_fog_factor(in.clip_position, surface_point));
    }
    return mix(bed, srgb_to_linear(seen), clamp(s.a, 0.0, 1.0));
}
