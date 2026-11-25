use std::collections::BTreeMap;
use std::process::ExitCode;

use mandrake_rec::CommandType;
use mandrake_rec::tfl::TflRecording;

fn main() -> ExitCode {
    let mut ok = true;
    for path in std::env::args().skip(1) {
        let film = match std::fs::read(&path)
            .map_err(|e| e.to_string())
            .and_then(|b| TflRecording::from_bytes(&b).map_err(|e| e.to_string()))
        {
            Ok(f) => f,
            Err(e) => {
                println!("film {path}: {e}");
                ok = false;
                continue;
            }
        };
        let h = &film.header;
        println!(
            "film {path}: end tick {}, local player {}, {} players",
            h.end_tick,
            h.local_player,
            h.players.len()
        );
        for p in &h.players {
            println!(
                "player {} {:?} net id {} team {} captain {}",
                p.slot, p.name, p.net_id, p.team, p.captain
            );
        }
        let mut counts: BTreeMap<u8, BTreeMap<String, usize>> = BTreeMap::new();
        let mut lines = Vec::new();
        for t in &film.tick_commands {
            for c in &t.commands {
                if matches!(c.command_type, CommandType::Sync | CommandType::Null) {
                    continue;
                }
                let kind = format!("{:?}", c.command_type);
                *counts
                    .entry(c.player_index)
                    .or_default()
                    .entry(kind.clone())
                    .or_default() += 1;
                lines.push(format!(
                    "order tick {} player {} {kind}",
                    c.tick, c.player_index
                ));
            }
        }
        for (player, kinds) in &counts {
            let total: usize = kinds.values().sum();
            let each: Vec<String> = kinds.iter().map(|(k, n)| format!("{k} {n}")).collect();
            println!("orders player {player}: {total} ({})", each.join(", "));
        }
        for l in lines {
            println!("{l}");
        }
    }
    if ok {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    }
}
