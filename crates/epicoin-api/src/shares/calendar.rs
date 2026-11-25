//! The campus calendar and streaks of working days with a share. Pure functions, no I/O.
//!
//! A streak is a run of consecutive **working days** (open weekdays that are not closures)
//! each with at least one share. Weekends and closures neither extend nor break a streak; a
//! share made on such a day counts for nothing here (it is still paid). Several shares on
//! the same day count once.

use std::collections::BTreeSet;

use epicoin_core::Amount;
use serde::{Deserialize, Serialize};
use time::Date;

/// How far back to look for the previous working day. A calendar without a working day in
/// ten years is a configuration error; the search stops instead of looping.
const MAX_GAP_DAYS: u32 = 3660;

/// Reaching `days` consecutive working days pays `bonus` units, once per streak.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Tier {
    pub days: u32,
    pub bonus: Amount,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Calendar {
    /// ISO weekday numbers the campus is open: 1 = Monday … 7 = Sunday.
    pub weekdays: BTreeSet<u8>,
    pub closures: BTreeSet<Date>,
}

impl Calendar {
    pub fn is_working_day(&self, day: Date) -> bool {
        self.weekdays.contains(&day.weekday().number_from_monday()) && !self.closures.contains(&day)
    }

    /// The last working day strictly before `day`.
    pub fn previous_working_day(&self, day: Date) -> Option<Date> {
        let mut current = day;
        for _ in 0..MAX_GAP_DAYS {
            current = current.previous_day()?;
            if self.is_working_day(current) {
                return Some(current);
            }
        }
        None
    }
}

/// A run of consecutive working days with a share.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Streak {
    pub start: Date,
    pub end: Date,
    pub days: u32,
}

/// The streaks formed by the days that have a share, oldest first.
pub fn streaks(days: &BTreeSet<Date>, calendar: &Calendar) -> Vec<Streak> {
    let mut result: Vec<Streak> = Vec::new();
    for &day in days.iter().filter(|day| calendar.is_working_day(**day)) {
        match result.last_mut() {
            Some(last) if calendar.previous_working_day(day) == Some(last.end) => {
                last.end = day;
                last.days += 1;
            }
            _ => result.push(Streak {
                start: day,
                end: day,
                days: 1,
            }),
        }
    }
    result
}

/// Where a student stands, as shown on their page.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct Progress {
    /// The streak still alive: its last day is today or the previous working day, so a
    /// share today would extend it. 0 when there is none.
    pub current: u32,
    pub best: u32,
    /// The first tier above `current`.
    pub next: Option<Tier>,
    /// Today is a working day and already has a share.
    pub today_counted: bool,
}

pub fn progress(
    days: &BTreeSet<Date>,
    today: Date,
    calendar: &Calendar,
    tiers: &[Tier],
) -> Progress {
    let all = streaks(days, calendar);
    let best = all.iter().map(|streak| streak.days).max().unwrap_or(0);
    let current = all
        .last()
        .filter(|streak| {
            streak.end == today || Some(streak.end) == calendar.previous_working_day(today)
        })
        .map_or(0, |streak| streak.days);
    let next = tiers
        .iter()
        .filter(|tier| tier.days > current)
        .min_by_key(|tier| tier.days)
        .copied();
    Progress {
        current,
        best,
        next,
        today_counted: calendar.is_working_day(today) && days.contains(&today),
    }
}

/// The tier a share claimed on `today` completes, if any, given the days that already had
/// a share: `today` must be a new working day that brings its streak exactly to a tier's
/// length. Whether that bonus was already paid is for the caller to check.
pub fn completed_tier(
    days_before: &BTreeSet<Date>,
    today: Date,
    calendar: &Calendar,
    tiers: &[Tier],
) -> Option<(Streak, Tier)> {
    if days_before.contains(&today) || !calendar.is_working_day(today) {
        return None;
    }
    let mut days = days_before.clone();
    days.insert(today);
    let streak = streaks(&days, calendar)
        .into_iter()
        .find(|streak| streak.end == today)?;
    let tier = tiers.iter().find(|tier| tier.days == streak.days)?;
    Some((streak, *tier))
}

