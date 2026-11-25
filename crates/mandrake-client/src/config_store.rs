use std::io;
use std::path::Path;

pub fn read_to_string(path: &Path) -> io::Result<String> {
    imp::read_to_string(path)
}

pub fn write(path: &Path, text: &str) -> io::Result<()> {
    if let Some(dir) = path.parent().filter(|d| !d.as_os_str().is_empty()) {
        imp::create_dir_all(dir)?;
    }
    imp::write(path, text)
}

pub fn write_replacing(path: &Path, temp: &str, text: &str) -> io::Result<()> {
    #[cfg(target_arch = "wasm32")]
    {
        let _ = temp;
        write(path, text)
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        let temp = path.with_extension(temp);
        write(&temp, text)?;
        imp::rename(&temp, path)
    }
}

pub fn write_bytes(path: &Path, data: &[u8]) -> io::Result<()> {
    if let Some(dir) = path.parent().filter(|d| !d.as_os_str().is_empty()) {
        imp::create_dir_all(dir)?;
    }
    imp::write_bytes(path, data)
}

pub fn write_bytes_replacing(path: &Path, temp: &str, data: &[u8]) -> io::Result<()> {
    #[cfg(target_arch = "wasm32")]
    {
        let _ = temp;
        write_bytes(path, data)
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        let mut name = path.as_os_str().to_owned();
        name.push(temp);
        let temp = std::path::PathBuf::from(name);
        write_bytes(&temp, data)?;
        imp::rename(&temp, path)
    }
}

pub fn is_file(path: &Path) -> bool {
    imp::is_file(path)
}

#[cfg(target_arch = "wasm32")]
pub type GameFilesSink = fn(&str, Option<Vec<u8>>);

#[cfg(target_arch = "wasm32")]
pub fn mirror_into_game_files(dir: &Path, sink: GameFilesSink) {
    imp::mirror_into_game_files(dir, sink);
}

#[cfg(not(target_arch = "wasm32"))]
mod imp {
    use std::io;
    use std::path::Path;

    pub fn read_to_string(path: &Path) -> io::Result<String> {
        std::fs::read_to_string(path)
    }

    pub fn create_dir_all(dir: &Path) -> io::Result<()> {
        std::fs::create_dir_all(dir)
    }

    pub fn write(path: &Path, text: &str) -> io::Result<()> {
        std::fs::write(path, text)
    }

    pub fn write_bytes(path: &Path, data: &[u8]) -> io::Result<()> {
        std::fs::write(path, data)
    }

    pub fn rename(from: &Path, to: &Path) -> io::Result<()> {
        std::fs::rename(from, to)
    }

    pub fn is_file(path: &Path) -> bool {
        path.is_file()
    }
}

#[cfg(target_arch = "wasm32")]
mod imp {
    use std::io;
    use std::path::Path;

    fn storage() -> io::Result<web_sys::Storage> {
        web_sys::window()
            .and_then(|w| w.local_storage().ok().flatten())
            .ok_or_else(|| io::Error::new(io::ErrorKind::Unsupported, "no localStorage"))
    }

    fn key(path: &Path) -> String {
        path.to_string_lossy().replace('\\', "/")
    }

    fn js_error(e: web_sys::wasm_bindgen::JsValue) -> io::Error {
        io::Error::other(format!("localStorage: {e:?}"))
    }

    pub fn read_to_string(path: &Path) -> io::Result<String> {
        storage()?
            .get_item(&key(path))
            .map_err(js_error)?
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, key(path)))
    }

    pub fn create_dir_all(_dir: &Path) -> io::Result<()> {
        Ok(())
    }

    pub fn write(path: &Path, text: &str) -> io::Result<()> {
        storage()?.set_item(&key(path), text).map_err(js_error)
    }

    pub fn write_bytes(path: &Path, data: &[u8]) -> io::Result<()> {
        write(
            path,
            &data.iter().map(|&b| char::from(b)).collect::<String>(),
        )?;
        mirror(&key(path), Some(data.to_vec()));
        Ok(())
    }

    fn bytes_of(text: &str) -> Vec<u8> {
        text.chars().map(|c| c as u32 as u8).collect()
    }

    static MIRROR: std::sync::OnceLock<(String, super::GameFilesSink)> = std::sync::OnceLock::new();

    fn mirror(key: &str, data: Option<Vec<u8>>) {
        if let Some((dir, sink)) = MIRROR.get()
            && key
                .strip_prefix(dir.as_str())
                .is_some_and(|name| !name.is_empty() && !name.contains('/'))
        {
            sink(key, data);
        }
    }

    pub fn mirror_into_game_files(dir: &Path, sink: super::GameFilesSink) {
        let dir = format!("{}/", key(dir).trim_end_matches('/'));
        if MIRROR.set((dir, sink)).is_err() {
            return;
        }
        let Ok(storage) = storage() else {
            return;
        };
        let keys: Vec<String> = (0..storage.length().unwrap_or(0))
            .filter_map(|i| storage.key(i).ok().flatten())
            .collect();
        for k in keys {
            if let Ok(Some(text)) = storage.get_item(&k) {
                mirror(&k, Some(bytes_of(&text)));
            }
        }
    }

    pub fn is_file(path: &Path) -> bool {
        storage()
            .ok()
            .and_then(|s| s.get_item(&key(path)).ok().flatten())
            .is_some()
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn a_file_of_bytes_is_written_whole_and_read_back() {
        let dir = std::env::temp_dir().join(format!("m2-config-store-{}", std::process::id()));
        let path = dir.join("tfl").join("RECO").join("LAST RECORDING");
        let data: Vec<u8> = (0..=255u8).chain([0, 0xff, 0x80]).collect();
        super::write_bytes_replacing(&path, ".tmp", &data).unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), data);
        assert!(!dir.join("tfl/RECO/LAST RECORDING.tmp").exists());
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
