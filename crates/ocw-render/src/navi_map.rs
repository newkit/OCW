//! The HUD's navigation map (spec §3.2 *Navigation map*): a bike
//! computer's map screen in the lower right corner, both apps.
//!
//! - **The device:** a rounded dark bezel; on top the next-turn banner
//!   ("↱ 250 m Street name": the turn arrow, the distance, the street);
//!   below it the map screen, heading-up with the rider's arrow fixed in
//!   the lower third, the compass needle, the scale bar and the zoom.
//! - **The map:** land use as soft fills, water, buildings as small blocks
//!   close in, roads by class (main roads wider), the route bold (route
//!   rides: the route; free ride: the path chosen so far and ahead, the
//!   next junction's other exits thin), other riders as dots; day and
//!   night palettes by the light at the rider (`ocw_map::navi::Daylight`).
//! - **Drawing:** the map is rasterised on the CPU into tiles (crate
//!   `ocw-map`, tiny-skia) that are cached in a wrap-around ring texture
//!   per zoom level and look ([`ocw_map::ring::Ring`]); as the rider moves
//!   only the tiles coming in are drawn (a time budget per frame, nearest
//!   first). A UI material (`navi_map.wesl`) turns the north-up texture
//!   heading-up on the GPU every frame. A zoom or look change fills a
//!   second texture in the background and swaps when its visible tiles
//!   are there; the route layer is its own texture, redrawn when the
//!   route changes. Nothing is re-tessellated per frame.
//! - **Zoom:** auto (`ocw_map::navi::AutoZoom`: by speed, closer at
//!   junctions) or one of seven levels; key Z (`keys.map_zoom`) steps
//!   auto → 1 … 7 → auto, the console `hud.map.zoom`, the setting
//!   `display.map_zoom`. On / off: key N (`keys.map`), `hud.map`, setting
//!   `display.map` (on); hidden with `hud off`, in unicorn flight and when
//!   the window has no room ([`fit_map`]).
//! - **Place names:** the world packages' places (section `PLCE`) fill
//!   [`NaviMapLabels`] whenever the loaded packages change
//!   (`ocw_map::labels::labels_of`); `ocw_map::labels::navi_labels` picks
//!   the names for the zoom (cities and towns always, villages and hamlets
//!   closer in), decluttered with the studio map's pass and its hysteresis,
//!   clear of the compass, scale, zoom and the rider's arrow. They are UI
//!   text over the map (a few nodes moved per frame, no tile is redrawn),
//!   so they stay upright as the map turns, with a halo in the day or
//!   night colours.

use std::sync::Arc;

use bevy::asset::{AssetPath, RenderAssetUsages, embedded_asset, embedded_path};
use bevy::image::{ImageAddressMode, ImageFilterMode, ImageSampler, ImageSamplerDescriptor};
use bevy::platform::time::Instant;
use bevy::prelude::*;
use bevy::render::render_resource::{
    AsBindGroup, Extent3d, ShaderType, TextureDimension, TextureFormat,
};
use bevy::shader::ShaderRef;
use ocw_map::labels::{PlaceLabel, labels_of, navi_label_font, navi_labels, navi_marks};
use ocw_map::navi::{
    AutoZoom, Banner, Daylight, RouteLines, ZOOM_LEVELS, ZoomMode, banner, ease_zoom, track_line,
};
use ocw_map::raster::{
    MapLine, TILE_PX, TileSpec, compass, render_base_tile, render_line_tile, rider_arrow,
    turn_arrow,
};
use ocw_map::ring::{Ring, slots_for};
use ocw_map::source::{MapSource, SourcePackage};
pub use ocw_map::style::Shade;
use ocw_map::style::{Palette, scale_bar};
use ocw_map::view::{NaviView, bearing, smooth_heading};

use crate::RideView;
use crate::hud::{HudPanel, LookAhead, ShowHud, Units};

pub struct NaviMapPlugin;

impl Plugin for NaviMapPlugin {
    fn build(&self, app: &mut App) {
        embedded_asset!(app, "navi_map.wesl");
        app.add_plugins(UiMaterialPlugin::<NaviMapMaterial>::default());
        add_logic(app);
        app.add_systems(Startup, spawn_device)
            .add_systems(Update, (fit_device.in_set(MapFitSet), update_map).chain());
    }
}

/// The map's layout this frame (`fit_device`): the workout panel is
/// placed after it, clear of the map (spec 3.2 Q1).
#[derive(SystemSet, Debug, Clone, PartialEq, Eq, Hash)]
pub struct MapFitSet;

/// The resources and the settings' side, without the drawing (headless
/// tests with `MinimalPlugins`).
pub fn add_logic(app: &mut App) {
    app.init_resource::<ShowMap>()
        .init_resource::<MapZoom>()
        .init_resource::<NaviMapLabels>()
        .init_resource::<NaviMapInfo>()
        .init_resource::<MapAvoidRects>();
}

/// Whether the navigation map is shown (console `hud.map`, key N,
/// setting `display.map`; on by default).
#[derive(Resource, Debug, Clone, Copy, PartialEq, Eq)]
pub struct ShowMap(pub bool);

impl Default for ShowMap {
    fn default() -> Self {
        Self(true)
    }
}

/// The map's zoom (console `hud.map.zoom`, key Z, setting
/// `display.map_zoom`).
#[derive(Resource, Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct MapZoom(pub ZoomMode);

/// The place names the map draws: the loaded packages' places (section
/// `PLCE`), refreshed when they change; empty for packages without it.
#[derive(Resource, Debug, Clone, Default, PartialEq)]
pub struct NaviMapLabels(pub Vec<PlaceLabel>);

/// What the map shows now – for the tests, the console and the browser
/// page (`window.ocw.hudMap()`: the Devices panel steps above it).
#[derive(Resource, Debug, Clone, Default, PartialEq)]
pub struct NaviMapInfo {
    /// The device on the window (logical px: left, top, width, height);
    /// `None`: hidden.
    pub rect: Option<[f32; 4]>,
    /// The banner as text ("↱ 250 m Street name").
    pub banner: String,
    /// The zoom: the level shown (1 … 7) and whether auto.
    pub level: usize,
    pub auto: bool,
    pub shade: Shade,
    /// Map tiles drawn so far (the cost, for the fps measurement).
    pub tiles_drawn: u64,
    /// The map screen's size (UI px).
    pub map: [f32; 2],
    /// Unicorn flight: north-up, centred on the unicorn (spec 3.2 Q7).
    pub flight: bool,
    /// The point the map is drawn round (package metres, x z): the rider
    /// or the unicorn.
    pub centre: [f64; 2],
}

/// Marks a HUD node the map keeps clear of (the top panel, the top-right
/// column, the event panel, the touch chevrons). The workout panel is not
/// one: it keeps clear of the map instead (spec 3.2 Q1).
#[derive(Component, Debug, Clone, Copy, Default)]
pub struct MapAvoid;

/// The rectangles (UI px) of the visible [`MapAvoid`] nodes last frame.
#[derive(Resource, Debug, Clone, Default, PartialEq)]
pub struct MapAvoidRects(pub Vec<Rect>);

// ---------------------------------------------------------------------
// Layout (pure)
// ---------------------------------------------------------------------

/// The device's margins from the window's right and bottom edges (UI px;
/// the version label stays visible under it).
pub const RIGHT_PX: f32 = 12.0;
pub const BOTTOM_PX: f32 = 24.0;
/// The bezel: padding and border (UI px), the banner's height and the gap
/// to the screen.
pub const BEZEL_PAD: f32 = 5.0;
pub const BEZEL_BORDER: f32 = 2.0;
pub const BANNER_H: f32 = 44.0;
pub const BANNER_GAP: f32 = 4.0;
/// The map screen's preferred and least width (UI px); its height is
/// [`MAP_ASPECT`] × the width (a portrait screen, as on a bike computer).
pub const MAP_W: f32 = 240.0;
pub const MAP_MIN_W: f32 = 120.0;
pub const MAP_ASPECT: f32 = 1.1;
/// Room kept to the other HUD elements (UI px).
pub const GAP_PX: f32 = 8.0;

