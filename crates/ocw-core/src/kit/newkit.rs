//! The "newk.it" kit (spec 14 Phase 8, 8E *The newk.it kit*): a designed
//! rider kit after the user's own jersey and bib-shorts design for their
//! brand "newk it" – the design and the name newk.it are the user's, used
//! in OCW with their permission. Drawn procedurally like the other
//! patterns (the gear shader `gear.wesl` in `ocw-render`, this module its
//! Rust twin): no texture, nothing to download, the same on phones.
//!
//! - *Jersey:* black with a grid of big rounded-square panels outlined in
//!   white (the logo's blocks blown up) round the torso – the mid-chest
//!   and side panels filled white, the lower back's right panel white –,
//!   the wordmark "newk.it" across the chest in black with a white outline
//!   (no gap before ".it", its dot on the zip), a white collar, the right
//!   sleeve white and the left one black (each with a line near the cuff);
//!   on the back a huge pink "newk" running down the left half and ".it"
//!   at the bottom right; the rider's own name in pink on the left chest
//!   and the left sleeve.
//! - *Bib shorts:* black, a big pink "newk.it" up the outside of the left
//!   thigh, the logo's three squares on the right thigh, the rider's name
//!   in pink across the back.
//! - *Colours:* black, white and pink (Pantone 214 C ≈ #CE0F69), the
//!   design's first, second and third colours ([`dress`]); socks white.
//!
//! The lettering is our own: the wordmark is drawn from stroke outlines
//! (a tall condensed rounded sans in the spirit of the brand's font, not
//! the font itself – its licence is personal use only), the names from a
//! 4 × 7 pixel capital font. Coordinates: the rider's rest pose
//! ([`super`]): metres, pelvis at the origin, +Y up, facing −Z, +X its
//! right. Left and right are the rider's, so a front view shows the
//! rider's right on the picture's left, as on the design sheet.

use super::{BodyRegion, Garment, KitDesign};
use crate::bike::{KitPattern, Rgb, RiderLook};

/// The kit's black (the jersey's first colour, the shorts).
pub const BLACK: Rgb = Rgb(0x111114);
/// The white of the panels, the outline and the collar (second colour).
pub const WHITE: Rgb = Rgb(0xF5F5F0);
/// Pantone 214 C (third colour): the back's and the shorts' lettering and
/// the names.
pub const PINK: Rgb = Rgb(0xCE0F69);
/// The socks.
pub const SOCKS: Rgb = Rgb(0xF2F2EE);

/// `l` in the newk.it kit: the pattern and its colours (black jersey,
/// white, pink; black shorts and gloves, white socks); the rest of the
/// look (frame, helmet, number, finishes, character) stays.
pub fn dress(l: RiderLook) -> RiderLook {
    RiderLook {
        jersey: BLACK,
        pattern: KitPattern::Newkit,
        design: KitDesign {
            second: Some(WHITE),
            third: Some(PINK),
            shorts: Some(BLACK),
            socks: Some(SOCKS),
            gloves: Some(BLACK),
            ..l.design
        },
        ..l
    }
}

/// A look whose pattern was just changed from `before`: switching to the
/// newk.it kit dresses it in the kit's colours ([`dress`]); explicit
/// colours applied after this win.
pub fn on_pattern_change(before: KitPattern, l: RiderLook) -> RiderLook {
    if l.pattern == KitPattern::Newkit && before != KitPattern::Newkit {
        dress(l)
    } else {
        l
    }
}

/// Where everything sits (rest pose, m; glyph units are the wordmark's
/// x-height). The same numbers are in `gear.wesl`.
pub mod layout {
    /// The wordmark's glyphs n, e, w, k, ".", i, t: their left edges in
    /// x-heights (no gap before ".it") and the right end.
    pub const STARTS: [f32; 7] = [0.0, 0.65, 1.30, 2.31, 2.96, 3.31, 3.60];
    pub const END: f32 = 4.04;
    /// The dot's centre (on the zip on the chest).
    pub const DOT_CENTRE: f32 = 3.09;
    /// The torso's section (an ellipse about x = 0, z = AXIS_Z with the
    /// semi-axes SEMI) and the metres per radian round it.
    pub const AXIS_Z: f32 = 0.035;
    pub const SEMI: [f32; 2] = [0.175, 0.11];
    pub const ROUND: f32 = 0.14;
    /// The panels' columns round the torso: the zip (0), the front's
    /// edges (±COLS[0]), the sides' edges (±COLS[1]), the spine.
    pub const COLS: [f32; 2] = [0.19, 0.31];
    /// The panels' rows (y): from the hem up to over the shoulders.
    pub const ROWS: [f32; 4] = [0.01, 0.13, 0.36, 0.62];
    /// Half the black gap between panels, the white outline's width and
    /// the corners' radius.
    pub const CELL_GAP: f32 = 0.007;
    pub const CELL_STROKE: f32 = 0.011;
    pub const CELL_RADIUS: f32 = 0.035;
    /// The white collar from this height up, within this |x| (the neck,
    /// not the shoulders).
    pub const COLLAR: f32 = 0.50;
    pub const COLLAR_X: f32 = 0.1;
    /// The chest's wordmark: x-height, baseline, white outline.
    pub const CHEST_XH: f32 = 0.059;
    pub const CHEST_BASE: f32 = 0.322;
    pub const OUTLINE: f32 = 0.006;
    /// The back's pink "newk" (reading downwards, the letters' tops to
    /// the rider's right): x-height, top, baseline (round the back from
    /// the spine, + to the right).
    pub const BACK_XH: f32 = 0.13;
    pub const BACK_TOP: f32 = 0.47;
    pub const BACK_BASE: f32 = -0.1;
    /// The back's ".it" at the bottom right.
    pub const BACK_IT_XH: f32 = 0.075;
    pub const BACK_IT_TOP: f32 = 0.11;
    pub const BACK_IT_BASE: f32 = 0.09;
    /// The sleeves: the line near the cuff (centre, half width), the cuff.
    pub const SLEEVE_LINE: f32 = 0.235;
    pub const SLEEVE_LINE_HALF: f32 = 0.004;
    pub const CUFF: f32 = 0.212;
    /// The sleeve's axis (|x|, z) and radius.
    pub const SLEEVE_AXIS: [f32; 2] = [0.18, 0.03];
    pub const SLEEVE_R: f32 = 0.046;
    /// The thighs' axis (|x|, z) and radius.
    pub const THIGH_AXIS: [f32; 2] = [0.09, 0.0];
    pub const THIGH_R: f32 = 0.07;
    /// The left thigh's "newk.it" (reading upwards, the tops to the
    /// front): x-height, start (bottom), baseline (round the thigh from
    /// its outside, + to the back).
    pub const LEG_XH: f32 = 0.065;
    pub const LEG_BOTTOM: f32 = -0.285;
    pub const LEG_BASE: f32 = 0.045;
    /// The right thigh's three squares: centres (y, bottom one filled),
    /// half size, corner radius, outline.
    pub const SQUARES: [f32; 3] = [-0.265, -0.205, -0.145];
    pub const SQUARE_HALF: f32 = 0.024;
    pub const SQUARE_RADIUS: f32 = 0.008;
    pub const SQUARE_STROKE: f32 = 0.007;
    /// The names: centre (along, y), the largest letter height, the
    /// widest line. The chest's centre is round the torso (left chest).
    pub const CHEST_NAME: [f32; 4] = [-0.085, 0.44, 0.022, 0.11];
    pub const SLEEVE_NAME: [f32; 4] = [0.0, 0.27, 0.02, 0.10];
    pub const SHORTS_NAME: [f32; 4] = [0.0, -0.092, 0.02, 0.2];
    /// The shorts' back from this z.
    pub const SHORTS_BACK_Z: f32 = 0.04;
}

