use crate::app::input::myth_key;
use crate::app::tick::window_title;
use crate::controls;
use crate::controls::{ControlAction, Controls, MythKey};
use crate::overlays::{
    MoveGesture, UnitOverlay, band_rectangle_overlays, facing_gesture_arrows,
    unit_selection_overlays,
};
use crate::region_view::retain_region_objects;
use crate::scenery;
use crate::sprite_manager::object_bitmap_world_size;
use crate::unit_anim::{SequenceVisual, UnitSpriteVisual};
use crate::unit_billboards::{unit_object_scale, unit_original_vitality};
use mandrake_common::FourCC;
use mandrake_math::{Fixed16, Vec3Fixed};
use mandrake_sim::Simulation;
use mandrake_tag::{InterfaceColorTable, MeshTag};
use mandrake_view::{billboard::BillboardInstance, camera::Camera, selection};
pub(crate) fn test_gpu() -> Option<(wgpu::Device, wgpu::Queue)> {
    static ADAPTER: std::sync::OnceLock<Option<wgpu::Adapter>> = std::sync::OnceLock::new();
    let adapter = ADAPTER
        .get_or_init(|| {
            let instance = wgpu::Instance::default();
            pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::default(),
                compatible_surface: None,
                force_fallback_adapter: false,
                apply_limit_buckets: false,
            }))
            .ok()
        })
        .as_ref()?;
    pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor::default())).ok()
}

#[test]
fn the_window_title_shows_the_tick_only_during_a_replay() {
    assert_eq!(
        window_title("Salvation", Some(1234)),
        "Myth II: Soulblighter - Salvation - tick 1234"
    );
    assert_eq!(
        window_title("Salvation", None),
        "Myth II: Soulblighter - Salvation"
    );
}

#[test]
fn objects_outside_the_view_region_are_not_drawn() {
    use mandrake_view::billboard::{BILLBOARD_FLAG_CONNECTOR, BILLBOARD_FLAG_SCREEN_OVERLAY};
    use mandrake_view::view_region::ViewRegion;
    let mut mesh = MeshTag::from_bytes(&[0u8; 1024]).ok().unwrap_or_else(|| {
        panic!("an empty mesh header parses");
    });
    mesh.cell_width = 4;
    mesh.cell_height = 4;
    let frame = mandrake_view::model_surfaces::WorldFrame::new(4, 4);
    let region = ViewRegion {
        first_row: 2,
        last_row: 3,
        spans: vec![(1, 2), (0x7fff, -0x8000)],
        built: Vec::new(),
        rect_last_row: 3,
    };
    let at = |col: f32, row: f32, flags: u32| {
        let mut b = BillboardInstance::new(
            frame.from_engine([col * 512.0 + 100.0, row * 512.0 + 100.0, 0.0]),
            [1.0, 1.0],
            [0.0; 2],
            [1.0; 2],
        );
        b.flags |= flags;
        b
    };
    let mut billboards = vec![
        at(1.0, 2.0, 0),
        at(2.0, 2.0, 0),
        at(3.0, 2.0, 0),
        at(
            1.0,
            2.0,
            mandrake_view::billboard::BILLBOARD_FLAG_REFLECTION,
        ),
        at(3.0, 0.0, BILLBOARD_FLAG_CONNECTOR),
        at(3.0, 0.0, BILLBOARD_FLAG_SCREEN_OVERLAY),
    ];
    retain_region_objects(&mut billboards, Some(&region), &mesh);
    assert_eq!(billboards.len(), 5);
    assert!(
        billboards
            .iter()
            .all(|b| b.position != at(3.0, 2.0, 0).position)
    );
}

