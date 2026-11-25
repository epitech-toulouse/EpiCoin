//! Badges and mentions. Pure functions, no I/O.
//!
//! Every badge is computed from the student's valid shares (claimed, payment not refused,
//! not cancelled), except "Légende du campus", given by hand by an admin. Computing instead
//! of storing means a cancelled share takes its badges with it, with nothing to reconcile.
//! Badges and mentions never pay anything. Their names and descriptions live in the front.

use std::collections::{BTreeMap, BTreeSet, HashSet};

use serde::Serialize;
use time::{Date, OffsetDateTime};
use uuid::Uuid;

use super::calendar::{first_day_reaching, Calendar};

/// What a staff member may add to a share, from the most common to the rarest. The last
/// one is reserved to admins ([`ADMIN_MENTION`]).
pub const MENTIONS: [&str; 10] = [
    "genius",
    "wtf",
    "mind_blown",
    "life_hack",
    "plot_twist",
    "showtime",
    "rabbit_hole",
    "mad_scientist",
    "culture_shock",
    "legendary",
];

pub const ADMIN_MENTION: &str = "legendary";

/// A first valid share.
pub const FIRST: &str = "first";
/// [`TIRELESS_DAYS`] consecutive working days.
pub const TIRELESS: &str = "tireless";
pub const TIRELESS_DAYS: u32 = 20;
/// [`ENCYCLOPEDIA_SHARES`] valid shares.
pub const ENCYCLOPEDIA: &str = "encyclopedia";
pub const ENCYCLOPEDIA_SHARES: usize = 50;
/// Shares validated by [`TOUR_STAFF`] different staff members.
pub const TOUR: &str = "tour";
pub const TOUR_STAFF: usize = 5;
/// A share paid more than the suggested amount of the time.
pub const TIP: &str = "tip";
/// Given by an admin.
pub const LEGEND: &str = "legend";

/// One valid share, as far as badges are concerned.
#[derive(Debug, Clone)]
pub struct Fact {
    pub claimed_at: OffsetDateTime,
    pub claim_day: Date,
    pub staff: Option<Uuid>,
    pub amount: i64,
    pub standard: i64,
    pub mention: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Earned {
    pub code: String,
    #[serde(with = "time::serde::rfc3339")]
    pub obtained_at: OffsetDateTime,
    /// How many times: mentions only, 1 otherwise.
    pub count: u32,
}

fn earned(code: &str, obtained_at: OffsetDateTime) -> Earned {
    Earned {
        code: code.to_owned(),
        obtained_at,
        count: 1,
    }
}

/// The badges of a student. `facts` must be sorted by `claimed_at`.
pub fn compute(facts: &[Fact], calendar: &Calendar, legend: Option<OffsetDateTime>) -> Vec<Earned> {
    let mut badges = Vec::new();
    if let Some(first) = facts.first() {
        badges.push(earned(FIRST, first.claimed_at));
    }
    let days: BTreeSet<Date> = facts.iter().map(|fact| fact.claim_day).collect();
    if let Some(day) = first_day_reaching(&days, calendar, TIRELESS_DAYS) {
        if let Some(fact) = facts.iter().find(|fact| fact.claim_day == day) {
            badges.push(earned(TIRELESS, fact.claimed_at));
        }
    }
    if let Some(fact) = facts.get(ENCYCLOPEDIA_SHARES - 1) {
        badges.push(earned(ENCYCLOPEDIA, fact.claimed_at));
    }
    let mut staff = HashSet::new();
    if let Some(fact) = facts
        .iter()
        .find(|fact| fact.staff.is_some_and(|id| staff.insert(id)) && staff.len() == TOUR_STAFF)
    {
        badges.push(earned(TOUR, fact.claimed_at));
    }
    if let Some(fact) = facts.iter().find(|fact| fact.amount > fact.standard) {
        badges.push(earned(TIP, fact.claimed_at));
    }
    if let Some(at) = legend {
        badges.push(earned(LEGEND, at));
    }

    let mut mentions: BTreeMap<usize, Earned> = BTreeMap::new();
    for fact in facts {
        let Some(index) = fact
            .mention
            .as_deref()
            .and_then(|mention| MENTIONS.iter().position(|known| *known == mention))
        else {
            continue;
        };
        mentions
            .entry(index)
            .and_modify(|badge| badge.count += 1)
            .or_insert_with(|| earned(MENTIONS[index], fact.claimed_at));
    }
    badges.extend(mentions.into_values());
    badges
}

#[cfg(test)]
mod tests {
    use super::*;
    use time::macros::{date, datetime};
    use time::Duration;

