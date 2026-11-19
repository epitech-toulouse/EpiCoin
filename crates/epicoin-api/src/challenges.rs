//! Challenges: students who succeed get EPC at once.
//!
//! A challenge is created by a distributor (staff or admin) and checked in one of two modes:
//!
//! - `answer`: the student types the answer. It is compared with its Argon2 hash (case and
//!   spaces do not matter) and a right answer is paid immediately. Attempts are limited.
//! - `review`: the student explains how they succeeded; the creator or an admin approves
//!   (the reward leaves then) or rejects (the student may try again).
//!
//! # Paying exactly once
//!
//! The reward is a `Mint` signed with the **creator's** key, with the reason `challenge`: it
//! counts against the creator's quota like any distribution. The submission becomes
//! `approved` in the same SQL transaction that records the signed mint, while holding the
//! creator's signer lock and a lock on the challenge row. So a student is a winner exactly
//! when their reward transaction exists, the winner limit holds, and two concurrent
//! attempts cannot both pay. If the chain later refuses that transaction, the submission
//! counts as unpaid and can be paid again.

use std::time::Duration;

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::routing::{get, patch, post};
use axum::{Json, Router};
use epicoin_core::{Amount, ReasonCode, TxPayload};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use time::format_description::well_known::Rfc3339;
use time::OffsetDateTime;
use uuid::Uuid;

use crate::auth::{hash_blocking, verify_blocking, CurrentUser};
use crate::chain::{record_tx, ChainError, TxMeta, STATUS_FAILED};
use crate::error::{ApiError, ApiResult};
use crate::models::{parse_cohort, Role, User, COHORTS};
use crate::state::AppState;
use crate::users::{capitalize, double_option};
use crate::{audit, authority, mint, wallets};

const MAX_TITLE_CHARS: usize = 100;
const MAX_DESCRIPTION_CHARS: usize = 2_000;
const MAX_ANSWER_CHARS: usize = 200;
const MAX_PROOF_CHARS: usize = 1_000;
const MAX_NOTE_CHARS: usize = 280;
/// Answers a student may try per challenge within [`ATTEMPT_WINDOW`].
const MAX_ATTEMPTS: u32 = 5;
const ATTEMPT_WINDOW: Duration = Duration::from_secs(15 * 60);
/// How long a reward waits for its inclusion before answering.
const WAIT: Duration = Duration::from_secs(20);

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/challenges", get(list_for_student))
        .route("/challenges/{id}/answer", post(answer))
        .route("/challenges/{id}/submission", post(submit_proof))
        .route("/staff/challenges", get(list_managed).post(create))
        .route("/staff/challenges/{id}", patch(update))
        .route("/staff/challenges/{id}/submissions", get(submissions))
        .route(
            "/staff/challenges/{id}/submissions/{submission_id}",
            post(decide),
        )
}

// --- Model -----------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Mode {
    /// The student types the answer; a right one is paid at once.
    Answer,
    /// The student explains how they succeeded; a staff member decides.
    Review,
}

impl Mode {
    pub fn as_str(self) -> &'static str {
        match self {
            Mode::Answer => "answer",
            Mode::Review => "review",
        }
    }
}

impl TryFrom<String> for Mode {
    type Error = String;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        match value.as_str() {
            "answer" => Ok(Mode::Answer),
            "review" => Ok(Mode::Review),
            _ => Err(format!("unknown challenge mode `{value}`")),
        }
    }
}

/// A `challenges` row. Holds the answer hash: never serialized as is.
#[derive(Debug, Clone, sqlx::FromRow)]
struct Challenge {
    id: Uuid,
    creator_user_id: Option<Uuid>,
    title: String,
    description: String,
    reward: i64,
    #[sqlx(try_from = "String")]
    mode: Mode,
    answer_hash: Option<String>,
    cohorts: Vec<String>,
    max_winners: Option<i32>,
    closes_at: Option<OffsetDateTime>,
    closed_at: Option<OffsetDateTime>,
    created_at: OffsetDateTime,
}

impl Challenge {
    /// Whether it takes new participations, the winner limit aside.
    fn accepting(&self, now: OffsetDateTime) -> bool {
        self.closed_at.is_none() && self.closes_at.is_none_or(|deadline| deadline > now)
    }

    fn open_to(&self, promo: &str) -> bool {
        self.cohorts.is_empty() || self.cohorts.iter().any(|cohort| cohort == promo)
    }

    fn full(&self, winners: i64) -> bool {
        self.max_winners
            .is_some_and(|max| winners >= i64::from(max))
    }
}

/// The number of winners of the challenge `$challenge` (an SQL expression): approved
/// submissions whose reward the chain did not refuse.
macro_rules! winners_of {
    ($challenge:literal) => {
        concat!(
            "(SELECT count(*) FROM challenge_submissions w \
              JOIN chain_txs wt ON wt.hash = w.tx_hash \
              WHERE w.challenge_id = ",
            $challenge,
            " AND w.status = 'approved' AND wt.status <> 'failed')"
        )
    };
}

