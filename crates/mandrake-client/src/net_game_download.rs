#[cfg(not(target_arch = "wasm32"))]
use std::fs::OpenOptions;
#[cfg(not(target_arch = "wasm32"))]
use std::fs::{self, File};
#[cfg(not(target_arch = "wasm32"))]
use std::io::{Read, Write};
#[cfg(not(target_arch = "wasm32"))]
use std::path::Path;
use std::path::PathBuf;
#[cfg(not(target_arch = "wasm32"))]
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, mpsc};
#[cfg(not(target_arch = "wasm32"))]
use std::time::Duration;

use mandrake_session::net::{DownloadProgress, Plugin};
#[cfg(not(target_arch = "wasm32"))]
use mandrake_tag::header::SBMonoHeader;
#[cfg(not(target_arch = "wasm32"))]
use tracing::{info, warn};

#[cfg(not(target_arch = "wasm32"))]
static ONE_AT_A_TIME: Mutex<()> = Mutex::new(());

enum Message {
    #[cfg_attr(target_arch = "wasm32", allow(dead_code))]
    Progress(DownloadProgress),
    Done(Result<(), String>),
}

pub(crate) struct Download {
    cancel: Arc<AtomicBool>,
    messages: mpsc::Receiver<Message>,
    done: bool,
}

#[derive(Debug, Default)]
pub(crate) struct Polled {
    pub(crate) progress: Option<DownloadProgress>,
    pub(crate) done: Option<Result<(), String>>,
}

impl Download {
    pub(crate) fn start(
        base: &str,
        host: &str,
        plugins: Vec<Plugin>,
        dir: PathBuf,
        max_bytes: u64,
    ) -> Download {
        let base = base.replace("{host}", host);
        let cancel = Arc::new(AtomicBool::new(false));
        let (send, messages) = mpsc::channel();
        let flag = cancel.clone();
        #[cfg(target_arch = "wasm32")]
        {
            let _ = (&base, &plugins, &dir, max_bytes, &flag);
            let _ = send.send(Message::Done(Err(
                "no plugin downloads in the browser".into()
            )));
        }
        #[cfg(not(target_arch = "wasm32"))]
        std::thread::spawn(move || {
            let _one = ONE_AT_A_TIME.lock().unwrap_or_else(|e| e.into_inner());
            let progress = |p| {
                let _ = send.send(Message::Progress(p));
            };
            let result = download_all(&base, &plugins, &dir, max_bytes, &flag, &progress);
            let _ = send.send(Message::Done(result));
        });
        Download {
            cancel,
            messages,
            done: false,
        }
    }

    pub(crate) fn poll(&mut self) -> Polled {
        let mut out = Polled::default();
        loop {
            match self.messages.try_recv() {
                Ok(Message::Progress(p)) => out.progress = Some(p),
                Ok(Message::Done(r)) => {
                    self.done = true;
                    out.done = Some(r);
                }
                Err(mpsc::TryRecvError::Empty) => break,
                Err(mpsc::TryRecvError::Disconnected) => {
                    if !self.done {
                        self.done = true;
                        out.done = Some(Err("the download's thread ended".into()));
                    }
                    break;
                }
            }
        }
        out
    }

    pub(crate) fn cancel(&self) {
        self.cancel.store(true, Ordering::SeqCst);
    }
}

impl Drop for Download {
    fn drop(&mut self) {
        self.cancel();
    }
}

pub(crate) fn host_of(address: &str) -> &str {
    let address = address.trim();
    if address.starts_with('[') {
        return address.find(']').map_or(address, |end| &address[..=end]);
    }
    match address.rsplit_once(':') {
        Some((host, port)) if !port.is_empty() && port.bytes().all(|b| b.is_ascii_digit()) => host,
        _ => address,
    }
}

#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn plain_file_name(name: &[u8]) -> Option<&str> {
    let name = std::str::from_utf8(name).ok()?;
    let plain = !name.is_empty()
        && name.len() <= 31
        && !name.starts_with('.')
        && !name.ends_with('.')
        && !name.ends_with(' ')
        && !name.starts_with(' ')
        && !name.chars().any(|c| {
            c.is_control() || matches!(c, '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|')
        });
    plain.then_some(name)
}

