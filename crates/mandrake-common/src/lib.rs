//! Myth II common types, error definitions, and platform-agnostic abstractions.

use std::borrow::Cow;
use std::collections::HashMap;
use std::fmt;
use std::fs::File;
use std::io::{self, BufReader, Cursor, Read, Seek};
use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::sync::{Arc, RwLock};
use thiserror::Error;

pub mod data_dirs;
pub mod game_options;
pub mod mac_roman;
pub mod stop;

pub use stop::Stop;

/// Common error types for the Myth II engine.
#[derive(Debug, Error)]
pub enum M2Error {
    #[error("I/O error: {0}")]
    Io(#[from] io::Error),

    #[error("Invalid magic identifier: expected {expected}, found {found}")]
    InvalidMagic { expected: String, found: String },

    #[error("Tag parsing error: {0}")]
    TagParse(String),

    #[error("Replay parsing error: {0}")]
    ReplayParse(String),

    #[error("Simulation error: {0}")]
    Simulation(String),
}

/// Represents a Bungie 4-character code (FourCC), stored as big-endian bytes.
#[derive(
    Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, serde::Serialize, serde::Deserialize,
)]
pub struct FourCC(pub [u8; 4]);

impl FourCC {
    pub const fn new(bytes: [u8; 4]) -> Self {
        Self(bytes)
    }

    pub fn as_str(&self) -> &str {
        std::str::from_utf8(&self.0).unwrap_or("????")
    }

    pub const fn as_bytes(&self) -> &[u8; 4] {
        &self.0
    }

    pub const fn as_u32(&self) -> u32 {
        u32::from_be_bytes(self.0)
    }

    pub const fn from_u32(val: u32) -> Self {
        Self(val.to_be_bytes())
    }
}

impl From<[u8; 4]> for FourCC {
    fn from(bytes: [u8; 4]) -> Self {
        Self(bytes)
    }
}

impl From<&[u8; 4]> for FourCC {
    fn from(bytes: &[u8; 4]) -> Self {
        Self(*bytes)
    }
}

impl TryFrom<&str> for FourCC {
    type Error = M2Error;

    fn try_from(s: &str) -> Result<Self, Self::Error> {
        let bytes = s.as_bytes();
        if bytes.len() != 4 {
            return Err(M2Error::TagParse(format!(
                "FourCC must be exactly 4 bytes, got '{}' (len {})",
                s,
                bytes.len()
            )));
        }
        let mut arr = [0u8; 4];
        arr.copy_from_slice(bytes);
        Ok(Self(arr))
    }
}

impl FromStr for FourCC {
    type Err = M2Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::try_from(s)
    }
}

impl PartialEq<str> for FourCC {
    fn eq(&self, other: &str) -> bool {
        self.as_str() == other
    }
}

impl PartialEq<&str> for FourCC {
    fn eq(&self, other: &&str) -> bool {
        self.as_str() == *other
    }
}

impl FourCC {
    /// The four bytes as text: printable ASCII as is, any other byte as `\xNN`.
    fn write_escaped(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for &b in &self.0 {
            if (0x20..0x7f).contains(&b) {
                write!(f, "{}", b as char)?;
            } else {
                write!(f, "\\x{b:02x}")?;
            }
        }
        Ok(())
    }
}

impl fmt::Debug for FourCC {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("FourCC('")?;
        self.write_escaped(f)?;
        f.write_str("')")
    }
}

impl fmt::Display for FourCC {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.write_escaped(f)
    }
}

/// Abstraction for virtual file access across native filesystem and browser WASM environments.
pub trait VirtualFileSystem: Send + Sync {
    /// Read an entire file as a byte vector.
    fn read_file(&self, path: &str) -> Result<Vec<u8>, M2Error>;

    /// Read an entire file as shared bytes, avoiding a copy when backed by memory.
    fn read_file_shared(&self, path: &str) -> Result<Arc<Vec<u8>>, M2Error> {
        self.read_file(path).map(Arc::new)
    }

    /// Open a readable, seekable stream to a file.
    fn open_reader(&self, path: &str) -> Result<Box<dyn SeekableRead + Send>, M2Error>;

