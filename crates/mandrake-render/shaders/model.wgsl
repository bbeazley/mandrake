struct ModelUniforms {
    model_matrix: mat4x4<f32>,
    tint: vec4<f32>,
    shade_kpqr: vec4<f32>,
    shade_xyz: vec4<f32>,
};

@group(1) @binding(0)
var t_diffuse: texture_2d<f32>;

@group(1) @binding(1)
var s_diffuse: sampler;

@group(2) @binding(0)
var<uniform> model: ModelUniforms;

@group(3) @binding(0)
var t_reflection_edge: texture_2d<f32>;

@group(3) @binding(1)
var s_reflection_edge: sampler;

struct VertexInput {
    @location(0) position: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) uv: vec2<f32>,
    @location(3) alpha: f32,
};

struct VertexOutput {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) alpha: f32,
    @location(2) affine: vec3<f32>,
    @location(3) world_pos: vec3<f32>,
    @location(4) normal: vec3<f32>,
};

@vertex
fn vs_main(in: VertexInput) -> VertexOutput {
    return model_vertex(in);
}

fn model_vertex(in: VertexInput) -> VertexOutput {
    var out: VertexOutput;
    let world_pos4 = model.model_matrix * vec4<f32>(in.position, 1.0);
    out.clip_position = scene.view_proj * world_pos4;
    out.world_pos = world_pos4.xyz;
    let n = (model.model_matrix * vec4<f32>(in.normal, 0.0)).xyz;
    let n_len = length(n);
    if (n_len > 1.0e-5) {
        out.normal = n / n_len;
    } else {
        out.normal = vec3<f32>(0.0, 1.0, 0.0);
    }
    out.uv = in.uv;
    out.alpha = in.alpha;
    out.affine = vec3<f32>(in.uv * out.clip_position.w, out.clip_position.w);
    return out;
}

struct KeyedVertexInput {
    @location(0) position: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) uv: vec2<f32>,
    @location(3) alpha: f32,
    @location(4) key: f32,
};

const TFL_LAST_BUCKET_MIDDLE: f32 = 45.5;

@vertex
fn vs_keyed(in: KeyedVertexInput) -> VertexOutput {
    var out = model_vertex(VertexInput(in.position, in.normal, in.uv, in.alpha));
    let c = out.clip_position;
    if (c.w > 1e-4 && in.key > 0.0) {
        let eye = scene.camera_pos.xyz;
        let p = eye + (out.world_pos - eye) * (min(in.key, TFL_LAST_BUCKET_MIDDLE) / c.w);
        let cp = scene.view_proj * vec4<f32>(p, 1.0);
        out.clip_position = vec4<f32>(c.xy, cp.z / cp.w * c.w, c.w);
    }
    return out;
}

@vertex
fn vs_reflection(in: VertexInput) -> VertexOutput {
    var out = model_vertex(in);
    out.normal = vec3<f32>(0.0, 1.0, 0.0);
    out.world_pos = fog_point_on_surface(out.world_pos, in.normal.y);
    return out;
}

const SAMPLING_SOFTWARE: u32 = 1u;
const SAMPLING_BICUBIC: u32 = 2u;
const SAMPLING_SHARP: u32 = 3u;

fn software_model_texel(in: VertexOutput) -> vec4<f32> {
    let uv = in.affine.xy / in.affine.z;
    let at = uv - (dpdx(uv) + dpdy(uv)) * 0.5;
    let dims = vec2<i32>(textureDimensions(t_diffuse));
    let coord = clamp(
        vec2<i32>(floor(fract(at) * vec2<f32>(dims))),
        vec2<i32>(0),
        dims - vec2<i32>(1),
    );
    return textureLoad(t_diffuse, coord, 0);
}

const FILTER_FADE_END: f32 = 1.41421356;

fn catmull_rom(uv: vec2<f32>) -> vec4<f32> {
    let dims = vec2<i32>(textureDimensions(t_diffuse));
    let c = uv * vec2<f32>(dims) - vec2<f32>(0.5);
    let base = floor(c);
    let wx = catmull_rom_weights(c.x - base.x);
    let wy = catmull_rom_weights(c.y - base.y);
    let first = vec2<i32>(base) - vec2<i32>(1);
    let last = dims - vec2<i32>(1);
    var sum = vec4<f32>(0.0);
    for (var j = 0; j < 4; j++) {
        for (var i = 0; i < 4; i++) {
            let at = clamp(first + vec2<i32>(i, j), vec2<i32>(0), last);
            sum += wx[i] * wy[j] * textureLoad(t_diffuse, at, 0);
        }
    }
    let a = clamp(sum.a, 0.0, 1.0);
    return vec4<f32>(clamp(sum.rgb / max(sum.a, 1.0), vec3<f32>(0.0), vec3<f32>(a)), a);
}

