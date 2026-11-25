use std::collections::{BTreeMap, VecDeque};
use std::io::{self, Read, Write};
use std::net::{Shutdown, SocketAddr, TcpListener, TcpStream, ToSocketAddrs, UdpSocket};
use std::path::Path;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

use anyhow::{Context, anyhow, bail, ensure};
use mandrake_net::wire::{self as net_wire, Body, Message, Splitter, TO_HOST};
use tracing::{info, warn};

use crate::config::TableKeys;
use crate::site::{self, Reply, SiteConfig};
use crate::ws::{self, Decoder, Frame};

pub const BUFFER_CAP: usize = 1 << 20;
pub const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(10);
pub const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
const SITE_WRITE_TIMEOUT: Duration = Duration::from_secs(60);
const MAX_HEAD: usize = 8192;
const READ_SIZE: usize = 16384;
const ANSWERED_KEPT: usize = 64;

#[derive(Debug, Clone)]
pub struct ProxyConfig {
    pub listen: String,
    pub path: String,
    pub target: String,
    pub targets: BTreeMap<String, String>,
    pub max_sessions: usize,
    pub site: SiteConfig,
    pub max_downloads: usize,
}

pub const DEFAULT_MAX_DOWNLOADS: usize = 4;

impl ProxyConfig {
    pub fn load(path: &Path) -> anyhow::Result<ProxyConfig> {
        let text =
            std::fs::read_to_string(path).with_context(|| format!("config {}", path.display()))?;
        ProxyConfig::parse(&text).with_context(|| format!("config {}", path.display()))
    }

    pub fn parse(text: &str) -> anyhow::Result<ProxyConfig> {
        let doc = toml_edit::Document::parse(text.to_string()).map_err(|e| anyhow!("{e}"))?;
        let mut root = TableKeys::new(doc.as_table());
        let path = opt_str(&mut root, "path", "/myth2-ws")?;
        ensure!(
            path.starts_with('/') && !path.ends_with('/') || path == "/",
            "path {path:?}: starts with / and doesn't end with one"
        );
        let mut targets = BTreeMap::new();
        if let Some(t) = root.get("targets") {
            let t = t
                .as_table_like()
                .ok_or_else(|| anyhow!("[targets] is not a table"))?;
            for (name, v) in t.iter() {
                let v = v
                    .as_str()
                    .ok_or_else(|| anyhow!("targets.{name}: not a string"))?;
                targets.insert(name.to_string(), v.to_string());
            }
        }
        let max_sessions = match root.get("max_sessions") {
            None => 64,
            Some(i) => usize::try_from(
                i.as_integer()
                    .ok_or_else(|| anyhow!("max_sessions: not an integer"))?,
            )?,
        };
        let max_downloads = match root.get("max_downloads") {
            None => DEFAULT_MAX_DOWNLOADS,
            Some(i) => usize::try_from(
                i.as_integer()
                    .ok_or_else(|| anyhow!("max_downloads: not an integer"))?,
            )?,
        };
        let offered_plugins = match root.get("offered_plugins") {
            None => None,
            Some(i) => Some(
                i.as_array()
                    .ok_or_else(|| anyhow!("offered_plugins: not an array"))?
                    .iter()
                    .map(|v| {
                        v.as_str()
                            .map(str::to_string)
                            .ok_or_else(|| anyhow!("offered_plugins: not a string"))
                    })
                    .collect::<anyhow::Result<Vec<_>>>()?,
            ),
        };
        let listen = opt_str(&mut root, "listen", "0.0.0.0:8080")?;
        let target = opt_str(&mut root, "target", "127.0.0.1:3453")?;
        let site = SiteConfig {
            web_root: opt_dir(&mut root, "web_root")?,
            game_dir: opt_dir(&mut root, "game_dir")?,
            plugins_dir: opt_dir(&mut root, "plugins_dir")?,
            downloads_dir: opt_dir(&mut root, "downloads_dir")?,
            offered_plugins,
            tfl_dir: opt_dir(&mut root, "tfl_dir")?,
        };
        root.finish("")?;
        Ok(ProxyConfig {
            listen,
            path,
            target,
            targets,
            max_sessions,
            site,
            max_downloads,
        })
    }

    pub fn route(&self, path: &str) -> Option<&str> {
        let (path, query) = path.split_once('?').unwrap_or((path, ""));
        let game = query
            .split('&')
            .find_map(|kv| kv.strip_prefix("game="))
            .filter(|g| !g.is_empty());
        let name = if path == self.path || path == "/" {
            game
        } else {
            let rest = if self.path == "/" {
                path.strip_prefix('/')?
            } else {
                path.strip_prefix(self.path.as_str())?.strip_prefix('/')?
            };
            if rest.is_empty() || rest.contains('/') || game.is_some() {
                return None;
            }
            Some(rest)
        };
        match name {
            None => Some(&self.target),
            Some(n) => self.targets.get(n).map(String::as_str),
        }
    }
}

fn opt_str(root: &mut TableKeys<'_>, key: &'static str, default: &str) -> anyhow::Result<String> {
    match root.get(key) {
        None => Ok(default.to_string()),
        Some(i) => Ok(i
            .as_str()
            .ok_or_else(|| anyhow!("{key}: not a string"))?
            .to_string()),
    }
}

fn opt_dir(
    root: &mut TableKeys<'_>,
    key: &'static str,
) -> anyhow::Result<Option<std::path::PathBuf>> {
    match root.get(key) {
        None => Ok(None),
        Some(i) => Ok(Some(
            i.as_str()
                .ok_or_else(|| anyhow!("{key}: not a string"))?
                .into(),
        )),
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Request {
    pub method: String,
    pub path: String,
    pub headers: Vec<(String, String)>,
}

impl Request {
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(n, _)| n == name)
            .map(|(_, v)| v.as_str())
    }

    fn has_token(&self, name: &str, token: &str) -> bool {
        self.headers
            .iter()
            .filter(|(n, _)| n == name)
            .flat_map(|(_, v)| v.split(','))
            .any(|t| t.trim().eq_ignore_ascii_case(token))
    }
}

fn read_head(s: &mut TcpStream) -> anyhow::Result<(Vec<u8>, Vec<u8>)> {
    read_head_within(s, HANDSHAKE_TIMEOUT)
}

