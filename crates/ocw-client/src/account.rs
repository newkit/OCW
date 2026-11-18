//! The Account page (spec §9.1, Settings → Account), shared by the desktop
//! and the browser: what the app's sign-in glue reports ([`AccountInfo`])
//! → the page's rows (each a console line of the `account` command), its
//! text and status line. Anonymous riding stays (Phase 10 Q5): signing in
//! is optional.

use ocw_auth::Provider;

use crate::menu::pages::{Activate, Page, Row};

/// The page's explanation.
pub const NOTE: &str = "Sign in with Google or Strava to keep your account across devices; you can always ride without one. OCW keeps only your name and the provider's account id.";

/// Where a sign-in stands.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum AccountState {
    #[default]
    SignedOut,
    /// The browser is open at the provider (or the backend's choice page).
    Waiting {
        provider: Option<Provider>,
        /// Linking it to the signed-in account.
        linking: bool,
    },
    /// The device flow (kiosk, headset): the code to confirm on a phone.
    DeviceCode {
        user_code: String,
        /// Where to enter it.
        uri: String,
        /// Unix seconds.
        expires_at: i64,
    },
    SignedIn {
        name: String,
        providers: Vec<Provider>,
    },
}

/// What the Account page shows (the app's sign-in glue fills it in).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AccountInfo {
    /// The accounts backend (`None`: sign-in is not set up in this app).
    pub issuer: Option<String>,
    pub state: AccountState,
    /// The last notice or error ("Sign-in cancelled").
    pub message: Option<String>,
}

impl AccountInfo {
    pub fn signed_in(&self) -> bool {
        matches!(self.state, AccountState::SignedIn { .. })
    }