/// What everyone may see of a challenge.
#[derive(Debug, Serialize)]
pub struct ChallengeView {
    pub id: Uuid,
    pub title: String,
    pub description: String,
    /// Per winner, in units.
    pub reward: i64,
    pub mode: Mode,
    pub cohorts: Vec<String>,
    pub max_winners: Option<i32>,
    #[serde(with = "time::serde::rfc3339::option")]
    pub closes_at: Option<OffsetDateTime>,
    /// Takes participations right now: not closed, before its deadline, places left.
    pub open: bool,
    pub winners: i64,
    pub creator_name: Option<String>,
    #[serde(with = "time::serde::rfc3339")]
    pub created_at: OffsetDateTime,
}

impl ChallengeView {
    fn new(challenge: Challenge, winners: i64, creator_name: Option<String>) -> Self {
        let open = challenge.accepting(OffsetDateTime::now_utc()) && !challenge.full(winners);
        Self {
            id: challenge.id,
            title: challenge.title,
            description: challenge.description,
            reward: challenge.reward,
            mode: challenge.mode,
            cohorts: challenge.cohorts,
            max_winners: challenge.max_winners,
            closes_at: challenge.closes_at,
            open,
            winners,
            creator_name,
            created_at: challenge.created_at,
        }
    }
}

async fn load(state: &AppState, id: Uuid) -> ApiResult<Challenge> {
    sqlx::query_as::<_, Challenge>("SELECT * FROM challenges WHERE id = $1")
        .bind(id)
        .fetch_optional(&state.db)
        .await?
        .ok_or_else(|| ApiError::not_found("Défi"))
}

// --- Input checks ----------------------------------------------------------------------

/// The form an answer is compared in: case and runs of spaces do not matter.
pub fn normalize_answer(text: &str) -> String {
    text.split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}

fn check_answer(text: &str) -> Result<String, String> {
    let answer = normalize_answer(text);
    if answer.is_empty() {
        return Err("la réponse attendue est obligatoire".into());
    }
    if answer.chars().count() > MAX_ANSWER_CHARS {
        return Err(format!(
            "la réponse est limitée à {MAX_ANSWER_CHARS} caractères"
        ));
    }
    Ok(answer)
}

fn check_title(text: &str) -> Result<String, String> {
    let title = text.trim();
    if title.is_empty() {
        return Err("le titre est obligatoire".into());
    }
    if title.chars().count() > MAX_TITLE_CHARS {
        return Err(format!(
            "le titre est limité à {MAX_TITLE_CHARS} caractères"
        ));
    }
    Ok(title.to_owned())
}

fn check_description(text: &str) -> Result<String, String> {
    let description = text.trim();
    if description.chars().count() > MAX_DESCRIPTION_CHARS {
        return Err(format!(
            "la description est limitée à {MAX_DESCRIPTION_CHARS} caractères"
        ));
    }
    Ok(description.to_owned())
}

fn check_reward(reward: Amount) -> Result<i64, String> {
    i64::try_from(reward)
        .ok()
        .filter(|reward| *reward > 0)
        .ok_or_else(|| "le gain doit être supérieur à zéro".into())
}

/// Turns typed cohorts into their codes, without duplicates, in display order.
fn check_cohorts(cohorts: &[String]) -> Result<Vec<String>, String> {
    let mut codes = Vec::new();
    for text in cohorts {
        let code = parse_cohort(text).ok_or_else(|| format!("cohorte inconnue « {text} »"))?;
        codes.push(code);
    }
    Ok(COHORTS
        .into_iter()
        .filter(|code| codes.contains(code))
        .map(str::to_owned)
        .collect())
}

fn check_max_winners(max: Option<i32>) -> Result<Option<i32>, String> {
    match max {
        Some(max) if max < 1 => Err("le nombre de gagnants doit être d'au moins 1".into()),
        other => Ok(other),
    }
}

/// A deadline, in RFC 3339. A new deadline must be in the future.
fn check_closes_at(
    text: Option<&str>,
    now: OffsetDateTime,
) -> Result<Option<OffsetDateTime>, String> {
    let Some(text) = text.filter(|t| !t.trim().is_empty()) else {
        return Ok(None);
    };
    let deadline = OffsetDateTime::parse(text.trim(), &Rfc3339)
        .map_err(|_| "date limite invalide".to_owned())?;
    if deadline <= now {
        return Err("la date limite doit être dans le futur".into());
    }
    Ok(Some(deadline))
}

fn invalid(message: String) -> ApiError {
    ApiError::unprocessable("invalid_challenge", capitalize(&message))
}

// --- Students --------------------------------------------------------------------------

#[derive(sqlx::FromRow)]
struct StudentRow {
    #[sqlx(flatten)]
    challenge: Challenge,
    creator_name: Option<String>,
    winners: i64,
    submission_status: Option<String>,
    proof: Option<String>,
    review_note: Option<String>,
    reward_status: Option<String>,
}

/// The student's own participation.
#[derive(Debug, Serialize)]
struct MySubmission {
    /// `pending`, `approved` or `rejected`.
    status: String,
    proof: String,
    review_note: String,
    /// The reward transaction once approved: `pending`, `included` or `failed`.
    reward_status: Option<String>,
}

