//! World → Time → Calendar and World → Time → Season (spec §14 Phase 9,
//! S9, 9J; decisions Q11, Q12), Bevy-free and shared by the desktop and the
//! browser (B6): the month grid with each day's look at the rider's place
//! and altitude, the moon's main phases and the sun of the focused day,
//! the date shortcuts and the seasons as date jumps.
//!
//! The page is [`Page::Calendar`] with its [`Cal`] (the month shown, the
//! day to open on, the stepper's time of day); the model
//! (`menu::model`) moves the focus with [`navigate`]. Every change of the
//! clock is a `time date …` console line the server checks (§6.1, Q2):
//! riders without the right see the calendar read-only, dimmed.
//!
//! Items: 0 – 41 the days ([`ocw_sky::calendar::month_grid`]), then the
//! buttons over the grid ([`YEAR_PREV`] … [`YEAR_NEXT`]), the time stepper
//! ([`TIME`]), Select ([`SELECT`]: the selected day with the stepper's
//! time, then back to World → Time, like a date picker's OK) and Back
//! ([`BACK`], the model's slot after the rows: cancels, the date kept).

use std::sync::Arc;

use ocw_render::menu::{CalendarDay, MenuCalendar};
use ocw_sky::Zone;
use ocw_sky::calendar::{self as cal, Date, Look, Season, SeasonYear};
use ocw_sky::moon::Phase;
use ocw_sky::season::SeasonModel;

use super::pages::{Activate, Page, Row};

/// The calendar page's state (part of its [`Page`]): the month shown, the
/// day the focus opens on, the time of day of the stepper (`None`: the
/// clock's, kept).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Cal {
    pub year: i32,
    pub month: u32,
    pub day: u32,
    pub minute: Option<u16>,
}

impl Cal {
    /// The month of `d`, the focus on `d`.
    pub fn at(d: Date) -> Self {
        let d = cal::clamp(d);
        Self {
            year: d.0,
            month: d.1,
            day: d.2,
            minute: None,
        }
    }

    pub fn grid(&self) -> [Date; cal::GRID_DAYS] {
        cal::month_grid(self.year, self.month)
    }

    /// The date of day cell `i`.
    pub fn date_of(&self, i: usize) -> Option<Date> {
        (i < DAYS).then(|| self.grid()[i])
    }

    pub fn cell_of(&self, d: Date) -> Option<usize> {
        cal::grid_index(self.year, self.month, d)
    }

    /// The selected day: the one the focus was last on (by keys, a click,
    /// a date jump), which Select sets.
    pub fn selected(&self) -> Date {
        (self.year, self.month, self.day)
    }

    fn in_month(&self, d: Date) -> bool {
        (d.0, d.1) == (self.year, self.month) && d >= cal::FIRST && d <= cal::LAST
    }
}

/// Columns of the grid.
pub const COLUMNS: usize = 7;
/// The day cells.
pub const DAYS: usize = ocw_render::menu::CALENDAR_DAYS;
/// The buttons over the grid.
pub const YEAR_PREV: usize = DAYS;
pub const MONTH_PREV: usize = DAYS + 1;
pub const TODAY: usize = DAYS + 2;
pub const DATES: usize = DAYS + 3;
pub const SEASON: usize = DAYS + 4;
pub const MONTH_NEXT: usize = DAYS + 5;
pub const YEAR_NEXT: usize = DAYS + 6;
/// Under the grid: the time stepper, Select (the selected day with the
/// stepper's time, then back), Back (cancel).
pub const TIME: usize = DAYS + 7;
pub const SELECT: usize = DAYS + 8;
pub const BACK: usize = DAYS + 9;
/// Rows of the page (Back is the model's).
pub const ROWS: usize = BACK;
/// The stepper's steps (minutes): ← / →, Shift for one minute.
pub const STEP_MIN: i32 = 15;

/// What the calendar knows of the clock and the place (refreshed by the
/// app every frame; the season year and the moon only when their key
/// changes).
#[derive(Debug, Clone, Default)]
pub struct CalendarInfo {
    /// The world clock's local date and time of day (minutes).
    pub date: Option<Date>,
    pub minute: u16,
    /// Today: the real date at the place.
    pub today: Option<Date>,
    pub may_change: bool,
    /// Who sets the time ("you", "the host", "the organizer").
    pub control: String,
    pub zone: Option<Zone>,
    /// The rider's place (lat, lon) and altitude (m).
    pub place: (f64, f64),
    pub alt_m: f64,
    pub model: Option<Arc<SeasonModel>>,
    pub year: Option<Arc<SeasonYear>>,
    /// The look of the world date with real weather's measured snow
    /// (S7 / S8), if it differs from the climatology's.
    pub measured: Option<Look>,
    /// The moon's main phases on the days of the month shown: (month,
    /// day, phase), and the month they are for.
    pub moon: Vec<(Date, Phase)>,
    pub moon_for: Option<(i32, u32)>,
    /// The focused day's line (sun, leaves, snow line, moon) and its date.
    pub line: Option<(Date, String)>,
}

impl CalendarInfo {
    /// The look of `d` at the rider (the measured snow on the world date).
    pub fn look(&self, d: Date) -> Option<Look> {
        if Some(d) == self.date
            && let Some(m) = self.measured
        {
            return Some(m);
        }
        self.year.as_ref().map(|y| y.look(d))
    }

    /// The moon's main phases of the month view of `y`-`m` in the place's
    /// zone (computed again when the month changes).
    pub fn ensure_moon(&mut self, y: i32, m: u32) {
        if self.moon_for == Some((y, m)) {
            return;
        }
        let Some(zone) = &self.zone else { return };
        self.moon = moon_days(y, m, zone);
        self.moon_for = Some((y, m));
    }

    /// The focused day's line, computed again for another day.
    pub fn ensure_line(&mut self, d: Date) {
        if self.line.as_ref().is_some_and(|(at, _)| *at == d) {
            return;
        }
        let text = self.day_line(d);
        self.line = Some((d, text));
    }

