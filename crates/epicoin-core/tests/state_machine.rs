//! Validation rules of the state machine, one behaviour per test.

mod common;

use common::*;
use epicoin_core::{Hash, Keypair, State, Transaction, TxError, TxPayload};

/// A world where staff[0] holds a 1000-unit quota and students[0] owns 500 units.
fn funded() -> (World, State) {
    let w = World::new();
    let mut s = w.state();
    let t = tx(&s, &w.admin, grant(w.staff[0].address(), 1_000));
    s.apply_tx(&t, T0).unwrap();
    let t = tx(&s, &w.staff[0], mint(w.students[0].address(), 500));
    s.apply_tx(&t, T0).unwrap();
    (w, s)
}

/// Applies a transaction that must fail and checks the state is left untouched.
fn rejected(state: &mut State, tx: &Transaction, at: u64) -> TxError {
    let before = state.clone();
    let err = state
        .apply_tx(tx, at)
        .expect_err("transaction should be rejected");
    assert_eq!(
        *state, before,
        "a rejected transaction must not change the state"
    );
    assert_eq!(state.root(), before.root());
    err
}

fn buy(state: &mut State, buyer: &Keypair, amount: u64) -> Hash {
    let t = tx(state, buyer, purchase(amount, 1));
    state.apply_tx(&t, T0).unwrap();
    t.hash()
}

// --- Signatures and nonces -------------------------------------------------------------

#[test]
fn invalid_signature_is_rejected() {
    let (w, mut s) = funded();
    let mut t = tx(&s, &w.students[0], purchase(100, 1));
    t.signature.0[5] ^= 0xff;
    assert_eq!(rejected(&mut s, &t, T0), TxError::BadSignature);
}

#[test]
fn tampered_payload_is_rejected() {
    let (w, mut s) = funded();
    let mut t = tx(&s, &w.staff[0], mint(w.students[1].address(), 10));
    t.payload = mint(w.students[1].address(), 400);
    assert_eq!(rejected(&mut s, &t, T0), TxError::BadSignature);
}

#[test]
fn signature_by_someone_else_is_rejected() {
    let (w, mut s) = funded();
    // students[1] signs, but claims to be students[0].
    let mut t = tx(&s, &w.students[1], purchase(100, 1));
    t.from = w.students[0].address();
    assert_eq!(rejected(&mut s, &t, T0), TxError::AddressMismatch);
}

#[test]
fn transaction_from_another_chain_is_rejected() {
    let (w, mut s) = funded();
    let t = Transaction::sign("epicoin-other", &w.students[0], 0, purchase(100, 1));
    assert_eq!(rejected(&mut s, &t, T0), TxError::BadSignature);
}

#[test]
fn wrong_nonce_is_rejected() {
    let (w, mut s) = funded();
    let future = Transaction::sign(CHAIN_ID, &w.students[0], 1, purchase(100, 1));
    assert_eq!(
        rejected(&mut s, &future, T0),
        TxError::BadNonce {
            expected: 0,
            got: 1
        }
    );
}

#[test]
fn replaying_a_transaction_is_rejected() {
    let (w, mut s) = funded();
    let t = tx(&s, &w.students[0], purchase(100, 1));
    s.apply_tx(&t, T0).unwrap();
    assert_eq!(
        rejected(&mut s, &t, T0),
        TxError::BadNonce {
            expected: 1,
            got: 0
        }
    );
    assert_eq!(s.balance(&w.students[0].address()), 400);
}

#[test]
fn nonce_increments_once_per_accepted_transaction() {
    let (w, mut s) = funded();
    assert_eq!(s.nonce(&w.admin.address()), 1);
    assert_eq!(s.nonce(&w.staff[0].address()), 1);
    assert_eq!(s.nonce(&w.students[0].address()), 0);
    buy(&mut s, &w.students[0], 10);
    assert_eq!(s.nonce(&w.students[0].address()), 1);
}

