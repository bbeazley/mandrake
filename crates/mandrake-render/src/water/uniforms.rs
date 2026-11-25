use super::*;

/// `WaterUniforms` of `water.wgsl`.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Pod, Zeroable)]
pub(super) struct WaterUniforms {
    pub(super) grid: [f32; 4],
    pub(super) wobble: [f32; 4],
    pub(super) phase: [f32; 4],
    pub(super) absorb: [f32; 4],
    pub(super) water_color: [f32; 4],
    pub(super) surface: [f32; 4],
    pub(super) light: [f32; 4],
    pub(super) light_color: [f32; 4],
    pub(super) falloff: [f32; 4],
    pub(super) extinction: [f32; 4],
    pub(super) backscatter: [f32; 4],
    pub(super) particle: [f32; 4],
    pub(super) refraction: [f32; 4],
    pub(super) ripple: [f32; 4],
    pub(super) wet: [f32; 4],
    /// 1 with `wind_ripples`, unused x3.
    pub(super) fft: [f32; 4],
    /// Per cascade: tile scale (uv per render unit), the band's middle wavenumber (per render
    /// unit), unused x2.
    pub(super) cascades: [[f32; 4]; crate::water::fft::CASCADES],
    /// `gusts`: strength (0 off), 1 / patch size (per render unit), the drift offset (patches).
    pub(super) gust: [f32; 4],
    /// The wind waves' spectrum (per metre): the developed peak `k_g`, the saturation constant
    /// `B`, the finest cascade's Nyquist, the short cut `2 pi / ripple_min_wavelength`.
    pub(super) spectrum: [f32; 4],
    /// `shelter`: 1 when on, the ripple wind (m/s), metres per render unit, the long cut
    /// `2 pi / ripple_max_wavelength` (per metre).
    pub(super) fetch: [f32; 4],
    /// `glint_sparkle`: strength, a facet's size (render units), a point's lifetime, the clock.
    pub(super) glint: [f32; 4],
    /// The reflections: unused, strength, distortion (render units), blur (screen heights).
    pub(super) reflection: [f32; 4],
    /// `glint_size` (pixels), `bed_blur`, unused x2.
    pub(super) reflection_extra: [f32; 4],
}

/// A 16-bit phase step as the alias nearest zero, in 1/65536 turns.
pub(super) fn wrapped_step(step: i32) -> f32 {
    f32::from(step as i16)
}

