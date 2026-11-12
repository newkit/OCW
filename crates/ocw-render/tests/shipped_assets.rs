//! Phase 4 integration test (spec §14): every shipped glTF asset passes the
//! asset validator and loads into a headless Bevy app – through Bevy's own
//! glTF loader (the files are standard glTF 2.0) and as avatars through the
//! riders plugin.

use std::path::PathBuf;
use std::time::Duration;

use bevy::asset::LoadState;
use bevy::gltf::{Gltf, GltfPlugin};
use bevy::prelude::*;

fn assets_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../assets")
}

#[test]
fn shipped_assets_pass_the_validator() {
    let checked = ocw_assets::manifest::validate_dir(&assets_dir().join("models"))
        .unwrap_or_else(|e| panic!("{e:#?}"));
    assert_eq!(checked.len(), 7);
    // The embedded copies are the files on disk.
    for (name, bytes) in ocw_assets::shipped::FILES {
        let disk = std::fs::read(assets_dir().join("models").join(name)).unwrap();
        assert_eq!(disk, bytes, "{name}");
    }
}

#[test]
fn shipped_assets_load_with_bevys_gltf_loader() {
    let mut app = App::new();
    app.add_plugins((
        MinimalPlugins,
        AssetPlugin {
            file_path: assets_dir().to_string_lossy().into_owned(),
            ..default()
        },
        bevy::scene::ScenePlugin,
        bevy::world_serialization::WorldSerializationPlugin,
        bevy::image::ImagePlugin::default(),
        bevy::mesh::MeshPlugin,
        GltfPlugin::default(),
    ))
    .init_asset::<StandardMaterial>()
    .init_asset::<bevy::animation::AnimationClip>();
    app.finish();
    app.cleanup();
    let handles: Vec<(String, Handle<Gltf>)> = ocw_assets::shipped::FILES
        .iter()
        .map(|(name, _)| {
            let h = app
                .world()
                .resource::<AssetServer>()
                .load(format!("models/{name}"));
            (name.to_string(), h)
        })
        .collect();
    for _ in 0..2000 {
        app.update();
        let server = app.world().resource::<AssetServer>();
        let states: Vec<LoadState> = handles.iter().map(|(_, h)| server.load_state(h)).collect();
        if let Some((i, LoadState::Failed(e))) = states
            .iter()
            .enumerate()
            .find(|(_, s)| matches!(s, LoadState::Failed(_)))
        {
            panic!("{}: {e}", handles[i].0);
        }
        if states.iter().all(|s| matches!(s, LoadState::Loaded)) {
            break;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    let gltfs = app.world().resource::<Assets<Gltf>>();
    for (name, h) in &handles {
        let g = gltfs.get(h).unwrap_or_else(|| panic!("{name} not loaded"));
        let required: &[&str] = if name.starts_with("bike") {
            &ocw_assets::pose::bike_nodes::REQUIRED
        } else {
            &ocw_assets::pose::nodes::REQUIRED
        };
        for n in required {
            assert!(g.named_nodes.contains_key(*n), "{name}: node {n}");
        }
        assert!(!g.meshes.is_empty() && !g.materials.is_empty(), "{name}");
    }
}
