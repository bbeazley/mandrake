/// [`WaterColorMode::Map`]'s colour of each cell (RGBA8, display values, row-major
/// `cell_width x cell_height`): the average of the landscape texels of each water cell (a cell
/// with a media triangle, flags `0x0800`/`0x1000`), then per cell the average of those over the
/// water cells within `radius` cells, each weighted by its depth squared (media height minus
/// physical height of the cell), so the deep water the artist painted decides it and the shore
/// does not. A cell with no water in reach gets the nearest computed colour of its row (or 0).
/// `None` when `rgba` does not cover the map.
pub(super) fn map_water_colors(
    mesh: &mandrake_tag::MeshTag,
    _mesh_flags: u32,
    rgba: &[u8],
    (width, height): (u32, u32),
    radius: f32,
) -> Option<Vec<u8>> {
    let (cw, ch) = (usize::from(mesh.cell_width), usize::from(mesh.cell_height));
    let (w, h) = (width as usize, height as usize);
    if cw == 0 || ch == 0 || rgba.len() < w * h * 4 || w < cw || h < ch {
        return None;
    }
    // Per water cell: its mean texel and its weight.
    let mut cell_rgb = vec![[0.0f32; 3]; cw * ch];
    let mut weight = vec![0.0f32; cw * ch];
    for r in 0..ch {
        for c in 0..cw {
            let Some(cell) = mesh.cell_at(c, r) else {
                continue;
            };
            if cell.flags & 0x1800 == 0 {
                continue;
            }
            let depth = (f32::from(cell.media_height) - f32::from(cell.physical_height)) / 512.0;
            let (x0, x1) = (c * w / cw, ((c + 1) * w / cw).max(c * w / cw + 1));
            let (y0, y1) = (r * h / ch, ((r + 1) * h / ch).max(r * h / ch + 1));
            let step = ((x1 - x0) / 4).max(1);
            let mut sum = [0.0f32; 3];
            let mut n = 0.0f32;
            for y in (y0..y1).step_by(step) {
                for x in (x0..x1).step_by(step) {
                    let o = (y * w + x) * 4;
                    for k in 0..3 {
                        sum[k] += f32::from(rgba[o + k]);
                    }
                    n += 1.0;
                }
            }
            if n > 0.0 {
                cell_rgb[r * cw + c] = sum.map(|v| v / n);
                weight[r * cw + c] = depth.max(0.02).powi(2);
            }
        }
    }
    let rad = radius.max(1.0).round() as i64;
    let mut out = vec![0u8; cw * ch * 4];
    for r in 0..ch as i64 {
        for c in 0..cw as i64 {
            let mut sum = [0.0f32; 3];
            let mut total = 0.0f32;
            for y in (r - rad).max(0)..=(r + rad).min(ch as i64 - 1) {
                for x in (c - rad).max(0)..=(c + rad).min(cw as i64 - 1) {
                    let i = y as usize * cw + x as usize;
                    let wgt = weight[i];
                    if wgt > 0.0 {
                        for k in 0..3 {
                            sum[k] += cell_rgb[i][k] * wgt;
                        }
                        total += wgt;
                    }
                }
            }
            if total > 0.0 {
                let o = (r as usize * cw + c as usize) * 4;
                for k in 0..3 {
                    out[o + k] = (sum[k] / total).round().clamp(0.0, 255.0) as u8;
                }
                out[o + 3] = 255;
            }
        }
    }
    fill_uncomputed_water_colors(&mut out, cw, ch);
    Some(out)
}

fn fill_uncomputed_water_colors(out: &mut [u8], cw: usize, ch: usize) {
    let mut row_populated = vec![false; ch];
    let mut left = vec![usize::MAX; cw];
    for (r, populated) in row_populated.iter_mut().enumerate() {
        let row_base = r * cw;
        let mut last = usize::MAX;
        for (c, slot) in left.iter_mut().enumerate() {
            if out[(row_base + c) * 4 + 3] != 0 {
                last = c;
                *populated = true;
            }
            *slot = last;
        }
        if !*populated {
            continue;
        }
        let mut right = usize::MAX;
        for c in (0..cw).rev() {
            let o = (row_base + c) * 4;
            if out[o + 3] != 0 {
                right = c;
                continue;
            }
            let src_c = match (left[c], right) {
                (usize::MAX, r_c) => r_c,
                (l_c, usize::MAX) => l_c,
                (l_c, r_c) if c - l_c <= r_c - c => l_c,
                (_, r_c) => r_c,
            };
            let src = (row_base + src_c) * 4;
            out.copy_within(src..src + 4, o);
        }
    }
    if row_populated.iter().all(|&p| p) || !row_populated.iter().any(|&p| p) {
        return;
    }
    let mut above = vec![usize::MAX; ch];
    let mut last = usize::MAX;
    for r in 0..ch {
        if row_populated[r] {
            last = r;
        }
        above[r] = last;
    }
    let mut below = usize::MAX;
    for r in (0..ch).rev() {
        if row_populated[r] {
            below = r;
            continue;
        }
        let src_r = match (above[r], below) {
            (usize::MAX, b) => b,
            (a, usize::MAX) => a,
            (a, b) if r - a <= b - r => a,
            (_, b) => b,
        };
        let src_start = src_r * cw * 4;
        let dst_start = r * cw * 4;
        out.copy_within(src_start..src_start + cw * 4, dst_start);
    }
}

