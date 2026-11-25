// `--water enhanced` ripples (`mandrake_render::water::ripples`): one step of the 2D wave
// equation on a heightfield over the map's water, `texels per cell` texels per cell, ping-pong
// (`r`: height now, `g`: the step before, render units). The speed is the shallow-water wave's,
// `sqrt(g depth)`, from the floor heights; dry texels stay still. The disturbances are the
// objects in the water this frame (`WaterShading::step_ripples`).

struct RippleUniforms {
    // Grid column and row of texel (0, 0)'s corner, texels per cell, unused.
    region: vec4<f32>,
    // Courant number squared per render unit of depth, its maximum (stability), the share of
    // the height kept per step, unused.
    wave: vec4<f32>,
    // Disturbances in `disturbances`.
    count: vec4<u32>,
    // Grid column, row, height added (render units), radius (cells).
    disturbances: array<vec4<f32>, 32>,
};

@group(0) @binding(0)
var<uniform> u: RippleUniforms;

@group(0) @binding(1)
var t_state: texture_2d<f32>;

// `water.wgsl`'s `t_floor`: per grid vertex the physical height, the water level, 1 where
// there is one, unused here.
@group(0) @binding(2)
var t_floor: texture_2d<f32>;

@vertex
fn vs_ripple(@builtin(vertex_index) i: u32) -> @builtin(position) vec4<f32> {
    let x = f32((i << 1u) & 2u);
    let y = f32(i & 2u);
    return vec4<f32>(x * 2.0 - 1.0, 1.0 - y * 2.0, 0.0, 1.0);
}

fn floor_info(cr: vec2<f32>) -> vec4<f32> {
    let dims = vec2<i32>(textureDimensions(t_floor));
    let last = dims - vec2<i32>(1);
    let c = clamp(cr, vec2<f32>(0.0), vec2<f32>(last));
    let i = vec2<i32>(floor(c));
    let f = c - floor(c);
    let h00 = textureLoad(t_floor, clamp(i, vec2<i32>(0), last), 0);
    let h10 = textureLoad(t_floor, clamp(i + vec2<i32>(1, 0), vec2<i32>(0), last), 0);
    let h01 = textureLoad(t_floor, clamp(i + vec2<i32>(0, 1), vec2<i32>(0), last), 0);
    let h11 = textureLoad(t_floor, clamp(i + vec2<i32>(1, 1), vec2<i32>(0), last), 0);
    return mix(mix(h00, h10, f.x), mix(h01, h11, f.x), f.y);
}

fn state(p: vec2<i32>) -> vec2<f32> {
    let last = vec2<i32>(textureDimensions(t_state)) - vec2<i32>(1);
    return textureLoad(t_state, clamp(p, vec2<i32>(0), last), 0).rg;
}

@fragment
fn fs_ripple(@builtin(position) frag: vec4<f32>) -> @location(0) vec4<f32> {
    let p = vec2<i32>(floor(frag.xy));
    let here = state(p);
    let h = here.x;
    let lap = state(p + vec2<i32>(1, 0)).x + state(p - vec2<i32>(1, 0)).x
        + state(p + vec2<i32>(0, 1)).x + state(p - vec2<i32>(0, 1)).x - 4.0 * h;
    let cr = u.region.xy + (vec2<f32>(p) + vec2<f32>(0.5)) / u.region.z;
    let info = floor_info(cr);
    let depth = select(0.0, info.y - info.x, info.z > 0.999);
    let c2 = min(u.wave.x * max(depth, 0.0), u.wave.y);
    var next = (2.0 * h - here.y + c2 * lap) * u.wave.z;
    for (var i = 0u; i < min(u.count.x, 32u); i++) {
        let d = u.disturbances[i];
        let r = distance(cr, d.xy) / max(d.w, 1.0e-3);
        if (r < 1.0) {
            let w = 1.0 - r * r;
            next += d.z * w * w;
        }
    }
    if (depth <= 0.0) {
        next = 0.0;
    }
    return vec4<f32>(next, h, 0.0, 0.0);
}
