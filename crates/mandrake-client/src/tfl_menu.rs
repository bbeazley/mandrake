pub(crate) use mandrake_dialogs::tfl as dialogs;

use crate::tfl_live::TflCarry;
use mandrake_common::FourCC;
pub(crate) use mandrake_session::tfl::{TFL_PROGRESS_FILE, TflLevelHeader, TflMovie, tfl_tag};
use mandrake_session::tfl::{TflGame, level_name, style_codes};
use mandrake_session::{Progress, Request, Session};
use mandrake_tag::TagArchive;
use mandrake_tag::tfl::menus::TflStringList;
use std::path::{Path, PathBuf};

impl mandrake_session::tfl::LevelCarry for TflCarry {
    fn counts(&self) -> (usize, usize) {
        (self.veterans.len(), self.permanent.len())
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum TflLaunch {
    Level { level: FourCC, difficulty: i16 },
    Film(PathBuf),
    Restore(PathBuf),
    Net(FourCC),
    Quit,
}

pub(crate) fn open_session(
    game_dir: &Path,
    our_films: Option<PathBuf>,
    progress: Option<PathBuf>,
) -> anyhow::Result<(Session, Option<ProgressFile>)> {
    let game = TflGame::new(game_dir, our_films).map_err(anyhow::Error::msg)?;
    Ok(tfl_session(game, progress))
}

fn tfl_session(game: TflGame, progress: Option<PathBuf>) -> (Session, Option<ProgressFile>) {
    let file = progress.map(|path| ProgressFile {
        path,
        game_dir: game.game_dir.clone(),
    });
    let (progress, file) = match file.as_ref().map(|f| (f.load(), f)) {
        Some((Ok(progress), _)) => (progress, file),
        Some((Err(e), f)) => {
            tracing::warn!(
                "TFL progress: {}: {e}; TFL's preferences, and nothing saved",
                f.path.display()
            );
            (None, None)
        }
        None => (None, None),
    };
    let session = Session::new_tfl(game, progress, crate::tfl_live::film::CHECKSUMS);
    (session, file)
}

pub(crate) struct ProgressFile {
    path: PathBuf,
    game_dir: PathBuf,
}

impl ProgressFile {
    fn load(&self) -> Result<Option<Progress>, String> {
        match crate::settings::load_progress(&self.path, &self.game_dir)? {
            crate::settings::SavedProgress::Own(text) => Progress::from_text(&text).map(Some),
            _ => Ok(None),
        }
    }

    fn save(&self, progress: &Progress) {
        if let Err(e) =
            crate::settings::save_progress(&self.path, &self.game_dir, &progress.to_text())
        {
            tracing::warn!("TFL progress: not saved to {}: {e}", self.path.display());
        }
    }
}

pub(crate) fn take_launch(
    session: &mut Session,
    progress: Option<&ProgressFile>,
) -> Option<TflLaunch> {
    let (launch, others) = take_launch_and_others(session, progress);
    for other in others {
        tracing::debug!(?other, "TFL menus: request not for TFL");
    }
    launch
}

pub(crate) fn take_launch_and_others(
    session: &mut Session,
    progress: Option<&ProgressFile>,
) -> (Option<TflLaunch>, Vec<Request>) {
    let mut launch = None;
    let mut others = Vec::new();
    for _ in 0..4 {
        let mut films = false;
        for request in session.take_requests() {
            match request {
                Request::SaveProgress(p) => {
                    if let Some(file) = progress {
                        file.save(&p);
                    }
                }
                Request::LoadGame(start) => launch = Some(launch_of(&start)),
                Request::Exit => launch = Some(TflLaunch::Quit),
                Request::ListSaves => {
                    list_saves(session, crate::tfl_live::save::saves_dir().as_deref())
                }
                Request::DeleteSave { file_name } => {
                    delete_save(crate::tfl_live::save::saves_dir().as_deref(), &file_name);
                }
                request @ (Request::ListFilms
                | Request::SaveFilm { .. }
                | Request::DeleteFilm { .. }
                | Request::RenameFilm { .. }) => {
                    film_request(session, request);
                    films = true;
                }
                other => others.push(other),
            }
        }
        if !films {
            break;
        }
    }
    (launch, others)
}

fn film_request(session: &mut Session, request: Request) {
    use mandrake_session::GameEvent;
    let Some(tfl) = session.tfl() else {
        return;
    };
    match request {
        Request::ListFilms => session.tfl_list_films(),
        Request::SaveFilm { name } => {
            let saved = match tfl.save_last_film(&name.0) {
                Ok(file) => {
                    println!("TFL menus: Save Film: {}", String::from_utf8_lossy(&file));
                    Some(file)
                }
                Err(e) => {
                    tracing::warn!(
                        "TFL menus: Save Film: {} wasn't saved: {e}",
                        name.to_plain_utf8()
                    );
                    None
                }
            };
            session.game_event(GameEvent::FilmSaved(saved));
        }
        Request::DeleteFilm { file_name } => {
            let shown = String::from_utf8_lossy(&file_name).into_owned();
            match tfl.delete_film(&file_name) {
                Ok(()) => println!("TFL menus: Delete Film: {shown}"),
                Err(e) => tracing::warn!("TFL menus: Delete Film: {shown} wasn't deleted: {e}"),
            }
        }
        Request::RenameFilm { file_name, name } => {
            let shown = String::from_utf8_lossy(&file_name).into_owned();
            let now = match tfl.rename_film(&file_name, &name.0) {
                Ok(now) => {
                    println!(
                        "TFL menus: Rename Film: {shown} is {}",
                        String::from_utf8_lossy(&now)
                    );
                    now
                }
                Err(e) => {
                    tracing::warn!("TFL menus: Rename Film: {shown} wasn't renamed: {e}");
                    file_name
                }
            };
            session.game_event(GameEvent::FilmRenamed(now));
        }
        _ => {}
    }
}

fn launch_of(start: &mandrake_session::GameStart) -> TflLaunch {
    if start.net.is_some() {
        return TflLaunch::Net(start.mesh);
    }
    match &start.film {
        Some(path) if start.restore => TflLaunch::Restore(path.clone()),
        Some(path) => TflLaunch::Film(path.clone()),
        None => TflLaunch::Level {
            level: start.mesh,
            difficulty: start.difficulty,
        },
    }
}

pub(crate) fn list_saves(session: &mut Session, saves: Option<&Path>) {
    let folder = saves_folder(session, saves);
    session.game_event(mandrake_session::GameEvent::SavesListed(folder));
}

fn saves_folder(session: &Session, saves: Option<&Path>) -> mandrake_session::saves::SaveFolder {
    let mut folder = crate::tfl_live::save::list_saves(saves, new_save_name(session));
    if let Some(tfl) = session.tfl() {
        for save in &mut folder.saves {
            name_level(&tfl.archive, save);
        }
    }
    folder
}

fn name_level(archive: &TagArchive, save: &mut mandrake_session::SaveFile) {
    let level = save.game.mesh;
    save.game.map_name = tfl_tag(archive, b"mesh", level)
        .and_then(|(_, d)| TflLevelHeader::from_mesh(&d))
        .map(|h| mandrake_session::Text(level_name(archive, level, &h)));
}

fn delete_save(saves: Option<&Path>, file_name: &[u8]) {
    let Some(dir) = saves else {
        tracing::error!("TFL menus: no folder for saved games");
        return;
    };
    let name = String::from_utf8_lossy(file_name);
    if name.contains(['/', '\\']) || name == ".." {
        tracing::error!("TFL menus: not a save's name: {name:?}");
        return;
    }
    let path = dir.join(name.as_ref());
    match std::fs::remove_file(&path) {
        Ok(()) => println!("TFL menus: deleted the save {}", path.display()),
        Err(e) => tracing::error!("TFL menus: {} wasn't deleted: {e}", path.display()),
    }
}

fn new_save_name(session: &Session) -> mandrake_session::Text {
    mandrake_session::Text(
        session
            .tfl()
            .and_then(|t| mandrake_session::tfl::tfl_string(&t.archive, FourCC(*b"inrf"), 1))
            .unwrap_or_else(|| b"New Saved Game".to_vec()),
    )
}

#[derive(Default)]
pub(crate) struct TflSnapshots {
    prompt: Option<mandrake_rec::saved_game::Snapshot>,
    prompt_wanted: bool,
    pending: Vec<(Vec<u8>, crate::tfl_live::save::TflSave, &'static str)>,
}

impl TflSnapshots {
    pub(crate) fn wanted(&self) -> bool {
        self.prompt_wanted || !self.pending.is_empty()
    }
}

fn write_tfl_save(
    session: &Session,
    saves: Option<&Path>,
    name: &[u8],
    save: &crate::tfl_live::save::TflSave,
) -> Option<mandrake_session::SaveFile> {
    let Some(dir) = saves else {
        tracing::error!("TFL game: no folder for saved games");
        return None;
    };
    match crate::tfl_live::save::write_save(dir, name, save) {
        Ok(mut file) => {
            println!(
                "TFL game: saved {:?} at tick {} as {}{}",
                String::from_utf8_lossy(name),
                file.end_tick,
                file.path.display(),
                if file.snapshot.is_some() {
                    ", with its snapshot"
                } else {
                    ""
                }
            );
            if let Some(tfl) = session.tfl() {
                name_level(&tfl.archive, &mut file);
            }
            Some(file)
        }
        Err(e) => {
            tracing::error!("TFL game: the save wasn't written: {e}");
            None
        }
    }
}

pub(crate) fn snapshot_drawn(
    session: &mut Session,
    snapshots: &mut TflSnapshots,
    snapshot: Option<mandrake_rec::saved_game::Snapshot>,
    saves: Option<&Path>,
) -> Vec<String> {
    if std::mem::take(&mut snapshots.prompt_wanted) {
        snapshots.prompt.clone_from(&snapshot);
        if let Some(s) = &snapshot {
            session.game_event(mandrake_session::GameEvent::Snapshot(s.clone()));
        }
    }
    let mut lines = Vec::new();
    for (name, mut save, line) in std::mem::take(&mut snapshots.pending) {
        save.snapshot.clone_from(&snapshot);
        let saved = write_tfl_save(session, saves, &name, &save);
        if saved.is_some() {
            lines.push(line.to_owned());
        }
        session.game_event(mandrake_session::GameEvent::GameSaved(saved));
    }
    lines
}

pub(crate) fn take_game_requests(
    session: &mut Session,
    progress: Option<&ProgressFile>,
    live: &crate::tfl_live::TflLive,
    saves: impl FnOnce() -> Option<PathBuf>,
    snapshots: &mut TflSnapshots,
) -> Option<TflLaunch> {
    let requests = session.take_requests();
    if requests.is_empty() {
        return None;
    }
    let mut launch = None;
    let mut saves_dir: Option<Option<PathBuf>> = None;
    let mut saves_fn = Some(saves);
    for request in requests {
        match request {
            Request::ListSaves => {
                let dir = saves_dir
                    .get_or_insert_with(|| saves_fn.take().and_then(|f| f()))
                    .as_deref();
                list_saves(session, dir);
            }
            Request::DeleteSave { file_name } => {
                let dir = saves_dir
                    .get_or_insert_with(|| saves_fn.take().and_then(|f| f()))
                    .as_deref();
                delete_save(dir, &file_name);
            }
            Request::TakeSnapshot => {
                snapshots.prompt = None;
                snapshots.prompt_wanted = true;
            }
            Request::SaveGame { name } => {
                let dir = saves_dir
                    .get_or_insert_with(|| saves_fn.take().and_then(|f| f()))
                    .as_deref();
                let saved = write_tfl_save(
                    session,
                    dir,
                    &name.0,
                    &live.saved_game(&name.0, snapshots.prompt.take()),
                );
                session.game_event(mandrake_session::GameEvent::GameSaved(saved));
            }
            Request::Quicksave => {
                let dir = saves_dir
                    .get_or_insert_with(|| saves_fn.take().and_then(|f| f()))
                    .as_deref();
                let name = mandrake_session::saves::quicksave_name(&saves_folder(session, dir));
                let save = live.saved_game(&name, None);
                snapshots.pending.push((name, save, "Quicksave created."));
            }
            Request::Autosave => {
                let now = crate::clock::SystemTime::now()
                    .duration_since(crate::clock::UNIX_EPOCH)
                    .map_or(0, |d| d.as_secs() as i64);
                let zone = mandrake_session::saves::LocalZone::system();
                let name = mandrake_session::saves::autosave_name(now, &zone);
                let save = live.saved_game(&name, None);
                snapshots.pending.push((name, save, "Autosave created."));
            }
            Request::BeginPlay => tracing::debug!("TFL game: begin play"),
            Request::SaveProgress(p) => {
                if let Some(file) = progress {
                    file.save(&p);
                }
            }
            Request::LoadGame(start) => launch = Some(launch_of(&start)),
            Request::Exit => launch = Some(TflLaunch::Quit),
            other => tracing::debug!(?other, "TFL game: request not for TFL"),
        }
    }
    launch
}

pub(crate) fn carry(session: &Session) -> TflCarry {
    session
        .tfl_carry()
        .and_then(|c| c.as_any().downcast_ref::<TflCarry>())
        .cloned()
        .unwrap_or_default()
}

fn report_strings(archive: &TagArchive) -> crate::carnage_report::ReportStrings {
    let list = |id: &[u8; 4], styles: bool| -> Vec<Vec<u8>> {
        tfl_tag(archive, b"stli", FourCC(*id))
            .and_then(|(v, d)| TflStringList::from_tag(FourCC(*id), v, &d).ok())
            .map(|l| {
                (0..l.len())
                    .map(|i| {
                        if styles {
                            style_codes(l.get(i))
                        } else {
                            l.get(i).to_vec()
                        }
                    })
                    .collect()
            })
            .unwrap_or_default()
    };
    crate::carnage_report::ReportStrings::epilogue_only(list(b"in03", true), list(b"indl", false))
}

fn mons_strings(archive: &TagArchive, mons: u32, offset: usize) -> Option<TflStringList> {
    let (_, d) = tfl_tag(archive, b"mons", FourCC(mons.to_be_bytes()))?;
    let id = FourCC(d.get(offset..offset + 4)?.try_into().ok()?);
    let (v, list) = tfl_tag(archive, b"stli", id)?;
    TflStringList::from_tag(id, v, &list).ok()
}

fn tfl_monster_spelling(archive: &TagArchive, mons: u32, count: i16) -> Option<Vec<u8>> {
    let list = mons_strings(archive, mons, 0x138)?;
    let k = usize::from(count != 1 && list.len() != 1);
    (k < list.len()).then(|| style_codes(list.get(k)))
}

pub(crate) fn tfl_unit_plural(archive: &TagArchive, unit: FourCC) -> Vec<u8> {
    let mons = tfl_tag(archive, b"unit", unit)
        .and_then(|(_, d)| Some(u32::from_be_bytes(d.get(..4)?.try_into().ok()?)));
    mons.and_then(|m| mons_strings(archive, m, 0x138))
        .filter(|l| l.len() > 1)
        .map(|l| style_codes(l.get(1)))
        .unwrap_or_default()
}

fn tfl_monster_name(archive: &TagArchive, mons: u32, index: i8) -> Option<Vec<u8>> {
    let list = mons_strings(archive, mons, 0x13c)?;
    let k = usize::try_from(index).ok()?;
    (k < list.len()).then(|| list.get(k).to_vec())
}

const IN02: FourCC = FourCC(*b"in02");
const IN02_TIME: usize = 7;
const IN02_SCORED: usize = 0xb;

pub(crate) fn carnage_pages(
    archive: &TagArchive,
    sim: &mandrake_tfl_sim::TflSim,
) -> Option<mandrake_session::FilmPages> {
    use mandrake_session::{FilmPages, Graph, GraphLine, RankRow, Text};
    let stats = &sim.statistics;
    let teams = sim.statistics_teams().ok()?;
    let header = &sim.data.film.header;
    let game_type = header.game_type;
    let in02 = tfl_tag(archive, b"stli", IN02)
        .and_then(|(v, d)| TflStringList::from_tag(IN02, v, &d).ok());
    let line = |n: usize| -> Vec<u8> {
        in02.as_ref()
            .filter(|l| n < l.len())
            .map(|l| style_codes(l.get(n)))
            .unwrap_or_default()
    };
    let colours = mandrake_tfl_sim::game_colours(&header.players);
    let color = |player: i16, secondary: bool| -> [u16; 3] {
        usize::try_from(player)
            .ok()
            .and_then(|p| colours.get(p))
            .map_or([0; 3], |(primary, second)| {
                let c = if secondary { second } else { primary };
                [c[0], c[1], c[2]]
            })
    };
    let team_player = |team: i16| -> i16 {
        usize::try_from(team)
            .ok()
            .and_then(|t| teams.claims.get(t).copied().flatten())
            .map_or(-1, |p| p as i16)
    };
    let name = |player: i16, team: bool| -> Text {
        usize::try_from(player)
            .ok()
            .and_then(|p| header.players.get(p))
            .map_or_else(
                || Text::from_plain_utf8(&format!("{}", player + 1)),
                |p| Text::from_plain_utf8(if team { &p.team_name } else { &p.name }),
            )
    };
    let time = |ticks: i32| -> Text {
        if ticks < 0 {
            return Text::default();
        }
        let format = line(IN02_TIME);
        let text = String::from_utf8_lossy(&format)
            .replacen("%d", &(ticks / 0x708).to_string(), 1)
            .replacen("%02d", &format!("{:02}", ticks % 0x708 / 0x1e), 1);
        Text::from_plain_utf8(&text)
    };
    let share = |part: i32, total: i32| -> Text {
        if total <= 0 {
            return Text::default();
        }
        Text::from_plain_utf8(&format!(
            "{:3.1}%",
            f64::from(part) / f64::from(total) * 100.0
        ))
    };
    let all_players: i32 = stats.players.iter().map(|r| r.damage.wrapping_abs()).sum();
    let all_teams: i32 = stats.teams.iter().map(|t| t.damage.wrapping_abs()).sum();
    let columns = mandrake_sim::statistics::ranking_columns(game_type);
    let player_team = |p: i16| -> i16 {
        usize::try_from(p)
            .ok()
            .and_then(|p| teams.player_teams.get(p).copied())
            .unwrap_or(-1)
    };
    let mut rank = Vec::new();
    for t in &stats.teams {
        let members: Vec<_> = stats
            .players
            .iter()
            .filter(|r| player_team(r.player) == t.team)
            .collect();
        if members.is_empty() {
            continue;
        }
        let in_team = members.len() > 1;
        let team_data = usize::try_from(t.team)
            .ok()
            .and_then(|k| stats.team_data.get(k))
            .copied()
            .unwrap_or_default();
        let score = match game_type {
            0 => share(t.damage, all_teams),
            1 | 2 | 5 | 6 if t.score != 0 => Text(line(IN02_SCORED)),
            7 | 8 | 10 => Text::from_plain_utf8(&t.score.to_string()),
            _ => Text::default(),
        };
        let mut rows = Vec::new();
        if in_team {
            let p = team_player(t.team);
            rows.push((
                name(p, true),
                (t.survivors, t.kills, t.losses),
                share(t.damage, all_teams),
                color(p, false),
                true,
                false,
            ));
        }
        for r in members {
            rows.push((
                name(r.player, false),
                (r.survivors, r.kills, r.losses),
                share(r.damage, all_players),
                color(r.player, in_team),
                false,
                in_team,
            ));
        }
        for (row_name, (survivors, kills, losses), damage, fill, is_team, under) in rows {
            let times = is_team || !under;
            let cells = (0u8..8)
                .map(|bit| {
                    if columns & (1 << bit) == 0 {
                        return Text::default();
                    }
                    match bit {
                        0 => score.clone(),
                        1 => row_name.clone(),
                        2 if times => time(team_data.last_taken),
                        3 if times => time(team_data.flag_time),
                        4 => Text::from_plain_utf8(&survivors.to_string()),
                        5 => Text::from_plain_utf8(&kills.to_string()),
                        6 => damage.clone(),
                        7 => Text::from_plain_utf8(&losses.to_string()),
                        _ => Text::default(),
                    }
                })
                .collect();
            rank.push(RankRow {
                color: fill,
                team: is_team,
                in_team: under,
                cells,
            });
        }
    }
    let perm = Some(
        stats
            .permanent
            .iter()
            .map(|r| {
                let score = r.points as f32 * 1.525_878_9e-5;
                let mut cells = vec![
                    Text::from_plain_utf8(&format!("{score:2.2}")),
                    name(r.player, false),
                ];
                cells.extend(
                    r.places
                        .iter()
                        .map(|p| Text::from_plain_utf8(&p.to_string())),
                );
                cells.push(Text::from_plain_utf8(&r.kills.to_string()));
                cells.push(Text::from_plain_utf8(
                    &mandrake_sim::statistics::format_damage(r.damage).to_string(),
                ));
                cells.push(Text::from_plain_utf8(&r.losses.to_string()));
                RankRow {
                    color: color(r.player, false),
                    team: false,
                    in_team: false,
                    cells,
                }
            })
            .collect(),
    );
    let count = i32::from(stats.sample.max(0));
    let local_team = sim.local_team().ok().flatten().unwrap_or(-1);
    let team_count = teams.teams;
    let value = |graph: i16, sample: usize, team: i16| -> i16 {
        let Some(m) = stats.samples.get(sample) else {
            return 0;
        };
        let mut sum = 0i32;
        for (attacker, row) in m.rows.iter().enumerate().take(16) {
            let from = player_team(attacker as i16);
            for victim in 0..16 {
                let to = player_team(victim as i16);
                let counted = match graph {
                    1 | 2 => from == team && to != team,
                    4 => from == local_team && to == team && to != local_team,
                    _ => false,
                };
                if counted {
                    sum = sum.wrapping_add(match graph {
                        2 => row[victim],
                        _ => row[16 + victim],
                    });
                }
            }
        }
        sum as i16
    };
    let graphs = (0i16..5)
        .map(|graph| {
            let valid = match graph {
                0 => game_type == 10,
                1 | 2 | 4 => true,
                _ => false,
            };
            valid.then(|| Graph {
                count,
                period: i32::from(stats.period),
                tick: i32::from(stats.counter),
                lines: (0..team_count as i16)
                    .map(|team| GraphLine {
                        color: color(team_player(team), false),
                        values: (0..count.max(0) as usize)
                            .map(|sample| value(graph, sample, team))
                            .collect(),
                    })
                    .collect(),
            })
        })
        .collect();
    Some(FilmPages {
        game_type,
        rank_columns: columns,
        rank,
        perm,
        graphs,
    })
}

pub(crate) fn epilogue_page(
    archive: &TagArchive,
    level: FourCC,
    difficulty: i16,
    stats: &mandrake_tfl_sim::statistics::Epilogue,
) -> mandrake_session::EpiloguePage {
    use mandrake_sim::statistics::{EpilogueEnemy, EpilogueHero, EpilogueReport, EpilogueUnit};
    let tag = |m: FourCC| u32::from_be_bytes(m.0);
    let report = EpilogueReport {
        difficulty,
        survivors: stats.survivors,
        units: stats
            .units
            .iter()
            .map(|u| EpilogueUnit {
                mons: tag(u.mons),
                damage_share: u.damage_share,
                kills: u.kills,
                dead: u.dead,
                fielded: u.fielded,
            })
            .collect(),
        heroes: stats
            .heroes
            .iter()
            .map(|h| EpilogueHero {
                mons: tag(h.mons),
                name_index: h.name_index as i8,
                custom_name: mandrake_session::Text(h.name.clone()).to_plain_utf8(),
                battles: i32::from(h.battles),
                experience: h.kills as u8,
            })
            .collect(),
        kills: stats.kills,
        enemies: stats
            .enemies
            .iter()
            .map(|e| EpilogueEnemy {
                mons: tag(e.mons),
                dead: e.dead,
                fielded: e.fielded,
            })
            .collect(),
        damage_ratio: stats.damage_ratio,
        kill_ratio: stats.kill_ratio,
    };
    let name = tfl_tag(archive, b"mesh", level)
        .and_then(|(_, d)| TflLevelHeader::from_mesh(&d))
        .map_or_else(
            || level.to_string().into_bytes(),
            |h| level_name(archive, level, &h),
        );
    crate::carnage_report::build_epilogue_with(
        &crate::carnage_report::TFL_EPILOGUE,
        &mandrake_session::Text(name),
        &report,
        &report_strings(archive),
        &|mons, count| tfl_monster_spelling(archive, mons, count),
        &|mons, index| tfl_monster_name(archive, mons, index),
    )
}
