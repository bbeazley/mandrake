use crate::media;
use crate::minimap;
use crate::model_instances::ModelInstance;
use crate::ui_scale;
use mandrake_common::FourCC;
use mandrake_tag::{MeshTag, TagArchive};
use mandrake_view::{billboard::BillboardInstance, camera::Camera};
use std::path::Path;
use tracing::warn;

pub(crate) static VIEW_BARS: ViewBars = ViewBars::new();

pub(crate) struct ViewBars {
    heights: std::sync::OnceLock<(i32, i32)>,
    control_bar: std::sync::atomic::AtomicBool,
}

impl ViewBars {
    pub(crate) const fn new() -> Self {
        Self {
            heights: std::sync::OnceLock::new(),
            control_bar: std::sync::atomic::AtomicBool::new(true),
        }
    }

    pub(crate) fn init(&self, open: impl FnOnce() -> (i32, i32), control_bar: bool) {
        self.heights.get_or_init(open);
        self.set_control_bar(control_bar);
    }

    pub(crate) fn set_control_bar(&self, shown: bool) {
        self.control_bar
            .store(shown, std::sync::atomic::Ordering::Relaxed);
    }

    pub(crate) fn get(&self) -> (i32, i32) {
        bars_in_view(
            self.heights.get().copied().unwrap_or((0, 0)),
            self.control_bar.load(std::sync::atomic::Ordering::Relaxed),
        )
    }
}

pub(crate) fn open_view_bars(archive_path: &Path) -> (i32, i32) {
    let vfs = mandrake_common::game_vfs();
    let files = archive_path
        .parent()
        .and_then(Path::parent)
        .map(|dir| mandrake_tag::engine_tag_files(dir, &[]).paths)
        .unwrap_or_default();
    let archives: Vec<TagArchive> = files
        .iter()
        .filter_map(|p| TagArchive::open(vfs, &p.to_string_lossy()).ok())
        .collect();
    let height = |id: FourCC| {
        archives.iter().rev().find_map(|a| {
            let e = a.find_tag_by_id(FourCC::new(*b"bina"), id)?;
            mandrake_tag::dialog::DialogDefinition::from_bytes(&a.get_tag_data(e).ok()?)
                .ok()
                .map(|d| d.height())
        })
    };
    match (
        height(mandrake_tag::hud::STATUS_BAR_ID),
        height(mandrake_tag::hud::CONTROL_BAR_ID),
    ) {
        (Some(top), Some(bottom)) => (top, bottom),
        _ => {
            warn!("Status / control bar dialogs not found; the view region uses the whole window");
            (0, 0)
        }
    }
}

pub(crate) fn bars_in_view((top, bottom): (i32, i32), control_bar: bool) -> (i32, i32) {
    (top, if control_bar { bottom } else { 0 })
}

pub(crate) fn view_bars(ui: &ui_scale::UiScale) -> (i32, i32) {
    ui.view_bars(VIEW_BARS.get())
}

fn engine_region_view(
    camera: &Camera,
    mesh_tag: &MeshTag,
    size: (u32, u32),
    bars: (i32, i32),
) -> mandrake_view::view_region::RegionView {
    let frame =
        mandrake_view::model_surfaces::WorldFrame::new(mesh_tag.cell_width, mesh_tag.cell_height);
    region_view_in(camera, frame, size, bars)
}

pub(crate) fn region_view_in(
    camera: &Camera,
    frame: mandrake_view::model_surfaces::WorldFrame,
    (width, height): (u32, u32),
    (top, bottom): (i32, i32),
) -> mandrake_view::view_region::RegionView {
    let (w, h) = (width as f32, height as f32);
    let focal = (h * 0.5) / (camera.fov_y * 0.5).tan();
    let cy_window = h * 0.5 * (1.0 - camera.principal_offset_y);
    let view_h = height as i32 - top - bottom;
    let conv = |v: [f32; 3]| [-v[0], v[2], v[1]];
    let (r, u, f) = camera.view_axes();
    mandrake_view::view_region::RegionView {
        eye: frame.to_engine(camera.eye_position()),
        axes: [conv(r), conv(u), conv(f).map(|c| -c)],
        extents: [
            (cy_window - top as f32) / focal,
            -(w * 0.5) / focal,
            (cy_window - top as f32 - view_h as f32) / focal,
            (w * 0.5) / focal,
        ],
        focal,
        center: {
            let [cx, cy] = camera.object_projection_center(width, height);
            [cx, cy - top]
        },
        size: [width as i32, view_h],
    }
}

