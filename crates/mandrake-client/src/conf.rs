use crate::args::Args;
use clap::Parser;
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::sync::RwLock;

pub const DEFAULT_NAME: &str = "mandrake.toml";

pub const DEFAULT_CONF_TOML: &str = include_str!("../mandrake.default.toml");

pub const MODERNRETRO_CONF_TOML: &str = include_str!("../mandrake.modernretro.toml");

pub fn shipped_table(table: &[&str]) -> String {
    table_text(DEFAULT_CONF_TOML, table)
}

#[cfg(not(target_arch = "wasm32"))]
fn dir_is_writable(dir: &Path) -> bool {
    let check_file = dir.join(format!(".mandrake-write-check-{}", std::process::id()));
    if std::fs::write(&check_file, b"").is_ok() {
        let _ = std::fs::remove_file(&check_file);
        true
    } else {
        false
    }
}

pub fn portable_config_dir() -> Option<PathBuf> {
    if cfg!(test) || cfg!(target_arch = "wasm32") {
        return None;
    }
    mandrake_common::data_dirs::portable_exe_dir()
}

pub fn default_path() -> Option<PathBuf> {
    #[cfg(not(target_arch = "wasm32"))]
    if let Some(dir) = portable_config_dir() {
        let portable = dir.join(DEFAULT_NAME);
        if portable.is_file() || dir_is_writable(&dir) {
            return Some(portable);
        }
    }
    crate::user_dirs::config_file(DEFAULT_NAME)
}

pub fn resolve_in(conf: &Path, folder: Option<&Path>) -> PathBuf {
    let bare = conf.parent().is_some_and(|p| p.as_os_str().is_empty());
    match folder {
        Some(folder) if bare => folder.join(conf),
        _ => conf.to_path_buf(),
    }
}

pub fn resolve(conf: &Path) -> PathBuf {
    let bare = conf.parent().is_some_and(|p| p.as_os_str().is_empty());
    if bare && let Some(dir) = portable_config_dir() {
        let beside = dir.join(conf);
        if crate::config_store::is_file(&beside) {
            return beside;
        }
    }
    resolve_in(conf, crate::user_dirs::config_dir().as_deref())
}

pub fn ensure_default_conf(conf: Option<&Path>, headless: bool) {
    if conf.is_some() || headless || cfg!(test) || cfg!(target_arch = "wasm32") {
        return;
    }
    let Some(path) = default_path() else {
        return;
    };
    if crate::config_store::is_file(&path) {
        return;
    }
    migrate(None);
    if crate::config_store::is_file(&path) {
        return;
    }
    if let Err(e) = crate::config_store::write(&path, MODERNRETRO_CONF_TOML) {
        eprintln!("settings: cannot create {}: {e}", path.display());
    }
}

pub fn to_read(conf: Option<&Path>, headless: bool) -> Option<PathBuf> {
    match conf {
        Some(path) => Some(path.to_path_buf()),
        None if headless => None,
        None => default_path().filter(|p| crate::config_store::is_file(p)),
    }
}

static IN_USE: RwLock<Option<PathBuf>> = RwLock::new(None);

pub fn set_in_use(conf: Option<PathBuf>) {
    if let Ok(mut in_use) = IN_USE.write() {
        *in_use = conf;
    }
}

pub fn in_use() -> Option<PathBuf> {
    IN_USE
        .read()
        .ok()
        .and_then(|c| c.clone())
        .or_else(default_path)
}

