//! The build id shown in the HUD, in `ocw-desktop --version` and in the log:
//! workspace version, short git commit, `-dirty` for uncommitted changes and
//! ` (debug)` for debug builds, e.g. `0.1.0-7d538c0-dirty (debug)`.
//!
//! `build.rs` includes this file too, so keep it free of crate imports.

/// The commit part: `7d538c0` or `7d538c0-dirty`; empty without a commit.
#[allow(dead_code)] // only build.rs uses it (and the tests)
pub fn commit_id(sha: &str, dirty: bool) -> String {
    let sha = sha.trim();
    if sha.is_empty() {
        String::new()
    } else if dirty {
        format!("{sha}-dirty")
    } else {
        sha.to_string()
    }
}

/// The full build id from the package version and [`commit_id`].
#[allow(dead_code)] // unused in build.rs
pub fn build_id(version: &str, commit: &str, debug: bool) -> String {
    let mut id = version.to_string();
    let commit = commit.trim();
    if !commit.is_empty() {
        id.push('-');
        id.push_str(commit);
    }
    if debug {
        id.push_str(" (debug)");
    }
    id
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn commit_with_and_without_changes() {
        assert_eq!(commit_id("7d538c0\n", false), "7d538c0");
        assert_eq!(commit_id("7d538c0", true), "7d538c0-dirty");
        assert_eq!(commit_id("", true), "");
    }

    #[test]
    fn build_id_formats() {
        assert_eq!(build_id("0.1.0", "7d538c0", false), "0.1.0-7d538c0");
        assert_eq!(
            build_id("0.1.0", "7d538c0-dirty", true),
            "0.1.0-7d538c0-dirty (debug)"
        );
        assert_eq!(build_id("0.1.0", "", false), "0.1.0");
        assert_eq!(build_id("0.1.0", " ", true), "0.1.0 (debug)");
    }
}
