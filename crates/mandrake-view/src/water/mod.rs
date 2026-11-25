pub mod bed;
pub mod grid;
pub mod light;
pub mod settings;
pub mod wind;

pub use grid::{coverage_for, grid_info_for};
pub use light::{LightFit, fit_light, light_from_angles};
pub use settings::{WaterColorMode, WaterSettings};

pub fn synthetic_mesh(
    cw: u16,
    ch: u16,
    height: impl Fn(usize, usize) -> i16,
) -> mandrake_tag::MeshTag {
    let cells = (0..usize::from(ch))
        .flat_map(|r| (0..usize::from(cw)).map(move |c| (c, r)))
        .map(|(c, r)| mandrake_tag::mesh::Myth2Cell {
            physical_height: height(c, r),
            normal: 0,
            flags: 0,
            first_object_index: -1,
            media_height: 0,
            model_index: -1,
        })
        .collect::<Vec<_>>();
    mandrake_tag::MeshTag {
        mesh_id: mandrake_common::FourCC::new(*b"test"),
        landscape_tag: mandrake_common::FourCC::new(*b"land"),
        media_tag: mandrake_common::FourCC::new(*b"wate"),
        submesh_w: 1,
        submesh_h: 1,
        cell_width: cw,
        cell_height: ch,
        total_cells: u32::from(cw) * u32::from(ch),
        mesh_offset: 0,
        mesh_size: 0,
        cells,
        palette_count: 0,
        palette_offset: 0,
        marker_count: 0,
        marker_offset: 0,
        palettes: Vec::new(),
        markers: Vec::new(),
        string_list_tag: None,
        overhead_tag: None,
        pregame_story_tag: None,
        postgame_tag: None,
        next_mesh_id: None,
        landscape_shading: Default::default(),
        reverb: None,
    }
}
