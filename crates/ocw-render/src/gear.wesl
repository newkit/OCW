// Rider and bike materials (spec §14 Phase 8 T6): the standard PBR fragment
// shader with real materials on the generated models – carbon weave under a
// clear coat, spun / brushed metal with anisotropic highlights, rubber with
// a file tread, grips, saddle leather, jersey knit with mesh side panels,
// lycra, plastics – and the jersey designer's patterns, colours and race
// numbers (Rust twin: ocw_core::kit). Our own procedural work; with the
// material library's gear layers (jersey knit, carbon twill, saddle leather,
// CC0 from ambientCG) those three are textured (triplanar).
//
// Vertex data (ocw-render `gear`): uv = (x, y), uv_b.x = z of the vertex in
// its rest coordinates (the rider: the model's rest pose, the pelvis at the
// origin, metres; bike parts: their node, e.g. a wheel about its hub, the
// axle along x); uv_b.y = the body region (0 torso, 1 arm, 2 leg, 3 head,
// 4 hand, 5 foot). Detail fades out by the pixel's footprint: far riders
// use the plain far model (StandardMaterial), so this runs only up close.
//
// Bindless where the platform has it (the materials of all riders in one
// slab: their draws batch; a bind group per rider and part cost several ms
// with 30 riders), plain bindings otherwise (WebGL 2). The material comes
// per draw from the bindless index table, so nothing below may assume it
// is uniform: no derivative builtins inside branches – the detail's slopes
// are differences of its height one pixel over in rest coordinates.

import bevy_pbr::render::{
    forward_io::{VertexOutput, FragmentOutput},
    pbr_fragment::pbr_input_from_standard_material,
    pbr_functions::{apply_pbr_lighting, main_pass_post_lighting_processing, visibility_range_dither},
    mesh_bindings::mesh,
};
import bevy_render::bindless::{bindless_samplers_filtering, bindless_textures_2d_array};

struct GearLook {
    // x: kind (KIND_*); y: garment (0 none, 1 jersey, 2 shorts, 3 helmet);
    // z: pattern (ocw_core::KitPattern wire number); w: flags (1 three
    // colours, 2 textured, 4 mesh side panels).
    mode: vec4<f32>,
    // x: race number (0 none); y: detail strength; z: detail size (m);
    // w: anisotropy (brushed metal).
    params: vec4<f32>,
    // Linear colours: the garment's own (base), first, second, third.
    base: vec4<f32>,
    first: vec4<f32>,
    second: vec4<f32>,
    third: vec4<f32>,
    // Per texture layer (knit, carbon, leather): mean albedo, repeat (m).
    layers: array<vec4<f32>, 3>,
    // The rider's name on the newk.it kit (ocw_core::kit::newkit::pack_name:
    // five 6-bit codes in x, y, z; the count in w; zero elsewhere).
    name: vec4<u32>,
}

@if(BINDLESS) {
// The extension's bindless index table (ocw-render `gear`: bindings 50-53).
struct GearIndices {
    look: u32,
    albedo: u32,
    albedo_sampler: u32,
    normal: u32,
}
@group(constants::MATERIAL_BIND_GROUP) @binding(100) var<storage> gear_indices: array<GearIndices>;
@group(constants::MATERIAL_BIND_GROUP) @binding(101) var<storage> gear_looks: array<GearLook>;
}
@else {
@group(constants::MATERIAL_BIND_GROUP) @binding(50) var<uniform> gear_look: GearLook;
@group(constants::MATERIAL_BIND_GROUP) @binding(51) var albedo_array: texture_2d_array<f32>;
@group(constants::MATERIAL_BIND_GROUP) @binding(52) var array_sampler: sampler;
@group(constants::MATERIAL_BIND_GROUP) @binding(53) var normal_array: texture_2d_array<f32>;
}

fn sample_albedo(slot: u32, uv: vec2<f32>, layer: i32, gx: vec2<f32>, gy: vec2<f32>) -> vec4<f32> {
@if(BINDLESS)
    return textureSampleGrad(bindless_textures_2d_array[gear_indices[slot].albedo], bindless_samplers_filtering[gear_indices[slot].albedo_sampler], uv, layer, gx, gy);
@else
    return textureSampleGrad(albedo_array, array_sampler, uv, layer, gx, gy);
}

fn sample_normal(slot: u32, uv: vec2<f32>, layer: i32, gx: vec2<f32>, gy: vec2<f32>) -> vec2<f32> {
@if(BINDLESS)
    return textureSampleGrad(bindless_textures_2d_array[gear_indices[slot].normal], bindless_samplers_filtering[gear_indices[slot].albedo_sampler], uv, layer, gx, gy).xy * 2.0 - 1.0;
@else
    return textureSampleGrad(normal_array, array_sampler, uv, layer, gx, gy).xy * 2.0 - 1.0;
}

const KIND_PLAIN: u32 = 0u;
const KIND_PAINT: u32 = 1u;
const KIND_CARBON: u32 = 2u;
const KIND_METAL: u32 = 3u;
const KIND_TYRE: u32 = 4u;
const KIND_GRIP: u32 = 5u;
const KIND_LEATHER: u32 = 6u;
const KIND_KNIT: u32 = 7u;
const KIND_LYCRA: u32 = 8u;
const KIND_PLASTIC: u32 = 9u;
const KIND_PAD: u32 = 10u;

// Bare carbon's share of the ambient light (Rust: gear::CARBON_AMBIENT):
// the warm ambient's specular on the dark base made it read grey-brown.
const CARBON_AMBIENT: f32 = 0.3;
// Brushing (Rust twin: gear::brush): spun rings about the node's x axle,
// linear along a tube, or plain (flat faces).
const BRUSH_SPUN: u32 = 0u;
const BRUSH_LINEAR: u32 = 1u;
const BRUSH_PLAIN: u32 = 2u;
// Within this distance (m) of the node's x axis a tube along it is an
// axle part (hub barrel, bottom bracket): spun.
const AXLE_R: f32 = 0.03;

// ocw_core::kit::layout.
const BAND_MID: f32 = 0.29;
const BAND_HALF: f32 = 0.05;
const PIN: f32 = 0.014;
const SASH_ANGLE: f32 = 0.62;
const SASH_HALF: f32 = 0.042;
const STRIPE: f32 = 0.045;
const CHEVRON_FROM: f32 = 0.16;
const CHEVRON: f32 = 0.07;
const CHEVRON_SLOPE: f32 = 0.9;
const SIDE_PANEL: f32 = 0.138;
const HELMET_STRIPE: f32 = 0.02;
const BIB_CENTRE: vec2<f32> = vec2<f32>(0.0, 0.29);
const BIB_HALF: vec2<f32> = vec2<f32>(0.1, 0.075);
const BIB_DIGIT: f32 = 0.075;
const BIB_BACK_Z: f32 = 0.07;
const HIP_X: f32 = 0.174;
const HIP_CENTRE: vec2<f32> = vec2<f32>(0.0, -0.07);
const HIP_DIGIT: f32 = 0.036;

