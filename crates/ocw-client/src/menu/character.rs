//! The character pages of the in-game menu (spec §14 Phase 4, 4N N10;
//! both apps, B6): Workshop → Character with Body (base body A / B / C,
//! the seven shapes bounded to the band of the weight with the *Match my
//! weight* marker), Face and age, Hair, Facial hair. Every change is a
//! `character …` console line (the same command in both apps; the
//! browser's anonymous rider gets the locked mirror's answer, Phase 10
//! Q5). Looks only, never physics.

use ocw_core::character::{
    self as ch, Age, BEARDS, GLASSES, HAIR_PALETTE, HAIR_STYLES, MOUSTACHES, SKIN_TONES,
};
use ocw_core::{BaseBody, BodyShape, Rgb};

use super::pages::{Activate, Page, Row};
use crate::riders::Rider;

/// The character's pages, the categories of N10.
pub const PAGES: [Page; 4] = [
    Page::CharacterBody,
    Page::CharacterFace,
    Page::CharacterHair,
    Page::CharacterFacialHair,
];

/// A `character` line.
fn line(args: &str) -> String {
    format!("character {args}")
}

/// A row whose ← / → step through `n` choices from `at` (wrapping), each
/// a `character` line from `arg(i)`.
fn stepper(label: &str, value: String, at: usize, n: usize, arg: impl Fn(usize) -> String) -> Row {
    let prev = line(&arg((at + n - 1) % n));
    let next = line(&arg((at + 1) % n));
    Row {
        label: label.into(),
        value: Some(value),
        left: Some(prev),
        right: Some(next.clone()),
        confirm: Some(Activate::Command(next)),
    }
}

/// The shape row (Q13): ← / → within the band only (no wrap), the value
/// with the *Match my weight* marker.
fn shape_row(now: BodyShape, marker: BodyShape) -> Row {
    let (lo, hi) = ch::shape_band(marker);
    let step = |d: isize| {
        let s = BodyShape::at(now.index() as isize + d).clamp(lo, hi);
        (s != now).then(|| line(&format!("--shape {}", s.id())))
    };
    let mark = if now == marker {
        " (matches your weight)".to_string()
    } else {
        format!(" (your weight: {})", marker.name())
    };
    Row {
        label: "Shape".into(),
        value: Some(format!("{}{mark}", now.name())),
        left: step(-1),
        right: step(1),
        confirm: None,
    }
}

/// The hair palette's name of `c`, or its hex code (a custom colour).
fn colour_name(c: Rgb) -> String {
    ch::hair_colour_name(c).map_or_else(|| format!("custom {c}"), str::to_string)
}

/// The index of `c` in the natural palette (a custom colour: before the
/// first).
fn palette_at(c: Rgb) -> Option<usize> {
    HAIR_PALETTE.iter().position(|(_, p)| *p == c)
}

