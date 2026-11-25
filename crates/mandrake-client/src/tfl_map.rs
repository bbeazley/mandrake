use mandrake_common::FourCC;
use mandrake_rec::tfl::TflFilmHeader;
use mandrake_tag::TagArchive;
use mandrake_tag::collection::TerrainTexture;
use mandrake_tag::mesh::{LandscapeShading, MeshTag, Myth2Cell};
use mandrake_tag::tfl::TflMesh;
use std::path::{Path, PathBuf};
use tracing::{info, warn};

pub(crate) fn default_tfl_dir() -> PathBuf {
    mandrake_common::data_dirs::tfl_dir()
}

pub(crate) fn last_recording(game_dir: &Path) -> PathBuf {
    game_dir
        .join("tags")
        .join("Local")
        .join("RECO")
        .join("LAST RECORDING")
}

pub(crate) fn resolve_tfl_film(path: &Path, game_dir: &Path) -> PathBuf {
    if path.is_absolute() || path.exists() {
        return path.to_path_buf();
    }
    game_dir.join("tags").join("Local").join("RECO").join(path)
}

pub(crate) struct TflMap {
    pub(crate) tfl_mesh: TflMesh,
    pub(crate) mesh: MeshTag,
    pub(crate) mesh_name: String,
    pub(crate) texture: Option<TerrainTexture>,
    pub(crate) archive: TagArchive,
    pub(crate) film: Option<TflFilmHeader>,
    pub(crate) film_bytes: Option<Vec<u8>>,
}

impl TflMap {
    pub(crate) fn tag(&self, group: &[u8; 4], id: FourCC) -> Option<Vec<u8>> {
        tag_data(&self.archive, group, id)
    }
}

pub(crate) fn tag_data(archive: &TagArchive, group: &[u8; 4], id: FourCC) -> Option<Vec<u8>> {
    let e = archive.find_tag_by_id(FourCC::new(*group), id)?;
    archive.get_tag_data(e).ok()
}

pub(crate) fn with_game_colours(mut header: TflFilmHeader) -> TflFilmHeader {
    mandrake_tfl_sim::set_game_colours(&mut header.players);
    header
}