use layout::*;

/// Round the torso from the zip (m, + to the rider's right; ±π·ROUND at
/// the spine).
pub fn around(x: f32, z: f32) -> f32 {
    (x / SEMI[0]).atan2(-(z - AXIS_Z) / SEMI[1]) * ROUND
}

/// Round the torso from the spine (m, + to the rider's right).
pub fn behind(x: f32, z: f32) -> f32 {
    (x / SEMI[0]).atan2((z - AXIS_Z) / SEMI[1]) * ROUND
}

fn sd_box(px: f32, py: f32, x0: f32, y0: f32, x1: f32, y1: f32) -> f32 {
    let dx = (x0 - px).max(px - x1);
    let dy = (y0 - py).max(py - y1);
    let (ox, oy) = (dx.max(0.0), dy.max(0.0));
    (ox * ox + oy * oy).sqrt() + dx.max(dy).min(0.0)
}

fn sd_circle(px: f32, py: f32, cx: f32, cy: f32, r: f32) -> f32 {
    ((px - cx).powi(2) + (py - cy).powi(2)).sqrt() - r
}

fn sd_capsule(px: f32, py: f32, a: [f32; 2], b: [f32; 2], h: f32) -> f32 {
    let (bax, bay) = (b[0] - a[0], b[1] - a[1]);
    let (pax, pay) = (px - a[0], py - a[1]);
    let t = ((pax * bax + pay * bay) / (bax * bax + bay * bay)).clamp(0.0, 1.0);
    ((pax - bax * t).powi(2) + (pay - bay * t).powi(2)).sqrt() - h
}

/// A ring of radius `r` and half width `h` about (cx, cy): its top half
/// (`part` 0), bottom half (1) or bottom-left quarter (2); flat ends.
fn sd_arc(px: f32, py: f32, cx: f32, cy: f32, r: f32, h: f32, part: u32) -> f32 {
    let (qx, qy) = (px - cx, py - cy);
    let inside = match part {
        0 => qy >= 0.0,
        1 => qy <= 0.0,
        _ => qx <= 0.0 && qy <= 0.0,
    };
    if inside {
        return ((qx * qx + qy * qy).sqrt() - r).abs() - h;
    }
    if part == 2 {
        let a = sd_box(px, py, cx - r - h, cy, cx - r + h, cy);
        let b = sd_box(px, py, cx, cy - r - h, cx, cy - r + h);
        return a.min(b);
    }
    let a = sd_box(px, py, cx - r - h, cy, cx - r + h, cy);
    let b = sd_box(px, py, cx + r - h, cy, cx + r + h, cy);
    a.min(b)
}

/// The stroke's half width (x-heights).
const H: f32 = 0.1;