fn hash1(p: vec3<f32>) -> f32 {
    return fract(sin(dot(p, vec3<f32>(12.9898, 78.233, 37.719))) * 43758.5453);
}

// Smooth value noise in 3D (0-1).
fn noise3(p: vec3<f32>) -> f32 {
    let i = floor(p);
    let f = fract(p);
    let u = f * f * (3.0 - 2.0 * f);
    let a = mix(hash1(i), hash1(i + vec3<f32>(1.0, 0.0, 0.0)), u.x);
    let b = mix(hash1(i + vec3<f32>(0.0, 1.0, 0.0)), hash1(i + vec3<f32>(1.0, 1.0, 0.0)), u.x);
    let c = mix(hash1(i + vec3<f32>(0.0, 0.0, 1.0)), hash1(i + vec3<f32>(1.0, 0.0, 1.0)), u.x);
    let d = mix(hash1(i + vec3<f32>(0.0, 1.0, 1.0)), hash1(i + vec3<f32>(1.0, 1.0, 1.0)), u.x);
    return mix(mix(a, b, u.y), mix(c, d, u.y), u.z);
}

// Coverage of s > e, box-filtered over aa.
fn over(s: f32, e: f32, aa: f32) -> f32 {
    return clamp((s - e) / (2.0 * aa) + 0.5, 0.0, 1.0);
}

// Coverage of |d| < half.
fn within(d: f32, half: f32, aa: f32) -> f32 {
    return clamp((half - abs(d)) / (2.0 * aa) + 0.5, 0.0, 1.0);
}

// Weights (base, first, second, third) of one of the pattern's colours:
// 0 base, 1 second, 2 third.
fn pick(i: i32) -> vec4<f32> {
    if i == 1 {
        return vec4<f32>(0.0, 0.0, 1.0, 0.0);
    }
    if i == 2 {
        return vec4<f32>(0.0, 0.0, 0.0, 1.0);
    }
    return vec4<f32>(1.0, 0.0, 0.0, 0.0);
}

// The second colour inside |d| < half, the third along its edges.
fn banded(d: f32, half: f32, three: bool, aa: f32) -> vec4<f32> {
    let a = within(d, half, aa);
    var b = 0.0;
    if three {
        b = max(within(d, half + PIN, aa) - a, 0.0);
    }
    return vec4<f32>(1.0 - a - b, 0.0, a, b);
}

// Index of stripe k (stripes) or chevron k (chevrons).
fn step_index(k: i32, chevrons: bool, three: bool) -> i32 {
    if chevrons {
        if k == 0 || k == 2 {
            return 1;
        }
        if k == 1 && three {
            return 2;
        }
        return 0;
    }
    let n = select(2, 3, three);
    return ((k % n) + n) % n;
}

// Steps of width w along s, anti-aliased across each boundary.
fn steps(s: f32, w: f32, aa: f32, chevrons: bool, three: bool) -> vec4<f32> {
    let t = s / w;
    let k = i32(floor(t));
    let f = t - floor(t);
    let d = min(f, 1.0 - f) * w;
    let own = clamp(0.5 + d / (2.0 * aa), 0.0, 1.0);
    let other = select(k + 1, k - 1, f < 0.5);
    return pick(step_index(k, chevrons, three)) * own
        + pick(step_index(other, chevrons, three)) * (1.0 - own);
}

// First to second (through the third for three colours) over t.
fn blend(t0: f32, three: bool) -> vec4<f32> {
    let t = smoothstep(0.0, 1.0, clamp(t0, 0.0, 1.0));
    if three {
        if t < 0.5 {
            let u = t * 2.0;
            return vec4<f32>(0.0, 1.0 - u, 0.0, u);
        }
        let u = t * 2.0 - 1.0;
        return vec4<f32>(0.0, 0.0, u, 1.0 - u);
    }
    return vec4<f32>(0.0, 1.0 - t, t, 0.0);
}

fn jersey_weights(p: vec3<f32>, region: u32, pattern: u32, three: bool, aa: f32) -> vec4<f32> {
    if pattern == 6u {
        return newkit_jersey(p, region, three, aa);
    }
    if region != 0u && pattern != 5u {
        return vec4<f32>(1.0, 0.0, 0.0, 0.0);
    }
    switch pattern {
        case 1u: {
            return banded(p.y - BAND_MID, BAND_HALF, three, aa);
        }
        case 2u: {
            let d = -p.x * sin(SASH_ANGLE) + (p.y - 0.29) * cos(SASH_ANGLE);
            return banded(d, SASH_HALF, three, aa);
        }
        case 3u: {
            return steps(p.x + 0.5 * STRIPE, STRIPE, aa, false, three);
        }
        case 4u: {
            let v = p.y - CHEVRON_SLOPE * abs(p.x) - CHEVRON_FROM;
            return steps(v, CHEVRON, aa * 1.35, true, three);
        }
        case 5u: {
            return blend((p.y - 0.02) / 0.48, three);
        }
        default: {
            return vec4<f32>(1.0, 0.0, 0.0, 0.0);
        }
    }
}

fn shorts_weights(p: vec3<f32>, region: u32, pattern: u32, three: bool, aa: f32) -> vec4<f32> {
    if pattern == 6u {
        return newkit_shorts(p, region, three, aa);
    }
    if pattern == 0u || (region != 0u && region != 2u) {
        return vec4<f32>(1.0, 0.0, 0.0, 0.0);
    }
    let side = abs(p.x);
    let panel = over(side, SIDE_PANEL, aa);
    let pin = max(over(side, SIDE_PANEL - PIN * 0.5, aa) - panel, 0.0);
    if three {
        return vec4<f32>(1.0 - panel - pin, panel, 0.0, pin);
    }
    return vec4<f32>(1.0 - panel - pin, panel, pin, 0.0);
}

fn helmet_weights(p: vec3<f32>, pattern: u32, three: bool, aa: f32) -> vec4<f32> {
    if pattern == 0u {
        return vec4<f32>(1.0, 0.0, 0.0, 0.0);
    }
    if pattern == 5u {
        let w = blend((p.z + 0.12) / 0.26, three);
        return vec4<f32>(0.0, w.x + w.y, w.z, w.w);
    }
    let a = within(p.x, HELMET_STRIPE, aa);
    var b = 0.0;
    if three {
        b = max(within(p.x, HELMET_STRIPE + PIN * 0.6, aa) - a, 0.0);
    }
    return vec4<f32>(0.0, 1.0 - a - b, a, b);
}

