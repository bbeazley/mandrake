use crate::media;
use crate::scenery;
use mandrake_common::FourCC;
use mandrake_render::TerrainMesh;
use mandrake_tag::collection::TerrainTexture;
use mandrake_tag::{MeshConnectorList, MeshTag, TagArchive};
use std::path::{Path, PathBuf};
use tracing::{info, warn};

const MODEL_ARCHIVE_NAMES: [&str; 1] = ["medium install"];

pub(crate) static MODEL_ARCHIVES: std::sync::OnceLock<Vec<TagArchive>> = std::sync::OnceLock::new();

pub(crate) static INTERNATIONAL_SMALL: std::sync::OnceLock<Option<TagArchive>> =
    std::sync::OnceLock::new();

pub(crate) fn find_tag_data(
    small_archive: &TagArchive,
    large_archive: Option<&TagArchive>,
    group: [u8; 4],
    id: FourCC,
    large_first: bool,
) -> Option<Vec<u8>> {
    let intl = INTERNATIONAL_SMALL.get().and_then(Option::as_ref);
    let archives = if large_first {
        [large_archive, Some(small_archive), intl]
    } else {
        [Some(small_archive), large_archive, intl]
    };
    archives.into_iter().flatten().find_map(|a| {
        a.find_tag_by_id(FourCC::new(group), id)
            .and_then(|e| a.get_tag_data(e).ok())
    })
}

pub(crate) fn open_model_archives(archive_path: &Path) -> Vec<TagArchive> {
    let vfs = mandrake_common::game_vfs();
    let Some(dir) = archive_path.parent() else {
        return Vec::new();
    };
    let path = dir.join("international small install");
    let _ = INTERNATIONAL_SMALL.set(
        crate::game_files::exists(&path)
            .then(|| TagArchive::open(vfs, &path.to_string_lossy()).ok())
            .flatten(),
    );
    MODEL_ARCHIVE_NAMES
        .iter()
        .map(|name| dir.join(name))
        .filter(|p| crate::game_files::exists(p))
        .filter_map(|p| TagArchive::open(vfs, &p.to_string_lossy()).ok())
        .collect()
}

pub(crate) fn resolve_film_path(path: &Path, archive: &Path) -> PathBuf {
    if path.is_absolute() || crate::game_files::exists(path) {
        return path.to_path_buf();
    }
    match archive.parent().and_then(Path::parent) {
        Some(data) => data.join("recordings").join(path),
        None => path.to_path_buf(),
    }
}

pub(crate) const LOCAL_PLAYER_TEAM: i16 = 0;

pub(crate) const DEFAULT_MESH: &str = "00tm";

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct MeshQuery {
    pub(crate) id: Option<FourCC>,
    name: String,
}

impl MeshQuery {
    pub(crate) fn film(id: FourCC) -> Self {
        Self {
            id: Some(id),
            name: String::new(),
        }
    }

    pub(crate) fn from_arg(query: &str) -> Self {
        let id = (!query.is_empty() && query.len() <= 4).then(|| {
            let mut b = [b' '; 4];
            b[..query.len()].copy_from_slice(query.as_bytes());
            FourCC::new(b)
        });
        Self {
            id,
            name: query.to_lowercase(),
        }
    }
}

impl std::fmt::Display for MeshQuery {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match (&self.id, self.name.as_str()) {
            (Some(id), "") => write!(f, "tag {id}"),
            (_, name) => write!(f, "'{name}'"),
        }
    }
}

pub(crate) fn find_mesh_entry<'a>(
    archives: &[&'a TagArchive],
    query: &MeshQuery,
) -> Option<(&'a TagArchive, &'a mandrake_tag::TagEntry)> {
    let mesh = FourCC::new(*b"mesh");
    let by_id = query.id.and_then(|id| {
        archives
            .iter()
            .find_map(|a| Some((*a, a.find_tag_by_id(mesh, id)?)))
    });
    by_id.or_else(|| {
        if query.name.is_empty() {
            return None;
        }
        archives.iter().find_map(|a| {
            let e = a
                .list_tags()
                .iter()
                .find(|e| e.tag_type == mesh && e.name.to_lowercase().contains(&query.name))?;
            Some((*a, e))
        })
    })
}

fn fallback_mesh_entry(archive: &TagArchive) -> Option<&mandrake_tag::TagEntry> {
    let mesh = FourCC::new(*b"mesh");
    let tags = archive.list_tags();
    tags.iter()
        .find(|e| e.tag_type == mesh && e.layer == 0)
        .or_else(|| tags.iter().find(|e| e.tag_type == mesh))
}