/// The rows of the character pages for `rider` (the active profile;
/// the default rider without profiles).
pub fn rows(page: Page, rider: &Rider) -> Vec<Row> {
    let c = rider.character();
    let marker = rider.weight_shape();
    match page {
        Page::Character => {
            let open = |label: &str, p: Page, value: String| Row {
                label: label.into(),
                value: Some(value),
                left: None,
                right: None,
                confirm: Some(Activate::Open(p)),
            };
            let facial = match (c.moustache, c.beard) {
                (0, 0) => "none".to_string(),
                (m, 0) => MOUSTACHES[m as usize].to_lowercase(),
                (0, b) => BEARDS[b as usize].to_lowercase(),
                (m, b) => format!(
                    "{}, {}",
                    MOUSTACHES[m as usize].to_lowercase(),
                    BEARDS[b as usize].to_lowercase()
                ),
            };
            vec![
                open(
                    "Body",
                    Page::CharacterBody,
                    format!("{}, {}", c.base.name(), c.shape.name().to_lowercase()),
                ),
                open(
                    "Face and age",
                    Page::CharacterFace,
                    format!("{}, skin {} of 12", c.age.id(), c.skin + 1),
                ),
                open(
                    "Hair",
                    Page::CharacterHair,
                    format!(
                        "{}, {}",
                        HAIR_STYLES[c.hair as usize],
                        colour_name(c.hair_colour).to_lowercase()
                    ),
                ),
                open("Facial hair", Page::CharacterFacialHair, facial),
            ]
        }
        Page::CharacterBody => {
            let at = BaseBody::ALL.iter().position(|b| *b == c.base).unwrap_or(0);
            vec![
                stepper(
                    "Base body",
                    format!("{} ({})", c.base.name(), c.base.about()),
                    at,
                    BaseBody::ALL.len(),
                    |i| format!("--body {}", BaseBody::ALL[i].id()),
                ),
                shape_row(c.shape, marker),
                Row {
                    label: "Match my weight".into(),
                    value: Some(marker.name().into()),
                    left: None,
                    right: None,
                    confirm: if c.shape == marker {
                        Some(Activate::Disabled)
                    } else {
                        Some(Activate::Command(line("--match-weight")))
                    },
                },
                // The height is the profile's (N4): changed on the rider's
                // page, shown here.
                Row {
                    label: "Height".into(),
                    value: Some(format!("{:.2} m (the rider profile's)", c.scale() * 1.78)),
                    left: None,
                    right: None,
                    confirm: None,
                },
            ]
        }
        Page::CharacterFace => {
            let age_at = Age::ALL.iter().position(|a| *a == c.age).unwrap_or(1);
            vec![
                stepper("Age", c.age.id().into(), age_at, Age::ALL.len(), |i| {
                    format!("--age {}", Age::ALL[i].id())
                }),
                stepper(
                    "Skin tone",
                    format!("{} of {}", c.skin + 1, SKIN_TONES.len()),
                    c.skin as usize,
                    SKIN_TONES.len(),
                    |i| format!("--skin {}", i + 1),
                ),
                Row {
                    label: "Keep my hair colour".into(),
                    value: Some(
                        if c.greying {
                            "off (greys with the age)"
                        } else {
                            "on"
                        }
                        .into(),
                    ),
                    left: None,
                    right: None,
                    confirm: Some(Activate::Command(line(if c.greying {
                        "--greying off"
                    } else {
                        "--greying on"
                    }))),
                },
                stepper(
                    "Glasses",
                    GLASSES[c.glasses as usize].into(),
                    c.glasses as usize,
                    GLASSES.len(),
                    |i| format!("--glasses {}", i + 1),
                ),
            ]
        }
        Page::CharacterHair => {
            let colour_at = palette_at(c.hair_colour);
            let n = HAIR_PALETTE.len();
            let colour = |i: usize| format!("--hair-colour {}", HAIR_PALETTE[i].1);
            vec![
                stepper(
                    "Hair style",
                    HAIR_STYLES[c.hair as usize].into(),
                    c.hair as usize,
                    HAIR_STYLES.len(),
                    |i| format!("--hair {}", ch::hair_id(i as u8)),
                ),
                Row {
                    label: "Hair colour".into(),
                    value: Some(colour_name(c.hair_colour)),
                    left: Some(line(&colour(colour_at.map_or(n - 1, |i| (i + n - 1) % n)))),
                    right: Some(line(&colour(colour_at.map_or(0, |i| (i + 1) % n)))),
                    confirm: None,
                },
            ]
        }
        Page::CharacterFacialHair => {
            let follows = c.facial_hair_colour.is_none();
            let n = HAIR_PALETTE.len();
            // ← / →: "follows the hair", then the palette.
            let at = c
                .facial_hair_colour
                .and_then(palette_at)
                .map_or(0, |i| i + 1);
            let pick = |k: usize| {
                if k == 0 {
                    "--facial-hair-colour hair".to_string()
                } else {
                    format!("--facial-hair-colour {}", HAIR_PALETTE[k - 1].1)
                }
            };
            vec![
                stepper(
                    "Moustache",
                    MOUSTACHES[c.moustache as usize].into(),
                    c.moustache as usize,
                    MOUSTACHES.len(),
                    |i| format!("--moustache {}", ch::list_id(&MOUSTACHES, i as u8)),
                ),
                stepper(
                    "Beard",
                    BEARDS[c.beard as usize].into(),
                    c.beard as usize,
                    BEARDS.len(),
                    |i| format!("--beard {}", ch::list_id(&BEARDS, i as u8)),
                ),
                stepper(
                    "Colour",
                    if follows {
                        "the hair's".into()
                    } else {
                        c.facial_hair_colour.map_or_else(String::new, colour_name)
                    },
                    at,
                    n + 1,
                    pick,
                ),
            ]
        }
        _ => Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rider(kg: f64, cm: Option<f64>) -> Rider {
        Rider {
            name: "Ada Rider".into(),
            weight_kg: kg,
            height_cm: cm,
            ..Rider::default()
        }
    }

    fn row<'a>(rows: &'a [Row], label: &str) -> &'a Row {
        rows.iter().find(|r| r.label == label).expect(label)
    }

    #[test]
    fn the_character_pages_have_their_rows() {
        let r = rider(75.0, Some(178.0));
        let top = rows(Page::Character, &r);
        let labels: Vec<&str> = top.iter().map(|r| r.label.as_str()).collect();
        assert_eq!(labels, ["Body", "Face and age", "Hair", "Facial hair"]);
        for (row, page) in top.iter().zip(PAGES) {
            assert_eq!(row.confirm, Some(Activate::Open(page)));
            assert!(!rows(page, &r).is_empty(), "{page:?}");
            // The ten-foot rule: Back makes at most seven items.
            assert!(rows(page, &r).len() <= 6, "{page:?}");
        }
        assert_eq!(top[0].value.as_deref(), Some("Body A, normal"));
        let face = rows(Page::CharacterFace, &r);
        assert_eq!(
            row(&face, "Age").right.as_deref(),
            Some("character --age 40s")
        );
        assert_eq!(
            row(&face, "Age").left.as_deref(),
            Some("character --age 20s")
        );
        assert_eq!(
            row(&face, "Keep my hair colour").confirm,
            Some(Activate::Command("character --greying off".into()))
        );
        let hair = rows(Page::CharacterHair, &r);
        assert_eq!(row(&hair, "Hair style").value.as_deref(), Some("Short"));
        assert_eq!(
            row(&hair, "Hair style").right.as_deref(),
            Some("character --hair buzz-cut")
        );
        assert_eq!(
            row(&hair, "Hair style").left.as_deref(),
            Some("character --hair cornrows")
        );
        assert_eq!(
            row(&hair, "Hair colour").value.as_deref(),
            Some("Dark brown")
        );
        assert_eq!(
            row(&hair, "Hair colour").right.as_deref(),
            Some("character --hair-colour #6B4426")
        );
        let facial = rows(Page::CharacterFacialHair, &r);
        assert_eq!(
            row(&facial, "Beard").right.as_deref(),
            Some("character --beard stubble")
        );
        assert_eq!(row(&facial, "Colour").value.as_deref(), Some("the hair's"));
    }

    /// The shape row stays in the band (4N Q13; one step either side,
    /// Q23 as proposed) and shows the *Match my weight* marker.
    #[test]
    fn the_shape_row_stays_in_the_band_of_the_weight() {
        let r = rider(75.0, Some(178.0));
        let body = rows(Page::CharacterBody, &r);
        let shape = row(&body, "Shape");
        assert_eq!(shape.value.as_deref(), Some("Normal (matches your weight)"));
        assert_eq!(shape.left.as_deref(), Some("character --shape slim"));
        assert_eq!(shape.right.as_deref(), Some("character --shape ample"));
        assert_eq!(
            row(&body, "Match my weight").confirm,
            Some(Activate::Disabled)
        );
        // At the band's edge the row does not go further.
        let mut ample = r.clone();
        ample.set_character(ocw_core::Character {
            shape: BodyShape::Ample,
            ..r.character()
        });
        let body = rows(Page::CharacterBody, &ample);
        let shape = row(&body, "Shape");
        assert_eq!(shape.value.as_deref(), Some("Ample (your weight: Normal)"));
        assert_eq!(shape.right, None, "fat is outside the band");
        assert_eq!(
            row(&body, "Match my weight").confirm,
            Some(Activate::Command("character --match-weight".into()))
        );
        // The weight and the height move the band.
        let heavy = rider(120.0, Some(178.0));
        let shape = row(&rows(Page::CharacterBody, &heavy), "Shape").clone();
        assert!(shape.value.unwrap().contains("your weight: Obese"));
        let tall = rider(75.0, Some(200.0));
        assert_eq!(tall.weight_shape(), BodyShape::Slim);
    }

    #[test]
    fn the_base_body_row_cycles_a_b_c() {
        let r = rider(70.0, None);
        let body = rows(Page::CharacterBody, &r);
        let base = row(&body, "Base body");
        assert!(base.value.as_deref().unwrap().starts_with("Body A"));
        assert_eq!(base.right.as_deref(), Some("character --body b"));
        assert_eq!(base.left.as_deref(), Some("character --body c"));
    }
}
