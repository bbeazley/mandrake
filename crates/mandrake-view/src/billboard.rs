use crate::selection::PixelRect;
use bytemuck::{Pod, Zeroable};

pub const BILLBOARD_FLAG_FLIP_X: u32 = 1 << 0;

pub const BILLBOARD_FLAG_INTERFACE_RECT: u32 = 1 << 2;

pub const BILLBOARD_FLAG_CYLINDRICAL: u32 = 1 << 3;

pub const BILLBOARD_FLAG_CENTERED: u32 = 1 << 4;

pub const BILLBOARD_FLAG_FACING_ARROW: u32 = 1 << 5;

pub const BILLBOARD_FLAG_CONNECTOR: u32 = 256;

pub const BILLBOARD_FLAG_SCREEN_ALIGNED: u32 = 1 << 9;

pub const BILLBOARD_FLAG_SCREEN_OVERLAY: u32 = 1 << 10;

pub const BILLBOARD_FLAG_MEDIA_CLIP: u32 = 1 << 11;

pub const BILLBOARD_FLAG_REFLECTION: u32 = 1 << 12;

pub const BILLBOARD_FLAG_SHADE: u32 = 1 << 13;

pub const BILLBOARD_FLAG_SUBMERGED: u32 = 1 << 14;

const WORLD_UNITS: f64 = crate::WORLD_UNITS_PER_CELL as f64;

const SCALED_WORD_TOLERANCE: f64 = 1.0 / 512.0;

#[derive(Clone, Copy, Debug)]
pub struct EngineSpriteView {
    engine: crate::camera::EngineView,
    eye: [f32; 3],
    right: [f32; 3],
    up: [f32; 3],
    forward: [f32; 3],
}

impl EngineSpriteView {
    pub fn of(camera: &crate::camera::Camera) -> Self {
        let (right, up, forward) = camera.view_axes();
        Self {
            engine: camera.engine_view(),
            eye: camera.eye_position(),
            right,
            up,
            forward,
        }
    }
}

pub fn place_sprites_in_engine_view(
    instances: &mut [BillboardInstance],
    camera: &crate::camera::Camera,
) {
    let view = EngineSpriteView::of(camera);
    for instance in instances {
        *instance = instance.in_engine_view(&view);
    }
}

pub fn sprite_plane_depth_units(num: f32, den: f32, depth: f32) -> f32 {
    if den == 0.0 {
        return if num > 0.0 { 1.0e9 } else { 0.0 };
    }
    let ratio = num / den;
    if ratio < 0.0 {
        return 0.0;
    }
    (ratio * depth).trunc()
}

pub const OBJECT_MIN_VIEW_DEPTH: f32 = 34.0 / 512.0;

pub const FACING_ARROW_SHAFT_HALF_WIDTH: f32 = 32.0 / 512.0;

pub const FACING_ARROW_SHAFT_LENGTH: f32 = 768.0 / 512.0;

pub const FACING_ARROW_TIP_DISTANCE: f32 = 896.0 / 512.0;

pub const FACING_ARROW_BARB_LENGTH: f32 = 256.0 / 512.0;

pub const FACING_ARROW_BARB_ANGLE: f32 = std::f32::consts::PI / 8.0;

pub const BILLBOARD_FLAG_VELOCITY_QUAD: u32 = 1 << 15;

pub const BILLBOARD_FLAG_MIRROR_Y: u32 = 1 << 16;

pub const BILLBOARD_FLAG_PRERENDER_LAYER: u32 = 1 << 17;

pub const BILLBOARD_FLAG_MEDIA_EFFECT: u32 = 1 << 18;

pub const BILLBOARD_FLAG_WORLD_QUAD: u32 = 1 << 19;

pub const BILLBOARD_FLAG_BUCKET_DEPTH: u32 = 1 << 20;

#[repr(C)]
#[derive(Copy, Clone, Debug, PartialEq, Pod, Zeroable)]
pub struct BillboardInstance {
    pub position: [f32; 3],
    pub flags: u32,
    pub size: [f32; 2],
    pub uv_min: [f32; 2],
    pub uv_max: [f32; 2],
    pub tint: [f32; 4],
    pub anchor: [f32; 2],
    pub media: [f32; 4],
    pub shade_kpqr: [f32; 4],
    pub shade_xyz: [f32; 4],
}

impl Default for BillboardInstance {
    fn default() -> Self {
        Self::new([0.0; 3], [0.0; 2], [0.0; 2], [0.0; 2])
    }
}

const NO_MEDIA: [f32; 4] = [0.0; 4];
const IDENTITY_KPQR: [f32; 4] = [1.0, 0.0, 0.0, 0.0];

impl BillboardInstance {
    pub fn new(position: [f32; 3], size: [f32; 2], uv_min: [f32; 2], uv_max: [f32; 2]) -> Self {
        Self {
            position,
            flags: 0,
            size,
            uv_min,
            uv_max,
            tint: [1.0, 1.0, 1.0, 1.0],
            anchor: [0.5, 1.0],
            media: NO_MEDIA,
            shade_kpqr: IDENTITY_KPQR,
            shade_xyz: NO_MEDIA,
        }
    }

    pub fn world_quad(corners: [[f32; 3]; 4], uv_min: [f32; 2], uv_max: [f32; 2]) -> Self {
        let [c0, c1, c2, c3] = corners;
        Self {
            flags: BILLBOARD_FLAG_WORLD_QUAD,
            size: [c1[0], c1[2]],
            anchor: [c2[0], c2[2]],
            media: [c3[0], c3[2], 0.0, 0.0],
            ..Self::new(c0, [0.0; 2], uv_min, uv_max)
        }
    }

    pub fn world_quad_corners(&self) -> [[f32; 3]; 4] {
        let y = self.position[1];
        [
            self.position,
            [self.size[0], y, self.size[1]],
            [self.anchor[0], y, self.anchor[1]],
            [self.media[0], y, self.media[1]],
        ]
    }