    /// Check if a path exists.
    fn exists(&self, path: &str) -> bool;

    /// Whether the path is a file (not a folder).
    fn is_file(&self, path: &str) -> bool {
        self.exists(path)
    }

    /// The paths of the folder's entries (`dir` joined with each name), in no particular order;
    /// empty when there is no such folder.
    fn list_dir(&self, dir: &str) -> Vec<String>;
}

/// The game folder's files for the whole process: the files the player chose in the
/// browser, natively the disk (a [`NativeVfs`] on the working directory, so a path means what it
/// means to `std::fs`). Set once, before the first read; unset, it is the disk.
static GAME_VFS: std::sync::OnceLock<Box<dyn VirtualFileSystem>> = std::sync::OnceLock::new();

/// The game's files: what [`set_game_vfs`] gave, else the disk.
pub fn game_vfs() -> &'static dyn VirtualFileSystem {
    GAME_VFS
        .get_or_init(|| Box::new(NativeVfs::default_cwd()))
        .as_ref()
}

/// Makes `vfs` the game's files, once and before any read; `Err` with it if they were set already.
pub fn set_game_vfs(vfs: Box<dyn VirtualFileSystem>) -> Result<(), Box<dyn VirtualFileSystem>> {
    GAME_VFS.set(vfs)
}

pub trait SeekableRead: Read + Seek {}
impl<T: Read + Seek> SeekableRead for T {}

/// Native filesystem implementation of [`VirtualFileSystem`].
#[derive(Debug, Clone)]
pub struct NativeVfs {
    base_path: PathBuf,
}

impl NativeVfs {
    /// Create a new `NativeVfs` rooted at `base_path`.
    pub fn new(base_path: impl Into<PathBuf>) -> Self {
        Self {
            base_path: base_path.into(),
        }
    }

    /// Create a `NativeVfs` rooted at the current working directory.
    pub fn default_cwd() -> Self {
        Self {
            base_path: PathBuf::from("."),
        }
    }

    /// Resolve a virtual path against the base path.
    pub fn resolve_path(&self, path: &str) -> PathBuf {
        let p = Path::new(path);
        if p.is_absolute() {
            p.to_path_buf()
        } else {
            self.base_path.join(p)
        }
    }

    pub fn base_path(&self) -> &Path {
        &self.base_path
    }
}

impl Default for NativeVfs {
    fn default() -> Self {
        Self::default_cwd()
    }
}

impl VirtualFileSystem for NativeVfs {
    fn read_file(&self, path: &str) -> Result<Vec<u8>, M2Error> {
        let full_path = self.resolve_path(path);
        std::fs::read(&full_path).map_err(M2Error::Io)
    }

    fn open_reader(&self, path: &str) -> Result<Box<dyn SeekableRead + Send>, M2Error> {
        let full_path = self.resolve_path(path);
        let file = File::open(&full_path).map_err(M2Error::Io)?;
        Ok(Box::new(BufReader::new(file)))
    }

    fn exists(&self, path: &str) -> bool {
        self.resolve_path(path).exists()
    }

    fn is_file(&self, path: &str) -> bool {
        self.resolve_path(path).is_file()
    }

    fn list_dir(&self, dir: &str) -> Vec<String> {
        let Ok(read) = std::fs::read_dir(self.resolve_path(dir)) else {
            return Vec::new();
        };
        read.filter_map(|e| e.ok())
            .map(|e| {
                Path::new(dir)
                    .join(e.file_name())
                    .to_string_lossy()
                    .into_owned()
            })
            .collect()
    }
}

/// In-memory implementation of [`VirtualFileSystem`] for testing and WASM.
#[derive(Debug, Clone, Default)]
pub struct MemoryVfs {
    /// Shared, so that inserting or reading a large archive doesn't copy it.
    files: Arc<RwLock<HashMap<String, Arc<Vec<u8>>>>>,
}

/// A reader's view of a file of a [`MemoryVfs`], without a copy.
#[derive(Debug, Clone)]
struct SharedBytes(Arc<Vec<u8>>);

