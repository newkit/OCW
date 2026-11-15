//! The backend's few HTML pages: the provider choice, the device-code
//! entry and confirmation (RFC 8628 §3.3, with a CSRF token), and the
//! messages after a sign-in. Plain HTML, no scripts; every value is
//! escaped, the pages are never cached or framed (see `routes`).

use ocw_auth::Provider;

pub fn escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            c => out.push(c),
        }
    }
    out
}

fn page(title: &str, body: &str) -> String {
    format!(
        r#"<!doctype html>
<html lang="en"><head><meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>{t} – Open Cycling World</title>
<style>
:root {{ color-scheme: light dark; --accent: #ff1472; }}
body {{ font: 18px/1.5 system-ui, sans-serif; margin: 0; display: grid; place-items: center; min-height: 100vh; background: Canvas; color: CanvasText; }}
main {{ max-width: 28rem; padding: 1.5rem; }}
h1 {{ font-size: 1.5rem; margin: 0 0 1rem; }}
.code {{ font: 700 2rem/1.2 ui-monospace, monospace; letter-spacing: .15em; margin: .5rem 0 1rem; }}
button, a.button {{ display: block; width: 100%; box-sizing: border-box; margin: .6rem 0; padding: .8rem 1rem; font: inherit; border-radius: .5rem; border: 1px solid GrayText; background: transparent; color: inherit; text-align: center; text-decoration: none; cursor: pointer; }}
button.primary, a.primary {{ background: var(--accent); border-color: var(--accent); color: white; }}
input {{ font: 700 1.5rem ui-monospace, monospace; letter-spacing: .1em; text-transform: uppercase; width: 100%; box-sizing: border-box; padding: .6rem; border-radius: .5rem; border: 1px solid GrayText; }}
.note {{ color: GrayText; font-size: .9rem; }}
.error {{ color: #d0021b; }}
</style></head>
<body><main>
<h1>{t}</h1>
{body}
</main></body></html>
"#,
        t = escape(title)
    )
}

/// Which provider to sign in with (`/authorize` without `provider`).
pub fn choose(client: &str, links: &[(Provider, String)]) -> String {
    let mut body = format!("<p>Sign in to <b>{}</b> with:</p>", escape(client));
    for (i, (p, url)) in links.iter().enumerate() {
        body.push_str(&format!(
            r#"<a class="button{}" href="{}">{}</a>"#,
            if i == 0 { " primary" } else { "" },
            escape(url),
            escape(p.name())
        ));
    }
    body.push_str(
        r#"<p class="note">Open Cycling World keeps only your name and the provider's account id (and your e-mail with Google). You can ride without an account too.</p>"#,
    );
    page("Sign in", &body)
}

/// The code entry of the device flow.
pub fn device_enter(error: Option<&str>) -> String {
    let err = error
        .map(|e| format!(r#"<p class="error">{}</p>"#, escape(e)))
        .unwrap_or_default();
    page(
        "Connect a device",
        &format!(
            r#"<p>Enter the code your kiosk or headset shows.</p>{err}
<form method="get" action="device">
<input name="user_code" autocomplete="off" autocapitalize="characters" placeholder="XXXX-XXXX" maxlength="12" required>
<button class="primary" type="submit">Continue</button>
</form>"#
        ),
    )
}

/// The confirmation: which device, which code, then the provider.
pub fn device_confirm(client: &str, user_code: &str, csrf: &str, providers: &[Provider]) -> String {
    let mut buttons = String::new();
    for (i, p) in providers.iter().enumerate() {
        buttons.push_str(&format!(
            r#"<button{} type="submit" name="action" value="{}">Sign in with {}</button>"#,
            if i == 0 { r#" class="primary""# } else { "" },
            p.id(),
            escape(p.name())
        ));
    }
    page(
        "Connect a device",
        &format!(
            r#"<p><b>{client}</b> asks to sign in to your Open Cycling World account.</p>
<div class="code">{code}</div>
<p class="note">Only continue if you started this on a device in front of you and it shows this code.</p>
<form method="post" action="device/confirm">
<input type="hidden" name="csrf" value="{csrf}">
<input type="hidden" name="user_code" value="{code}">
{buttons}
<button type="submit" name="action" value="deny">Cancel</button>
</form>"#,
            client = escape(client),
            code = escape(user_code),
            csrf = escape(csrf),
        ),
    )
}

pub fn message(title: &str, text: &str) -> String {
    page(title, &format!("<p>{}</p>", escape(text)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn values_are_escaped() {
        let p = device_confirm(
            "<script>x</script>",
            "BCDF-GHJK",
            "c\"srf",
            &[Provider::Google],
        );
        assert!(!p.contains("<script>x"), "{p}");
        assert!(p.contains("&lt;script&gt;") && p.contains("c&quot;srf"));
        assert!(
            choose(
                "OCW",
                &[(Provider::Strava, "/authorize?a=1&provider=strava".into())]
            )
            .contains("a=1&amp;provider=strava")
        );
        assert!(device_enter(Some("<b>")).contains("&lt;b&gt;"));
    }
}