    pub fn from_atlas_frame(
        frame: &AtlasFrame,
        position: [f32; 3],
        size: [f32; 2],
        flip_x: bool,
    ) -> Self {
        Self::new(position, size, frame.uv_min, frame.uv_max)
            .with_screen_aligned(true)
            .with_anchor_and_flip(
                frame.offset_x,
                frame.offset_y,
                frame.width,
                frame.height,
                flip_x,
            )
    }

    pub fn interface_rect(depth_anchor: [f32; 3], rect: PixelRect, color: [f32; 4]) -> Self {
        Self {
            position: depth_anchor,
            flags: BILLBOARD_FLAG_INTERFACE_RECT,
            size: [0.0, 0.0],
            uv_min: [rect.left as f32, rect.top as f32],
            uv_max: [rect.right as f32, rect.bottom as f32],
            tint: color,
            anchor: [0.0, 0.0],
            media: NO_MEDIA,
            shade_kpqr: IDENTITY_KPQR,
            shade_xyz: NO_MEDIA,
        }
    }

    pub fn screen_overlay_rect(rect: PixelRect, color: [f32; 4]) -> Self {
        let mut r = Self::interface_rect([0.0; 3], rect, color);
        r.flags |= BILLBOARD_FLAG_SCREEN_OVERLAY;
        r
    }

    pub fn is_interface_rect(&self) -> bool {
        self.flags & BILLBOARD_FLAG_INTERFACE_RECT != 0
    }

    pub fn sprite_basis(&self, camera_pos: [f32; 3]) -> ([f32; 3], [f32; 3]) {
        let to_cam = [
            camera_pos[0] - self.position[0],
            camera_pos[1] - self.position[1],
            camera_pos[2] - self.position[2],
        ];
        if (self.flags & BILLBOARD_FLAG_CYLINDRICAL) != 0 {
            let len = (to_cam[0] * to_cam[0] + to_cam[2] * to_cam[2]).sqrt();
            if len < 1e-6 {
                return ([1.0, 0.0, 0.0], [0.0, 1.0, 0.0]);
            }
            let (dx, dz) = (to_cam[0] / len, to_cam[2] / len);
            ([-dz, 0.0, dx], [0.0, 1.0, 0.0])
        } else {
            let len =
                (to_cam[0] * to_cam[0] + to_cam[1] * to_cam[1] + to_cam[2] * to_cam[2]).sqrt();
            let dir = if len > 1e-4 {
                [to_cam[0] / len, to_cam[1] / len, to_cam[2] / len]
            } else {
                [0.0, 0.0, 1.0]
            };
            let r = [dir[2], 0.0, -dir[0]];
            let lr = (r[0] * r[0] + r[2] * r[2]).sqrt();
            let right = if lr > 1e-4 {
                [r[0] / lr, 0.0, r[2] / lr]
            } else {
                [1.0, 0.0, 0.0]
            };
            let up = [
                dir[1] * right[2] - dir[2] * right[1],
                dir[2] * right[0] - dir[0] * right[2],
                dir[0] * right[1] - dir[1] * right[0],
            ];
            (right, up)
        }
    }

    pub fn facing_arrow(position: [f32; 3], facing_yaw: f32, color: [f32; 4]) -> Self {
        let (barb_sin, barb_cos) = FACING_ARROW_BARB_ANGLE.sin_cos();
        let head_half_width = FACING_ARROW_BARB_LENGTH * barb_sin;
        Self {
            position,
            flags: BILLBOARD_FLAG_FACING_ARROW,
            size: [
                2.0 * head_half_width.max(FACING_ARROW_SHAFT_HALF_WIDTH),
                FACING_ARROW_TIP_DISTANCE,
            ],
            uv_min: [FACING_ARROW_SHAFT_HALF_WIDTH, FACING_ARROW_SHAFT_LENGTH],
            uv_max: [
                FACING_ARROW_TIP_DISTANCE - FACING_ARROW_BARB_LENGTH * barb_cos,
                barb_sin / barb_cos,
            ],
            tint: color,
            anchor: [facing_yaw, 0.0],
            media: NO_MEDIA,
            shade_kpqr: IDENTITY_KPQR,
            shade_xyz: NO_MEDIA,
        }
    }

    pub fn facing_arrow_contains(&self, lateral: f32, forward: f32) -> bool {
        let lateral = lateral.abs();
        let [shaft_half_width, shaft_length] = self.uv_min;
        let [head_base, head_slope] = self.uv_max;
        let tip = self.size[1];
        let in_shaft = lateral <= shaft_half_width && (0.0..=shaft_length).contains(&forward);
        let in_head =
            (head_base..=tip).contains(&forward) && lateral <= (tip - forward) * head_slope;
        in_shaft || in_head
    }

    pub fn with_anchor_and_flip(
        mut self,
        offset_x: i16,
        offset_y: i16,
        frame_width: u32,
        frame_height: u32,
        flip_x: bool,
    ) -> Self {
        self.anchor = [
            offset_x as f32 / frame_width.max(1) as f32,
            offset_y as f32 / frame_height.max(1) as f32,
        ];
        self.with_flip_x(flip_x)
    }

    pub fn with_anchor(mut self, anchor: [f32; 2]) -> Self {
        self.anchor = anchor;
        self
    }

    pub fn effective_quad_bounds(&self) -> ([f32; 2], [f32; 2]) {
        let mut ax = self.anchor[0];
        let ay = self.anchor[1];
        if (self.flags & BILLBOARD_FLAG_FLIP_X) != 0 {
            ax = 1.0 - ax;
        }
        let x_min = -ax * self.size[0];
        let x_max = (1.0 - ax) * self.size[0];

        let (y_min, y_max) = if (self.flags & BILLBOARD_FLAG_CENTERED) != 0 {
            (-0.5 * self.size[1], 0.5 * self.size[1])
        } else {
            (-(1.0 - ay) * self.size[1], ay * self.size[1])
        };

        ([x_min, x_max], [y_min, y_max])
    }