pub type TablePath<'a> = &'a [&'a str];

struct Header {
    start: usize,
    end: usize,
    path: Vec<String>,
    array: bool,
}

fn headers(table: &toml_edit::Table, path: &mut Vec<String>, out: &mut Vec<Header>) {
    for (key, item) in table.iter() {
        path.push(key.to_owned());
        match item {
            toml_edit::Item::Table(t) => {
                if let Some(span) = t.span().filter(|_| !t.is_implicit() && !t.is_dotted()) {
                    out.push(Header {
                        start: span.start,
                        end: span.end,
                        path: path.clone(),
                        array: false,
                    });
                }
                headers(t, path, out);
            }
            toml_edit::Item::ArrayOfTables(a) => {
                for t in a.iter() {
                    if let Some(span) = t.span() {
                        out.push(Header {
                            start: span.start,
                            end: span.end,
                            path: path.clone(),
                            array: true,
                        });
                    }
                    headers(t, path, out);
                }
            }
            _ => {}
        }
        path.pop();
    }
}

pub(crate) fn toml_string(text: &str) -> String {
    let mut out = String::from("\"");
    for c in text.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            c if c.is_control() => out.push_str(&format!("\\u{:04X}", u32::from(c))),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

fn key_repr(key: &str) -> String {
    let bare = !key.is_empty()
        && key
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-');
    if bare {
        key.to_owned()
    } else {
        toml_string(key)
    }
}

fn header_line(path: &[String], array: bool) -> String {
    let keys: Vec<String> = path.iter().map(|k| key_repr(k)).collect();
    if array {
        format!("[[{}]]", keys.join("."))
    } else {
        format!("[{}]", keys.join("."))
    }
}

fn written_inline(doc: &toml_edit::Table, table: TablePath) -> bool {
    let mut at = doc;
    for key in table {
        match at.get(key) {
            Some(toml_edit::Item::Table(t)) => {
                if t.is_dotted() {
                    return true;
                }
                at = t;
            }
            Some(toml_edit::Item::Value(_)) => return true,
            _ => return false,
        }
    }
    false
}

fn lines_by_header<'t>(
    text: &'t str,
    found: &'t [Header],
) -> impl Iterator<Item = (&'t str, Option<&'t Header>, bool)> + 't {
    let mut next = 0;
    let mut offset = 0;
    let mut current: Option<&Header> = None;
    text.split_inclusive('\n').map(move |line| {
        let end = offset + line.len();
        let mut own = false;
        while next < found.len() && found[next].start < end {
            current = Some(&found[next]);
            own = true;
            next += 1;
        }
        offset = end;
        (line, current, own)
    })
}

fn sorted_headers(doc: &toml_edit::Table) -> Vec<Header> {
    let mut found = Vec::new();
    headers(doc, &mut Vec::new(), &mut found);
    found.sort_by_key(|h| h.start);
    found
}

pub fn table_text(text: &str, table: TablePath) -> String {
    let Ok(doc) = toml_edit::Document::parse(text.to_owned()) else {
        return text.to_owned();
    };
    if written_inline(doc.as_table(), table) {
        eprintln!(
            "settings: `{}` is written inline or with dotted keys; write it as a [{}] table. \
             Its shipped values are used",
            table.join("."),
            table.join(".")
        );
        return String::new();
    }
    let found = sorted_headers(doc.as_table());
    let ours =
        |path: &[String]| path.len() >= table.len() && path.iter().zip(table).all(|(a, b)| a == b);
    let mut out = String::with_capacity(text.len());
    for (line, header, own) in lines_by_header(text, &found) {
        let newline = if line.ends_with("\r\n") {
            "\r\n"
        } else if line.ends_with('\n') {
            "\n"
        } else {
            ""
        };
        let keep = header.map_or(table.is_empty(), |h| ours(&h.path));
        match header {
            Some(h) if own => {
                if keep && h.path.len() > table.len() {
                    out.push_str(&header_line(&h.path[table.len()..], h.array));
                }
            }
            _ if keep => out.push_str(line.trim_end_matches(['\r', '\n'])),
            _ => {}
        }
        out.push_str(newline);
    }
    out
}

pub fn read_table(path: &Path, table: TablePath) -> std::io::Result<String> {
    crate::config_store::read_to_string(path).map(|text| table_text(&text, table))
}

pub fn load_table<T>(
    path: Option<PathBuf>,
    table: TablePath,
    defaults: impl FnOnce() -> T,
    parse: impl FnOnce(&str, &str) -> (T, Vec<crate::controls::Diagnostic>),
) -> (T, Option<PathBuf>, Vec<crate::controls::Diagnostic>) {
    let Some(path) = path else {
        return (defaults(), None, Vec::new());
    };
    match read_table(&path, table) {
        Ok(text) => {
            let (settings, diags) = parse(&text, &path.display().to_string());
            (settings, Some(path), diags)
        }
        Err(e) => (
            defaults(),
            None,
            vec![crate::controls::Diagnostic {
                source: path.display().to_string(),
                line: None,
                message: format!("cannot read settings file: {e}; using the defaults"),
            }],
        ),
    }
}