/// Signed distance (x-heights) to glyph `g` (0 n, 1 e, 2 w, 3 k, 4 ".",
/// 5 i, 6 t) at (x, y): x from its left edge, y from the baseline.
pub fn glyph_sd(g: u32, x: f32, y: f32) -> f32 {
    match g {
        0 => sd_box(x, y, 0.0, 0.0, 0.2, 0.72)
            .min(sd_box(x, y, 0.36, 0.0, 0.56, 0.72))
            .min(sd_arc(x, y, 0.28, 0.72, 0.18, H, 0)),
        1 => {
            let d = sd_arc(x, y, 0.28, 0.72, 0.18, H, 0)
                .min(sd_arc(x, y, 0.28, 0.28, 0.18, H, 1))
                .min(sd_box(x, y, 0.0, 0.28, 0.2, 0.72))
                .min(sd_box(x, y, 0.36, 0.28, 0.56, 0.72))
                .min(sd_box(x, y, 0.0, 0.44, 0.56, 0.56));
            // The open lower right.
            d.max(-sd_box(x, y, 0.3, 0.14, 0.7, 0.44))
        }
        2 => sd_box(x, y, 0.0, 0.28, 0.2, 1.0)
            .min(sd_box(x, y, 0.36, 0.28, 0.56, 1.0))
            .min(sd_box(x, y, 0.72, 0.28, 0.92, 1.0))
            .min(sd_arc(x, y, 0.28, 0.28, 0.18, H, 1))
            .min(sd_arc(x, y, 0.64, 0.28, 0.18, H, 1)),
        3 => {
            let diag = sd_capsule(x, y, [0.15, 0.38], [0.48, 0.92], H)
                .min(sd_capsule(x, y, [0.26, 0.5], [0.48, 0.08], H))
                .max(sd_box(x, y, 0.0, 0.0, 0.56, 1.0));
            sd_box(x, y, 0.0, 0.0, 0.2, 1.42).min(diag)
        }
        4 => sd_circle(x, y, 0.13, 0.13, 0.13),
        5 => sd_box(x, y, 0.0, 0.0, 0.2, 1.0).min(sd_circle(x, y, 0.1, 1.24, 0.11)),
        _ => sd_box(x, y, 0.04, 0.26, 0.24, 1.3)
            .min(sd_arc(x, y, 0.3, 0.26, 0.16, H, 2))
            .min(sd_box(x, y, 0.3, 0.0, 0.44, 0.2))
            .min(sd_box(x, y, 0.0, 0.82, 0.44, 1.0)),
    }
}

/// Signed distance (x-heights) to the glyphs `first..=last` of "newk.it"
/// set at [`layout::STARTS`] (x from the "n"'s left edge).
pub fn word_sd(x: f32, y: f32, first: u32, last: u32) -> f32 {
    if y < -0.5 || y > 1.9 || x < STARTS[first as usize] - 0.5 || x > END + 0.5 {
        return 1.0;
    }
    let mut d = 1.0f32;
    for g in first..=last {
        d = d.min(glyph_sd(g, x - STARTS[g as usize], y));
    }
    d
}

/// Signed distance to a rounded panel centred at (cu, cv) with the half
/// sizes (hu, hv) (the gap taken off).
fn panel_sd(u: f32, y: f32, cu: f32, cv: f32, hu: f32, hv: f32) -> f32 {
    let qx = (u - cu).abs() - (hu - CELL_GAP) + CELL_RADIUS;
    let qy = (y - cv).abs() - (hv - CELL_GAP) + CELL_RADIUS;
    (qx.max(0.0).powi(2) + qy.max(0.0).powi(2)).sqrt() + qx.max(qy).min(0.0) - CELL_RADIUS
}

/// The panel under round-the-torso `u` and height `y`: whether it is
/// filled white, and the signed distance (m) to its rounded outline. The
/// back above the lowest row is one big black panel across the spine; the
/// lowest row is split at the spine (its right half white).
pub fn panel(u: f32, y: f32) -> (bool, f32) {
    let pi_r = std::f32::consts::PI * ROUND;
    if u.abs() > COLS[1] && y >= ROWS[1] {
        let s = if u > 0.0 { pi_r - u } else { -pi_r - u };
        let w = pi_r - COLS[1];
        let sd = panel_sd(
            s,
            y,
            0.0,
            0.5 * (ROWS[1] + ROWS[3]),
            w,
            0.5 * (ROWS[3] - ROWS[1]),
        );
        return (false, sd);
    }
    let cols = [-pi_r, -COLS[1], -COLS[0], 0.0, COLS[0], COLS[1], pi_r];
    let mut k = 5;
    for i in 0..6 {
        if u < cols[i + 1] {
            k = i;
            break;
        }
    }
    let j = if y < ROWS[1] {
        0
    } else if y < ROWS[2] {
        1
    } else {
        2
    };
    let (u0, u1, y0, y1) = (cols[k], cols[k + 1], ROWS[j], ROWS[j + 1]);
    let sd = panel_sd(
        u,
        y,
        0.5 * (u0 + u1),
        0.5 * (y0 + y1),
        0.5 * (u1 - u0),
        0.5 * (y1 - y0),
    );
    let white = (j == 1 && (1..=4).contains(&k)) || (j == 0 && k == 5);
    (white, sd)
}

fn one(i: usize) -> [f32; 4] {
    let mut w = [0.0; 4];
    w[i] = 1.0;
    w
}

/// Weights `[base, first, second, third]` of the jersey at `p` (hard
/// edges; the shader anti-aliases them). Black is the base, white the
/// second colour, pink the third (the second without a third colour).
pub fn jersey(three: bool, region: BodyRegion, p: [f32; 3]) -> [f32; 4] {
    let [x, y, z] = p;
    let pink = if three { 3 } else { 2 };
    match region {
        BodyRegion::Torso => {
            if y >= COLLAR && x.abs() < COLLAR_X {
                return one(2);
            }
            let u = around(x, z);
            let (white, sd) = panel(u, y);
            let mut w = if sd < -CELL_STROKE {
                one(if white { 2 } else { 0 })
            } else if sd < 0.0 {
                one(2)
            } else {
                one(0)
            };
            if z < AXIS_Z {
                let d = word_sd(DOT_CENTRE - u / CHEST_XH, (y - CHEST_BASE) / CHEST_XH, 0, 6)
                    * CHEST_XH;
                if d < 0.0 {
                    w = one(0);
                } else if d < OUTLINE {
                    w = one(2);
                }
            } else {
                let s = behind(x, z);
                let newk = word_sd((BACK_TOP - y) / BACK_XH, (s - BACK_BASE) / BACK_XH, 0, 3);
                let it = word_sd(
                    STARTS[4] + (BACK_IT_TOP - y) / BACK_IT_XH,
                    (s - BACK_IT_BASE) / BACK_IT_XH,
                    4,
                    6,
                );
                if newk < 0.0 || it < 0.0 {
                    w = one(pink);
                }
            }
            w
        }
        BodyRegion::Arm => {
            let line = (y - SLEEVE_LINE).abs() < SLEEVE_LINE_HALF;
            if y < CUFF {
                one(0)
            } else if x > 0.0 {
                // The right sleeve: white with a black line.
                one(if line { 0 } else { 2 })
            } else {
                one(if line { 2 } else { 0 })
            }
        }
        _ => one(0),
    }
}

