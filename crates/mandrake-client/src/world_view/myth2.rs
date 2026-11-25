use mandrake_math::Fixed16;
use mandrake_sim::{CombatEvent, Simulation, UnitEntity};
use mandrake_tag::MonsterSequenceSlot;

use std::collections::HashMap;

use super::{
    MinimapStart, MinimapView, UnitObject, UnitPose, ViewGame, ViewMonster, ViewScenery, ViewUnit,
    WorldView,
};
use crate::corpse::LandscapeEdit;
use crate::interface_state;
use crate::map_load::LOCAL_PLAYER_TEAM;
use crate::media;
use crate::minimap;
use crate::object_history;
use crate::object_sprites::{DrawnObject, MonsterDraw, ObjectKind};
use crate::projfx_log::ProjfxLog;
use crate::scenery::SceneryObject;
use crate::sprite_manager::ClientSpriteManager;
use crate::unit_anim;
use crate::unit_billboards;

pub(crate) fn drawn_world(
    sim: &Simulation,
    animator: &unit_anim::UnitAnimator,
    sprite_mgr: &ClientSpriteManager,
) -> WorldView {
    let mut world = world_view(sim, animator.fraction(), sprite_mgr.projfx_log.as_ref());
    world.monsters = monsters(&world, animator, sprite_mgr);
    world
}

pub(crate) fn drawn_units(
    sim: &Simulation,
    animator: &unit_anim::UnitAnimator,
    sprite_mgr: &ClientSpriteManager,
) -> WorldView {
    let mut world = WorldView {
        units: live_unit_views(sim),
        ..header(sim)
    };
    world.monsters = monsters(&world, animator, sprite_mgr);
    world
}

fn monsters(
    world: &WorldView,
    animator: &unit_anim::UnitAnimator,
    sprite_mgr: &ClientSpriteManager,
) -> Vec<DrawnObject> {
    world
        .units
        .iter()
        .filter_map(|u| unit_object(u, &world.game, animator, sprite_mgr))
        .collect()
}

pub(crate) fn world_view(sim: &Simulation, fraction: i32, trace: Option<&ProjfxLog>) -> WorldView {
    let objects = match trace {
        Some(t) => t.objects_at(sim.current_tick),
        None => object_history::drawn_projectile_views(sim, fraction),
    };
    let parasites = if trace.is_some() {
        Vec::new()
    } else {
        object_history::parasite_views(sim)
            .iter()
            .map(|p| (p.host, DrawnObject::from(&p.view)))
            .collect()
    };
    WorldView {
        objects: objects.iter().map(DrawnObject::from).collect(),
        parasites,
        ..units_view(sim)
    }
}

pub(crate) fn units_view(sim: &Simulation) -> WorldView {
    WorldView {
        units: live_unit_views(sim),
        minimap: minimap_view(sim),
        blips: blip_monsters(&sim.engine),
        ..header(sim)
    }
}

pub(crate) fn header(sim: &Simulation) -> WorldView {
    let zero = sim.world_units_to_sim([0, 0, 0]);
    WorldView {
        tick: sim.current_tick,
        engine_stopped: sim.engine_stop.is_some(),
        game: game(sim),
        origin: [zero.x.to_f32(), zero.z.to_f32()],
        ..WorldView::default()
    }
}

pub(crate) fn game(sim: &Simulation) -> ViewGame {
    let context = mandrake_view::selection::state::GameContext {
        game_mode: sim.engine.mode as u16,
        game_type: sim.engine.game_type as u16,
        local_team: sim.engine.local_team(),
        local_player: sim.engine.local_player as i8,
        controllable_flag: sim
            .engine
            .players
            .local_without_team(sim.engine.local_player),
    };
    ViewGame {
        context,
        alliances: interface_state::game_alliances(&sim.engine),

        underwater: underwater_game(sim),
    }
}

fn underwater_game(sim: &Simulation) -> media::UnderwaterGame {
    let options = sim
        .game_setup
        .as_ref()
        .map_or(mandrake_sim::players::COMMAND_LINE_GAME_OPTIONS, |s| {
            s.options
        });
    media::UnderwaterGame {
        local_team: sim.engine.local_team(),
        mode: sim.engine.mode,
        game_options_high: (options >> 16) as u16,
    }
}