    /// "sunrise 07:33 · sunset 16:47 · day 9 h 14 min · beech bare · snow
    /// line 1,350 m · full moon" (spec 9J's tooltip line).
    pub fn day_line(&self, d: Date) -> String {
        let mut parts = Vec::new();
        if let Some(zone) = &self.zone {
            let sun = ocw_sky::sun::day(d, self.place.0, self.place.1);
            let hm = |t: Option<i64>| t.map_or_else(|| "-".to_owned(), |t| zone.local(t).hh_mm());
            parts.push(format!("sunrise {}", hm(sun.sun.morning)));
            parts.push(format!("sunset {}", hm(sun.sun.evening)));
            if let (Some(a), Some(b)) = (sun.sun.morning, sun.sun.evening) {
                let min = (b - a).max(0) / 60_000;
                parts.push(format!("day {} h {:02} min", min / 60, min % 60));
            }
        }
        if let (Some(y), Some(look)) = (&self.year, self.look(d)) {
            parts.push(look_phrase(look, cal::evergreen(&y.climate)).into());
        }
        if let Some(model) = &self.model {
            let day = model.day(f64::from(cal::day_of_year(d)));
            parts.push(match day.snow_line_m() {
                Some(h) if h <= ocw_sky::season::MIN_ALT_M => "snow everywhere".into(),
                Some(h) => format!("snow line {} m", thousands(h)),
                None => "no snow".into(),
            });
        }
        if let Some((_, p)) = self.moon.iter().find(|(at, _)| *at == d) {
            parts.push(cal::phase_name(*p).into());
        }
        parts.join(" · ")
    }
}

/// "1,350".
fn thousands(h: f64) -> String {
    let n = (h / 10.0).round() as i64 * 10;
    if n.abs() >= 1000 {
        format!("{},{:03}", n / 1000, (n % 1000).abs())
    } else {
        n.to_string()
    }
}

/// The leaves (or meadows, where trees are evergreen) of a look.
pub fn look_phrase(look: Look, evergreen: bool) -> &'static str {
    match (look, evergreen) {
        (Look::Snow, _) => "lying snow",
        (Look::Dry, _) => "meadows dry",
        (Look::Summer, true) => "meadows green",
        (Look::Fresh, _) => "beech in fresh leaf",
        (Look::Summer, false) => "beech in full leaf",
        (Look::Autumn, _) => "beech in autumn colours",
        (Look::Bare, _) => "beech bare",
    }
}

/// The main phases of the moon on the days of the month view of `y`-`m`
/// (local dates in `zone`).
pub fn moon_days(y: i32, m: u32, zone: &Zone) -> Vec<(Date, Phase)> {
    let g = cal::month_grid(y, m);
    let start = cal::day_number(g[0]) * ocw_sky::time::MS_PER_DAY - 14 * 3_600_000;
    let end =
        (cal::day_number(g[cal::GRID_DAYS - 1]) + 1) * ocw_sky::time::MS_PER_DAY + 14 * 3_600_000;
    cal::moon_phases(start, end)
        .into_iter()
        .map(|(t, p)| {
            let l = zone.local(t);
            ((l.year, l.month, l.day), p)
        })
        .filter(|(d, _)| g.contains(d))
        .collect()
}

/// "14:20".
fn hh_mm(minute: u16) -> String {
    format!("{:02}:{:02}", minute / 60, minute % 60)
}

/// The `time date` line of `d` with the stepper's time (none: the time of
/// day kept).
pub fn date_line(d: Date, minute: Option<u16>) -> String {
    match minute {
        Some(m) => format!("time date {} {}", cal::iso(d), hh_mm(m)),
        None => format!("time date {}", cal::iso(d)),
    }
}

/// The date of a `time date …` line (the model follows it).
pub fn line_date(line: &str) -> Option<Date> {
    let rest = line.strip_prefix("time date ")?;
    let d = rest.split_whitespace().next()?;
    let mut it = d.splitn(3, '-');
    let y = it.next()?.parse().ok()?;
    let m = it.next()?.parse().ok()?;
    let day = it.next()?.parse().ok()?;
    Some((y, m, day))
}

/// The stepper's time: its own, else the clock's.
fn shown_minute(c: &Cal, info: &CalendarInfo) -> u16 {
    c.minute.unwrap_or(info.minute)
}

/// A day of the range 1900 – 2100 (the others cannot be set).
fn in_range(d: Date) -> bool {
    d >= cal::FIRST && d <= cal::LAST
}

/// "Select Tue 17 Nov 2026".
pub fn select_label(d: Date) -> String {
    let wd = cal::WEEKDAYS[cal::weekday(d) as usize];
    format!("Select {wd} {}", cal::short_date(d))
}

/// The rows of the calendar page: the 42 days, the buttons, the time,
/// Select.
pub fn rows(c: &Cal, info: &CalendarInfo) -> Vec<Row> {
    let set = |line: String| {
        Some(if info.may_change {
            Activate::Command(line)
        } else {
            Activate::Disabled
        })
    };
    let mut out: Vec<Row> = c
        .grid()
        .into_iter()
        .map(|d| Row {
            label: d.2.to_string(),
            value: None,
            left: None,
            right: None,
            confirm: if !in_range(d) {
                Some(Activate::Disabled)
            } else {
                set(date_line(d, c.minute))
            },
        })
        .collect();
    let button = |label: &str, confirm: Option<Activate>| Row {
        label: label.into(),
        value: None,
        left: None,
        right: None,
        confirm,
    };
    let prev = cal::add_months((c.year, c.month, 1), -1);
    let next = cal::add_months((c.year, c.month, 1), 1);
    let short = |m: u32| &cal::MONTHS[(m - 1) as usize][..3];
    out.push(button(&format!("« {}", c.year - 1), None));
    out.push(button(&format!("‹ {}", short(prev.1)), None));
    out.push(button(
        "Today",
        info.today.and_then(|t| set(date_line(t, c.minute))),
    ));
    out.push(button(
        "Dates",
        Some(Activate::Open(Page::CalendarDates(*c))),
    ));
    out.push(button("Season", Some(Activate::Open(Page::Season))));
    out.push(button(&format!("{} ›", short(next.1)), None));
    out.push(button(&format!("{} »", c.year + 1), None));
    let minute = shown_minute(c, info);
    out.push(Row {
        label: "Time".into(),
        value: Some(hh_mm(minute)),
        left: Some(String::new()),
        right: Some(String::new()),
        confirm: if info.may_change {
            Some(Activate::Command(format!("time {}", hh_mm(minute))))
        } else {
            Some(Activate::Disabled)
        },
    });
    // Select: the selected day with the stepper's time (none: the time of
    // day kept), then back to World → Time.
    let sel = c.selected();
    out.push(button(
        &select_label(sel),
        Some(if info.may_change && in_range(sel) {
            Activate::CommandBack(date_line(sel, c.minute))
        } else {
            Activate::Disabled
        }),
    ));
    out
}