pub(crate) fn mesh_fallback_warning(
    query: &MeshQuery,
    missing_plugins: &[String],
    fallback: &str,
) -> String {
    let plugins = if missing_plugins.is_empty() {
        String::new()
    } else {
        format!(" (plugins not installed: {})", missing_plugins.join(", "))
    };
    format!("Mesh {query} not found in the tag files{plugins}; falling back to mesh '{fallback}'")
}

pub(crate) struct LoadedMap {
    pub(crate) mesh_tag: MeshTag,
    pub(crate) mesh_name: String,
    pub(crate) mesh_data: Vec<u8>,
    pub(crate) terrain_texture: Option<TerrainTexture>,
    pub(crate) archive: TagArchive,
    pub(crate) large_archive: Option<TagArchive>,
    pub(crate) connectors: MeshConnectorList,
    pub(crate) ambient_tag: Option<FourCC>,
}

pub(crate) fn open_archive_role(
    path: &Path,
    archive_path: &Path,
    plugins: &[&str],
) -> anyhow::Result<(TagArchive, Vec<String>)> {
    let vfs = mandrake_common::game_vfs();
    let base = TagArchive::open(vfs, &path.to_string_lossy())
        .map_err(|e| anyhow::anyhow!("Failed to open tag archive '{:?}': {}", path, e))?;
    let (files, missing) = scenery::open_override_tag_files(archive_path, plugins);
    Ok((base.with_overrides(files), missing))
}

pub(crate) fn load_mesh_and_texture(
    archive_path: &Path,
    textures_archive_path: Option<&Path>,
    query: &MeshQuery,
    plugins: &[&str],
) -> anyhow::Result<LoadedMap> {
    let (archive, missing_plugins) = open_archive_role(archive_path, archive_path, plugins)?;
    if !missing_plugins.is_empty() {
        warn!(
            "The game names plugins that are not installed: {}",
            missing_plugins.join(", ")
        );
    }

    let candidate_path = textures_archive_path.map(Path::to_path_buf).or_else(|| {
        archive_path
            .parent()
            .map(|p| p.join("large install"))
            .filter(|p| crate::game_files::exists(p))
    });
    let large_archive = candidate_path.as_deref().and_then(|p| {
        open_archive_role(p, archive_path, plugins)
            .ok()
            .map(|(a, _)| a)
    });

    let search: Vec<&TagArchive> = std::iter::once(&archive)
        .chain(MODEL_ARCHIVES.get().into_iter().flatten())
        .chain(large_archive.as_ref())
        .collect();
    let (source, entry) = match find_mesh_entry(&search, query) {
        Some(found) => found,
        None => {
            let entry = fallback_mesh_entry(&archive)
                .ok_or_else(|| anyhow::anyhow!("No mesh tag found matching {query}"))?;
            warn!(
                "{}",
                mesh_fallback_warning(query, &missing_plugins, &entry.name)
            );
            (&archive, entry)
        }
    };

    let name = entry.name.clone();
    let tag_id = entry.tag_id;
    let data = source
        .get_tag_data(entry)
        .map_err(|e| anyhow::anyhow!("Failed to extract mesh tag data: {}", e))?;
    let mut mesh_tag = MeshTag::from_bytes(&data)
        .map_err(|e| anyhow::anyhow!("Failed to deserialize mesh tag: {}", e))?;
    mesh_tag.mesh_id = tag_id;
    let ambient_tag = MeshTag::ambient_sound_tag_from_bytes(&data);
    let connectors = MeshConnectorList::from_mesh_bytes(&data).unwrap_or_else(|e| {
        warn!("Failed to parse mesh connector list: {}", e);
        MeshConnectorList::default()
    });
    info!(
        "Mesh connector list: tag {:?}, {} connector chains",
        connectors.connector_tag,
        connectors.connectors.len()
    );

    let landscape_tag = mesh_tag.landscape_tag;
    let terrain_texture = search.iter().find_map(|a| {
        extract_landscape_texture(a, landscape_tag, mesh_tag.submesh_w, mesh_tag.submesh_h)
    });
    if let Some(tex) = &terrain_texture {
        info!(
            "Successfully loaded landscape texture ({}x{} px)",
            tex.width, tex.height
        );
    }

    Ok(LoadedMap {
        mesh_tag,
        mesh_name: name,
        mesh_data: data,
        terrain_texture,
        archive,
        large_archive,
        connectors,
        ambient_tag,
    })
}