fn split_table(text: &str, table: &str) -> Option<(String, String)> {
    let doc = toml_edit::Document::parse(text.to_owned()).ok()?;
    let found = sorted_headers(doc.as_table());
    let (mut ours, mut rest) = (String::new(), String::new());
    for (line, header, _) in lines_by_header(text, &found) {
        let ours_line = header.is_some_and(|h| h.path.first().is_some_and(|k| k == table));
        let to = if ours_line { &mut ours } else { &mut rest };
        to.push_str(line);
        if !line.ends_with('\n') {
            to.push('\n');
        }
    }
    Some((ours, rest))
}

#[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
pub fn with_table_of(text: &str, from: &str, table: &str) -> Result<String, String> {
    let Some((theirs, _)) = split_table(from, table).filter(|(t, _)| !t.is_empty()) else {
        return Ok(text.to_owned());
    };
    let (_, rest) = split_table(text, table).ok_or("the shipped file is not TOML")?;
    let gap = if rest.is_empty() || rest.ends_with("\n\n") {
        ""
    } else {
        "\n"
    };
    let out = format!("{rest}{gap}{theirs}");
    toml_edit::Document::parse(out.clone()).map_err(|e| e.message().to_owned())?;
    Ok(out)
}

const NEW_FILE_HEADER: &str = "# Myth II source port settings, merged over the shipped ones \
(mandrake --write-default-conf FILE writes them all,\n# with their comments). The client writes \
the control bar, Ctrl +/- zoom and the multiplayer name here.\n";

#[derive(Clone, Debug, PartialEq)]
pub enum Value {
    Bool(bool),
    Int(i64),
    Str(String),
    Ints(Vec<i64>),
}

impl From<bool> for Value {
    fn from(v: bool) -> Self {
        Self::Bool(v)
    }
}

impl From<i64> for Value {
    fn from(v: i64) -> Self {
        Self::Int(v)
    }
}

impl From<&str> for Value {
    fn from(v: &str) -> Self {
        Self::Str(v.to_owned())
    }
}

impl From<String> for Value {
    fn from(v: String) -> Self {
        Self::Str(v)
    }
}

impl Value {
    fn repr(&self) -> String {
        match self {
            Self::Bool(b) => b.to_string(),
            Self::Int(i) => i.to_string(),
            Self::Str(s) => toml_string(s),
            Self::Ints(v) => {
                let v: Vec<String> = v.iter().map(i64::to_string).collect();
                format!("[{}]", v.join(", "))
            }
        }
    }
}

pub fn with_values(
    text: &str,
    table: TablePath,
    values: &[(&str, Value)],
) -> Result<String, String> {
    let message = |e: toml_edit::TomlError| e.message().to_owned();
    let doc = toml_edit::Document::parse(text.to_owned()).map_err(message)?;
    if written_inline(doc.as_table(), table) {
        return Err(format!(
            "`{}` is written inline or with dotted keys; left as it is",
            table.join(".")
        ));
    }
    let mut at = Some(doc.as_table());
    for key in table {
        at = at.and_then(|t| t.get(key)).and_then(|i| i.as_table());
    }
    let header = at.filter(|t| !t.is_implicit()).and_then(|t| t.span());
    let mut edits: Vec<(std::ops::Range<usize>, String)> = Vec::new();
    let mut added = String::new();
    for (key, value) in values {
        match at.and_then(|t| t.get(key)) {
            Some(toml_edit::Item::Value(old)) if header.is_some() => {
                let span = old.span().ok_or("a value without a place")?;
                edits.push((span, value.repr()));
            }
            Some(toml_edit::Item::Value(_)) | None => {
                added.push_str(&format!("{} = {}\n", key_repr(key), value.repr()));
            }
            Some(_) => return Err(format!("`{key}` is a table")),
        }
    }
    let mut out = text.to_owned();
    if !added.is_empty() {
        match &header {
            Some(span) => {
                let line_end = text[span.end..]
                    .find('\n')
                    .map_or(text.len(), |i| span.end + i + 1);
                let lead = if line_end == text.len() && !text.ends_with('\n') {
                    "\n"
                } else {
                    ""
                };
                edits.push((line_end..line_end, format!("{lead}{added}")));
            }
            None => {
                let gap = match () {
                    _ if text.is_empty() || text.ends_with("\n\n") => "",
                    _ if text.ends_with('\n') => "\n",
                    _ => "\n\n",
                };
                let path: Vec<String> = table.iter().map(|k| (*k).to_owned()).collect();
                let at_end = text.len();
                edits.push((
                    at_end..at_end,
                    format!("{gap}{}\n{added}", header_line(&path, false)),
                ));
            }
        }
    }
    edits.sort_by_key(|(range, _)| std::cmp::Reverse(range.start));
    for (range, what) in edits {
        out.replace_range(range, &what);
    }
    toml_edit::Document::parse(out.clone()).map_err(message)?;
    Ok(out)
}

