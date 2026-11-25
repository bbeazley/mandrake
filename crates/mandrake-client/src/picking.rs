use std::cell::OnceCell;

use crate::media;
use crate::object_sprites::DrawnObject;
use crate::region_view::{engine_mesh_vertex, frame_view_parts};
use crate::selection_input;
use crate::sprite_manager::ClientSpriteManager;
use crate::unit_anim::UnitAnimator;
use crate::unit_billboards::drawn_monster_sprite;
use crate::world_view::WorldView;
use crate::world_view::pick::Myth2PickSim;
use mandrake_tag::MeshTag;
use mandrake_tag::model_hit::ModelPlacement;
use mandrake_view::model_surfaces::EngineGeometry;
use mandrake_view::{
    billboard::{BILLBOARD_FLAG_SCREEN_ALIGNED, BillboardInstance},
    camera::Camera,
    selection,
};

pub(crate) fn unproject_cursor(
    cursor_x: f32,
    cursor_y: f32,
    width: u32,
    height: u32,
    camera: &Camera,
) -> ([f32; 3], [f32; 3]) {
    camera.screen_ray(cursor_x, cursor_y, width, height)
}

pub(crate) fn ray_terrain_intersection_scaled(
    ray_origin: [f32; 3],
    ray_dir: [f32; 3],
    mesh_tag: &MeshTag,
    cell_size: f32,
) -> Option<[f32; 3]> {
    let half_w = mesh_tag.cell_width as f32 * cell_size * 0.5;
    let half_d = mesh_tag.cell_height as f32 * cell_size * 0.5;
    let max_c = mesh_tag.cell_width as f32;
    let max_r = mesh_tag.cell_height as f32;
    let step = 0.25f32;
    let mut prev_above = true;
    for k in 1..4000 {
        let t = k as f32 * step;
        let p = [
            ray_origin[0] + ray_dir[0] * t,
            ray_origin[1] + ray_dir[1] * t,
            ray_origin[2] + ray_dir[2] * t,
        ];
        let cx = (half_w - p[0]) / cell_size;
        let cy = (p[2] + half_d) / cell_size;
        let h = mesh_tag.terrain_height_at(cx, cy);
        let above = p[1] > h;
        if prev_above && !above {
            return (cx >= 0.0 && cy >= 0.0 && cx < max_c && cy < max_r).then_some([p[0], h, p[2]]);
        }
        prev_above = above;
    }
    None
}

pub(crate) fn ray_terrain_intersection(
    ray_origin: [f32; 3],
    ray_dir: [f32; 3],
    mesh_tag: &MeshTag,
) -> Option<[f32; 3]> {
    if ray_dir[1].abs() < 1e-4 {
        return None;
    }
    ray_terrain_intersection_scaled(ray_origin, ray_dir, mesh_tag, 1.0)
}

fn monster_visible(
    sim: &dyn Myth2PickSim,
    animator: Option<&UnitAnimator>,
    margin: i32,
    id: u32,
) -> bool {
    match sim.under_media(id, margin) {
        None => true,
        Some(under) => {
            !under
                && animator
                    .and_then(|a| a.visibility(id))
                    .is_none_or(|v| v.drawn)
        }
    }
}

fn media_margin(media: Option<&media::MapMedia>) -> i32 {
    media.map_or(0, |m| i32::from(m.ripple_amplitude))
}