#[test]
fn test_scenery_and_shadow_bitmap_world_size() {
    let frame = mandrake_tag::BitmapFrame {
        index: 0,
        name: String::new(),
        width: 27,
        height: 41,
        offset_x: 0,
        offset_y: 0,
        flags: 0,
        pixel_data_offset: 0,
        pixel_data_size: 0,
        pixels: Vec::new(),
        wide_pixels: Vec::new(),
    };
    let seq = mandrake_tag::CollectionSequence {
        index: 8,
        name: String::new(),
        flags: 0,
        pixels_to_world: 14,
        num_views: 1,
        frames_per_view: 1,
        ticks_per_frame: 0,
        key_frame: 0,
        loop_frame: 0,
        bitmap_indices: vec![0],
        shadow_indices: Vec::new(),
        bounds: mandrake_tag::SequenceBounds {
            pixels_to_world: 0x10000,
            ..Default::default()
        },
    };
    let [w, h] = object_bitmap_world_size(&frame, &seq, 14, 0xe5);
    let per_px = 10.0 * (0xe5 as f32 / 256.0) / 512.0;
    assert!((w - 27.0 * per_px).abs() < 1e-6);
    assert!((h - 41.0 * per_px).abs() < 1e-6);
}

#[test]
fn test_unit_object_scale_and_vitality_fall_back_to_the_obje_lower_bounds() {
    let def = mandrake_tag::ObjectDefinition {
        tag_id: FourCC::new(*b"deoc"),
        flags: 0,
        vitality: 256,
        vitality_delta: 257,
        scale: 229,
        scale_delta: 53,
    };
    let visual = UnitSpriteVisual {
        object: Some(def),
        ..Default::default()
    };
    let mut sim = Simulation::new(0);
    let id = sim.spawn_unit(FourCC::new(*b"deer"), 1, Vec3Fixed::ZERO);
    let unit = sim.units[&id].clone();
    assert_eq!(unit_object_scale(&unit, &visual), 229);
    assert_eq!(unit_original_vitality(&unit, &def), 256);
    let mut drawn = unit;
    drawn.engine_object = Some(mandrake_sim::EngineUnitObject {
        scale: 274,
        vitality: 350,
        permutation: 0,
    });
    assert_eq!(unit_object_scale(&drawn, &visual), 274);
    assert_eq!(unit_original_vitality(&drawn, &def), 350);
    let k = scenery::unit_scale_factor(unit_object_scale(&drawn, &visual), def.scale);
    assert!((k - 274.0 / 229.0).abs() < 1e-6);
}

#[test]
fn test_move_gesture_drag_sets_facing() {
    let mut gesture = MoveGesture {
        origin: [10.0, 1.0, 20.0],
        on_selection: false,
        facing: None,
    };
    gesture.drag_to([10.1, 1.0, 20.1]);
    let yaw = gesture.facing.unwrap();
    assert!((yaw - std::f32::consts::FRAC_PI_4).abs() < 1e-3);
    gesture.drag_to([10.0, 1.3, 23.0]);
    let yaw = gesture.facing.unwrap();
    assert!((yaw - std::f32::consts::FRAC_PI_2).abs() < 1e-5);
}

#[test]
fn test_facing_gesture_arrows_placement_and_color() {
    let mut sim = Simulation::new(3);
    let a = sim.spawn_unit(
        FourCC::new(*b"wam1"),
        0,
        Vec3Fixed::new(Fixed16::from_i32(5), Fixed16::ZERO, Fixed16::from_i32(6)),
    );
    let _b = sim.spawn_unit(
        FourCC::new(*b"wam1"),
        0,
        Vec3Fixed::new(Fixed16::from_i32(8), Fixed16::ZERO, Fixed16::from_i32(6)),
    );
    sim.units.get_mut(&a).unwrap().selected = true;
    let colors =
        InterfaceColorTable::from_colors(vec![[0, 0, 0, 255], [200, 100, 50, 255]], [0, 0, 0, 255]);

    let mut gesture = MoveGesture {
        origin: [1.0, 0.5, 2.0],
        on_selection: false,
        facing: None,
    };
    assert!(
        facing_gesture_arrows(
            &gesture,
            &crate::world_view::myth2::units_view(&sim),
            &colors
        )
        .is_empty()
    );

    gesture.facing = Some(0.5);
    let arrows = facing_gesture_arrows(
        &gesture,
        &crate::world_view::myth2::units_view(&sim),
        &colors,
    );
    assert_eq!(arrows.len(), 1);
    assert_eq!(arrows[0].position, [1.0, 0.5, 2.0]);
    assert_eq!(arrows[0].anchor[0], 0.5);
    assert_eq!(
        arrows[0].tint,
        colors.color_f32(mandrake_tag::interface::COLOR_FACING_ARROW)
    );

    gesture.on_selection = true;
    let arrows = facing_gesture_arrows(
        &gesture,
        &crate::world_view::myth2::units_view(&sim),
        &colors,
    );
    assert_eq!(arrows.len(), 1);
    assert_eq!(arrows[0].position, [5.0, 0.0, 6.0]);
}

