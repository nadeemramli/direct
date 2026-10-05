//! Stamp the binary with the source it was built from (DIR-82).
//!
//! The commit and a dirty flag come from git at build time. A tree with any
//! uncommitted change is stamped dirty; a build without git is stamped
//! unknown. `DIRECT_BUILD_COMMIT` / `DIRECT_BUILD_DIRTY` override both, for
//! packaging from an exported tree.
use std::process::Command;

fn git(args: &[&str]) -> Option<String> {
    let out = Command::new("git").args(args).output().ok()?;
    out.status
        .success()
        .then(|| String::from_utf8_lossy(&out.stdout).trim().to_string())
}

fn main() {
    println!("cargo:rerun-if-env-changed=DIRECT_BUILD_COMMIT");
    println!("cargo:rerun-if-env-changed=DIRECT_BUILD_DIRTY");
    // Rebuild the stamp when HEAD moves or the index changes.
    if let Some(dir) = git(&["rev-parse", "--git-dir"]) {
        println!("cargo:rerun-if-changed={dir}/HEAD");
        println!("cargo:rerun-if-changed={dir}/index");
        if let Some(head) = git(&["symbolic-ref", "-q", "HEAD"]) {
            println!("cargo:rerun-if-changed={dir}/{head}");
        }
    }
    let commit = std::env::var("DIRECT_BUILD_COMMIT")
        .ok()
        .or_else(|| git(&["rev-parse", "HEAD"]))
        .unwrap_or_else(|| "unknown".into());
    let dirty = match std::env::var("DIRECT_BUILD_DIRTY").ok() {
        Some(v) => v,
        None if commit == "unknown" => "unknown".into(),
        None => match git(&["status", "--porcelain"]) {
            Some(s) if s.is_empty() => "false".into(),
            Some(_) => "true".into(),
            None => "unknown".into(),
        },
    };
    let built_at = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    println!("cargo:rustc-env=DIRECT_BUILD_COMMIT={commit}");
    println!("cargo:rustc-env=DIRECT_BUILD_DIRTY={dirty}");
    println!("cargo:rustc-env=DIRECT_BUILD_AT={built_at}");
}
