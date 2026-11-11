//! Log file for the macOS app bundle (spec §11.3).
//!
//! Started from Finder or the Dock, the app has no terminal and its stderr
//! goes nowhere. So when the binary runs from `….app/Contents/MacOS/` and
//! stderr is not a terminal, the log goes to
//! `~/Library/Logs/OCW/ocw-desktop.log` instead (Console.app lists it under
//! "Log Reports"); the previous run's log is kept as `ocw-desktop.old.log`.
//! Everywhere else (Linux, Windows, a terminal) the log stays on stderr.

use bevy::log::BoxedFmtLayer;
use bevy::prelude::*;

/// For [`bevy::log::LogPlugin::fmt_layer`]: the log file layer in an app
/// bundle on macOS, otherwise `None` (Bevy's default stderr layer).
#[cfg(target_os = "macos")]
pub fn fmt_layer(_app: &mut App) -> Option<BoxedFmtLayer> {
    use std::io::IsTerminal;
    if std::io::stderr().is_terminal() {
        return None;
    }
    let path = bundle_log_path(&std::env::current_exe().ok()?, &dirs::home_dir()?)?;
    let file = std::sync::Arc::new(open_log(&path).ok()?);
    // Panic messages go to stderr too: log them first.
    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        error!("{info}");
        default_hook(info);
    }));
    Some(Box::new(
        bevy::log::tracing_subscriber::fmt::Layer::default()
            .with_ansi(false)
            .with_writer(file),
    ))
}

#[cfg(not(target_os = "macos"))]
pub fn fmt_layer(_app: &mut App) -> Option<BoxedFmtLayer> {
    None
}

/// `HOME/Library/Logs/OCW/ocw-desktop.log` if `exe` is the executable of an
/// app bundle (`X.app/Contents/MacOS/exe`), else `None`.
#[cfg(any(target_os = "macos", test))]
fn bundle_log_path(exe: &std::path::Path, home: &std::path::Path) -> Option<std::path::PathBuf> {
    let macos = exe.parent()?;
    let contents = macos.parent()?;
    let bundle = contents.parent()?;
    (macos.file_name()? == "MacOS"
        && contents.file_name()? == "Contents"
        && bundle.extension()? == "app")
        .then(|| home.join("Library/Logs/OCW/ocw-desktop.log"))
}

/// Creates the log file (and its directory), keeping the previous one as
/// `*.old.log`.
#[cfg(any(target_os = "macos", test))]
fn open_log(path: &std::path::Path) -> std::io::Result<std::fs::File> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    if path.exists() {
        std::fs::rename(path, path.with_extension("old.log"))?;
    }
    std::fs::File::create(path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn only_an_app_bundle_logs_to_a_file() {
        let home = Path::new("/Users/me");
        assert_eq!(
            bundle_log_path(
                Path::new("/Applications/Open Cycling World.app/Contents/MacOS/ocw-desktop"),
                home
            ),
            Some(home.join("Library/Logs/OCW/ocw-desktop.log"))
        );
        for exe in [
            "/Users/me/OCW/target/release/ocw-desktop",
            "/opt/Contents/MacOS/ocw-desktop",
            "/x.app/MacOS/ocw-desktop",
            "ocw-desktop",
        ] {
            assert_eq!(bundle_log_path(Path::new(exe), home), None, "{exe}");
        }
    }

    #[test]
    fn the_previous_log_is_kept() {
        let dir = std::env::temp_dir().join(format!("ocw-logfile-test-{}", std::process::id()));
        let path = dir.join("Logs/OCW/ocw-desktop.log");
        use std::io::Write;
        open_log(&path).unwrap().write_all(b"first").unwrap();
        open_log(&path).unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "");
        let old = dir.join("Logs/OCW/ocw-desktop.old.log");
        assert_eq!(std::fs::read_to_string(old).unwrap(), "first");
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