/// Where the device goes.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MapFit {
    /// The device's box (UI px).
    pub rect: Rect,
    /// The map screen's size (UI px).
    pub map: Vec2,
}

/// The device's extra width and height around the map screen.
pub fn bezel_extra() -> Vec2 {
    let side = 2.0 * (BEZEL_PAD + BEZEL_BORDER);
    Vec2::new(side, side + BANNER_H + BANNER_GAP)
}

/// Fits the device into the lower right corner of a window of `window`
/// UI px (the window's logical size over the HUD scale): right of the
/// junction fan (`fan`, always kept free) and of the elevation profile
/// (`left_column`: its box) and below every box of `above` that shares
/// its column (top panel, top-right column, event panel, touch chevrons).
/// The workout panel no longer shares the corner (spec 3.2 Q1: it moved
/// to the left column, [`crate::workout::fit_workout`]). `None`: no room
/// for the least map (hidden).
pub fn fit_map(
    window: Vec2,
    fan: Rect,
    left_column: Option<Rect>,
    above: &[Rect],
) -> Option<MapFit> {
    let extra = bezel_extra();
    let right = window.x - RIGHT_PX;
    let mut left_limit = fan.max.x + GAP_PX;
    if let Some(l) = left_column {
        left_limit = left_limit.max(l.max.x + GAP_PX);
    }
    let bottom = window.y - BOTTOM_PX;
    // The widest map whose column is clear of the boxes above (a box
    // that no longer shares a narrower column stops counting).
    let mut w = (right - left_limit - extra.x).min(MAP_W).floor();
    while w >= MAP_MIN_W {
        let left = right - w - extra.x;
        let top_limit = above
            .iter()
            .filter(|r| r.max.x + GAP_PX > left && r.min.x < right && r.min.y < bottom)
            .map(|r| r.max.y + GAP_PX)
            .fold(0.0f32, f32::max);
        let h = (w * MAP_ASPECT).floor();
        if bottom - top_limit >= h + extra.y {
            let size = Vec2::new(w, h) + extra;
            let min = Vec2::new(right - size.x, bottom - size.y);
            return Some(MapFit {
                rect: Rect::from_corners(min, min + size),
                map: Vec2::new(w, h),
            });
        }
        w -= 2.0;
    }
    None
}

/// The junction fan's box in UI px (`crate::junction::fan_geometry` at
/// HUD scale 1).
pub fn fan_rect(window: Vec2) -> Rect {
    let g = crate::junction::fan_geometry(window, 1.0);
    Rect::from_corners(g.min, g.max)
}

// ---------------------------------------------------------------------
// The material
// ---------------------------------------------------------------------

/// Uniform of `navi_map.wesl`.
#[derive(Debug, Clone, Copy, PartialEq, Default, Reflect, ShaderType)]
pub struct NaviMapParams {
    pub ring: Vec4,
    pub axes: Vec4,
    pub anchor: Vec4,
    pub window: Vec4,
    pub ground: Vec4,
}

#[derive(Asset, TypePath, AsBindGroup, Debug, Clone)]
pub struct NaviMapMaterial {
    #[uniform(0)]
    pub params: NaviMapParams,
    #[texture(1)]
    #[sampler(2)]
    pub base: Handle<Image>,
    #[texture(3)]
    #[sampler(4)]
    pub route: Handle<Image>,
}

impl UiMaterial for NaviMapMaterial {
    fn fragment_shader() -> ShaderRef {
        ShaderRef::Path(
            AssetPath::from_path_buf(embedded_path!("navi_map.wesl")).with_source("embedded"),
        )
    }
}