// --- Mint ------------------------------------------------------------------------------

#[test]
fn mint_credits_the_student_and_the_supply() {
    let (w, s) = funded();
    assert_eq!(s.balance(&w.students[0].address()), 500);
    assert_eq!(s.total_minted(), 500);
    assert_eq!(s.circulating_supply(), 500);
    assert_eq!(total_balances(&s), 500);
}

#[test]
fn unauthorized_mint_is_rejected() {
    let (w, mut s) = funded();
    // A student, a staff member who was never granted, the treasury and even the admin.
    for signer in [&w.students[0], &w.staff[1], &w.treasury, &w.admin] {
        let t = tx(&s, signer, mint(w.students[1].address(), 10));
        assert_eq!(rejected(&mut s, &t, T0), TxError::NotAuthority);
    }
}

#[test]
fn revoked_authority_cannot_mint() {
    let (w, mut s) = funded();
    let t = tx(&s, &w.admin, revoke(w.staff[0].address()));
    s.apply_tx(&t, T0).unwrap();
    let t = tx(&s, &w.staff[0], mint(w.students[0].address(), 10));
    assert_eq!(rejected(&mut s, &t, T0), TxError::NotAuthority);
}

#[test]
fn mint_over_quota_is_rejected() {
    let (w, mut s) = funded();
    // 500 of 1000 already used.
    let t = tx(&s, &w.staff[0], mint(w.students[1].address(), 501));
    assert_eq!(
        rejected(&mut s, &t, T0),
        TxError::QuotaExceeded {
            quota: 1_000,
            used: 500,
            requested: 501
        }
    );
    // Exactly reaching the quota is fine; one more unit is not.
    let t = tx(&s, &w.staff[0], mint(w.students[1].address(), 500));
    s.apply_tx(&t, T0).unwrap();
    let t = tx(&s, &w.staff[0], mint(w.students[1].address(), 1));
    assert!(matches!(
        rejected(&mut s, &t, T0),
        TxError::QuotaExceeded { .. }
    ));
}

#[test]
fn quota_resets_at_the_next_period() {
    let (w, mut s) = funded();
    let staff = w.staff[0].address();
    assert_eq!(s.mint_quota_remaining(&staff, T0), Some(500));
    assert_eq!(
        s.mint_quota_remaining(&staff, GENESIS_TIME + PERIOD_MS),
        Some(1_000)
    );

    let last_ms_of_period = GENESIS_TIME + PERIOD_MS - 1;
    let t = tx(&s, &w.staff[0], mint(w.students[1].address(), 1_000));
    assert!(matches!(
        rejected(&mut s, &t, last_ms_of_period),
        TxError::QuotaExceeded { .. }
    ));
    s.apply_tx(&t, GENESIS_TIME + PERIOD_MS).unwrap();
    assert_eq!(
        s.mint_quota_remaining(&staff, GENESIS_TIME + PERIOD_MS),
        Some(0)
    );
    assert_eq!(s.mint_quota_remaining(&w.students[0].address(), T0), None);
}

#[test]
fn zero_amount_mint_is_rejected() {
    let (w, mut s) = funded();
    let t = tx(&s, &w.staff[0], mint(w.students[0].address(), 0));
    assert_eq!(rejected(&mut s, &t, T0), TxError::ZeroAmount);
}

#[test]
fn authority_cannot_mint_to_itself() {
    let (w, mut s) = funded();
    let t = tx(&s, &w.staff[0], mint(w.staff[0].address(), 10));
    assert_eq!(rejected(&mut s, &t, T0), TxError::SelfMint);
}

