use std::collections::HashMap;
use std::io::Write;
use std::path::Path;

use mandrake_rec::CommandPacket;
use mandrake_sim::Simulation;
use tracing::{info, warn};

const ORDER_KINDS: [u8; 4] = [2, 3, 4, 5];
const FIRST_TICK: u32 = 60;
const EVERY: u32 = 15;
const PER_SIDE: usize = 90;
const MAX_OFFSET: i32 = 4 * 512;

struct Planned {
    tick: u32,
    ours: bool,
    kind: u8,
    payload: Vec<u8>,
}

#[derive(Default)]
struct Slots(HashMap<u16, usize>);

impl Slots {
    fn slot(&mut self, film_id: u16) -> usize {
        let n = self.0.len();
        *self.0.entry(film_id).or_insert(n)
    }
}

pub(crate) struct Orders {
    plan: Vec<Planned>,
    next: usize,
    host_out: Option<std::fs::File>,
    units: [Slots; 2],
    targets: [Slots; 2],
    attacks: [usize; 2],
    pub(crate) ours: usize,
    pub(crate) host: usize,
}

impl Orders {
    pub(crate) fn load(
        film: &Path,
        host_out: Option<&Path>,
        hosting: bool,
    ) -> anyhow::Result<Self> {
        let data = std::fs::read(film)?;
        let rec = mandrake_rec::Recording::from_bytes(&data)
            .map_err(|e| anyhow::anyhow!("--net-orders {}: {e}", film.display()))?;
        let orders: Vec<&CommandPacket> = rec
            .iter_commands()
            .filter(|c| ORDER_KINDS.contains(&c.command_type.to_u8()))
            .collect();
        let mut counts: HashMap<u8, usize> = HashMap::new();
        for c in &orders {
            *counts.entry(c.player_index).or_default() += 1;
        }
        let mut busiest: Vec<(usize, u8)> = counts.iter().map(|(&p, &n)| (n, p)).collect();
        busiest.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
        anyhow::ensure!(
            busiest.len() >= 2,
            "--net-orders: the film has one player's orders"
        );
        let mut plan = Vec::new();
        for (side, &(_, player)) in busiest.iter().take(2).enumerate() {
            let mine: Vec<&&CommandPacket> =
                orders.iter().filter(|c| c.player_index == player).collect();
            let others = mine.iter().filter(|c| c.command_type.to_u8() != 3).count();
            let moves = mine.len() - others;
            let want_moves = PER_SIDE.saturating_sub(others.min(PER_SIDE));
            let step = (moves / want_moves.max(1)).max(1);
            let mut seen_moves = 0;
            let picked: Vec<&&CommandPacket> = mine
                .into_iter()
                .filter(|c| {
                    if c.command_type.to_u8() != 3 {
                        return true;
                    }
                    seen_moves += 1;
                    (seen_moves - 1) % step == 0
                })
                .take(PER_SIDE)
                .collect();
            let ours = (side == 0) != hosting;
            info!(
                "net-orders: film player {player} as {} ({}): {} orders",
                if ours { "ours" } else { "the other side's" },
                if side == 0 {
                    "the joiner's"
                } else {
                    "the host's"
                },
                picked.len()
            );
            for (i, c) in picked.into_iter().enumerate() {
                plan.push(Planned {
                    tick: FIRST_TICK + EVERY * i as u32 + if side == 0 { 0 } else { EVERY / 2 },
                    ours,
                    kind: c.command_type.to_u8(),
                    payload: c.raw_payload.clone(),
                });
            }
        }
        plan.sort_by_key(|p| p.tick);
        let host_out = match host_out {
            Some(p) => Some(std::fs::File::create(p)?),
            None => None,
        };
        Ok(Self {
            plan,
            next: 0,
            host_out,
            units: Default::default(),
            targets: Default::default(),
            attacks: [0; 2],
            ours: 0,
            host: 0,
        })
    }

    pub(crate) fn due(&mut self, sim: &Simulation, local: u8) -> Vec<(u8, Vec<u8>)> {
        let mut out = Vec::new();
        let w = &sim.engine;
        let tick = sim.current_tick;
        let local = i16::from(local);
        let mut sides: [Vec<(u16, (i32, i32))>; 2] = [Vec::new(), Vec::new()];
        for m in &w.monsters {
            if m.identifier == 0 || m.owner < 0 || m.object.vitality <= 0 {
                continue;
            }
            let side = usize::from(m.owner != local);
            sides[side].push((m.identifier as u16, m.position));
        }
        for s in &mut sides {
            s.sort_by_key(|u| u.0);
        }
        while let Some(p) = self.plan.get(self.next) {
            if p.tick > tick {
                break;
            }
            let (ours, kind, payload) = (p.ours, p.kind, p.payload.clone());
            self.next += 1;
            let side = usize::from(!ours);
            let (own, enemy) = (&sides[side], &sides[1 - side]);
            if own.is_empty() || enemy.is_empty() {
                warn!("net-orders: tick {tick}: a side has no units; the order is dropped");
                continue;
            }
            let Some(payload) = self.rewrite(side, kind, &payload, own, enemy) else {
                warn!("net-orders: tick {tick}: a {kind} order the aid can't read is dropped");
                continue;
            };
            if ours {
                self.ours += 1;
                out.push((kind, payload));
            } else if let Some(f) = self.host_out.as_mut() {
                self.host += 1;
                let hex: String = payload.iter().map(|b| format!("{b:02x}")).collect();
                let _ = writeln!(f, "{tick} {kind} {hex}");
                let _ = f.flush();
            }
        }
        out
    }

