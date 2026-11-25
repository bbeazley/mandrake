fn catmull_rom_weights(t: f32) -> vec4<f32> {
    let t2 = t * t;
    let t3 = t2 * t;
    return vec4<f32>(
        -0.5 * t + t2 - 0.5 * t3,
        1.0 - 2.5 * t2 + 1.5 * t3,
        0.5 * t + 2.0 * t2 - 1.5 * t3,
        -0.5 * t2 + 0.5 * t3,
    );
}

fn sharp_coord(t: vec2<f32>, rate: vec2<f32>) -> vec2<f32> {
    let scale = vec2<f32>(1.0) / clamp(rate, vec2<f32>(1.0e-4), vec2<f32>(1.0));
    let d = fract(t) - vec2<f32>(0.5);
    let inner = vec2<f32>(0.5) - vec2<f32>(0.5) / scale;
    return floor(t) + (d - clamp(d, -inner, inner)) * scale + vec2<f32>(0.5);
}