/// The uniforms at the load: the settings and the map's (`mesh` placed with `cell_size`, the media
/// `wobble`), with the light, the map colour, the ripple field and the wind set later
/// ([`WaterShading::set_landscape`], [`WaterShading::new`], [`WaterShading::set_wind`]).
pub(super) fn initial(
    mesh: &mandrake_tag::MeshTag,
    cell_size: f32,
    wobble: &MediaWobble,
    settings: &WaterSettings,
    display_format: wgpu::TextureFormat,
) -> WaterUniforms {
    // The glint is the sun's mirror image, so white; the mesh header's light colour is the
    // baked shading's palette tint (e.g. Salvation's (1, 0.63, 0)), mixed in by `sheen_tint`.
    let light = mesh
        .landscape_shading
        .light_color
        .map(|c| 1.0 + (f32::from(c) / 65535.0 - 1.0) * settings.sheen_tint);
    // An explicit direction now; otherwise the sheen waits for the fit
    // ([`Self::set_landscape`]).
    let (dir, specular) = match settings.light_direction {
        Some(d) => (normalized(d), settings.specular),
        None => ([0.0, 1.0, 0.0], 0.0),
    };
    let f = i32::from(wobble.frequency[2]);
    // Coefficients per metre to per render unit.
    let per_cell = settings.meters_per_cell / cell_size;
    let uniforms = WaterUniforms {
        grid: [
            f32::from(mesh.cell_width) * cell_size * 0.5,
            f32::from(mesh.cell_height) * cell_size * 0.5,
            1.0 / cell_size,
            settings.wave_depth * cell_size,
        ],
        wobble: [
            f32::from(wobble.amplitude[2]) / 512.0,
            settings.normal_strength,
            settings.shore_fade,
            settings.surface_fade,
        ],
        phase: [
            0.0,
            wrapped_step(0x179i32.wrapping_mul(f)),
            wrapped_step(0xd3i32.wrapping_mul(f)),
            0.0,
        ],
        absorb: [
            settings.absorption[0] / cell_size,
            settings.absorption[1] / cell_size,
            settings.absorption[2] / cell_size,
            // `Map` stays the fixed colour until [`Self::set_map_colors`].
            0.0,
        ],
        water_color: [
            settings.water_color[0],
            settings.water_color[1],
            settings.water_color[2],
            settings.color_scale,
        ],
        surface: [
            settings.surface_opacity,
            settings.reflection_min,
            settings.fresnel_power,
            // The under-draws write display values unless the display view is sRGB.
            if display_format.is_srgb() { 0.0 } else { 1.0 },
        ],
        light: [dir[0], dir[1], dir[2], specular],
        light_color: [light[0], light[1], light[2], settings.shininess],
        falloff: [
            settings.shallow_tint,
            settings.deep_max,
            settings.swell,
            0.0,
        ],
        extinction: {
            let a = settings.pure_absorption;
            let d = settings.dissolved_absorption;
            let p = settings.turbidity * settings.turbidity_absorption;
            [
                (a[0] + d[0] + p) * per_cell,
                (a[1] + d[1] + p) * per_cell,
                (a[2] + d[2] + p) * per_cell,
                if settings.physical { 1.0 } else { 0.0 },
            ]
        },
        backscatter: [
            PURE_BACKSCATTER[0] * per_cell,
            PURE_BACKSCATTER[1] * per_cell,
            PURE_BACKSCATTER[2] * per_cell,
            if settings.scatter_tint { 1.0 } else { 0.0 },
        ],
        particle: [
            settings.turbidity * settings.backscatter_ratio * per_cell,
            0.0,
            0.0,
            0.0,
        ],
        refraction: [
            if settings.refraction { 1.0 } else { 0.0 },
            settings.refraction_strength,
            1.0 / WATER_IOR,
            0.0,
        ],
        // [`Self::new`] sets it once the field is made.
        ripple: [0.0; 4],
        wet: [
            settings.wet_height * cell_size,
            // 0 with wet sand off: the bed under the water is not darkened either.
            if settings.wet_sand {
                settings.wet_darken
            } else {
                0.0
            },
            if settings.wet_sand {
                settings.wet_specular
            } else {
                0.0
            },
            settings.wet_distance,
        ],
        fft: [if settings.wind_ripples { 1.0 } else { 0.0 }, 0.0, 0.0, 0.0],
        gust: [
            if settings.gusts && settings.wind_ripples {
                settings.gust_strength
            } else {
                0.0
            },
            per_cell / settings.gust_scale.max(1e-3),
            0.0,
            0.0,
        ],
        cascades: std::array::from_fn(|c| {
            let (lo, hi) = crate::water::fft::band(c);
            let middle = if lo > 0.0 { (lo * hi).sqrt() } else { hi * 0.5 };
            [
                per_cell / crate::water::fft::TILE_METERS[c],
                (middle * f64::from(per_cell)) as f32,
                middle as f32,
                0.0,
            ]
        }),
        reflection: [
            0.0,
            settings.reflection_strength,
            settings.reflection_distortion * cell_size,
            settings.reflection_blur,
        ],
        // [`Self::set_wind`].
        spectrum: [0.0; 4],
        glint: [
            settings.glint_sparkle.max(0.0),
            settings.glint_facet / per_cell.max(1e-6),
            settings.glint_lifetime,
            0.0,
        ],
        reflection_extra: [
            settings.glint_size,
            settings.bed_blur,
            settings.shore_swash,
            settings.min_depth / per_cell.max(1e-6),
        ],
        fetch: [
            if settings.shelter && settings.wind_ripples {
                1.0
            } else {
                0.0
            },
            0.0,
            per_cell,
            std::f32::consts::TAU / settings.ripple_max_wavelength.max(1e-3),
        ],
    };
    tracing::info!(
        "--water enhanced: wobble amplitude {:?} frequency {:?}, phase steps {:?}, light colour {:?}",
        wobble.amplitude,
        wobble.frequency,
        &uniforms.phase[1..3],
        light
    );
    uniforms
}