fn glyph(d: u32) -> u32 {
    var g = array<u32, 10>(31599u, 29850u, 29671u, 31207u, 18925u, 31183u, 31695u, 9383u, 31727u, 31215u);
    return g[min(d, 9u)];
}

// Ink of the number n at (u, v) in a line of digits h high centred at the
// origin (ocw_core::kit::digits_ink).
fn digits_ink(n: u32, u: f32, v: f32, h: f32) -> f32 {
    let px = h / 5.0;
    var count = 1u;
    if n >= 100u {
        count = 3u;
    } else if n >= 10u {
        count = 2u;
    }
    let width = f32(4u * count - 1u) * px;
    let gx = (u + 0.5 * width) / px;
    let gy = (0.5 * h - v) / px;
    if gx < 0.0 || gy < 0.0 || gx >= width / px || gy >= 5.0 {
        return 0.0;
    }
    let cell = u32(floor(gx / 4.0));
    let col = u32(floor(gx)) - cell * 4u;
    if col >= 3u {
        return 0.0;
    }
    var div = 1u;
    for (var i = cell + 1u; i < count; i++) {
        div = div * 10u;
    }
    let digit = (n / div) % 10u;
    let row = u32(floor(gy));
    return f32((glyph(digit) >> (row * 3u + col)) & 1u);
}

// Box-filtered over the pixel (four taps).
fn digits_cover(n: u32, u: f32, v: f32, h: f32, aa: f32) -> f32 {
    let o = 0.5 * aa;
    return 0.25 * (digits_ink(n, u - o, v - o, h) + digits_ink(n, u + o, v - o, h)
        + digits_ink(n, u - o, v + o, h) + digits_ink(n, u + o, v + o, h));
}

// The newk.it kit (pattern 6; spec 14 Phase 8, 8E; the design and the name
// newk.it are the user's, used with permission; Rust twin:
// ocw_core::kit::newkit, the same layout constants).
const NK_END: f32 = 4.04;
const NK_DOT_CENTRE: f32 = 3.09;
const NK_AXIS_Z: f32 = 0.035;
const NK_SEMI: vec2<f32> = vec2<f32>(0.175, 0.11);
const NK_ROUND: f32 = 0.14;
const NK_COLS: vec2<f32> = vec2<f32>(0.19, 0.31);
const NK_ROWS: vec4<f32> = vec4<f32>(0.01, 0.13, 0.36, 0.62);
const NK_CELL_GAP: f32 = 0.007;
const NK_CELL_STROKE: f32 = 0.011;
const NK_CELL_RADIUS: f32 = 0.035;
const NK_COLLAR: f32 = 0.5;
const NK_COLLAR_X: f32 = 0.1;
const NK_CHEST_XH: f32 = 0.059;
const NK_CHEST_BASE: f32 = 0.322;
const NK_OUTLINE: f32 = 0.006;
const NK_BACK_XH: f32 = 0.13;
const NK_BACK_TOP: f32 = 0.47;
const NK_BACK_BASE: f32 = -0.1;
const NK_BACK_IT_XH: f32 = 0.075;
const NK_BACK_IT_TOP: f32 = 0.11;
const NK_BACK_IT_BASE: f32 = 0.09;
const NK_SLEEVE_LINE: f32 = 0.235;
const NK_SLEEVE_LINE_HALF: f32 = 0.004;
const NK_CUFF: f32 = 0.212;
const NK_SLEEVE_AXIS: vec2<f32> = vec2<f32>(0.18, 0.03);
const NK_SLEEVE_R: f32 = 0.046;
const NK_THIGH_AXIS: vec2<f32> = vec2<f32>(0.09, 0.0);
const NK_THIGH_R: f32 = 0.07;
const NK_LEG_XH: f32 = 0.065;
const NK_LEG_BOTTOM: f32 = -0.285;
const NK_LEG_BASE: f32 = 0.045;
const NK_SQUARES: vec3<f32> = vec3<f32>(-0.265, -0.205, -0.145);
const NK_SQUARE_HALF: f32 = 0.024;
const NK_SQUARE_RADIUS: f32 = 0.008;
const NK_SQUARE_STROKE: f32 = 0.007;
const NK_CHEST_NAME: vec4<f32> = vec4<f32>(-0.085, 0.44, 0.022, 0.11);
const NK_SLEEVE_NAME: vec4<f32> = vec4<f32>(0.0, 0.27, 0.02, 0.1);
const NK_SHORTS_NAME: vec4<f32> = vec4<f32>(0.0, -0.092, 0.02, 0.2);
const NK_SHORTS_BACK_Z: f32 = 0.04;
const NK_NAME_MAX: u32 = 12u;
const NK_H: f32 = 0.1;

fn nk_box(p: vec2<f32>, lo: vec2<f32>, hi: vec2<f32>) -> f32 {
    let d = max(lo - p, p - hi);
    return length(max(d, vec2<f32>(0.0))) + min(max(d.x, d.y), 0.0);
}

fn nk_circle(p: vec2<f32>, c: vec2<f32>, r: f32) -> f32 {
    return length(p - c) - r;
}

fn nk_capsule(p: vec2<f32>, a: vec2<f32>, b: vec2<f32>, h: f32) -> f32 {
    let pa = p - a;
    let ba = b - a;
    let t = clamp(dot(pa, ba) / dot(ba, ba), 0.0, 1.0);
    return length(pa - ba * t) - h;
}

// A ring of radius r, half width h about c: the top half (part 0), the
// bottom half (1) or the bottom-left quarter (2); flat ends.
fn nk_arc(p: vec2<f32>, c: vec2<f32>, r: f32, h: f32, part: u32) -> f32 {
    let q = p - c;
    var inside = q.y >= 0.0;
    if part == 1u {
        inside = q.y <= 0.0;
    } else if part == 2u {
        inside = q.x <= 0.0 && q.y <= 0.0;
    }
    if inside {
        return abs(length(q) - r) - h;
    }
    let a = nk_box(p, vec2<f32>(c.x - r - h, c.y), vec2<f32>(c.x - r + h, c.y));
    if part == 2u {
        return min(a, nk_box(p, vec2<f32>(c.x, c.y - r - h), vec2<f32>(c.x, c.y - r + h)));
    }
    return min(a, nk_box(p, vec2<f32>(c.x + r - h, c.y), vec2<f32>(c.x + r + h, c.y)));
}

