//! Windows resources for the OCW executables (spec §11.3 *Windows icon and
//! version info*): a logo of `artwork/logos/` as a multi-size icon
//! (16, 24, 32, 48, 64, 128 and 256 px; the game's [`OCW`] or the
//! editor's [`OCW_STUDIO`]) and a version block (product
//! "Open Cycling World", file and product version from Cargo).
//!
//! A build-script helper: an app's `build.rs` calls [`embed`], which on an
//! MSVC Windows target writes a compiled resource file (`.res`) into
//! `OUT_DIR` and hands it to the linker (`link.exe` and `lld-link` take
//! `.res` files directly, so neither `rc.exe` nor a resource-compiler crate
//! is needed). On every other target it does nothing, so Linux, macOS and
//! the wasm32 builds are unchanged. Explorer, the taskbar and shortcuts to
//! the exe (the VR kiosk's "OCW VR Kiosk – Start / Stop") show this icon.
//!
//! The icon is made at build time from the committed PNGs (no binary
//! `.ico` in git, reproducible): 24 px, which `artwork/logos/` has no file
//! for, is the 48 px PNG halved (2×2 box filter on premultiplied alpha).
//! Sizes below 256 px are stored as 32-bit DIBs (every Windows version
//! reads them), 256 px as the PNG itself (Vista and later). The licence of
//! the icon is the logo's.

use std::io::Cursor;

/// Resource id of the icon group: the app's first icon, which Explorer and
/// shortcuts show; `winit`'s `Icon::from_resource(1, …)` loads it.
pub const ICON_ID: u16 = 1;

/// The icon sizes in the `.ico` and the resource, smallest first.
pub const ICON_SIZES: [u32; 7] = [16, 24, 32, 48, 64, 128, 256];