/// A `cell_width x cell_height` RGBA8 texture, one texel per cell.
pub(super) fn cell_texture(
    device: &wgpu::Device,
    mesh: &mandrake_tag::MeshTag,
    label: &str,
) -> wgpu::Texture {
    device.create_texture(&wgpu::TextureDescriptor {
        label: Some(label),
        size: wgpu::Extent3d {
            width: u32::from(mesh.cell_width).max(1),
            height: u32::from(mesh.cell_height).max(1),
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8Unorm,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    })
}

/// Upload a [`cell_texture`]'s texels.
pub(super) fn write_cell_texture(
    queue: &wgpu::Queue,
    texture: &wgpu::Texture,
    mesh: &mandrake_tag::MeshTag,
    rgba: &[u8],
) {
    let (cw, ch) = (
        u32::from(mesh.cell_width).max(1),
        u32::from(mesh.cell_height).max(1),
    );
    queue.write_texture(
        wgpu::TexelCopyTextureInfo {
            texture,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        rgba,
        wgpu::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(cw * 4),
            rows_per_image: Some(ch),
        },
        wgpu::Extent3d {
            width: cw,
            height: ch,
            depth_or_array_layers: 1,
        },
    );
}

pub(super) fn srgb_to_linear(c: f32) -> f32 {
    if c <= 0.04045 {
        c / 12.92
    } else {
        ((c + 0.055) / 1.055).powf(2.4)
    }
}

/// The light on each cell (RGBA8, row-major `cell_width x cell_height`, half the value so up to
/// 2 fits): per channel, the cell's landscape texels as shaded over the same texels at the
/// shading table's plain level (`m = (k * 255) >> 16`, the palette colour itself), in linear
/// values. The baked shading is the only light Myth II has; this is how much of it falls on
/// the cell, with the mesh header's light and dark colours' tint. `None` when the texels do
/// not cover the map.
pub(super) fn cell_light(
    mesh: &mandrake_tag::MeshTag,
    texels: &crate::landscape::LandscapeTexels,
) -> Option<Vec<u8>> {
    let (cw, ch) = (usize::from(mesh.cell_width), usize::from(mesh.cell_height));
    let (w, h) = (texels.width as usize, texels.height as usize);
    let (shade, index, table) = (texels.shade(), texels.color_index(), texels.table());
    if cw == 0
        || ch == 0
        || shade.len() < w * h
        || index.len() < w * h
        || w < cw
        || h < ch
        || table.len() < 256 * 256
    {
        return None;
    }
    let plain = ((u64::from(mesh.landscape_shading.transition) * 255) >> 16).min(255) as usize;
    let lin: Vec<f32> = (0..=255)
        .map(|v| srgb_to_linear(v as f32 / 255.0))
        .collect();
    let mut out = vec![0u8; cw * ch * 4];
    for r in 0..ch {
        for c in 0..cw {
            let (x0, x1) = (c * w / cw, ((c + 1) * w / cw).max(c * w / cw + 1));
            let (y0, y1) = (r * h / ch, ((r + 1) * h / ch).max(r * h / ch + 1));
            let step = ((x1 - x0) / 4).max(1);
            let mut lit = [0.0f32; 3];
            let mut base = [0.0f32; 3];
            for y in (y0..y1).step_by(step) {
                for x in (x0..x1).step_by(step) {
                    let i = usize::from(index[y * w + x]);
                    let shaded = table[usize::from(shade[y * w + x]) << 8 | i];
                    let unshaded = table[plain << 8 | i];
                    for k in 0..3 {
                        lit[k] += lin[usize::from(shaded[k])];
                        base[k] += lin[usize::from(unshaded[k])];
                    }
                }
            }
            let o = (r * cw + c) * 4;
            for k in 0..3 {
                let e = if base[k] > 1e-4 {
                    lit[k] / base[k]
                } else {
                    1.0
                };
                out[o + k] = (e * 0.5 * 255.0).round().clamp(0.0, 255.0) as u8;
            }
            out[o + 3] = 255;
        }
    }
    Some(out)
}
