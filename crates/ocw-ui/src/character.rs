//! The `character` console command of both apps (spec §2.6, §14 Phase 4,
//! 4C and 4N; B6: one definition, the apps apply it – the desktop saves
//! the rider profile and sends the look, the browser's anonymous rider
//! gets the locked mirror's answer, Phase 10 Q5). The rows of the menu's
//! character pages (`ocw_client::menu::character`) are lines of it.
//!
//! Every shape can be chosen whatever weight and height say (4N Q23);
//! `--match-weight` takes the shape they propose. Looks only, never
//! physics.

use bevy::prelude::*;
use clap::{Arg, ArgAction, ArgMatches, Command};
use ocw_client::menu::character as editor;
use ocw_client::menu::pages as sub;
use ocw_core::character::{self as ch, Age, BEARDS, GLASSES, MOUSTACHES, SKIN_TONES};
use ocw_core::{BaseBody, BodyShape, Character, Rgb};

fn index_arg(name: &'static str, n: usize) -> Arg {
    Arg::new(name)
        .long(name)
        .value_name("N")
        .value_parser(clap::value_parser!(u8).range(1..=n as i64))
        .help(format!("1 … {n}"))
}

/// A colour `#RRGGBB` or a name of the natural hair palette ("blond").
fn hair_colour(s: &str) -> Result<Rgb, String> {
    Rgb::parse(s)
        .or_else(|| {
            ch::HAIR_PALETTE
                .iter()
                .find(|(n, _)| n.eq_ignore_ascii_case(s.trim()))
                .map(|(_, c)| *c)
        })
        .ok_or_else(|| format!("{s:?} is not a colour #RRGGBB or a hair colour"))
}

/// The command's definition (the same in both apps).
pub fn command() -> Command {
    Command::new("character")
        .about("The rider's character: free choices, never physics (no options: the current one)")
        .arg(
            Arg::new("body")
                .long("body")
                .value_name("a|b|c")
                .value_parser(|s: &str| {
                    BaseBody::parse(s).ok_or_else(|| format!("{s:?}: body a, b or c"))
                })
                .help("Base body: a (broader shoulders), b (wider hips, a chest), c (in between)"),
        )
        .arg(
            Arg::new("shape")
                .long("shape")
                .value_name("SHAPE")
                .value_parser(|s: &str| {
                    BodyShape::parse(s).ok_or_else(|| {
                        format!("{s:?}: anorexia, skinny, slim, normal, ample, fat or obese")
                    })
                })
                .help("Body shape (any; --match-weight: the one your weight and height propose)"),
        )
        .arg(
            Arg::new("build")
                .long("build")
                .value_parser(ocw_core::BodyBuild::ALL.map(|b| b.id()))
                .hide(true),
        )
        .arg(
            Arg::new("match_weight")
                .long("match-weight")
                .action(ArgAction::SetTrue)
                .conflicts_with_all(["shape", "build"])
                .help("The shape your weight and height give"),
        )
        .arg(
            Arg::new("age")
                .long("age")
                .value_name("20s … 70+")
                .value_parser(|s: &str| {
                    Age::parse(s).ok_or_else(|| format!("{s:?}: 20s, 30s, 40s, 50s, 60s or 70+"))
                }),
        )
        .arg(index_arg("skin", SKIN_TONES.len()))
        .arg(index_arg("face", ch::FACES.len()))
        .arg(
            Arg::new("hair")
                .long("hair")
                .value_name("STYLE")
                .value_parser(|s: &str| {
                    // 1-based numbers, as the other index options.
                    match s.trim().parse::<u8>() {
                        Ok(n) if (1..=ch::HAIR_STYLES.len() as u8).contains(&n) => Ok(n - 1),
                        Ok(_) => Err(format!("1 … {}", ch::HAIR_STYLES.len())),
                        Err(_) => ch::hair_parse(s).ok_or_else(|| format!("no hair style {s:?}")),
                    }
                }),
        )
        .arg(
            Arg::new("hair_colour")
                .long("hair-colour")
                .value_name("#RRGGBB")
                .value_parser(hair_colour),
        )
        .arg(
            Arg::new("greying")
                .long("greying")
                .value_parser(["on", "off"])
                .help("off: keep my hair colour"),
        )
        .arg(
            Arg::new("moustache")
                .long("moustache")
                .value_parser(|s: &str| {
                    ch::list_parse(&MOUSTACHES, s).ok_or_else(|| format!("no moustache {s:?}"))
                }),
        )
        .arg(Arg::new("beard").long("beard").value_parser(|s: &str| {
            ch::list_parse(&BEARDS, s).ok_or_else(|| format!("no beard {s:?}"))
        }))
        .arg(
            Arg::new("facial_hair_colour")
                .long("facial-hair-colour")
                .value_name("#RRGGBB|hair")
                .value_parser(|s: &str| {
                    if s.trim().eq_ignore_ascii_case("hair") {
                        Ok(None)
                    } else {
                        hair_colour(s).map(Some)
                    }
                }),
        )
        .arg(index_arg("glasses", GLASSES.len()))
}

