struct DownsampleParams {
    in_w: u32,
    in_h: u32,
    out_w: u32,
    out_h: u32,
    scale: f32,
    timestep: f32,
    has_flow: u32,
    _pad: u32,
};

@group(0) @binding(0) var<uniform> ds_p: DownsampleParams;
@group(0) @binding(1) var<storage, read> ds_img0: array<f32>;
@group(0) @binding(2) var<storage, read> ds_img1: array<f32>;
@group(0) @binding(3) var<storage, read> ds_flow: array<f32>;
@group(0) @binding(4) var<storage, read> ds_mask: array<f32>;
@group(0) @binding(5) var<storage, read_write> ds_out: array<f32>;

@compute @workgroup_size(8, 8, 1)
fn downsample_concat(@builtin(global_invocation_id) gid: vec3<u32>) {
    let x = gid.x;
    let y = gid.y;
    if (x >= ds_p.out_w || y >= ds_p.out_h) {
        return;
    }
    let sx = clamp((f32(x) + 0.5) * ds_p.scale - 0.5, 0.0, f32(ds_p.in_w - 1u));
    let sy = clamp((f32(y) + 0.5) * ds_p.scale - 0.5, 0.0, f32(ds_p.in_h - 1u));
    let x0 = u32(floor(sx));
    let y0 = u32(floor(sy));
    let x1 = min(x0 + 1u, ds_p.in_w - 1u);
    let y1 = min(y0 + 1u, ds_p.in_h - 1u);
    let fx = sx - f32(x0);
    let fy = sy - f32(y0);
    let in_hw = ds_p.in_w * ds_p.in_h;
    let i00 = y0 * ds_p.in_w + x0;
    let i01 = y0 * ds_p.in_w + x1;
    let i10 = y1 * ds_p.in_w + x0;
    let i11 = y1 * ds_p.in_w + x1;

    let out_hw = ds_p.out_w * ds_p.out_h;
    let pix = y * ds_p.out_w + x;

    for (var c = 0u; c < 3u; c++) {
        let base = c * in_hw;
        let v0 = mix(
            mix(ds_img0[base + i00], ds_img0[base + i01], fx),
            mix(ds_img0[base + i10], ds_img0[base + i11], fx),
            fy,
        );
        let v1 = mix(
            mix(ds_img1[base + i00], ds_img1[base + i01], fx),
            mix(ds_img1[base + i10], ds_img1[base + i11], fx),
            fy,
        );
        ds_out[c * out_hw + pix] = v0;
        ds_out[(c + 3u) * out_hw + pix] = v1;
    }
    ds_out[6u * out_hw + pix] = ds_p.timestep;

    if (ds_p.has_flow != 0u) {
        let m = mix(
            mix(ds_mask[i00], ds_mask[i01], fx),
            mix(ds_mask[i10], ds_mask[i11], fx),
            fy,
        );
        ds_out[7u * out_hw + pix] = m;
        let inv_scale = 1.0 / ds_p.scale;
        for (var c = 0u; c < 4u; c++) {
            let base = c * in_hw;
            let fl = mix(
                mix(ds_flow[base + i00], ds_flow[base + i01], fx),
                mix(ds_flow[base + i10], ds_flow[base + i11], fx),
                fy,
            );
            ds_out[(8u + c) * out_hw + pix] = fl * inv_scale;
        }
    }
}

struct ConvParams {
    in_c: u32,
    in_c_pairs: u32,
    out_c: u32,
    in_w: u32,
    in_h: u32,
    out_w: u32,
    out_h: u32,
    stride: u32,
    has_res: u32,
    _p0: u32,
    _p1: u32,
    _p2: u32,
};

@group(0) @binding(0) var<uniform> c_p: ConvParams;
@group(0) @binding(1) var<storage, read> c_in: array<f32>;
@group(0) @binding(2) var<storage, read> c_w: array<u32>;
@group(0) @binding(3) var<storage, read> c_b: array<f32>;
@group(0) @binding(4) var<storage, read_write> c_out: array<f32>;

@compute @workgroup_size(8, 8, 1)
fn conv2d_3x3(@builtin(global_invocation_id) gid: vec3<u32>) {
    let x = gid.x;
    let y = gid.y;
    let oc = gid.z;
    if (x >= c_p.out_w || y >= c_p.out_h || oc >= c_p.out_c) {
        return;
    }
    let in_hw = c_p.in_w * c_p.in_h;
    let pairs = c_p.in_c_pairs;
    var sum = c_b[oc];

    for (var ky = 0u; ky < 3u; ky++) {
        let iy = i32(y * c_p.stride) + i32(ky) - 1;
        if (iy < 0 || iy >= i32(c_p.in_h)) {
            continue;
        }
        let row_off = u32(iy) * c_p.in_w;
        for (var kx = 0u; kx < 3u; kx++) {
            let ix = i32(x * c_p.stride) + i32(kx) - 1;
            if (ix < 0 || ix >= i32(c_p.in_w)) {
                continue;
            }
            let in_pix = row_off + u32(ix);
            let w_base = (oc * 9u + ky * 3u + kx) * pairs;
            for (var p = 0u; p < pairs; p++) {
                let w2 = unpack2x16float(c_w[w_base + p]);
                let ic0 = p * 2u;
                sum += w2.x * c_in[ic0 * in_hw + in_pix];
                if (ic0 + 1u < c_p.in_c) {
                    sum += w2.y * c_in[(ic0 + 1u) * in_hw + in_pix];
                }
            }
        }
    }

    let out_hw = c_p.out_w * c_p.out_h;
    let out_pix = y * c_p.out_w + x;
    if (c_p.has_res != 0u) {
        let beta = c_b[c_p.out_c + oc];
        sum = sum * beta + c_in[oc * out_hw + out_pix];
    }
    if (sum < 0.0) {
        sum *= 0.2;
    }
    c_out[oc * out_hw + out_pix] = sum;
}

