@group(1) @binding(0)
var t_diffuse: texture_2d<f32>;

@group(1) @binding(1)
var s_diffuse: sampler;

struct VertexInput {
    @location(0) position: vec3<f32>,
    @location(1) uv: vec2<f32>,
    @location(2) cell_flags: u32,
};

struct VertexOutput {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) affine: vec3<f32>,
    @location(2) world_pos: vec3<f32>,
};

@vertex
fn vs_main(in: VertexInput) -> VertexOutput {
    var out: VertexOutput;
    out.clip_position = scene.view_proj * vec4<f32>(in.position, 1.0);
    out.uv = in.uv;
    out.world_pos = in.position;
    out.affine = vec3<f32>(in.uv * out.clip_position.w, out.clip_position.w);
    return out;
}

const SAMPLING_SOFTWARE: u32 = 1u;
const SAMPLING_BICUBIC: u32 = 2u;
const SAMPLING_SHARP: u32 = 3u;

fn software_landscape_grad(
    clip_position: vec4<f32>,
    uv: vec2<f32>,
    uv_dx: vec2<f32>,
    uv_dy: vec2<f32>,
) -> vec3<f32> {
    let at = uv + uv_dx * (floor(clip_position.x / 2.0) * 2.0 - clip_position.x)
        - uv_dy * 0.5;
    let dims = vec2<i32>(textureDimensions(t_diffuse));
    let t = at * vec2<f32>(dims);
    let texel = vec2<i32>(floor(t));
    let frac = vec2<i32>(floor((t - floor(t)) * 4.0));
    var neighbour = array<vec2<i32>, 16>(
        vec2<i32>(-1, 0), vec2<i32>(0, -1), vec2<i32>(1, -1), vec2<i32>(0, -1),
        vec2<i32>(-1, -1), vec2<i32>(0, 0), vec2<i32>(0, 0), vec2<i32>(1, 0),
        vec2<i32>(-1, 0), vec2<i32>(0, 0), vec2<i32>(0, 0), vec2<i32>(1, 1),
        vec2<i32>(0, 1), vec2<i32>(-1, 1), vec2<i32>(0, 1), vec2<i32>(1, 0),
    );
    let last = dims - vec2<i32>(1);
    let a = textureLoad(t_diffuse, clamp(texel, vec2<i32>(0), last), 0).rgb;
    let n = neighbour[frac.y * 4 + frac.x];
    let b = textureLoad(t_diffuse, clamp(texel + n, vec2<i32>(0), last), 0).rgb;
    return (a + b) * 0.5;
}

fn software_landscape(clip_position: vec4<f32>, affine: vec3<f32>) -> vec3<f32> {
    let uv = affine.xy / affine.z;
    return software_landscape_grad(clip_position, uv, dpdx(uv), dpdy(uv));
}

const FILTER_FADE_END: f32 = 1.41421356;

fn catmull_rom(uv: vec2<f32>) -> vec3<f32> {
    let dims = vec2<i32>(textureDimensions(t_diffuse));
    let c = uv * vec2<f32>(dims) - vec2<f32>(0.5);
    let base = floor(c);
    let wx = catmull_rom_weights(c.x - base.x);
    let wy = catmull_rom_weights(c.y - base.y);
    let first = vec2<i32>(base) - vec2<i32>(1);
    let last = dims - vec2<i32>(1);
    var sum = vec3<f32>(0.0);
    for (var j = 0; j < 4; j++) {
        for (var i = 0; i < 4; i++) {
            let at = clamp(first + vec2<i32>(i, j), vec2<i32>(0), last);
            sum += wx[i] * wy[j] * textureLoad(t_diffuse, at, 0).rgb;
        }
    }
    return clamp(sum, vec3<f32>(0.0), vec3<f32>(1.0));
}

fn filtered_landscape_grad(
    mode: u32,
    uv: vec2<f32>,
    uv_dx: vec2<f32>,
    uv_dy: vec2<f32>,
    linear: vec3<f32>,
) -> vec3<f32> {
    let dims = vec2<f32>(textureDimensions(t_diffuse));
    let t = uv * dims;
    let t_dx = uv_dx * dims;
    let t_dy = uv_dy * dims;
    let rho = max(length(t_dx), length(t_dy));
    if (!(rho < FILTER_FADE_END)) {
        return linear;
    }
    var filtered: vec3<f32>;
    if (mode == SAMPLING_BICUBIC) {
        filtered = catmull_rom(uv);
    } else {
        let rate = vec2<f32>(length(vec2<f32>(t_dx.x, t_dy.x)), length(vec2<f32>(t_dx.y, t_dy.y)));
        filtered = textureSampleLevel(t_diffuse, s_diffuse, sharp_coord(t, rate) / dims, 0.0).rgb;
    }
    return mix(filtered, linear, clamp((rho - 1.0) / (FILTER_FADE_END - 1.0), 0.0, 1.0));
}

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    let color = landscape_color(in.clip_position, in.uv, in.affine);
    return scene_out(lit_and_fogged(color, in.clip_position, in.world_pos));
}

fn landscape_color(clip_position: vec4<f32>, uv: vec2<f32>, affine: vec3<f32>) -> vec3<f32> {
    let sampling = u32(scene.render_params.x);
    if (sampling == SAMPLING_SOFTWARE) {
        return software_landscape(clip_position, affine);
    }
    var color = textureSample(t_diffuse, s_diffuse, uv).rgb;
    if (sampling != 0u) {
        color = filtered_landscape_grad(sampling, uv, dpdx(uv), dpdy(uv), color);
    }
    return color;
}

