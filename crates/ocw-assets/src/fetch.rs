//! `ocw-assets materials fetch` (`scripts/fetch-materials.sh`): downloads
//! the pinned sources of `assets/materials.toml` once into a cache,
//! checks every SHA-256, converts them ([`crate::convert`]) and writes the
//! library folder (spec §4.6, Phase 8 T1). Materials whose textures are
//! already there with the same stamp are skipped, so the script is cheap
//! to run in every build path.

use std::path::{Path, PathBuf};

use crate::convert::{self, Plane, SourceMaps};
use crate::materials::{
    CREDITS_FILE, LIBRARY_MANIFEST, Material, MaterialManifest, credits_markdown, verify,
};

/// What a fetch did.
#[derive(Debug, Default)]
pub struct Report {
    pub converted: Vec<String>,
    pub up_to_date: Vec<String>,
    pub downloaded_bytes: u64,
}

/// Where downloads are cached: `$OCW_MATERIAL_CACHE`, else the user cache
/// folder (shared by all checkouts on a machine).
pub fn default_cache() -> PathBuf {
    if let Ok(d) = std::env::var("OCW_MATERIAL_CACHE")
        && !d.is_empty()
    {
        return PathBuf::from(d);
    }
    let base = std::env::var_os("XDG_CACHE_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("LOCALAPPDATA").map(PathBuf::from))
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".cache")))
        .unwrap_or_else(|| PathBuf::from("target"));
    base.join("ocw").join("material-sources")
}

/// File name of a URL in the cache (its last path segment / query value,
/// made safe, plus a short hash of the whole URL).
pub fn cache_name(url: &str) -> String {
    let last = url
        .rsplit(['/', '=', '?'])
        .find(|s| !s.is_empty())
        .unwrap_or("download");
    let safe: String = last
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_') {
                c
            } else {
                '_'
            }
        })
        .collect();
    format!(
        "{}-{safe}",
        &crate::materials::sha256_hex(url.as_bytes())[..8]
    )
}

fn download(url: &str, cache: &Path, report: &mut Report) -> Result<Vec<u8>, String> {
    let path = cache.join(cache_name(url));
    if let Ok(b) = std::fs::read(&path) {
        return Ok(b);
    }
    eprintln!("downloading {url}");
    let agent = ureq::Agent::new_with_defaults();
    let mut resp = agent
        .get(url)
        .header(
            "User-Agent",
            "ocw-assets (Open Cycling World material fetch)",
        )
        .call()
        .map_err(|e| format!("{url}: {e}"))?;
    let bytes = resp
        .body_mut()
        .with_config()
        .limit(512 * 1024 * 1024)
        .read_to_vec()
        .map_err(|e| format!("{url}: {e}"))?;
    report.downloaded_bytes += bytes.len() as u64;
    std::fs::create_dir_all(cache).map_err(|e| format!("{}: {e}", cache.display()))?;
    let tmp = path.with_extension("part");
    std::fs::write(&tmp, &bytes).map_err(|e| format!("{}: {e}", tmp.display()))?;
    std::fs::rename(&tmp, &path).map_err(|e| format!("{}: {e}", path.display()))?;
    Ok(bytes)
}

/// A member of a zip archive.
pub fn zip_member(archive: &[u8], name: &str) -> Result<Vec<u8>, String> {
    use std::io::Read;
    let mut z = zip::ZipArchive::new(std::io::Cursor::new(archive)).map_err(|e| e.to_string())?;
    let mut f = z
        .by_name(name)
        .map_err(|e| format!("{name} not in the archive: {e}"))?;
    let mut out = Vec::new();
    f.read_to_end(&mut out).map_err(|e| e.to_string())?;
    Ok(out)
}

/// The bytes of every source file of `m`, checked against their pins.
fn sources(
    m: &Material,
    cache: &Path,
    report: &mut Report,
) -> Result<Vec<(&'static str, Vec<u8>)>, String> {
    let archive = match &m.archive {
        Some(url) => Some(download(url, cache, report)?),
        None => None,
    };
    let mut out = Vec::new();
    for (role, f) in m.files() {
        let bytes = match &archive {
            Some(a) => zip_member(a, &f.file)?,
            None => download(&f.file, cache, report)?,
        };
        verify(&format!("{} {role} ({})", m.id, f.file), &bytes, &f.sha256)?;
        out.push((role, bytes));
    }
    Ok(out)
}

