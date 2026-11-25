use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use anyhow::{Context, anyhow, bail, ensure};
use mandrake_common::game_options::{COOPERATIVE, HOST_OBSERVER};
use mandrake_net::game_description::{FLAG_PASSWORD, GameDescription};
use mandrake_net::game_kind::GameKind;
use mandrake_net::tfl::DataChecksums;

pub const DEFAULT_PORT: u16 = 3453;
const TICKS_A_MINUTE: i32 = 1800;
pub const DEFAULT_TFL_DATA_WORDS: DataChecksums = [0x1841_8fcf, 0x22cb_50e1];

pub(crate) struct TableKeys<'a> {
    table: &'a dyn toml_edit::TableLike,
    seen: BTreeSet<&'static str>,
}

impl<'a> TableKeys<'a> {
    pub(crate) fn new(table: &'a dyn toml_edit::TableLike) -> Self {
        Self {
            table,
            seen: BTreeSet::new(),
        }
    }

    pub(crate) fn get(&mut self, key: &'static str) -> Option<&'a toml_edit::Item> {
        self.seen.insert(key);
        self.table.get(key)
    }

    pub(crate) fn finish(self, prefix: &str) -> anyhow::Result<()> {
        for (key, _) in self.table.iter() {
            ensure!(
                self.seen.contains(key),
                if prefix.is_empty() {
                    format!("unknown key {key:?}")
                } else {
                    format!("unknown key {prefix} {key:?}")
                }
            );
        }
        Ok(())
    }
}

#[derive(Debug, Clone)]
pub struct Config {
    pub kind: GameKind,
    pub tfl_data_words: DataChecksums,
    pub data_dir: PathBuf,
    pub plugins_dir: Option<PathBuf>,
    pub port: u16,
    pub name: String,
    pub films_dir: PathBuf,
    pub work_dir: PathBuf,
    pub download_url: Option<String>,
    pub game: GameConfig,
}

pub const DEFAULT_DOWNLOAD_URL: &str = "http://{host}:8080/plugins/";

#[derive(Debug, Clone, Default)]
pub struct GameConfig {
    pub description_file: Option<PathBuf>,
    pub map: Option<[u8; 4]>,
    pub game_type: Option<i16>,
    pub time_limit_minutes: Option<i32>,
    pub difficulty: Option<i16>,
    pub max_players: Option<i16>,
    pub cooperative: Option<bool>,
    pub password: String,
}

impl Config {
    pub fn load(path: &Path) -> anyhow::Result<Config> {
        let text =
            std::fs::read_to_string(path).with_context(|| format!("config {}", path.display()))?;
        let base = path.parent().unwrap_or(Path::new("."));
        Config::parse(&text, base).with_context(|| format!("config {}", path.display()))
    }