/// An empty ring texture of `side` texels (sRGB, wrap-around, linear).
pub fn ring_image(side: u32) -> Image {
    let mut img = Image::new_fill(
        Extent3d {
            width: side,
            height: side,
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        &[0, 0, 0, 0],
        TextureFormat::Rgba8UnormSrgb,
        RenderAssetUsages::default(),
    );
    img.sampler = ImageSampler::Descriptor(ImageSamplerDescriptor {
        address_mode_u: ImageAddressMode::Repeat,
        address_mode_v: ImageAddressMode::Repeat,
        mag_filter: ImageFilterMode::Linear,
        min_filter: ImageFilterMode::Linear,
        ..default()
    });
    img
}

fn icon_image(px: u32, rgba: Vec<u8>) -> Image {
    Image::new(
        Extent3d {
            width: px,
            height: px,
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        rgba,
        TextureFormat::Rgba8UnormSrgb,
        RenderAssetUsages::default(),
    )
}

/// Copies a tile's texels into slot `slot` of a ring image's data
/// (`None`: clears it).
pub fn write_slot(data: &mut [u8], side: u32, ring: &Ring, slot: usize, tile: Option<&[u8]>) {
    let (ox, oy) = ring.slot_origin(slot);
    let row = (TILE_PX * 4) as usize;
    for y in 0..TILE_PX {
        let start = (((oy + y) * side + ox) * 4) as usize;
        let dst = &mut data[start..start + row];
        match tile {
            Some(t) => dst.copy_from_slice(&t[(y as usize) * row..(y as usize + 1) * row]),
            None => dst.fill(0),
        }
    }
}

// ---------------------------------------------------------------------
// The device's nodes
// ---------------------------------------------------------------------

#[derive(Component)]
struct Device;
#[derive(Component)]
struct BannerBox;
#[derive(Component)]
struct BannerIcon;
#[derive(Component)]
struct BannerDistance;
#[derive(Component)]
struct BannerStreet;
#[derive(Component)]
struct Screen;
#[derive(Component)]
struct RiderArrow;
#[derive(Component)]
struct Dot(usize);
#[derive(Component)]
struct Label(usize);
#[derive(Component)]
struct Compass;
#[derive(Component)]
struct ScaleBar;
#[derive(Component)]
struct ScaleText;
#[derive(Component)]
struct ZoomText;

/// The stacking of the map screen's children (`ZIndex` among siblings):
/// the other riders' dots, over them the place names, over those the
/// rider's arrow and the marks (compass, scale, zoom).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MapLayers {
    pub dots: i32,
    pub labels: i32,
    pub marks: i32,
}

pub const MAP_LAYERS: MapLayers = MapLayers {
    dots: 1,
    labels: 2,
    marks: 3,
};

const DOTS: usize = 16;
const LABELS: usize = 8;
const ARROW_PX: f32 = 30.0;
const ICON_PX: u32 = 64;
const COMPASS_PX: f32 = 24.0;
const BEZEL: Color = Color::srgb(0.09, 0.10, 0.11);
const BEZEL_EDGE: Color = Color::srgb(0.27, 0.29, 0.32);
const BANNER_DAY: Color = Color::srgb(0.05, 0.36, 0.25);
const BANNER_NIGHT: Color = Color::srgb(0.04, 0.20, 0.15);
/// The scale bar's longest length (UI px).
const SCALE_MAX_PX: f64 = 70.0;

fn spawn_device(
    mut commands: Commands,
    images: Option<ResMut<Assets<Image>>>,
    materials: Option<ResMut<Assets<NaviMapMaterial>>>,
) {
    let (Some(mut images), Some(mut materials)) = (images, materials) else {
        return;
    };
    let base = images.add(ring_image(TILE_PX));
    let route = images.add(ring_image(TILE_PX));
    let material = materials.add(NaviMapMaterial {
        params: NaviMapParams::default(),
        base,
        route,
    });
    let arrow = images.add(icon_image(
        ICON_PX,
        rider_arrow(ICON_PX, Palette::of(Shade::Day).rider_dot),
    ));
    let needle = images.add(icon_image(ICON_PX, compass(ICON_PX)));
    let turn = images.add(icon_image(ICON_PX, turn_arrow(0.0, ICON_PX, [255; 4])));
    let text = |size: f32, c: Color| {
        (
            Text::new(""),
            TextFont {
                font_size: size.into(),
                ..default()
            },
            TextColor(c),
        )
    };
    commands
        .spawn((
            Node {
                position_type: PositionType::Absolute,
                right: px(RIGHT_PX),
                bottom: px(BOTTOM_PX),
                padding: UiRect::all(px(BEZEL_PAD)),
                border: UiRect::all(px(BEZEL_BORDER)),
                border_radius: BorderRadius::all(px(14)),
                flex_direction: FlexDirection::Column,
                row_gap: px(BANNER_GAP),
                display: Display::None,
                ..default()
            },
            BackgroundColor(BEZEL),
            BorderColor::all(BEZEL_EDGE),
            Device,
            HudPanel,
            Pickable::IGNORE,
        ))
        .with_children(|d| {
            d.spawn((
                Node {
                    height: px(BANNER_H),
                    flex_direction: FlexDirection::Row,
                    align_items: AlignItems::Center,
                    column_gap: px(6),
                    padding: UiRect::axes(px(6), px(2)),
                    border_radius: BorderRadius::all(px(9)),
                    overflow: Overflow::clip(),
                    ..default()
                },
                BackgroundColor(BANNER_DAY),
                BannerBox,
                Pickable::IGNORE,
            ))
            .with_children(|b| {
                b.spawn((
                    Node {
                        width: px(36),
                        height: px(36),
                        flex_shrink: 0.0,
                        ..default()
                    },
                    ImageNode::new(turn),
                    BannerIcon,
                ));
                b.spawn(Node {
                    flex_direction: FlexDirection::Column,
                    overflow: Overflow::clip(),
                    ..default()
                })
                .with_children(|c| {
                    c.spawn((text(19.0, Color::WHITE), BannerDistance));
                    c.spawn((text(12.5, Color::srgb(0.86, 0.93, 0.89)), BannerStreet));
                });
            });
            d.spawn((
                Node {
                    width: px(MAP_W),
                    height: px(MAP_W * MAP_ASPECT),
                    border_radius: BorderRadius::all(px(9)),
                    overflow: Overflow::clip(),
                    ..default()
                },
                MaterialNode(material),
                Screen,
                Pickable::IGNORE,
            ))
            .with_children(|m| {
                for i in 0..DOTS {
                    m.spawn((
                        Node {
                            position_type: PositionType::Absolute,
                            width: px(10),
                            height: px(10),
                            border: UiRect::all(px(1.5)),
                            border_radius: BorderRadius::all(px(5)),
                            display: Display::None,
                            ..default()
                        },
                        BackgroundColor(Color::srgb(0.12, 0.55, 0.9)),
                        BorderColor::all(Color::WHITE),
                        ZIndex(MAP_LAYERS.dots),
                        Dot(i),
                    ));
                }
                // Over the other riders' dots, under the rider's arrow and
                // the map's marks (`MAP_LAYERS`).
                for i in 0..LABELS {
                    m.spawn((
                        Node {
                            position_type: PositionType::Absolute,
                            display: Display::None,
                            padding: UiRect::axes(px(ocw_map::labels::PILL_PAD_PX), px(0)),
                            border_radius: BorderRadius::all(px(5)),
                            ..default()
                        },
                        text(11.0, Color::srgb(0.15, 0.15, 0.18)),
                        TextLayout::new(Justify::Center, LineBreak::NoWrap),
                        // A translucent pill in the map's ground colour: a
                        // road or the route under the name does not cut it.
                        BackgroundColor(Color::srgba(0.93, 0.93, 0.89, 0.8)),
                        ZIndex(MAP_LAYERS.labels),
                        Label(i),
                    ));
                }
                m.spawn((
                    Node {
                        position_type: PositionType::Absolute,
                        width: px(ARROW_PX),
                        height: px(ARROW_PX),
                        ..default()
                    },
                    ImageNode::new(arrow),
                    ZIndex(MAP_LAYERS.marks),
                    RiderArrow,
                ));
                m.spawn((
                    Node {
                        position_type: PositionType::Absolute,
                        top: px(6),
                        right: px(6),
                        width: px(COMPASS_PX),
                        height: px(COMPASS_PX),
                        ..default()
                    },
                    ImageNode::new(needle),
                    ZIndex(MAP_LAYERS.marks),
                    Compass,
                ));
                m.spawn((
                    Node {
                        position_type: PositionType::Absolute,
                        left: px(8),
                        bottom: px(6),
                        flex_direction: FlexDirection::Column,
                        row_gap: px(1),
                        ..default()
                    },
                    ZIndex(MAP_LAYERS.marks),
                ))
                .with_children(|s| {
                    s.spawn((text(10.5, Color::srgb(0.1, 0.1, 0.12)), ScaleText));
                    s.spawn((
                        Node {
                            width: px(50),
                            height: px(4),
                            border: UiRect::all(px(1)),
                            ..default()
                        },
                        BackgroundColor(Color::srgb(0.1, 0.1, 0.12)),
                        BorderColor::all(Color::WHITE),
                        ScaleBar,
                    ));
                });
                m.spawn((
                    Node {
                        position_type: PositionType::Absolute,
                        right: px(7),
                        bottom: px(5),
                        padding: UiRect::axes(px(4), px(1)),
                        border_radius: BorderRadius::all(px(4)),
                        ..default()
                    },
                    ZIndex(MAP_LAYERS.marks),
                    BackgroundColor(Color::srgba(0.0, 0.0, 0.0, 0.45)),
                ))
                .with_child((text(10.5, Color::WHITE), ZoomText));
            });
        });
}

/// The window in UI px (logical size over the HUD scale).
fn window_ui_px(window: &Window, ui_scale: f32) -> Vec2 {
    Vec2::new(window.width(), window.height()) / ui_scale.max(0.1)
}

#[allow(clippy::type_complexity, clippy::too_many_arguments)] // a Bevy system
fn fit_device(
    windows: Query<&Window, With<bevy::window::PrimaryWindow>>,
    ui_scale: Option<Res<UiScale>>,
    avoid: Query<
        (
            &ComputedNode,
            &UiGlobalTransform,
            &InheritedVisibility,
            &Node,
        ),
        With<MapAvoid>,
    >,
    profile: Query<(&ComputedNode, &UiGlobalTransform, &Node), With<crate::hud::ProfilePanel>>,
    mut device: Query<
        &mut Node,
        (
            With<Device>,
            Without<Screen>,
            Without<MapAvoid>,
            Without<crate::hud::ProfilePanel>,
        ),
    >,
    mut screen: Query<
        &mut Node,
        (
            With<Screen>,
            Without<Device>,
            Without<MapAvoid>,
            Without<crate::hud::ProfilePanel>,
        ),
    >,
    show: Res<ShowMap>,
    hud: Option<Res<ShowHud>>,
    view: Option<Res<RideView>>,
    mut info: ResMut<NaviMapInfo>,
    mut rects: ResMut<MapAvoidRects>,
) {
    let Ok(window) = windows.single() else {
        return;
    };
    let scale = ui_scale.map_or(1.0, |s| s.0);
    let win = window_ui_px(window, scale);
    let ui_rect = |c: &ComputedNode, t: &UiGlobalTransform| {
        let k = c.inverse_scale_factor();
        let centre = t.translation * k;
        let half = c.size() * k * 0.5;
        Rect::from_corners(centre - half, centre + half)
    };
    let shown = |n: &Node, c: &ComputedNode| n.display != Display::None && c.size().x > 0.0;
    // The profile query reads Node too: disjoint from the device's by
    // its marker.
    let above: Vec<Rect> = avoid
        .iter()
        .filter(|(c, _, v, n)| v.get() && shown(n, c))
        .map(|(c, t, _, _)| ui_rect(c, t))
        .collect();
    rects.set_if_neq(MapAvoidRects(above.clone()));
    let left = profile
        .iter()
        .find(|(c, _, n)| shown(n, c))
        .map(|(c, t, _)| ui_rect(c, t));
    let want = show.0 && hud.is_none_or(|h| h.0) && view.as_deref().is_some_and(map_wanted);
    let fit = want
        .then(|| fit_map(win, fan_rect(win), left, &above))
        .flatten();
    let Ok(mut d) = device.single_mut() else {
        return;
    };
    match fit {
        Some(f) => {
            if d.display != Display::Flex {
                d.display = Display::Flex;
            }
            let bottom = px(win.y - f.rect.max.y);
            if d.bottom != bottom {
                d.bottom = bottom;
            }
            if let Ok(mut s) = screen.single_mut() {
                let (w, h) = (px(f.map.x), px(f.map.y));
                if s.width != w || s.height != h {
                    s.width = w;
                    s.height = h;
                }
            }
            let r = [
                f.rect.min.x * scale,
                f.rect.min.y * scale,
                f.rect.width() * scale,
                f.rect.height() * scale,
            ];
            if info.rect != Some(r) {
                info.rect = Some(r);
            }
            if info.map != [f.map.x, f.map.y] {
                info.map = [f.map.x, f.map.y];
            }
        }
        None => {
            if d.display != Display::None {
                d.display = Display::None;
            }
            if info.rect.is_some() {
                info.rect = None;
            }
        }
    }
}

// ---------------------------------------------------------------------
// The cache and the frame's update
// ---------------------------------------------------------------------

/// What a ring texture holds.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Look {
    level: usize,
    shade: Shade,
    /// Texels per map point × 4 (1, 1.25, 1.5).
    density4: u32,
    n: i32,
    generation: u64,
}

