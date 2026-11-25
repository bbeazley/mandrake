use crate::draw_site;
use crate::engine_rng::Unmodelled;
use crate::geometry_filter::{PARAMETER_POOL_SIZE, parameter_bytes};
use crate::map_actions::{MapActionContext, MapActions, TYPE_ACTIONS};
use mandrake_common::game_options;
use mandrake_tag::map_action::MapActionParameter;

const fn tag(s: &[u8; 4]) -> u32 {
    u32::from_be_bytes(*s)
}

const AACT: u32 = tag(b"aact");
const ADEA: u32 = tag(b"adea");
const ADEL: u32 = tag(b"adel");
const ASUC: u32 = tag(b"asuc");
const AREB: u32 = tag(b"areb");
const PINS: u32 = tag(b"pins");
const PREP: u32 = tag(b"prep");
const PDEL: u32 = tag(b"pdel");
const PAPP: u32 = tag(b"papp");

fn is_munge_name(name: u32) -> bool {
    matches!(
        name,
        AACT | ADEA | ADEL | ASUC | AREB | PINS | PREP | PDEL | PAPP
    )
}

#[derive(Clone, Copy)]
struct Found {
    name: u32,
    kind: i16,
    count: i16,
    index: usize,
}

impl MapActions {
    fn next_own_parameter(&self, action: usize, cursor: &mut usize) -> Option<Found> {
        let params = &self.actions[action].parameters;
        while let Some(p) = params.get(*cursor) {
            let index = *cursor;
            *cursor += 1;
            if p.count > 0 {
                return Some(Found {
                    name: u32::from_be_bytes(p.name.0),
                    kind: p.kind,
                    count: p.count,
                    index,
                });
            }
        }
        None
    }

    pub(crate) fn munger(
        &mut self,
        i: usize,
        ctx: &mut MapActionContext,
    ) -> Result<u32, Unmodelled> {
        let gameplay = ctx.world.formation_globals.gameplay_version;
        let mut cursor = 0;
        while let Some(mut current) = self.next_own_parameter(i, &mut cursor) {
            if current.kind != TYPE_ACTIONS {
                continue;
            }
            let list = current.index;
            let mut element = 0;
            while element < i32::from(current.count) {
                let identifier = self.actions[i]
                    .parameters
                    .get(list)
                    .filter(|p| p.kind == TYPE_ACTIONS)
                    .and_then(|p| p.values.get(element as usize))
                    .copied();
                element += 1;
                let Some(identifier) = identifier else {
                    if is_munge_name(current.name) {
                        return Err(Unmodelled::new(
                            draw_site::MUNGER_READ_PAST_END,
                            "munger_action_proc reads an edited list past its end under a name \
                             that acts",
                        ));
                    }
                    continue;
                };
                let Some(target) = self.index_of(identifier) else {
                    continue;
                };
                let open = gameplay > 0;
                match current.name {
                    AACT if open => self.activate(Some(target), ctx)?,
                    ADEA if open => self.deactivate(Some(target), ctx)?,
                    ADEL if open => {
                        if target == i {
                            return Err(Unmodelled::new(
                                draw_site::MUNGER_DELETES_ITSELF,
                                "munger_action_proc deletes its own action",
                            ));
                        }
                        self.actions[target].identifier = 0;
                    }
                    ASUC if open => self.succeed(target, ctx)?,
                    AREB if gameplay > game_options::GAMEPLAY_1_7_2 => self.reboot(target, ctx)?,
                    PINS | PREP | PDEL | PAPP if open => {
                        let name = current.name;
                        let Some(next) = self.next_own_parameter(i, &mut cursor) else {
                            continue;
                        };
                        current = next;
                        let source = self.actions[i].parameters[next.index].clone();
                        match name {
                            PINS => self.insert_parameter(target, &source),
                            PREP => {
                                self.delete_parameters_like(target, &source);
                                self.insert_parameter(target, &source);
                            }
                            PDEL => self.delete_parameters_like(target, &source),
                            _ => {
                                return Err(Unmodelled::new(
                                    draw_site::MUNGER_APPEND_FROM_OWN_ARRAY,
                                    "munger_action_proc 'papp': the append proc appends from the \
                                     target's own array, by a pointer the pool's move leaves \
                                     stale",
                                ));
                            }
                        }
                    }
                    _ => {}
                }
            }
        }
        Ok(0)
    }

    fn delete_parameters_like(&mut self, target: usize, source: &MapActionParameter) {
        self.actions[target]
            .parameters
            .retain(|p| !(p.name == source.name && p.kind == source.kind));
    }

    fn insert_parameter(&mut self, target: usize, source: &MapActionParameter) {
        let used = self.parameter_pool_used();
        let action = self.action_bytes(target);
        if used + action + 8 >= PARAMETER_POOL_SIZE {
            return;
        }
        let empty = MapActionParameter {
            kind: source.kind,
            count: 0,
            name: source.name,
            values: Vec::new(),
        };
        let grown = parameter_bytes(source) - parameter_bytes(&empty);
        let fits = grown <= 0 || used + 8 + action + 8 + grown < PARAMETER_POOL_SIZE;
        self.actions[target]
            .parameters
            .push(if fits { source.clone() } else { empty });
    }

    fn reboot(&mut self, target: usize, ctx: &mut MapActionContext) -> Result<(), Unmodelled> {
        let a = &mut self.actions[target];
        a.parameters.retain(|p| p.name.0[0] != b'#');
        let was = a.flags;
        a.flags &= !0x1e0;
        if was & crate::map_actions::FLAG_INITIALLY_ACTIVE != 0 {
            self.activate(Some(target), ctx)?;
        }
        Ok(())
    }
}
