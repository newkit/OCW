//! The licence rule for shipped assets (spec §13, Phase 8A): code-like own
//! work, public domain and attribution-only licences are accepted;
//! share-alike, non-commercial, no-derivatives, proprietary and unknown
//! licences are not. Models (`assets/models/manifest.toml`) and materials
//! (`assets/materials.toml`) use the same rule.

/// What a licence obliges us to do.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Obligation {
    /// Nothing (we still credit the source).
    None,
    /// Name the author / source on the credits screen.
    Attribution,
}

/// The accepted licences (identifiers as written in the manifests; SPDX
/// where SPDX has one) and their obligation.
pub const ACCEPTED: [(&str, Obligation); 11] = [
    ("MIT OR Apache-2.0", Obligation::None),
    ("MIT", Obligation::None),
    ("Apache-2.0", Obligation::None),
    ("CC0-1.0", Obligation::None),
    ("public-domain", Obligation::None),
    ("CC-BY-4.0", Obligation::Attribution),
    ("DL-DE-ZERO-2.0", Obligation::None),
    ("DL-DE-BY-2.0", Obligation::Attribution),
    ("OGD-BY", Obligation::Attribution),
    ("Copernicus", Obligation::Attribution),
    ("ODbL-1.0", Obligation::Attribution),
];

/// The accepted licence identifiers.
pub fn accepted() -> impl Iterator<Item = &'static str> {
    ACCEPTED.iter().map(|(l, _)| *l)
}

/// Why a licence is rejected.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Rejection {
    /// No licence given.
    Missing,
    /// Share-alike (copyleft for data / media): would bind the whole
    /// package or app.
    ShareAlike,
    /// Non-commercial: excludes commercial use.
    NonCommercial,
    /// No derivatives: we compress, resize and tint every texture.
    NoDerivatives,
    /// Anything not on the accepted list (proprietary, unclear, typos).
    Unknown,
}

impl std::fmt::Display for Rejection {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Rejection::Missing => "no licence",
            Rejection::ShareAlike => "share-alike licences are not accepted (spec Phase 8A)",
            Rejection::NonCommercial => "non-commercial licences are not accepted (spec Phase 8A)",
            Rejection::NoDerivatives => "no-derivatives licences are not accepted (spec Phase 8A)",
            Rejection::Unknown => "unknown licence (spec Phase 8A lists the accepted ones)",
        })
    }
}

/// Checks a licence identifier against the rule.
pub fn check(licence: &str) -> Result<Obligation, Rejection> {
    let l = licence.trim();
    if l.is_empty() {
        return Err(Rejection::Missing);
    }
    if let Some((_, o)) = ACCEPTED.iter().find(|(a, _)| *a == l) {
        return Ok(*o);
    }
    // Name the clause that excludes a licence, so the error says why.
    let words: Vec<String> = l
        .to_ascii_uppercase()
        .split(|c: char| !c.is_ascii_alphanumeric())
        .map(str::to_owned)
        .collect();
    let has = |w: &str| words.iter().any(|x| x == w);
    let upper = l.to_ascii_uppercase().replace([' ', '_'], "");
    if has("NC") || upper.contains("NONCOMMERCIAL") {
        Err(Rejection::NonCommercial)
    } else if has("ND") || upper.contains("NODERIV") {
        Err(Rejection::NoDerivatives)
    } else if has("SA") || upper.contains("SHAREALIKE") || has("GPL") || has("LGPL") {
        Err(Rejection::ShareAlike)
    } else {
        Err(Rejection::Unknown)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_phase_8a_licences_are_accepted() {
        for l in accepted() {
            assert!(check(l).is_ok(), "{l}");
        }
        assert_eq!(check("CC0-1.0"), Ok(Obligation::None));
        assert_eq!(check(" CC-BY-4.0 "), Ok(Obligation::Attribution));
        assert_eq!(check("DL-DE-BY-2.0"), Ok(Obligation::Attribution));
        assert_eq!(check("OGD-BY"), Ok(Obligation::Attribution));
        assert_eq!(check("Copernicus"), Ok(Obligation::Attribution));
    }

    #[test]
    fn share_alike_non_commercial_no_derivatives_and_unknown_are_rejected() {
        for (l, why) in [
            ("", Rejection::Missing),
            ("  ", Rejection::Missing),
            ("CC-BY-SA-4.0", Rejection::ShareAlike),
            ("CC BY-SA 3.0", Rejection::ShareAlike),
            ("GPL-3.0", Rejection::ShareAlike),
            ("CC-BY-NC-4.0", Rejection::NonCommercial),
            ("CC-BY-NC-SA-4.0", Rejection::NonCommercial),
            ("Creative Commons NonCommercial", Rejection::NonCommercial),
            ("CC-BY-ND-4.0", Rejection::NoDerivatives),
            ("CC-BY-NC-ND-4.0", Rejection::NonCommercial),
            ("Proprietary", Rejection::Unknown),
            ("CC0", Rejection::Unknown),
            ("Google Maps", Rejection::Unknown),
        ] {
            assert_eq!(check(l), Err(why.clone()), "{l:?}");
            assert!(!why.to_string().is_empty());
        }
    }
}
