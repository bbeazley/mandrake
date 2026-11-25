use crate::interface_state;
use crate::sprite_manager::ClientSpriteManager;
use crate::unit_anim::UnitSpriteVisual;
use crate::unit_billboards::unit_original_vitality;
use crate::world_view::{ViewGame, ViewUnit, WorldView};
use mandrake_tag::InterfaceColorTable;
use mandrake_view::{billboard::BillboardInstance, camera::Camera, selection};

#[derive(Clone, Copy, Debug)]
pub(crate) struct UnitOverlay {
    pub(crate) state: selection::state::OverlayState,
    pub(crate) object_flags: u32,
    pub(crate) frame_color: usize,
    pub(crate) outer_frame: bool,
    pub(crate) submerged: bool,
    pub(crate) thickness: i32,
    pub(crate) mana: Option<selection::mana::MonsterMana>,
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn unit_selection_overlays(
    unit: &ViewUnit,
    position: [f32; 3],
    visual: &UnitSpriteVisual,
    bounds: selection::ObjectBounds,
    colors: &InterfaceColorTable,
    camera: &Camera,
    viewport: (u32, u32),
    overlay: UnitOverlay,
) -> Vec<BillboardInstance> {
    let frame_bounds = if overlay.object_flags & selection::state::OBJECT_FLIPPED != 0 {
        bounds.flipped()
    } else {
        bounds
    };
    let Some(rect) = frame_rect(position, frame_bounds, camera, viewport) else {
        return Vec::new();
    };
    let obje_frame = visual.object.is_none_or(|d| d.has_selection_frame());
    let frame = selection::state::draws_frame(overlay.state, obje_frame, overlay.object_flags)
        .then_some(FrameLook {
            color: overlay.frame_color,
            thickness: overlay.thickness,
            outer: overlay.outer_frame,
        });
    let bar = visual.object.filter(|d| d.has_vitality_bar()).map(|def| {
        let original = unit_original_vitality(unit, &def);
        let max_health = unit.max_health;
        let fraction = if max_health > 0.0 {
            (unit.health / max_health).clamp(0.0, 1.0)
        } else {
            0.0
        };
        let vitality = selection::ObjectVitality {
            current: (fraction * original as f32) as i16,
            maximum: original,
            original,
        };
        selection::vitality_bar(rect, bounds, vitality)
    });
    overlay_rectangles(
        &ObjectOverlay {
            rect,
            depth_anchor: position,
            transparency: selection::state::overlay_transparency(overlay.state, overlay.submerged),
            frame,
            bar,
            mana: overlay.mana,
        },
        colors,
    )
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct FrameLook {
    pub(crate) color: usize,
    pub(crate) thickness: i32,
    pub(crate) outer: bool,
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct ObjectOverlay {
    pub(crate) rect: selection::PixelRect,
    pub(crate) depth_anchor: [f32; 3],
    pub(crate) transparency: u16,
    pub(crate) frame: Option<FrameLook>,
    pub(crate) bar: Option<selection::VitalityBar>,
    pub(crate) mana: Option<selection::mana::MonsterMana>,
}

pub(crate) fn frame_rect(
    position: [f32; 3],
    bounds: selection::ObjectBounds,
    camera: &Camera,
    viewport: (u32, u32),
) -> Option<selection::PixelRect> {
    let proj = camera.frame_projection(position, viewport.0, viewport.1)?;
    selection::selection_frame_rect(bounds, &proj)
}

pub(crate) fn overlay_rectangles(
    overlay: &ObjectOverlay,
    colors: &InterfaceColorTable,
) -> Vec<BillboardInstance> {
    let mut out = Vec::new();
    let rect = overlay.rect;
    let alpha = selection::overlay_alpha(overlay.transparency) as f32 / 255.0;
    let with_alpha = |c: [f32; 4]| [c[0], c[1], c[2], alpha];
    let mut push = |r: selection::PixelRect, color: [f32; 4]| {
        if !r.is_empty() {
            out.push(BillboardInstance::interface_rect(
                overlay.depth_anchor,
                r,
                color,
            ));
        }
    };

    if let Some(frame) = overlay.frame {
        let color = with_alpha(colors.color_f32(frame.color));
        for edge in selection::frame_edges(rect, frame.thickness) {
            push(edge, color);
        }
        if frame.outer {
            let [r, g, b] = selection::state::OUTER_FRAME_COLOR;
            let a = selection::overlay_alpha(selection::state::OUTER_FRAME_TRANSPARENCY);
            let color = [r, g, b, a].map(|v| v as f32 / 255.0);
            for edge in selection::state::outer_frame_edges(rect, frame.thickness) {
                push(edge, color);
            }
        }
    }

    if let Some(bar) = overlay.bar {
        for (seg, color) in bar.segments {
            let base = match color {
                selection::BarColor::Fill => mandrake_tag::interface::COLOR_VITALITY_FILL_BASE,
                selection::BarColor::Background => {
                    mandrake_tag::interface::COLOR_VITALITY_BACKGROUND_BASE
                }
            };
            push(seg, with_alpha(colors.color_f32(base + bar.step)));
        }
    }

    if let Some(mana) = overlay.mana {
        let [r, g, b] = selection::mana::BAR_COLOR;
        let color = with_alpha([r, g, b, 0xff].map(|v| v as f32 / 255.0));
        push(selection::mana::bar(rect, &mana), color);
    }
    out
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct TflMonsterOverlay {
    pub(crate) eval: u32,
    pub(crate) flags: u32,
    pub(crate) vitality: selection::tfl::MonsterVitality,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct TflFrameStyle {
    pub(crate) hovered_only: bool,
    pub(crate) frame: bool,
    pub(crate) thick: bool,
    pub(crate) submerged: bool,
}

impl TflFrameStyle {
    #[cfg(test)]
    const SELECTED: Self = Self {
        hovered_only: false,
        frame: true,
        thick: false,
        submerged: false,
    };
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn tfl_selection_overlays(
    position: [f32; 3],
    bounds: selection::ObjectBounds,
    flipped: bool,
    monster: Option<TflMonsterOverlay>,
    style: TflFrameStyle,
    colors: &InterfaceColorTable,
    camera: &Camera,
    viewport: (u32, u32),
) -> Vec<BillboardInstance> {
    let frame_bounds = if flipped { bounds.flipped() } else { bounds };
    let Some(rect) = frame_rect(position, frame_bounds, camera, viewport) else {
        return Vec::new();
    };
    let state = selection::state::OverlayState {
        transparency: if style.hovered_only {
            selection::TRANSPARENCY_HIGHLIGHTED
        } else {
            selection::TRANSPARENCY_SELECTED
        },
        frame: style.frame,
    };
    overlay_rectangles(
        &ObjectOverlay {
            rect,
            depth_anchor: position,
            transparency: selection::state::overlay_transparency(state, style.submerged),
            frame: style.frame.then(|| FrameLook {
                color: selection::tfl::frame_color_index(monster.map(|m| (m.eval, m.flags))),
                thickness: selection::frame_thickness(style.thick),
                outer: false,
            }),
            bar: monster.and_then(|m| selection::tfl::vitality_bar(rect, bounds, m.vitality)),
            mana: None,
        },
        colors,
    )
}

pub(crate) fn object_overlay(
    game: &ViewGame,
    index: i32,
    flags: u32,
    iface: &interface_state::InterfaceView,
) -> Option<UnitOverlay> {
    if !iface.targets.is_object_targeted(index) {
        return None;
    }
    let object_flags =
        selection::state::OBJECT_TARGETED | (flags & selection::state::OBJECT_FLIPPED);
    let state = selection::state::overlay_state(
        &game.context,
        &game.alliances,
        selection::state::OverlayObject {
            flags: object_flags,
            monster: None,
        },
        iface.caps_lock,
    )?;
    Some(UnitOverlay {
        state,
        object_flags,
        frame_color: selection::state::frame_color_index(&game.context, None),
        outer_frame: false,
        submerged: false,
        thickness: selection::frame_thickness(iface.tfl_selection_boxes),
        mana: None,
    })
}

pub(crate) fn object_selection_overlays(
    position: [f32; 3],
    bounds: selection::ObjectBounds,
    colors: &InterfaceColorTable,
    camera: &Camera,
    viewport: (u32, u32),
    overlay: UnitOverlay,
) -> Vec<BillboardInstance> {
    let frame_bounds = if overlay.object_flags & selection::state::OBJECT_FLIPPED != 0 {
        bounds.flipped()
    } else {
        bounds
    };
    let Some(rect) = frame_rect(position, frame_bounds, camera, viewport) else {
        return Vec::new();
    };
    let frame = selection::state::draws_frame(overlay.state, false, overlay.object_flags)
        .then_some(FrameLook {
            color: overlay.frame_color,
            thickness: overlay.thickness,
            outer: false,
        });
    overlay_rectangles(
        &ObjectOverlay {
            rect,
            depth_anchor: position,
            transparency: selection::state::overlay_transparency(overlay.state, overlay.submerged),
            frame,
            bar: None,
            mana: None,
        },
        colors,
    )
}

fn overlay_monster(
    ctx: &selection::state::GameContext,
    unit: &ViewUnit,
    placed_raw: Option<(i16, u32)>,
    definition: (u32, i16),
) -> selection::state::MonsterInterface {
    let placed = placed_raw.map_or((0, 0), |(palette_flags, marker_flags)| {
        interface_state::placed_monster_flags(palette_flags, marker_flags, ctx.game_type)
    });
    let mut monster =
        interface_state::monster_interface(unit.team, ctx.local_team, placed, definition);
    if let Some(m) = unit.monster {
        monster.team = m.team as i8;
        monster.player = m.owner as i8;
        monster.flags = m.flags;
        monster.flags8 = m.flags_8 | (placed.1 & selection::state::MONSTER_FLAGS8_OUTER_FRAME);
    }
    monster
}

pub(crate) fn unit_overlay(
    game: &ViewGame,
    unit: &ViewUnit,
    visual: &UnitSpriteVisual,
    sprite_mgr: &ClientSpriteManager,
    iface: &interface_state::InterfaceView,
    submerged: bool,
) -> Option<UnitOverlay> {
    let (ctx, alliances) = (&game.context, &game.alliances);
    let monster = overlay_monster(
        ctx,
        unit,
        sprite_mgr.placed_monster_flags.get(&unit.id).copied(),
        visual.definition,
    );
    let object_flags = iface.object_flags(unit.id, unit.selected);
    let state = selection::state::overlay_state(
        ctx,
        alliances,
        selection::state::OverlayObject {
            flags: object_flags,
            monster: Some(&monster),
        },
        iface.caps_lock,
    )?;
    let eval = selection::state::evaluate_monster(ctx, alliances, &monster);
    let mana = monster_mana(unit, monster.team).filter(|m| selection::mana::draws_bar(ctx, m));
    Some(UnitOverlay {
        state,
        object_flags,
        frame_color: selection::state::frame_color_index(ctx, Some((&monster, eval))),
        outer_frame: selection::state::draws_outer_frame(ctx, Some(&monster)),
        submerged,
        thickness: selection::frame_thickness(iface.tfl_selection_boxes),
        mana,
    })
}

fn monster_mana(unit: &ViewUnit, team: i8) -> Option<selection::mana::MonsterMana> {
    let (current, maximum, definition_flags) = unit.monster?.mana?;
    Some(selection::mana::MonsterMana {
        current,
        maximum,
        definition_flags,
        team,
    })
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct MoveGesture {
    pub(crate) origin: [f32; 3],
    pub(crate) on_selection: bool,
    pub(crate) facing: Option<f32>,
}

impl MoveGesture {
    pub(crate) fn drag_to(&mut self, ground: [f32; 3]) {
        let dx = ground[0] - self.origin[0];
        let dz = ground[2] - self.origin[2];
        self.facing = Some(dz.atan2(dx));
    }
}

pub(crate) fn facing_gesture_arrows(
    gesture: &MoveGesture,
    view: &WorldView,
    colors: &InterfaceColorTable,
) -> Vec<BillboardInstance> {
    facing_arrows_at(
        gesture,
        view.units.iter().filter(|u| u.selected).map(|u| u.position),
        colors,
    )
}

pub(crate) fn facing_arrows_at(
    gesture: &MoveGesture,
    selected: impl IntoIterator<Item = [f32; 3]>,
    colors: &InterfaceColorTable,
) -> Vec<BillboardInstance> {
    let Some(yaw) = gesture.facing else {
        return Vec::new();
    };
    let color = colors.color_f32(mandrake_tag::interface::COLOR_FACING_ARROW);
    if !gesture.on_selection {
        return vec![BillboardInstance::facing_arrow(gesture.origin, yaw, color)];
    }
    selected
        .into_iter()
        .map(|p| BillboardInstance::facing_arrow(p, yaw, color))
        .collect()
}

pub(crate) fn band_rectangle_overlays(
    bounds: selection::PixelRect,
    colors: &InterfaceColorTable,
    viewport: (u32, u32),
) -> Vec<BillboardInstance> {
    let viewport = [viewport.0 as i32, viewport.1 as i32];
    selection::band::rectangles(bounds, viewport, true)
        .into_iter()
        .filter(|(r, _)| !r.is_empty())
        .map(|(rect, part)| {
            let alpha = selection::overlay_alpha(part.transparency()) as f32 / 255.0;
            let [r, g, b, _] = match part {
                selection::band::Part::Edge => colors.color_f32(selection::band::EDGE_COLOR_INDEX),
                selection::band::Part::Fill => {
                    let [r, g, b] = selection::band::FILL_COLOR.map(|c| c as f32 / 255.0);
                    [r, g, b, 1.0]
                }
            };
            BillboardInstance::screen_overlay_rect(rect, [r, g, b, alpha])
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use mandrake_view::billboard::BILLBOARD_FLAG_INTERFACE_RECT;
    use mandrake_view::selection::tfl::MonsterVitality;
    use mandrake_view::selection::{ObjectBounds, PixelRect};

    fn px(b: &BillboardInstance) -> PixelRect {
        let [l, t, r, bt] = [b.uv_min[0], b.uv_min[1], b.uv_max[0], b.uv_max[1]].map(|v| v as i32);
        PixelRect::new(l, t, r, bt)
    }

    #[test]
    fn tfl_overlays_are_the_frame_and_the_bar() {
        let colors = InterfaceColorTable::from_colors(
            (0..64u8).map(|i| [i, i, i, 255]).collect(),
            [0, 0, 0, 255],
        );
        let camera = Camera::new([0.0, 0.0, 0.0], 12.0, 0.7, 0.6);
        let bounds = ObjectBounds {
            half_width: 120,
            bottom: -70,
            top: 580,
        };
        let monster = |eval, current| TflMonsterOverlay {
            eval,
            flags: 0,
            vitality: MonsterVitality {
                current,
                maximum: 3000,
                definition: 3000,
            },
        };
        let at = [0.0, 0.0, 0.0];
        let size = (1280, 720);
        let rect = frame_rect(at, bounds, &camera, size).expect("on screen");
        let selected = TflFrameStyle::SELECTED;
        let draw =
            |m| tfl_selection_overlays(at, bounds, false, m, selected, &colors, &camera, size);

        let own = draw(Some(monster(7, 1500)));
        assert_eq!(own.len(), 7);
        let alpha = selection::overlay_alpha(selection::TRANSPARENCY_SELECTED) as f32 / 255.0;
        assert_eq!(
            selection::overlay_alpha(selection::TRANSPARENCY_SELECTED),
            0xaa
        );
        for b in &own {
            assert_eq!(b.flags, BILLBOARD_FLAG_INTERFACE_RECT);
            assert_eq!(b.position, at);
            assert_eq!(b.tint[3], alpha);
        }
        let edges: Vec<PixelRect> = own[..4].iter().map(px).collect();
        assert_eq!(edges, selection::frame_edges(rect, 1));
        let shade = |b: &BillboardInstance| (b.tint[0] * 255.0).round() as usize;
        assert!(own[..4].iter().all(|b| shade(b) == 2), "the local player's");
        let bar = selection::tfl::vitality_bar(rect, bounds, monster(7, 1500).vitality).unwrap();
        let segments: Vec<PixelRect> = own[4..].iter().map(px).collect();
        assert_eq!(segments, bar.segments.map(|s| s.0));
        assert_eq!(
            own[4..].iter().map(shade).collect::<Vec<_>>(),
            [0x0c + 10, 0x22 + 10, 0x0c + 10]
        );
        assert_eq!(segments[0].left, rect.right + 2);
        assert_eq!(segments[0].right, rect.right + 8);

        assert_eq!(draw(Some(monster(7, 3000))).len(), 6);
        assert_eq!(shade(&draw(Some(monster(0, 3000)))[0]), 3);
        let object = draw(None);
        assert_eq!(object.len(), 4);
        assert_eq!(shade(&object[0]), 5);
        let flipped =
            tfl_selection_overlays(at, bounds, true, None, selected, &colors, &camera, size);
        let turned = frame_rect(at, bounds.flipped(), &camera, size).expect("on screen");
        assert_eq!(px(&flipped[0]), selection::frame_edges(turned, 1)[0]);

        let hovered = tfl_selection_overlays(
            at,
            bounds,
            false,
            Some(monster(0, 1500)),
            TflFrameStyle {
                hovered_only: true,
                ..selected
            },
            &colors,
            &camera,
            size,
        );
        assert_eq!(hovered.len(), 7);
        assert_eq!(shade(&hovered[0]), 3, "another team's: red");
        let faint = selection::overlay_alpha(0xaaaa) as f32 / 255.0;
        assert!(hovered.iter().all(|b| b.tint[3] == faint));

        let bar_only = tfl_selection_overlays(
            at,
            bounds,
            false,
            Some(monster(7, 1500)),
            TflFrameStyle {
                hovered_only: true,
                frame: false,
                ..selected
            },
            &colors,
            &camera,
            size,
        );
        assert_eq!(bar_only.iter().map(px).collect::<Vec<_>>(), segments);

        let thick = tfl_selection_overlays(
            at,
            bounds,
            false,
            Some(monster(7, 1500)),
            TflFrameStyle {
                thick: true,
                ..selected
            },
            &colors,
            &camera,
            size,
        );
        let edges: Vec<PixelRect> = thick[..4].iter().map(px).collect();
        assert_eq!(edges, selection::frame_edges(rect, 2));
        assert!(thick.iter().all(|b| b.tint[3] == alpha));
        let deep =
            selection::overlay_alpha(selection::state::TRANSPARENCY_SUBMERGED) as f32 / 255.0;
        assert_eq!(selection::overlay_alpha(0xd554), 0x2a);
        for hovered_only in [false, true] {
            let submerged = tfl_selection_overlays(
                at,
                bounds,
                false,
                Some(monster(7, 1500)),
                TflFrameStyle {
                    hovered_only,
                    submerged: true,
                    ..selected
                },
                &colors,
                &camera,
                size,
            );
            assert_eq!(submerged.len(), 7);
            assert!(submerged.iter().all(|b| b.tint[3] == deep));
        }
    }

    #[test]
    fn a_pick_up_target_is_framed_for_seven_ticks() {
        let colors = InterfaceColorTable::from_colors(
            (0..64u8).map(|i| [i, i, i, 255]).collect(),
            [0, 0, 0, 255],
        );
        let camera = Camera::new([0.0, 0.0, 0.0], 12.0, 0.7, 0.6);
        let bounds = ObjectBounds {
            half_width: 40,
            bottom: -10,
            top: 60,
        };
        let (at, size) = ([0.0, 0.0, 0.0], (1280, 720));
        let game = ViewGame::default();
        let mut iface = interface_state::InterfaceView::default();
        assert!(object_overlay(&game, 960, 0, &iface).is_none());
        iface.targets.pick_up(960);
        assert!(
            object_overlay(&game, 961, 0, &iface).is_none(),
            "another object"
        );
        let overlay = object_overlay(&game, 960, 0, &iface).expect("the target is framed");
        let drawn = object_selection_overlays(at, bounds, &colors, &camera, size, overlay);
        assert_eq!(drawn.len(), 4, "the frame alone");
        let rect = frame_rect(at, bounds, &camera, size).expect("on screen");
        let edges: Vec<PixelRect> = drawn.iter().map(px).collect();
        assert_eq!(edges, selection::frame_edges(rect, 1));
        let alpha = selection::overlay_alpha(selection::TRANSPARENCY_SELECTED) as f32 / 255.0;
        for b in &drawn {
            assert_eq!(b.flags, BILLBOARD_FLAG_INTERFACE_RECT);
            assert_eq!((b.tint[0] * 255.0).round() as usize, 5);
            assert_eq!(b.tint[3], alpha);
        }
        let flipped = object_overlay(&game, 960, selection::state::OBJECT_FLIPPED, &iface).unwrap();
        let turned = frame_rect(at, bounds.flipped(), &camera, size).expect("on screen");
        assert_eq!(
            px(&object_selection_overlays(at, bounds, &colors, &camera, size, flipped)[0]),
            selection::frame_edges(turned, 1)[0]
        );
        for _ in 0..6 {
            iface.targets.update(1);
        }
        assert!(object_overlay(&game, 960, 0, &iface).is_some(), "tick 6");
        iface.targets.update(1);
        assert!(
            object_overlay(&game, 960, 0, &iface).is_none(),
            "gone at tick 7"
        );
    }

    #[test]
    fn unit_overlay_uses_game_type_local_team_and_runtime_monster_flags() {
        let ctx = selection::state::GameContext {
            game_mode: 1,
            game_type: selection::state::GAME_TYPE_HUNTED,
            local_team: 1,
            local_player: 1,
            controllable_flag: false,
        };
        let placed_raw = Some((
            interface_state::PALETTE_FLAG_HUNTED,
            interface_state::MARKER_FLAG_OUTER_FRAME,
        ));
        let base_unit = ViewUnit {
            id: 42,
            archetype: mandrake_common::FourCC::new(*b"wam1"),
            team: 1,
            selected: false,
            position: [0.0; 3],
            engine_facing: 0,
            state_slot: mandrake_tag::MonsterSequenceSlot(0),
            still: true,
            health: 10.0,
            max_health: 10.0,
            object: None,
            monster: None,
            colour_row: None,
            pick: None,
        };
        let fallback = overlay_monster(&ctx, &base_unit, placed_raw, (0, 0));
        assert_eq!(fallback.player, interface_state::LOCAL_PLAYER);
        assert_ne!(
            fallback.flags & selection::state::MONSTER_FLAG_ALLY_COLORED,
            0
        );
        assert_ne!(fallback.flags8 & selection::state::MONSTER_FLAGS8_HUNTED, 0);
        assert_ne!(
            fallback.flags8 & selection::state::MONSTER_FLAGS8_OUTER_FRAME,
            0
        );
        let alliances = selection::state::Alliances::new(4);
        let eval = selection::state::evaluate_monster(&ctx, &alliances, &fallback);
        assert_eq!(
            selection::state::frame_color_index(&ctx, Some((&fallback, eval))),
            mandrake_tag::interface::COLOR_SELECTION_ENEMY
        );

        let mut runtime_unit = base_unit;
        runtime_unit.monster = Some(crate::world_view::ViewMonster {
            team: 2,
            owner: 2,
            flags: selection::state::MONSTER_FLAG_ALLY_COLORED,
            flags_8: selection::state::MONSTER_FLAGS8_HUNTED,
            definition_199: 0,
            mana: None,
        });
        let runtime = overlay_monster(&ctx, &runtime_unit, placed_raw, (0, 0));
        assert_eq!(runtime.team, 2);
        assert_eq!(runtime.player, 2);
        assert_ne!(
            runtime.flags8 & selection::state::MONSTER_FLAGS8_OUTER_FRAME,
            0
        );
        let eval = selection::state::evaluate_monster(&ctx, &alliances, &runtime);
        assert_eq!(
            selection::state::frame_color_index(&ctx, Some((&runtime, eval))),
            mandrake_tag::interface::COLOR_SELECTION_ENEMY
        );
    }
}