    fn calendar() -> Calendar {
        Calendar {
            weekdays: (1..=5).collect(),
            closures: BTreeSet::new(),
        }
    }

    /// One share a day from Monday 2026-01-05, working days only.
    fn facts(count: usize, staff: &[Uuid]) -> Vec<Fact> {
        let mut day = date!(2026 - 01 - 05);
        let mut result = Vec::new();
        while result.len() < count {
            if calendar().is_working_day(day) {
                result.push(Fact {
                    claimed_at: day.midnight().assume_utc() + Duration::hours(10),
                    claim_day: day,
                    staff: Some(staff[result.len() % staff.len()]),
                    amount: 1_000,
                    standard: 1_000,
                    mention: None,
                });
            }
            day = day.next_day().unwrap();
        }
        result
    }

    fn codes(badges: &[Earned]) -> Vec<&str> {
        badges.iter().map(|badge| badge.code.as_str()).collect()
    }

    #[test]
    fn nothing_without_shares() {
        assert!(compute(&[], &calendar(), None).is_empty());
    }

    #[test]
    fn the_first_share_gives_the_first_badge() {
        let facts = facts(1, &[Uuid::new_v4()]);
        let badges = compute(&facts, &calendar(), None);
        assert_eq!(codes(&badges), vec![FIRST]);
        assert_eq!(badges[0].obtained_at, facts[0].claimed_at);
    }

    #[test]
    fn twenty_working_days_in_a_row_and_fifty_shares() {
        let staff = [Uuid::new_v4()];
        let nineteen = compute(&facts(19, &staff), &calendar(), None);
        assert!(!codes(&nineteen).contains(&TIRELESS));
        let twenty = facts(20, &staff);
        let badges = compute(&twenty, &calendar(), None);
        let tireless = badges.iter().find(|b| b.code == TIRELESS).unwrap();
        assert_eq!(tireless.obtained_at, twenty[19].claimed_at);
        assert!(!codes(&badges).contains(&ENCYCLOPEDIA));

        let fifty = facts(50, &staff);
        let badges = compute(&fifty, &calendar(), None);
        let encyclopedia = badges.iter().find(|b| b.code == ENCYCLOPEDIA).unwrap();
        assert_eq!(encyclopedia.obtained_at, fifty[49].claimed_at);
    }

    #[test]
    fn five_different_staff_members_make_the_tour() {
        let four: Vec<Uuid> = (0..4).map(|_| Uuid::new_v4()).collect();
        assert!(!codes(&compute(&facts(12, &four), &calendar(), None)).contains(&TOUR));
        let five: Vec<Uuid> = (0..5).map(|_| Uuid::new_v4()).collect();
        let facts = facts(12, &five);
        let badges = compute(&facts, &calendar(), None);
        let tour = badges.iter().find(|b| b.code == TOUR).unwrap();
        assert_eq!(tour.obtained_at, facts[4].claimed_at);
    }

    #[test]
    fn more_than_the_suggested_amount_is_a_tip() {
        let mut facts = facts(3, &[Uuid::new_v4()]);
        assert!(!codes(&compute(&facts, &calendar(), None)).contains(&TIP));
        facts[1].amount = 1_500;
        let badges = compute(&facts, &calendar(), None);
        assert_eq!(
            badges.iter().find(|b| b.code == TIP).unwrap().obtained_at,
            facts[1].claimed_at
        );
    }

    #[test]
    fn mentions_are_counted_in_catalogue_order_and_the_legend_is_given() {
        let mut facts = facts(4, &[Uuid::new_v4()]);
        facts[0].mention = Some("wtf".into());
        facts[1].mention = Some("genius".into());
        facts[2].mention = Some("wtf".into());
        facts[3].mention = Some("unknown".into());
        let at = datetime!(2026-02-01 12:00 UTC);
        let badges = compute(&facts, &calendar(), Some(at));
        assert_eq!(codes(&badges), vec![FIRST, LEGEND, "genius", "wtf"]);
        let wtf = badges.iter().find(|b| b.code == "wtf").unwrap();
        assert_eq!(wtf.count, 2);
        assert_eq!(wtf.obtained_at, facts[0].claimed_at);
    }

    #[test]
    fn the_admin_mention_is_a_known_mention() {
        assert!(MENTIONS.contains(&ADMIN_MENTION));
    }
}
