//! The desktop app's start choice (spec 3.2.4 Q3, Q5): *Ride on the
//! public server* (spec 11.5: for now ocw-server.fly.dev; `net.public_server
//! off` hides it), *Connect to a server* (Ride → Join: an address field and
//! the recent servers) or *Ride locally* (the local server, the local rider
//! of `ocw.toml`, Training mode – still the default: a solo ride never goes
//! online by itself). Asked at every start the command line does not decide,
//! the last choice preselected so Enter takes it. The browser app keeps
//! its own start page (Q6): this page is the desktop's.

use super::pages::{Activate, Page, Row};

/// What the rider chose at the start.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Choice {
    /// The public server (spec 11.5): its join info, then Ride → Join.
    Public,
    Server,
    /// The default before anything was chosen.
    #[default]
    Local,
}

impl Choice {
    /// Its id in `ocw.toml` (`[start] choice`) and the console.
    pub fn id(self) -> &'static str {
        match self {
            Self::Public => "public",
            Self::Server => "server",
            Self::Local => "local",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "public" => Some(Self::Public),
            "server" | "connect" => Some(Self::Server),
            "local" | "locally" => Some(Self::Local),
            _ => None,
        }
    }

    /// Its row on the page (the rows' order is fixed; the public server's
    /// row only when there is one – else its choice is *Connect to a
    /// server*'s row).
    pub fn row(self, public: bool) -> usize {
        let first = usize::from(public);
        match self {
            Self::Public => 0,
            Self::Server => first,
            Self::Local => first + 1,
        }
    }
}

/// The play mode of *Ride locally* (spec 3.2.4 Q4).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum LocalMode {
    /// Every bike and all gear, no hearts, nothing locked.
    #[default]
    Training,
    /// Planned, postponed (spec 3.2.4 Q4): the rider starts with one old
    /// rusty city bike and earns the rest. Not offered yet.
    Campaign,
}

impl LocalMode {
    pub fn name(self) -> &'static str {
        match self {
            Self::Training => "Training mode",
            Self::Campaign => "Campaign mode",
        }
    }

    /// Whether this mode can be chosen yet (Campaign mode is postponed).
    pub fn built(self) -> bool {
        self == Self::Training
    }
}

/// The page's text over the rows.
pub const NOTE: &str = "Ride on the public server, connect to a server, or ride locally on this computer. Enter takes the marked choice (your last one).";

/// The rows: the public server (its join info URL, `None`: none), Connect
/// to a server, Ride locally (`last` marked).
pub fn rows(last: Choice, rider: &str, public: Option<&str>) -> Vec<Row> {
    let last = match last {
        Choice::Public if public.is_none() => Choice::Server,
        c => c,
    };
    let mark = |c: Choice| (c == last).then(|| "last choice".to_string());
    let mut out = Vec::new();
    if let Some(url) = public {
        out.push(Row {
            label: format!(
                "Ride on the public server ({})",
                crate::public::host_of(url)
            ),
            value: mark(Choice::Public),
            left: None,
            right: None,
            confirm: Some(Activate::RunOpen("start.choose public".into(), Page::Join)),
        });
    }
    out.extend([
        Row {
            label: "Connect to a server".into(),
            value: mark(Choice::Server),
            left: None,
            right: None,
            confirm: Some(Activate::RunOpen("start.choose server".into(), Page::Join)),
        },
        Row {
            label: format!("Ride locally as {rider} ({})", LocalMode::Training.name()),
            value: mark(Choice::Local),
            left: None,
            right: None,
            confirm: Some(Activate::Run("start.choose local".into())),
        },
    ]);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_last_choice_is_marked_and_each_row_runs_its_line() {
        let r = rows(Choice::Local, "Ada Rider", None);
        assert_eq!(r.len(), 2);
        assert_eq!(r[0].label, "Connect to a server");
        assert_eq!(r[0].value, None);
        assert_eq!(r[1].label, "Ride locally as Ada Rider (Training mode)");
        assert_eq!(r[1].value.as_deref(), Some("last choice"));
        assert_eq!(
            r[0].confirm,
            Some(Activate::RunOpen("start.choose server".into(), Page::Join))
        );
        assert_eq!(
            r[1].confirm,
            Some(Activate::Run("start.choose local".into()))
        );
        let r = rows(Choice::Server, "Ada Rider", None);
        assert_eq!(r[0].value.as_deref(), Some("last choice"));
        assert_eq!(Choice::Server.row(false), 0);
        assert_eq!(Choice::Local.row(false), 1);
        // A public server chosen last and cleared since: Connect to a server.
        let r = rows(Choice::Public, "Ada Rider", None);
        assert_eq!(r[0].value.as_deref(), Some("last choice"));
    }

    /// The public server is listed first (spec 11.5); riding locally
    /// stays the default choice.
    #[test]
    fn the_public_server_is_listed_first() {
        let url = crate::public::PUBLIC_JOIN_INFO;
        let r = rows(Choice::default(), "Bea Climber", Some(url));
        let labels: Vec<&str> = r.iter().map(|r| r.label.as_str()).collect();
        assert_eq!(
            labels,
            [
                "Ride on the public server (ocw-server.fly.dev)",
                "Connect to a server",
                "Ride locally as Bea Climber (Training mode)"
            ]
        );
        assert_eq!(
            r[0].confirm,
            Some(Activate::RunOpen("start.choose public".into(), Page::Join))
        );
        assert_eq!(r[2].value.as_deref(), Some("last choice"));
        assert_eq!(Choice::Public.row(true), 0);
        assert_eq!(Choice::Server.row(true), 1);
        assert_eq!(Choice::Local.row(true), 2);
        let r = rows(Choice::Public, "Bea Climber", Some(url));
        assert_eq!(r[0].value.as_deref(), Some("last choice"));
    }

    #[test]
    fn choices_parse_and_campaign_mode_is_not_built_yet() {
        for c in [Choice::Public, Choice::Server, Choice::Local] {
            assert_eq!(Choice::parse(c.id()), Some(c));
        }
        assert_eq!(Choice::parse(" Connect "), Some(Choice::Server));
        assert_eq!(Choice::parse("later"), None);
        assert_eq!(Choice::default(), Choice::Local);
        assert!(LocalMode::Training.built());
        assert!(!LocalMode::Campaign.built());
    }
}
