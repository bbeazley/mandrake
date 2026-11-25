use byteorder::{BigEndian, ReadBytesExt};
use mandrake_common::{FourCC, M2Error};
use std::io::Cursor;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MeshHeightMode {
    Surface = 0,
    Physical = 1,
    Media = 2,
}

pub const MESH_HEIGHT_VOID: i32 = -0x1000;

pub type MeshHeightCorner = (i16, i16, u16);

pub fn get_mesh_height<F>(
    width: usize,
    height: usize,
    x: i32,
    y: i32,
    mode: MeshHeightMode,
    corner: F,
) -> i32
where
    F: Fn(usize, usize) -> MeshHeightCorner,
{
    if width == 0 || height == 0 {
        return 0;
    }
    let clamp_cell = |c: i32, n: usize| -> usize {
        if c >= 0 && (c as usize) < n {
            c as usize
        } else if c >= n as i32 {
            n - 1
        } else {
            0
        }
    };
    let cx = clamp_cell(x >> 9, width);
    let cy = clamp_cell(y >> 9, height);
    if corner(cx, cy).2 & 0x4000 != 0 {
        return MESH_HEIGHT_VOID;
    }

    let fx = x & 0x1ff;
    let fy = y & 0x1ff;
    let (verts, fxp, fyp): ([(usize, usize); 3], i32, i32) = if (x ^ y) & 0x200 == 0 {
        if fx >= fy {
            ([(1, 0), (0, 0), (1, 1)], 0x200 - fx, fy)
        } else {
            ([(0, 1), (1, 1), (0, 0)], fx, 0x200 - fy)
        }
    } else if fy >= 0x200 - fx {
        ([(1, 1), (0, 1), (1, 0)], 0x200 - fx, 0x200 - fy)
    } else {
        ([(0, 0), (1, 0), (0, 1)], fx, fy)
    };

    let h = |(dx, dy): (usize, usize)| -> i32 {
        let (physical, media, flags) = corner((cx + dx).min(width - 1), (cy + dy).min(height - 1));
        let use_media = match mode {
            MeshHeightMode::Surface => flags & 0x1800 != 0,
            MeshHeightMode::Physical => false,
            MeshHeightMode::Media => true,
        };
        if use_media {
            media as i32
        } else {
            physical as i32
        }
    };
    let h0 = h(verts[0]);
    let h1 = h(verts[1]);
    let h2 = h(verts[2]);
    h0 + ((fxp * (h1 - h0) + fyp * (h2 - h0)) >> 9)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
pub struct Myth2Cell {
    pub physical_height: i16,
    pub normal: u16,
    pub flags: u16,
    pub first_object_index: i16,
    pub media_height: i16,
    pub model_index: i16,
}

impl Myth2Cell {
    pub fn terrain_types(&self) -> (u8, u8) {
        (((self.flags >> 4) & 0xf) as u8, (self.flags & 0xf) as u8)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct MarkerPaletteEntry {
    pub marker_type: i16,
    pub flags: i16,
    pub tag_id: FourCC,
    pub user_data: [u8; 24],
}

impl MarkerPaletteEntry {
    pub const SIZE: usize = 32;

    pub const TYPE_OBSERVER: i16 = 0;
    pub const TYPE_SCENERY: i16 = 1;
    pub const TYPE_UNIT: i16 = 3;
    pub const TYPE_SOUND: i16 = 5;
    pub const TYPE_MODEL: i16 = 6;
    pub const TYPE_PROJECTILE: i16 = 9;
    pub const TYPE_WAYPOINT: i16 = 10;
    pub const TYPE_MODEL_ANIM: i16 = 11;

    pub fn team_index(&self) -> i16 {
        i16::from_be_bytes([self.user_data[0], self.user_data[1]])
    }
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct MarkerEntry {
    pub flags: u32,
    pub marker_type: i16,
    pub palette_index: i16,
    #[serde(rename = "sub_type")]
    pub marker_id: i16,
    pub difficulty_flags: i16,
    pub x: i32,
    pub y: i32,
    pub z: i32,
    pub yaw: i16,
    pub pitch: i16,
    pub roll: i16,
    pub type_data: [u8; 32],
}

impl MarkerEntry {
    pub const SIZE: usize = 64;
    pub const WORLD_ONE: f32 = 512.0;

    pub fn normalized_col(&self, _cell_width: u16) -> f32 {
        self.x as f32 / Self::WORLD_ONE
    }

    pub fn normalized_row(&self, _cell_height: u16) -> f32 {
        self.y as f32 / Self::WORLD_ONE
    }

    pub fn elevation_world(&self) -> f32 {
        self.z as f32 / Self::WORLD_ONE
    }

    pub fn yaw_radians(&self) -> f32 {
        (self.yaw as u16 as f32) * std::f32::consts::TAU / 65536.0
    }

    pub fn model_permutation(&self) -> usize {
        i16::from_be_bytes([self.type_data[4], self.type_data[5]]).max(0) as usize
    }
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct UnitPlacement {
    pub marker_index: usize,
    pub marker_id: i16,
    pub unit_tag: FourCC,
    pub team: i16,
    pub squad: i16,
    pub difficulty_flags: i16,
    pub position: (f32, f32, f32),
    pub yaw: i16,
    pub pitch: i16,
    pub roll: i16,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct MeshTag {
    pub mesh_id: FourCC,
    pub landscape_tag: FourCC,
    pub media_tag: FourCC,
    pub submesh_w: u16,
    pub submesh_h: u16,
    pub cell_width: u16,
    pub cell_height: u16,
    pub total_cells: u32,
    pub mesh_offset: u32,
    pub mesh_size: u32,
    pub cells: Vec<Myth2Cell>,
    pub palette_count: u32,
    pub palette_offset: u32,
    pub marker_count: u32,
    pub marker_offset: u32,
    pub palettes: Vec<MarkerPaletteEntry>,
    pub markers: Vec<MarkerEntry>,
    pub string_list_tag: Option<FourCC>,
    pub overhead_tag: Option<FourCC>,
    pub pregame_story_tag: Option<FourCC>,
    pub postgame_tag: Option<FourCC>,
    pub next_mesh_id: Option<FourCC>,
    pub landscape_shading: LandscapeShading,
    #[serde(default)]
    pub reverb: Option<i32>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
pub struct LandscapeShading {
    pub dark_weight: i16,
    pub light_weight: i16,
    pub dark_color: [u16; 3],
    pub light_color: [u16; 3],
    pub transition: u32,
    pub fog: MeshFog,
}

impl LandscapeShading {
    pub fn from_mesh_bytes(data: &[u8]) -> Self {
        let be16 = |o: usize| {
            data.get(o..o + 2)
                .map_or(0, |b| u16::from_be_bytes([b[0], b[1]]))
        };
        let be32 = |o: usize| {
            data.get(o..o + 4)
                .map_or(0, |b| u32::from_be_bytes([b[0], b[1], b[2], b[3]]))
        };
        Self {
            dark_weight: be16(0x58) as i16,
            light_weight: be16(0x5a) as i16,
            dark_color: [be16(0x5c), be16(0x5e), be16(0x60)],
            light_color: [be16(0x64), be16(0x66), be16(0x68)],
            transition: be32(0x6c),
            fog: MeshFog::from_mesh_bytes(data),
        }
    }

    pub fn transition_level(&self, levels: u32) -> i32 {
        ((u64::from(self.transition) * u64::from(levels.saturating_sub(1))) >> 16) as i32
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
pub struct MeshFog {
    pub color: [u16; 3],
    pub density_bits: u32,
}

impl MeshFog {
    pub fn from_mesh_bytes(data: &[u8]) -> Self {
        let be16 = |o: usize| {
            data.get(o..o + 2)
                .map_or(0, |b| u16::from_be_bytes([b[0], b[1]]))
        };
        Self {
            color: [be16(0x1f0), be16(0x1f2), be16(0x1f4)],
            density_bits: data
                .get(0x1f8..0x1fc)
                .map_or(0, |b| u32::from_be_bytes([b[0], b[1], b[2], b[3]])),
        }
    }

    pub fn density(&self) -> f32 {
        f32::from_bits(self.density_bits)
    }
}

pub(crate) fn read_marker_palettes(
    data: &[u8],
    palette_count: u32,
    palette_offset: u32,
) -> Result<Vec<MarkerPaletteEntry>, M2Error> {
    let mut palettes = Vec::new();
    let count = palette_count as usize;
    let Some(range) = crate::parse::checked_table_range(
        MeshTag::HEADER_SIZE,
        palette_offset as usize,
        count,
        MarkerPaletteEntry::SIZE,
        data.len(),
    ) else {
        return Ok(palettes);
    };
    if count == 0 {
        return Ok(palettes);
    }
    let mut cursor = Cursor::new(data);
    palettes.reserve_exact(count);
    cursor.set_position(range.start as u64);
    for _ in 0..count {
        let marker_type = cursor.read_i16::<BigEndian>()?;
        let flags = cursor.read_i16::<BigEndian>()?;
        let mut tag_bytes = [0u8; 4];
        std::io::Read::read_exact(&mut cursor, &mut tag_bytes)?;
        let tag_id = FourCC::new(tag_bytes);
        let mut user_data = [0u8; 24];
        std::io::Read::read_exact(&mut cursor, &mut user_data)?;

        palettes.push(MarkerPaletteEntry {
            marker_type,
            flags,
            tag_id,
            user_data,
        });
    }

    Ok(palettes)
}

pub(crate) fn read_markers(
    data: &[u8],
    marker_count: u32,
    marker_offset: u32,
) -> Result<Vec<MarkerEntry>, M2Error> {
    let mut markers = Vec::new();
    let count = marker_count as usize;
    let Some(range) = crate::parse::checked_table_range(
        MeshTag::HEADER_SIZE,
        marker_offset as usize,
        count,
        MarkerEntry::SIZE,
        data.len(),
    ) else {
        return Ok(markers);
    };
    if count == 0 {
        return Ok(markers);
    }
    let mut cursor = Cursor::new(data);
    markers.reserve_exact(count);
    cursor.set_position(range.start as u64);
    for _ in 0..count {
        let flags = cursor.read_u32::<BigEndian>()?;
        let marker_type = cursor.read_i16::<BigEndian>()?;
        let palette_index = cursor.read_i16::<BigEndian>()?;
        let marker_id = cursor.read_i16::<BigEndian>()?;
        let difficulty_flags = cursor.read_i16::<BigEndian>()?;
        let x = cursor.read_i32::<BigEndian>()?;
        let y = cursor.read_i32::<BigEndian>()?;
        let z = cursor.read_i32::<BigEndian>()?;
        let legacy_yaw = cursor.read_i16::<BigEndian>()?;
        let legacy_pitch = cursor.read_i16::<BigEndian>()?;
        let roll = cursor.read_i16::<BigEndian>()?;

        let mut pad2 = [0u8; 2];
        std::io::Read::read_exact(&mut cursor, &mut pad2)?;

        let mut type_data = [0u8; 32];
        std::io::Read::read_exact(&mut cursor, &mut type_data)?;

        let real_yaw = i16::from_be_bytes([type_data[0], type_data[1]]);
        let real_pitch = i16::from_be_bytes([type_data[2], type_data[3]]);
        let yaw = if real_yaw != 0 || real_pitch != 0 {
            real_yaw
        } else {
            legacy_yaw
        };
        let pitch = if real_yaw != 0 || real_pitch != 0 {
            real_pitch
        } else {
            legacy_pitch
        };

        markers.push(MarkerEntry {
            flags,
            marker_type,
            palette_index,
            marker_id,
            difficulty_flags,
            x,
            y,
            z,
            yaw,
            pitch,
            roll,
            type_data,
        });
    }

    Ok(markers)
}

impl MeshTag {
    pub const HEADER_SIZE: usize = 1024;
    pub const CELL_SIZE: usize = 12;

    pub fn from_bytes(data: &[u8]) -> Result<Self, M2Error> {
        if data.len() < Self::HEADER_SIZE {
            return Err(M2Error::TagParse(format!(
                "Mesh tag data too short: expected >= 1024 bytes, got {}",
                data.len()
            )));
        }

        let mut cursor = Cursor::new(data);

        let mut landscape_bytes = [0u8; 4];
        std::io::Read::read_exact(&mut cursor, &mut landscape_bytes)?;
        let landscape_tag = FourCC::new(landscape_bytes);

        let mut media_bytes = [0u8; 4];
        std::io::Read::read_exact(&mut cursor, &mut media_bytes)?;
        let media_tag = FourCC::new(media_bytes);

        let submesh_w = cursor.read_u16::<BigEndian>()?;
        let submesh_h = cursor.read_u16::<BigEndian>()?;
        let mesh_offset = cursor.read_u32::<BigEndian>()?;
        let mesh_size = cursor.read_u32::<BigEndian>()?;

        let cell_width_u32 = u32::from(submesh_w) * 32;
        let cell_height_u32 = u32::from(submesh_h) * 32;
        let cell_width = u16::try_from(cell_width_u32).map_err(|_| {
            M2Error::TagParse(format!("mesh submesh_w {submesh_w} overflows cell_width"))
        })?;
        let cell_height = u16::try_from(cell_height_u32).map_err(|_| {
            M2Error::TagParse(format!("mesh submesh_h {submesh_h} overflows cell_height"))
        })?;
        let total_cells = cell_width_u32
            .checked_mul(cell_height_u32)
            .ok_or_else(|| M2Error::TagParse("mesh cell count overflow".into()))?;

        cursor.set_position(36);
        let palette_count = cursor.read_u32::<BigEndian>()?;
        let palette_offset = cursor.read_u32::<BigEndian>()?;

        cursor.set_position(52);
        let marker_count = cursor.read_u32::<BigEndian>()?;
        let marker_offset = cursor.read_u32::<BigEndian>()?;

        let mut string_list_tag = None;
        let mut overhead_tag = None;
        let mut pregame_story_tag = None;
        let mut postgame_tag = None;
        let mut next_mesh_id = None;

        cursor.set_position(140);
        let mut buf = [0u8; 4];
        if std::io::Read::read_exact(&mut cursor, &mut buf).is_ok() && Self::is_valid_fourcc(&buf) {
            string_list_tag = Some(FourCC::new(buf));
        }
        if std::io::Read::read_exact(&mut cursor, &mut buf).is_ok() && Self::is_valid_fourcc(&buf) {
            postgame_tag = Some(FourCC::new(buf));
        }
        if std::io::Read::read_exact(&mut cursor, &mut buf).is_ok() && Self::is_valid_fourcc(&buf) {
            pregame_story_tag = Some(FourCC::new(buf));
        }

        cursor.set_position(152);
        if std::io::Read::read_exact(&mut cursor, &mut buf).is_ok() && Self::is_valid_fourcc(&buf) {
            overhead_tag = Some(FourCC::new(buf));
        }
        if std::io::Read::read_exact(&mut cursor, &mut buf).is_ok() && Self::is_valid_fourcc(&buf) {
            next_mesh_id = Some(FourCC::new(buf));
        }

        let cell_range = crate::parse::checked_table_range(
            Self::HEADER_SIZE,
            mesh_offset as usize,
            total_cells as usize,
            Self::CELL_SIZE,
            data.len(),
        );
        let expected_bytes = (total_cells as usize).checked_mul(Self::CELL_SIZE);

        if mesh_size != 0 && expected_bytes != Some(mesh_size as usize) {
            return Err(M2Error::TagParse(format!(
                "mesh cells: {mesh_size} bytes in a {}-byte tag, \
                 expected {total_cells} cells of {} bytes",
                data.len(),
                Self::CELL_SIZE
            )));
        }

        let cell_range = cell_range.ok_or_else(|| {
            M2Error::TagParse(format!(
                "mesh cells: {total_cells} cells at {}+{} exceed {}-byte tag",
                Self::HEADER_SIZE,
                mesh_offset,
                data.len(),
            ))
        })?;

        let mut cells = Vec::with_capacity(total_cells as usize);
        cursor.set_position(cell_range.start as u64);
        for _ in 0..total_cells {
            let physical_height = cursor.read_i16::<BigEndian>()?;
            let normal = cursor.read_u16::<BigEndian>()?;
            let flags = cursor.read_u16::<BigEndian>()?;
            let first_object_index = cursor.read_i16::<BigEndian>()?;
            let media_height = cursor.read_i16::<BigEndian>()?;
            let model_index = cursor.read_i16::<BigEndian>()?;

            cells.push(Myth2Cell {
                physical_height,
                normal,
                flags,
                first_object_index,
                media_height,
                model_index,
            });
        }

        let palettes = read_marker_palettes(data, palette_count, palette_offset)?;
        let markers = read_markers(data, marker_count, marker_offset)?;

        Ok(Self {
            mesh_id: landscape_tag,
            landscape_tag,
            media_tag,
            submesh_w,
            submesh_h,
            cell_width,
            cell_height,
            total_cells,
            mesh_offset,
            mesh_size,
            cells,
            palette_count,
            palette_offset,
            marker_count,
            marker_offset,
            palettes,
            markers,
            string_list_tag,
            overhead_tag,
            pregame_story_tag,
            postgame_tag,
            next_mesh_id,
            landscape_shading: LandscapeShading::from_mesh_bytes(data),
            reverb: Some(i32::from_be_bytes(
                data[Self::REVERB_OFFSET..Self::REVERB_OFFSET + 4]
                    .try_into()
                    .expect("the header is 1024 bytes"),
            )),
        })
    }

    pub const AMBIENT_SOUND_TAG_OFFSET: usize = 0x7c;

    pub const REVERB_OFFSET: usize = 0x10c;

    pub fn ambient_sound_tag_from_bytes(data: &[u8]) -> Option<FourCC> {
        let o = Self::AMBIENT_SOUND_TAG_OFFSET;
        let bytes: [u8; 4] = data.get(o..o + 4)?.try_into().ok()?;
        match u32::from_be_bytes(bytes) {
            0 | 0xffff_ffff => None,
            _ => Some(FourCC::new(bytes)),
        }
    }

    pub const CUTSCENES_OFFSET: usize = 0x12c;
    pub const CUTSCENE_NAME_LENGTH: usize = 0x40;

    pub fn cutscene_names_from_bytes(data: &[u8]) -> [Option<Vec<u8>>; 3] {
        std::array::from_fn(|index| {
            let o = Self::CUTSCENES_OFFSET + index * Self::CUTSCENE_NAME_LENGTH;
            let field = data.get(o..o + Self::CUTSCENE_NAME_LENGTH)?;
            let name = &field[..field.iter().position(|&b| b == 0).unwrap_or(field.len())];
            (!name.is_empty()).then(|| name.to_vec())
        })
    }

    pub const FLAGS_OFFSET: usize = 0x4c;
    pub const FLAG_NO_SPEED_KEYS: u32 = 0x0080_0000;

    pub fn flags_from_bytes(data: &[u8]) -> u32 {
        let o = Self::FLAGS_OFFSET;
        data.get(o..o + 4)
            .and_then(|b| b.try_into().ok())
            .map_or(0, u32::from_be_bytes)
    }

    pub fn blocks_speed_keys_from_bytes(data: &[u8]) -> bool {
        Self::flags_from_bytes(data) & Self::FLAG_NO_SPEED_KEYS != 0
    }

    pub fn dimensions(&self) -> (u16, u16) {
        (self.cell_width, self.cell_height)
    }

    pub fn elevation_at(&self, x: usize, y: usize) -> Option<i16> {
        let w = self.cell_width as usize;
        let h = self.cell_height as usize;
        if x < w && y < h {
            let idx = y * w + x;
            self.cells.get(idx).map(|c| c.physical_height)
        } else {
            None
        }
    }

    pub fn cell_at(&self, x: usize, y: usize) -> Option<&Myth2Cell> {
        let w = self.cell_width as usize;
        let h = self.cell_height as usize;
        if x < w && y < h {
            let idx = y * w + x;
            self.cells.get(idx)
        } else {
            None
        }
    }

    pub fn terrain_height_at(&self, col: f32, row: f32) -> f32 {
        if self.cell_width == 0 || self.cell_height == 0 || self.cells.is_empty() {
            return 0.0;
        }
        let max_c = (self.cell_width - 1) as f32;
        let max_r = (self.cell_height - 1) as f32;
        let c = col.clamp(0.0, max_c);
        let r = row.clamp(0.0, max_r);
        let x0 = (c.floor() as usize).min(self.cell_width as usize - 1);
        let y0 = (r.floor() as usize).min(self.cell_height as usize - 1);
        let x1 = (x0 + 1).min(self.cell_width as usize - 1);
        let y1 = (y0 + 1).min(self.cell_height as usize - 1);
        let tx = c - x0 as f32;
        let ty = r - y0 as f32;

        let h00 = self.elevation_at(x0, y0).unwrap_or(0) as f32 / MarkerEntry::WORLD_ONE;
        let h10 = self.elevation_at(x1, y0).unwrap_or(0) as f32 / MarkerEntry::WORLD_ONE;
        let h11 = self.elevation_at(x1, y1).unwrap_or(0) as f32 / MarkerEntry::WORLD_ONE;
        let h01 = self.elevation_at(x0, y1).unwrap_or(0) as f32 / MarkerEntry::WORLD_ONE;

        if ((x0 ^ y0) & 1) == 0 {
            if tx >= ty {
                h00 + tx * (h10 - h00) + ty * (h11 - h10)
            } else {
                h00 + tx * (h11 - h01) + ty * (h01 - h00)
            }
        } else if tx + ty <= 1.0 {
            h00 + tx * (h10 - h00) + ty * (h01 - h00)
        } else {
            h11 + (1.0 - tx) * (h01 - h11) + (1.0 - ty) * (h10 - h11)
        }
    }

    pub fn mesh_height(&self, x: i32, y: i32, mode: MeshHeightMode) -> i32 {
        get_mesh_height(
            self.cell_width as usize,
            self.cell_height as usize,
            x,
            y,
            mode,
            |cx, cy| {
                self.cell_at(cx, cy)
                    .map(|c| (c.physical_height, c.media_height, c.flags))
                    .unwrap_or_default()
            },
        )
    }

    pub fn marker_world_position(&self, marker: &MarkerEntry, cell_size: f32) -> (f32, f32, f32) {
        let norm_c = marker.x as f32 / MarkerEntry::WORLD_ONE;
        let norm_r = marker.y as f32 / MarkerEntry::WORLD_ONE;
        let half_w = self.cell_width as f32 * cell_size * 0.5;
        let half_d = self.cell_height as f32 * cell_size * 0.5;
        let world_x = half_w - (norm_c * cell_size);
        let world_z = (norm_r * cell_size) - half_d;
        let world_y = if marker.z != 0 {
            marker.z as f32 / MarkerEntry::WORLD_ONE
        } else {
            self.terrain_height_at(norm_c, norm_r)
        };
        (world_x, world_y, world_z)
    }

    pub fn palette_for_marker(&self, marker: &MarkerEntry) -> Option<&MarkerPaletteEntry> {
        if marker.palette_index < 0 {
            return None;
        }
        let idx = marker.palette_index as usize;
        self.palettes
            .iter()
            .filter(|p| p.marker_type == marker.marker_type)
            .nth(idx)
            .or_else(|| self.palettes.get(idx))
    }

    pub fn palette_tag_for_marker(&self, marker: &MarkerEntry) -> Option<FourCC> {
        self.palette_for_marker(marker).map(|p| p.tag_id)
    }

    pub fn markers_by_type(&self, marker_type: i16) -> impl Iterator<Item = &MarkerEntry> {
        self.markers
            .iter()
            .filter(move |m| m.marker_type == marker_type)
    }

    pub fn scenery_marker_by_identifier(&self, identifier: i16) -> Option<&MarkerEntry> {
        if identifier == -1 {
            return None;
        }
        self.scenery_markers().find(|m| m.marker_id == identifier)
    }

    pub fn observer_markers(&self) -> impl Iterator<Item = &MarkerEntry> {
        self.markers_by_type(MarkerPaletteEntry::TYPE_OBSERVER)
    }

    pub fn start_observer_marker(&self, local_team: i16) -> Option<&MarkerEntry> {
        let mut fallback = None;
        for marker in self.observer_markers() {
            let Some(palette) = self.palette_for_marker(marker) else {
                continue;
            };
            if palette.team_index() == local_team {
                return Some(marker);
            }
            fallback.get_or_insert(marker);
        }
        fallback.or_else(|| self.observer_markers().next())
    }

    pub fn unit_markers(&self) -> impl Iterator<Item = &MarkerEntry> {
        self.markers_by_type(MarkerPaletteEntry::TYPE_UNIT)
    }

    pub fn scenery_markers(&self) -> impl Iterator<Item = &MarkerEntry> {
        self.markers_by_type(MarkerPaletteEntry::TYPE_SCENERY)
    }

    pub fn sound_markers(&self) -> impl Iterator<Item = &MarkerEntry> {
        self.markers_by_type(MarkerPaletteEntry::TYPE_SOUND)
    }

    pub fn model_markers(&self) -> impl Iterator<Item = &MarkerEntry> {
        self.markers_by_type(MarkerPaletteEntry::TYPE_MODEL)
    }

    pub fn model_anim_markers(&self) -> impl Iterator<Item = &MarkerEntry> {
        self.markers_by_type(MarkerPaletteEntry::TYPE_MODEL_ANIM)
    }

    pub fn unit_placements(&self, cell_size: f32) -> Vec<UnitPlacement> {
        let mut placements = Vec::new();
        for (idx, marker) in self.markers.iter().enumerate() {
            if marker.marker_type == MarkerPaletteEntry::TYPE_UNIT {
                let palette = self.palette_for_marker(marker);
                let unit_tag = palette
                    .map(|p| p.tag_id)
                    .unwrap_or_else(|| FourCC::new(*b"none"));
                let team = palette.map(|p| p.team_index()).unwrap_or(0);
                let squad = marker.palette_index;
                let position = self.marker_world_position(marker, cell_size);

                placements.push(UnitPlacement {
                    marker_index: idx,
                    marker_id: marker.marker_id,
                    unit_tag,
                    team,
                    squad,
                    difficulty_flags: marker.difficulty_flags,
                    position,
                    yaw: marker.yaw,
                    pitch: marker.pitch,
                    roll: marker.roll,
                });
            }
        }
        placements
    }

    pub fn export_obj(
        &self,
        output_path: &std::path::Path,
        height_scale: f32,
    ) -> Result<(), M2Error> {
        use std::io::Write;

        let vw = (self.cell_width + 1) as usize;
        let vh = (self.cell_height + 1) as usize;
        let total_verts = vw * vh;

        let half_w = (vw - 1) as f32 * 0.5;
        let half_h = (vh - 1) as f32 * 0.5;

        let mut vx = vec![0.0f32; total_verts];
        let mut vy = vec![0.0f32; total_verts];
        let mut vz = vec![0.0f32; total_verts];

        for y in 0..vh {
            for x in 0..vw {
                let vi = y * vw + x;
                let cx = x.min(self.cell_width as usize - 1);
                let cy = y.min(self.cell_height as usize - 1);
                let c = self.cell_at(cx, cy).copied().unwrap_or_default();

                vx[vi] = half_w - x as f32;
                vy[vi] = c.physical_height as f32 * height_scale;
                vz[vi] = y as f32 - half_h;
            }
        }

        let mut file = std::fs::File::create(output_path)?;

        writeln!(file, "# Myth II terrain -- {}", self.mesh_id)?;
        writeln!(
            file,
            "# Submeshes: {}x{}  Cells: {}x{}  Verts: {}  Tris: {}",
            self.submesh_w,
            self.submesh_h,
            self.cell_width,
            self.cell_height,
            total_verts,
            self.cell_width as usize * self.cell_height as usize * 2
        )?;
        writeln!(file, "# Height scale: {}", height_scale)?;
        writeln!(file)?;

        for i in 0..total_verts {
            writeln!(file, "v {:.4} {:.6} {:.4}", vx[i], vy[i], vz[i])?;
        }

        writeln!(file)?;
        writeln!(file, "# UVs")?;
        for y in 0..vh {
            let v = 1.0 - (y as f32 / (vh - 1) as f32);
            for x in 0..vw {
                let u = 1.0 - (x as f32 / (vw - 1) as f32);
                writeln!(file, "vt {:.6} {:.6}", u, v)?;
            }
        }

        writeln!(file)?;
        writeln!(file, "# Faces")?;
        for y in 0..(self.cell_height as usize) {
            for x in 0..(self.cell_width as usize) {
                let a = y * vw + x + 1;
                let b = y * vw + (x + 1) + 1;
                let c = (y + 1) * vw + (x + 1) + 1;
                let d = (y + 1) * vw + x + 1;

                if ((x ^ y) & 1) == 0 {
                    writeln!(file, "f {0}/{0} {1}/{1} {2}/{2}", a, b, c)?;
                    writeln!(file, "f {0}/{0} {1}/{1} {2}/{2}", a, c, d)?;
                } else {
                    writeln!(file, "f {0}/{0} {1}/{1} {2}/{2}", a, b, d)?;
                    writeln!(file, "f {0}/{0} {1}/{1} {2}/{2}", b, c, d)?;
                }
            }
        }

        Ok(())
    }

    fn is_valid_fourcc(buf: &[u8; 4]) -> bool {
        if *buf == [0, 0, 0, 0] || *buf == [0xFF, 0xFF, 0xFF, 0xFF] {
            return false;
        }
        buf.iter().all(|&b| b.is_ascii_graphic() || b == b' ')
    }
}

pub const MEDIA_COVERAGE_PIXEL_SHIFT: u32 = 6;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MediaCoverage {
    pub width: i32,
    pub height: i32,
    pub rows: Vec<Vec<i16>>,
}

impl MediaCoverage {
    pub const SECTION_OFFSET: usize = 0xc0;

    pub fn from_mesh_bytes(data: &[u8], cell_width: u16, cell_height: u16) -> Self {
        let width = cell_width as i32 * 8;
        let height = cell_height as i32 * 8;
        let be32 = |o: usize| {
            data.get(o..o + 4)
                .map(|b| u32::from_be_bytes([b[0], b[1], b[2], b[3]]) as usize)
        };
        let section = match (be32(Self::SECTION_OFFSET), be32(Self::SECTION_OFFSET + 4)) {
            (Some(off), Some(size)) if size > 0 => {
                let start = MeshTag::HEADER_SIZE.saturating_add(off);
                data.get(start..start.saturating_add(size))
            }
            _ => None,
        };
        let mut rows = Vec::with_capacity(height.max(0) as usize);
        match section {
            Some(bytes) => {
                let mut words = bytes
                    .as_chunks::<2>()
                    .0
                    .iter()
                    .map(|w| i16::from_be_bytes(*w));
                for _ in 0..height {
                    let mut row = Vec::new();
                    for w in words.by_ref() {
                        row.push(w);
                        if w as i32 >= width {
                            break;
                        }
                    }
                    if row.last().is_none_or(|&w| (w as i32) < width) {
                        row.push(width as i16);
                    }
                    rows.push(row);
                }
            }
            None => rows.resize(height.max(0) as usize, vec![width as i16]),
        }
        Self {
            width,
            height,
            rows,
        }
    }

    pub fn pixel_is_media(&self, x: i32, y: i32) -> bool {
        if x < 0 || x >= self.width || y < 0 || y >= self.height {
            return false;
        }
        let Some(row) = self.rows.get(y as usize) else {
            return false;
        };
        let mut media = false;
        let mut i = 0;
        if row.first().is_none_or(|&e| x <= e as i32) {
            return false;
        }
        loop {
            i += 1;
            media = !media;
            match row.get(i) {
                Some(&e) if x > e as i32 => continue,
                _ => return media,
            }
        }
    }

    pub fn point_is_media(&self, x: i32, y: i32) -> bool {
        self.pixel_is_media(
            x >> MEDIA_COVERAGE_PIXEL_SHIFT,
            y >> MEDIA_COVERAGE_PIXEL_SHIFT,
        )
    }
}

pub const LOD_EDGE_SUBDIVIDED_MASK: u16 = 0x000f;
pub const LOD_CELL_DRAWN_SHIFT: u32 = 4;
pub const LOD_MEDIA_SHIFT: u32 = 8;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MeshLod {
    pub block_width: usize,
    pub block_height: usize,
    pub words: Vec<u16>,
}

impl MeshLod {
    pub const SECTION_OFFSET: usize = 0xcc;

    pub fn from_mesh_bytes(data: &[u8], cell_width: u16, cell_height: u16) -> Option<Self> {
        let be32 = |o: usize| {
            data.get(o..o + 4)
                .map(|b| u32::from_be_bytes([b[0], b[1], b[2], b[3]]) as usize)
        };
        let (off, size) = (be32(Self::SECTION_OFFSET)?, be32(Self::SECTION_OFFSET + 4)?);
        let (block_width, block_height) = (cell_width as usize / 2, cell_height as usize / 2);
        let count = block_width * block_height;
        if count == 0 || size < count * 2 {
            return None;
        }
        let start = MeshTag::HEADER_SIZE.checked_add(off)?;
        let bytes = data.get(start..start.checked_add(count * 2)?)?;
        Some(Self {
            block_width,
            block_height,
            words: bytes
                .as_chunks::<2>()
                .0
                .iter()
                .map(|w| u16::from_be_bytes(*w))
                .collect(),
        })
    }

    pub fn word(&self, block_x: usize, block_y: usize) -> Option<u16> {
        if block_x >= self.block_width || block_y >= self.block_height {
            return None;
        }
        self.words
            .get(block_y * self.block_width + block_x)
            .copied()
    }
}

pub fn lod_media_bits_from_cells(c00: u16, c10: u16, c11: u16, c01: u16) -> u16 {
    let b = |flags: u16, mask: u16, bit: u32| if flags & mask != 0 { 1 << bit } else { 0 };
    b(c00, 0x0800, 15)
        | b(c00, 0x1000, 8)
        | b(c10, 0x0800, 9)
        | b(c10, 0x1000, 10)
        | b(c11, 0x0800, 12)
        | b(c11, 0x1000, 11)
        | b(c01, 0x0800, 14)
        | b(c01, 0x1000, 13)
}

pub const MYTH2_MESH_VERSION: i16 = 11;

#[derive(Debug, Clone, PartialEq)]
pub enum GameMesh {
    Myth2(MeshTag),
    Tfl(crate::tfl::mesh::TflMesh),
}

impl GameMesh {
    pub fn from_tag(version: i16, data: &[u8]) -> Result<Self, M2Error> {
        match version {
            MYTH2_MESH_VERSION => MeshTag::from_bytes(data).map(Self::Myth2),
            v if v as u16 == crate::tfl::mesh::TFL_MESH_VERSION => {
                crate::tfl::mesh::TflMesh::from_bytes(data).map(Self::Tfl)
            }
            v => Err(M2Error::TagParse(format!(
                "mesh version {v}: not Myth II's {MYTH2_MESH_VERSION} or TFL's {}",
                crate::tfl::mesh::TFL_MESH_VERSION
            ))),
        }
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn the_three_cutscene_names() {
        let mut data = vec![0u8; 0x400];
        assert_eq!(
            MeshTag::cutscene_names_from_bytes(&data),
            [None, None, None]
        );
        data[0x12c..0x12c + 6].copy_from_slice(b"01.smk");
        data[0x1ac..0x1ac + 0x40].fill(b'a');
        let names = MeshTag::cutscene_names_from_bytes(&data);
        assert_eq!(names[0].as_deref(), Some(&b"01.smk"[..]));
        assert_eq!(names[1], None);
        assert_eq!(names[2].as_ref().map(Vec::len), Some(0x40));
        assert_eq!(
            MeshTag::cutscene_names_from_bytes(&data[..0x16c]),
            [Some(b"01.smk".to_vec()), None, None]
        );
    }

    #[test]
    fn the_flags_word_and_the_speed_key_flag() {
        let mut data = vec![0u8; 0x50];
        assert_eq!(MeshTag::flags_from_bytes(&data), 0);
        assert!(!MeshTag::blocks_speed_keys_from_bytes(&data));
        data[0x4d] = 0x80;
        assert_eq!(MeshTag::flags_from_bytes(&data), 0x0080_0000);
        assert!(MeshTag::blocks_speed_keys_from_bytes(&data));
        data[0x4d] = 0x7f;
        data[0x4f] = 0xff;
        assert!(!MeshTag::blocks_speed_keys_from_bytes(&data));
        assert_eq!(MeshTag::flags_from_bytes(&data[..0x4f]), 0);
    }

    #[test]
    fn mesh_fog_from_the_header() {
        let mut d = vec![0u8; 0x400];
        d[0x1f0..0x1f6].copy_from_slice(&[0xcc, 0xcc, 0xcc, 0xcc, 0xcc, 0xcc]);
        d[0x1f8..0x1fc].copy_from_slice(&3e-5f32.to_be_bytes());
        let fog = MeshFog::from_mesh_bytes(&d);
        assert_eq!(fog.color, [52428; 3]);
        assert_eq!(fog.density(), 3e-5);
        assert_eq!(LandscapeShading::from_mesh_bytes(&d).fog, fog);
        assert_eq!(MeshFog::from_mesh_bytes(&[]), MeshFog::default());
    }

    use super::*;

    #[test]
    fn media_coverage_toggles_at_row_columns() {
        let mut data = vec![0u8; 1024];
        let words: Vec<i16> = [vec![2, 5, 8], vec![8; 7]].concat();
        data[0xc0..0xc4].copy_from_slice(&0u32.to_be_bytes());
        data[0xc4..0xc8].copy_from_slice(&((words.len() * 2) as u32).to_be_bytes());
        for w in &words {
            data.extend_from_slice(&w.to_be_bytes());
        }
        let c = MediaCoverage::from_mesh_bytes(&data, 1, 1);
        assert_eq!(c.rows.len(), 8);
        let row0: Vec<bool> = (0..8).map(|x| c.pixel_is_media(x, 0)).collect();
        assert_eq!(row0, [false, false, false, true, true, true, false, false]);
        assert!(!c.pixel_is_media(4, 1));
        assert!(!c.pixel_is_media(-1, 0));
        assert!(!c.pixel_is_media(8, 0));
        assert!(c.point_is_media(256, 63));
        assert!(!c.point_is_media(256, 64));
        let empty = MediaCoverage::from_mesh_bytes(&vec![0u8; 1024], 1, 1);
        assert!((0..8).all(|x| !empty.pixel_is_media(x, 0)));
    }

    #[test]
    fn test_mesh_deserializer_dimensions() {
        let mut data = vec![0u8; 1024 + 32 * 32 * 12];
        data[0..4].copy_from_slice(b"00tm");
        data[4..8].copy_from_slice(b"wate");
        data[8] = 0;
        data[9] = 1;
        data[10] = 0;
        data[11] = 1;
        data[12..16].copy_from_slice(&0u32.to_be_bytes());
        data[16..20].copy_from_slice(&(32 * 32 * 12u32).to_be_bytes());

        data[1024..1026].copy_from_slice(&512i16.to_be_bytes());

        let mesh = MeshTag::from_bytes(&data).expect("Should parse mesh tag");
        assert_eq!(mesh.submesh_w, 1);
        assert_eq!(mesh.submesh_h, 1);
        assert_eq!(mesh.cell_width, 32);
        assert_eq!(mesh.cell_height, 32);
        assert_eq!(mesh.total_cells, 1024);
        assert_eq!(mesh.elevation_at(0, 0), Some(512));
    }

    #[test]
    fn test_mesh_markers_and_unit_placements() {
        let cells_bytes = 32 * 32 * 12;
        let pal_offset = cells_bytes;
        let pal_bytes = 32;
        let mkr_offset = pal_offset + pal_bytes;
        let mkr_bytes = 64;
        let total_size = 1024 + mkr_offset + mkr_bytes;

        let mut data = vec![0u8; total_size];
        data[0..4].copy_from_slice(b"00tm");
        data[4..8].copy_from_slice(b"wate");
        data[8..10].copy_from_slice(&1u16.to_be_bytes());
        data[10..12].copy_from_slice(&1u16.to_be_bytes());
        data[12..16].copy_from_slice(&0u32.to_be_bytes());
        data[16..20].copy_from_slice(&(cells_bytes as u32).to_be_bytes());

        data[36..40].copy_from_slice(&1u32.to_be_bytes());
        data[40..44].copy_from_slice(&(pal_offset as u32).to_be_bytes());

        data[52..56].copy_from_slice(&1u32.to_be_bytes());
        data[56..60].copy_from_slice(&(mkr_offset as u32).to_be_bytes());

        let pal_idx = 1024 + pal_offset;
        data[pal_idx..pal_idx + 2].copy_from_slice(&3i16.to_be_bytes());
        data[pal_idx + 2..pal_idx + 4].copy_from_slice(&0i16.to_be_bytes());
        data[pal_idx + 4..pal_idx + 8].copy_from_slice(b"wam1");
        data[pal_idx + 8..pal_idx + 10].copy_from_slice(&1i16.to_be_bytes());

        let mkr_idx = 1024 + mkr_offset;
        data[mkr_idx..mkr_idx + 4].copy_from_slice(&0u32.to_be_bytes());
        data[mkr_idx + 4..mkr_idx + 6].copy_from_slice(&3i16.to_be_bytes());
        data[mkr_idx + 6..mkr_idx + 8].copy_from_slice(&0i16.to_be_bytes());
        data[mkr_idx + 8..mkr_idx + 10].copy_from_slice(&10001i16.to_be_bytes());
        data[mkr_idx + 10..mkr_idx + 12].copy_from_slice(&0i16.to_be_bytes());
        data[mkr_idx + 12..mkr_idx + 16].copy_from_slice(&8192i32.to_be_bytes());
        data[mkr_idx + 16..mkr_idx + 20].copy_from_slice(&8192i32.to_be_bytes());
        data[mkr_idx + 20..mkr_idx + 24].copy_from_slice(&1024i32.to_be_bytes());
        data[mkr_idx + 32..mkr_idx + 34].copy_from_slice(&500i16.to_be_bytes());

        let mesh = MeshTag::from_bytes(&data).expect("Should parse mesh with markers");
        assert_eq!(mesh.palette_count, 1);
        assert_eq!(mesh.marker_count, 1);
        assert_eq!(mesh.palettes.len(), 1);
        assert_eq!(mesh.markers.len(), 1);

        let marker = &mesh.markers[0];
        assert_eq!(marker.marker_type, MarkerPaletteEntry::TYPE_UNIT);
        assert_eq!(marker.normalized_col(mesh.cell_width), 16.0);
        assert_eq!(marker.normalized_row(mesh.cell_height), 16.0);
        assert_eq!(marker.elevation_world(), 2.0);

        let (wx, wy, wz) = mesh.marker_world_position(marker, 1.0);
        assert_eq!(wx, 0.0);
        assert_eq!(wz, 0.0);
        assert_eq!(wy, 2.0);

        let placements = mesh.unit_placements(1.0);
        assert_eq!(placements.len(), 1);
        assert_eq!(placements[0].unit_tag, "wam1");
        assert_eq!(placements[0].team, 1);
        assert_eq!(placements[0].squad, 0);
        assert_eq!(placements[0].position, (0.0, 2.0, 0.0));
        assert_eq!(placements[0].yaw, 500);
    }

    fn corner_table(cx: usize, cy: usize) -> MeshHeightCorner {
        const H: [[i16; 3]; 2] = [[0, 512, 256], [1024, 2048, 512]];
        (H[cy.min(1)][cx.min(2)], 0, 0)
    }

    #[test]
    fn test_get_mesh_height_even_parity_triangles() {
        let h = get_mesh_height(3, 2, 384, 128, MeshHeightMode::Physical, corner_table);
        assert_eq!(h, 512 + ((128 * (0 - 512) + 128 * (2048 - 512)) >> 9));
        assert_eq!(h, 768);
        let h = get_mesh_height(3, 2, 128, 384, MeshHeightMode::Physical, corner_table);
        assert_eq!(h, 1024);
        assert_eq!(
            get_mesh_height(3, 2, 0, 0, MeshHeightMode::Physical, corner_table),
            0
        );
        assert_eq!(
            get_mesh_height(3, 2, 512, 512, MeshHeightMode::Physical, corner_table),
            2048
        );
    }

    #[test]
    fn test_get_mesh_height_odd_parity_triangles() {
        let h = get_mesh_height(3, 2, 512 + 128, 128, MeshHeightMode::Physical, corner_table);
        assert_eq!(h, 512 + ((128 * (256 - 512) + 128 * (2048 - 512)) >> 9));
        assert_eq!(h, 832);
        let h = get_mesh_height(3, 2, 512 + 448, 256, MeshHeightMode::Physical, corner_table);
        assert_eq!(h, 512 + ((64 * (2048 - 512) + 256 * (256 - 512)) >> 9));
        assert_eq!(h, 576);
    }

    #[test]
    fn test_get_mesh_height_floors_negative_slopes() {
        let corners_rev =
            |cx: usize, _cy: usize| -> MeshHeightCorner { (if cx == 0 { 0 } else { 3 }, 0, 0) };
        let h = get_mesh_height(2, 2, 1, 0, MeshHeightMode::Physical, corners_rev);
        assert_eq!(h, 0);
    }

    #[test]
    fn test_get_mesh_height_media_modes_and_void() {
        let corners = |cx: usize, cy: usize| -> MeshHeightCorner {
            let (phys, _, _) = corner_table(cx, cy);
            if (cx, cy) == (1, 1) {
                (phys, 3000, 0x0800)
            } else {
                (phys, 0, 0)
            }
        };
        let surface = get_mesh_height(3, 2, 128, 384, MeshHeightMode::Surface, corners);
        assert_eq!(
            surface,
            1024 + ((128 * (3000 - 1024) + 128 * (0 - 1024)) >> 9)
        );
        assert_eq!(surface, 1262);
        let physical = get_mesh_height(3, 2, 128, 384, MeshHeightMode::Physical, corners);
        assert_eq!(physical, 1024);
        let media = get_mesh_height(3, 2, 128, 384, MeshHeightMode::Media, corners);
        assert_eq!(media, (128 * 3000) >> 9);

        let void = |_: usize, _: usize| -> MeshHeightCorner { (77, 0, 0x4000) };
        assert_eq!(
            get_mesh_height(2, 2, 100, 100, MeshHeightMode::Physical, void),
            MESH_HEIGHT_VOID
        );
    }

    #[test]
    fn test_get_mesh_height_clamps_outside_grid() {
        let flat = |_: usize, _: usize| -> MeshHeightCorner { (640, 0, 0) };
        assert_eq!(
            get_mesh_height(4, 4, -700, 99_999, MeshHeightMode::Physical, flat),
            640
        );
    }

    #[test]
    fn test_mesh_height_matches_rendered_terrain_interpolation() {
        let mut data = vec![0u8; 1024 + 32 * 32 * 12];
        data[0..4].copy_from_slice(b"00tm");
        data[9] = 1;
        data[11] = 1;
        data[16..20].copy_from_slice(&(32 * 32 * 12u32).to_be_bytes());
        let mut seed = 0x1234_5678u32;
        for i in 0..32 * 32 {
            seed = seed.wrapping_mul(1_103_515_245).wrapping_add(12_345);
            let h = ((seed >> 16) % 4096) as i16 - 1024;
            data[1024 + i * 12..1024 + i * 12 + 2].copy_from_slice(&h.to_be_bytes());
        }
        let mesh = MeshTag::from_bytes(&data).expect("mesh");
        for i in 0..2000u32 {
            seed = seed.wrapping_mul(1_103_515_245).wrapping_add(12_345);
            let x = ((seed >> 8) % (31 * 512)) as i32;
            seed = seed.wrapping_mul(1_103_515_245).wrapping_add(12_345);
            let y = ((seed >> 8) % (31 * 512)) as i32;
            let fixed = mesh.mesh_height(x, y, MeshHeightMode::Physical) as f32 / 512.0;
            let float = mesh.terrain_height_at(x as f32 / 512.0, y as f32 / 512.0);
            assert!(
                (fixed - float).abs() <= 1.5 / 512.0,
                "sample {i} ({x},{y}): fixed {fixed} vs float {float}"
            );
        }
    }
}
