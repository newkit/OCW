//! FIT activity file writer (spec §2.8: every ride is saved as a FIT file).
//!
//! Writes the subset of the Garmin FIT protocol (2.0, little endian) that
//! training platforms such as Strava need for a virtual ride: `file_id`,
//! `device_info`, timer `event`s, one `record` per sample, the `lap`s (one
//! per [`Activity::laps`] segment), one `session` and the `activity`, with
//! the header and file CRCs.
//!
//! Pure and dependency-free, so it also builds for `wasm32-unknown-unknown`
//! (the web companion can offer FIT downloads with the same code).

/// Seconds between the Unix epoch and the FIT epoch (1989-12-31 00:00 UTC).
pub const FIT_EPOCH_OFFSET_S: i64 = 631_065_600;

/// FIT profile version written to the header (21.x).
const PROFILE_VERSION: u16 = 2132;
/// Protocol version 2.0.
const PROTOCOL_VERSION: u8 = 0x20;

/// `manufacturer` value "development".
const MANUFACTURER_DEVELOPMENT: u16 = 255;
const SPORT_CYCLING: u8 = 2;
const SUB_SPORT_VIRTUAL_ACTIVITY: u8 = 58;

// Global message numbers.
const MESG_FILE_ID: u16 = 0;
const MESG_SESSION: u16 = 18;
const MESG_LAP: u16 = 19;
const MESG_RECORD: u16 = 20;
const MESG_EVENT: u16 = 21;
const MESG_DEVICE_INFO: u16 = 23;
const MESG_ACTIVITY: u16 = 34;

// Event / event type values.
const EVENT_TIMER: u8 = 0;
const EVENT_SESSION: u8 = 8;
const EVENT_LAP: u8 = 9;
const EVENT_ACTIVITY: u8 = 26;
const EVENT_TYPE_START: u8 = 0;
const EVENT_TYPE_STOP: u8 = 1;
const EVENT_TYPE_STOP_ALL: u8 = 4;

/// One sample of the ride (typically 1 Hz).
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Record {
    /// Seconds since the start of the activity.
    pub elapsed_s: f64,
    /// Distance since the start in metres.
    pub distance_m: f64,
    /// Speed in m/s.
    pub speed_mps: f64,
    pub power_w: Option<f64>,
    pub cadence_rpm: Option<f64>,
    pub heart_rate_bpm: Option<f64>,
    pub altitude_m: Option<f64>,
    /// Latitude and longitude in degrees (virtual GPS track).
    pub position: Option<(f64, f64)>,
}

/// A finished activity.
#[derive(Debug, Clone, PartialEq)]
pub struct Activity {
    /// Start time as Unix seconds (UTC).
    pub start_unix_s: i64,
    /// Samples in time order.
    pub records: Vec<Record>,
    /// Name of the recording software (`file_id.product_name`).
    pub product_name: String,
    /// Software version × 100 (e.g. 10 for 0.10).
    pub software_version: u16,
    /// Lap marks: elapsed seconds at which a new lap begins (a workout
    /// step, an event's start or finish, an event lap). Marks outside the
    /// records or closer than a second to another are ignored; empty: the
    /// whole activity is one lap.
    pub laps: Vec<f64>,
}

impl Activity {
    /// The records of each lap. Neighbouring laps share the boundary
    /// record, so the laps' times and distances add up to the session's.
    pub fn lap_records(&self) -> Vec<&[Record]> {
        let r = &self.records;
        let (Some(first), Some(last)) = (r.first(), r.last()) else {
            return vec![r.as_slice()];
        };
        let mut marks: Vec<f64> = self
            .laps
            .iter()
            .copied()
            .filter(|&t| t > first.elapsed_s + 0.5 && t < last.elapsed_s - 0.5)
            .collect();
        marks.sort_by(f64::total_cmp);
        let mut out = Vec::new();
        let mut from = 0;
        for t in marks {
            // The first record at or after the mark starts the new lap.
            let i = r.partition_point(|x| x.elapsed_s < t - 1e-6);
            if i > from && i < r.len() - 1 {
                out.push(&r[from..=i]);
                from = i;
            }
        }
        out.push(&r[from..]);
        out
    }
}

