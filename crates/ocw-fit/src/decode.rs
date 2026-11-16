//! FIT activity reader (spec §14 Phase 7, training history): reads back
//! what [`crate::encode`] writes – and the same messages of other
//! recorders – into an [`Activity`], so the training history can be
//! built from rides saved before it existed (`history.import`).
//!
//! Reads the subset the history needs: the `file_id`'s product name, the
//! creator's software version, the `record`s (time, position, altitude,
//! heart rate, cadence, distance, speed, power; also the `enhanced_`
//! speed and altitude), the `user_marker` events (god mode, crashes) and
//! the laps' start times. Normal and compressed-timestamp headers, both
//! byte orders and developer fields (skipped) are understood; the header
//! and file CRCs are checked. Pure and dependency-free (wasm).

use crate::{Activity, FIT_EPOCH_OFFSET_S, Marker, MarkerKind, Record, crc16};

/// Why a file could not be read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DecodeError {
    /// Not a FIT file (no `.FIT` signature, a short header).
    NotFit,
    /// The file ends before its data does.
    Truncated,
    /// The header or file CRC does not match.
    BadCrc,
    /// A data message of a local type without a definition.
    Undefined(u8),
    /// No records: not an activity.
    NoRecords,
}

impl std::fmt::Display for DecodeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotFit => f.write_str("not a FIT file"),
            Self::Truncated => f.write_str("the FIT file is truncated"),
            Self::BadCrc => f.write_str("the FIT file's checksum does not match"),
            Self::Undefined(t) => write!(f, "a message of undefined local type {t}"),
            Self::NoRecords => f.write_str("the FIT file has no records (not an activity)"),
        }
    }
}

impl std::error::Error for DecodeError {}

/// One field of a definition: number, size, base type.
#[derive(Debug, Clone, Copy)]
struct FieldDef {
    num: u8,
    size: u8,
    base: u8,
}

#[derive(Debug, Clone)]
struct Definition {
    global: u16,
    big_endian: bool,
    fields: Vec<FieldDef>,
    /// Bytes of developer fields after the fields (skipped).
    dev_bytes: usize,
}

/// A field value as read: unsigned raw bits (or a string's bytes), with
/// its base type for the "invalid" check.
#[derive(Debug, Clone)]
enum Raw {
    Int { bits: u64, base: u8, size: u8 },
    Bytes(Vec<u8>),
}

impl Raw {
    /// The value as unsigned, `None` if it is the base type's invalid one.
    fn unsigned(&self) -> Option<u64> {
        let Raw::Int { bits, base, size } = *self else {
            return None;
        };
        let all_ones = if size >= 8 {
            u64::MAX
        } else {
            (1u64 << (8 * u32::from(size))) - 1
        };
        // The "z" types use 0 as invalid; the others all ones (and the
        // signed types their maximum).
        let invalid = match base & 0x1F {
            0x0A | 0x0B | 0x0C | 0x10 => bits == 0,
            0x01 | 0x03 | 0x05 | 0x0E => bits == all_ones >> 1,
            _ => bits == all_ones,
        };
        (!invalid).then_some(bits)
    }

    /// The value as a signed number of its size.
    fn signed(&self) -> Option<i64> {
        let Raw::Int { size, .. } = *self else {
            return None;
        };
        let v = self.unsigned()?;
        let shift = 64 - 8 * u32::from(size.min(8));
        Some(((v << shift) as i64) >> shift)
    }

    fn text(&self) -> Option<String> {
        match self {
            Raw::Bytes(b) => {
                let end = b.iter().position(|&c| c == 0).unwrap_or(b.len());
                let s = String::from_utf8_lossy(&b[..end]).into_owned();
                (!s.is_empty()).then_some(s)
            }
            Raw::Int { .. } => None,
        }
    }
}

struct Reader<'a> {
    data: &'a [u8],
    at: usize,
}

impl Reader<'_> {
    fn take(&mut self, n: usize) -> Result<&[u8], DecodeError> {
        let end = self.at.checked_add(n).ok_or(DecodeError::Truncated)?;
        let s = self.data.get(self.at..end).ok_or(DecodeError::Truncated)?;
        self.at = end;
        Ok(s)
    }

    fn u8(&mut self) -> Result<u8, DecodeError> {
        Ok(self.take(1)?[0])
    }
}