impl AsRef<[u8]> for SharedBytes {
    fn as_ref(&self) -> &[u8] {
        self.0.as_slice()
    }
}

impl MemoryVfs {
    /// Create a new empty in-memory VFS.
    pub fn new() -> Self {
        Self {
            files: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Insert or replace a virtual file.
    pub fn insert(&self, path: impl Into<String>, data: Vec<u8>) {
        let mut lock = self.files.write().expect("MemoryVfs lock poisoned");
        lock.insert(Self::normalize_path(&path.into()), Arc::new(data));
    }

    /// Remove a file from the VFS.
    pub fn remove(&self, path: &str) -> Option<Vec<u8>> {
        let mut lock = self.files.write().expect("MemoryVfs lock poisoned");
        let key = Self::normalize_path_cow(path);
        lock.remove(key.as_ref())
            .map(|data| Arc::try_unwrap(data).unwrap_or_else(|arc| (*arc).clone()))
    }

    /// The bytes of a file, shared rather than copied.
    fn shared(&self, path: &str) -> Result<Arc<Vec<u8>>, M2Error> {
        let lock = self.files.read().expect("MemoryVfs lock poisoned");
        let key = Self::normalize_path_cow(path);
        lock.get(key.as_ref()).cloned().ok_or_else(|| {
            M2Error::Io(io::Error::new(
                io::ErrorKind::NotFound,
                format!("File not found in MemoryVfs: {path}"),
            ))
        })
    }

    fn normalize_path_cow(path: &str) -> Cow<'_, str> {
        if path.contains('\\') {
            Cow::Owned(path.replace('\\', "/").trim_start_matches('/').to_string())
        } else {
            Cow::Borrowed(path.trim_start_matches('/'))
        }
    }

    /// Normalize paths to simplify lookups.
    pub fn normalize_path(path: &str) -> String {
        Self::normalize_path_cow(path).into_owned()
    }
}

impl VirtualFileSystem for MemoryVfs {
    fn read_file(&self, path: &str) -> Result<Vec<u8>, M2Error> {
        Ok(self.shared(path)?.as_ref().clone())
    }

    fn read_file_shared(&self, path: &str) -> Result<Arc<Vec<u8>>, M2Error> {
        self.shared(path)
    }

    fn open_reader(&self, path: &str) -> Result<Box<dyn SeekableRead + Send>, M2Error> {
        Ok(Box::new(Cursor::new(SharedBytes(self.shared(path)?))))
    }

    fn exists(&self, path: &str) -> bool {
        let normalized = Self::normalize_path_cow(path);
        let lock = self.files.read().expect("MemoryVfs lock poisoned");
        lock.contains_key(normalized.as_ref())
            || lock.keys().any(|k| {
                k.strip_prefix(normalized.as_ref())
                    .is_some_and(|r| r.starts_with('/'))
            })
    }

    fn is_file(&self, path: &str) -> bool {
        let normalized = Self::normalize_path_cow(path);
        let lock = self.files.read().expect("MemoryVfs lock poisoned");
        lock.contains_key(normalized.as_ref())
    }