// Signed distance (x-heights) to glyph g of "newk.it" (0 n, 1 e, 2 w, 3 k,
// 4 ".", 5 i, 6 t), p from its left edge and the baseline.
fn nk_glyph(g: u32, p: vec2<f32>) -> f32 {
    switch g {
        case 0u: {
            return min(min(nk_box(p, vec2<f32>(0.0, 0.0), vec2<f32>(0.2, 0.72)),
                nk_box(p, vec2<f32>(0.36, 0.0), vec2<f32>(0.56, 0.72))),
                nk_arc(p, vec2<f32>(0.28, 0.72), 0.18, NK_H, 0u));
        }
        case 1u: {
            var d = min(nk_arc(p, vec2<f32>(0.28, 0.72), 0.18, NK_H, 0u),
                nk_arc(p, vec2<f32>(0.28, 0.28), 0.18, NK_H, 1u));
            d = min(d, nk_box(p, vec2<f32>(0.0, 0.28), vec2<f32>(0.2, 0.72)));
            d = min(d, nk_box(p, vec2<f32>(0.36, 0.28), vec2<f32>(0.56, 0.72)));
            d = min(d, nk_box(p, vec2<f32>(0.0, 0.44), vec2<f32>(0.56, 0.56)));
            return max(d, -nk_box(p, vec2<f32>(0.3, 0.14), vec2<f32>(0.7, 0.44)));
        }
        case 2u: {
            var d = min(nk_box(p, vec2<f32>(0.0, 0.28), vec2<f32>(0.2, 1.0)),
                nk_box(p, vec2<f32>(0.36, 0.28), vec2<f32>(0.56, 1.0)));
            d = min(d, nk_box(p, vec2<f32>(0.72, 0.28), vec2<f32>(0.92, 1.0)));
            d = min(d, nk_arc(p, vec2<f32>(0.28, 0.28), 0.18, NK_H, 1u));
            return min(d, nk_arc(p, vec2<f32>(0.64, 0.28), 0.18, NK_H, 1u));
        }
        case 3u: {
            let diag = max(min(nk_capsule(p, vec2<f32>(0.15, 0.38), vec2<f32>(0.48, 0.92), NK_H),
                nk_capsule(p, vec2<f32>(0.26, 0.5), vec2<f32>(0.48, 0.08), NK_H)),
                nk_box(p, vec2<f32>(0.0, 0.0), vec2<f32>(0.56, 1.0)));
            return min(nk_box(p, vec2<f32>(0.0, 0.0), vec2<f32>(0.2, 1.42)), diag);
        }
        case 4u: {
            return nk_circle(p, vec2<f32>(0.13, 0.13), 0.13);
        }
        case 5u: {
            return min(nk_box(p, vec2<f32>(0.0, 0.0), vec2<f32>(0.2, 1.0)),
                nk_circle(p, vec2<f32>(0.1, 1.24), 0.11));
        }
        default: {
            var d = min(nk_box(p, vec2<f32>(0.04, 0.26), vec2<f32>(0.24, 1.3)),
                nk_arc(p, vec2<f32>(0.3, 0.26), 0.16, NK_H, 2u));
            d = min(d, nk_box(p, vec2<f32>(0.3, 0.0), vec2<f32>(0.44, 0.2)));
            return min(d, nk_box(p, vec2<f32>(0.0, 0.82), vec2<f32>(0.44, 1.0)));
        }
    }
}

// Signed distance (x-heights) to the glyphs first..=last of "newk.it".
fn nk_word(p: vec2<f32>, first: u32, last: u32) -> f32 {
    var starts = array<f32, 7>(0.00, 0.65, 1.30, 2.31, 2.96, 3.31, 3.60);
    if p.y < -0.5 || p.y > 1.9 || p.x < starts[first] - 0.5 || p.x > NK_END + 0.5 {
        return 1.0;
    }
    var d = 1.0;
    for (var g = first; g <= last; g++) {
        d = min(d, nk_glyph(g, vec2<f32>(p.x - starts[g], p.y)));
    }
    return d;
}

// Round the torso from the zip and from the spine (m, + to the right).
fn nk_around(p: vec3<f32>) -> f32 {
    return atan2(p.x / NK_SEMI.x, -(p.z - NK_AXIS_Z) / NK_SEMI.y) * NK_ROUND;
}

fn nk_behind(p: vec3<f32>) -> f32 {
    return atan2(p.x / NK_SEMI.x, (p.z - NK_AXIS_Z) / NK_SEMI.y) * NK_ROUND;
}

// Signed distance to a rounded panel centred at c with the half sizes h
// (the gap taken off).
fn nk_panel_sd(p: vec2<f32>, c: vec2<f32>, h: vec2<f32>) -> f32 {
    let q = abs(p - c) - (h - NK_CELL_GAP) + NK_CELL_RADIUS;
    return length(max(q, vec2<f32>(0.0))) + min(max(q.x, q.y), 0.0) - NK_CELL_RADIUS;
}

// The panel at (u, y): x 1 when filled white, y its signed distance (m).
// The back above the lowest row is one panel across the spine.
fn nk_panel(u: f32, y: f32) -> vec2<f32> {
    let pi_r = 3.14159265 * NK_ROUND;
    if abs(u) > NK_COLS.y && y >= NK_ROWS.y {
        let s = select(-pi_r - u, pi_r - u, u > 0.0);
        let w = pi_r - NK_COLS.y;
        let sd = nk_panel_sd(vec2<f32>(s, y), vec2<f32>(0.0, 0.5 * (NK_ROWS.y + NK_ROWS.w)),
            vec2<f32>(w, 0.5 * (NK_ROWS.w - NK_ROWS.y)));
        return vec2<f32>(0.0, sd);
    }
    var cols = array<f32, 7>(-pi_r, -NK_COLS.y, -NK_COLS.x, 0.0, NK_COLS.x, NK_COLS.y, pi_r);
    var k = 5;
    for (var i = 0; i < 6; i++) {
        if u < cols[i + 1] {
            k = i;
            break;
        }
    }
    var j = 2;
    if y < NK_ROWS.y {
        j = 0;
    } else if y < NK_ROWS.z {
        j = 1;
    }
    var rows = array<f32, 4>(NK_ROWS.x, NK_ROWS.y, NK_ROWS.z, NK_ROWS.w);
    let u0 = cols[k];
    let u1 = cols[k + 1];
    let y0 = rows[j];
    let y1 = rows[j + 1];
    let sd = nk_panel_sd(vec2<f32>(u, y), vec2<f32>(0.5 * (u0 + u1), 0.5 * (y0 + y1)),
        vec2<f32>(0.5 * (u1 - u0), 0.5 * (y1 - y0)));
    let white = (j == 1 && k >= 1 && k <= 4) || (j == 0 && k == 5);
    return vec2<f32>(select(0.0, 1.0, white), sd);
}