fn landscape_color_grad(
    clip_position: vec4<f32>,
    uv: vec2<f32>,
    uv_dx: vec2<f32>,
    uv_dy: vec2<f32>,
    affine_uv: vec2<f32>,
    affine_dx: vec2<f32>,
    affine_dy: vec2<f32>,
) -> vec3<f32> {
    let sampling = u32(scene.render_params.x);
    if (sampling == SAMPLING_SOFTWARE) {
        return software_landscape_grad(clip_position, affine_uv, affine_dx, affine_dy);
    }
    var color = textureSampleGrad(t_diffuse, s_diffuse, uv, uv_dx, uv_dy).rgb;
    if (sampling != 0u) {
        color = filtered_landscape_grad(sampling, uv, uv_dx, uv_dy, color);
    }
    return color;
}

fn lit_and_fogged(color_in: vec3<f32>, clip_position: vec4<f32>, world_pos: vec3<f32>) -> vec4<f32> {
    return lit_and_fogged_at(color_in, clip_position, world_pos, world_pos);
}

fn lit_and_fogged_at(
    color_in: vec3<f32>,
    clip_position: vec4<f32>,
    world_pos: vec3<f32>,
    fog_pos: vec3<f32>,
) -> vec4<f32> {
    var color = color_in;
    if (scene.ambient_tint.a > 0.0) {
        let base_ambient = color * scene.ambient_tint.rgb;
        var lit_color = base_ambient;
        let count = min(scene.light_params.x, 16u);
        if (count > 0u) {
            for (var i = 0u; i < count; i = i + 1u) {
                let light = scene.lights[i];
                let light_pos = light.pos.xyz;
                let radius = light.pos.w;
                let delta = light_pos - world_pos;
                let d2 = dot(delta, delta);
                let r2 = radius * radius;
                if (d2 < r2 && radius > 0.0) {
                    let d = sqrt(d2);
                    let norm_d = 1.0 - d / radius;
                    let atten = (norm_d * norm_d) / (1.0 + 0.1 * d2);
                    lit_color += light.color.rgb * (light.color.w * atten) * color;
                }
            }
        }
        color = lit_color;
    }
    if (scene.fog.a != 0.0 || scene.fog_height.x != 0.0) {
        color = mix(scene.fog.rgb, color, height_fog_factor(clip_position, fog_pos));
    }
    return vec4<f32>(srgb_to_linear(color), 1.0);
}

@group(2) @binding(0)
var t_detail: texture_2d_array<f32>;

@group(2) @binding(1)
var s_detail: sampler;

struct DetailVertexInput {
    @location(0) position: vec3<f32>,
    @location(1) uv: vec2<f32>,
    @location(2) cell_flags: u32,
    @location(3) coord0: vec2<f32>,
    @location(4) coord1: vec2<f32>,
    @location(5) coord2: vec2<f32>,
    @location(6) layers: u32,
    @location(7) weights: vec4<f32>,
};

struct DetailVertexOutput {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) affine: vec3<f32>,
    @location(2) world_pos: vec3<f32>,
    @location(3) coord0: vec2<f32>,
    @location(4) coord1: vec2<f32>,
    @location(5) coord2: vec2<f32>,
    @location(6) @interpolate(flat) layers: u32,
    @location(7) weights: vec3<f32>,
};

@vertex
fn vs_detail(in: DetailVertexInput) -> DetailVertexOutput {
    var out: DetailVertexOutput;
    out.clip_position = scene.view_proj * vec4<f32>(in.position, 1.0);
    out.uv = in.uv;
    out.world_pos = in.position;
    out.affine = vec3<f32>(in.uv * out.clip_position.w, out.clip_position.w);
    out.coord0 = in.coord0;
    out.coord1 = in.coord1;
    out.coord2 = in.coord2;
    out.layers = in.layers;
    out.weights = in.weights.xyz;
    return out;
}

fn detail_term(coord: vec2<f32>, dx: vec2<f32>, dy: vec2<f32>, layer: u32) -> vec3<f32> {
    return 2.0 * textureSampleGrad(t_detail, s_detail, coord, layer, dx, dy).rgb - vec3<f32>(1.0);
}

@fragment
fn fs_detail(in: DetailVertexOutput) -> @location(0) vec4<f32> {
    let c0_dx = dpdx(in.coord0);
    let c0_dy = dpdy(in.coord0);
    let c1_dx = dpdx(in.coord1);
    let c1_dy = dpdy(in.coord1);
    let c2_dx = dpdx(in.coord2);
    let c2_dy = dpdy(in.coord2);
    let color = landscape_color(in.clip_position, in.uv, in.affine);
    var factor = vec3<f32>(1.0);
    if (in.weights.x > 0.0) {
        factor += detail_term(in.coord0, c0_dx, c0_dy, in.layers & 0xffu) * in.weights.x;
    }
    if (in.weights.y > 0.0) {
        factor += detail_term(in.coord1, c1_dx, c1_dy, (in.layers >> 8u) & 0xffu) * in.weights.y;
    }
    if (in.weights.z > 0.0) {
        factor += detail_term(in.coord2, c2_dx, c2_dy, (in.layers >> 16u) & 0xffu) * in.weights.z;
    }
    return scene_out(lit_and_fogged(color * factor, in.clip_position, in.world_pos));
}