pub(crate) const PRESS_PICK_SLOP: i32 = 15;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct PointHit {
    id: u32,
    near_miss: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PickSurface {
    Monster { id: u32, outside: i32 },
    Mesh { point: [i32; 2], cell_flags: u16 },
    Model { point: [i32; 3] },
    Object { index: i32 },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Picked {
    Monster(PointHit),
    Mesh { cell_flags: u16 },
    Model,
    Nothing,
}

fn pick(list: &[PickSurface], at: &mut usize) -> Picked {
    let mut best: Option<(i32, u32)> = None;
    let near = |(_, id): (i32, u32)| {
        Picked::Monster(PointHit {
            id,
            near_miss: true,
        })
    };
    while let Some(&surface) = list.get(*at) {
        match surface {
            PickSurface::Monster { id, outside } => {
                *at += 1;
                if outside <= 0 {
                    return Picked::Monster(PointHit {
                        id,
                        near_miss: false,
                    });
                }
                if best.is_none_or(|(d, _)| outside < d) {
                    best = Some((outside, id));
                }
            }
            PickSurface::Object { .. } => *at += 1,
            polygon => {
                if let Some(b) = best {
                    return near(b);
                }
                *at += 1;
                return match polygon {
                    PickSurface::Mesh { cell_flags, .. } => Picked::Mesh { cell_flags },
                    _ => Picked::Model,
                };
            }
        }
    }
    best.map_or(Picked::Nothing, near)
}

fn press_pick(list: &[PickSurface], visible: impl Fn(u32) -> bool) -> Option<PointHit> {
    let mut at = 0;
    loop {
        match pick(list, &mut at) {
            Picked::Monster(hit) if visible(hit.id) => return Some(hit),
            Picked::Monster(_) => {}
            Picked::Mesh { cell_flags } if cell_flags & media::CELL_FLAG_MEDIA != 0 => loop {
                match pick(list, &mut at) {
                    Picked::Nothing => return None,
                    Picked::Monster(hit) if visible(hit.id) => return Some(hit),
                    Picked::Monster(_) => break,
                    Picked::Mesh { .. } | Picked::Model => {}
                }
            },
            Picked::Mesh { .. } | Picked::Model | Picked::Nothing => return None,
        }
    }
}

fn second_pick(list: &[PickSurface]) -> selection_input::PressTarget {
    use selection_input::PressTarget;
    list.iter()
        .find_map(|s| match *s {
            PickSurface::Mesh { point, .. } => Some(PressTarget::Ground {
                point: (point[0], point[1]),
            }),
            PickSurface::Model { point } => Some(PressTarget::Model {
                point: (point[0], point[1], point[2]),
            }),
            PickSurface::Monster { .. } | PickSurface::Object { .. } => None,
        })
        .unwrap_or(PressTarget::Nothing)
}

fn pick_up_object<T>(
    list: &[PickSurface],
    mut could_pick_up: impl FnMut(i32) -> Option<T>,
) -> Option<(i32, T)> {
    for s in list {
        match *s {
            PickSurface::Object { index } => {
                if let Some(t) = could_pick_up(index) {
                    return Some((index, t));
                }
            }
            PickSurface::Monster { .. } => {}
            PickSurface::Mesh { .. } | PickSurface::Model { .. } => return None,
        }
    }
    None
}

fn polygon_point(list: &[PickSurface]) -> Option<[i32; 2]> {
    list.iter().find_map(|s| match *s {
        PickSurface::Mesh { point, .. } => Some(point),
        PickSurface::Model { point } => Some([point[0], point[1]]),
        PickSurface::Monster { .. } | PickSurface::Object { .. } => None,
    })
}

struct InReach {
    origin: [f32; 3],
    depth: f32,
    id: u32,
    outside: i32,
}

fn monsters_in_reach(
    world: &WorldView,
    camera: &Camera,
    viewport: (u32, u32),
    cursor: (f32, f32),
    sprite_mgr: &ClientSpriteManager,
    slop: i32,
) -> Vec<InReach> {
    let (px, py) = (cursor.0 as i32, cursor.1 as i32);
    let eye = camera.eye_position();
    let forward = camera.view_axes().2;
    let mut in_reach: Vec<InReach> = world
        .monsters
        .iter()
        .filter_map(|o| {
            let id = o.monster?.id;
            let f = monster_frame_rect(o, camera, viewport)?;
            let outside = (f.left - px)
                .max(px - f.right)
                .max(f.top - py)
                .max(py - f.bottom);
            if outside > slop {
                return None;
            }
            let sprite = drawn_monster_sprite(o, world, camera, sprite_mgr)?;
            let [l, t, r, b] = sprite_screen_rect(&sprite, camera, viewport)?.map(|v| v as i32);
            if px < l - slop || px > r + slop || py < t - slop || py > b + slop {
                return None;
            }
            let p = sprite.position;
            let depth = (p[0] - eye[0]) * forward[0]
                + (p[1] - eye[1]) * forward[1]
                + (p[2] - eye[2]) * forward[2];
            Some(InReach {
                origin: p,
                depth,
                id,
                outside,
            })
        })
        .collect();
    in_reach.sort_by(|a, b| a.depth.total_cmp(&b.depth).then(a.id.cmp(&b.id)));
    in_reach
}

fn polygons_under(
    sim: &dyn Myth2PickSim,
    view: &mandrake_view::view_region::RegionView,
    region: &mandrake_view::view_region::ViewRegion,
    mesh_tag: &MeshTag,
    map: &media::MapMedia,
    local_time: i32,
    p: [i32; 2],
) -> Vec<(u32, PickSurface)> {
    use mandrake_view::mesh_pick::{landscape_surfaces, triangle_contains, triangle_point};
    let lod = map.lod.as_ref();
    let (cw, ch) = (
        i32::from(mesh_tag.cell_width),
        i32::from(mesh_tag.cell_height),
    );
    let mut under: Vec<(u32, PickSurface)> = landscape_surfaces(
        view,
        region,
        |col, row| {
            mandrake_render::mesh::block_triangles(mesh_tag, lod, col, row)
                .map(|(t, _)| t)
                .collect::<Vec<_>>()
        },
        engine_mesh_vertex(mesh_tag, map, local_time),
    )
    .iter()
    .filter(|s| triangle_contains(&s.corners, p))
    .map(|s| {
        let point = triangle_point(&s.corners, p);
        let i = ((point[1] >> 9).clamp(0, ch - 1) * cw + (point[0] >> 9).clamp(0, cw - 1)) as usize;
        let cell_flags = map.cells.get(i).map_or(0, |c| c.flags);
        (s.key, PickSurface::Mesh { point, cell_flags })
    })
    .collect();
    for (geometry, placement) in sim.drawn_models(region, map, mesh_tag).into_iter().rev() {
        model_polygons_under(view, region, geometry, placement, p, &mut under);
    }
    under
}

fn model_polygons_under(
    view: &mandrake_view::view_region::RegionView,
    region: &mandrake_view::view_region::ViewRegion,
    geometry: &EngineGeometry,
    placement: &ModelPlacement,
    p: [i32; 2],
    under: &mut Vec<(u32, PickSurface)>,
) {
    use mandrake_tag::model::GEOMETRY_SURFACE_TWO_SIDED;
    use mandrake_tag::model_hit::{
        transform_geometry_point_to_world, transform_world_point_to_geometry,
    };
    use mandrake_view::mesh_pick::{PickVertex, triangle_contains};
    use mandrake_view::model_surfaces::{apply_draw_after, surface_depth};
    let corners: Vec<([i32; 3], [f32; 3])> = geometry
        .vertices
        .iter()
        .map(|&v| {
            let w = transform_geometry_point_to_world(
                placement.position,
                geometry.origin,
                v,
                &placement.matrix,
            );
            let w = w.map(|c| c as f32);
            (view.view_coords(w), w)
        })
        .collect();
    let eye = transform_world_point_to_geometry(
        placement.position,
        geometry.origin,
        view.eye.map(|v| v as i32),
        &placement.inverse,
    )
    .map(f32::from);
    let flagged = |v: u16| {
        let (c, w) = corners[usize::from(v)];
        c[2] < -0x22 && region.vertex_inside(w[0] as i32 >> 9, w[1] as i32 >> 9)
    };
    let mut depths: Vec<Option<f32>> = geometry
        .surfaces
        .iter()
        .map(|s| {
            if !s.has_material || !s.faces(eye) || !s.vertices.iter().any(|&v| flagged(v)) {
                return None;
            }
            let d = s.vertices.map(|v| corners[usize::from(v)].0[2] as f32);
            Some(surface_depth(s.flags, d)).filter(|&d| d < 0.0)
        })
        .collect();
    apply_draw_after(&geometry.surfaces, &mut depths);
    for (s, depth) in geometry.surfaces.iter().zip(&depths).rev() {
        let Some(depth) = *depth else {
            continue;
        };
        let back = s.flags & GEOMETRY_SURFACE_TWO_SIDED != 0 && s.plane_side(eye) <= 0.0;
        let order = if back { [2, 1, 0] } else { [0, 1, 2] };
        let at = order.map(|k| corners[usize::from(s.vertices[k])].1);
        let screen = at.map(|w| view.screen_point(w));
        let [Some(a), Some(b), Some(c)] = screen else {
            continue;
        };
        let clip = at.map(|w| view.clip_word(w, 0, false));
        let area = (b[0] - a[0]) * (c[1] - a[1]) + (c[0] - a[0]) * (a[1] - b[1]);
        if clip[0] & clip[1] & clip[2] & 0x55 != 0 || area < 1 {
            continue;
        }
        let t = [a, b, c].map(|screen| PickVertex {
            cell: [0; 2],
            screen,
        });
        if triangle_contains(&t, p) {
            let v = at.map(|w| view.view_coords(w));
            under.push((
                ((depth as i32) << 14) as u32,
                PickSurface::Model {
                    point: model_polygon_point(view, [a, b, c], v, p),
                },
            ));
        }
    }
}

fn model_polygon_point(
    view: &mandrake_view::view_region::RegionView,
    s: [[i32; 2]; 3],
    v: [[i32; 3]; 3],
    p: [i32; 2],
) -> [i32; 3] {
    let (bx, by) = (s[1][0] - s[0][0], s[1][1] - s[0][1]);
    let (cx, cy) = (s[2][0] - s[0][0], s[2][1] - s[0][1]);
    let d = (cx * by - bx * cy) as f32;
    let (dx, dy) = ((p[0] - s[0][0]) as f32, (p[1] - s[0][1]) as f32);
    let in_view = [0, 1, 2].map(|k| {
        let (b, c) = (v[1][k] - v[0][k], v[2][k] - v[0][k]);
        let t = ((by * c - cy * b) as f32 / d) * dx + v[0][k] as f32;
        (((b * cx - c * bx) as f32 / d) * dy + t) as i32
    });
    [0, 1, 2].map(|k| {
        let along = |r: usize| view.axes[r][k] * in_view[r] as f32;
        (along(0) + along(1) + along(2)) as i32 + view.eye[k] as i32
    })
}

pub(crate) fn hover_pick(
    sim: &dyn Myth2PickSim,
    camera: &Camera,
    viewport: (u32, u32),
    cursor: (f32, f32),
    sprites: Option<(&ClientSpriteManager, &UnitAnimator)>,
) -> Option<selection::state::HoverPick> {
    let (sprite_mgr, animator) = sprites?;
    let world = sim.drawn_units(animator, sprite_mgr);
    let in_reach = monsters_in_reach(&world, camera, viewport, cursor, sprite_mgr, 0);
    let id = in_reach.first()?.id;
    Some(selection::state::HoverPick {
        id,
        selected: world.unit(id)?.selected,
        monster: true,
    })
}

pub(crate) fn monster_frame_rect(
    o: &DrawnObject,
    camera: &Camera,
    viewport: (u32, u32),
) -> Option<selection::PixelRect> {
    let bounds = o.monster?.bounds;
    let proj = camera.frame_projection(o.position, viewport.0, viewport.1)?;
    Some(selection::projected_frame_rect(bounds, &proj))
}

pub(crate) fn sprite_window_rect(
    sprite: &BillboardInstance,
    camera: &Camera,
    viewport: (u32, u32),
) -> [f32; 4] {
    let (w, h) = (viewport.0 as f32, viewport.1 as f32);
    let eye = camera.eye_position();
    let (right, up, forward) = camera.view_axes();
    let vp = camera.view_proj_matrix(w / h);
    let sprite = sprite.in_engine_view(&mandrake_view::billboard::EngineSpriteView::of(camera));
    let window = |corner: [f32; 2]| {
        let (q, _) = sprite.screen_aligned_corner(eye, right, up, forward, corner);
        let ndc = vp.transform_point3(q);
        [(ndc[0] + 1.0) * 0.5 * w, (1.0 - ndc[1]) * 0.5 * h]
    };
    let [l, b] = window([0.0, 0.0]);
    let [r, t] = window([1.0, 1.0]);
    [l, r, t, b]
}

#[derive(Clone, Copy)]
pub(crate) struct PickScene<'a> {
    pub(crate) camera: &'a Camera,
    pub(crate) viewport: (u32, u32),
    pub(crate) mesh_tag: &'a MeshTag,
}

pub(crate) enum GamePicks<'a> {
    Myth2(Myth2Picks<'a>),
    Tfl(crate::tfl_input::TflPicks<'a>),
}

pub(crate) struct PickWorld<'a> {
    pub(crate) scene: PickScene<'a>,
    pub(crate) selection: Vec<u32>,
    pub(crate) game: GamePicks<'a>,
}

impl<'a> PickWorld<'a> {
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn myth2(
        sim: &'a dyn Myth2PickSim,
        mesh_tag: &'a MeshTag,
        camera: &'a Camera,
        viewport: (u32, u32),
        sprites: Option<(&'a ClientSpriteManager, &'a UnitAnimator)>,
        selection: Vec<u32>,
        map_media: Option<&'a media::MapMedia>,
        (bars, local_time): ((i32, i32), i32),
    ) -> Self {
        Self {
            scene: PickScene {
                camera,
                viewport,
                mesh_tag,
            },
            selection,
            game: GamePicks::Myth2(Myth2Picks {
                sim,
                sprites,
                map_media,
                bars,
                local_time,
                units: OnceCell::new(),
            }),
        }
    }
}

pub(crate) struct Myth2Picks<'a> {
    sim: &'a dyn Myth2PickSim,
    sprites: Option<(&'a ClientSpriteManager, &'a UnitAnimator)>,
    map_media: Option<&'a media::MapMedia>,
    bars: (i32, i32),
    local_time: i32,
    units: OnceCell<Box<WorldView>>,
}