#[derive(Debug, Serialize)]
struct StudentChallenge {
    #[serde(flatten)]
    challenge: ChallengeView,
    submission: Option<MySubmission>,
}

/// The challenges a student can take part in, and those they already took part in.
async fn list_for_student(
    State(state): State<AppState>,
    current: CurrentUser,
) -> ApiResult<Json<Vec<StudentChallenge>>> {
    current.require(&[Role::Student])?;
    let rows = sqlx::query_as::<_, StudentRow>(concat!(
        "SELECT ch.*, cu.first_name || ' ' || cu.last_name AS creator_name, ",
        winners_of!("ch.id"),
        " AS winners, s.status AS submission_status, s.proof, s.review_note, \
           t.status AS reward_status \
         FROM challenges ch \
         LEFT JOIN users cu ON cu.id = ch.creator_user_id \
         LEFT JOIN challenge_submissions s ON s.challenge_id = ch.id AND s.student_user_id = $1 \
         LEFT JOIN chain_txs t ON t.hash = s.tx_hash \
         WHERE s.id IS NOT NULL \
            OR ((cardinality(ch.cohorts) = 0 OR $2 = ANY(ch.cohorts)) \
                AND ch.closed_at IS NULL AND (ch.closes_at IS NULL OR ch.closes_at > now())) \
         ORDER BY ch.created_at DESC LIMIT 200"
    ))
    .bind(current.id())
    .bind(&current.user.promo)
    .fetch_all(&state.db)
    .await?;

    let items = rows
        .into_iter()
        .filter_map(|row| {
            let submission = row.submission_status.map(|status| MySubmission {
                status,
                proof: row.proof.unwrap_or_default(),
                review_note: row.review_note.unwrap_or_default(),
                reward_status: row.reward_status,
            });
            let challenge = ChallengeView::new(row.challenge, row.winners, row.creator_name);
            // A full challenge only stays on the list of those who took part.
            (challenge.open || submission.is_some()).then_some(StudentChallenge {
                challenge,
                submission,
            })
        })
        .collect();
    Ok(Json(items))
}

/// A challenge the current student may take part in.
async fn challenge_for_student(
    state: &AppState,
    current: &CurrentUser,
    id: Uuid,
) -> ApiResult<Challenge> {
    current.require(&[Role::Student])?;
    let challenge = load(state, id).await?;
    if !challenge.open_to(&current.user.promo) {
        return Err(ApiError::not_found("Défi"));
    }
    if !challenge.accepting(OffsetDateTime::now_utc()) {
        return Err(ApiError::conflict(
            "challenge_closed",
            "Ce défi est terminé.",
        ));
    }
    Ok(challenge)
}

fn wrong_mode(expected: Mode) -> ApiError {
    match expected {
        Mode::Answer => ApiError::unprocessable(
            "wrong_mode",
            "Ce défi se valide par un encadrant : décrivez comment vous l'avez réussi.",
        ),
        Mode::Review => ApiError::unprocessable(
            "wrong_mode",
            "Ce défi se valide en donnant la bonne réponse.",
        ),
    }
}

#[derive(Deserialize)]
struct AnswerRequest {
    answer: String,
}

/// A right answer is paid at once; a wrong one counts as an attempt.
async fn answer(
    State(state): State<AppState>,
    current: CurrentUser,
    Path(id): Path<Uuid>,
    Json(body): Json<AnswerRequest>,
) -> ApiResult<Json<Value>> {
    let challenge = challenge_for_student(&state, &current, id).await?;
    if challenge.mode != Mode::Answer {
        return Err(wrong_mode(Mode::Answer));
    }
    if won(&state, id, current.id()).await? {
        return Err(already_completed());
    }
    if !state.limiter.hit(
        &format!("challenge:{id}:{}", current.id()),
        MAX_ATTEMPTS,
        ATTEMPT_WINDOW,
    ) {
        return Err(ApiError::too_many_requests());
    }
    let expected = challenge.answer_hash.clone().unwrap_or_default();
    let given = normalize_answer(&body.answer);
    if given.chars().count() > MAX_ANSWER_CHARS || !verify_blocking(given, expected).await {
        return Err(ApiError::unprocessable(
            "wrong_answer",
            "Ce n'est pas la bonne réponse.",
        ));
    }
    let paid = reward(&state, &challenge, current.id(), Admission::Answer).await?;
    Ok(Json(paid))
}

#[derive(Deserialize)]
struct ProofRequest {
    proof: String,
}

