pub fn port_model_matrix(m: &[i16; 9], translation: [f32; 3]) -> [[f32; 4]; 4] {
    let e = |i: usize| f32::from(m[i]) / crate::WORLD_UNITS_PER_CELL;
    let col = |r: usize| [-e(3 * r), e(3 * r + 2), e(3 * r + 1), 0.0];
    [
        col(1),
        col(2),
        col(0),
        [translation[0], translation[1], translation[2], 1.0],
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use mandrake_tag::model_hit::{model_world_matrix, transform_geometry_point_to_world};

    #[test]
    fn reference_marker_matrices() {
        assert_eq!(
            model_world_matrix(0x4000, 0xc000),
            [512, 0, 0, 0, 512, 0, 0, 0, 512]
        );
        assert_eq!(
            model_world_matrix(0xc000, 0xc000),
            [-512, 0, 0, 0, -512, 0, 0, 0, 512]
        );
        assert_eq!(
            model_world_matrix(0, 0xc000),
            [0, 512, 0, -512, 0, 0, 0, 0, 512]
        );
    }

    #[test]
    fn port_matrix_matches_engine_transform() {
        let (hw, hh) = (64.0_f32, 48.0_f32);
        let pos = [45661, 28404, 0];
        let origin = [1443_i16, 3340, 0];
        for (yaw, pitch) in [
            (0xc000, 0xc000),
            (0x4000, 0xc000),
            (0, 0xc000),
            (0x1234, 0xc000),
        ] {
            let m = model_world_matrix(yaw, pitch);
            let t = [
                hw - pos[0] as f32 / 512.0,
                pos[2] as f32 / 512.0,
                pos[1] as f32 / 512.0 - hh,
            ];
            let pm = port_model_matrix(&m, t);
            for g in [[1536_i16, 2560, 0], [2560, 3584, 40], [-700, 100, 2000]] {
                let want = transform_geometry_point_to_world(pos, origin, g, &m);
                let local = [
                    (f32::from(g[1]) - f32::from(origin[1])) / crate::WORLD_UNITS_PER_CELL,
                    (f32::from(g[2]) - f32::from(origin[2])) / crate::WORLD_UNITS_PER_CELL,
                    (f32::from(g[0]) - f32::from(origin[0])) / crate::WORLD_UNITS_PER_CELL,
                ];
                let p: [f32; 3] = std::array::from_fn(|i| {
                    pm[0][i] * local[0] + pm[1][i] * local[1] + pm[2][i] * local[2] + pm[3][i]
                });
                let got = [(hw - p[0]) * 512.0, (p[2] + hh) * 512.0, p[1] * 512.0];
                for c in 0..3 {
                    assert!(
                        (got[c] - want[c] as f32).abs() < 1.0,
                        "yaw {yaw:#x} {g:?}: {got:?} vs {want:?}"
                    );
                }
            }
        }
    }
}
