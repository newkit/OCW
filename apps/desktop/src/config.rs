//! One local config file, `ocw.toml`, and its secrets, `secrets.toml`
//! (spec 3.2.4). Bevy-free except the [`ConfigFiles`] resource.
//!
//! - **`ocw.toml`** in the platform's config folder (Linux
//!   `~/.config/ocw/`, Windows `%APPDATA%\ocw\`, macOS `~/Library/Application
//!   Support/ocw/`; `OCW_CONFIG_DIR` overrides it, `--settings-file PATH`
//!   names another file) holds every local setting in sections:
//!   `[settings]` (the settings registry, its groups as `[settings.audio]`,
//!   …), `[riders]` (the local riders, the one *Ride locally* rides),
//!   `[devices]` (remembered trainers and sensors), `[servers]` (the
//!   recent servers of *Connect to a server*), `[start]` (the last start
//!   choice) and `[strava]` (Strava's switches that are not secret). It is
//!   edited in place: a section the app saves replaces only that section,
//!   comments and order elsewhere stay.
//! - **`secrets.toml`** next to it, readable only by the user (mode 0600 on
//!   Unix; on Windows the user's profile folder is private): `[strava]`
//!   (the API application's `client_id` / `client_secret`),
//!   `[strava_token]` (its tokens) and `[account]` (the sign-in session
//!   when no system keyring keeps it). `ocw.toml` never holds a secret
//!   ([`SECRET_KEYS`], checked by a test).
//! - **Migration** ([`migrate`]): the old files of the config folder
//!   (`settings.toml`, `riders.toml`, `devices.toml`, `joins.txt`,
//!   `strava.toml`, `strava-token.toml`, `account.json`) are read into
//!   their sections once and kept as `*.bak` (never deleted); a
//!   `--settings-file` in the old format is converted in place the same
//!   way. An old file that appears again later (written by hand or by an
//!   older script) replaces its section, the file before it kept as
//!   `ocw.toml.bak`.
//!
//! The workshop's garage and hearts ledger (`<data dir>/ocw/workshop.toml`,
//! `hearts-ledger.jsonl`) are game data, not settings, and stay in the data
//! folder; so do the trusted world keys (`trusted-keys.json`, managed by
//! `ocw-worldstore`).

use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use bevy::prelude::Resource;
use toml_edit::{DocumentMut, Item, Table};

/// The config file's name.
pub const CONFIG_FILE: &str = "ocw.toml";
/// The secrets' file, next to it.
pub const SECRETS_FILE: &str = "secrets.toml";

/// Keys that are secrets: never in `ocw.toml` (only in `secrets.toml`).
pub const SECRET_KEYS: &[&str] = &[
    "client_id",
    "client_secret",
    "access_token",
    "refresh_token",
    "session",
    "id_token",
    "password",
    "token",
];

/// The sections of `ocw.toml`.
pub const SECTIONS: &[&str] = &[
    "settings", "riders", "devices", "servers", "start", "strava",
];

/// The first lines of a new `ocw.toml`.
const CONFIG_HEADER: &str = "# Open Cycling World: the local settings of this computer (spec 3.2.4).\n# Edit by hand while the app is closed; secrets are in secrets.toml.\n";
/// The first lines of a new `secrets.toml`.
const SECRETS_HEADER: &str =
    "# Open Cycling World: secrets (readable only by you). Do not share this file.\n";

/// One read-modify-write of a file at a time (the Strava thread saves its
/// tokens while the game saves settings).
static WRITE: Mutex<()> = Mutex::new(());

/// The app's config folder: `OCW_CONFIG_DIR`, else the platform's config
/// directory + `ocw`.
pub fn config_dir() -> Option<PathBuf> {
    match std::env::var_os("OCW_CONFIG_DIR") {
        Some(d) => Some(PathBuf::from(d)),
        None => Some(dirs::config_dir()?.join("ocw")),
    }
}

/// Where the local config lives: `ocw.toml` and `secrets.toml`.
#[derive(Resource, Debug, Clone, PartialEq, Eq)]
pub struct ConfigFiles {
    pub config: PathBuf,
    pub secrets: PathBuf,
}

impl ConfigFiles {
    /// `ocw.toml` and `secrets.toml` in `dir`.
    pub fn in_dir(dir: &Path) -> Self {
        Self {
            config: dir.join(CONFIG_FILE),
            secrets: dir.join(SECRETS_FILE),
        }
    }

    /// `--settings-file PATH` (the whole config file; `secrets.toml` next
    /// to it), else the config folder's (`None`: no config folder).
    pub fn from_args(settings_file: Option<&Path>) -> Option<Self> {
        match settings_file {
            Some(f) => Some(Self {
                config: f.to_path_buf(),
                secrets: f.with_file_name(SECRETS_FILE),
            }),
            None => config_dir().map(|d| Self::in_dir(&d)),
        }
    }

    /// The folder of the old files (the config file's).
    pub fn dir(&self) -> PathBuf {
        self.config
            .parent()
            .map_or_else(|| PathBuf::from("."), Path::to_path_buf)
    }

    /// A section of `ocw.toml`.
    pub fn section(&self, key: &str) -> Slot {
        Slot::Section {
            file: self.config.clone(),
            key: key.into(),
            private: false,
        }
    }

    /// A section of `secrets.toml`.
    pub fn secret(&self, key: &str) -> Slot {
        Slot::Section {
            file: self.secrets.clone(),
            key: key.into(),
            private: true,
        }
    }
}

/// Where one TOML text is kept: a whole file of its own (`--devices-file`,
/// `OCW_ACCOUNT_FILE`, tests) or a section of `ocw.toml` / `secrets.toml`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Slot {
    File(PathBuf),
    Section {
        file: PathBuf,
        key: String,
        /// Written readable only by the user (`secrets.toml`).
        private: bool,
    },
}

