//! The character pages of the in-game menu (spec §14 Phase 4, 4N N10;
//! both apps, B6): Workshop → Character with Body (base body A / B / C,
//! the seven shapes – any of them, weight and height only propose one with
//! the *Match my
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

/// The shape row: ← / → through all seven (4N Q23: no limit; no wrap
/// from obese to super skinny), the value with the *Match my weight* marker
/// (the shape weight and height propose, Q13).
fn shape_row(now: BodyShape, marker: BodyShape) -> Row {
    let step = |d: isize| {
        let s = BodyShape::at(now.index() as isize + d);
        (s != now).then(|| line(&format!("--shape {}", s.id())))
    };
    let mark = if now == marker {
        " (your weight)".to_string()
    } else {
        format!(" (weight: {})", marker.name())
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
/// the default rider without profiles): the categories' rows (Confirm
/// opens an attribute's tiles, ← / → step it) and the tile pages.
pub fn rows(page: Page, rider: &Rider) -> Vec<Row> {
    match page {
        Page::CharacterPick(a) => tile_rows(a, rider),
        p => with_tiles(p, category_rows(p, rider)),
    }
}

fn category_rows(page: Page, rider: &Rider) -> Vec<Row> {
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
                    format!(
                        "{}, skin {} of {}",
                        c.age.id(),
                        c.skin + 1,
                        SKIN_TONES.len()
                    ),
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
                    c.base.name().into(),
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
                    value: Some(format!("{:.2} m (profile)", c.scale() * 1.78)),
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
                    label: "Grey hair with age".into(),
                    value: Some(if c.greying { "on" } else { "off" }.into()),
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

/// One attribute of the character, chosen on a page of tiles (spec §14
/// Phase 4, 4N *Visual editor*: every choice a picture or a swatch, the
/// rider beside the panel showing the focused one at once).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Attr {
    Base,
    Shape,
    Age,
    Skin,
    Glasses,
    HairStyle,
    HairColour,
    Moustache,
    Beard,
    FacialColour,
}

impl Attr {
    pub const ALL: [Attr; 10] = [
        Attr::Base,
        Attr::Shape,
        Attr::Age,
        Attr::Skin,
        Attr::Glasses,
        Attr::HairStyle,
        Attr::HairColour,
        Attr::Moustache,
        Attr::Beard,
        Attr::FacialColour,
    ];

    /// The row of the category pages that opens its tiles.
    pub fn row_label(self) -> &'static str {
        match self {
            Attr::Base => "Base body",
            Attr::Shape => "Shape",
            Attr::Age => "Age",
            Attr::Skin => "Skin tone",
            Attr::Glasses => "Glasses",
            Attr::HairStyle => "Hair style",
            Attr::HairColour => "Hair colour",
            Attr::Moustache => "Moustache",
            Attr::Beard => "Beard",
            Attr::FacialColour => "Colour",
        }
    }

    /// The tiles' page title.
    pub fn title(self) -> &'static str {
        match self {
            Attr::Base => "Base body",
            Attr::Shape => "Body shape",
            Attr::Age => "Age",
            Attr::Skin => "Skin tone",
            Attr::Glasses => "Glasses",
            Attr::HairStyle => "Hair style",
            Attr::HairColour => "Hair colour",
            Attr::Moustache => "Moustache",
            Attr::Beard => "Beard",
            Attr::FacialColour => "Beard colour",
        }
    }

    /// The category page it belongs to.
    pub fn page(self) -> Page {
        match self {
            Attr::Base | Attr::Shape => Page::CharacterBody,
            Attr::Age | Attr::Skin | Attr::Glasses => Page::CharacterFace,
            Attr::HairStyle | Attr::HairColour => Page::CharacterHair,
            Attr::Moustache | Attr::Beard | Attr::FacialColour => Page::CharacterFacialHair,
        }
    }

    /// Whether the camera frames the face (else the whole rider).
    pub fn face(self) -> bool {
        !matches!(self, Attr::Base | Attr::Shape)
    }

    fn of_row(label: &str, page: Page) -> Option<Attr> {
        Attr::ALL
            .into_iter()
            .find(|a| a.row_label() == label && a.page() == page)
    }
}

/// Columns of the tile pages.
pub const TILE_COLUMNS: usize = 5;