/// Asks for a review. A rejected (or unpaid) participation can be sent again.
async fn submit_proof(
    State(state): State<AppState>,
    current: CurrentUser,
    Path(id): Path<Uuid>,
    Json(body): Json<ProofRequest>,
) -> ApiResult<(StatusCode, Json<Value>)> {
    let challenge = challenge_for_student(&state, &current, id).await?;
    if challenge.mode != Mode::Review {
        return Err(wrong_mode(Mode::Review));
    }
    let proof = body.proof.trim();
    if proof.is_empty() || proof.chars().count() > MAX_PROOF_CHARS {
        return Err(ApiError::unprocessable(
            "invalid_proof",
            format!(
                "Expliquez comment vous avez réussi le défi ({MAX_PROOF_CHARS} caractères au plus)."
            ),
        ));
    }
    let winners: i64 = sqlx::query_scalar(concat!("SELECT ", winners_of!("$1")))
        .bind(id)
        .fetch_one(&state.db)
        .await?;
    if challenge.full(winners) {
        return Err(challenge_full());
    }

    let submitted: Option<Uuid> = sqlx::query_scalar(
        "INSERT INTO challenge_submissions (id, challenge_id, student_user_id, status, proof) \
         VALUES ($1, $2, $3, 'pending', $4) \
         ON CONFLICT (challenge_id, student_user_id) DO UPDATE \
           SET status = 'pending', proof = EXCLUDED.proof, review_note = '', \
               reviewer_user_id = NULL, tx_hash = NULL, submitted_at = now(), decided_at = NULL \
           WHERE challenge_submissions.status = 'rejected' \
              OR (challenge_submissions.status = 'approved' AND EXISTS ( \
                    SELECT 1 FROM chain_txs t \
                    WHERE t.hash = challenge_submissions.tx_hash AND t.status = 'failed')) \
         RETURNING id",
    )
    .bind(Uuid::new_v4())
    .bind(id)
    .bind(current.id())
    .bind(proof)
    .fetch_optional(&state.db)
    .await?;
    if submitted.is_none() {
        return Err(ApiError::conflict(
            "already_submitted",
            "Votre participation est déjà en attente de validation, ou déjà validée.",
        ));
    }
    Ok((StatusCode::CREATED, Json(json!({ "status": "pending" }))))
}

/// Whether `student` already won `challenge` (with a reward the chain did not refuse).
async fn won(state: &AppState, challenge: Uuid, student: Uuid) -> ApiResult<bool> {
    Ok(sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM challenge_submissions s \
           JOIN chain_txs t ON t.hash = s.tx_hash \
           WHERE s.challenge_id = $1 AND s.student_user_id = $2 \
             AND s.status = 'approved' AND t.status <> 'failed')",
    )
    .bind(challenge)
    .bind(student)
    .fetch_one(&state.db)
    .await?)
}

fn already_completed() -> ApiError {
    ApiError::conflict("already_completed", "Vous avez déjà réussi ce défi.")
}

fn challenge_full() -> ApiError {
    ApiError::conflict(
        "challenge_full",
        "Toutes les places de ce défi sont déjà prises.",
    )
}

// --- Paying a winner -------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Admission {
    /// A right answer: the challenge must still take participations.
    Answer,
    /// A reviewer approves a pending participation, sent while the challenge was open.
    Review { reviewer: Uuid },
}

fn chain_unavailable(error: ChainError) -> ApiError {
    match error {
        ChainError::Db(error) => error.into(),
        other => {
            tracing::warn!(error = %other, "challenge reward not sent");
            ApiError::unavailable(
                "La blockchain est momentanément indisponible. Réessayez dans un instant.",
            )
        }
    }
}