impl Myth2Picks<'_> {
    fn drawn(&self) -> Option<(&WorldView, &ClientSpriteManager)> {
        let (sprite_mgr, animator) = self.sprites?;
        let world = self
            .units
            .get_or_init(|| Box::new(self.sim.drawn_units(animator, sprite_mgr)));
        Some((world, sprite_mgr))
    }

    fn polygons(
        &self,
        scene: &PickScene,
        cursor: (i16, i16),
    ) -> Option<(
        mandrake_view::view_region::RegionView,
        Vec<(u32, PickSurface)>,
    )> {
        let map = self.map_media?;
        let (view, region) = frame_view_parts(
            scene.camera,
            scene.mesh_tag,
            Some(map),
            scene.viewport,
            self.bars,
            self.local_time,
        )?;
        let p = mandrake_view::mesh_pick::clamp_cursor(
            &view,
            [i32::from(cursor.0), i32::from(cursor.1) - self.bars.0],
        );
        let under = polygons_under(
            self.sim,
            &view,
            &region,
            scene.mesh_tag,
            map,
            self.local_time,
            p,
        );
        Some((view, under))
    }

    fn ground_point(&self, scene: &PickScene, cursor: (i16, i16)) -> Option<(i32, i32)> {
        if self.map_media.is_some() {
            let (_, mut under) = self.polygons(scene, cursor)?;
            sort_for_walk(&mut under);
            let list: Vec<PickSurface> = under.into_iter().map(|(_, s)| s).collect();
            return polygon_point(&list).map(|[x, y]| (x, y));
        }
        let (x, y) = (f32::from(cursor.0), f32::from(cursor.1));
        let (ray_orig, ray_dir) =
            unproject_cursor(x, y, scene.viewport.0, scene.viewport.1, scene.camera);
        let hit = ray_terrain_intersection(ray_orig, ray_dir, scene.mesh_tag)?;
        Some(self.sim.command_point(hit))
    }

    fn press_list(&self, scene: &PickScene, cursor: (i16, i16)) -> Vec<PickSurface> {
        let in_reach = self.drawn().map_or_else(Vec::new, |(world, sprite_mgr)| {
            monsters_in_reach(
                world,
                scene.camera,
                scene.viewport,
                (f32::from(cursor.0), f32::from(cursor.1)),
                sprite_mgr,
                PRESS_PICK_SLOP,
            )
        });
        let Some((view, mut list)) = self.polygons(scene, cursor) else {
            return in_reach
                .iter()
                .map(|m| PickSurface::Monster {
                    id: m.id,
                    outside: m.outside,
                })
                .collect();
        };
        let frame = mandrake_view::model_surfaces::WorldFrame::new(
            scene.mesh_tag.cell_width,
            scene.mesh_tag.cell_height,
        );
        list.extend(in_reach.iter().map(|m| {
            let depth = view.view_coords(frame.to_engine(m.origin))[2];
            (
                ((depth << 14) + 0x2000) as u32,
                PickSurface::Monster {
                    id: m.id,
                    outside: m.outside,
                },
            )
        }));
        sort_for_walk(&mut list);
        list.into_iter().map(|(_, s)| s).collect()
    }

    fn objects_in_reach(
        &self,
        scene: &PickScene,
        view: &mandrake_view::view_region::RegionView,
        cursor: (i16, i16),
    ) -> Vec<(u32, PickSurface)> {
        let Some((sprite_mgr, animator)) = self.sprites else {
            return Vec::new();
        };
        let (px, py) = (i32::from(cursor.0), i32::from(cursor.1));
        let frame = mandrake_view::model_surfaces::WorldFrame::new(
            scene.mesh_tag.cell_width,
            scene.mesh_tag.cell_height,
        );
        let world = self
            .sim
            .drawn_objects(animator.fraction(), sprite_mgr.projfx_log.as_ref());
        crate::unit_billboards::drawn_object_sprites(&world, scene.camera, animator, sprite_mgr)
            .into_iter()
            .filter_map(|(index, sprite)| {
                let [l, t, r, b] =
                    sprite_screen_rect(&sprite, scene.camera, scene.viewport)?.map(|v| v as i32);
                if px < l || px > r || py < t || py > b {
                    return None;
                }
                let depth = view.view_coords(frame.to_engine(sprite.position))[2];
                Some((
                    ((depth << 14) + 0x2000) as u32,
                    PickSurface::Object { index },
                ))
            })
            .collect()
    }

    fn press_target(
        &self,
        scene: &PickScene,
        selection: &[u32],
        cursor: (i16, i16),
    ) -> selection_input::PressTarget {
        use selection_input::PressTarget;
        let margin = media_margin(self.map_media);
        let animator = self.sprites.map(|s| s.1);
        let list = self.press_list(scene, cursor);
        let hit = press_pick(&list, |id| monster_visible(self.sim, animator, margin, id));
        if let Some(PointHit { id, near_miss }) = hit {
            let pick = self.sim.monster_pick(id, selection.contains(&id));
            return PressTarget::Unit(selection_input::MonsterPick { near_miss, ..pick });
        }
        if self.map_media.is_some() {
            return second_pick(&list);
        }
        self.ground_point(scene, cursor)
            .map_or(PressTarget::Nothing, |point| PressTarget::Ground { point })
    }

    fn pick_up_target(
        &self,
        scene: &PickScene,
        cursor: (i16, i16),
        selection: &[u32],
    ) -> Option<(i16, i16)> {
        let (view, mut list) = self.polygons(scene, cursor)?;
        let objects = self.objects_in_reach(scene, &view, cursor);
        if objects.is_empty() {
            return None;
        }
        list.extend(objects);
        sort_for_walk(&mut list);
        let list: Vec<PickSurface> = list.into_iter().map(|(_, s)| s).collect();
        let (index, identifier) = pick_up_object(&list, |i| self.sim.could_pick_up(selection, i))?;
        Some((i16::try_from(index).ok()?, identifier))
    }

    fn polygon_target(
        &self,
        scene: &PickScene,
        cursor: (i16, i16),
    ) -> selection_input::PressTarget {
        use selection_input::PressTarget;
        if self.map_media.is_none() {
            return self
                .ground_point(scene, cursor)
                .map_or(PressTarget::Nothing, |point| PressTarget::Ground { point });
        }
        let Some((_, mut under)) = self.polygons(scene, cursor) else {
            return PressTarget::Nothing;
        };
        sort_for_walk(&mut under);
        let list: Vec<PickSurface> = under.into_iter().map(|(_, s)| s).collect();
        second_pick(&list)
    }

    fn monster_visible(&self, id: u32) -> bool {
        let margin = media_margin(self.map_media);
        monster_visible(self.sim, self.sprites.map(|(_, anim)| anim), margin, id)
    }

    fn band_units(&self, scene: &PickScene, rect: selection::PixelRect) -> Vec<u32> {
        let Some((world, sprite_mgr)) = self.drawn() else {
            return Vec::new();
        };
        units_in_band(
            world,
            self.sim,
            scene.camera,
            sprite_mgr,
            scene.viewport,
            rect,
        )
    }

    fn gesture_point(&self, scene: &PickScene, cursor: (i16, i16)) -> Option<(i32, i32)> {
        let (top, bottom) = self.bars;
        let (w, h) = (scene.viewport.0 as i32, scene.viewport.1 as i32);
        let x = i32::from(cursor.0).clamp(0, (w - 1).max(0));
        let y = i32::from(cursor.1).clamp(top, (h - bottom - 1).max(top));
        self.ground_point(scene, (x as i16, y as i16))
    }
}