fn read_head_within(s: &mut TcpStream, limit: Duration) -> anyhow::Result<(Vec<u8>, Vec<u8>)> {
    let deadline = Instant::now() + limit;
    let mut buf = Vec::new();
    let mut chunk = [0u8; 1024];
    loop {
        if let Some(i) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
            let rest = buf.split_off(i + 4);
            return Ok((buf, rest));
        }
        ensure!(buf.len() <= MAX_HEAD, "a head over {MAX_HEAD} bytes");
        let left = deadline.saturating_duration_since(Instant::now());
        ensure!(!left.is_zero(), "no whole head in {limit:?}");
        s.set_read_timeout(Some(left))?;
        let n = s.read(&mut chunk)?;
        ensure!(n > 0, "closed in the handshake");
        buf.extend_from_slice(&chunk[..n]);
    }
}

pub fn parse_request(head: &[u8]) -> anyhow::Result<Request> {
    let text = std::str::from_utf8(head).map_err(|_| anyhow!("not text"))?;
    let mut lines = text.split("\r\n");
    let first = lines.next().unwrap_or("");
    let mut parts = first.split(' ');
    let (Some(method), Some(path), Some(version)) = (parts.next(), parts.next(), parts.next())
    else {
        bail!("request line {first:?}");
    };
    ensure!(version.starts_with("HTTP/1."), "version {version:?}");
    let mut headers = Vec::new();
    for l in lines.filter(|l| !l.is_empty()) {
        let (n, v) = l.split_once(':').ok_or_else(|| anyhow!("header {l:?}"))?;
        headers.push((n.trim().to_ascii_lowercase(), v.trim().to_string()));
    }
    Ok(Request {
        method: method.to_string(),
        path: path.to_string(),
        headers,
    })
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Answer {
    Upgrade { target: String, response: Vec<u8> },
    Plain(Vec<u8>),
    Site { reply: Reply, head_only: bool },
}

fn plain_bytes(status: &str, extra: &str, body: &str, kind: &str, head_only: bool) -> Vec<u8> {
    let sent = if head_only { "" } else { body };
    format!(
        "HTTP/1.1 {status}\r\nContent-Type: {kind}\r\nContent-Length: {}\r\n{extra}Connection: close\r\n\r\n{sent}",
        body.len()
    )
    .into_bytes()
}

fn plain(status: &str, extra: &str, body: &str, kind: &str, head_only: bool) -> Answer {
    Answer::Plain(plain_bytes(status, extra, body, kind, head_only))
}

pub fn answer(config: &ProxyConfig, req: &Request, sessions: usize) -> Answer {
    let text = "text/plain";
    let head_only = req.method == "HEAD";
    if req.method != "GET" && !head_only {
        return plain(
            "405 Method Not Allowed",
            "Allow: GET, HEAD\r\n",
            "GET only\n",
            text,
            false,
        );
    }
    let upgrade = req.has_token("upgrade", "websocket");
    if !upgrade
        && let Some(reply) = site::answer_range(
            &config.site,
            &req.path,
            req.header("if-none-match"),
            req.header("accept-encoding"),
            req.header("range"),
        )
    {
        return Answer::Site { reply, head_only };
    }
    if req.path.split('?').next() == Some("/healthz") {
        let body = format!(
            "{{\"status\":\"ok\",\"sessions\":{sessions},\"max_sessions\":{}}}\n",
            config.max_sessions
        );
        return plain("200 OK", "", &body, "application/json", head_only);
    }
    let Some(target) = config.route(&req.path) else {
        return plain("404 Not Found", "", "no such game\n", text, head_only);
    };
    if !upgrade || !req.has_token("connection", "upgrade") {
        return plain(
            "426 Upgrade Required",
            "Upgrade: websocket\r\nConnection: Upgrade\r\n",
            "a WebSocket\n",
            text,
            head_only,
        );
    }
    if req.header("sec-websocket-version") != Some("13") {
        return plain(
            "426 Upgrade Required",
            "Sec-WebSocket-Version: 13\r\n",
            "WebSocket version 13\n",
            text,
            head_only,
        );
    }
    let Some(key) = req.header("sec-websocket-key").filter(|k| !k.is_empty()) else {
        return plain(
            "400 Bad Request",
            "",
            "no Sec-WebSocket-Key\n",
            text,
            head_only,
        );
    };
    if sessions >= config.max_sessions {
        return plain("503 Service Unavailable", "", "full\n", text, head_only);
    }
    let protocol = req
        .header("sec-websocket-protocol")
        .and_then(|p| p.split(',').next())
        .map(str::trim)
        .filter(|p| !p.is_empty())
        .map(|p| format!("Sec-WebSocket-Protocol: {p}\r\n"))
        .unwrap_or_default();
    Answer::Upgrade {
        target: target.to_string(),
        response: format!(
            "HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Accept: {}\r\n{protocol}\r\n",
            ws::accept_key(key)
        )
        .into_bytes(),
    }
}

struct Pipe {
    state: Mutex<PipeState>,
    wake: Condvar,
}

#[derive(Default)]
struct PipeState {
    chunks: VecDeque<Vec<u8>>,
    bytes: usize,
    finished: bool,
    aborted: bool,
}

impl Pipe {
    fn new() -> Pipe {
        Pipe {
            state: Mutex::new(PipeState::default()),
            wake: Condvar::new(),
        }
    }

    fn push(&self, chunk: Vec<u8>) -> Result<bool, ()> {
        let mut s = self.state.lock().unwrap();
        if s.finished || s.aborted {
            return Ok(false);
        }
        if chunk.is_empty() {
            return Ok(true);
        }
        if s.bytes + chunk.len() > BUFFER_CAP {
            return Err(());
        }
        s.bytes += chunk.len();
        s.chunks.push_back(chunk);
        self.wake.notify_all();
        Ok(true)
    }

    fn finish(&self) {
        self.state.lock().unwrap().finished = true;
        self.wake.notify_all();
    }

    fn abort(&self) {
        let mut s = self.state.lock().unwrap();
        s.aborted = true;
        s.chunks.clear();
        s.bytes = 0;
        self.wake.notify_all();
    }

    fn pop(&self) -> Option<Vec<u8>> {
        let mut s = self.state.lock().unwrap();
        loop {
            if s.aborted {
                return None;
            }
            if let Some(chunk) = s.chunks.pop_front() {
                s.bytes -= chunk.len();
                return Some(chunk);
            }
            if s.finished {
                return None;
            }
            s = self.wake.wait(s).unwrap();
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Side {
    Server { udp_port: u16 },
    Client,
}

#[derive(Debug, Default)]
pub struct Stats {
    pub ws_to_tcp: AtomicU64,
    pub tcp_to_ws: AtomicU64,
    pub pongs_answered: AtomicU64,
    pub pongs_dropped: AtomicU64,
    pub why: Mutex<Option<String>>,
}

impl Stats {
    fn note(&self, why: impl Into<String>) {
        let mut w = self.why.lock().unwrap();
        if w.is_none() {
            *w = Some(why.into());
        }
    }
}

struct Session {
    ws: TcpStream,
    tcp: TcpStream,
    to_ws: Pipe,
    to_tcp: Pipe,
    side: Side,
    stats: Arc<Stats>,
    mask: Mutex<Masks>,
    pongs: Mutex<Pongs>,
}

#[derive(Default)]
struct Pongs {
    from: Option<u8>,
    answered: VecDeque<u32>,
}

struct Follow {
    splitter: Splitter,
    whole: bool,
    held: Vec<u8>,
    learned: Option<u8>,
}

impl Session {
    fn abort(&self, why: impl Into<String>) {
        self.stats.note(why);
        self.to_ws.abort();
        self.to_tcp.abort();
        let _ = self.ws.shutdown(Shutdown::Both);
        let _ = self.tcp.shutdown(Shutdown::Both);
    }

    fn finish(&self, why: impl Into<String>, last: Option<Vec<u8>>) {
        self.stats.note(why);
        if let Some(f) = last {
            self.push_ws(f);
        }
        self.to_ws.finish();
        self.to_tcp.finish();
    }

    fn push_ws(&self, frame: Vec<u8>) {
        if self.to_ws.push(frame).is_err() {
            self.abort("the WebSocket's buffer passed 1 MiB");
        }
    }

    fn frame(&self, opcode: u8, payload: &[u8]) -> Vec<u8> {
        let mask = match self.side {
            Side::Server { .. } => None,
            Side::Client => Some(self.mask.lock().unwrap().next()),
        };
        ws::encode(opcode, payload, mask)
    }

    fn read_ws(&self, early: Vec<u8>) {
        let mut d = Decoder::new(matches!(self.side, Side::Server { .. }));
        d.push(&early);
        let mut greeting = match self.side {
            Side::Server { udp_port } => udp_port.to_be_bytes().to_vec(),
            Side::Client => Vec::new(),
        };
        let mut follow = matches!(self.side, Side::Server { .. }).then(|| Follow {
            splitter: Splitter::with_greeting(),
            whole: false,
            held: Vec::new(),
            learned: None,
        });
        let mut buf = vec![0u8; READ_SIZE];
        let mut ws = &self.ws;
        loop {
            loop {
                match d.next_frame() {
                    Ok(None) => break,
                    Ok(Some(Frame::Data(mut p))) => {
                        let n = greeting.len().min(p.len());
                        p[..n].copy_from_slice(&greeting[..n]);
                        greeting.drain(..n);
                        self.stats
                            .ws_to_tcp
                            .fetch_add(p.len() as u64, Ordering::Relaxed);
                        if self.to_host(&mut follow, p).is_err() {
                            return self.abort("the TCP stream's buffer passed 1 MiB");
                        }
                    }
                    Ok(Some(Frame::Ping(p))) => self.push_ws(self.frame(ws::OP_PONG, &p)),
                    Ok(Some(Frame::Pong)) => {}
                    Ok(Some(Frame::Close(code))) => {
                        let reply = match self.side {
                            Side::Server { .. } => Some(self.frame(
                                ws::OP_CLOSE,
                                &code.map(ws::close_payload).unwrap_or_default(),
                            )),
                            Side::Client => None,
                        };
                        return self.finish(format!("the WebSocket closed ({code:?})"), reply);
                    }
                    Err(e) => {
                        let f = self.frame(ws::OP_CLOSE, &ws::close_payload(e.code));
                        return self.finish(format!("{} ({})", e.why, e.code), Some(f));
                    }
                }
            }
            match ws.read(&mut buf) {
                Ok(0) => return self.finish("the WebSocket's EOF", None),
                Ok(n) => d.push(&buf[..n]),
                Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
                Err(e) => return self.finish(format!("the WebSocket's read: {e}"), None),
            }
        }
    }

    fn to_host(&self, follow: &mut Option<Follow>, p: Vec<u8>) -> Result<(), ()> {
        let Some(f) = follow.as_mut() else {
            return self.to_tcp.push(p).map(drop);
        };
        let passing = !f.whole;
        if !passing {
            f.held.extend_from_slice(&p);
        }
        f.splitter.push(&p);
        loop {
            match f.splitter.next_frame() {
                Ok(None) => break,
                Ok(Some(net_wire::Frame::Greeting(_))) => {}
                Ok(Some(net_wire::Frame::Message(m))) => {
                    if !f.whole {
                        if let Ok(Message {
                            from,
                            body: Body::Pong { .. },
                            ..
                        }) = Message::decode(&m)
                        {
                            f.learned = Some(from);
                        }
                        continue;
                    }
                    let rest = f.held.split_off(m.len());
                    let m = std::mem::replace(&mut f.held, rest);
                    if !self.page_pong_answered(&m) {
                        self.to_tcp.push(m)?;
                    }
                }
                Err(e) => {
                    warn!("proxy: the browser's stream: {e:?}; its pings are its own");
                    self.pongs.lock().unwrap().from = None;
                    let held = std::mem::take(&mut f.held);
                    *follow = None;
                    let rest = if passing { p } else { held };
                    return self.to_tcp.push(rest).map(drop);
                }
            }
        }
        if passing {
            if f.splitter.pending() == 0 && f.learned.is_some() {
                f.whole = true;
                let mut pongs = self.pongs.lock().unwrap();
                pongs.from = f.learned;
                return self.to_tcp.push(p).map(drop);
            }
            self.to_tcp.push(p)?;
        }
        Ok(())
    }

    fn page_pong_answered(&self, m: &[u8]) -> bool {
        let Ok(Message {
            from,
            body: Body::Pong { sdl_ticks },
            ..
        }) = Message::decode(m)
        else {
            return false;
        };
        let mut p = self.pongs.lock().unwrap();
        let answered = p.answered.iter().position(|t| *t == sdl_ticks);
        if let Some(i) = answered {
            p.answered.remove(i);
        } else if p.answered.len() >= ANSWERED_KEPT {
            p.answered.pop_front();
        }
        let same = p.from == Some(from);
        p.from = Some(from);
        let drop = answered.is_some() && same;
        if drop {
            self.stats.pongs_dropped.fetch_add(1, Ordering::Relaxed);
        }
        drop
    }

    fn answer_ping(&self, m: &[u8]) {
        let Ok(Message {
            body: Body::Ping { sdl_ticks, .. },
            ..
        }) = Message::decode(m)
        else {
            return;
        };
        let mut p = self.pongs.lock().unwrap();
        let Some(from) = p.from else {
            return;
        };
        if p.answered.len() >= ANSWERED_KEPT {
            return;
        }
        let Ok(pong) = Message::new(TO_HOST, from, Body::Pong { sdl_ticks }).encode() else {
            return;
        };
        p.answered.push_back(sdl_ticks);
        if self.to_tcp.push(pong).is_err() {
            drop(p);
            return self.abort("the TCP stream's buffer passed 1 MiB");
        }
        self.stats.pongs_answered.fetch_add(1, Ordering::Relaxed);
    }

    fn read_tcp(&self) {
        let mut buf = vec![0u8; READ_SIZE];
        let mut tcp = &self.tcp;
        let mut follow = matches!(self.side, Side::Server { .. }).then(Splitter::with_greeting);
        loop {
            match tcp.read(&mut buf) {
                Ok(0) => {
                    let close = self.frame(ws::OP_CLOSE, &ws::close_payload(ws::CLOSE_NORMAL));
                    return self.finish("the TCP stream's EOF", Some(close));
                }
                Ok(n) => {
                    self.stats.tcp_to_ws.fetch_add(n as u64, Ordering::Relaxed);
                    if let Some(s) = follow.as_mut() {
                        s.push(&buf[..n]);
                        loop {
                            match s.next_frame() {
                                Ok(Some(net_wire::Frame::Message(m))) => self.answer_ping(&m),
                                Ok(Some(net_wire::Frame::Greeting(_))) => {}
                                Ok(None) => break,
                                Err(_) => {
                                    self.pongs.lock().unwrap().from = None;
                                    follow = None;
                                    break;
                                }
                            }
                        }
                    }
                    let f = self.frame(ws::OP_BINARY, &buf[..n]);
                    self.push_ws(f);
                }
                Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
                Err(e) => return self.finish(format!("the TCP stream's read: {e}"), None),
            }
        }
    }

    fn write(&self, pipe: &Pipe, mut s: &TcpStream) {
        while let Some(bytes) = pipe.pop() {
            if let Err(e) = s.write_all(&bytes) {
                return self.abort(format!("a write: {e}"));
            }
        }
        let _ = s.shutdown(Shutdown::Both);
    }
}

struct Masks(u64);

impl Masks {
    fn new() -> Masks {
        use std::hash::{BuildHasher, Hasher};
        let mut h = std::collections::hash_map::RandomState::new().build_hasher();
        h.write_u64(std::process::id().into());
        Masks(h.finish() | 1)
    }

    fn next(&mut self) -> [u8; 4] {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        (self.0 as u32).to_le_bytes()
    }
}

pub fn run_session(ws: TcpStream, tcp: TcpStream, side: Side, early: Vec<u8>) -> Arc<Stats> {
    let stats = Arc::new(Stats::default());
    let _ = ws.set_read_timeout(None);
    let _ = ws.set_nodelay(true);
    let _ = tcp.set_nodelay(true);
    let session = Arc::new(Session {
        ws,
        tcp,
        to_ws: Pipe::new(),
        to_tcp: Pipe::new(),
        side,
        stats: stats.clone(),
        mask: Mutex::new(Masks::new()),
        pongs: Mutex::new(Pongs::default()),
    });
    std::thread::scope(|s| {
        let a = &session;
        s.spawn(move || a.read_ws(early));
        s.spawn(move || a.read_tcp());
        s.spawn(move || a.write(&a.to_ws, &a.ws));
        s.spawn(move || a.write(&a.to_tcp, &a.tcp));
    });
    stats
}

fn resolve(target: &str) -> io::Result<SocketAddr> {
    target
        .to_socket_addrs()?
        .next()
        .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, format!("{target}: no address")))
}

static DOWNLOADS: AtomicUsize = AtomicUsize::new(0);

struct Slot<'a>(&'a AtomicUsize);

impl<'a> Slot<'a> {
    fn take(counter: &'a AtomicUsize, max: usize) -> Option<Slot<'a>> {
        counter
            .try_update(Ordering::SeqCst, Ordering::SeqCst, |n| {
                (n < max).then_some(n + 1)
            })
            .ok()
            .map(|_| Slot(counter))
    }
}

impl Drop for Slot<'_> {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::SeqCst);
    }
}

