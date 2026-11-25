use mandrake_common::{SeekableRead, game_vfs};
use std::io;
use std::path::{Path, PathBuf};

fn text(path: &Path) -> std::borrow::Cow<'_, str> {
    path.to_string_lossy()
}

fn io_error(e: mandrake_common::M2Error) -> io::Error {
    match e {
        mandrake_common::M2Error::Io(e) => e,
        other => io::Error::other(other.to_string()),
    }
}

pub(crate) fn read(path: &Path) -> io::Result<Vec<u8>> {
    game_vfs().read_file(&text(path)).map_err(io_error)
}

pub(crate) fn open(path: &Path) -> io::Result<Box<dyn SeekableRead + Send>> {
    game_vfs().open_reader(&text(path)).map_err(io_error)
}

pub(crate) fn exists(path: &Path) -> bool {
    game_vfs().exists(&text(path))
}

pub(crate) fn is_file(path: &Path) -> bool {
    game_vfs().is_file(&text(path))
}

pub(crate) fn list_dir(dir: &Path) -> Vec<PathBuf> {
    game_vfs()
        .list_dir(&text(dir))
        .into_iter()
        .map(PathBuf::from)
        .collect()
}