    fn rewrite(
        &mut self,
        side: usize,
        kind: u8,
        film: &[u8],
        own: &[(u16, (i32, i32))],
        enemy: &[(u16, (i32, i32))],
    ) -> Option<Vec<u8>> {
        let mut p = film.to_vec();
        let be16 = |p: &[u8], o: usize| -> Option<u16> {
            Some(u16::from_be_bytes(p.get(o..o + 2)?.try_into().ok()?))
        };
        let be32 = |p: &[u8], o: usize| -> Option<i32> {
            Some(i32::from_be_bytes(p.get(o..o + 4)?.try_into().ok()?))
        };
        let put32 = |p: &mut [u8], o: usize, v: i32| p[o..o + 4].copy_from_slice(&v.to_be_bytes());
        let flags = be16(&p, 0)?;
        let list = |p: &mut Vec<u8>,
                    o: usize,
                    slots: &mut Slots,
                    live: &[(u16, (i32, i32))],
                    matched: bool|
         -> Option<usize> {
            let n = usize::from(be16(p, o)?);
            let film_ids: Vec<u16> = (0..n)
                .map(|i| be16(p, o + 2 + 2 * i))
                .collect::<Option<_>>()?;
            let mut ids: Vec<u16> = Vec::with_capacity(n);
            for f in film_ids {
                let id = if f == 0xffff {
                    0xffff
                } else {
                    live[slots.slot(f) % live.len()].0
                };
                if id != 0xffff && ids.contains(&id) {
                    if matched {
                        ids.push(0xffff);
                    }
                } else {
                    ids.push(id);
                }
            }
            let mut bytes = (ids.len() as u16).to_be_bytes().to_vec();
            for id in &ids {
                bytes.extend_from_slice(&id.to_be_bytes());
            }
            p.splice(o..o + 2 + 2 * n, bytes);
            Some(o + 2 + 2 * ids.len())
        };
        let centre = |live: &[(u16, (i32, i32))]| {
            let n = live.len() as i64;
            let (x, y) = live.iter().fold((0i64, 0i64), |(x, y), u| {
                (x + i64::from(u.1.0), y + i64::from(u.1.1))
            });
            ((x / n) as i32, (y / n) as i32)
        };
        match kind {
            2 => {
                list(&mut p, 4, &mut self.units[side], own, false)?;
            }
            3 => {
                let matched = flags & mandrake_rec::MOVEMENT_FLAG_MATCHED != 0;
                let o = list(&mut p, 8, &mut self.units[side], own, matched)?;
                let n = usize::from(be16(&p, o)?).min(mandrake_rec::MAX_WAYPOINTS);
                if n == 0 {
                    return Some(p);
                }
                let points = n + usize::from(matched);
                let last = o + 2 + 8 * (n - 1);
                let (dx, dy) = (be32(&p, last)?, be32(&p, last + 4)?);
                let goal = centre(enemy);
                for i in 0..points {
                    let at = o + 2 + 8 * i;
                    let (x, y) = (be32(&p, at)?, be32(&p, at + 4)?);
                    let off = |v: i32, d: i32| (v - d).clamp(-MAX_OFFSET, MAX_OFFSET);
                    put32(&mut p, at, goal.0 + off(x, dx));
                    put32(&mut p, at + 4, goal.1 + off(y, dy));
                }
            }
            4 => {
                let matched = flags & mandrake_rec::TARGET_FLAG_MATCHED != 0;
                let o = list(&mut p, 2, &mut self.units[side], own, matched)?;
                list(&mut p, o, &mut self.targets[side], enemy, matched)?;
            }
            5 => {
                be32(&p, 12)?;
                let at = enemy[self.attacks[side] % enemy.len()].1;
                self.attacks[side] += 1;
                put32(&mut p, 4, at.0);
                put32(&mut p, 8, at.1);
                list(&mut p, 16, &mut self.units[side], own, false)?;
            }
            _ => return None,
        }
        Some(p)
    }

    pub(crate) fn done(&self) -> bool {
        self.next >= self.plan.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn empty() -> Orders {
        Orders {
            plan: Vec::new(),
            next: 0,
            host_out: None,
            units: Default::default(),
            targets: Default::default(),
            attacks: [0; 2],
            ours: 0,
            host: 0,
        }
    }

    fn ids(v: &[u16]) -> Vec<u8> {
        let mut b = (v.len() as u16).to_be_bytes().to_vec();
        for i in v {
            b.extend_from_slice(&i.to_be_bytes());
        }
        b
    }

    #[test]
    fn a_film_order_goes_onto_this_games_units() {
        let own = [(101, (0, 0)), (102, (512, 0))];
        let enemy = [(201, (10_000, 10_000)), (202, (12_000, 10_000))];
        let mut o = empty();
        let mut target = 0x200u16.to_be_bytes().to_vec();
        target.extend(ids(&[7, 8, 9]));
        target.extend(ids(&[50, 0xffff, 51]));
        let got = o.rewrite(0, 4, &target, &own, &enemy).unwrap();
        let mut want = 0x200u16.to_be_bytes().to_vec();
        want.extend(ids(&[101, 102, 0xffff]));
        want.extend(ids(&[201, 0xffff, 202]));
        assert_eq!(got, want);
        let mut mv = vec![0, 0, 0, 3, 0, 0, 0, 0];
        mv.extend(ids(&[8]));
        mv.extend(2u16.to_be_bytes());
        for v in [1000i32, 1000, 1512, 2000] {
            mv.extend(v.to_be_bytes());
        }
        let got = o.rewrite(0, 3, &mv, &own, &enemy).unwrap();
        let mut want = vec![0, 0, 0, 3, 0, 0, 0, 0];
        want.extend(ids(&[102]));
        want.extend(2u16.to_be_bytes());
        for v in [11_000i32 - 512, 10_000 - 1000, 11_000, 10_000] {
            want.extend(v.to_be_bytes());
        }
        assert_eq!(got, want);
    }
}
