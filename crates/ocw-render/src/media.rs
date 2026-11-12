//! Metadata for every picture and video the app takes (spec §11.4): the
//! ISO 8601 creation time and the software that made it.
//!
//! - JPEG screenshots (the default, quality [`JPEG_QUALITY`]): EXIF
//!   `DateTimeOriginal` / `CreateDate` (`DateTimeDigitized`) / `DateTime`
//!   in local time, `OffsetTimeOriginal` / `OffsetTime` /
//!   `OffsetTimeDigitized` with the UTC offset, `Software`.
//! - PNG (a path ending in `.png`, and the lossless frames of `--capture`):
//!   `tEXt` chunks `Creation Time` (ISO 8601 with offset) and `Software`.
//! - MP4 videos (ffmpeg): `creation_time` (UTC, `Z`) and `comment`.
//!
//! All pure (no Bevy): bytes in, bytes out.

use ocw_core::timestamp::{exif_local, format_offset, iso8601_local, iso8601_utc};

/// The app's name in `Software` / `comment`.
pub const APP_NAME: &str = "Open Cycling World";
/// JPEG quality of screenshots (spec §11.4).
pub const JPEG_QUALITY: u8 = 90;

/// `Open Cycling World 0.1.0-7d538c0` (just the name without a build id).
pub fn software(build_id: &str) -> String {
    let id = build_id.trim();
    if id.is_empty() {
        APP_NAME.to_string()
    } else {
        format!("{APP_NAME} {id}")
    }
}

/// A moment, to the second, and the local UTC offset then.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct MediaTime {
    /// Unix seconds (UTC).
    pub unix_s: i64,
    /// Local time − UTC, in seconds.
    pub offset_s: i32,
}

impl MediaTime {
    /// `2026-11-12T22:14:05+01:00`: the HUD stamp and PNG `Creation Time`.
    pub fn iso_local(&self) -> String {
        iso8601_local(self.unix_s, self.offset_s)
    }

    /// `2026-11-12T21:14:05Z`: ffmpeg's `creation_time`.
    pub fn iso_utc(&self) -> String {
        iso8601_utc(self.unix_s)
    }
}

/// The local UTC offset at `unix_s` in time zone `tz` (daylight saving
/// time included).
pub fn offset_in<Tz: chrono::TimeZone>(tz: &Tz, unix_s: i64) -> i32 {
    use chrono::Offset;
    let utc = chrono::DateTime::from_timestamp(unix_s, 0)
        .unwrap_or_default()
        .naive_utc();
    tz.offset_from_utc_datetime(&utc).fix().local_minus_utc()
}

/// The system's local UTC offset at `unix_s`.
pub fn local_offset_s(unix_s: i64) -> i32 {
    offset_in(&chrono::Local, unix_s)
}

// --- EXIF (a little-endian TIFF structure, without the "Exif\0\0" header,
// which the JPEG encoder adds) ---

struct Entry {
    tag: u16,
    kind: u16,
    count: u32,
    data: Vec<u8>,
}

const ASCII: u16 = 2;
const LONG: u16 = 4;
const UNDEFINED: u16 = 7;

fn ascii(tag: u16, s: &str) -> Entry {
    let mut data: Vec<u8> = s.bytes().filter(|b| b.is_ascii() && *b != 0).collect();
    data.push(0);
    Entry {
        tag,
        kind: ASCII,
        count: data.len() as u32,
        data,
    }
}

/// One IFD (entries sorted by tag) located at `start`, its values after it.
fn ifd(entries: &[Entry], start: u32, next: u32) -> Vec<u8> {
    let head = 2 + 12 * entries.len() + 4;
    let mut data_at = start as usize + head;
    let mut out = Vec::new();
    let mut tail = Vec::new();
    out.extend_from_slice(&(entries.len() as u16).to_le_bytes());
    for e in entries {
        out.extend_from_slice(&e.tag.to_le_bytes());
        out.extend_from_slice(&e.kind.to_le_bytes());
        out.extend_from_slice(&e.count.to_le_bytes());
        if e.data.len() <= 4 {
            let mut v = e.data.clone();
            v.resize(4, 0);
            out.extend_from_slice(&v);
        } else {
            out.extend_from_slice(&(data_at as u32).to_le_bytes());
            tail.extend_from_slice(&e.data);
            if e.data.len() % 2 == 1 {
                tail.push(0); // values start on a word boundary
            }
            data_at = start as usize + head + tail.len();
        }
    }
    out.extend_from_slice(&next.to_le_bytes());
    out.extend_from_slice(&tail);
    out
}

