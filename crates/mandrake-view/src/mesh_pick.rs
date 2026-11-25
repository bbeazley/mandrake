use crate::view_region::{RegionView, ViewIntegers, ViewRegion};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PickVertex {
    pub cell: [i32; 2],
    pub screen: [i32; 2],
}

pub fn clamp_cursor(view: &RegionView, cursor: [i32; 2]) -> [i32; 2] {
    let clamp = |v: i32, size: i32| {
        let v = v as i16;
        if v < 0 { 0 } else { i32::from(v).min(size) }
    };
    [
        clamp(cursor[0], view.size[0]),
        clamp(cursor[1], view.size[1]),
    ]
}

pub fn triangle_contains(t: &[PickVertex; 3], p: [i32; 2]) -> bool {
    let [a, b, c] = t.map(|v| v.screen);
    let area = (b[0].wrapping_sub(a[0]))
        .wrapping_mul(c[1].wrapping_sub(a[1]))
        .wrapping_add((c[0].wrapping_sub(a[0])).wrapping_mul(a[1].wrapping_sub(b[1])));
    if area < 1 {
        return false;
    }
    let [px, py] = p;
    let edge = |u: [i32; 2], v: [i32; 2]| {
        let l = (u[0].wrapping_sub(v[0])).wrapping_mul(py.wrapping_sub(v[1]));
        let r = (px.wrapping_sub(v[0])).wrapping_mul(u[1].wrapping_sub(v[1]));
        l.wrapping_sub(r) >= 0
    };
    edge(a, c) && edge(b, a) && edge(c, b)
}