pub(crate) fn load_tfl_map(
    game_dir: &Path,
    film: Option<&Path>,
    mesh: Option<&str>,
) -> anyhow::Result<TflMap> {
    let archive = mandrake_tag::tag_files::open_tfl_tags(game_dir)
        .map_err(|e| anyhow::anyhow!("TFL tags in {}: {e}", game_dir.display()))?;
    info!(
        "TFL: {} tags from {}",
        archive.tag_count(),
        game_dir.display()
    );
    let film_path = match (film, mesh) {
        (Some(f), _) => Some(resolve_tfl_film(f, game_dir)),
        (None, Some(_)) => None,
        (None, None) => Some(last_recording(game_dir))
            .filter(|p| mandrake_common::game_vfs().is_file(&p.to_string_lossy())),
    };
    let film = film_path
        .map(|p| {
            let bytes = mandrake_common::game_vfs()
                .read_file(&p.to_string_lossy())
                .map_err(|e| anyhow::anyhow!("TFL film {}: {e}", p.display()))?;
            let rec = mandrake_rec::tfl::TflRecording::from_bytes(&bytes)
                .map_err(|e| anyhow::anyhow!("TFL film {}: {e}", p.display()))?;
            info!(
                "TFL film '{}': mesh {}, {} ticks, game type {}, difficulty {}",
                rec.header.name,
                rec.header.mesh_tag_id,
                rec.header.end_tick,
                rec.header.game_type,
                rec.header.difficulty
            );
            anyhow::Ok((with_game_colours(rec.header), bytes))
        })
        .transpose()?;
    let (film, film_bytes) = film.unzip();
    let entry = match (&film, mesh) {
        (Some(f), _) => archive
            .find_tag_by_id(FourCC::new(*b"mesh"), f.mesh_tag_id)
            .ok_or_else(|| {
                anyhow::anyhow!("TFL: the film's mesh {} is not in the tags", f.mesh_tag_id)
            })?,
        (None, Some(q)) => {
            let query = crate::map_load::MeshQuery::from_arg(q);
            crate::map_load::find_mesh_entry(&[&archive], &query)
                .map(|(_, e)| e)
                .ok_or_else(|| anyhow::anyhow!("TFL: no mesh matches '{q}'"))?
        }
        (None, None) => archive
            .find_tags_by_type(FourCC::new(*b"mesh"))
            .into_iter()
            .next()
            .ok_or_else(|| anyhow::anyhow!("TFL: no mesh in the tags"))?,
    };
    let mesh_name = entry.name.clone();
    let mesh_id = entry.tag_id;
    let data = archive
        .get_tag_data(entry)
        .map_err(|e| anyhow::anyhow!("TFL mesh {mesh_id}: {e}"))?;
    let tfl_mesh = TflMesh::from_bytes(&data)?;
    let mesh = renderer_mesh(&tfl_mesh, mesh_id, &data);
    info!(
        "TFL mesh {mesh_id} '{mesh_name}': {}x{} cells, landscape {}, {} markers, {} palette entries",
        mesh.cell_width,
        mesh.cell_height,
        mesh.landscape_tag,
        mesh.markers.len(),
        mesh.palettes.len()
    );
    let texture = tag_data(&archive, b".256", tfl_mesh.landscape_tag).and_then(|d| {
        landscape_texture(&d, &tfl_mesh)
            .map_err(|e| warn!("TFL landscape {}: {e}", tfl_mesh.landscape_tag))
            .ok()
    });
    Ok(TflMap {
        tfl_mesh,
        mesh,
        mesh_name,
        texture,
        archive,
        film,
        film_bytes,
    })
}

pub(crate) fn renderer_mesh(tfl: &TflMesh, mesh_id: FourCC, data: &[u8]) -> MeshTag {
    let cells: Vec<Myth2Cell> = tfl
        .cells
        .iter()
        .map(|c| Myth2Cell {
            physical_height: c.physical_height,
            normal: c.word_2,
            flags: 0,
            first_object_index: c.first_object_index,
            media_height: 0,
            model_index: -1,
        })
        .collect();
    MeshTag {
        mesh_id,
        landscape_tag: tfl.landscape_tag,
        media_tag: tfl.media_tag.unwrap_or(FourCC::new([0xff; 4])),
        submesh_w: tfl.submesh_w,
        submesh_h: tfl.submesh_h,
        cell_width: tfl.cell_width as u16,
        cell_height: tfl.cell_height as u16,
        total_cells: cells.len() as u32,
        mesh_offset: 0,
        mesh_size: (cells.len() * mandrake_tag::tfl::TflCell::SIZE) as u32,
        cells,
        palette_count: tfl.palettes.len() as u32,
        palette_offset: 0,
        marker_count: tfl.markers.len() as u32,
        marker_offset: 0,
        palettes: tfl.palettes.clone(),
        markers: tfl.markers.clone(),
        string_list_tag: tfl.string_list_tag,
        overhead_tag: tfl.overhead_tag,
        pregame_story_tag: tfl.pregame_tag,
        postgame_tag: tfl.postgame_tag,
        next_mesh_id: tfl.next_mesh_tag,
        landscape_shading: tfl_landscape_shading(data),
        reverb: None,
    }
}

fn tfl_landscape_shading(data: &[u8]) -> LandscapeShading {
    let s = LandscapeShading::from_mesh_bytes(data);
    LandscapeShading {
        fog: Default::default(),
        ..s
    }
}

fn landscape_texture(
    data: &[u8],
    mesh: &TflMesh,
) -> Result<TerrainTexture, mandrake_common::M2Error> {
    TerrainTexture::from_dot256(data, mesh.submesh_w as u32, mesh.submesh_h as u32)
}