pub(crate) fn engine_mesh_vertex<'a>(
    mesh_tag: &MeshTag,
    map: &'a media::MapMedia,
    local_time: i32,
) -> impl Fn(i32, i32) -> [f32; 3] + 'a {
    let (cw, ch) = (
        i32::from(mesh_tag.cell_width),
        i32::from(mesh_tag.cell_height),
    );
    let index = move |c: i32, r: i32| (r.clamp(0, ch - 1) * cw + c.clamp(0, cw - 1)) as usize;
    let sine = &mandrake_math::engine::trig_tables().sine;
    move |c: i32, r: i32| {
        let cell = map.cells[index(c, r)];
        let (x, y) = (c << 9, r << 9);
        let model = map.cell_models.get(index(c, r)).copied().unwrap_or(-1);
        let mesh_flags = map.surface.mesh_flags;
        let z = mandrake_view::media::mesh_vertex_height(
            cell.physical_height,
            cell.media_height,
            cell.flags,
            model,
            mesh_flags,
        );
        let o = if mandrake_view::media::is_media_vertex(cell.flags, model, mesh_flags) {
            map.surface.wobble.offset(c, r, local_time, sine)
        } else {
            [0; 3]
        };
        [
            (x + o[0]) as f32,
            (y + o[1]) as f32,
            (i32::from(z) + o[2]) as f32,
        ]
    }
}

pub(crate) fn frame_view_parts(
    camera: &Camera,
    mesh_tag: &MeshTag,
    map_media: Option<&media::MapMedia>,
    (width, height): (u32, u32),
    bars: (i32, i32),
    local_time: i32,
) -> Option<(
    mandrake_view::view_region::RegionView,
    mandrake_view::view_region::ViewRegion,
)> {
    let map = map_media?;
    let view = engine_region_view(camera, mesh_tag, (width, height), bars);
    let region = engine_view_region(&view, mesh_tag, map, local_time, true);
    Some((view, region))
}

pub(crate) fn engine_view_region(
    view: &mandrake_view::view_region::RegionView,
    mesh_tag: &MeshTag,
    map: &media::MapMedia,
    local_time: i32,
    dynamic_features: bool,
) -> mandrake_view::view_region::ViewRegion {
    use mandrake_view::view_region::{RegionMap, build_view_region};
    let (cw, ch) = (
        i32::from(mesh_tag.cell_width),
        i32::from(mesh_tag.cell_height),
    );
    let index = |c: i32, r: i32| (r.clamp(0, ch - 1) * cw + c.clamp(0, cw - 1)) as usize;
    let model_of = |c: i32, r: i32| map.cell_models.get(index(c, r)).copied().unwrap_or(-1);
    let mesh_flags = map.surface.mesh_flags;
    let vertex = engine_mesh_vertex(mesh_tag, map, local_time);
    let model_vertex = |c: i32, r: i32| {
        model_of(c, r) != -1
            && mesh_flags & mandrake_view::media::MESH_FLAG_MODEL_CELLS_NOT_MEDIA != 0
    };
    let near_reflective_media = |c: i32, r: i32| {
        model_of(c, r) != -1
            && map
                .reflective_media_distance
                .get(index(c, r))
                .is_some_and(|&d| d <= mandrake_view::view_region::REFLECTIVE_MEDIA_REACH)
    };
    build_view_region(
        view,
        &RegionMap {
            width: cw,
            height: ch,
            min_cell_height: map.min_cell_height,
            small_region: false,
            vertex: &vertex,
            model_vertex: &model_vertex,
            dynamic_features,
            near_reflective_media: &near_reflective_media,
        },
    )
}