fn read_field(r: &mut Reader, f: FieldDef, big: bool) -> Result<Raw, DecodeError> {
    let bytes = r.take(usize::from(f.size))?;
    // Strings (0x07) and byte arrays (0x0D), and arrays of a numeric type
    // (a size larger than the type's): kept as bytes.
    let type_size = match f.base & 0x1F {
        0x00 | 0x01 | 0x02 | 0x0A | 0x0D => 1,
        0x03 | 0x04 | 0x0B => 2,
        0x05 | 0x06 | 0x08 | 0x0C => 4,
        0x09 | 0x0E | 0x0F | 0x10 => 8,
        _ => 0,
    };
    if type_size == 0 || f.size != type_size || f.base & 0x1F == 0x0D {
        return Ok(Raw::Bytes(bytes.to_vec()));
    }
    let mut bits = 0u64;
    for (i, &b) in bytes.iter().enumerate() {
        let shift = if big {
            8 * (bytes.len() - 1 - i)
        } else {
            8 * i
        };
        bits |= u64::from(b) << shift;
    }
    Ok(Raw::Int {
        bits,
        base: f.base,
        size: f.size,
    })
}

// Global message and field numbers (FIT profile).
const MESG_FILE_ID: u16 = 0;
const MESG_SESSION: u16 = 18;
const MESG_LAP: u16 = 19;
const MESG_RECORD: u16 = 20;
const MESG_EVENT: u16 = 21;
const MESG_DEVICE_INFO: u16 = 23;
const EVENT_USER_MARKER: u64 = 32;
const TIMESTAMP: u8 = 253;