fn serve(config: &ProxyConfig, mut s: TcpStream, open: &AtomicUsize, id: u64) {
    let peer = s.peer_addr().ok();
    let _ = s.set_read_timeout(Some(HANDSHAKE_TIMEOUT));
    let (head, early) = match read_head(&mut s) {
        Ok(h) => h,
        Err(e) => return info!("proxy #{id} {peer:?}: no request: {e:#}"),
    };
    let req = match parse_request(&head) {
        Ok(r) => r,
        Err(e) => {
            let _ = s.write_all(&plain_bytes("400 Bad Request", "", "", "text/plain", false));
            return info!("proxy #{id} {peer:?}: a bad request: {e:#}");
        }
    };
    let forwarded = req.header("x-forwarded-for").unwrap_or("-").to_string();
    let (target, response) = match answer(config, &req, open.load(Ordering::SeqCst)) {
        Answer::Plain(r) => {
            let _ = s.write_all(&r);
            let status = String::from_utf8_lossy(&r[9..r.len().min(12)]).into_owned();
            if req.path != "/healthz" {
                info!(
                    "proxy #{id} {peer:?} ({forwarded}): {} {} -> {status}",
                    req.method, req.path
                );
            }
            return;
        }
        Answer::Site { reply, head_only } => {
            let _slot = match &reply {
                Reply::Part { .. } if !head_only => {
                    match Slot::take(&DOWNLOADS, config.max_downloads) {
                        Some(slot) => Some(slot),
                        None => {
                            let _ = s.write_all(&plain_bytes(
                                "503 Service Unavailable",
                                "Retry-After: 5\r\n",
                                "",
                                "text/plain",
                                false,
                            ));
                            return info!(
                                "site #{id} {peer:?} ({forwarded}): {} -> 503, {} downloads at once",
                                req.path, config.max_downloads
                            );
                        }
                    }
                }
                _ => None,
            };
            let status = reply.status().to_string();
            let _ = s.set_write_timeout(Some(SITE_WRITE_TIMEOUT));
            let sent = match &reply {
                Reply::Bytes { head, body } => s
                    .write_all(head)
                    .and_then(|_| if head_only { Ok(()) } else { s.write_all(body) }),
                Reply::File { head, path } => s.write_all(head).and_then(|_| {
                    if head_only {
                        return Ok(());
                    }
                    let mut f = std::fs::File::open(path)?;
                    io::copy(&mut f, &mut s).map(|_| ())
                }),
                Reply::Part {
                    head,
                    path,
                    start,
                    len,
                } => s.write_all(head).and_then(|_| {
                    if head_only {
                        return Ok(());
                    }
                    use std::io::Seek;
                    let mut f = std::fs::File::open(path)?;
                    f.seek(io::SeekFrom::Start(*start))?;
                    io::copy(&mut f.take(*len), &mut s).map(|_| ())
                }),
            };
            let _ = s.shutdown(Shutdown::Write);
            match sent {
                Ok(()) => info!(
                    "site #{id} {peer:?} ({forwarded}): {} {} -> {status}",
                    req.method, req.path
                ),
                Err(e) => info!(
                    "site #{id} {peer:?} ({forwarded}): {} {} -> {status}, cut: {e}",
                    req.method, req.path
                ),
            }
            return;
        }
        Answer::Upgrade { target, response } => (target, response),
    };
    let Some(_session) = Slot::take(open, config.max_sessions) else {
        let _ = s.write_all(&plain_bytes(
            "503 Service Unavailable",
            "",
            "full\n",
            "text/plain",
            false,
        ));
        return info!("proxy #{id} {peer:?} ({forwarded}): {} -> 503", req.path);
    };
    let upstream = resolve(&target).and_then(|a| TcpStream::connect_timeout(&a, CONNECT_TIMEOUT));
    let tcp = match upstream {
        Ok(t) => t,
        Err(e) => {
            let _ = s.write_all(&plain_bytes("502 Bad Gateway", "", "", "text/plain", false));
            return warn!("proxy #{id} {peer:?} ({forwarded}): {target}: {e}");
        }
    };
    let udp = match tcp.local_addr().and_then(|a| UdpSocket::bind((a.ip(), 0))) {
        Ok(u) => u,
        Err(e) => {
            let _ = s.write_all(&plain_bytes(
                "500 Internal Server Error",
                "",
                "",
                "text/plain",
                false,
            ));
            return warn!("proxy #{id}: a UDP socket: {e}");
        }
    };
    let udp_port = udp.local_addr().map(|a| a.port()).unwrap_or(0);
    if let Err(e) = s.write_all(&response) {
        return info!("proxy #{id} {peer:?}: the 101: {e}");
    }
    info!(
        "proxy #{id} {peer:?} ({forwarded}): {} to {target} ({:?}), UDP port {udp_port}",
        req.path,
        tcp.peer_addr().ok()
    );
    let stats = run_session(s, tcp, Side::Server { udp_port }, early);
    drop(udp);
    info!(
        "proxy #{id}: closed, {} bytes up, {} down, pings answered {} (the page's pongs dropped {}): {}",
        stats.ws_to_tcp.load(Ordering::Relaxed),
        stats.tcp_to_ws.load(Ordering::Relaxed),
        stats.pongs_answered.load(Ordering::Relaxed),
        stats.pongs_dropped.load(Ordering::Relaxed),
        stats.why.lock().unwrap().as_deref().unwrap_or("?")
    );
}