pub(crate) fn engine_ground_pick(
    camera: &Camera,
    mesh_tag: &MeshTag,
    map_media: Option<&media::MapMedia>,
    size: (u32, u32),
    bars: (i32, i32),
    local_time: i32,
    cursor: (f32, f32),
) -> Option<(i32, i32)> {
    let map = map_media?;
    let (view, region) = frame_view_parts(camera, mesh_tag, Some(map), size, bars, local_time)?;
    let lod = map.lod.as_ref();
    let point = mandrake_view::mesh_pick::pick_ground(
        &view,
        &region,
        |col, row| {
            mandrake_render::mesh::block_triangles(mesh_tag, lod, col, row)
                .map(|(t, _)| t)
                .collect::<Vec<_>>()
        },
        engine_mesh_vertex(mesh_tag, map, local_time),
        [cursor.0 as i32, cursor.1 as i32 - bars.0],
    )?;
    Some((point[0], point[1]))
}

pub(crate) fn minimap_outline(
    camera: &Camera,
    mesh_tag: &MeshTag,
    map_media: Option<&media::MapMedia>,
    parts: Option<(
        &mandrake_view::view_region::RegionView,
        &mandrake_view::view_region::ViewRegion,
    )>,
) -> minimap::OutlineView {
    let frame =
        mandrake_view::model_surfaces::WorldFrame::new(mesh_tag.cell_width, mesh_tag.cell_height);
    let eye = frame.to_engine(camera.eye_position());
    let target = frame.to_engine(camera.target);
    let mut out = minimap::OutlineView {
        from: (eye[0] as i32, eye[1] as i32),
        to: (target[0] as i32, target[1] as i32),
        polygon: Vec::new(),
    };
    let (Some((view, region)), Some(map)) = (parts, map_media) else {
        return out;
    };
    let cw = i32::from(mesh_tag.cell_width);
    let (mut sum, mut n) = (0i64, 0i64);
    for row in region.first_row..=region.last_row {
        let Some(&(first, last)) = region.spans.get((row - region.first_row) as usize) else {
            continue;
        };
        if first >= last || row < 0 {
            continue;
        }
        for col in i32::from(first)..=i32::from(last) {
            let Some(c) = map.cells.get((row * cw + col) as usize) else {
                continue;
            };
            sum += i64::from(if c.flags & 0x100 != 0 {
                c.media_height
            } else {
                c.physical_height
            });
            n += 1;
        }
    }
    let h = if n == 0 { 0 } else { (sum / n) as i16 as i32 };
    let corners: Vec<[i32; 2]> = (0..4)
        .map(|i| view.ground_point((i & 1) as f32, (i >> 1) as f32, h))
        .collect();
    out.polygon = mandrake_view::view_region::convex_hull(&corners);
    out.from = (view.eye[0] as i32, view.eye[1] as i32);
    let to = view.ground_point(0.5, 0.25, h);
    out.to = (to[0], to[1]);
    out
}

pub(crate) fn object_cell_in_region(
    region: &mandrake_view::view_region::ViewRegion,
    col: i32,
    row: i32,
) -> bool {
    region
        .span(row)
        .is_some_and(|(first, last)| col >= i32::from(first) && col <= i32::from(last))
}

pub(crate) fn retain_region_objects(
    billboards: &mut Vec<BillboardInstance>,
    region: Option<&mandrake_view::view_region::ViewRegion>,
    mesh_tag: &MeshTag,
) {
    use mandrake_view::billboard::{
        BILLBOARD_FLAG_CONNECTOR, BILLBOARD_FLAG_FACING_ARROW, BILLBOARD_FLAG_SCREEN_OVERLAY,
    };
    let Some(region) = region else {
        return;
    };
    let frame =
        mandrake_view::model_surfaces::WorldFrame::new(mesh_tag.cell_width, mesh_tag.cell_height);
    billboards.retain(|b| {
        if b.flags
            & (BILLBOARD_FLAG_CONNECTOR
                | BILLBOARD_FLAG_FACING_ARROW
                | BILLBOARD_FLAG_SCREEN_OVERLAY)
            != 0
        {
            return true;
        }
        point_in_region(region, &frame, b.position)
    });
}

pub(crate) fn point_in_region(
    region: &mandrake_view::view_region::ViewRegion,
    frame: &mandrake_view::model_surfaces::WorldFrame,
    position: [f32; 3],
) -> bool {
    let e = frame.to_engine(position);
    object_cell_in_region(region, e[0].floor() as i32 >> 9, e[1].floor() as i32 >> 9)
}

