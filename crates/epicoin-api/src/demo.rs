//! Demonstration data: fictitious users, products and distributions.
//!
//! Every demo account is already activated with the same, published password. This is only
//! ever meant for a demo or a local trial: the command refuses to run on a database that
//! already has users.

use std::collections::HashSet;
use std::time::Duration;

use anyhow::{bail, Context};
use epicoin_core::{AuthorityAction, ReasonCode, TxPayload, UNITS_PER_EPC};
use serde_json::json;
use uuid::Uuid;

use crate::chain::TxMeta;
use crate::models::{Role, User, COHORTS};
use crate::state::AppState;
use crate::users::{self, NewUser};
use crate::{audit, secrets, wallets};

pub const STUDENTS: usize = 150;
pub const STAFF: usize = 5;
const WAIT: Duration = Duration::from_secs(60);

const FIRST_NAMES: [&str; 30] = [
    "Camille", "Léa", "Hugo", "Lucas", "Chloé", "Emma", "Louis", "Inès", "Jules", "Manon",
    "Nathan", "Sarah", "Théo", "Zoé", "Adam", "Lina", "Raphaël", "Jade", "Arthur", "Louise",
    "Noah", "Alice", "Gabriel", "Rose", "Ethan", "Anna", "Mathis", "Lou", "Yanis", "Nina",
];

const LAST_NAMES: [&str; 30] = [
    "Martin",
    "Bernard",
    "Dubois",
    "Thomas",
    "Robert",
    "Richard",
    "Petit",
    "Durand",
    "Leroy",
    "Moreau",
    "Simon",
    "Laurent",
    "Lefebvre",
    "Michel",
    "Garcia",
    "David",
    "Bertrand",
    "Roux",
    "Vincent",
    "Fournier",
    "Morel",
    "Girard",
    "André",
    "Mercier",
    "Dupont",
    "Lambert",
    "Bonnet",
    "François",
    "Martinez",
    "Legrand",
];

/// Name, description, price in EPC, stock.
const PRODUCTS: [(&str, &str, u64, i32); 10] = [
    (
        "Mug EpiCoin",
        "Le mug officiel, pour le café des lendemains de rush.",
        15,
        40,
    ),
    (
        "Sweat à capuche",
        "Sweat brodé, coton bio. Préciser la taille au retrait.",
        120,
        25,
    ),
    ("T-shirt Epitech", "T-shirt noir, logo blanc.", 45, 60),
    (
        "Lot de stickers",
        "Dix stickers pour décorer votre ordinateur.",
        5,
        200,
    ),
    (
        "Gourde isotherme",
        "Garde l'eau fraîche pendant les longues soirées de code.",
        35,
        30,
    ),
    (
        "Carnet de notes",
        "Pour les schémas d'architecture faits à la main.",
        10,
        80,
    ),
    (
        "Clavier mécanique",
        "Switches tactiles, disposition AZERTY.",
        400,
        5,
    ),
    (
        "Place réservée en salle",
        "Une place réservée pendant une semaine en salle de projet.",
        60,
        10,
    ),
    (
        "Coaching CV",
        "Trente minutes avec l'équipe relations entreprises.",
        80,
        12,
    ),
    (
        "Repas offert",
        "Un repas à la cafétéria partenaire.",
        20,
        50,
    ),
];

/// One staff distribution: reason, amount in EPC per student, how many students, comment.
const DISTRIBUTIONS: [(ReasonCode, u64, usize, &str); 3] = [
    (
        ReasonCode::Participation,
        10,
        20,
        "Merci pour votre présence à la soutenance.",
    ),
    (
        ReasonCode::PeerHelp,
        15,
        10,
        "Pour l'aide apportée aux autres groupes.",
    ),
    (
        ReasonCode::Challenge,
        20,
        5,
        "Vainqueurs du défi algorithmique de la semaine.",
    ),
];

fn email_local_part(first: &str, last: &str) -> String {
    let fold = |text: &str| -> String {
        text.chars()
            .map(|c| match c {
                'à' | 'â' | 'ä' => 'a',
                'é' | 'è' | 'ê' | 'ë' => 'e',
                'î' | 'ï' => 'i',
                'ô' | 'ö' => 'o',
                'ù' | 'û' | 'ü' => 'u',
                'ç' => 'c',
                other => other,
            })
            .filter(|c| c.is_ascii_alphanumeric())
            .collect::<String>()
            .to_lowercase()
    };
    format!("{}.{}", fold(first), fold(last))
}