#[test]
fn test_physical_keys_reach_every_default_binding() {
    use winit::keyboard::KeyCode::*;
    let all = [
        Escape,
        F1,
        F2,
        F3,
        F4,
        F5,
        F6,
        F7,
        F8,
        F9,
        F10,
        F11,
        F12,
        F13,
        F14,
        F15,
        Backquote,
        Digit1,
        Digit2,
        Digit3,
        Digit4,
        Digit5,
        Digit6,
        Digit7,
        Digit8,
        Digit9,
        Digit0,
        Minus,
        Equal,
        Backspace,
        Tab,
        KeyQ,
        KeyW,
        KeyE,
        KeyR,
        KeyT,
        KeyY,
        KeyU,
        KeyI,
        KeyO,
        KeyP,
        BracketLeft,
        BracketRight,
        Backslash,
        CapsLock,
        KeyA,
        KeyS,
        KeyD,
        KeyF,
        KeyG,
        KeyH,
        KeyJ,
        KeyK,
        KeyL,
        Semicolon,
        Quote,
        Enter,
        ShiftLeft,
        KeyZ,
        KeyX,
        KeyC,
        KeyV,
        KeyB,
        KeyN,
        KeyM,
        Comma,
        Period,
        Slash,
        ControlLeft,
        AltLeft,
        Space,
        ArrowUp,
        ArrowDown,
        ArrowLeft,
        ArrowRight,
        Insert,
        Home,
        PageUp,
        Delete,
        End,
        PageDown,
        NumLock,
        NumpadEqual,
        NumpadDivide,
        NumpadMultiply,
        Numpad0,
        Numpad1,
        Numpad2,
        Numpad3,
        Numpad4,
        Numpad5,
        Numpad6,
        Numpad7,
        Numpad8,
        Numpad9,
        NumpadSubtract,
        NumpadAdd,
        NumpadEnter,
        NumpadDecimal,
    ];
    let reachable: std::collections::HashSet<MythKey> =
        all.iter().filter_map(|&k| myth_key(k)).collect();
    for set in controls::PRESETS {
        let c = Controls::preset(set).unwrap();
        for &action in ControlAction::ALL {
            for b in c.bindings(action) {
                assert!(reachable.contains(&b.key), "{set}: {b} unreachable");
            }
        }
    }
    assert_eq!(myth_key(KeyW), Some(MythKey(0x20)));
    assert_eq!(myth_key(KeyA), Some(MythKey(0x2d)));
    assert_eq!(myth_key(Numpad8), Some(MythKey(0x5e)));
    assert_eq!(myth_key(SuperLeft), Some(MythKey::ALT_COMMAND));
    assert_eq!(myth_key(Fn), None);
}

