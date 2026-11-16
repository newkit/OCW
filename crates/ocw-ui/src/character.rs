//! The `character` console command of both apps (spec §2.6, §14 Phase 4,
//! 4C and 4N; B6: one definition, the apps apply it – the desktop saves
//! the rider profile and sends the look, the browser's anonymous rider
//! gets the locked mirror's answer, Phase 10 Q5). The rows of the menu's
//! character pages (`ocw_client::menu::character`) are lines of it.
//!
//! The shape is bounded to the band around what the profile's weight and
//! height give (4N Q13; `--match-weight` snaps to it); looks only, never
//! physics.

use clap::{Arg, ArgAction, ArgMatches, Command};
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
                .help("Body shape, within the band of your weight and height"),
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
/// `height_cm` (the profile's): `Err` for a shape outside the band
/// (Q13). The result is validated and bounded.
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
        if !s.in_band(marker) {
            let (lo, hi) = ch::shape_band(marker);
            return Err(format!(
                "{} is outside your band: {} to {} (Match my weight: {}, from your weight and height)",
                s.id(),
                lo.id(),
                hi.id(),
                marker.id()
            ));
        }
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
    *c = c.validated().bounded(weight_kg);
    Ok(())
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

    /// The shape stays in the band of the weight (4N Q13), and
    /// `--match-weight` snaps to its marker.
    #[test]
    fn the_shape_is_bounded_and_matches_the_weight() {
        let mut c = Character::default();
        let e = run(&mut c, "--shape obese", 75.0).unwrap_err();
        assert!(e.contains("slim to ample"), "{e}");
        assert_eq!(c.shape, BodyShape::Normal, "unchanged");
        run(&mut c, "--shape ample", 75.0).unwrap();
        assert_eq!(c.shape, BodyShape::Ample);
        run(&mut c, "--match-weight", 75.0).unwrap();
        assert_eq!(c.shape, BodyShape::Normal);
        run(&mut c, "--match-weight", 120.0).unwrap();
        assert_eq!(c.shape, BodyShape::Obese);
        // An older line (W4 builds) still works, inside the band.
        run(&mut c, "--build strong", 90.0).unwrap();
        assert_eq!(c.shape, BodyShape::Ample);
    }
}