pub fn triangle_point(t: &[PickVertex; 3], p: [i32; 2]) -> [i32; 2] {
    let [a, b, c] = *t;
    let w = |v: PickVertex| [v.cell[0] << 9, v.cell[1] << 9];
    let (aw, bw, cw) = (w(a), w(b), w(c));
    let (bsx, bsy) = (b.screen[0] - a.screen[0], b.screen[1] - a.screen[1]);
    let (csx, csy) = (c.screen[0] - a.screen[0], c.screen[1] - a.screen[1]);
    let d = csx.wrapping_mul(bsy).wrapping_sub(bsx.wrapping_mul(csy)) as f32;
    if d == 0.0 {
        return aw;
    }
    let dx = (p[0] - a.screen[0]) as f32;
    let dy = (p[1] - a.screen[1]) as f32;
    let axis = |k: usize| {
        let (bk, ck) = (bw[k] - aw[k], cw[k] - aw[k]);
        let n1 = bsy.wrapping_mul(ck).wrapping_sub(csy.wrapping_mul(bk)) as f32;
        let n2 = bk.wrapping_mul(csx).wrapping_sub(ck.wrapping_mul(bsx)) as f32;
        let t = n1 / d * dx + aw[k] as f32;
        (n2 / d * dy + t) as i32
    };
    [axis(0), axis(1)]
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LandscapeSurface {
    pub key: u32,
    pub corners: [PickVertex; 3],
}

pub fn surface_key(depths: [i32; 3]) -> u32 {
    let depth = depths[0].min(depths[1]).min(depths[2]);
    (depth.wrapping_sub(0x80) as u32) << 14
}

pub fn landscape_surfaces<I>(
    view: &RegionView,
    region: &ViewRegion,
    triangles: impl Fn(u32, u32) -> I,
    vertex: impl Fn(i32, i32) -> [f32; 3],
) -> Vec<LandscapeSurface>
where
    I: IntoIterator<Item = [(u32, u32); 3]>,
{
    let mut built = Vec::new();
    for_each_landscape_surface(view, region, triangles, vertex, |surface| {
        built.push(surface);
    });
    built.sort_by_key(|s| s.key);
    built.reverse();
    built
}

fn for_each_landscape_surface<I>(
    view: &RegionView,
    region: &ViewRegion,
    triangles: impl Fn(u32, u32) -> I,
    vertex: impl Fn(i32, i32) -> [f32; 3],
    mut visit: impl FnMut(LandscapeSurface),
) where
    I: IntoIterator<Item = [(u32, u32); 3]>,
{
    let integers = view.integers();
    let mut row = region.first_row & !1;
    while row < region.last_row.min(region.rect_last_row) {
        let spans = [row, row + 1].map(|r| region.span(r));
        let first = spans.iter().flatten().map(|s| i32::from(s.0)).min();
        let last = spans.iter().flatten().map(|s| i32::from(s.1)).max();
        if let (Some(first), Some(last)) = (first, last) {
            let mut col = first & !1;
            while col < last {
                for corners in triangles(col as u32, row as u32) {
                    if let Some(surface) = landscape_surface(&integers, region, corners, &vertex) {
                        visit(surface);
                    }
                }
                col += 2;
            }
        }
        row += 2;
    }
}

fn landscape_surface(
    integers: &ViewIntegers<'_>,
    region: &ViewRegion,
    corners: [(u32, u32); 3],
    vertex: &impl Fn(i32, i32) -> [f32; 3],
) -> Option<LandscapeSurface> {
    let mut out = [PickVertex {
        cell: [0; 2],
        screen: [0; 2],
    }; 3];
    let mut depths = [0; 3];
    for (k, &(c, r)) in corners.iter().enumerate() {
        let (c, r) = (c as i32, r as i32);
        if !region.vertex_built(c, r) {
            return None;
        }
        let w = vertex(c, r);
        let (screen, depth) = integers.screen_point_and_depth(w)?;
        out[k] = PickVertex {
            cell: [c, r],
            screen,
        };
        depths[k] = depth;
    }
    Some(LandscapeSurface {
        key: surface_key(depths),
        corners: out,
    })
}

pub fn pick_ground<I>(
    view: &RegionView,
    region: &ViewRegion,
    triangles: impl Fn(u32, u32) -> I,
    vertex: impl Fn(i32, i32) -> [f32; 3],
    cursor: [i32; 2],
) -> Option<[i32; 2]>
where
    I: IntoIterator<Item = [(u32, u32); 3]>,
{
    let p = clamp_cursor(view, cursor);
    let mut best: Option<LandscapeSurface> = None;
    for_each_landscape_surface(view, region, triangles, vertex, |s| {
        if best.is_none_or(|b| s.key >= b.key) && triangle_contains(&s.corners, p) {
            best = Some(s);
        }
    });
    best.map(|s| triangle_point(&s.corners, p))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v(cell: [i32; 2], screen: [i32; 2]) -> PickVertex {
        PickVertex { cell, screen }
    }

    #[test]
    fn contains_needs_the_engine_winding_and_keeps_edges() {
        let t = [v([0, 0], [0, 0]), v([0, 1], [10, 0]), v([1, 0], [0, 10])];
        assert!(triangle_contains(&t, [2, 2]));
        assert!(
            triangle_contains(&t, [0, 5]),
            "a point on an edge is inside"
        );
        assert!(triangle_contains(&t, [0, 0]), "a corner is inside");
        assert!(!triangle_contains(&t, [8, 8]));
        assert!(!triangle_contains(&t, [-1, 2]));
        let r = [t[0], t[2], t[1]];
        assert!(!triangle_contains(&r, [2, 2]));
    }

    #[test]
    fn point_interpolates_the_cells_affinely() {
        let t = [
            v([10, 20], [100, 100]),
            v([11, 20], [100, 200]),
            v([10, 21], [200, 100]),
        ];
        assert_eq!(triangle_point(&t, [100, 100]), [10 << 9, 20 << 9]);
        assert_eq!(triangle_point(&t, [100, 200]), [11 << 9, 20 << 9]);
        assert_eq!(triangle_point(&t, [200, 100]), [10 << 9, 21 << 9]);
        assert_eq!(triangle_point(&t, [100, 150]), [(10 << 9) + 256, 20 << 9]);
        assert_eq!(triangle_point(&t, [125, 100]), [10 << 9, (20 << 9) + 128]);
    }

    #[test]
    fn cursor_clamps_to_the_view_size() {
        let view = RegionView {
            eye: [0.0; 3],
            axes: [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]],
            extents: [0.0; 4],
            focal: 1.0,
            center: [512, 211],
            size: [1024, 672],
        };
        assert_eq!(clamp_cursor(&view, [-5, 700]), [0, 672]);
        assert_eq!(clamp_cursor(&view, [150, 596]), [150, 596]);
        assert_eq!(clamp_cursor(&view, [2000, -1]), [1024, 0]);
    }

    #[test]
    fn surface_key_is_the_farthest_corner_less_0x80_shifted() {
        assert_eq!(surface_key([-980, -241, -823]), 4276813824);
        assert!(surface_key([-100, -100, -100]) > surface_key([-200, -100, -100]));
    }

    #[test]
    fn degenerate_collinear_triangle_is_rejected_and_triangle_point_guards_zero_det() {
        let degenerate = [
            v([3, 4], [100, 100]),
            v([4, 4], [200, 100]),
            v([5, 4], [300, 100]),
        ];
        assert!(!triangle_contains(&degenerate, [900, 100]));
        assert!(!triangle_contains(&degenerate, [150, 100]));
        assert_eq!(triangle_point(&degenerate, [900, 100]), [3 << 9, 4 << 9]);

        let view = RegionView {
            eye: [0.0, 0.0, 0.0],
            axes: [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]],
            extents: [0.0; 4],
            focal: 512.0,
            center: [512, 336],
            size: [1024, 672],
        };
        let region = ViewRegion {
            first_row: 0,
            last_row: 2,
            spans: vec![(0, 2), (0, 2)],
            built: vec![(0, 2), (0, 2), (0, 2)],
            rect_last_row: 2,
        };
        let triangles = |_: u32, _: u32| [[(0, 0), (1, 0), (2, 0)]];
        let vertex = |c: i32, _: i32| [c as f32 * 100.0, 0.0, -2000.0];
        assert_eq!(
            pick_ground(&view, &region, triangles, vertex, [900, 336]),
            None
        );
    }

    #[test]
    fn pick_ground_matches_landscape_surfaces_tie_break() {
        let view = RegionView {
            eye: [0.0, 0.0, 0.0],
            axes: [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]],
            extents: [0.0; 4],
            focal: 512.0,
            center: [512, 336],
            size: [1024, 672],
        };
        let region = ViewRegion {
            first_row: 0,
            last_row: 2,
            spans: vec![(0, 2), (0, 2)],
            built: vec![(0, 2), (0, 2), (0, 2)],
            rect_last_row: 2,
        };
        let triangles = |_: u32, _: u32| [[(0, 0), (2, 0), (0, 2)], [(0, 0), (2, 0), (1, 2)]];
        let vertex = |c: i32, r: i32| [c as f32 * 100.0, -r as f32 * 100.0, -2000.0];
        let cursor = [520, 345];
        let list = landscape_surfaces(&view, &region, triangles, vertex);
        let expected = list
            .iter()
            .find(|s| triangle_contains(&s.corners, cursor))
            .map(|s| triangle_point(&s.corners, cursor));
        assert!(expected.is_some());
        assert_eq!(
            pick_ground(&view, &region, triangles, vertex, cursor),
            expected
        );
    }
}
