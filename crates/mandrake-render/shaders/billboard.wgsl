@group(1) @binding(0)
var t_diffuse: texture_2d<f32>;

@group(1) @binding(1)
var s_diffuse: sampler;

@group(2) @binding(0)
var t_reflection_edge: texture_2d<f32>;

@group(2) @binding(1)
var s_reflection_edge: sampler;

struct InstanceInput {
    @location(0) position: vec3<f32>,
    @location(1) flags: u32,
    @location(2) size: vec2<f32>,
    @location(3) uv_min: vec2<f32>,
    @location(4) uv_max: vec2<f32>,
    @location(5) tint: vec4<f32>,
    @location(6) anchor: vec2<f32>,
    @location(7) media: vec4<f32>,
    @location(8) shade_kpqr: vec4<f32>,
    @location(9) shade_xyz: vec4<f32>,
};

struct VertexOutput {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) quad_uv: vec2<f32>,
    @location(2) tint: vec4<f32>,
    @location(3) @interpolate(flat) flags: u32,
    @location(4) world_pos: vec3<f32>,
    @location(5) overlay: vec4<f32>,
    @location(6) shade_kpqr: vec4<f32>,
    @location(7) shade_xyz: vec4<f32>,
    @location(8) @interpolate(flat) frame: vec4<f32>,
    @location(9) @interpolate(flat) software_snap: vec4<f32>,
};

fn sprite_basis(instance: InstanceInput) -> array<vec3<f32>, 2> {
    let to_camera = scene.camera_pos.xyz - instance.position;
    var cam_right: vec3<f32>;
    var cam_up: vec3<f32>;
    if ((instance.flags & 8u) != 0u) {
        let xz = vec3<f32>(to_camera.x, 0.0, to_camera.z);
        let xz_len = length(xz);
        var dir_xz = vec3<f32>(0.0, 0.0, 1.0);
        if (xz_len > 1.0e-4) {
            dir_xz = xz / xz_len;
        }
        cam_right = vec3<f32>(-dir_xz.z, 0.0, dir_xz.x);
        cam_up = vec3<f32>(0.0, 1.0, 0.0);
    } else {
        let len = length(to_camera);
        var dir = vec3<f32>(0.0, 0.0, 1.0);
        if (len > 0.0001) {
            dir = to_camera / len;
        }
        let world_up = vec3<f32>(0.0, 1.0, 0.0);
        let r = cross(world_up, dir);
        let len_r = length(r);
        if (len_r > 0.0001) {
            cam_right = r / len_r;
        } else {
            cam_right = vec3<f32>(1.0, 0.0, 0.0);
        }
        cam_up = cross(dir, cam_right);
    }
    return array<vec3<f32>, 2>(cam_right, cam_up);
}

fn sprite_corner(instance: InstanceInput, corner: vec2<f32>) -> vec3<f32> {
    let basis = sprite_basis(instance);
    let offset = sprite_offset(instance, corner);
    return instance.position + (basis[0] * offset.x) + (basis[1] * offset.y);
}

fn sprite_offset(instance: InstanceInput, corner: vec2<f32>) -> vec2<f32> {
    var anchor_x = instance.anchor.x;
    let anchor_y = instance.anchor.y;
    if ((instance.flags & 1u) != 0u) {
        anchor_x = 1.0 - anchor_x;
    }

    let offset_x = (corner.x - anchor_x) * instance.size.x;
    var offset_y: f32;
    if ((instance.flags & 16u) != 0u) {
        offset_y = (corner.y - 0.5) * instance.size.y;
    } else {
        offset_y = (corner.y - (1.0 - anchor_y)) * instance.size.y;
    }

    return vec2<f32>(offset_x, offset_y);
}

fn sprite_growth(size: vec2<f32>, texels: vec2<f32>) -> vec2<f32> {
    let per_texel = trunc(abs(size) * 512.0 / max(texels, vec2<f32>(1.0)) + 1.0e-3);
    return trunc(per_texel / 2.0) / 512.0;
}