fn live_unit_views(sim: &Simulation) -> Vec<ViewUnit> {
    let mut units: Vec<&UnitEntity> = sim.units.values().filter(|u| u.is_alive()).collect();
    units.sort_by_key(|u| u.id);
    let mut by_id: HashMap<u16, usize> = HashMap::with_capacity(sim.engine.monsters.len());
    for (i, m) in sim.engine.monsters.iter().enumerate() {
        let id = m.identifier as u16;
        if id != 0 {
            by_id.entry(id).or_insert(i);
        }
    }
    units
        .into_iter()
        .map(|u| {
            let id = u.id as u16;
            let index = (id != 0).then(|| by_id.get(&id).copied()).flatten();
            unit_view_with_index(sim, u, index)
        })
        .collect()
}

pub(crate) fn unit_view(sim: &Simulation, unit: &UnitEntity) -> ViewUnit {
    let index = sim.engine.monster_index_by_identifier(unit.id as u16);
    unit_view_with_index(sim, unit, index)
}

fn unit_view_with_index(sim: &Simulation, unit: &UnitEntity, index: Option<usize>) -> ViewUnit {
    let w = &sim.engine;
    let monster = index.map(|i| {
        let m = &w.monsters[i];
        let def = w.monster_definitions.get(m.definition);
        ViewMonster {
            team: m.team,
            owner: m.owner,
            flags: m.flags,
            flags_8: m.flags_8,
            definition_199: def.map_or(0, |d| d.field_199),
            mana: def.map(|d| (w.combat(i).mana, d.ai.word(0x214), d.ai.byte_198)),
        }
    });
    let colour_row = u16::try_from(unit.id).ok().and_then(|_| {
        let i = index?;
        let (tag, player) = w.combat(i).unit;
        Some((mandrake_common::FourCC::new(tag.to_be_bytes()), player))
    });
    ViewUnit {
        id: unit.id,
        archetype: unit.archetype,
        team: unit.team,
        selected: unit.selected,
        position: unit.position(),
        engine_facing: unit.engine_facing(),
        state_slot: unit.state_slot(),
        still: unit.velocity.x == Fixed16::ZERO
            && unit.velocity.y == Fixed16::ZERO
            && unit.velocity.z == Fixed16::ZERO,
        health: unit.health.to_f32(),
        max_health: unit.max_health.to_f32(),
        object: unit.object(),
        monster,
        colour_row,
        pick: None,
    }
}

pub(crate) fn unit_object(
    unit: &ViewUnit,
    game: &ViewGame,
    animator: &unit_anim::UnitAnimator,
    sprite_mgr: &ClientSpriteManager,
) -> Option<DrawnObject> {
    let visual = sprite_mgr.unit_visuals.get(&unit.archetype)?;
    let index = unit_billboards::drawn_unit_sequence_index(unit, visual, animator)?;
    let sequence = visual.sequences.get(&index)?;
    let drawn = animator.drawn(unit);
    let team = unit.monster.map_or(i16::from(unit.team), |m| m.team);
    let flags_8 = unit.monster.map_or(0, |m| m.flags_8);
    Some(DrawnObject {
        kind: ObjectKind::Dormant,
        object: unit.id as i32,
        world: drawn.world,
        position: drawn.position,
        velocity: drawn.motion.map_or([0; 3], |m| m.velocity),
        facing: drawn.facing,
        collection: unit.archetype,
        sequence: u8::try_from(index).ok().filter(|&s| s != 0xff)?,
        frame: drawn.shape.map_or(0, |s| s.frame as i32),
        view: drawn.shape.map_or(0, |s| s.view),
        scale: i32::from(unit_billboards::unit_object_scale(unit, visual)),
        flags: drawn.flags,
        hidden: false,
        mirror_x: drawn.mirrored,
        mirror_y: false,
        permutation: unit.object.map_or(0, |o| o.permutation),
        player: -1,
        shading_slot: drawn.shape.map_or(0, |s| s.shading_slot),
        core: sprite_mgr.unit_core_ids.get(&unit.archetype).copied(),
        transfer: (0, 0, 0),
        monster: Some(MonsterDraw {
            id: unit.id,
            bob_height: drawn.bob_height,
            colour_row: unit.colour_row,
            bounds: unit_billboards::drawn_unit_bounds(unit, visual, sequence, animator),
            still: drawn.motion.map_or(unit.still, |m| m.velocity == [0; 3]),
            definition_199: unit.monster.map_or(0, |m| m.definition_199),
            underwater: game.underwater.monster(team, flags_8),
            outer_ring: drawn.visibility.is_some_and(|v| !v.full),
        }),
    })
}

