struct WaterUniforms {
    grid: vec4<f32>,
    wobble: vec4<f32>,
    phase: vec4<f32>,
    absorb: vec4<f32>,
    water_color: vec4<f32>,
    surface: vec4<f32>,
    light: vec4<f32>,
    light_color: vec4<f32>,
    falloff: vec4<f32>,
    extinction: vec4<f32>,
    backscatter: vec4<f32>,
    particle: vec4<f32>,
    refraction: vec4<f32>,
    ripple: vec4<f32>,
    wet: vec4<f32>,
    fft: vec4<f32>,
    cascades: array<vec4<f32>, 3>,
    gust: vec4<f32>,
    spectrum: vec4<f32>,
    fetch: vec4<f32>,
    glint: vec4<f32>,
    reflection: vec4<f32>,
    reflection_extra: vec4<f32>,
};

const WATER_G: f32 = 9.81;
const FETCH_LOG2_MAX: f32 = 8.0056245;

@group(3) @binding(0)
var<uniform> water: WaterUniforms;

@group(3) @binding(1)
var t_floor: texture_2d<f32>;

@group(3) @binding(2)
var t_water_color: texture_2d<f32>;

@group(3) @binding(3)
var t_cell_light: texture_2d<f32>;

@group(3) @binding(4)
var t_ripple: texture_2d<f32>;

@group(3) @binding(5)
var t_waves: texture_2d_array<f32>;

@group(3) @binding(6)
var s_waves: sampler;

@group(3) @binding(7)
var t_fetch: texture_2d<f32>;

@group(3) @binding(8)
var t_reflections: texture_2d<f32>;

@group(3) @binding(9)
var t_submerged: texture_2d<f32>;

@group(3) @binding(10)
var t_submerged_height: texture_2d<f32>;

struct WaterVertexOutput {
    @invariant @builtin(position) clip_position: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) affine: vec3<f32>,
    @location(2) world_pos: vec3<f32>,
    @location(3) level: vec2<f32>,
};

fn vertex_grid(uv: vec2<f32>) -> vec2<f32> {
    let dims = vec2<f32>(textureDimensions(t_floor)) - vec2<f32>(1.0);
    return round(uv * dims * 2.0) * 0.5;
}

fn grid_info(cr: vec2<f32>) -> vec4<f32> {
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

fn water_vertex(position: vec3<f32>, uv: vec2<f32>, info: vec4<f32>) -> WaterVertexOutput {
    var out: WaterVertexOutput;
    out.clip_position = scene.view_proj * vec4<f32>(position, 1.0);
    out.uv = uv;
    out.world_pos = position;
    out.affine = vec3<f32>(uv * out.clip_position.w, out.clip_position.w);
    out.level = info.yz;
    return out;
}

@vertex
fn vs_bed(in: VertexInput) -> WaterVertexOutput {
    return water_vertex(in.position, in.uv, grid_info(vertex_grid(in.uv)));
}

@vertex
fn vs_surface(in: VertexInput) -> WaterVertexOutput {
    let cr = vertex_grid(in.uv);
    let info = grid_info(cr);
    let cell = 1.0 / water.grid.z;
    var p = vec3<f32>(water.grid.x - cr.x * cell, info.y, cr.y * cell - water.grid.y);
    if (info.w > 0.999 && all(fract(cr) == vec2<f32>(0.0))) {
        let depth = max(info.y - info.x, 0.0);
        var d = (in.position - p) * wave_scale(depth) * water.falloff.z;
        d.y = max(d.y, -0.9 * depth);
        p = p + d;
    }
    return water_vertex(p, in.uv, info);
}

fn wave_scale(depth: f32) -> f32 {
    if (water.grid.w <= 0.0) {
        return 1.0;
    }
    return clamp(depth / water.grid.w, 0.0, 1.0);
}

fn water_grid(world: vec3<f32>) -> vec2<f32> {
    return vec2<f32>(
        (water.grid.x - world.x) * water.grid.z,
        (world.z + water.grid.y) * water.grid.z,
    );
}

const WATER_TAU: f32 = 6.28318530718;