impl Slot {
    /// The file it is in.
    pub fn path(&self) -> &Path {
        match self {
            Self::File(p) | Self::Section { file: p, .. } => p,
        }
    }

    /// "~/.config/ocw/ocw.toml [devices]", "~/x/devices.toml".
    pub fn describe(&self) -> String {
        match self {
            Self::File(p) => home_relative(p),
            Self::Section { file, key, .. } => format!("{} [{key}]", home_relative(file)),
        }
    }

    /// The text (a section as a TOML document of its own); `None`: nothing
    /// saved yet.
    pub fn read(&self) -> Result<Option<String>, String> {
        match self {
            Self::File(p) => read_text(p),
            Self::Section { file, key, .. } => {
                let Some(doc) = read_doc(file)? else {
                    return Ok(None);
                };
                section_text(&doc, key).map_err(|e| format!("{}: {e}", file.display()))
            }
        }
    }

    /// Saves `text` (TOML; a section replaces only that section).
    pub fn write(&self, text: &str) -> Result<(), String> {
        match self {
            Self::File(p) => write_file(p, text, false),
            Self::Section { file, key, private } => {
                let _lock = WRITE.lock().unwrap_or_else(|e| e.into_inner());
                let mut doc = read_doc(file)?.unwrap_or_else(|| new_doc(*private));
                set_section(&mut doc, key, text)?;
                write_file(file, &doc.to_string(), *private)
            }
        }
    }

    /// Removes it (a file, or the section).
    pub fn remove(&self) -> Result<(), String> {
        match self {
            Self::File(p) => match std::fs::remove_file(p) {
                Err(e) if e.kind() != std::io::ErrorKind::NotFound => {
                    Err(format!("{}: {e}", p.display()))
                }
                _ => Ok(()),
            },
            Self::Section { file, key, private } => {
                let _lock = WRITE.lock().unwrap_or_else(|e| e.into_inner());
                let Some(mut doc) = read_doc(file)? else {
                    return Ok(());
                };
                if doc.remove(key).is_some() {
                    write_file(file, &doc.to_string(), *private)?;
                }
                Ok(())
            }
        }
    }
}

/// Settings registry keys kept in another section of `ocw.toml` than
/// `[settings]`: (registry group, key, section). `strava.auto_upload` is
/// the one `[strava] auto_upload` (spec 2.8.1 Q6, 3.2.4).
pub const SETTINGS_ELSEWHERE: &[(&str, &str, &str)] = &[("strava", "auto_upload", "strava")];

/// The settings' text store: `ocw.toml [settings]`, with the keys of
/// [`SETTINGS_ELSEWHERE`] read from and written to their own sections
/// (`[strava] auto_upload`), so each switch lives in one place.
pub struct SettingsStore {
    pub files: ConfigFiles,
}

impl SettingsStore {
    fn table(slot: &Slot) -> Result<toml::Table, String> {
        match slot.read()? {
            Some(t) => toml::from_str(&t).map_err(|e| format!("{}: {e}", slot.describe())),
            None => Ok(toml::Table::new()),
        }
    }
}

impl ocw_ui::settings::TextStore for SettingsStore {
    fn load(&self) -> Result<Option<String>, String> {
        let settings = self.files.section("settings");
        let text = settings.read()?;
        let mut moved = Vec::new();
        for &(group, key, section) in SETTINGS_ELSEWHERE {
            if let Some(v) = Self::table(&self.files.section(section))?.get(key) {
                moved.push((group, key, v.clone()));
            }
        }
        if moved.is_empty() {
            return Ok(text);
        }
        let mut t: toml::Table = match &text {
            Some(t) => toml::from_str(t).map_err(|e| format!("{}: {e}", settings.describe()))?,
            None => toml::Table::new(),
        };
        for (group, key, v) in moved {
            let g = t
                .entry(group)
                .or_insert_with(|| toml::Value::Table(toml::Table::new()));
            if let Some(g) = g.as_table_mut() {
                g.insert(key.into(), v);
            }
        }
        toml::to_string(&t).map(Some).map_err(|e| e.to_string())
    }

    fn save(&mut self, text: &str) -> Result<(), String> {
        let mut t: toml::Table = toml::from_str(text).map_err(|e| e.to_string())?;
        let mut any = false;
        for &(group, key, section) in SETTINGS_ELSEWHERE {
            let v = t
                .get_mut(group)
                .and_then(toml::Value::as_table_mut)
                .and_then(|g| g.remove(key));
            any |= v.is_some();
            if t.get(group)
                .and_then(toml::Value::as_table)
                .is_some_and(toml::Table::is_empty)
            {
                t.remove(group);
            }
            let slot = self.files.section(section);
            let mut own = Self::table(&slot)?;
            let changed = match v {
                Some(v) => own.insert(key.into(), v.clone()) != Some(v),
                None => own.remove(key).is_some(),
            };
            if changed {
                slot.write(&toml::to_string(&own).map_err(|e| e.to_string())?)?;
            }
        }
        if !any {
            // Nothing kept elsewhere: the text as it is.
            return self.files.section("settings").write(text);
        }
        self.files
            .section("settings")
            .write(&toml::to_string(&t).map_err(|e| e.to_string())?)
    }

    fn describe(&self) -> String {
        format!("file: {}", self.files.section("settings").describe())
    }
}

/// The recent servers (`[[servers.recent]]` with `link` and `world`,
/// newest first) as the Join page's text store (its lines: link, a tab,
/// what the server rode).
pub struct ServersStore(pub Slot);

