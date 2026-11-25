use std::sync::Arc;

use mandrake_common::FourCC;
use mandrake_sim::engine_rng::{EngineMonster, EngineObject, EngineWorld, GAMEPLAY_VERSION_1_8_3};
use mandrake_sim::model_animations::{
    AnimationDefinition, AnimationFrame, FrameModel, ModelAnimations,
};
use mandrake_sim::models::{ModelShape, new_model_cells_for_version};
use mandrake_sim::passability::{CELL_KILLED, MeshCells};
use mandrake_tag::model_anim::{ModelAnimationFrame, ModelAnimationTag};
use mandrake_tag::model_hit::HitGeometry;

fn monster(position: (i32, i32)) -> EngineMonster {
    EngineMonster {
        index: 0,
        identifier: 10000,
        first_draw: 0,
        definition: 0,
        object: EngineObject::new(0, FourCC::new(*b"test")),
        mode: 1,
        anim_state: 0x1a,
        speed: 0,
        position,
        flags: 0,
        flags_8: 0,
        neighbours: 0,
        impassable: 0x4000,
        ambient: None,
        unmodelled: None,
        mirrored: false,
        team: 0,
        owner: 0,
        facing: 0,
        formation: Default::default(),
        cell_link: 0,
        flying_height: 0,
    }
}

fn frame_model(cells: &[(i16, i16, u16)]) -> FrameModel {
    let bounds = [(0, 0, 0), (1024, 1024, 64)];
    let geometry = HitGeometry {
        origin: [0, 0, 0],
        vertices: vec![],
        surfaces: vec![],
        bounds: [[0, 0, 0], [1024, 1024, 64]],
    };
    FrameModel {
        shape: Arc::new(ModelShape {
            cells: cells.to_vec(),
            origin: (0, 0, 0),
            bounds,
            ground_vertices: Vec::new(),
            permutation_count: 1,
        }),
        geometry: Arc::new(geometry),
        geometry_tag: None,
        surfaces: Default::default(),
    }
}

fn definition(frames: Vec<FrameModel>, flags: u32, ticks: i16) -> Arc<AnimationDefinition> {
    let tags: Vec<FourCC> = (0..frames.len())
        .map(|i| FourCC::new([b'f', b'r', b'0', b'0' + i as u8]))
        .collect();
    Arc::new(AnimationDefinition {
        tag: ModelAnimationTag {
            flags,
            ticks_per_frame: ticks,
            frames: tags
                .iter()
                .map(|&model_tag| ModelAnimationFrame {
                    flags: 0,
                    model_tag,
                })
                .collect(),
            sounds: [Some(FourCC::new(*b"opn ")), Some(FourCC::new(*b"cls "))],
        },
        frames: frames
            .into_iter()
            .zip(tags)
            .map(|(m, mode_tag)| AnimationFrame {
                mode_tag,
                permutation: 0,
                model: Some(m),
            })
            .collect(),
    })
}

fn placed(def: Arc<AnimationDefinition>) -> (EngineWorld, ModelAnimations) {
    let mut w = EngineWorld::default();
    w.cells = MeshCells {
        width: 16,
        height: 16,
        flags: vec![0x0066; 256],
        models: vec![-1; 256],
    };
    let position = (4 << 9, 4 << 9, 0);
    let frame = def.frames[0].model.as_ref().unwrap();
    new_model_cells_for_version(
        &mut w.cells,
        0,
        position,
        0,
        0xc000,
        &frame.shape,
        w.formation_globals.gameplay_version,
    );
    w.models.push(mandrake_sim::models::WorldModel::new(
        1,
        [position.0, position.1, 0],
        0,
        0xc000,
        0,
        frame.geometry.clone(),
        frame.shape.clone(),
        true,
        0,
    ));
    let mut t = ModelAnimations::default();
    t.add(7, 10000, def, 0, position, 0xc000, 0);
    (w, t)
}

