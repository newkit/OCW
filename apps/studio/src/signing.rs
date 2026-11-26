//! The studio user's own key (spec 4.7 *User keys*; 14 Phase 16, 16C
//! *Signing and trust*): *Build package* signs the patched package with
//! it, so the game shows the result as *unofficial, 'name'* and another
//! computer asks with the trust dialog before loading it.
//!
//! The key is the one `world-builder keys init-user` makes – a root of
//! the user's own and a signing key that embeds it – in the same folder
//! (`$OCW_KEY_DIR`, else `<config dir>/ocw/keys`;
//! [`ocw_worldstore::keys`]). The first build asks for the name ("Your
//! name for signing worlds") and makes the key; later builds reuse it. An
//! existing key is never overwritten: a key file that cannot be used
//! (damaged, not a user key, expired) stops the build with a message, and
//! no new key is made over it. The key made here is added to this
//! computer's trusted keys (the game's list), so the user's own builds
//! load here without the dialog; a corrupt list is left untouched.
//!
//! A friend's key never signs: it is not on this computer – only the
//! user's own key in the key folder does.

use bevy::prelude::*;
use ocw_world::manifest::SigningKeyPair;
use ocw_worldstore::keys;

use crate::edit::{Studio, shown_path};

/// The status line while the name is asked for.
pub const ASK: &str = "your name for signing worlds: the package you build is signed with your own key, made now (`studio sign name NAME`, or `studio sign cancel`)";

/// The dialog's title.
pub const TITLE: &str = "Your name for signing worlds";

/// The dialog's text.
pub const TEXT: &str = "The packages you build are signed with a key of your own, made now on this computer and kept in its key folder. Others who get your world see this name as \"unofficial, 'name'\" (unverified) and its fingerprint, which they compare with you before trusting it.";

/// The longest name kept (shown in the World menu and the trust dialog).
pub const MAX_NAME: usize = ocw_worldstore::userkeys::MAX_TEXT;

/// The user's signing key: `Ok(None)` when none was made yet (ask for
/// the name), an error when the key file cannot be used (the build
/// stops; nothing is replaced).
pub fn own_key(studio: &Studio) -> Result<Option<SigningKeyPair>, String> {
    keys::user_signing_key(&studio.key_dir, ocw_worldstore::now_unix_s()).map_err(|e| {
        format!(
            "not built: your signing key cannot be used ({e}). A package you build is signed with your own key; no new key is made over an existing one – restore the file from a backup, or move the files user-root.key, user-root.pub.json and user.key out of {} to make a new key (others then see a new fingerprint)",
            shown_path(&studio.key_dir)
        )
    })
}

/// A name for the key, checked: trimmed, not empty, not too long.
pub fn check_name(name: &str) -> Result<String, String> {
    let n = name.trim();
    if n.is_empty() {
        return Err("type your name for signing worlds".into());
    }
    if n.chars().count() > MAX_NAME {
        return Err(format!("the name is longer than {MAX_NAME} characters"));
    }
    if n.chars().any(char::is_control) {
        return Err("the name has control characters".into());
    }
    Ok(n.to_string())
}

/// Makes the user's key named `name` (the dialog's *Make my key and
/// build*, `studio sign name NAME`), trusts it on this computer and starts
/// the build that waited for it.
pub fn make_key(world: &mut World, name: &str) -> Result<String, String> {
    let name = check_name(name)?;
    let now = ocw_worldstore::now_unix_s();
    let dir = world.resource::<Studio>().key_dir.clone();
    if keys::user_signing_key(&dir, now)
        .map_err(|e| format!("no new key made: {e}"))?
        .is_some()
    {
        return Err(format!(
            "you have a key already (in {}): it is never replaced",
            shown_path(&dir)
        ));
    }
    let made = keys::init_user(&dir, &name, keys::USER_KEY_DAYS, now)?;
    let fp = made.fingerprint();
    info!(
        "made your key '{name}' (fingerprint {fp}) in {}",
        shown_path(&dir)
    );
    let mut msg = format!("made your key '{name}' (fingerprint {fp})");
    // Your own builds load here without asking.
    if let Some(mut trusted) = world.get_resource_mut::<ocw_ui::trust::TrustedKeys>() {
        if trusted.warning.is_some() {
            msg.push_str("; the trusted keys' list cannot be read, so it is left as it is (trust your key in the game)");
        } else if !trusted.keys.contains(made.root.key_id)
            && let Err(e) = trusted.trust(
                &made.root,
                None,
                Some("your own key (made by OCW Studio)"),
                now,
            )
        {
            msg.push_str(&format!("; not added to the trusted keys: {e}"));
        }
    }
    let pending = world.resource_mut::<Studio>().sign_prompt.take();
    if let Some(req) = pending {
        let built = crate::edit::start_build(&mut world.resource_mut::<Studio>(), req)?;
        msg = format!("{msg}; {built}");
    }
    Ok(msg)
}