/// Decodes the checked source files into the converter's input.
pub fn source_maps(files: &[(&'static str, Vec<u8>)]) -> Result<SourceMaps, String> {
    let get = |role: &str| files.iter().find(|(r, _)| *r == role).map(|(_, b)| b);
    let albedo = convert::decode_image(get("albedo").ok_or("no albedo")?, 3)?;
    let normal = convert::decode_image(get("normal").ok_or("no normal")?, 3)?;
    let (roughness, ao): (Plane, Option<Plane>) = if let Some(arm) = get("arm") {
        let p = convert::decode_image(arm, 3)?;
        (convert::channel(&p, 1), Some(convert::channel(&p, 0)))
    } else {
        let r = convert::decode_image(get("roughness").ok_or("no roughness")?, 1)?;
        let ao = get("ao").map(|b| convert::decode_image(b, 1)).transpose()?;
        (r, ao)
    };
    Ok(SourceMaps {
        albedo,
        normal,
        roughness,
        ao,
    })
}

fn write_atomic(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let tmp = path.with_extension("tmp");
    std::fs::write(&tmp, bytes).map_err(|e| format!("{}: {e}", tmp.display()))?;
    std::fs::rename(&tmp, path).map_err(|e| format!("{}: {e}", path.display()))
}

/// Fetches and converts every material of `manifest` into `out`.
pub fn fetch(
    manifest: &MaterialManifest,
    out: &Path,
    cache: &Path,
    only: Option<&[String]>,
) -> Result<Report, String> {
    std::fs::create_dir_all(out).map_err(|e| format!("{}: {e}", out.display()))?;
    let mut report = Report::default();
    for m in &manifest.materials {
        if only.is_some_and(|o| !o.contains(&m.id)) {
            continue;
        }
        let stamp_path = out.join(format!("{}.stamp", m.id));
        let fresh = std::fs::read_to_string(&stamp_path).is_ok_and(|s| s == m.stamp())
            && out.join(m.albedo_file()).is_file()
            && out.join(m.normal_file()).is_file();
        if fresh {
            report.up_to_date.push(m.id.clone());
            continue;
        }
        let files = sources(m, cache, &mut report)?;
        let maps = source_maps(&files).map_err(|e| format!("{}: {e}", m.id))?;
        let size = convert::SIZE;
        let albedo = convert::encode_albedo(&convert::albedo_rough(&maps, size), &m.id);
        let normal = convert::encode_normal(&convert::normals(&maps, size), &m.id);
        write_atomic(&out.join(m.albedo_file()), &crate::ktx2::write(&albedo)?)?;
        write_atomic(&out.join(m.normal_file()), &crate::ktx2::write(&normal)?)?;
        write_atomic(&stamp_path, m.stamp().as_bytes())?;
        eprintln!("converted {}", m.id);
        report.converted.push(m.id.clone());
    }
    write_atomic(&out.join(LIBRARY_MANIFEST), manifest.to_toml().as_bytes())?;
    write_atomic(
        &out.join(CREDITS_FILE),
        credits_markdown(manifest).as_bytes(),
    )?;
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cache_names_are_safe_and_distinct() {
        let a = cache_name("https://ambientcg.com/get?file=Grass004_1K-JPG.zip");
        assert!(a.ends_with("-Grass004_1K-JPG.zip"), "{a}");
        let b = cache_name("https://dl.polyhaven.org/file/x/farm_soil_diff_1k.jpg");
        assert!(b.ends_with("-farm_soil_diff_1k.jpg"), "{b}");
        assert_ne!(
            cache_name("https://a.org/x/1k.jpg"),
            cache_name("https://b.org/x/1k.jpg")
        );
        assert!(!cache_name("https://a.org/../../etc/passwd").contains('/'));
    }

    /// The pure part of a fetch: members come out of a (pinned) archive,
    /// and a member that does not match its pin is refused.
    #[test]
    fn archive_members_are_extracted_and_checked() {
        use std::io::Write;
        let mut buf = std::io::Cursor::new(Vec::new());
        {
            let mut z = zip::ZipWriter::new(&mut buf);
            let opts = zip::write::SimpleFileOptions::default()
                .compression_method(zip::CompressionMethod::Deflated);
            z.start_file("Grass_Color.jpg", opts).unwrap();
            z.write_all(b"colour bytes").unwrap();
            z.finish().unwrap();
        }
        let zip = buf.into_inner();
        let m = zip_member(&zip, "Grass_Color.jpg").unwrap();
        assert_eq!(m, b"colour bytes");
        assert!(
            zip_member(&zip, "missing.jpg")
                .unwrap_err()
                .contains("not in the archive")
        );
        let pin = crate::materials::sha256_hex(b"colour bytes");
        assert!(verify("meadow albedo", &m, &pin).is_ok());
        assert!(verify("meadow albedo", b"other", &pin).is_err());
    }

    /// Decoding a PNG source and the Poly Haven ARM split.
    #[test]
    fn source_maps_split_the_packed_map() {
        let png = |rgb: [u8; 3]| {
            let img = image::RgbImage::from_pixel(8, 8, image::Rgb(rgb));
            let mut b = std::io::Cursor::new(Vec::new());
            img.write_to(&mut b, image::ImageFormat::Png).unwrap();
            b.into_inner()
        };
        let files = vec![
            ("albedo", png([200, 100, 50])),
            ("normal", png([128, 128, 255])),
            ("arm", png([255, 64, 0])),
        ];
        let maps = source_maps(&files).unwrap();
        assert!((maps.roughness.at(3, 3, 0) - 64.0 / 255.0).abs() < 1e-6);
        assert!((maps.ao.as_ref().unwrap().at(3, 3, 0) - 1.0).abs() < 1e-6);
        assert!((maps.albedo.at(0, 0, 0) - 200.0 / 255.0).abs() < 1e-6);
    }
}
