use mandrake_math::Fixed16;
use mandrake_math::Vec3Fixed;
use mandrake_tag::{MeshHeightMode, MeshTag, get_mesh_height};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct MeshGridCell {
    pub elevation: Fixed16,
    pub media_elevation: Fixed16,
    pub mesh_flags: u16,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct MeshGrid {
    pub width: usize,
    pub height: usize,
    pub cell_size: Fixed16,
    pub cells: Vec<MeshGridCell>,
}

impl MeshGrid {
    pub const DEFAULT_CELL_SIZE: Fixed16 = Fixed16::ONE;

    pub fn new(width: usize, height: usize) -> Self {
        Self {
            width,
            height,
            cell_size: Self::DEFAULT_CELL_SIZE,
            cells: vec![MeshGridCell::default(); width * height],
        }
    }

    pub fn from_mesh(mesh: &MeshTag) -> Self {
        let width = mesh.cell_width as usize;
        let height = mesh.cell_height as usize;
        let mut cells = Vec::with_capacity(width * height);
        for y in 0..height {
            for x in 0..width {
                cells.push(
                    mesh.cell_at(x, y)
                        .map(|c| MeshGridCell {
                            elevation: Fixed16::from_raw(i32::from(c.physical_height) * 128),
                            media_elevation: Fixed16::from_raw(i32::from(c.media_height) * 128),
                            mesh_flags: c.flags,
                        })
                        .unwrap_or_default(),
                );
            }
        }
        Self {
            width,
            height,
            cell_size: Self::DEFAULT_CELL_SIZE,
            cells,
        }
    }

    #[inline]
    pub fn dimensions(&self) -> (usize, usize) {
        (self.width, self.height)
    }

    pub fn update_cell_height(&mut self, x: usize, y: usize, delta: i16, is_media: bool) {
        if x >= self.width || y >= self.height {
            return;
        }
        let idx = y * self.width + x;
        if let Some(cell) = self.cells.get_mut(idx) {
            let delta_raw = i32::from(delta) * 128;
            if is_media {
                cell.media_elevation += Fixed16::from_raw(delta_raw);
            } else {
                cell.elevation += Fixed16::from_raw(delta_raw);
            }
        }
    }

    #[inline]
    pub fn index_of(&self, cell_x: usize, cell_y: usize) -> Option<usize> {
        (cell_x < self.width && cell_y < self.height).then(|| cell_y * self.width + cell_x)
    }

    #[inline]
    pub fn cell_at(&self, cell_x: usize, cell_y: usize) -> Option<&MeshGridCell> {
        self.index_of(cell_x, cell_y)
            .and_then(|i| self.cells.get(i))
    }

    #[inline]
    pub fn cell_at_mut(&mut self, cell_x: usize, cell_y: usize) -> Option<&mut MeshGridCell> {
        self.index_of(cell_x, cell_y)
            .and_then(move |i| self.cells.get_mut(i))
    }

    #[inline]
    pub fn elevation_at(&self, cell_x: usize, cell_y: usize) -> Fixed16 {
        self.cell_at(cell_x, cell_y)
            .map(|c| c.elevation)
            .unwrap_or(Fixed16::ZERO)
    }

    fn half_extents(&self) -> (Fixed16, Fixed16) {
        (
            Fixed16::from_i32(self.width as i32) / Fixed16::from_i32(2),
            Fixed16::from_i32(self.height as i32) / Fixed16::from_i32(2),
        )
    }

    pub fn world_to_grid(&self, pos_x: Fixed16, pos_z: Fixed16) -> (usize, usize) {
        let (half_w, half_d) = self.half_extents();
        let col = ((half_w - pos_x) / self.cell_size)
            .to_i32()
            .clamp(0, self.width.saturating_sub(1) as i32) as usize;
        let row = ((pos_z + half_d) / self.cell_size)
            .to_i32()
            .clamp(0, self.height.saturating_sub(1) as i32) as usize;
        (col, row)
    }

    pub fn grid_to_world(&self, cell_x: usize, cell_y: usize) -> Vec3Fixed {
        let (half_w, half_d) = self.half_extents();
        let cx = Fixed16::from_i32(cell_x as i32) + Fixed16::HALF;
        let cy = Fixed16::from_i32(cell_y as i32) + Fixed16::HALF;
        Vec3Fixed::new(
            half_w - cx * self.cell_size,
            self.elevation_at(cell_x, cell_y),
            cy * self.cell_size - half_d,
        )
    }

    pub fn world_to_mesh_units(&self, pos_x: Fixed16, pos_z: Fixed16) -> (i32, i32) {
        let (half_w, half_d) = self.half_extents();
        let col = (half_w - pos_x) / self.cell_size;
        let row = (pos_z + half_d) / self.cell_size;
        (col.to_raw() >> 7, row.to_raw() >> 7)
    }

    pub fn mesh_height_at_world(
        &self,
        pos_x: Fixed16,
        pos_z: Fixed16,
        mode: MeshHeightMode,
    ) -> Fixed16 {
        let (x, y) = self.world_to_mesh_units(pos_x, pos_z);
        let h = get_mesh_height(self.width, self.height, x, y, mode, |cx, cy| {
            self.cell_at(cx, cy)
                .map(|c| {
                    (
                        (c.elevation.to_raw() >> 7) as i16,
                        (c.media_elevation.to_raw() >> 7) as i16,
                        c.mesh_flags,
                    )
                })
                .unwrap_or_default()
        });
        Fixed16::from_raw(h * 128)
    }

    pub fn elevation_at_world(&self, pos_x: Fixed16, pos_z: Fixed16) -> Fixed16 {
        self.mesh_height_at_world(pos_x, pos_z, MeshHeightMode::Physical)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sloped_grid() -> MeshGrid {
        let mut grid = MeshGrid::new(4, 4);
        for (x, y, h) in [(0, 0, 0), (1, 0, 512), (0, 1, 1024), (1, 1, 2048)] {
            grid.cell_at_mut(x, y).unwrap().elevation = Fixed16::from_raw(h * 128);
        }
        grid
    }

    #[test]
    fn elevation_interpolates_like_get_mesh_height() {
        let grid = sloped_grid();
        let x = Fixed16::from_f32(1.25);
        let z = Fixed16::from_f32(-1.75);
        assert_eq!(grid.world_to_mesh_units(x, z), (384, 128));
        assert_eq!(grid.elevation_at_world(x, z), Fixed16::from_raw(768 * 128));
        assert_eq!(grid.elevation_at(0, 0), Fixed16::ZERO);
        let x = Fixed16::from_f32(1.75);
        let z = Fixed16::from_f32(-1.25);
        assert_eq!(grid.elevation_at_world(x, z), Fixed16::from_i32(2));
    }

    #[test]
    fn mesh_height_modes_use_media_corners() {
        let mut grid = sloped_grid();
        if let Some(c) = grid.cell_at_mut(1, 1) {
            c.media_elevation = Fixed16::from_raw(3000 * 128);
            c.mesh_flags = 0x0800;
        }
        let x = Fixed16::from_f32(1.75);
        let z = Fixed16::from_f32(-1.25);
        assert_eq!(
            grid.mesh_height_at_world(x, z, MeshHeightMode::Surface),
            Fixed16::from_raw(1262 * 128)
        );
        assert_eq!(
            grid.mesh_height_at_world(x, z, MeshHeightMode::Physical),
            Fixed16::from_i32(2)
        );
    }

    #[test]
    fn world_to_grid_roundtrip() {
        let grid = MeshGrid::new(32, 32);
        let world = grid.grid_to_world(5, 10);
        assert_eq!(grid.world_to_grid(world.x, world.z), (5, 10));
    }
}
