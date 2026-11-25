use std::collections::{BTreeMap, HashMap};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

const PLUGIN_MISS_RESCAN: Duration = Duration::from_secs(2);

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SiteConfig {
    pub web_root: Option<PathBuf>,
    pub game_dir: Option<PathBuf>,
    pub plugins_dir: Option<PathBuf>,
    pub downloads_dir: Option<PathBuf>,
    pub offered_plugins: Option<Vec<String>>,
    pub tfl_dir: Option<PathBuf>,
}

const TFL_PRIVATE_GROUPS: [&str; 1] = ["RECO"];

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Reply {
    File {
        head: Vec<u8>,
        path: PathBuf,
    },
    Bytes {
        head: Vec<u8>,
        body: Vec<u8>,
    },
    Part {
        head: Vec<u8>,
        path: PathBuf,
        start: u64,
        len: u64,
    },
}

impl Reply {
    pub fn status(&self) -> &str {
        let head = match self {
            Reply::File { head, .. } | Reply::Bytes { head, .. } | Reply::Part { head, .. } => head,
        };
        std::str::from_utf8(&head[9..12.min(head.len())]).unwrap_or("?")
    }
}

pub fn content_type(path: &Path) -> &'static str {
    match path.extension().and_then(|e| e.to_str()) {
        Some("html") => "text/html; charset=utf-8",
        Some("js" | "mjs") => "text/javascript; charset=utf-8",
        Some("wasm") => "application/wasm",
        Some("json") => "application/json",
        Some("css") => "text/css; charset=utf-8",
        Some("png") => "image/png",
        Some("ico") => "image/x-icon",
        Some("txt" | "md") => "text/plain; charset=utf-8",
        _ => "application/octet-stream",
    }
}

fn percent_decode(s: &str) -> Option<String> {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' {
            let pair = b.get(i + 1..i + 3)?;
            if !pair.iter().all(u8::is_ascii_hexdigit) {
                return None;
            }
            let hex = std::str::from_utf8(pair).ok()?;
            out.push(u8::from_str_radix(hex, 16).ok()?);
            i += 3;
        } else {
            out.push(b[i]);
            i += 1;
        }
    }
    String::from_utf8(out).ok()
}

fn segment(s: &str) -> Option<String> {
    let s = percent_decode(s)?;
    let ok = !s.is_empty()
        && !s.starts_with('.')
        && !s.contains(['/', '\\', '\0'])
        && !s.chars().any(char::is_control);
    ok.then_some(s)
}

fn version(meta: &fs::Metadata) -> String {
    let t = meta
        .modified()
        .ok()
        .and_then(|m| m.duration_since(UNIX_EPOCH).ok())
        .map_or(0, |d| d.as_secs());
    format!("{t}-{}", meta.len())
}

fn head(status: &str, kind: &str, len: u64, cache: &str, etag: Option<&str>) -> Vec<u8> {
    head_with(status, kind, len, cache, etag, "")
}

fn head_with(
    status: &str,
    kind: &str,
    len: u64,
    cache: &str,
    etag: Option<&str>,
    extra: &str,
) -> Vec<u8> {
    let etag = etag
        .map(|e| format!("ETag: \"{e}\"\r\n"))
        .unwrap_or_default();
    format!(
        "HTTP/1.1 {status}\r\nContent-Type: {kind}\r\nContent-Length: {len}\r\nCache-Control: {cache}\r\n{etag}{extra}X-Content-Type-Options: nosniff\r\nConnection: close\r\n\r\n"
    )
    .into_bytes()
}

const CODINGS: [(&str, &str); 2] = [("br", "br"), ("gz", "gzip")];

fn accepts(accept_encoding: &str, coding: &str) -> bool {
    let mut named = None;
    let mut star = None;
    for item in accept_encoding.split(',') {
        let mut params = item.split(';');
        let name = params.next().unwrap_or("").trim();
        let q = params
            .filter_map(|p| p.trim().strip_prefix("q=").or(p.trim().strip_prefix("Q=")))
            .find_map(|q| q.trim().parse::<f32>().ok())
            .unwrap_or(1.0);
        if name.eq_ignore_ascii_case(coding) {
            named = Some(q > 0.0);
        } else if name == "*" {
            star = Some(q > 0.0);
        }
    }
    named.or(star).unwrap_or(false)
}