fn extract_landscape_texture(
    archive: &TagArchive,
    landscape_tag: FourCC,
    submesh_w: u16,
    submesh_h: u16,
) -> Option<TerrainTexture> {
    let tex_entry = archive
        .list_tags()
        .iter()
        .find(|e| e.tag_type == FourCC::new(*b".256") && e.tag_id == landscape_tag)?;
    let tex_data = archive.get_tag_data(tex_entry).ok()?;
    match TerrainTexture::from_dot256(&tex_data, submesh_w as u32, submesh_h as u32) {
        Ok(tex) => Some(tex),
        Err(e) => {
            warn!("Failed to decode landscape texture: {}", e);
            None
        }
    }
}

const TYPE_ENHANCEMENT: i16 = 6;

pub(crate) fn enhancement_packs(archive_path: &Path, names: &[String]) -> Vec<(String, PathBuf)> {
    let Some(dir) = archive_path
        .parent()
        .and_then(Path::parent)
        .map(|g| g.join("plugins"))
    else {
        return Vec::new();
    };
    let mut packs: Vec<(PathBuf, String)> = crate::game_files::list_dir(&dir)
        .into_iter()
        .filter(|p| crate::game_files::is_file(p))
        .filter_map(|p| {
            let h =
                mandrake_tag::header::SBMonoHeader::read(&mut crate::game_files::open(&p).ok()?)
                    .ok()?;
            let name = mandrake_tag::tag_files::plugin_name(&p)?;
            (h.r#type == TYPE_ENHANCEMENT).then_some((p, name))
        })
        .collect();
    packs.sort();
    names
        .iter()
        .filter_map(|n| {
            packs
                .iter()
                .find(|(_, name)| name.eq_ignore_ascii_case(n))
                .map(|(p, _)| (n.clone(), p.clone()))
        })
        .collect()
}

pub(crate) fn load_landscape_detail(
    archives: &[&TagArchive],
    mesh: &MeshTag,
) -> Option<std::sync::Arc<mandrake_render::detail::LandscapeDetail>> {
    let id = mesh.landscape_tag;
    let tag = |group: &[u8; 4], id: FourCC| {
        archives.iter().find_map(|a| {
            a.find_tag_by_id(FourCC::new(*group), id)
                .and_then(|e| a.get_tag_data(e).ok())
        })
    };
    let Some(data) = tag(b"dmap", id) else {
        info!("Detail textures: no dmap {id} for this landscape; drawing without detail");
        return None;
    };
    let map = match mandrake_tag::DetailMap::from_bytes(&data) {
        Ok(map) => map,
        Err(e) => {
            warn!("Detail textures: dmap {id}: {e}; drawing without detail");
            return None;
        }
    };
    if !map.matches_mesh(mesh.submesh_w, mesh.submesh_h) {
        warn!(
            "Warning: Detail map not applied because its dimensions do not match mesh \
             (dmap {id} {}x{}, mesh {}x{} cells)",
            map.width,
            map.height,
            mesh.submesh_w as u32 * 32,
            mesh.submesh_h as u32 * 32
        );
        return None;
    }
    let detail = mandrake_render::detail::LandscapeDetail::new(&map, |tex| {
        let data = tag(b"dtex", tex)?;
        match mandrake_tag::DetailTexture::from_bytes(&data).and_then(|t| t.decode()) {
            Ok(img) => Some(img),
            Err(e) => {
                warn!("Detail textures: dtex {tex}: {e}");
                None
            }
        }
    });
    match detail {
        Some(detail) => {
            info!(
                "Detail textures: dmap {id}, {} textures",
                detail.textures.len()
            );
            Some(std::sync::Arc::new(detail))
        }
        None => {
            warn!("Warning: Detail map not applied because some textures could not be found.");
            None
        }
    }
}

pub(crate) fn build_terrain_mesh(
    device: &wgpu::Device,
    mesh_tag: &MeshTag,
    cell_size: f32,
    map: Option<&media::MapMedia>,
    detail: Option<&mandrake_render::detail::LandscapeDetail>,
    lod: Option<&mandrake_tag::mesh::MeshLod>,
) -> TerrainMesh {
    let mut mesh = std::borrow::Cow::Borrowed(mesh_tag);
    if let Some(map) = map
        && map.surface.mesh_flags & mandrake_view::media::MESH_FLAG_MODEL_CELLS_NOT_MEDIA != 0
    {
        for (i, cell) in mesh.to_mut().cells.iter_mut().enumerate() {
            cell.model_index = map.cell_models.get(i).copied().unwrap_or(-1);
        }
    }
    TerrainMesh::from_mesh_tag_detailed(
        device,
        &mesh,
        cell_size,
        map.map(|m| m.surface),
        lod.or(map.and_then(|m| m.lod.as_ref())),
        detail,
    )
}
