//! Default command-line arguments from a file (for installed test builds:
//! the kiosk starts the app with fixed arguments, a deployment can add
//! more, e.g. `--world`, without touching the kiosk's autostart entry).
//!
//! The file is `$OCW_ARGS_FILE` if set (empty: none), otherwise
//! `ocw-desktop.args` next to the executable, if it exists. `--no-args-file`
//! on the command line ignores it.
//!
//! Format: arguments separated by whitespace or newlines, as in a shell:
//! `'...'` and `"..."` quote (inside `"..."`, `\"` and `\\` escape), `#`
//! at the start of a word comments out the rest of the line. No variables,
//! no globbing. The file's arguments go before the real command-line
//! arguments, so the command line wins for single-value options
//! (`args_override_self`); repeatable options (`--cmd`, `--at`, `--exec`,
//! `--trust-root`) accumulate, the file's first.

use std::ffi::OsString;
use std::path::{Path, PathBuf};

/// The file name looked for next to the executable.
pub const FILE_NAME: &str = "ocw-desktop.args";
/// Environment variable naming the file (empty: no file).
pub const ENV: &str = "OCW_ARGS_FILE";
/// Command-line flag that ignores the file.
pub const NO_ARGS_FILE: &str = "--no-args-file";

/// Splits the file text into arguments (see the module docs).
pub fn split(text: &str) -> Result<Vec<String>, String> {
    let mut out = Vec::new();
    let mut chars = text.chars().peekable();
    let mut line = 1;
    loop {
        // Skip whitespace and comments between words.
        while let Some(&c) = chars.peek() {
            if c == '\n' {
                line += 1;
                chars.next();
            } else if c.is_whitespace() {
                chars.next();
            } else if c == '#' {
                while chars.peek().is_some_and(|&c| c != '\n') {
                    chars.next();
                }
            } else {
                break;
            }
        }
        if chars.peek().is_none() {
            return Ok(out);
        }
        let mut word = String::new();
        while let Some(&c) = chars.peek() {
            if c.is_whitespace() {
                break;
            }
            chars.next();
            match c {
                '\'' => loop {
                    match chars.next() {
                        Some('\'') => break,
                        Some(c) => {
                            line += usize::from(c == '\n');
                            word.push(c);
                        }
                        None => return Err(format!("line {line}: unterminated '")),
                    }
                },
                '"' => loop {
                    match chars.next() {
                        Some('"') => break,
                        Some('\\') if matches!(chars.peek(), Some('"' | '\\')) => {
                            word.push(chars.next().unwrap());
                        }
                        Some(c) => {
                            line += usize::from(c == '\n');
                            word.push(c);
                        }
                        None => return Err(format!("line {line}: unterminated \"")),
                    }
                },
                c => word.push(c),
            }
        }
        out.push(word);
    }
}

/// The real command line with the file's arguments inserted after the
/// program name (before everything else, so the command line wins).
pub fn merge(file_args: Vec<String>, argv: Vec<OsString>) -> Vec<OsString> {
    let mut argv = argv.into_iter();
    let mut out: Vec<OsString> = argv.next().into_iter().collect();
    out.extend(file_args.into_iter().map(OsString::from));
    out.extend(argv);
    out
}

/// Which file to read: `$OCW_ARGS_FILE` (`env`; empty: none) or
/// `ocw-desktop.args` beside the executable, if it exists.
pub fn locate(env: Option<OsString>, exe: Option<&Path>) -> Option<PathBuf> {
    match env {
        Some(p) if p.is_empty() => None,
        Some(p) => Some(PathBuf::from(p)),
        None => exe
            .and_then(Path::parent)
            .map(|d| d.join(FILE_NAME))
            .filter(|p| p.is_file()),
    }
}

/// The process's arguments with the defaults file applied, and a log line
/// saying which file was used (or why it was not).
pub fn args_os() -> (Vec<OsString>, Option<String>) {
    let argv: Vec<OsString> = std::env::args_os().collect();
    if argv.iter().skip(1).any(|a| a == NO_ARGS_FILE) {
        return (argv, None);
    }
    let exe = std::env::current_exe().ok();
    let Some(path) = locate(std::env::var_os(ENV), exe.as_deref()) else {
        return (argv, None);
    };
    match std::fs::read_to_string(&path)
        .map_err(|e| e.to_string())
        .and_then(|t| split(&t))
    {
        Ok(words) => {
            let msg = format!(
                "default arguments from {}: {}",
                path.display(),
                words.join(" ")
            );
            (merge(words, argv), Some(msg))
        }
        Err(e) => (argv, Some(format!("{}: {e} (ignored)", path.display()))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn os(v: &[&str]) -> Vec<OsString> {
        v.iter().map(OsString::from).collect()
    }

    #[test]
    fn splits_words_lines_quotes_and_comments() {
        let text = "# kiosk defaults\n--world /var/tmp/ocw/w.ocwpkg   # the pilot\n\n\
                    --cmd 'teleport 3.3' --at \"4=say \\\"hi\\\"\"\n  --name Rider#1\n";
        assert_eq!(
            split(text).unwrap(),
            [
                "--world",
                "/var/tmp/ocw/w.ocwpkg",
                "--cmd",
                "teleport 3.3",
                "--at",
                "4=say \"hi\"",
                "--name",
                "Rider#1",
            ]
        );
        assert_eq!(split("").unwrap(), Vec::<String>::new());
        assert_eq!(split("  # only a comment").unwrap(), Vec::<String>::new());
        assert_eq!(split("a''b \"\"").unwrap(), ["ab", ""]);
        assert!(split("--cmd 'open").unwrap_err().contains("line 1"));
        assert!(split("x\n--cmd \"open").unwrap_err().contains("line 2"));
    }

    #[test]
    fn file_arguments_go_first() {
        let merged = merge(
            vec!["--world".into(), "w.ocwpkg".into()],
            os(&["ocw-desktop", "--virtual"]),
        );
        assert_eq!(
            merged,
            os(&["ocw-desktop", "--world", "w.ocwpkg", "--virtual"])
        );
        assert_eq!(merge(vec!["-x".into()], vec![]), os(&["-x"]));
    }

    #[test]
    fn locates_the_file() {
        let dir = std::env::temp_dir().join(format!("ocw-argsfile-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let exe = dir.join("ocw-desktop");
        // Nothing beside the executable: no file.
        assert_eq!(locate(None, Some(&exe)), None);
        std::fs::write(dir.join(FILE_NAME), "--virtual\n").unwrap();
        assert_eq!(locate(None, Some(&exe)), Some(dir.join(FILE_NAME)));
        // The environment variable wins; empty disables it.
        assert_eq!(
            locate(Some("/etc/x.args".into()), Some(&exe)),
            Some(PathBuf::from("/etc/x.args"))
        );
        assert_eq!(locate(Some("".into()), Some(&exe)), None);
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
