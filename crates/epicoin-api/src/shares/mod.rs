//! Knowledge shares: "Apprends-nous quelque chose !" (formerly the separate Epicreds app).
//!
//! A student teaches a staff member something: an anecdote, a trick, a discovery. The staff
//! member (or an admin) generates a QR code for an amount, optionally with a mention and the
//! topic; the student scans it within a couple of minutes and is paid at once.
//!
//! - [`manage`]: QR codes (staff and admins), their status and history; cancellation, legend
//!   badges and settings (admins).
//! - [`claim`]: the student side of a QR code, and the payment.
//! - [`progress`]: a student's streak, badges and shares; the ranking.
//! - [`calendar`] / [`badges`]: the pure rules (working days, streak tiers, badges).
//!
//! # Paying exactly once
//!
//! The payment is **one** `Mint` signed with the key of the staff member who generated the
//! QR code, reason `knowledge_share`, counted against their quota like a challenge reward.
//! When the scan completes a streak tier (say 5 consecutive working days), the bonus is
//! added to that same mint and paid by the same person: no key or authority of its own, and
//! nothing paid twice because one transaction carries everything.
//!
//! The share becomes claimed in the same SQL transaction that records the signed mint, while
//! holding the staff member's signer lock, a lock on the share row and a lock on the
//! student's row (so the same student scanning two QR codes at once cannot be paid the same
//! bonus twice, nor pass the daily limit). A share is claimed exactly when its transaction
//! exists and the chain did not refuse it; after a refusal, the QR code can be scanned again
//! until it expires.
//!
//! # What stays off chain
//!
//! Topics, mentions, streaks, badges, the ranking and cancellations live in Postgres only:
//! the chain sees a mint and its reason code. A share cancelled by an admin leaves streaks,
//! badges and the ranking, but its EPC stay: the chain cannot take them back.
//!
//! The secret of a QR code travels in the fragment of the link (`/partage#token=…`), like
//! emailed links, and reaches the API only in request bodies. Only its hash is stored.

pub mod badges;
pub mod calendar;
pub mod claim;
pub mod manage;
pub mod progress;

use std::collections::BTreeSet;

use axum::routing::{get, post, put};
use axum::Router;
use epicoin_core::Amount;
use serde::{Deserialize, Serialize};
use sqlx::types::Json;
use sqlx::PgExecutor;
use time::macros::format_description;
use time::Date;

use crate::error::{ApiError, ApiResult};
use crate::state::AppState;
use calendar::{Calendar, Tier};

/// The largest amount a single share, or a streak bonus, may be configured to: 1 000 EPC.
pub const MAX_CONFIGURABLE_AMOUNT: Amount = 1_000 * epicoin_core::UNITS_PER_EPC;
pub const MAX_TOPIC_CHARS: usize = 300;
pub const MAX_TIERS: usize = 10;
pub const MAX_CLOSURES: usize = 1_000;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/shares/settings", get(manage::settings))
        .route(
            "/shares/me",
            get(progress::mine).patch(progress::update_mine),
        )
        .route("/shares/ranking", get(progress::ranking))
        .route("/shares/preview", post(claim::preview))
        .route("/shares/claim", post(claim::claim))
        .route("/staff/shares", get(manage::history).post(manage::create))
        .route("/staff/shares/{id}", get(manage::status))
        .route("/staff/shares/{id}/invalidate", post(manage::invalidate))
        .route("/admin/shares/settings", put(manage::update_settings))
        .route("/admin/shares/{id}/cancel", post(manage::cancel))
        .route(
            "/admin/shares/legends/{user_id}",
            put(manage::grant_legend).delete(manage::remove_legend),
        )
}

// --- Settings --------------------------------------------------------------------------

/// The campus settings of the feature (`share_settings`), as the API reads and writes them.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Settings {
    /// Suggested amount of a share, in units.
    pub standard: Amount,
    /// Largest amount of a share, in units.
    pub max_amount: Amount,
    /// How long a QR code can be scanned.
    pub qr_seconds: u32,
    /// The campus time zone (IANA name): which day a share belongs to.
    pub time_zone: String,
    /// ISO weekdays the campus is open: 1 = Monday … 7 = Sunday.
    pub weekdays: Vec<u8>,
    /// Days the campus is closed, `YYYY-MM-DD`.
    pub closures: Vec<String>,
    pub tiers: Vec<Tier>,
    /// Shares per student and day; `None`: no limit.
    pub daily_limit: Option<u32>,
}

#[derive(sqlx::FromRow)]
struct SettingsRow {
    standard: i64,
    max_amount: i64,
    qr_seconds: i32,
    time_zone: String,
    weekdays: Vec<i16>,
    closures: Vec<Date>,
    tiers: Json<Vec<Tier>>,
    daily_limit: Option<i32>,
}

pub fn iso_date(day: Date) -> String {
    day.format(format_description!("[year]-[month]-[day]"))
        .unwrap_or_default()
}

pub fn parse_iso_date(text: &str) -> Option<Date> {
    Date::parse(text.trim(), format_description!("[year]-[month]-[day]")).ok()
}

