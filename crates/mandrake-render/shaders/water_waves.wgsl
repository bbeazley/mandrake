// `--water enhanced`: the surface's slope (the engine's wobble, the interactive ripples, the
// FFT wind waves under the gusts and the fetch limit). Part of `water.wgsl`'s module
// (`WaterShading::SHADER_SRC`).

// The surface normal of the engine's z wobble (`MediaWobble::offset`): the vertex at (col, row)
// rises by `a * sin(2 pi (t f + col 0x179 f + row 0xd3 f) / 65536)`. Its analytic derivative,
// with the per-cell phase steps taken as the alias nearest zero, so the continuous wave passes
// through the vertices' own values. `normal strength` scales the slope.
//
// Returns the normal with the wind ripples' slope variance faded out for the pixel footprint
// `footprint` (render units, `water_footprint`) in `w`, for the sheen's roughness.
fn water_normal(world: vec3<f32>, footprint: f32) -> vec4<f32> {
    let cr = water_grid(world);
    let turns = (water.phase.x + cr.x * water.phase.y + cr.y * water.phase.z) / 65536.0;
    let wave = cos(WATER_TAU * fract(turns));
    let info = grid_info(cr);
    let depth = max(info.y - info.x, 0.0);
    let k = water.wobble.x * wave * WATER_TAU / 65536.0 * water.wobble.y
        * wave_scale(depth) * water.falloff.z;
    let dh_dcol = k * water.phase.y;
    let dh_drow = k * water.phase.z;
    let ripple = ripple_slope(cr);
    let k_p = fetch_peak(cr);
    var wind = vec3<f32>(0.0);
    if (water.fft.x > 0.5) {
        wind = fft_slope(world, depth, footprint, k_p);
    }
    let local = gust_at(world);
    wind = vec3<f32>(wind.xy * local, wind.z * local * local);
    // x falls as the column grows, z grows with the row.
    let dh_dx = -(dh_dcol + ripple.x) * water.grid.z + wind.x;
    let dh_dz = (dh_drow + ripple.y) * water.grid.z + wind.y;
    return vec4<f32>(normalize(vec3<f32>(-dh_dx, 1.0, -dh_dz)), wind.z);
}

// `wind_ripples` (`water_fft`; the developer's exception for the wave pattern): the
// cascades' slope at `world` and the variance under the pixel. Each cascade is sampled at the mip
// of the pixel's footprint, so its mean is the visible slope and `E[s^2] - E[s]^2` the slope the
// pixel averages away (Toksvig); the shallows keep `sqrt(tanh(k h))` of a cascade (its band's
// middle `k`).
fn fft_slope(world: vec3<f32>, depth: f32, footprint: f32, k_p: f32) -> vec3<f32> {
    var slope = vec2<f32>(0.0);
    var lost = 0.0;
    for (var c = 0; c < 3; c++) {
        let cascade = water.cascades[c];
        let g = footprint * cascade.x;
        let t = textureSampleGrad(
            t_waves,
            s_waves,
            world.xz * cascade.x,
            c,
            vec2<f32>(g, 0.0),
            vec2<f32>(0.0, g),
        );
        // The shallows and the fetch: the band's middle wavenumber.
        let keep = sqrt(tanh(cascade.y * max(depth, 1.0e-4))) * peak_weight(cascade.z, k_p);
        slope += t.xy * keep;
        lost += max(t.z - dot(t.xy, t.xy), 0.0) * keep * keep;
    }
    lost += sub_band_variance(k_p, water.spectrum.z);
    return vec3<f32>(slope, lost);
}

// `gusts` (the developer's exception for the wave pattern): the local share of the ripples, a
// smooth two-octave value noise (`fog.wgsl`'s) of patches drifting with the wind, `1 +
// strength * (2 n - 1)`: rough patches and glassy calm between.
fn gust_at(world: vec3<f32>) -> f32 {
    if (water.gust.x <= 0.0) {
        return 1.0;
    }
    let p = world.xz * water.gust.y + water.gust.zw;
    let n = 0.65 * fog_value_noise(p) + 0.35 * fog_value_noise(p * 2.13 + vec2<f32>(17.0, 5.0));
    return max(1.0 + water.gust.x * (2.0 * n - 1.0), 0.0);
}