/// One choice shown as a tile: its label, a colour swatch (colours) or
/// none (a named choice), whether it is the current one, the console line
/// that applies it and the character it shows (the preview).
#[derive(Debug, Clone, PartialEq)]
pub struct Tile {
    pub label: String,
    pub swatch: Option<Rgb>,
    pub current: bool,
    pub line: String,
    pub preview: ocw_core::Character,
}

/// The tiles of `attr` for `rider` (the active profile), in list order.
/// The shape's tile that weight and height propose says so (*Match my
/// weight*, 4N Q13 / Q23); every tile can be chosen.
pub fn tiles(attr: Attr, rider: &Rider) -> Vec<Tile> {
    let c = rider.character();
    let marker = rider.weight_shape();
    let tile =
        |label: String, swatch: Option<Rgb>, args: String, preview: ocw_core::Character| Tile {
            current: preview == c,
            label,
            swatch,
            line: line(&args),
            preview,
        };
    use ocw_core::Character as C;
    match attr {
        Attr::Base => BaseBody::ALL
            .into_iter()
            .map(|b| {
                tile(
                    b.name().into(),
                    None,
                    format!("--body {}", b.id()),
                    C { base: b, ..c },
                )
            })
            .collect(),
        Attr::Shape => BodyShape::ALL
            .into_iter()
            .map(|s| {
                let label = if s == marker {
                    format!("{} (Match my weight)", s.name())
                } else {
                    s.name().into()
                };
                tile(
                    label,
                    None,
                    format!("--shape {}", s.id()),
                    C { shape: s, ..c },
                )
            })
            .collect(),
        Attr::Age => Age::ALL
            .into_iter()
            .map(|a| {
                tile(
                    a.id().into(),
                    None,
                    format!("--age {}", a.id()),
                    C { age: a, ..c },
                )
            })
            .collect(),
        Attr::Skin => (0..SKIN_TONES.len() as u8)
            .map(|i| {
                let p = C { skin: i, ..c };
                tile(
                    format!("{}", i + 1),
                    Some(p.skin_colour()),
                    format!("--skin {}", i + 1),
                    p,
                )
            })
            .collect(),
        Attr::Glasses => (0..GLASSES.len() as u8)
            .map(|i| {
                tile(
                    GLASSES[i as usize].into(),
                    None,
                    format!("--glasses {}", i + 1),
                    C { glasses: i, ..c },
                )
            })
            .collect(),
        Attr::HairStyle => (0..HAIR_STYLES.len() as u8)
            .map(|i| {
                tile(
                    HAIR_STYLES[i as usize].into(),
                    None,
                    format!("--hair {}", ch::hair_id(i)),
                    C { hair: i, ..c },
                )
            })
            .collect(),
        Attr::HairColour => HAIR_PALETTE
            .iter()
            .map(|(name, rgb)| {
                tile(
                    (*name).into(),
                    Some(*rgb),
                    format!("--hair-colour {rgb}"),
                    C {
                        hair_colour: *rgb,
                        ..c
                    },
                )
            })
            .collect(),
        Attr::Moustache => (0..MOUSTACHES.len() as u8)
            .map(|i| {
                tile(
                    MOUSTACHES[i as usize].into(),
                    None,
                    format!("--moustache {}", ch::list_id(&MOUSTACHES, i)),
                    C { moustache: i, ..c },
                )
            })
            .collect(),
        Attr::Beard => (0..BEARDS.len() as u8)
            .map(|i| {
                tile(
                    BEARDS[i as usize].into(),
                    None,
                    format!("--beard {}", ch::list_id(&BEARDS, i)),
                    C { beard: i, ..c },
                )
            })
            .collect(),
        Attr::FacialColour => std::iter::once(tile(
            "Hair's".into(),
            Some(
                C {
                    facial_hair_colour: None,
                    ..c
                }
                .facial_hair_shown(),
            ),
            "--facial-hair-colour hair".into(),
            C {
                facial_hair_colour: None,
                ..c
            },
        ))
        .chain(HAIR_PALETTE.iter().map(|(name, rgb)| {
            tile(
                (*name).into(),
                Some(*rgb),
                format!("--facial-hair-colour {rgb}"),
                C {
                    facial_hair_colour: Some(*rgb),
                    ..c
                },
            )
        }))
        .collect(),
    }
}