#[test]
fn mint_overflowing_a_balance_is_rejected() {
    let w = World::new();
    let mut s = w.state();
    let t = tx(&s, &w.admin, grant(w.staff[0].address(), u64::MAX));
    s.apply_tx(&t, T0).unwrap();
    let t = tx(&s, &w.admin, grant(w.staff[1].address(), u64::MAX));
    s.apply_tx(&t, T0).unwrap();
    let t = tx(&s, &w.staff[0], mint(w.students[0].address(), u64::MAX));
    s.apply_tx(&t, T0).unwrap();
    let t = tx(&s, &w.staff[1], mint(w.students[0].address(), 1));
    assert_eq!(rejected(&mut s, &t, T0), TxError::Overflow);
    // Same for the total supply, on a different recipient.
    let t = tx(&s, &w.staff[1], mint(w.students[1].address(), 1));
    assert_eq!(rejected(&mut s, &t, T0), TxError::Overflow);
}

// --- Authority updates -----------------------------------------------------------------

#[test]
fn only_the_admin_can_update_authorities() {
    let (w, mut s) = funded();
    for signer in [&w.staff[0], &w.students[0], &w.treasury] {
        let t = tx(&s, signer, grant(w.staff[1].address(), 1_000_000));
        assert_eq!(rejected(&mut s, &t, T0), TxError::NotAdmin);
        let t = tx(&s, signer, revoke(w.staff[0].address()));
        assert_eq!(rejected(&mut s, &t, T0), TxError::NotAdmin);
    }
}

#[test]
fn revoking_an_unknown_or_revoked_authority_is_rejected() {
    let (w, mut s) = funded();
    let t = tx(&s, &w.admin, revoke(w.staff[1].address()));
    assert_eq!(rejected(&mut s, &t, T0), TxError::AuthorityNotFound);

    let t = tx(&s, &w.admin, revoke(w.staff[0].address()));
    s.apply_tx(&t, T0).unwrap();
    let t = tx(&s, &w.admin, revoke(w.staff[0].address()));
    assert_eq!(rejected(&mut s, &t, T0), TxError::AuthorityNotFound);
}

#[test]
fn granting_again_updates_the_quota_but_keeps_the_usage() {
    let (w, mut s) = funded();
    let staff = w.staff[0].address();

    // Revoke then re-grant within the period: the 500 already minted still count.
    let t = tx(&s, &w.admin, revoke(staff));
    s.apply_tx(&t, T0).unwrap();
    let t = tx(&s, &w.admin, grant(staff, 600));
    s.apply_tx(&t, T0).unwrap();
    assert_eq!(s.mint_quota_remaining(&staff, T0), Some(100));

    // Lowering the quota below the usage leaves nothing to mint, without underflow.
    let t = tx(&s, &w.admin, grant(staff, 200));
    s.apply_tx(&t, T0).unwrap();
    assert_eq!(s.mint_quota_remaining(&staff, T0), Some(0));
    let t = tx(&s, &w.staff[0], mint(w.students[0].address(), 1));
    assert!(matches!(
        rejected(&mut s, &t, T0),
        TxError::QuotaExceeded { .. }
    ));
}

// --- Purchases and refunds -------------------------------------------------------------

#[test]
fn purchase_moves_funds_to_the_treasury() {
    let (w, mut s) = funded();
    let id = buy(&mut s, &w.students[0], 200);
    assert_eq!(s.balance(&w.students[0].address()), 300);
    assert_eq!(s.balance(&w.treasury.address()), 200);
    let record = s.purchases()[&id];
    assert_eq!(record.buyer, w.students[0].address());
    assert_eq!(record.amount, 200);
    assert!(!record.refunded);
    assert_eq!(total_balances(&s), 500);
}

#[test]
fn purchase_with_insufficient_balance_is_rejected() {
    let (w, mut s) = funded();
    let t = tx(&s, &w.students[0], purchase(501, 1));
    assert_eq!(
        rejected(&mut s, &t, T0),
        TxError::InsufficientBalance {
            balance: 500,
            needed: 501
        }
    );
    let t = tx(&s, &w.students[1], purchase(1, 1));
    assert!(matches!(
        rejected(&mut s, &t, T0),
        TxError::InsufficientBalance { balance: 0, .. }
    ));
}

