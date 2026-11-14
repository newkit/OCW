// Clouds (spec §14 Phase 9, S4): the view ray meets the cloud layer, layered
// value noise shapes the cloud type there, a tap towards the sun shades it,
// the haze swallows it towards the horizon, lightning lights it up; the sky
// behind dense fog becomes the fog's colour. `weather::clouds::density` is
// the CPU twin of `cloud_density`.

import bevy_pbr::render::{
    forward_io::VertexOutput,
    mesh_view_bindings::view,
};

struct Clouds {
    // x: cover, y: base altitude (scene y), z: thickness, w: type.
    layer: vec4<f32>,
    // x, z: drift (m), y: feature scale (m), w: darkness.
    drift: vec4<f32>,
    // xyz: towards the key light, w: its illuminance above the clouds.
    sun: vec4<f32>,
    // rgb: key colour, w: lightning flash.
    sun_rgb: vec4<f32>,
    // rgb: ambient (colour x brightness), w: wind direction (rad).
    ambient: vec4<f32>,
    // rgb: fog colour (linear), w: visibility (m).
    fog: vec4<f32>,
    // xyz: towards the flash.
    flash: vec4<f32>,
}

@group(constants::MATERIAL_BIND_GROUP) @binding(0)
var<uniform> clouds: Clouds;

const PI: f32 = 3.14159265;

fn hash(x: i32, y: i32) -> f32 {
    var h = u32(x) * 374761393u + u32(y) * 668265263u;
    h = (h ^ (h >> 13u)) * 1274126177u;
    h = h ^ (h >> 16u);
    return f32(h & 0xffffffu) / 16777216.0;
}

fn vnoise(p: vec2<f32>) -> f32 {
    let i = floor(p);
    let f = p - i;
    let u = f * f * (vec2<f32>(3.0) - 2.0 * f);
    let x = i32(i.x);
    let y = i32(i.y);
    let ab = mix(hash(x, y), hash(x + 1, y), u.x);
    let cd = mix(hash(x, y + 1), hash(x + 1, y + 1), u.x);
    return mix(ab, cd, u.y);
}

fn fbm(p: vec2<f32>, octaves: u32) -> f32 {
    var s = 0.0;
    var a = 0.5;
    var q = p;
    var norm = 0.0;
    for (var i = 0u; i < octaves; i = i + 1u) {
        s = s + a * vnoise(q);
        norm = norm + a;
        q = vec2<f32>(0.8 * q.x - 0.6 * q.y, 0.6 * q.x + 0.8 * q.y) * 2.03 + vec2<f32>(17.1, 3.7);
        a = a * 0.5;
    }
    return s / norm;
}

fn cloud_density(q: vec2<f32>, cover: f32, kind: u32, scale: f32, wind: f32) -> f32 {
    let thr = 0.5 - (cover - 0.5) * 0.7;
    var n: f32;
    var soft: f32;
    if kind == 1u {
        n = fbm(q / scale, 4u);
        soft = 0.55;
    } else if kind == 2u {
        let s = sin(wind);
        let c = cos(wind);
        let r = vec2<f32>(c * q.x + s * q.y, -s * q.x + c * q.y);
        n = fbm(vec2<f32>(r.x / (scale * 5.0), r.y / scale), 4u);
        soft = 0.3;
    } else if kind == 3u {
        n = fbm(q / scale, 5u);
        soft = 0.25;
    } else {
        n = fbm(q / scale, 5u);
        soft = 0.14;
    }
    let d = smoothstep(thr - soft * 0.5, thr + soft * 0.5, n);
    if kind == 2u {
        return d * 0.7;
    }
    return d;
}

// Optical strength of the types.
fn opacity(kind: u32) -> f32 {
    if kind == 1u { return 2.6; }
    if kind == 2u { return 0.6; }
    if kind == 3u { return 5.0; }
    return 3.2;
}