fn newkit_jersey(p: vec3<f32>, region: u32, three: bool, aa: f32) -> vec4<f32> {
    let black = vec4<f32>(1.0, 0.0, 0.0, 0.0);
    let white = vec4<f32>(0.0, 0.0, 1.0, 0.0);
    let pink = select(white, vec4<f32>(0.0, 0.0, 0.0, 1.0), three);
    if region == 0u {
        let u = nk_around(p);
        let pn = nk_panel(u, p.y);
        let inside = over(-pn.y, 0.0, aa);
        let core = over(-pn.y, NK_CELL_STROKE, aa);
        var w = mix(black, white, select(inside - core, inside, pn.x > 0.5));
        if p.z < NK_AXIS_Z {
            let d = nk_word(vec2<f32>(NK_DOT_CENTRE - u / NK_CHEST_XH, (p.y - NK_CHEST_BASE) / NK_CHEST_XH), 0u, 6u) * NK_CHEST_XH;
            w = mix(w, white, over(-d, -NK_OUTLINE, aa));
            w = mix(w, black, over(-d, 0.0, aa));
        } else {
            let s = nk_behind(p);
            let newk = nk_word(vec2<f32>((NK_BACK_TOP - p.y) / NK_BACK_XH, (s - NK_BACK_BASE) / NK_BACK_XH), 0u, 3u) * NK_BACK_XH;
            let it = nk_word(vec2<f32>(2.96 + (NK_BACK_IT_TOP - p.y) / NK_BACK_IT_XH, (s - NK_BACK_IT_BASE) / NK_BACK_IT_XH), 4u, 6u) * NK_BACK_IT_XH;
            w = mix(w, pink, over(-min(newk, it), 0.0, aa));
        }
        return mix(w, white, over(p.y, NK_COLLAR, aa) * within(p.x, NK_COLLAR_X, aa));
    }
    if region == 1u {
        let line = within(p.y - NK_SLEEVE_LINE, NK_SLEEVE_LINE_HALF, aa);
        var w = mix(black, white, line);
        if p.x > 0.0 {
            // The right sleeve: white with a black line.
            w = mix(white, black, line);
        }
        return mix(w, black, 1.0 - over(p.y, NK_CUFF, aa));
    }
    return black;
}

// Round the thigh from its outside (m): + to the back on the left leg,
// + to the front on the right one.
fn nk_round_thigh(p: vec3<f32>) -> f32 {
    let dx = abs(p.x) - NK_THIGH_AXIS.x;
    let dz = p.z - NK_THIGH_AXIS.y;
    return atan2(select(-dz, dz, p.x < 0.0), dx) * NK_THIGH_R;
}

fn newkit_shorts(p: vec3<f32>, region: u32, three: bool, aa: f32) -> vec4<f32> {
    let black = vec4<f32>(1.0, 0.0, 0.0, 0.0);
    let white = vec4<f32>(0.0, 0.0, 1.0, 0.0);
    if region != 2u {
        return black;
    }
    let s = nk_round_thigh(p);
    if p.x < 0.0 {
        let pink = select(white, vec4<f32>(0.0, 0.0, 0.0, 1.0), three);
        let d = nk_word(vec2<f32>((p.y - NK_LEG_BOTTOM) / NK_LEG_XH, (NK_LEG_BASE - s) / NK_LEG_XH), 0u, 6u) * NK_LEG_XH;
        return mix(black, pink, over(-d, 0.0, aa));
    }
    var squares = array<f32, 3>(NK_SQUARES.x, NK_SQUARES.y, NK_SQUARES.z);
    var cov = 0.0;
    for (var i = 0; i < 3; i++) {
        let q = abs(vec2<f32>(s, p.y - squares[i])) - NK_SQUARE_HALF + NK_SQUARE_RADIUS;
        let d = length(max(q, vec2<f32>(0.0))) + min(max(q.x, q.y), 0.0) - NK_SQUARE_RADIUS;
        var c = over(-d, 0.0, aa);
        if i > 0 {
            c = c - over(-d, NK_SQUARE_STROKE, aa);
        }
        cov = max(cov, c);
    }
    return mix(black, white, cov);
}

// The name font (ocw_core::kit::newkit::NAME_GLYPHS): 4 x 7 pixels, bit
// row * 4 + col.
fn nk_name_glyph(c: u32) -> u32 {
    var g = array<u32, 40>(0u, 161085846u, 127498647u, 235999518u, 127506839u, 252801311u, 17920287u, 244961566u, 161085849u, 119677479u, 110659724u, 160772953u, 252776721u, 161062905u, 161340345u, 110729622u, 17922455u, 177576342u, 160790935u, 126378270u, 35791399u, 110729625u, 107583897u, 167745945u, 160851609u, 35792213u, 252781711u, 110861718u, 119677490u, 252856470u, 126380167u, 143194521u, 110659359u, 110719254u, 35792015u, 110717334u, 109635990u, 28672u, 33554432u, 34u);
    return g[min(c, 39u)];
}

// Ink of the packed name at (u, v) in its spot (centred, u to the reader's
// right): ocw_core::kit::newkit::name_ink.
fn nk_name_ink(name: vec4<u32>, u: f32, v: f32, h_max: f32, w_max: f32) -> f32 {
    let n = min(name.w, NK_NAME_MAX);
    if n == 0u {
        return 0.0;
    }
    let px = min(h_max / 7.0, w_max / f32(6u * n - 2u));
    let h = px * 7.0;
    let width = f32(6u * n - 2u) * px;
    let gx = (u + 0.5 * width) / px;
    let gy = (0.5 * h - v) / px;
    if gx < 0.0 || gy < 0.0 || gx >= width / px || gy >= 7.0 {
        return 0.0;
    }
    let cell = u32(floor(gx / 6.0));
    let col = u32(floor(gx)) - cell * 6u;
    if col >= 4u || cell >= n {
        return 0.0;
    }
    var words = array<u32, 3>(name.x, name.y, name.z);
    let code = (words[cell / 5u] >> (6u * (cell % 5u))) & 63u;
    let row = u32(floor(gy));
    return f32((nk_name_glyph(code) >> (row * 4u + col)) & 1u);
}

