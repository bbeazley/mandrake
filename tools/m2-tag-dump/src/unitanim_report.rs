use std::path::Path;

use mandrake_common::{FourCC, NativeVfs};
use mandrake_sim::engine_rng::GameParameters;
use mandrake_sim::{LocalMovement, Simulation};
use mandrake_tag::{MeshTag, TagArchive, engine_tag_files};

#[derive(Debug, Clone)]
pub struct QueuedMove {
    pub after: u32,
    pub ids: Vec<u32>,
    pub points: Vec<(i32, i32)>,
}

pub fn parse_move(s: &str) -> anyhow::Result<QueuedMove> {
    let parts: Vec<&str> = s.split(':').collect();
    let [after, ids, pts] = parts.as_slice() else {
        anyhow::bail!("--move takes AFTER:ID,ID,...:X,Y[;X,Y], got {s}");
    };
    let ids = ids
        .split(',')
        .map(|v| v.trim().parse::<u32>())
        .collect::<Result<Vec<_>, _>>()?;
    let mut points = Vec::new();
    for p in pts.split(';') {
        let (x, y) = p
            .split_once(',')
            .ok_or_else(|| anyhow::anyhow!("point {p} is not X,Y"))?;
        points.push((x.trim().parse()?, y.trim().parse()?));
    }
    Ok(QueuedMove {
        after: after.trim().parse()?,
        ids,
        points,
    })
}

fn open(paths: &[std::path::PathBuf]) -> Vec<TagArchive> {
    let vfs = NativeVfs::default_cwd();
    paths
        .iter()
        .filter_map(|p| TagArchive::open(&vfs, &p.to_string_lossy()).ok())
        .collect()
}

fn line(k: u32, sim: &Simulation, only: &[u32]) -> String {
    let mut s = String::new();
    let w = &sim.engine;
    for m in &w.monsters {
        let Ok(id) = u32::try_from(m.identifier) else {
            continue;
        };
        if !only.is_empty() && !only.contains(&id) {
            continue;
        }
        let o = &m.object;
        let def = &w.monster_definitions[m.definition];
        let slots = [
            (0x08, def.idle_sequence),
            (0x0a, def.walk_sequence),
            (0x0e, def.blocked_sequence),
            (0x10, def.turn_sequence),
            (0x1c, def.idle_sequence_1c),
            (0x1e, def.idle_sequence_1e),
            (0x20, def.idle_sequence_20),
            (0x22, def.sequence_22),
            (0x2a, def.sequence_2a),
        ];
        let matching: Vec<i32> = slots
            .iter()
            .filter(|(_, q)| *q >= 0 && *q == i16::from(o.sequence))
            .map(|(off, _)| *off)
            .collect();
        let loaded = matching
            .iter()
            .any(|off| [0x08, 0x0a, 0x10, 0x1c, 0x1e, 0x20].contains(off));
        let slot = if matching.is_empty() {
            "-".to_string()
        } else {
            matching
                .iter()
                .map(|off| format!("{off:#x}"))
                .collect::<Vec<_>>()
                .join("+")
        };
        let (uf, us, ux, uy, uz) = match sim.units.get(&id) {
            Some(u) => {
                let (x, y) = u.engine.as_ref().map_or((0, 0), |e| (e.x, e.y));
                (
                    i32::from(u.facing.to_raw()),
                    format!("{:?}", u.state),
                    x,
                    y,
                    (u.position.y.to_f32() * 512.0).round() as i32,
                )
            }
            None => (-1, "-".into(), 0, 0, 0),
        };
        s += &format!(
            "K k={k} m={} id={id} mode={} st={:#x} seq={} frame={} ft={} tpf={} aflags={:#x} \
             facing={} mirrored={} scale={} perm={} pos={},{},{} unit_facing={uf} \
             unit_state={us} unit_pos={ux},{uy} unit_z={uz} slot={slot} client_loads={}\n",
            m.index,
            m.mode,
            m.anim_state,
            o.sequence,
            o.frame,
            o.tick,
            o.ticks_per_frame,
            o.anim_flags,
            m.facing,
            u8::from(m.mirrored),
            o.scale,
            o.permutation,
            m.position.0,
            m.position.1,
            o.z,
            u8::from(loaded),
        );
    }
    s
}