// `shelter`: the peak wavenumber (per metre) of the wind waves at grid point `cr`, limited by
// the fetch `F` there (JONSWAP: `w_p = 22 (g^2 / (U F))^(1/3)`, `k_p = w_p^2 / g`), never below
// the fully developed peak. With `shelter` off, the developed peak.
fn fetch_peak(cr: vec2<f32>) -> f32 {
    let developed = water.spectrum.x;
    if (water.fetch.x < 0.5 || water.fft.x < 0.5) {
        return developed;
    }
    let dims = vec2<f32>(textureDimensions(t_fetch));
    let r = textureSampleLevel(t_fetch, s_diffuse, (cr + vec2<f32>(0.5)) / dims, 0.0).r;
    let fetch = max(exp2(r * FETCH_LOG2_MAX) - 1.0, 0.05);
    let w_p = 22.0 * pow(WATER_G * WATER_G / (max(water.fetch.y, 0.1) * fetch), 1.0 / 3.0);
    return max(w_p * w_p / WATER_G, developed);
}

// The share of a wave component's (or band's) slope at wavenumber `k` (per metre) kept under a
// peak at `k_p`: the spectrum's `exp(-(k_p / k)^2)` against the developed one the waves were
// made for, as amplitude.
fn peak_weight(k: f32, k_p: f32) -> f32 {
    let g = water.spectrum.x;
    return exp(-0.5 * (k_p * k_p - g * g) / max(k * k, 1.0e-6));
}

// The slope variance of the ripples shorter than the finest cascade under a peak at `k_p`: `B * int exp(-(k_p/k)^2) exp(-(k/k_hi)^2) d ln k` from
// `lowest` to four times the short cut, 6 steps. They are below any pixel: sheen roughness only.
fn sub_band_variance(k_p: f32, lowest: f32) -> f32 {
    let to = 4.0 * water.spectrum.w;
    if (to <= lowest || lowest <= 0.0) {
        return 0.0;
    }
    let d = log(to / lowest) / 6.0;
    var sum = 0.0;
    for (var i = 0; i < 6; i++) {
        let k = lowest * exp((f32(i) + 0.5) * d);
        let x = k_p / k;
        let y = k / water.spectrum.w;
        sum += exp(-x * x - y * y);
    }
    return water.spectrum.y * sum * d;
}

// How much of the water is there at grid point `cr`: 0 at the drawn surface's edge (the sim's
// media coverage, a cell stair), 1 from 1.5 cells inside it, smooth over the interpolated signed
// distance, so the contour is rounded and the water fades out to nothing at the edge whatever the
// terrain does there (it may still be under the level beyond). A true shore, where the terrain
// rises out of the water, is well inside it.
fn coverage_at(cr: vec2<f32>) -> f32 {
    let dims = vec2<f32>(textureDimensions(t_fetch));
    let g = textureSampleLevel(t_fetch, s_diffuse, (cr + vec2<f32>(0.5)) / dims, 0.0).g;
    return smoothstep(1.0, 2.5, g * 16.0 - 8.0);
}

// The world size of a pixel at a point (render units); take it before any `discard`.
fn water_footprint(world: vec3<f32>) -> f32 {
    return max(length(dpdx(world)), length(dpdy(world)));
}

// `ripples`: the wave field's slope at grid point `cr` (render units per cell, along the column
// and the row), central differences of the bilinear field one texel each way.
fn ripple_slope(cr: vec2<f32>) -> vec2<f32> {
    if (water.ripple.x < 0.5) {
        return vec2<f32>(0.0);
    }
    let dims = vec2<f32>(textureDimensions(t_ripple));
    let uv = (cr - water.ripple.yz) * water.ripple.w / dims;
    let e = vec2<f32>(1.0) / dims;
    let hx = textureSampleLevel(t_ripple, s_diffuse, uv + vec2<f32>(e.x, 0.0), 0.0).r
        - textureSampleLevel(t_ripple, s_diffuse, uv - vec2<f32>(e.x, 0.0), 0.0).r;
    let hy = textureSampleLevel(t_ripple, s_diffuse, uv + vec2<f32>(0.0, e.y), 0.0).r
        - textureSampleLevel(t_ripple, s_diffuse, uv - vec2<f32>(0.0, e.y), 0.0).r;
    return vec2<f32>(hx, hy) * 0.5 * water.ripple.w;
}