// Where the name sits at p (xy: the spot's coordinates, z: the largest
// letter height, 0 = none; w: the widest line).
fn nk_name_spot(garment: u32, region: u32, p: vec3<f32>) -> vec4<f32> {
    if garment == 1u && region == 0u && p.z < NK_AXIS_Z && (p.y < NK_COLLAR || abs(p.x) >= NK_COLLAR_X) {
        let c = NK_CHEST_NAME;
        return vec4<f32>(c.x - nk_around(p), p.y - c.y, c.z, c.w);
    }
    if garment == 1u && region == 1u && p.x < 0.0 {
        let dx = -(p.x + NK_SLEEVE_AXIS.x);
        let dz = p.z - NK_SLEEVE_AXIS.y;
        if dx > 0.0 {
            let c = NK_SLEEVE_NAME;
            return vec4<f32>(atan2(dz, dx) * NK_SLEEVE_R - c.x, p.y - c.y, c.z, c.w);
        }
    }
    if garment == 2u && region == 0u && p.z > NK_SHORTS_BACK_Z {
        let c = NK_SHORTS_NAME;
        return vec4<f32>(p.x - c.x, p.y - c.y, c.z, c.w);
    }
    return vec4<f32>(0.0);
}

// The name's ink, box-filtered over the pixel (four taps).
fn nk_name_cover(name: vec4<u32>, s: vec4<f32>, aa: f32) -> f32 {
    if s.z <= 0.0 {
        return 0.0;
    }
    let o = 0.5 * aa;
    return 0.25 * (nk_name_ink(name, s.x - o, s.y - o, s.z, s.w) + nk_name_ink(name, s.x + o, s.y - o, s.z, s.w)
        + nk_name_ink(name, s.x - o, s.y + o, s.z, s.w) + nk_name_ink(name, s.x + o, s.y + o, s.z, s.w));
}

// The rest-to-world frame of the surface from screen derivatives: the
// mesh's rest coordinates are an affine image of its world position, so
// (dlx, dly) ↦ (dwx, dwy) is the same linear map everywhere on it.
struct Frame {
    l1: vec3<f32>,
    l2: vec3<f32>,
    w1: vec3<f32>,
    w2: vec3<f32>,
}

// A world direction (the normal) in rest coordinates.
fn to_rest(f: Frame, n: vec3<f32>) -> vec3<f32> {
    let wn = cross(f.w1, f.w2);
    let ln = cross(f.l1, f.l2);
    let det = dot(wn, wn);
    if det <= 0.0 || dot(ln, ln) <= 0.0 {
        return vec3<f32>(0.0, 1.0, 0.0);
    }
    let a = dot(n, cross(f.w2, wn)) / det;
    let b = dot(n, cross(wn, f.w1)) / det;
    let c = dot(n, wn) / det;
    let s = length(f.w1) / max(length(f.l1), 1e-12);
    return normalize(a * f.l1 + b * f.l2 + c * s * ln);
}

// A rest-coordinate direction in the world.
fn to_world(f: Frame, v: vec3<f32>) -> vec3<f32> {
    let ln = cross(f.l1, f.l2);
    let det = dot(ln, ln);
    if det <= 0.0 {
        return vec3<f32>(0.0, 1.0, 0.0);
    }
    let a = dot(v, cross(f.l2, ln)) / det;
    let b = dot(v, cross(ln, f.l1)) / det;
    let c = dot(v, ln) / det;
    let s = length(f.w1) / max(length(f.l1), 1e-12);
    return a * f.w1 + b * f.w2 + c * cross(f.w1, f.w2) / max(s, 1e-12);
}

// Mikkelsen's bump mapping from the height's screen derivatives.
fn bump(n: vec3<f32>, f: Frame, dhx: f32, dhy: f32) -> vec3<f32> {
    let r1 = cross(f.w2, n);
    let r2 = cross(n, f.w1);
    let det = dot(f.w1, r1);
    let g = sign(det) * (dhx * r1 + dhy * r2);
    return normalize(abs(det) * n - g);
}

struct Detail {
    albedo: vec3<f32>,
    rough: f32,
    // Height derivatives along the screen (bump).
    dhx: f32,
    dhy: f32,
}

// A texture layer, triplanar in rest coordinates: albedo (tinted to
// `tone`), roughness and the bump.
fn triplanar(lk: GearLook, slot: u32, layer: i32, p: vec3<f32>, nl: vec3<f32>, f: Frame, tone: vec3<f32>, strength: f32) -> Detail {
    // A variable: arrays are indexed by a run-time layer.
    var look = lk;
    let size = look.layers[layer].w;
    var w = pow(abs(nl), vec3<f32>(4.0));
    w = w / max(w.x + w.y + w.z, 1e-6);
    // Planes: x → (z, y), y → (x, z), z → (x, y).
    let uvx = p.zy / size;
    let uvy = p.xz / size;
    let uvz = p.xy / size;
    let ax = sample_albedo(slot, uvx, layer, f.l1.zy / size, f.l2.zy / size);
    let ay = sample_albedo(slot, uvy, layer, f.l1.xz / size, f.l2.xz / size);
    let az = sample_albedo(slot, uvz, layer, f.l1.xy / size, f.l2.xy / size);
    let nx = sample_normal(slot, uvx, layer, f.l1.zy / size, f.l2.zy / size);
    let ny = sample_normal(slot, uvy, layer, f.l1.xz / size, f.l2.xz / size);
    let nz = sample_normal(slot, uvz, layer, f.l1.xy / size, f.l2.xy / size);
    let a = ax * w.x + ay * w.y + az * w.z;
    // Slopes of the height per rest metre along each plane's axes.
    let g = vec3<f32>(0.0, nx.y, nx.x) * w.x + vec3<f32>(ny.x, 0.0, ny.y) * w.y
        + vec3<f32>(nz.x, nz.y, 0.0) * w.z;
    // Height in units of the repeat: slope × size.
    let gs = -g * strength * size * 0.15;
    var d: Detail;
    let mean = max(look.layers[layer].rgb, vec3<f32>(0.02));
    d.albedo = clamp(a.rgb * tone / mean, vec3<f32>(0.0), vec3<f32>(1.0));
    d.rough = clamp(a.a, 0.05, 1.0);
    d.dhx = dot(gs, f.l1) / size;
    d.dhy = dot(gs, f.l2) / size;
    return d;
}

// The detail heights (m) of the procedural materials at rest point q.
fn h_twill(q: vec3<f32>, plane: u32, strength: f32) -> vec2<f32> {
    // A 2 × 2 twill of 3 mm tows on the dominant plane; y: the tow's
    // shade (along u lighter).
    var uv = q.xy;
    if plane == 0u {
        uv = q.zy;
    } else if plane == 1u {
        uv = q.xz;
    }
    let t = uv / 0.003;
    let cell = floor(t);
    let fr = t - cell;
    let along_u = (i32(cell.x) - i32(cell.y) + 4096) % 4 < 2;
    let across = select(fr.x, fr.y, along_u);
    return vec2<f32>(sin(3.14159 * across) * 0.0004 * strength, select(0.8, 1.25, along_u));
}

