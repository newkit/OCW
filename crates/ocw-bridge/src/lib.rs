//! The local sensor bridge (spec 14 Phase 10, 10D.2 *Built (local
//! bridge)*, v5.72): the desktop app keeps the trainer and the heart-rate
//! strap on Bluetooth and relays power, cadence and heart rate to **one
//! browser session on the same network** – the Quest browser in WebXR, an
//! iPad, any browser – and the ride's trainer control (SIM gradient, ERG
//! target) back to the trainer. Paired with a short code, no account.
//!
//! The transport is **WebTransport with `serverCertificateHashes`**: the
//! page comes over HTTPS from GitHub Pages, so it cannot open `ws://` to a
//! local address (mixed content) and rejects a self-signed `wss://`
//! certificate; WebTransport takes a self-signed ECDSA certificate of at
//! most 14 days by its SHA-256 (Chromium 100+, the Quest browser, Firefox
//! 125+, Safari 26.4+). The certificate is derived from the pairing code's
//! secret ([`cert`]), so the code ([`code`]) is all the browser needs.
//!
//! - [`code`]: the pairing code (address, port slot, secret, check symbol).
//! - [`cert`]: the certificate both sides derive.
//! - [`msg`]: the messages (JSON frames on one stream).
//! - [`relay`]: both sides' logic (pairing, ordering, clocks, control).
//! - `host` (native): the desktop's endpoint.

pub mod cert;
pub mod code;
#[cfg(not(target_arch = "wasm32"))]
pub mod host;
pub mod msg;
pub mod relay;

pub use code::PairingCode;

/// The WebTransport session's path (the protocol's version).
pub const PATH: &str = "/ocw-bridge/1";

/// The browser app's page option and its page address on the public
/// preview: `?bridge=CODE` connects at once.
pub const PAGE_OPTION: &str = "bridge";

/// The public preview (GitHub Pages, spec 14 Phase 10 B8a).
pub const PREVIEW_URL: &str = "https://newkit.github.io/OCW/";

/// The link that opens the preview with the code filled in (the desktop's
/// QR code).
pub fn page_link(base: &str, code: &PairingCode) -> String {
    let sep = if base.contains('?') { '&' } else { '?' };
    format!("{base}{sep}{PAGE_OPTION}={}", code.compact())
}

/// What the desktop shows of the paired browser, from its user agent (no
/// version numbers, nothing that tells one person's device from another).
pub fn browser_name(user_agent: &str) -> String {
    let ua = user_agent;
    let browser = if ua.contains("OculusBrowser") {
        return "Quest Browser".into();
    } else if ua.contains("Firefox/") {
        "Firefox"
    } else if ua.contains("Edg/") {
        "Edge"
    } else if ua.contains("Chrome/") || ua.contains("CriOS/") || ua.contains("Chromium/") {
        "Chrome"
    } else if ua.contains("Safari/") {
        "Safari"
    } else {
        "A browser"
    };
    let system = if ua.contains("iPad") {
        "iPad"
    } else if ua.contains("iPhone") {
        "iPhone"
    } else if ua.contains("Android") {
        "Android"
    } else if ua.contains("Mac OS X") || ua.contains("Macintosh") {
        "macOS"
    } else if ua.contains("Windows") {
        "Windows"
    } else if ua.contains("Linux") || ua.contains("X11") {
        "Linux"
    } else {
        return browser.into();
    };
    format!("{browser} on {system}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::Ipv4Addr;

    #[test]
    fn browsers_are_named_without_versions() {
        let quest = "Mozilla/5.0 (X11; Linux x86_64; Quest) AppleWebKit/537.36 (KHTML, like Gecko) OculusBrowser/23.2.0 SamsungBrowser/4.0 Chrome/104.0.0.0 VR Safari/537.36";
        assert_eq!(browser_name(quest), "Quest Browser");
        let ipad = "Mozilla/5.0 (iPad; CPU OS 26_4 like Mac OS X) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/26.4 Mobile/15E148 Safari/604.1";
        assert_eq!(browser_name(ipad), "Safari on iPad");
        let chrome = "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) HeadlessChrome/156.0.0.0 Safari/537.36";
        assert_eq!(browser_name(chrome), "Chrome on Linux");
        let ff = "Mozilla/5.0 (Windows NT 10.0; Win64; x64; rv:145.0) Gecko/20100101 Firefox/145.0";
        assert_eq!(browser_name(ff), "Firefox on Windows");
        assert_eq!(browser_name("curl/8"), "A browser");
    }

    #[test]
    fn the_page_link_carries_the_code() {
        let c = PairingCode::new(Ipv4Addr::new(192, 168, 1, 2), 0, 5);
        let link = page_link(PREVIEW_URL, &c);
        assert!(link.starts_with("https://newkit.github.io/OCW/?bridge="));
        assert_eq!(
            PairingCode::parse(link.rsplit('=').next().unwrap()),
            Ok(c)
        );
        assert!(page_link("http://localhost:8000/?world=x", &c).contains("?world=x&bridge="));
    }
}