fn sprite_grown_corner(corner: f32, extent: f32, grow: f32) -> f32 {
    if (abs(extent) < 1.0e-9) {
        return corner;
    }
    return (corner * (extent + 2.0 * grow) - grow) / extent;
}

fn sprite_border_coord(t: f32, texels: f32) -> f32 {
    let w = max(texels, 1.0);
    return (t * (w + 1.0) - 0.5) / w;
}

fn sprite_local_u(instance: InstanceInput, corner_x: f32, texels: f32) -> f32 {
    var t = corner_x;
    if ((instance.flags & 1u) != 0u) {
        t = 1.0 - corner_x;
    }
    return sprite_border_coord(t, texels);
}

fn r_edge(corner_y: f32, r_lo: f32, r_hi: f32, gr: f32) -> f32 {
    return mix(max(r_lo, -gr), min(r_hi, 1.0 + gr), corner_y);
}

fn plane_depth_units(num: f32, den: f32, depth: f32) -> f32 {
    if (den == 0.0) {
        return select(0.0, 1.0e9, num > 0.0);
    }
    let ratio = num / den;
    if (ratio < 0.0) {
        return 0.0;
    }
    return trunc(ratio * depth);
}

fn software_centre_shift(half: vec2<f32>) -> vec2<f32> {
    let vp = scene.view_proj;
    let axis = vp * vec4<f32>(vp[0][3], vp[1][3], vp[2][3], 0.0);
    let exact = axis.xy / axis.w * half * vec2<f32>(1.0, -1.0) + half;
    return floor(half) + trunc(exact - half + vec2<f32>(0.5)) - exact;
}

fn software_sprite_snap(a: vec3<f32>, b: vec3<f32>) -> vec4<f32> {
    let half = scene.render_params.zw * 0.5;
    let ca = scene.view_proj * vec4<f32>(a, 1.0);
    let cb = scene.view_proj * vec4<f32>(b, 1.0);
    if (ca.w <= 0.0 || cb.w <= 0.0) {
        return vec4<f32>(0.0);
    }
    let flip = vec2<f32>(1.0, -1.0);
    let pa = ca.xy / ca.w * half * flip + half;
    let pb = cb.xy / cb.w * half * flip + half;
    let exact_lo = min(pa, pb);
    let exact_hi = max(pa, pb);
    let shift = software_centre_shift(half);
    let whole_lo = vec2<f32>(trunc(exact_lo.x + shift.x), floor(exact_lo.y + shift.y));
    let whole_hi = vec2<f32>(trunc(exact_hi.x + shift.x), floor(exact_hi.y + shift.y));
    let scale = (exact_hi - exact_lo) / max(whole_hi - whole_lo, vec2<f32>(1.0));
    return vec4<f32>(scale - vec2<f32>(1.0), exact_lo - whole_lo * scale);
}