fn sort_for_walk(list: &mut [(u32, PickSurface)]) {
    list.sort_by_key(|&(key, _)| std::cmp::Reverse(key));
}

impl selection_input::SelectionWorld for PickWorld<'_> {
    fn press_target(&mut self, cursor: (i16, i16)) -> selection_input::PressTarget {
        match &self.game {
            GamePicks::Myth2(g) => g.press_target(&self.scene, &self.selection, cursor),
            GamePicks::Tfl(g) => g.press_target(&self.scene, cursor),
        }
    }

    fn pick_up_target(&mut self, cursor: (i16, i16), selection: &[u32]) -> Option<(i16, i16)> {
        match &self.game {
            GamePicks::Myth2(g) => g.pick_up_target(&self.scene, cursor, selection),
            GamePicks::Tfl(g) => g.pick_up_target(&self.scene, cursor, selection),
        }
    }

    fn polygon_target(&mut self, cursor: (i16, i16)) -> selection_input::PressTarget {
        match &self.game {
            GamePicks::Myth2(g) => g.polygon_target(&self.scene, cursor),
            GamePicks::Tfl(g) => g.surface(&self.scene, cursor),
        }
    }

    fn monster_visible(&mut self, id: u32) -> bool {
        match &self.game {
            GamePicks::Myth2(g) => g.monster_visible(id),
            GamePicks::Tfl(_) => true,
        }
    }

    fn special_against_friendlies(&mut self, selection: &[u32]) -> Vec<u32> {
        match &self.game {
            GamePicks::Myth2(g) => g.sim.special_against_friendlies(selection),
            GamePicks::Tfl(_) => selection.to_vec(),
        }
    }

    fn band_units(&mut self, rect: selection::PixelRect) -> Vec<u32> {
        match &self.game {
            GamePicks::Myth2(g) => g.band_units(&self.scene, rect),
            GamePicks::Tfl(g) => g.band_units(&self.scene, rect),
        }
    }

    fn gesture_point(&mut self, cursor: (i16, i16)) -> Option<(i32, i32)> {
        match &self.game {
            GamePicks::Myth2(g) => g.gesture_point(&self.scene, cursor),
            GamePicks::Tfl(g) => g.gesture_point(&self.scene, cursor),
        }
    }

    fn gesture_facing(&mut self, from: (i16, i16), to: (i16, i16)) -> u16 {
        match &self.game {
            GamePicks::Myth2(_) => 0,
            GamePicks::Tfl(g) => g.gesture_facing(&self.scene, from, to),
        }
    }

    fn unit_location(&mut self, id: u32) -> Option<(i32, i32)> {
        match &self.game {
            GamePicks::Myth2(g) => g.sim.unit_location(id),
            GamePicks::Tfl(g) => g.unit_location(id),
        }
    }

    fn same_type_flood(&mut self, base: u32) -> Vec<u32> {
        match &self.game {
            GamePicks::Myth2(g) => g.sim.same_type_flood(base),
            GamePicks::Tfl(g) => g.same_type_flood(&self.scene, base),
        }
    }

    fn all_controlled(&mut self, selection: &[u32]) -> bool {
        match &self.game {
            GamePicks::Myth2(g) => g.sim.all_controlled(selection),
            GamePicks::Tfl(_) => false,
        }
    }
}