    pub fn parse(text: &str, base: &Path) -> anyhow::Result<Config> {
        let doc = toml_edit::Document::parse(text.to_string()).map_err(|e| anyhow!("{e}"))?;
        let mut root = TableKeys::new(doc.as_table());
        let data_dir = string(root.get("data_dir"), "data_dir")?
            .map(|s| base.join(s))
            .ok_or_else(|| anyhow!("no data_dir"))?;
        let kind = match string(root.get("game_kind"), "game_kind")?.as_deref() {
            None | Some("myth2") => GameKind::Myth2,
            Some("tfl") => GameKind::Tfl,
            Some(other) => bail!("game_kind {other:?}: \"myth2\" or \"tfl\""),
        };
        let tfl_words_item = root.get("tfl_data_words");
        let has_tfl_words = tfl_words_item.is_some();
        let tfl_data_words = match tfl_words_item {
            None => DEFAULT_TFL_DATA_WORDS,
            Some(item) => {
                let words: Vec<u32> = item
                    .as_array()
                    .map(|a| {
                        a.iter()
                            .filter_map(|v| v.as_integer().and_then(|v| u32::try_from(v).ok()))
                            .collect()
                    })
                    .unwrap_or_default();
                ensure!(
                    words.len() == 2,
                    "tfl_data_words: two 32-bit words, the install's first and second data checksums"
                );
                [words[0], words[1]]
            }
        };
        ensure!(
            kind == GameKind::Tfl || !has_tfl_words,
            "tfl_data_words is a TFL instance's (game_kind = \"tfl\")"
        );
        let port = match integer(root.get("port"), "port")? {
            None => DEFAULT_PORT,
            Some(p) => u16::try_from(p).map_err(|_| anyhow!("port {p}"))?,
        };
        let game = match root.get("game") {
            None => GameConfig::default(),
            Some(item) => {
                let t = item
                    .as_table_like()
                    .ok_or_else(|| anyhow!("[game] is not a table"))?;
                GameConfig::parse(t, base)?
            }
        };
        let download_url = match string(root.get("download_url"), "download_url")? {
            None => Some(DEFAULT_DOWNLOAD_URL.to_string()),
            Some(u) if u.is_empty() => None,
            Some(u) => {
                ensure!(
                    (u.starts_with("http://") || u.starts_with("https://"))
                        && u.ends_with('/')
                        && u.len() <= mandrake_net::ours::DOWNLOADS_URL_MAX
                        && !u.contains(char::is_control),
                    "download_url {u:?}: an http:// or https:// URL ending in /"
                );
                Some(u)
            }
        };
        let download_url = download_url.filter(|_| kind == GameKind::Myth2);
        let plugins_dir = string(root.get("plugins_dir"), "plugins_dir")?.map(|s| base.join(s));
        let name = string(root.get("name"), "name")?.unwrap_or_else(|| "Myth II Server".into());
        let films_dir = string(root.get("films_dir"), "films_dir")?
            .map(|s| base.join(s))
            .unwrap_or_else(|| base.join("films"));
        let work_dir = string(root.get("work_dir"), "work_dir")?
            .map(|s| base.join(s))
            .unwrap_or_else(|| base.join("work"));
        root.finish("")?;
        Ok(Config {
            kind,
            tfl_data_words,
            data_dir,
            download_url,
            plugins_dir,
            port,
            name,
            films_dir,
            work_dir,
            game,
        })
    }

    pub fn game_dir(&self) -> anyhow::Result<PathBuf> {
        let Some(plugins) = self
            .plugins_dir
            .as_ref()
            .filter(|_| self.kind == GameKind::Myth2)
        else {
            return Ok(self.data_dir.clone());
        };
        if same(plugins, &self.data_dir.join("plugins")) {
            return Ok(self.data_dir.clone());
        }
        let dir = self.work_dir.join("game");
        if dir.exists() {
            std::fs::remove_dir_all(&dir).with_context(|| format!("{}", dir.display()))?;
        }
        std::fs::create_dir_all(&dir).with_context(|| format!("{}", dir.display()))?;
        let abs = |p: &Path| std::fs::canonicalize(p).with_context(|| format!("{}", p.display()));
        for e in std::fs::read_dir(&self.data_dir)
            .with_context(|| format!("data_dir {}", self.data_dir.display()))?
        {
            let e = e?;
            if e.file_name() != "plugins" {
                link(&abs(&e.path())?, &dir.join(e.file_name()))?;
            }
        }
        link(&abs(plugins)?, &dir.join("plugins"))?;
        Ok(dir)
    }