fn screen_aligned_vertex(instance: InstanceInput, corner: vec2<f32>, base: VertexOutput) -> VertexOutput {
    var out = base;
    let vp = scene.view_proj;
    let right = normalize(vec3<f32>(vp[0][0], vp[1][0], vp[2][0]));
    let fwd = normalize(vec3<f32>(vp[0][3], vp[1][3], vp[2][3]));
    let up = cross(right, fwd);

    let reflect = (instance.flags & 4096u) != 0u;
    var origin = instance.position;
    var lo = sprite_offset(instance, vec2<f32>(corner.x, 0.0)).y;
    var hi = sprite_offset(instance, vec2<f32>(corner.x, 1.0)).y;
    let mirror_y = (instance.flags & 65536u) != 0u;
    if (mirror_y) {
        let t = lo;
        lo = -hi;
        hi = -t;
    }
    if (reflect) {
        hi = hi + instance.media.z;
        origin = origin + up * ((instance.media.y - instance.position.y) * up.y);
    }
    if (hi <= lo) {
        return culled_vertex(instance);
    }
    var r_lo = -1.0e9;
    var r_hi = 1.0e9;
    if ((instance.flags & 2048u) != 0u) {
        let m = vec3<f32>(instance.position.x, instance.media.x, instance.position.z);
        var c = dot(m - origin, up);
        if (reflect) {
            c = -c;
        }
        let t = (c - lo) / (hi - lo);
        if ((instance.flags & 16384u) != 0u) {
            if (t <= 0.0) {
                return culled_vertex(instance);
            }
            r_hi = t;
        } else {
            if (t >= 1.0) {
                return culled_vertex(instance);
            }
            r_lo = t;
        }
    }
    let texels = out.frame.zw;
    let g = sprite_growth(vec2<f32>(instance.size.x, hi - lo), texels);
    let gr = g.y / (hi - lo);
    let r = r_edge(corner.y, r_lo, r_hi, gr);
    let kept = mix(max(r_lo, 0.0), min(r_hi, 1.0), corner.y);
    var row = 1.0 - kept;
    if (mirror_y) {
        row = kept;
    }
    out.uv = vec2<f32>(sprite_local_u(instance, corner.x, texels.x), sprite_border_coord(row, texels.y));
    let cx = sprite_grown_corner(corner.x, instance.size.x, g.x);
    var offset = vec2<f32>(sprite_offset(instance, vec2<f32>(cx, corner.y)).x, lo + r * (hi - lo));
    if (reflect) {
        offset.y = -offset.y;
    }
    let q = origin + right * offset.x + up * offset.y;
    let cq = vp * vec4<f32>(q, 1.0);
    if (u32(scene.render_params.y) == SPRITE_FILTER_SOFTWARE) {
        var rect_lo = lo;
        var rect_hi = hi;
        if (reflect) {
            rect_lo = -rect_lo;
            rect_hi = -rect_hi;
        }
        out.software_snap = software_sprite_snap(
            origin + right * sprite_offset(instance, vec2<f32>(0.0, 0.0)).x + up * rect_lo,
            origin + right * sprite_offset(instance, vec2<f32>(1.0, 0.0)).x + up * rect_hi,
        );
    }
    if (cq.w <= 34.0 / 512.0) {
        return culled_vertex(instance);
    }

    let eye = scene.camera_pos.xyz;
    var z_ndc = cq.z / cq.w;
    let h = vec3<f32>(fwd.x, 0.0, fwd.z);
    let hl = length(h);
    if (hl > 1e-6) {
        let n = h / hl;
        let num = dot(origin - eye, n);
        let depth = round(cq.w * 512.0);
        var side = dot(up, n);
        if (reflect) {
            side = -side;
        }
        let bottom = plane_depth_units(num, num + side * (lo + r_edge(0.0, r_lo, r_hi, gr) * (hi - lo)), depth);
        let top = plane_depth_units(num, num + side * (lo + r_edge(1.0, r_lo, r_hi, gr) * (hi - lo)), depth);
        if (bottom == 0.0 || top == 0.0) {
            return culled_vertex(instance);
        }
        let units = mix(bottom, top, corner.y);
        let p = eye + (q - eye) * (units / depth);
        let cp = vp * vec4<f32>(p, 1.0);
        z_ndc = cp.z / cp.w;
    }
    if ((instance.flags & 1048576u) != 0u) {
        let key = min((vp * vec4<f32>(origin, 1.0)).w, 45.5);
        let p = eye + (q - eye) * (key / cq.w);
        let cp = vp * vec4<f32>(p, 1.0);
        z_ndc = cp.z / cp.w;
    }
    out.world_pos = q;
    if (reflect) {
        out.world_pos = fog_point_on_surface(q, instance.media.x);
    }
    out.clip_position = vec4<f32>(cq.xy, z_ndc * cq.w, cq.w);
    return out;
}