/// The rows of a tile page: one per tile (Confirm applies it), then the
/// Back the menu adds.
pub fn tile_rows(attr: Attr, rider: &Rider) -> Vec<Row> {
    tiles(attr, rider)
        .into_iter()
        .map(|t| Row {
            label: t.label,
            value: None,
            left: None,
            right: None,
            confirm: Some(Activate::Command(t.line)),
        })
        .collect()
}

/// What a tile's picture frames (spec §14 Phase 4, 4N: a rendering of
/// the model for every choice that is not a colour); `None`: a colour
/// swatch.
pub fn thumb_shot(attr: Attr) -> Option<ocw_render::thumbs::RiderShot> {
    use ocw_render::thumbs::RiderShot;
    match attr {
        Attr::Base | Attr::Shape => Some(RiderShot::Body),
        Attr::Glasses => Some(RiderShot::FaceFront),
        Attr::Age | Attr::HairStyle | Attr::Moustache | Attr::Beard => Some(RiderShot::Face),
        Attr::Skin | Attr::HairColour | Attr::FacialColour => None,
    }
}

/// The picture of tile `t` of `attr`: the rider standing with `look` (its
/// kit colours) and the tile's character, wearing `kit`.
pub fn tile_thumb(
    attr: Attr,
    t: &Tile,
    look: &ocw_core::RiderLook,
    kit: &[String],
) -> Option<ocw_render::thumbs::ThumbKey> {
    use ocw_render::thumbs::{RiderThumb, ThumbKey};
    let shot = thumb_shot(attr)?;
    let mut kit = kit.to_vec();
    // The face's pictures without the helmet (4N Q28: the hair as it is).
    if matches!(
        shot,
        ocw_render::thumbs::RiderShot::Face | ocw_render::thumbs::RiderShot::FaceFront
    ) && !kit.iter().any(|k| k == ocw_render::thumbs::HELMET_OFF)
    {
        kit.push(ocw_render::thumbs::HELMET_OFF.to_string());
    }
    Some(ThumbKey::Rider(RiderThumb {
        look: ocw_core::RiderLook {
            character: t.preview,
            ..*look
        },
        kit,
        shot,
    }))
}

/// The tile page as a grid of cells (renderings for the shapes, styles,
/// beards …, swatches for the colours, the current choice framed), Back
/// last – for the default look without kit ([`grid_for`]: the rider's).
pub fn grid(attr: Attr, rider: &Rider) -> ocw_render::menu::MenuGrid {
    grid_for(attr, rider, &ocw_core::RiderLook::default(), &[])
}

/// [`grid`] with the pictures of the rider's `look` and worn `kit`.
pub fn grid_for(
    attr: Attr,
    rider: &Rider,
    look: &ocw_core::RiderLook,
    kit: &[String],
) -> ocw_render::menu::MenuGrid {
    use ocw_render::menu::{GridCell, MenuGrid};
    MenuGrid {
        columns: TILE_COLUMNS,
        cells: tiles(attr, rider)
            .into_iter()
            .map(|t| GridCell {
                thumb: tile_thumb(attr, &t, look, kit),
                icon: None,
                swatch: t.swatch.map(|c| c.channels()),
                marked: t.current,
                label: t.label,
            })
            .chain([GridCell {
                label: "Back".into(),
                ..Default::default()
            }])
            .collect(),
    }
}

/// The status line of a tile page: the focused tile's name where the
/// pictures stand without names (a large grid of renderings, the hair
/// styles), else empty.
pub fn status(attr: Attr, rider: &Rider, focus: usize) -> String {
    let g = grid(attr, rider);
    if ocw_render::menu::grid_labels(&g) {
        return String::new();
    }
    g.cells
        .get(focus)
        .filter(|c| c.thumb.is_some())
        .map(|c| c.label.clone())
        .unwrap_or_default()
}

/// The character the rider beside the panel shows while tile `focus` of
/// `attr` has the focus (the live preview; `None` on Back: the saved
/// character).
pub fn preview(attr: Attr, focus: usize, rider: &Rider) -> Option<ocw_core::Character> {
    tiles(attr, rider).get(focus).map(|t| t.preview)
}

/// Whether `page` is one of the character editor's pages (the side panel
/// and the framed rider).
pub fn is_editor_page(page: Page) -> bool {
    matches!(
        page,
        Page::Character
            | Page::CharacterBody
            | Page::CharacterFace
            | Page::CharacterHair
            | Page::CharacterFacialHair
            | Page::CharacterPick(_)
    )
}