impl Settings {
    pub async fn load(db: impl PgExecutor<'_>) -> ApiResult<Self> {
        let row = sqlx::query_as::<_, SettingsRow>(
            "SELECT standard, max_amount, qr_seconds, time_zone, weekdays, closures, tiers, \
                    daily_limit FROM share_settings WHERE id = 1",
        )
        .fetch_one(db)
        .await?;
        Ok(Self {
            standard: row.standard.max(0) as Amount,
            max_amount: row.max_amount.max(0) as Amount,
            qr_seconds: row.qr_seconds.max(0) as u32,
            time_zone: row.time_zone,
            weekdays: row
                .weekdays
                .into_iter()
                .filter_map(|day| u8::try_from(day).ok())
                .collect(),
            closures: row.closures.into_iter().map(iso_date).collect(),
            tiers: row.tiers.0,
            daily_limit: row.daily_limit.map(|limit| limit.max(0) as u32),
        })
    }

    pub fn calendar(&self) -> Calendar {
        Calendar {
            weekdays: self.weekdays.iter().copied().collect(),
            closures: self
                .closures
                .iter()
                .filter_map(|day| parse_iso_date(day))
                .collect(),
        }
    }

    /// Checks and normalizes settings sent by an admin (sorted, without duplicates). The time
    /// zone is checked against the database's list separately.
    pub fn validated(mut self) -> Result<Self, String> {
        if self.standard == 0 || self.max_amount == 0 {
            return Err("Les montants doivent être supérieurs à zéro.".into());
        }
        if self.max_amount > MAX_CONFIGURABLE_AMOUNT {
            return Err("Le montant maximal d'un partage est limité à 1 000 EPC.".into());
        }
        if self.standard > self.max_amount {
            return Err("Le montant conseillé ne peut pas dépasser le montant maximal.".into());
        }
        if !(15..=600).contains(&self.qr_seconds) {
            return Err("Un QR code reste valable entre 15 secondes et 10 minutes.".into());
        }
        self.time_zone = self.time_zone.trim().to_owned();
        if self.time_zone.is_empty() || self.time_zone.len() > 64 {
            return Err("Fuseau horaire inconnu.".into());
        }
        let weekdays: BTreeSet<u8> = self.weekdays.iter().copied().collect();
        if weekdays.is_empty() || weekdays.iter().any(|day| !(1..=7).contains(day)) {
            return Err("Choisissez au moins un jour d'ouverture.".into());
        }
        self.weekdays = weekdays.into_iter().collect();
        if self.closures.len() > MAX_CLOSURES {
            return Err(format!("{MAX_CLOSURES} jours de fermeture au plus."));
        }
        let mut closures = BTreeSet::new();
        for text in &self.closures {
            let day = parse_iso_date(text)
                .ok_or_else(|| format!("Date de fermeture invalide : « {} ».", text.trim()))?;
            closures.insert(day);
        }
        self.closures = closures.into_iter().map(iso_date).collect();
        if self.tiers.len() > MAX_TIERS {
            return Err(format!("{MAX_TIERS} paliers au plus."));
        }
        self.tiers.sort_by_key(|tier| tier.days);
        if self
            .tiers
            .windows(2)
            .any(|pair| pair[0].days == pair[1].days)
        {
            return Err("Deux paliers ne peuvent pas avoir le même nombre de jours.".into());
        }
        for tier in &self.tiers {
            if !(1..=365).contains(&tier.days) {
                return Err("Un palier compte entre 1 et 365 jours.".into());
            }
            if tier.bonus == 0 || tier.bonus > MAX_CONFIGURABLE_AMOUNT {
                return Err("Le bonus d'un palier va de 0,01 à 1 000 EPC.".into());
            }
        }
        if self
            .daily_limit
            .is_some_and(|limit| !(1..=100).contains(&limit))
        {
            return Err("La limite quotidienne va de 1 à 100 partages.".into());
        }
        Ok(self)
    }

    pub async fn save(&self, db: impl PgExecutor<'_>) -> ApiResult<()> {
        let weekdays: Vec<i16> = self.weekdays.iter().map(|day| i16::from(*day)).collect();
        let closures: Vec<Date> = self
            .closures
            .iter()
            .filter_map(|day| parse_iso_date(day))
            .collect();
        sqlx::query(
            "UPDATE share_settings SET standard = $1, max_amount = $2, qr_seconds = $3, \
               time_zone = $4, weekdays = $5, closures = $6, tiers = $7, daily_limit = $8, \
               updated_at = now() \
             WHERE id = 1",
        )
        .bind(i64::try_from(self.standard).map_err(ApiError::internal)?)
        .bind(i64::try_from(self.max_amount).map_err(ApiError::internal)?)
        .bind(i32::try_from(self.qr_seconds).map_err(ApiError::internal)?)
        .bind(&self.time_zone)
        .bind(weekdays)
        .bind(closures)
        .bind(Json(&self.tiers))
        .bind(self.daily_limit.map(|limit| limit as i32))
        .execute(db)
        .await?;
        Ok(())
    }
}