fn velocity_quad_vertex(instance: InstanceInput, corner: vec2<f32>, base: VertexOutput) -> VertexOutput {
    var out = base;
    let vp = scene.view_proj;
    let right = normalize(vec3<f32>(vp[0][0], vp[1][0], vp[2][0]));
    let fwd = normalize(vec3<f32>(vp[0][3], vp[1][3], vp[2][3]));
    let up = cross(right, fwd);
    let u = instance.media.xyz;
    let a = mix(instance.size.x, instance.size.y, corner.x);
    let b = mix(instance.anchor.y, instance.anchor.x, corner.y);
    let view = vec3<f32>(-a * u.x - b * u.y, -a * u.y + b * u.x, -a * u.z);
    let q = instance.position + right * view.x + up * view.y - fwd * view.z;
    out.uv = vec2<f32>(mix(64.0, 65472.0, corner.x), mix(65472.0, 64.0, corner.y)) / 65536.0;
    out.world_pos = q;
    out.clip_position = vp * vec4<f32>(q, 1.0);
    return out;
}

fn world_quad_vertex(instance: InstanceInput, corner: vec2<f32>, base: VertexOutput) -> VertexOutput {
    var out = base;
    let y = instance.position.y;
    var q = instance.position;
    if (corner.x > 0.5 && corner.y < 0.5) {
        q = vec3<f32>(instance.media.x, y, instance.media.y);
    } else if (corner.x > 0.5) {
        q = vec3<f32>(instance.anchor.x, y, instance.anchor.y);
    } else if (corner.y > 0.5) {
        q = vec3<f32>(instance.size.x, y, instance.size.y);
    }
    out.uv = vec2<f32>(mix(65472.0, 64.0, corner.x), mix(64.0, 65472.0, corner.y)) / 65536.0;
    out.world_pos = q;
    out.clip_position = scene.view_proj * vec4<f32>(q, 1.0);
    return out;
}

fn culled_vertex(instance: InstanceInput) -> VertexOutput {
    var out: VertexOutput;
    out.clip_position = vec4<f32>(0.0, 0.0, -1.0, 1.0);
    out.uv = vec2<f32>(0.0);
    out.quad_uv = vec2<f32>(0.0);
    out.tint = vec4<f32>(0.0);
    out.flags = instance.flags;
    out.world_pos = vec3<f32>(0.0);
    out.overlay = vec4<f32>(0.0);
    out.shade_kpqr = vec4<f32>(1.0, 0.0, 0.0, 0.0);
    out.shade_xyz = vec4<f32>(0.0);
    out.frame = vec4<f32>(0.0, 0.0, 1.0, 1.0);
    return out;
}

fn connector_screen_x(p: vec3<f32>) -> f32 {
    let clip = scene.view_proj * vec4<f32>(p, 1.0);
    return clip.x / max(clip.w, 1.0e-4);
}

@vertex
fn vs_main(
    @builtin(vertex_index) vertex_idx: u32,
    instance: InstanceInput,
) -> VertexOutput {
    if ((instance.flags & 4u) != 0u) {
        return culled_vertex(instance);
    }
    return billboard_vertex(vertex_idx, instance);
}

@vertex
fn vs_interface(
    @builtin(vertex_index) vertex_idx: u32,
    instance: InstanceInput,
) -> VertexOutput {
    if ((instance.flags & 4u) == 0u) {
        return culled_vertex(instance);
    }
    return billboard_vertex(vertex_idx, instance);
}