    /// Only the files directly in `dir`: a folder is a name with files under it.
    /// The paths keep `dir` as it was given.
    fn list_dir(&self, dir: &str) -> Vec<String> {
        let normalized = Self::normalize_path_cow(dir);
        let key = normalized.trim_end_matches('/');
        let given = dir.trim_end_matches(['/', '\\']);
        let leading_slash = dir.starts_with('/') || dir.starts_with('\\');
        let lock = self.files.read().expect("MemoryVfs lock poisoned");
        let mut names: Vec<String> = lock
            .keys()
            .filter_map(|k| {
                let rest = if key.is_empty() {
                    k.as_str()
                } else {
                    k.strip_prefix(key)?.strip_prefix('/')?
                };
                let name = rest.split('/').next().unwrap_or(rest);
                if name.is_empty() {
                    return None;
                }
                Some(if given.is_empty() {
                    if leading_slash {
                        format!("/{name}")
                    } else {
                        name.to_string()
                    }
                } else {
                    format!("{given}/{name}")
                })
            })
            .collect();
        names.sort();
        names.dedup();
        names
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Non-printing bytes print escaped, so a stop message never shows raw NULs.
    #[test]
    fn fourcc_prints_non_printing_bytes_escaped() {
        assert_eq!(format!("{}", FourCC(*b"scen")), "scen");
        assert_eq!(format!("{:?}", FourCC(*b"scen")), "FourCC('scen')");
        let f = FourCC([0, 0, 0, 0x20]);
        assert_eq!(format!("{f}"), "\\x00\\x00\\x00 ");
        assert_eq!(format!("{f:?}"), "FourCC('\\x00\\x00\\x00 ')");
        assert_eq!(
            format!("{}", FourCC([0xff, b'a', 0x7f, b'b'])),
            "\\xffa\\x7fb"
        );
    }

    /// A folder is the names with files under it; `is_file` only the files themselves.
    #[test]
    fn memory_vfs_lists_a_folder_and_shares_its_bytes() {
        let vfs = MemoryVfs::new();
        vfs.insert("/game/tags/small install", vec![1, 2, 3]);
        vfs.insert("/game/plugins/Patch 1.2", vec![4]);
        vfs.insert("/game/plugins/deep/inner", vec![5]);
        assert_eq!(
            vfs.list_dir("/game/plugins"),
            vec!["/game/plugins/Patch 1.2", "/game/plugins/deep"]
        );
        assert_eq!(vfs.list_dir(""), vec!["game"]);
        assert_eq!(vfs.list_dir("/"), vec!["/game"]);
        assert!(vfs.list_dir("/game/none").is_empty());
        assert!(vfs.is_file("/game/tags/small install"));
        assert!(!vfs.is_file("/game/tags"));
        assert!(vfs.exists("/game/tags"));
        let shared = vfs.read_file_shared("game/tags/small install").unwrap();
        assert_eq!(shared.as_slice(), [1, 2, 3]);
        let mut reader = vfs.open_reader("game/tags/small install").unwrap();
        let mut bytes = Vec::new();
        reader.read_to_end(&mut bytes).unwrap();
        assert_eq!(bytes, [1, 2, 3]);
    }

    #[test]
    fn test_fourcc() {
        let tag = FourCC::new(*b"mth2");
        assert_eq!(tag.to_string(), "mth2");
        assert_eq!(tag.as_str(), "mth2");
        assert_eq!(tag.as_bytes(), b"mth2");
        assert_eq!(tag, "mth2");

        let parsed: FourCC = "mesh".parse().unwrap();
        assert_eq!(parsed, "mesh");
        assert_eq!(parsed.as_u32(), u32::from_be_bytes(*b"mesh"));
        assert_eq!(FourCC::from_u32(parsed.as_u32()), parsed);

        assert!("toolong".parse::<FourCC>().is_err());
    }

    #[test]
    fn test_memory_vfs() {
        let vfs = MemoryVfs::new();
        assert!(!vfs.exists("tags/small install"));

        let sample_data = b"Bungie Myth II archive test payload".to_vec();
        vfs.insert("tags/small install", sample_data.clone());

        assert!(vfs.exists("tags/small install"));
        assert!(vfs.exists("/tags/small install"));

        let read_back = vfs.read_file("tags/small install").unwrap();
        assert_eq!(read_back, sample_data);

        let mut reader = vfs.open_reader("tags/small install").unwrap();
        let mut buf = Vec::new();
        reader.read_to_end(&mut buf).unwrap();
        assert_eq!(buf, sample_data);

        assert!(vfs.read_file("nonexistent.txt").is_err());
    }

    #[test]
    fn test_native_vfs() {
        let vfs = NativeVfs::new(".");
        assert!(vfs.exists("Cargo.toml"));

        let cargo_toml = vfs.read_file("Cargo.toml").unwrap();
        assert!(String::from_utf8_lossy(&cargo_toml).contains("mandrake-common"));

        let mut reader = vfs.open_reader("Cargo.toml").unwrap();
        let mut line = [0u8; 9];
        reader.read_exact(&mut line).unwrap();
        assert_eq!(&line, b"[package]");

        assert!(!vfs.exists("nonexistent_random_file_12345.bin"));
    }
}
