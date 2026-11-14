//! The jersey designer of the character corner (spec §14 Phase 8 T6),
//! pure: two pages of fields – the jersey (pattern, three colours, race
//! number, the pattern on the helmet) and the kit (shorts, socks, shoes,
//! gloves, helmet colour, helmet and shoe finish) – each stepped with ← / →
//! on a draft of the look, shown live on the rider in the mirror, applied
//! with Confirm as one `kit.design` console line (the menu's rule: menu,
//! console and scripts share one code path). Everything here is free.

use ocw_core::bike::PALETTE;
use ocw_core::kit::MAX_RACE_NUMBER;
use ocw_core::{Finish, KitPattern, Rgb, RiderLook};

/// A page of the designer (one slot of the character corner each).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DesignPage {
    Jersey,
    Kit,
}

impl DesignPage {
    pub fn name(self) -> &'static str {
        match self {
            Self::Jersey => "Jersey designer",
            Self::Kit => "Kit colours",
        }
    }

    pub fn fields(self) -> &'static [Field] {
        match self {
            Self::Jersey => &[
                Field::Pattern,
                Field::First,
                Field::Second,
                Field::Third,
                Field::Number,
                Field::HelmetPattern,
            ],
            Self::Kit => &[
                Field::Shorts,
                Field::Socks,
                Field::Shoes,
                Field::Gloves,
                Field::Helmet,
                Field::HelmetFinish,
                Field::ShoeFinish,
            ],
        }
    }
}

/// A field of the designer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Field {
    Pattern,
    First,
    Second,
    Third,
    Number,
    HelmetPattern,
    Shorts,
    Socks,
    Shoes,
    Gloves,
    Helmet,
    HelmetFinish,
    ShoeFinish,
}

/// The palette with "none" first (optional colours).
fn step_opt(c: Option<Rgb>, d: i32) -> Option<Rgb> {
    let n = PALETTE.len() as i32 + 1;
    let i = c
        .and_then(|c| PALETTE.iter().position(|p| *p == c))
        .map_or(0, |i| i as i32 + 1);
    let j = (i + d).rem_euclid(n);
    (j > 0).then(|| PALETTE[j as usize - 1])
}

fn step_colour(c: Rgb, d: i32) -> Rgb {
    ocw_core::bike::palette_step(c, d)
}

fn step_finish(f: Finish, d: i32) -> Finish {
    let n = Finish::ALL.len() as i32;
    Finish::ALL[(f as i32 + d).rem_euclid(n) as usize]
}

fn opt_name(c: Option<Rgb>, none: &str) -> String {
    c.map_or_else(|| none.to_string(), |c| c.to_string())
}

impl Field {
    pub fn name(self) -> &'static str {
        match self {
            Self::Pattern => "Pattern",
            Self::First => "First colour",
            Self::Second => "Second colour",
            Self::Third => "Third colour",
            Self::Number => "Race number",
            Self::HelmetPattern => "Pattern on the helmet",
            Self::Shorts => "Shorts",
            Self::Socks => "Socks",
            Self::Shoes => "Shoes",
            Self::Gloves => "Gloves",
            Self::Helmet => "Helmet colour",
            Self::HelmetFinish => "Helmet finish",
            Self::ShoeFinish => "Shoe finish",
        }
    }

    /// What the field says about it (the detail card).
    pub fn help(self) -> &'static str {
        match self {
            Self::Pattern => {
                "Our own patterns: plain, chest band, sash, stripes, chevrons, gradient."
            }
            Self::First => "The jersey's main colour.",
            Self::Second => {
                "The pattern's colour (auto: white on dark jerseys, black on light ones); also the zip."
            }
            Self::Third => "A third colour for pinstripes, every third stripe, the middle chevron.",
            Self::Number => {
                "On the back (a paper bib) and both hips of the shorts; ← / → step by one."
            }
            Self::HelmetPattern => "The helmet shell in the jersey colours with a centre stripe.",
            Self::Shorts | Self::Socks | Self::Shoes | Self::Gloves => {
                "Standard: the kit's own colour. Variants (tights, overshoes, aero socks …) take it too."
            }
            Self::Helmet => "The shell's colour without the pattern.",
            Self::HelmetFinish | Self::ShoeFinish => "Gloss: a clear coat; matte: rough, no shine.",
        }
    }

    /// The field's value in `l` as shown.
    pub fn value(self, l: &RiderLook) -> String {
        let d = &l.design;
        match self {
            Self::Pattern => l.pattern.name().to_string(),
            Self::First => l.jersey.to_string(),
            Self::Second => opt_name(d.second, "auto"),
            Self::Third => opt_name(d.third, "none (two colours)"),
            Self::Number => {
                if d.number == 0 {
                    "none".into()
                } else {
                    d.number.to_string()
                }
            }
            Self::HelmetPattern => if d.helmet_pattern { "on" } else { "off" }.into(),
            Self::Shorts => opt_name(d.shorts, "standard (black)"),
            Self::Socks => opt_name(d.socks, "standard (white)"),
            Self::Shoes => opt_name(d.shoes, "standard"),
            Self::Gloves => opt_name(d.gloves, "standard (black)"),
            Self::Helmet => l.helmet.to_string(),
            Self::HelmetFinish => d.helmet_finish.name().into(),
            Self::ShoeFinish => d.shoe_finish.name().into(),
        }
    }

    /// `l` with this field stepped by `d` (← −1, → +1).
    pub fn step(self, mut l: RiderLook, d: i32) -> RiderLook {
        let k = &mut l.design;
        match self {
            Self::Pattern => {
                let n = KitPattern::ALL.len() as i32;
                l.pattern = KitPattern::ALL[(l.pattern.wire() as i32 + d).rem_euclid(n) as usize];
            }
            Self::First => l.jersey = step_colour(l.jersey, d),
            Self::Second => k.second = step_opt(k.second, d),
            Self::Third => k.third = step_opt(k.third, d),
            Self::Number => {
                let n = i32::from(MAX_RACE_NUMBER) + 1;
                k.number = (i32::from(k.number) + d).rem_euclid(n) as u16;
            }
            Self::HelmetPattern => k.helmet_pattern = !k.helmet_pattern,
            Self::Shorts => k.shorts = step_opt(k.shorts, d),
            Self::Socks => k.socks = step_opt(k.socks, d),
            Self::Shoes => k.shoes = step_opt(k.shoes, d),
            Self::Gloves => k.gloves = step_opt(k.gloves, d),
            Self::Helmet => l.helmet = step_colour(l.helmet, d),
            Self::HelmetFinish => k.helmet_finish = step_finish(k.helmet_finish, d),
            Self::ShoeFinish => k.shoe_finish = step_finish(k.shoe_finish, d),
        }
        l
    }
}