pub fn set_values(path: &Path, table: TablePath, values: &[(&str, Value)]) -> Result<(), String> {
    let (text, exists) = match crate::config_store::read_to_string(path) {
        Ok(text) => (text, true),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => (NEW_FILE_HEADER.to_owned(), false),
        Err(e) => return Err(e.to_string()),
    };
    let new = with_values(&text, table, values)?;
    if exists && new == text {
        return Ok(());
    }
    crate::config_store::write_replacing(path, "toml.new", &new).map_err(|e| e.to_string())
}

pub const PRESET_KEYS: [(&str, &str); 14] = [
    ("ui_filter", "--ui-filter"),
    ("ui_scale", "--ui-scale"),
    ("sprite_filter", "--sprite-filter"),
    ("sampling", "--sampling"),
    ("lighting", "--lighting"),
    ("fog", "--fog"),
    ("detail_shadows", "--detail-shadows"),
    ("water", "--water"),
    ("water_maps", "--water-maps"),
    ("detail_textures", "--detail-textures"),
    ("vsync", "--vsync"),
    ("ten_bit", "--ten-bit"),
    ("hdr", "--hdr"),
    ("fps_cap", "--fps-cap"),
];

fn flag_word(item: &toml_edit::Item) -> Option<String> {
    let value = item.as_value()?;
    match value {
        toml_edit::Value::String(s) => Some(s.value().clone()),
        toml_edit::Value::Integer(i) => Some(i.value().to_string()),
        toml_edit::Value::Float(f) => Some(f.value().to_string()),
        toml_edit::Value::Boolean(b) => Some(if *b.value() { "on" } else { "off" }.to_owned()),
        toml_edit::Value::Array(a) => a
            .iter()
            .map(|v| v.as_str().map(str::to_owned))
            .collect::<Option<Vec<_>>>()
            .map(|v| v.join(",")),
        _ => None,
    }
}