/// Makes `student` a winner of `challenge` and mints their reward with the creator's key:
/// see the module documentation for why this pays exactly once.
async fn reward(
    state: &AppState,
    challenge: &Challenge,
    student: Uuid,
    admission: Admission,
) -> ApiResult<Value> {
    let unfunded = || {
        ApiError::conflict(
            "challenge_unfunded",
            "Ce défi ne peut pas verser d'EPC pour le moment : le quota de distribution de \
             son créateur est épuisé ou inactif. Réessayez plus tard.",
        )
    };
    // --- The creator must be able to pay; checked again by the chain itself.
    let creator_id = challenge.creator_user_id.ok_or_else(unfunded)?;
    let creator = sqlx::query_as::<_, User>("SELECT * FROM users WHERE id = $1")
        .bind(creator_id)
        .fetch_optional(&state.db)
        .await?
        .ok_or_else(unfunded)?;
    if authority::desired_quota(&creator, &state.settings).is_none() {
        return Err(unfunded());
    }
    let keypair = wallets::load_keypair(&state.db, &state.vault, creator_id).await?;
    let quota = mint::quota_of(state, &keypair.address()).await?;
    if !quota.active || quota.remaining < challenge.reward as Amount {
        return Err(unfunded());
    }
    let student_ok: bool = sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM users \
           WHERE id = $1 AND role = 'student' AND disabled_at IS NULL)",
    )
    .bind(student)
    .fetch_one(&state.db)
    .await?;
    if !student_ok {
        return Err(ApiError::conflict(
            "not_a_student",
            "Ce compte n'est plus un compte étudiant actif.",
        ));
    }
    let to = wallets::address_of(&state.db, student).await?;

    // --- Signing rights first, then the winner and the signed mint, atomically.
    let mut session = state
        .chain
        .begin(&keypair)
        .await
        .map_err(chain_unavailable)?;
    let mut tx = state.db.begin().await?;
    let locked =
        sqlx::query_as::<_, Challenge>("SELECT * FROM challenges WHERE id = $1 FOR UPDATE")
            .bind(challenge.id)
            .fetch_one(&mut *tx)
            .await?;
    if admission == Admission::Answer && !locked.accepting(OffsetDateTime::now_utc()) {
        return Err(ApiError::conflict(
            "challenge_closed",
            "Ce défi est terminé.",
        ));
    }
    let existing: Option<(String, Option<String>)> = sqlx::query_as(
        "SELECT s.status, t.status FROM challenge_submissions s \
         LEFT JOIN chain_txs t ON t.hash = s.tx_hash \
         WHERE s.challenge_id = $1 AND s.student_user_id = $2",
    )
    .bind(challenge.id)
    .bind(student)
    .fetch_optional(&mut *tx)
    .await?;
    match (&existing, admission) {
        (Some((status, reward)), _)
            if status == "approved" && reward.as_deref() != Some(STATUS_FAILED) =>
        {
            return Err(already_completed());
        }
        (Some((status, _)), Admission::Review { .. }) if status != "pending" => {
            return Err(already_decided());
        }
        (None, Admission::Review { .. }) => return Err(ApiError::not_found("Participation")),
        _ => {}
    }
    let winners: i64 = sqlx::query_scalar(concat!("SELECT ", winners_of!("$1")))
        .bind(challenge.id)
        .fetch_one(&mut *tx)
        .await?;
    if locked.full(winners) {
        return Err(challenge_full());
    }

    // The reward as it stands now, under the lock.
    let amount = locked.reward as Amount;
    let batch_id = Uuid::new_v4();
    let reason = mint::reason_name(ReasonCode::Challenge);
    sqlx::query(
        "INSERT INTO mint_batches (id, actor_user_id, amount, recipient_count, reason_code, comment) \
         VALUES ($1, $2, $3, 1, $4, $5)",
    )
    .bind(batch_id)
    .bind(creator_id)
    .bind(locked.reward)
    .bind(&reason)
    .bind(format!("Défi : {}", locked.title))
    .execute(&mut *tx)
    .await?;
    let mint = session.sign(TxPayload::Mint {
        to,
        amount,
        reason: ReasonCode::Challenge,
    });
    record_tx(
        &mut tx,
        &mint,
        &TxMeta {
            actor_user_id: Some(creator_id),
            subject_user_id: Some(student),
            amount: Some(amount),
            reason_code: Some(reason),
            batch_id: Some(batch_id),
            order_id: None,
        },
    )
    .await?;
    let hash = mint.hash();
    let reviewer = match admission {
        Admission::Review { reviewer } => Some(reviewer),
        Admission::Answer => None,
    };
    sqlx::query(
        "INSERT INTO challenge_submissions \
           (id, challenge_id, student_user_id, status, reviewer_user_id, tx_hash, decided_at) \
         VALUES ($1, $2, $3, 'approved', $4, $5, now()) \
         ON CONFLICT (challenge_id, student_user_id) DO UPDATE \
           SET status = 'approved', reviewer_user_id = EXCLUDED.reviewer_user_id, \
               tx_hash = EXCLUDED.tx_hash, review_note = '', decided_at = now()",
    )
    .bind(Uuid::new_v4())
    .bind(challenge.id)
    .bind(student)
    .bind(reviewer)
    .bind(hash.to_string())
    .execute(&mut *tx)
    .await?;
    audit::record(
        &mut *tx,
        Some(reviewer.unwrap_or(student)),
        audit::action::CHALLENGE_REWARDED,
        Some(("challenge", challenge.id.to_string())),
        json!({ "student": student, "amount": amount, "tx": hash, "mode": locked.mode }),
    )
    .await?;
    tx.commit().await?;

    // --- Submit. A refusal leaves the submission unpaid, so it can be paid again.
    let submitted = session.submit(&mint).await;
    drop(session);
    match submitted {
        Ok(()) => {
            if let Err(error) = state.chain.wait_final(&[hash], WAIT).await {
                tracing::warn!(%error, "challenge reward recorded, inclusion not confirmed yet");
            }
        }
        Err(ChainError::Rejected(error)) => {
            return Err(ApiError::conflict(
                "reward_refused",
                "La blockchain a refusé le versement. Réessayez plus tard.",
            )
            .with_details(json!({ "reason": crate::chain::error_code(&error) })));
        }
        Err(ChainError::Unavailable(reason)) => {
            tracing::warn!(%reason, "challenge reward recorded but not delivered yet");
        }
        Err(other) => return Err(chain_unavailable(other)),
    }
    let row = state.chain.row(&hash).await?;
    Ok(json!({ "amount": amount, "reward_status": row.status, "tx_hash": row.hash }))
}

fn already_decided() -> ApiError {
    ApiError::conflict("already_decided", "Cette participation a déjà été traitée.")
}

// --- Staff and admins ------------------------------------------------------------------

#[derive(sqlx::FromRow)]
struct ManagedRow {
    #[sqlx(flatten)]
    challenge: Challenge,
    creator_name: Option<String>,
    winners: i64,
    pending: i64,
}