pub(crate) fn sprite_screen_rect(
    sprite: &BillboardInstance,
    camera: &Camera,
    viewport: (u32, u32),
) -> Option<[f32; 4]> {
    let (right, up) = if sprite.flags & BILLBOARD_FLAG_SCREEN_ALIGNED != 0 {
        let (r, u, _) = camera.view_axes();
        (r, u)
    } else {
        sprite.sprite_basis(camera.eye_position())
    };
    let ([x0, x1], [y0, y1]) = sprite.effective_quad_bounds();
    let mut rect = [f32::MAX, f32::MAX, f32::MIN, f32::MIN];
    for (ox, oy) in [(x0, y0), (x1, y0), (x0, y1), (x1, y1)] {
        let p = sprite.position;
        let corner = [
            p[0] + right[0] * ox + up[0] * oy,
            p[1] + right[1] * ox + up[1] * oy,
            p[2] + right[2] * ox + up[2] * oy,
        ];
        let [x, y] = camera
            .frame_projection(corner, viewport.0, viewport.1)?
            .origin;
        rect = [
            rect[0].min(x),
            rect[1].min(y),
            rect[2].max(x),
            rect[3].max(y),
        ];
    }
    Some(rect)
}

fn units_in_band(
    world: &WorldView,
    sim: &dyn Myth2PickSim,
    camera: &Camera,
    sprite_mgr: &ClientSpriteManager,
    viewport: (u32, u32),
    rect: selection::PixelRect,
) -> Vec<u32> {
    drawn_units_in_rect(world, camera, sprite_mgr, viewport, rect, |id| {
        sim.band_eligible(id)
    })
}