#[test]
fn zero_amount_purchase_is_rejected() {
    let (w, mut s) = funded();
    let t = tx(&s, &w.students[0], purchase(0, 1));
    assert_eq!(rejected(&mut s, &t, T0), TxError::ZeroAmount);
}

#[test]
fn treasury_cannot_purchase() {
    let (w, mut s) = funded();
    buy(&mut s, &w.students[0], 200);
    let t = tx(&s, &w.treasury, purchase(50, 1));
    assert_eq!(rejected(&mut s, &t, T0), TxError::TreasuryCannotPurchase);
}

#[test]
fn refund_gives_the_full_amount_back_once() {
    let (w, mut s) = funded();
    let id = buy(&mut s, &w.students[0], 200);

    let t = tx(&s, &w.treasury, TxPayload::Refund { purchase: id });
    s.apply_tx(&t, T0).unwrap();
    assert_eq!(s.balance(&w.students[0].address()), 500);
    assert_eq!(s.balance(&w.treasury.address()), 0);
    assert!(s.purchases()[&id].refunded);

    let t = tx(&s, &w.treasury, TxPayload::Refund { purchase: id });
    assert_eq!(rejected(&mut s, &t, T0), TxError::AlreadyRefunded);
}

#[test]
fn only_the_treasury_can_refund() {
    let (w, mut s) = funded();
    let id = buy(&mut s, &w.students[0], 200);
    for signer in [&w.students[0], &w.staff[0], &w.admin] {
        let t = tx(&s, signer, TxPayload::Refund { purchase: id });
        assert_eq!(rejected(&mut s, &t, T0), TxError::NotTreasury);
    }
}

#[test]
fn refund_of_an_unknown_purchase_is_rejected() {
    let (w, mut s) = funded();
    buy(&mut s, &w.students[0], 200);
    let t = tx(
        &s,
        &w.treasury,
        TxPayload::Refund {
            purchase: Hash([0xee; 32]),
        },
    );
    assert_eq!(rejected(&mut s, &t, T0), TxError::UnknownPurchase);
}

// --- Transfers -------------------------------------------------------------------------

#[test]
fn student_to_student_transfer_is_rejected() {
    let (w, mut s) = funded();
    // Perfectly well-formed, signed, funded — and still refused in V1.
    let t = tx(
        &s,
        &w.students[0],
        TxPayload::Transfer {
            to: w.students[1].address(),
            amount: 100,
        },
    );
    assert_eq!(rejected(&mut s, &t, T0), TxError::TransfersDisabled);
    assert_eq!(s.balance(&w.students[0].address()), 500);
    assert_eq!(s.balance(&w.students[1].address()), 0);
}

// --- Determinism -----------------------------------------------------------------------

#[test]
fn same_transactions_give_the_same_state_root() {
    let (_, a) = funded();
    let (_, b) = funded();
    assert_eq!(a.root(), b.root());
    assert_ne!(a.root(), World::new().state().root());
}

#[test]
fn state_round_trips_through_its_canonical_encoding() {
    let (w, mut s) = funded();
    buy(&mut s, &w.students[0], 120);
    let bytes = borsh::to_vec(&s).unwrap();
    let restored: State = borsh::from_slice(&bytes).unwrap();
    assert_eq!(restored, s);
    assert_eq!(restored.root(), s.root());
}

#[test]
fn errors_serialize_with_a_machine_readable_code() {
    let err = TxError::BadNonce {
        expected: 1,
        got: 0,
    };
    let json = serde_json::to_value(&err).unwrap();
    assert_eq!(json["code"], "bad_nonce");
    assert_eq!(json["expected"], 1);
    assert_eq!(serde_json::from_value::<TxError>(json).unwrap(), err);
    assert_eq!(
        serde_json::to_string(&TxError::TransfersDisabled).unwrap(),
        r#"{"code":"transfers_disabled"}"#
    );
}