/// Totals over an activity's records (lap and session messages).
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Summary {
    pub elapsed_s: f64,
    pub distance_m: f64,
    pub avg_speed_mps: f64,
    pub max_speed_mps: f64,
    pub avg_power_w: Option<f64>,
    pub max_power_w: Option<f64>,
    pub avg_heart_rate_bpm: Option<f64>,
    pub max_heart_rate_bpm: Option<f64>,
    pub avg_cadence_rpm: Option<f64>,
    pub max_cadence_rpm: Option<f64>,
    pub ascent_m: f64,
    pub descent_m: f64,
    /// Mechanical work in kJ; at ~24 % efficiency this is also ≈ kcal.
    pub work_kj: f64,
}

impl Summary {
    pub fn of(records: &[Record]) -> Self {
        let (Some(first), Some(last)) = (records.first(), records.last()) else {
            return Self::default();
        };
        let elapsed_s = (last.elapsed_s - first.elapsed_s).max(0.0);
        // Distance of these records (a lap starts part-way into the ride).
        let distance_m = (last.distance_m - first.distance_m).max(0.0);
        let stat = |f: fn(&Record) -> Option<f64>| {
            let v: Vec<f64> = records.iter().filter_map(f).collect();
            if v.is_empty() {
                (None, None)
            } else {
                let avg = v.iter().sum::<f64>() / v.len() as f64;
                (Some(avg), Some(v.iter().copied().fold(f64::MIN, f64::max)))
            }
        };
        let (avg_power_w, max_power_w) = stat(|r| r.power_w);
        let (avg_heart_rate_bpm, max_heart_rate_bpm) = stat(|r| r.heart_rate_bpm);
        let (avg_cadence_rpm, max_cadence_rpm) = stat(|r| r.cadence_rpm);
        let (mut ascent_m, mut descent_m, mut work_j) = (0.0, 0.0, 0.0);
        for w in records.windows(2) {
            if let (Some(a), Some(b)) = (w[0].altitude_m, w[1].altitude_m) {
                if b > a {
                    ascent_m += b - a;
                } else {
                    descent_m += a - b;
                }
            }
            let dt = (w[1].elapsed_s - w[0].elapsed_s).max(0.0);
            work_j += w[1].power_w.unwrap_or(0.0) * dt;
        }
        Self {
            elapsed_s,
            distance_m,
            avg_speed_mps: if elapsed_s > 0.0 {
                distance_m / elapsed_s
            } else {
                0.0
            },
            max_speed_mps: records.iter().map(|r| r.speed_mps).fold(0.0, f64::max),
            avg_power_w,
            max_power_w,
            avg_heart_rate_bpm,
            max_heart_rate_bpm,
            avg_cadence_rpm,
            max_cadence_rpm,
            ascent_m,
            descent_m,
            work_kj: work_j / 1000.0,
        }
    }
}

/// FIT CRC-16 (the SDK's nibble-table algorithm), continuing from `crc`.
pub fn crc16(mut crc: u16, bytes: &[u8]) -> u16 {
    const TABLE: [u16; 16] = [
        0x0000, 0xCC01, 0xD801, 0x1400, 0xF001, 0x3C00, 0x2800, 0xE401, 0xA001, 0x6C00, 0x7800,
        0xB401, 0x5000, 0x9C01, 0x8801, 0x4400,
    ];
    for &b in bytes {
        let tmp = TABLE[(crc & 0xF) as usize];
        crc = ((crc >> 4) & 0x0FFF) ^ tmp ^ TABLE[(b & 0xF) as usize];
        let tmp = TABLE[(crc & 0xF) as usize];
        crc = ((crc >> 4) & 0x0FFF) ^ tmp ^ TABLE[((b >> 4) & 0xF) as usize];
    }
    crc
}

/// Unix seconds → FIT `date_time` (clamped to the representable range).
pub fn fit_time(unix_s: i64) -> u32 {
    (unix_s - FIT_EPOCH_OFFSET_S).clamp(0, i64::from(u32::MAX - 1)) as u32
}

/// Degrees → semicircles (2^31 per 180°).
pub fn semicircles(deg: f64) -> i32 {
    (deg * (2f64.powi(31) / 180.0))
        .round()
        .clamp(f64::from(i32::MIN + 1), f64::from(i32::MAX - 1)) as i32
}

/// A field value; `None` variants write the FIT "invalid" value.
#[derive(Debug, Clone)]
enum Value {
    Enum(Option<u8>),
    U8(Option<u8>),
    U16(Option<u16>),
    U32(Option<u32>),
    U32z(u32),
    S32(Option<i32>),
    /// Fixed-size, NUL-terminated string field.
    Str(String, u8),
}