/// Where the focus goes (and which month shows) after a key on the
/// calendar page; `rows` are the page's rows (the stepper's shown time).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Nav {
    Left,
    Right,
    Up,
    Down,
    /// Page Up / Down, the shoulder buttons, a swipe: a month.
    Month(i32),
    /// The « » buttons: a year.
    Year(i32),
    /// The time stepper by minutes.
    Minutes(i32),
}

/// The item under the grid's column `k`: the stepper (columns 0 – 2),
/// Select (3 – 5), Back (6), as the renderer draws them.
fn under(k: usize) -> usize {
    match k {
        0..=2 => TIME,
        3..=5 => SELECT,
        _ => BACK,
    }
}

/// The grid's column over an item under the grid (its middle).
fn over(f: usize) -> usize {
    match f {
        TIME => 1,
        SELECT => 4,
        _ => 6,
    }
}

/// Moves the focus `focus` on page `c` (the days by day and week, past the
/// month's first / last week to the buttons over and under the grid, the
/// month and year by their keys and buttons, the stepper by minutes;
/// everything within 1900 – 2100). `minute` is the stepper's shown time.
pub fn navigate(c: &mut Cal, focus: usize, nav: Nav, minute: u16) -> usize {
    let col = |i: usize| i % COLUMNS;
    // The day of column `k` in the first / last week of the month.
    let first_in = |c: &Cal, k: usize| {
        let g = c.grid();
        (0..cal::GRID_DAYS)
            .filter(|&i| i % COLUMNS == k)
            .find(|&i| c.in_month(g[i]))
            .unwrap_or(k)
    };
    let last_in = |c: &Cal, k: usize| {
        let g = c.grid();
        (0..cal::GRID_DAYS)
            .rev()
            .filter(|&i| i % COLUMNS == k)
            .find(|&i| c.in_month(g[i]))
            .unwrap_or(k)
    };
    let to_day = |c: &mut Cal, d: Date| {
        let d = cal::clamp(d);
        *c = Cal {
            year: d.0,
            month: d.1,
            day: d.2,
            ..*c
        };
        c.cell_of(d).unwrap_or(0)
    };
    match (nav, focus) {
        (Nav::Month(n), f) => month_jump(c, f, n),
        (Nav::Year(n), f) => month_jump(c, f, 12 * n),
        (Nav::Minutes(m), f) => {
            let t = (i32::from(minute) + m).rem_euclid(24 * 60);
            c.minute = Some(t as u16);
            f
        }
        (Nav::Left | Nav::Right, f) if f < DAYS => {
            let d = c.date_of(f).expect("a day");
            to_day(c, cal::add_days(d, if nav == Nav::Right { 1 } else { -1 }))
        }
        (Nav::Up | Nav::Down, f) if f < DAYS => {
            let d = c.date_of(f).expect("a day");
            let down = nav == Nav::Down;
            let to = cal::add_days(d, if down { 7 } else { -7 });
            if c.in_month(to) && to != d {
                to_day(c, to)
            } else if down {
                // Under the grid: the stepper, Select or Back below.
                under(col(f))
            } else {
                YEAR_PREV + col(f)
            }
        }
        // The buttons over the grid: a row of seven.
        (Nav::Left | Nav::Right, f) if (YEAR_PREV..=YEAR_NEXT).contains(&f) => {
            let k = (f - YEAR_PREV) as isize + if nav == Nav::Right { 1 } else { -1 };
            YEAR_PREV + k.rem_euclid(COLUMNS as isize) as usize
        }
        (Nav::Down, f) if (YEAR_PREV..=YEAR_NEXT).contains(&f) => {
            let i = first_in(&*c, f - YEAR_PREV);
            c.day = c.grid()[i].2;
            i
        }
        (Nav::Up, f) if (YEAR_PREV..=YEAR_NEXT).contains(&f) => under(f - YEAR_PREV),
        // Under the grid: ← / → step the time (Shift: a minute).
        (Nav::Left | Nav::Right, TIME) => {
            let step = if nav == Nav::Right {
                STEP_MIN
            } else {
                -STEP_MIN
            };
            navigate(c, TIME, Nav::Minutes(step), minute)
        }
        (Nav::Left, SELECT) => TIME,
        (Nav::Right, SELECT) => BACK,
        (Nav::Left, BACK) => SELECT,
        (Nav::Right, BACK) => BACK,
        // Up into the last week, down to the buttons over the grid: the
        // middle column under each (the stepper 0 – 2, Select 3 – 5, Back 6).
        (Nav::Up, f @ (TIME | SELECT | BACK)) => {
            let i = last_in(&*c, over(f));
            c.day = c.grid()[i].2;
            i
        }
        (Nav::Down, f @ (TIME | SELECT | BACK)) => YEAR_PREV + over(f),
        (_, f) => f,
    }
}

/// A month (or year) step: on a day the same day of that month, on a
/// button the focus stays (the selected day kept in the new month, so
/// Select shows it).
fn month_jump(c: &mut Cal, focus: usize, n: i32) -> usize {
    let day = c.date_of(focus).filter(|d| c.in_month(*d)).map(|d| d.2);
    let d = cal::add_months((c.year, c.month, day.unwrap_or(c.day)), n);
    *c = Cal {
        year: d.0,
        month: d.1,
        day: d.2,
        ..*c
    };
    match day {
        Some(_) => c.cell_of(d).unwrap_or(focus),
        None => focus,
    }
}

/// The view of the page for the renderer.
pub fn view(c: &Cal, info: &CalendarInfo, rows: &[Row]) -> MenuCalendar {
    let southern = info.place.0 < 0.0;
    let days = c
        .grid()
        .into_iter()
        .map(|d| CalendarDay {
            label: d.2.to_string(),
            band: info.look(d).map(Look::rgb),
            moon: info
                .moon
                .iter()
                .find(|(at, _)| *at == d)
                .map(|(_, p)| cal::phase_glyph(*p, southern).to_owned()),
            outside: !c.in_month(d),
            today: Some(d) == info.today,
            selected: Some(d) == info.date,
        })
        .collect();
    let looks: Vec<Look> = match &info.year {
        Some(y) => Look::ALL
            .into_iter()
            .filter(|l| y.looks.contains(l))
            .collect(),
        None => Vec::new(),
    };
    let evergreen = info
        .year
        .as_ref()
        .is_some_and(|y| cal::evergreen(&y.climate));
    MenuCalendar {
        title: format!("{} {}", cal::MONTHS[(c.month - 1) as usize], c.year),
        weekdays: cal::WEEKDAYS.iter().map(|s| (*s).to_owned()).collect(),
        days,
        controls: rows
            .iter()
            .skip(DAYS)
            .map(super::model::row_label)
            .chain(["Back".to_owned()])
            .collect(),
        legend: looks
            .into_iter()
            .map(|l| {
                let name = match (l, evergreen) {
                    (Look::Summer, true) => "green",
                    (l, _) => l.label(),
                };
                (name.to_owned(), l.rgb())
            })
            .collect(),
        moon_legend: format!(
            "● full  ○ new  {} {} quarters",
            cal::phase_glyph(Phase::FirstQuarter, southern),
            cal::phase_glyph(Phase::LastQuarter, southern)
        ),
    }
}

