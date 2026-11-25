struct OccluderInput {
    @location(0) position: vec3<f32>,
    @location(1) depth0: vec3<f32>,
    @location(2) depth1: vec3<f32>,
    @location(3) depth2: vec3<f32>,
    @location(4) depth3: vec3<f32>,
    @location(5) depth4: vec3<f32>,
    @location(6) depth5: vec3<f32>,
};

fn view_depth(p: vec3<f32>) -> f32 {
    return (scene.view_proj * vec4<f32>(p, 1.0)).w;
}

const MESH_DEPTH_BIAS: f32 = 0.25;

@vertex
fn vs_main(v: OccluderInput) -> @builtin(position) vec4<f32> {
    let c = scene.view_proj * vec4<f32>(v.position, 1.0);
    if (c.w <= 1e-4) {
        return c;
    }
    let far = max(
        max(view_depth(v.depth0), view_depth(v.depth1)),
        max(max(view_depth(v.depth2), view_depth(v.depth3)), max(view_depth(v.depth4), view_depth(v.depth5))),
    );
    let d = far + MESH_DEPTH_BIAS;
    let eye_z = (scene.view_proj * vec4<f32>(scene.camera_pos.xyz, 1.0)).z;
    let s = d / c.w;
    return vec4<f32>(c.xy * s, c.z * s + eye_z * (1.0 - s), d);
}

const TFL_LAST_BUCKET: f32 = 45.0;

@vertex
fn vs_tfl_bucket(v: OccluderInput) -> @builtin(position) vec4<f32> {
    let c = scene.view_proj * vec4<f32>(v.position, 1.0);
    if (c.w <= 1e-4) {
        return c;
    }
    let far = max(
        max(view_depth(v.depth0), view_depth(v.depth1)),
        max(max(view_depth(v.depth2), view_depth(v.depth3)), max(view_depth(v.depth4), view_depth(v.depth5))),
    );
    let bucket = min(floor(max(far, 0.0)), TFL_LAST_BUCKET);
    let d = bucket + 1.0;
    let eye_z = (scene.view_proj * vec4<f32>(scene.camera_pos.xyz, 1.0)).z;
    let s = d / c.w;
    return vec4<f32>(c.xy * s, c.z * s + eye_z * (1.0 - s), d);
}

@fragment
fn fs_main() -> @location(0) vec4<f32> {
    return vec4<f32>(0.0);
}