    pub fn in_engine_view(mut self, view: &EngineSpriteView) -> Self {
        const NOT_OBJECT: u32 = BILLBOARD_FLAG_INTERFACE_RECT
            | BILLBOARD_FLAG_CENTERED
            | BILLBOARD_FLAG_FACING_ARROW
            | BILLBOARD_FLAG_CONNECTOR
            | BILLBOARD_FLAG_SCREEN_OVERLAY;
        if self.flags & BILLBOARD_FLAG_SCREEN_ALIGNED == 0 || self.flags & NOT_OBJECT != 0 {
            return self;
        }
        let [x, _, z] = self.position;
        let [vx, vy, vz] = view.engine.view_coords(self.position);
        let up_y = view.up[1];

        let units = |c: f32| (f64::from(c) * WORLD_UNITS + SCALED_WORD_TOLERANCE).floor();
        let ([x0, x1], [y0, y1]) = self.effective_quad_bounds();
        let [x0, x1] = [units(x0), units(x1)];
        let [y0, y1] = if self.flags & BILLBOARD_FLAG_MIRROR_Y != 0 {
            [-units(-y0), -units(-y1)]
        } else {
            [units(y0), units(y1)]
        };
        if x1 > x0 && y1 > y0 {
            self.size = [
                ((x1 - x0) / WORLD_UNITS) as f32,
                ((y1 - y0) / WORLD_UNITS) as f32,
            ];
            let ax = (-x0 / (x1 - x0)) as f32;
            self.anchor = [
                if self.flags & BILLBOARD_FLAG_FLIP_X != 0 {
                    1.0 - ax
                } else {
                    ax
                },
                (y1 / (y1 - y0)) as f32,
            ];
        }

        let at = |i: usize| {
            let along = |axis: [f32; 3], c: i32| f64::from(axis[i]) * f64::from(c) / WORLD_UNITS;
            (f64::from(view.eye[i]) + along(view.right, vx) + along(view.up, vy)
                - along(view.forward, vz)) as f32
        };
        self.position = [at(0), at(1), at(2)];

        if up_y.abs() > 1e-6 {
            let height = |h: f32| {
                let [_, v, _] = view.engine.view_coords([x, h, z]);
                self.position[1] + (f64::from(v - vy) / WORLD_UNITS) as f32 / up_y
            };
            if self.flags & BILLBOARD_FLAG_MEDIA_CLIP != 0 {
                self.media[0] = height(self.media[0]);
            }
            if self.flags & BILLBOARD_FLAG_REFLECTION != 0 {
                self.media[1] = height(self.media[1]);
            }
        }
        self
    }

    pub fn with_tint(mut self, tint: [f32; 4]) -> Self {
        self.tint = tint;
        self
    }

    pub fn with_flags(mut self, flags: u32) -> Self {
        self.flags = flags;
        self
    }

    pub fn with_flip_x(mut self, flip: bool) -> Self {
        if flip {
            self.flags |= BILLBOARD_FLAG_FLIP_X;
        } else {
            self.flags &= !BILLBOARD_FLAG_FLIP_X;
        }
        self
    }

    pub fn with_cylindrical(mut self, cylindrical: bool) -> Self {
        if cylindrical {
            self.flags |= BILLBOARD_FLAG_CYLINDRICAL;
        } else {
            self.flags &= !BILLBOARD_FLAG_CYLINDRICAL;
        }
        self
    }

    pub fn with_screen_aligned(mut self, screen_aligned: bool) -> Self {
        if screen_aligned {
            self.flags |= BILLBOARD_FLAG_SCREEN_ALIGNED;
        } else {
            self.flags &= !BILLBOARD_FLAG_SCREEN_ALIGNED;
        }
        self
    }

    pub fn screen_aligned_corner(
        &self,
        eye: [f32; 3],
        right: [f32; 3],
        up: [f32; 3],
        forward: [f32; 3],
        corner: [f32; 2],
    ) -> ([f32; 3], Option<[f32; 3]>) {
        let ([x_min, x_max], [y_min, y_max]) = self.effective_quad_bounds();
        let ox = x_min + (x_max - x_min) * corner[0];
        let oy = y_min + (y_max - y_min) * corner[1];
        let p = self.position;
        let q = [
            p[0] + right[0] * ox + up[0] * oy,
            p[1] + right[1] * ox + up[1] * oy,
            p[2] + right[2] * ox + up[2] * oy,
        ];
        let hl = (forward[0] * forward[0] + forward[2] * forward[2]).sqrt();
        if hl < 1e-6 {
            return (q, Some(q));
        }
        let n = [forward[0] / hl, 0.0, forward[2] / hl];
        let ray = [q[0] - eye[0], q[1] - eye[1], q[2] - eye[2]];
        let denom = ray[0] * n[0] + ray[2] * n[2];
        let num = (p[0] - eye[0]) * n[0] + (p[2] - eye[2]) * n[2];
        if denom == 0.0 || num / denom < 0.0 {
            return (q, None);
        }
        let t = num / denom;
        let z = [
            eye[0] + ray[0] * t,
            eye[1] + ray[1] * t,
            eye[2] + ray[2] * t,
        ];
        (q, Some(z))
    }

    pub fn with_media_clip(mut self, surface_y: f32) -> Self {
        self.flags |= BILLBOARD_FLAG_MEDIA_CLIP;
        self.media[0] = surface_y;
        self
    }

    pub fn with_shade(mut self, tint: [u8; 8]) -> Self {
        let (kpqr, xyz) = crate::media::tint_factors(tint);
        self.flags |= BILLBOARD_FLAG_SHADE;
        self.shade_kpqr = kpqr;
        self.shade_xyz = xyz;
        self
    }