/// The EXIF block (TIFF) of a screenshot taken at `time` by `software`.
pub fn exif_tiff(time: &MediaTime, software: &str) -> Vec<u8> {
    let local = exif_local(time.unix_s, time.offset_s);
    let offset = format_offset(time.offset_s);
    let ifd0 = |exif_at: u32| {
        vec![
            ascii(0x0131, software),
            ascii(0x0132, &local),
            Entry {
                tag: 0x8769, // Exif IFD pointer
                kind: LONG,
                count: 1,
                data: exif_at.to_le_bytes().to_vec(),
            },
        ]
    };
    let exif = [
        Entry {
            tag: 0x9000, // ExifVersion 2.32
            kind: UNDEFINED,
            count: 4,
            data: b"0232".to_vec(),
        },
        ascii(0x9003, &local),  // DateTimeOriginal
        ascii(0x9004, &local),  // DateTimeDigitized (CreateDate)
        ascii(0x9010, &offset), // OffsetTime
        ascii(0x9011, &offset), // OffsetTimeOriginal
        ascii(0x9012, &offset), // OffsetTimeDigitized
    ];
    let mut out = b"II\x2a\x00\x08\x00\x00\x00".to_vec();
    let len0 = ifd(&ifd0(0), 8, 0).len() as u32;
    out.extend(ifd(&ifd0(8 + len0), 8, 0));
    out.extend(ifd(&exif, 8 + len0, 0));
    out
}

/// Encodes an RGB8 image as JPEG with the EXIF block `exif`.
pub fn encode_jpeg(
    rgb: &[u8],
    w: u32,
    h: u32,
    quality: u8,
    exif: Vec<u8>,
) -> Result<Vec<u8>, String> {
    use image::ImageEncoder;
    let mut out = Vec::new();
    let mut enc = image::codecs::jpeg::JpegEncoder::new_with_quality(&mut out, quality);
    enc.set_exif_metadata(exif).map_err(|e| e.to_string())?;
    enc.write_image(rgb, w, h, image::ExtendedColorType::Rgb8)
        .map_err(|e| e.to_string())?;
    Ok(out)
}

/// Encodes an RGB8 image as PNG with `tEXt` chunks `text`.
pub fn encode_png(
    rgb: &[u8],
    w: u32,
    h: u32,
    text: &[(String, String)],
) -> Result<Vec<u8>, String> {
    use image::ImageEncoder;
    let mut out = Vec::new();
    image::codecs::png::PngEncoder::new(&mut out)
        .write_image(rgb, w, h, image::ExtendedColorType::Rgb8)
        .map_err(|e| e.to_string())?;
    insert_png_text(&out, text)
}

// --- PNG text chunks ---

/// CRC-32 (ISO 3309, as PNG uses it).
pub fn crc32(bytes: &[u8]) -> u32 {
    let mut c = 0xffff_ffffu32;
    for &b in bytes {
        c ^= u32::from(b);
        for _ in 0..8 {
            c = if c & 1 != 0 {
                0xedb8_8320 ^ (c >> 1)
            } else {
                c >> 1
            };
        }
    }
    !c
}

const PNG_SIGNATURE: &[u8] = b"\x89PNG\r\n\x1a\n";

/// One `tEXt` chunk (Latin-1 keyword of 1–79 characters, a NUL, the text;
/// characters outside Latin-1 become `?`).
pub fn png_text_chunk(keyword: &str, text: &str) -> Vec<u8> {
    let latin1 = |s: &str| -> Vec<u8> {
        s.chars()
            .map(|c| u8::try_from(u32::from(c)).unwrap_or(b'?'))
            .filter(|&b| b != 0)
            .collect()
    };
    let mut body = b"tEXt".to_vec();
    let mut key = latin1(keyword);
    key.truncate(79);
    body.extend(key);
    body.push(0);
    body.extend(latin1(text));
    let mut chunk = ((body.len() - 4) as u32).to_be_bytes().to_vec();
    chunk.extend_from_slice(&body);
    chunk.extend_from_slice(&crc32(&body).to_be_bytes());
    chunk
}

/// `png` with `tEXt` chunks for `entries` right after its header (IHDR).
pub fn insert_png_text(png: &[u8], entries: &[(String, String)]) -> Result<Vec<u8>, String> {
    if !png.starts_with(PNG_SIGNATURE) || png.len() < 33 || &png[12..16] != b"IHDR" {
        return Err("not a PNG file".into());
    }
    let ihdr_len = u32::from_be_bytes([png[8], png[9], png[10], png[11]]) as usize;
    let after_ihdr = 8 + 12 + ihdr_len;
    if png.len() < after_ihdr {
        return Err("truncated PNG header".into());
    }
    let mut out = png[..after_ihdr].to_vec();
    for (k, v) in entries {
        out.extend(png_text_chunk(k, v));
    }
    out.extend_from_slice(&png[after_ihdr..]);
    Ok(out)
}