/// Round the thigh of the leg on `x`'s side from its outside (m): + to
/// the back on the left leg, + to the front on the right one (to the
/// right of someone looking at that side).
pub fn round_thigh(x: f32, z: f32) -> f32 {
    let dx = x.abs() - THIGH_AXIS[0];
    let dz = z - THIGH_AXIS[1];
    if x < 0.0 {
        dz.atan2(dx) * THIGH_R
    } else {
        (-dz).atan2(dx) * THIGH_R
    }
}

/// Signed distance (m) to the right thigh's square `i` at (s, y).
pub fn square_sd(i: usize, s: f32, y: f32) -> f32 {
    let qx = s.abs() - SQUARE_HALF + SQUARE_RADIUS;
    let qy = (y - SQUARES[i]).abs() - SQUARE_HALF + SQUARE_RADIUS;
    (qx.max(0.0).powi(2) + qy.max(0.0).powi(2)).sqrt() + qx.max(qy).min(0.0) - SQUARE_RADIUS
}

/// Weights of the bib shorts at `p` (base: the shorts' black).
pub fn shorts(three: bool, region: BodyRegion, p: [f32; 3]) -> [f32; 4] {
    let [x, y, z] = p;
    if region != BodyRegion::Leg {
        return one(0);
    }
    let s = round_thigh(x, z);
    if x < 0.0 {
        let d = word_sd((y - LEG_BOTTOM) / LEG_XH, (LEG_BASE - s) / LEG_XH, 0, 6);
        return if d < 0.0 {
            one(if three { 3 } else { 2 })
        } else {
            one(0)
        };
    }
    for i in 0..3 {
        let d = square_sd(i, s, y);
        if d < 0.0 && (i == 0 || d > -SQUARE_STROKE) {
            return one(2);
        }
    }
    one(0)
}

/// The longest name drawn (characters; longer ones are cut).
pub const NAME_MAX: usize = 12;

/// The characters of the name font, in the order of [`NAME_FONT`].
pub const NAME_CHARS: &str = " ABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789-.'";

/// The name font: condensed capitals of 4 × 7 pixels (own work), seven
/// rows per character of [`NAME_CHARS`].
pub const NAME_FONT: [&str; 280] = [
    "....", "....", "....", "....", "....", "....", "....", // space
    ".##.", "#..#", "#..#", "####", "#..#", "#..#", "#..#", // A
    "###.", "#..#", "#..#", "###.", "#..#", "#..#", "###.", // B
    ".###", "#...", "#...", "#...", "#...", "#...", ".###", // C
    "###.", "#..#", "#..#", "#..#", "#..#", "#..#", "###.", // D
    "####", "#...", "#...", "###.", "#...", "#...", "####", // E
    "####", "#...", "#...", "###.", "#...", "#...", "#...", // F
    ".###", "#...", "#...", "#.##", "#..#", "#..#", ".###", // G
    "#..#", "#..#", "#..#", "####", "#..#", "#..#", "#..#", // H
    "###.", ".#..", ".#..", ".#..", ".#..", ".#..", "###.", // I
    "..##", "...#", "...#", "...#", "...#", "#..#", ".##.", // J
    "#..#", "#.#.", "##..", "##..", "#.#.", "#..#", "#..#", // K
    "#...", "#...", "#...", "#...", "#...", "#...", "####", // L
    "#..#", "####", "####", "#..#", "#..#", "#..#", "#..#", // M
    "#..#", "##.#", "##.#", "#.##", "#.##", "#..#", "#..#", // N
    ".##.", "#..#", "#..#", "#..#", "#..#", "#..#", ".##.", // O
    "###.", "#..#", "#..#", "###.", "#...", "#...", "#...", // P
    ".##.", "#..#", "#..#", "#..#", "#.#.", "#..#", ".#.#", // Q
    "###.", "#..#", "#..#", "###.", "#.#.", "#..#", "#..#", // R
    ".###", "#...", "#...", ".##.", "...#", "...#", "###.", // S
    "###.", ".#..", ".#..", ".#..", ".#..", ".#..", ".#..", // T
    "#..#", "#..#", "#..#", "#..#", "#..#", "#..#", ".##.", // U
    "#..#", "#..#", "#..#", "#..#", "#..#", ".##.", ".##.", // V
    "#..#", "#..#", "#..#", "#..#", "####", "####", "#..#", // W
    "#..#", "#..#", ".##.", ".##.", ".##.", "#..#", "#..#", // X
    "#.#.", "#.#.", "#.#.", ".#..", ".#..", ".#..", ".#..", // Y
    "####", "...#", "..#.", ".#..", "#...", "#...", "####", // Z
    ".##.", "#..#", "#.##", "#..#", "##.#", "#..#", ".##.", // 0
    ".#..", "##..", ".#..", ".#..", ".#..", ".#..", "###.", // 1
    ".##.", "#..#", "...#", "..#.", ".#..", "#...", "####", // 2
    "###.", "...#", "...#", ".##.", "...#", "...#", "###.", // 3
    "#..#", "#..#", "#..#", "####", "...#", "...#", "...#", // 4
    "####", "#...", "###.", "...#", "...#", "#..#", ".##.", // 5
    ".##.", "#...", "#...", "###.", "#..#", "#..#", ".##.", // 6
    "####", "...#", "..#.", ".#..", ".#..", ".#..", ".#..", // 7
    ".##.", "#..#", "#..#", ".##.", "#..#", "#..#", ".##.", // 8
    ".##.", "#..#", "#..#", ".###", "...#", "...#", ".##.", // 9
    "....", "....", "....", "###.", "....", "....", "....", // -
    "....", "....", "....", "....", "....", "....", ".#..", // .
    ".#..", ".#..", "....", "....", "....", "....", "....", // '
];

