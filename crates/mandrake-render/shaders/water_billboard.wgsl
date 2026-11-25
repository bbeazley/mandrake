
// `--water enhanced` with physics (`mandrake_render::water::WaterShading`): category 4 split
// in two. Appended to billboard.wgsl and fog.wgsl; billboard.wgsl itself is unchanged.
//
// - The reflections (`fs_water_reflection*`) go to the reflection layer, weighted there by the
//   surface's Fresnel term.
// - What is seen through or on the water (`fs_water_submerged`): the engine's underwater copies
//   of clipped sprites (`BILLBOARD_FLAG_SUBMERGED`, the engine's own flag) and the media's local
//   effects on the surface (`BILLBOARD_FLAG_MEDIA_EFFECT`: splashes, fish) go to the submerged
//   layer: their colour, and their height (render units) for the water's fog per pixel in
//   `water.wgsl` (`fs_bed_under`). The copy is drawn at the object's own opacity, not the
//   engine's 31/255 (`crate::media::surface_opacity(0xe000)`): the water's fog replaces it.

// The display view is sRGB (the frame has no non-sRGB view): linear output as `fs_main_srgb`.
override WATER_SRGB: bool = false;

const WATER_FLAG_SUBMERGED: u32 = 16384u;
const WATER_FLAG_MEDIA_EFFECT: u32 = 262144u;
// `crate::media::surface_opacity(SUBMERGED_COPY_TRANSPARENCY)`.
const WATER_SUBMERGED_OPACITY: f32 = 31.0 / 255.0;

fn water_seen_through(flags: u32) -> bool {
    return (flags & (WATER_FLAG_SUBMERGED | WATER_FLAG_MEDIA_EFFECT)) != 0u;
}

fn water_target(c: vec4<f32>) -> vec4<f32> {
    if (!WATER_SRGB || c.a <= 0.0) {
        return c;
    }
    return vec4<f32>(srgb_to_linear(clamp(c.rgb / c.a, vec3<f32>(0.0), vec3<f32>(1.0))) * c.a, c.a);
}

@vertex
fn vs_water_reflection(
    @builtin(vertex_index) vertex_idx: u32,
    instance: InstanceInput,
) -> VertexOutput {
    if ((instance.flags & 4u) != 0u || water_seen_through(instance.flags)) {
        return culled_vertex(instance);
    }
    return billboard_vertex(vertex_idx, instance);
}

@vertex
fn vs_water_submerged(
    @builtin(vertex_index) vertex_idx: u32,
    instance: InstanceInput,
) -> VertexOutput {
    if ((instance.flags & 4u) != 0u || !water_seen_through(instance.flags)) {
        return culled_vertex(instance);
    }
    return billboard_vertex(vertex_idx, instance);
}

@fragment
fn fs_water_reflection(in: VertexOutput) -> @location(0) vec4<f32> {
    let c = shade_sprite(in);
    if (water_seen_through(in.flags)) {
        discard;
    }
    return water_target(c);
}

@fragment
fn fs_water_reflection_soft(in: VertexOutput) -> @location(0) vec4<f32> {
    let c = shade_category4_soft(in);
    if (water_seen_through(in.flags)) {
        discard;
    }
    return water_target(c);
}

struct WaterSubmergedOut {
    // Premultiplied display colour (as the reflections).
    @location(0) color: vec4<f32>,
    // Height times coverage, coverage (blended over like the colour: height = r / g).
    @location(1) height: vec4<f32>,
};

@fragment
fn fs_water_submerged(in: VertexOutput) -> WaterSubmergedOut {
    var v = in;
    if ((in.flags & WATER_FLAG_SUBMERGED) != 0u) {
        v.tint.a = min(in.tint.a / WATER_SUBMERGED_OPACITY, 1.0);
    }
    let c = shade_sprite(v);
    if (!water_seen_through(in.flags)) {
        discard;
    }
    var out: WaterSubmergedOut;
    out.color = water_target(c);
    out.height = vec4<f32>(in.world_pos.y * c.a, c.a, 0.0, c.a);
    return out;
}
