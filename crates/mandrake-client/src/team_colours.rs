use mandrake_common::FourCC;
use mandrake_math::BungieRng;
use mandrake_sim::engine_rng::EngineWorld;
use mandrake_sim::players::PlayerColor;
use mandrake_tag::MeshTag;
use std::collections::{BTreeMap, BTreeSet};
use tracing::warn;

pub(crate) const CUSTOM_CORE_DRAW: u32 = mandrake_sim::draw_site::CUSTOM_CORE;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TeamTable {
    pub base: usize,
    pub primary: PlayerColor,
    pub secondary: PlayerColor,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TeamColours {
    pub rows: BTreeMap<(FourCC, i16), TeamTable>,
    pub monsters: BTreeMap<u32, (FourCC, i16)>,
    pub base_permutations: BTreeMap<FourCC, BTreeSet<usize>>,
    pub flags: Option<FlagTables>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FlagTables {
    pub scenery: FourCC,
    pub teams: Vec<Option<(PlayerColor, PlayerColor)>>,
}

impl FlagTables {
    pub fn of_load(world: &EngineWorld, mesh: &MeshTag) -> Option<Self> {
        let first = world.flag_data.records.first()?.as_ref()?;
        let scenery = mesh.palette_tag_for_marker(mesh.markers.get(first.marker)?)?;
        let teams = world
            .players
            .teams
            .iter()
            .map(|t| {
                let c = world.players.colors.get(usize::try_from(t.player).ok()?)?;
                Some((c[0], c[1]))
            })
            .collect();
        Some(Self { scenery, teams })
    }
}

impl TeamColours {
    pub fn of_load(
        world: &EngineWorld,
        mesh: &MeshTag,
        draws: &[u32],
        permutations: &dyn Fn(FourCC) -> Option<i16>,
    ) -> Self {
        let key = |tag: FourCC| u32::from_be_bytes(tag.0);
        let custom = |tag: FourCC, player: i16| {
            world
                .unit_rows
                .get(&(key(tag), player))
                .is_some_and(|r| r.custom_core)
        };
        let mut seen = BTreeSet::new();
        let mut order = Vec::new();
        for p in mesh.palettes.iter().filter(|p| p.marker_type == 3) {
            let team = p.team_index();
            if p.flags as u16 & 0x10 != 0 {
                if world.unit_rows.contains_key(&(key(p.tag_id), 0)) {
                    seen.insert((p.tag_id, 0));
                }
                continue;
            }
            if team == -1 {
                continue;
            }
            for (j, pl) in world.players.players.iter().enumerate() {
                let j = j as i16;
                if i16::from(pl.team) == team && seen.insert((p.tag_id, j)) && custom(p.tag_id, j) {
                    order.push((p.tag_id, j));
                }
            }
        }
        let rows_with_core = world.unit_rows.values().filter(|r| r.custom_core).count();
        if order.len() != rows_with_core || order.len() != draws.len() {
            warn!(
                "Team colours: {} rows with their own core, {} in palette order, {} draws at \
                 {CUSTOM_CORE_DRAW:#x}; the rest keep the unit's permutations",
                rows_with_core,
                order.len(),
                draws.len()
            );
        }
        let mut rows = BTreeMap::new();
        for (&(tag, player), &state) in order.iter().zip(draws) {
            let (Some(n), Some(colors)) = (
                permutations(tag).filter(|&n| n > 0),
                world.players.colors.get(player as usize),
            ) else {
                continue;
            };
            let value = BungieRng { state }.next_u16();
            rows.insert(
                (tag, player),
                TeamTable {
                    base: (i32::from(value) % i32::from(n)) as usize,
                    primary: colors[0],
                    secondary: colors[1],
                },
            );
        }
        let monsters = world
            .monsters
            .iter()
            .enumerate()
            .filter_map(|(i, m)| {
                let (tag, player) = world.combat(i).unit;
                let row = (FourCC::new(tag.to_be_bytes()), player);
                rows.contains_key(&row)
                    .then_some((i32::from(m.identifier) as u32, row))
            })
            .collect::<BTreeMap<u32, (FourCC, i16)>>();
        let mut base_permutations: BTreeMap<FourCC, BTreeSet<usize>> = monsters
            .values()
            .map(|&(tag, _)| (tag, BTreeSet::from([0])))
            .collect();
        for (i, m) in world.monsters.iter().enumerate() {
            let tag = FourCC::new(world.combat(i).unit.0.to_be_bytes());
            if let Some(set) = base_permutations.get_mut(&tag)
                && !monsters.contains_key(&(i32::from(m.identifier) as u32))
            {
                set.insert(m.object.permutation.max(0) as usize);
            }
        }
        Self {
            rows,
            monsters,
            base_permutations,
            flags: FlagTables::of_load(world, mesh),
        }
    }

    pub fn known_row(&self, row: (FourCC, i16)) -> Option<(FourCC, i16)> {
        self.rows.contains_key(&row).then_some(row)
    }

    fn row_of_index(&self, world: &EngineWorld, i: usize) -> Option<(FourCC, i16)> {
        let (tag, player) = world.combat(i).unit;
        let row = (FourCC::new(tag.to_be_bytes()), player);
        self.rows.contains_key(&row).then_some(row)
    }

    pub fn drawn_rows(&self, world: &EngineWorld) -> BTreeSet<(FourCC, i16)> {
        if self.rows.is_empty() {
            return BTreeSet::new();
        }
        (0..world.monsters.len())
            .filter(|&i| world.monsters[i].identifier != 0)
            .filter_map(|i| self.row_of_index(world, i))
            .collect()
    }

    pub fn any_drawn_row(
        &self,
        world: &EngineWorld,
        mut pred: impl FnMut((FourCC, i16)) -> bool,
    ) -> bool {
        if self.rows.is_empty() {
            return false;
        }
        (0..world.monsters.len())
            .filter(|&i| world.monsters[i].identifier != 0)
            .filter_map(|i| self.row_of_index(world, i))
            .any(&mut pred)
    }
}

pub fn core_permutations(
    tags: &dyn Fn([u8; 4], FourCC) -> Option<Vec<u8>>,
    unit: FourCC,
) -> Option<i16> {
    let data = tags(*b"unit", unit)?;
    let core = FourCC::new(data.get(4..8)?.try_into().ok()?);
    let core = tags(*b"core", core)?;
    Some(i16::from_be_bytes(core.get(4..6)?.try_into().ok()?))
}

pub fn with_custom_core_draws<T>(load: impl FnOnce() -> T) -> (T, Vec<u32>) {
    use mandrake_sim::engine_rng::DRAW_LOG;
    let (was_on, from) = DRAW_LOG.with(|l| {
        let mut l = l.borrow_mut();
        let was_on = l.is_some();
        let from = l.get_or_insert_with(Vec::new).len();
        (was_on, from)
    });
    let out = load();
    let draws = DRAW_LOG.with(|l| {
        let mut l = l.borrow_mut();
        let draws = l
            .as_ref()
            .map(|v| {
                v.get(from..)
                    .unwrap_or_default()
                    .iter()
                    .filter(|&&(site, _)| site == CUSTOM_CORE_DRAW)
                    .map(|&(_, state)| state)
                    .collect()
            })
            .unwrap_or_default();
        if !was_on {
            *l = None;
        }
        draws
    });
    (out, draws)
}