impl Value {
    fn size(&self) -> u8 {
        match self {
            Self::Enum(_) | Self::U8(_) => 1,
            Self::U16(_) => 2,
            Self::U32(_) | Self::U32z(_) | Self::S32(_) => 4,
            Self::Str(_, n) => *n,
        }
    }

    fn base_type(&self) -> u8 {
        match self {
            Self::Enum(_) => 0x00,
            Self::U8(_) => 0x02,
            Self::U16(_) => 0x84,
            Self::S32(_) => 0x85,
            Self::U32(_) => 0x86,
            Self::Str(..) => 0x07,
            Self::U32z(_) => 0x8C,
        }
    }

    fn write(&self, out: &mut Vec<u8>) {
        match self {
            Self::Enum(v) | Self::U8(v) => out.push(v.unwrap_or(0xFF)),
            Self::U16(v) => out.extend_from_slice(&v.unwrap_or(0xFFFF).to_le_bytes()),
            Self::U32(v) => out.extend_from_slice(&v.unwrap_or(0xFFFF_FFFF).to_le_bytes()),
            Self::U32z(v) => out.extend_from_slice(&v.to_le_bytes()),
            Self::S32(v) => out.extend_from_slice(&v.unwrap_or(0x7FFF_FFFF).to_le_bytes()),
            Self::Str(s, n) => {
                let n = usize::from(*n);
                let mut bytes: Vec<u8> = s.bytes().take(n - 1).collect();
                // Do not cut a multi-byte UTF-8 character in half.
                while std::str::from_utf8(&bytes).is_err() {
                    bytes.pop();
                }
                bytes.resize(n, 0);
                out.extend_from_slice(&bytes);
            }
        }
    }
}

/// Scaled unsigned values; out-of-range values become invalid.
fn u8_of(v: Option<f64>) -> Value {
    Value::U8(v.and_then(|v| {
        (v.is_finite() && (0.0..255.0).contains(&v.round())).then(|| v.round() as u8)
    }))
}
fn u16_of(v: Option<f64>, scale: f64, offset: f64) -> Value {
    Value::U16(v.and_then(|v| {
        let x = ((v + offset) * scale).round();
        (x.is_finite() && (0.0..65535.0).contains(&x)).then_some(x as u16)
    }))
}
fn u32_of(v: Option<f64>, scale: f64) -> Value {
    Value::U32(v.and_then(|v| {
        let x = (v * scale).round();
        (x.is_finite() && (0.0..4_294_967_295.0).contains(&x)).then_some(x as u32)
    }))
}

/// A message layout: global number and (field number, size, base type).
type Layout = (u16, Vec<(u8, u8, u8)>);

/// Builds the message stream: definitions are written when the layout of
/// local message type 0 changes (consecutive records share one).
struct Writer {
    data: Vec<u8>,
    current: Option<Layout>,
}

impl Writer {
    fn message(&mut self, global: u16, fields: &[(u8, Value)]) {
        let layout: Vec<(u8, u8, u8)> = fields
            .iter()
            .map(|(num, v)| (*num, v.size(), v.base_type()))
            .collect();
        if self.current.as_ref() != Some(&(global, layout.clone())) {
            // Definition message, local type 0, little endian.
            self.data.extend_from_slice(&[0x40, 0, 0]);
            self.data.extend_from_slice(&global.to_le_bytes());
            self.data.push(layout.len() as u8);
            for (num, size, base) in &layout {
                self.data.extend_from_slice(&[*num, *size, *base]);
            }
            self.current = Some((global, layout));
        }
        self.data.push(0x00);
        for (_, v) in fields {
            v.write(&mut self.data);
        }
    }
}

