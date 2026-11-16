//! FIT course files (spec §14 Phase 16, 16D export, 16F E2): a route for
//! head units – `file_id` of type *course*, the `course` message with its
//! name, one `lap`, the timer events and one `record` per route point
//! (position, altitude, distance; times at a steady speed). [`crate::decode`]
//! reads such files back (their records), so a course can also be
//! imported.

use crate::{
    MANUFACTURER_DEVELOPMENT, PROFILE_VERSION, PROTOCOL_VERSION, SPORT_CYCLING, Value, Writer,
    crc16, fit_time, semicircles, u16_of, u32_of,
};

const MESG_FILE_ID: u16 = 0;
const MESG_LAP: u16 = 19;
const MESG_RECORD: u16 = 20;
const MESG_EVENT: u16 = 21;
const MESG_COURSE: u16 = 31;
const FILE_TYPE_COURSE: u8 = 6;

/// One point of a course.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CoursePoint {
    pub lat: f64,
    pub lon: f64,
    pub altitude_m: Option<f64>,
    /// Distance from the start (m).
    pub distance_m: f64,
}

/// A course to write.
#[derive(Debug, Clone, PartialEq)]
pub struct Course {
    /// Cut to 15 bytes (most head units show no more).
    pub name: String,
    pub points: Vec<CoursePoint>,
    /// The time of the first point (Unix s; a course needs one).
    pub start_unix_s: i64,
    /// The speed the records' times assume (m/s).
    pub speed_mps: f64,
}

/// Encodes `course` as a complete FIT course file.
pub fn encode_course(course: &Course) -> Vec<u8> {
    let start = fit_time(course.start_unix_s);
    let speed = course.speed_mps.max(0.5);
    let ts = |d: f64| start.saturating_add((d.max(0.0) / speed).round() as u32);
    let total = course.points.last().map_or(0.0, |p| p.distance_m);
    let end = ts(total);
    let mut w = Writer {
        data: Vec::new(),
        current: None,
    };
    w.message(
        MESG_FILE_ID,
        &[
            (0, Value::Enum(Some(FILE_TYPE_COURSE))),
            (1, Value::U16(Some(MANUFACTURER_DEVELOPMENT))),
            (2, Value::U16(Some(0))),
            (3, Value::U32z(1)),
            (4, Value::U32(Some(start))),
        ],
    );
    w.message(
        MESG_COURSE,
        &[
            (4, Value::Enum(Some(SPORT_CYCLING))),
            (5, Value::Str(course.name.clone(), 16)),
        ],
    );
    let pos = |p: Option<&CoursePoint>| match p {
        Some(p) => (Some(semicircles(p.lat)), Some(semicircles(p.lon))),
        None => (None, None),
    };
    let (slat, slon) = pos(course.points.first());
    let (elat, elon) = pos(course.points.last());
    let time = Some(total / speed);
    w.message(
        MESG_LAP,
        &[
            (253, Value::U32(Some(end))),
            (2, Value::U32(Some(start))),
            (3, Value::S32(slat)),
            (4, Value::S32(slon)),
            (5, Value::S32(elat)),
            (6, Value::S32(elon)),
            (7, u32_of(time, 1000.0)),
            (8, u32_of(time, 1000.0)),
            (9, u32_of(Some(total), 100.0)),
        ],
    );
    let event = |w: &mut Writer, t: u32, event_type: u8| {
        w.message(
            MESG_EVENT,
            &[
                (253, Value::U32(Some(t))),
                (0, Value::Enum(Some(0))), // timer
                (1, Value::Enum(Some(event_type))),
            ],
        );
    };
    event(&mut w, start, 0); // start
    for p in &course.points {
        w.message(
            MESG_RECORD,
            &[
                (253, Value::U32(Some(ts(p.distance_m)))),
                (0, Value::S32(Some(semicircles(p.lat)))),
                (1, Value::S32(Some(semicircles(p.lon)))),
                (2, u16_of(p.altitude_m, 5.0, 500.0)),
                (5, u32_of(Some(p.distance_m), 100.0)),
            ],
        );
    }
    event(&mut w, end, 9); // stop_disable_all
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
    use super::*;
    use fitparser::profile::MesgNum;

    fn course() -> Course {
        Course {
            name: "Ada's grid loop, long name".into(),
            points: (0..50)
                .map(|i| CoursePoint {
                    lat: 47.14 + f64::from(i) * 1e-4,
                    lon: 9.52,
                    altitude_m: Some(450.0 + f64::from(i)),
                    distance_m: f64::from(i) * 11.1,
                })
                .collect(),
            start_unix_s: 1_767_225_600,
            speed_mps: 7.0,
        }
    }

    #[test]
    fn a_course_reads_back_in_our_reader() {
        let c = course();
        let bytes = encode_course(&c);
        let a = crate::decode(&bytes).unwrap();
        assert_eq!(a.records.len(), 50);
        for (r, p) in a.records.iter().zip(&c.points) {
            let (lat, lon) = r.position.unwrap();
            assert!((lat - p.lat).abs() < 1e-6 && (lon - p.lon).abs() < 1e-6);
            assert!((r.altitude_m.unwrap() - p.altitude_m.unwrap()).abs() <= 0.2);
            assert!((r.distance_m - p.distance_m).abs() < 0.01);
        }
    }

    #[test]
    fn an_independent_reader_sees_a_course_with_its_name() {
        let bytes = encode_course(&course());
        let records = fitparser::from_bytes(&bytes).unwrap();
        let file_id = records
            .iter()
            .find(|r| r.kind() == MesgNum::FileId)
            .unwrap();
        let ty = file_id
            .fields()
            .iter()
            .find(|f| f.name() == "type")
            .unwrap();
        assert_eq!(ty.value().to_string(), "course");
        let c = records
            .iter()
            .find(|r| r.kind() == MesgNum::Course)
            .unwrap();
        let name = c.fields().iter().find(|f| f.name() == "name").unwrap();
        assert_eq!(name.value().to_string(), "Ada's grid loop");
        assert_eq!(
            records
                .iter()
                .filter(|r| r.kind() == MesgNum::Record)
                .count(),
            50
        );
    }
}