/// The demo population: deterministic, with unique emails.
pub fn people(domain: &str) -> Vec<NewUser> {
    let mut seen = HashSet::new();
    let mut unique_email = |first: &str, last: &str| {
        let base = email_local_part(first, last);
        let mut local = base.clone();
        let mut n = 2;
        while !seen.insert(local.clone()) {
            local = format!("{base}{n}");
            n += 1;
        }
        format!("{local}@{domain}")
    };
    let person = |email: String, first: &str, last: &str, promo: &str, role| NewUser {
        email,
        first_name: first.to_owned(),
        last_name: last.to_owned(),
        promo: promo.to_owned(),
        role,
    };

    let mut people = vec![
        person(
            unique_email("admin", "demo"),
            "Alex",
            "Admin",
            "",
            Role::Admin,
        ),
        person(
            unique_email("boutique", "demo"),
            "Bastien",
            "Boutique",
            "",
            Role::ShopManager,
        ),
    ];
    for i in 0..STAFF {
        let (first, last) = (FIRST_NAMES[(i * 7 + 3) % 30], LAST_NAMES[(i * 11 + 5) % 30]);
        let email = unique_email(&format!("encadrant{}", i + 1), "demo");
        people.push(person(email, first, last, "", Role::Staff));
    }
    for i in 0..STUDENTS {
        let first = FIRST_NAMES[i % 30];
        let last = LAST_NAMES[(i / 30 + i * 7) % 30];
        let email = unique_email(first, last);
        people.push(person(
            email,
            first,
            last,
            COHORTS[i % COHORTS.len()],
            Role::Student,
        ));
    }
    people
}

#[derive(Debug)]
pub struct SeedReport {
    pub accounts: Vec<(Role, String)>,
    pub students: usize,
    pub products: usize,
    pub mints: usize,
}