/// The PNG text entries: `Creation Time` (registered keyword, ISO 8601)
/// and `Software`.
pub fn png_entries(time: &MediaTime, software: &str) -> Vec<(String, String)> {
    vec![
        ("Creation Time".into(), time.iso_local()),
        ("Software".into(), software.into()),
    ]
}

// --- videos (ffmpeg) ---

/// The MP4 metadata: `creation_time` (UTC, the start of the recording) and
/// `comment` (the software; ffmpeg writes its own `encoder` tag).
pub fn video_metadata(start: &MediaTime, software: &str) -> Vec<(String, String)> {
    vec![
        ("creation_time".into(), start.iso_utc()),
        ("comment".into(), software.into()),
    ]
}

/// `-metadata key=value` arguments for ffmpeg.
pub fn metadata_args(meta: &[(String, String)]) -> Vec<String> {
    meta.iter()
        .flat_map(|(k, v)| ["-metadata".to_string(), format!("{k}={v}")])
        .collect()
}

/// An ffmpeg metadata file (`-f ffmetadata -i FILE -map_metadata N`), as
/// frame capture writes it for the scripts that encode its frames.
pub fn ffmetadata(meta: &[(String, String)]) -> String {
    let esc = |s: &str| {
        let mut o = String::new();
        for c in s.chars() {
            if matches!(c, '=' | ';' | '#' | '\\' | '\n') {
                o.push('\\');
            }
            o.push(c);
        }
        o
    };
    let mut out = String::from(";FFMETADATA1\n");
    for (k, v) in meta {
        out.push_str(&format!("{}={}\n", esc(k), esc(v)));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 2026-11-12T21:14:05Z, at +01:00.
    const T: MediaTime = MediaTime {
        unix_s: 1_794_518_045,
        offset_s: 3600,
    };

    #[test]
    fn names_and_times() {
        assert_eq!(
            software("0.1.0-7d538c0"),
            "Open Cycling World 0.1.0-7d538c0"
        );
        assert_eq!(software(" "), "Open Cycling World");
        assert_eq!(T.iso_local(), "2026-11-12T22:14:05+01:00");
        assert_eq!(T.iso_utc(), "2026-11-12T21:14:05Z");
    }

    #[test]
    fn utc_offsets_of_a_time_zone() {
        let cet = chrono::FixedOffset::east_opt(3600).unwrap();
        assert_eq!(offset_in(&cet, T.unix_s), 3600);
        let nst = chrono::FixedOffset::west_opt(12_600).unwrap();
        assert_eq!(offset_in(&nst, T.unix_s), -12_600);
        assert_eq!(offset_in(&chrono::Utc, T.unix_s), 0);
        // The system's zone: whole minutes, within ±14 h.
        let local = local_offset_s(T.unix_s);
        assert!(local.abs() <= 14 * 3600 && local % 60 == 0, "{local}");
    }

    /// A minimal reader of our TIFF: tag → raw value bytes, both IFDs.
    fn read_tiff(t: &[u8]) -> std::collections::BTreeMap<u16, Vec<u8>> {
        assert_eq!(&t[..4], b"II\x2a\x00");
        let u16_at = |o: usize| u16::from_le_bytes([t[o], t[o + 1]]);
        let u32_at = |o: usize| u32::from_le_bytes([t[o], t[o + 1], t[o + 2], t[o + 3]]) as usize;
        let mut tags = std::collections::BTreeMap::new();
        let mut todo = vec![u32_at(4)];
        while let Some(at) = todo.pop() {
            let n = u16_at(at) as usize;
            let mut last = 0;
            for i in 0..n {
                let e = at + 2 + 12 * i;
                let (tag, kind, count) = (u16_at(e), u16_at(e + 2), u32_at(e + 4));
                assert!(tag > last, "sorted tags");
                last = tag;
                let size = count * if kind == LONG { 4 } else { 1 };
                let v = if size <= 4 {
                    t[e + 8..e + 8 + size].to_vec()
                } else {
                    let o = u32_at(e + 8);
                    assert_eq!(o % 2, 0, "word aligned");
                    t[o..o + size].to_vec()
                };
                if tag == 0x8769 {
                    todo.push(u32_at(e + 8));
                }
                tags.insert(tag, v);
            }
            assert_eq!(u32_at(at + 2 + 12 * n), 0);
        }
        tags
    }

    fn text(v: &[u8]) -> &str {
        std::str::from_utf8(v.strip_suffix(&[0]).expect("NUL terminated")).unwrap()
    }

    #[test]
    fn exif_has_the_dates_offset_and_software() {
        let t = exif_tiff(&T, "Open Cycling World 0.1.0-abc");
        let tags = read_tiff(&t);
        assert_eq!(text(&tags[&0x0131]), "Open Cycling World 0.1.0-abc");
        for tag in [0x0132, 0x9003, 0x9004] {
            assert_eq!(text(&tags[&tag]), "2026:11:12 22:14:05", "{tag:x}");
        }
        for tag in [0x9010, 0x9011, 0x9012] {
            assert_eq!(text(&tags[&tag]), "+01:00", "{tag:x}");
        }
        assert_eq!(tags[&0x9000], b"0232");
    }

    fn gradient(w: u32, h: u32) -> Vec<u8> {
        (0..w * h)
            .flat_map(|i| [(i % w * 4) as u8, (i / w * 4) as u8, 128])
            .collect()
    }

    /// The JPEG decodes, and its EXIF block reads back with the dates.
    #[test]
    fn jpeg_screenshot_decodes_with_its_exif() {
        use image::ImageDecoder;
        let (w, h) = (64, 48);
        let exif = exif_tiff(&T, "Open Cycling World test");
        let jpg = encode_jpeg(&gradient(w, h), w, h, JPEG_QUALITY, exif.clone()).unwrap();
        assert_eq!(&jpg[..2], b"\xff\xd8");
        let img = image::load_from_memory_with_format(&jpg, image::ImageFormat::Jpeg).unwrap();
        assert_eq!((img.width(), img.height()), (w, h));
        let mut dec = image::codecs::jpeg::JpegDecoder::new(std::io::Cursor::new(&jpg)).unwrap();
        let read = dec.exif_metadata().unwrap().expect("EXIF present");
        let read = read.strip_prefix(b"Exif\0\0").unwrap_or(&read).to_vec();
        assert_eq!(read, exif);
        let tags = read_tiff(&read);
        assert_eq!(text(&tags[&0x9003]), "2026:11:12 22:14:05");
        assert_eq!(text(&tags[&0x9011]), "+01:00");
    }

    #[test]
    fn crc_of_known_data() {
        assert_eq!(crc32(b""), 0);
        assert_eq!(crc32(b"123456789"), 0xcbf4_3926);
        assert_eq!(crc32(b"IEND"), 0xae42_6082);
    }

    /// The PNG with its text chunks is valid (CRCs checked by the png
    /// decoder) and the text reads back.
    #[test]
    fn png_text_chunks_read_back() {
        let (w, h) = (16, 8);
        let entries = png_entries(&T, "Open Cycling World test");
        let png = encode_png(&gradient(w, h), w, h, &entries).unwrap();
        let dec = png::Decoder::new(std::io::Cursor::new(&png));
        let mut reader = dec.read_info().unwrap();
        let mut buf = vec![0; reader.output_buffer_size().unwrap()];
        reader.next_frame(&mut buf).unwrap();
        let info = reader.info();
        let texts: Vec<(String, String)> = info
            .uncompressed_latin1_text
            .iter()
            .map(|t| (t.keyword.clone(), t.text.clone()))
            .collect();
        assert_eq!(texts, entries);
        assert_eq!(texts[0].1, "2026-11-12T22:14:05+01:00");
        // The image crate reads it too.
        let img = image::load_from_memory(&png).unwrap();
        assert_eq!((img.width(), img.height()), (w, h));
        assert!(insert_png_text(b"GIF89a", &entries).is_err());
    }

    #[test]
    fn png_chunk_layout() {
        let c = png_text_chunk("Software", "OCW ü");
        assert_eq!(&c[..4], &14u32.to_be_bytes());
        assert_eq!(&c[4..8], b"tEXt");
        assert_eq!(&c[8..22], b"Software\0OCW \xfc");
        assert_eq!(&c[22..], &crc32(&c[4..22]).to_be_bytes());
    }

    #[test]
    fn video_metadata_for_ffmpeg() {
        let meta = video_metadata(&T, "Open Cycling World 0.1.0-abc");
        assert_eq!(
            metadata_args(&meta),
            [
                "-metadata",
                "creation_time=2026-11-12T21:14:05Z",
                "-metadata",
                "comment=Open Cycling World 0.1.0-abc"
            ]
        );
        assert_eq!(
            ffmetadata(&meta),
            ";FFMETADATA1\ncreation_time=2026-11-12T21:14:05Z\ncomment=Open Cycling World 0.1.0-abc\n"
        );
        let odd = vec![("comment".to_string(), "a=b;c#d\\e".to_string())];
        assert_eq!(
            ffmetadata(&odd),
            ";FFMETADATA1\ncomment=a\\=b\\;c\\#d\\\\e\n"
        );
    }
}
