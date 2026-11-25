//! Build script for mandrake-client:
//! 1. Records the git commit for screenshot metadata (`M2_CLIENT_COMMIT`).
//! 2. On Windows targets, embeds `assets/mandrake.ico` into `mandrake.exe`.

use std::env;
use std::path::PathBuf;
use std::process::Command;

fn git(args: &[&str]) -> Option<String> {
    let output = Command::new("git").args(args).output().ok()?;
    if !output.status.success() {
        return None;
    }
    Some(String::from_utf8(output.stdout).ok()?.trim().to_owned())
}

fn embed_windows_icon() {
    println!("cargo:rerun-if-changed=assets/mandrake.rc");
    println!("cargo:rerun-if-changed=assets/mandrake.ico");

    if env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }

    let manifest_dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap());
    let rc_file = manifest_dir.join("assets").join("mandrake.rc");
    let out_dir = PathBuf::from(env::var("OUT_DIR").unwrap());
    let res_file = out_dir.join("mandrake.res");

    let compiled = Command::new("rc.exe")
        .arg("/nologo")
        .arg("/fo")
        .arg(&res_file)
        .arg(&rc_file)
        .status()
        .is_ok_and(|s| s.success())
        || Command::new("llvm-rc")
            .arg("/FO")
            .arg(&res_file)
            .arg(&rc_file)
            .status()
            .is_ok_and(|s| s.success());

    if compiled && res_file.is_file() {
        println!("cargo:rustc-link-arg-bin=mandrake={}", res_file.display());
    }
}

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    // The branch's file moves with each commit, and HEAD with each switch.
    let branch = git(&["symbolic-ref", "-q", "HEAD"]);
    for name in std::iter::once("HEAD").chain(branch.as_deref()) {
        if let Some(path) = git(&["rev-parse", "--git-path", name]) {
            println!("cargo:rerun-if-changed={path}");
        }
    }
    let commit = git(&["rev-parse", "HEAD"]).unwrap_or_else(|| "unknown".to_owned());
    println!("cargo:rustc-env=M2_CLIENT_COMMIT={commit}");

    embed_windows_icon();
}