fn billboard_vertex(vertex_idx: u32, instance: InstanceInput) -> VertexOutput {
    var out: VertexOutput;

    var corner = vec2<f32>(0.0, 0.0);
    switch vertex_idx {
        case 0u: { corner = vec2<f32>(0.0, 0.0); }
        case 1u: { corner = vec2<f32>(1.0, 0.0); }
        case 2u: { corner = vec2<f32>(1.0, 1.0); }
        case 3u: { corner = vec2<f32>(0.0, 0.0); }
        case 4u: { corner = vec2<f32>(1.0, 1.0); }
        case 5u: { corner = vec2<f32>(0.0, 1.0); }
        default: { corner = vec2<f32>(0.0, 0.0); }
    }

    let is_flipped = (instance.flags & 1u) != 0u;
    var u: f32;
    if (is_flipped) {
        u = mix(instance.uv_max.x, instance.uv_min.x, corner.x);
    } else {
        u = mix(instance.uv_min.x, instance.uv_max.x, corner.x);
    }

    let v = mix(instance.uv_max.y, instance.uv_min.y, corner.y);

    out.uv = vec2<f32>(u, v);
    out.quad_uv = corner;
    let dims = vec2<f32>(textureDimensions(t_diffuse));
    let rect_min = round(min(instance.uv_min, instance.uv_max) * dims);
    let rect_size = max(round(abs(instance.uv_max - instance.uv_min) * dims), vec2<f32>(1.0));
    out.frame = vec4<f32>(rect_min, rect_size);
    out.tint = instance.tint;
    out.flags = instance.flags;
    out.overlay = vec4<f32>(0.0);
    out.shade_kpqr = instance.shade_kpqr;
    out.shade_xyz = instance.shade_xyz;

    if ((instance.flags & 4u) != 0u) {
        var viewport = scene.render_params.zw;
        if (viewport.x < 1.0 || viewport.y < 1.0) {
            viewport = vec2<f32>(1024.0, 768.0);
        }
        let px = mix(instance.uv_min, instance.uv_max, corner);
        let ndc_pos = vec2<f32>(px.x / viewport.x * 2.0 - 1.0, 1.0 - px.y / viewport.y * 2.0);
        if ((instance.flags & 1024u) != 0u) {
            out.clip_position = vec4<f32>(ndc_pos, 0.0, 1.0);
            out.world_pos = instance.position;
            return out;
        }
        let clip = scene.view_proj * vec4<f32>(instance.position, 1.0);
        if (clip.w <= 0.0001) {
            out.clip_position = vec4<f32>(0.0, 0.0, -1.0, 1.0);
            return out;
        }
        let depth = clip.z / clip.w;
        out.clip_position = vec4<f32>(ndc_pos, clamp(depth - 0.0002, 0.0, 1.0), 1.0);
        out.world_pos = instance.position;
        return out;
    }

    var world_pos: vec3<f32>;

    if ((instance.flags & 32768u) != 0u) {
        return velocity_quad_vertex(instance, corner, out);
    }
    if ((instance.flags & 524288u) != 0u) {
        return world_quad_vertex(instance, corner, out);
    }

    if ((instance.flags & 256u) != 0u) {
        let span = vec3<f32>(instance.anchor.x, instance.size.x, instance.anchor.y);
        let up = vec3<f32>(0.0, instance.size.y, 0.0);
        let end_is_left = connector_screen_x(instance.position + span) < connector_screen_x(instance.position)
            || connector_screen_x(instance.position + span + up) < connector_screen_x(instance.position + up);
        let edge = select(1.0 - corner.x, corner.x, end_is_left);
        let local = (vec2<f32>(mix(instance.uv_min.x, instance.uv_max.x, edge), v) * dims - rect_min) / rect_size;
        out.uv = 64.0 / 65536.0 + local * (65408.0 / 65536.0);
        world_pos = instance.position + span * corner.x + vec3<f32>(0.0, instance.size.y * corner.y, 0.0);
    } else if ((instance.flags & 32u) != 0u) {
        let yaw = instance.anchor.x;
        let fwd = vec3<f32>(cos(yaw), 0.0, sin(yaw));
        let right = vec3<f32>(-sin(yaw), 0.0, cos(yaw));
        let lateral = (corner.x - 0.5) * instance.size.x;
        let forward = corner.y * instance.size.y;
        let arrow_pos = instance.position + right * lateral + fwd * forward;
        out.overlay = vec4<f32>(lateral, forward, instance.uv_min);
        out.uv = vec2<f32>(instance.uv_max.x, (instance.size.y - forward) * instance.uv_max.y);
        out.world_pos = arrow_pos;
        let clip = scene.view_proj * vec4<f32>(arrow_pos, 1.0);
        out.clip_position = vec4<f32>(clip.xy, 0.0, clip.w);
        return out;
    } else if ((instance.flags & 512u) != 0u) {
        return screen_aligned_vertex(instance, corner, out);
    } else {
        let g = sprite_growth(instance.size, out.frame.zw);
        let grown = vec2<f32>(
            sprite_grown_corner(corner.x, instance.size.x, g.x),
            sprite_grown_corner(corner.y, instance.size.y, g.y),
        );
        out.uv = vec2<f32>(
            sprite_local_u(instance, corner.x, out.frame.z),
            sprite_border_coord(1.0 - corner.y, out.frame.w),
        );
        world_pos = sprite_corner(instance, grown);
    }

    out.world_pos = world_pos;
    out.clip_position = scene.view_proj * vec4<f32>(world_pos, 1.0);
    return out;
}

