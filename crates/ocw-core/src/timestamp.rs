//! ISO 8601 timestamps for screenshots and videos (spec §3.2, §11.4) and
//! the clock they read.
//!
//! Every picture and video the app takes shows its date and time in the
//! HUD's top-right corner and carries it in the file's metadata. Pure
//! functions: the caller supplies the wall clock (Unix time) and the local
//! UTC offset, so this builds for `wasm32-unknown-unknown` and is tested
//! without a clock.

/// Civil (proleptic Gregorian) date and time of `unix_s` seconds after
/// 1970-01-01T00:00:00 (H. Hinnant's days-from-civil inverse):
/// `(year, month 1..=12, day 1..=31, hour, minute, second)`.
pub fn civil(unix_s: i64) -> (i64, u32, u32, u32, u32, u32) {
    let days = unix_s.div_euclid(86_400);
    let secs = unix_s.rem_euclid(86_400);
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    (
        year,
        month as u32,
        day as u32,
        (secs / 3600) as u32,
        (secs % 3600 / 60) as u32,
        (secs % 60) as u32,
    )
}

/// A UTC offset as ISO 8601 `+01:00`, `-03:30`, `+00:00` (minutes; the
/// seconds of historic offsets are dropped).
pub fn format_offset(offset_s: i32) -> String {
    let sign = if offset_s < 0 { '-' } else { '+' };
    let m = offset_s.unsigned_abs() / 60;
    format!("{sign}{:02}:{:02}", m / 60, m % 60)
}

/// Local time with its UTC offset, to the second:
/// `2026-11-12T22:14:05+01:00` (`unix_s` is UTC, `offset_s` local − UTC).
pub fn iso8601_local(unix_s: i64, offset_s: i32) -> String {
    let (y, mo, d, h, mi, s) = civil(unix_s + i64::from(offset_s));
    format!(
        "{y:04}-{mo:02}-{d:02}T{h:02}:{mi:02}:{s:02}{}",
        format_offset(offset_s)
    )
}

/// UTC with a `Z`, to the second: `2026-11-12T21:14:05Z` (ffmpeg's
/// `creation_time`).
pub fn iso8601_utc(unix_s: i64) -> String {
    let (y, mo, d, h, mi, s) = civil(unix_s);
    format!("{y:04}-{mo:02}-{d:02}T{h:02}:{mi:02}:{s:02}Z")
}

/// Local time in EXIF's form, `2026:11:12 22:14:05` (the offset goes
/// into its own tag, `OffsetTimeOriginal`, as [`format_offset`]).
pub fn exif_local(unix_s: i64, offset_s: i32) -> String {
    let (y, mo, d, h, mi, s) = civil(unix_s + i64::from(offset_s));
    format!("{y:04}:{mo:02}:{d:02} {h:02}:{mi:02}:{s:02}")
}

/// Which time the stamp shows.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum MediaClock {
    /// The wall clock (normal play, screenshots, live recording with V).
    Wall,
    /// Frame capture (`--capture`): the wall clock when the capture run
    /// started plus the game time since, which advances a fixed step per
    /// frame, so the time runs smoothly in the video however slowly the
    /// frames render.
    Stepped {
        /// Wall clock (Unix seconds) at game time `start_game_s`.
        start_unix_s: f64,
        start_game_s: f64,
    },
}

impl MediaClock {
    /// The stamp's time in Unix seconds, given the wall clock and the game
    /// time now.
    pub fn unix_s(&self, wall_unix_s: f64, game_s: f64) -> f64 {
        match *self {
            Self::Wall => wall_unix_s,
            Self::Stepped {
                start_unix_s,
                start_game_s,
            } => start_unix_s + (game_s - start_game_s).max(0.0),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 2026-11-12T21:14:05Z.
    const T: i64 = 1_794_518_045;

    #[test]
    fn civil_dates() {
        assert_eq!(civil(0), (1970, 1, 1, 0, 0, 0));
        assert_eq!(civil(-1), (1969, 12, 31, 23, 59, 59));
        assert_eq!(civil(951_782_400), (2000, 2, 29, 0, 0, 0));
        assert_eq!(civil(T), (2026, 11, 12, 21, 14, 5));
    }

    #[test]
    fn offsets() {
        assert_eq!(format_offset(0), "+00:00");
        assert_eq!(format_offset(3600), "+01:00");
        assert_eq!(format_offset(7200), "+02:00");
        assert_eq!(format_offset(-12_600), "-03:30");
        assert_eq!(format_offset(20_700), "+05:45");
        assert_eq!(format_offset(-36_000), "-10:00");
        // Historic offsets with seconds (e.g. Amsterdam 1937, +00:19:32).
        assert_eq!(format_offset(1172), "+00:19");
    }

    #[test]
    fn local_and_utc_stamps() {
        assert_eq!(iso8601_utc(T), "2026-11-12T21:14:05Z");
        assert_eq!(iso8601_local(T, 0), "2026-11-12T21:14:05+00:00");
        assert_eq!(iso8601_local(T, 3600), "2026-11-12T22:14:05+01:00");
        // The offset can move the date, both ways.
        assert_eq!(iso8601_local(T, 10 * 3600), "2026-11-13T07:14:05+10:00");
        assert_eq!(
            iso8601_local(T - 21 * 3600, -12_600),
            "2026-11-11T20:44:05-03:30"
        );
        assert_eq!(iso8601_local(0, 0).len(), 25);
        assert_eq!(exif_local(T, 3600), "2026:11:12 22:14:05");
        assert_eq!(exif_local(T, 10 * 3600), "2026:11:13 07:14:05");
    }

    #[test]
    fn wall_and_stepped_clocks() {
        assert_eq!(MediaClock::Wall.unix_s(100.5, 7.0), 100.5);
        let c = MediaClock::Stepped {
            start_unix_s: 1000.0,
            start_game_s: 2.0,
        };
        // Game time only: the wall clock is ignored.
        assert_eq!(c.unix_s(5000.0, 2.0), 1000.0);
        assert_eq!(c.unix_s(1.0, 3.04), 1001.04);
        // 25 fps frames: one second of video is one second of stamp.
        let frames: Vec<i64> = (0..=50)
            .map(|n| c.unix_s(0.0, 2.0 + f64::from(n) / 25.0).floor() as i64)
            .collect();
        assert_eq!(frames[0], 1000);
        assert_eq!(frames[24], 1000);
        assert_eq!(frames[25], 1001);
        assert_eq!(frames[50], 1002);
        assert_eq!(c.unix_s(0.0, 0.0), 1000.0, "never before the start");
    }
}