#[test]
fn test_unit_selection_overlay_states() {
    use mandrake_view::selection::{state as st, state::OverlayState};
    let colors = InterfaceColorTable::from_colors(
        (0..0x38u8)
            .map(|i| [i, i.wrapping_mul(2), i.wrapping_mul(3), 255])
            .collect(),
        [0, 0, 0, 255],
    );
    let seq = mandrake_tag::CollectionSequence {
        index: 0,
        name: String::new(),
        flags: 0,
        pixels_to_world: 0,
        num_views: 1,
        frames_per_view: 1,
        ticks_per_frame: 0,
        key_frame: 0,
        loop_frame: 0,
        bitmap_indices: vec![0],
        shadow_indices: Vec::new(),
        bounds: mandrake_tag::SequenceBounds {
            pixels_to_world: 0x10000,
            half_width: 12 << 16,
            bottom: -(13 << 15),
            top: 58 << 16,
        },
    };
    let sequence = SequenceVisual {
        object_bounds: seq.bounds.world(),
        sequence: seq,
        frames: Vec::new(),
    };
    let def = |flags| mandrake_tag::ObjectDefinition {
        tag_id: FourCC::new(*b"wamo"),
        flags,
        vitality: 1408,
        vitality_delta: 0,
        scale: 0x100,
        scale_delta: 0,
    };
    let visual = |flags| UnitSpriteVisual {
        object: Some(def(flags)),
        ..Default::default()
    };
    let mut sim = Simulation::new(0);
    let id = sim.spawn_unit(FourCC::new(*b"wam1"), 0, Vec3Fixed::ZERO);
    let unit = crate::world_view::myth2::unit_view(&sim, &sim.units[&id]);
    let camera = Camera::new([0.0, 0.0, 0.0], 12.0, 0.7, 0.6);
    let overlay = |transparency, frame_color, thickness| UnitOverlay {
        state: OverlayState {
            transparency,
            frame: true,
        },
        object_flags: 0,
        frame_color,
        outer_frame: false,
        submerged: false,
        thickness,
        mana: None,
    };
    let draw = |v: &UnitSpriteVisual, o: UnitOverlay| {
        unit_selection_overlays(
            &unit,
            unit.position,
            v,
            selection::ObjectBounds::from_sequence(
                sequence.object_bounds,
                unit_object_scale(&unit, v) as i16,
            ),
            &colors,
            &camera,
            (1280, 720),
            o,
        )
    };
    let px = |b: &BillboardInstance| {
        [b.uv_min[0], b.uv_min[1], b.uv_max[0], b.uv_max[1]].map(|v| v as i32)
    };
    let both = visual(3);

    let ally = colors.color_f32(mandrake_tag::interface::COLOR_SELECTION_ALLY);
    let hovered = draw(&both, overlay(selection::TRANSPARENCY_HIGHLIGHTED, 4, 1));
    assert_eq!(hovered.len(), 6);
    for r in &hovered {
        assert_eq!(r.tint[3], 85.0 / 255.0);
    }
    assert_eq!(hovered[0].tint[..3], ally[..3]);
    let top = px(&hovered[0]);
    assert_eq!(top[3] - top[1], 1);

    let thick = draw(&both, overlay(selection::TRANSPARENCY_SELECTED, 2, 2));
    let t = px(&thick[0]);
    assert_eq!(t[3] - t[1], 2);
    assert_eq!(thick[0].tint[3], 170.0 / 255.0);

    let mut o = overlay(selection::TRANSPARENCY_SELECTED, 2, 1);
    o.outer_frame = true;
    let outer = draw(&both, o);
    assert_eq!(outer.len(), 10);
    assert_eq!(outer[4].tint, [1.0, 127.0 / 255.0, 0.0, 1.0]);
    let (frame_top, outer_top) = (px(&outer[0]), px(&outer[4]));
    assert_eq!(
        outer_top,
        [
            frame_top[0] - 1,
            frame_top[1] - 1,
            frame_top[2] + 1,
            frame_top[1]
        ]
    );

    let mut o = overlay(selection::TRANSPARENCY_SELECTED, 2, 1);
    o.submerged = true;
    assert!(draw(&both, o).iter().all(|r| r.tint[3] == 42.0 / 255.0));

    let bar_only = visual(2);
    assert_eq!(draw(&bar_only, overlay(0x5555, 2, 1)).len(), 2);
    let mut o = overlay(0x5555, 3, 1);
    o.object_flags = st::OBJECT_TARGETED;
    assert_eq!(draw(&bar_only, o).len(), 6);
    let mut o = overlay(selection::TRANSPARENCY_HIGHLIGHTED, 2, 1);
    o.state.frame = false;
    assert_eq!(draw(&both, o).len(), 2);

    let mana = |current| selection::mana::MonsterMana {
        current,
        maximum: 25600,
        definition_flags: 0,
        team: 0,
    };
    let mut o = overlay(selection::TRANSPARENCY_SELECTED, 2, 1);
    o.mana = Some(mana(12800));
    let with_mana = draw(&both, o);
    assert_eq!(with_mana.len(), 7);
    let (frame_top, bar) = (px(&with_mana[0]), px(&with_mana[6]));
    assert_eq!(with_mana[6].tint, [0.0, 0.0, 1.0, 170.0 / 255.0]);
    let width = frame_top[2] - frame_top[0];
    assert_eq!(
        bar,
        [
            frame_top[0],
            frame_top[1] - 8,
            frame_top[0] + width / 2,
            frame_top[1] - 2
        ]
    );
    o.state.frame = false;
    assert_eq!(draw(&both, o).len(), 3);
    o.mana = Some(mana(0));
    assert_eq!(draw(&both, o).len(), 2);
}