impl ocw_ui::settings::TextStore for ServersStore {
    fn load(&self) -> Result<Option<String>, String> {
        let Some(text) = self.0.read()? else {
            return Ok(None);
        };
        let t: toml::Table = toml::from_str(&text).map_err(|e| e.to_string())?;
        let lines: Vec<String> = t
            .get("recent")
            .and_then(toml::Value::as_array)
            .map(|a| {
                a.iter()
                    .filter_map(|e| {
                        let link = e.get("link")?.as_str()?;
                        let world = e.get("world").and_then(toml::Value::as_str).unwrap_or("");
                        Some(format!("{link}\t{world}"))
                    })
                    .collect()
            })
            .unwrap_or_default();
        Ok(Some(lines.join("\n")))
    }
    fn save(&mut self, text: &str) -> Result<(), String> {
        self.0.write(&servers_text(text))
    }
    fn describe(&self) -> String {
        format!("file: {}", self.0.describe())
    }
}

/// `[servers]` from the lines of the old `joins.txt` (link, a tab, what
/// the server rode).
fn servers_text(lines: &str) -> String {
    let recent: Vec<toml::Value> = lines
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .map(|l| {
            let (link, world) = l.split_once('\t').unwrap_or((l, ""));
            let mut e = toml::Table::new();
            e.insert("link".into(), toml::Value::String(link.trim().into()));
            e.insert("world".into(), toml::Value::String(world.trim().into()));
            toml::Value::Table(e)
        })
        .collect();
    let mut t = toml::Table::new();
    t.insert("recent".into(), toml::Value::Array(recent));
    toml::to_string(&t).expect("a table serializes")
}

fn home_relative(p: &Path) -> String {
    match dirs::home_dir().and_then(|h| p.strip_prefix(h).ok().map(Path::to_path_buf)) {
        Some(rest) => format!("~/{}", rest.display()),
        None => p.display().to_string(),
    }
}