impl UnitPose for UnitEntity {
    fn id(&self) -> u32 {
        self.id
    }
    fn position(&self) -> [f32; 3] {
        [
            self.position.x.to_f32(),
            self.position.y.to_f32(),
            self.position.z.to_f32(),
        ]
    }
    fn engine_facing(&self) -> u16 {
        unit_anim::client_facing_to_engine(self.facing)
    }
    fn state_slot(&self) -> MonsterSequenceSlot {
        unit_anim::slot_for_state(self.state)
    }
    fn object(&self) -> Option<UnitObject> {
        self.engine_object.map(|o| UnitObject {
            scale: o.scale,
            vitality: o.vitality,
            permutation: o.permutation,
        })
    }
}

pub(crate) fn minimap_view(sim: &Simulation) -> Option<MinimapView> {
    let world = &sim.engine;
    Some(MinimapView {
        visibility: visibility_monsters(world),
        rules: game_blip_rules(world),
        local_team: local_team(world),
        game_options: world.game_options,
    })
}

pub(crate) fn scenery_view(
    world: &mandrake_sim::engine_rng::EngineWorld,
) -> impl Iterator<Item = ViewScenery> + '_ {
    world.scenery.iter().map(|s| ViewScenery {
        marker: s.marker,
        deleted: s.deleted,
        object: scenery_object(s, &world.flag_data),
    })
}

pub(crate) fn scenery_object(
    s: &mandrake_sim::engine_rng::EngineScenery,
    flags: &mandrake_sim::flags::FlagData,
) -> SceneryObject {
    SceneryObject {
        flag_team: (s.object.kind == mandrake_sim::flags::OBJECT_KIND_FLAG)
            .then(|| flags.flag(s.object.owner))
            .flatten()
            .map(|f| f.team),
        scale: s.object.scale.clamp(0, i32::from(u16::MAX)) as u16,
        frame: s.object.frame.max(0) as usize,
        view: usize::from(s.object.view),
        tick: s.object.tick,
        permutation: s.object.permutation,
        sequence: s.object.sequence,
        kind: s.object.kind,
    }
}

pub(crate) fn minimap_start(sim: &Simulation) -> Option<MinimapStart> {
    let world = &sim.engine;
    if std::env::var("MYTH2_MINIMAP_DUMP").is_ok() {
        for (i, m) in world.monsters.iter().enumerate() {
            let d = &world.monster_definitions[m.definition];
            eprintln!(
                "MM monster {i} id {} def {} team {} owner {} pos {:?} flags {:#x}/{:#x} class {} defflags {:#x} stencil {} obj {}",
                m.identifier,
                d.tag,
                m.team,
                m.owner,
                m.position,
                m.flags,
                m.flags_8,
                d.field_1b6,
                d.flags,
                d.ai.word(0x1f0),
                m.object.index
            );
        }
        eprintln!(
            "MM local team {} player {} mode {} opts {:#x}",
            local_team(world),
            world.local_player,
            world.mode,
            world.game_options
        );
    }
    Some(MinimapStart {
        mesh_cells: world.mesh_cells,
        team_colors: team_blip_colors(&world.players),
        team_mode: team_mode_for_new_game(world),
        view: minimap_view(sim)?,
    })
}

pub(crate) fn team_blip_colors(players: &mandrake_sim::players::Players) -> Vec<[u16; 3]> {
    let claimed: Vec<Option<[u16; 3]>> = players
        .teams
        .iter()
        .map(|t| {
            usize::try_from(t.player)
                .ok()
                .and_then(|p| players.colors.get(p))
                .map(|c| [c[0][0], c[0][1], c[0][2]])
        })
        .collect();
    minimap::team_blip_colors(&claimed)
}

