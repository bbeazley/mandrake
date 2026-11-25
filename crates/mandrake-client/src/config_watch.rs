use std::path::{Path, PathBuf};
use std::time::Duration;

#[cfg_attr(target_arch = "wasm32", allow(dead_code))]
pub(crate) const INTERVAL: Duration = Duration::from_millis(250);

#[cfg(not(target_arch = "wasm32"))]
type Stamp = Option<(std::time::SystemTime, u64)>;

#[cfg(not(target_arch = "wasm32"))]
fn stamp(path: &Path) -> Stamp {
    let meta = std::fs::metadata(path).ok()?;
    Some((meta.modified().ok()?, meta.len()))
}

#[derive(Debug)]
pub(crate) struct FileWatch {
    path: PathBuf,
    #[cfg(not(target_arch = "wasm32"))]
    seen: Stamp,
    #[cfg(not(target_arch = "wasm32"))]
    pending: Option<Stamp>,
    #[cfg(not(target_arch = "wasm32"))]
    next: std::time::Instant,
}

impl FileWatch {
    pub(crate) fn new(path: PathBuf) -> Option<Self> {
        #[cfg(target_arch = "wasm32")]
        {
            let _ = path;
            None
        }
        #[cfg(not(target_arch = "wasm32"))]
        {
            let seen = stamp(&path);
            Some(Self {
                path,
                seen,
                pending: None,
                next: std::time::Instant::now() + INTERVAL,
            })
        }
    }

    pub(crate) fn path(&self) -> &Path {
        &self.path
    }

    pub(crate) fn poll(&mut self) -> Option<Result<String, String>> {
        #[cfg(target_arch = "wasm32")]
        {
            None
        }
        #[cfg(not(target_arch = "wasm32"))]
        {
            self.poll_at(std::time::Instant::now())
        }
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn poll_at(&mut self, now: std::time::Instant) -> Option<Result<String, String>> {
        if now < self.next {
            return None;
        }
        self.next = now + INTERVAL;
        let current = stamp(&self.path);
        if current == self.seen {
            self.pending = None;
            return None;
        }
        if self.pending != Some(current) {
            self.pending = Some(current);
            return None;
        }
        self.pending = None;
        self.seen = current;
        Some(match current {
            None => Err("the file is gone".to_owned()),
            Some(_) => std::fs::read_to_string(&self.path).map_err(|e| e.to_string()),
        })
    }
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    use super::*;
    use std::time::{Instant, SystemTime};

    #[test]
    fn a_change_is_read_once_it_settles_and_once_only() {
        let dir = std::env::temp_dir().join(format!("m2-config-watch-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("fog.toml");
        let _ = std::fs::remove_file(&path);
        let mut watch = FileWatch::new(path.clone()).unwrap();
        let t = std::cell::Cell::new(Instant::now());
        let look = |watch: &mut FileWatch| {
            t.set(t.get() + INTERVAL);
            watch.poll_at(t.get())
        };
        assert_eq!(look(&mut watch), None, "nothing there, nothing changed");

        std::fs::write(&path, "[default]\nopacity = 0.3\n").unwrap();
        assert_eq!(look(&mut watch), None, "a change waits for a second look");
        assert_eq!(
            look(&mut watch),
            Some(Ok("[default]\nopacity = 0.3\n".to_owned()))
        );
        assert_eq!(look(&mut watch), None, "and is read once");

        std::fs::write(&path, "[default]\nopa").unwrap();
        assert_eq!(look(&mut watch), None);
        let f = std::fs::File::options().append(true).open(&path).unwrap();
        f.set_modified(SystemTime::now() + Duration::from_secs(5))
            .unwrap();
        std::fs::write(&path, "[default]\nopacity = 0.4\n").unwrap();
        f.set_modified(SystemTime::now() + Duration::from_secs(9))
            .unwrap();
        assert_eq!(look(&mut watch), None);
        assert_eq!(
            look(&mut watch),
            Some(Ok("[default]\nopacity = 0.4\n".to_owned()))
        );

        assert_eq!(watch.poll_at(t.get()), None);

        std::fs::remove_file(&path).unwrap();
        assert_eq!(look(&mut watch), None);
        assert_eq!(look(&mut watch), Some(Err("the file is gone".to_owned())));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