#[derive(Debug, Serialize)]
struct ManagedChallenge {
    #[serde(flatten)]
    challenge: ChallengeView,
    creator_id: Option<Uuid>,
    /// Closed by hand.
    closed: bool,
    /// Participations waiting for a review.
    pending: i64,
}

impl From<ManagedRow> for ManagedChallenge {
    fn from(row: ManagedRow) -> Self {
        let creator_id = row.challenge.creator_user_id;
        let closed = row.challenge.closed_at.is_some();
        Self {
            challenge: ChallengeView::new(row.challenge, row.winners, row.creator_name),
            creator_id,
            closed,
            pending: row.pending,
        }
    }
}

macro_rules! managed_select {
    () => {
        concat!(
            "SELECT ch.*, cu.first_name || ' ' || cu.last_name AS creator_name, ",
            winners_of!("ch.id"),
            " AS winners, (SELECT count(*) FROM challenge_submissions p \
               WHERE p.challenge_id = ch.id AND p.status = 'pending') AS pending \
             FROM challenges ch LEFT JOIN users cu ON cu.id = ch.creator_user_id "
        )
    };
}

/// Staff see their own challenges; admins see every challenge.
async fn list_managed(
    State(state): State<AppState>,
    current: CurrentUser,
) -> ApiResult<Json<Vec<ManagedChallenge>>> {
    current.require(&Role::DISTRIBUTORS)?;
    let creator = (current.user.role != Role::Admin).then(|| current.id());
    let rows = sqlx::query_as::<_, ManagedRow>(concat!(
        managed_select!(),
        "WHERE $1::uuid IS NULL OR ch.creator_user_id = $1 \
         ORDER BY (ch.closed_at IS NULL) DESC, ch.created_at DESC LIMIT 200"
    ))
    .bind(creator)
    .fetch_all(&state.db)
    .await?;
    Ok(Json(rows.into_iter().map(Into::into).collect()))
}

async fn load_managed(state: &AppState, id: Uuid) -> ApiResult<ManagedChallenge> {
    let row = sqlx::query_as::<_, ManagedRow>(concat!(managed_select!(), "WHERE ch.id = $1"))
        .bind(id)
        .fetch_one(&state.db)
        .await?;
    Ok(row.into())
}

/// A challenge the current user manages: their own, or any for an admin. Someone else's
/// challenge is reported as not found.
async fn managed_challenge(
    state: &AppState,
    current: &CurrentUser,
    id: Uuid,
) -> ApiResult<Challenge> {
    current.require(&Role::DISTRIBUTORS)?;
    let challenge = load(state, id).await?;
    if current.user.role != Role::Admin && challenge.creator_user_id != Some(current.id()) {
        return Err(ApiError::not_found("Défi"));
    }
    Ok(challenge)
}

#[derive(Deserialize)]
struct NewChallenge {
    title: String,
    #[serde(default)]
    description: String,
    /// Per winner, in units.
    reward: Amount,
    mode: Mode,
    /// The expected answer (answer mode).
    #[serde(default)]
    answer: String,
    #[serde(default)]
    cohorts: Vec<String>,
    max_winners: Option<i32>,
    /// RFC 3339.
    closes_at: Option<String>,
}

async fn create(
    State(state): State<AppState>,
    current: CurrentUser,
    Json(body): Json<NewChallenge>,
) -> ApiResult<(StatusCode, Json<ManagedChallenge>)> {
    current.require(&Role::DISTRIBUTORS)?;
    let now = OffsetDateTime::now_utc();
    let title = check_title(&body.title).map_err(invalid)?;
    let description = check_description(&body.description).map_err(invalid)?;
    let reward = check_reward(body.reward).map_err(invalid)?;
    let cohorts = check_cohorts(&body.cohorts).map_err(invalid)?;
    let max_winners = check_max_winners(body.max_winners).map_err(invalid)?;
    let closes_at = check_closes_at(body.closes_at.as_deref(), now).map_err(invalid)?;
    let answer_hash = match body.mode {
        Mode::Answer => Some(hash_blocking(check_answer(&body.answer).map_err(invalid)?).await?),
        Mode::Review => None,
    };

    let id = Uuid::new_v4();
    let mut tx = state.db.begin().await?;
    sqlx::query(
        "INSERT INTO challenges (id, creator_user_id, title, description, reward, mode, \
           answer_hash, cohorts, max_winners, closes_at) \
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)",
    )
    .bind(id)
    .bind(current.id())
    .bind(&title)
    .bind(&description)
    .bind(reward)
    .bind(body.mode.as_str())
    .bind(&answer_hash)
    .bind(&cohorts)
    .bind(max_winners)
    .bind(closes_at)
    .execute(&mut *tx)
    .await?;
    audit::record(
        &mut *tx,
        Some(current.id()),
        audit::action::CHALLENGE_CREATED,
        Some(("challenge", id.to_string())),
        json!({ "mode": body.mode, "reward": reward, "cohorts": cohorts, "max_winners": max_winners }),
    )
    .await?;
    tx.commit().await?;
    Ok((StatusCode::CREATED, Json(load_managed(&state, id).await?)))
}