/// Fills an empty platform with demo data. Fails if any user already exists.
///
/// With `distributions`, the staff also reward some students, so that balances, histories
/// and the dashboard have content; without, every balance starts at zero.
pub async fn seed(
    state: &AppState,
    password: &str,
    distributions: bool,
) -> anyhow::Result<SeedReport> {
    let existing: i64 = sqlx::query_scalar("SELECT count(*) FROM users")
        .fetch_one(&state.db)
        .await?;
    if existing > 0 {
        bail!("the database already has users: demo data is only seeded on an empty platform");
    }
    state
        .chain
        .info()
        .await
        .context("the node must be running to seed demo data")?;

    // --- Accounts, all activated with the demo password.
    let password_hash = secrets::hash_password(password)
        .map_err(|e| anyhow::anyhow!("cannot hash the demo password: {e}"))?;
    let mut tx = state.db.begin().await?;
    let mut created: Vec<User> = Vec::new();
    for person in people(&state.settings.allowed_email_domain) {
        let person = users::validate(&person, &state.settings)
            .map_err(|message| anyhow::anyhow!("invalid demo user {}: {message}", person.email))?;
        let user = users::create(&mut tx, &state.vault, &person)
            .await
            .map_err(|e| anyhow::anyhow!("cannot create {}: {}", person.email, e.message))?;
        created.push(user);
    }
    sqlx::query("UPDATE users SET password_hash = $1, activated_at = now()")
        .bind(&password_hash)
        .execute(&mut *tx)
        .await?;

    // --- Products.
    for (name, description, price, stock) in PRODUCTS {
        sqlx::query(
            "INSERT INTO products (id, name, description, price, stock) VALUES ($1, $2, $3, $4, $5)",
        )
        .bind(Uuid::new_v4())
        .bind(name)
        .bind(description)
        .bind((price * UNITS_PER_EPC) as i64)
        .bind(stock)
        .execute(&mut *tx)
        .await?;
    }
    audit::record(
        &mut *tx,
        None,
        "demo.seeded",
        None,
        json!({ "users": created.len(), "products": PRODUCTS.len() }),
    )
    .await?;
    tx.commit().await?;

    let staff: Vec<&User> = created.iter().filter(|u| u.role == Role::Staff).collect();
    let students: Vec<&User> = created.iter().filter(|u| u.role == Role::Student).collect();

    // --- Mint authorities for the staff, in one block.
    let mut grants = Vec::new();
    for member in &staff {
        let address = wallets::address_of(&state.db, member.id)
            .await
            .map_err(|e| anyhow::anyhow!(e.message))?;
        grants.push((
            TxPayload::AuthorityUpdate {
                authority: address,
                action: AuthorityAction::Grant {
                    quota: state.settings.default_mint_quota,
                },
            },
            TxMeta {
                subject_user_id: Some(member.id),
                amount: Some(state.settings.default_mint_quota),
                ..TxMeta::default()
            },
        ));
    }
    let rows = state
        .chain
        .send_batch(&state.admin_key, grants, WAIT)
        .await?;
    if rows.len() != staff.len() || rows.iter().any(|row| !row.is_included()) {
        bail!("the staff authorities could not be granted on chain");
    }

    // --- Each staff member rewards a group of students. Different signers: in parallel.
    let mut tasks = Vec::new();
    for (index, member) in staff.iter().enumerate().filter(|_| distributions) {
        let state = state.clone();
        let staff_id = member.id;
        // Each staff member follows a different slice of the students.
        let group: Vec<Uuid> = students
            .iter()
            .cycle()
            .skip(index * students.len() / staff.len())
            .take(DISTRIBUTIONS.iter().map(|d| d.2).max().unwrap_or(0) + 10)
            .map(|student| student.id)
            .collect();
        let addresses = {
            let mut addresses = Vec::new();
            for id in &group {
                addresses.push(
                    wallets::address_of(&state.db, *id)
                        .await
                        .map_err(|e| anyhow::anyhow!(e.message))?,
                );
            }
            addresses
        };
        tasks.push(tokio::spawn(async move {
            let keypair = wallets::load_keypair(&state.db, &state.vault, staff_id)
                .await
                .map_err(|e| anyhow::anyhow!(e.message))?;
            let mut minted = 0;
            for (round, (reason, amount, count, comment)) in DISTRIBUTIONS.iter().enumerate() {
                let batch_id = Uuid::new_v4();
                let reason_name = serde_json::to_value(reason)?
                    .as_str()
                    .unwrap_or_default()
                    .to_owned();
                let units = amount * UNITS_PER_EPC;
                sqlx::query(
                    "INSERT INTO mint_batches \
                     (id, actor_user_id, amount, recipient_count, reason_code, comment) \
                     VALUES ($1, $2, $3, $4, $5, $6)",
                )
                .bind(batch_id)
                .bind(staff_id)
                .bind(units as i64)
                .bind(*count as i32)
                .bind(&reason_name)
                .bind(*comment)
                .execute(&state.db)
                .await?;
                let items = group
                    .iter()
                    .zip(&addresses)
                    .skip(round * 3)
                    .take(*count)
                    .map(|(student, address)| {
                        (
                            TxPayload::Mint {
                                to: *address,
                                amount: units,
                                reason: *reason,
                            },
                            TxMeta {
                                actor_user_id: Some(staff_id),
                                subject_user_id: Some(*student),
                                amount: Some(units),
                                reason_code: Some(reason_name.clone()),
                                batch_id: Some(batch_id),
                                order_id: None,
                            },
                        )
                    })
                    .collect();
                let rows = state.chain.send_batch(&keypair, items, WAIT).await?;
                minted += rows.iter().filter(|row| row.is_included()).count();
            }
            anyhow::Ok(minted)
        }));
    }
    let mut mints = 0;
    for task in tasks {
        mints += task.await.context("a demo distribution task panicked")??;
    }

    let accounts = created
        .iter()
        .filter(|user| user.role != Role::Student)
        .map(|user| (user.role, user.email.clone()))
        .chain(students.first().map(|s| (Role::Student, s.email.clone())))
        .collect();
    Ok(SeedReport {
        accounts,
        students: students.len(),
        products: PRODUCTS.len(),
        mints,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Settings;

    #[test]
    fn the_demo_population_is_valid_and_unique() {
        let settings = Settings::development();
        let people = people(&settings.allowed_email_domain);
        assert_eq!(people.len(), 2 + STAFF + STUDENTS);
        assert_eq!(
            people.iter().filter(|p| p.role == Role::Student).count(),
            STUDENTS
        );
        assert_eq!(
            people.iter().filter(|p| p.role == Role::Staff).count(),
            STAFF
        );
        let emails: HashSet<&str> = people.iter().map(|p| p.email.as_str()).collect();
        assert_eq!(emails.len(), people.len(), "emails are unique");
        for person in &people {
            assert!(
                users::validate(person, &settings).is_ok(),
                "{}",
                person.email
            );
        }
        assert!(emails.contains("admin.demo@epitech.eu"));
        assert!(emails.contains("encadrant1.demo@epitech.eu"));
    }

    #[test]
    fn accents_are_folded_in_emails() {
        assert_eq!(email_local_part("Léa", "André"), "lea.andre");
        assert_eq!(email_local_part("Raphaël", "François"), "raphael.francois");
    }

    #[test]
    fn demo_distributions_fit_in_the_default_quota() {
        let per_staff: u64 = DISTRIBUTIONS
            .iter()
            .map(|(_, amount, count, _)| amount * *count as u64)
            .sum();
        assert!(per_staff * UNITS_PER_EPC <= Settings::development().default_mint_quota);
    }
}