/// Today on the campus calendar.
pub async fn today(db: impl PgExecutor<'_>, time_zone: &str) -> ApiResult<Date> {
    Ok(sqlx::query_scalar("SELECT (now() AT TIME ZONE $1)::date")
        .bind(time_zone)
        .fetch_one(db)
        .await?)
}

/// Whether Postgres knows the time zone.
pub async fn known_time_zone(db: impl PgExecutor<'_>, name: &str) -> ApiResult<bool> {
    Ok(
        sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM pg_timezone_names WHERE name = $1)")
            .bind(name)
            .fetch_one(db)
            .await?,
    )
}

/// `1250` → "12,50 EPC", `10000` → "100 EPC": amounts in French messages.
pub fn epc_fr(units: Amount) -> String {
    let (whole, cents) = (
        units / epicoin_core::UNITS_PER_EPC,
        units % epicoin_core::UNITS_PER_EPC,
    );
    if cents == 0 {
        format!("{whole} EPC")
    } else {
        format!("{whole},{cents:02} EPC")
    }
}

/// "Camille D.": how a student appears in the ranking.
pub fn short_name(first_name: &str, last_name: &str) -> String {
    match last_name.chars().next() {
        Some(initial) => format!("{first_name} {}.", initial.to_uppercase()),
        None => first_name.to_owned(),
    }
}

/// The SQL condition of a valid share `s` (claimed, payment not refused, not cancelled), its
/// transaction being `t`.
macro_rules! valid_share {
    () => {
        "s.cancelled_at IS NULL AND t.status <> 'failed'"
    };
}
pub(crate) use valid_share;

#[cfg(test)]
mod tests {
    use super::*;

    fn settings() -> Settings {
        Settings {
            standard: 1_000,
            max_amount: 10_000,
            qr_seconds: 120,
            time_zone: "Europe/Paris".into(),
            weekdays: vec![5, 1, 2, 3, 4, 1],
            closures: vec![
                "2026-12-25".into(),
                " 2026-11-11 ".into(),
                "2026-12-25".into(),
            ],
            tiers: vec![
                Tier {
                    days: 10,
                    bonus: 5_000,
                },
                Tier {
                    days: 5,
                    bonus: 2_000,
                },
            ],
            daily_limit: None,
        }
    }

    #[test]
    fn settings_are_sorted_without_duplicates() {
        let valid = settings().validated().unwrap();
        assert_eq!(valid.weekdays, vec![1, 2, 3, 4, 5]);
        assert_eq!(valid.closures, vec!["2026-11-11", "2026-12-25"]);
        assert_eq!(valid.tiers[0].days, 5);
        let calendar = valid.calendar();
        assert!(calendar
            .closures
            .contains(&time::macros::date!(2026 - 11 - 11)));
    }

    #[test]
    fn invalid_settings_are_refused() {
        let cases: Vec<fn(&mut Settings)> = vec![
            |s| s.standard = 0,
            |s| s.standard = 20_000,
            |s| s.max_amount = MAX_CONFIGURABLE_AMOUNT + 1,
            |s| s.qr_seconds = 5,
            |s| s.qr_seconds = 601,
            |s| s.time_zone = " ".into(),
            |s| s.weekdays = vec![],
            |s| s.weekdays = vec![0],
            |s| s.weekdays = vec![8],
            |s| s.closures = vec!["25/12/2026".into()],
            |s| s.tiers = vec![Tier { days: 5, bonus: 1 }, Tier { days: 5, bonus: 2 }],
            |s| s.tiers = vec![Tier { days: 0, bonus: 1 }],
            |s| {
                s.tiers = vec![Tier {
                    days: 400,
                    bonus: 1,
                }]
            },
            |s| s.tiers = vec![Tier { days: 5, bonus: 0 }],
            |s| s.tiers = (1..=11).map(|days| Tier { days, bonus: 1 }).collect(),
            |s| s.daily_limit = Some(0),
            |s| s.daily_limit = Some(101),
        ];
        for (index, change) in cases.into_iter().enumerate() {
            let mut candidate = settings();
            change(&mut candidate);
            assert!(
                candidate.validated().is_err(),
                "case {index} should be refused"
            );
        }
    }

    #[test]
    fn dates_round_trip() {
        let day = parse_iso_date("2026-02-03").unwrap();
        assert_eq!(iso_date(day), "2026-02-03");
        assert_eq!(parse_iso_date("2026-02-30"), None);
    }

    #[test]
    fn amounts_in_messages() {
        assert_eq!(epc_fr(10_000), "100 EPC");
        assert_eq!(epc_fr(1_250), "12,50 EPC");
        assert_eq!(epc_fr(5), "0,05 EPC");
    }

    #[test]
    fn ranking_names_keep_the_initial_only() {
        assert_eq!(short_name("Camille", "dupont"), "Camille D.");
        assert_eq!(short_name("Élise", "Ébrard"), "Élise É.");
        assert_eq!(short_name("Sam", ""), "Sam");
    }
}