pub(crate) fn drawn_units_in_rect(
    world: &WorldView,
    camera: &Camera,
    sprite_mgr: &ClientSpriteManager,
    viewport: (u32, u32),
    rect: selection::PixelRect,
    keep: impl Fn(u32) -> bool,
) -> Vec<u32> {
    let pick = [
        rect.left.max(0) as f32,
        rect.top.max(0) as f32,
        rect.right.min(viewport.0 as i32) as f32,
        rect.bottom.min(viewport.1 as i32) as f32,
    ];
    let eye = camera.eye_position();
    let forward = camera.view_axes().2;
    let mut hits: Vec<(f32, u32)> = world
        .monsters
        .iter()
        .filter_map(|o| {
            let id = o.monster?.id;
            if world.unit(id).is_none() || !keep(id) {
                return None;
            }
            let f = monster_frame_rect(o, camera, viewport)?;
            let frame_hit = pick[0] < f.right as f32
                && pick[2] > f.left as f32
                && pick[1] < f.bottom as f32
                && pick[3] > f.top as f32;
            if !frame_hit {
                return None;
            }
            let sprite = drawn_monster_sprite(o, world, camera, sprite_mgr)?;
            let [l, t, r, b] = sprite_screen_rect(&sprite, camera, viewport)?;
            if !(pick[0] < r && pick[2] > l && pick[1] < b && pick[3] > t) {
                return None;
            }
            let p = sprite.position;
            let depth = (p[0] - eye[0]) * forward[0]
                + (p[1] - eye[1]) * forward[1]
                + (p[2] - eye[2]) * forward[2];
            Some((depth, id))
        })
        .collect();
    hits.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.cmp(&b.1)));
    hits.into_iter().map(|(_, id)| id).collect()
}