pub fn run(config: ProxyConfig, listener: TcpListener) -> anyhow::Result<()> {
    let config = Arc::new(config);
    let open = Arc::new(AtomicUsize::new(0));
    let mut id = 0u64;
    for s in listener.incoming() {
        let s = match s {
            Ok(s) => s,
            Err(e) => {
                warn!("proxy: accept: {e}");
                continue;
            }
        };
        id += 1;
        let (config, open) = (config.clone(), open.clone());
        std::thread::spawn(move || serve(&config, s, &open, id));
    }
    Ok(())
}

pub fn tunnel(listener: TcpListener, url: &str, once: bool) -> anyhow::Result<()> {
    let rest = url
        .strip_prefix("ws://")
        .ok_or_else(|| anyhow!("{url}: not ws://"))?;
    let (authority, path) = match rest.find('/') {
        Some(i) => (&rest[..i], &rest[i..]),
        None => (rest, "/"),
    };
    let (authority, path) = (authority.to_string(), path.to_string());
    for (n, s) in listener.incoming().enumerate() {
        let local = s?;
        let (authority, path) = (authority.clone(), path.clone());
        let t = std::thread::spawn(move || match connect_ws(&authority, &path) {
            Ok((ws, early)) => {
                info!("tunnel #{n}: through ws://{authority}{path}");
                let stats = run_session(ws, local, Side::Client, early);
                info!(
                    "tunnel #{n}: closed, {} bytes up, {} down: {}",
                    stats.tcp_to_ws.load(Ordering::Relaxed),
                    stats.ws_to_tcp.load(Ordering::Relaxed),
                    stats.why.lock().unwrap().as_deref().unwrap_or("?")
                );
            }
            Err(e) => warn!("tunnel #{n}: ws://{authority}{path}: {e:#}"),
        });
        if once {
            let _ = t.join();
            break;
        }
    }
    Ok(())
}