/// The first day on which a streak reached `length` working days.
pub fn first_day_reaching(days: &BTreeSet<Date>, calendar: &Calendar, length: u32) -> Option<Date> {
    let mut run: Option<(Date, u32)> = None;
    for &day in days.iter().filter(|day| calendar.is_working_day(**day)) {
        let count = match run {
            Some((end, count)) if calendar.previous_working_day(day) == Some(end) => count + 1,
            _ => 1,
        };
        if count == length {
            return Some(day);
        }
        run = Some((day, count));
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use time::macros::date;

    /// Monday to Friday, with one closure on Wednesday 2026-11-11.
    fn campus() -> Calendar {
        Calendar {
            weekdays: (1..=5).collect(),
            closures: [date!(2026 - 11 - 11)].into_iter().collect(),
        }
    }

    fn tiers() -> Vec<Tier> {
        vec![
            Tier {
                days: 5,
                bonus: 2_000,
            },
            Tier {
                days: 10,
                bonus: 5_000,
            },
        ]
    }

    fn set(days: &[Date]) -> BTreeSet<Date> {
        days.iter().copied().collect()
    }

    // 2026-11-02 is a Monday.

    #[test]
    fn weekends_do_not_break_a_streak() {
        let days = set(&[
            date!(2026 - 11 - 05),
            date!(2026 - 11 - 06),
            date!(2026 - 11 - 09),
        ]);
        let all = streaks(&days, &campus());
        assert_eq!(all.len(), 1);
        assert_eq!(all[0].days, 3);
        assert_eq!(all[0].start, date!(2026 - 11 - 05));
    }

    #[test]
    fn closures_do_not_break_a_streak() {
        // Tuesday, (Wednesday closed), Thursday.
        let days = set(&[date!(2026 - 11 - 10), date!(2026 - 11 - 12)]);
        assert_eq!(streaks(&days, &campus())[0].days, 2);
    }

    #[test]
    fn a_missed_working_day_breaks_a_streak() {
        let days = set(&[date!(2026 - 11 - 02), date!(2026 - 11 - 04)]);
        let all = streaks(&days, &campus());
        assert_eq!(all.iter().map(|s| s.days).collect::<Vec<_>>(), vec![1, 1]);
    }

    #[test]
    fn a_share_on_a_closed_day_counts_for_nothing() {
        let days = set(&[date!(2026 - 11 - 07), date!(2026 - 11 - 11)]);
        assert!(streaks(&days, &campus()).is_empty());
        let days = set(&[
            date!(2026 - 11 - 06),
            date!(2026 - 11 - 07),
            date!(2026 - 11 - 09),
        ]);
        assert_eq!(streaks(&days, &campus())[0].days, 2);
    }

    #[test]
    fn the_current_streak_survives_until_the_next_working_day() {
        let days = set(&[date!(2026 - 11 - 05), date!(2026 - 11 - 06)]);
        let on = |today| progress(&days, today, &campus(), &tiers());
        // Friday itself, the weekend, and Monday morning: still alive.
        assert_eq!(on(date!(2026 - 11 - 06)).current, 2);
        assert_eq!(on(date!(2026 - 11 - 08)).current, 2);
        assert_eq!(on(date!(2026 - 11 - 09)).current, 2);
        // Tuesday without a share on Monday: broken, the best remains.
        let broken = on(date!(2026 - 11 - 10));
        assert_eq!(broken.current, 0);
        assert_eq!(broken.best, 2);
        assert_eq!(
            broken.next,
            Some(Tier {
                days: 5,
                bonus: 2_000
            })
        );
    }

    #[test]
    fn progress_names_the_next_tier_and_whether_today_counts() {
        let days = set(&[
            date!(2026 - 11 - 02),
            date!(2026 - 11 - 03),
            date!(2026 - 11 - 04),
            date!(2026 - 11 - 05),
            date!(2026 - 11 - 06),
        ]);
        let friday = progress(&days, date!(2026 - 11 - 06), &campus(), &tiers());
        assert_eq!(friday.current, 5);
        assert_eq!(
            friday.next,
            Some(Tier {
                days: 10,
                bonus: 5_000
            })
        );
        assert!(friday.today_counted);
        let monday = progress(&days, date!(2026 - 11 - 09), &campus(), &tiers());
        assert!(!monday.today_counted);
        let past_all = progress(&days, date!(2026 - 11 - 06), &campus(), &tiers()[..1]);
        assert_eq!(past_all.next, None);
    }

    #[test]
    fn the_fifth_working_day_completes_the_first_tier_exactly_once() {
        let before = set(&[
            date!(2026 - 11 - 02),
            date!(2026 - 11 - 03),
            date!(2026 - 11 - 04),
            date!(2026 - 11 - 05),
        ]);
        let (streak, tier) =
            completed_tier(&before, date!(2026 - 11 - 06), &campus(), &tiers()).unwrap();
        assert_eq!(tier.days, 5);
        assert_eq!(streak.start, date!(2026 - 11 - 02));
        assert_eq!(streak.days, 5);

        // A second share the same day completes nothing more.
        let mut with_friday = before.clone();
        with_friday.insert(date!(2026 - 11 - 06));
        assert_eq!(
            completed_tier(&with_friday, date!(2026 - 11 - 06), &campus(), &tiers()),
            None
        );
        // The sixth day is between tiers.
        assert_eq!(
            completed_tier(&with_friday, date!(2026 - 11 - 09), &campus(), &tiers()),
            None
        );
    }

    #[test]
    fn a_closed_day_completes_no_tier() {
        let before = set(&[
            date!(2026 - 11 - 02),
            date!(2026 - 11 - 03),
            date!(2026 - 11 - 04),
            date!(2026 - 11 - 05),
            date!(2026 - 11 - 06),
        ]);
        assert_eq!(
            completed_tier(&before, date!(2026 - 11 - 07), &campus(), &tiers()),
            None
        );
    }

    #[test]
    fn first_day_reaching_a_length() {
        let days = set(&[
            date!(2026 - 11 - 02),
            date!(2026 - 11 - 04),
            date!(2026 - 11 - 05),
            date!(2026 - 11 - 06),
        ]);
        assert_eq!(
            first_day_reaching(&days, &campus(), 3),
            Some(date!(2026 - 11 - 06))
        );
        assert_eq!(first_day_reaching(&days, &campus(), 4), None);
        assert_eq!(
            first_day_reaching(&days, &campus(), 1),
            Some(date!(2026 - 11 - 02))
        );
    }

    #[test]
    fn a_calendar_without_working_days_does_not_loop() {
        let closed = Calendar {
            weekdays: BTreeSet::new(),
            closures: BTreeSet::new(),
        };
        assert_eq!(closed.previous_working_day(date!(2026 - 11 - 02)), None);
        let days = set(&[date!(2026 - 11 - 02)]);
        assert!(streaks(&days, &closed).is_empty());
        assert_eq!(
            progress(&days, date!(2026 - 11 - 02), &closed, &tiers()).current,
            0
        );
    }
}