/// One logo of `artwork/logos/`: its PNG renders by size.
#[derive(Clone, Copy, Debug)]
pub struct Logo {
    /// The file stem, e.g. `ocw` (`ocw-16.png`, …).
    pub name: &'static str,
    pub pngs: [(u32, &'static [u8]); 6],
}

/// The PNG renders of `artwork/logos/<name>-<size>.png`, 16 … 256 px.
macro_rules! logo {
    ($name:literal) => {
        Logo {
            name: $name,
            pngs: [
                logo!(@png $name, 16),
                logo!(@png $name, 32),
                logo!(@png $name, 48),
                logo!(@png $name, 64),
                logo!(@png $name, 128),
                logo!(@png $name, 256),
            ],
        }
    };
    (@png $name:literal, $size:literal) => {
        (
            $size,
            include_bytes!(concat!("../../../artwork/logos/", $name, "-", $size, ".png")),
        )
    };
}

/// The game's logo (the unicorn on a bike over a rainbow): `ocw-desktop`,
/// `ocw-server`.
pub const OCW: Logo = logo!("ocw");

/// OCW Studio's logo (the pencil drawing the rainbow): `ocw-studio`.
pub const OCW_STUDIO: Logo = logo!("ocw-studio");

/// The product name in every executable's version block.
pub const PRODUCT_NAME: &str = "Open Cycling World";

// Resource types and the language of the entries (US English, as rc.exe
// writes them by default; the version block's translation matches).
const RT_ICON: u16 = 3;
const RT_GROUP_ICON: u16 = 14;
const RT_VERSION: u16 = 16;
const LANG_EN_US: u16 = 0x0409;
const CODEPAGE_UNICODE: u16 = 1200;

/// One square image of the icon.
#[derive(Clone, Debug, PartialEq)]
pub struct IconImage {
    pub size: u32,
    /// Straight (not premultiplied) RGBA8, top row first.
    pub rgba: Vec<u8>,
    /// The PNG it was decoded from (stored as is for 256 px).
    pub png: Option<Vec<u8>>,
}

impl IconImage {
    /// The image as stored in an `.ico` or `RT_ICON` resource: a PNG for
    /// 256 px, otherwise a 32-bit DIB (BITMAPINFOHEADER, bottom-up BGRA,
    /// then the 1-bit AND mask, set where the pixel is fully transparent).
    pub fn encoded(&self) -> Vec<u8> {
        if self.size >= 256
            && let Some(png) = &self.png
        {
            return png.clone();
        }
        let s = self.size as usize;
        let mask_row = s.div_ceil(32) * 4;
        let mut out = Vec::with_capacity(40 + s * s * 4 + mask_row * s);
        out.extend_from_slice(&40u32.to_le_bytes()); // biSize
        out.extend_from_slice(&(s as i32).to_le_bytes()); // biWidth
        out.extend_from_slice(&(2 * s as i32).to_le_bytes()); // XOR + AND
        out.extend_from_slice(&1u16.to_le_bytes()); // biPlanes
        out.extend_from_slice(&32u16.to_le_bytes()); // biBitCount
        out.extend_from_slice(&0u32.to_le_bytes()); // BI_RGB
        out.extend_from_slice(&((s * s * 4 + mask_row * s) as u32).to_le_bytes());
        out.extend_from_slice(&[0; 16]); // resolution, palette
        for y in (0..s).rev() {
            for x in 0..s {
                let p = &self.rgba[(y * s + x) * 4..][..4];
                out.extend_from_slice(&[p[2], p[1], p[0], p[3]]);
            }
        }
        for y in (0..s).rev() {
            let mut row = vec![0u8; mask_row];
            for x in 0..s {
                if self.rgba[(y * s + x) * 4 + 3] == 0 {
                    row[x / 8] |= 0x80 >> (x % 8);
                }
            }
            out.extend_from_slice(&row);
        }
        out
    }
}

/// Decodes a PNG into RGBA8: (pixels, width, height).
pub fn decode_png(png: &[u8]) -> Result<(Vec<u8>, u32, u32), String> {
    let mut decoder = png::Decoder::new(Cursor::new(png));
    decoder.set_transformations(png::Transformations::EXPAND | png::Transformations::STRIP_16);
    let mut reader = decoder.read_info().map_err(|e| e.to_string())?;
    let mut buf = vec![0; reader.output_buffer_size().ok_or("PNG too large")?];
    let info = reader.next_frame(&mut buf).map_err(|e| e.to_string())?;
    buf.truncate(info.buffer_size());
    let rgba = match info.color_type {
        png::ColorType::Rgba => buf,
        png::ColorType::Rgb => buf
            .as_chunks::<3>()
            .0
            .iter()
            .flat_map(|p| [p[0], p[1], p[2], 255])
            .collect(),
        png::ColorType::GrayscaleAlpha => buf
            .as_chunks::<2>()
            .0
            .iter()
            .flat_map(|p| [p[0], p[0], p[0], p[1]])
            .collect(),
        png::ColorType::Grayscale => buf.iter().flat_map(|&g| [g, g, g, 255]).collect(),
        other => return Err(format!("unexpected PNG colour type {other:?}")),
    };
    Ok((rgba, info.width, info.height))
}

/// Halves a square RGBA8 image (2×2 box filter on premultiplied alpha).
pub fn halve(rgba: &[u8], size: u32) -> Vec<u8> {
    let (s, h) = (size as usize, size as usize / 2);
    let mut out = Vec::with_capacity(h * h * 4);
    for y in 0..h {
        for x in 0..h {
            let mut sum = [0u32; 4];
            for (dx, dy) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
                let p = &rgba[((2 * y + dy) * s + 2 * x + dx) * 4..][..4];
                let a = u32::from(p[3]);
                for (acc, &v) in sum.iter_mut().zip(&p[..3]) {
                    *acc += u32::from(v) * a;
                }
                sum[3] += a;
            }
            if sum[3] == 0 {
                out.extend_from_slice(&[0; 4]);
            } else {
                let rgb = |c: usize| ((sum[c] + sum[3] / 2) / sum[3]) as u8;
                out.extend_from_slice(&[rgb(0), rgb(1), rgb(2), ((sum[3] + 2) / 4) as u8]);
            }
        }
    }
    out
}

/// The logo in every size of [`ICON_SIZES`].
pub fn logo_images(logo: &Logo) -> Result<Vec<IconImage>, String> {
    let mut images = Vec::new();
    for (size, png) in logo.pngs {
        let (rgba, w, h) = decode_png(png)?;
        if (w, h) != (size, size) {
            return Err(format!("{}-{size}.png is {w}×{h}", logo.name));
        }
        images.push(IconImage {
            size,
            rgba,
            png: Some(png.to_vec()),
        });
    }
    let from48 = images
        .iter()
        .find(|i| i.size == 48)
        .ok_or("no 48 px logo")?;
    images.push(IconImage {
        size: 24,
        rgba: halve(&from48.rgba, 48),
        png: None,
    });
    images.sort_by_key(|i| i.size);
    Ok(images)
}

/// Width / height byte of an icon directory entry (0 means 256).
fn dim_byte(size: u32) -> u8 {
    if size >= 256 { 0 } else { size as u8 }
}

/// An `.ico` file of the images.
pub fn ico_file(images: &[IconImage]) -> Vec<u8> {
    let data: Vec<Vec<u8>> = images.iter().map(IconImage::encoded).collect();
    let mut out = Vec::new();
    out.extend_from_slice(&[0, 0, 1, 0]); // reserved, type 1 = icon
    out.extend_from_slice(&(images.len() as u16).to_le_bytes());
    let mut offset = 6 + 16 * images.len();
    for (image, bytes) in images.iter().zip(&data) {
        let d = dim_byte(image.size);
        out.extend_from_slice(&[d, d, 0, 0]);
        out.extend_from_slice(&1u16.to_le_bytes()); // planes
        out.extend_from_slice(&32u16.to_le_bytes()); // bits per pixel
        out.extend_from_slice(&(bytes.len() as u32).to_le_bytes());
        out.extend_from_slice(&(offset as u32).to_le_bytes());
        offset += bytes.len();
    }
    for bytes in data {
        out.extend_from_slice(&bytes);
    }
    out
}

/// What the version block says about one executable.
#[derive(Clone, Debug)]
pub struct AppInfo {
    /// The executable's name without `.exe`, e.g. `ocw-desktop`.
    pub name: String,
    /// The file description Explorer shows (the crate's description).
    pub description: String,
    /// `major.minor.patch` (the workspace version).
    pub version: String,
}

impl AppInfo {
    /// From the variables Cargo sets for a build script.
    pub fn from_cargo_env() -> Self {
        let var = |k: &str| std::env::var(k).unwrap_or_default();
        let bin = var("CARGO_PKG_NAME");
        AppInfo {
            name: bin,
            description: var("CARGO_PKG_DESCRIPTION"),
            version: var("CARGO_PKG_VERSION"),
        }
    }