fn h_grain(q: vec3<f32>, size: f32, amp: f32) -> f32 {
    return noise3(q / max(size, 1e-4)) * size * amp;
}

fn h_metal(q: vec3<f32>, size: f32, strength: f32) -> f32 {
    // Spun: fine rings about the node's x axis.
    let r = length(q.yz);
    return (noise3(vec3<f32>(r / max(size, 1e-4), q.x * 3.0, 0.5)) - 0.5) * size * 0.2 * strength;
}

// Linear brushing: fine grooves along the rest direction `a` (unit), with
// `b1`, `b2` across it.
fn h_brushed(q: vec3<f32>, a: vec3<f32>, b1: vec3<f32>, b2: vec3<f32>, size: f32, strength: f32) -> f32 {
    let s = max(size, 1e-4);
    return (noise3(vec3<f32>(dot(q, a) * 3.0, dot(q, b1) / s, dot(q, b2) / s)) - 0.5) * s * 0.2 * strength;
}

// How a metal surface is brushed (Rust twin: gear::brush): `nl` the rest
// normal, `al` the tube's axis in rest coordinates (zero on flat faces),
// `q` the rest point.
fn brush(nl: vec3<f32>, al: vec3<f32>, q: vec3<f32>) -> u32 {
    if abs(nl.x) > 0.7 {
        // A face across the axle: discs, flanges, rings, rotors.
        return BRUSH_SPUN;
    }
    if dot(al, al) < 0.25 {
        return BRUSH_PLAIN;
    }
    if abs(al.x) > 0.9 && length(q.yz) < AXLE_R {
        return BRUSH_SPUN;
    }
    return BRUSH_LINEAR;
}

fn h_tyre(q: vec3<f32>, size: f32, strength: f32) -> f32 {
    // About the hub: a slick centre strip, a file tread on the shoulders
    // (diamonds of `size`, a whole number round the wheel: no seam).
    let count = floor(6.2832 * 0.33 / size);
    let u = atan2(q.y, q.z) / 6.2832 * count * size;
    let a = fract((u + q.x) / size) - 0.5;
    let b = fract((u - q.x) / size) - 0.5;
    let diamond = (abs(a) + abs(b)) * size;
    let centre = 1.0 - smoothstep(0.003, 0.006, abs(q.x));
    return diamond * 0.12 * strength * (1.0 - centre) + noise3(q * 900.0) * 0.00015;
}

fn h_rib(q: vec3<f32>, size: f32, strength: f32) -> f32 {
    return sin(q.y / max(size, 1e-4) * 6.2832) * size * 0.03 * strength;
}

// Height of `kind`'s detail at q (the twill: its own function).
fn detail_height(kind: u32, q: vec3<f32>, size: f32, strength: f32) -> f32 {
    if kind == KIND_METAL {
        return h_metal(q, size, strength);
    }
    if kind == KIND_TYRE {
        return h_tyre(q, size, strength);
    }
    if kind == KIND_GRIP || kind == KIND_PAD {
        return h_grain(q, size, 0.25 * strength);
    }
    if kind == KIND_LYCRA {
        return h_rib(q, size, strength);
    }
    // Knit and leather without the library: a fine grain.
    return h_grain(q, size, 0.08 * strength);
}

