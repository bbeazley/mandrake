use byteorder::{BigEndian, ReadBytesExt};
use mandrake_common::{FourCC, M2Error};
use std::io::{Cursor, Read, Write};

use crate::header::decode_c_string;

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct GeometryMaterial {
    pub name: String,
    pub texture_index: i16,
    pub flags: u16,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
pub struct GeometryVertex {
    pub x: i16,
    pub y: i16,
    pub z: i16,
}

impl GeometryVertex {
    pub fn x_f32(&self) -> f32 {
        self.x as f32 / 512.0
    }

    pub fn y_f32(&self) -> f32 {
        self.y as f32 / 512.0
    }

    pub fn z_f32(&self) -> f32 {
        self.z as f32 / 512.0
    }

    pub fn to_tuple(&self) -> (f32, f32, f32) {
        (self.x_f32(), self.y_f32(), self.z_f32())
    }

    pub fn to_renderer_space(&self, origin: (i16, i16, i16)) -> (f32, f32, f32) {
        let dx = (self.x as i32 - origin.0 as i32) as f32 / 512.0;
        let dy = (self.y as i32 - origin.1 as i32) as f32 / 512.0;
        let dz = (self.z as i32 - origin.2 as i32) as f32 / 512.0;
        (dy, dz, dx)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
pub struct SurfaceVertex {
    pub u: u16,
    pub v: u16,
    pub vertex_index: u16,
    pub flags: u16,
}

impl SurfaceVertex {
    pub fn u_f32(&self) -> f32 {
        self.u as f32 / 65535.0
    }

    pub fn v_f32(&self) -> f32 {
        self.v as f32 / 65535.0
    }
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct GeometrySurface {
    pub flags: u16,
    pub normal: (i16, i16, i16),
    pub distance: i32,
    pub transparency: u16,
    pub material_index: i16,
    pub vertices: [SurfaceVertex; 3],
    #[serde(default = "no_draw_after_list")]
    pub draw_after_list: i16,
}

fn no_draw_after_list() -> i16 {
    -1
}

pub const GEOMETRY_SURFACE_TWO_SIDED: u16 = 0x0002;
pub const GEOMETRY_SURFACE_DEPTH_FARTHER: u16 = 0x0004;
pub const GEOMETRY_SURFACE_CATEGORY_2: u16 = 0x0008;
pub const GEOMETRY_SURFACE_DEPTH_NEAREST: u16 = 0x0010;
pub const GEOMETRY_SURFACE_DEPTH_NEAREST_FARTHER: u16 = 0x0090;
pub const GEOMETRY_SURFACE_NOT_SOLID: u16 = 0x0040;

impl GeometrySurface {
    pub fn alpha_u8(&self) -> u8 {
        ((0xFFFFu32 - self.transparency as u32) >> 8) as u8
    }

    pub fn opacity(&self) -> f32 {
        self.alpha_u8() as f32 / 255.0
    }

    pub fn is_translucent(&self) -> bool {
        self.transparency != 0
    }
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct GeometryTag {
    pub flags: u32,
    pub texture_tag: FourCC,
    pub origin: (i16, i16, i16),
    pub material_count: u16,
    pub vertex_count: u16,
    pub surface_count: u16,
    pub materials: Vec<GeometryMaterial>,
    pub vertices: Vec<GeometryVertex>,
    pub surfaces: Vec<GeometrySurface>,
    #[serde(default)]
    pub draw_after_lists: Vec<i16>,
    #[serde(default)]
    pub bounds: [[i16; 3]; 2],
}

impl GeometryTag {
    pub const HEADER_SIZE: usize = 128;
    pub const MATERIAL_SIZE: usize = 64;
    pub const VERTEX_SIZE: usize = 6;
    pub const SURFACE_SIZE: usize = 64;

    pub fn from_bytes(data: &[u8]) -> Result<Self, M2Error> {
        if data.len() < Self::HEADER_SIZE {
            return Err(M2Error::TagParse(format!(
                "Geometry data too short: expected >= 128 bytes, got {}",
                data.len()
            )));
        }

        let mut cursor = Cursor::new(data);

        let flags = cursor.read_u32::<BigEndian>()?;

        let mut tex_bytes = [0u8; 4];
        cursor.read_exact(&mut tex_bytes)?;
        let texture_tag = FourCC::new(tex_bytes);

        let material_count = cursor.read_u16::<BigEndian>()?;
        let vertex_count = cursor.read_u16::<BigEndian>()?;
        let surface_count = cursor.read_u16::<BigEndian>()?;
        let draw_after_count = cursor.read_u16::<BigEndian>()?;

        cursor.set_position(16);
        let ox = cursor.read_i16::<BigEndian>()?;
        let oy = cursor.read_i16::<BigEndian>()?;
        let oz = cursor.read_i16::<BigEndian>()?;
        let origin = (ox, oy, oz);

        cursor.set_position(28);
        let mat_offset = cursor.read_u32::<BigEndian>()? as usize;
        let _mat_size = cursor.read_u32::<BigEndian>()? as usize;

        cursor.set_position(40);
        let vert_offset = cursor.read_u32::<BigEndian>()? as usize;
        let _vert_size = cursor.read_u32::<BigEndian>()? as usize;

        cursor.set_position(52);
        let surf_offset = cursor.read_u32::<BigEndian>()? as usize;
        let _surf_size = cursor.read_u32::<BigEndian>()? as usize;

        cursor.set_position(64);
        let list_offset = cursor.read_u32::<BigEndian>()? as usize;

        cursor.set_position(0x58);
        let mut bounds = [[0i16; 3]; 2];
        for corner in &mut bounds {
            for c in corner.iter_mut() {
                *c = cursor.read_i16::<BigEndian>()?;
            }
        }
        let draw_after_lists: Vec<i16> = crate::parse::checked_table_range(
            Self::HEADER_SIZE,
            list_offset,
            draw_after_count as usize,
            2,
            data.len(),
        )
        .and_then(|r| data.get(r))
        .map(|b| {
            b.as_chunks::<2>()
                .0
                .iter()
                .map(|&w| i16::from_be_bytes(w))
                .collect()
        })
        .unwrap_or_default();

        let mut materials = Vec::new();
        if let Some(mat_range) = crate::parse::checked_table_range(
            Self::HEADER_SIZE,
            mat_offset,
            material_count as usize,
            Self::MATERIAL_SIZE,
            data.len(),
        ) {
            materials.reserve_exact(material_count as usize);
            cursor.set_position(mat_range.start as u64);
            for _ in 0..material_count {
                let mut name_bytes = [0u8; 32];
                cursor.read_exact(&mut name_bytes)?;
                let name = decode_c_string(&name_bytes);
                let texture_index = cursor.read_i16::<BigEndian>()?;
                let flags = cursor.read_u16::<BigEndian>()?;
                let mut pad = [0u8; 28];
                cursor.read_exact(&mut pad)?;

                materials.push(GeometryMaterial {
                    name,
                    texture_index,
                    flags,
                });
            }
        }

        let mut vertices = Vec::new();
        if let Some(vert_range) = crate::parse::checked_table_range(
            Self::HEADER_SIZE,
            vert_offset,
            vertex_count as usize,
            Self::VERTEX_SIZE,
            data.len(),
        ) {
            vertices.reserve_exact(vertex_count as usize);
            cursor.set_position(vert_range.start as u64);
            for _ in 0..vertex_count {
                let x = cursor.read_i16::<BigEndian>()?;
                let y = cursor.read_i16::<BigEndian>()?;
                let z = cursor.read_i16::<BigEndian>()?;
                vertices.push(GeometryVertex { x, y, z });
            }
        }

        let mut surfaces = Vec::new();
        if let Some(surf_range) = crate::parse::checked_table_range(
            Self::HEADER_SIZE,
            surf_offset,
            surface_count as usize,
            Self::SURFACE_SIZE,
            data.len(),
        ) {
            surfaces.reserve_exact(surface_count as usize);
            cursor.set_position(surf_range.start as u64);
            for _ in 0..surface_count {
                let flags = cursor.read_u16::<BigEndian>()?;
                let nx = cursor.read_i16::<BigEndian>()?;
                let ny = cursor.read_i16::<BigEndian>()?;
                let nz = cursor.read_i16::<BigEndian>()?;
                let distance = cursor.read_i32::<BigEndian>()?;

                let mut skip12 = [0u8; 12];
                cursor.read_exact(&mut skip12)?;

                let transparency = cursor.read_u16::<BigEndian>()?;

                let material_index = cursor.read_i16::<BigEndian>()?;

                let mut v_entries = [SurfaceVertex::default(); 3];
                for v in &mut v_entries {
                    let v_coord = cursor.read_u16::<BigEndian>()?;
                    let u = cursor.read_u16::<BigEndian>()?;
                    let vertex_index = cursor.read_u16::<BigEndian>()?;
                    let v_flags = cursor.read_u16::<BigEndian>()?;
                    *v = SurfaceVertex {
                        u,
                        v: v_coord,
                        vertex_index,
                        flags: v_flags,
                    };
                }

                let mut skip8_tail = [0u8; 8];
                cursor.read_exact(&mut skip8_tail)?;
                let draw_after_list = cursor.read_i16::<BigEndian>()?;
                let mut skip2_tail = [0u8; 2];
                cursor.read_exact(&mut skip2_tail)?;

                surfaces.push(GeometrySurface {
                    flags,
                    normal: (nx, ny, nz),
                    distance,
                    transparency,
                    material_index,
                    vertices: v_entries,
                    draw_after_list,
                });
            }
        }

        Ok(Self {
            flags,
            texture_tag,
            origin,
            material_count,
            vertex_count,
            surface_count,
            materials,
            vertices,
            surfaces,
            draw_after_lists,
            bounds,
        })
    }

    pub fn draw_after(&self, index: usize) -> Vec<u16> {
        let Some(first) = self
            .surfaces
            .get(index)
            .map(|s| s.draw_after_list)
            .filter(|&l| l >= 0)
        else {
            return Vec::new();
        };
        self.draw_after_lists
            .iter()
            .skip(first as usize)
            .take_while(|&&w| w != -1)
            .map(|&w| w as u16)
            .collect()
    }

    pub fn resolve_texture_tag(&self, archive: &crate::TagArchive) -> FourCC {
        if let Some(entry) = archive.find_tag_by_id(FourCC::new(*b"core"), self.texture_tag)
            && let Ok(core_bytes) = archive.get_tag_data(entry)
            && core_bytes.len() >= 4
        {
            let mut cc = [0u8; 4];
            cc.copy_from_slice(&core_bytes[0..4]);
            if cc.iter().all(|&b| b.is_ascii_graphic() || b == b' ') {
                return FourCC::new(cc);
            }
        }
        self.texture_tag
    }

    pub fn export_obj<W: Write>(&self, writer: &mut W) -> Result<(), M2Error> {
        writeln!(
            writer,
            "# Myth II 3D Geometry -- Texture {}",
            self.texture_tag
        )?;
        writeln!(
            writer,
            "# Vertices: {}, Surfaces: {}, Materials: {}",
            self.vertices.len(),
            self.surfaces.len(),
            self.materials.len()
        )?;
        writeln!(writer)?;

        for (i, v) in self.vertices.iter().enumerate() {
            writeln!(
                writer,
                "v {:.6} {:.6} {:.6}",
                v.x_f32(),
                v.y_f32(),
                v.z_f32()
            )?;
            if i % 100 == 99 {
                writer.flush()?;
            }
        }

        writeln!(writer)?;
        writeln!(writer, "# UV coordinates")?;
        for surf in &self.surfaces {
            for sv in &surf.vertices {
                writeln!(writer, "vt {:.6} {:.6}", sv.u_f32(), 1.0 - sv.v_f32())?;
            }
        }

        writeln!(writer)?;
        writeln!(writer, "# Faces")?;
        let mut uv_idx = 1usize;
        for surf in &self.surfaces {
            let v0 = surf.vertices[0].vertex_index as usize + 1;
            let v1 = surf.vertices[1].vertex_index as usize + 1;
            let v2 = surf.vertices[2].vertex_index as usize + 1;

            let uv0 = uv_idx;
            let uv1 = uv_idx + 1;
            let uv2 = uv_idx + 2;
            uv_idx += 3;

            writeln!(writer, "f {}/{} {}/{} {}/{}", v0, uv0, v1, uv1, v2, uv2)?;
        }

        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ModelPermutation {
    pub color_table: i16,
    pub material_views: [u8; 32],
}

impl ModelPermutation {
    pub const SIZE: usize = 64;
    pub const NO_VIEW: u8 = 0xFF;

    pub fn material_view(&self, material_index: usize) -> Option<usize> {
        match self.material_views.get(material_index) {
            Some(&v) if v != Self::NO_VIEW => Some(v as usize),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ModelTag {
    pub flags: u32,
    pub geometry_tag: FourCC,
    pub vertex_count: u32,
    #[serde(rename = "submesh_count")]
    pub permutation_count: u16,
    #[serde(rename = "node_count")]
    pub cell_count: u16,
    pub permutations: Vec<ModelPermutation>,
    pub raw_data: Vec<u8>,
}

impl ModelTag {
    pub fn from_bytes(data: &[u8]) -> Result<Self, M2Error> {
        if data.len() < 16 {
            return Err(M2Error::TagParse(format!(
                "Model data too short: expected >= 16 bytes, got {}",
                data.len()
            )));
        }

        let mut cursor = Cursor::new(data);

        let flags = cursor.read_u32::<BigEndian>()?;

        let mut geom_bytes = [0u8; 4];
        cursor.read_exact(&mut geom_bytes)?;
        let geometry_tag = FourCC::new(geom_bytes);

        let vertex_count = cursor.read_u32::<BigEndian>()?;

        let permutation_count = cursor.read_u16::<BigEndian>()?;

        let cell_count = cursor.read_u16::<BigEndian>()?;

        let mut permutations = Vec::new();
        if data.len() >= 0x40
            && let Some(start) = crate::parse::be_u32(data, 0x34).and_then(|base| {
                (base as usize).checked_add(crate::parse::be_u32(data, 0x1c)? as usize)
            })
        {
            for i in 0..permutation_count as usize {
                let Some(o) = i
                    .checked_mul(ModelPermutation::SIZE)
                    .and_then(|off| start.checked_add(off))
                else {
                    break;
                };
                let Some(end) = o.checked_add(ModelPermutation::SIZE) else {
                    break;
                };
                let Some(rec) = data.get(o..end) else {
                    break;
                };
                let mut material_views = [0u8; 32];
                material_views.copy_from_slice(&rec[2..34]);
                permutations.push(ModelPermutation {
                    color_table: i16::from_be_bytes([rec[0], rec[1]]),
                    material_views,
                });
            }
        }

        Ok(Self {
            flags,
            geometry_tag,
            vertex_count,
            permutation_count,
            cell_count,
            permutations,
            raw_data: data.to_vec(),
        })
    }

    pub fn permutation(&self, index: usize) -> Option<&ModelPermutation> {
        if self.permutations.is_empty() {
            return None;
        }
        self.permutations.get(index % self.permutations.len())
    }

    pub fn permutation_index(&self, index: usize) -> usize {
        index.checked_rem(self.permutations.len()).unwrap_or(0)
    }
}

impl GeometryTag {
    pub fn material_bitmap_index(
        &self,
        material_index: usize,
        collection: &crate::CollectionTag,
        permutation: Option<&ModelPermutation>,
    ) -> Option<usize> {
        let mat = self.materials.get(material_index)?;
        if mat.texture_index < 0 {
            return None;
        }
        let view = match permutation {
            Some(p) => p.material_view(material_index)?,
            None => 0,
        };
        match collection.sequence(mat.texture_index as usize) {
            Some(seq) => seq.bitmap_index(view, 0),
            None => Some(mat.texture_index as usize),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_model_tag_synthetic() {
        let mut data = vec![0u8; 64];
        data[0..4].copy_from_slice(&0u32.to_be_bytes());
        data[4..8].copy_from_slice(b"wind");
        data[8..12].copy_from_slice(&57u32.to_be_bytes());
        data[12..14].copy_from_slice(&1u16.to_be_bytes());
        data[14..16].copy_from_slice(&12u16.to_be_bytes());

        let model = ModelTag::from_bytes(&data).expect("Should parse synthetic mode tag");
        assert_eq!(model.geometry_tag, "wind");
        assert_eq!(model.vertex_count, 57);
        assert_eq!(model.permutation_count, 1);
        assert_eq!(model.cell_count, 12);
    }

    #[test]
    fn test_model_tag_permutations_synthetic() {
        let mut data = vec![0u8; 64 + 8 + 128];
        data[4..8].copy_from_slice(b"fa01");
        data[10..12].copy_from_slice(&2u16.to_be_bytes());
        data[12..14].copy_from_slice(&2u16.to_be_bytes());
        data[0x1c..0x20].copy_from_slice(&8u32.to_be_bytes());
        data[0x34..0x38].copy_from_slice(&64u32.to_be_bytes());
        let p0 = 64 + 8;
        data[p0 + 2..p0 + 6].copy_from_slice(&[0, 22, 0xFF, 3]);
        let p1 = p0 + 64;
        data[p1..p1 + 2].copy_from_slice(&1i16.to_be_bytes());
        data[p1 + 2..p1 + 4].copy_from_slice(&[5, 9]);

        let model = ModelTag::from_bytes(&data).unwrap();
        assert_eq!(model.permutations.len(), 2);
        let perm = &model.permutations[0];
        assert_eq!(perm.color_table, 0);
        assert_eq!(perm.material_view(1), Some(22));
        assert_eq!(perm.material_view(2), None, "0xFF = untextured");
        assert_eq!(perm.material_view(40), None);
        assert_eq!(model.permutations[1].color_table, 1);
        assert_eq!(model.permutation(3), Some(&model.permutations[1]));
        assert_eq!(model.permutation_index(4), 0);
        assert!(
            ModelTag::from_bytes(&data[..64])
                .unwrap()
                .permutations
                .is_empty()
        );
    }

    #[test]
    fn test_geometry_tag_synthetic() {
        let total_size = 128 + 64 + 18 + 64;
        let mut data = vec![0u8; total_size];

        data[0..4].copy_from_slice(&1u32.to_be_bytes());
        data[4..8].copy_from_slice(b"wite");
        data[8..10].copy_from_slice(&1u16.to_be_bytes());
        data[10..12].copy_from_slice(&3u16.to_be_bytes());
        data[12..14].copy_from_slice(&1u16.to_be_bytes());

        data[28..32].copy_from_slice(&0u32.to_be_bytes());
        data[32..36].copy_from_slice(&64u32.to_be_bytes());

        data[40..44].copy_from_slice(&64u32.to_be_bytes());
        data[44..48].copy_from_slice(&18u32.to_be_bytes());

        data[52..56].copy_from_slice(&82u32.to_be_bytes());
        data[56..60].copy_from_slice(&64u32.to_be_bytes());

        data[128..134].copy_from_slice(b"roof2\0");
        data[160..162].copy_from_slice(&0i16.to_be_bytes());

        data[192..194].copy_from_slice(&0i16.to_be_bytes());
        data[194..196].copy_from_slice(&512i16.to_be_bytes());
        data[196..198].copy_from_slice(&0i16.to_be_bytes());
        data[198..200].copy_from_slice(&512i16.to_be_bytes());
        data[200..202].copy_from_slice(&0i16.to_be_bytes());
        data[202..204].copy_from_slice(&0i16.to_be_bytes());
        data[204..206].copy_from_slice(&0i16.to_be_bytes());
        data[206..208].copy_from_slice(&0i16.to_be_bytes());
        data[208..210].copy_from_slice(&512i16.to_be_bytes());

        data[236..238].copy_from_slice(&0i16.to_be_bytes());
        data[238..240].copy_from_slice(&0u16.to_be_bytes());
        data[240..242].copy_from_slice(&0u16.to_be_bytes());
        data[242..244].copy_from_slice(&0u16.to_be_bytes());
        data[246..248].copy_from_slice(&65535u16.to_be_bytes());
        data[248..250].copy_from_slice(&0u16.to_be_bytes());
        data[250..252].copy_from_slice(&1u16.to_be_bytes());
        data[254..256].copy_from_slice(&0u16.to_be_bytes());
        data[256..258].copy_from_slice(&65535u16.to_be_bytes());
        data[258..260].copy_from_slice(&2u16.to_be_bytes());

        let geom = GeometryTag::from_bytes(&data).expect("Should parse synthetic geometry tag");
        assert_eq!(geom.texture_tag, "wite");
        assert_eq!(geom.materials.len(), 1);
        assert_eq!(geom.materials[0].name, "roof2");
        assert_eq!(geom.vertices.len(), 3);
        assert_eq!(geom.vertices[1].x_f32(), 1.0);
        assert_eq!(geom.surfaces.len(), 1);
        assert_eq!(geom.surfaces[0].vertices[1].vertex_index, 1);
        assert_eq!(geom.surfaces[0].vertices[1].v, 65535);
        assert_eq!(geom.surfaces[0].vertices[1].u, 0);
        assert_eq!(geom.surfaces[0].vertices[2].v, 0);
        assert_eq!(geom.surfaces[0].vertices[2].u, 65535);

        let mut obj_out = Vec::new();
        geom.export_obj(&mut obj_out).expect("Should export OBJ");
        let obj_str = String::from_utf8(obj_out).unwrap();
        assert!(obj_str.contains("v 0.000000 1.000000 0.000000"));
        assert!(obj_str.contains("f 1/1 2/2 3/3"));
    }

    #[test]
    fn test_geometry_vertex_to_renderer_space() {
        let v = GeometryVertex {
            x: 512,
            y: 1024,
            z: 256,
        };
        let origin = (256, 512, 128);
        let (rx, ry, rz) = v.to_renderer_space(origin);
        assert_eq!((rx, ry, rz), (1.0, 0.25, 0.5));
    }
}