    pub fn reflection(&self, surface_y: f32, ripple: f32, clip: bool, tint: [u8; 8]) -> Self {
        let z = self.position[1];
        let mirrored = 2.0 * surface_y - z;
        let mut r = *self;
        r.flags = (self.flags & !BILLBOARD_FLAG_MEDIA_CLIP) | BILLBOARD_FLAG_REFLECTION;
        if clip {
            r.flags |= BILLBOARD_FLAG_MEDIA_CLIP;
        }
        if z > surface_y {
            r.position[1] = mirrored;
        }
        r.media = [surface_y, mirrored, ripple, 0.0];
        r.with_shade(tint)
    }

    pub fn with_top_extended(mut self, by: f32) -> Self {
        let ([_, _], [lo, hi]) = self.effective_quad_bounds();
        let height = hi - lo + by;
        if height > 0.0 {
            self.size[1] = height;
            self.anchor[1] = (hi + by) / height;
        }
        self
    }

    pub fn submerged_copy(&self, transparency: u16) -> Self {
        let mut r = *self;
        r.flags |= BILLBOARD_FLAG_MEDIA_CLIP | BILLBOARD_FLAG_SUBMERGED;
        r.tint[3] *= crate::media::surface_opacity(transparency);
        r
    }

    pub fn media_span(&self, up: [f32; 3]) -> ([f32; 3], f32, f32, Option<[f32; 2]>) {
        let (_, [lo, hi0]) = self.effective_quad_bounds();
        let reflect = self.flags & BILLBOARD_FLAG_REFLECTION != 0;
        let hi = if reflect { hi0 + self.media[2] } else { hi0 };
        let mut origin = self.position;
        if reflect {
            let d = (self.media[1] - self.position[1]) * up[1];
            origin = [
                origin[0] + up[0] * d,
                origin[1] + up[1] * d,
                origin[2] + up[2] * d,
            ];
        }
        let (dlo, dhi) = if reflect { (-lo, -hi) } else { (lo, hi) };
        if self.flags & BILLBOARD_FLAG_MEDIA_CLIP == 0 {
            return (origin, dlo, dhi, Some([0.0, 1.0]));
        }
        let m = [self.position[0], self.media[0], self.position[2]];
        let clip =
            (m[0] - origin[0]) * up[0] + (m[1] - origin[1]) * up[1] + (m[2] - origin[2]) * up[2];
        let c = if reflect { -clip } else { clip };
        let t0 = if hi > lo {
            ((c - lo) / (hi - lo)).clamp(0.0, 1.0)
        } else {
            1.0
        };
        if t0 >= 1.0 {
            (origin, dlo, dhi, None)
        } else if self.flags & BILLBOARD_FLAG_SUBMERGED != 0 {
            if t0 <= 0.0 {
                (origin, dlo, dhi, None)
            } else {
                (origin, dlo, dhi, Some([0.0, t0]))
            }
        } else {
            (origin, dlo, dhi, Some([t0, 1.0]))
        }
    }

    pub fn with_centered(mut self, centered: bool) -> Self {
        if centered {
            self.flags |= BILLBOARD_FLAG_CENTERED;
        } else {
            self.flags &= !BILLBOARD_FLAG_CENTERED;
        }
        self
    }
}

pub fn select_sprite_view(unit_yaw: f32, camera_yaw: f32, num_views: usize) -> (usize, bool) {
    if num_views <= 1 {
        return (0, false);
    }

    let two_pi = core::f32::consts::PI * 2.0;
    let rel = (unit_yaw - camera_yaw).rem_euclid(two_pi);

    let (total_sectors, is_symmetrical) = match num_views {
        3 => (4, true),
        5 => (8, true),
        9 => (16, true),
        17 => (32, true),
        other => (other, false),
    };

    let sector_size = two_pi / total_sectors as f32;
    let sector = ((rel + sector_size * 0.5) / sector_size).floor() as usize % total_sectors;

    if is_symmetrical {
        let half = num_views - 1;
        if sector <= half {
            (sector, false)
        } else {
            let mirrored = total_sectors - sector;
            (mirrored, true)
        }
    } else {
        (sector, false)
    }
}

pub fn select_sprite_view_symmetrical(
    unit_yaw: f32,
    camera_yaw: f32,
    total_views: usize,
) -> (usize, bool) {
    if total_views <= 1 {
        return (0, false);
    }
    let half_stored = (total_views / 2) + 1;
    select_sprite_view(unit_yaw, camera_yaw, half_stored)
}

#[derive(Clone, Debug, PartialEq)]
pub struct AtlasFrame {
    pub index: usize,
    pub uv_min: [f32; 2],
    pub uv_max: [f32; 2],
    pub width: u32,
    pub height: u32,
    pub offset_x: i16,
    pub offset_y: i16,
}

pub fn sprite_mip_sizes(w: u32, h: u32) -> Vec<(u32, u32)> {
    let mut sizes = vec![(w.max(1), h.max(1))];
    while let Some(&(a, b)) = sizes.last() {
        if a == 1 && b == 1 {
            break;
        }
        sizes.push(((a >> 1).max(1), (b >> 1).max(1)));
    }
    sizes
}

pub fn frame_level_offsets(w: u32, h: u32) -> Vec<(u32, u32)> {
    let sizes = sprite_mip_sizes(w, h);
    let mut offsets = Vec::with_capacity(sizes.len());
    let mut y = 0;
    for (k, &(_, lh)) in sizes.iter().enumerate() {
        if k == 0 {
            offsets.push((0, 0));
        } else {
            offsets.push((w.max(1) + 1, y));
            y += lh + 1;
        }
    }
    offsets
}

