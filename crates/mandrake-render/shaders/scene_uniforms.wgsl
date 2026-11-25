struct PointLight {
    pos: vec4<f32>,
    color: vec4<f32>,
};

struct SceneUniforms {
    view_proj: mat4x4<f32>,
    camera_pos: vec4<f32>,
    render_params: vec4<f32>,
    fog: vec4<f32>,
    fog_height: vec4<f32>,
    fog_motion: vec4<f32>,
    fog_shape: vec4<f32>,
    ambient_tint: vec4<f32>,
    light_params: vec4<u32>,
    lights: array<PointLight, 16>,
};

@group(0) @binding(0)
var<uniform> scene: SceneUniforms;
