// The HUD's navigation map (spec §3.2 *Navigation map*): the cached
// north-up map tiles (a wrap-around ring of tiles, `ocw_map::ring`) turned
// heading-up around the rider, the route layer over them, the rounded
// screen of the bike computer. Mirrors `ocw_map::view::NaviView::texel_axes`.

import bevy_ui_render::ui_vertex_output::UiVertexOutput;

struct NaviMapParams {
    // xy: the rider's texel in the ring (mod its size); z: the ring's
    // size in texels; w: 1 = the route layer is drawn.
    ring: vec4<f32>,
    // xy: texels per whole map width (u); zw: per whole map height (v).
    axes: vec4<f32>,
    // xy: the rider's uv on the map; zw: the cached window's lowest
    // corner relative to the rider (texels).
    anchor: vec4<f32>,
    // xy: the window's highest corner relative to the rider (texels).
    window: vec4<f32>,
    // The ground's colour where no tile is drawn yet (linear rgb).
    ground: vec4<f32>,
}

@group(1) @binding(0)
var<uniform> params: NaviMapParams;
@group(1) @binding(1)
var base_texture: texture_2d<f32>;
@group(1) @binding(2)
var base_sampler: sampler;
@group(1) @binding(3)
var route_texture: texture_2d<f32>;
@group(1) @binding(4)
var route_sampler: sampler;

// Signed distance to a box of half size `half` with corner radius `r`.
fn rounded_box(p: vec2<f32>, half: vec2<f32>, r: f32) -> f32 {
    let q = abs(p) - half + vec2<f32>(r, r);
    return length(max(q, vec2<f32>(0.0, 0.0))) + min(max(q.x, q.y), 0.0) - r;
}

@fragment
fn fragment(in: UiVertexOutput) -> @location(0) vec4<f32> {
    let d = in.uv - params.anchor.xy;
    let t = params.axes.xy * d.x + params.axes.zw * d.y;
    let tex = (params.ring.xy + t) / params.ring.z;
    // Sampled everywhere (uniform control flow), masked after.
    let base = textureSample(base_texture, base_sampler, tex);
    let route = textureSample(route_texture, route_sampler, tex);
    let lo = params.anchor.zw;
    let hi = params.window.xy;
    let inside = select(0.0, 1.0, t.x >= lo.x && t.y >= lo.y && t.x < hi.x && t.y < hi.y);
    // Both layers are premultiplied; an undrawn slot is transparent.
    var col = base.rgb * inside + params.ground.rgb * (1.0 - base.a * inside);
    let ra = route.a * inside * params.ring.w;
    col = route.rgb * inside * params.ring.w + col * (1.0 - ra);
    // The screen's rounded corners and a soft inner shadow at its edge.
    let size = in.size;
    let p = in.uv * size - size * 0.5;
    let r = in.border_radius_x.x;
    let sd = rounded_box(p, size * 0.5, r);
    let alpha = clamp(0.5 - sd, 0.0, 1.0);
    let shade = 1.0 - 0.22 * (1.0 - smoothstep(0.0, 7.0, -sd));
    return vec4<f32>(col * shade, alpha);
}