#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn plugin_url(base: &str, checksum: u32) -> String {
    format!("{base}{checksum:08x}")
}

#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn check_plugin_file(path: &Path, checksum: u32, max_bytes: u64) -> Result<(), String> {
    let len = fs::metadata(path).map_err(|e| e.to_string())?.len();
    if len > max_bytes {
        return Err(format!("{len} bytes, more than the cap of {max_bytes}"));
    }
    let mut f = File::open(path).map_err(|e| e.to_string())?;
    let h = SBMonoHeader::read(&mut f).map_err(|e| format!("no tag-file header: {e}"))?;
    if h.r#type != mandrake_tag::tag_files::TYPE_PLUGIN {
        return Err(format!("a file of type {}, not a plugin", h.r#type));
    }
    if h.signature != SBMonoHeader::SIG_DNG2 && h.signature != SBMonoHeader::SIG_MTH2 {
        return Err("no tag-file signature".into());
    }
    let theirs = u32::from_be_bytes(h.checksum);
    if theirs != checksum {
        return Err(format!(
            "checksum {theirs:08x}, the game's is {checksum:08x}"
        ));
    }
    if u64::from(h.size) != len {
        return Err(format!("{len} bytes, its header says {}", h.size));
    }
    Ok(())
}

#[cfg(not(target_arch = "wasm32"))]
fn agent() -> ureq::Agent {
    ureq::Agent::config_builder()
        .http_status_as_error(false)
        .max_redirects(0)
        .timeout_connect(Some(Duration::from_secs(10)))
        .timeout_recv_response(Some(Duration::from_secs(15)))
        .timeout_recv_body(Some(Duration::from_secs(30)))
        .user_agent("mandrake")
        .build()
        .into()
}

#[cfg(not(target_arch = "wasm32"))]
fn content_length(r: &ureq::http::Response<ureq::Body>) -> Option<u64> {
    r.headers()
        .get("content-length")?
        .to_str()
        .ok()?
        .trim()
        .parse()
        .ok()
}

#[cfg(not(target_arch = "wasm32"))]
fn check_expected_length(received: u64, len: u64, max_bytes: u64) -> Result<(), String> {
    let expected = received.saturating_add(len);
    if expected > max_bytes {
        return Err(format!(
            "{expected} bytes, more than the cap of {max_bytes}"
        ));
    }
    Ok(())
}

#[cfg(not(target_arch = "wasm32"))]
fn download_all(
    base: &str,
    plugins: &[Plugin],
    dir: &Path,
    max_bytes: u64,
    cancel: &AtomicBool,
    progress: &dyn Fn(DownloadProgress),
) -> Result<(), String> {
    let agent = agent();
    fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let mut names = Vec::new();
    for p in plugins {
        let name = plain_file_name(&p.name).ok_or_else(|| {
            format!(
                "{:?} is not a plain file name",
                String::from_utf8_lossy(&p.name)
            )
        })?;
        names.push(name);
    }
    let mut sizes = Vec::new();
    for p in plugins {
        if cancel.load(Ordering::SeqCst) {
            return Err("cancelled".into());
        }
        let size = agent
            .head(plugin_url(base, p.checksum))
            .call()
            .ok()
            .filter(|r| r.status() == 200)
            .and_then(|r| content_length(&r))
            .unwrap_or(0);
        if size > max_bytes {
            return Err(format!(
                "{}: {size} bytes, more than the cap of {max_bytes}",
                String::from_utf8_lossy(&p.name)
            ));
        }
        sizes.push(size);
    }
    let total: u64 = sizes.iter().sum();
    let mut before = 0;
    for (index, (p, name)) in plugins.iter().zip(&names).enumerate() {
        let url = plugin_url(base, p.checksum);
        let part = dir.join(format!("{:08x}.part", p.checksum));
        info!("download: {name} from {url}");
        let report = |received: u64| {
            progress(DownloadProgress {
                index,
                received: before + received,
                total,
            })
        };
        report(0);
        let got = fetch(&agent, &url, &part, max_bytes, cancel, &report);
        let checked = got.and_then(|()| check_plugin_file(&part, p.checksum, max_bytes));
        match checked {
            Ok(()) => {
                let to = dir.join(name);
                fs::rename(&part, &to).map_err(|e| format!("{}: {e}", to.display()))?;
                info!("download: {name} is in, {}", to.display());
            }
            Err(e) if cancel.load(Ordering::SeqCst) => return Err(e),
            Err(e) => {
                warn!("download: {name}: {e}; deleted");
                let _ = fs::remove_file(&part);
                return Err(format!("{name}: {e}"));
            }
        }
        before += sizes[index].max(fs::metadata(dir.join(name)).map_or(0, |m| m.len()));
    }
    Ok(())
}