/// The line under the title: the focused day (on Select the selected
/// day, on another button the world date), read-only in a shared ride without the right.
pub fn status(c: &Cal, info: &CalendarInfo, focus: usize) -> String {
    let d = status_date(c, info, focus);
    let day = match (d, &info.line) {
        (Some(d), Some((at, line))) if *at == d => {
            let wd = cal::WEEKDAYS[cal::weekday(d) as usize];
            format!("{wd} {}: {line}", cal::short_date(d))
        }
        (Some(d), _) => cal::short_date(d),
        _ => String::new(),
    };
    if info.may_change {
        day
    } else {
        format!("Read only, set by {} · {day}", info.control)
    }
}

/// The day the status describes (the app computes its line): the focused
/// day, on Select the selected one, on another button the world date.
pub fn status_date(c: &Cal, info: &CalendarInfo, focus: usize) -> Option<Date> {
    c.date_of(focus)
        .or_else(|| (focus == SELECT).then(|| c.selected()))
        .or(info.date)
}

/// Calendar → Dates: today, the equinoxes and solstices, the season starts
/// of the place – all in the shown year, each a date jump.
pub fn dates_rows(c: &Cal, info: &CalendarInfo) -> Vec<Row> {
    let jump = |label: &str, d: Option<Date>| Row {
        label: label.into(),
        value: Some(d.map_or_else(|| "none here".into(), cal::short_date)),
        left: None,
        right: None,
        confirm: Some(match d {
            Some(d) if info.may_change => Activate::CommandBack(date_line(d, c.minute)),
            _ => Activate::Disabled,
        }),
    };
    let mut out = vec![jump("Today", info.today)];
    if let Some(zone) = &info.zone {
        for (name, t) in cal::SOLAR_EVENTS.iter().zip(cal::solar_events(c.year)) {
            let l = zone.local(t);
            out.push(jump(name, Some((l.year, l.month, l.day))));
        }
    }
    if let Some(y) = &info.year {
        for (name, doy) in y.starts.list() {
            out.push(jump(name, doy.map(|n| cal::date_of_doy(c.year, n))));
        }
    }
    out
}

/// World → Time → Season: the six seasons of the place, each its
/// representative day nearest to the world date (decision Q11: the date
/// jumps there, the time of day kept).
pub fn season_rows(info: &CalendarInfo) -> Vec<Row> {
    let (Some(y), Some(today)) = (&info.year, info.date) else {
        return Vec::new();
    };
    let now = y.season_of(cal::day_of_year(today));
    Season::ALL
        .into_iter()
        .map(|s| {
            let d = y.season_date(s, today);
            Row {
                label: if s == now {
                    format!("{} (now)", s.label())
                } else {
                    s.label().to_owned()
                },
                value: Some(cal::short_date(d)),
                left: None,
                right: None,
                confirm: Some(if info.may_change {
                    // The date itself (`time season` gives the same):
                    // the calendar under it follows the line.
                    Activate::CommandBack(date_line(d, None))
                } else {
                    Activate::Disabled
                }),
            }
        })
        .collect()
}

/// The Season row of World → Time: the season of the world date at the
/// place; ← / → the previous / next season's day, Enter the Season page.
pub fn season_row(info: &CalendarInfo) -> Row {
    let (Some(y), Some(today)) = (&info.year, info.date) else {
        return super::pages::open_row("Season", Some("-".into()), Page::Season);
    };
    let step = |dir| {
        let (_, d) = y.step_season(today, dir);
        format!("time date {}", cal::iso(d))
    };
    Row {
        label: "Season".into(),
        value: Some(y.season_of(cal::day_of_year(today)).label().to_owned()),
        left: info.may_change.then(|| step(-1)),
        right: info.may_change.then(|| step(1)),
        confirm: Some(Activate::Open(Page::Season)),
    }
}