    /// One line for the page's status strip and the console.
    pub fn status(&self) -> String {
        if self.issuer.is_none() {
            return "Sign-in is not set up (no account server): riding anonymously".into();
        }
        match &self.state {
            AccountState::SignedOut => "Not signed in: riding anonymously".into(),
            AccountState::Waiting { provider, linking } => format!(
                "{} {} in your browser ...",
                if *linking {
                    "Linking"
                } else {
                    "Signing in with"
                },
                provider.map_or("your provider", Provider::name)
            ),
            AccountState::DeviceCode { user_code, .. } => {
                format!("Waiting for the code {user_code} ...")
            }
            AccountState::SignedIn { name, providers } => format!(
                "Signed in as {name} ({})",
                providers
                    .iter()
                    .map(|p| p.name())
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
        }
    }

    /// The text under the title: the code to enter while the device flow
    /// waits, and the last message.
    pub fn text(&self) -> Vec<String> {
        let mut out = Vec::new();
        if let AccountState::DeviceCode { user_code, uri, .. } = &self.state {
            out.push(format!("On your phone, open {uri}"));
            out.push(format!("and enter the code   {user_code}"));
        }
        if let Some(m) = &self.message {
            out.push(m.clone());
        }
        out
    }
}

fn info_row(label: &str, value: Option<String>) -> Row {
    Row {
        label: label.into(),
        value,
        left: None,
        right: None,
        confirm: Some(Activate::Disabled),
    }
}

/// The rows of the Account page. `desktop`: the desktop app (the device
/// flow for kiosks, the rider profiles).
pub fn rows(info: &AccountInfo, desktop: bool) -> Vec<Row> {
    let mut out = Vec::new();
    if info.issuer.is_none() {
        out.push(info_row("Sign-in", Some("not set up in this app".into())));
    } else {
        match &info.state {
            AccountState::SignedOut => {
                for p in Provider::ALL {
                    out.push(Row::action(
                        &format!("Sign in with {}", p.name()),
                        format!("account signin {}", p.id()),
                    ));
                }
                if desktop {
                    out.push(Row::action(
                        "Sign in with a code (kiosk)",
                        "account signin --code",
                    ));
                }
            }
            AccountState::Waiting { .. } | AccountState::DeviceCode { .. } => {
                out.push(info_row("Status", Some(info.status())));
                out.push(Row::action("Cancel the sign-in", "account cancel"));
            }
            AccountState::SignedIn { name, providers } => {
                out.push(info_row(
                    "Signed in",
                    Some(format!(
                        "{name} ({})",
                        providers
                            .iter()
                            .map(|p| p.name())
                            .collect::<Vec<_>>()
                            .join(", ")
                    )),
                ));
                for p in Provider::ALL {
                    if !providers.contains(&p) {
                        out.push(Row::action(
                            &format!("Link {}", p.name()),
                            format!("account link {}", p.id()),
                        ));
                    } else if providers.len() > 1 {
                        // Never the last way to sign in.
                        out.push(Row::action(
                            &format!("Unlink {}", p.name()),
                            format!("account unlink {}", p.id()),
                        ));
                    }
                }
                out.push(Row::action("Sign out", "account signout"));
            }
        }
    }
    if desktop {
        out.push(Row {
            label: "Rider profiles".into(),
            value: None,
            left: None,
            right: None,
            confirm: Some(Activate::Open(Page::Riders)),
        });
        // The activities folder: everything OCW saves (spec §2.8.1; the
        // Settings page is full).
        out.push(Row {
            label: "Folders".into(),
            value: Some("~/Activities/OCW".into()),
            left: None,
            right: None,
            confirm: Some(Activate::Open(Page::Folders)),
        });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lines(rows: &[Row]) -> Vec<String> {
        rows.iter()
            .map(|r| match &r.confirm {
                Some(Activate::Command(l)) => l.clone(),
                Some(Activate::Open(p)) => format!("open {p:?}"),
                Some(Activate::Disabled) => format!("({})", r.label),
                other => format!("{other:?}"),
            })
            .collect()
    }

    fn info(state: AccountState) -> AccountInfo {
        AccountInfo {
            issuer: Some("https://accounts.example.org".into()),
            state,
            message: None,
        }
    }

    #[test]
    fn signed_out_offers_both_providers_and_the_code() {
        let i = info(AccountState::SignedOut);
        assert_eq!(
            lines(&rows(&i, true)),
            [
                "account signin google",
                "account signin strava",
                "account signin --code",
                "open Riders",
                "open Folders"
            ]
        );
        assert_eq!(
            lines(&rows(&i, false)),
            ["account signin google", "account signin strava"]
        );
        assert!(i.status().contains("anonymously"));
    }

    #[test]
    fn signed_in_links_the_other_and_never_unlinks_the_last() {
        let one = info(AccountState::SignedIn {
            name: "Ada Rider".into(),
            providers: vec![Provider::Google],
        });
        assert_eq!(
            lines(&rows(&one, false)),
            ["(Signed in)", "account link strava", "account signout"]
        );
        assert_eq!(
            rows(&one, false)[0].value.as_deref(),
            Some("Ada Rider (Google)")
        );
        let both = info(AccountState::SignedIn {
            name: "Ada Rider".into(),
            providers: vec![Provider::Google, Provider::Strava],
        });
        assert_eq!(
            lines(&rows(&both, false)),
            [
                "(Signed in)",
                "account unlink google",
                "account unlink strava",
                "account signout"
            ]
        );
        assert_eq!(both.status(), "Signed in as Ada Rider (Google, Strava)");
    }

    #[test]
    fn waiting_and_the_device_code() {
        let d = info(AccountState::DeviceCode {
            user_code: "BCDF-GHJK".into(),
            uri: "https://accounts.example.org/device".into(),
            expires_at: 0,
        });
        assert_eq!(
            lines(&rows(&d, true)),
            ["(Status)", "account cancel", "open Riders", "open Folders"]
        );
        assert_eq!(
            d.text(),
            [
                "On your phone, open https://accounts.example.org/device",
                "and enter the code   BCDF-GHJK"
            ]
        );
        let none = AccountInfo::default();
        assert_eq!(lines(&rows(&none, false)), ["(Sign-in)"]);
        // The ten-foot rule: at most six rows.
        for s in [
            AccountState::SignedOut,
            AccountState::SignedIn {
                name: "x".into(),
                providers: Provider::ALL.to_vec(),
            },
        ] {
            assert!(rows(&info(s), true).len() <= 6);
        }
    }
}