pub fn visibility_monsters(
    world: &mandrake_sim::engine_rng::EngineWorld,
) -> Vec<minimap::VisibilityMonster> {
    world
        .monsters
        .iter()
        .enumerate()
        .map(|(i, m)| {
            let def = world.monster_definitions.get(m.definition);
            minimap::VisibilityMonster {
                used: m.identifier != 0,
                flag_8_2: m.flags_8 & 2 != 0,
                flag_7_40: m.flags & 0x4000_0000 != 0,
                class: def.map_or(0, |d| d.field_1b6),
                stencil: def.map_or(-1, |d| d.ai.word(0x1f0)),
                marker_flag_8: world.combat(i).marker_flags & 8 != 0,
                team: m.team,
                object_flag_80: m.object.flags & 0x80 != 0,
                position: m.position,
            }
        })
        .collect()
}

pub fn local_team(world: &mandrake_sim::engine_rng::EngineWorld) -> i16 {
    world.local_team()
}

pub fn solo_replay(world: &mandrake_sim::engine_rng::EngineWorld) -> bool {
    world.replay.is_some_and(|r| r.solo)
}

pub fn team_mode_for_new_game(world: &mandrake_sim::engine_rng::EngineWorld) -> bool {
    (world.mode == 0 && !solo_replay(world)) || (world.mode == 1 && local_team(world) == -1)
}

pub fn blip_monsters(world: &mandrake_sim::engine_rng::EngineWorld) -> Vec<minimap::BlipMonster> {
    world
        .monsters
        .iter()
        .map(|m| {
            let def = world.monster_definitions.get(m.definition);
            minimap::BlipMonster {
                used: m.identifier != 0,
                flag_7_40: m.flags & 0x4000_0000 != 0,
                flag_7_10: m.flags & 0x1000_0000 != 0,
                flag_9_10: m.flags_8 & 0x1000 != 0,
                flag_9_20: m.flags_8 & 0x2000 != 0,
                team: m.team,
                owner: m.owner,
                definition_flags: def.map_or(0, |d| d.flags),
                class: def.map_or(0, |d| d.field_1b6),
                object: m.object.index,
                object_flags: m.object.flags,
                object_33: m.object.sub_kind,
                position: m.position,
            }
        })
        .collect()
}

pub fn game_blip_rules(world: &mandrake_sim::engine_rng::EngineWorld) -> minimap::BlipRules {
    minimap::BlipRules {
        mode: world.mode,
        game_type: world.game_type,
        game_options: world.game_options,
        local_team: local_team(world),
        local_player: world.local_player,
        team_mode: false,
        solo_replay: solo_replay(world),
        allied_selectable: false,
        alliances: world.alliance_view(),
    }
}

const PROJECTILE_FLAG_BLOODY: u32 = 0x8000;

pub(crate) fn landscape_edit(event: &CombatEvent, sim: &Simulation) -> Option<LandscapeEdit> {
    match event {
        CombatEvent::StainMesh { stain, .. } => Some(LandscapeEdit::Stain(*stain)),
        CombatEvent::Footstep { point, .. } => Some(LandscapeEdit::Footstep { point: *point }),
        CombatEvent::ProjectileDetonated {
            tick,
            definition,
            position,
            ..
        } => {
            let flags = sim.engine.catalog.projectiles.get(definition)?.flags;
            (flags & PROJECTILE_FLAG_BLOODY != 0).then(|| LandscapeEdit::Bloody {
                tick: *tick,
                point: sim.world_to_command_point(*position),
            })
        }
        _ => None,
    }
}

impl super::pick::PickSim for Simulation {
    fn could_pick_up(&self, selection: &[u32], object: i32) -> Option<i16> {
        Simulation::could_pick_up(self, selection, object)
    }
}

impl super::pick::Myth2PickSim for Simulation {
    fn drawn_units(
        &self,
        animator: &unit_anim::UnitAnimator,
        sprite_mgr: &ClientSpriteManager,
    ) -> WorldView {
        drawn_units(self, animator, sprite_mgr)
    }

