//! Sets `OCW_COMMIT` (short git commit, `-dirty` with uncommitted changes)
//! for the build id (src/build_id.rs). Builds on remote machines get no
//! `.git` (scripts/remote.sh copies only the files), so the scripts pass
//! `OCW_BUILD_ID` (from scripts/build-id.sh); otherwise `git` is asked,
//! and without either the build id is the plain version.

use std::process::Command;

#[path = "src/build_id.rs"]
mod build_id;

fn git(args: &[&str]) -> Option<String> {
    let out = Command::new("git").args(args).output().ok()?;
    out.status
        .success()
        .then(|| String::from_utf8_lossy(&out.stdout).trim().to_string())
}

fn main() {
    println!("cargo:rerun-if-env-changed=OCW_BUILD_ID");
    let commit = match std::env::var("OCW_BUILD_ID") {
        Ok(id) => id.trim().to_string(),
        Err(_) => match git(&["rev-parse", "--short", "HEAD"]) {
            Some(sha) => {
                // Rebuild when HEAD moves or the index changes.
                for path in ["HEAD", "index"] {
                    if let Some(p) =
                        git(&["rev-parse", "--path-format=absolute", "--git-path", path])
                    {
                        println!("cargo:rerun-if-changed={p}");
                    }
                }
                let dirty = git(&["status", "--porcelain", "--untracked-files=no"])
                    .is_some_and(|s| !s.is_empty());
                build_id::commit_id(&sha, dirty)
            }
            None => String::new(),
        },
    };
    println!("cargo:rustc-env=OCW_COMMIT={commit}");
}
