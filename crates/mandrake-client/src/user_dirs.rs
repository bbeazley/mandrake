use std::ffi::OsString;
use std::path::PathBuf;

pub const APP_DIR: &str = "myth2-sourceport";

pub fn config_base_from(env: impl Fn(&str) -> Option<OsString>, windows: bool) -> Option<PathBuf> {
    env("XDG_CONFIG_HOME")
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .or_else(|| {
            env("HOME")
                .filter(|h| !(windows && h.is_empty()))
                .map(|h| PathBuf::from(h).join(".config"))
        })
        .or_else(|| {
            windows
                .then(|| env("APPDATA"))
                .flatten()
                .filter(|v| !v.is_empty())
                .map(PathBuf::from)
        })
}

pub fn pictures_home_from(
    env: impl Fn(&str) -> Option<OsString>,
    windows: bool,
) -> Option<OsString> {
    let set = |name: &str| env(name).filter(|v| !v.is_empty());
    set("HOME").or_else(|| windows.then(|| set("USERPROFILE")).flatten())
}

pub fn cache_base_from(env: impl Fn(&str) -> Option<OsString>, windows: bool) -> Option<PathBuf> {
    let set = |name: &str| env(name).filter(|v| !v.is_empty());
    set("XDG_CACHE_HOME")
        .map(PathBuf::from)
        .or_else(|| set("HOME").map(|h| PathBuf::from(h).join(".cache")))
        .or_else(|| {
            windows
                .then(|| {
                    set("LOCALAPPDATA").map(PathBuf::from).or_else(|| {
                        set("USERPROFILE").map(|u| PathBuf::from(u).join("AppData").join("Local"))
                    })
                })
                .flatten()
        })
}

pub fn cache_base() -> Option<PathBuf> {
    if cfg!(target_arch = "wasm32") {
        return None;
    }
    cache_base_from(real_env, cfg!(windows))
}

fn real_env(name: &str) -> Option<OsString> {
    std::env::var_os(name)
}

pub fn config_base() -> Option<PathBuf> {
    if cfg!(target_arch = "wasm32") {
        return Some(PathBuf::from(WEB_CONFIG_BASE));
    }
    if cfg!(test) {
        return Some(
            std::env::temp_dir().join(format!("{TEST_CONFIG_BASE}-{}", std::process::id())),
        );
    }
    config_base_from(real_env, cfg!(windows))
}

pub const TEST_CONFIG_BASE: &str = "mandrake-client-test-config";

#[cfg(test)]
pub fn config_dir_from(env: impl Fn(&str) -> Option<OsString>, windows: bool) -> Option<PathBuf> {
    Some(config_base_from(env, windows)?.join(APP_DIR))
}

pub const WEB_CONFIG_BASE: &str = "/config";

pub fn config_file(name: &str) -> Option<PathBuf> {
    Some(config_dir()?.join(name))
}

pub fn config_dir() -> Option<PathBuf> {
    Some(config_base()?.join(APP_DIR))
}

pub fn pictures_home() -> Option<String> {
    pictures_home_from(real_env, cfg!(windows)).and_then(|h| h.into_string().ok())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn env<'a>(vars: &'a [(&'a str, &'a str)]) -> impl Fn(&str) -> Option<OsString> + 'a {
        move |name| {
            vars.iter()
                .find(|(k, _)| *k == name)
                .map(|(_, v)| OsString::from(v))
        }
    }

    fn base(vars: &[(&str, &str)], windows: bool) -> Option<PathBuf> {
        config_base_from(env(vars), windows)
    }

    #[test]
    fn config_base_order_per_os() {
        let all = [
            ("XDG_CONFIG_HOME", "/x"),
            ("HOME", "/h"),
            ("APPDATA", r"C:\Users\b\AppData\Roaming"),
        ];
        for windows in [false, true] {
            assert_eq!(base(&all, windows), Some(PathBuf::from("/x")));
            let no_xdg = [("XDG_CONFIG_HOME", ""), all[1], all[2]];
            assert_eq!(base(&no_xdg, windows), Some(PathBuf::from("/h/.config")));
            assert_eq!(base(&[], windows), None);
        }
        let appdata_only = [all[2]];
        assert_eq!(base(&appdata_only, false), None);
        assert_eq!(
            base(&appdata_only, true),
            Some(PathBuf::from(r"C:\Users\b\AppData\Roaming"))
        );
        assert_eq!(base(&[("APPDATA", "")], true), None);
        let empty_home = [("HOME", ""), all[2]];
        assert_eq!(base(&empty_home, false), Some(PathBuf::from(".config")));
        assert_eq!(
            base(&empty_home, true),
            Some(PathBuf::from(r"C:\Users\b\AppData\Roaming"))
        );
    }

    #[test]
    fn cache_base_order_per_os() {
        let cache = |vars: &[(&str, &str)], windows: bool| cache_base_from(env(vars), windows);
        assert_eq!(
            cache(&[("XDG_CACHE_HOME", "/c"), ("HOME", "/h")], false),
            Some(PathBuf::from("/c"))
        );
        assert_eq!(
            cache(&[("XDG_CACHE_HOME", ""), ("HOME", "/h")], false),
            Some(PathBuf::from("/h/.cache"))
        );
        assert_eq!(cache(&[("XDG_CACHE_HOME", "")], false), None);
        assert_eq!(
            cache(&[("LOCALAPPDATA", r"C:\Users\b\AppData\Local")], false),
            None
        );
        assert_eq!(
            cache(&[("LOCALAPPDATA", r"C:\Users\b\AppData\Local")], true),
            Some(PathBuf::from(r"C:\Users\b\AppData\Local"))
        );
        assert_eq!(
            cache(&[("USERPROFILE", r"C:\Users\b")], true),
            Some(PathBuf::from(r"C:\Users\b").join("AppData").join("Local"))
        );
    }

    #[test]
    fn a_tests_config_folder_is_its_own() {
        let dir = config_dir().unwrap();
        assert!(dir.starts_with(std::env::temp_dir()), "{}", dir.display());
        assert!(
            dir.parent()
                .and_then(|b| b.file_name())
                .is_some_and(|n| n.to_string_lossy().starts_with(TEST_CONFIG_BASE)),
            "{}",
            dir.display()
        );
        let real = config_dir_from(real_env, cfg!(windows));
        assert_ne!(Some(dir), real);
    }

    #[test]
    fn pictures_home_order_per_os() {
        let home = |vars: &[(&str, &str)], windows| pictures_home_from(env(vars), windows);
        let both = [("HOME", "/h"), ("USERPROFILE", r"C:\Users\b")];
        assert_eq!(home(&both, false), Some("/h".into()));
        assert_eq!(home(&both, true), Some("/h".into()));
        let profile = [("HOME", ""), both[1]];
        assert_eq!(home(&profile, false), None);
        assert_eq!(home(&profile, true), Some(r"C:\Users\b".into()));
        assert_eq!(home(&[], true), None);
    }
}
