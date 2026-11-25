//! The version the landing page shows: the commit the web client is built from and its date.
//! `M2_REVISION` (the image's build argument, when the build context has no git) wins over git.

use std::path::Path;
use std::process::Command;

fn git(args: &[&str]) -> Option<String> {
    let output = Command::new("git").args(args).output().ok()?;
    if !output.status.success() {
        return None;
    }
    Some(String::from_utf8(output.stdout).ok()?.trim().to_owned())
}

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-env-changed=M2_REVISION");
    println!("cargo:rerun-if-env-changed=M2_REVISION_DATE");
    // Watch HEAD, the branch's loose ref if present, and packed-refs (after git pack-refs/gc).
    let branch = git(&["symbolic-ref", "-q", "HEAD"]);
    for name in ["HEAD", "packed-refs"].into_iter().chain(branch.as_deref()) {
        if let Some(path) = git(&["rev-parse", "--git-path", name])
            && Path::new(&path).exists()
        {
            println!("cargo:rerun-if-changed={path}");
        }
    }
    let given = |name: &str| {
        std::env::var(name)
            .ok()
            .filter(|v| !v.is_empty() && v != "unknown")
    };
    let revision = given("M2_REVISION")
        .or_else(|| git(&["rev-parse", "--short", "HEAD"]))
        .unwrap_or_else(|| "unknown".to_owned());
    let date = given("M2_REVISION_DATE")
        .or_else(|| git(&["log", "-1", "--format=%cs"]))
        .unwrap_or_default();
    println!("cargo:rustc-env=M2_WEB_REVISION={revision}");
    println!("cargo:rustc-env=M2_WEB_REVISION_DATE={date}");
}