impl Look {
    fn density(&self) -> f64 {
        f64::from(self.density4) / 4.0
    }

    fn m_per_texel(&self) -> f64 {
        ZOOM_LEVELS[self.level] / self.density()
    }
}

struct Buffer {
    look: Look,
    ring: Ring,
    image: Handle<Image>,
}

type SourceKey = (usize, u64, u64);

#[derive(Default)]
struct Cache {
    source: Option<(Option<SourceKey>, Arc<MapSource>)>,
    generation: u64,
    front: Option<Buffer>,
    back: Option<Buffer>,
    route_ring: Option<Ring>,
    route_image: Option<Handle<Image>>,
    route_key: Option<(u64, i64, Shade)>,
    route_lines: Vec<MapLine>,
    route_dirty: bool,
    zoom: AutoZoom,
    daylight: Daylight,
    heading: Option<f64>,
    /// The arrow's place (share of the map's height), eased between
    /// riding (0.7) and flight (0.5).
    anchor_v: Option<f32>,
    mpp: f64,
    turn_key: Option<i32>,
    arrow_shade: Option<Shade>,
    /// The place names shown last frame (indices into the labels): the
    /// hysteresis of their layout.
    labels_shown: std::collections::BTreeSet<u32>,
}

/// The map tiles' time budget per frame (s); at least one tile is drawn
/// when one the map shows is missing.
const BUDGET_S: f64 = 0.0025;

fn source_of(world: Option<&crate::package::PackageWorld>) -> (Option<SourceKey>, Arc<MapSource>) {
    match world {
        Some(w) => {
            let set = &w.set;
            let key = (
                Arc::as_ptr(set) as usize,
                set.generation(),
                set.tiles_version(),
            );
            let packages = set
                .members()
                .iter()
                .filter_map(|m| {
                    m.package().map(|p| SourcePackage {
                        pkg: p.clone(),
                        frame: m.frame,
                    })
                })
                .collect();
            (Some(key), Arc::new(MapSource::new(packages, set.graph())))
        }
        None => (None, Arc::new(MapSource::default())),
    }
}

/// Whether the map is shown for this view: with a local rider – riding,
/// and in Unicorn mode's flight too (spec 3.2 Q7, decided 2026-11-19:
/// "show the map centred on the unicorn"); not in the studio (no rider).
pub fn map_wanted(view: &RideView) -> bool {
    view.local().is_some()
}

/// Where the map is and which way it looks.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MapPose {
    /// The map's centre point: the rider or the unicorn (x, z).
    pub at: [f64; 2],
    /// The map's up (compass bearing, rad) it turns to: the road's
    /// direction while riding, north (0) in flight.
    pub up: f64,
    /// The arrow's bearing (rad): the rider's or the unicorn's heading.
    pub arrow: f64,
    /// Where the arrow sits (share of the map's size).
    pub anchor: [f32; 2],
    pub flight: bool,
}

/// The map's pose: heading-up on the local rider (lower third), or in
/// Unicorn flight north-up, centred on the unicorn (spec 3.2 Q7: the
/// unicorn banks, circles and looks around freely, so a heading-up map
/// would spin with every turn; north-up reads like a map of the land
/// below, and the arrow shows where it flies).
pub fn map_pose(view: &RideView) -> Option<MapPose> {
    if let Some(f) = &view.flight {
        // The flight's yaw is counter-clockwise from north, a bearing
        // clockwise (`ocw_sim::god`: forward = (−sin yaw, −cos yaw)).
        return Some(MapPose {
            at: [f.pos[0], f.pos[2]],
            up: 0.0,
            arrow: -f.yaw,
            anchor: ocw_map::view::FLIGHT_ANCHOR,
            flight: true,
        });
    }
    let local = view.local()?;
    let track = view.track(local.id);
    let s = track.sample(local.state.distance);
    let p = s.offset_position(local.state.lateral_offset);
    let road = bearing(s.direction);
    Some(MapPose {
        at: [p[0], p[2]],
        up: road,
        arrow: road + local.state.heading,
        anchor: ocw_map::view::RIDER_ANCHOR,
        flight: false,
    })
}

/// The route layer's lines now and their key (rebuilt when it changes).
/// The route layer's key: it changes when the lines must be drawn again
/// (every 400 m, a new path or junction choice); cheap, every frame.
fn route_key(view: &RideView) -> Option<u64> {
    let local = view.local()?;
    let bucket = (local.state.distance / 400.0).floor() as i64;
    let mut key = bucket as u64;
    let mut mix = |v: u64| key = key.wrapping_mul(0x100_0000_01b3) ^ v;
    if let Some(path) = view.paths.get(&local.id) {
        mix(Arc::as_ptr(path) as usize as u64);
        mix(path.next_index());
        mix(path.end_m().to_bits());
        if let Some(j) = &view.junction {
            mix(j.selected as u64 + 1);
            mix(u64::from(j.locked));
            mix(j.exits.len() as u64);
        }
    } else {
        mix(view.route.length().to_bits());
    }
    Some(key)
}

fn route_lines(view: &RideView, ahead: &LookAhead) -> Option<(u64, RouteLines)> {
    let key = route_key(view)?;
    let local = view.local()?;
    let d = local.state.distance;
    let bucket = (d / 400.0).floor() as i64;
    let mut lines = RouteLines::default();
    if let Some(path) = view.paths.get(&local.id) {
        // Free ride: the path behind, the preview ahead (the selected
        // exit, the defaults after it), the next junction's other exits.
        let from = bucket as f64 * 400.0;
        lines.behind = track_line(path.as_ref(), from - 1600.0, d + 1.0, 8.0);
        if let Some(j) = &view.junction {
            let at = d + j.distance_m;
            for i in 0..j.exits.len() {
                if i == j.selected {
                    continue;
                }
                if let Some(p) = path.preview(d, j.distance_m.max(0.0) + 220.0, Some(i)) {
                    lines.exits.push(track_line(&p, at, at + 200.0, 6.0));
                }
            }
        }
        let track = ahead.track(view, local.id);
        lines.ahead = track_line(track, d - 1.0, from + 2400.0, 8.0);
    } else {
        // A route: the whole route around the rider.
        let from = bucket as f64 * 400.0;
        lines.behind = track_line(&view.route, from - 1600.0, d + 1.0, 8.0);
        lines.ahead = track_line(&view.route, d - 1.0, from + 4000.0, 8.0);
    }
    Some((key, lines))
}

/// The density (texels per map point × 4) for the window's pixels.
fn density4(window: Option<&Window>, ui_scale: f32) -> u32 {
    let k = window.map_or(1.0, |w| w.scale_factor()) * ui_scale;
    ((k * 4.0).round() as u32).clamp(4, 6)
}

