// Original code: Copyright (c) 2021 Advanced Micro Devices, Inc. All rights reserved.
//
// Permission is hereby granted, free of charge, to any person obtaining a copy
// of this software and associated documentation files (the "Software"), to deal
// in the Software without restriction, including without limitation the rights
// to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
// copies of the Software, and to permit persons to whom the Software is
// furnished to do so, subject to the following conditions:
// The above copyright notice and this permission notice shall be included in
// all copies or substantial portions of the Software.
//
// THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
// IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
// FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT.  IN NO EVENT SHALL THE
// AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
// LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
// OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN
// THE SOFTWARE.

struct FsrParams {
    in_size: vec2<f32>,
    out_size: vec2<f32>,
    out_origin: vec2<f32>,
    sharpness: f32,
    flags: u32,
};

@group(0) @binding(0) var<uniform> params: FsrParams;
@group(0) @binding(1) var src: texture_2d<f32>;

@vertex
fn vs_main(@builtin(vertex_index) vi: u32) -> @builtin(position) vec4<f32> {
    let corner = vec2<f32>(f32(vi & 1u), f32((vi >> 1u) & 1u));
    return vec4<f32>(corner.x * 2.0 - 1.0, 1.0 - corner.y * 2.0, 0.0, 1.0);
}

fn srgb_encode(c: vec3<f32>) -> vec3<f32> {
    let lo = c * 12.92;
    let hi = 1.055 * pow(max(c, vec3<f32>(0.0031308)), vec3<f32>(1.0 / 2.4)) - 0.055;
    return select(hi, lo, c <= vec3<f32>(0.0031308));
}

fn srgb_decode(c: vec3<f32>) -> vec3<f32> {
    let lo = c / 12.92;
    let hi = pow((max(c, vec3<f32>(0.04045)) + 0.055) / 1.055, vec3<f32>(2.4));
    return select(hi, lo, c <= vec3<f32>(0.04045));
}

fn encoded() -> bool {
    return (params.flags & 1u) != 0u;
}

fn rcp_lo(a: f32) -> f32 {
    return bitcast<f32>(0x7ef07ebbu - bitcast<u32>(a));
}

fn rsq_lo(a: f32) -> f32 {
    return bitcast<f32>(0x5f347d74u - (bitcast<u32>(a) >> 1u));
}

fn rcp_med(a: f32) -> f32 {
    let b = bitcast<f32>(0x7ef19fffu - bitcast<u32>(a));
    return b * (-b * a + 2.0);
}

fn luma2(c: vec4<f32>) -> f32 {
    return c.b * 0.5 + (c.r * 0.5 + c.g);
}

fn load_src(p: vec2<i32>) -> vec4<f32> {
    let hi = vec2<i32>(params.in_size) - vec2<i32>(1);
    let c = textureLoad(src, clamp(p, vec2<i32>(0), hi), 0);
    if encoded() {
        return vec4<f32>(srgb_encode(c.rgb), c.a);
    }
    return c;
}

fn easu_set(
    dir: ptr<function, vec2<f32>>,
    len: ptr<function, f32>,
    w: f32,
    la: f32,
    lb: f32,
    lc: f32,
    ld: f32,
    le: f32,
) {
    let dc = ld - lc;
    let cb = lc - lb;
    var len_x = max(abs(dc), abs(cb));
    len_x = rcp_lo(len_x);
    let dir_x = ld - lb;
    (*dir).x += dir_x * w;
    len_x = clamp(abs(dir_x) * len_x, 0.0, 1.0);
    len_x *= len_x;
    *len += len_x * w;
    let ec = le - lc;
    let ca = lc - la;
    var len_y = max(abs(ec), abs(ca));
    len_y = rcp_lo(len_y);
    let dir_y = le - la;
    (*dir).y += dir_y * w;
    len_y = clamp(abs(dir_y) * len_y, 0.0, 1.0);
    len_y *= len_y;
    *len += len_y * w;
}