/// Applies the options to `c` for a rider of `weight_kg` and
/// `height_cm` (the profile's: they give `--match-weight`'s shape). The
/// result is validated.
pub fn apply(
    c: &mut Character,
    m: &ArgMatches,
    weight_kg: f64,
    height_cm: Option<f64>,
) -> Result<(), String> {
    let marker = ch::weight_shape(weight_kg, height_cm);
    if let Some(b) = m.get_one::<BaseBody>("body") {
        c.base = *b;
    }
    let shape = m.get_one::<BodyShape>("shape").copied().or_else(|| {
        m.get_one::<String>("build")
            .and_then(|b| ocw_core::BodyBuild::parse(b))
            .map(BodyShape::from_build)
    });
    if let Some(s) = shape {
        c.shape = s;
    }
    if m.get_flag("match_weight") {
        c.shape = marker;
    }
    if let Some(a) = m.get_one::<Age>("age") {
        c.age = *a;
    }
    let idx = |k: &str| m.get_one::<u8>(k).map(|v| v - 1);
    if let Some(v) = idx("skin") {
        c.skin = v;
    }
    if let Some(v) = idx("face") {
        c.face = v;
    }
    if let Some(v) = m.get_one::<u8>("hair") {
        c.hair = *v;
    }
    if let Some(v) = m.get_one::<Rgb>("hair_colour") {
        c.hair_colour = *v;
    }
    if let Some(g) = m.get_one::<String>("greying") {
        c.greying = g == "on";
    }
    if let Some(v) = m.get_one::<u8>("moustache") {
        c.moustache = *v;
    }
    if let Some(v) = m.get_one::<u8>("beard") {
        c.beard = *v;
    }
    if let Some(v) = m.get_one::<Option<Rgb>>("facial_hair_colour") {
        c.facial_hair_colour = *v;
    }
    if let Some(v) = idx("glasses") {
        c.glasses = v;
    }
    *c = c.validated();
    Ok(())
}

/// The character editor's live view (spec §14 Phase 4, 4N *Visual
/// editor*; both apps): while a character page of the menu is open the
/// panel is docked at the side (`MenuView::side`), the camera turns to the
/// rider's front – close on the face for face, hair and age, the whole
/// rider for the body – and on a tile page the rider shows the focused
/// tile at once (no apply step); leaving the editor puts the camera back
/// and the rider shows its saved character again.
pub struct CharacterEditorPlugin;

impl Plugin for CharacterEditorPlugin {
    fn build(&self, app: &mut App) {
        crate::configure_sets(app);
        app.init_resource::<SavedOrbit>().add_systems(
            Update,
            (preview_focused_tile, frame_the_rider).in_set(crate::ViewExtrasSet),
        );
    }
}

/// The open menu page and its focus, if the menu is open.
fn open_page(world: &World) -> Option<(ocw_client::menu::model::Page, usize)> {
    let state = world.get_resource::<crate::menu::MenuState>()?;
    let m = &state.menu;
    if !m.is_open() {
        return None;
    }
    m.page().map(|p| (p, m.focus()))
}

/// The editor page open now (a character page of Workshop → Character).
pub fn editor_page(world: &World) -> Option<(sub::Page, usize)> {
    match open_page(world)? {
        (ocw_client::menu::model::Page::Sub(p), focus) if editor::is_editor_page(p) => {
            Some((p, focus))
        }
        _ => None,
    }
}

/// The character the rider beside the panel shows (`None`: the saved
/// one): the focused tile's on a tile page.
pub fn previewed(world: &World) -> Option<Character> {
    let (sub::Page::CharacterPick(attr), focus) = editor_page(world)? else {
        return None;
    };
    let hooks = world
        .get_resource::<crate::menu::MenuHooks>()
        .copied()
        .unwrap_or_default();
    let rider = (hooks.riders)(world).map_or_else(
        ocw_client::riders::Rider::default,
        ocw_client::riders::Riders::active,
    );
    editor::preview(attr, focus, &rider)
}

fn preview_focused_tile(world: &mut World) {
    let Some(c) = previewed(world) else {
        return;
    };
    let Some(mut view) = world.get_resource_mut::<ocw_render::RideView>() else {
        return;
    };
    if let Some(r) = view.riders.iter_mut().find(|r| r.is_local) {
        let mut look = r.look.unwrap_or_default();
        // The height stays the profile's (and the view's).
        look.character = Character {
            height_cm: look.character.height_cm,
            ..c
        };
        if r.look != Some(look) {
            r.look = Some(look);
        }
    }
}