/// The name font as bits: `row * 4 + col` (row 0 at the top). The same
/// table is in `gear.wesl`.
pub const NAME_GLYPHS: [u32; 40] = name_glyphs();

const fn name_glyphs() -> [u32; 40] {
    let mut out = [0u32; 40];
    let mut g = 0;
    while g < 40 {
        let mut bits = 0u32;
        let mut r = 0;
        while r < 7 {
            let row = NAME_FONT[g * 7 + r].as_bytes();
            let mut c = 0;
            while c < 4 {
                if row[c] == b'#' {
                    bits |= 1 << (r * 4 + c);
                }
                c += 1;
            }
            r += 1;
        }
        out[g] = bits;
        g += 1;
    }
    out
}

/// The base letter of an accented Latin letter (upper case), else `c`.
fn base_letter(c: char) -> char {
    match c {
        'À' | 'Á' | 'Â' | 'Ã' | 'Ä' | 'Å' | 'Ā' | 'Ą' => 'A',
        'Ç' | 'Ć' | 'Č' => 'C',
        'Ď' | 'Đ' => 'D',
        'È' | 'É' | 'Ê' | 'Ë' | 'Ē' | 'Ę' | 'Ě' => 'E',
        'Ğ' => 'G',
        'Ì' | 'Í' | 'Î' | 'Ï' | 'İ' => 'I',
        'Ł' => 'L',
        'Ñ' | 'Ń' | 'Ň' => 'N',
        'Ò' | 'Ó' | 'Ô' | 'Õ' | 'Ö' | 'Ø' | 'Ő' => 'O',
        'Ř' => 'R',
        'Ś' | 'Š' | 'Ş' | 'ß' => 'S',
        'Ť' => 'T',
        'Ù' | 'Ú' | 'Û' | 'Ü' | 'Ů' | 'Ű' => 'U',
        'Ý' | 'Ÿ' => 'Y',
        'Ź' | 'Ż' | 'Ž' => 'Z',
        '’' | '‘' | '`' => '\'',
        '–' | '—' | '_' => '-',
        _ => c,
    }
}

/// The name as the kit prints it: upper case, accents to their base
/// letters, other characters the font lacks dropped, runs of spaces as
/// one, at most [`NAME_MAX`] characters (cut, no ellipsis), trimmed.
pub fn name_text(name: &str) -> String {
    let mut out = String::new();
    for c in name.chars().flat_map(char::to_uppercase) {
        let c = if c.is_whitespace() {
            ' '
        } else {
            base_letter(c)
        };
        if !NAME_CHARS.contains(c) || (c == ' ' && (out.is_empty() || out.ends_with(' '))) {
            continue;
        }
        if out.chars().count() == NAME_MAX {
            break;
        }
        out.push(c);
    }
    out.trim_end_matches([' ', '-', '.']).to_string()
}

/// A name for the shader: the codes of [`name_text`] (indices into
/// [`NAME_CHARS`]), five 6-bit codes per word in x, y, z, the count in w.
pub fn pack_name(name: &str) -> [u32; 4] {
    let mut p = [0u32; 4];
    let mut n = 0;
    for (i, c) in name_text(name).chars().enumerate() {
        let code = NAME_CHARS.find(c).unwrap_or(0) as u32;
        p[i / 5] |= code << (6 * (i % 5));
        n += 1;
    }
    p[3] = n;
    p
}

/// The code of character `i` of a packed name.
pub fn name_code(p: &[u32; 4], i: u32) -> u32 {
    (p[(i / 5) as usize] >> (6 * (i % 5))) & 63
}

/// The letter height of a name of `n` characters in a spot at most
/// `h_max` high and `w_max` wide: as high as it may be, smaller when it
/// would be too wide (spaced capitals: 4 pixels and a gap of 2).
pub fn name_height(n: u32, h_max: f32, w_max: f32) -> f32 {
    if n == 0 {
        return 0.0;
    }
    let px = (h_max / 7.0).min(w_max / (6 * n - 2) as f32);
    px * 7.0
}

/// Whether the packed name has ink at (u, v) in its spot (centred at the
/// origin, u to the reader's right, v up).
pub fn name_ink(p: &[u32; 4], u: f32, v: f32, h_max: f32, w_max: f32) -> bool {
    let n = p[3].min(NAME_MAX as u32);
    if n == 0 {
        return false;
    }
    let h = name_height(n, h_max, w_max);
    let px = h / 7.0;
    let width = (6 * n - 2) as f32 * px;
    let gx = (u + 0.5 * width) / px;
    let gy = (0.5 * h - v) / px;
    if gx < 0.0 || gy < 0.0 || gx >= width / px || gy >= 7.0 {
        return false;
    }
    let cell = (gx / 6.0).floor() as u32;
    let col = gx.floor() as u32 - cell * 6;
    if col >= 4 || cell >= n {
        return false;
    }
    let row = gy.floor() as u32;
    let g = NAME_GLYPHS[name_code(p, cell).min(39) as usize];
    g & (1 << (row * 4 + col)) != 0
}