fn easu_tap(
    acc_c: ptr<function, vec4<f32>>,
    acc_w: ptr<function, f32>,
    off: vec2<f32>,
    dir: vec2<f32>,
    len: vec2<f32>,
    lob: f32,
    clp: f32,
    c: vec4<f32>,
) {
    var v: vec2<f32>;
    v.x = off.x * dir.x + off.y * dir.y;
    v.y = off.x * (-dir.y) + off.y * dir.x;
    v *= len;
    var d2 = v.x * v.x + v.y * v.y;
    d2 = min(d2, clp);
    var wb = (2.0 / 5.0) * d2 - 1.0;
    var wa = lob * d2 - 1.0;
    wb *= wb;
    wa *= wa;
    wb = (25.0 / 16.0) * wb - (25.0 / 16.0 - 1.0);
    let w = wb * wa;
    *acc_c += c * w;
    *acc_w += w;
}

fn easu(ip: vec2<f32>) -> vec4<f32> {
    let scale = params.in_size / params.out_size;
    var pp = ip * scale + (0.5 * scale - vec2<f32>(0.5));
    let fp = floor(pp);
    pp -= fp;
    let o = vec2<i32>(fp);
    let b = load_src(o + vec2<i32>(0, -1));
    let c = load_src(o + vec2<i32>(1, -1));
    let e = load_src(o + vec2<i32>(-1, 0));
    let f = load_src(o + vec2<i32>(0, 0));
    let g = load_src(o + vec2<i32>(1, 0));
    let h = load_src(o + vec2<i32>(2, 0));
    let i = load_src(o + vec2<i32>(-1, 1));
    let j = load_src(o + vec2<i32>(0, 1));
    let k = load_src(o + vec2<i32>(1, 1));
    let l = load_src(o + vec2<i32>(2, 1));
    let n = load_src(o + vec2<i32>(0, 2));
    let oo = load_src(o + vec2<i32>(1, 2));
    let bl = luma2(b);
    let cl = luma2(c);
    let el = luma2(e);
    let fl = luma2(f);
    let gl = luma2(g);
    let hl = luma2(h);
    let il = luma2(i);
    let jl = luma2(j);
    let kl = luma2(k);
    let ll = luma2(l);
    let nl = luma2(n);
    let ol = luma2(oo);
    var dir = vec2<f32>(0.0);
    var len = 0.0;
    easu_set(&dir, &len, (1.0 - pp.x) * (1.0 - pp.y), bl, el, fl, gl, jl);
    easu_set(&dir, &len, pp.x * (1.0 - pp.y), cl, fl, gl, hl, kl);
    easu_set(&dir, &len, (1.0 - pp.x) * pp.y, fl, il, jl, kl, nl);
    easu_set(&dir, &len, pp.x * pp.y, gl, jl, kl, ll, ol);
    let dir2 = dir * dir;
    var dir_r = dir2.x + dir2.y;
    let zro = dir_r < 1.0 / 32768.0;
    dir_r = rsq_lo(dir_r);
    dir_r = select(dir_r, 1.0, zro);
    dir.x = select(dir.x, 1.0, zro);
    dir *= dir_r;
    len = len * 0.5;
    len *= len;
    let stretch = (dir.x * dir.x + dir.y * dir.y) * rcp_lo(max(abs(dir.x), abs(dir.y)));
    let len2 = vec2<f32>(1.0 + (stretch - 1.0) * len, 1.0 - 0.5 * len);
    let lob = 0.5 + ((1.0 / 4.0 - 0.04) - 0.5) * len;
    let clp = rcp_lo(lob);
    let min4 = min(min(f, g), min(j, k));
    let max4 = max(max(f, g), max(j, k));
    var acc_c = vec4<f32>(0.0);
    var acc_w = 0.0;
    easu_tap(&acc_c, &acc_w, vec2<f32>(0.0, -1.0) - pp, dir, len2, lob, clp, b);
    easu_tap(&acc_c, &acc_w, vec2<f32>(1.0, -1.0) - pp, dir, len2, lob, clp, c);
    easu_tap(&acc_c, &acc_w, vec2<f32>(-1.0, 1.0) - pp, dir, len2, lob, clp, i);
    easu_tap(&acc_c, &acc_w, vec2<f32>(0.0, 1.0) - pp, dir, len2, lob, clp, j);
    easu_tap(&acc_c, &acc_w, vec2<f32>(0.0, 0.0) - pp, dir, len2, lob, clp, f);
    easu_tap(&acc_c, &acc_w, vec2<f32>(-1.0, 0.0) - pp, dir, len2, lob, clp, e);
    easu_tap(&acc_c, &acc_w, vec2<f32>(1.0, 1.0) - pp, dir, len2, lob, clp, k);
    easu_tap(&acc_c, &acc_w, vec2<f32>(2.0, 1.0) - pp, dir, len2, lob, clp, l);
    easu_tap(&acc_c, &acc_w, vec2<f32>(2.0, 0.0) - pp, dir, len2, lob, clp, h);
    easu_tap(&acc_c, &acc_w, vec2<f32>(1.0, 0.0) - pp, dir, len2, lob, clp, g);
    easu_tap(&acc_c, &acc_w, vec2<f32>(1.0, 2.0) - pp, dir, len2, lob, clp, oo);
    easu_tap(&acc_c, &acc_w, vec2<f32>(0.0, 2.0) - pp, dir, len2, lob, clp, n);
    return min(max4, max(min4, acc_c * (1.0 / acc_w)));
}