/// The camera of the editor: in front of the rider, a little to its side
/// (the panel covers the window's left third), close for the face.
pub fn editor_orbit(page: sub::Page) -> ocw_render::camera::CameraOrbit {
    let face = match page {
        sub::Page::CharacterPick(a) => a.face(),
        sub::Page::CharacterBody | sub::Page::Character => false,
        _ => true,
    };
    ocw_render::camera::CameraOrbit {
        yaw: std::f32::consts::PI - 0.45,
        pitch: if face { 0.42 } else { 0.22 },
        distance: if face {
            ocw_render::camera::MIN_DISTANCE
        } else {
            4.4
        },
    }
}

/// The camera's orbit before the editor opened (put back on leaving).
#[derive(Resource, Default)]
struct SavedOrbit(Option<ocw_render::camera::CameraOrbit>);

fn frame_the_rider(world: &mut World) {
    let page = editor_page(world).map(|(p, _)| p);
    let saved = world.get_resource::<SavedOrbit>().and_then(|s| s.0);
    let Some(orbit) = world
        .get_resource::<ocw_render::camera::CameraOrbit>()
        .copied()
    else {
        return;
    };
    match (page, saved) {
        (Some(p), _) => {
            if saved.is_none() {
                world.insert_resource(SavedOrbit(Some(orbit)));
            }
            let want = editor_orbit(p);
            if orbit != want {
                *world.resource_mut::<ocw_render::camera::CameraOrbit>() = want;
            }
        }
        (None, Some(o)) => {
            *world.resource_mut::<ocw_render::camera::CameraOrbit>() = o;
            world.insert_resource(SavedOrbit(None));
        }
        (None, None) => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(c: &mut Character, line: &str, kg: f64) -> Result<(), String> {
        let args = std::iter::once("character").chain(line.split_whitespace());
        let m = command()
            .try_get_matches_from(args)
            .map_err(|e| e.to_string())?;
        apply(c, &m, kg, Some(178.0))
    }

    #[test]
    fn the_command_sets_every_4n_choice() {
        let mut c = Character::default();
        run(
            &mut c,
            "--body b --shape slim --age 60s --hair single-braid --hair-colour blond \
             --greying off --moustache handlebar --beard short-boxed \
             --facial-hair-colour #6B4426 --skin 8 --glasses 3",
            75.0,
        )
        .unwrap();
        assert_eq!(c.base, BaseBody::B);
        assert_eq!(c.shape, BodyShape::Slim);
        assert_eq!(c.age, Age::Sixties);
        assert_eq!(ch::HAIR_STYLES[c.hair as usize], "Single braid");
        assert_eq!(c.hair_colour, Rgb(0xD8B27A));
        assert!(!c.greying);
        assert_eq!(MOUSTACHES[c.moustache as usize], "Handlebar");
        assert_eq!(BEARDS[c.beard as usize], "Short boxed");
        assert_eq!(c.facial_hair_colour, Some(Rgb(0x6B4426)));
        assert_eq!((c.skin, c.glasses), (7, 2));
        run(&mut c, "--facial-hair-colour hair --hair 4", 75.0).unwrap();
        assert_eq!(c.facial_hair_colour, None);
        assert_eq!(c.hair, 3, "1-based numbers");
        assert!(run(&mut c, "--hair mohawk", 75.0).is_err());
        assert!(run(&mut c, "--age 90s", 75.0).is_err());
    }

    /// Every shape can be chosen for any weight and height (4N Q23);
    /// `--match-weight` takes the proposed one.
    #[test]
    fn every_shape_is_selectable_and_match_weight_proposes() {
        for kg in [45.0, 75.0, 130.0] {
            for s in BodyShape::ALL {
                let mut c = Character::default();
                run(&mut c, &format!("--shape {}", s.id()), kg).unwrap();
                assert_eq!(c.shape, s, "{kg} kg");
            }
        }
        let mut c = Character::default();
        run(&mut c, "--match-weight", 75.0).unwrap();
        assert_eq!(c.shape, BodyShape::Normal);
        run(&mut c, "--match-weight", 120.0).unwrap();
        assert_eq!(c.shape, BodyShape::Obese);
        run(&mut c, "--match-weight", 45.0).unwrap();
        assert_eq!(c.shape, BodyShape::Anorexia);
        // An older line (W4 builds) still works.
        run(&mut c, "--build strong", 50.0).unwrap();
        assert_eq!(c.shape, BodyShape::Ample);
    }
}