pub fn report(
    game_dir: &Path,
    mesh: &str,
    film: Option<&Path>,
    seed: u32,
    ticks: u32,
    moves: &[QueuedMove],
    only: &[u32],
) -> anyhow::Result<String> {
    let film_bytes = film.map(std::fs::read).transpose()?;
    let plugins: Vec<String> = Vec::new();
    let plugin_refs: Vec<&str> = plugins.iter().map(String::as_str).collect();
    let files = engine_tag_files(game_dir, &plugin_refs);
    let archives = open(&files.paths);
    let tags = |g: [u8; 4], id: FourCC| {
        archives.iter().rev().find_map(|a| {
            a.find_tag_by_id(FourCC::new(g), id)
                .and_then(|e| a.get_tag_data(e).ok())
        })
    };
    let mesh_id = match (&film_bytes, mesh) {
        (Some(f), "-") => mandrake_rec::Recording::from_bytes(f)?.header.mesh_tag_id,
        _ => FourCC::new(
            <[u8; 4]>::try_from(format!("{mesh:<4}").as_bytes())
                .map_err(|_| anyhow::anyhow!("mesh id {mesh} is not 4 characters"))?,
        ),
    };
    let data = tags(*b"mesh", mesh_id).ok_or_else(|| anyhow::anyhow!("no mesh {mesh}"))?;
    let mesh_tag = MeshTag::from_bytes(&data)?;
    let (mut sim, params) = match &film_bytes {
        Some(f) => (
            Simulation::from_film(f)?,
            GameParameters::from_film(f).unwrap_or(GameParameters::COMMAND_LINE),
        ),
        None => (
            Simulation::command_line_game(seed),
            GameParameters::COMMAND_LINE,
        ),
    };
    sim.load_mesh(&mesh_tag);
    let mut out = String::new();
    if let Err(e) = sim.load_engine_map(&data, &tags, params) {
        anyhow::bail!("engine map load stopped: {e}");
    }
    let monsters = sim.engine.monsters.clone();
    sim.spawn_units_for_monsters(&mesh_tag, &monsters);
    sim.apply_engine_unit_objects(&monsters);
    let load = sim.attach_engine_movement(&mesh_tag, &tags);
    out += &format!(
        "# unitanim-report mesh={mesh_id} film={} seed={seed} monsters={} attached={} stop_at_load={}\n",
        film.map_or("-".into(), |p| p.display().to_string()),
        monsters.len(),
        load.attached,
        sim.engine_stop
            .as_ref()
            .map_or("none".into(), |e| e.to_string())
    );
    out += &line(0, &sim, only);
    let mut stopped = sim.engine_stop.is_some();
    for k in 1..=ticks {
        for mv in moves.iter().filter(|m| m.after + 1 == k) {
            let header = sim
                .local_movement_header(&mv.ids, None)
                .map_err(|e| anyhow::anyhow!("movement header: {e}"))?;
            out += &format!(
                "# k={k}: local movement {:?} to {:?}, header {header:?}\n",
                mv.ids, mv.points
            );
            sim.queue_local_movement(LocalMovement {
                unit_ids: mv.ids.clone(),
                points: mv.points.clone(),
                header,
            });
        }
        sim.step();
        if !stopped && let Some(e) = &sim.engine_stop {
            out += &format!("# engine model stopped in step {k}: {e}\n");
            stopped = true;
        }
        out += &line(k, &sim, only);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn move_spec_parses() {
        let m = parse_move("55:10007,10008:31000,42000;31500,42500").unwrap();
        assert_eq!(m.after, 55);
        assert_eq!(m.ids, vec![10007, 10008]);
        assert_eq!(m.points, vec![(31000, 42000), (31500, 42500)]);
        assert!(parse_move("55:1").is_err());
    }
}