/// Category rows open their tiles on Confirm (← / → still step).
fn with_tiles(page: Page, rows: Vec<Row>) -> Vec<Row> {
    rows.into_iter()
        .map(|mut r| {
            if let Some(a) = Attr::of_row(&r.label, page) {
                r.confirm = Some(Activate::Open(Page::CharacterPick(a)));
            }
            r
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The skin tone selector (asked 2026-11-17: "add skin tone selector
    /// to character editor"): the face page's row opens a page of all 16
    /// tones as swatches, light to dark, the current one framed; each tile
    /// previews its tone on the rider (the lips and the age's tone follow
    /// it) and applies with one `character --skin N` line; Back previews
    /// nothing.
    #[test]
    fn the_skin_tone_page_is_a_grid_of_every_tone() {
        let r = Rider::default();
        let row = rows(Page::CharacterFace, &r)
            .into_iter()
            .find(|row| row.label == "Skin tone")
            .expect("the face page's skin row");
        assert_eq!(
            row.confirm,
            Some(Activate::Open(Page::CharacterPick(Attr::Skin)))
        );
        let g = grid(Attr::Skin, &r);
        assert_eq!(g.cells.len(), SKIN_TONES.len() + 1, "16 tones and Back");
        for (i, (cell, tone)) in g.cells.iter().zip(SKIN_TONES).enumerate() {
            assert_eq!(cell.swatch, Some(tone.channels()), "{i}");
            assert!(cell.thumb.is_none(), "a swatch, not a rendering");
            assert_eq!(cell.marked, i == usize::from(r.character().skin));
        }
        let ts = tiles(Attr::Skin, &r);
        for (i, t) in ts.iter().enumerate() {
            assert_eq!(t.line, format!("character --skin {}", i + 1));
            let c = preview(Attr::Skin, i, &r).expect("a preview");
            assert_eq!(usize::from(c.skin), i);
            assert_eq!(c.skin_colour(), SKIN_TONES[i], "30s: the tone itself");
            // The lips and an older face follow the tone.
            assert_ne!(c.lip_colour(), c.skin_colour());
            let old = ocw_core::Character {
                age: Age::Seventies,
                ..c
            };
            assert_ne!(old.skin_colour(), c.skin_colour());
            if i > 0 {
                let prev = preview(Attr::Skin, i - 1, &r).unwrap();
                assert_ne!(prev.lip_colour(), c.lip_colour());
            }
        }
        assert_eq!(preview(Attr::Skin, SKIN_TONES.len(), &r), None, "Back");
        assert_eq!(thumb_shot(Attr::Skin), None, "colours stay swatches");
    }

    /// The tiles' pictures (spec §14 Phase 4, 4N): every choice that is
    /// not a colour shows a rendering of the rider with that choice – the
    /// face for hair, facial hair and age, the face from the front for the
    /// glasses, the whole rider for the bodies and shapes – keyed by the
    /// look with the tile's character, the rider's kit colours and its
    /// worn kit; colours stay swatches; Back is a name.
    #[test]
    fn every_non_colour_tile_shows_a_rendering_of_its_look() {
        use ocw_render::thumbs::{RiderShot, ThumbKey};
        let r = Rider::default();
        let look = ocw_core::RiderLook {
            jersey: Rgb(0x2266AA),
            ..Default::default()
        };
        let kit = vec!["helmet-road".to_string()];
        for a in Attr::ALL {
            let g = grid_for(a, &r, &look, &kit);
            let ts = tiles(a, &r);
            assert_eq!(g.cells.len(), ts.len() + 1, "{a:?}");
            let back = g.cells.last().unwrap();
            assert!(back.thumb.is_none() && back.swatch.is_none());
            for (cell, t) in g.cells.iter().zip(&ts) {
                match thumb_shot(a) {
                    None => {
                        assert!(cell.thumb.is_none() && cell.swatch.is_some(), "{a:?}")
                    }
                    Some(shot) => {
                        let Some(ThumbKey::Rider(k)) = &cell.thumb else {
                            panic!("{a:?}: a rendering");
                        };
                        assert!(cell.swatch.is_none());
                        assert_eq!(k.shot, shot);
                        assert_eq!(k.look.character, t.preview, "{a:?}: the tile's look");
                        assert_eq!(k.look.jersey, look.jersey, "the rider's kit colours");
                        assert!(k.kit.starts_with(&kit), "the worn kit");
                        let face = matches!(shot, RiderShot::Face | RiderShot::FaceFront);
                        assert_eq!(
                            k.kit.iter().any(|k| k == ocw_render::thumbs::HELMET_OFF),
                            face,
                            "{a:?}: the face's pictures bare-headed (Q28)"
                        );
                    }
                }
            }
            // Each tile its own picture.
            let mut keys: Vec<_> = g.cells.iter().filter_map(|c| c.thumb.clone()).collect();
            let n = keys.len();
            keys.dedup();
            assert_eq!(keys.len(), n, "{a:?}");
        }
        assert_eq!(thumb_shot(Attr::Shape), Some(RiderShot::Body));
        assert_eq!(thumb_shot(Attr::Glasses), Some(RiderShot::FaceFront));
        assert_eq!(thumb_shot(Attr::HairStyle), Some(RiderShot::Face));
        // The 21 hair styles stand without names: the status names the
        // focused one; the shapes keep theirs (the *Match my weight* one).
        assert_eq!(status(Attr::HairStyle, &r, 3), HAIR_STYLES[3]);
        assert_eq!(status(Attr::Shape, &r, 3), "");
        assert_eq!(status(Attr::HairStyle, &r, HAIR_STYLES.len()), "", "Back");
    }

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
            row(&face, "Grey hair with age").confirm,
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

    /// The shape row reaches every shape for any weight (4N Q23) and
    /// shows the *Match my weight* proposal.
    #[test]
    fn the_shape_row_reaches_every_shape_and_marks_the_proposal() {
        let r = rider(75.0, Some(178.0));
        let body = rows(Page::CharacterBody, &r);
        let shape = row(&body, "Shape");
        assert_eq!(shape.value.as_deref(), Some("Normal (your weight)"));
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
        assert_eq!(shape.value.as_deref(), Some("Ample (weight: Normal)"));
        assert_eq!(shape.right.as_deref(), Some("character --shape fat"));
        assert_eq!(
            row(&body, "Match my weight").confirm,
            Some(Activate::Command("character --match-weight".into()))
        );
        // The weight and the height move the band.
        let heavy = rider(120.0, Some(178.0));
        let shape = row(&rows(Page::CharacterBody, &heavy), "Shape").clone();
        assert!(shape.value.unwrap().contains("(weight: Obese)"));
        let tall = rider(75.0, Some(200.0));
        assert_eq!(tall.weight_shape(), BodyShape::Slim);
        // Walking the row from super skinny reaches obese at 75 kg.
        let mut walk = r.clone();
        walk.set_character(ocw_core::Character {
            shape: BodyShape::SuperSkinny,
            ..r.character()
        });
        assert_eq!(row(&rows(Page::CharacterBody, &walk), "Shape").left, None);
        let mut seen = vec![walk.character().shape];
        while let Some(next) = row(&rows(Page::CharacterBody, &walk), "Shape")
            .right
            .clone()
        {
            let id = next.trim_start_matches("character --shape ");
            let shape = BodyShape::parse(id).unwrap();
            walk.set_character(ocw_core::Character {
                shape,
                ..walk.character()
            });
            seen.push(shape);
        }
        assert_eq!(seen, BodyShape::ALL);
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

    /// The tile model (4N *Visual editor*): every attribute's choices as
    /// tiles – colours as swatches, the current one marked, the shape that
    /// weight and height propose labelled *Match my weight* –, each with
    /// the line that applies it and the character it previews.
    #[test]
    fn every_attribute_has_its_tiles() {
        let r = rider(75.0, Some(178.0));
        let c = r.character();
        let counts = [
            (Attr::Base, 3),
            (Attr::Shape, 7),
            (Attr::Age, 6),
            (Attr::Skin, 16),
            (Attr::Glasses, 3),
            (Attr::HairStyle, 21),
            (Attr::HairColour, 12),
            (Attr::Moustache, 6),
            (Attr::Beard, 8),
            (Attr::FacialColour, 13),
        ];
        for (a, n) in counts {
            let t = tiles(a, &r);
            assert_eq!(t.len(), n, "{a:?}");
            assert_eq!(t.iter().filter(|t| t.current).count(), 1, "{a:?}");
            let colours = matches!(a, Attr::Skin | Attr::HairColour | Attr::FacialColour);
            assert!(t.iter().all(|t| t.swatch.is_some() == colours), "{a:?}");
            assert!(t.iter().all(|t| t.line.starts_with("character --")));
            // Each tile previews a different character; the current one is
            // the saved character.
            let previews: std::collections::BTreeSet<String> =
                t.iter().map(|t| format!("{:?}", t.preview)).collect();
            assert_eq!(previews.len(), n, "{a:?}");
            assert!(t.iter().any(|t| t.preview == c));
            // The grid: one cell per tile, Back last, the current framed.
            let g = grid(a, &r);
            assert_eq!(g.cells.len(), n + 1);
            assert_eq!(g.cells.last().unwrap().label, "Back");
            assert_eq!(g.columns, TILE_COLUMNS);
            assert_eq!(tile_rows(a, &r).len(), n);
            assert_eq!(preview(a, n, &r), None, "Back previews nothing");
            // Each attribute's row opens its tiles.
            let row_ = rows(a.page(), &r)
                .into_iter()
                .find(|row| row.label == a.row_label())
                .expect("its row");
            assert_eq!(row_.confirm, Some(Activate::Open(Page::CharacterPick(a))));
            assert_eq!(Page::CharacterPick(a).parent(), Some(a.page()));
        }
        let shape = tiles(Attr::Shape, &rider(120.0, Some(178.0)));
        assert_eq!(shape[6].label, "Obese (Match my weight)");
        assert!(shape.iter().filter(|t| t.label.contains("Match")).count() == 1);
        let skin = tiles(Attr::Skin, &r);
        assert_eq!(skin[3].swatch, Some(SKIN_TONES[3]));
        assert_eq!(skin[3].line, "character --skin 4");
        assert_eq!(
            preview(Attr::HairStyle, 16, &r).map(|c| c.hair),
            Some(16),
            "the focused tile is what the rider shows"
        );
    }

    /// The tiles are selectable with every input (4N *Visual editor*):
    /// the arrows (keyboard and pad) move the focus over the grid – right
    /// one tile, down a row –, the pointer's hover focuses a tile and its
    /// click applies it, Confirm applies the focused one.
    #[test]
    fn tiles_are_selected_with_keys_pad_and_pointer() {
        use crate::menu::model::{Action, Effect, Menu, Page as M};
        use crate::riders::Riders;
        let s = crate::settings::Settings::default();
        let mut riders = Riders::default();
        riders.add(rider(75.0, Some(178.0))).unwrap();
        let act = |m: &mut Menu, a: Action| {
            m.apply_with(a, |p| super::super::pages::rows(p, &s, &riders))
        };
        let mut m = Menu::default();
        act(&mut m, Action::RiderPage);
        assert_eq!(m.page(), Some(M::Sub(Page::Workshop)));
        // Workshop: the rider, Bike, Character, Edit riders.
        act(&mut m, Action::Click(2));
        assert_eq!(m.page(), Some(M::Sub(Page::Character)));
        act(&mut m, Action::Click(2));
        assert_eq!(m.page(), Some(M::Sub(Page::CharacterHair)));
        act(&mut m, Action::Click(0));
        assert_eq!(m.page(), Some(M::Sub(Page::CharacterPick(Attr::HairStyle))));
        assert_eq!(m.grid_columns(), Some(TILE_COLUMNS));
        let at = m.focus();
        act(&mut m, Action::Right);
        assert_eq!(m.focus(), at + 1);
        act(&mut m, Action::Down);
        assert_eq!(m.focus(), at + 1 + TILE_COLUMNS);
        assert_eq!(
            act(&mut m, Action::Confirm),
            Effect::Command(format!(
                "character --hair {}",
                ch::hair_id((at + 1 + TILE_COLUMNS) as u8)
            ))
        );
        act(&mut m, Action::Hover(11));
        assert_eq!(m.focus(), 11);
        assert_eq!(
            act(&mut m, Action::Click(16)),
            Effect::Command("character --hair single-braid".into())
        );
        // Back goes to the category page.
        act(&mut m, Action::Back);
        assert_eq!(m.page(), Some(M::Sub(Page::CharacterHair)));
    }
}