#[cfg(not(target_arch = "wasm32"))]
fn fetch(
    agent: &ureq::Agent,
    url: &str,
    part: &Path,
    max_bytes: u64,
    cancel: &AtomicBool,
    report: &dyn Fn(u64),
) -> Result<(), String> {
    let have = fs::metadata(part).map_or(0, |m| m.len());
    let mut request = agent.get(url);
    if have > 0 {
        request = request.header("Range", format!("bytes={have}-"));
    }
    let mut response = request.call().map_err(|e| e.to_string())?;
    let (mut file, mut received) = match response.status().as_u16() {
        206 if have > 0 => {
            info!("download: resumed at {have} bytes");
            let f = OpenOptions::new()
                .append(true)
                .open(part)
                .map_err(|e| e.to_string())?;
            (f, have)
        }
        200 => (File::create(part).map_err(|e| e.to_string())?, 0),
        416 if have > 0 => return Ok(()),
        s => return Err(format!("the server answered {s}")),
    };
    if let Some(len) = content_length(&response) {
        check_expected_length(received, len, max_bytes)?;
    }
    let mut body = response.body_mut().with_config().limit(u64::MAX).reader();
    let mut buffer = vec![0u8; 64 * 1024];
    report(received);
    loop {
        if cancel.load(Ordering::SeqCst) {
            return Err("cancelled".into());
        }
        let n = body.read(&mut buffer).map_err(|e| e.to_string())?;
        if n == 0 {
            break;
        }
        received += n as u64;
        if received > max_bytes {
            return Err(format!("more than the cap of {max_bytes} bytes"));
        }
        file.write_all(&buffer[..n]).map_err(|e| e.to_string())?;
        report(received);
    }
    file.sync_all().map_err(|e| e.to_string())?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_names_only() {
        for good in [
            "Creep MWC2020 v1.1",
            "JG's Tower Defense B12",
            "Patch 1.8.5 Maps",
        ] {
            assert_eq!(plain_file_name(good.as_bytes()), Some(good));
        }
        for bad in [
            "",
            ".",
            "..",
            "../x",
            "a/b",
            "a\\b",
            "/etc/passwd",
            "C:x",
            "x\0y",
            "x\ny",
            ".hidden",
            "trailing.",
            "a name that is longer than 31 bytes",
        ] {
            assert_eq!(plain_file_name(bad.as_bytes()), None, "{bad:?}");
        }
        assert_eq!(plain_file_name(b"\xff\xfe"), None);
    }

    #[test]
    fn the_host_of_a_join_address() {
        assert_eq!(host_of("myth.example:3453"), "myth.example");
        assert_eq!(host_of("127.0.0.1:3453"), "127.0.0.1");
        assert_eq!(host_of("[::1]:3453"), "[::1]");
        assert_eq!(host_of("myth.example"), "myth.example");
        assert_eq!(
            plugin_url("http://myth.example:8080/plugins/", 0x0bad_f00d),
            "http://myth.example:8080/plugins/0badf00d"
        );
    }

    #[test]
    fn check_expected_length_saturates_on_overflow() {
        assert!(check_expected_length(100, u64::MAX, 1 << 20).is_err());
        assert!(check_expected_length(u64::MAX, 1, 1 << 20).is_err());
        assert!(check_expected_length(512, 512, 1024).is_ok());
        assert!(check_expected_length(512, 513, 1024).is_err());
    }
}