/// Where a name sits at `p` of a garment: the point in the spot's
/// coordinates (u to the reader's right, v up) and the spot's largest
/// letter height and width – the left chest, the left sleeve, the
/// shorts' back; `None` elsewhere.
pub fn name_spot(garment: Garment, region: BodyRegion, p: [f32; 3]) -> Option<[f32; 4]> {
    let [x, y, z] = p;
    match (garment, region) {
        (Garment::Jersey, BodyRegion::Torso)
            if z < AXIS_Z && (y < COLLAR || x.abs() >= COLLAR_X) =>
        {
            // Read from the front: to the reader's right is the rider's left.
            let c = CHEST_NAME;
            Some([c[0] - around(x, z), y - c[1], c[2], c[3]])
        }
        (Garment::Jersey, BodyRegion::Arm) if x < 0.0 => {
            // Read from the rider's left: to the reader's right is the back.
            let dx = -(x + SLEEVE_AXIS[0]);
            let dz = z - SLEEVE_AXIS[1];
            if dx <= 0.0 {
                return None;
            }
            let c = SLEEVE_NAME;
            Some([dz.atan2(dx) * SLEEVE_R - c[0], y - c[1], c[2], c[3]])
        }
        (Garment::Shorts, BodyRegion::Torso) if z > SHORTS_BACK_Z => {
            let c = SHORTS_NAME;
            Some([x - c[0], y - c[1], c[2], c[3]])
        }
        _ => None,
    }
}

