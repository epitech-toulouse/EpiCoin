//! Database rows and their JSON views.

use serde::{Deserialize, Serialize};
use time::OffsetDateTime;
use uuid::Uuid;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Role {
    Student,
    Staff,
    ShopManager,
    Admin,
}

impl Role {
    pub const ALL: [Role; 4] = [Role::Student, Role::Staff, Role::ShopManager, Role::Admin];

    pub fn as_str(self) -> &'static str {
        match self {
            Role::Student => "student",
            Role::Staff => "staff",
            Role::ShopManager => "shop_manager",
            Role::Admin => "admin",
        }
    }

    /// The roles that distribute EPC, each with an on-chain mint authority and a quota.
    pub const DISTRIBUTORS: [Role; 2] = [Role::Staff, Role::Admin];

    pub fn distributes(self) -> bool {
        Self::DISTRIBUTORS.contains(&self)
    }

    /// Parses the stored value, and the French names accepted in CSV imports.
    pub fn parse(text: &str) -> Option<Self> {
        match text.trim().to_lowercase().as_str() {
            "student" | "etudiant" | "étudiant" | "etudiante" | "étudiante" => {
                Some(Role::Student)
            }
            "staff" | "encadrant" | "encadrante" => Some(Role::Staff),
            "shop_manager" | "boutique" | "gestionnaire" => Some(Role::ShopManager),
            "admin" | "administrateur" | "administratrice" => Some(Role::Admin),
            _ => None,
        }
    }
}

impl TryFrom<String> for Role {
    type Error = String;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        Role::parse(&value).ok_or_else(|| format!("unknown role `{value}`"))
    }
}

/// Student cohorts, in display order. `users.promo` holds one of these codes, or `''` for
/// none. A new MSc year is one more line here: nothing else lists them.
pub const COHORTS: [&str; 9] = [
    "msc2027", "msc2028", "msc2029", "pge1", "pge2", "pge3", "bach1", "bach2", "bach3",
];

/// The `promo` filter value that selects students without a cohort from [`COHORTS`].
pub const NO_COHORT: &str = "none";

/// Recognizes a cohort however it is typed: `PGE 1`, `MSc-2027`, `Bachelor 2`…
pub fn parse_cohort(text: &str) -> Option<&'static str> {
    let mut compact: String = text
        .chars()
        .filter(|c| !c.is_whitespace() && *c != '-' && *c != '_')
        .flat_map(char::to_lowercase)
        .collect();
    if let Some(rest) = compact.strip_prefix("bachelor") {
        compact = format!("bach{rest}");
    }
    COHORTS.into_iter().find(|code| *code == compact)
}

/// A `users` row. Contains the password hash: never serialized — see [`UserView`].
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct User {
    pub id: Uuid,
    pub email: String,
    pub first_name: String,
    pub last_name: String,
    pub promo: String,
    #[sqlx(try_from = "String")]
    pub role: Role,
    pub password_hash: Option<String>,
    pub mint_quota: Option<i64>,
    pub activated_at: Option<OffsetDateTime>,
    pub disabled_at: Option<OffsetDateTime>,
    pub created_at: OffsetDateTime,
}

impl User {
    pub fn is_active(&self) -> bool {
        self.activated_at.is_some() && self.disabled_at.is_none()
    }
}

/// What the API returns about a user.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UserView {
    pub id: Uuid,
    pub email: String,
    pub first_name: String,
    pub last_name: String,
    pub promo: String,
    pub role: Role,
    pub activated: bool,
    pub disabled: bool,
    /// Personal mint quota in units, if one was set (staff only).
    pub mint_quota: Option<i64>,
    #[serde(with = "time::serde::rfc3339")]
    pub created_at: OffsetDateTime,
}

impl From<&User> for UserView {
    fn from(user: &User) -> Self {
        Self {
            id: user.id,
            email: user.email.clone(),
            first_name: user.first_name.clone(),
            last_name: user.last_name.clone(),
            promo: user.promo.clone(),
            role: user.role,
            activated: user.activated_at.is_some(),
            disabled: user.disabled_at.is_some(),
            mint_quota: user.mint_quota,
            created_at: user.created_at,
        }
    }
}

/// Paging parameters shared by list endpoints.
#[derive(Debug, Clone, Copy, Deserialize)]
pub struct Page {
    pub limit: Option<i64>,
    pub offset: Option<i64>,
}

impl Page {
    pub fn limit(&self) -> i64 {
        self.limit.unwrap_or(50).clamp(1, 200)
    }

    pub fn offset(&self) -> i64 {
        self.offset.unwrap_or(0).max(0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roles_round_trip_and_accept_french_names() {
        for role in Role::ALL {
            assert_eq!(Role::parse(role.as_str()), Some(role));
            assert_eq!(
                serde_json::to_string(&role).unwrap(),
                format!("\"{}\"", role.as_str())
            );
        }
        assert_eq!(Role::parse(" Étudiant "), Some(Role::Student));
        assert_eq!(Role::parse("ENCADRANT"), Some(Role::Staff));
        assert_eq!(Role::parse("boutique"), Some(Role::ShopManager));
        assert_eq!(Role::parse("superuser"), None);
        assert!(Role::try_from("nope".to_owned()).is_err());
    }

    #[test]
    fn cohorts_are_recognized_however_they_are_typed() {
        for code in COHORTS {
            assert_eq!(parse_cohort(code), Some(code));
        }
        assert_eq!(parse_cohort(" PGE 1 "), Some("pge1"));
        assert_eq!(parse_cohort("MSc-2027"), Some("msc2027"));
        assert_eq!(parse_cohort("msc_2029"), Some("msc2029"));
        assert_eq!(parse_cohort("Bachelor 3"), Some("bach3"));
        assert_eq!(parse_cohort("BACH2"), Some("bach2"));
        for unknown in ["", "2027", "pge4", "msc2026", "pge", NO_COHORT] {
            assert_eq!(parse_cohort(unknown), None, "{unknown}");
        }
    }

    #[test]
    fn paging_is_clamped() {
        let page = Page {
            limit: Some(10_000),
            offset: Some(-5),
        };
        assert_eq!(page.limit(), 200);
        assert_eq!(page.offset(), 0);
        let default = Page {
            limit: None,
            offset: None,
        };
        assert_eq!(default.limit(), 50);
    }
}
