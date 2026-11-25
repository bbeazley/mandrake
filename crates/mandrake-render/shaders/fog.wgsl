fn fog_factor(frag: vec4<f32>) -> f32 {
    return clamp(exp(-scene.fog.a * (frag.z / frag.w)), 0.0, 1.0);
}

fn fog_hash(p: vec2<f32>) -> f32 {
    var q = fract(vec3<f32>(p.x, p.y, p.x) * 0.1031);
    q += dot(q, q.yzx + vec3<f32>(33.33));
    return fract((q.x + q.y) * q.z);
}

fn fog_value_noise(p: vec2<f32>) -> f32 {
    let i = floor(p);
    let f = fract(p);
    let u = f * f * (vec2<f32>(3.0) - 2.0 * f);
    return mix(
        mix(fog_hash(i), fog_hash(i + vec2<f32>(1.0, 0.0)), u.x),
        mix(fog_hash(i + vec2<f32>(0.0, 1.0)), fog_hash(i + vec2<f32>(1.0, 1.0)), u.x),
        u.y,
    );
}

fn fog_patch(world: vec3<f32>) -> f32 {
    let amount = scene.fog_motion.y;
    if (amount == 0.0) {
        return 1.0;
    }
    let size = scene.fog_motion.x;
    let drift = scene.fog_motion.zw * scene.fog_height.w;
    let a = fog_value_noise((world.xz + drift) / size);
    let b = fog_value_noise((world.xz - drift.yx * 1.4) / (size * 0.43) + vec2<f32>(17.0, 5.0));
    let t = clamp((a * 0.65 + b * 0.35 - scene.fog_shape.x) * scene.fog_shape.y, 0.0, 1.0);
    let n = t * t * (3.0 - 2.0 * t);
    return mix(1.0, n * 2.0, amount);
}

fn fog_point_on_surface(point: vec3<f32>, surface: f32) -> vec3<f32> {
    let eye = scene.camera_pos.xyz;
    let drop = eye.y - point.y;
    if (drop <= 0.0) {
        return point;
    }
    return eye + (point - eye) * clamp((eye.y - surface) / drop, 0.0, 1.0);
}

fn height_fog_factor(frag: vec4<f32>, world: vec3<f32>) -> f32 {
    let d0 = scene.fog_height.x;
    if (d0 == 0.0) {
        return fog_factor(frag);
    }
    let eye = scene.camera_pos.xyz;
    let ray = world - eye;
    let inv_h = scene.fog_height.z;
    let at_eye = exp(-(eye.y - scene.fog_height.y) * inv_h);
    let at_point = exp(-(world.y - scene.fog_height.y) * inv_h);
    let k = ray.y * inv_h;
    let mean = select((at_eye - at_point) / k, at_point, abs(k) < 0.001);
    return clamp(exp(-d0 * length(ray) * mean * fog_patch(world)), 0.0, 1.0);
}

override SCENE_ENCODE_SRGB: bool = false;

fn srgb_to_linear(c: vec3<f32>) -> vec3<f32> {
    let lo = c / 12.92;
    let hi = pow((c + vec3<f32>(0.055)) / 1.055, vec3<f32>(2.4));
    return select(hi, lo, c <= vec3<f32>(0.04045));
}

fn scene_encode_srgb(linear: vec3<f32>) -> vec3<f32> {
    let c = clamp(linear, vec3<f32>(0.0), vec3<f32>(1.0));
    let lo = c * 12.92;
    let hi = 1.055 * pow(c, vec3<f32>(1.0 / 2.4)) - vec3<f32>(0.055);
    return select(hi, lo, c <= vec3<f32>(0.0031308));
}

fn scene_out(c: vec4<f32>) -> vec4<f32> {
    if (SCENE_ENCODE_SRGB) {
        return vec4<f32>(scene_encode_srgb(c.rgb), c.a);
    }
    return c;
}