#[cfg(test)]
impl PickWorld<'_> {}

#[cfg(test)]
mod tests {
    use super::*;
    use PickSurface::{Mesh, Model, Monster, Object};

    #[test]
    fn a_pick_up_is_the_first_object_taken_before_the_polygon() {
        let ground = Mesh {
            point: [0, 0],
            cell_flags: 0,
        };
        let takes = |i: i32| (i % 2 == 0).then_some(i * 10);
        let list = [
            Monster { id: 1, outside: 0 },
            Object { index: 3 },
            Object { index: 4 },
            ground,
            Object { index: 6 },
        ];
        assert_eq!(pick_up_object(&list, takes), Some((4, 40)));
        assert_eq!(pick_up_object(&list[..2], takes), None);
        let behind = [
            Object { index: 3 },
            Model { point: [0; 3] },
            Object { index: 6 },
        ];
        assert_eq!(pick_up_object(&behind, takes), None);
        assert_eq!(
            pick(&[Object { index: 4 }, ground], &mut 0),
            Picked::Mesh { cell_flags: 0 }
        );
    }

    fn hit(id: u32, near_miss: bool) -> Option<PointHit> {
        Some(PointHit { id, near_miss })
    }

    const HILL: PickSurface = Mesh {
        point: [100, 200],
        cell_flags: 0,
    };
    const WATER: PickSurface = Mesh {
        point: [100, 200],
        cell_flags: 0x800,
    };
    const ROOF: PickSurface = Model {
        point: [300, 400, 500],
    };