    pub fn description(&self) -> anyhow::Result<GameDescription> {
        let g = &self.game;
        let mut d = match &g.description_file {
            Some(path) => {
                let bytes = std::fs::read(path)
                    .with_context(|| format!("description_file {}", path.display()))?;
                let m = mandrake_net::wire::Message::decode(&bytes)
                    .map_err(|e| anyhow!("description_file {}: {e}", path.display()))?;
                GameDescription::from_body(&m.body)
                    .map_err(|e| anyhow!("description_file {}: {e}", path.display()))?
            }
            None if self.kind == GameKind::Tfl => mandrake_session::net::tfl_host_description(
                &mandrake_session::Text(self.name.as_bytes().to_vec()),
                &mandrake_session::Text(g.password.as_bytes().to_vec()),
                self.tfl_data_words,
            ),
            None => mandrake_session::net::host_description(
                &mandrake_session::Text(self.name.as_bytes().to_vec()),
                &mandrake_session::Text(g.password.as_bytes().to_vec()),
                0,
            ),
        };
        ensure!(
            GameKind::of(&d) == self.kind,
            "description_file: a {:?} game on a {:?} server",
            GameKind::of(&d),
            self.kind
        );
        d.game_name = self.name.as_bytes().to_vec();
        if let Some(mesh) = g.map {
            d.parameters.mesh = mesh;
        }
        if let Some(t) = g.game_type {
            d.parameters.game_type = t;
        }
        if let Some(m) = g.time_limit_minutes {
            d.parameters.time_limit = m
                .checked_mul(TICKS_A_MINUTE)
                .ok_or_else(|| anyhow!("[game] time_limit_minutes {m} overflows ticks"))?;
        }
        if let Some(v) = g.difficulty {
            d.parameters.difficulty = v;
        }
        if let Some(v) = g.max_players {
            d.parameters.max_players = v;
        }
        match g.cooperative {
            Some(true) => d.parameters.options |= COOPERATIVE,
            Some(false) => d.parameters.options &= !COOPERATIVE,
            None => {}
        }
        if self.kind == GameKind::Myth2 {
            d.parameters.options |= HOST_OBSERVER;
        }
        if g.password.is_empty() {
            d.flags &= !FLAG_PASSWORD;
        } else {
            d.flags |= FLAG_PASSWORD;
        }
        ensure!(d.parameters.mesh != [0; 4], "[game]: no map");
        Ok(d)
    }
}

impl GameConfig {
    fn parse(t: &dyn toml_edit::TableLike, base: &Path) -> anyhow::Result<GameConfig> {
        let mut keys = TableKeys::new(t);
        let map = match string(keys.get("map"), "map")? {
            None => None,
            Some(m) => {
                let b = m.as_bytes();
                ensure!(
                    b.len() == 4,
                    "[game] map {m:?}: a mesh tag is four characters"
                );
                Some([b[0], b[1], b[2], b[3]])
            }
        };
        let cooperative = match keys.get("cooperative") {
            None => None,
            Some(i) => Some(
                i.as_bool()
                    .ok_or_else(|| anyhow!("[game] cooperative: not true or false"))?,
            ),
        };
        let description_file =
            string(keys.get("description_file"), "description_file")?.map(|s| base.join(s));
        let game_type = short(&mut keys, "game_type")?;
        let time_limit_minutes = integer(keys.get("time_limit_minutes"), "time_limit_minutes")?
            .map(|v| i32::try_from(v).map_err(|_| anyhow!("time_limit_minutes {v}")))
            .transpose()?;
        let difficulty = short(&mut keys, "difficulty")?;
        let max_players = short(&mut keys, "max_players")?;
        let password = string(keys.get("password"), "password")?.unwrap_or_default();
        keys.finish("[game]")?;
        Ok(GameConfig {
            description_file,
            map,
            game_type,
            time_limit_minutes,
            difficulty,
            max_players,
            cooperative,
            password,
        })
    }
}

fn short(keys: &mut TableKeys<'_>, key: &'static str) -> anyhow::Result<Option<i16>> {
    integer(keys.get(key), key)?
        .map(|v| i16::try_from(v).map_err(|_| anyhow!("[game] {key} {v}")))
        .transpose()
}

fn string(item: Option<&toml_edit::Item>, key: &str) -> anyhow::Result<Option<String>> {
    match item {
        None => Ok(None),
        Some(i) => match i.as_str() {
            Some(s) => Ok(Some(s.to_string())),
            None => bail!("{key}: not a string"),
        },
    }
}

fn integer(item: Option<&toml_edit::Item>, key: &str) -> anyhow::Result<Option<i64>> {
    match item {
        None => Ok(None),
        Some(i) => match i.as_integer() {
            Some(v) => Ok(Some(v)),
            None => bail!("{key}: not an integer"),
        },
    }
}

fn same(a: &Path, b: &Path) -> bool {
    match (std::fs::canonicalize(a), std::fs::canonicalize(b)) {
        (Ok(a), Ok(b)) => a == b,
        _ => false,
    }
}

#[cfg(unix)]
fn link(from: &Path, to: &Path) -> anyhow::Result<()> {
    std::os::unix::fs::symlink(from, to)
        .with_context(|| format!("link {} -> {}", to.display(), from.display()))
}

#[cfg(not(unix))]
fn link(_from: &Path, _to: &Path) -> anyhow::Result<()> {
    bail!("plugins_dir apart from data_dir needs symbolic links (Unix)")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_config_reads_with_its_defaults_and_paths() {
        let c = Config::parse(
            "data_dir = \"/data/myth2\"\nname = \"Ours\"\n[game]\nmap = \"wabe\"\n\
             time_limit_minutes = 1\ngame_type = 4\n",
            Path::new("/etc/m2"),
        )
        .unwrap();
        assert_eq!(c.data_dir, PathBuf::from("/data/myth2"));
        assert_eq!(c.port, DEFAULT_PORT);
        assert_eq!(c.films_dir, PathBuf::from("/etc/m2/films"));
        assert_eq!(c.download_url.as_deref(), Some(DEFAULT_DOWNLOAD_URL));
        let d = c.description().unwrap();
        assert_eq!(&d.parameters.mesh, b"wabe");
        assert_eq!(d.parameters.time_limit, 1800);
        assert_eq!(d.parameters.game_type, 4);
        assert_eq!(d.game_name, b"Ours");
        assert_ne!(
            d.parameters.options & HOST_OBSERVER,
            0,
            "Server is Observer"
        );
        assert_eq!(d.flags & FLAG_PASSWORD, 0);
    }

    #[test]
    fn unknown_keys_and_bad_values_are_refused() {
        let base = Path::new(".");
        assert!(Config::parse("port = 1\n", base).is_err(), "no data_dir");
        assert!(Config::parse("data_dir = \"d\"\nprot = 1\n", base).is_err());
        assert!(Config::parse("data_dir = \"d\"\nport = 70000\n", base).is_err());
        assert!(Config::parse("data_dir = \"d\"\n[game]\nmap = \"long\"\nx = 1\n", base).is_err());
        assert!(Config::parse("data_dir = \"d\"\n[game]\nmap = \"abc\"\n", base).is_err());
        let url =
            |u: &str| Config::parse(&format!("data_dir = \"d\"\ndownload_url = \"{u}\"\n"), base);
        assert_eq!(url("").unwrap().download_url, None);
        assert_eq!(
            url("https://myth.example/plugins/")
                .unwrap()
                .download_url
                .as_deref(),
            Some("https://myth.example/plugins/")
        );
        assert!(url("ftp://x/").is_err());
        assert!(url("http://x/plugins").is_err());
        assert!(Config::parse("data_dir = \"d\"\ngame_kind = \"tfl2\"\n", base).is_err());
        assert!(
            Config::parse("data_dir = \"d\"\ntfl_data_words = [1, 2]\n", base).is_err(),
            "a Myth II instance's"
        );
        assert!(
            Config::parse(
                "data_dir = \"d\"\ngame_kind = \"tfl\"\ntfl_data_words = [1]\n",
                base
            )
            .is_err()
        );
        let overflow = Config::parse(
            "data_dir = \"d\"\n[game]\nmap = \"85gi\"\ntime_limit_minutes = 2000000\n",
            base,
        )
        .unwrap();
        assert!(overflow.description().is_err());
    }

    #[test]
    fn a_tfl_instance_offers_tfls_game_with_no_observer() {
        let c = Config::parse(
            "game_kind = \"tfl\"\ndata_dir = \"/data/tfl\"\nplugins_dir = \"/data/plugins\"\n\
             name = \"TFL\"\n[game]\ntime_limit_minutes = 2\n",
            Path::new("/etc/m2"),
        )
        .unwrap();
        assert_eq!(c.kind, GameKind::Tfl);
        assert_eq!(c.tfl_data_words, DEFAULT_TFL_DATA_WORDS);
        assert_eq!(c.download_url, None, "no plugins");
        assert_eq!(c.game_dir().unwrap(), PathBuf::from("/data/tfl"));
        let d = c.description().unwrap();
        assert_eq!(GameKind::of(&d), GameKind::Tfl);
        assert_eq!(&d.parameters.mesh, b"58cm");
        assert_eq!(d.parameters.time_limit, 3600);
        assert_eq!(
            d.parameters.options & HOST_OBSERVER,
            0,
            "TFL has no observer"
        );
        assert_eq!(
            mandrake_net::tfl::check_data(&d, DEFAULT_TFL_DATA_WORDS),
            Ok(()),
            "our TFL clients' data words"
        );
        let words = Config::parse(
            "game_kind = \"tfl\"\ndata_dir = \"d\"\ntfl_data_words = [0x11, 0xffffffff]\n",
            Path::new("."),
        )
        .unwrap();
        assert_eq!(words.tfl_data_words, [0x11, u32::MAX]);
        let d = words.description().unwrap();
        assert_eq!(mandrake_net::tfl::plugin_checksum(&d), None);
    }
}
