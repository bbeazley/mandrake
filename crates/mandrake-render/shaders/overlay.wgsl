struct Screen {
    size: vec2<f32>,
    pad: vec2<f32>,
};

@group(0) @binding(0) var<uniform> screen: Screen;
@group(1) @binding(0) var image: texture_2d<f32>;
@group(1) @binding(1) var image_sampler: sampler;

struct VsOut {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) tint: vec4<f32>,
};

@vertex
fn vs_main(
    @builtin(vertex_index) vi: u32,
    @location(0) dst: vec4<f32>,
    @location(1) uv: vec4<f32>,
    @location(2) tint: vec4<f32>,
) -> VsOut {
    let corner = vec2<f32>(f32(vi & 1u), f32((vi >> 1u) & 1u));
    let px = dst.xy + corner * dst.zw;
    var out: VsOut;
    out.position = vec4<f32>(px.x / screen.size.x * 2.0 - 1.0, 1.0 - px.y / screen.size.y * 2.0, 0.0, 1.0);
    out.uv = mix(uv.xy, uv.zw, corner);
    out.tint = tint;
    return out;
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    let c = textureSample(image, image_sampler, in.uv);
    return c * in.tint;
}

fn srgb_to_linear(c: vec3<f32>) -> vec3<f32> {
    let lo = c / 12.92;
    let hi = pow((c + vec3<f32>(0.055)) / 1.055, vec3<f32>(2.4));
    return select(hi, lo, c <= vec3<f32>(0.04045));
}

@fragment
fn fs_srgb(in: VsOut) -> @location(0) vec4<f32> {
    let c = textureSample(image, image_sampler, in.uv);
    let t = c * in.tint;
    return vec4<f32>(srgb_to_linear(t.rgb), t.a);
}
