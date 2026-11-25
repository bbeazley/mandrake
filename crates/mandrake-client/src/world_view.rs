pub(crate) mod myth2;
pub(crate) mod pick;
pub(crate) mod tfl;

use mandrake_common::FourCC;
use mandrake_tag::MonsterSequenceSlot;
use mandrake_view::selection;

use crate::media;
use crate::minimap::{BlipMonster, BlipRules, VisibilityMonster};
use crate::object_sprites::DrawnObject;
use crate::render_passes::PassRule;
use crate::scenery::SceneryObject;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ClientRules {
    pub sprite_scale: f32,
    pub sprites_bob: bool,
    pub sight: ObjectSight,
    pub water_only_flag: u32,
    pub water_cells: u8,
    pub reflect_cells: u8,
    pub bucket_depth: bool,
    pub lightning_first: bool,
    pub(crate) pass: PassRule,
}

impl ClientRules {
    pub const MYTH2: Self = Self {
        sprite_scale: 1.0,
        sprites_bob: true,
        sight: ObjectSight::All,
        water_only_flag: 0,
        water_cells: 0,
        reflect_cells: 0,
        bucket_depth: false,
        lightning_first: false,
        pass: PassRule::Myth2,
    };
    pub const TFL: Self = Self {
        sprite_scale: 7.0 / 10.0,
        sprites_bob: false,
        sight: ObjectSight::Tfl,
        water_only_flag: 0x4000,
        water_cells: 0x18,
        reflect_cells: 0x38,
        bucket_depth: true,
        lightning_first: true,
        pass: PassRule::Tfl,
    };
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ObjectSight {
    All,
    Tfl,
}

impl ObjectSight {
    pub(crate) fn rule(self, flags: u32, bits: u8, explored: bool) -> Option<bool> {
        match self {
            Self::All => Some(true),
            Self::Tfl => {
                if flags & 0x200 != 0 || !explored {
                    return None;
                }
                if flags & 0x100 != 0 && flags & 0x400_0000 == 0 {
                    return Some(true);
                }
                (bits & 1 != 0).then_some(bits & 2 != 0 && flags & 0x1000_0000 == 0)
            }
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct WorldView {
    pub tick: u32,
    pub engine_stopped: bool,
    pub game: ViewGame,
    pub units: Vec<ViewUnit>,
    pub objects: Vec<DrawnObject>,
    pub parasites: Vec<(u32, DrawnObject)>,
    pub monsters: Vec<DrawnObject>,
    pub origin: [f32; 2],
    pub minimap: Option<MinimapView>,
    pub blips: Vec<BlipMonster>,
    pub sight: Option<ViewSight>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ViewSight {
    pub cells: (i32, i32),
    pub visible: Vec<u8>,
    pub explored: Vec<bool>,
    pub centre: (i16, i16),
}

impl ViewSight {
    fn index(&self, (col, row): (i32, i32)) -> Option<usize> {
        (col >= 0 && row >= 0 && col < self.cells.0 && row < self.cells.1)
            .then(|| (row * self.cells.0 + col) as usize)
    }

    pub fn bits_at(&self, p: (i32, i32)) -> u8 {
        self.index((p.0 >> 10, p.1 >> 10))
            .and_then(|i| self.visible.get(i).copied())
            .unwrap_or(0)
    }

    pub fn explored(&self, cell: (i32, i32)) -> bool {
        self.index(cell)
            .and_then(|i| self.explored.get(i).copied())
            .unwrap_or(false)
    }

    pub fn all_seen(cells: (i32, i32), points: &[(i32, i32)]) -> Self {
        let n = (cells.0.max(0) * cells.1.max(0)) as usize;
        let centre = if points.is_empty() {
            (cells.0 / 2, cells.1 / 2)
        } else {
            let k = points.len() as i64;
            let sx: i64 = points.iter().map(|p| i64::from(p.0 >> 10)).sum();
            let sy: i64 = points.iter().map(|p| i64::from(p.1 >> 10)).sum();
            ((sx / k) as i32, (sy / k) as i32)
        };
        Self {
            cells,
            visible: vec![3; n],
            explored: vec![true; n],
            centre: (centre.0 as i16, centre.1 as i16),
        }
    }
}

impl WorldView {
    pub fn unit(&self, id: u32) -> Option<&ViewUnit> {
        let i = self.units.binary_search_by_key(&id, |u| u.id).ok()?;
        self.units.get(i)
    }

    pub fn ground_point(&self, xz: [f32; 2]) -> (i32, i32) {
        let one = mandrake_tag::MarkerEntry::WORLD_ONE;
        (
            ((self.origin[0] - xz[0]) * one).round() as i32,
            ((xz[1] - self.origin[1]) * one).round() as i32,
        )
    }
}

#[derive(Clone, Debug)]
pub struct ViewGame {
    pub context: selection::state::GameContext,
    pub alliances: selection::state::Alliances,
    pub underwater: media::UnderwaterGame,
}

impl Default for ViewGame {
    fn default() -> Self {
        Self {
            context: crate::interface_state::solo_game(crate::map_load::LOCAL_PLAYER_TEAM),
            alliances: selection::state::Alliances::new(16),
            underwater: media::UnderwaterGame {
                local_team: crate::map_load::LOCAL_PLAYER_TEAM,
                mode: 0,
                game_options_high: 0,
            },
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct UnitObject {
    pub scale: u16,
    pub vitality: i16,
    pub permutation: i16,
}

pub trait UnitPose {
    fn id(&self) -> u32;
    fn position(&self) -> [f32; 3];
    fn engine_facing(&self) -> u16;
    fn state_slot(&self) -> MonsterSequenceSlot;
    fn object(&self) -> Option<UnitObject>;
}

#[derive(Clone, Debug, PartialEq)]
pub struct ViewUnit {
    pub id: u32,
    pub archetype: FourCC,
    pub team: u8,
    pub selected: bool,
    pub position: [f32; 3],
    pub engine_facing: u16,
    pub state_slot: MonsterSequenceSlot,
    pub still: bool,
    pub health: f32,
    pub max_health: f32,
    pub object: Option<UnitObject>,
    pub monster: Option<ViewMonster>,
    pub colour_row: Option<(FourCC, i16)>,
    pub pick: Option<UnitPick>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct UnitPick {
    pub monster: u32,
    pub world: [i32; 3],
    pub bounds: selection::ObjectBounds,
    pub vitality: Option<selection::tfl::MonsterVitality>,
    pub evaluation: u8,
    pub definition: FourCC,
    pub dying: bool,
}

impl UnitPose for ViewUnit {
    fn id(&self) -> u32 {
        self.id
    }
    fn position(&self) -> [f32; 3] {
        self.position
    }
    fn engine_facing(&self) -> u16 {
        self.engine_facing
    }
    fn state_slot(&self) -> MonsterSequenceSlot {
        self.state_slot
    }
    fn object(&self) -> Option<UnitObject> {
        self.object
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ViewMonster {
    pub team: i16,
    pub owner: i16,
    pub flags: u32,
    pub flags_8: u32,
    pub definition_199: u8,
    pub mana: Option<(i16, i16, u8)>,
}

#[derive(Clone, Debug)]
pub struct MinimapView {
    pub visibility: Vec<VisibilityMonster>,
    pub rules: BlipRules,
    pub local_team: i16,
    pub game_options: u32,
}

#[derive(Clone, Debug)]
pub struct MinimapStart {
    pub mesh_cells: (i32, i32),
    pub team_colors: Vec<[u16; 3]>,
    pub team_mode: bool,
    pub view: MinimapView,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ViewScenery {
    pub marker: usize,
    pub deleted: bool,
    pub object: SceneryObject,
}