    /// The version as the four 16-bit parts of a Windows file version.
    pub fn version_parts(&self) -> [u16; 4] {
        let mut parts = [0u16; 4];
        let numbers = self.version.split(['.', '-', '+']);
        for (part, n) in parts.iter_mut().zip(numbers) {
            *part = n.parse().unwrap_or(0);
        }
        parts
    }
}

fn utf16z(s: &str) -> Vec<u8> {
    s.encode_utf16()
        .chain([0])
        .flat_map(u16::to_le_bytes)
        .collect()
}

fn pad4(buf: &mut Vec<u8>) {
    while !buf.len().is_multiple_of(4) {
        buf.push(0);
    }
}

enum Value<'a> {
    None,
    Text(&'a str),
    Binary(&'a [u8]),
}

/// One node of a VS_VERSIONINFO tree: length, value length, type, key,
/// value, children (every part DWORD-aligned).
fn version_node(key: &str, value: Value, children: &[Vec<u8>]) -> Vec<u8> {
    let (value_bytes, value_len, kind) = match value {
        Value::None => (Vec::new(), 0, 1u16),
        Value::Text(t) => {
            let b = utf16z(t);
            let chars = (b.len() / 2) as u16;
            (b, chars, 1)
        }
        Value::Binary(b) => (b.to_vec(), b.len() as u16, 0),
    };
    let mut buf = vec![0u8; 6];
    buf.extend_from_slice(&utf16z(key));
    pad4(&mut buf);
    buf.extend_from_slice(&value_bytes);
    for child in children {
        pad4(&mut buf);
        buf.extend_from_slice(child);
    }
    let len = buf.len() as u16;
    buf[0..2].copy_from_slice(&len.to_le_bytes());
    buf[2..4].copy_from_slice(&value_len.to_le_bytes());
    buf[4..6].copy_from_slice(&kind.to_le_bytes());
    buf
}

/// The VS_VERSIONINFO block of an executable.
pub fn version_info(app: &AppInfo) -> Vec<u8> {
    let [a, b, c, d] = app.version_parts().map(u32::from);
    let (ms, ls) = ((a << 16) | b, (c << 16) | d);
    let fixed: Vec<u8> = [
        0xFEEF_04BD, // signature
        0x0001_0000, // structure version
        ms,
        ls, // file version
        ms,
        ls,          // product version
        0x3F,        // flags mask
        0,           // flags
        0x0004_0004, // VOS_NT_WINDOWS32
        1,           // VFT_APP
        0,           // subtype
        0,
        0, // date
    ]
    .iter()
    .flat_map(|v: &u32| v.to_le_bytes())
    .collect();
    let exe = format!("{}.exe", app.name);
    let description = if app.description.is_empty() {
        PRODUCT_NAME
    } else {
        &app.description
    };
    let strings: Vec<Vec<u8>> = [
        ("CompanyName", PRODUCT_NAME),
        ("FileDescription", description),
        ("FileVersion", &app.version),
        ("InternalName", &app.name),
        ("LegalCopyright", "MIT OR Apache-2.0"),
        ("OriginalFilename", &exe),
        ("ProductName", PRODUCT_NAME),
        ("ProductVersion", &app.version),
    ]
    .iter()
    .map(|(k, v)| version_node(k, Value::Text(v), &[]))
    .collect();
    let table_key = format!("{LANG_EN_US:04X}{CODEPAGE_UNICODE:04X}");
    let table = version_node(&table_key, Value::None, &strings);
    let string_info = version_node("StringFileInfo", Value::None, &[table]);
    let translation: Vec<u8> = [LANG_EN_US, CODEPAGE_UNICODE]
        .iter()
        .flat_map(|v| v.to_le_bytes())
        .collect();
    let var = version_node("Translation", Value::Binary(&translation), &[]);
    let var_info = version_node("VarFileInfo", Value::None, &[var]);
    version_node(
        "VS_VERSION_INFO",
        Value::Binary(&fixed),
        &[string_info, var_info],
    )
}

/// One entry of a `.res` file (numeric type and name).
fn res_entry(out: &mut Vec<u8>, kind: u16, name: u16, flags: u16, lang: u16, data: &[u8]) {
    out.extend_from_slice(&(data.len() as u32).to_le_bytes());
    out.extend_from_slice(&32u32.to_le_bytes()); // header size
    for v in [0xFFFF, kind, 0xFFFF, name] {
        out.extend_from_slice(&u16::to_le_bytes(v));
    }
    out.extend_from_slice(&0u32.to_le_bytes()); // data version
    out.extend_from_slice(&flags.to_le_bytes());
    out.extend_from_slice(&lang.to_le_bytes());
    out.extend_from_slice(&[0; 8]); // version, characteristics
    out.extend_from_slice(data);
    pad4(out);
}

/// A compiled resource file (`.res`, as rc.exe writes it): the icon images
/// (`RT_ICON` 1…n), their group ([`ICON_ID`]) and the version block.
pub fn res_file(images: &[IconImage], app: &AppInfo) -> Vec<u8> {
    const MOVEABLE_DISCARDABLE: u16 = 0x1010;
    const MOVEABLE_PURE: u16 = 0x0030;
    let mut out = Vec::new();
    res_entry(&mut out, 0, 0, 0, 0, &[]); // the empty first entry
    let mut group = vec![0, 0, 1, 0];
    group.extend_from_slice(&(images.len() as u16).to_le_bytes());
    for (i, image) in images.iter().enumerate() {
        let id = i as u16 + 1;
        let bytes = image.encoded();
        res_entry(
            &mut out,
            RT_ICON,
            id,
            MOVEABLE_DISCARDABLE,
            LANG_EN_US,
            &bytes,
        );
        let d = dim_byte(image.size);
        group.extend_from_slice(&[d, d, 0, 0]);
        group.extend_from_slice(&1u16.to_le_bytes());
        group.extend_from_slice(&32u16.to_le_bytes());
        group.extend_from_slice(&(bytes.len() as u32).to_le_bytes());
        group.extend_from_slice(&id.to_le_bytes());
    }
    res_entry(
        &mut out,
        RT_GROUP_ICON,
        ICON_ID,
        MOVEABLE_DISCARDABLE,
        LANG_EN_US,
        &group,
    );
    res_entry(
        &mut out,
        RT_VERSION,
        1,
        MOVEABLE_PURE,
        LANG_EN_US,
        &version_info(app),
    );
    out
}

/// For an app's `build.rs`: [`embed_logo`] with the game's logo, [`OCW`].
pub fn embed(description: &str) {
    embed_logo(description, &OCW);
}

/// For an app's `build.rs`: writes `<logo>.ico` into `OUT_DIR` and, when
/// the target is Windows with MSVC, `<name>.res` linked into the package's
/// binaries (the logo's icon and the version info; the package needs a bin
/// target). Other targets get no resource.
/// `description` is the file description, which Task Manager and the
/// taskbar's jump list show as the app's name.
pub fn embed_logo(description: &str, logo: &Logo) {
    let out_dir = std::path::PathBuf::from(std::env::var_os("OUT_DIR").expect("OUT_DIR"));
    let os = std::env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
    let env = std::env::var("CARGO_CFG_TARGET_ENV").unwrap_or_default();
    let images = logo_images(logo).expect("the logo decodes");
    let ico = format!("{}.ico", logo.name);
    std::fs::write(out_dir.join(&ico), ico_file(&images)).expect("write the .ico file");
    if os != "windows" {
        return;
    }
    if env != "msvc" {
        println!(
            "cargo:warning=no icon resource for the {env} toolchain (only MSVC links .res files)"
        );
        return;
    }
    let app = AppInfo {
        description: description.to_string(),
        ..AppInfo::from_cargo_env()
    };
    let res = out_dir.join(format!("{}.res", app.name));
    std::fs::write(&res, res_file(&images, &app)).expect("write the .res file");
    println!("cargo:rustc-link-arg-bins={}", res.display());
}

#[cfg(test)]
mod tests {
    use super::*;

    fn u16_at(b: &[u8], i: usize) -> u16 {
        u16::from_le_bytes([b[i], b[i + 1]])
    }
    fn u32_at(b: &[u8], i: usize) -> u32 {
        u32::from_le_bytes(b[i..i + 4].try_into().unwrap())
    }

    /// Decodes one stored icon image (PNG or DIB) into (size, RGBA).
    fn decode_entry(bytes: &[u8]) -> (u32, Vec<u8>) {
        if bytes.starts_with(b"\x89PNG") {
            let (rgba, w, h) = decode_png(bytes).unwrap();
            assert_eq!(w, h);
            return (w, rgba);
        }
        assert_eq!(u32_at(bytes, 0), 40, "BITMAPINFOHEADER");
        let s = u32_at(bytes, 4) as usize;
        assert_eq!(u32_at(bytes, 8) as usize, 2 * s);
        assert_eq!(u16_at(bytes, 14), 32);
        let mask_row = s.div_ceil(32) * 4;
        assert_eq!(bytes.len(), 40 + s * s * 4 + mask_row * s);
        let mut rgba = vec![0; s * s * 4];
        for y in 0..s {
            for x in 0..s {
                let src = &bytes[40 + ((s - 1 - y) * s + x) * 4..][..4];
                rgba[(y * s + x) * 4..][..4].copy_from_slice(&[src[2], src[1], src[0], src[3]]);
                let mask = bytes[40 + s * s * 4 + (s - 1 - y) * mask_row + x / 8];
                let transparent = mask & (0x80 >> (x % 8)) != 0;
                assert_eq!(transparent, src[3] == 0, "AND mask at {x},{y}");
            }
        }
        (s as u32, rgba)
    }

    #[test]
    fn the_ico_has_every_size_and_each_decodes() {
        for logo in [OCW, OCW_STUDIO] {
            ico_has_every_size_and_each_decodes(&logo);
        }
    }

    #[test]
    fn the_studio_gets_its_own_logo() {
        assert_eq!(OCW_STUDIO.name, "ocw-studio");
        let game = logo_images(&OCW).unwrap();
        let studio = logo_images(&OCW_STUDIO).unwrap();
        for (g, s) in game.iter().zip(&studio) {
            assert_eq!(g.size, s.size);
            assert_ne!(g.rgba, s.rgba, "{} px: the studio's icon differs", s.size);
        }
    }

    fn ico_has_every_size_and_each_decodes(logo: &Logo) {
        let images = logo_images(logo).unwrap();
        let ico = ico_file(&images);
        assert_eq!(&ico[..4], &[0, 0, 1, 0]);
        let count = u16_at(&ico, 4) as usize;
        assert_eq!(count, ICON_SIZES.len());
        let mut sizes = Vec::new();
        for i in 0..count {
            let e = &ico[6 + 16 * i..][..16];
            let dim = if e[0] == 0 { 256 } else { u32::from(e[0]) };
            assert_eq!(e[0], e[1], "square");
            assert_eq!(u16_at(e, 6), 32, "32 bits per pixel");
            let (len, off) = (u32_at(e, 8) as usize, u32_at(e, 12) as usize);
            let (size, rgba) = decode_entry(&ico[off..off + len]);
            assert_eq!(size, dim);
            assert_eq!(rgba.len(), (size * size * 4) as usize);
            // The rounded tile: transparent corner, opaque middle.
            assert_eq!(rgba[3], 0, "{size} px: top-left corner transparent");
            let mid = ((size / 2 * size + size / 2) * 4 + 3) as usize;
            assert_eq!(rgba[mid], 255, "{size} px: middle opaque");
            sizes.push(size);
        }
        assert_eq!(sizes, ICON_SIZES);
        // The DIBs keep the PNGs' pixels exactly.
        for image in &images {
            assert_eq!(decode_entry(&image.encoded()).1, image.rgba);
        }
    }

    #[test]
    fn halving_averages_with_alpha() {
        // 2×2: one opaque red, one opaque blue, two transparent (any colour).
        let px = [255, 0, 0, 255, 0, 0, 255, 255, 9, 9, 9, 0, 9, 9, 9, 0];
        assert_eq!(halve(&px, 2), vec![128, 0, 128, 128]);
        assert_eq!(halve(&[0; 16], 2), vec![0; 4]);
    }

    fn app() -> AppInfo {
        AppInfo {
            name: "ocw-desktop".into(),
            description: "Open Cycling World desktop client".into(),
            version: "0.2.2".into(),
        }
    }

    fn contains_utf16(hay: &[u8], s: &str) -> bool {
        let needle = utf16z(s);
        hay.windows(needle.len()).any(|w| w == needle)
    }

    #[test]
    fn the_version_block_names_the_product_and_version() {
        assert_eq!(app().version_parts(), [0, 2, 2, 0]);
        let v = version_info(&app());
        assert_eq!(u16_at(&v, 0) as usize, v.len());
        assert_eq!(u16_at(&v, 2), 52, "VS_FIXEDFILEINFO");
        let fixed = 6 + utf16z("VS_VERSION_INFO").len() + 2; // key padded to 4
        assert_eq!(u32_at(&v, fixed), 0xFEEF_04BD);
        assert_eq!(u32_at(&v, fixed + 8), 2, "file version MS = 0.2");
        assert_eq!(u32_at(&v, fixed + 12), 2 << 16, "file version LS = 2.0");
        for s in [
            "Open Cycling World",
            "ocw-desktop.exe",
            "0.2.2",
            "040904B0",
            "Translation",
        ] {
            assert!(contains_utf16(&v, s), "{s}");
        }
    }

    #[test]
    fn the_res_file_holds_icons_group_and_version() {
        let images = logo_images(&OCW).unwrap();
        let res = res_file(&images, &app());
        let mut i = 0;
        let mut entries = Vec::new();
        while i < res.len() {
            let (data, header) = (u32_at(&res, i) as usize, u32_at(&res, i + 4) as usize);
            assert_eq!(header, 32);
            assert_eq!(u16_at(&res, i + 8), 0xFFFF);
            let (kind, name) = (u16_at(&res, i + 10), u16_at(&res, i + 14));
            entries.push((kind, name, res[i + header..i + header + data].to_vec()));
            i += (header + data).div_ceil(4) * 4;
        }
        assert_eq!(i, res.len());
        assert_eq!(
            (entries[0].0, entries[0].2.len()),
            (0, 0),
            "empty first entry"
        );
        let icons: Vec<_> = entries.iter().filter(|e| e.0 == RT_ICON).collect();
        assert_eq!(icons.len(), ICON_SIZES.len());
        let group = entries.iter().find(|e| e.0 == RT_GROUP_ICON).unwrap();
        assert_eq!(group.1, ICON_ID);
        assert_eq!(u16_at(&group.2, 4) as usize, ICON_SIZES.len());
        for (n, size) in ICON_SIZES.iter().enumerate() {
            let e = &group.2[6 + 14 * n..][..14];
            let id = u16_at(e, 12);
            let icon = icons.iter().find(|i| i.1 == id).unwrap();
            assert_eq!(u32_at(e, 8) as usize, icon.2.len());
            assert_eq!(decode_entry(&icon.2).0, *size);
        }
        let version = entries.iter().find(|e| e.0 == RT_VERSION).unwrap();
        assert_eq!(version.2, version_info(&app()));
    }
}