fn read_text(p: &Path) -> Result<Option<String>, String> {
    match std::fs::read_to_string(p) {
        Ok(t) => Ok(Some(t)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(format!("{}: {e}", p.display())),
    }
}

fn read_doc(p: &Path) -> Result<Option<DocumentMut>, String> {
    match read_text(p)? {
        Some(t) => t
            .parse::<DocumentMut>()
            .map(Some)
            .map_err(|e| format!("{}: {e}", p.display())),
        None => Ok(None),
    }
}

fn new_doc(private: bool) -> DocumentMut {
    let mut doc = DocumentMut::new();
    doc.decor_mut().set_prefix(if private {
        SECRETS_HEADER
    } else {
        CONFIG_HEADER
    });
    doc
}

/// Writes through a temporary file (a crash never leaves half a file);
/// `private`: readable only by the user.
fn write_file(path: &Path, text: &str, private: bool) -> Result<(), String> {
    let err = |e: std::io::Error| format!("{}: {e}", path.display());
    if let Some(dir) = path.parent()
        && !dir.as_os_str().is_empty()
    {
        std::fs::create_dir_all(dir).map_err(err)?;
    }
    let mut tmp = path.as_os_str().to_owned();
    tmp.push(".part");
    let tmp = PathBuf::from(tmp);
    let mut opts = std::fs::OpenOptions::new();
    opts.write(true).create(true).truncate(true);
    #[cfg(unix)]
    if private {
        use std::os::unix::fs::OpenOptionsExt;
        opts.mode(0o600);
    }
    let mut f = opts.open(&tmp).map_err(err)?;
    #[cfg(unix)]
    if private {
        // The mode only applies to new files.
        use std::os::unix::fs::PermissionsExt;
        f.set_permissions(std::fs::Permissions::from_mode(0o600))
            .map_err(err)?;
    }
    #[cfg(not(unix))]
    let _ = private;
    f.write_all(text.as_bytes()).map_err(err)?;
    f.sync_all().map_err(err)?;
    drop(f);
    std::fs::rename(&tmp, path).map_err(err)
}

/// The section `key` of `doc` as a document of its own.
pub fn section_text(doc: &DocumentMut, key: &str) -> Result<Option<String>, String> {
    let Some(item) = doc.get(key) else {
        return Ok(None);
    };
    let Some(table) = item.as_table() else {
        return Err(format!("{key} is not a section"));
    };
    let mut out = DocumentMut::new();
    for (k, v) in table.iter() {
        out.insert(k, v.clone());
    }
    Ok(Some(out.to_string()))
}

/// Replaces the section `key` of `doc` with `text` (a TOML document);
/// the comment above the section and above its tables stays.
pub fn set_section(doc: &mut DocumentMut, key: &str, text: &str) -> Result<(), String> {
    let parsed: DocumentMut = text.parse().map_err(|e| format!("[{key}]: {e}"))?;
    let mut t: Table = parsed.as_table().clone();
    clear_positions(&mut t);
    let has_values = t.iter().any(|(_, v)| v.is_value());
    t.set_implicit(!has_values && !t.is_empty());
    if let Some(old) = doc.get(key).and_then(Item::as_table) {
        *t.decor_mut() = old.decor().clone();
        keep_decor(&mut t, old);
    } else if t
        .decor()
        .prefix()
        .is_none_or(|p| p.as_str().is_none_or(str::is_empty))
    {
        t.decor_mut().set_prefix("\n");
    }
    // Tables after the first get a blank line before them.
    let bare = |d: &toml_edit::Decor| {
        d.prefix()
            .is_none_or(|p| p.as_str().is_none_or(str::is_empty))
    };
    for (_, v) in t.iter_mut() {
        match v {
            Item::Table(sub) if bare(sub.decor()) => sub.decor_mut().set_prefix("\n"),
            Item::ArrayOfTables(a) => {
                if let Some(first) = a.iter_mut().next()
                    && bare(first.decor())
                {
                    first.decor_mut().set_prefix("\n");
                }
            }
            _ => {}
        }
    }
    doc.insert(key, Item::Table(t));
    Ok(())
}

/// The comments of `old` kept in `new`: the decor of every table, key and
/// value that is in both (a comment above `master = 30` in
/// `[settings.audio]` stays when the app saves the settings, spec 3.2.4,
/// 3.2.5).
fn keep_decor(new: &mut Table, old: &Table) {
    let keys: Vec<String> = new.iter().map(|(k, _)| k.to_string()).collect();
    for k in keys {
        let Some(o) = old.get(&k) else {
            continue;
        };
        if let (Some(mut nk), Some(ok)) = (new.key_mut(&k), old.key(&k)) {
            *nk.leaf_decor_mut() = ok.leaf_decor().clone();
        }
        match (new.get_mut(&k), o) {
            (Some(Item::Table(n)), Item::Table(o)) => {
                *n.decor_mut() = o.decor().clone();
                keep_decor(n, o);
            }
            (Some(Item::Value(n)), Item::Value(o)) => *n.decor_mut() = o.decor().clone(),
            _ => {}
        }
    }
}

/// Tables without a position are written after the table before them:
/// the section stays together wherever it is in the file.
fn clear_positions(t: &mut Table) {
    t.set_position(None);
    for (_, item) in t.iter_mut() {
        match item {
            Item::Table(sub) => clear_positions(sub),
            Item::ArrayOfTables(a) => {
                for sub in a.iter_mut() {
                    clear_positions(sub);
                }
            }
            _ => {}
        }
    }
}

/// The keys of a TOML text, dotted ("strava.client_id"), for the secret
/// check.
#[cfg(test)]
pub fn keys(text: &str) -> Vec<String> {
    fn walk(prefix: &str, v: &toml::Value, out: &mut Vec<String>) {
        match v {
            toml::Value::Table(t) => {
                for (k, v) in t {
                    let key = if prefix.is_empty() {
                        k.clone()
                    } else {
                        format!("{prefix}.{k}")
                    };
                    out.push(key.clone());
                    walk(&key, v, out);
                }
            }
            toml::Value::Array(a) => {
                for v in a {
                    walk(prefix, v, out);
                }
            }
            _ => {}
        }
    }
    let mut out = Vec::new();
    if let Ok(t) = toml::from_str::<toml::Table>(text) {
        walk("", &toml::Value::Table(t), &mut out);
    }
    out
}

/// The secret keys found in a TOML text (none for a good `ocw.toml`).
#[cfg(test)]
pub fn secrets_in(text: &str) -> Vec<String> {
    keys(text)
        .into_iter()
        .filter(|k| {
            let last = k.rsplit('.').next().unwrap_or(k);
            SECRET_KEYS.contains(&last)
        })
        .collect()
}

/// How an old file goes into its section.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Old {
    /// The TOML file is the section (`ocw.toml`).
    Config(&'static str),
    /// … a section of `secrets.toml`.
    Secret(&'static str),
    /// `joins.txt`: one recent server per line → `[servers] recent`.
    Servers,
    /// `strava.toml`: the credentials → `secrets.toml [strava]`, the rest
    /// (`auto_upload`) → `ocw.toml [strava]`.
    Strava,
    /// `account.json` (the sign-in session) → `secrets.toml [account]
    /// session = "<json>"`.
    Account,
}

/// The old files of the config folder and where they go.
const OLD_FILES: &[(&str, Old)] = &[
    ("settings.toml", Old::Config("settings")),
    ("riders.toml", Old::Config("riders")),
    ("devices.toml", Old::Config("devices")),
    ("joins.txt", Old::Servers),
    ("strava.toml", Old::Strava),
    ("strava-token.toml", Old::Secret("strava_token")),
    ("account.json", Old::Account),
];

/// The Strava files only (looked at again while the app runs: the
/// credentials may come later, spec 2.8.1).
pub const STRAVA_FILES: &[&str] = &["strava.toml", "strava-token.toml"];

/// The settings registry's groups (`[audio]`, `[display]`, …): a config
/// file with one of them at the top is an old `settings.toml`.
fn is_old_settings(doc: &DocumentMut) -> bool {
    let groups: Vec<&str> = ocw_client::settings::SETTINGS
        .iter()
        .filter_map(|d| d.key.split_once('.').map(|(g, _)| g))
        .collect();
    doc.iter()
        .any(|(k, _)| groups.contains(&k) && !SECTIONS.contains(&k))
}

/// `path.bak`, or `path.bak.2`, … if that exists (nothing is overwritten).
fn backup_path(path: &Path) -> PathBuf {
    let mut n = 1;
    loop {
        let mut s = path.as_os_str().to_owned();
        s.push(if n == 1 {
            ".bak".to_string()
        } else {
            format!(".bak.{n}")
        });
        let p = PathBuf::from(s);
        if !p.exists() {
            return p;
        }
        n += 1;
    }
}

/// Copies `path` to a backup (before it is changed); the backup's path.
fn back_up_copy(path: &Path) -> Result<Option<PathBuf>, String> {
    if !path.exists() {
        return Ok(None);
    }
    let to = backup_path(path);
    std::fs::copy(path, &to).map_err(|e| format!("{}: {e}", path.display()))?;
    Ok(Some(to))
}

/// Reads the old files of the config folder into `ocw.toml` and
/// `secrets.toml` (spec 3.2.4); each is kept as `*.bak`. Idempotent: an
/// old file is gone after it was read, so a second run does nothing.
/// Returns one line per file moved (for the log) and the problems (a
/// broken old file stays where it is).
pub fn migrate(files: &ConfigFiles) -> (Vec<String>, Vec<String>) {
    migrate_only(files, None)
}

/// [`migrate`] for the files named in `only` (`None`: all).
pub fn migrate_only(files: &ConfigFiles, only: Option<&[&str]>) -> (Vec<String>, Vec<String>) {
    let mut notes = Vec::new();
    let mut problems = Vec::new();
    // A --settings-file in the old format: converted in place.
    if only.is_none()
        && let Err(e) = convert_old_config(files, &mut notes)
    {
        problems.push(e);
    }
    let dir = files.dir();
    for (name, how) in OLD_FILES {
        if only.is_some_and(|o| !o.contains(name)) {
            continue;
        }
        let old = dir.join(name);
        if old == files.config || old == files.secrets || !old.is_file() {
            continue;
        }
        match migrate_file(files, &old, *how) {
            Ok(note) => notes.push(note),
            Err(e) => problems.push(format!("{}: {e}; left where it is", home_relative(&old))),
        }
    }
    (notes, problems)
}

fn convert_old_config(files: &ConfigFiles, notes: &mut Vec<String>) -> Result<(), String> {
    let Some(doc) = read_doc(&files.config).unwrap_or(None) else {
        return Ok(());
    };
    if !is_old_settings(&doc) {
        return Ok(());
    }
    let _lock = WRITE.lock().unwrap_or_else(|e| e.into_inner());
    let text = doc.to_string();
    let bak = back_up_copy(&files.config)?.expect("it exists");
    let mut new = new_doc(false);
    set_section(&mut new, "settings", &text)?;
    write_file(&files.config, &new.to_string(), false)?;
    notes.push(format!(
        "{}: an old settings file, now its [settings] section (the old one kept as {})",
        home_relative(&files.config),
        home_relative(&bak)
    ));
    Ok(())
}

/// One old file into its section(s); then it is renamed to `*.bak`.
fn migrate_file(files: &ConfigFiles, old: &Path, how: Old) -> Result<String, String> {
    let text = std::fs::read_to_string(old).map_err(|e| e.to_string())?;
    // (slot, text) to write.
    let mut writes: Vec<(Slot, String)> = Vec::new();
    match how {
        Old::Config(key) => {
            toml::from_str::<toml::Table>(&text).map_err(|e| e.to_string())?;
            writes.push((files.section(key), text.clone()));
        }
        Old::Secret(key) => {
            toml::from_str::<toml::Table>(&text).map_err(|e| e.to_string())?;
            writes.push((files.secret(key), text.clone()));
        }
        Old::Servers => writes.push((files.section("servers"), servers_text(&text))),
        Old::Strava => {
            let t: toml::Table = toml::from_str(&text).map_err(|e| e.to_string())?;
            let (secret, rest): (toml::Table, toml::Table) = t
                .into_iter()
                .partition(|(k, _)| SECRET_KEYS.contains(&k.as_str()));
            // The example file (everything commented out) has neither.
            if !secret.is_empty() {
                let mut s = section_table(files.secret("strava"))?;
                s.extend(secret);
                writes.push((
                    files.secret("strava"),
                    toml::to_string(&s).expect("a table"),
                ));
            }
            if !rest.is_empty() {
                let mut s = section_table(files.section("strava"))?;
                s.extend(rest);
                writes.push((
                    files.section("strava"),
                    toml::to_string(&s).expect("a table"),
                ));
            }
        }
        Old::Account => {
            serde_json::from_str::<serde_json::Value>(&text).map_err(|e| e.to_string())?;
            let mut t = toml::Table::new();
            t.insert("session".into(), toml::Value::String(text.trim().into()));
            writes.push((
                files.secret("account"),
                toml::to_string(&t).expect("a table"),
            ));
        }
    }
    // A section that is there already is replaced (the old file came
    // again: it is newer); the file before that change is kept.
    let mut replaced = Vec::new();
    for (slot, _) in &writes {
        if matches!(how, Old::Strava) {
            continue;
        }
        if slot.read().ok().flatten().is_some()
            && let Some(b) = back_up_copy(slot.path())?
        {
            replaced.push(home_relative(&b));
        }
    }
    for (slot, text) in &writes {
        slot.write(text)?;
    }
    let bak = backup_path(old);
    std::fs::rename(old, &bak).map_err(|e| e.to_string())?;
    let to: Vec<String> = writes.iter().map(|(s, _)| s.describe()).collect();
    let mut note = format!(
        "{} → {} (kept as {})",
        home_relative(old),
        if to.is_empty() {
            "nothing to keep".to_string()
        } else {
            to.join(", ")
        },
        bak.file_name().unwrap_or_default().to_string_lossy()
    );
    if !replaced.is_empty() {
        note.push_str(&format!(
            "; it replaced the section there (before: {})",
            replaced.join(", ")
        ));
    }
    Ok(note)
}

/// A section's table (empty if none).
fn section_table(slot: Slot) -> Result<toml::Table, String> {
    match slot.read()? {
        Some(t) => toml::from_str(&t).map_err(|e| e.to_string()),
        None => Ok(toml::Table::new()),
    }
}

/// The last start choice (`[start] choice`, spec 3.2.4 Q3).
pub fn load_start_choice(files: &ConfigFiles) -> Option<ocw_client::menu::start::Choice> {
    let t = section_table(files.section("start")).ok()?;
    ocw_client::menu::start::Choice::parse(t.get("choice")?.as_str()?)
}

/// Saves the start choice.
pub fn save_start_choice(
    files: &ConfigFiles,
    choice: ocw_client::menu::start::Choice,
) -> Result<(), String> {
    let slot = files.section("start");
    let mut t = section_table(slot.clone())?;
    t.insert("choice".into(), toml::Value::String(choice.id().into()));
    slot.write(&toml::to_string(&t).expect("a table"))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A fresh temporary config folder (never the real home).
    pub fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("ocw-config-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn listing(dir: &Path) -> Vec<String> {
        let mut v: Vec<String> = std::fs::read_dir(dir)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        v.sort();
        v
    }

    /// The setting `strava.auto_upload` (spec 2.8.1 Q6) is the one
    /// `[strava] auto_upload` of `ocw.toml`: read from there into the
    /// settings, saved there – never a second key in `[settings]`; the
    /// other keys of `[strava]` stay; other settings as they are.
    #[test]
    fn the_strava_switch_is_the_one_strava_auto_upload() {
        use ocw_ui::settings::TextStore;
        let dir = temp_dir("auto-upload");
        let files = ConfigFiles::in_dir(&dir);
        files
            .section("strava")
            .write("auto_upload = false\nnote = \"kept\"\n")
            .unwrap();
        files.section("settings").write(SETTINGS).unwrap();
        let mut store = SettingsStore {
            files: files.clone(),
        };
        let text = store.load().unwrap().unwrap();
        let (s, _) = ocw_client::settings::Settings::parse(&text).unwrap();
        assert_eq!(
            s.get("strava.auto_upload").unwrap(),
            ocw_client::settings::Value::Bool(false)
        );
        assert_eq!(
            s.get("ride.bots").unwrap(),
            ocw_client::settings::Value::Int(3)
        );
        // Switched on and saved: [strava] changes, [settings] has no copy.
        let mut s = s;
        s.set("strava.auto_upload", "on").unwrap();
        store.save(&s.to_toml()).unwrap();
        let strava: toml::Table =
            toml::from_str(&files.section("strava").read().unwrap().unwrap()).unwrap();
        assert_eq!(strava["auto_upload"].as_bool(), Some(true));
        assert_eq!(strava["note"].as_str(), Some("kept"));
        let settings = files.section("settings").read().unwrap().unwrap();
        assert!(!settings.contains("auto_upload"), "{settings}");
        assert!(settings.contains("bots = 3"), "{settings}");
        let whole = std::fs::read_to_string(&files.config).unwrap();
        assert_eq!(whole.matches("auto_upload").count(), 1, "{whole}");
        // Nothing in [strava]: the settings text as it is.
        let other = ConfigFiles::in_dir(&temp_dir("auto-upload-none"));
        other.section("settings").write(SETTINGS).unwrap();
        let st = SettingsStore { files: other };
        let t = st.load().unwrap().unwrap();
        assert!(t.contains("bots = 3") && !t.contains("auto_upload"), "{t}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    const SETTINGS: &str = "[audio]\nmaster = 40\n\n[ride]\nbots = 3\n";
    const RIDERS: &str = "active = \"Ada Rider\"\n\n[[rider]]\nname = \"Ada Rider\"\nweight_kg = 61.0\nheight_cm = 168.0\nftp_w = 230\n\n[[rider]]\nname = \"Bea Climber\"\nweight_kg = 55.0\n";
    const DEVICES: &str =
        "[[device]]\nid = \"hci0/AA:BB\"\nname = \"Made-up KICKR\"\nkind = \"trainer\"\n";
    const STRAVA: &str =
        "client_id = \"12345\"\nclient_secret = \"made-up-secret\"\nauto_upload = false\n";
    const TOKEN: &str =
        "access_token = \"a-made-up\"\nrefresh_token = \"r-made-up\"\nexpires_at = 1800000000\n";
    const ACCOUNT: &str =
        "{\"issuer\":\"https://accounts.example.org\",\"access_token\":\"made-up\"}";

    fn old_files(dir: &Path) {
        std::fs::write(dir.join("settings.toml"), SETTINGS).unwrap();
        std::fs::write(dir.join("riders.toml"), RIDERS).unwrap();
        std::fs::write(dir.join("devices.toml"), DEVICES).unwrap();
        std::fs::write(
            dir.join("joins.txt"),
            "ocw://127.0.0.1:47800\troute hills\n",
        )
        .unwrap();
        std::fs::write(dir.join("strava.toml"), STRAVA).unwrap();
        std::fs::write(dir.join("strava-token.toml"), TOKEN).unwrap();
        std::fs::write(dir.join("account.json"), ACCOUNT).unwrap();
    }

    #[test]
    fn every_old_file_moves_into_its_section_and_is_kept_as_bak() {
        let dir = temp_dir("migrate");
        old_files(&dir);
        let files = ConfigFiles::in_dir(&dir);
        let (notes, problems) = migrate(&files);
        assert!(problems.is_empty(), "{problems:?}");
        assert_eq!(notes.len(), 7, "{notes:?}");
        assert_eq!(
            listing(&dir),
            [
                "account.json.bak",
                "devices.toml.bak",
                "joins.txt.bak",
                "ocw.toml",
                "riders.toml.bak",
                "secrets.toml",
                "settings.toml.bak",
                "strava-token.toml.bak",
                "strava.toml.bak",
            ]
        );
        // The backups are the old files, unchanged.
        assert_eq!(
            std::fs::read_to_string(dir.join("settings.toml.bak")).unwrap(),
            SETTINGS
        );
        // Each section reads back as the old file did.
        let s = files.section("settings").read().unwrap().unwrap();
        let (s, w) = ocw_client::settings::Settings::parse(&s).unwrap();
        assert!(w.is_empty(), "{w:?}");
        assert_eq!(s.get("audio.master").unwrap().to_string(), "40");
        let r = files.section("riders").read().unwrap().unwrap();
        let (r, _) = ocw_client::riders::Riders::parse(&r).unwrap();
        assert_eq!(r.riders.len(), 2);
        assert_eq!(r.active().name, "Ada Rider");
        assert_eq!(r.active().ftp_w, Some(230));
        let d = files.section("devices").read().unwrap().unwrap();
        assert!(d.contains("Made-up KICKR"), "{d}");
        let servers = ServersStore(files.section("servers"));
        use ocw_ui::settings::TextStore;
        assert_eq!(
            servers.load().unwrap().as_deref(),
            Some("ocw://127.0.0.1:47800\troute hills")
        );
        // Strava: the credentials are secrets, the switch is not.
        let sec = files.secret("strava").read().unwrap().unwrap();
        assert!(sec.contains("client_secret = \"made-up-secret\""), "{sec}");
        assert!(!sec.contains("auto_upload"), "{sec}");
        let st = files.section("strava").read().unwrap().unwrap();
        assert_eq!(st.trim(), "auto_upload = false");
        let tok = files.secret("strava_token").read().unwrap().unwrap();
        assert!(tok.contains("r-made-up"));
        let acc = files.secret("account").read().unwrap().unwrap();
        let t: toml::Table = toml::from_str(&acc).unwrap();
        assert_eq!(t["session"].as_str(), Some(ACCOUNT));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn migrating_twice_changes_nothing() {
        let dir = temp_dir("twice");
        old_files(&dir);
        let files = ConfigFiles::in_dir(&dir);
        migrate(&files);
        let config = std::fs::read_to_string(&files.config).unwrap();
        let secrets = std::fs::read_to_string(&files.secrets).unwrap();
        let before = listing(&dir);
        let (notes, problems) = migrate(&files);
        assert!(
            notes.is_empty() && problems.is_empty(),
            "{notes:?} {problems:?}"
        );
        assert_eq!(listing(&dir), before);
        assert_eq!(std::fs::read_to_string(&files.config).unwrap(), config);
        assert_eq!(std::fs::read_to_string(&files.secrets).unwrap(), secrets);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn an_old_file_that_comes_again_replaces_its_section_and_keeps_both() {
        let dir = temp_dir("again");
        let files = ConfigFiles::in_dir(&dir);
        std::fs::write(dir.join("devices.toml"), DEVICES).unwrap();
        migrate(&files);
        // A deploy script writes devices.toml again.
        std::fs::write(
            dir.join("devices.toml"),
            "[[device]]\nid = \"x\"\nname = \"Second made-up trainer\"\n",
        )
        .unwrap();
        let (notes, _) = migrate(&files);
        assert!(notes[0].contains("replaced"), "{notes:?}");
        assert!(dir.join("devices.toml.bak").exists());
        assert!(dir.join("devices.toml.bak.2").exists());
        assert!(dir.join("ocw.toml.bak").exists());
        let d = files.section("devices").read().unwrap().unwrap();
        assert!(
            d.contains("Second made-up trainer") && !d.contains("KICKR"),
            "{d}"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_broken_old_file_stays_where_it_is() {
        let dir = temp_dir("broken");
        std::fs::write(dir.join("riders.toml"), "[[rider\n").unwrap();
        let (notes, problems) = migrate(&ConfigFiles::in_dir(&dir));
        assert!(notes.is_empty());
        assert_eq!(problems.len(), 1);
        assert!(dir.join("riders.toml").exists());
        assert!(!dir.join("ocw.toml").exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn an_old_settings_file_given_with_settings_file_is_converted_in_place() {
        let dir = temp_dir("inplace");
        let f = dir.join("kiosk-settings.toml");
        std::fs::write(&f, SETTINGS).unwrap();
        std::fs::write(dir.join("riders.toml"), RIDERS).unwrap();
        let files = ConfigFiles::from_args(Some(&f)).unwrap();
        assert_eq!(files.secrets, dir.join("secrets.toml"));
        let (notes, problems) = migrate(&files);
        assert!(problems.is_empty(), "{problems:?}");
        assert_eq!(notes.len(), 2, "{notes:?}");
        assert_eq!(
            std::fs::read_to_string(dir.join("kiosk-settings.toml.bak")).unwrap(),
            SETTINGS
        );
        let text = std::fs::read_to_string(&f).unwrap();
        assert!(text.contains("[settings.audio]"), "{text}");
        assert!(text.contains("[[riders.rider]]"), "{text}");
        // A new-format file is left alone.
        let (notes, _) = migrate(&files);
        assert!(notes.is_empty(), "{notes:?}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_section_saved_keeps_the_rest_of_the_file_and_its_comments() {
        let dir = temp_dir("edit");
        let files = ConfigFiles::in_dir(&dir);
        std::fs::write(
            &files.config,
            "# my notes\n[settings.audio]\n# quieter at night\nmaster = 30\n\n[start]\nchoice = \"local\"\n",
        )
        .unwrap();
        files
            .section("devices")
            .write("[[device]]\nid = \"a\"\nname = \"Made-up power meter\"\n")
            .unwrap();
        files
            .section("settings")
            .write("[audio]\nmaster = 50\n")
            .unwrap();
        let text = std::fs::read_to_string(&files.config).unwrap();
        assert!(text.starts_with("# my notes\n[settings.audio]"), "{text}");
        assert!(text.contains("master = 50"), "{text}");
        assert!(text.contains("[start]\nchoice = \"local\""), "{text}");
        assert!(text.contains("\n\n[[devices.device]]"), "{text}");
        // The order of the sections stays: settings, start, devices.
        let (a, b, c) = (
            text.find("[settings.audio]").unwrap(),
            text.find("[start]").unwrap(),
            text.find("[[devices.device]]").unwrap(),
        );
        assert!(a < b && b < c, "{text}");
        // Reads back.
        assert_eq!(
            files.section("devices").read().unwrap().unwrap().trim(),
            "[[device]]\nid = \"a\"\nname = \"Made-up power meter\""
        );
        files.section("devices").remove().unwrap();
        assert_eq!(files.section("devices").read().unwrap(), None);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Spec 3.2.5: the input mapping is saved in `ocw.toml`
    /// (`[settings.keys]`, `[settings.pad]`) with only the changed
    /// bindings; the other sections and the comments stay; a reset takes
    /// the tables out again.
    #[test]
    fn the_input_mapping_is_saved_in_ocw_toml_with_only_the_changes() {
        use ocw_client::bindings::{Action, Bindings, Device};
        use ocw_client::settings::Settings;
        use ocw_ui::settings::TextStore;
        let dir = temp_dir("input-mapping");
        let files = ConfigFiles::in_dir(&dir);
        std::fs::write(
            &files.config,
            "# my notes\n[settings.audio]\n# quieter at night\nmaster = 30\n\n[start]\n# the last choice\nchoice = \"local\"\n",
        )
        .unwrap();
        let mut store = SettingsStore {
            files: files.clone(),
        };
        let load = |store: &SettingsStore| {
            Settings::parse(&store.load().unwrap().unwrap_or_default())
                .unwrap()
                .0
        };
        let mut s = load(&store);
        // A capture of W for autodrive: wild mode gets A (the swap).
        let r = Bindings::default().assign(Action::Autodrive, Device::Key, "W", true);
        let r2 = Bindings::default().assign(Action::UTurn, Device::Pad, "LB", true);
        for (k, v) in r.changes.into_iter().chain(r2.changes) {
            match v {
                Some(v) => s.set(&k, &v).map(|_| ()),
                None => s.unset(&k),
            }
            .unwrap();
        }
        store.save(&s.to_toml()).unwrap();
        let text = std::fs::read_to_string(&files.config).unwrap();
        assert!(text.starts_with("# my notes\n[settings.audio]"), "{text}");
        assert!(text.contains("# quieter at night\nmaster = 30"), "{text}");
        assert!(
            text.contains("[start]\n# the last choice\nchoice = \"local\""),
            "{text}"
        );
        assert!(text.contains("[settings.keys]"), "{text}");
        assert!(text.contains("autodrive = \"W\""), "{text}");
        assert!(text.contains("wild = \"A\""), "{text}");
        assert!(text.contains("[settings.pad]\nu_turn = \"LB\""), "{text}");
        // Only the three changed bindings, none of the defaults.
        for id in ["faster", "exit_left", "camera", "map"] {
            assert!(!text.contains(&format!("{id} =")), "{id}: {text}");
        }
        // Read again: the same bindings.
        let back = Bindings::from_settings(&load(&store));
        assert_eq!(back, Bindings::from_settings(&s));
        assert_eq!(back.key(Action::Wild), "A");
        assert_eq!(back.conflicts(true), []);
        // Reset: the tables go, the rest stays.
        let mut s = load(&store);
        for k in Bindings::setting_keys() {
            s.unset(&k).unwrap();
        }
        store.save(&s.to_toml()).unwrap();
        let text = std::fs::read_to_string(&files.config).unwrap();
        assert!(!text.contains("[settings.keys]"), "{text}");
        assert!(!text.contains("[settings.pad]"), "{text}");
        assert!(text.contains("# quieter at night\nmaster = 30"), "{text}");
        assert!(text.contains("# the last choice"), "{text}");
        assert!(Bindings::from_settings(&load(&store)).is_default());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn secrets_are_only_in_secrets_toml_and_private() {
        let dir = temp_dir("secrets");
        old_files(&dir);
        let files = ConfigFiles::in_dir(&dir);
        migrate(&files);
        let config = std::fs::read_to_string(&files.config).unwrap();
        assert_eq!(secrets_in(&config), Vec::<String>::new(), "{config}");
        assert!(!config.contains("made-up-secret") && !config.contains("r-made-up"));
        let secrets = std::fs::read_to_string(&files.secrets).unwrap();
        let found = secrets_in(&secrets);
        for k in [
            "strava.client_id",
            "strava.client_secret",
            "strava_token.access_token",
            "strava_token.refresh_token",
            "account.session",
        ] {
            assert!(found.iter().any(|f| f == k), "{k} in {found:?}");
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&files.secrets)
                .unwrap()
                .permissions()
                .mode();
            assert_eq!(mode & 0o777, 0o600);
            // Also after a later save.
            files.secret("strava_token").write(TOKEN).unwrap();
            let mode = std::fs::metadata(&files.secrets)
                .unwrap()
                .permissions()
                .mode();
            assert_eq!(mode & 0o777, 0o600);
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_start_choice_is_kept() {
        use ocw_client::menu::start::Choice;
        let dir = temp_dir("start");
        let files = ConfigFiles::in_dir(&dir);
        assert_eq!(load_start_choice(&files), None);
        save_start_choice(&files, Choice::Server).unwrap();
        assert_eq!(load_start_choice(&files), Some(Choice::Server));
        save_start_choice(&files, Choice::Local).unwrap();
        assert_eq!(load_start_choice(&files), Some(Choice::Local));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_recent_servers_round_trip() {
        use ocw_ui::settings::TextStore;
        let dir = temp_dir("servers");
        let files = ConfigFiles::in_dir(&dir);
        let mut s = ServersStore(files.section("servers"));
        assert_eq!(s.load().unwrap(), None);
        // The Join page's own text (`RecentJoins::to_text`).
        let mut recent = ocw_client::join::RecentJoins::default();
        recent.push(
            &ocw_protocol::join::JoinLink::new("127.0.0.1", 47800),
            "route hills",
        );
        recent.push(
            &ocw_protocol::join::JoinLink::new("rides.example.org", 47800),
            "Black Forest north",
        );
        s.save(&recent.to_text()).unwrap();
        let again = ocw_client::join::RecentJoins::parse(&s.load().unwrap().unwrap());
        assert_eq!(again.entries, recent.entries);
        let text = std::fs::read_to_string(&files.config).unwrap();
        assert!(text.contains("[[servers.recent]]"), "{text}");
        assert!(text.contains("world = \"Black Forest north\""), "{text}");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
