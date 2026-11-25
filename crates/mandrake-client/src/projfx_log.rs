use mandrake_common::FourCC;
use mandrake_sim::{ProjectileView, Simulation, views::ObjectKind};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LoggedStain {
    pub tick: u32,
    pub collection: FourCC,
    pub sequence: i16,
    pub point: (i32, i32),
    pub random: u16,
    pub mirror: (bool, bool),
}

#[derive(Clone, Debug, Default)]
pub struct ProjfxLog {
    objects: Vec<(u32, ProjectileView)>,
    pub stains: Vec<LoggedStain>,
}

fn fourcc(s: &str) -> anyhow::Result<FourCC> {
    let b: [u8; 4] = s
        .as_bytes()
        .try_into()
        .map_err(|_| anyhow::anyhow!("bad tag id {s:?}"))?;
    Ok(FourCC::new(b))
}

impl ProjfxLog {
    pub fn parse(text: &str, sim: &Simulation) -> anyhow::Result<Self> {
        let mut out = Self::default();
        for line in text
            .lines()
            .map(str::trim)
            .filter(|l| !l.is_empty() && !l.starts_with('#'))
        {
            let f: Vec<&str> = line.split_whitespace().collect();
            let field = |i: usize| -> anyhow::Result<&str> {
                f.get(i)
                    .copied()
                    .ok_or_else(|| anyhow::anyhow!("short line {line:?}"))
            };
            let int = |i: usize| -> anyhow::Result<i32> { Ok(field(i)?.parse()?) };
            match field(0)? {
                "P" => {
                    let world = [int(11)?, int(12)?, int(13)?];
                    let velocity = [int(14)? as i16, int(15)? as i16, int(16)? as i16];
                    let flags = int(10)? as u32;
                    let tick = int(1)? as u32;
                    out.objects.push((
                        tick,
                        ProjectileView {
                            kind: if int(2)? == 3 {
                                ObjectKind::Projectile
                            } else {
                                ObjectKind::Dormant
                            },
                            id: 0,
                            record: -1,
                            object: int(20)?,
                            definition: FourCC::new([0; 4]),
                            owner: 0,
                            target: None,
                            world,
                            position: sim.world_units_to_sim(world),
                            velocity,
                            facing: int(7)? as u16,
                            elevation: int(8)? as u16,
                            heading: 0,
                            pitch: 0,
                            launched_tick: 0,
                            collection: fourcc(field(3)?)?,
                            sequence: int(4)? as u8,
                            frame: int(5)? as i8,
                            view: int(6)? as u8,
                            scale: int(9)?,
                            flags,
                            hidden: false,
                            mirror_x: flags & 0x2_0000 != 0,
                            mirror_y: flags & 0x4_0000 != 0,
                            permutation: 0,
                            shading_slot: 0,
                            core: None,
                            transfer: (int(17)? as u8, int(18)? as u8, int(19)? as u8),
                        },
                    ));
                }
                "S" => out.stains.push(LoggedStain {
                    tick: int(1)? as u32,
                    collection: fourcc(field(2)?)?,
                    sequence: int(3)? as i16,
                    point: (int(4)?, int(5)?),
                    random: int(6)? as u16,
                    mirror: (int(7)? != 0, int(8)? != 0),
                }),
                other => anyhow::bail!("unknown trace record {other:?}"),
            }
        }
        Ok(out)
    }

    pub fn objects_at(&self, tick: u32) -> Vec<ProjectileView> {
        self.objects
            .iter()
            .filter(|(t, _)| *t == tick)
            .map(|(_, v)| v.clone())
            .collect()
    }

    pub fn stains_before(&self, tick: u32) -> impl Iterator<Item = &LoggedStain> {
        self.stains.iter().filter(move |s| s.tick < tick)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_log_parses_and_selects_by_tick() {
        let sim = Simulation::command_line_game(1);
        let t = ProjfxLog::parse(
            "# comment\n\
             P 3366 3 dwm2 24 2 1 49344 8320 256 327680 31907 19696 57 5 -55 -54 1 3 21 77\n\
             P 3367 4 guts 0 0 0 0 0 200 0 1 2 3 0 0 0 0 0 5 78\n\
             S 3366 dwm2 22 31907 19696 46339 1 0\n",
            &sim,
        )
        .unwrap();
        let a = t.objects_at(3366);
        assert_eq!(a.len(), 1);
        assert_eq!(a[0].collection, FourCC::new(*b"dwm2"));
        assert!(a[0].mirror_y);
        assert_eq!(a[0].transfer, (1, 3, 21));
        assert_eq!(t.objects_at(3367)[0].kind, ObjectKind::Dormant);
        assert_eq!(t.stains_before(3366).count(), 0);
        assert_eq!(t.stains_before(3367).count(), 1);
    }

    #[test]
    fn parse_handles_whitespace_lines_and_truncated_records() {
        let sim = Simulation::command_line_game(1);
        let parsed = ProjfxLog::parse("   \n\t  \n  # indented comment\n", &sim).unwrap();
        assert!(parsed.stains.is_empty());
        assert!(ProjfxLog::parse("S 10", &sim).is_err());
        assert!(ProjfxLog::parse("P 10 3", &sim).is_err());
    }
}