/// Whether the name `p` (packed) is printed at `p` of the garment.
pub fn name_at(name: &[u32; 4], garment: Garment, region: BodyRegion, at: [f32; 3]) -> bool {
    name_spot(garment, region, at).is_some_and(|[u, v, h, w]| name_ink(name, u, v, h, w))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::kit::{dominant_jersey_colour, pattern_weights, torso_shares};

    const W: [f32; 4] = [0.0, 0.0, 1.0, 0.0];
    const B: [f32; 4] = [1.0, 0.0, 0.0, 0.0];
    const P: [f32; 4] = [0.0, 0.0, 0.0, 1.0];

    /// The chest's wordmark inside the front panels (|u| below the front's
    /// edge), its top under the name, its feet on the white panels.
    const LAYOUT_FITS: () = {
        assert!(DOT_CENTRE * CHEST_XH + OUTLINE < COLS[0]);
        assert!((END - DOT_CENTRE) * CHEST_XH < COLS[0]);
        assert!(CHEST_BASE + 1.42 * CHEST_XH + OUTLINE < CHEST_NAME[1] - 0.011);
        assert!(CHEST_BASE + 0.1 * CHEST_XH < ROWS[2]);
    };

    #[test]
    fn the_layout_fits() {
        let () = LAYOUT_FITS;
    }

    /// A point on the torso's section at round-the-torso `u`, height y.
    fn torso_at(u: f32, y: f32) -> [f32; 3] {
        let a = u / ROUND;
        [SEMI[0] * a.sin(), y, AXIS_Z - SEMI[1] * a.cos()]
    }

    #[test]
    fn dressing_sets_the_kit_colours_and_keeps_the_rest() {
        let l = RiderLook {
            frame: Rgb(0x123456),
            design: KitDesign {
                number: 12,
                ..Default::default()
            },
            ..Default::default()
        };
        let d = dress(l);
        assert_eq!(d.pattern, KitPattern::Newkit);
        assert_eq!(d.kit_colours(), [BLACK, WHITE, PINK]);
        assert_eq!(d.design.shorts, Some(BLACK));
        assert_eq!(d.design.socks, Some(SOCKS));
        assert_eq!((d.frame, d.design.number), (l.frame, 12));
        assert_eq!(PINK, Rgb(0xCE0F69), "Pantone 214 C");
        // Only a change to the kit dresses: a recoloured newk.it kit
        // keeps its colours.
        let mine = RiderLook {
            jersey: Rgb(0x1E88E5),
            ..d
        };
        assert_eq!(on_pattern_change(KitPattern::Newkit, mine), mine);
        assert_eq!(on_pattern_change(KitPattern::Band, mine).jersey, BLACK);
        let band = RiderLook {
            pattern: KitPattern::Band,
            ..l
        };
        assert_eq!(on_pattern_change(KitPattern::Plain, band), band);
    }

    #[test]
    fn the_wordmark_has_its_letters_and_no_gap_before_the_dot() {
        // Each glyph has ink inside its box and none far outside.
        for g in 0..7u32 {
            let x0 = STARTS[g as usize];
            let w = if g == 6 {
                END
            } else {
                STARTS[g as usize + 1] - 0.09
            } - x0;
            let mut ink = 0;
            for i in 0..40 {
                for j in 0..60 {
                    let (x, y) = (w * (i as f32 + 0.5) / 40.0, 1.5 * (j as f32 + 0.5) / 60.0);
                    if glyph_sd(g, x, y) < 0.0 {
                        ink += 1;
                    }
                }
            }
            assert!(ink > 120, "glyph {g}: {ink}");
            assert!(
                glyph_sd(g, w + 0.15, 0.5) > 0.0,
                "glyph {g} stays in its box"
            );
        }
        // n: two stems and an arch, open at the bottom middle.
        assert!(glyph_sd(0, 0.1, 0.3) < 0.0 && glyph_sd(0, 0.46, 0.3) < 0.0);
        assert!(glyph_sd(0, 0.28, 0.95) < 0.0, "the arch");
        assert!(glyph_sd(0, 0.28, 0.3) > 0.0, "open below the arch");
        // e: the bar, the open lower right.
        assert!(glyph_sd(1, 0.28, 0.5) < 0.0);
        assert!(glyph_sd(1, 0.5, 0.3) > 0.0);
        // k and t reach above the x-height, the i has its dot.
        assert!(glyph_sd(3, 0.1, 1.3) < 0.0 && glyph_sd(6, 0.14, 1.2) < 0.0);
        assert!(glyph_sd(5, 0.1, 1.24) < 0.0 && glyph_sd(5, 0.1, 1.07) > 0.0);
        // "newk.it" without a gap: the dot follows the k as closely as the
        // letters follow each other, and sits centred at DOT_CENTRE.
        let starts = std::hint::black_box(STARTS);
        let gap = |a: usize| starts[a + 1] - starts[a];
        assert!((starts[4] - (starts[3] + 0.56) - 0.09).abs() < 1e-4);
        assert!(gap(4) < 0.4);
        assert!((std::hint::black_box(DOT_CENTRE) - starts[4] - 0.13).abs() < 1e-5);
        assert!(word_sd(DOT_CENTRE, 0.13, 0, 6) < -0.1);
    }

    /// The chest: the dot on the zip, "newk" on the rider's right (the
    /// picture's left in a front view), all of it inside the front panels
    /// (no seam or side panel cuts it), black with a white outline.
    #[test]
    fn the_chest_wordmark_sits_on_the_zip_and_inside_the_front() {
        let at = |u: f32, y: f32| jersey(true, BodyRegion::Torso, torso_at(u, y));
        let y_dot = CHEST_BASE + 0.13 * CHEST_XH;
        assert_eq!(at(0.0, y_dot), B, "the dot on the zip");
        // The n's left stem on the rider's right.
        let u_n = (DOT_CENTRE - 0.1) * CHEST_XH;
        assert_eq!(at(u_n, CHEST_BASE + 0.2 * CHEST_XH), B);
        // Within the front: |x| < 0.15 (the side panels' mesh), |u| < the
        // front's edge.
        // (The layout's own limits: `LAYOUT_FITS` below.)
        // The letters' feet on the white panel: their black shows, with
        // the outline in white.
        let mid = CHEST_BASE + 0.1 * CHEST_XH;
        assert_eq!(at(u_n, mid), B);
        // Just left of the k's ascender, on the black upper panel.
        let beside = (DOT_CENTRE - STARTS[3] + 0.5 * OUTLINE / CHEST_XH) * CHEST_XH;
        assert_eq!(at(beside, CHEST_BASE + 1.2 * CHEST_XH), W, "white outline");
        assert_eq!(
            at(beside - OUTLINE, CHEST_BASE + 1.2 * CHEST_XH),
            B,
            "the k"
        );
    }

    #[test]
    fn the_panels_follow_the_design() {
        let at = |u: f32, y: f32| jersey(true, BodyRegion::Torso, torso_at(u, y));
        // Mid-chest white on both sides of the zip, the lower torso and
        // the upper chest black, with white outlines.
        assert_eq!(at(0.09, 0.2), W);
        assert_eq!(at(-0.09, 0.2), W);
        assert_eq!(at(0.09, 0.07), B);
        assert_eq!(at(-0.1, 0.44), B);
        assert_eq!(
            at(0.09, ROWS[1] - CELL_GAP - 0.5 * CELL_STROKE),
            W,
            "outline"
        );
        assert_eq!(at(0.09, ROWS[1]), B, "the gap between panels");
        // The lower back: the rider's right panel white, the left black.
        let pi_r = std::f32::consts::PI * ROUND;
        assert_eq!(at(pi_r - 0.06, 0.115), W);
        assert_eq!(at(-(pi_r - 0.06), 0.05), B);
        // Collar white; the back above the lowest row one black panel.
        assert_eq!(at(0.0, 0.51), W);
        assert_eq!(at(pi_r - 0.06, 0.3), B);
        assert_eq!(at(-(pi_r - 0.005), 0.48), B, "no seam on the spine");
    }

    /// Left and right are the rider's: the white sleeve and the back's
    /// ".it" on the right, the black sleeve, the name and the back's
    /// "newk" on the left – not mirrored.
    #[test]
    fn the_design_is_not_mirrored() {
        let arm = |x: f32| jersey(true, BodyRegion::Arm, [x, 0.3, 0.03]);
        assert_eq!(arm(0.2), W, "right sleeve white");
        assert_eq!(arm(-0.2), B, "left sleeve black");
        assert_eq!(jersey(true, BodyRegion::Arm, [0.2, 0.2, 0.03]), B, "cuff");
        // The back: "newk" down the left half (its n's stem near the top),
        // ".it" at the bottom right; read from behind, the tops to the
        // right.
        // Mean position round the back of the pink above and below 0.12 m.
        let (mut newk, mut it) = ((0.0f32, 0), (0.0f32, 0));
        for i in 0..120 {
            for j in 0..100 {
                let s = -0.3 + 0.6 * (i as f32 + 0.5) / 120.0;
                let y = 0.5 * (j as f32 + 0.5) / 100.0;
                let b = s / ROUND;
                let p = [SEMI[0] * b.sin(), y, AXIS_Z + SEMI[1] * b.cos()];
                // Below the k's foot (BACK_TOP - 2.87 x-heights): ".it".
                if jersey(true, BodyRegion::Torso, p) == P && (y > 0.12 || y < 0.095) {
                    let m = if y > 0.12 { &mut newk } else { &mut it };
                    m.0 += s;
                    m.1 += 1;
                }
            }
        }
        assert!(newk.1 > 500 && it.1 > 60, "{newk:?} {it:?}");
        assert!(newk.0 / (newk.1 as f32) < -0.02, "newk on the left half");
        assert!(it.0 / (it.1 as f32) > 0.08, ".it at the bottom right");
        // The names: on the left chest and sleeve only.
        let name = pack_name("Ada Rider");
        let chest = |u: f32| {
            (0..40).any(|i| {
                let y = CHEST_NAME[1] - 0.012 + 0.0006 * i as f32;
                (0..80).any(|k| {
                    name_at(
                        &name,
                        Garment::Jersey,
                        BodyRegion::Torso,
                        torso_at(u - 0.06 + 0.0015 * k as f32, y),
                    )
                })
            })
        };
        assert!(chest(CHEST_NAME[0]));
        assert!(!chest(-CHEST_NAME[0]));
        assert!(name_spot(Garment::Jersey, BodyRegion::Arm, [0.2, 0.27, 0.03]).is_none());
        assert!(name_spot(Garment::Jersey, BodyRegion::Arm, [-0.226, 0.27, 0.03]).is_some());
    }

    #[test]
    fn the_shorts_carry_the_leg_lettering_and_the_squares() {
        // Left thigh outside: pink lettering; right thigh: three squares,
        // the lowest filled; the inside of the legs and the pelvis black.
        let mut pink = 0;
        for i in 0..40 {
            for j in 0..60 {
                let a = -0.9 + 1.8 * (i as f32 + 0.5) / 40.0;
                let y = -0.3 + 0.33 * (j as f32 + 0.5) / 60.0;
                let p = [
                    -THIGH_AXIS[0] - THIGH_R * a.cos(),
                    y,
                    THIGH_AXIS[1] + THIGH_R * a.sin(),
                ];
                if shorts(true, BodyRegion::Leg, p) == P {
                    pink += 1;
                }
            }
        }
        assert!(pink > 200, "{pink}");
        let right = |y: f32| shorts(true, BodyRegion::Leg, [THIGH_AXIS[0] + THIGH_R, y, 0.0]);
        assert_eq!(right(SQUARES[0]), W, "filled square");
        assert_eq!(right(SQUARES[1]), B, "outlined: black inside");
        assert_eq!(right(SQUARES[1] + SQUARE_HALF - 0.003), W, "its outline");
        assert_eq!(right(-0.05), B);
        assert_eq!(shorts(true, BodyRegion::Torso, [0.0, -0.05, -0.1]), B);
        assert!(name_spot(Garment::Shorts, BodyRegion::Torso, [0.0, -0.07, 0.09]).is_some());
        assert!(name_spot(Garment::Shorts, BodyRegion::Torso, [0.0, -0.07, -0.09]).is_none());
    }

    #[test]
    fn the_kit_is_deterministic_and_mostly_black() {
        let a = torso_shares(KitPattern::Newkit, true);
        assert_eq!(a, torso_shares(KitPattern::Newkit, true));
        assert!(a[0] > 0.3 && a[2] > 0.15 && a[3] > 0.04, "{a:?}");
        // The far model's jersey: the kit's black (its first colour).
        assert_eq!(dominant_jersey_colour(&dress(RiderLook::default())), BLACK);
        let blue = RiderLook {
            jersey: Rgb(0x1E88E5),
            ..dress(RiderLook::default())
        };
        assert_eq!(dominant_jersey_colour(&blue), Rgb(0x1E88E5));
        // pattern_weights is this module.
        let p = torso_at(0.09, 0.2);
        assert_eq!(
            pattern_weights(
                KitPattern::Newkit,
                true,
                Garment::Jersey,
                BodyRegion::Torso,
                p
            ),
            jersey(true, BodyRegion::Torso, p)
        );
    }

    #[test]
    fn names_are_cleaned_packed_and_fitted() {
        assert_eq!(name_text("Ada Rider"), "ADA RIDER");
        assert_eq!(name_text("  Zoë   Côté-Ñúñez "), "ZOE COTE-NUN");
        assert_eq!(name_text("Bea_Climber🚴"), "BEA-CLIMBER");
        assert_eq!(name_text("李小龙"), "", "nothing the font can draw");
        assert_eq!(name_text("O’Neill"), "O'NEILL");
        assert_eq!(
            name_text("Abcdefghijk mno"),
            "ABCDEFGHIJK",
            "cut, no trailing space"
        );
        assert!(name_text(&"W".repeat(40)).len() == NAME_MAX);
        let p = pack_name("Ada Rider 42");
        assert_eq!(p[3], 12);
        let text: String = (0..p[3])
            .map(|i| NAME_CHARS.as_bytes()[name_code(&p, i) as usize] as char)
            .collect();
        assert_eq!(text, "ADA RIDER 42");
        assert_eq!(pack_name(""), [0; 4]);
        // Short names at full height, long ones smaller but within the
        // spot's width; every glyph distinct.
        let [_, _, h, w] = CHEST_NAME;
        assert_eq!(name_height(3, h, w), h);
        let long = name_height(12, h, w);
        assert!(long < h && long > 0.4 * h, "{long}");
        assert!((6.0 * 12.0 - 2.0) * long / 7.0 <= w + 1e-6);
        for (i, a) in NAME_GLYPHS.iter().enumerate() {
            for b in &NAME_GLYPHS[i + 1..] {
                assert_ne!(a, b);
            }
        }
        // Ink: "I" has its stem in column 1; nothing beyond the name.
        let i = pack_name("I");
        let px = h / 7.0;
        let width = 4.0 * px;
        assert!(name_ink(&i, -0.5 * width + 1.5 * px, 0.0, h, w));
        assert!(!name_ink(&i, -0.5 * width + 3.5 * px, 0.0, h, w));
        assert!(!name_ink(&i, 0.05, 0.0, h, w));
        assert!(!name_ink(&pack_name(""), 0.0, 0.0, h, w));
    }

    /// Two names print differently at the same place.
    #[test]
    fn each_rider_shows_their_own_name() {
        let [_, _, h, w] = SHORTS_NAME;
        let ink = |n: &str| -> Vec<bool> {
            let p = pack_name(n);
            (0..2000)
                .map(|k| {
                    let u = -0.5 * w + w * ((k % 100) as f32 + 0.5) / 100.0;
                    let v = -0.5 * h + h * ((k / 100) as f32 + 0.5) / 20.0;
                    name_ink(&p, u, v, h, w)
                })
                .collect()
        };
        assert_ne!(ink("Ada Rider"), ink("Bea Climber"));
        assert_eq!(ink("Ada Rider"), ink("ada rider"));
    }
}