struct LastConvParams {
    in_c: u32,
    in_c_pairs: u32,
    fw: u32,
    fh: u32,
    out_w: u32,
    out_h: u32,
    _p0: u32,
    _p1: u32,
};

@group(0) @binding(0) var<uniform> lc_p: LastConvParams;
@group(0) @binding(1) var<storage, read> lc_in: array<f32>;
@group(0) @binding(2) var<storage, read> lc_w: array<u32>;
@group(0) @binding(3) var<storage, read> lc_b: array<f32>;
@group(0) @binding(4) var<storage, read_write> lc_out: array<f32>;

@compute @workgroup_size(8, 8, 1)
fn conv_transpose_ps(@builtin(global_invocation_id) gid: vec3<u32>) {
    let px = gid.x;
    let py = gid.y;
    let oc = gid.z;
    if (px >= lc_p.out_w || py >= lc_p.out_h || oc >= 5u) {
        return;
    }
    let tx = px >> 1u;
    let sx = px & 1u;
    let ty = py >> 1u;
    let sy = py & 1u;
    let ct_oc = oc * 4u + sy * 2u + sx;

    let ky0 = (ty + 1u) & 1u;
    let kx0 = (tx + 1u) & 1u;
    let fhw = lc_p.fw * lc_p.fh;
    let pairs = lc_p.in_c_pairs;
    var sum = lc_b[ct_oc];

    for (var dy = 0u; dy < 2u; dy++) {
        let ky = ky0 + dy * 2u;
        let iy = (i32(ty) + 1 - i32(ky)) / 2;
        if (iy < 0 || iy >= i32(lc_p.fh)) {
            continue;
        }
        let row_off = u32(iy) * lc_p.fw;
        for (var dx = 0u; dx < 2u; dx++) {
            let kx = kx0 + dx * 2u;
            let ix = (i32(tx) + 1 - i32(kx)) / 2;
            if (ix < 0 || ix >= i32(lc_p.fw)) {
                continue;
            }
            let in_pix = row_off + u32(ix);
            let w_base = (ct_oc * 16u + ky * 4u + kx) * pairs;
            for (var p = 0u; p < pairs; p++) {
                let w2 = unpack2x16float(lc_w[w_base + p]);
                let ic0 = p * 2u;
                sum += w2.x * lc_in[ic0 * fhw + in_pix] + w2.y * lc_in[(ic0 + 1u) * fhw + in_pix];
            }
        }
    }

    lc_out[oc * lc_p.out_w * lc_p.out_h + py * lc_p.out_w + px] = sum;
}

struct UpsampleParams {
    in_w: u32,
    in_h: u32,
    out_w: u32,
    out_h: u32,
    scale: f32,
    accum: u32,
    _p0: u32,
    _p1: u32,
};

@group(0) @binding(0) var<uniform> up_p: UpsampleParams;
@group(0) @binding(1) var<storage, read> up_in: array<f32>;
@group(0) @binding(2) var<storage, read_write> up_flow: array<f32>;
@group(0) @binding(3) var<storage, read_write> up_mask: array<f32>;

@compute @workgroup_size(8, 8, 1)
fn upsample_accum(@builtin(global_invocation_id) gid: vec3<u32>) {
    let x = gid.x;
    let y = gid.y;
    if (x >= up_p.out_w || y >= up_p.out_h) {
        return;
    }
    let sx = clamp((f32(x) + 0.5) / up_p.scale - 0.5, 0.0, f32(up_p.in_w - 1u));
    let sy = clamp((f32(y) + 0.5) / up_p.scale - 0.5, 0.0, f32(up_p.in_h - 1u));
    let x0 = u32(floor(sx));
    let y0 = u32(floor(sy));
    let x1 = min(x0 + 1u, up_p.in_w - 1u);
    let y1 = min(y0 + 1u, up_p.in_h - 1u);
    let fx = sx - f32(x0);
    let fy = sy - f32(y0);
    let in_hw = up_p.in_w * up_p.in_h;
    let i00 = y0 * up_p.in_w + x0;
    let i01 = y0 * up_p.in_w + x1;
    let i10 = y1 * up_p.in_w + x0;
    let i11 = y1 * up_p.in_w + x1;

    let out_hw = up_p.out_w * up_p.out_h;
    let pix = y * up_p.out_w + x;

    for (var c = 0u; c < 4u; c++) {
        let base = c * in_hw;
        let v = mix(
            mix(up_in[base + i00], up_in[base + i01], fx),
            mix(up_in[base + i10], up_in[base + i11], fx),
            fy,
        ) * up_p.scale;
        if (up_p.accum != 0u) {
            up_flow[c * out_hw + pix] += v;
        } else {
            up_flow[c * out_hw + pix] = v;
        }
    }
    let base_m = 4u * in_hw;
    let m = mix(
        mix(up_in[base_m + i00], up_in[base_m + i01], fx),
        mix(up_in[base_m + i10], up_in[base_m + i11], fx),
        fy,
    );
    if (up_p.accum != 0u) {
        up_mask[pix] += m;
    } else {
        up_mask[pix] = m;
    }
}