pub fn connect_ws(authority: &str, path: &str) -> anyhow::Result<(TcpStream, Vec<u8>)> {
    let addr = resolve(authority)?;
    let mut s = TcpStream::connect_timeout(&addr, CONNECT_TIMEOUT)?;
    s.set_read_timeout(Some(HANDSHAKE_TIMEOUT))?;
    let mut key = [0u8; 16];
    let mut m = Masks::new();
    for c in key.chunks_mut(4) {
        c.copy_from_slice(&m.next());
    }
    let key = ws::base64(&key);
    write!(
        s,
        "GET {path} HTTP/1.1\r\nHost: {authority}\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Key: {key}\r\nSec-WebSocket-Version: 13\r\n\r\n"
    )?;
    let (head, early) = read_head(&mut s)?;
    let text = String::from_utf8_lossy(&head);
    ensure!(
        text.starts_with("HTTP/1.1 101"),
        "{}",
        text.lines().next().unwrap_or("")
    );
    let want = ws::accept_key(&key);
    ensure!(
        text.lines().any(|l| l
            .split_once(':')
            .is_some_and(
                |(n, v)| n.trim().eq_ignore_ascii_case("sec-websocket-accept") && v.trim() == want
            )),
        "a wrong Sec-WebSocket-Accept"
    );
    Ok((s, early))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::Ipv4Addr;

    fn config(target: &str) -> ProxyConfig {
        ProxyConfig::parse(&format!(
            "listen = \"127.0.0.1:0\"\ntarget = \"{target}\"\n[targets]\nother = \"10.1.2.3:3453\"\n"
        ))
        .unwrap()
    }

    #[test]
    fn a_head_must_arrive_whole_within_the_limit() {
        let l = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        let a = l.local_addr().unwrap();
        let trickle = std::thread::spawn(move || {
            let mut c = TcpStream::connect(a).unwrap();
            let head = format!("GET / HTTP/1.1\r\nHost: x\r\nX-Pad: {}\r\n", "p".repeat(75));
            for b in head.as_bytes() {
                if c.write_all(&[*b]).is_err() {
                    break;
                }
                std::thread::sleep(Duration::from_millis(40));
            }
        });
        let (mut s, _) = l.accept().unwrap();
        let t0 = Instant::now();
        let e = read_head_within(&mut s, Duration::from_millis(300)).unwrap_err();
        assert!(t0.elapsed() < Duration::from_millis(2500), "{e:#}");
        let e = format!("{e:#}");
        assert!(
            ["no whole head", "timed out", "temporarily unavailable"]
                .iter()
                .any(|w| e.contains(w)),
            "{e}"
        );
        drop(s);
        trickle.join().unwrap();
        let sender = std::thread::spawn(move || {
            let mut c = TcpStream::connect(a).unwrap();
            c.write_all(b"GET / HTTP/1.1\r\n\r\nrest").unwrap();
            std::thread::sleep(Duration::from_millis(100));
        });
        let (mut s, _) = l.accept().unwrap();
        let (head, rest) = read_head_within(&mut s, Duration::from_secs(5)).unwrap();
        assert_eq!(&head[..], b"GET / HTTP/1.1\r\n\r\n");
        assert!(b"rest".starts_with(&rest), "{rest:?}");
        sender.join().unwrap();
    }

    #[test]
    fn targets_come_from_the_config_only() {
        let c = config("127.0.0.1:3453");
        assert_eq!(c.route("/myth2-ws"), Some("127.0.0.1:3453"));
        assert_eq!(c.route("/"), Some("127.0.0.1:3453"));
        assert_eq!(c.route("/myth2-ws/other"), Some("10.1.2.3:3453"));
        assert_eq!(c.route("/myth2-ws?game=other"), Some("10.1.2.3:3453"));
        assert_eq!(c.route("/?game=other&x=1"), Some("10.1.2.3:3453"));
        for bad in [
            "/myth2-ws/none",
            "/myth2-ws?game=8.8.8.8:3453",
            "/myth2-ws/other/x",
            "/myth2-wsx",
            "/x",
            "/myth2-ws/",
        ] {
            assert_eq!(c.route(bad), None, "{bad}");
        }
        assert!(ProxyConfig::parse("nope = 1").is_err());
    }

    #[test]
    fn the_handshake_answers_with_the_accept_key() {
        let c = config("127.0.0.1:3453");
        let head = b"GET /myth2-ws HTTP/1.1\r\nHost: x\r\nUpgrade: WebSocket\r\nConnection: keep-alive, Upgrade\r\nSec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\nSec-WebSocket-Version: 13\r\nSec-WebSocket-Protocol: myth2, other\r\n\r\n";
        let req = parse_request(&head[..head.len() - 4]).unwrap();
        let Answer::Upgrade { target, response } = answer(&c, &req, 0) else {
            panic!("no upgrade");
        };
        assert_eq!(target, "127.0.0.1:3453");
        let r = String::from_utf8(response).unwrap();
        assert!(r.starts_with("HTTP/1.1 101 "), "{r}");
        assert!(r.contains("Sec-WebSocket-Accept: s3pPLMBiTxaQ9kYGzzhZRbK+xOo=\r\n"));
        assert!(r.contains("Sec-WebSocket-Protocol: myth2\r\n"));
        let status = |a: Answer| match a {
            Answer::Plain(r) => String::from_utf8(r[9..12].to_vec()).unwrap(),
            Answer::Upgrade { .. } => "101".into(),
            Answer::Site { reply, .. } => reply.status().to_string(),
        };
        assert_eq!(status(answer(&c, &req, 64)), "503");
        let mut no_key = req.clone();
        no_key.headers.retain(|(n, _)| n != "sec-websocket-key");
        assert_eq!(status(answer(&c, &no_key, 0)), "400");
        let mut elsewhere = req.clone();
        elsewhere.path = "/myth2-ws/8.8.8.8".into();
        assert_eq!(status(answer(&c, &elsewhere, 0)), "404");
        let plain_get = parse_request(b"GET /myth2-ws HTTP/1.1\r\nHost: x").unwrap();
        assert_eq!(status(answer(&c, &plain_get, 0)), "426");
        let health = parse_request(b"GET /healthz HTTP/1.1").unwrap();
        assert_eq!(status(answer(&c, &health, 3)), "200");
    }

    #[test]
    fn the_site_and_the_websocket_share_the_port() {
        let base = std::env::temp_dir().join(format!("m2-proxy-site-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(base.join("web")).unwrap();
        std::fs::create_dir_all(base.join("game/tags")).unwrap();
        std::fs::write(base.join("web/index.html"), "<p>myth</p>").unwrap();
        std::fs::write(base.join("game/tags/small install"), "abc").unwrap();
        let mut c = config("127.0.0.1:3453");
        c.site = ProxyConfig::parse(&format!(
            "web_root = \"{0}/web\"\ngame_dir = \"{0}/game\"\n",
            base.display()
        ))
        .unwrap()
        .site;
        let get = |path: &str, extra: &str| {
            parse_request(format!("GET {path} HTTP/1.1\r\nHost: x{extra}").as_bytes()).unwrap()
        };
        assert!(matches!(answer(&c, &get("/", ""), 0), Answer::Site { .. }));
        let up = "\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Key: a2V5\r\nSec-WebSocket-Version: 13";
        assert!(matches!(
            answer(&c, &get("/", up), 0),
            Answer::Upgrade { .. }
        ));
        assert!(matches!(
            answer(&c, &get("/myth2-ws", ""), 0),
            Answer::Plain(_)
        ));
        let mut head = get("/", "");
        head.method = "HEAD".into();
        assert!(matches!(
            answer(&c, &head, 0),
            Answer::Site {
                head_only: true,
                ..
            }
        ));

        let l = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        let a = l.local_addr().unwrap();
        std::thread::spawn(move || run(c, l));
        let fetch = |path: &str| {
            let mut s = TcpStream::connect(a).unwrap();
            write!(s, "GET {path} HTTP/1.1\r\nHost: x\r\n\r\n").unwrap();
            let mut out = String::new();
            s.read_to_string(&mut out).unwrap();
            out
        };
        let page = fetch("/");
        assert!(page.starts_with("HTTP/1.1 200 OK\r\n"), "{page}");
        assert!(page.ends_with("\r\n\r\n<p>myth</p>"), "{page}");
        let list = fetch("/game/list");
        assert!(
            list.contains("[{\"path\":\"tags/small install\",\"size\":3,"),
            "{list}"
        );
        let tags = fetch("/game/tags/small%20install");
        assert!(tags.ends_with("\r\n\r\nabc"), "{tags}");
        std::fs::remove_dir_all(base).unwrap();
    }

    fn proxy(target: SocketAddr) -> SocketAddr {
        let l = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        let a = l.local_addr().unwrap();
        std::thread::spawn(move || run(config(&target.to_string()), l));
        a
    }

    fn read_frame(s: &mut TcpStream, d: &mut Decoder) -> Frame {
        let mut b = [0u8; 4096];
        loop {
            if let Some(f) = d.next_frame().unwrap() {
                return f;
            }
            let n = s.read(&mut b).unwrap();
            assert!(n > 0, "EOF");
            d.push(&b[..n]);
        }
    }

    #[test]
    fn bytes_pass_unchanged_but_the_greeting_ping_close_and_the_cap() {
        let up = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        let p = proxy(up.local_addr().unwrap());
        let (mut ws, early) = connect_ws(&p.to_string(), "/myth2-ws").unwrap();
        assert!(early.is_empty());
        let (mut host, _) = up.accept().unwrap();
        host.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
        let key = Some([0x11, 0x22, 0x33, 0x44]);
        ws.write_all(&ws::encode(ws::OP_BINARY, &[0xff], key))
            .unwrap();
        ws.write_all(&ws::encode(ws::OP_BINARY, &[0xee, 1, 2, 3], key))
            .unwrap();
        let mut got = [0u8; 5];
        host.read_exact(&mut got).unwrap();
        let port = u16::from_be_bytes([got[0], got[1]]);
        assert_ne!(port, 0xffee, "the greeting is replaced");
        assert_ne!(port, 0);
        assert_eq!(&got[2..], &[1, 2, 3]);
        assert!(UdpSocket::bind((Ipv4Addr::LOCALHOST, port)).is_err());
        host.write_all(&[0xff, 0xee, 9, 9]).unwrap();
        let mut d = Decoder::new(false);
        assert_eq!(
            read_frame(&mut ws, &mut d),
            Frame::Data(vec![0xff, 0xee, 9, 9])
        );
        ws.write_all(&ws::encode(ws::OP_PING, b"x", key)).unwrap();
        assert_eq!(read_frame(&mut ws, &mut d), Frame::Pong);
        ws.write_all(&ws::encode(ws::OP_CLOSE, &ws::close_payload(1000), key))
            .unwrap();
        assert_eq!(read_frame(&mut ws, &mut d), Frame::Close(Some(1000)));
        let mut rest = Vec::new();
        host.read_to_end(&mut rest).unwrap();
        assert!(rest.is_empty());

        let (mut ws, _) = connect_ws(&p.to_string(), "/").unwrap();
        let (host, _) = up.accept().unwrap();
        drop(host);
        let mut d = Decoder::new(false);
        assert_eq!(read_frame(&mut ws, &mut d), Frame::Close(Some(1000)));

        let (mut ws, _) = connect_ws(&p.to_string(), "/").unwrap();
        let (_host, _) = up.accept().unwrap();
        ws.write_all(&ws::encode(ws::OP_BINARY, b"ab", None))
            .unwrap();
        let mut d = Decoder::new(false);
        assert_eq!(read_frame(&mut ws, &mut d), Frame::Close(Some(1002)));

        let (ws, _) = connect_ws(&p.to_string(), "/").unwrap();
        let (mut host, _) = up.accept().unwrap();
        host.set_write_timeout(Some(Duration::from_secs(2)))
            .unwrap();
        let chunk = vec![7u8; 65536];
        let mut sent = 0usize;
        while let Ok(n) = host.write(&chunk) {
            sent += n;
            assert!(sent < 64 << 20, "never closed");
        }
        assert!(sent > BUFFER_CAP, "{sent} bytes");
        host.set_read_timeout(Some(Duration::from_secs(2))).unwrap();
        let mut b = [0u8; 16];
        assert!(
            matches!(host.read(&mut b), Ok(0)),
            "the proxy closed the host's side"
        );
        drop(ws);
    }

    #[test]
    fn the_proxy_answers_the_hosts_pings_for_the_page() {
        let up = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        let p = proxy(up.local_addr().unwrap());
        let (mut ws, _) = connect_ws(&p.to_string(), "/myth2-ws").unwrap();
        let (mut host, _) = up.accept().unwrap();
        host.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
        let key = Some([1, 2, 3, 4]);
        let enc = |to: u8, from: u8, body: Body| Message::new(to, from, body).encode().unwrap();
        let ping = |ticks: u32| {
            enc(
                0xff,
                0,
                Body::Ping {
                    sdl_ticks: ticks,
                    latencies: vec![0; 8],
                },
            )
        };
        let pong = |ticks: u32| enc(TO_HOST, 3, Body::Pong { sdl_ticks: ticks });
        let chat = enc(TO_HOST, 3, Body::ServerCommand(7));
        let mut ws2 = ws.try_clone().unwrap();
        let mut d = Decoder::new(false);
        let mut to_page = |host: &mut TcpStream, bytes: &[u8], seen: &[u8]| {
            host.write_all(bytes).unwrap();
            let mut got = Vec::new();
            while got.len() < seen.len() {
                match read_frame(&mut ws, &mut d) {
                    Frame::Data(b) => got.extend(b),
                    f => panic!("{f:?}"),
                }
            }
            assert_eq!(got, seen);
        };
        ws2.write_all(&ws::encode(ws::OP_BINARY, &[0, 0], key))
            .unwrap();
        let mut got = [0u8; 2];
        host.read_exact(&mut got).unwrap();
        to_page(&mut host, &[0x0d, 0x7d], &[0x0d, 0x7d]);
        to_page(&mut host, &ping(100), &ping(100));
        ws2.write_all(&ws::encode(ws::OP_BINARY, &pong(100), key))
            .unwrap();
        let mut got = vec![0u8; pong(100).len()];
        host.read_exact(&mut got).unwrap();
        assert_eq!(got, pong(100));
        to_page(&mut host, &ping(200), &ping(200));
        let mut got = vec![0u8; pong(200).len()];
        host.read_exact(&mut got).unwrap();
        assert_eq!(got, pong(200));
        let mut late = pong(200);
        late.extend_from_slice(&chat[..3]);
        ws2.write_all(&ws::encode(ws::OP_BINARY, &late, key))
            .unwrap();
        ws2.write_all(&ws::encode(ws::OP_BINARY, &chat[3..], key))
            .unwrap();
        let mut got = vec![0u8; chat.len()];
        host.read_exact(&mut got).unwrap();
        assert_eq!(got, chat);
        let third = ping(300);
        host.write_all(&third[..5]).unwrap();
        std::thread::sleep(Duration::from_millis(20));
        to_page(&mut host, &third[5..], &third);
        let mut got = vec![0u8; pong(300).len()];
        host.read_exact(&mut got).unwrap();
        assert_eq!(got, pong(300));
        ws2.write_all(&ws::encode(ws::OP_BINARY, &pong(300), key))
            .unwrap();
        ws2.write_all(&ws::encode(ws::OP_CLOSE, &ws::close_payload(1000), key))
            .unwrap();
        let mut rest = Vec::new();
        host.read_to_end(&mut rest).unwrap();
        assert!(rest.is_empty(), "{rest:?}");
    }

    #[test]
    fn root_path_routes_named_targets_and_head_omits_plain_body() {
        let c = ProxyConfig::parse(
            "path = \"/\"\ntarget = \"127.0.0.1:3453\"\n[targets]\nother = \"10.1.2.3:3453\"\n",
        )
        .unwrap();
        assert_eq!(c.route("/"), Some("127.0.0.1:3453"));
        assert_eq!(c.route("/other"), Some("10.1.2.3:3453"));
        assert_eq!(c.route("/?game=other"), Some("10.1.2.3:3453"));
        for bad in ["/none", "/other/x", "//other", "/other?game=other"] {
            assert_eq!(c.route(bad), None, "{bad}");
        }

        for path in ["/healthz", "/none", "/"] {
            let req = parse_request(format!("HEAD {path} HTTP/1.1\r\nHost: x").as_bytes()).unwrap();
            let Answer::Plain(resp) = answer(&c, &req, 0) else {
                panic!("expected Plain for HEAD {path}");
            };
            let text = String::from_utf8(resp).unwrap();
            assert!(
                text.ends_with("\r\n\r\n"),
                "HEAD {path} sent body: {text:?}"
            );
            assert!(text.contains("Content-Length: "), "{text:?}");
        }
    }

    #[test]
    fn session_cap_is_enforced_with_slot_and_released_on_close() {
        let up = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        let target = up.local_addr().unwrap();
        let cfg = ProxyConfig::parse(&format!(
            "listen = \"127.0.0.1:0\"\ntarget = \"{target}\"\nmax_sessions = 1\n"
        ))
        .unwrap();
        let l = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        let addr = l.local_addr().unwrap();
        std::thread::spawn(move || run(cfg, l));

        let (ws1, _) = connect_ws(&addr.to_string(), "/myth2-ws").unwrap();
        let (host1, _) = up.accept().unwrap();
        let err = connect_ws(&addr.to_string(), "/myth2-ws").unwrap_err();
        assert!(format!("{err:#}").contains("503"), "{err:#}");

        drop(ws1);
        drop(host1);
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            match connect_ws(&addr.to_string(), "/myth2-ws") {
                Ok((ws2, _)) => {
                    drop(ws2);
                    break;
                }
                Err(e) if Instant::now() < deadline => {
                    let _ = e;
                    std::thread::sleep(Duration::from_millis(10));
                }
                Err(e) => panic!("slot not released after close: {e:#}"),
            }
        }
    }

    #[test]
    fn stalled_page_stops_proxy_pongs_and_resumes_when_page_pongs() {
        let up = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        let p = proxy(up.local_addr().unwrap());
        let (mut ws, _) = connect_ws(&p.to_string(), "/myth2-ws").unwrap();
        let (mut host, _) = up.accept().unwrap();
        host.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
        let key = Some([1, 2, 3, 4]);
        let enc = |to: u8, from: u8, body: Body| Message::new(to, from, body).encode().unwrap();
        let ping = |ticks: u32| {
            enc(
                0xff,
                0,
                Body::Ping {
                    sdl_ticks: ticks,
                    latencies: vec![0; 8],
                },
            )
        };
        let pong = |ticks: u32| enc(TO_HOST, 3, Body::Pong { sdl_ticks: ticks });
        let mut ws2 = ws.try_clone().unwrap();
        let mut d = Decoder::new(false);

        ws2.write_all(&ws::encode(ws::OP_BINARY, &[0, 0], key))
            .unwrap();
        let mut got = [0u8; 2];
        host.read_exact(&mut got).unwrap();
        host.write_all(&[0x0d, 0x7d]).unwrap();
        let _ = read_frame(&mut ws, &mut d);

        let p0 = ping(1);
        host.write_all(&p0).unwrap();
        let mut seen = Vec::new();
        while seen.len() < p0.len() {
            if let Frame::Data(b) = read_frame(&mut ws, &mut d) {
                seen.extend(b);
            }
        }
        ws2.write_all(&ws::encode(ws::OP_BINARY, &pong(1), key))
            .unwrap();
        let mut buf = vec![0u8; pong(1).len()];
        host.read_exact(&mut buf).unwrap();
        assert_eq!(buf, pong(1));

        for i in 0..ANSWERED_KEPT as u32 {
            let tick = 100 + i;
            host.write_all(&ping(tick)).unwrap();
            let mut got = vec![0u8; pong(tick).len()];
            host.read_exact(&mut got).unwrap();
            assert_eq!(got, pong(tick));
        }
        let stalled_tick = 100 + ANSWERED_KEPT as u32;
        host.write_all(&ping(stalled_tick)).unwrap();
        host.set_read_timeout(Some(Duration::from_millis(150)))
            .unwrap();
        let mut extra = [0u8; 32];
        assert!(
            host.read(&mut extra).is_err(),
            "proxy should stop answering pings once ANSWERED_KEPT go unanswered by the page"
        );

        host.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
        ws2.write_all(&ws::encode(ws::OP_BINARY, &pong(100), key))
            .unwrap();
        let marker = enc(TO_HOST, 3, Body::ServerCommand(9));
        ws2.write_all(&ws::encode(ws::OP_BINARY, &marker, key))
            .unwrap();
        let mut got = vec![0u8; marker.len()];
        host.read_exact(&mut got).unwrap();
        assert_eq!(got, marker);

        let resumed_tick = stalled_tick + 1;
        host.write_all(&ping(resumed_tick)).unwrap();
        let mut got = vec![0u8; pong(resumed_tick).len()];
        host.read_exact(&mut got).unwrap();
        assert_eq!(got, pong(resumed_tick));
    }
}