#[allow(clippy::too_many_arguments)]
#[allow(clippy::type_complexity)] // a Bevy system: disjoint UI queries
fn update_map(
    (time, units, zoom_mode, mut labels, ui_scale): (
        Res<Time>,
        Option<Res<Units>>,
        Res<MapZoom>,
        ResMut<NaviMapLabels>,
        Option<Res<UiScale>>,
    ),
    view: Option<Res<RideView>>,
    world: Option<Res<crate::package::PackageWorld>>,
    ahead: Res<LookAhead>,
    windows: Query<&Window, With<bevy::window::PrimaryWindow>>,
    mut info: ResMut<NaviMapInfo>,
    mut images: ResMut<Assets<Image>>,
    mut materials: ResMut<Assets<NaviMapMaterial>>,
    mut cache: Local<Cache>,
    screen: Query<&MaterialNode<NaviMapMaterial>, With<Screen>>,
    mut parts: ParamSet<(
        Query<(&mut Node, &mut UiTransform), With<RiderArrow>>,
        Query<(&Dot, &mut Node, &mut BackgroundColor)>,
        Query<(
            &Label,
            &mut Node,
            &mut Text,
            &mut TextFont,
            &mut TextColor,
            &mut BackgroundColor,
        )>,
        Query<&mut UiTransform, With<Compass>>,
        Query<&mut Node, With<ScaleBar>>,
    )>,
    mut texts: ParamSet<(
        Query<&mut Text, (With<BannerDistance>, Without<Label>)>,
        Query<&mut Text, (With<BannerStreet>, Without<Label>)>,
        Query<&mut Text, (With<ScaleText>, Without<Label>)>,
        Query<&mut Text, (With<ZoomText>, Without<Label>)>,
    )>,
    (mut banner_bg, banner_icon, arrow_image, mut scale_ink): (
        Query<&mut BackgroundColor, (With<BannerBox>, Without<Dot>, Without<Label>)>,
        Query<(&ImageNode, &ChildOf), With<BannerIcon>>,
        Query<&ImageNode, With<RiderArrow>>,
        Query<&mut TextColor, (With<ScaleText>, Without<Label>)>,
    ),
) {
    let Some(view) = view else { return };
    if info.rect.is_none() {
        return;
    }
    let Ok(mat_node) = screen.single() else {
        return;
    };
    let cache = &mut *cache;
    let Some(pose) = map_pose(&view) else {
        return;
    };
    let (rider, rider_bearing) = (pose.at, pose.arrow);
    let Some(local) = view.local() else { return };
    let dt = time.delta_secs_f64();
    let size = Vec2::new(info.map[0].max(1.0), info.map[1].max(1.0));
    let imperial = units.is_some_and(|u| *u == Units::Imperial);
    // The source (the world set's loaded packages).
    let key = cache.source.as_ref().and_then(|s| s.0);
    let now_key = world.as_ref().map(|w| {
        let s = &w.set;
        (Arc::as_ptr(s) as usize, s.generation(), s.tiles_version())
    });
    if cache.source.is_none() || key != now_key {
        cache.source = Some(source_of(world.as_deref()));
        cache.generation += 1;
        // The place names of the loaded packages.
        let names = cache
            .source
            .as_ref()
            .map(|s| labels_of(&s.1))
            .unwrap_or_default();
        if labels.0 != names {
            labels.0 = names;
        }
        cache.labels_shown.clear();
    }
    let src = cache
        .source
        .as_ref()
        .map(|s| s.1.clone())
        .expect("set above");
    // Heading, zoom, look.
    let heading = match cache.heading {
        Some(h) => smooth_heading(h, pose.up, dt, 0.45),
        None => pose.up,
    };
    cache.heading = Some(heading);
    let anchor_v = match cache.anchor_v {
        Some(a) => {
            let k = 1.0 - (-(dt as f32) / 0.35).exp();
            let v = a + (pose.anchor[1] - a) * k;
            if (v - pose.anchor[1]).abs() < 1e-3 {
                pose.anchor[1]
            } else {
                v
            }
        }
        None => pose.anchor[1],
    };
    cache.anchor_v = Some(anchor_v);
    let anchor_uv = [pose.anchor[0], anchor_v];
    let ahead_px = f64::from(size.y) * f64::from(anchor_v);
    let junction_m = view.junction.as_ref().map(|j| j.distance_m);
    let auto_level = match &view.flight {
        Some(f) => cache
            .zoom
            .update_flight(f.speed_mps, f.height_m, ahead_px, dt),
        None => cache
            .zoom
            .update(local.state.speed, junction_m, ahead_px, dt),
    };
    let level = match zoom_mode.0 {
        ZoomMode::Auto => auto_level,
        ZoomMode::Fixed(i) => i.min(ZOOM_LEVELS.len() - 1),
    };
    let target_mpp = ZOOM_LEVELS[level];
    cache.mpp = if cache.mpp > 0.0 {
        ease_zoom(cache.mpp, target_mpp, dt, 0.35)
    } else {
        target_mpp
    };
    let shade = cache
        .daylight
        .update(view.sky.as_ref().map(|s| s.sky.sun.elevation_deg));
    let pal = Palette::of(shade);
    let nview = NaviView {
        rider,
        heading,
        m_per_px: cache.mpp,
        size: [size.x, size.y],
        anchor_uv,
    };
    let d4 = density4(
        windows.single().ok(),
        ui_scale.as_ref().map_or(1.0, |s| s.0),
    );
    let reach_pts = f64::from(nview.reach_px());
    let n = slots_for(reach_pts * f64::from(d4) / 4.0 * 1.25);
    let want = Look {
        level,
        shade,
        density4: d4,
        n,
        generation: cache.generation,
    };
    // A new look: build it in the back buffer (or at once, the first time).
    let new_buffer = |images: &mut Assets<Image>, look: Look| Buffer {
        look,
        ring: Ring::new(look.n, look.m_per_texel()),
        image: images.add(ring_image(look.n as u32 * TILE_PX)),
    };
    if cache.front.is_none() {
        cache.front = Some(new_buffer(&mut images, want));
        cache.route_dirty = true;
    } else if cache.front.as_ref().is_some_and(|f| f.look != want)
        && cache.back.as_ref().is_none_or(|b| b.look != want)
    {
        cache.back = Some(new_buffer(&mut images, want));
    }
    // Draw the missing tiles: the back buffer's first (it replaces the
    // front), within the budget; the visible ones of the front always.
    let start = Instant::now();
    let mut drawn = 0u64;
    let reach_for = |b: &Buffer, mpp: f64| reach_pts * mpp / b.look.m_per_texel();
    let mpp = cache.mpp;
    for which in [1usize, 0] {
        let buf = if which == 1 {
            cache.back.as_mut()
        } else {
            cache.front.as_mut()
        };
        let Some(buf) = buf else { continue };
        let stale = buf.ring.follow(rider);
        let missing = buf.ring.missing(rider);
        if stale.is_empty() && missing.is_empty() {
            continue;
        }
        let Some(mut img) = images.get_mut(&buf.image) else {
            continue;
        };
        let side = buf.ring.size_texels();
        let Some(data) = img.data.as_mut() else {
            continue;
        };
        for s in stale {
            write_slot(data, side, &buf.ring, s, None);
        }
        let visible = reach_for(buf, mpp) / f64::from(TILE_PX) + 1.0;
        let rt = buf.ring.tile_at(rider);
        for k in missing {
            let near = f64::from((k.0 - rt.0).abs().max((k.1 - rt.1).abs())) <= visible;
            let over = start.elapsed().as_secs_f64() > BUDGET_S;
            if over && !(which == 0 && near) {
                break;
            }
            let spec = TileSpec {
                ix: k.0,
                iz: k.1,
                m_per_texel: buf.look.m_per_texel(),
                density: buf.look.density(),
            };
            let tile = render_base_tile(&src, &Palette::of(buf.look.shade), &spec);
            let slot = buf.ring.slot(k);
            write_slot(data, side, &buf.ring, slot, Some(&tile));
            buf.ring.mark(k);
            drawn += 1;
        }
    }
    info.tiles_drawn += drawn;
    // Swap when the back buffer has what the map shows.
    if let Some(b) = &cache.back
        && b.ring.complete_within(rider, reach_for(b, mpp) + 2.0)
    {
        cache.front = cache.back.take();
        cache.route_dirty = true;
    }
    let Some(front) = cache.front.as_ref() else {
        return;
    };
    let flook = front.look;
    let fring = front.ring.clone();
    let fimage = front.image.clone();
    // The route layer: same ring as the front.
    let rkey = route_key(&view).map(|k| (k, flook.level as i64, flook.shade));
    if rkey.is_some()
        && cache.route_key != rkey
        && let Some((_, lines)) = route_lines(&view, &ahead)
    {
        cache.route_key = rkey;
        cache.route_lines = lines.lines(&Palette::of(flook.shade));
        cache.route_dirty = true;
    }
    let route_side = fring.size_texels();
    let route_ok = cache
        .route_image
        .as_ref()
        .and_then(|h| images.get(h))
        .is_some_and(|i| i.width() == route_side);
    if !route_ok {
        cache.route_image = Some(images.add(ring_image(route_side)));
        cache.route_dirty = true;
    }
    if cache.route_dirty || cache.route_ring.as_ref().is_none_or(|r| r.n != fring.n) {
        cache.route_ring = Some(Ring::new(fring.n, fring.m_per_texel));
        cache.route_dirty = false;
        // A full redraw: every slot cleared first.
        if let Some(h) = &cache.route_image
            && let Some(mut img) = images.get_mut(h)
            && let Some(d) = img.data.as_mut()
        {
            d.fill(0);
        }
    }
    if let (Some(rring), Some(h)) = (cache.route_ring.as_mut(), cache.route_image.clone()) {
        rring.m_per_texel = fring.m_per_texel;
        let stale = rring.follow(rider);
        let missing = rring.missing(rider);
        if (!stale.is_empty() || !missing.is_empty())
            && let Some(mut img) = images.get_mut(&h)
            && let Some(data) = img.data.as_mut()
        {
            for s in stale {
                write_slot(data, route_side, rring, s, None);
            }
            for k in missing {
                let spec = TileSpec {
                    ix: k.0,
                    iz: k.1,
                    m_per_texel: flook.m_per_texel(),
                    density: flook.density(),
                };
                let tile = render_line_tile(&cache.route_lines, &spec);
                let slot = rring.slot(k);
                write_slot(data, route_side, rring, slot, tile.as_deref());
                rring.mark(k);
            }
        }
    }
    // The material: the front texture turned heading-up.
    let mt = flook.m_per_texel();
    let (ax, ay) = nview.texel_axes(mt);
    let ring_size = f64::from(fring.size_texels());
    let rt = [rider[0] / mt, rider[1] / mt];
    let (lo, hi) = fring
        .window_texels()
        .unwrap_or(([rt[0], rt[1]], [rt[0], rt[1]]));
    let g = pal.ground;
    let ground = Color::srgb_u8(g[0], g[1], g[2]).to_linear();
    let params = NaviMapParams {
        ring: Vec4::new(
            rt[0].rem_euclid(ring_size) as f32,
            rt[1].rem_euclid(ring_size) as f32,
            ring_size as f32,
            1.0,
        ),
        axes: Vec4::new(ax[0] as f32, ax[1] as f32, ay[0] as f32, ay[1] as f32),
        anchor: Vec4::new(
            anchor_uv[0],
            anchor_uv[1],
            (lo[0] - rt[0]) as f32,
            (lo[1] - rt[1]) as f32,
        ),
        window: Vec4::new((hi[0] - rt[0]) as f32, (hi[1] - rt[1]) as f32, 0.0, 0.0),
        ground: Vec4::new(ground.red, ground.green, ground.blue, 1.0),
    };
    if let Some(mut m) = materials.get_mut(&mat_node.0) {
        m.params = params;
        if m.base != fimage {
            m.base = fimage;
        }
        if let Some(r) = &cache.route_image
            && m.route != *r
        {
            m.route = r.clone();
        }
    }
    // The rider's arrow at the anchor, turned by the rider's own heading
    // against the map's.
    let a = nview.anchor();
    if let Ok((mut node, mut tf)) = parts.p0().single_mut() {
        let (l, t) = (px(a[0] - ARROW_PX / 2.0), px(a[1] - ARROW_PX / 2.0));
        if node.left != l || node.top != t {
            node.left = l;
            node.top = t;
        }
        let rot = Rot2::radians((rider_bearing - heading) as f32);
        if tf.rotation != rot {
            tf.rotation = rot;
        }
    }
    if cache.arrow_shade != Some(shade)
        && let Ok(img) = arrow_image.single()
        && let Some(mut i) = images.get_mut(&img.image)
    {
        cache.arrow_shade = Some(shade);
        i.data = Some(rider_arrow(ICON_PX, [255, 120, 30, 255]));
    }
    // Other riders as dots.
    let others: Vec<[f32; 2]> = view
        .riders
        .iter()
        .filter(|r| !r.is_local)
        .filter_map(|r| {
            let s = view.track(r.id).sample(r.state.distance);
            let p = s.offset_position(r.state.lateral_offset);
            let sp = nview.to_screen([p[0], p[2]]);
            nview.inside(sp, 5.0, 9.0).then_some(sp)
        })
        .take(DOTS)
        .collect();
    let dot = pal.rider_dot;
    let dot_colour = Color::srgb_u8(dot[0], dot[1], dot[2]);
    for (Dot(i), mut node, mut bg) in &mut parts.p1() {
        match others.get(*i) {
            Some(s) => {
                node.display = Display::Flex;
                node.left = px(s[0] - 5.0);
                node.top = px(s[1] - 5.0);
                if bg.0 != dot_colour {
                    bg.0 = dot_colour;
                }
            }
            None => {
                if node.display != Display::None {
                    node.display = Display::None;
                }
            }
        }
    }
    // Place names: by zoom, decluttered, clear of the map's marks.
    let marks = navi_marks(nview.size, nview.anchor());
    let shown = navi_labels(&labels.0, &nview, &marks, 6.0, &cache.labels_shown, LABELS);
    cache.labels_shown = shown.iter().map(|l| l.index as u32).collect();
    let ink = if shade == Shade::Night {
        Color::srgb(0.93, 0.94, 0.97)
    } else {
        Color::srgb(0.12, 0.12, 0.15)
    };
    // The pill behind a name: the map's ground colour, translucent.
    let g = pal.ground;
    let pill = Color::srgba_u8(g[0], g[1], g[2], 205);
    for (Label(i), mut node, mut text, mut font, mut colour, mut bg) in &mut parts.p2() {
        match shown.get(*i) {
            Some(l) => {
                let name = &labels.0[l.index].name;
                let (size, _) = navi_label_font(labels.0[l.index].rank);
                // The box the layout placed (moved into the map when the
                // name is near its edge), not one centred on the place.
                node.display = Display::Flex;
                node.left = px(l.min[0]);
                node.width = px(l.max[0] - l.min[0]);
                node.top = px(l.min[1]);
                // The size follows the name's rank: set with the name.
                if text.0 != *name {
                    text.0.clone_from(name);
                    font.font_size = size.into();
                }
                if colour.0 != ink {
                    colour.0 = ink;
                }
                if bg.0 != pill {
                    bg.0 = pill;
                }
            }
            None => {
                if node.display != Display::None {
                    node.display = Display::None;
                }
            }
        }
    }
    // The compass needle points north.
    let nv = nview.north();
    if let Ok(mut tf) = parts.p3().single_mut() {
        let rot = Rot2::radians(nv[0].atan2(-nv[1]));
        if tf.rotation != rot {
            tf.rotation = rot;
        }
    }
    // The scale bar.
    let (bar_px, bar_label) = scale_bar(cache.mpp, SCALE_MAX_PX, imperial);
    if let Ok(mut node) = parts.p4().single_mut() {
        let w = px(bar_px.round() as f32);
        if node.width != w {
            node.width = w;
        }
    }
    let set = |t: &mut Text, s: &str| {
        if t.0 != s {
            t.0 = s.to_string();
        }
    };
    if let Ok(mut t) = texts.p2().single_mut() {
        set(&mut t, &bar_label);
    }
    // The scale's colour follows the map (light at night).
    let ink = if shade == Shade::Night {
        Color::srgb(0.9, 0.92, 0.95)
    } else {
        Color::srgb(0.1, 0.1, 0.12)
    };
    for mut c in &mut scale_ink {
        if c.0 != ink {
            c.0 = ink;
        }
    }
    let zoom_label = match zoom_mode.0 {
        ZoomMode::Auto => format!("AUTO {}", level + 1),
        ZoomMode::Fixed(i) => format!("ZOOM {}", i + 1),
    };
    if let Ok(mut t) = texts.p3().single_mut() {
        set(&mut t, &zoom_label);
    }
    // The banner: the next turn; in flight the height and the speed.
    let b = match &view.flight {
        Some(f) => flight_banner(f, imperial),
        None => make_banner(&view, imperial),
    };
    if let Ok(mut t) = texts.p0().single_mut() {
        set(&mut t, b.distance.as_deref().unwrap_or(""));
    }
    if let Ok(mut t) = texts.p1().single_mut() {
        set(&mut t, &b.street);
    }
    let turn_key = (b.angle_deg.unwrap_or(0.0) / 5.0).round() as i32;
    if let Ok((icon, parent)) = banner_icon.single() {
        if cache.turn_key != Some(turn_key)
            && let Some(mut i) = images.get_mut(&icon.image)
        {
            cache.turn_key = Some(turn_key);
            i.data = Some(turn_arrow(f64::from(turn_key) * 5.0, ICON_PX, [255; 4]));
        }
        if let Ok(mut bg) = banner_bg.get_mut(parent.parent()) {
            let c = if shade == Shade::Night {
                BANNER_NIGHT
            } else {
                BANNER_DAY
            };
            if bg.0 != c {
                bg.0 = c;
            }
        }
    }
    let text = b.text();
    let auto = zoom_mode.0 == ZoomMode::Auto;
    let flight = pose.flight;
    if info.banner != text
        || info.level != level + 1
        || info.auto != auto
        || info.shade != shade
        || info.flight != flight
        || info.centre != rider
    {
        info.banner = text;
        info.level = level + 1;
        info.auto = auto;
        info.shade = shade;
        info.flight = flight;
        info.centre = rider;
    }
}