fn compressed(
    path: &Path,
    meta: &fs::Metadata,
    accept_encoding: Option<&str>,
) -> (Option<(PathBuf, &'static str, fs::Metadata)>, bool) {
    let mut any = false;
    for (suffix, coding) in CODINGS {
        let mut name = path.as_os_str().to_owned();
        name.push(".");
        name.push(suffix);
        let copy = PathBuf::from(name);
        let Ok(m) = fs::metadata(&copy) else {
            continue;
        };
        if !m.is_file() || m.modified().ok() < meta.modified().ok() {
            continue;
        }
        any = true;
        if accept_encoding.is_some_and(|a| accepts(a, coding)) {
            return (Some((copy, coding, m)), true);
        }
    }
    (None, any)
}

fn not_found() -> Reply {
    Reply::Bytes {
        head: head("404 Not Found", "text/plain", 10, "no-store", None),
        body: b"not found\n".to_vec(),
    }
}

fn file(
    dir: &Path,
    name: &str,
    cache: &str,
    if_none_match: Option<&str>,
    accept_encoding: Option<&str>,
    allow_compressed: bool,
) -> Reply {
    let path = dir.join(name);
    let Ok(meta) = fs::metadata(&path) else {
        return not_found();
    };
    if !meta.is_file() {
        return not_found();
    }
    let kind = content_type(&path);
    let (copy, varies) = if allow_compressed {
        compressed(&path, &meta, accept_encoding)
    } else {
        (None, false)
    };
    let vary = if varies {
        "Vary: Accept-Encoding\r\n"
    } else {
        ""
    };
    let (send, v, extra) = match copy {
        Some((copy, coding, m)) => (
            copy,
            format!("{}-{coding}", version(&m)),
            format!("Content-Encoding: {coding}\r\n{vary}"),
        ),
        None => (path, version(&meta), vary.to_string()),
    };
    if if_none_match.is_some_and(|m| m.split(',').any(|t| t.trim().trim_matches('"') == v)) {
        return Reply::Bytes {
            head: format!(
                "HTTP/1.1 304 Not Modified\r\nCache-Control: {cache}\r\nETag: \"{v}\"\r\n{vary}Connection: close\r\n\r\n"
            )
            .into_bytes(),
            body: Vec::new(),
        };
    }
    let len = fs::metadata(&send).map_or(0, |m| m.len());
    Reply::File {
        head: head_with("200 OK", kind, len, cache, Some(&v), &extra),
        path: send,
    }
}

fn files_of(dir: &Path) -> io::Result<Vec<(String, fs::Metadata)>> {
    let mut out = Vec::new();
    for e in fs::read_dir(dir)? {
        let Ok(e) = e else {
            continue;
        };
        let Ok(name) = e.file_name().into_string() else {
            continue;
        };
        if name.starts_with('.') {
            continue;
        }
        let Ok(meta) = fs::metadata(e.path()) else {
            continue;
        };
        if meta.is_file() {
            out.push((name, meta));
        }
    }
    out.sort_by(|a, b| a.0.cmp(&b.0));
    Ok(out)
}

fn json_string(s: &str) -> String {
    let mut out = String::from("\"");
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

fn push_list_rows(rows: &mut Vec<String>, prefix: &str, dir: &Path) {
    for (name, meta) in files_of(dir).unwrap_or_default() {
        rows.push(format!(
            "{{\"path\":{},\"size\":{},\"version\":{}}}",
            json_string(&format!("{prefix}/{name}")),
            meta.len(),
            json_string(&version(&meta))
        ));
    }
}

pub fn game_list(config: &SiteConfig) -> String {
    let mut rows = Vec::new();
    if let Some(g) = &config.game_dir {
        push_list_rows(&mut rows, "tags", &g.join("tags"));
    }
    if let Some(p) = &config.plugins_dir {
        push_list_rows(&mut rows, "plugins", p);
    }
    format!("[{}]\n", rows.join(","))
}

pub fn tfl_list(config: &SiteConfig) -> String {
    let mut rows = Vec::new();
    if let Some(t) = &config.tfl_dir {
        let tags = t.join("tags");
        push_list_rows(&mut rows, "tags", &tags);
        let mut groups: Vec<String> = fs::read_dir(tags.join("Local"))
            .map(|d| {
                d.filter_map(|e| e.ok())
                    .filter(|e| e.path().is_dir())
                    .filter_map(|e| e.file_name().into_string().ok())
                    .filter(|g| !g.starts_with('.') && !TFL_PRIVATE_GROUPS.contains(&g.as_str()))
                    .collect()
            })
            .unwrap_or_default();
        groups.sort();
        for g in groups {
            push_list_rows(
                &mut rows,
                &format!("tags/Local/{g}"),
                &tags.join("Local").join(&g),
            );
        }
    }
    format!("[{}]\n", rows.join(","))
}

pub fn plugin_checksum(path: &Path) -> Option<u32> {
    use std::io::Read;
    let mut b = [0u8; mandrake_tag::header::SBMonoHeader::SIZE];
    fs::File::open(path).ok()?.read_exact(&mut b).ok()?;
    let h = mandrake_tag::header::SBMonoHeader::read(&mut &b[..]).ok()?;
    let signed = h.signature == mandrake_tag::header::SBMonoHeader::SIG_DNG2
        || h.signature == mandrake_tag::header::SBMonoHeader::SIG_MTH2;
    (signed && h.r#type == mandrake_tag::tag_files::TYPE_PLUGIN)
        .then(|| u32::from_be_bytes(h.checksum))
}

#[derive(Clone)]
struct CachedFile {
    modified: Option<SystemTime>,
    size: u64,
    checksum: Option<u32>,
}

struct DirCache {
    dir_modified: Option<SystemTime>,
    last_scan: Instant,
    files: BTreeMap<String, CachedFile>,
    by_checksum: HashMap<u32, Vec<String>>,
}

impl DirCache {
    fn find(&self, offered: Option<&[String]>, checksum: u32) -> Option<(&str, &CachedFile)> {
        let names = self.by_checksum.get(&checksum)?;
        let name = names
            .iter()
            .find(|n| offered.is_none_or(|o| o.iter().any(|x| x == *n)))?;
        self.files.get(name).map(|f| (name.as_str(), f))
    }

    fn scan(dir: &Path, prev: Option<&DirCache>) -> Option<DirCache> {
        let dir_modified = fs::metadata(dir).ok().and_then(|m| m.modified().ok());
        let listed = files_of(dir).ok()?;
        let mut files = BTreeMap::new();
        let mut by_checksum: HashMap<u32, Vec<String>> = HashMap::new();
        for (name, meta) in listed {
            let modified = meta.modified().ok();
            let size = meta.len();
            let checksum = prev
                .and_then(|p| p.files.get(&name))
                .filter(|c| c.modified == modified && c.size == size)
                .map_or_else(|| plugin_checksum(&dir.join(&name)), |c| c.checksum);
            if let Some(c) = checksum {
                by_checksum.entry(c).or_default().push(name.clone());
            }
            files.insert(
                name,
                CachedFile {
                    modified,
                    size,
                    checksum,
                },
            );
        }
        Some(DirCache {
            dir_modified,
            last_scan: Instant::now(),
            files,
            by_checksum,
        })
    }
}

fn plugin_cache() -> &'static Mutex<HashMap<PathBuf, DirCache>> {
    static CACHE: OnceLock<Mutex<HashMap<PathBuf, DirCache>>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

pub fn offered_plugin(config: &SiteConfig, checksum: u32) -> Option<PathBuf> {
    let dir = config.downloads_dir.as_ref()?;
    let offered = config.offered_plugins.as_deref();
    let hit = {
        let cache = plugin_cache().lock().unwrap();
        cache.get(dir).map(|entry| {
            let found = entry
                .find(offered, checksum)
                .map(|(name, file)| (dir.join(name), file.modified, file.size));
            (found, entry.dir_modified, entry.last_scan)
        })
    };
    if let Some((Some((path, modified, size)), _, _)) = &hit
        && let Ok(meta) = fs::metadata(path)
        && meta.is_file()
        && meta.modified().ok() == *modified
        && meta.len() == *size
    {
        return Some(path.clone());
    }
    if let Some((None, cached_mtime, last_scan)) = hit {
        let now_mtime = fs::metadata(dir).ok().and_then(|m| m.modified().ok());
        if now_mtime == cached_mtime && last_scan.elapsed() < PLUGIN_MISS_RESCAN {
            return None;
        }
    }
    let mut cache = plugin_cache().lock().unwrap();
    let scanned = DirCache::scan(dir, cache.get(dir));
    match scanned {
        Some(entry) => {
            let found = entry
                .find(offered, checksum)
                .map(|(name, _)| dir.join(name));
            cache.insert(dir.clone(), entry);
            found
        }
        None => {
            cache.remove(dir);
            None
        }
    }
}

fn byte_range(range: Option<&str>, size: u64) -> Result<Option<(u64, u64)>, ()> {
    let Some(spec) = range.and_then(|r| r.trim().strip_prefix("bytes=")) else {
        return Ok(None);
    };
    let Some((from, to)) = spec.split_once('-') else {
        return Ok(None);
    };
    if spec.contains(',') || from.trim().is_empty() {
        return Ok(None);
    }
    let Ok(start) = from.trim().parse::<u64>() else {
        return Ok(None);
    };
    let end = match to.trim() {
        "" => size,
        t => match t.parse::<u64>() {
            Ok(e) => e.saturating_add(1).min(size),
            Err(_) => return Ok(None),
        },
    };
    if start >= size || end <= start {
        return Err(());
    }
    Ok(Some((start, end - start)))
}

fn plugin_download(config: &SiteConfig, name: &str, range: Option<&str>) -> Reply {
    let checksum = (name.len() == 8)
        .then(|| u32::from_str_radix(name, 16).ok())
        .flatten();
    let Some(path) = checksum.and_then(|c| offered_plugin(config, c)) else {
        return not_found();
    };
    let Ok(meta) = fs::metadata(&path) else {
        return not_found();
    };
    let size = meta.len();
    let kind = "application/octet-stream";
    let v = version(&meta);
    match byte_range(range, size) {
        Ok(None) => Reply::Part {
            head: head_with(
                "200 OK",
                kind,
                size,
                "no-store",
                Some(&v),
                "Accept-Ranges: bytes\r\n",
            ),
            path,
            start: 0,
            len: size,
        },
        Ok(Some((start, len))) => Reply::Part {
            head: head_with(
                "206 Partial Content",
                kind,
                len,
                "no-store",
                Some(&v),
                &format!(
                    "Accept-Ranges: bytes\r\nContent-Range: bytes {start}-{}/{size}\r\n",
                    start + len - 1
                ),
            ),
            path,
            start,
            len,
        },
        Err(()) => Reply::Bytes {
            head: head_with(
                "416 Range Not Satisfiable",
                "text/plain",
                0,
                "no-store",
                None,
                &format!("Content-Range: bytes */{size}\r\n"),
            ),
            body: Vec::new(),
        },
    }
}

pub fn answer_range(
    config: &SiteConfig,
    path: &str,
    if_none_match: Option<&str>,
    accept_encoding: Option<&str>,
    range: Option<&str>,
) -> Option<Reply> {
    let page_file =
        |dir: &Path, name: &str| file(dir, name, "no-cache", if_none_match, accept_encoding, true);
    let game_file =
        |dir: &Path, name: &str| file(dir, name, "no-store", if_none_match, accept_encoding, false);
    let path = path.split(['?', '#']).next().unwrap_or("");
    let parts: Vec<&str> = path.strip_prefix('/')?.split('/').collect();
    const GAME: &str = "no-store";
    match parts.as_slice() {
        ["plugins", name] if config.downloads_dir.is_some() => {
            Some(plugin_download(config, name, range))
        }
        ["game", "list"] if config.game_dir.is_some() || config.plugins_dir.is_some() => {
            let body = game_list(config).into_bytes();
            Some(Reply::Bytes {
                head: head("200 OK", "application/json", body.len() as u64, GAME, None),
                body,
            })
        }
        ["game", "tfl", "list"] if config.tfl_dir.is_some() => {
            let body = tfl_list(config).into_bytes();
            Some(Reply::Bytes {
                head: head("200 OK", "application/json", body.len() as u64, GAME, None),
                body,
            })
        }
        ["game", "tfl", "tags", name] if config.tfl_dir.is_some() => {
            let dir = config.tfl_dir.as_ref()?.join("tags");
            Some(segment(name).map_or_else(not_found, |n| game_file(&dir, &n)))
        }
        ["game", "tfl", "tags", "Local", group, name] if config.tfl_dir.is_some() => {
            let dir = config.tfl_dir.as_ref()?.join("tags").join("Local");
            let file_of = |g: String, n: String| {
                if TFL_PRIVATE_GROUPS.contains(&g.as_str()) {
                    not_found()
                } else {
                    game_file(&dir.join(g), &n)
                }
            };
            Some(match (segment(group), segment(name)) {
                (Some(g), Some(n)) => file_of(g, n),
                _ => not_found(),
            })
        }
        ["game", "tfl", ..] if config.tfl_dir.is_some() => Some(not_found()),
        ["game", "tags", name] => {
            let dir = config.game_dir.as_ref()?.join("tags");
            Some(segment(name).map_or_else(not_found, |n| game_file(&dir, &n)))
        }
        ["game", "plugins", name] => {
            let dir = config.plugins_dir.as_ref()?;
            Some(segment(name).map_or_else(not_found, |n| game_file(dir, &n)))
        }
        ["game", ..] if config.game_dir.is_some() || config.plugins_dir.is_some() => {
            Some(not_found())
        }
        [""] | ["index.html"] => Some(page_file(config.web_root.as_ref()?, "index.html")),
        [sub @ ("pkg" | "js"), name] => {
            let dir = config.web_root.as_ref()?.join(sub);
            Some(segment(name).map_or_else(not_found, |n| page_file(&dir, &n)))
        }
        [name] => {
            let root = config.web_root.as_ref()?;
            let n = segment(name)?;
            root.join(&n).is_file().then(|| page_file(root, &n))
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn answer(
        config: &SiteConfig,
        path: &str,
        if_none_match: Option<&str>,
        accept_encoding: Option<&str>,
    ) -> Option<Reply> {
        answer_range(config, path, if_none_match, accept_encoding, None)
    }

    fn site(tag: &str) -> (SiteConfig, PathBuf) {
        let base = std::env::temp_dir().join(format!("m2-site-{}-{tag}", std::process::id()));
        let _ = fs::remove_dir_all(&base);
        for d in ["web/pkg", "web/js", "game/tags", "plugins"] {
            fs::create_dir_all(base.join(d)).unwrap();
        }
        fs::write(base.join("web/index.html"), "<!doctype html>").unwrap();
        fs::write(base.join("web/client.js"), "1").unwrap();
        fs::write(base.join("web/pkg/mandrake_web_bg.wasm"), "\0asm").unwrap();
        fs::write(base.join("game/tags/small install"), "tags").unwrap();
        fs::write(base.join("game/secret.txt"), "no").unwrap();
        fs::write(base.join("plugins/Patch 1.2"), "p").unwrap();
        fs::write(base.join("plugins/.hidden"), "h").unwrap();
        fs::create_dir_all(base.join("plugins/folder")).unwrap();
        (
            SiteConfig {
                web_root: Some(base.join("web")),
                game_dir: Some(base.join("game")),
                plugins_dir: Some(base.join("plugins")),
                ..SiteConfig::default()
            },
            base,
        )
    }

    fn text(r: &Reply) -> String {
        match r {
            Reply::File { head, .. } | Reply::Bytes { head, .. } | Reply::Part { head, .. } => {
                String::from_utf8_lossy(head).into_owned()
            }
        }
    }

    #[test]
    fn the_site_serves_the_client_and_the_mounted_game_only() {
        let (c, base) = site("plain");
        let page = answer(&c, "/", None, None).unwrap();
        assert_eq!(page.status(), "200");
        assert!(text(&page).contains("Content-Type: text/html"));
        assert!(text(&page).contains("Cache-Control: no-cache"));
        let wasm = answer(&c, "/pkg/mandrake_web_bg.wasm", None, None).unwrap();
        assert!(text(&wasm).contains("Content-Type: application/wasm"));
        let js = answer(&c, "/client.js?x=1", None, None).unwrap();
        assert!(text(&js).contains("text/javascript"));
        let tags = answer(&c, "/game/tags/small%20install", None, None).unwrap();
        assert_eq!(tags.status(), "200");
        assert!(text(&tags).contains("Cache-Control: no-store"));
        assert!(matches!(&tags, Reply::File { path, .. } if path.ends_with("tags/small install")));
        let etag = text(&tags)
            .lines()
            .find_map(|l| l.strip_prefix("ETag: ").map(str::to_string))
            .unwrap();
        let again = answer(&c, "/game/tags/small%20install", Some(&etag), None).unwrap();
        assert_eq!(again.status(), "304");
        for bad in [
            "/game/tags/..%2Fsecret.txt",
            "/game/tags/../secret.txt",
            "/game/secret.txt",
            "/game/plugins/.hidden",
            "/game/plugins/folder",
            "/game/plugins/%2e%2e",
            "/game/tags/%zz",
            "/pkg/..",
        ] {
            assert_eq!(
                answer(&c, bad, None, None)
                    .map(|r| r.status().to_string())
                    .as_deref(),
                Some("404"),
                "{bad}"
            );
        }
        assert_eq!(answer(&c, "/myth2-ws", None, None), None);
        assert_eq!(answer(&c, "/healthz", None, None), None);
        assert_eq!(answer(&c, "/a/b/c", None, None), None);
        let list = game_list(&c);
        assert!(
            list.starts_with("[{\"path\":\"tags/small install\",\"size\":4,\"version\":\""),
            "{list}"
        );
        assert!(
            list.contains("{\"path\":\"plugins/Patch 1.2\",\"size\":1,"),
            "{list}"
        );
        assert!(!list.contains("hidden") && !list.contains("folder") && !list.contains("secret"));
        assert_eq!(answer(&SiteConfig::default(), "/", None, None), None);
        assert_eq!(
            answer(&SiteConfig::default(), "/game/list", None, None),
            None
        );
        fs::remove_dir_all(base).unwrap();
    }

    #[test]
    fn the_site_serves_tfl_tags_and_local_groups_but_not_its_films() {
        let (mut c, base) = site("tfl");
        for no in ["/game/tfl/list", "/game/tfl/tags/TAGS.GOR"] {
            assert_eq!(answer(&c, no, None, None).unwrap().status(), "404", "{no}");
        }
        let tfl = base.join("tfl");
        for d in ["tags/Local/stli", "tags/Local/RECO"] {
            fs::create_dir_all(tfl.join(d)).unwrap();
        }
        fs::write(tfl.join("tags/TAGS.GOR"), "gor").unwrap();
        fs::write(tfl.join("tags/Local/stli/inns"), "stli").unwrap();
        fs::write(tfl.join("tags/Local/RECO/LAST RECORDING"), "film").unwrap();
        fs::write(tfl.join("secret.txt"), "no").unwrap();
        c.tfl_dir = Some(tfl);
        let list = answer(&c, "/game/tfl/list", None, None).unwrap();
        let Reply::Bytes { body, .. } = &list else {
            panic!("{list:?}")
        };
        let body = String::from_utf8_lossy(body);
        assert!(body.contains("\"tags/TAGS.GOR\""), "{body}");
        assert!(body.contains("\"tags/Local/stli/inns\""), "{body}");
        assert!(!body.contains("RECO"), "{body}");
        for ok in ["/game/tfl/tags/TAGS.GOR", "/game/tfl/tags/Local/stli/inns"] {
            assert_eq!(answer(&c, ok, None, None).unwrap().status(), "200", "{ok}");
        }
        for no in [
            "/game/tfl/tags/Local/RECO/LAST%20RECORDING",
            "/game/tfl/tags/..%2Fsecret.txt",
            "/game/tfl/tags/Local/..%2F..%2F/secret.txt",
            "/game/tfl/secret.txt",
            "/game/tfl/tags/Local",
        ] {
            assert_eq!(answer(&c, no, None, None).unwrap().status(), "404", "{no}");
        }
        let Reply::Bytes { body, .. } = answer(&c, "/game/list", None, None).unwrap() else {
            panic!()
        };
        assert!(!String::from_utf8_lossy(&body).contains("GOR"));
        let _ = fs::remove_dir_all(&base);
    }

    fn header(r: &Reply, name: &str) -> Option<String> {
        text(r)
            .lines()
            .find_map(|l| l.strip_prefix(name).map(str::to_string))
    }

    #[test]
    fn a_pre_compressed_copy_goes_to_a_client_that_accepts_it_and_the_plain_file_otherwise() {
        let (c, base) = site("coded");
        let pkg = base.join("web/pkg");
        fs::write(pkg.join("mandrake_web_bg.wasm.gz"), "gz").unwrap();
        fs::write(pkg.join("mandrake_web_bg.wasm.br"), "b").unwrap();
        let wasm = "/pkg/mandrake_web_bg.wasm";
        let sent = |r: &Reply| match r {
            Reply::File { path, .. } => path.file_name().unwrap().to_string_lossy().into_owned(),
            _ => panic!("not a file"),
        };
        let br = answer(&c, wasm, None, Some("gzip, deflate, br, zstd")).unwrap();
        assert_eq!(sent(&br), "mandrake_web_bg.wasm.br");
        assert_eq!(header(&br, "Content-Encoding: ").as_deref(), Some("br"));
        assert_eq!(
            header(&br, "Content-Type: ").as_deref(),
            Some("application/wasm")
        );
        assert_eq!(header(&br, "Content-Length: ").as_deref(), Some("1"));
        assert_eq!(header(&br, "Vary: ").as_deref(), Some("Accept-Encoding"));
        let gz = answer(&c, wasm, None, Some("gzip;q=1.0, br;q=0")).unwrap();
        assert_eq!(sent(&gz), "mandrake_web_bg.wasm.gz");
        assert_eq!(header(&gz, "Content-Encoding: ").as_deref(), Some("gzip"));
        assert_eq!(header(&gz, "Content-Length: ").as_deref(), Some("2"));
        assert_eq!(
            sent(&answer(&c, wasm, None, Some("*")).unwrap()),
            "mandrake_web_bg.wasm.br"
        );
        for ae in [
            None,
            Some("identity"),
            Some("deflate"),
            Some("br;q=0, gzip;q=0"),
        ] {
            let plain = answer(&c, wasm, None, ae).unwrap();
            assert_eq!(sent(&plain), "mandrake_web_bg.wasm", "{ae:?}");
            assert_eq!(header(&plain, "Content-Encoding: "), None);
            assert_eq!(header(&plain, "Content-Length: ").as_deref(), Some("4"));
            assert_eq!(header(&plain, "Vary: ").as_deref(), Some("Accept-Encoding"));
        }
        let etag = header(&br, "ETag: ").unwrap();
        assert!(etag.ends_with("-br\""), "{etag}");
        assert_eq!(
            answer(&c, wasm, Some(&etag), Some("br")).unwrap().status(),
            "304"
        );
        assert_eq!(
            answer(&c, wasm, Some(&etag), Some("gzip"))
                .unwrap()
                .status(),
            "200"
        );
        assert_eq!(answer(&c, wasm, Some(&etag), None).unwrap().status(), "200");
        let js = answer(&c, "/client.js", None, Some("br, gzip")).unwrap();
        assert_eq!(header(&js, "Content-Encoding: "), None);
        assert_eq!(header(&js, "Vary: "), None);
        let old = std::time::SystemTime::now() - std::time::Duration::from_secs(3600);
        for copy in ["mandrake_web_bg.wasm.br", "mandrake_web_bg.wasm.gz"] {
            fs::File::options()
                .write(true)
                .open(pkg.join(copy))
                .unwrap()
                .set_modified(old)
                .unwrap();
        }
        let stale = answer(&c, wasm, None, Some("br, gzip")).unwrap();
        assert_eq!(sent(&stale), "mandrake_web_bg.wasm");
        assert_eq!(header(&stale, "Vary: "), None);
        fs::remove_dir_all(base).unwrap();
    }

    fn plugin(path: &Path, checksum: u32, body: &[u8]) {
        let mut b = vec![0u8; 128];
        b[0..2].copy_from_slice(&1i16.to_be_bytes());
        b[0x68..0x6c].copy_from_slice(&checksum.to_be_bytes());
        b[124..128].copy_from_slice(b"mth2");
        b.extend_from_slice(body);
        fs::write(path, b).unwrap();
    }

    #[test]
    fn plugins_download_by_checksum_with_ranges() {
        let (mut c, base) = site("downloads");
        let dir = base.join("plugins");
        plugin(&dir.join("Wanted"), 0x1234_abcd, b"0123456789");
        plugin(&dir.join("Other"), 0x0bad_f00d, b"x");
        c.downloads_dir = Some(dir.clone());
        let whole = answer(&c, "/plugins/1234abcd", None, None).unwrap();
        assert_eq!(whole.status(), "200");
        assert!(
            matches!(&whole, Reply::Part { path, start: 0, len: 138, .. } if path.ends_with("Wanted"))
        );
        assert_eq!(header(&whole, "Accept-Ranges: ").as_deref(), Some("bytes"));
        assert_eq!(header(&whole, "Content-Length: ").as_deref(), Some("138"));
        let r =
            |range: &str| answer_range(&c, "/plugins/1234ABCD", None, None, Some(range)).unwrap();
        let part = r("bytes=100-");
        assert_eq!(part.status(), "206");
        assert!(matches!(
            part,
            Reply::Part {
                start: 100,
                len: 38,
                ..
            }
        ));
        assert_eq!(
            header(&part, "Content-Range: ").as_deref(),
            Some("bytes 100-137/138")
        );
        assert!(matches!(
            r("bytes=0-9"),
            Reply::Part {
                start: 0,
                len: 10,
                ..
            }
        ));
        assert_eq!(r("bytes=138-").status(), "416");
        assert_eq!(
            header(&r("bytes=500-"), "Content-Range: ").as_deref(),
            Some("bytes */138")
        );
        assert_eq!(r("bytes=-5").status(), "200");
        for bad in [
            "/plugins/00000000",
            "/plugins/1234abc",
            "/plugins/zzzzzzzz",
            "/plugins/..",
            "/plugins/a/b",
        ] {
            assert_ne!(
                answer(&c, bad, None, None)
                    .map(|r| r.status().to_string())
                    .as_deref(),
                Some("200"),
                "{bad}"
            );
        }
        assert_eq!(
            answer(&c, "/plugins/00000000", None, None)
                .unwrap()
                .status(),
            "404"
        );
        c.offered_plugins = Some(vec!["Other".into()]);
        assert_eq!(
            answer(&c, "/plugins/1234abcd", None, None)
                .unwrap()
                .status(),
            "404"
        );
        assert_eq!(
            answer(&c, "/plugins/0badf00d", None, None)
                .unwrap()
                .status(),
            "200"
        );
        c.downloads_dir = None;
        assert_eq!(answer(&c, "/plugins/0badf00d", None, None), None);
        fs::remove_dir_all(base).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn dangling_symlink_does_not_empty_lists_or_break_plugins() {
        let (mut c, base) = site("dangling");
        std::os::unix::fs::symlink(base.join("missing"), base.join("game/tags/broken")).unwrap();
        std::os::unix::fs::symlink(base.join("missing"), base.join("plugins/broken")).unwrap();
        plugin(&base.join("plugins/Good"), 0xfeed_beef, b"ok");
        c.downloads_dir = Some(base.join("plugins"));
        let list = game_list(&c);
        assert!(list.contains("\"tags/small install\""), "{list}");
        assert!(list.contains("\"plugins/Patch 1.2\""), "{list}");
        assert!(!list.contains("broken"), "{list}");
        let dl = answer(&c, "/plugins/feedbeef", None, None).unwrap();
        assert_eq!(dl.status(), "200");
        fs::remove_dir_all(base).unwrap();
    }

    #[test]
    fn game_and_plugin_files_never_use_precompressed_siblings() {
        let (mut c, base) = site("no-game-gz");
        let tfl = base.join("tfl");
        fs::create_dir_all(tfl.join("tags")).unwrap();
        fs::write(tfl.join("tags/TAGS.GOR"), "plain").unwrap();
        fs::write(tfl.join("tags/TAGS.GOR.gz"), "gz").unwrap();
        fs::write(base.join("game/tags/small install.br"), "br").unwrap();
        fs::write(base.join("plugins/Patch 1.2.gz"), "gz").unwrap();
        c.tfl_dir = Some(tfl);
        for (url, expected_name) in [
            ("/game/tags/small%20install", "small install"),
            ("/game/plugins/Patch%201.2", "Patch 1.2"),
            ("/game/tfl/tags/TAGS.GOR", "TAGS.GOR"),
        ] {
            let reply = answer(&c, url, None, Some("br, gzip")).unwrap();
            assert_eq!(reply.status(), "200", "{url}");
            assert_eq!(header(&reply, "Content-Encoding: "), None, "{url}");
            assert_eq!(header(&reply, "Vary: "), None, "{url}");
            assert!(
                matches!(&reply, Reply::File { path, .. } if path.file_name().and_then(|n| n.to_str()) == Some(expected_name)),
                "{url}: {reply:?}"
            );
        }
        fs::remove_dir_all(base).unwrap();
    }

    #[test]
    fn percent_decode_rejects_signed_hex() {
        assert_eq!(percent_decode("%20").as_deref(), Some(" "));
        assert_eq!(percent_decode("%+1"), None);
        assert_eq!(percent_decode("%-1"), None);
        let (c, base) = site("signed-pct");
        assert_eq!(
            answer(&c, "/game/tags/%+1", None, None).unwrap().status(),
            "404"
        );
        fs::remove_dir_all(base).unwrap();
    }

    #[test]
    fn plugin_cache_invalidates_on_replace_and_removal() {
        let (mut c, base) = site("plugin-cache");
        let dir = base.join("plugins");
        let p = dir.join("Pack");
        plugin(&p, 0x1122_3344, b"first");
        c.downloads_dir = Some(dir.clone());
        assert_eq!(offered_plugin(&c, 0x1122_3344), Some(p.clone()));
        assert_eq!(offered_plugin(&c, 0x1122_3344), Some(p.clone()));
        plugin(&p, 0x5566_7788, b"second-longer");
        assert_eq!(offered_plugin(&c, 0x1122_3344), None);
        assert_eq!(offered_plugin(&c, 0x5566_7788), Some(p.clone()));
        fs::remove_file(&p).unwrap();
        assert_eq!(offered_plugin(&c, 0x5566_7788), None);
        fs::remove_dir_all(base).unwrap();
    }
}