fn filtered_model_texel(mode: u32, uv: vec2<f32>, linear: vec4<f32>) -> vec4<f32> {
    let dims = vec2<f32>(textureDimensions(t_diffuse));
    let t = uv * dims;
    let t_dx = dpdx(t);
    let t_dy = dpdy(t);
    let rho = max(length(t_dx), length(t_dy));
    if (!(rho < FILTER_FADE_END)) {
        return linear;
    }
    var filtered: vec4<f32>;
    if (mode == SAMPLING_BICUBIC) {
        filtered = catmull_rom(uv);
    } else {
        let rate = vec2<f32>(length(vec2<f32>(t_dx.x, t_dy.x)), length(vec2<f32>(t_dx.y, t_dy.y)));
        filtered = textureSampleLevel(t_diffuse, s_diffuse, sharp_coord(t, rate) / dims, 0.0);
    }
    return mix(filtered, linear, clamp((rho - 1.0) / (FILTER_FADE_END - 1.0), 0.0, 1.0));
}

fn shade_model(in: VertexOutput) -> vec4<f32> {
    let sampling = u32(scene.render_params.x);
    var tex_color: vec4<f32>;
    if (sampling == SAMPLING_SOFTWARE) {
        tex_color = software_model_texel(in);
    } else {
        tex_color = textureSample(t_diffuse, s_diffuse, in.uv);
        if (sampling != 0u) {
            tex_color = filtered_model_texel(sampling, in.uv, tex_color);
        }
    }
    if (tex_color.a < 0.5) {
        discard;
    }
    let albedo = tex_color.rgb / tex_color.a;

    let alpha = min(in.alpha, model.tint.a);
    let c = albedo * model.tint.rgb;
    let l = dot(vec3<f32>(0.4, 0.6, 0.2), c);
    var s = c * model.shade_kpqr.x + alpha * model.shade_kpqr.yzw + l * model.shade_xyz.xyz;
    if (scene.ambient_tint.a > 0.0) {
        let base_ambient = s * scene.ambient_tint.rgb;
        var lit_color = base_ambient;
        let count = min(scene.light_params.x, 16u);
        if (count > 0u) {
            let n_len = length(in.normal);
            var N = vec3<f32>(0.0, 1.0, 0.0);
            if (n_len > 1.0e-5) {
                N = in.normal / n_len;
            }
            for (var i = 0u; i < count; i = i + 1u) {
                let light = scene.lights[i];
                let light_pos = light.pos.xyz;
                let radius = light.pos.w;
                let to_light = light_pos - in.world_pos;
                let d2 = dot(to_light, to_light);
                let r2 = radius * radius;
                if (d2 < r2 && radius > 0.0) {
                    let d = max(sqrt(d2), 0.0001);
                    let L = to_light / d;
                    let diff = max(dot(N, L), 0.0);
                    let norm_d = 1.0 - d / radius;
                    let atten = (norm_d * norm_d) / (1.0 + 0.1 * d2);
                    lit_color += light.color.rgb * (light.color.w * atten * diff) * s;
                }
            }
        }
        s = lit_color;
    }
    if (scene.fog.a != 0.0 || scene.fog_height.x != 0.0) {
        s = mix(scene.fog.rgb, s, height_fog_factor(in.clip_position, in.world_pos));
    }
    return vec4<f32>(clamp(s, vec3<f32>(0.0), vec3<f32>(1.0)), alpha);
}

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    let c = shade_model(in);
    return scene_out(vec4<f32>(srgb_to_linear(c.rgb), c.a));
}

@fragment
fn fs_media_display(in: VertexOutput) -> @location(0) vec4<f32> {
    return shade_model(in);
}

@fragment
fn fs_media_display_soft(in: VertexOutput) -> @location(0) vec4<f32> {
    let c = shade_model(in);
    return vec4<f32>(c.rgb, c.a * reflection_edge_fade(in.clip_position.xy));
}

@fragment
fn fs_main_soft(in: VertexOutput) -> @location(0) vec4<f32> {
    let c = shade_model(in);
    return scene_out(vec4<f32>(srgb_to_linear(c.rgb), c.a * reflection_edge_fade(in.clip_position.xy)));
}