@fragment
fn fs_easu(@builtin(position) pos: vec4<f32>) -> @location(0) vec4<f32> {
    return easu(floor(pos.xy));
}

const RCAS_LIMIT: f32 = 0.25 - 1.0 / 16.0;

fn rcas(ip: vec2<i32>) -> vec4<f32> {
    let hi = vec2<i32>(params.in_size) - vec2<i32>(1);
    let b = textureLoad(src, clamp(ip + vec2<i32>(0, -1), vec2<i32>(0), hi), 0);
    let d = textureLoad(src, clamp(ip + vec2<i32>(-1, 0), vec2<i32>(0), hi), 0);
    let e = textureLoad(src, clamp(ip, vec2<i32>(0), hi), 0);
    let f = textureLoad(src, clamp(ip + vec2<i32>(1, 0), vec2<i32>(0), hi), 0);
    let h = textureLoad(src, clamp(ip + vec2<i32>(0, 1), vec2<i32>(0), hi), 0);
    let mn4 = min(min(b.rgb, d.rgb), min(f.rgb, h.rgb));
    let mx4 = max(max(b.rgb, d.rgb), max(f.rgb, h.rgb));
    let peak_c = vec2<f32>(1.0, -1.0 * 4.0);
    let hit_min = min(mn4, e.rgb) * (vec3<f32>(1.0) / max(4.0 * mx4, vec3<f32>(1e-20)));
    let hit_max = (peak_c.x - max(mx4, e.rgb))
        * (vec3<f32>(1.0) / min(4.0 * mn4 + peak_c.y, vec3<f32>(-1e-20)));
    let lobe_rgb = max(-hit_min, hit_max);
    let lobe = max(-RCAS_LIMIT, min(max(max(lobe_rgb.r, lobe_rgb.g), lobe_rgb.b), 0.0))
        * params.sharpness;
    let rcp_l = rcp_med(4.0 * lobe + 1.0);
    let rgb = (lobe * b.rgb + lobe * d.rgb + lobe * h.rgb + lobe * f.rgb + e.rgb) * rcp_l;
    return vec4<f32>(rgb, e.a);
}

@fragment
fn fs_rcas(@builtin(position) pos: vec4<f32>) -> @location(0) vec4<f32> {
    let p = rcas(vec2<i32>(floor(pos.xy - params.out_origin)));
    var rgb = clamp(p.rgb, vec3<f32>(0.0), vec3<f32>(1.0));
    if encoded() {
        rgb = srgb_decode(rgb);
    }
    let a = clamp(p.a, 0.0, 1.0);
    return vec4<f32>(min(rgb, vec3<f32>(a)), a);
}
