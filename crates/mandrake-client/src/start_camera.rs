use crate::args::Args;
use crate::camera_control;
use crate::map_load::LOCAL_PLAYER_TEAM;
use mandrake_common::FourCC;
use mandrake_sim::Simulation;
use mandrake_tag::{MarkerEntry, MeshTag, TagArchive};
use mandrake_view::camera::myth_observer::ObserverPhysicsConstants;
use mandrake_view::camera::{Camera, myth_observer};
use tracing::{info, warn};

const LEGACY_ZOOM: f32 = 18.0;

const LEGACY_YAW: f32 = -0.42;

const LEGACY_PITCH: f32 = 0.56;

pub(crate) fn load_observer_physics_constants(archive: &TagArchive) -> ObserverPhysicsConstants {
    let obpc = FourCC::new(*b"obpc");
    let parsed = archive
        .find_tag_by_id(obpc, obpc)
        .and_then(|entry| archive.get_tag_data(entry).ok())
        .and_then(|data| ObserverPhysicsConstants::from_tag_bytes(&data));
    match parsed {
        Some(constants) => {
            info!("Observer physics constants from obpc tag: {constants:?}");
            constants
        }
        None => {
            warn!("No obpc tag in the archive; using the shipped tag's observer physics values");
            ObserverPhysicsConstants::SHIPPED_TAG
        }
    }
}

fn camera_overridden(args: &Args) -> bool {
    args.target_x.is_some()
        || args.target_z.is_some()
        || args.zoom.is_some()
        || args.yaw.is_some()
        || args.pitch.is_some()
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct StartTeams {
    pub mode: i16,
    pub local_team: i16,
    pub players: Vec<u8>,
}

impl StartTeams {
    pub(crate) fn solo() -> Self {
        Self {
            mode: 2,
            local_team: LOCAL_PLAYER_TEAM,
            players: Vec::new(),
        }
    }

    pub(crate) fn of(sim: &Simulation) -> Self {
        Self {
            mode: sim.engine.mode,
            local_team: sim.engine.local_team(),
            players: sim.engine.players.teams.iter().map(|t| t.players).collect(),
        }
    }

    fn claimed(&self, team: i16) -> bool {
        usize::try_from(team)
            .ok()
            .and_then(|t| self.players.get(t))
            .is_some_and(|&n| n != 0)
    }
}

pub(crate) fn start_observer_marker<'a>(
    mesh_tag: &'a MeshTag,
    teams: &StartTeams,
) -> Option<&'a MarkerEntry> {
    let any_claimed = teams.mode == 0 || (teams.mode == 1 && teams.local_team == -1);
    let mut fallback = None;
    for marker in mesh_tag.observer_markers() {
        let Some(team) = mesh_tag.palette_for_marker(marker).map(|p| p.team_index()) else {
            continue;
        };
        if team == -1 {
            continue;
        }
        if team == teams.local_team || (any_claimed && teams.claimed(team)) {
            return Some(marker);
        }
        fallback.get_or_insert(marker);
    }
    fallback
}

fn start_observer(
    mesh_tag: &MeshTag,
    args: &Args,
    teams: &StartTeams,
    cell_size: f32,
) -> Option<(f32, f32, f32, f32)> {
    match args.observer.as_deref() {
        Some(&[x, y, facing, zoom]) => Some((x, y, facing, zoom)),
        _ if camera_overridden(args) => None,
        _ => Some(match start_observer_marker(mesh_tag, teams) {
            Some(marker) => (
                marker.x as f32 / MarkerEntry::WORLD_ONE,
                marker.y as f32 / MarkerEntry::WORLD_ONE,
                marker.yaw_radians(),
                myth_observer::START_ZOOM,
            ),
            None => (
                f32::from(mesh_tag.cell_width) * cell_size * 0.5,
                f32::from(mesh_tag.cell_height) * cell_size * 0.5,
                0.0,
                myth_observer::START_ZOOM,
            ),
        }),
    }
}

const MESH_MIN_ZOOM_OFFSET: usize = 0x248;

pub(crate) fn start_min_zoom(mesh_data: &[u8], teams: &StartTeams, restoring: bool) -> f32 {
    let standard = myth_observer::MIN_ZOOM_STANDARD;
    let extended = myth_observer::MIN_ZOOM_EXTENDED;
    if !restoring && (teams.mode == 0 || (teams.mode == 1 && teams.local_team == -1)) {
        return extended;
    }
    let mesh = mesh_data
        .get(MESH_MIN_ZOOM_OFFSET..MESH_MIN_ZOOM_OFFSET + 4)
        .map_or(0.0, |b| f32::from_be_bytes([b[0], b[1], b[2], b[3]]));
    if (extended..=standard).contains(&mesh) {
        mesh
    } else {
        standard
    }
}