@fragment
fn fs_interface(in: VertexOutput) -> @location(0) vec4<f32> {
    return in.tint;
}

@fragment
fn fs_interface_srgb(in: VertexOutput) -> @location(0) vec4<f32> {
    return vec4<f32>(srgb_to_linear(in.tint.rgb), in.tint.a);
}

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    return shade_sprite(in);
}

@fragment
fn fs_main_srgb(in: VertexOutput) -> @location(0) vec4<f32> {
    let c = shade_sprite(in);
    if (c.a <= 0.0) {
        return vec4<f32>(0.0);
    }
    return vec4<f32>(srgb_to_linear(clamp(c.rgb / c.a, vec3<f32>(0.0), vec3<f32>(1.0))) * c.a, c.a);
}

fn shade_category4_soft(in: VertexOutput) -> vec4<f32> {
    let c = shade_sprite(in);
    if ((in.flags & 4096u) == 0u) {
        return c;
    }
    return c * reflection_edge_fade(in.clip_position.xy);
}

@fragment
fn fs_reflection_soft(in: VertexOutput) -> @location(0) vec4<f32> {
    return shade_category4_soft(in);
}

@fragment
fn fs_reflection_soft_srgb(in: VertexOutput) -> @location(0) vec4<f32> {
    let c = shade_category4_soft(in);
    if (c.a <= 0.0) {
        return vec4<f32>(0.0);
    }
    return vec4<f32>(srgb_to_linear(clamp(c.rgb / c.a, vec3<f32>(0.0), vec3<f32>(1.0))) * c.a, c.a);
}

const SPRITE_LOD_BIAS: f32 = -0.5;

fn sprite_level_origin(frame: vec4<f32>, k: u32) -> vec2<f32> {
    if (k == 0u) {
        return frame.xy;
    }
    let h = u32(frame.w);
    var y = frame.y;
    for (var j = 1u; j < k; j = j + 1u) {
        y = y + f32(max(h >> j, 1u) + 1u);
    }
    return vec2<f32>(frame.x + frame.z + 1.0, y);
}

fn sample_sprite_level(frame: vec4<f32>, f: vec2<f32>, k: u32) -> vec4<f32> {
    let base = vec2<u32>(frame.zw);
    let size = vec2<f32>(max(base >> vec2<u32>(k), vec2<u32>(1u)));
    let t = clamp(f * size, vec2<f32>(-0.5), size + vec2<f32>(0.5));
    let dims = vec2<f32>(textureDimensions(t_diffuse));
    return textureSampleLevel(t_diffuse, s_diffuse, (sprite_level_origin(frame, k) + t) / dims, 0.0);
}

const SPRITE_FILTER_SHARP: u32 = 1u;
const SPRITE_FILTER_NEAREST: u32 = 2u;
const SPRITE_FILTER_LANCZOS2: u32 = 3u;
const SPRITE_FILTER_SOFTWARE: u32 = 4u;

const SPRITE_FILTER_FADE_END: f32 = 1.41421356;