/// The banner in Unicorn flight: the height above the ground and the
/// speed with the compass heading ("120 m up", "54 km/h, heading 080 E").
pub fn flight_banner(f: &crate::FlightView, imperial: bool) -> Banner {
    let units = if imperial {
        Units::Imperial
    } else {
        Units::Metric
    };
    let h = f.height_m.max(0.0);
    let height = if imperial {
        format!("{:.0} ft up", h / ocw_map::style::METRES_PER_FOOT)
    } else {
        format!("{h:.0} m up")
    };
    let (speed, unit) = units.speed(f.speed_mps * 3.6);
    let deg = ocw_sim::god::heading_deg(f.yaw);
    Banner {
        angle_deg: None,
        distance: Some(height),
        street: format!(
            "{speed:.0} {unit}, heading {deg:03.0} {}",
            crate::hud::compass(deg)
        ),
    }
}

/// The banner of the ride now: the junction ahead (the exit that will be
/// taken), else the street ridden.
pub fn make_banner(view: &RideView, imperial: bool) -> Banner {
    let street_now = view
        .local()
        .map(|l| view.track(l.id).name_at(l.state.distance))
        .unwrap_or_default();
    let next = view
        .junction
        .as_ref()
        .filter(|j| j.distance_m > -5.0)
        .and_then(|j| {
            let e = j.exits.get(j.selected)?;
            Some((j.distance_m, e.angle_deg, e.name.as_str()))
        });
    banner(next, &street_now, imperial)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ExitView, JunctionView};
    use ocw_world::Route;

    /// The supported windows: every 16:9 size from 1280 × 720 up and 4:3,
    /// with the HUD at 100 – 150 %.
    const SIZES: [(f32, f32); 9] = [
        (1280.0, 720.0),
        (1366.0, 768.0),
        (1600.0, 900.0),
        (1920.0, 1080.0),
        (2560.0, 1440.0),
        (3840.0, 2160.0),
        (1024.0, 768.0),
        (1280.0, 960.0),
        (1600.0, 1200.0),
    ];

    /// The other HUD elements in UI px for a window of `w` × `h` UI px:
    /// the elevation profile with the GPS line (bottom left), the voice
    /// panel (left, above it), the top panel (its height with the longest
    /// lines, as the top panel's fit test has them) and the top-right
    /// column (time stamp, REC, FPS).
    fn hud_boxes(w: f32, h: f32) -> (Rect, Rect, Vec<Rect>) {
        // Profile: 300 px chart + padding, at most 60 vw; GPS line under it.
        let pw = (300.0f32 + 16.0).min(0.6 * w);
        let profile = Rect::new(12.0, h - 12.0 - 56.0 - 30.0 - 24.0, 12.0 + pw, h - 12.0);
        // The voice HUD: bottom 120, left 12, at most 420 wide.
        let voice = Rect::new(
            12.0,
            h - 120.0 - 140.0,
            12.0 + 420.0f32.min(w - 24.0),
            h - 120.0,
        );
        // The top panel: the left part, its tall case (8 lines of text).
        let (top_w, top_h) = crate::hud::tests_support::tall_top_panel(w);
        let top = Rect::new(12.0, 12.0, 12.0 + top_w, top_h);
        // The top-right column: stamp, REC, FPS (about 110 px).
        let column = Rect::new(w - 12.0 - 230.0, 12.0, w - 12.0, 12.0 + 110.0);
        (profile, voice, vec![top, column])
    }

    /// Regression (2026-11-18): the place names were drawn under the other
    /// riders' dots ("Vadu" covered). The map screen's children stack
    /// dots < names < the rider's arrow and the marks, and every name has
    /// its backing pill.
    #[test]
    fn names_lie_over_the_dots_and_under_the_arrow() {
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, bevy::asset::AssetPlugin::default()))
            .init_asset::<Image>()
            .init_asset::<NaviMapMaterial>()
            .add_systems(Startup, spawn_device);
        app.update();
        let world = app.world_mut();
        let z = |world: &mut World, f: &dyn Fn(&EntityRef) -> bool| -> Vec<i32> {
            let mut q = world.query::<(EntityRef, &ZIndex)>();
            q.iter(world)
                .filter(|(e, _)| f(e))
                .map(|(_, z)| z.0)
                .collect()
        };
        let dots = z(world, &|e| e.contains::<Dot>());
        let names = z(world, &|e| e.contains::<Label>());
        let arrow = z(world, &|e| e.contains::<RiderArrow>());
        let compass = z(world, &|e| e.contains::<Compass>());
        assert_eq!((dots.len(), names.len(), arrow.len()), (DOTS, LABELS, 1));
        let top_dot = dots.iter().max().unwrap();
        assert!(names.iter().all(|n| n > top_dot && *n < arrow[0]));
        assert!(compass.iter().all(|c| *c > names[0]));
        // The names sit in the same parent as the dots (ZIndex compares
        // siblings) and have a backing.
        let mut q = world.query_filtered::<(&ChildOf, &BackgroundColor), With<Label>>();
        let parents: Vec<Entity> = q.iter(world).map(|(p, _)| p.parent()).collect();
        let mut d = world.query_filtered::<&ChildOf, With<Dot>>();
        let dot_parent = d.iter(world).next().unwrap().parent();
        assert!(parents.iter().all(|p| *p == dot_parent));
        let mut b = world.query_filtered::<&BackgroundColor, With<Label>>();
        assert!(b.iter(world).all(|c| c.0.alpha() > 0.5));
    }

    /// The map never overlaps the elevation profile, the junction fan, the
    /// voice panel, the top panel or the top-right column, stays in the
    /// lower right corner on screen, and has its preferred size on large
    /// windows – at every supported size and HUD scale.
    #[test]
    fn the_map_fits_every_window_beside_the_other_hud_elements() {
        for (pw, ph) in SIZES {
            for scale in [1.0f32, 1.25, 1.5] {
                let win = Vec2::new(pw, ph) / scale;
                let fan = fan_rect(win);
                let (profile, voice, above) = hud_boxes(win.x, win.y);
                let what = format!("{pw}×{ph} at {scale}");
                let fit = fit_map(win, fan, Some(profile), &above)
                    .unwrap_or_else(|| panic!("{what}: room for the map"));
                let r = fit.rect;
                for (name, o) in [("fan", fan), ("profile", profile), ("voice", voice)]
                    .into_iter()
                    .chain(above.iter().map(|a| ("top", *a)))
                {
                    assert!(
                        r.intersect(o).is_empty(),
                        "{what}: overlaps the {name}: {r:?} {o:?}"
                    );
                }
                assert!(r.min.x >= 0.0 && r.min.y >= 0.0, "{what}");
                assert!((r.max.x - (win.x - RIGHT_PX)).abs() < 0.5, "{what}: right");
                assert!(
                    (r.max.y - (win.y - BOTTOM_PX)).abs() < 0.5,
                    "{what}: bottom"
                );
                assert!(fit.map.x >= MAP_MIN_W, "{what}");
                assert!((fit.map.y - (fit.map.x * MAP_ASPECT).floor()).abs() < 0.5);
                if pw >= 1920.0 && scale <= 1.25 {
                    assert_eq!(fit.map.x, MAP_W, "{what}: full size");
                }
            }
        }
        // No room: hidden rather than overlapping.
        let win = Vec2::new(400.0, 300.0);
        assert!(fit_map(win, fan_rect(win), None, &[]).is_none());
    }

    fn local_rider(distance: f64) -> crate::RiderView {
        crate::RiderView {
            id: ocw_core::RiderId(1),
            name: "Ada Rider".into(),
            state: ocw_sim::RiderState {
                distance,
                ..default()
            },
            power_w: 0.0,
            cadence_rpm: None,
            heart_rate_bpm: None,
            is_local: true,
            bike: Default::default(),
            look: None,
            parts: Vec::new(),
            crash: None,
            wobble: 0.0,
            damage: None,
            cue: Default::default(),
            signs: Default::default(),
        }
    }

    fn junction_view(distance_m: f64, selected: usize) -> JunctionView {
        let exit = |angle_deg: f64, name: &str| ExitView {
            angle_deg,
            name: name.into(),
            dead_end: false,
            unpaved: false,
            steep: false,
        };
        JunctionView {
            distance_m,
            exits: vec![
                exit(-80.0, "Kirchweg"),
                exit(5.0, "Landstrasse"),
                exit(88.0, "Bahnhofstrasse"),
            ],
            selected,
            default: 1,
            locked: false,
            route: None,
        }
    }

    /// The banner shows the exit that will be taken with its distance and
    /// street; without a junction the street ridden now.
    #[test]
    fn the_banner_follows_the_junction_ahead() {
        let mut view = RideView::new(Route::test_loop());
        view.riders.push(local_rider(300.0));
        view.junction = Some(junction_view(247.0, 2));
        assert_eq!(make_banner(&view, false).text(), "↱ 250 m Bahnhofstrasse");
        view.junction = Some(junction_view(64.0, 0));
        assert_eq!(make_banner(&view, false).text(), "↰ 65 m Kirchweg");
        assert_eq!(make_banner(&view, true).distance.as_deref(), Some("200 ft"));
        // Passed: the street ridden.
        view.junction = Some(junction_view(-12.0, 0));
        assert_eq!(make_banner(&view, false).distance, None);
        view.junction = None;
        assert_eq!(make_banner(&view, false).angle_deg, None);
    }

    /// Spec 3.2 Q7 (2026-11-19: "show the map centred on the unicorn"):
    /// in Unicorn flight the map is shown, north-up, centred on the
    /// unicorn with its arrow turned to where it flies; riding, it is
    /// heading-up on the rider in the lower third. Without a local rider
    /// (the studio) it stays hidden.
    #[test]
    fn the_map_shows_in_flight_centred_on_the_unicorn() {
        let mut view = RideView::new(Route::test_loop());
        assert!(!map_wanted(&view), "no rider");
        view.riders.push(local_rider(300.0));
        assert!(map_wanted(&view));
        let ride = map_pose(&view).unwrap();
        assert!(!ride.flight);
        assert_eq!(ride.anchor, ocw_map::view::RIDER_ANCHOR);
        let road = bearing(view.route.sample(300.0).direction);
        assert!((ride.up - road).abs() < 1e-9);
        view.flight = Some(crate::FlightView {
            pos: [120.0, 640.0, -45.0],
            yaw: std::f64::consts::FRAC_PI_2, // facing west
            speed_mps: 15.0,
            height_m: 90.0,
            ..default()
        });
        assert!(map_wanted(&view), "shown in flight");
        let fly = map_pose(&view).unwrap();
        assert!(fly.flight);
        assert_eq!(fly.at, [120.0, -45.0], "centred on the unicorn");
        assert_eq!(fly.anchor, ocw_map::view::FLIGHT_ANCHOR);
        assert_eq!(fly.up, 0.0, "north-up");
        // West is a bearing of 270°: the arrow points left on the map.
        let west = ocw_map::view::direction(fly.arrow);
        assert!(
            (west[0] + 1.0).abs() < 1e-9 && west[1].abs() < 1e-9,
            "{west:?}"
        );
        // The projection: the unicorn in the middle of a 240 × 264 map.
        let v = NaviView {
            rider: fly.at,
            heading: fly.up,
            m_per_px: 2.0,
            size: [240.0, 264.0],
            anchor_uv: fly.anchor,
        };
        assert_eq!(v.to_screen(fly.at), [120.0, 132.0]);
        let b = flight_banner(view.flight.as_ref().unwrap(), false);
        assert_eq!(b.distance.as_deref(), Some("90 m up"));
        assert_eq!(b.street, "54 km/h, heading 270 W");
        assert_eq!(
            flight_banner(view.flight.as_ref().unwrap(), true)
                .distance
                .as_deref(),
            Some("295 ft up")
        );
    }

    /// The route layer: the route around the rider ahead and behind; free
    /// ride adds the other exits of the next junction.
    #[test]
    fn route_lines_follow_the_ride() {
        let mut view = RideView::new(Route::test_loop());
        view.riders.push(local_rider(300.0));
        let ahead = LookAhead::default();
        let (k1, lines) = route_lines(&view, &ahead).unwrap();
        assert!(lines.ahead.len() >= 2 && lines.behind.len() >= 2);
        let p = view.route.sample(300.0).position;
        let first = lines.ahead[0];
        let near =
            |a: [f64; 2], b: [f64; 3]| ((a[0] - b[0]).powi(2) + (a[1] - b[2]).powi(2)).sqrt() < 2.0;
        assert!(near(first, p));
        assert!(lines.exits.is_empty(), "a route has no exits drawn");
        // The same 400 m: the same key; farther: a new one.
        view.riders[0].state.distance = 350.0;
        assert_eq!(route_lines(&view, &ahead).unwrap().0, k1);
        view.riders[0].state.distance = 900.0;
        assert_ne!(route_lines(&view, &ahead).unwrap().0, k1);
    }

    /// The tile cache writes tiles into their slots and clears stale ones.
    #[test]
    fn tiles_land_in_their_slots() {
        let ring = Ring::new(4, 1.0);
        let side = ring.size_texels();
        let mut data = vec![0u8; (side * side * 4) as usize];
        let tile = vec![7u8; (TILE_PX * TILE_PX * 4) as usize];
        let slot = ring.slot((5, -2));
        write_slot(&mut data, side, &ring, slot, Some(&tile));
        let (ox, oy) = ring.slot_origin(slot);
        let at = |x: u32, y: u32| data[((y * side + x) * 4) as usize];
        assert_eq!(at(ox, oy), 7);
        assert_eq!(at(ox + TILE_PX - 1, oy + TILE_PX - 1), 7);
        assert_eq!(at((ox + TILE_PX) % side, oy), 0);
        write_slot(&mut data, side, &ring, slot, None);
        assert_eq!(data[((oy * side + ox) * 4) as usize], 0);
        assert_eq!(density4(None, 1.0), 4);
        assert_eq!(density4(None, 3.0), 6, "capped at 1.5 texels per point");
    }
}
