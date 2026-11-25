use std::path::Path;

use mandrake_common::FourCC;
use mandrake_common::game_options::{CAMERA_TRACKING, CARRY_VETERANS, COOPERATIVE, VETERANS};
use mandrake_tag::mesh::{MarkerEntry, MarkerPaletteEntry};

use crate::Stop;
use crate::TflSim;
use crate::draw_site;
use crate::monsters::TflMonster;
use crate::objects::{TflObject, set_object_shape};

const SCENERY: i16 = 1;
pub(crate) const UNIT: i16 = 3;
const PROJECTILE: i16 = 9;

pub(crate) fn set_marker_monster(m: &mut MarkerEntry, monster: Option<usize>) {
    let v = monster.map_or(-1, |i| i as i16);
    m.type_data[0x1c..0x1e].copy_from_slice(&v.to_be_bytes());
}

pub(crate) fn marker_monster(m: &MarkerEntry) -> i16 {
    i16::from_be_bytes([m.type_data[0x1c], m.type_data[0x1d]])
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MonsterInitializer {
    pub used: bool,
    pub player: u32,
    pub unit: FourCC,
    pub mons: FourCC,
    pub experience: i8,
    pub field_11: u8,
    pub permutation: i8,
    pub field_13: u8,
    pub name: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnitCopy {
    pub unit: FourCC,
    pub player: usize,
    pub permutation: Option<i16>,
}

impl MonsterInitializer {
    pub const SIZE: usize = 0x28;
}

pub fn film_monster_initializers(film: &[u8]) -> Result<Vec<MonsterInitializer>, Stop> {
    let short = || {
        Stop::new(
            draw_site::MONSTER_INITIALIZERS_SHORT,
            "film: monster initializers truncated",
        )
    };
    let d = crate::be::u32_at(film, 0x30).ok_or_else(short)? as usize;
    let offset = crate::be::u32_at(film, d + 0x14).ok_or_else(short)? as usize;
    let size = crate::be::u32_at(film, d + 0x18).ok_or_else(short)? as usize;
    if crate::film_game_options(film)? & CARRY_VETERANS == 0 || size == 0 {
        return Ok(Vec::new());
    }
    let block = film.get(d + offset..d + offset + size).ok_or_else(short)?;
    Ok(block
        .as_chunks::<{ MonsterInitializer::SIZE }>()
        .0
        .iter()
        .map(|r| {
            let tag = |o: usize| FourCC(r[o..o + 4].try_into().unwrap());
            let name = &r[0x16..];
            MonsterInitializer {
                used: false,
                player: u32::from_be_bytes(r[4..8].try_into().unwrap()),
                unit: tag(8),
                mons: tag(0xc),
                experience: r[0x10] as i8,
                field_11: r[0x11],
                permutation: r[0x12] as i8,
                field_13: r[0x13],
                name: name[..name.iter().position(|&c| c == 0).unwrap_or(name.len())].to_vec(),
            }
        })
        .collect())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct InitializerPlayer {
    pub id: u32,
    pub team: i16,
}

fn get_monster_initializer(
    list: &mut [MonsterInitializer],
    game_options: u32,
    unit: FourCC,
    mons: FourCC,
    entry_player: Option<usize>,
    players: &[InitializerPlayer],
) -> Option<usize> {
    if game_options & VETERANS == 0 {
        return None;
    }
    let owner = players[entry_player?];
    let mut best: Option<usize> = None;
    for (k, v) in list.iter().enumerate() {
        if v.used || !(v.unit == unit || v.mons == mons) {
            continue;
        }
        if v.player != owner.id
            && !players
                .iter()
                .any(|q| q.id == v.player && q.team == owner.team)
        {
            continue;
        }
        if best.is_none_or(|b| list[b].experience < v.experience) {
            best = Some(k);
        }
    }
    if let Some(b) = best {
        list[b].used = true;
    }
    best
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct TeamClaims {
    pub claims: Vec<Option<usize>>,
    pub player_teams: Vec<i16>,
    pub leaders: Vec<bool>,
}

fn team_claims(
    header: &mandrake_rec::tfl::TflFilmHeader,
    team_count: i16,
    game_options: u32,
) -> Result<TeamClaims, Stop> {
    let players = &header.players;
    let n = team_count.max(0) as usize;
    let mut claims = vec![None; n];
    let mut captain: Vec<Option<usize>> = players
        .iter()
        .map(|p| {
            (p.captain != -1)
                .then(|| players.iter().position(|q| q.net_id == p.captain))
                .flatten()
        })
        .collect();
    if n != 0 && game_options & COOPERATIVE == 0 {
        let mut most = team_count;
        if 0 < header.team_size && header.team_size <= team_count {
            most = header.team_size;
        }
        let mut counts = vec![0i16; players.len()];
        let mut leaders = 0i16;
        for (i, c) in captain.iter().enumerate() {
            match *c {
                Some(c) if c == i => {
                    leaders += 1;
                    counts[c] += 1;
                }
                None => leaders += 1,
                Some(c) => counts[c] += 1,
            }
        }
        if most < leaders && captain.iter().any(Option::is_none) {
            return Err(Stop::new(
                draw_site::TEAM_CLAIMS,
                "initialize_players_for_new_map: more players than teams (a random pick)",
            ));
        }
        for (i, c) in captain.iter_mut().enumerate() {
            if *c == Some(i) && counts[i] < 2 {
                *c = None;
            }
        }
    }
    let mut player_teams = vec![-1i16; players.len()];
    let mut leaders = vec![false; players.len()];
    if n == 0 {
        return Ok(TeamClaims {
            claims,
            player_teams,
            leaders,
        });
    }
    for (i, p) in players.iter().enumerate() {
        if captain[i].is_some_and(|c| c != i) {
            continue;
        }
        leaders[i] = true;
        let t = if p.team == -1 {
            ((header.parameters_14 as u16 as i32 + i as i32) % i32::from(team_count)) as i16
        } else {
            p.team
        };
        let start = if (0..team_count).contains(&t) {
            t as usize
        } else {
            0
        };
        let mut t = start;
        loop {
            if claims[t].is_none() {
                claims[t] = Some(i);
                player_teams[i] = t as i16;
                break;
            }
            t = (t + 1) % n;
            if t == start {
                break;
            }
        }
    }
    for i in 0..players.len() {
        if player_teams[i] == -1
            && let Some(c) = captain[i]
        {
            player_teams[i] = player_teams[c];
        }
    }
    Ok(TeamClaims {
        claims,
        player_teams,
        leaders,
    })
}

impl TflSim {
    pub(crate) fn player_kind(&self, player: i8) -> Result<i16, Stop> {
        self.data
            .film
            .header
            .players
            .get(player as u8 as usize)
            .map(|p| p.field_02)
            .ok_or_else(|| {
                Stop::new(
                    draw_site::PLAYER_PAST_FILM,
                    format!("the players' table: player {player} past the film's players"),
                )
            })
    }

    pub fn set_up(game_dir: &Path, film: &[u8]) -> Result<Self, Box<(Option<Self>, Stop)>> {
        Self::set_up_with_options(game_dir, film, 0, false)
    }

    pub fn set_up_recording(
        game_dir: &Path,
        film: &[u8],
    ) -> Result<Self, Box<(Option<Self>, Stop)>> {
        Self::set_up_with_options(game_dir, film, 0, true)
    }

    pub fn set_up_as(
        game_dir: &Path,
        film: &[u8],
        game_mode: i16,
    ) -> Result<Self, Box<(Option<Self>, Stop)>> {
        Self::set_up_with_options(game_dir, film, game_mode, false)
    }

    pub fn set_up_as_recording(
        game_dir: &Path,
        film: &[u8],
        game_mode: i16,
    ) -> Result<Self, Box<(Option<Self>, Stop)>> {
        Self::set_up_with_options(game_dir, film, game_mode, true)
    }

    fn set_up_with_options(
        game_dir: &Path,
        film: &[u8],
        game_mode: i16,
        record_draws: bool,
    ) -> Result<Self, Box<(Option<Self>, Stop)>> {
        let fail = |s: Stop| Box::new((None, s));
        let data = crate::TflData::load(game_dir, film).map_err(fail)?;
        let rng = crate::TflRng::with_recording(data.film.header.initial_seed, record_draws);
        let difficulty = crate::film_difficulty(film).map_err(fail)?;
        let game_options = crate::film_game_options(film).map_err(fail)?;
        let initializers = film_monster_initializers(film).map_err(fail)?;
        let mut sim = Self {
            data,
            rng,
            difficulty,
            tick: 0,
            objects: Vec::new(),
            monsters: Vec::new(),
            mons_counters: std::collections::HashMap::new(),
            unit_copies: Vec::new(),
            map_actions: crate::map_actions::MapActions::default(),
            orders: Vec::new(),
            record_orders: false,
            command_cursor: 0,
            cached_local_team: None,
            meanders: std::collections::HashMap::new(),
            spatial_groups: None,
            spatial_supergroups: None,
            mesh_effects: Default::default(),
            legions: std::collections::HashMap::new(),
            suicides: std::collections::HashMap::new(),
            game_mode,
            cells: Vec::new(),
            cell_heads: Vec::new(),
            winning_team: -1,
            game_options,
            paths: crate::paths::PathPool::default(),
            obstacles: Vec::new(),
            media: crate::terrain::MediaCoverage::default(),
            object_pool: crate::objects::DatumPool::new(crate::objects::MAX_OBJECTS),
            monster_pool: crate::objects::DatumPool::new(crate::monsters::MAX_MONSTERS),
            projectiles: crate::projectiles::Projectiles::default(),
            monster_initializers: initializers,
            attack_ai: Default::default(),
            sight: crate::sight::SightMaps::new(0, 0),
            explosions: Default::default(),
            lightning: Default::default(),
            model_instances: Vec::new(),
            model_pool: crate::objects::DatumPool::new(crate::models::MAX_MODELS),
            model_effects: Default::default(),
            marker_motion: Vec::new(),
            particles: None,
            sound_events: Vec::new(),
            sound_sources: Vec::new(),
            stains: Vec::new(),
            marks: Vec::new(),
            colour_sources: Vec::new(),
            mark_count: 0,
            local_group_starts: Vec::new(),
            interest_events: Vec::new(),
            local_projectile_group_orders: Vec::new(),
            sniffer: None,
            overhead_map_state: None,
            camera_control_flags: if game_options & CAMERA_TRACKING != 0 {
                4
            } else {
                0
            },
            camera_movements: Vec::new(),
            statistics: crate::statistics::Statistics::default(),
            flags: crate::flags::FlagData::default(),
            player_table: None,
            interface_commands: Vec::new(),
        };
        sim.cell_heads = vec![-1; sim.data.mesh.cells.len()];
        sim.cells = sim.data.mesh.cells.clone();
        sim.sight = crate::sight::SightMaps::new(
            sim.data.mesh.cell_width as i32,
            sim.data.mesh.cell_height as i32,
        );
        let r = (|| {
            sim.media = crate::terrain::MediaCoverage::new(
                &sim.data.mesh_bytes,
                sim.data.mesh.cell_width,
                sim.data.mesh.cell_height,
            )?;
            sim.marker_motion = marker_motion(&sim.data.mesh_bytes, sim.data.mesh.markers.len());
            sim.initialize_particle_system()?;
            sim.add_markers()?;
            sim.place_marker_objects()?;
            sim.cache_local_team()?;
            sim.initialize_sight()?;
            sim.interest_events.clear();
            sim.map_actions.actions = crate::map_actions::parse(&sim.data.mesh_bytes)?;
            sim.note_definitions()?;
            sim.set_up_statistics_teams(&[])?;
            sim.update_player_statistics()?;
            sim.update_map_actions()
        })();
        match r {
            Ok(()) => Ok(sim),
            Err(s) => Err(Box::new((Some(sim), s))),
        }
    }

    pub(crate) fn add_markers(&mut self) -> Result<(), Stop> {
        let palettes = self.data.mesh.palettes.clone();
        let markers = self.data.mesh.markers.clone();
        for ty in [SCENERY, UNIT] {
            if ty == UNIT {
                self.new_monster_palette_entries(&palettes)?;
            }
            for (k, m) in markers
                .iter()
                .enumerate()
                .filter(|(_, m)| m.marker_type == ty)
            {
                let Some(p) = palettes
                    .iter()
                    .filter(|p| p.marker_type == ty)
                    .nth(m.palette_index.max(0) as usize)
                else {
                    continue;
                };
                match ty {
                    SCENERY => self.new_scenery(k, m, p)?,
                    _ => {
                        let made = self.new_monster(k, m, p, false)?;
                        set_marker_monster(&mut self.data.mesh.markers[k], made);
                    }
                }
            }
        }
        self.add_sound_sources();
        self.add_models()?;
        let b = self.data.mesh_bytes.clone();
        let offset = crate::be::u32_at(&b, 0x38).unwrap_or(0) as usize;
        for (k, m) in markers.iter().enumerate() {
            if m.marker_type != PROJECTILE {
                continue;
            }
            let Some(p) = palettes
                .iter()
                .filter(|p| p.marker_type == PROJECTILE)
                .nth(m.palette_index.max(0) as usize)
            else {
                return Err(Stop::new(
                    draw_site::MARKER_NO_PALETTE,
                    format!("marker {k}: no palette entry {}", m.palette_index),
                ));
            };
            let def = (p.tag_id.0 != [0xff; 4] && p.tag_id.0 != [0; 4]).then_some(p.tag_id);
            let at = 1024 + offset + 64 * k;
            let raw = b.get(at..at + 64).ok_or_else(|| {
                Stop::new(
                    draw_site::MARKER_OFF_MESH,
                    format!("marker {k} off the mesh"),
                )
            })?;
            self.new_projectile_marker(m, def, raw)?;
        }
        Ok(())
    }

    fn new_monster_palette_entries(&mut self, palettes: &[MarkerPaletteEntry]) -> Result<(), Stop> {
        let teams = self.team_claims()?;
        for p in palettes.iter().filter(|p| p.marker_type == UNIT) {
            self.new_monster_palette_entry(p.tag_id, p.team_index(), &teams.player_teams)?;
        }
        Ok(())
    }

    fn new_monster_palette_entry(
        &mut self,
        unit: FourCC,
        team: i16,
        player_teams: &[i16],
    ) -> Result<(), Stop> {
        if team == -1 {
            return Ok(());
        }
        for (player, &t) in player_teams.iter().enumerate() {
            if t != team
                || self
                    .unit_copies
                    .iter()
                    .any(|c| c.unit == unit && c.player == player)
            {
                continue;
            }
            let coloured = self.data.film.header.players[player].flags & 1 != 0;
            let mut permutation = None;
            if coloured {
                let core_id = self.data.def(b"unit", unit)?.tag(4);
                if let Some(core_id) = core_id {
                    let core = self.data.core(core_id)?;
                    let count = i32::from(core.permutation_count);
                    permutation = Some(
                        self.rng
                            .draw_mod(draw_site::PALETTE_PERMUTATION, count)
                            .ok_or_else(|| {
                                Stop::new(
                                    draw_site::PALETTE_PERMUTATION,
                                    format!("core {core_id}: no permutations"),
                                )
                            })? as i16,
                    );
                }
            }
            self.unit_copies.push(UnitCopy {
                unit,
                player,
                permutation,
            });
        }
        Ok(())
    }

    pub fn unit_copies(&self) -> &[UnitCopy] {
        &self.unit_copies
    }

    pub(crate) fn team_claims(&self) -> Result<TeamClaims, Stop> {
        if let Some(t) = &self.player_table {
            return Ok(t.claims.clone());
        }
        self.map_team_claims()
    }

    pub(crate) fn map_team_claims(&self) -> Result<TeamClaims, Stop> {
        team_claims(
            &self.data.film.header,
            self.unit_team_count(),
            self.game_options,
        )
    }

    fn unit_team_count(&self) -> i16 {
        self.data
            .mesh
            .palettes
            .iter()
            .filter(|q| q.marker_type == UNIT)
            .map(|q| q.team_index() + 1)
            .max()
            .unwrap_or(0)
    }

    fn new_scenery(
        &mut self,
        k: usize,
        m: &MarkerEntry,
        p: &MarkerPaletteEntry,
    ) -> Result<(), Stop> {
        if m.difficulty_flags > self.difficulty {
            return Ok(());
        }
        let scen = self.data.def_arc(b"scen", p.tag_id)?;
        let core_id = scen.tag(0x08).ok_or_else(|| {
            Stop::new(
                draw_site::SCENERY_NO_CORE,
                format!("scen {}: no core", p.tag_id),
            )
        })?;
        let core = self.data.core(core_id)?;
        let perm = self
            .rng
            .draw_mod(
                draw_site::SCENERY_PERMUTATION,
                i32::from(core.permutation_count),
            )
            .ok_or_else(|| {
                Stop::new(
                    draw_site::SCENERY_PERMUTATION,
                    format!("core {core_id}: no permutations"),
                )
            })?;
        let scale_range = scen.i16(0x24);
        let radius = if scale_range >= 1 {
            self.rng
                .draw_scaled(draw_site::SCENERY_SCALE, i32::from(scale_range)) as i16
        } else {
            0
        }
        .wrapping_add(scen.i16(0x22));
        let (mut facing, mut field_1e) = (m.yaw as u16, m.pitch as u16);
        if scen.u32(0) & 2 != 0 {
            facing = self.rng.draw(draw_site::SCENERY_FACING);
            field_1e = self.rng.draw(draw_site::SCENERY_PITCH);
        }
        let mut obj = TflObject::new(m.x, m.y, core.collection, core.permutations[perm as usize]);
        obj.permutation_index = perm as i16;
        obj.facing = facing;
        obj.field_1e = field_1e;
        self.set_marker_motion(&mut obj, k);
        obj.scale = radius;
        obj.scenery = Some(p.tag_id);
        if let Some(proj) = scen.tag(0x1c) {
            if self.data.def(b"proj", proj).is_err() {
                return Err(Stop::new(
                    draw_site::SCENERY_PROJECTILE_LOAD,
                    format!("scen {}: its projectile {proj} doesn't load", p.tag_id),
                ));
            }
            obj.kind = 4;
            obj.sleeping = Some(proj);
        }
        obj.physics = scen.tag(0x18);
        let headers = self.data.sequences(core.collection)?;
        set_object_shape(&mut obj, headers, scen.i16(0x3c), 0x100, 0xf, &mut self.rng)?;
        let f = scen.u32(0);
        if f & 1 != 0 {
            obj.flags |= 0x400;
        }
        if f & 0x10 != 0 {
            obj.flags |= 0x400_0000;
        }
        obj.flags |= 0x100;
        let o = self.add_object(obj)?;
        if f & 8 != 0 {
            let made = scen.i16(0x26) == self.data.film.header.game_type
                && self.new_flag(k, o, p.team_index(), scen.i16(0x28), m.flags)?;
            if !made {
                self.delete_object(o)?;
            }
        }
        Ok(())
    }

    fn set_marker_motion(&self, obj: &mut TflObject, k: usize) {
        let w = self.marker_motion.get(k).copied().unwrap_or_default();
        obj.velocity = [w[0], w[1], w[2]];
        obj.above_ground = w[3];
    }

    pub(crate) fn try_add_object(&mut self, mut obj: TflObject) -> Option<usize> {
        let objects = &self.objects;
        let (index, salt) = self
            .object_pool
            .allocate(|k| objects.get(k).is_some_and(|o| o.identifier != 0))?;
        obj.identifier = salt.wrapping_neg();
        let cell = self.cell_index(obj.x, obj.y);
        obj.next_in_cell = self.cell_heads[cell];
        self.cell_heads[cell] = index as i16;
        if index == self.objects.len() {
            self.objects.push(obj);
        } else {
            self.objects[index] = obj;
        }
        if self.objects[index].interest != 0 {
            self.interest_event(crate::tracking::site::NEW_OBJECT, index, -1);
        }
        Some(index)
    }

    pub(crate) fn add_object(&mut self, obj: TflObject) -> Result<usize, Stop> {
        self.try_add_object(obj)
            .ok_or_else(|| Stop::new(draw_site::OBJECT_LIST_FULL, "the object list is full"))
    }

    pub(crate) fn cell_index(&self, x: i32, y: i32) -> usize {
        let (w, h) = (
            self.data.mesh.cell_width as i32,
            self.data.mesh.cell_height as i32,
        );
        let (cx, cy) = (x >> 10, y >> 10);
        if cx >= 0 && cx < w && cy >= 0 && cy < h {
            (w * cy + cx) as usize
        } else {
            0
        }
    }

    pub(crate) fn new_monster(
        &mut self,
        marker: usize,
        m: &MarkerEntry,
        p: &MarkerPaletteEntry,
        whole: bool,
    ) -> Result<Option<usize>, Stop> {
        let pflags = (p.flags as u16 as u32) << 16;
        let team = p.team_index();
        let teams = self.team_claims()?;
        let owner = (team >= 0)
            .then(|| teams.claims.get(team as usize).copied().flatten())
            .flatten();
        let mut placed = true;
        let mut unit_id = p.tag_id;
        let mut veteran = None;
        if m.flags & 1 == 0 || pflags & 0x80000 != 0 {
            if pflags & 0x40000 != 0 || (pflags & 0x80000 != 0 && m.flags & 1 != 0) {
                let mons_id = self.data.def(b"unit", p.tag_id)?.tag(0).ok_or_else(|| {
                    Stop::new(
                        draw_site::NEW_MONSTER,
                        format!("unit {}: no mons", p.tag_id),
                    )
                })?;
                let players: Vec<InitializerPlayer> = self
                    .data
                    .film
                    .header
                    .players
                    .iter()
                    .zip(&teams.player_teams)
                    .map(|(q, &team)| InitializerPlayer {
                        id: u32::from(q.net_id as u16),
                        team,
                    })
                    .collect();
                let entry_player = owner.filter(|_| pflags & 0x10000 == 0);
                match get_monster_initializer(
                    &mut self.monster_initializers,
                    self.game_options,
                    p.tag_id,
                    mons_id,
                    entry_player,
                    &players,
                ) {
                    None if pflags & 0x80000 != 0 => placed = false,
                    None => {}
                    Some(k) => {
                        let v = self.monster_initializers[k].clone();
                        if v.unit != p.tag_id {
                            self.new_monster_palette_entry(v.unit, team, &teams.player_teams)?;
                        }
                        unit_id = v.unit;
                        veteran = Some(v);
                    }
                }
            }
            if m.difficulty_flags > self.difficulty {
                placed = false;
            }
        } else {
            placed = false;
        }
        let Some(owner) = owner.filter(|_| placed) else {
            return Ok(None);
        };
        let coloured = if pflags & 0x10000 == 0 {
            self.unit_copies
                .iter()
                .find(|c| c.unit == unit_id && c.player == owner)
                .and_then(|c| c.permutation)
        } else {
            None
        };
        let unit = self.data.def_arc(b"unit", unit_id)?;
        let (Some(mons_id), Some(core_id)) = (unit.tag(0), unit.tag(4)) else {
            return Err(Stop::new(
                draw_site::NEW_MONSTER,
                format!("unit {unit_id}: no mons or core"),
            ));
        };
        let core = self.data.core(core_id)?;
        let mons = self.data.def_arc(b"mons", mons_id)?;
        let count = if coloured.is_some() {
            1
        } else {
            i32::from(core.permutation_count)
        };
        let perm = match &veteran {
            Some(v) => i32::from(v.permutation),
            None => self
                .rng
                .draw_mod(draw_site::MONSTER_PERMUTATION, count)
                .ok_or_else(|| {
                    Stop::new(
                        draw_site::MONSTER_PERMUTATION,
                        format!("core {core_id}: no permutations"),
                    )
                })?,
        };
        let perm = match (coloured, &veteran) {
            (Some(c), None) => i32::from(c),
            (Some(c), Some(_)) if perm == 0 => i32::from(c),
            (Some(_), Some(_)) if perm >= i32::from(core.permutation_count) => {
                return Err(Stop::new(
                    draw_site::NEW_MONSTER,
                    format!(
                        "a veteran's permutation {perm} past core {core_id}'s {} in its coloured copy",
                        core.permutation_count
                    ),
                ));
            }
            _ => perm,
        };
        let index = perm;
        let perm = *core.permutations.get(perm.max(0) as usize).ok_or_else(|| {
            Stop::new(
                draw_site::MONSTER_PERMUTATION,
                format!("core {core_id}: permutation {perm}"),
            )
        })?;
        let mut size = [0i16; 2];
        for (k, (site, at)) in [
            (draw_site::MONSTER_SCALE, 0x11a),
            (draw_site::MONSTER_SIZE_11E, 0x11e),
        ]
        .into_iter()
        .enumerate()
        {
            let r = if mons.i16(at) >= 1 {
                self.rng.draw_scaled(site, i32::from(mons.i16(at))) as i16
            } else {
                0
            };
            size[k] = r.wrapping_add(mons.i16(at - 2));
        }
        let mut obj = TflObject::new(m.x, m.y, core.collection, perm);
        obj.permutation_index = index as i16;
        obj.scale = size[0];
        obj.facing = m.yaw as u16;
        obj.field_1e = m.pitch as u16;
        self.set_marker_motion(&mut obj, marker);
        obj.kind = 2;
        let monsters = &self.monsters;
        let (index, _) = self
            .monster_pool
            .allocate(|k| monsters.get(k).is_some_and(|m| m.identifier != 0))
            .ok_or_else(|| Stop::new(draw_site::NEW_MONSTER, "the monster list is full"))?;
        let monsters = &self.monsters;
        let identifier = self
            .monster_pool
            .set_identifier_with(index, m.marker_id, |j| {
                monsters.get(j).map_or(0, |mo| mo.identifier)
            })
            .ok_or_else(|| {
                Stop::new(
                    draw_site::UNIT_MARKER_ZERO,
                    "a unit marker with identifier 0",
                )
            })?;
        obj.owner = index as i16;
        let ammunition = if mons.i16(0x82) >= 1 {
            self.rng
                .draw_scaled(draw_site::MONSTER_AMMUNITION, i32::from(mons.i16(0x82)))
        } else {
            0
        };
        let counter = self.mons_counters.entry(mons_id).or_insert(-1);
        if *counter < 0 {
            *counter = (self.rng.draw(draw_site::MONSTER_NAME_COUNTER) & 0x7fff) as i16;
        }
        let count = *counter;
        *counter = counter.wrapping_add(1);
        let mut name_index = (count as u8) & 0x7f;
        if let Some(names) = mons.tag(0x13c) {
            let n = self.data.string_count(names)?;
            if n > 0 {
                name_index = (i32::from(count) % i32::from(n)) as u8;
            }
        }
        obj.interest = 0x19;
        let object = self.add_object(obj)?;
        let mut monster = TflMonster::new(identifier, mons_id, object, team);
        monster.unit = unit_id;
        monster.player = owner as i8;
        let entry_player = if pflags & 0x10000 != 0 {
            -1
        } else {
            owner as i16
        };
        monster.palette = self.statistics.entry(unit_id, entry_player, mons_id);
        let vitality = mons.i16(0x128);
        monster.vitality = vitality;
        monster.max_vitality = vitality;
        self.statistics
            .add_monsters(monster.palette, 1, i32::from(vitality));
        monster.name_index = name_index;
        if pflags & 0x10000 != 0 {
            monster.flags_44 |= 4;
            monster.flags |= 0x1000_0000;
        }
        if index == self.monsters.len() {
            self.monsters.push(monster);
        } else {
            self.monsters[index] = monster;
        }
        if whole {
            self.new_object_state(object)?;
        }
        self.set_monster_action(index, 1, 0)?;
        let f = mons.u32(0);
        let o = &mut self.objects[object];
        o.flags |= 0x40_0400;
        if f & 0x1_0000 != 0 {
            o.flags |= 0x840_0400;
        }
        if f & 0x2000 == 0 {
            o.flags |= 0x400_0100;
        }
        if f & 0x10 != 0 {
            o.flags |= 0x2000;
        }
        if f & 4 != 0 {
            o.flags |= 0x1800;
        }
        if f & 0x800 != 0 {
            self.monsters[index].flags_44 |= 0x20;
        }
        let r = self.rng.draw(draw_site::MONSTER_MIRRORED);
        if r < mons.u16(0x8c) {
            self.objects[object].flags |= 0x20000;
        }
        self.monsters[index].ammunition = (mons.i16(0x80) as i8).wrapping_add(ammunition as i8);
        if let Some(v) = &veteran {
            self.monsters[index].experience = v.experience;
            self.monsters[index].name_index = v.field_13;
            self.monsters[index].field_02 = v.field_11;
            self.monsters[index].name = v.name.clone();
        }
        let t = self.find_target(index)?;
        if t != -1 {
            self.try_and_switch_target(index, t as usize, 10)?;
        }
        if m.flags & 4 != 0 {
            if !whole {
                self.new_object_state(object)?;
            }
            let o = &self.objects[object];
            let ctx = crate::damage::DamageContext {
                attacker_palette: 0,
                attacker: -1,
                attacker_id: 0,
                group: None,
                point: [o.x, o.y, o.z],
                velocity: [0; 3],
                scale: 0,
                force: 0x50,
                amount: 0,
                flags: 0,
                record: None,
            };
            self.kill_monster(index, None, &ctx, 0x26)?;
            return Ok(None);
        }
        Ok(Some(index))
    }

    pub(crate) fn make_monster_visible_new(
        &mut self,
        identifier: i16,
    ) -> Result<Option<(usize, bool)>, Stop> {
        let Some(k) = self
            .data
            .mesh
            .markers
            .iter()
            .position(|m| m.marker_type == UNIT && m.marker_id == identifier)
        else {
            return Ok(None);
        };
        self.data.mesh.markers[k].flags &= !1;
        let existing = marker_monster(&self.data.mesh.markers[k]);
        if existing == -1 {
            let m = self.data.mesh.markers[k].clone();
            let Some(p) = self
                .data
                .mesh
                .palettes
                .iter()
                .filter(|p| p.marker_type == UNIT)
                .nth(m.palette_index.max(0) as usize)
                .cloned()
            else {
                return Ok(None);
            };
            let made = self.new_monster(k, &m, &p, true)?;
            set_marker_monster(&mut self.data.mesh.markers[k], made);
            return Ok(made.map(|k| (k, true)));
        }
        let i = existing as usize;
        match self.monsters.get(i) {
            Some(mo) if mo.identifier == identifier => {
                self.monsters[i].flags &= !0x4000_0000;
                let o = self.monsters[i].object;
                self.objects[o].flags &= !0x200;
                Ok(Some((i, false)))
            }
            _ => Ok(None),
        }
    }
}

fn marker_motion(b: &[u8], count: usize) -> Vec<[i16; 4]> {
    let be32 = |o: usize| {
        b.get(o..o + 4)
            .map_or(0, |v| u32::from_be_bytes([v[0], v[1], v[2], v[3]]) as usize)
    };
    let offset = be32(0x38);
    (0..count)
        .map(|k| {
            let at = 1024 + offset + 64 * k + 0x18;
            let w = |i: usize| {
                b.get(at + 2 * i..at + 2 * i + 2)
                    .map_or(0, |v| i16::from_be_bytes([v[0], v[1]]))
            };
            [w(0), w(1), w(2), w(3)]
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn veteran(player: u32, unit: &[u8; 4], experience: i8) -> MonsterInitializer {
        MonsterInitializer {
            used: false,
            player,
            unit: FourCC(*unit),
            mons: FourCC(*b"mons"),
            experience,
            field_11: 1,
            permutation: 0,
            field_13: 0,
            name: Vec::new(),
        }
    }

    #[test]
    fn a_veteran_is_its_players_or_a_teammates() {
        let players = [
            InitializerPlayer { id: 3, team: 0 },
            InitializerPlayer { id: 5, team: 0 },
            InitializerPlayer { id: 7, team: 1 },
        ];
        let mut list = vec![
            veteran(7, b"warr", 9),
            veteran(3, b"warr", 2),
            veteran(5, b"warr", 4),
            veteran(3, b"warr", 4),
            veteran(9, b"warr", 8),
        ];
        let unit = FourCC(*b"warr");
        let other_mons = FourCC(*b"none");
        let take = |list: &mut Vec<MonsterInitializer>, options, player| {
            get_monster_initializer(list, options, unit, other_mons, player, &players)
        };
        assert_eq!(take(&mut list, 0, Some(0)), None, "option 0x20 clear");
        assert_eq!(take(&mut list, VETERANS, None), None, "no entry player");
        assert_eq!(take(&mut list, VETERANS, Some(0)), Some(2));
        assert_eq!(take(&mut list, VETERANS, Some(1)), Some(3));
        assert_eq!(take(&mut list, VETERANS, Some(0)), Some(1));
        assert_eq!(
            take(&mut list, VETERANS, Some(0)),
            None,
            "net id 7's and 9's aren't"
        );
        assert_eq!(take(&mut list, VETERANS, Some(2)), Some(0));
        assert!(!list[4].used, "no player has net id 9");
    }
}