    #[test]
    fn a_polygon_nearer_than_the_sprite_ends_the_press() {
        let unit = Monster { id: 1, outside: 0 };
        assert_eq!(press_pick(&[unit, HILL], |_| true), hit(1, false));
        assert_eq!(press_pick(&[HILL, unit], |_| true), None);
        assert_eq!(press_pick(&[ROOF, unit], |_| true), None);
        assert_eq!(polygon_point(&[unit, ROOF, HILL]), Some([300, 400]));
    }

    #[test]
    fn a_near_miss_seen_before_a_polygon_is_the_result() {
        let near = Monster { id: 1, outside: 7 };
        let body = Monster { id: 2, outside: 0 };
        assert_eq!(press_pick(&[near, HILL, body], |_| true), hit(1, true));
        assert_eq!(press_pick(&[near, HILL, body], |id| id != 1), None);
    }

    #[test]
    fn the_walk_goes_on_past_the_surface_of_media() {
        let unit = Monster { id: 1, outside: 0 };
        assert_eq!(press_pick(&[WATER, unit], |_| true), hit(1, false));
        assert_eq!(
            press_pick(&[WATER, HILL, ROOF, unit], |_| true),
            hit(1, false)
        );
        assert_eq!(press_pick(&[WATER, HILL], |_| true), None);
        let other = Monster { id: 2, outside: 0 };
        assert_eq!(press_pick(&[WATER, unit, HILL, other], |id| id != 1), None);
    }

    #[test]
    fn the_walk_over_monsters_keeps_its_rules() {
        let near = Monster { id: 1, outside: 3 };
        let nearer = Monster { id: 2, outside: 2 };
        let body = Monster { id: 3, outside: 0 };
        assert_eq!(press_pick(&[near, nearer], |_| true), hit(2, true));
        assert_eq!(press_pick(&[near, body, nearer], |_| true), hit(3, false));
        assert_eq!(
            press_pick(&[near, body, nearer], |id| id != 3),
            hit(2, true)
        );
        assert_eq!(press_pick(&[near, nearer], |id| id != 2), None);
    }
}