/// Encodes `activity` as a complete FIT file.
pub fn encode(activity: &Activity) -> Vec<u8> {
    let start = fit_time(activity.start_unix_s);
    let summary = Summary::of(&activity.records);
    let ts_of = |r: &Record| {
        start.saturating_add(r.elapsed_s.max(0.0).round().min(f64::from(u32::MAX)) as u32)
    };
    let end = activity.records.last().map_or(start, ts_of);
    let first_pos = activity.records.iter().find_map(|r| r.position);
    let mut w = Writer {
        data: Vec::new(),
        current: None,
    };
    let name_len = 32;
    w.message(
        MESG_FILE_ID,
        &[
            (0, Value::Enum(Some(4))), // type: activity
            (1, Value::U16(Some(MANUFACTURER_DEVELOPMENT))),
            (2, Value::U16(Some(0))),
            (3, Value::U32z(1)),
            (4, Value::U32(Some(start))),
            (8, Value::Str(activity.product_name.clone(), name_len)),
        ],
    );
    w.message(
        MESG_DEVICE_INFO,
        &[
            (253, Value::U32(Some(start))),
            (0, Value::U8(Some(0))), // device_index: creator
            (2, Value::U16(Some(MANUFACTURER_DEVELOPMENT))),
            (4, Value::U16(Some(0))),
            (5, Value::U16(Some(activity.software_version))),
            (27, Value::Str(activity.product_name.clone(), name_len)),
        ],
    );
    let event = |w: &mut Writer, ts: u32, event: u8, event_type: u8| {
        w.message(
            MESG_EVENT,
            &[
                (253, Value::U32(Some(ts))),
                (0, Value::Enum(Some(event))),
                (1, Value::Enum(Some(event_type))),
            ],
        );
    };
    event(&mut w, start, EVENT_TIMER, EVENT_TYPE_START);
    for r in &activity.records {
        let (lat, lon) = match r.position {
            Some((lat, lon)) => (Some(semicircles(lat)), Some(semicircles(lon))),
            None => (None, None),
        };
        w.message(
            MESG_RECORD,
            &[
                (253, Value::U32(Some(ts_of(r)))),
                (0, Value::S32(lat)),
                (1, Value::S32(lon)),
                (2, u16_of(r.altitude_m, 5.0, 500.0)),
                (3, u8_of(r.heart_rate_bpm)),
                (4, u8_of(r.cadence_rpm)),
                (5, u32_of(Some(r.distance_m), 100.0)),
                (6, u16_of(Some(r.speed_mps), 1000.0, 0.0)),
                (7, u16_of(r.power_w, 1.0, 0.0)),
            ],
        );
    }
    event(&mut w, end, EVENT_TIMER, EVENT_TYPE_STOP_ALL);

    let s = &summary;
    let time_ms = Some(s.elapsed_s);
    let (start_lat, start_lon) = match first_pos {
        Some((lat, lon)) => (Some(semicircles(lat)), Some(semicircles(lon))),
        None => (None, None),
    };
    let laps = activity.lap_records();
    for (i, records) in laps.iter().enumerate() {
        let l = Summary::of(records);
        let lap_start = records.first().map_or(start, ts_of);
        let lap_end = records.last().map_or(start, ts_of);
        let lap_time = Some(l.elapsed_s);
        let (lat, lon) = match records.iter().find_map(|r| r.position) {
            Some((lat, lon)) => (Some(semicircles(lat)), Some(semicircles(lon))),
            None => (None, None),
        };
        w.message(
            MESG_LAP,
            &[
                (254, Value::U16(Some(i as u16))),
                (253, Value::U32(Some(lap_end))),
                (0, Value::Enum(Some(EVENT_LAP))),
                (1, Value::Enum(Some(EVENT_TYPE_STOP))),
                (2, Value::U32(Some(lap_start))),
                (3, Value::S32(lat)),
                (4, Value::S32(lon)),
                (7, u32_of(lap_time, 1000.0)),
                (8, u32_of(lap_time, 1000.0)),
                (9, u32_of(Some(l.distance_m), 100.0)),
                (11, u16_of(Some(l.work_kj), 1.0, 0.0)),
                (13, u16_of(Some(l.avg_speed_mps), 1000.0, 0.0)),
                (14, u16_of(Some(l.max_speed_mps), 1000.0, 0.0)),
                (15, u8_of(l.avg_heart_rate_bpm)),
                (16, u8_of(l.max_heart_rate_bpm)),
                (17, u8_of(l.avg_cadence_rpm)),
                (18, u8_of(l.max_cadence_rpm)),
                (19, u16_of(l.avg_power_w, 1.0, 0.0)),
                (20, u16_of(l.max_power_w, 1.0, 0.0)),
                (21, u16_of(Some(l.ascent_m), 1.0, 0.0)),
                (22, u16_of(Some(l.descent_m), 1.0, 0.0)),
                (25, Value::Enum(Some(SPORT_CYCLING))),
                (39, Value::Enum(Some(SUB_SPORT_VIRTUAL_ACTIVITY))),
            ],
        );
    }
    w.message(
        MESG_SESSION,
        &[
            (254, Value::U16(Some(0))),
            (253, Value::U32(Some(end))),
            (0, Value::Enum(Some(EVENT_SESSION))),
            (1, Value::Enum(Some(EVENT_TYPE_STOP))),
            (2, Value::U32(Some(start))),
            (3, Value::S32(start_lat)),
            (4, Value::S32(start_lon)),
            (5, Value::Enum(Some(SPORT_CYCLING))),
            (6, Value::Enum(Some(SUB_SPORT_VIRTUAL_ACTIVITY))),
            (7, u32_of(time_ms, 1000.0)),
            (8, u32_of(time_ms, 1000.0)),
            (9, u32_of(Some(s.distance_m), 100.0)),
            (11, u16_of(Some(s.work_kj), 1.0, 0.0)),
            (14, u16_of(Some(s.avg_speed_mps), 1000.0, 0.0)),
            (15, u16_of(Some(s.max_speed_mps), 1000.0, 0.0)),
            (16, u8_of(s.avg_heart_rate_bpm)),
            (17, u8_of(s.max_heart_rate_bpm)),
            (18, u8_of(s.avg_cadence_rpm)),
            (19, u8_of(s.max_cadence_rpm)),
            (20, u16_of(s.avg_power_w, 1.0, 0.0)),
            (21, u16_of(s.max_power_w, 1.0, 0.0)),
            (22, u16_of(Some(s.ascent_m), 1.0, 0.0)),
            (23, u16_of(Some(s.descent_m), 1.0, 0.0)),
            (25, Value::U16(Some(0))),                 // first_lap_index
            (26, Value::U16(Some(laps.len() as u16))), // num_laps
        ],
    );
    w.message(
        MESG_ACTIVITY,
        &[
            (253, Value::U32(Some(end))),
            (0, u32_of(time_ms, 1000.0)),
            (1, Value::U16(Some(1))),               // num_sessions
            (2, Value::Enum(Some(0))),              // type: manual
            (3, Value::Enum(Some(EVENT_ACTIVITY))), // event
            (4, Value::Enum(Some(EVENT_TYPE_STOP))),
        ],
    );

    let mut out = Vec::with_capacity(w.data.len() + 16);
    out.push(14);
    out.push(PROTOCOL_VERSION);
    out.extend_from_slice(&PROFILE_VERSION.to_le_bytes());
    out.extend_from_slice(&(w.data.len() as u32).to_le_bytes());
    out.extend_from_slice(b".FIT");
    let header_crc = crc16(0, &out);
    out.extend_from_slice(&header_crc.to_le_bytes());
    out.extend_from_slice(&w.data);
    let file_crc = crc16(0, &out);
    out.extend_from_slice(&file_crc.to_le_bytes());
    out
}

