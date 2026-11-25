use mandrake_sim::models::{ModelShape, new_model_cells_for_version};
use mandrake_sim::passability::MeshCells;

fn shape() -> ModelShape {
    ModelShape {
        cells: vec![(0, 0, 1)],
        origin: (0, 0, 0),
        bounds: [(0, 0, -64), (512, 512, 64)],
        ground_vertices: Vec::new(),
        permutation_count: 1,
    }
}

#[test]
fn an_overlapping_model_leaves_the_cells_and_is_reported() {
    let mut cells = MeshCells {
        width: 16,
        height: 16,
        flags: vec![0x0066; 256],
        models: vec![-1; 256],
    };
    let s = shape();
    let at = (4 << 9, 4 << 9, 0);
    let r = new_model_cells_for_version(&mut cells, 0, at, 0, 0xc000, &s, 0x800);
    assert!(!r.overlapped);
    let mine: Vec<usize> = (0..256).filter(|&i| cells.models[i] == 0).collect();
    assert!(!mine.is_empty());
    let r = new_model_cells_for_version(&mut cells, 1, at, 0, 0xc000, &s, 0x800);
    assert!(r.overlapped);
    assert!(mine.iter().all(|&i| cells.models[i] == 0));
    assert!(!cells.models.contains(&1));
    let r = new_model_cells_for_version(&mut cells, 1, (10 << 9, 10 << 9, 0), 0, 0xc000, &s, 0x800);
    assert!(!r.overlapped);
    assert!(cells.models.contains(&1));
}
