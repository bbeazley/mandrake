const REFLECTION_EDGE_RADIUS: f32 = 24.0;
const REFLECTION_EDGE_REFERENCE_LINES: f32 = 1080.0;

fn reflection_edge_fade(frag: vec2<f32>) -> f32 {
    var disc = array<vec2<f32>, 32>(
        vec2<f32>(0.1250, 0.0000),
        vec2<f32>(-0.1596, 0.1462),
        vec2<f32>(0.0244, -0.2784),
        vec2<f32>(0.2012, 0.2625),
        vec2<f32>(-0.3693, -0.0653),
        vec2<f32>(0.3498, -0.2225),
        vec2<f32>(-0.1170, 0.4352),
        vec2<f32>(-0.2231, -0.4296),
        vec2<f32>(0.4841, 0.1768),
        vec2<f32>(-0.5036, 0.2079),
        vec2<f32>(0.2428, -0.5188),
        vec2<f32>(0.1794, 0.5720),
        vec2<f32>(-0.5408, -0.3134),
        vec2<f32>(0.6344, -0.1395),
        vec2<f32>(-0.3871, 0.5507),
        vec2<f32>(-0.0894, -0.6902),
        vec2<f32>(0.5491, 0.4628),
        vec2<f32>(-0.7389, 0.0306),
        vec2<f32>(0.5390, -0.5363),
        vec2<f32>(-0.0361, 0.7798),
        vec2<f32>(-0.5128, -0.6145),
        vec2<f32>(0.8124, 0.1093),
        vec2<f32>(-0.6883, 0.4789),
        vec2<f32>(0.1881, -0.8361),
        vec2<f32>(0.4350, 0.7592),
        vec2<f32>(-0.8504, -0.2713),
        vec2<f32>(0.8261, -0.3817),
        vec2<f32>(-0.3579, 0.8552),
        vec2<f32>(-0.3194, -0.8880),
        vec2<f32>(0.8499, 0.4467),
        vec2<f32>(-0.9440, 0.2488),
        vec2<f32>(0.5366, -0.8345)
    );
    let dims = vec2<f32>(textureDimensions(t_reflection_edge));
    let radius = REFLECTION_EDGE_RADIUS * max(dims.y / REFLECTION_EDGE_REFERENCE_LINES, 1.0);
    var dry = 0.0;
    for (var i = 0; i < 32; i++) {
        let at = (frag + disc[i] * radius) / dims;
        dry += textureSampleLevel(t_reflection_edge, s_reflection_edge, at, 0.0).r;
    }
    return smoothstep(0.0, 1.0, 1.0 - dry / 16.0);
}
