// Water (spec 4.6 *Water surfaces*, v6.58): lakes, rivers and the sea with
// the colour of their depth, the sky reflected by the Fresnel law,
// ripples that move with the wind and soft, shallow shores.
//
// Vertex data (ocw-render `water`): uv.x = the water's depth there (m;
// below 0 where the shore rises out of it: the surface fades out before
// the shoreline, so the land and the water meet softly), uv.y = the kind
// (0 lake, 1 river, 2 sea). The sky is the fog's colour (the haze at the
// horizon as the screen shows it) turning bluer towards the zenith; the
// sun's glint is the PBR light's specular on the rippled normal. Cheap
// enough for the Quest and WebGL 2: no textures, a few sines and one
// value noise.

import bevy_pbr::render::{
    forward_io::{VertexOutput, FragmentOutput},
    pbr_fragment::pbr_input_from_standard_material,
    pbr_functions::{apply_pbr_lighting, main_pass_post_lighting_processing},
    mesh_view_bindings::{view, globals, fog},
};

struct WaterLook {
    // x: ripple strength (0 flat), y: wind speed (m/s), z: wind direction
    // (rad), w: reflection strength.
    params: vec4<f32>,
}

@group(constants::MATERIAL_BIND_GROUP) @binding(100)
var<uniform> water: WaterLook;

fn hash2(p: vec2<f32>) -> f32 {
    return fract(sin(dot(p, vec2<f32>(127.1, 311.7))) * 43758.5453);
}

fn vnoise(p: vec2<f32>) -> f32 {
    let i = floor(p);
    let f = fract(p);
    let u = f * f * (3.0 - 2.0 * f);
    let a = hash2(i);
    let b = hash2(i + vec2<f32>(1.0, 0.0));
    let c = hash2(i + vec2<f32>(0.0, 1.0));
    let d = hash2(i + vec2<f32>(1.0, 1.0));
    return mix(mix(a, b, u.x), mix(c, d, u.x), u.y);
}

// Slope (d height / dx, dz) of a few directional waves and a noise layer.
fn ripple_slope(p: vec2<f32>, t: f32, wind: vec2<f32>, footprint: f32) -> vec2<f32> {
    var s = vec2<f32>(0.0);
    // Wavelength (m), amplitude share, turn from the wind.
    let waves = array<vec3<f32>, 4>(
        vec3<f32>(7.0, 0.55, 0.0),
        vec3<f32>(3.1, 0.35, 0.6),
        vec3<f32>(1.7, 0.25, -0.5),
        vec3<f32>(0.9, 0.15, 1.1),
    );
    for (var i = 0u; i < 4u; i = i + 1u) {
        let w = waves[i];
        // Waves shorter than a few pixels fade out (no shimmer far away).
        let keep = 1.0 - smoothstep(w.x * 0.15, w.x * 0.6, footprint);
        let c = cos(w.z);
        let sn = sin(w.z);
        let dir = vec2<f32>(wind.x * c - wind.y * sn, wind.x * sn + wind.y * c);
        let k = 6.2831853 / w.x;
        let speed = sqrt(9.81 / k);
        let phase = dot(dir, p) * k - speed * k * t;
        s = s + dir * cos(phase) * w.y * keep;
    }
    let nk = 1.0 - smoothstep(0.4, 1.6, footprint);
    let q = p * 1.3 + wind * t * 0.6;
    let e = 0.15;
    let n0 = vnoise(q);
    s = s + vec2<f32>(vnoise(q + vec2<f32>(e, 0.0)) - n0, vnoise(q + vec2<f32>(0.0, e)) - n0) / e * 0.08 * nk;
    return s;
}

@fragment
fn fragment(
    in: VertexOutput,
    @builtin(front_facing) is_front: bool,
) -> FragmentOutput {
    var pbr_input = pbr_input_from_standard_material(in, is_front);
    let wp = in.world_position.xyz;
    let depth = in.uv.x;
    let kind = in.uv.y;
    let to_eye = view.world_position.xyz - wp;
    let dist = length(to_eye);
    let v = to_eye / max(dist, 1e-3);
    // The size of a pixel on the water (m).
    let footprint = length(fwidth(wp.xz));

    // Ripples: stronger with wind, calmer on small lakes than on the sea.
    let wind_dir = vec2<f32>(cos(water.params.z), sin(water.params.z));
    let calm = select(1.0, 1.4, kind > 1.5);
    let strength = water.params.x * calm * clamp(0.25 + water.params.y / 8.0, 0.25, 1.6);
    let slope = ripple_slope(wp.xz, globals.time, wind_dir, footprint) * 0.09 * strength;
    let n = normalize(vec3<f32>(-slope.x, 1.0, -slope.y));
    pbr_input.N = n;
    pbr_input.world_normal = n;

    // The water's own colour by depth and kind (linear): shallow water
    // shows the brown-green of its bed, deep water dark blue-green;
    // rivers greener and murkier, the sea bluer.
    let deep_lake = vec3<f32>(0.010, 0.034, 0.040);
    let deep_river = vec3<f32>(0.020, 0.045, 0.035);
    let deep_sea = vec3<f32>(0.004, 0.030, 0.060);
    var deep = deep_lake;
    if kind > 1.5 {
        deep = deep_sea;
    } else if kind > 0.5 {
        deep = deep_river;
    }
    let shallow = vec3<f32>(0.090, 0.090, 0.060);
    let d = max(depth, 0.0);
    let body = mix(deep, shallow, exp(-d / 1.2));
    pbr_input.material.base_color = vec4<f32>(body, 1.0);
    pbr_input.material.perceptual_roughness = 0.06;
    pbr_input.material.reflectance = vec3<f32>(0.02);
    pbr_input.material.metallic = 0.0;

    var out: FragmentOutput;
    var lit = apply_pbr_lighting(pbr_input);

    // The sky in the water: the horizon's haze towards a bluer zenith, by
    // Schlick's Fresnel term (2 % looking down, all of it at grazing view).
    let r = reflect(-v, n);
    let up = clamp(r.y, 0.0, 1.0);
    let horizon = fog.base_color.rgb;
    let zenith = horizon * vec3<f32>(0.55, 0.72, 1.05);
    let sky = mix(horizon, zenith, pow(up, 0.6));
    let cos_v = clamp(dot(n, v), 0.0, 1.0);
    let fresnel = 0.02 + 0.98 * pow(1.0 - cos_v, 5.0);
    let rgb = mix(lit.rgb, sky, clamp(fresnel * water.params.w, 0.0, 1.0));

    // The shore: a light, broken band of foam on the shallowest water,
    // and the surface fading out where it gets thin, so the bank shows
    // through instead of a hard edge.
    let foam_n = vnoise(wp.xz * 0.9 + vec2<f32>(globals.time * 0.15, 0.0));
    let foam = (1.0 - smoothstep(0.02, 0.22, depth)) * smoothstep(0.35, 0.75, foam_n)
        * (1.0 - smoothstep(30.0, 120.0, dist));
    let foam_rgb = mix(rgb, horizon * 1.6 + vec3<f32>(0.05), foam * 0.35);
    let alpha = smoothstep(-0.02, 0.35, depth) * mix(0.82, 0.97, smoothstep(0.0, 1.5, d));
    out.color = vec4<f32>(foam_rgb, alpha);
    out.color = main_pass_post_lighting_processing(pbr_input, out.color);
    return out;
}