/// Fields that can change. An absent field is left untouched; `max_winners: null` and
/// `closes_at: null` remove the limit; an empty `answer` keeps the current one.
#[derive(Deserialize)]
struct ChallengeUpdate {
    title: Option<String>,
    description: Option<String>,
    reward: Option<Amount>,
    answer: Option<String>,
    cohorts: Option<Vec<String>>,
    #[serde(default, deserialize_with = "double_option")]
    max_winners: Option<Option<i32>>,
    #[serde(default, deserialize_with = "double_option")]
    closes_at: Option<Option<String>>,
    closed: Option<bool>,
}

async fn update(
    State(state): State<AppState>,
    current: CurrentUser,
    Path(id): Path<Uuid>,
    Json(body): Json<ChallengeUpdate>,
) -> ApiResult<Json<ManagedChallenge>> {
    let before = managed_challenge(&state, &current, id).await?;
    let now = OffsetDateTime::now_utc();
    let title = match &body.title {
        Some(text) => check_title(text).map_err(invalid)?,
        None => before.title.clone(),
    };
    let description = match &body.description {
        Some(text) => check_description(text).map_err(invalid)?,
        None => before.description.clone(),
    };
    let reward = match body.reward {
        Some(reward) => check_reward(reward).map_err(invalid)?,
        None => before.reward,
    };
    let cohorts = match &body.cohorts {
        Some(cohorts) => check_cohorts(cohorts).map_err(invalid)?,
        None => before.cohorts.clone(),
    };
    let max_winners = match body.max_winners {
        Some(max) => check_max_winners(max).map_err(invalid)?,
        None => before.max_winners,
    };
    let closes_at = match &body.closes_at {
        Some(text) => check_closes_at(text.as_deref(), now).map_err(invalid)?,
        None => before.closes_at,
    };
    let new_answer = body.answer.as_deref().filter(|a| !a.trim().is_empty());
    let answer_hash = match (before.mode, new_answer) {
        (Mode::Answer, Some(answer)) => {
            Some(hash_blocking(check_answer(answer).map_err(invalid)?).await?)
        }
        (Mode::Review, Some(_)) => {
            return Err(invalid(
                "un défi validé par un encadrant n'a pas de réponse attendue".into(),
            ))
        }
        (_, None) => before.answer_hash.clone(),
    };
    let closed = body.closed.unwrap_or(before.closed_at.is_some());

    let mut tx = state.db.begin().await?;
    sqlx::query(
        "UPDATE challenges SET title = $2, description = $3, reward = $4, answer_hash = $5, \
           cohorts = $6, max_winners = $7, closes_at = $8, \
           closed_at = CASE WHEN $9 THEN COALESCE(closed_at, now()) ELSE NULL END \
         WHERE id = $1",
    )
    .bind(id)
    .bind(&title)
    .bind(&description)
    .bind(reward)
    .bind(&answer_hash)
    .bind(&cohorts)
    .bind(max_winners)
    .bind(closes_at)
    .bind(closed)
    .execute(&mut *tx)
    .await?;
    audit::record(
        &mut *tx,
        Some(current.id()),
        audit::action::CHALLENGE_UPDATED,
        Some(("challenge", id.to_string())),
        json!({
            "reward": { "from": before.reward, "to": reward },
            "closed": { "from": before.closed_at.is_some(), "to": closed },
            "answer_changed": new_answer.is_some(),
        }),
    )
    .await?;
    tx.commit().await?;
    Ok(Json(load_managed(&state, id).await?))
}

#[derive(Debug, Serialize, sqlx::FromRow)]
struct SubmissionView {
    id: Uuid,
    student_id: Uuid,
    student_name: String,
    student_email: String,
    student_promo: String,
    /// `pending`, `approved` or `rejected`.
    status: String,
    proof: String,
    review_note: String,
    /// The reward transaction once approved: `pending`, `included` or `failed`.
    reward_status: Option<String>,
    #[serde(with = "time::serde::rfc3339")]
    submitted_at: OffsetDateTime,
    #[serde(with = "time::serde::rfc3339::option")]
    decided_at: Option<OffsetDateTime>,
}

macro_rules! submission_select {
    () => {
        "SELECT s.id, s.student_user_id AS student_id, \
       u.first_name || ' ' || u.last_name AS student_name, u.email AS student_email, \
       u.promo AS student_promo, s.status, s.proof, s.review_note, t.status AS reward_status, \
       s.submitted_at, s.decided_at \
     FROM challenge_submissions s \
     JOIN users u ON u.id = s.student_user_id \
     LEFT JOIN chain_txs t ON t.hash = s.tx_hash "
    };
}

/// Participations, waiting ones first: winners and those to review.
async fn submissions(
    State(state): State<AppState>,
    current: CurrentUser,
    Path(id): Path<Uuid>,
) -> ApiResult<Json<Vec<SubmissionView>>> {
    managed_challenge(&state, &current, id).await?;
    let items = sqlx::query_as::<_, SubmissionView>(concat!(
        submission_select!(),
        "WHERE s.challenge_id = $1 \
         ORDER BY (s.status = 'pending') DESC, s.submitted_at DESC LIMIT 500"
    ))
    .bind(id)
    .fetch_all(&state.db)
    .await?;
    Ok(Json(items))
}