pub(crate) fn mark_region_models(
    instances: &[ModelInstance],
    region: Option<&mandrake_view::view_region::ViewRegion>,
    map_media: Option<&media::MapMedia>,
    mesh_tag: &MeshTag,
    engine: Option<&mandrake_sim::engine_rng::EngineWorld>,
) {
    let (Some(region), Some(map)) = (region, map_media) else {
        for inst in instances {
            inst.in_region.set(inst.present);
        }
        return;
    };
    let drawn = drawn_placed_models(region, map, mesh_tag);
    let sim_drawn = engine
        .filter(|_| instances.iter().any(|i| i.animation.is_some()))
        .map(|w| sim_models_in_region(region, w));
    for inst in instances {
        let by_frame = inst
            .animation
            .as_ref()
            .filter(|a| a.in_sim)
            .zip(sim_drawn.as_ref())
            .map(|(a, d)| {
                usize::try_from(a.sim_model)
                    .ok()
                    .is_some_and(|m| d.get(m).copied().unwrap_or(false))
            });
        let in_region = by_frame.unwrap_or_else(|| {
            inst.placed
                .is_none_or(|p| drawn.get(p).copied().unwrap_or(true))
        });
        inst.in_region.set(inst.present && in_region);
    }
}

fn sim_models_in_region(
    region: &mandrake_view::view_region::ViewRegion,
    world: &mandrake_sim::engine_rng::EngineWorld,
) -> Vec<bool> {
    let mut drawn = vec![false; world.models.len()];
    for row in region.first_row..region.last_row {
        let Some((first, last)) = region.span(row) else {
            continue;
        };
        for col in i32::from(first)..i32::from(last) {
            if let Some(&m) = world
                .cells
                .models
                .get((row * world.cells.width + col) as usize)
                && let Some(d) = usize::try_from(m).ok().and_then(|m| drawn.get_mut(m))
            {
                *d = true;
            }
        }
    }
    drawn
}

pub(crate) fn drawn_placed_models(
    region: &mandrake_view::view_region::ViewRegion,
    map: &media::MapMedia,
    mesh_tag: &MeshTag,
) -> Vec<bool> {
    let mut drawn = vec![false; map.placed_models.len()];
    let w = i32::from(mesh_tag.cell_width);
    for row in region.first_row..region.last_row {
        let Some((first, last)) = region.span(row) else {
            continue;
        };
        for col in i32::from(first)..i32::from(last) {
            if let Some(&m) = map.cell_models.get((row * w + col) as usize)
                && m >= 0
                && let Some(d) = drawn.get_mut(m as usize)
            {
                *d = true;
            }
        }
    }
    drawn
}

#[cfg(test)]
mod tests {
    use super::*;
    use mandrake_common::FourCC;

    fn mesh() -> MeshTag {
        MeshTag {
            mesh_id: FourCC::new(*b"test"),
            landscape_tag: FourCC::new(*b"test"),
            media_tag: FourCC::new(*b"test"),
            submesh_w: 3,
            submesh_h: 3,
            cell_width: 64,
            cell_height: 64,
            total_cells: 0,
            mesh_offset: 0,
            mesh_size: 0,
            cells: Vec::new(),
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

    #[test]
    fn a_hidden_control_bar_gives_the_view_its_height() {
        assert_eq!(bars_in_view((64, 32), true), (64, 32));
        let bars = bars_in_view((64, 32), false);
        assert_eq!(bars, (64, 0));

        let (mesh, camera) = (mesh(), Camera::new([0.0, 0.0, 0.0], 12.0, 0.7, 0.6));
        let shown = engine_region_view(&camera, &mesh, (1280, 720), (64, 32));
        let hidden = engine_region_view(&camera, &mesh, (1280, 720), bars);
        assert_eq!(shown.size, [1280, 624]);
        assert_eq!(hidden.size, [1280, 656]);
        assert_eq!(hidden.focal, shown.focal);
        assert_eq!(hidden.center, shown.center);
        assert_eq!(hidden.axes, shown.axes);
        assert_eq!(hidden.extents[0], shown.extents[0]);
        assert_eq!(hidden.extents[1], shown.extents[1]);
        assert_eq!(hidden.extents[3], shown.extents[3]);
        assert!((shown.extents[2] - hidden.extents[2] - 32.0 / shown.focal).abs() < 1e-5);
    }
}