#[test]
fn delete_and_new_change_the_cells_as_the_tables_say() {
    let a = frame_model(&[(0, 0, 1), (1, 0, 2), (0, 1, 4), (1, 1, 0)]);
    let b = frame_model(&[(1, 1, 1)]);
    let (mut w, mut t) = placed(definition(vec![a, b], 0, 4));
    let c = |w: &EngineWorld, x: i32, y: i32| w.cells.cell_index(x, y);
    assert_eq!(w.cells.flags[c(&w, 3, 4)], CELL_KILLED | 0xee);
    assert_eq!(w.cells.flags[c(&w, 3, 5)], 0xee);
    assert_eq!(w.cells.flags[c(&w, 2, 4)], 0xff);
    assert_eq!(w.cells.flags[c(&w, 2, 5)], 0x66);
    assert_eq!(w.cells.models[c(&w, 2, 5)], 0);
    w.monsters.push(monster((1 << 9, 4 << 9)));

    t.jump(0, 1, &mut w).unwrap();

    assert_eq!(w.cells.flags[c(&w, 3, 4)], 0x66);
    assert_eq!(w.cells.flags[c(&w, 3, 5)], 0x66);
    assert_eq!(w.cells.flags[c(&w, 2, 4)], 0x66);
    assert_eq!(w.cells.flags[c(&w, 2, 5)], CELL_KILLED | 0xee);
    for (x, y) in [(3, 4), (3, 5), (2, 4), (2, 5)] {
        assert_eq!(w.cells.models[c(&w, x, y)], 0);
    }
    assert_eq!(w.cells.models[c(&w, 4, 4)], -1);
    assert_eq!(t.animations[0].model, 0, "new_datum gives the index back");
    assert_eq!(w.models.len(), 1);
    assert!(w.cells_changed);
    assert!(!w.impassability_updates.is_empty());
    assert_eq!(w.monsters[0].flags & 0x20, 0x20);
    let mask = w.monsters[0].impassable;
    let (x, y) = w.monsters[0].position;
    assert_eq!(w.monsters[0].neighbours, w.cells.neighbour_mask(mask, x, y));
}

#[test]
fn the_1_8_5_new_model_disables_the_media_animation() {
    for (version, want) in [(GAMEPLAY_VERSION_1_8_3, 0x400), (0x73c, 0)] {
        let a = frame_model(&[(0, 0, 2)]);
        let b = frame_model(&[(0, 0, 1)]);
        let (mut w, mut t) = placed(definition(vec![a, b], 0, 4));
        w.formation_globals.gameplay_version = version;
        for (x, y) in [(3, 4), (4, 4), (3, 5), (4, 5), (5, 4)] {
            let i = w.cells.cell_index(x, y);
            w.cells.flags[i] |= 0x400;
        }
        t.jump(0, 1, &mut w).unwrap();
        for (x, y) in [(3, 4), (4, 4), (3, 5), (4, 5)] {
            assert_eq!(w.cells.flags[w.cells.cell_index(x, y)] & 0x400, want);
        }
        let outside = w.cells.cell_index(5, 4);
        assert_eq!(w.cells.flags[outside] & 0x400, 0x400);
    }
}

#[test]
fn a_frame_without_a_model_frees_the_index_and_the_next_takes_it() {
    let a = frame_model(&[(0, 0, 1)]);
    let def = definition(vec![a.clone(), a], 0, 4);
    let mut d = (*def).clone();
    d.frames[1].model = None;
    d.frames.push(d.frames[0].clone());
    d.frames[2].mode_tag = FourCC::new(*b"fr02");
    d.tag.frames.push(ModelAnimationFrame {
        flags: 0,
        model_tag: FourCC::new(*b"fr02"),
    });
    let (mut w, mut t) = placed(Arc::new(d));
    t.jump(0, 1, &mut w).unwrap();
    assert_eq!(t.animations[0].model, -1, "new_model returned -1");
    assert_eq!(w.cells.models[w.cells.cell_index(3, 4)], -1);
    t.jump(0, 2, &mut w).unwrap();
    assert_eq!(t.animations[0].model, 0);
}

#[test]
fn the_same_model_with_another_word_keeps_the_cells() {
    let a = frame_model(&[(0, 0, 1)]);
    let def = definition(vec![a.clone(), a], 0, 4);
    let mut d = (*def).clone();
    d.frames[1].mode_tag = d.frames[0].mode_tag;
    d.frames[1].permutation = 1;
    let (mut w, mut t) = placed(Arc::new(d));
    t.jump(0, 1, &mut w).unwrap();
    assert!(!w.cells_changed, "no delete, no new");
    assert_eq!(t.animations[0].playback.frame, 1);
}

#[test]
fn frames_accessor_gives_the_state() {
    let frames: Vec<FrameModel> = (0..3).map(|_| frame_model(&[(0, 0, 1)])).collect();
    let (mut w, mut t) = placed(definition(frames, 0, 4));
    t.jump(0, 2, &mut w).unwrap();
    let f = t.frames();
    assert_eq!(f.len(), 1);
    assert_eq!((f[0].marker, f[0].identifier, f[0].frame), (7, 10000, 2));
    assert_eq!((f[0].running, f[0].model), (false, 0));
}