#[test]
fn test_band_rectangle_overlays() {
    use mandrake_view::billboard::BILLBOARD_FLAG_SCREEN_OVERLAY;
    use mandrake_view::selection::PixelRect;
    let colors =
        InterfaceColorTable::from_colors(vec![[0, 0, 0, 255], [200, 100, 50, 255]], [0, 0, 0, 255]);
    let bounds = selection::band::bounds((300, 260), (100, 200));
    let rects = band_rectangle_overlays(bounds, &colors, (1280, 720));
    assert_eq!(rects.len(), 5);
    let px = |b: &BillboardInstance| {
        [b.uv_min[0], b.uv_min[1], b.uv_max[0], b.uv_max[1]].map(|v| v as i32)
    };
    assert_eq!(px(&rects[0]), [100, 200, 300, 202]);
    assert_eq!(px(&rects[1]), [100, 258, 300, 260]);
    assert_eq!(px(&rects[2]), [100, 202, 102, 258]);
    assert_eq!(px(&rects[3]), [298, 202, 300, 258]);
    assert_eq!(px(&rects[4]), [102, 202, 298, 258]);
    let edge = colors.color_f32(1);
    for r in &rects[..4] {
        assert_eq!(r.tint, [edge[0], edge[1], edge[2], 127.0 / 255.0]);
    }
    assert_eq!(rects[4].tint, [0.0, 0.0, 0.0, 42.0 / 255.0]);
    assert!(rects.iter().all(|r| r.flags
        == mandrake_view::billboard::BILLBOARD_FLAG_INTERFACE_RECT
            | BILLBOARD_FLAG_SCREEN_OVERLAY));
    let thin = band_rectangle_overlays(PixelRect::new(10, 10, 14, 80), &colors, (1280, 720));
    assert_eq!(thin.len(), 1);
    assert_eq!(px(&thin[0]), [10, 10, 14, 80]);
    let short = InterfaceColorTable::from_colors(vec![[1, 2, 3, 255]], [90, 80, 70, 255]);
    let rects = band_rectangle_overlays(bounds, &short, (1280, 720));
    assert_eq!(
        rects[0].tint,
        [90.0 / 255.0, 80.0 / 255.0, 70.0 / 255.0, 127.0 / 255.0]
    );
}