pub(crate) fn start_observer_state(
    mesh_tag: &MeshTag,
    args: &Args,
    teams: &StartTeams,
) -> Option<camera_control::ObserverState> {
    start_observer(mesh_tag, args, teams, 1.0).map(|(x, y, facing, zoom)| {
        camera_control::ObserverState {
            x,
            y,
            z: myth_observer::START_HEIGHT,
            facing,
            pitch: camera_control::START_PITCH,
            zoom,
        }
    })
}

pub(crate) fn initial_camera_for(mesh_tag: &MeshTag, args: &Args, teams: &StartTeams) -> Camera {
    initial_camera_with(mesh_tag, args, teams, 1.0)
}

pub(crate) fn initial_camera_with(
    mesh_tag: &MeshTag,
    args: &Args,
    teams: &StartTeams,
    cell_size: f32,
) -> Camera {
    let observer = start_observer_marker(mesh_tag, teams);
    if let Some((col, row, facing, zoom)) = start_observer(mesh_tag, args, teams, cell_size) {
        let ground = |c: f32, r: f32| mesh_tag.terrain_height_at(c / cell_size, r / cell_size);
        let ([ec, er, ez], pitch) = myth_observer::eye_and_pitch(col, row, facing, zoom, ground);
        let half_w = mesh_tag.cell_width as f32 * cell_size * 0.5;
        let half_d = mesh_tag.cell_height as f32 * cell_size * 0.5;
        let eye = [half_w - ec, ez, er - half_d];
        let ground_y = ground(col, row);
        info!(
            "Map-start observer camera: marker ({:.2}, {:.2}) facing {:.1} deg, eye height {:.2}, pitch {:.1} deg",
            col,
            row,
            facing.to_degrees(),
            ez,
            pitch.to_degrees()
        );
        return Camera::from_myth_observer(eye, facing, pitch, ground_y);
    }

    let target = if let (Some(x), Some(z)) = (args.target_x, args.target_z) {
        [x, 0.0, z]
    } else if let Some(marker) = observer {
        let (ox, oy, oz) = mesh_tag.marker_world_position(marker, 1.0);
        [ox, oy, oz]
    } else {
        [4.5, 0.0, 11.5]
    };
    Camera::new(
        target,
        args.zoom.unwrap_or(LEGACY_ZOOM),
        args.yaw.unwrap_or(LEGACY_YAW),
        args.pitch.unwrap_or(LEGACY_PITCH),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn teams(mode: i16, local_team: i16) -> StartTeams {
        StartTeams {
            mode,
            local_team,
            players: Vec::new(),
        }
    }

    fn mesh(min_zoom: f32) -> Vec<u8> {
        let mut data = vec![0u8; 0x300];
        data[0x248..0x24c].copy_from_slice(&min_zoom.to_be_bytes());
        data
    }

    #[test]
    fn the_start_min_zoom_follows_the_mode() {
        let standard = myth_observer::MIN_ZOOM_STANDARD;
        let extended = myth_observer::MIN_ZOOM_EXTENDED;
        let own = mesh(-3.0);
        assert_eq!(start_min_zoom(&own, &teams(0, 0), false), extended);
        assert_eq!(start_min_zoom(&own, &teams(1, -1), false), extended);
        assert_eq!(start_min_zoom(&own, &teams(2, 0), false), -3.0);
        assert_eq!(start_min_zoom(&own, &teams(1, 3), false), -3.0);
        assert_eq!(start_min_zoom(&own, &teams(0, 0), true), -3.0);
        assert_eq!(start_min_zoom(&own, &teams(1, -1), true), -3.0);
        assert_eq!(start_min_zoom(&mesh(-5.0), &teams(2, 0), false), extended);
        assert_eq!(
            start_min_zoom(&mesh(standard), &teams(2, 0), false),
            standard
        );
        assert_eq!(start_min_zoom(&mesh(0.0), &teams(2, 0), false), standard);
        assert_eq!(start_min_zoom(&mesh(-5.5), &teams(1, 0), false), standard);
        assert_eq!(
            start_min_zoom(&mesh(f32::NAN), &teams(2, 0), false),
            standard
        );
        assert_eq!(start_min_zoom(&[], &teams(2, 0), false), standard);
        assert_eq!(start_min_zoom(&[], &teams(0, 0), false), extended);
    }
}