fn sprite_sharp_coord(frame: vec4<f32>, f: vec2<f32>, rate: vec2<f32>) -> vec2<f32> {
    let size = frame.zw;
    let t = clamp(f * size, vec2<f32>(-0.5), size + vec2<f32>(0.5));
    return sharp_coord(t, rate) / size;
}

fn sprite_nearest_coord(frame: vec4<f32>, f: vec2<f32>) -> vec2<f32> {
    let size = frame.zw;
    let t = clamp(f * size, vec2<f32>(-0.5), size + vec2<f32>(0.5));
    return (floor(t) + vec2<f32>(0.5)) / size;
}

fn sprite_lanczos2_weight(x: f32) -> f32 {
    let ax = abs(x);
    if (ax < 1e-5) {
        return 1.0;
    }
    if (ax >= 2.0) {
        return 0.0;
    }
    let px = 3.14159265 * ax;
    return 2.0 * sin(px) * sin(px * 0.5) / (px * px);
}

fn sprite_lanczos2(frame: vec4<f32>, f: vec2<f32>) -> vec4<f32> {
    let c = f * frame.zw - vec2<f32>(0.5);
    let base = floor(c);
    var wx: array<f32, 4>;
    var wy: array<f32, 4>;
    var sx = 0.0;
    var sy = 0.0;
    for (var k = 0; k < 4; k++) {
        let o = base + vec2<f32>(f32(k - 1));
        wx[k] = sprite_lanczos2_weight(o.x - c.x);
        wy[k] = sprite_lanczos2_weight(o.y - c.y);
        sx += wx[k];
        sy += wy[k];
    }
    let origin = vec2<i32>(frame.xy);
    let size = vec2<i32>(frame.zw);
    let first = vec2<i32>(base) - vec2<i32>(1);
    var sum = vec4<f32>(0.0);
    for (var j = 0; j < 4; j++) {
        let y = first.y + j;
        if (y < 0 || y >= size.y) {
            continue;
        }
        var row = vec4<f32>(0.0);
        for (var i = 0; i < 4; i++) {
            let x = first.x + i;
            if (x >= 0 && x < size.x) {
                row += wx[i] * textureLoad(t_diffuse, origin + vec2<i32>(x, y), 0);
            }
        }
        sum += wy[j] * row;
    }
    let p = clamp(sum / (sx * sy), vec4<f32>(0.0), vec4<f32>(1.0));
    return vec4<f32>(min(p.rgb, vec3<f32>(p.a)), p.a);
}

fn sample_sprite_base(frame: vec4<f32>, f: vec2<f32>, rho: f32, rate: vec2<f32>) -> vec4<f32> {
    let mode = u32(scene.render_params.y);
    if (mode == SPRITE_FILTER_SHARP) {
        return sample_sprite_level(frame, sprite_sharp_coord(frame, f, rate), 0u);
    }
    if (mode != SPRITE_FILTER_NEAREST && mode != SPRITE_FILTER_LANCZOS2) {
        return sample_sprite_level(frame, f, 0u);
    }
    var filtered: vec4<f32>;
    if (mode == SPRITE_FILTER_NEAREST) {
        filtered = sample_sprite_level(frame, sprite_nearest_coord(frame, f), 0u);
    } else {
        filtered = sprite_lanczos2(frame, f);
    }
    if (rho <= 1.0) {
        return filtered;
    }
    let fade = (rho - 1.0) / (SPRITE_FILTER_FADE_END - 1.0);
    return mix(filtered, sample_sprite_level(frame, f, 0u), clamp(fade, 0.0, 1.0));
}

fn sample_sprite(frame: vec4<f32>, f: vec2<f32>, rho: f32, rate: vec2<f32>) -> vec4<f32> {
    let lambda = log2(rho) + SPRITE_LOD_BIAS;
    if (!(lambda > 0.0)) {
        return sample_sprite_base(frame, f, rho, rate);
    }
    let q = f32(firstLeadingBit(u32(max(frame.z, frame.w))));
    let d1 = min(floor(lambda), q);
    let d2 = min(floor(lambda) + 1.0, q);
    let a = sample_sprite_level(frame, f, u32(d1));
    if (d2 <= d1) {
        return a;
    }
    let b = sample_sprite_level(frame, f, u32(d2));
    return mix(a, b, fract(lambda));
}

