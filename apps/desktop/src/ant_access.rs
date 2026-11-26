//! Access to the ANT USB stick on Linux (spec 3.4.1 *Linux access rule*,
//! 14 Phase 17 Q8 (b)): `ocw-desktop --install-udev-rule` and the Devices
//! page's *Install access for the ANT stick* install a udev rule that lets
//! the logged-in user open the stick – its USB device (the `nusb` link)
//! and its `/dev/ttyUSBn` (the kernel's serial driver) – so the `dialout`
//! group is never needed.
//!
//! The rule is written by a small shell script run as root through
//! `pkexec` (polkit asks for the password), which then reloads udev. The
//! script gets the rule as an argument, so it works the same from the
//! AppImage (whose files root cannot read inside the user's FUSE mount)
//! and from the tar.xz. Without `pkexec` (or when polkit refuses) nothing
//! is changed and the rule and the manual command are shown instead. A
//! different rule already in the file is never overwritten without asking
//! (`--replace-udev-rule`, or *y* in a terminal); the same rule is left
//! alone (idempotent).
//!
//! `pkexec` is reached only through the [`Runner`] seam: tests use a fake
//! and never run it.

use std::io::IsTerminal;
use std::path::{Path, PathBuf};
use std::process::Command;

/// Where the rule goes (60: before `73-seat-late.rules` turns `uaccess`
/// into the logged-in user's ACL).
pub const RULE_PATH: &str = "/etc/udev/rules.d/60-ocw-ant.rules";

/// The ids: ANT USB1 (`0fcf:1004`, a CP2102: `cp210x` → tty), ANT USB2
/// (`1008`) and ANT USB-m (`1009`) and their clones.
const MATCH: &str =
    r#"ATTRS{idVendor}=="0fcf", ATTRS{idProduct}=="1004|1008|1009", MODE="0660", TAG+="uaccess""#;

/// The rule file's text (without a final newline; the file has one).
pub fn rule_text() -> String {
    format!(
        "# Open Cycling World: the logged-in user may open ANT USB sticks\n\
         # (ANT USB-m, USB2, USB1 and clones) - the USB device and its tty.\n\
         # Installed by `ocw-desktop --install-udev-rule`.\n\
         SUBSYSTEM==\"usb\", {MATCH}\n\
         SUBSYSTEM==\"tty\", {MATCH}"
    )
}

/// The rule as one line per subsystem, for the manual route.
pub fn rule_lines() -> [String; 2] {
    [
        format!("SUBSYSTEM==\"usb\", {MATCH}"),
        format!("SUBSYSTEM==\"tty\", {MATCH}"),
    ]
}

/// What to type as root without pkexec.
pub fn manual_command(path: &Path) -> String {
    let [usb, tty] = rule_lines();
    format!(
        "printf '%s\\n' '{usb}' '{tty}' | sudo tee {p} >/dev/null && sudo udevadm control --reload-rules && sudo udevadm trigger",
        p = path.display()
    )
}

/// What the rule file holds now.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Existing {
    Missing,
    /// Our rule (whitespace at line ends and the final newline aside).
    Same,
    /// Something else.
    Different,
}

fn normalise(s: &str) -> String {
    s.lines()
        .map(str::trim_end)
        .collect::<Vec<_>>()
        .join("\n")
        .trim_end()
        .to_owned()
}

pub fn existing(path: &Path) -> Existing {
    match std::fs::read_to_string(path) {
        Ok(t) if normalise(&t) == normalise(&rule_text()) => Existing::Same,
        Ok(_) => Existing::Different,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Existing::Missing,
        // There but unreadable: not ours to overwrite blindly.
        Err(_) => Existing::Different,
    }
}

/// How an install went.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    /// The same rule was there already: nothing changed.
    AlreadyInstalled,
    Installed,
    /// Refused: a different rule is in the file (ask first).
    DifferentRule,
    /// Refused: no `pkexec` on this system.
    NoPkexec,
    /// The password dialog was dismissed.
    Cancelled,
    /// polkit did not allow it (no authentication agent, not authorised).
    NotAuthorised,
    Failed(String),
}

impl Outcome {
    pub fn ok(&self) -> bool {
        matches!(self, Self::Installed | Self::AlreadyInstalled)
    }

    /// One short line (the Devices page).
    pub fn short(&self) -> String {
        match self {
            Self::AlreadyInstalled => "already installed".into(),
            Self::Installed => "installed: plug the stick in again".into(),
            Self::DifferentRule => "a different rule is there: not replaced".into(),
            Self::NoPkexec => "no pkexec: install by hand (see console)".into(),
            Self::Cancelled => "cancelled".into(),
            Self::NotAuthorised => "not allowed by polkit (see console)".into(),
            Self::Failed(e) => format!("failed: {e}"),
        }
    }