struct WarpParams {
    w: u32,
    h: u32,
    _p0: u32,
    _p1: u32,
};

@group(0) @binding(0) var<uniform> wp_p: WarpParams;
@group(0) @binding(1) var<storage, read> wp_img0: array<f32>;
@group(0) @binding(2) var<storage, read> wp_img1: array<f32>;
@group(0) @binding(3) var<storage, read> wp_flow: array<f32>;
@group(0) @binding(4) var<storage, read_write> wp_w0: array<f32>;
@group(0) @binding(5) var<storage, read_write> wp_w1: array<f32>;

@compute @workgroup_size(8, 8, 1)
fn warp_bilinear(@builtin(global_invocation_id) gid: vec3<u32>) {
    let x = gid.x;
    let y = gid.y;
    if (x >= wp_p.w || y >= wp_p.h) {
        return;
    }
    let hw = wp_p.w * wp_p.h;
    let pix = y * wp_p.w + x;

    let sx0 = clamp(f32(x) + wp_flow[pix], 0.0, f32(wp_p.w - 1u));
    let sy0 = clamp(f32(y) + wp_flow[hw + pix], 0.0, f32(wp_p.h - 1u));
    let x0a = u32(floor(sx0));
    let y0a = u32(floor(sy0));
    let x1a = min(x0a + 1u, wp_p.w - 1u);
    let y1a = min(y0a + 1u, wp_p.h - 1u);
    let fx0 = sx0 - f32(x0a);
    let fy0 = sy0 - f32(y0a);
    let a00 = y0a * wp_p.w + x0a;
    let a01 = y0a * wp_p.w + x1a;
    let a10 = y1a * wp_p.w + x0a;
    let a11 = y1a * wp_p.w + x1a;

    let sx1 = clamp(f32(x) + wp_flow[2u * hw + pix], 0.0, f32(wp_p.w - 1u));
    let sy1 = clamp(f32(y) + wp_flow[3u * hw + pix], 0.0, f32(wp_p.h - 1u));
    let x0b = u32(floor(sx1));
    let y0b = u32(floor(sy1));
    let x1b = min(x0b + 1u, wp_p.w - 1u);
    let y1b = min(y0b + 1u, wp_p.h - 1u);
    let fx1 = sx1 - f32(x0b);
    let fy1 = sy1 - f32(y0b);
    let b00 = y0b * wp_p.w + x0b;
    let b01 = y0b * wp_p.w + x1b;
    let b10 = y1b * wp_p.w + x0b;
    let b11 = y1b * wp_p.w + x1b;

    for (var c = 0u; c < 3u; c++) {
        let base = c * hw;
        wp_w0[base + pix] = mix(
            mix(wp_img0[base + a00], wp_img0[base + a01], fx0),
            mix(wp_img0[base + a10], wp_img0[base + a11], fx0),
            fy0,
        );
        wp_w1[base + pix] = mix(
            mix(wp_img1[base + b00], wp_img1[base + b01], fx1),
            mix(wp_img1[base + b10], wp_img1[base + b11], fx1),
            fy1,
        );
    }
}

struct ExtractParams {
    pw: u32,
    ph: u32,
    w: u32,
    h: u32,
};

@group(0) @binding(0) var<uniform> ex_p: ExtractParams;
@group(0) @binding(1) var<storage, read> ex_flow: array<f32>;
@group(0) @binding(2) var<storage, read> ex_mask: array<f32>;
@group(0) @binding(3) var<storage, read_write> ex_out: array<f32>;

@compute @workgroup_size(8, 8, 1)
fn extract_output(@builtin(global_invocation_id) gid: vec3<u32>) {
    let x = gid.x;
    let y = gid.y;
    if (x >= ex_p.w || y >= ex_p.h) {
        return;
    }
    let phw = ex_p.pw * ex_p.ph;
    let spix = y * ex_p.pw + x;
    let dpix = (y * ex_p.w + x) * 5u;

    ex_out[dpix] = ex_flow[spix];
    ex_out[dpix + 1u] = ex_flow[phw + spix];
    ex_out[dpix + 2u] = ex_flow[2u * phw + spix];
    ex_out[dpix + 3u] = ex_flow[3u * phw + spix];
    let m = ex_mask[spix];
    ex_out[dpix + 4u] = 1.0 / (1.0 + exp(-m));
}
