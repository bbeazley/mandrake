use crate::map_actions::{combined, find};
use crate::trig::arctangent;
use crate::{Stop, TflSim};

const MAX_SUBJECTS: usize = 0x12c;
const TYPE_MONSTERS: i16 = 2;
const TYPE_TEAM: i16 = 5;
const RINGS: i32 = 20;

fn free_cell_near(
    centre: (i32, i32),
    (wmax, hmax): (i32, i32),
    mut free: impl FnMut(i32, i32) -> bool,
) -> Option<(i32, i32)> {
    let (cx, cy) = (
        i32::from((centre.0 >> 10) as i16),
        i32::from((centre.1 >> 10) as i16),
    );
    let clamp = |v: i32, max: i32| if v < 0 { 0 } else { v.min(max) };
    let middle = |x: i32, y: i32| ((x << 10) + 0x200, (y << 10) + 0x200);
    for r in 0..RINGS {
        let (x0, x1) = (clamp(cx - r, wmax), clamp(cx + r, wmax));
        let (y0, y1) = (clamp(cy - r, hmax), clamp(cy + r, hmax));
        let found = (y0..=y1)
            .find_map(|y| [x0, x1].into_iter().find(|&x| free(x, y)).map(|x| (x, y)))
            .or_else(|| {
                (x0..=x1).find_map(|x| [y0, y1].into_iter().find(|&y| free(x, y)).map(|y| (x, y)))
            });
        if let Some((x, y)) = found {
            return (r + 1 != RINGS).then(|| middle(x, y));
        }
    }
    None
}

impl TflSim {
    pub(crate) fn move_marker_action(&mut self, i: usize) -> Result<u32, Stop> {
        let subjects = combined(
            &self.map_actions.actions,
            i,
            b"subj",
            TYPE_MONSTERS,
            MAX_SUBJECTS,
        );
        if subjects.is_empty() {
            return Ok(2);
        }
        let Some(team) = find(&self.map_actions.actions, i, 0, b"team", TYPE_TEAM)
            .filter(|p| p.count > 0)
            .map(|p| p.i32_at(0))
        else {
            return Ok(2);
        };
        let mut most = 0i16;
        let mut centre = None;
        for g in self.spatial_supergroups()? {
            if i32::from(g.team) == team && g.members > most {
                most = g.members;
                centre = Some(g.centre);
            }
        }
        let Some(centre) = centre else {
            return Ok(2);
        };
        let max = (self.width() - 1, self.height_cells() - 1);
        let point =
            free_cell_near(centre, max, |x, y| self.cell_has_no_obstacles(x, y)).unwrap_or(centre);
        for &id in &subjects {
            let Some(k) = self
                .data
                .mesh
                .markers
                .iter()
                .position(|m| m.marker_type == crate::setup::UNIT && m.marker_id == id)
            else {
                continue;
            };
            let z = i32::from(self.mesh_height(centre.0, centre.1)? as i16);
            let yaw = arctangent(
                centre.0.wrapping_sub(point.0),
                centre.1.wrapping_sub(point.1),
            );
            crate::tc2_trace::log(|| {
                format!(
                    "MM {} {i} {id} {} {} {z} {yaw} {} {}",
                    self.tick, point.0, point.1, centre.0, centre.1
                )
            });
            let m = &mut self.data.mesh.markers[k];
            m.x = point.0;
            m.y = point.1;
            m.z = z;
            m.yaw = yaw as i16;
            m.type_data[0..2].copy_from_slice(&yaw.to_be_bytes());
        }
        Ok(2)
    }

    fn cell_has_no_obstacles(&self, x: i32, y: i32) -> bool {
        let c = self.cell_at(x, y);
        if self.cells[c].flags & 0x80 != 0 {
            return false;
        }
        let mut k = self.cell_heads[c];
        while k >= 0 {
            let o = &self.objects[k as usize];
            if o.flags & 0x600 == 0x400 {
                return false;
            }
            k = o.next_in_cell;
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::free_cell_near;

    #[test]
    fn free_cell_near_rings() {
        assert_eq!(
            free_cell_near((0x1234, 0x5678), (63, 63), |_, _| true),
            Some((0x1200, 0x5600))
        );
        assert_eq!(
            free_cell_near((0x1000, 0x1400), (63, 63), |x, y| (x, y) == (6, 3)),
            Some((0x1a00, 0xe00))
        );
        assert_eq!(free_cell_near((0, 0), (63, 63), |x, _| x == 19), None);
        assert_eq!(free_cell_near((0, 0), (63, 63), |_, _| false), None);
        assert_eq!(
            free_cell_near((0, 0), (63, 63), |x, _| x == 18),
            Some((18 * 1024 + 0x200, 0x200))
        );
    }
}
