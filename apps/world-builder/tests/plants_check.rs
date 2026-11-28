//! The build fails with a plant on a road (the world-building rule of
//! 2026-11-28): `world-builder plants-check` and `check` refuse a package
//! that stores a tree on a road and pass the same package without it.

use std::process::Command;

use ocw_world::TileKey;
use ocw_world::road_clear::fixture::world;

fn run(args: &[&str]) -> (i32, String) {
    let out = Command::new(env!("CARGO_BIN_EXE_world-builder"))
        .args(args)
        .output()
        .expect("world-builder runs");
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    (out.status.code().unwrap_or(-1), text)
}

#[test]
fn a_tree_on_a_road_fails_the_build() {
    let dir = tempfile::tempdir().unwrap();
    // Mapped trees stored on the roads (as an older world builder kept
    // them): the drawn placement leaves them out, the check still fails.
    let bad = dir.path().join("bad.ocwpkg");
    let mut pkg = world();
    std::fs::write(&bad, pkg.to_bytes()).unwrap();
    let bad = bad.to_str().unwrap();
    let (code, text) = run(&["plants-check", "--world", bad]);
    assert_eq!(code, 1, "{text}");
    assert!(text.contains("stored tree (VEGE)"), "{text}");
    assert!(text.contains("x 250.0 z 301.0"), "{text}");
    assert!(
        text.contains("no tree or plant may stand on a road"),
        "{text}"
    );
    let (code, text) = run(&["check", "--world", bad]);
    assert_eq!(code, 1, "{text}");
    let (code, _) = run(&["plants-check", "--world", bad, "--report-only"]);
    assert_eq!(code, 0);
    // Only the trees the placement draws: the check passes.
    let key = TileKey { x: 0, z: 0 };
    let kept: Vec<_> = ocw_world::plantings::mapped_trees(&pkg, key)
        .into_iter()
        .map(|(_, t)| t)
        .collect();
    pkg.look
        .vegetation
        .as_mut()
        .unwrap()
        .tiles
        .get_mut(&key)
        .unwrap()
        .trees = kept;
    let good = dir.path().join("good.ocwpkg");
    std::fs::write(&good, pkg.to_bytes()).unwrap();
    let (code, text) = run(&["plants-check", "--world", good.to_str().unwrap()]);
    assert_eq!(code, 0, "{text}");
    assert!(text.contains("plants on roads: 0 of"), "{text}");
}