#[cfg(test)]
mod tests {
    use fitparser::profile::MesgNum;
    use fitparser::{FitDataRecord, Value as V};

    use super::*;

    fn ride(n: usize, with_position: bool) -> Activity {
        let records = (0..n)
            .map(|i| {
                let t = i as f64;
                Record {
                    elapsed_s: t,
                    distance_m: 8.0 * t,
                    speed_mps: 8.0 + (t * 0.1).sin(),
                    power_w: Some(200.0 + (i % 50) as f64),
                    cadence_rpm: Some(85.0),
                    heart_rate_bpm: (i > 2).then_some(120.0 + (i % 30) as f64),
                    altitude_m: Some(455.0 + t * 0.2),
                    position: with_position.then_some((47.15 + t * 1e-5, 9.55 + t * 2e-5)),
                }
            })
            .collect();
        Activity {
            start_unix_s: 1_762_862_400, // 2025-11-11 12:00 UTC
            records,
            product_name: "Open Cycling World".into(),
            software_version: 10,
            laps: Vec::new(),
        }
    }

    fn num(v: &V) -> Option<f64> {
        Some(match v {
            V::UInt8(x) | V::UInt8z(x) | V::Byte(x) | V::Enum(x) => f64::from(*x),
            V::SInt8(x) => f64::from(*x),
            V::UInt16(x) | V::UInt16z(x) => f64::from(*x),
            V::SInt16(x) => f64::from(*x),
            V::UInt32(x) | V::UInt32z(x) => f64::from(*x),
            V::SInt32(x) => f64::from(*x),
            V::Float32(x) => f64::from(*x),
            V::Float64(x) => *x,
            _ => return None,
        })
    }