pub fn preset_flags(display: &str) -> (Vec<(&'static str, [String; 2])>, Vec<String>) {
    let mut flags = Vec::new();
    let mut problems = Vec::new();
    let Ok(doc) = toml_edit::Document::parse(display.to_owned()) else {
        return (flags, problems);
    };
    for (key, flag) in PRESET_KEYS {
        let Some(item) = doc.as_table().get(key) else {
            continue;
        };
        match flag_word(item) {
            Some(word) => flags.push((key, [flag.to_owned(), word])),
            None => problems.push(format!("`{key}` must be what {flag} takes")),
        }
    }
    (flags, problems)
}

pub fn parse_args(words: Vec<OsString>) -> Result<Args, clap::Error> {
    use clap::{CommandFactory, FromArgMatches};
    let fail =
        |message: String| clap::Error::raw(clap::error::ErrorKind::ValueValidation, message + "\n");
    let matches = Args::command().try_get_matches_from(&words)?;
    let mut args = Args::from_arg_matches(&matches)?;
    args.conf = args.conf.as_deref().map(resolve);
    if let Some(path) = &args.conf
        && !crate::config_store::is_file(path)
    {
        return Err(fail(format!("--conf: {}: no such file", path.display())));
    }
    let headless = args.headless || args.screenshot.is_some();
    ensure_default_conf(args.conf.as_deref(), headless);
    let Some(path) = to_read(args.conf.as_deref(), headless) else {
        mandrake_common::data_dirs::set_from_settings(None, None);
        return Ok(args);
    };
    let text = crate::config_store::read_to_string(&path)
        .map_err(|e| fail(format!("--conf: {}: {e}", path.display())))?;
    let text = match with_new_look_keys(&text) {
        Some((new, moved)) => {
            let written = !headless
                && match crate::config_store::write_replacing(&path, "toml.new", &new) {
                    Ok(()) => true,
                    Err(e) => {
                        eprintln!("settings: cannot rewrite {}: {e}", path.display());
                        false
                    }
                };
            println!("{}", old_keys_notice(&path, &moved, written));
            new
        }
        None => text,
    };
    let (game, problems) = parse_game(&table_text(&text, &["game"]), path.parent());
    for p in problems {
        eprintln!("game: {}: {p}; left out", path.display());
    }
    mandrake_common::data_dirs::set_from_settings(game.myth2_data.clone(), game.tfl_data.clone());
    let display = table_text(&text, &["display"]);
    let (flags, problems) = preset_flags(&display);
    for p in problems {
        eprintln!("display: {}: {p}; left out", path.display());
    }
    let program = words.first().cloned().unwrap_or_else(|| "mandrake".into());
    let mut taken: Vec<OsString> = Vec::new();
    let mut from_conf = Vec::new();
    for (id, [flag, word]) in flags {
        if matches.value_source(id) == Some(clap::parser::ValueSource::CommandLine) {
            continue;
        }
        let one = [program.clone(), flag.clone().into(), word.clone().into()];
        match Args::try_parse_from(one) {
            Ok(_) => {
                taken.extend([flag.into(), word.into()]);
                from_conf.push(id);
            }
            Err(e) => eprintln!(
                "display: {}: `{id}` = {word:?}: {}; left out",
                path.display(),
                e.kind()
            ),
        }
    }
    if !taken.is_empty() {
        let conf = args.conf.take();
        let mut all = vec![program];
        all.extend(taken);
        all.extend(words.into_iter().skip(1));
        args = Args::try_parse_from(all)?;
        args.conf = conf;
        args.from_conf = from_conf;
    }
    let given = |id| matches.value_source(id) == Some(clap::parser::ValueSource::CommandLine);
    if !given("game") && game.game.is_some() {
        args.game = game.game;
    }
    if !given("archive") {
        args.archive = crate::args::default_archive();
    }
    Ok(args)
}

pub const OLD_LOOK_KEYS: [(&str, &str, bool); 5] = [
    ("tfl_fog", "fog", false),
    ("tfl_lighting", "lighting", false),
    ("tfl_detail_shadows", "detail_shadows", false),
    ("tfl_smoothing", "smoothing", true),
    ("tfl_hover_frames", "hover_frames", true),
];

pub type Moved = (&'static str, Option<String>);

pub fn with_new_look_keys(text: &str) -> Option<(String, Vec<Moved>)> {
    let doc = toml_edit::Document::parse(text.to_owned()).ok()?;
    if written_inline(doc.as_table(), &["display"]) {
        return None;
    }
    let display = doc.as_table().get("display")?.as_table()?;
    display.span()?;
    let line_of = |span: &std::ops::Range<usize>| {
        let start = text[..span.start].rfind('\n').map_or(0, |i| i + 1);
        let end = text[span.end..]
            .find('\n')
            .map_or(text.len(), |i| span.end + i + 1);
        start..end
    };
    let mut edits: Vec<(std::ops::Range<usize>, String)> = Vec::new();
    let mut to_tfl = String::new();
    let mut moved = Vec::new();
    for (old, new, tfl_only) in OLD_LOOK_KEYS {
        let Some(toml_edit::Item::Value(value)) = display.get(old) else {
            continue;
        };
        let span = value.span()?;
        let line = line_of(&span);
        let tfl = display.get("tfl").and_then(|t| t.as_table());
        let taken = if tfl_only {
            tfl.is_some_and(|t| t.contains_key(new))
        } else {
            display.contains_key(new)
        };
        if taken {
            edits.push((line, String::new()));
            moved.push((old, None));
            continue;
        }
        if tfl_only {
            let rest = text[span.start..line.end].trim_end_matches(['\r', '\n']);
            to_tfl.push_str(&format!("{new} = {rest}\n"));
            edits.push((line, String::new()));
            moved.push((old, Some(format!("[display.tfl] {new}"))));
        } else {
            let head = &text[line.start..span.start];
            let at = head.find(old)?;
            let from = line.start + at;
            let off = value.as_bool() == Some(false) || value.as_str() == Some("off");
            let linear = display.get("sampling").and_then(|v| v.as_str()) == Some("linear");
            if old == "tfl_fog" && off && linear {
                edits.push((from..span.end, format!("{new} = \"engine\"")));
                moved.push((old, Some(format!("[display] {new} = \"engine\""))));
            } else {
                edits.push((from..from + old.len(), new.to_owned()));
                moved.push((old, Some(format!("[display] {new}"))));
            }
        }
    }
    if moved.is_empty() {
        return None;
    }
    if !to_tfl.is_empty() {
        let tfl_header = display
            .get("tfl")
            .and_then(|t| t.as_table())
            .filter(|t| !t.is_implicit())
            .and_then(|t| t.span());
        match tfl_header {
            Some(span) => {
                let at = line_of(&span).end;
                let lead = if at == text.len() && !text.ends_with('\n') {
                    "\n"
                } else {
                    ""
                };
                edits.push((at..at, format!("{lead}{to_tfl}")));
            }
            None => {
                let found = sorted_headers(doc.as_table());
                let display_start = display.span()?.start;
                let at = found
                    .iter()
                    .find(|h| {
                        h.start > display_start && h.path.first().is_none_or(|k| k != "display")
                    })
                    .map_or(text.len(), |h| line_of(&(h.start..h.end)).start);
                let lead = if at == text.len() && !text.ends_with('\n') {
                    "\n"
                } else {
                    ""
                };
                let gap = if at == text.len() { "\n" } else { "" };
                let tail = if at == text.len() { "" } else { "\n" };
                edits.push((at..at, format!("{lead}{gap}[display.tfl]\n{to_tfl}{tail}")));
            }
        }
    }
    edits.sort_by_key(|(range, _)| std::cmp::Reverse((range.start, range.end)));
    let mut out = text.to_owned();
    for (range, what) in edits {
        out.replace_range(range, &what);
    }
    match toml_edit::Document::parse(out.clone()) {
        Ok(_) => Some((out, moved)),
        Err(e) => {
            eprintln!(
                "settings: the old keys can't be converted ({}); they are not read",
                e.message().trim()
            );
            None
        }
    }
}

pub fn old_keys_notice(path: &Path, moved: &[Moved], written: bool) -> String {
    let what: Vec<String> = moved
        .iter()
        .map(|(old, new)| match new {
            Some(new) => format!("{old} is now {new}"),
            None => format!("{old} is dropped (the new key is set)"),
        })
        .collect();
    let done = if written {
        "rewritten in place"
    } else {
        "read so; the file is rewritten the next time the client starts in a window with it"
    };
    format!(
        "settings: {}: old keys: {} ([display] fog, lighting and detail_shadows are both games'; \
         [display.tfl] smoothing and hover_frames TFL's); {done}",
        path.display(),
        what.join(", ")
    )
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct GameTable {
    pub game: Option<mandrake_tag::tfl::Game>,
    pub myth2_data: Option<PathBuf>,
    pub tfl_data: Option<PathBuf>,
}

pub fn game_folder(value: &str, folder: Option<&Path>, home: Option<&Path>) -> PathBuf {
    if let (Some(rest), Some(home)) = (value.strip_prefix("~/"), home) {
        return home.join(rest);
    }
    let path = PathBuf::from(value);
    match folder {
        Some(folder) if path.is_relative() => folder.join(path),
        _ => path,
    }
}

pub fn parse_game(text: &str, folder: Option<&Path>) -> (GameTable, Vec<String>) {
    let mut table = GameTable::default();
    let mut problems = Vec::new();
    let doc = match toml_edit::Document::parse(text.to_owned()) {
        Ok(doc) => doc,
        Err(e) => {
            problems.push(format!("not TOML: {}", e.message().trim()));
            return (table, problems);
        }
    };
    let home = crate::user_dirs::pictures_home_from(|name| std::env::var_os(name), cfg!(windows))
        .map(PathBuf::from);
    let dir = |value: &str| game_folder(value, folder, home.as_deref());
    for (key, item) in doc.as_table().iter() {
        let word = item.as_str().filter(|w| !w.is_empty());
        match (key, word) {
            ("game", Some(w)) => match crate::args::parse_game(w) {
                Ok(game) => table.game = Some(game),
                Err(e) => problems.push(format!("`game`: {e}")),
            },
            ("myth2_data", Some(w)) => table.myth2_data = Some(dir(w)),
            ("tfl_data", Some(w)) => table.tfl_data = Some(dir(w)),
            ("game" | "myth2_data" | "tfl_data", None) => {
                problems.push(format!("`{key}` must be a non-empty string"))
            }
            _ => problems.push(format!(
                "unknown key `{key}` (expected game, myth2_data or tfl_data)"
            )),
        }
    }
    (table, problems)
}

const OLD_FILES: [(&str, TablePath); 7] = [
    ("interface.toml", &["interface"]),
    ("controls.toml", &["controls"]),
    ("fog.toml", &["fog"]),
    ("lighting.toml", &["lighting"]),
    ("water.toml", &["water"]),
    ("settings.toml", &["display"]),
    ("tfl/controls.toml", &["tfl", "controls"]),
];

pub fn nested(text: &str, table: TablePath) -> Option<String> {
    let doc = toml_edit::Document::parse(text.to_owned()).ok()?;
    let found = sorted_headers(doc.as_table());
    let owned: Vec<String> = table.iter().map(|k| (*k).to_owned()).collect();
    let mut out = format!("{}\n", header_line(&owned, false));
    let mut offset = 0;
    for (line, header, own) in lines_by_header(text, &found) {
        let end = offset + line.len();
        match header {
            Some(h) if own => {
                let path = if table == ["display"] && h.path.first().is_some_and(|k| k == "net") {
                    h.path.clone()
                } else {
                    owned.iter().chain(&h.path).cloned().collect()
                };
                let after = text.get(h.end.min(end)..end).unwrap_or("");
                out.push_str(&header_line(&path, h.array));
                out.push_str(after.trim_end_matches(['\r', '\n']));
            }
            _ => out.push_str(line.trim_end_matches(['\r', '\n'])),
        }
        out.push('\n');
        offset = end;
    }
    toml_edit::Document::parse(out.clone()).ok()?;
    Some(out)
}

pub fn migrated(folder: &Path) -> Option<(String, Vec<&'static str>)> {
    let mut out = String::from(
        "# Myth II source port settings, built once from the old files of this folder,\n\
         # which are no longer read. mandrake --write-default-conf FILE writes every setting with\n\
         # its shipped value and comments.\n",
    );
    let mut taken = Vec::new();
    for (name, table) in OLD_FILES {
        let path = folder.join(name);
        let Ok(text) = crate::config_store::read_to_string(&path) else {
            continue;
        };
        match nested(&text, table) {
            Some(part) => {
                out.push_str(&format!("\n# From {name}.\n"));
                out.push_str(&part);
                taken.push(name);
            }
            None => eprintln!(
                "settings: {}: not TOML; left out of {DEFAULT_NAME}",
                path.display()
            ),
        }
    }
    if taken.is_empty() {
        return None;
    }
    if toml_edit::Document::parse(out.clone()).is_err() {
        eprintln!("settings: the old files don't make one {DEFAULT_NAME}; not built");
        return None;
    }
    Some((out, taken))
}

pub fn migrate(conf: Option<&Path>) {
    if conf.is_some() {
        return;
    }
    let (Some(path), Some(folder)) = (default_path(), crate::user_dirs::config_dir()) else {
        return;
    };
    if crate::config_store::is_file(&path) {
        return;
    }
    let Some((text, taken)) = migrated(&folder) else {
        return;
    };
    match crate::config_store::write(&path, &text) {
        Ok(()) => println!(
            "settings: built {} from {} (they are no longer read; they stay where they are)",
            path.display(),
            taken.join(", ")
        ),
        Err(e) => eprintln!("settings: cannot write {}: {e}", path.display()),
    }
}

pub fn write_defaults(path: &Path) -> std::io::Result<()> {
    crate::config_store::write(path, DEFAULT_CONF_TOML)
}

pub(crate) mod layered;

#[cfg(test)]
mod tests;