pub fn frame_cell_size(w: u32, h: u32) -> (u32, u32) {
    let sizes = sprite_mip_sizes(w, h);
    let offsets = frame_level_offsets(w, h);
    let (mut cw, mut ch) = (0, 0);
    for (&(lw, lh), &(ox, oy)) in sizes.iter().zip(offsets.iter()) {
        cw = cw.max(ox + lw + 2);
        ch = ch.max(oy + lh + 2);
    }
    (cw, ch)
}

pub fn sprite_quad_growth(extent: i32, texels: i32) -> i32 {
    (extent / texels.max(1)) / 2
}

pub fn sprite_growth_cells(size: f32, texels: u32) -> f32 {
    let per_texel = (size.abs() * 512.0 / texels.max(1) as f32 + 1.0e-3).trunc();
    (per_texel / 2.0).trunc() / 512.0
}

pub fn sprite_border_uv(texels: u32) -> (f32, f32) {
    let w = texels.max(1) as f32;
    (-0.5 / w, (w + 1.0) / w)
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SpriteQuadVertex {
    pub x: i32,
    pub y: i32,
    pub u: f32,
    pub v: f32,
}

pub fn engine_sprite_quad(
    rect: [i32; 4],
    clip: [i32; 2],
    bitmap: (u32, u32),
    surface_flags: u16,
) -> [SpriteQuadVertex; 4] {
    let [left, b, right, t] = rect;
    let top_kept = clip[0].min(b);
    let bottom_kept = clip[1].max(t);
    let h = t - b;
    let (mut c0, mut c1) = (top_kept - b, bottom_kept - b);
    if surface_flags & 0x40 != 0 {
        (c0, c1) = (h - c0, h - c1);
    }
    let mirror = surface_flags & 0x20 != 0;
    let hy = sprite_quad_growth(h, bitmap.1 as i32);
    let hx = sprite_quad_growth(right - left, bitmap.0 as i32);
    let y0 = (top_kept - hy).min(clip[0]);
    let y1 = (bottom_kept + hy).max(clip[1]);
    let (x0, x1) = (left - hx, right + hx);
    let (u0, du) = sprite_border_uv(bitmap.0);
    let (v0, dv) = sprite_border_uv(bitmap.1);
    let (ul, ur) = if mirror { (u0 + du, u0) } else { (u0, u0 + du) };
    let va = v0 + dv * c0 as f32 / h as f32;
    let vb = v0 + dv * c1 as f32 / h as f32;
    [
        SpriteQuadVertex {
            x: x0,
            y: y0,
            u: ul,
            v: va,
        },
        SpriteQuadVertex {
            x: x0,
            y: y1,
            u: ul,
            v: vb,
        },
        SpriteQuadVertex {
            x: x1,
            y: y1,
            u: ur,
            v: vb,
        },
        SpriteQuadVertex {
            x: x1,
            y: y0,
            u: ur,
            v: va,
        },
    ]
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AtlasLayout {
    pub width: u32,
    pub height: u32,
    pub placements: Vec<(u32, u32, u32, u32)>,
}

pub fn pack_shelves(sizes: &[(u32, u32)], width: u32) -> AtlasLayout {
    let mut placements = Vec::with_capacity(sizes.len());
    let (mut x, mut y, mut row_h) = (0u32, 0u32, 0u32);
    for &(w, h) in sizes {
        if x + w > width {
            x = 0;
            y += row_h;
            row_h = 0;
        }
        placements.push((x, y, w, h));
        x += w;
        row_h = row_h.max(h);
    }
    AtlasLayout {
        width,
        height: (y + row_h).max(1),
        placements,
    }
}

pub fn pack_shelves_ffdh(sizes: &[(u32, u32)], width: u32) -> AtlasLayout {
    let mut order: Vec<usize> = (0..sizes.len()).collect();
    order.sort_by(|&a, &b| {
        let (sa, sb) = (sizes[a], sizes[b]);
        sb.1.cmp(&sa.1).then(sb.0.cmp(&sa.0)).then(a.cmp(&b))
    });
    let mut placements = vec![(0, 0, 0, 0); sizes.len()];
    let mut shelves: Vec<(u32, u32, u32)> = Vec::new();
    let mut bottom = 0u32;
    for i in order {
        let (w, h) = sizes[i];
        let shelf = shelves
            .iter_mut()
            .find(|(_, sh, sx)| h <= *sh && *sx + w <= width);
        let (x, y) = match shelf {
            Some((sy, _, sx)) => {
                let x = *sx;
                *sx += w;
                (x, *sy)
            }
            None => {
                shelves.push((bottom, h, w));
                let y = bottom;
                bottom += h;
                (0, y)
            }
        };
        placements[i] = (x, y, w, h);
    }
    AtlasLayout {
        width,
        height: bottom.max(1),
        placements,
    }
}

pub fn atlas_layout(sizes: &[(u32, u32)], max_dim: u32) -> AtlasLayout {
    let widest = sizes.iter().map(|s| s.0).max().unwrap_or(1);
    let start = 1024u32.max(widest);
    let widths = || {
        let mut width = start;
        std::iter::from_fn(move || {
            let w = width;
            if w == 0 {
                return None;
            }
            width = if w >= max_dim {
                0
            } else {
                (w * 2).min(max_dim.max(widest))
            };
            Some(w)
        })
    };
    let mut in_order = None;
    for width in widths() {
        let layout = pack_shelves(sizes, width);
        if layout.height <= max_dim {
            return layout;
        }
        in_order = Some(layout);
    }
    let mut densest: Option<AtlasLayout> = None;
    for width in widths() {
        let layout = pack_shelves_ffdh(sizes, width);
        if layout.height <= max_dim {
            return layout;
        }
        densest = Some(layout);
    }
    densest
        .or(in_order)
        .unwrap_or_else(|| pack_shelves(sizes, start))
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct SpritePassOrder {
    pub sorted: core::ops::Range<u32>,
    pub keys: Vec<f32>,
    pub interface: Vec<bool>,
    pub overlay: core::ops::Range<u32>,
    pub reflections: core::ops::Range<u32>,
    pub reflection_keys: Vec<f32>,
}

pub fn order_for_sprite_pass(
    instances: &mut Vec<BillboardInstance>,
    eye: [f32; 3],
    forward: [f32; 3],
) -> SpritePassOrder {
    if instances.is_empty() {
        return SpritePassOrder::default();
    }
    let key = |i: &BillboardInstance| {
        if i.flags & BILLBOARD_FLAG_PRERENDER_LAYER != 0 {
            return f32::NEG_INFINITY;
        }
        let p = if i.flags & BILLBOARD_FLAG_CONNECTOR != 0 {
            [
                i.position[0] + 0.5 * i.anchor[0],
                i.position[1] + 0.5 * (i.size[0] + i.size[1]),
                i.position[2] + 0.5 * i.anchor[1],
            ]
        } else {
            i.position
        };
        (p[0] - eye[0]) * forward[0] + (p[1] - eye[1]) * forward[1] + (p[2] - eye[2]) * forward[2]
    };
    let len = instances.len();
    let mut sorted_idx: Vec<(f32, u32)> = Vec::with_capacity(len);
    let mut overlay_idx: Vec<u32> = Vec::new();
    let mut refl_idx: Vec<(f32, u32)> = Vec::new();
    for (idx, inst) in instances.iter().enumerate() {
        let i = idx as u32;
        if inst.flags
            & (BILLBOARD_FLAG_REFLECTION | BILLBOARD_FLAG_SUBMERGED | BILLBOARD_FLAG_MEDIA_EFFECT)
            != 0
        {
            refl_idx.push((key(inst), i));
        } else if inst.flags & (BILLBOARD_FLAG_FACING_ARROW | BILLBOARD_FLAG_SCREEN_OVERLAY) != 0 {
            overlay_idx.push(i);
        } else {
            sorted_idx.push((key(inst), i));
        }
    }
    let cmp = |a: &(f32, u32), b: &(f32, u32)| b.0.total_cmp(&a.0).then_with(|| a.1.cmp(&b.1));
    sorted_idx.sort_unstable_by(cmp);
    refl_idx.sort_unstable_by(cmp);
    let n1 = sorted_idx.len() as u32;
    let n2 = overlay_idx.len() as u32;
    let mut keys = Vec::with_capacity(sorted_idx.len());
    let mut interface = Vec::with_capacity(sorted_idx.len());
    let mut reflection_keys = Vec::with_capacity(refl_idx.len());
    let mut reordered = Vec::with_capacity(len);
    for &(k, idx) in &sorted_idx {
        let inst = instances[idx as usize];
        keys.push(k);
        interface.push(inst.is_interface_rect());
        reordered.push(inst);
    }
    for &idx in &overlay_idx {
        reordered.push(instances[idx as usize]);
    }
    for &(k, idx) in &refl_idx {
        reflection_keys.push(k);
        reordered.push(instances[idx as usize]);
    }
    *instances = reordered;
    SpritePassOrder {
        sorted: 0..n1,
        keys,
        interface,
        overlay: n1..n1 + n2,
        reflections: n1 + n2..len as u32,
        reflection_keys,
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Category4Step {
    Billboards(core::ops::Range<u32>),
    Triangles(core::ops::Range<u32>),
}

pub fn category4_steps(order: &SpritePassOrder, triangle_keys: &[f32]) -> Vec<Category4Step> {
    let base = order.reflections.start;
    let n = order.reflection_keys.len();
    let (mut b, mut t) = (0usize, 0usize);
    let mut steps: Vec<Category4Step> = Vec::new();
    while b < n || t < triangle_keys.len() {
        let take_billboard =
            t >= triangle_keys.len() || (b < n && order.reflection_keys[b] >= triangle_keys[t]);
        if take_billboard {
            let i = base + b as u32;
            match steps.last_mut() {
                Some(Category4Step::Billboards(r)) if r.end == i => r.end = i + 1,
                _ => steps.push(Category4Step::Billboards(i..i + 1)),
            }
            b += 1;
        } else {
            let i = t as u32;
            match steps.last_mut() {
                Some(Category4Step::Triangles(r)) if r.end == i => r.end = i + 1,
                _ => steps.push(Category4Step::Triangles(i..i + 1)),
            }
            t += 1;
        }
    }
    steps
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SpriteStep {
    Billboards(core::ops::Range<u32>),
    Triangles(core::ops::Range<u32>),
    Interface(core::ops::Range<u32>),
}

impl SpriteStep {
    pub fn is_interface(&self) -> bool {
        matches!(self, SpriteStep::Interface(_))
    }
}

pub fn ndc_rect(
    view_proj: &crate::camera::Mat4,
    points: impl IntoIterator<Item = [f32; 3]>,
) -> Option<[f32; 4]> {
    let c = &view_proj.cols;
    let mut out = [
        f32::INFINITY,
        f32::INFINITY,
        f32::NEG_INFINITY,
        f32::NEG_INFINITY,
    ];
    for [x, y, z] in points {
        let row = |r: usize| c[0][r] * x + c[1][r] * y + c[2][r] * z + c[3][r];
        let w = row(3);
        if w.is_nan() || w <= 1.0e-4 {
            return None;
        }
        let (nx, ny) = (row(0) / w, row(1) / w);
        if !(nx.is_finite() && ny.is_finite()) {
            return None;
        }
        out = [
            out[0].min(nx),
            out[1].min(ny),
            out[2].max(nx),
            out[3].max(ny),
        ];
    }
    Some(out)
}

const SCREEN_RECT_MARGIN: f32 = 0.01;

fn screen_rects_overlap(a: &[f32; 4], b: &[f32; 4]) -> bool {
    a[0] <= b[2] + SCREEN_RECT_MARGIN
        && b[0] <= a[2] + SCREEN_RECT_MARGIN
        && a[1] <= b[3] + SCREEN_RECT_MARGIN
        && b[1] <= a[3] + SCREEN_RECT_MARGIN
}

impl BillboardInstance {
    pub fn screen_bounds(&self, view_proj: &crate::camera::Mat4) -> Option<[f32; 4]> {
        const UNBOUNDED: u32 = BILLBOARD_FLAG_INTERFACE_RECT
            | BILLBOARD_FLAG_SCREEN_OVERLAY
            | BILLBOARD_FLAG_FACING_ARROW
            | BILLBOARD_FLAG_REFLECTION;
        if self.flags & UNBOUNDED != 0 {
            return None;
        }
        let p = self.position;
        let finite = |v: &[f32]| v.iter().all(|c| c.is_finite());
        if !finite(&p) || !finite(&self.size) || !finite(&self.anchor) || !finite(&self.media) {
            return None;
        }
        if self.flags & BILLBOARD_FLAG_CONNECTOR != 0 {
            let span = [self.anchor[0], self.size[0], self.anchor[1]];
            let up = self.size[1];
            let pts = [
                p,
                [p[0], p[1] + up, p[2]],
                [p[0] + span[0], p[1] + span[1], p[2] + span[2]],
                [p[0] + span[0], p[1] + span[1] + up, p[2] + span[2]],
            ];
            return ndc_rect(view_proj, pts);
        }
        if self.flags & BILLBOARD_FLAG_WORLD_QUAD != 0 {
            return ndc_rect(view_proj, self.world_quad_corners());
        }
        let radius = if self.flags & BILLBOARD_FLAG_VELOCITY_QUAD != 0 {
            let a = self.size[0].abs().max(self.size[1].abs());
            let b = self.anchor[0].abs().max(self.anchor[1].abs());
            let u = self.media[0].abs() + self.media[1].abs() + self.media[2].abs();
            (a + b) * u
        } else {
            let ([x0, x1], [y0, y1]) = self.effective_quad_bounds();
            let x = x0.abs().max(x1.abs()) + 0.5 * self.size[0].abs();
            let y = y0.abs().max(y1.abs()) + 0.5 * self.size[1].abs();
            x + y
        };
        ndc_cube_rect(view_proj, p, radius * 1.001 + 1.0e-4)
    }
}

fn ndc_cube_rect(view_proj: &crate::camera::Mat4, center: [f32; 3], r: f32) -> Option<[f32; 4]> {
    let c = &view_proj.cols;
    let [x, y, z] = center;
    let row = |k: usize| c[0][k] * x + c[1][k] * y + c[2][k] * z + c[3][k];
    let extent = |k: usize| r * (c[0][k].abs() + c[1][k].abs() + c[2][k].abs());
    let (cw, ew) = (row(3), extent(3));
    let (w_lo, w_hi) = (cw - ew, cw + ew);
    if w_lo.is_nan() || w_lo <= 1.0e-4 || !w_hi.is_finite() {
        return None;
    }
    let span = |k: usize| {
        let (cv, ev) = (row(k), extent(k));
        let (lo, hi) = (cv - ev, cv + ev);
        ((lo / w_lo).min(lo / w_hi), (hi / w_lo).max(hi / w_hi))
    };
    let ((x0, x1), (y0, y1)) = (span(0), span(1));
    let out = [x0, y0, x1, y1];
    out.iter().all(|v| v.is_finite()).then_some(out)
}

pub fn merge_sprite_steps(
    steps: Vec<SpriteStep>,
    instances: &[BillboardInstance],
    view_proj: &crate::camera::Mat4,
    triangle_bounds: impl Fn(core::ops::Range<u32>) -> Option<[f32; 4]>,
) -> Vec<SpriteStep> {
    if steps
        .iter()
        .filter(|s| matches!(s, SpriteStep::Triangles(_)))
        .count()
        < 2
    {
        return steps;
    }
    let last_triangles = steps
        .iter()
        .rposition(|s| matches!(s, SpriteStep::Triangles(_)))
        .unwrap_or(0);
    let mut out: Vec<SpriteStep> = Vec::with_capacity(steps.len());
    let mut group: Option<usize> = None;
    let mut since: Option<Vec<[f32; 4]>> = Some(Vec::new());
    for (k, step) in steps.into_iter().enumerate() {
        match step {
            SpriteStep::Triangles(range) => {
                let joins = match (group, since.as_ref()) {
                    (Some(g), Some(rects)) => {
                        matches!(&out[g], SpriteStep::Triangles(r) if r.end == range.start)
                            && triangle_bounds(range.clone())
                                .is_some_and(|t| !rects.iter().any(|b| screen_rects_overlap(b, &t)))
                    }
                    _ => false,
                };
                if let (true, Some(g)) = (joins, group) {
                    if let SpriteStep::Triangles(r) = &mut out[g] {
                        r.end = range.end;
                    }
                } else {
                    group = Some(out.len());
                    since = Some(Vec::new());
                    out.push(SpriteStep::Triangles(range));
                }
            }
            SpriteStep::Billboards(range) => {
                if group.is_some()
                    && k < last_triangles
                    && let Some(rects) = since.as_mut()
                {
                    for i in range.clone() {
                        match instances
                            .get(i as usize)
                            .and_then(|b| b.screen_bounds(view_proj))
                        {
                            Some(rect) => rects.push(rect),
                            None => {
                                since = None;
                                break;
                            }
                        }
                    }
                }
                match out.last_mut() {
                    Some(SpriteStep::Billboards(r)) if r.end == range.start => r.end = range.end,
                    _ => out.push(SpriteStep::Billboards(range)),
                }
            }
            SpriteStep::Interface(range) => {
                since = None;
                match out.last_mut() {
                    Some(SpriteStep::Interface(r)) if r.end == range.start => r.end = range.end,
                    _ => out.push(SpriteStep::Interface(range)),
                }
            }
        }
    }
    out
}

pub fn sprite_category_steps(
    instances: &[BillboardInstance],
    order: &SpritePassOrder,
    tri_keys: &[f32],
) -> Vec<SpriteStep> {
    let mut steps = Vec::new();
    let push_billboards = |steps: &mut Vec<SpriteStep>, range: core::ops::Range<u32>| {
        let mut start = range.start;
        while start < range.end {
            let iface = instances[start as usize].is_interface_rect();
            let mut end = start + 1;
            while end < range.end && instances[end as usize].is_interface_rect() == iface {
                end += 1;
            }
            steps.push(if iface {
                SpriteStep::Interface(start..end)
            } else {
                SpriteStep::Billboards(start..end)
            });
            start = end;
        }
    };
    let sprite_keys = &order.keys;
    let (mut i, mut j) = (0usize, 0usize);
    while i < sprite_keys.len() || j < tri_keys.len() {
        let tri_first = |i: usize, j: usize| {
            j < tri_keys.len()
                && (i >= sprite_keys.len()
                    || (!order.interface[i] && tri_keys[j] >= sprite_keys[i]))
        };
        if tri_first(i, j) {
            let start = j;
            while tri_first(i, j) {
                j += 1;
            }
            steps.push(SpriteStep::Triangles(start as u32..j as u32));
        } else {
            let start = i;
            while i < sprite_keys.len() && !tri_first(i, j) {
                i += 1;
            }
            let base = order.sorted.start;
            push_billboards(&mut steps, base + start as u32..base + i as u32);
        }
    }
    push_billboards(&mut steps, order.overlay.clone());
    steps
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::camera::Camera;

    #[test]
    fn world_quad_screen_bounds_covers_all_four_corners_and_blocks_reordering() {
        let cam = Camera::new([0.0, 0.0, 0.0], 20.0, 0.0, 0.7);
        let vp = cam.view_proj_matrix(16.0 / 9.0);
        let corners = [
            [-4.0, 0.0, -4.0],
            [0.0, 0.0, 0.0],
            [4.0, 0.0, -4.0],
            [0.0, 0.0, -8.0],
        ];
        let quad = BillboardInstance::world_quad(corners, [0.0, 0.0], [1.0, 1.0]);
        let expected = ndc_rect(&vp, corners).unwrap();
        let got = quad.screen_bounds(&vp).unwrap();
        assert_eq!(got, expected);
        assert!(
            got[2] - got[0] > 0.2,
            "bounds must span all corners: {got:?}"
        );

        let steps = vec![
            SpriteStep::Triangles(0..1),
            SpriteStep::Billboards(0..1),
            SpriteStep::Triangles(1..2),
        ];
        let c2_rect = ndc_rect(&vp, [corners[2]]).unwrap();
        let merged = merge_sprite_steps(steps.clone(), &[quad], &vp, |_| Some(c2_rect));
        assert_eq!(merged, steps);

        let behind_eye = BillboardInstance::world_quad(
            [
                [0.0, 0.0, 50.0],
                [1.0, 0.0, 50.0],
                [1.0, 0.0, 51.0],
                [0.0, 0.0, 51.0],
            ],
            [0.0, 0.0],
            [1.0, 1.0],
        );
        assert_eq!(behind_eye.screen_bounds(&vp), None);
    }

    #[test]
    fn order_for_sprite_pass_preserves_stability_and_groups() {
        let mut a = BillboardInstance::new([0.0, 0.0, -5.0], [1.0; 2], [0.0; 2], [1.0; 2]);
        a.tint[0] = 1.0;
        let mut b = BillboardInstance::new([0.0, 0.0, -5.0], [1.0; 2], [0.0; 2], [1.0; 2]);
        b.tint[0] = 2.0;
        let mut far = BillboardInstance::new([0.0, 0.0, -10.0], [1.0; 2], [0.0; 2], [1.0; 2]);
        far.tint[0] = 3.0;
        let mut ov1 = BillboardInstance::new([0.0, 0.0, -1.0], [1.0; 2], [0.0; 2], [1.0; 2]);
        ov1.flags = BILLBOARD_FLAG_SCREEN_OVERLAY;
        ov1.tint[0] = 4.0;
        let mut ov2 = BillboardInstance::new([0.0, 0.0, -9.0], [1.0; 2], [0.0; 2], [1.0; 2]);
        ov2.flags = BILLBOARD_FLAG_FACING_ARROW;
        ov2.tint[0] = 5.0;
        let mut r1 = BillboardInstance::new([0.0, 0.0, -3.0], [1.0; 2], [0.0; 2], [1.0; 2]);
        r1.flags = BILLBOARD_FLAG_REFLECTION;
        r1.tint[0] = 6.0;
        let mut r2 = BillboardInstance::new([0.0, 0.0, -8.0], [1.0; 2], [0.0; 2], [1.0; 2]);
        r2.flags = BILLBOARD_FLAG_SUBMERGED;
        r2.tint[0] = 7.0;

        let mut list = vec![ov1, a, r1, far, b, ov2, r2];
        let order = order_for_sprite_pass(&mut list, [0.0; 3], [0.0, 0.0, -1.0]);
        assert_eq!(order.sorted, 0..3);
        assert_eq!(order.overlay, 3..5);
        assert_eq!(order.reflections, 5..7);
        assert_eq!(order.keys, vec![10.0, 5.0, 5.0]);
        assert_eq!(order.reflection_keys, vec![8.0, 3.0]);
        let ids: Vec<f32> = list.iter().map(|i| i.tint[0]).collect();
        assert_eq!(ids, vec![3.0, 1.0, 2.0, 4.0, 5.0, 7.0, 6.0]);
    }
}