    /// The full text (console, terminal): the manual route where it helps.
    pub fn message(&self, path: &Path) -> String {
        let p = path.display();
        let manual = || {
            let [usb, tty] = rule_lines();
            format!(
                "\nThe rule ({p}):\n  {usb}\n  {tty}\nTo install it by hand, as root:\n  {}",
                manual_command(path)
            )
        };
        match self {
            Self::AlreadyInstalled => {
                format!("The ANT stick's access rule is already installed ({p}).")
            }
            Self::Installed => format!(
                "Installed the ANT stick's access rule ({p}) and reloaded udev: plug the stick in again."
            ),
            Self::DifferentRule => format!(
                "{p} holds a different rule: not replaced. Replace it with `ocw-desktop --install-udev-rule --replace-udev-rule`.{}",
                manual()
            ),
            Self::NoPkexec => format!(
                "pkexec (polkit) is not installed, so the rule cannot be installed from here.{}",
                manual()
            ),
            Self::Cancelled => "Cancelled: the access rule was not installed.".into(),
            Self::NotAuthorised => format!(
                "pkexec could not get permission (no polkit authentication agent, or not allowed).{}",
                manual()
            ),
            Self::Failed(e) => format!("Installing the access rule failed: {e}.{}", manual()),
        }
    }
}

/// Runs programs (the system; a fake in tests).
pub trait Runner: Send + Sync {
    /// The program on `PATH`.
    fn find(&self, program: &str) -> Option<PathBuf>;
    /// Runs it and waits: its exit code (`None`: killed by a signal).
    fn run(&self, program: &Path, args: &[String]) -> std::io::Result<Option<i32>>;
}

/// The real system.
pub struct System;

impl Runner for System {
    fn find(&self, program: &str) -> Option<PathBuf> {
        let path = std::env::var_os("PATH")?;
        std::env::split_paths(&path)
            .map(|d| d.join(program))
            .find(|p| p.is_file())
    }

    fn run(&self, program: &Path, args: &[String]) -> std::io::Result<Option<i32>> {
        Command::new(program).args(args).status().map(|s| s.code())
    }
}

/// The root script: `$1` the rule, `$2` the file, `$3` `replace` or
/// `keep`. Exit codes: 0 done, 3 a different rule is there, 4 no udevadm,
/// 5 cannot write, 6 udevadm failed (126 / 127 are pkexec's own).
pub const SCRIPT: &str = r#"rule="$1"; path="$2"; mode="$3"
if [ -e "$path" ] && [ "$(cat "$path")" != "$rule" ] && [ "$mode" != replace ]; then exit 3; fi
command -v udevadm >/dev/null 2>&1 || exit 4
mkdir -p "$(dirname "$path")" || exit 5
umask 022
printf '%s\n' "$rule" > "$path.ocw-new" || exit 5
mv -f "$path.ocw-new" "$path" || exit 5
udevadm control --reload-rules || exit 6
udevadm trigger || exit 6
"#;

/// The arguments for `pkexec`: `/bin/sh -c SCRIPT sh RULE PATH MODE`.
pub fn pkexec_args(path: &Path, replace: bool) -> Vec<String> {
    vec![
        "/bin/sh".into(),
        "-c".into(),
        SCRIPT.into(),
        "sh".into(),
        rule_text(),
        path.display().to_string(),
        if replace { "replace" } else { "keep" }.into(),
    ]
}

/// Installs the rule at `path` through `pkexec` (unless it is there).
pub fn install(runner: &dyn Runner, path: &Path, replace: bool) -> Outcome {
    match existing(path) {
        Existing::Same => return Outcome::AlreadyInstalled,
        Existing::Different if !replace => return Outcome::DifferentRule,
        _ => {}
    }
    let Some(pkexec) = runner.find("pkexec") else {
        return Outcome::NoPkexec;
    };
    match runner.run(&pkexec, &pkexec_args(path, replace)) {
        Ok(Some(0)) => match existing(path) {
            Existing::Same => Outcome::Installed,
            _ => Outcome::Failed(format!("{} does not hold the rule", path.display())),
        },
        Ok(Some(3)) => Outcome::DifferentRule,
        Ok(Some(4)) => Outcome::Failed("udevadm not found".into()),
        Ok(Some(5)) => Outcome::Failed(format!("cannot write {}", path.display())),
        Ok(Some(6)) => Outcome::Failed("udevadm could not reload the rules".into()),
        Ok(Some(126)) => Outcome::Cancelled,
        Ok(Some(127)) => Outcome::NotAuthorised,
        Ok(Some(c)) => Outcome::Failed(format!("pkexec exited with {c}")),
        Ok(None) => Outcome::Failed("pkexec was stopped".into()),
        Err(e) => Outcome::Failed(format!("cannot run pkexec: {e}")),
    }
}