    fn drawn_objects(&self, fraction: i32, trace: Option<&ProjfxLog>) -> WorldView {
        world_view(self, fraction, trace)
    }

    fn monster_pick(&self, id: u32, selected: bool) -> crate::selection_input::MonsterPick {
        use crate::selection_rules;
        (selection_rules::monster_pick(&self.engine, id, selected)).unwrap_or_else(|| {
            let own = self
                .units
                .get(&id)
                .is_some_and(|u| i16::from(u.team) == LOCAL_PLAYER_TEAM);
            selection_rules::fallback_pick(id, own, selected)
        })
    }

    fn under_media(&self, id: u32, margin: i32) -> Option<bool> {
        let w = &self.engine;
        let i = w.monster_index_by_identifier(id as u16)?;
        Some(crate::selection_rules::under_media(w, i, margin))
    }

    fn band_eligible(&self, id: u32) -> bool {
        crate::selection_rules::band_eligible(&self.engine, id)
    }

    fn same_type_flood(&self, base: u32) -> Vec<u32> {
        crate::selection_rules::engine_same_type_flood(&self.engine, base)
    }

    fn all_controlled(&self, selection: &[u32]) -> bool {
        crate::selection_rules::all_controlled(&self.engine, selection)
    }

    fn special_against_friendlies(&self, selection: &[u32]) -> Vec<u32> {
        let w = &self.engine;
        selection
            .iter()
            .copied()
            .filter(|&id| {
                w.monster_index_by_identifier(id as u16).is_some_and(|i| {
                    crate::command_bar::special_against_friendlies(w, i).unwrap_or(false)
                })
            })
            .collect()
    }

    fn unit_location(&self, id: u32) -> Option<(i32, i32)> {
        if let Some(i) = self.engine.monster_index_by_identifier(id as u16) {
            return Some(self.engine.monsters[i].position);
        }
        let u = self.units.get(&id)?;
        Some(self.world_to_command_point(u.position))
    }

    fn command_point(&self, [x, y, z]: [f32; 3]) -> (i32, i32) {
        self.world_to_command_point(mandrake_math::Vec3Fixed::new(
            Fixed16::from_f32(x),
            Fixed16::from_f32(y),
            Fixed16::from_f32(z),
        ))
    }

    fn drawn_models<'a>(
        &'a self,
        region: &mandrake_view::view_region::ViewRegion,
        map: &'a media::MapMedia,
        _mesh_tag: &mandrake_tag::MeshTag,
    ) -> Vec<(
        &'a mandrake_view::model_surfaces::EngineGeometry,
        &'a mandrake_tag::model_hit::ModelPlacement,
    )> {
        let w = &self.engine;
        let frames = w.model_animations.frames();
        let width = w.cells.width;
        let mut seen = vec![false; w.models.len()];
        let mut drawn = Vec::new();
        for row in region.first_row..region.last_row {
            let Some((first, last)) = region.span(row) else {
                continue;
            };
            for col in i32::from(first)..i32::from(last) {
                let Some(&m) = w.cells.models.get((row * width + col) as usize) else {
                    continue;
                };
                let Some(record) = usize::try_from(m).ok().and_then(|i| w.models.get(i)) else {
                    continue;
                };
                if std::mem::replace(&mut seen[m as usize], true) {
                    continue;
                }
                let geometry = match frames.iter().find(|f| f.model == m) {
                    Some(f) => map.animation_frame_geometry(f.marker, f.frame),
                    None => map.placed_model_geometry(&record.placement),
                };
                if let Some(geometry) = geometry {
                    drawn.push((geometry, &record.placement));
                }
            }
        }
        drawn
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn observer_underwater_game_passes_minus_one_local_team() {
        let mut sim = Simulation::command_line_game(1);
        sim.engine.mode = 1;
        sim.engine.local_player = -1;
        let ug = underwater_game(&sim);
        assert_eq!(ug.mode, 1);
        assert_eq!(ug.local_team, -1);
        assert_eq!(ug.monster(2, 0), media::UnderwaterVisibility::Visible);
    }
}
