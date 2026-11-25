pub const WORLD_UNITS_PER_CELL: f32 = 512.0;

pub mod billboard;
pub mod camera;
pub mod canvas_fit;
pub mod connector;
pub mod interface_bitmap;
pub mod interface_picture;
pub mod landscape;
pub mod media;
pub mod mesh_pick;
pub mod model;
pub mod model_surfaces;
pub mod projectile;
pub mod rotation;
pub mod selection;
#[cfg(test)]
#[path = "../../mandrake-common/tests/support/require_data.rs"]
pub(crate) mod test_data;
pub mod view_region;
pub mod water;