    fn field<'a>(r: &'a FitDataRecord, name: &str) -> Option<&'a V> {
        r.fields()
            .iter()
            .find(|f| f.name() == name)
            .map(|f| f.value())
    }

    fn of_kind(msgs: &[FitDataRecord], kind: MesgNum) -> Vec<&FitDataRecord> {
        msgs.iter().filter(|m| m.kind() == kind).collect()
    }

    #[test]
    fn crc_matches_the_sdk_check_value() {
        // CRC-16/ARC ("123456789") – the FIT CRC is this polynomial.
        assert_eq!(crc16(0, b"123456789"), 0xBB3D);
        // A buffer followed by its CRC checks to zero.
        let mut b = b"hello fit".to_vec();
        let c = crc16(0, &b);
        b.extend_from_slice(&c.to_le_bytes());
        assert_eq!(crc16(0, &b), 0);
    }

    #[test]
    fn header_and_file_crc() {
        let bytes = encode(&ride(10, true));
        assert_eq!(bytes[0], 14);
        assert_eq!(&bytes[8..12], b".FIT");
        let data_len = u32::from_le_bytes(bytes[4..8].try_into().unwrap()) as usize;
        assert_eq!(bytes.len(), 14 + data_len + 2);
        assert_eq!(crc16(0, &bytes[..14]), 0);
        assert_eq!(crc16(0, &bytes), 0);
    }

    #[test]
    fn round_trip_through_a_fit_reader() {
        let activity = ride(600, true);
        let bytes = encode(&activity);
        // fitparser checks the header and file CRCs.
        let msgs = fitparser::from_bytes(&bytes).expect("valid FIT file");

        let file_id = of_kind(&msgs, MesgNum::FileId);
        assert_eq!(file_id.len(), 1);
        assert_eq!(
            field(file_id[0], "type"),
            Some(&V::String("activity".into()))
        );
        assert_eq!(
            field(file_id[0], "product_name"),
            Some(&V::String("Open Cycling World".into()))
        );
        assert_eq!(of_kind(&msgs, MesgNum::DeviceInfo).len(), 1);
        assert_eq!(of_kind(&msgs, MesgNum::Event).len(), 2);

        let records = of_kind(&msgs, MesgNum::Record);
        assert_eq!(records.len(), 600);
        for (r, want) in records.iter().zip(&activity.records) {
            let get = |name: &str| field(r, name).and_then(num);
            assert_eq!(get("power"), want.power_w);
            assert_eq!(get("cadence"), want.cadence_rpm);
            assert_eq!(get("heart_rate"), want.heart_rate_bpm);
            assert!((get("distance").unwrap() - want.distance_m).abs() < 0.01);
            let speed = get("speed").or_else(|| get("enhanced_speed")).unwrap();
            assert!((speed - want.speed_mps).abs() < 0.001);
            let alt = get("altitude")
                .or_else(|| get("enhanced_altitude"))
                .unwrap();
            assert!((alt - want.altitude_m.unwrap()).abs() <= 0.1);
            let (lat, lon) = want.position.unwrap();
            let semi = 180.0 / 2f64.powi(31);
            assert!((get("position_lat").unwrap() * semi - lat).abs() < 1e-6);
            assert!((get("position_long").unwrap() * semi - lon).abs() < 1e-6);
        }
        // Timestamps advance one second per record from the start time.
        let ts = |r: &FitDataRecord| match field(r, "timestamp") {
            Some(V::Timestamp(t)) => t.timestamp(),
            other => panic!("timestamp: {other:?}"),
        };
        assert_eq!(ts(records[0]), activity.start_unix_s);
        assert_eq!(ts(records[599]), activity.start_unix_s + 599);

        let session = of_kind(&msgs, MesgNum::Session);
        assert_eq!(session.len(), 1);
        let s = session[0];
        assert_eq!(field(s, "sport"), Some(&V::String("cycling".into())));
        assert_eq!(
            field(s, "sub_sport"),
            Some(&V::String("virtual_activity".into()))
        );
        let get = |name: &str| field(s, name).and_then(num);
        assert!((get("total_elapsed_time").unwrap() - 599.0).abs() < 1e-6);
        assert!((get("total_distance").unwrap() - 8.0 * 599.0).abs() < 0.01);
        assert_eq!(get("max_power"), Some(249.0));
        assert_eq!(get("avg_cadence"), Some(85.0));
        assert_eq!(get("num_laps"), Some(1.0));
        assert_eq!(of_kind(&msgs, MesgNum::Lap).len(), 1);
        let act = of_kind(&msgs, MesgNum::Activity);
        assert_eq!(act.len(), 1);
        assert_eq!(field(act[0], "num_sessions").and_then(num), Some(1.0));
    }

    #[test]
    fn lap_marks_split_the_ride_into_laps() {
        let mut a = ride(600, true);
        // Unsorted, one before the start, one past the end, one too close.
        a.laps = vec![400.0, 120.0, -5.0, 900.0, 599.8];
        let parts = a.lap_records();
        assert_eq!(parts.len(), 3);
        assert_eq!(parts[0].last().unwrap().elapsed_s, 120.0);
        assert_eq!(parts[1].first().unwrap().elapsed_s, 120.0);
        assert_eq!(parts[2].first().unwrap().elapsed_s, 400.0);
        let msgs = fitparser::from_bytes(&encode(&a)).expect("valid FIT file");
        let laps = of_kind(&msgs, MesgNum::Lap);
        assert_eq!(laps.len(), 3);
        let get = |m: &FitDataRecord, name: &str| field(m, name).and_then(num).unwrap();
        let times: Vec<f64> = laps.iter().map(|l| get(l, "total_elapsed_time")).collect();
        assert_eq!(times, [120.0, 280.0, 199.0]);
        // The laps' distances add up to the session's.
        let sum: f64 = laps.iter().map(|l| get(l, "total_distance")).sum();
        let session = of_kind(&msgs, MesgNum::Session)[0];
        assert!((sum - get(session, "total_distance")).abs() < 0.05);
        assert_eq!(get(session, "num_laps"), 3.0);
        // Each lap starts where the previous one ended.
        let ts = |m: &FitDataRecord, name: &str| match field(m, name) {
            Some(V::Timestamp(t)) => t.timestamp(),
            other => panic!("{name}: {other:?}"),
        };
        assert_eq!(ts(laps[1], "start_time"), ts(laps[0], "timestamp"));
        assert_eq!(ts(laps[2], "start_time"), a.start_unix_s + 400);
    }

    #[test]
    fn missing_values_are_left_out() {
        let mut a = ride(5, false);
        for r in &mut a.records {
            r.power_w = None;
            r.heart_rate_bpm = None;
            r.cadence_rpm = Some(400.0); // out of range for uint8
        }
        let msgs = fitparser::from_bytes(&encode(&a)).expect("valid FIT file");
        for r in of_kind(&msgs, MesgNum::Record) {
            for name in [
                "power",
                "heart_rate",
                "cadence",
                "position_lat",
                "position_long",
            ] {
                assert!(field(r, name).is_none(), "{name} in {:?}", r.fields());
            }
        }
    }

    #[test]
    fn empty_activity_is_still_a_valid_file() {
        let mut a = ride(0, false);
        a.product_name = "ä".repeat(40); // longer than the field, multi-byte
        let msgs = fitparser::from_bytes(&encode(&a)).expect("valid FIT file");
        assert_eq!(of_kind(&msgs, MesgNum::Record).len(), 0);
        assert_eq!(of_kind(&msgs, MesgNum::Session).len(), 1);
    }

    #[test]
    fn summary_totals() {
        let s = Summary::of(&ride(11, false).records);
        assert_eq!(s.elapsed_s, 10.0);
        assert_eq!(s.distance_m, 80.0);
        assert!((s.avg_speed_mps - 8.0).abs() < 1e-9);
        assert!((s.ascent_m - 2.0).abs() < 1e-9 && s.descent_m == 0.0);
        assert_eq!(s.max_power_w, Some(210.0));
        // Work: powers 201…210 W for one second each.
        assert!((s.work_kj - 2.055).abs() < 1e-9);
        assert_eq!(Summary::of(&[]), Summary::default());
    }

    #[test]
    fn time_and_angle_conversions() {
        assert_eq!(fit_time(FIT_EPOCH_OFFSET_S), 0);
        assert_eq!(fit_time(0), 0);
        assert_eq!(semicircles(180.0), i32::MAX - 1);
        assert_eq!(semicircles(90.0), 1 << 30);
        assert_eq!(semicircles(-90.0), -(1 << 30));
    }
}