override SPRITE_HEIGHT_FOG_AFTER: bool = false;

fn shade_sprite(in: VertexOutput) -> vec4<f32> {
    let tex0 = in.uv * in.frame.zw;
    let tex0_dx = dpdx(tex0);
    let tex0_dy = dpdy(tex0);
    let rho = max(length(tex0_dx), length(tex0_dy));
    let rate = vec2<f32>(length(vec2<f32>(tex0_dx.x, tex0_dy.x)), length(vec2<f32>(tex0_dx.y, tex0_dy.y)));
    let pixel = floor(in.clip_position.xy);
    let exact = pixel * (vec2<f32>(1.0) + in.software_snap.xy) + in.software_snap.zw;
    let shift = exact - in.clip_position.xy;
    let software_uv = in.uv + dpdx(in.uv) * shift.x + dpdy(in.uv) * shift.y;

    if ((in.flags & 4u) != 0u) {
        discard;
    }

    if ((in.flags & 32u) != 0u) {
        let lateral = abs(in.overlay.x);
        let forward = in.overlay.y;
        let in_shaft = lateral <= in.overlay.z && forward >= 0.0 && forward <= in.overlay.w;
        let in_head = forward >= in.uv.x && lateral <= in.uv.y;
        if (!in_shaft && !in_head) {
            discard;
        }
        return vec4<f32>(in.tint.rgb * in.tint.a, in.tint.a);
    }

    var color: vec4<f32>;
    if (u32(scene.render_params.y) == SPRITE_FILTER_SOFTWARE) {
        color = sample_sprite_level(in.frame, sprite_nearest_coord(in.frame, software_uv), 0u);
    } else {
        color = sample_sprite(in.frame, in.uv, rho, rate);
    }

    if ((in.flags & 8192u) != 0u) {
        let l = dot(vec3<f32>(0.4, 0.6, 0.2), color.rgb);
        let s = color.rgb * in.shade_kpqr.x + color.a * in.shade_kpqr.yzw + l * in.shade_xyz.xyz;
        color = vec4<f32>(s, color.a);
    }

    color = color * in.tint.a;
    color = vec4<f32>(color.rgb * in.tint.rgb, color.a);
    if (color.a < 0.01) {
        return vec4<f32>(0.0);
    }
    if (scene.ambient_tint.a > 0.0) {
        let base_ambient = color.rgb * scene.ambient_tint.rgb;
        var lit_color = base_ambient;
        let count = min(scene.light_params.x, 16u);
        if (count > 0u) {
            for (var i = 0u; i < count; i = i + 1u) {
                let light = scene.lights[i];
                let light_pos = light.pos.xyz;
                let radius = light.pos.w;
                let delta = light_pos - in.world_pos;
                let d2 = dot(delta, delta);
                let r2 = radius * radius;
                if (d2 < r2 && radius > 0.0) {
                    let d = sqrt(d2);
                    let norm_d = 1.0 - d / radius;
                    let atten = (norm_d * norm_d) / (1.0 + 0.1 * d2);
                    lit_color += light.color.rgb * (light.color.w * atten) * color.rgb;
                }
            }
        }
        color = vec4<f32>(lit_color, color.a);
    }
    let height_fog_after = SPRITE_HEIGHT_FOG_AFTER && scene.fog_height.x != 0.0;
    if ((scene.fog.a != 0.0 || scene.fog_height.x != 0.0) && !height_fog_after) {
        color = vec4<f32>(mix(color.a * scene.fog.rgb, color.rgb, height_fog_factor(in.clip_position, in.world_pos)), color.a);
    }
    return clamp(color, vec4<f32>(0.0), vec4<f32>(1.0));
}