/// Reads a FIT activity file.
pub fn decode(bytes: &[u8]) -> Result<Activity, DecodeError> {
    let header_len = usize::from(*bytes.first().ok_or(DecodeError::NotFit)?);
    if !(header_len == 12 || header_len == 14) || bytes.len() < header_len {
        return Err(DecodeError::NotFit);
    }
    if &bytes[8..12] != b".FIT" {
        return Err(DecodeError::NotFit);
    }
    if header_len == 14 {
        let crc = u16::from_le_bytes([bytes[12], bytes[13]]);
        if crc != 0 && crc16(0, &bytes[..12]) != crc {
            return Err(DecodeError::BadCrc);
        }
    }
    let data_len = u32::from_le_bytes(bytes[4..8].try_into().expect("4 bytes")) as usize;
    let end = header_len
        .checked_add(data_len)
        .ok_or(DecodeError::Truncated)?;
    if bytes.len() < end + 2 {
        return Err(DecodeError::Truncated);
    }
    if crc16(0, &bytes[..end + 2]) != 0 {
        return Err(DecodeError::BadCrc);
    }
    let mut r = Reader {
        data: &bytes[..end],
        at: header_len,
    };
    let mut defs: [Option<Definition>; 16] = Default::default();
    let mut last_ts: Option<u32> = None;
    let mut product_name = String::new();
    let mut software_version = 0u16;
    // (timestamp, record) and (timestamp, marker), lap start times, the
    // session's start.
    let mut records: Vec<(u32, Record)> = Vec::new();
    let mut markers: Vec<(u32, MarkerKind)> = Vec::new();
    let mut lap_starts: Vec<u32> = Vec::new();
    let mut session_start: Option<u32> = None;
    while r.at < end {
        let h = r.u8()?;
        let (local, compressed_ts) = if h & 0x80 != 0 {
            ((h >> 5) & 0x03, Some(u32::from(h & 0x1F)))
        } else if h & 0x40 != 0 {
            let local = h & 0x0F;
            let dev = h & 0x20 != 0;
            r.u8()?; // reserved
            let big = r.u8()? == 1;
            let g = r.take(2)?;
            let global = if big {
                u16::from_be_bytes([g[0], g[1]])
            } else {
                u16::from_le_bytes([g[0], g[1]])
            };
            let n = r.u8()?;
            let mut fields = Vec::with_capacity(usize::from(n));
            for _ in 0..n {
                let f = r.take(3)?;
                fields.push(FieldDef {
                    num: f[0],
                    size: f[1],
                    base: f[2],
                });
            }
            let mut dev_bytes = 0;
            if dev {
                let n = r.u8()?;
                for _ in 0..n {
                    dev_bytes += usize::from(r.take(3)?[1]);
                }
            }
            defs[usize::from(local)] = Some(Definition {
                global,
                big_endian: big,
                fields,
                dev_bytes,
            });
            continue;
        } else {
            (h & 0x0F, None)
        };
        let def = defs[usize::from(local)]
            .clone()
            .ok_or(DecodeError::Undefined(local))?;
        let mut values: Vec<(u8, Raw)> = Vec::with_capacity(def.fields.len());
        for f in &def.fields {
            values.push((f.num, read_field(&mut r, *f, def.big_endian)?));
        }
        r.take(def.dev_bytes)?;
        let get = |num: u8| values.iter().find(|(n, _)| *n == num).map(|(_, v)| v);
        let uint = |num: u8| get(num).and_then(Raw::unsigned);
        // The message's time: its timestamp field, or the compressed
        // header's offset from the last one (5 bits, rolling over).
        let ts = match (uint(TIMESTAMP), compressed_ts, last_ts) {
            (Some(t), _, _) => Some(t as u32),
            (None, Some(off), Some(last)) => {
                let mut t = (last & !0x1F) | off;
                if off < (last & 0x1F) {
                    t += 0x20;
                }
                Some(t)
            }
            _ => None,
        };
        if let Some(t) = ts {
            last_ts = Some(t);
        }
        match def.global {
            MESG_FILE_ID => {
                if let Some(s) = get(8).and_then(Raw::text) {
                    product_name = s;
                }
            }
            MESG_DEVICE_INFO => {
                // The creator (device index 0, or none given).
                if uint(0).is_none_or(|i| i == 0)
                    && let Some(v) = uint(5)
                {
                    software_version = v as u16;
                }
            }
            MESG_RECORD => {
                let Some(t) = ts else { continue };
                let semi = 180.0 / 2f64.powi(31);
                let lat = get(0).and_then(Raw::signed);
                let lon = get(1).and_then(Raw::signed);
                let speed = uint(6)
                    .map(|v| v as f64 / 1000.0)
                    .or_else(|| uint(73).map(|v| v as f64 / 1000.0));
                let altitude = uint(2)
                    .map(|v| v as f64 / 5.0 - 500.0)
                    .or_else(|| uint(78).map(|v| v as f64 / 5.0 - 500.0));
                records.push((
                    t,
                    Record {
                        elapsed_s: 0.0,
                        distance_m: uint(5).map_or(0.0, |v| v as f64 / 100.0),
                        speed_mps: speed.unwrap_or(0.0),
                        power_w: uint(7).map(|v| v as f64),
                        cadence_rpm: uint(4).map(|v| v as f64),
                        heart_rate_bpm: uint(3).map(|v| v as f64),
                        altitude_m: altitude,
                        position: match (lat, lon) {
                            (Some(a), Some(b)) => Some((a as f64 * semi, b as f64 * semi)),
                            _ => None,
                        },
                    },
                ));
            }
            MESG_EVENT => {
                if let (Some(t), Some(EVENT_USER_MARKER)) = (ts, uint(0)) {
                    let kind = match uint(3) {
                        Some(1) => Some(MarkerKind::GodModeStart),
                        Some(2) => Some(MarkerKind::GodModeEnd),
                        Some(3) => Some(MarkerKind::Crash),
                        _ => None,
                    };
                    if let Some(k) = kind {
                        markers.push((t, k));
                    }
                }
            }
            MESG_LAP => {
                if let Some(s) = uint(2) {
                    lap_starts.push(s as u32);
                }
            }
            MESG_SESSION => {
                if let Some(s) = uint(2) {
                    session_start = Some(s as u32);
                }
            }
            _ => {}
        }
    }
    let first = records
        .first()
        .map(|(t, _)| *t)
        .ok_or(DecodeError::NoRecords)?;
    let start = session_start.map_or(first, |s| s.min(first));
    let elapsed = |t: u32| f64::from(t.saturating_sub(start));
    let records: Vec<Record> = records
        .into_iter()
        .map(|(t, mut rec)| {
            rec.elapsed_s = elapsed(t);
            rec
        })
        .collect();
    let mut laps: Vec<f64> = lap_starts.into_iter().map(elapsed).collect();
    laps.sort_by(f64::total_cmp);
    laps.dedup();
    // The first lap starts with the ride: not a mark.
    laps.retain(|&t| t > 0.5);
    Ok(Activity {
        start_unix_s: i64::from(start) + FIT_EPOCH_OFFSET_S,
        records,
        product_name,
        software_version,
        laps,
        markers: markers
            .into_iter()
            .map(|(t, kind)| Marker {
                elapsed_s: elapsed(t),
                kind,
            })
            .collect(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::encode;

    fn ride(n: usize) -> Activity {
        Activity {
            start_unix_s: 1_762_862_400,
            records: (0..n)
                .map(|i| {
                    let t = i as f64;
                    Record {
                        elapsed_s: t,
                        distance_m: 8.0 * t,
                        speed_mps: 8.0 + (t * 0.1).sin(),
                        power_w: (i % 7 != 3).then_some(200.0 + (i % 50) as f64),
                        cadence_rpm: Some(85.0),
                        heart_rate_bpm: (i > 2).then_some(120.0 + (i % 30) as f64),
                        altitude_m: Some(455.0 + t * 0.2),
                        position: (i % 11 != 0).then_some((47.15 + t * 1e-5, -9.55 - t * 2e-5)),
                    }
                })
                .collect(),
            product_name: "Open Cycling World".into(),
            software_version: 12,
            laps: vec![120.0, 400.0],
            markers: vec![
                Marker {
                    elapsed_s: 30.0,
                    kind: MarkerKind::Crash,
                },
                Marker {
                    elapsed_s: 50.0,
                    kind: MarkerKind::GodModeStart,
                },
                Marker {
                    elapsed_s: 70.0,
                    kind: MarkerKind::GodModeEnd,
                },
            ],
        }
    }

    #[test]
    fn reads_back_what_the_writer_wrote() {
        let a = ride(600);
        let b = decode(&encode(&a)).expect("decodes");
        assert_eq!(b.start_unix_s, a.start_unix_s);
        assert_eq!(b.product_name, a.product_name);
        assert_eq!(b.software_version, 12);
        assert_eq!(b.records.len(), a.records.len());
        for (x, y) in a.records.iter().zip(&b.records) {
            assert_eq!(x.elapsed_s, y.elapsed_s);
            assert!((x.distance_m - y.distance_m).abs() < 0.006);
            assert!((x.speed_mps - y.speed_mps).abs() < 0.0006);
            assert_eq!(x.power_w, y.power_w);
            assert_eq!(x.cadence_rpm, y.cadence_rpm);
            assert_eq!(x.heart_rate_bpm, y.heart_rate_bpm);
            assert!((x.altitude_m.unwrap() - y.altitude_m.unwrap()).abs() <= 0.1);
            match (x.position, y.position) {
                (Some((a1, o1)), Some((a2, o2))) => {
                    assert!((a1 - a2).abs() < 1e-6 && (o1 - o2).abs() < 1e-6);
                }
                (None, None) => {}
                other => panic!("{other:?}"),
            }
        }
        assert_eq!(b.laps, a.laps);
        assert_eq!(b.markers, a.markers);
    }

    #[test]
    fn broken_files_are_refused() {
        let bytes = encode(&ride(20));
        assert_eq!(decode(b"hello"), Err(DecodeError::NotFit));
        assert_eq!(decode(&[]), Err(DecodeError::NotFit));
        assert_eq!(
            decode(&bytes[..bytes.len() - 5]),
            Err(DecodeError::Truncated)
        );
        let mut bad = bytes.clone();
        bad[40] ^= 0x55;
        assert_eq!(decode(&bad), Err(DecodeError::BadCrc));
        let empty = encode(&Activity {
            records: Vec::new(),
            ..ride(0)
        });
        assert_eq!(decode(&empty), Err(DecodeError::NoRecords));
    }

    /// Messages of other recorders: big-endian definitions, compressed
    /// timestamp headers, developer fields, enhanced speed and altitude.
    #[test]
    fn reads_compressed_timestamps_big_endian_and_developer_fields() {
        let mut d = Vec::new();
        // Definition, local 0, big endian, record: timestamp, enhanced
        // speed (u32), power (u16); one developer field of 2 bytes.
        d.extend_from_slice(&[0x60, 0, 1, 0, 20, 3]);
        d.extend_from_slice(&[253, 4, 0x86, 73, 4, 0x86, 7, 2, 0x84]);
        d.extend_from_slice(&[1, 0, 2, 0]);
        let t0: u32 = 1_000_000_000;
        d.push(0x00);
        d.extend_from_slice(&t0.to_be_bytes());
        d.extend_from_slice(&5_500u32.to_be_bytes());
        d.extend_from_slice(&250u16.to_be_bytes());
        d.extend_from_slice(&[0xAA, 0xBB]);
        // Definition, local 1, little endian: record without timestamp
        // (power only), used with compressed headers.
        d.extend_from_slice(&[0x41, 0, 0, 20, 0, 1, 7, 2, 0x84]);
        for (off, p) in [(1u8, 260u16), (2, 270)] {
            // Compressed header: local type 1, offset (t0 + off) & 31.
            d.push(0x80 | (1 << 5) | ((t0 as u8).wrapping_add(off) & 0x1F));
            d.extend_from_slice(&p.to_le_bytes());
        }
        let mut f = vec![12u8, 0x20, 0x54, 0x08];
        f.extend_from_slice(&(d.len() as u32).to_le_bytes());
        f.extend_from_slice(b".FIT");
        f.extend_from_slice(&d);
        let crc = crc16(0, &f);
        f.extend_from_slice(&crc.to_le_bytes());
        let a = decode(&f).expect("decodes");
        assert_eq!(a.start_unix_s, i64::from(t0) + FIT_EPOCH_OFFSET_S);
        let p: Vec<_> = a.records.iter().map(|r| (r.elapsed_s, r.power_w)).collect();
        assert_eq!(
            p,
            [(0.0, Some(250.0)), (1.0, Some(260.0)), (2.0, Some(270.0))]
        );
        assert!((a.records[0].speed_mps - 5.5).abs() < 1e-9);
    }
}
