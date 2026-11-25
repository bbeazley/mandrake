//! `mandrake_view::mesh_pick`'s tests that triangulate with the landscape's LOD blocks
//! ([`crate::mesh::lod_block_triangles`], which stays here with the mesh).

use crate::mesh::lod_block_triangles;
use mandrake_view::mesh_pick::*;
use mandrake_view::view_region::{RegionView, ViewRegion};

#[test]
fn pick_ground_finds_the_point_under_a_projected_cell_point() {
    // A view straight down on a flat 20x20-cell mesh: right = +x, up = +y, back = +z.
    let view = RegionView {
        eye: [5000.0, 5000.0, 6000.0],
        axes: [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]],
        extents: [0.0; 4],
        focal: 600.0,
        center: [512, 336],
        size: [1024, 672],
    };
    let region = ViewRegion {
        first_row: 0,
        last_row: 20,
        spans: vec![(0, 20); 20],
        built: vec![(0, 20); 21],
        rect_last_row: 20,
    };
    let triangles = |col: u32, row: u32| {
        // The engine's two-triangle blocks (LOD word 0), both diagonals across the mesh.
        lod_block_triangles(0, col, row)
            .into_iter()
            .map(move |t| t.corners.map(|(dc, dr)| (col + dc, row + dr)))
    };
    let vertex = |c: i32, r: i32| [(c << 9) as f32, (r << 9) as f32, 0.0];
    for target in [[5000.0, 5000.0], [4321.0, 5876.0], [6100.0, 3900.0]] {
        let s = view.screen_point([target[0], target[1], 0.0]).unwrap();
        let p = pick_ground(&view, &region, triangles, vertex, s).unwrap();
        // One pixel is 10 world units at this depth.
        assert!((p[0] - target[0] as i32).abs() <= 10, "{target:?} -> {p:?}");
        assert!((p[1] - target[1] as i32).abs() <= 10, "{target:?} -> {p:?}");
    }
    // Off the mesh: nothing.
    let s = view.screen_point([-3000.0, 5000.0, 0.0]).unwrap();
    assert_eq!(pick_ground(&view, &region, triangles, vertex, s), None);
}

/// Block 0 is near the eye, blocks 1 and 2 touch the far columns: the walk takes the largest
/// key, and of equal keys the last built.
#[test]
fn walk_order_is_key_then_last_built() {
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
        spans: vec![(0, 6), (0, 6)],
        built: vec![(0, 6), (0, 6), (0, 6)],
        rect_last_row: 2,
    };
    let triangles = |col: u32, row: u32| {
        lod_block_triangles(0, col, row)
            .into_iter()
            .map(move |t| t.corners.map(|(dc, dr)| (col + dc, row + dr)))
    };
    // Depth by column: every triangle of blocks 1 and 2 has a corner at column 4 or more.
    let vertex = |c: i32, r: i32| {
        let z = if c >= 4 { -4000.0 } else { -2000.0 };
        [(c - 3) as f32 * 100.0, (r - 1) as f32 * 100.0, z]
    };
    let list = landscape_surfaces(&view, &region, triangles, vertex);
    assert_eq!(list.len(), 6);
    let cells = |s: &LandscapeSurface| s.corners.map(|v| v.cell);
    let build: Vec<_> = (0..3)
        .flat_map(|b| triangles(b * 2, 0))
        .map(|t| t.map(|(c, r)| [c as i32, r as i32]))
        .collect();
    let order: Vec<_> = list.iter().map(cells).collect();
    let expected: Vec<_> = [1, 0, 5, 4, 3, 2].iter().map(|&i| build[i]).collect();
    assert_eq!(order, expected);
    assert!(list.windows(2).all(|w| w[0].key >= w[1].key));
}

#[test]
fn a_triangle_with_a_vertex_not_built_is_not_in_the_list() {
    let view = RegionView {
        eye: [0.0, 0.0, 0.0],
        axes: [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]],
        extents: [0.0; 4],
        focal: 512.0,
        center: [512, 336],
        size: [1024, 672],
    };
    // The span holds cell column 0 only, and vertices up to column 1 were built.
    let region = ViewRegion {
        first_row: 0,
        last_row: 2,
        spans: vec![(0, 1), (0, 1)],
        built: vec![(0, 1), (0, 1), (0, 1)],
        rect_last_row: 2,
    };
    let triangles = |col: u32, row: u32| {
        lod_block_triangles(0x00ff, col, row)
            .into_iter()
            .map(move |t| t.corners.map(|(dc, dr)| (col + dc, row + dr)))
    };
    let vertex = |c: i32, r: i32| [c as f32 * 100.0, r as f32 * 100.0, -2000.0];
    let list = landscape_surfaces(&view, &region, triangles, vertex);
    // Of the fan's 8 triangles 4 have every corner at column 0 or 1: half of the top, half
    // of the bottom and both of the left side.
    assert_eq!(list.len(), 4);
    assert!(
        list.iter()
            .all(|s| s.corners.iter().all(|v| v.cell[0] <= 1))
    );
}