/// The calendar page of the world date (World → Time → Day, Enter).
pub fn open_at(info: &CalendarInfo) -> Page {
    Page::Calendar(Cal::at(info.date.unwrap_or((2026, 1, 1))))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn info() -> CalendarInfo {
        let zone = Zone::get("Europe/Vaduz").unwrap();
        let place = (47.141, 9.521);
        let model = Arc::new(SeasonModel::for_place(place.0, place.1));
        let year = Arc::new(SeasonYear::new(&model, 455.0, f64::from));
        CalendarInfo {
            date: Some((2026, 11, 16)),
            minute: 14 * 60 + 20,
            today: Some((2026, 11, 16)),
            may_change: true,
            control: "you".into(),
            zone: Some(zone),
            place,
            alt_m: 455.0,
            model: Some(model),
            year: Some(year),
            ..Default::default()
        }
    }

    #[test]
    fn rows_are_days_buttons_and_the_time() {
        let i = info();
        let c = Cal::at((2026, 11, 16));
        let r = rows(&c, &i);
        assert_eq!(r.len(), ROWS);
        assert_eq!(c.cell_of((2026, 11, 16)), Some(21));
        assert_eq!(
            r[21].confirm,
            Some(Activate::Command("time date 2026-11-16".into()))
        );
        assert_eq!(r[0].label, "26", "Monday 26 October");
        assert_eq!(r[YEAR_PREV].label, "« 2025");
        assert_eq!(r[MONTH_PREV].label, "‹ Oct");
        assert_eq!(r[MONTH_NEXT].label, "Dec ›");
        assert_eq!(r[YEAR_NEXT].label, "2027 »");
        assert_eq!(r[TIME].value.as_deref(), Some("14:20"));
        assert_eq!(
            r[TODAY].confirm,
            Some(Activate::Command("time date 2026-11-16".into()))
        );
        // With the stepper's time.
        let c = Cal {
            minute: Some(7 * 60 + 45),
            ..c
        };
        let r = rows(&c, &i);
        assert_eq!(
            r[22].confirm,
            Some(Activate::Command("time date 2026-11-17 07:45".into()))
        );
        assert_eq!(
            line_date("time date 2026-11-17 07:45"),
            Some((2026, 11, 17))
        );
        assert_eq!(line_date("time +1d"), None);
        // Every line parses as a time command.
        for row in &r {
            if let Some(Activate::Command(l)) = &row.confirm {
                let args: Vec<&str> = l.split_whitespace().skip(1).collect();
                ocw_sky::machine::parse(&args).unwrap_or_else(|e| panic!("{l}: {e}"));
            }
        }
        for s in r.iter().map(super::super::model::row_label) {
            assert!(ocw_render::font::covers(&s), "{s}");
        }
    }

    /// Read-only in a shared ride without the right: the days and Today
    /// dimmed (Disabled), the status says who sets the time.
    #[test]
    fn read_only_without_the_right() {
        let mut i = info();
        i.may_change = false;
        i.control = "the host".into();
        let c = Cal::at((2026, 11, 16));
        let r = rows(&c, &i);
        assert!(
            r[..DAYS]
                .iter()
                .all(|r| r.confirm == Some(Activate::Disabled))
        );
        assert_eq!(r[TODAY].confirm, Some(Activate::Disabled));
        assert_eq!(r[TIME].confirm, Some(Activate::Disabled));
        // Navigation still works.
        assert_eq!(
            r[DATES].confirm,
            Some(Activate::Open(Page::CalendarDates(c)))
        );
        assert!(status(&c, &i, 21).starts_with("Read only, set by the host"));
        assert!(
            dates_rows(&c, &i)
                .iter()
                .all(|r| r.confirm == Some(Activate::Disabled))
        );
        assert!(
            season_rows(&i)
                .iter()
                .all(|r| r.confirm == Some(Activate::Disabled))
        );
        let s = season_row(&i);
        assert!(s.left.is_none() && s.right.is_none());
    }

    /// Day, week and month steps; past the month's weeks to the buttons;
    /// the range's ends hold.
    #[test]
    fn the_focus_moves_by_day_week_and_month() {
        let mut c = Cal::at((2026, 11, 16));
        let f = c.cell_of((2026, 11, 16)).unwrap();
        let f = navigate(&mut c, f, Nav::Right, 0);
        assert_eq!(c.date_of(f), Some((2026, 11, 17)));
        let f = navigate(&mut c, f, Nav::Down, 0);
        assert_eq!(c.date_of(f), Some((2026, 11, 24)));
        let f = navigate(&mut c, f, Nav::Down, 0);
        assert_eq!(f, TIME, "past the last week: the time stepper");
        let f = navigate(&mut c, f, Nav::Up, 0);
        assert_eq!(
            c.date_of(f),
            Some((2026, 11, 24)),
            "back to the last Tuesday"
        );
        // Over the month's end by day: December shows.
        let mut c = Cal::at((2026, 11, 30));
        let f = {
            let at = c.cell_of((2026, 11, 30)).unwrap();
            navigate(&mut c, at, Nav::Right, 0)
        };
        assert_eq!((c.year, c.month), (2026, 12));
        assert_eq!(c.date_of(f), Some((2026, 12, 1)));
        let f = navigate(&mut c, f, Nav::Left, 0);
        assert_eq!(
            (c.year, c.month, c.date_of(f)),
            (2026, 11, Some((2026, 11, 30)))
        );
        // Up past the first week: the buttons; Down again: the first week.
        let f = {
            let at = c.cell_of((2026, 11, 4)).unwrap();
            navigate(&mut c, at, Nav::Up, 0)
        };
        assert_eq!(f, YEAR_PREV + 2, "Wednesday's column: Today");
        let f = navigate(&mut c, f, Nav::Down, 0);
        assert_eq!(c.date_of(f), Some((2026, 11, 4)));
        // Month and year, the day kept (31 January → 28 February).
        let mut c = Cal::at((2026, 1, 31));
        let f = {
            let at = c.cell_of((2026, 1, 31)).unwrap();
            navigate(&mut c, at, Nav::Month(1), 0)
        };
        assert_eq!(c.date_of(f), Some((2026, 2, 28)));
        let f = navigate(&mut c, f, Nav::Year(-1), 0);
        assert_eq!(c.date_of(f), Some((2025, 2, 28)));
        let f2 = navigate(&mut c, MONTH_NEXT, Nav::Month(1), 0);
        assert_eq!(
            (f2, c.month),
            (MONTH_NEXT, 3),
            "on a button the focus stays"
        );
        // The buttons' row wraps; the stepper steps 15 minutes (Shift 1).
        assert_eq!(navigate(&mut c, YEAR_NEXT, Nav::Right, 0), YEAR_PREV);
        assert_eq!(navigate(&mut c, TIME, Nav::Left, 10), TIME);
        assert_eq!(c.minute, Some(24 * 60 - 5));
        navigate(&mut c, TIME, Nav::Minutes(1), 14 * 60 + 20);
        assert_eq!(c.minute, Some(14 * 60 + 21));
        // The range: 1900 – 2100.
        let mut c = Cal::at((2100, 12, 31));
        let f = {
            let at = c.cell_of((2100, 12, 31)).unwrap();
            navigate(&mut c, at, Nav::Right, 0)
        };
        assert_eq!(c.date_of(f), Some((2100, 12, 31)));
        navigate(&mut c, f, Nav::Month(1), 0);
        assert_eq!((c.year, c.month), (2100, 12));
        let mut c = Cal::at((1900, 1, 1));
        let f = {
            let at = c.cell_of((1900, 1, 1)).unwrap();
            navigate(&mut c, at, Nav::Left, 0)
        };
        assert_eq!(c.date_of(f), Some((1900, 1, 1)));
        // The last grid's days after 2100 cannot be set.
        let c = Cal::at((2100, 12, 31));
        let r = rows(&c, &info());
        assert_eq!(r[cal::GRID_DAYS - 1].confirm, Some(Activate::Disabled));
    }

    /// The bands match the season model of S8 at the same days (15 January
    /// bare, 1 May fresh, 15 October colouring in Vaduz), the moon's main
    /// phases sit on their days, the selected day and today are marked.
    #[test]
    fn the_view_shows_bands_moon_and_marks() {
        let mut i = info();
        let c = Cal::at((2026, 11, 16));
        i.ensure_moon(2026, 11);
        let v = view(&c, &i, &rows(&c, &i));
        assert_eq!(v.title, "November 2026");
        assert_eq!(v.days.len(), DAYS);
        assert_eq!(v.controls.len(), BACK - DAYS + 1);
        let d = &v.days[c.cell_of((2026, 11, 24)).unwrap()];
        assert_eq!(d.moon.as_deref(), Some("●"), "full moon on 24 November");
        assert!(v.days[c.cell_of((2026, 11, 9)).unwrap()].moon.as_deref() == Some("○"));
        assert!(v.days[21].selected && v.days[21].today);
        assert!(v.days[0].outside && !v.days[10].outside);
        assert_eq!(v.days[21].band, Some(Look::Bare.rgb()), "16 November: bare");
        let model = SeasonModel::for_place(47.141, 9.521);
        for (d, want) in [
            ((2026, 1, 15), Look::Bare),
            ((2026, 5, 1), Look::Fresh),
            ((2026, 10, 20), Look::Autumn),
        ] {
            assert_eq!(i.look(d), Some(want), "{d:?}");
            // The same SeasonDay the renderer uses.
            let doy = f64::from(cal::day_of_year(d));
            let day = model.day(doy);
            let s = day.state(455.0);
            assert_eq!(cal::look_of(&day.climate, &s, s.snow), want);
        }
        // The measured snow of real weather on the world date.
        i.measured = Some(Look::Snow);
        assert_eq!(i.look((2026, 11, 16)), Some(Look::Snow));
        assert_eq!(i.look((2026, 11, 17)), Some(Look::Bare));
        assert!(v.legend.iter().any(|(n, _)| n == "autumn colours"));
        assert!(
            ocw_render::font::covers(&v.moon_legend),
            "{}",
            v.moon_legend
        );
        for d in &v.days {
            if let Some(m) = &d.moon {
                assert!(ocw_render::font::covers(m));
            }
        }
    }

    /// The tooltip line: the sun of the focused day in the local zone
    /// (NOAA times of `ocw_sky::sun`'s tests: Vaduz on 21 June 05:26 /
    /// 21:21 CEST), the leaves, the snow line; on the DST days the local
    /// clock jumps by an hour.
    #[test]
    fn the_day_line_and_dst_days() {
        let i = info();
        let l = i.day_line((2026, 6, 21));
        assert!(
            l.starts_with("sunrise 05:26 · sunset 21:21 · day 15 h 55 min"),
            "{l}"
        );
        assert!(l.contains("beech in full leaf"), "{l}");
        assert!(l.contains("snow line"), "{l}");
        // 28 / 29 March 2026: CET → CEST, sunrise an hour later on the clock.
        let rise = |d| {
            let l = i.day_line(d);
            let t = l["sunrise ".len().."sunrise ".len() + 5].to_owned();
            let (h, m) = t.split_once(':').unwrap();
            h.parse::<i32>().unwrap() * 60 + m.parse::<i32>().unwrap()
        };
        let jump = rise((2026, 3, 29)) - rise((2026, 3, 28));
        assert!((56..=60).contains(&jump), "{jump}");
        let back = rise((2026, 10, 25)) - rise((2026, 10, 24));
        assert!((-60..=-56).contains(&back), "{back}");
        // A date on the DST day sets as any other (the server's zone maps
        // the local time).
        let c = Cal {
            minute: Some(2 * 60 + 30),
            ..Cal::at((2026, 3, 29))
        };
        let r = rows(&c, &i);
        let f = c.cell_of((2026, 3, 29)).unwrap();
        assert_eq!(
            r[f].confirm,
            Some(Activate::Command("time date 2026-03-29 02:30".into()))
        );
        let zone = i.zone.clone().unwrap();
        assert!(
            zone.instant((2026, 3, 29), (2, 30, 0)).is_ok(),
            "the gap maps"
        );
        assert_eq!(thousands(1349.0), "1,350");
        assert_eq!(thousands(980.0), "980");
    }

    /// The menu model on the calendar page: it opens on the day, the
    /// arrows and Page Down move as `navigate` says, a click on a button
    /// changes the month, Enter on a day runs its `time date` line, a
    /// shortcut of Calendar → Dates goes back to the calendar showing its
    /// month, the stepper's arrows (mouse, touch) step 15 minutes.
    #[test]
    fn the_menu_model_drives_the_calendar() {
        use super::super::model::{Action, Effect, Menu, Page as M, Platform};
        let i = info();
        let tm = super::super::time::TimeMachineInfo {
            date: Some("Mon 16 Nov 2026".into()),
            time: Some("14:20 CET".into()),
            control: "you".into(),
            may_change: true,
            ..Default::default()
        };
        let home = super::super::pages::WorldHome {
            name: "Test loop".into(),
            ..Default::default()
        };
        let weather = super::super::weather::WeatherState::default();
        let rows = |p: Page| match p {
            Page::WorldHome => super::super::pages::world_home_rows(&home, &tm, &weather),
            Page::TimeMachine => super::super::time::rows(&tm, &i),
            Page::Calendar(c) => rows(&c, &i),
            Page::CalendarDates(c) => dates_rows(&c, &i),
            Page::Season => season_rows(&i),
            _ => Vec::new(),
        };
        let cal_page = Page::Calendar(Cal::at((2026, 11, 16)));
        for platform in [Platform::Desktop, Platform::Browser] {
            // Top → World → Time → Day (Enter): the calendar.
            let mut m = Menu::for_platform(platform);
            m.apply(Action::Open);
            let world = m.labels().iter().position(|l| l == "World").unwrap();
            m.apply_with(Action::Click(world), rows);
            assert_eq!(m.page(), Some(M::Sub(Page::WorldHome)));
            m.apply_with(Action::Click(1), rows);
            assert_eq!(m.page(), Some(M::Sub(Page::TimeMachine)));
            m.apply_with(Action::Down, rows);
            assert_eq!(m.apply_with(Action::Confirm, rows), Effect::None);
            assert_eq!(m.page(), Some(M::Sub(cal_page)));
            assert_eq!(m.focus(), 21, "16 November");
            assert_eq!(m.item_count(), BACK + 1);
            m.apply_with(Action::Right, rows);
            assert_eq!(m.focus(), 22);
            assert_eq!(
                m.apply_with(Action::Confirm, rows),
                Effect::Command("time date 2026-11-17".into())
            );
            m.apply_with(Action::TabNext, rows);
            let Some(M::Sub(Page::Calendar(c))) = m.page() else {
                panic!()
            };
            assert_eq!((c.year, c.month), (2026, 12));
            assert_eq!(c.date_of(m.focus()), Some((2026, 12, 17)));
            // A click on « : the year before.
            m.apply_with(Action::Click(YEAR_PREV), rows);
            let Some(M::Sub(Page::Calendar(c))) = m.page() else {
                panic!()
            };
            assert_eq!((c.year, c.month), (2025, 12));
            assert_eq!(m.focus(), YEAR_PREV);
            // The stepper's › arrow: 15 minutes, Shift+→ one.
            m.apply_with(Action::Step(TIME, 1), rows);
            assert_eq!(m.focus(), TIME);
            m.apply_with(Action::Fine(1), rows);
            let Some(M::Sub(Page::Calendar(c))) = m.page() else {
                panic!()
            };
            assert_eq!(c.minute, Some(14 * 60 + 36));
            // Calendar → Dates → March equinox: back on the calendar in
            // March, the line with the stepper's time.
            m.apply_with(Action::Click(DATES), rows);
            assert!(matches!(m.page(), Some(M::Sub(Page::CalendarDates(_)))));
            let e = m.apply_with(Action::Click(1), rows);
            assert_eq!(e, Effect::Command("time date 2025-03-20 14:36".into()));
            let Some(M::Sub(Page::Calendar(c))) = m.page() else {
                panic!("{:?}", m.page())
            };
            assert_eq!((c.year, c.month), (2025, 3));
            assert_eq!(c.date_of(m.focus()), Some((2025, 3, 20)));
            // Esc: back to the Time page.
            m.apply_with(Action::Back, rows);
            assert_eq!(m.page(), Some(M::Sub(Page::TimeMachine)));
        }
    }

    /// The Select button (user feedback 2026-11-17: "i miss the select
    /// button that accepts and sets the selected date"): its row shows the
    /// selected day, sets it with the stepper's time and goes back; dimmed
    /// without the clock right or out of the range.
    #[test]
    fn select_sets_the_selected_day_and_goes_back() {
        let i = info();
        let c = Cal::at((2026, 11, 17));
        let r = rows(&c, &i);
        assert_eq!(r.len(), ROWS);
        assert_eq!(r[SELECT].label, "Select Tue 17 Nov 2026");
        assert_eq!(
            r[SELECT].confirm,
            Some(Activate::CommandBack("time date 2026-11-17".into())),
            "the time of day kept"
        );
        // With the stepper's time.
        let c = Cal {
            minute: Some(6 * 60 + 15),
            ..c
        };
        let r = rows(&c, &i);
        let Some(Activate::CommandBack(line)) = &r[SELECT].confirm else {
            panic!("{:?}", r[SELECT].confirm)
        };
        assert_eq!(line, "time date 2026-11-17 06:15");
        let args: Vec<&str> = line.split_whitespace().skip(1).collect();
        ocw_sky::machine::parse(&args).unwrap();
        assert!(ocw_render::font::covers(&r[SELECT].label));
        // The view's controls: the seven buttons, Time, Select, Back.
        let v = view(&c, &i, &r);
        assert_eq!(v.controls.len(), 10);
        assert_eq!(v.controls[SELECT - DAYS], "Select Tue 17 Nov 2026");
        assert_eq!(v.controls[BACK - DAYS], "Back");
        // The status on Select describes the selected day.
        assert_eq!(status_date(&c, &i, SELECT), Some((2026, 11, 17)));
        assert_eq!(status_date(&c, &i, TODAY), i.date, "another button");
        // Without the right: dimmed, the label still the day.
        let mut ro = info();
        ro.may_change = false;
        let r = rows(&c, &ro);
        assert_eq!(r[SELECT].confirm, Some(Activate::Disabled));
        assert_eq!(r[SELECT].label, "Select Tue 17 Nov 2026");
        // Out of the range 1900 – 2100: dimmed like the day cells.
        for (year, month, day) in [(2101, 1, 3), (1899, 12, 31)] {
            let out = Cal {
                year,
                month,
                day,
                minute: None,
            };
            assert_eq!(rows(&out, &i)[SELECT].confirm, Some(Activate::Disabled));
        }
        // The ends of the range are fine.
        let last = Cal::at((2100, 12, 31));
        assert_eq!(
            rows(&last, &i)[SELECT].confirm,
            Some(Activate::CommandBack("time date 2100-12-31".into()))
        );
    }

    /// The arrows reach Select: down from the middle columns of the last
    /// week, between the stepper and Back, up from the buttons over the
    /// grid; the month buttons keep the selected day.
    #[test]
    fn the_arrows_reach_select() {
        // November 2026: the 26th is a Thursday (column 3) of the last week.
        let mut c = Cal::at((2026, 11, 26));
        let f = c.cell_of((2026, 11, 26)).unwrap();
        assert_eq!(f % COLUMNS, 3);
        assert_eq!(navigate(&mut c, f, Nav::Down, 0), SELECT);
        assert_eq!(navigate(&mut c, SELECT, Nav::Left, 0), TIME);
        assert_eq!(navigate(&mut c, SELECT, Nav::Right, 0), BACK);
        assert_eq!(navigate(&mut c, BACK, Nav::Left, 0), SELECT);
        assert_eq!(navigate(&mut c, BACK, Nav::Right, 0), BACK);
        assert_eq!(navigate(&mut c, SELECT, Nav::Down, 0), SEASON);
        assert_eq!(navigate(&mut c, TIME, Nav::Down, 0), MONTH_PREV);
        assert_eq!(navigate(&mut c, BACK, Nav::Down, 0), YEAR_NEXT);
        assert_eq!(navigate(&mut c, SEASON, Nav::Up, 0), SELECT);
        assert_eq!(navigate(&mut c, DATES, Nav::Up, 0), SELECT);
        assert_eq!(navigate(&mut c, MONTH_PREV, Nav::Up, 0), TIME);
        assert_eq!(navigate(&mut c, YEAR_NEXT, Nav::Up, 0), BACK);
        assert_eq!(c.selected(), (2026, 11, 26), "the buttons keep the day");
        // Up from Select: the last week's Friday, now the selected day.
        let f = navigate(&mut c, SELECT, Nav::Up, 0);
        assert_eq!(c.date_of(f), Some((2026, 11, 27)));
        assert_eq!(c.selected(), (2026, 11, 27));
        // The month buttons: the selected day kept (31 → 28 in February).
        let mut c = Cal::at((2026, 1, 31));
        assert_eq!(navigate(&mut c, MONTH_NEXT, Nav::Month(1), 0), MONTH_NEXT);
        assert_eq!(c.selected(), (2026, 2, 28));
        navigate(&mut c, YEAR_PREV, Nav::Year(-1), 0);
        assert_eq!(c.selected(), (2025, 2, 28));
    }

    /// Select in the menu model, in both apps: Enter (A) or a click on it
    /// sets the selected day with the stepper's time and goes back to
    /// World → Time; Esc (B) / Back cancels with no command; dimmed it
    /// does nothing. Enter on a day still sets the day and stays (as
    /// before Select).
    #[test]
    fn select_in_the_menu_model() {
        use super::super::model::{Action, Effect, Menu, Page as M, Platform};
        let tm = super::super::time::TimeMachineInfo {
            date: Some("Mon 16 Nov 2026".into()),
            time: Some("14:20 CET".into()),
            control: "you".into(),
            may_change: true,
            ..Default::default()
        };
        let pages = |i: CalendarInfo| {
            let tm = tm.clone();
            let home = super::super::pages::WorldHome {
                name: "Test loop".into(),
                ..Default::default()
            };
            let weather = super::super::weather::WeatherState::default();
            move |p: Page| match p {
                Page::WorldHome => super::super::pages::world_home_rows(&home, &tm, &weather),
                Page::TimeMachine => super::super::time::rows(&tm, &i),
                Page::Calendar(c) => rows(&c, &i),
                _ => Vec::new(),
            }
        };
        // Top → World → Time → Day (Enter): the calendar on 16 November.
        let open = |platform, rows: &dyn Fn(Page) -> Vec<Row>| {
            let mut m = Menu::for_platform(platform);
            m.apply(Action::Open);
            let world = m.labels().iter().position(|l| l == "World").unwrap();
            m.apply_with(Action::Click(world), rows);
            m.apply_with(Action::Click(1), rows);
            assert_eq!(m.page(), Some(M::Sub(Page::TimeMachine)));
            m.apply_with(Action::Down, rows);
            m.apply_with(Action::Confirm, rows);
            assert!(matches!(m.page(), Some(M::Sub(Page::Calendar(_)))));
            assert_eq!(m.focus(), 21);
            m
        };
        let calendar = |m: &Menu| matches!(m.page(), Some(M::Sub(Page::Calendar(_))));
        let rows = pages(info());
        let rows = &rows;
        for platform in [Platform::Desktop, Platform::Browser] {
            // By keys: ↓ past the last week to the stepper (Monday's
            // column), → 15 minutes on, ↑ into the last week (Tuesday the
            // 24th), → → Thursday the 26th, ↓ Select, Enter.
            let mut m = open(platform, rows);
            for _ in 0..3 {
                m.apply_with(Action::Down, rows);
            }
            assert_eq!(m.focus(), TIME);
            m.apply_with(Action::Right, rows);
            m.apply_with(Action::Up, rows);
            m.apply_with(Action::Right, rows);
            m.apply_with(Action::Right, rows);
            m.apply_with(Action::Down, rows);
            assert_eq!(m.focus(), SELECT, "below Thursday: Select");
            assert_eq!(m.labels()[SELECT], "Select Thu 26 Nov 2026");
            let e = m.apply_with(Action::Confirm, rows);
            assert_eq!(e, Effect::Command("time date 2026-11-26 14:35".into()));
            assert_eq!(
                m.page(),
                Some(M::Sub(Page::TimeMachine)),
                "back to World → Time"
            );
            // A click or tap on it: the same.
            let mut m = open(platform, rows);
            m.apply_with(Action::Left, rows);
            let e = m.apply_with(Action::Click(SELECT), rows);
            assert_eq!(e, Effect::Command("time date 2026-11-15".into()));
            assert_eq!(m.page(), Some(M::Sub(Page::TimeMachine)));
            // Esc / B and the Back button: no command, back.
            let mut m = open(platform, rows);
            m.apply_with(Action::Right, rows);
            assert_eq!(m.apply_with(Action::Back, rows), Effect::None);
            assert_eq!(m.page(), Some(M::Sub(Page::TimeMachine)));
            let mut m = open(platform, rows);
            assert_eq!(m.apply_with(Action::Click(BACK), rows), Effect::None);
            assert_eq!(m.page(), Some(M::Sub(Page::TimeMachine)));
            // Enter on a day: sets it and stays on the calendar (as before).
            let mut m = open(platform, rows);
            m.apply_with(Action::Right, rows);
            assert_eq!(
                m.apply_with(Action::Confirm, rows),
                Effect::Command("time date 2026-11-17".into())
            );
            assert!(calendar(&m));
            assert_eq!(m.labels()[SELECT], "Select Tue 17 Nov 2026");
        }
        // Without the right: Select dimmed, does nothing, the page stays.
        let mut ro = info();
        ro.may_change = false;
        let ro = pages(ro);
        let mut m = open(Platform::Desktop, &ro);
        assert_eq!(m.apply_with(Action::Click(SELECT), &ro), Effect::None);
        assert!(calendar(&m));
    }

    /// Calendar → Dates and World → Time → Season.
    #[test]
    fn dates_and_seasons() {
        let i = info();
        let c = Cal::at((2026, 11, 16));
        let d = dates_rows(&c, &i);
        assert_eq!(d.len(), 1 + 4 + 5);
        assert_eq!(d[0].value.as_deref(), Some("16 Nov 2026"));
        assert_eq!(d[1].label, "March equinox");
        assert_eq!(d[1].value.as_deref(), Some("20 Mar 2026"));
        assert_eq!(d[4].value.as_deref(), Some("21 Dec 2026"));
        assert_eq!(d[9].label, "First lying snow");
        assert_eq!(d[9].value.as_deref(), Some("none here"), "Vaduz");
        assert_eq!(d[9].confirm, Some(Activate::Disabled));
        assert!(
            matches!(&d[5].confirm, Some(Activate::CommandBack(l)) if l.starts_with("time date 2026-04"))
        );
        let s = season_rows(&i);
        assert_eq!(s.len(), 6);
        assert!(s.len() < super::super::model::MAX_ITEMS);
        assert!(s.iter().any(|r| r.label.ends_with("(now)")));
        let autumn = i
            .year
            .as_ref()
            .unwrap()
            .season_date(Season::Autumn, (2026, 11, 16));
        assert_eq!(
            s[3].confirm,
            Some(Activate::CommandBack(format!(
                "time date {}",
                cal::iso(autumn)
            )))
        );
        let row = season_row(&i);
        assert_eq!(row.label, "Season");
        assert_eq!(row.confirm, Some(Activate::Open(Page::Season)));
        let r = row.right.unwrap();
        let next = line_date(&r).unwrap();
        assert!(next > (2026, 11, 16), "{r}");
        let l = row.left.unwrap();
        assert!(line_date(&l).unwrap() < (2026, 11, 16), "{l}");
        assert_eq!(open_at(&i), Page::Calendar(Cal::at((2026, 11, 16))));
    }
}