@fragment
fn fragment(in: VertexOutput) -> @location(0) vec4<f32> {
    let eye = view.world_position.xyz;
    let d = normalize(in.world_position.xyz - eye);
    let cover = clouds.layer.x;
    let base = clouds.layer.y;
    let top = base + clouds.layer.z;
    let kind = u32(clouds.layer.w + 0.5);
    let scale = clouds.drift.y;
    let darkness = clouds.drift.w;
    let exposure = view.exposure;

    var alpha = 0.0;
    var col = vec3<f32>(0.0);
    var t = -1.0;
    let below = eye.y < base;
    if below && d.y > 0.0015 {
        t = (base - eye.y) / d.y;
    } else if eye.y > top && d.y < -0.0015 {
        t = (top - eye.y) / d.y;
    }
    // The lit colour of a cloud of density `dens`.
    let sun_dir = clouds.sun.xyz;
    let amb = clouds.ambient.rgb;
    if t > 0.0 && cover > 0.0 {
        let p = eye.xz + d.xz * t - clouds.drift.xz;
        let dens = cloud_density(p, cover, kind, scale, clouds.ambient.w);
        if dens > 0.001 {
            // A tap towards the sun: thick clouds shade their own far side.
            let lean = sun_dir.xz / max(sun_dir.y, 0.2) * clouds.layer.z * 0.6;
            let ds = cloud_density(p + lean, cover, kind, scale, clouds.ambient.w);
            let k = opacity(kind);
            let lit = exp(-ds * k * 0.7);
            var through = 1.0;
            var under = 1.0;
            if below {
                // Seen from below: the light comes through the cloud.
                through = 0.12 + 0.88 * exp(-dens * k * 0.55);
                under = mix(1.0, 0.45, dens * min(k / 3.2, 1.0));
            }
            let silver = 1.2 * pow(max(dot(d, sun_dir), 0.0), 10.0) * (1.0 - dens);
            let sun_term = clouds.sun.w * clouds.sun_rgb.rgb * max(sun_dir.y + 0.1, 0.0)
                * (0.15 + 0.85 * lit) * through * (0.6 + silver);
            let dark = 1.0 - darkness * dens * 0.8;
            col = 0.92 * (sun_term / PI + amb * under) * dark;
            // Lightning lights the clouds from inside, most towards it.
            let towards = pow(max(dot(normalize(d.xz + vec2<f32>(1e-4)), normalize(clouds.flash.xz + vec2<f32>(1e-4))), 0.0), 3.0);
            col = col + clouds.sun_rgb.w * vec3<f32>(0.75, 0.8, 1.0) * 2400.0 * dens * (0.3 + 0.7 * towards);
            col = col * exposure;
            let path = min(1.0 / max(abs(d.y), 0.05), 8.0);
            alpha = 1.0 - exp(-dens * k * (0.6 + 0.4 * path));
            // The haze towards the horizon.
            let haze = 1.0 - exp(-t / 45000.0);
            col = mix(col, amb * exposure * 0.9, haze * 0.6);
            alpha = alpha * (1.0 - haze * 0.65);
        }
    }
    // Under a closed deck the horizon is cloud, not blue sky.
    if below && d.y > -0.05 && d.y < 0.12 && cover > 0.5 {
        let band = smoothstep(0.12, 0.0, d.y) * smoothstep(0.5, 1.0, cover);
        let c = amb * exposure * 0.85 * (1.0 - darkness * 0.6);
        let a2 = band * 0.9;
        col = (col * alpha * (1.0 - a2) + c * a2) / max(alpha * (1.0 - a2) + a2, 1e-4);
        alpha = alpha + a2 * (1.0 - alpha);
    }
    // Inside the layer: the cloud is all around (low cloud on a climb).
    if !below && eye.y <= top && cover > 0.6 {
        alpha = 1.0;
        col = amb * exposure * 0.85 * (1.0 - darkness * 0.5);
    }
    // Dense fog: the sky behind it is the fog's colour.
    let vis = max(clouds.fog.w, 1.0);
    // Through the air up to the cloud (at most 2.5 km).
    var reach = 2500.0;
    if t > 0.0 {
        reach = min(t, 2500.0);
    }
    let fog_k = (1.0 - exp(-3.912 * reach / vis)) * smoothstep(15000.0, 3000.0, vis);
    let out_a = 1.0 - (1.0 - fog_k) * (1.0 - alpha);
    if out_a < 0.002 {
        discard;
    }
    let rgb = (fog_k * clouds.fog.rgb + (1.0 - fog_k) * alpha * col) / out_a;
    return vec4<f32>(rgb, out_a);
}