/// The `kit.design` line that makes `l`'s design (every field, so the
/// line alone reproduces it).
pub fn line(l: &RiderLook) -> String {
    let d = l.design.validated();
    let opt = |c: Option<Rgb>, none: &str| c.map_or_else(|| none.to_string(), |c| c.to_string());
    format!(
        "kit.design --pattern {} --jersey {} --second {} --third {} --number {} --helmet-pattern {} \
         --shorts {} --socks {} --shoes {} --gloves {} --helmet {} --helmet-finish {} --shoe-finish {}",
        l.pattern.id(),
        l.jersey,
        opt(d.second, "auto"),
        opt(d.third, "none"),
        if d.number == 0 {
            "none".to_string()
        } else {
            d.number.to_string()
        },
        if d.helmet_pattern { "on" } else { "off" },
        opt(d.shorts, "standard"),
        opt(d.socks, "standard"),
        opt(d.shoes, "standard"),
        opt(d.gloves, "standard"),
        l.helmet,
        d.helmet_finish.id(),
        d.shoe_finish.id(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_field_steps_and_wraps() {
        let l = RiderLook::default();
        for page in [DesignPage::Jersey, DesignPage::Kit] {
            for f in page.fields() {
                let next = f.step(l, 1);
                assert_ne!(next, l, "{f:?} changes the look");
                assert_ne!(f.value(&next), f.value(&l), "{f:?} shows it");
                assert_eq!(f.step(next, -1), l, "{f:?}: and back");
            }
        }
        // The number wraps 999 → none → 1.
        let mut n = l;
        n.design.number = 999;
        assert_eq!(Field::Number.step(n, 1).design.number, 0);
        assert_eq!(Field::Number.step(l, -1).design.number, 999);
        // Optional colours run none → the palette → none.
        let mut c = l;
        for _ in 0..=PALETTE.len() {
            c = Field::Third.step(c, 1);
        }
        assert_eq!(c.design.third, None);
        // All patterns are reachable.
        let mut p = l;
        let mut seen = Vec::new();
        for _ in 0..KitPattern::ALL.len() {
            seen.push(p.pattern);
            p = Field::Pattern.step(p, 1);
        }
        assert_eq!(seen, KitPattern::ALL.to_vec());
    }

    #[test]
    fn the_line_names_every_field() {
        let l = Field::Number.step(Field::Pattern.step(RiderLook::default(), 3), 42);
        let s = line(&l);
        assert!(s.starts_with("kit.design --pattern stripes "), "{s}");
        assert!(
            s.contains("--number 42") && s.contains("--second auto"),
            "{s}"
        );
        assert!(
            s.contains("--third none") && s.contains("--helmet-finish gloss"),
            "{s}"
        );
        assert!(line(&RiderLook::default()).contains("--number none"));
    }
}