/// `studio sign`: the key that signs the builds.
pub fn info(studio: &Studio) -> String {
    let dir = shown_path(&studio.key_dir);
    match keys::user_signing_key(&studio.key_dir, ocw_worldstore::now_unix_s()) {
        Ok(Some(k)) => format!(
            "your key '{}' signs the packages you build (fingerprint {}; in {dir})",
            k.claims().name,
            k.root
                .as_ref()
                .map(|r| r.fingerprint().to_string())
                .unwrap_or_default()
        ),
        Ok(None) => {
            format!("no key yet: the first Build package asks for your name and makes it in {dir}")
        }
        Err(e) => format!("your key cannot be used: {e}"),
    }
}

#[cfg(all(test, unix))]
mod tests {
    use std::path::{Path, PathBuf};

    use super::*;
    use crate::edit::{BuildRequest, StudioCommand, apply};
    use ocw_world::manifest::{Attribution, Manifest, SignedManifest, VerifyError};
    use ocw_worldstore::trust::TrustConfig;
    use ocw_worldstore::{LoadError, LoadOptions, WorldCache};

    fn tmp() -> PathBuf {
        let d = std::env::temp_dir().join(format!(
            "ocw-studio-sign-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    /// A stand-in for `world-builder package … --out OUT`: it writes the
    /// prepared package and its unsigned manifest to OUT, as the real
    /// step does.
    fn fake_builder(d: &Path, built: &Path) -> PathBuf {
        use std::os::unix::fs::PermissionsExt;
        let p = d.join("world-builder");
        std::fs::write(
            &p,
            format!(
                "#!/bin/sh\nout=\"\"\nwhile [ $# -gt 0 ]; do [ \"$1\" = \"--out\" ] && out=\"$2\"; shift; done\necho \"  ok road 70\"\ncp '{0}' \"$out\" && cp '{0}.manifest.json' \"$out.manifest.json\"\n",
                built.display()
            ),
        )
        .unwrap();
        std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).unwrap();
        p
    }

    /// The studio with the test package open in `d`, its sources next to
    /// it, the fake builder, and its key folder and trusted keys in `d`.
    fn world(d: &Path) -> World {
        let mut pkg = crate::patch::tests::package();
        pkg.file = d.join("testland.ocwpkg");
        let bytes = pkg.pkg.to_bytes();
        std::fs::write(&pkg.file, &bytes).unwrap();
        for f in ["testland.osm.pbf", "testland.poly", "N47E009.hgt"] {
            std::fs::write(d.join(f), b"x").unwrap();
        }
        // What the build writes: the package and its manifest, unsigned.
        let built = d.join("built.ocwpkg");
        std::fs::write(&built, &bytes).unwrap();
        let mut m = Manifest::new(
            "local/testland",
            "Testland",
            2,
            ocw_worldstore::now_unix_s(),
            "test",
            vec![Attribution::openstreetmap()],
        )
        .unwrap();
        m.add_file("testland.ocwpkg", &bytes).unwrap();
        std::fs::write(
            d.join("built.ocwpkg.manifest.json"),
            SignedManifest::unsigned(&m).to_json(),
        )
        .unwrap();
        let mut s = Studio::new(None, None);
        let details = ocw_worldstore::details::WorldDetails {
            id: "local/testland".into(),
            region: "Testland".into(),
            ..Default::default()
        };
        let file = pkg.file.display().to_string();
        s.open_package(&file, &file, &details, pkg, 0.0);
        s.key_dir = d.join("keys");
        s.world_builder = Some(fake_builder(d, &built));
        let mut w = World::new();
        w.insert_resource(s);
        w.insert_resource(crate::flight::StudioFlight::default());
        w.insert_resource(trusted(&d.join("trusted-keys.json")));
        w
    }

    fn trusted(file: &Path) -> ocw_ui::trust::TrustedKeys {
        ocw_ui::trust::TrustedKeys::load(
            Some(Box::new(ocw_ui::settings::FileStore(file.to_path_buf()))),
            TrustConfig::for_build().builtin_store().unwrap(),
        )
    }

    fn request(d: &Path) -> StudioCommand {
        let pkg = d.join("testland.ocwpkg");
        StudioCommand::Build(BuildRequest {
            sources: crate::build::find_sources(&pkg),
            out: crate::build::default_output(&pkg),
            source_dir: None,
        })
    }

    fn wait(w: &World) {
        let job = w.resource::<Studio>().build.clone().expect("a build");
        for _ in 0..1000 {
            if !job.running() {
                return;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        panic!("the build did not end");
    }

    /// The loader of another computer (a fresh trust store) or of this
    /// one (its trusted keys' file).
    fn options(keys: Option<&Path>) -> LoadOptions {
        let trust = TrustConfig {
            user_keys_file: keys.map(Path::to_path_buf),
            ..TrustConfig::for_build()
        }
        .trust_store()
        .unwrap();
        LoadOptions::new(
            trust,
            WorldCache::new(std::env::temp_dir().join("ocw-studio-sign-no-cache")),
        )
    }

    fn signed(out: &Path) -> SignedManifest {
        let mut side = out.as_os_str().to_owned();
        side.push(".manifest.json");
        SignedManifest::parse(&std::fs::read_to_string(PathBuf::from(side)).unwrap()).unwrap()
    }

    #[test]
    fn the_first_build_makes_the_key_and_signs_and_the_next_reuses_it() {
        let d = tmp();
        let mut w = world(&d);
        let out = crate::build::default_output(&d.join("testland.ocwpkg"));
        // No key yet: the build waits for the name; no key file is made.
        let msg = apply(&mut w, request(&d)).unwrap();
        assert_eq!(msg, ASK);
        assert!(w.resource::<Studio>().sign_prompt.is_some());
        assert!(w.resource::<Studio>().build.is_none());
        assert!(!d.join("keys").exists());
        assert!(apply(&mut w, StudioCommand::SignName("  ".into())).is_err());
        // The name: the key is made (secret files 0600), trusted here, and
        // the build starts.
        let msg = apply(&mut w, StudioCommand::SignName("Ada Rider".into())).unwrap();
        assert!(
            msg.starts_with("made your key 'Ada Rider' (fingerprint "),
            "{msg}"
        );
        assert!(msg.contains("building"), "{msg}");
        assert!(w.resource::<Studio>().sign_prompt.is_none());
        use std::os::unix::fs::PermissionsExt;
        for f in [keys::USER_SIGNING, keys::USER_ROOT_SECRET] {
            let mode = std::fs::metadata(d.join("keys").join(f))
                .unwrap()
                .permissions()
                .mode();
            assert_eq!(mode & 0o777, 0o600, "{f}");
        }
        wait(&w);
        let summary = w.resource::<Studio>().build.as_ref().unwrap().summary();
        assert!(summary.contains("signed as 'Ada Rider'"), "{summary}");
        let m = signed(&out);
        assert!(m.is_signed());
        let root = m.embedded_root().expect("a user key").clone();
        assert_eq!(root.name, "Ada Rider");
        let spec = out.display().to_string();
        // Another computer: an intact package by an unknown key – the
        // trust dialog, never loaded unasked.
        let fresh = options(None);
        let e = ocw_worldstore::load_world(&spec, &fresh).unwrap_err();
        assert!(
            matches!(e, LoadError::Verify(_, VerifyError::UntrustedKey(_))),
            "{e}"
        );
        let req = ocw_worldstore::consent_needed(&spec, &fresh)
            .unwrap()
            .expect("asks");
        assert_eq!(req.claimed_signer, "Ada Rider");
        assert_eq!(req.fingerprint(), root.fingerprint());
        // … Trust this key there: loads, unofficial.
        let other = d.join("other-trusted-keys.json");
        trusted(&other)
            .trust(&req.root, None, None, ocw_worldstore::now_unix_s())
            .unwrap();
        let loaded = ocw_worldstore::load_world(&spec, &options(Some(&other))).unwrap();
        assert!(loaded.trust.is_trusted());
        assert!(
            loaded.trust.to_string().contains("UNOFFICIAL"),
            "{}",
            loaded.trust
        );
        // This computer trusts its user's own key already.
        let here = ocw_worldstore::load_world(&spec, &options(Some(&d.join("trusted-keys.json"))));
        assert!(here.unwrap().trust.is_trusted());
        // The second build reuses the key: no question, same signer.
        let key_file = std::fs::read(d.join("keys").join(keys::USER_SIGNING)).unwrap();
        let msg = apply(&mut w, request(&d)).unwrap();
        assert!(msg.starts_with("building"), "{msg}");
        wait(&w);
        assert_eq!(
            signed(&out).embedded_root().map(|r| r.fingerprint()),
            Some(root.fingerprint())
        );
        assert_eq!(
            std::fs::read(d.join("keys").join(keys::USER_SIGNING)).unwrap(),
            key_file
        );
        assert!(apply(&mut w, StudioCommand::SignName("Bea Climber".into())).is_err());
        assert!(info(w.resource::<Studio>()).contains("'Ada Rider'"));
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn a_corrupt_key_refuses_the_build_and_makes_no_new_key() {
        let d = tmp();
        let mut w = world(&d);
        std::fs::create_dir_all(d.join("keys")).unwrap();
        let file = d.join("keys").join(keys::USER_SIGNING);
        std::fs::write(&file, "{ not a key").unwrap();
        let e = apply(&mut w, request(&d)).unwrap_err();
        assert!(
            e.starts_with("not built: your signing key cannot be used"),
            "{e}"
        );
        assert!(e.contains("no new key is made"), "{e}");
        let s = w.resource::<Studio>();
        assert!(s.sign_prompt.is_none() && s.build.is_none());
        assert!(apply(&mut w, StudioCommand::SignName("Ada Rider".into())).is_err());
        assert_eq!(std::fs::read_to_string(&file).unwrap(), "{ not a key");
        assert!(!d.join("keys").join(keys::USER_ROOT_SECRET).exists());
        // Cancel with nothing waiting says nothing.
        assert_eq!(apply(&mut w, StudioCommand::SignCancel).unwrap(), "");
        std::fs::remove_dir_all(&d).unwrap();
    }
}