@fragment
fn fragment(
    in: VertexOutput,
    @builtin(front_facing) is_front: bool,
) -> FragmentOutput {
@if(VISIBILITY_RANGE_DITHER)
    visibility_range_dither(in.position, in.visibility_range_dither);

    var pbr_input = pbr_input_from_standard_material(in, is_front);

    let slot = mesh[in.instance_index].material_and_lightmap_bind_group_slot & 0xffffu;
@if(BINDLESS)
    var look = gear_looks[gear_indices[slot].look];
@else
    var look = gear_look;

    let p = vec3<f32>(in.uv.x, in.uv.y, in.uv_b.x);
    let region = u32(in.uv_b.y + 0.5);
    // The only derivatives: in uniform control flow, before any branch.
    var f: Frame;
    f.l1 = dpdx(p);
    f.l2 = dpdy(p);
    f.w1 = dpdx(in.world_position.xyz);
    f.w2 = dpdy(in.world_position.xyz);
    // How the normal turns across the pixel (brushed metal: round a tube,
    // not along it).
    let dn1 = dpdx(in.world_normal);
    let dn2 = dpdy(in.world_normal);
    let px = max(max(length(f.l1), length(f.l2)), 1e-6);
    let aa = 0.75 * px;

    let kind = u32(look.mode.x + 0.5);
    let garment = u32(look.mode.y + 0.5);
    let pattern = u32(look.mode.z + 0.5);
    let flags = u32(look.mode.w + 0.5);
    let three = (flags & 1u) != 0u;
    let textured = (flags & 2u) != 0u;
    let strength = look.params.y;
    let size = look.params.z;

    var n = pbr_input.N;
    var rough = pbr_input.material.perceptual_roughness;
    var albedo = look.base.rgb;

    // The design's colours (garments).
    if garment != 0u {
        var w = vec4<f32>(1.0, 0.0, 0.0, 0.0);
        if garment == 1u {
            w = jersey_weights(p, region, pattern, three, aa);
        } else if garment == 2u {
            w = shorts_weights(p, region, pattern, three, aa);
        } else {
            w = helmet_weights(p, pattern, three, aa);
        }
        albedo = w.x * look.base.rgb + w.y * look.first.rgb + w.z * look.second.rgb + w.w * look.third.rgb;
    }

    // The detail of the material, fading out as the pixel grows: slopes
    // from the height one pixel over (rest coordinates).
    let fade = 1.0 - smoothstep(size * 0.15, size * 0.6, px);
    var dhx = 0.0;
    var dhy = 0.0;
    let textured_kind = kind == KIND_CARBON || kind == KIND_KNIT || kind == KIND_LEATHER;
    if textured_kind && textured {
        let layer = select(select(2, 0, kind == KIND_KNIT), 1, kind == KIND_CARBON);
        let tfade = 1.0 - smoothstep(look.layers[layer].w * 0.004, look.layers[layer].w * 0.03, px);
        let nl = to_rest(f, n);
        let d = triplanar(look, slot, layer, p, nl, f, albedo, strength);
        albedo = mix(albedo, d.albedo, tfade);
        rough = mix(rough, mix(rough, d.rough, 0.5), tfade);
        dhx = d.dhx * tfade;
        dhy = d.dhy * tfade;
    } else if kind == KIND_CARBON {
        let nl = abs(to_rest(f, n));
        var plane = 2u;
        if nl.x > nl.y && nl.x > nl.z {
            plane = 0u;
        } else if nl.y > nl.z {
            plane = 1u;
        }
        let h0 = h_twill(p, plane, strength);
        dhx = (h_twill(p + f.l1, plane, strength).x - h0.x) * fade;
        dhy = (h_twill(p + f.l2, plane, strength).x - h0.x) * fade;
        albedo = albedo * mix(1.0, h0.y, fade);
        // The tows along u a little glossier: the weave shows in the
        // highlights of the dark base, too.
        rough = rough * mix(1.0, select(1.15, 0.8, h0.y > 1.0), fade);
    } else if kind == KIND_METAL {
        // Spun about an axle, or brushed along a tube (spec §14 Phase 8 T6:
        // fork stanchions, posts, rails, spokes), plain on flat faces.
        let nl = to_rest(f, n);
        var c = dn1 - n * dot(dn1, n);
        let c2 = dn2 - n * dot(dn2, n);
        if dot(c2, c2) > dot(c, c) {
            c = c2;
        }
        var axis_w = vec3<f32>(0.0);
        var al = vec3<f32>(0.0);
        // Curved: the normal turns by more than ~0.1° per pixel.
        if dot(c, c) > 3e-6 {
            axis_w = normalize(cross(n, normalize(c)));
            al = to_rest(f, axis_w);
        }
        let mode = brush(nl, al, p);
        if mode == BRUSH_LINEAR {
            var ref_dir = vec3<f32>(0.0, 1.0, 0.0);
            if abs(al.y) > 0.9 {
                ref_dir = vec3<f32>(1.0, 0.0, 0.0);
            }
            let b1 = normalize(cross(al, ref_dir));
            let b2 = cross(al, b1);
            let h0 = h_brushed(p, al, b1, b2, size, strength);
            dhx = (h_brushed(p + f.l1, al, b1, b2, size, strength) - h0) * fade;
            dhy = (h_brushed(p + f.l2, al, b1, b2, size, strength) - h0) * fade;
            var t = axis_w - n * dot(axis_w, n);
            if look.params.w > 0.0 && dot(t, t) > 1e-8 {
                t = normalize(t);
                pbr_input.anisotropy_strength = look.params.w;
                pbr_input.anisotropy_T = t;
                pbr_input.anisotropy_B = normalize(cross(n, t));
            }
        } else if mode == BRUSH_SPUN {
            let h0 = h_metal(p, size, strength);
            dhx = (h_metal(p + f.l1, size, strength) - h0) * fade;
            dhy = (h_metal(p + f.l2, size, strength) - h0) * fade;
            // Highlights stretched along the rings (anisotropic).
            let tl = vec3<f32>(0.0, -p.z, p.y);
            if look.params.w > 0.0 && dot(tl, tl) > 1e-10 {
                var t = to_world(f, normalize(tl));
                t = t - n * dot(t, n);
                if dot(t, t) > 1e-8 {
                    t = normalize(t);
                    pbr_input.anisotropy_strength = look.params.w;
                    pbr_input.anisotropy_T = t;
                    pbr_input.anisotropy_B = normalize(cross(n, t));
                }
            }
        } else {
            pbr_input.anisotropy_strength = 0.0;
        }
    } else if kind != KIND_PLAIN && kind != KIND_PAINT && kind != KIND_PLASTIC && fade > 0.0 {
        let h0 = detail_height(kind, p, size, strength);
        dhx = (detail_height(kind, p + f.l1, size, strength) - h0) * fade;
        dhy = (detail_height(kind, p + f.l2, size, strength) - h0) * fade;
        if kind == KIND_TYRE {
            rough = rough - 0.1 * (1.0 - smoothstep(0.003, 0.006, abs(p.x)));
        }
    }
    if kind == KIND_CARBON {
        // Near-black under the clear coat: little of the ambient light (its
        // specular on the base read grey-brown in the bright workshop); the
        // lamps' highlights on the coat and the weave stay.
        pbr_input.diffuse_occlusion = pbr_input.diffuse_occlusion * CARBON_AMBIENT;
    }

    // Mesh side panels of the jersey: a perforated knit under the arms.
    if (flags & 4u) != 0u {
        let panel = select(0.0, 1.0, region == 0u) * over(abs(p.x), 0.15, 0.01) * (1.0 - over(p.y, 0.4, 0.01));
        let cell = p.yz / 0.004;
        let q = fract(cell) - 0.5;
        let hole = 1.0 - smoothstep(0.18, 0.3, length(q));
        let pfade = 1.0 - smoothstep(0.0006, 0.002, px);
        albedo = albedo * (1.0 - 0.35 * hole * panel * pfade);
    }

    // The race number: the paper bib on the jersey's back, the paper
    // plates on the hips (on the pelvis: on the thighs the digits turned
    // with the pedals).
    let number = u32(look.params.x + 0.5);
    if number > 0u && garment == 1u && region == 0u && p.z > BIB_BACK_Z {
        let uv = p.xy - BIB_CENTRE;
        let paper = within(uv.x, BIB_HALF.x, aa) * within(uv.y, BIB_HALF.y, aa);
        let ink = digits_cover(number, uv.x, uv.y, BIB_DIGIT, aa) * paper;
        albedo = mix(albedo, vec3<f32>(0.82, 0.82, 0.79), paper);
        albedo = mix(albedo, vec3<f32>(0.012, 0.012, 0.014), ink);
        rough = mix(rough, 0.85, paper);
    }
    if number > 0u && garment == 2u && region == 0u && abs(p.x) > HIP_X {
        let u = select(-p.z, p.z, p.x < 0.0) - HIP_CENTRE.x;
        let ink = digits_cover(number, u, p.y - HIP_CENTRE.y, HIP_DIGIT, aa);
        albedo = mix(vec3<f32>(0.82, 0.82, 0.79), vec3<f32>(0.012, 0.012, 0.014), ink);
        rough = 0.85;
        dhx = 0.0;
        dhy = 0.0;
    }

    // The rider's own name on the newk.it kit: the left chest and sleeve,
    // the shorts' back, in the third colour (pink).
    if pattern == 6u && look.name.w > 0u && (garment == 1u || garment == 2u) {
        let ink = nk_name_cover(look.name, nk_name_spot(garment, region, p), aa);
        albedo = mix(albedo, look.third.rgb, ink);
    }

    if dhx != 0.0 || dhy != 0.0 {
        n = bump(n, f, dhx, dhy);
    }

    pbr_input.material.base_color = vec4<f32>(albedo, 1.0);
    pbr_input.material.perceptual_roughness = clamp(rough, 0.04, 1.0);
    pbr_input.N = n;

    var out: FragmentOutput;
    out.color = apply_pbr_lighting(pbr_input);
    out.color = main_pass_post_lighting_processing(pbr_input, out.color);
    return out;
}