#[derive(Deserialize)]
struct Decision {
    approve: bool,
    /// Why it is rejected, shown to the student.
    #[serde(default)]
    note: String,
}

/// Approves (and pays) or rejects a participation waiting for review.
async fn decide(
    State(state): State<AppState>,
    current: CurrentUser,
    Path((id, submission_id)): Path<(Uuid, Uuid)>,
    Json(body): Json<Decision>,
) -> ApiResult<Json<SubmissionView>> {
    let challenge = managed_challenge(&state, &current, id).await?;
    if challenge.mode != Mode::Review {
        return Err(ApiError::unprocessable(
            "wrong_mode",
            "Les participations à ce défi sont validées par leur réponse.",
        ));
    }
    let (student, status): (Uuid, String) = sqlx::query_as(
        "SELECT student_user_id, status FROM challenge_submissions \
         WHERE id = $1 AND challenge_id = $2",
    )
    .bind(submission_id)
    .bind(id)
    .fetch_optional(&state.db)
    .await?
    .ok_or_else(|| ApiError::not_found("Participation"))?;
    if status != "pending" {
        return Err(already_decided());
    }

    if body.approve {
        reward(
            &state,
            &challenge,
            student,
            Admission::Review {
                reviewer: current.id(),
            },
        )
        .await?;
    } else {
        let note = body.note.trim();
        if note.chars().count() > MAX_NOTE_CHARS {
            return Err(ApiError::unprocessable(
                "invalid_note",
                format!("Le motif du refus est limité à {MAX_NOTE_CHARS} caractères."),
            ));
        }
        let mut tx = state.db.begin().await?;
        let rejected = sqlx::query(
            "UPDATE challenge_submissions SET status = 'rejected', review_note = $2, \
               reviewer_user_id = $3, decided_at = now() \
             WHERE id = $1 AND status = 'pending'",
        )
        .bind(submission_id)
        .bind(note)
        .bind(current.id())
        .execute(&mut *tx)
        .await?;
        if rejected.rows_affected() == 0 {
            return Err(already_decided());
        }
        audit::record(
            &mut *tx,
            Some(current.id()),
            audit::action::CHALLENGE_SUBMISSION_REJECTED,
            Some(("challenge", id.to_string())),
            json!({ "student": student }),
        )
        .await?;
        tx.commit().await?;
    }
    let view =
        sqlx::query_as::<_, SubmissionView>(concat!(submission_select!(), "WHERE s.id = $1"))
            .bind(submission_id)
            .fetch_one(&state.db)
            .await?;
    Ok(Json(view))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn answers_ignore_case_and_spaces() {
        assert_eq!(
            normalize_answer("  Le   Petit\tPrince \n"),
            "le petit prince"
        );
        assert_eq!(normalize_answer("FLAG{AbC}"), "flag{abc}");
        assert_eq!(normalize_answer("   "), "");
        assert!(check_answer(" ").is_err());
        assert!(check_answer(&"x".repeat(201)).is_err());
        assert_eq!(check_answer(" 42 ").unwrap(), "42");
    }

    #[test]
    fn cohorts_are_codes_in_display_order_without_duplicates() {
        let typed = ["bach1".to_owned(), "PGE 2".to_owned(), "pge2".to_owned()];
        assert_eq!(check_cohorts(&typed).unwrap(), ["pge2", "bach1"]);
        assert!(check_cohorts(&["pge9".to_owned()]).is_err());
        assert!(check_cohorts(&[]).unwrap().is_empty());
    }

    #[test]
    fn fields_are_checked() {
        assert!(check_title("  ").is_err());
        assert!(check_title(&"x".repeat(101)).is_err());
        assert_eq!(check_title(" Énigme ").unwrap(), "Énigme");
        assert!(check_reward(0).is_err());
        assert!(check_reward(u64::MAX).is_err());
        assert_eq!(check_reward(500).unwrap(), 500);
        assert!(check_max_winners(Some(0)).is_err());
        assert_eq!(check_max_winners(None).unwrap(), None);
    }

    #[test]
    fn a_deadline_must_be_in_the_future() {
        let now = OffsetDateTime::now_utc();
        assert_eq!(check_closes_at(None, now).unwrap(), None);
        assert_eq!(check_closes_at(Some(" "), now).unwrap(), None);
        assert!(check_closes_at(Some("2020-01-01T00:00:00Z"), now).is_err());
        assert!(check_closes_at(Some("demain"), now).is_err());
        assert!(check_closes_at(Some("2999-01-01T12:00:00+01:00"), now)
            .unwrap()
            .is_some());
    }

    #[test]
    fn modes_round_trip() {
        for mode in [Mode::Answer, Mode::Review] {
            assert_eq!(Mode::try_from(mode.as_str().to_owned()), Ok(mode));
        }
        assert!(Mode::try_from("quiz".to_owned()).is_err());
    }
}