/// `ocw-desktop --install-udev-rule [--replace-udev-rule]` in a terminal:
/// asks before replacing a different rule. The exit code.
pub fn cli(replace: bool) -> i32 {
    if !cfg!(target_os = "linux") {
        eprintln!("--install-udev-rule is for Linux only.");
        return 2;
    }
    let path = Path::new(RULE_PATH);
    let mut replace = replace;
    if !replace && existing(path) == Existing::Different && std::io::stdin().is_terminal() {
        eprint!(
            "{} holds a different rule. Replace it? [y/N] ",
            path.display()
        );
        let mut answer = String::new();
        let _ = std::io::stdin().read_line(&mut answer);
        replace = matches!(answer.trim(), "y" | "Y" | "yes");
    }
    let outcome = install(&System, path, replace);
    let text = outcome.message(path);
    if outcome.ok() {
        println!("{text}");
        0
    } else {
        eprintln!("{text}");
        1
    }
}

#[cfg(all(test, unix))]
mod tests {
    use std::sync::Mutex;

    use super::*;

    fn dir(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("ocw-udev-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    /// A fake `pkexec`: runs the script itself (as this user, on a file in
    /// a temp folder) with a fake `udevadm` that logs its calls – the real
    /// pkexec and udevadm are never run.
    struct FakePkexec {
        has_pkexec: bool,
        /// Instead of running the script: this exit code (126, 127).
        answer: Option<i32>,
        bin: PathBuf,
        calls: Mutex<Vec<Vec<String>>>,
    }

    impl FakePkexec {
        fn new(dir: &Path) -> Self {
            let bin = dir.join("bin");
            std::fs::create_dir_all(&bin).unwrap();
            let udevadm = bin.join("udevadm");
            std::fs::write(
                &udevadm,
                format!(
                    "#!/bin/sh\necho \"$@\" >> {}\n",
                    dir.join("udevadm.log").display()
                ),
            )
            .unwrap();
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&udevadm, std::fs::Permissions::from_mode(0o755)).unwrap();
            Self {
                has_pkexec: true,
                answer: None,
                bin,
                calls: Mutex::new(Vec::new()),
            }
        }
    }

    impl Runner for FakePkexec {
        fn find(&self, program: &str) -> Option<PathBuf> {
            assert_eq!(program, "pkexec");
            self.has_pkexec.then(|| PathBuf::from("/usr/bin/pkexec"))
        }

        fn run(&self, program: &Path, args: &[String]) -> std::io::Result<Option<i32>> {
            assert_eq!(program, Path::new("/usr/bin/pkexec"));
            self.calls.lock().unwrap().push(args.to_vec());
            if let Some(code) = self.answer {
                return Ok(Some(code));
            }
            let path = format!("{}:/usr/bin:/bin", self.bin.display());
            Command::new(&args[0])
                .args(&args[1..])
                .env("PATH", path)
                .status()
                .map(|s| s.code())
        }
    }

    #[test]
    fn the_rule_covers_the_usb_device_and_its_tty() {
        let t = rule_text();
        let rules: Vec<&str> = t.lines().filter(|l| !l.starts_with('#')).collect();
        assert_eq!(
            rules,
            [
                r#"SUBSYSTEM=="usb", ATTRS{idVendor}=="0fcf", ATTRS{idProduct}=="1004|1008|1009", MODE="0660", TAG+="uaccess""#,
                r#"SUBSYSTEM=="tty", ATTRS{idVendor}=="0fcf", ATTRS{idProduct}=="1004|1008|1009", MODE="0660", TAG+="uaccess""#,
            ]
        );
        assert!(t.is_ascii() && !t.contains("dialout") && !t.contains("GROUP"));
        assert_eq!(rule_lines(), [rules[0].to_owned(), rules[1].to_owned()]);
        assert_eq!(RULE_PATH, "/etc/udev/rules.d/60-ocw-ant.rules");
        let m = manual_command(Path::new(RULE_PATH));
        assert!(
            m.contains("sudo tee /etc/udev/rules.d/60-ocw-ant.rules"),
            "{m}"
        );
        assert!(m.ends_with("sudo udevadm control --reload-rules && sudo udevadm trigger"));
    }

    #[test]
    fn installs_through_pkexec_once_then_leaves_it() {
        let d = dir("install");
        let path = d.join("rules.d/60-ocw-ant.rules");
        let fake = FakePkexec::new(&d);
        assert_eq!(install(&fake, &path, false), Outcome::Installed);
        assert_eq!(existing(&path), Existing::Same);
        let text = std::fs::read_to_string(&path).unwrap();
        assert_eq!(text, format!("{}\n", rule_text()));
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o644);
        let log = std::fs::read_to_string(d.join("udevadm.log")).unwrap();
        assert_eq!(log, "control --reload-rules\ntrigger\n");
        // pkexec ran a shell with the rule as an argument (no file of ours
        // needed: the AppImage's mount is not root's to read).
        let calls = fake.calls.lock().unwrap().clone();
        assert_eq!(calls.len(), 1);
        assert_eq!(&calls[0][..2], ["/bin/sh", "-c"]);
        assert_eq!(calls[0][4], rule_text());
        // Idempotent: the second time nothing runs.
        assert_eq!(install(&fake, &path, false), Outcome::AlreadyInstalled);
        assert_eq!(install(&fake, &path, true), Outcome::AlreadyInstalled);
        assert_eq!(fake.calls.lock().unwrap().len(), 1);
        // Trailing spaces or a missing newline still read as ours.
        std::fs::write(&path, format!("{}  \n\n", rule_text())).unwrap();
        assert_eq!(existing(&path), Existing::Same);
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn a_different_rule_is_not_overwritten_without_asking() {
        let d = dir("different");
        let path = d.join("60-ocw-ant.rules");
        let other = "SUBSYSTEM==\"usb\", ATTRS{idVendor}==\"0fcf\", GROUP=\"plugdev\"\n";
        std::fs::write(&path, other).unwrap();
        let fake = FakePkexec::new(&d);
        let o = install(&fake, &path, false);
        assert_eq!(o, Outcome::DifferentRule);
        assert!(o.message(&path).contains("--replace-udev-rule"));
        assert!(fake.calls.lock().unwrap().is_empty(), "pkexec not asked");
        assert_eq!(std::fs::read_to_string(&path).unwrap(), other);
        // The script refuses too (the file changed after the check).
        let mut args = pkexec_args(&path, false);
        args.remove(0);
        let code = Command::new("/bin/sh")
            .args(&args)
            .env("PATH", format!("{}:/usr/bin:/bin", fake.bin.display()))
            .status()
            .unwrap()
            .code();
        assert_eq!(code, Some(3));
        assert_eq!(std::fs::read_to_string(&path).unwrap(), other);
        // Asked: replaced.
        assert_eq!(install(&fake, &path, true), Outcome::Installed);
        assert_eq!(existing(&path), Existing::Same);
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn without_pkexec_or_polkit_it_refuses_and_shows_the_rule() {
        let d = dir("nopkexec");
        let path = d.join("60-ocw-ant.rules");
        let mut fake = FakePkexec::new(&d);
        fake.has_pkexec = false;
        let o = install(&fake, &path, false);
        assert_eq!(o, Outcome::NoPkexec);
        assert!(!o.ok());
        assert_eq!(existing(&path), Existing::Missing);
        let m = o.message(&path);
        let [usb, tty] = rule_lines();
        assert!(m.contains(&usb) && m.contains(&tty), "{m}");
        assert!(m.contains(&manual_command(&path)), "{m}");
        assert!(o.short().len() <= 40);
        // pkexec there, but the dialog was dismissed / polkit said no.
        fake.has_pkexec = true;
        fake.answer = Some(126);
        assert_eq!(install(&fake, &path, false), Outcome::Cancelled);
        fake.answer = Some(127);
        let o = install(&fake, &path, false);
        assert_eq!(o, Outcome::NotAuthorised);
        assert!(o.message(&path).contains(&manual_command(&path)));
        // pkexec said yes but nothing was written.
        fake.answer = Some(0);
        assert!(matches!(install(&fake, &path, false), Outcome::Failed(_)));
        assert_eq!(existing(&path), Existing::Missing);
        for o in [
            Outcome::AlreadyInstalled,
            Outcome::Installed,
            Outcome::DifferentRule,
            Outcome::NoPkexec,
            Outcome::Cancelled,
            Outcome::NotAuthorised,
        ] {
            assert!(o.short().len() <= 40 && o.short().is_ascii(), "{o:?}");
        }
        let _ = std::fs::remove_dir_all(&d);
    }
}
