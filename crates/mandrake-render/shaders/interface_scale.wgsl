struct Params {
    size: vec2<f32>,
    scale: vec2<f32>,
    uv_offset: vec2<f32>,
    view_size: vec2<f32>,
    fade_rows: f32,
    alpha: f32,
    mode: u32,
    sharpen: f32,
};

@group(0) @binding(0) var<uniform> params: Params;
@group(0) @binding(1) var layer: texture_2d<f32>;
@group(0) @binding(2) var linear_sampler: sampler;
@group(0) @binding(3) var nearest_sampler: sampler;

struct VsOut {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

@vertex
fn vs_main(@builtin(vertex_index) vi: u32) -> VsOut {
    let corner = vec2<f32>(f32(vi & 1u), f32((vi >> 1u) & 1u));
    var out: VsOut;
    out.position = vec4<f32>(corner.x * 2.0 - 1.0, 1.0 - corner.y * 2.0, 0.0, 1.0);
    out.uv = corner;
    return out;
}

fn sharp_uv(uv: vec2<f32>) -> vec2<f32> {
    let t = uv * params.size;
    let d = fract(t) - vec2<f32>(0.5);
    let inner = max(vec2<f32>(0.5) - vec2<f32>(0.5) / params.scale, vec2<f32>(0.0));
    let o = (d - clamp(d, -inner, inner)) * params.scale + vec2<f32>(0.5);
    return (floor(t) + o) / params.size;
}

fn lanczos2(x: f32) -> f32 {
    let ax = abs(x);
    if ax < 1e-5 {
        return 1.0;
    }
    if ax >= 2.0 {
        return 0.0;
    }
    let px = 3.14159265 * ax;
    return 2.0 * sin(px) * sin(px * 0.5) / (px * px);
}

fn lanczos2_sample(uv: vec2<f32>) -> vec4<f32> {
    let s = clamp(params.sharpen, 0.0, 1.0);
    let c = mix(uv, sharp_uv(uv), s) * params.size - vec2<f32>(0.5);
    let base = floor(c);
    let lobe_boost = 1.0 + 0.5 * s;
    var wx: array<f32, 4>;
    var wy: array<f32, 4>;
    var sx = 0.0;
    var sy = 0.0;
    for (var k = 0; k < 4; k++) {
        let o = base + vec2<f32>(f32(k - 1));
        var w_x = lanczos2(o.x - c.x);
        var w_y = lanczos2(o.y - c.y);
        if w_x < 0.0 {
            w_x *= lobe_boost;
        }
        if w_y < 0.0 {
            w_y *= lobe_boost;
        }
        wx[k] = w_x;
        wy[k] = w_y;
        sx += w_x;
        sy += w_y;
    }
    let hi = vec2<i32>(params.size) - vec2<i32>(1);
    var sum = vec4<f32>(0.0);
    var lo = vec4<f32>(1.0);
    var hi_c = vec4<f32>(0.0);
    for (var j = 0; j < 4; j++) {
        let y = clamp(i32(base.y) + j - 1, 0, hi.y);
        var row = vec4<f32>(0.0);
        for (var i = 0; i < 4; i++) {
            let x = clamp(i32(base.x) + i - 1, 0, hi.x);
            let t = textureLoad(layer, vec2<i32>(x, y), 0);
            if i >= 1 && i <= 2 && j >= 1 && j <= 2 {
                lo = min(lo, t);
                hi_c = max(hi_c, t);
            }
            row += wx[i] * t;
        }
        sum += wy[j] * row;
    }
    let raw = sum / (sx * sy);
    let deringed = mix(raw, clamp(raw, lo, hi_c), 0.5 * s);
    let p = clamp(deringed, vec4<f32>(0.0), vec4<f32>(1.0));
    return vec4<f32>(min(p.rgb, vec3<f32>(p.a)), p.a);
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    let item_pos = in.uv * params.view_size;
    let uv = (params.uv_offset + item_pos) / params.size;
    var color: vec4<f32>;
    if params.mode == 1u {
        color = textureSampleLevel(layer, nearest_sampler, uv, 0.0);
    } else if params.mode == 2u {
        color = textureSampleLevel(layer, linear_sampler, uv, 0.0);
    } else if params.mode == 3u {
        color = lanczos2_sample(uv);
    } else {
        color = textureSampleLevel(layer, linear_sampler, sharp_uv(uv), 0.0);
    }
    var weight = params.alpha;
    if params.fade_rows > 0.0 {
        let top = (item_pos.y + 0.5) / params.fade_rows;
        let bottom = (params.view_size.y + 0.5 - item_pos.y) / params.fade_rows;
        weight *= clamp(min(top, bottom), 0.0, 1.0);
    }
    return color * weight;
}
