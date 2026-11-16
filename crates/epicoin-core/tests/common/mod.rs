//! Shared fixtures for the integration tests.
#![allow(dead_code)]

use epicoin_core::{
    Address, Amount, AuthorityAction, Genesis, Hash, Keypair, ReasonCode, State, Transaction,
    TxPayload,
};

pub const CHAIN_ID: &str = "epicoin-test";
pub const GENESIS_TIME: u64 = 1_700_000_000_000;
pub const PERIOD_MS: u64 = 1_000_000;
/// A timestamp inside the first quota period.
pub const T0: u64 = GENESIS_TIME + 1;

/// A deterministic cast of characters and the genesis that binds them.
pub struct World {
    pub validator: Keypair,
    pub admin: Keypair,
    pub treasury: Keypair,
    pub staff: Vec<Keypair>,
    pub students: Vec<Keypair>,
    pub genesis: Genesis,
}

fn key(seed: u8) -> Keypair {
    Keypair::from_secret_bytes(&[seed; 32])
}

impl World {
    pub fn new() -> Self {
        let validator = key(1);
        let admin = key(2);
        let treasury = key(3);
        let genesis = Genesis {
            chain_id: CHAIN_ID.into(),
            timestamp: GENESIS_TIME,
            validators: vec![validator.public_key()],
            admin: admin.address(),
            treasury: treasury.address(),
            quota_period_ms: PERIOD_MS,
        };
        Self {
            validator,
            admin,
            treasury,
            staff: (10..12).map(key).collect(),
            students: (20..23).map(key).collect(),
            genesis,
        }
    }

    pub fn state(&self) -> State {
        State::new(self.genesis.clone()).expect("valid genesis")
    }

    /// Every key of the world, for tests that pick signers at random.
    pub fn actors(&self) -> Vec<&Keypair> {
        [&self.admin, &self.treasury]
            .into_iter()
            .chain(&self.staff)
            .chain(&self.students)
            .collect()
    }
}

/// Signs `payload` with the signer's current nonce in `state`.
pub fn tx(state: &State, signer: &Keypair, payload: TxPayload) -> Transaction {
    Transaction::sign(CHAIN_ID, signer, state.nonce(&signer.address()), payload)
}

pub fn grant(authority: Address, quota: Amount) -> TxPayload {
    TxPayload::AuthorityUpdate {
        authority,
        action: AuthorityAction::Grant { quota },
    }
}

pub fn revoke(authority: Address) -> TxPayload {
    TxPayload::AuthorityUpdate {
        authority,
        action: AuthorityAction::Revoke,
    }
}

pub fn mint(to: Address, amount: Amount) -> TxPayload {
    TxPayload::Mint {
        to,
        amount,
        reason: ReasonCode::Participation,
    }
}

pub fn purchase(amount: Amount, order: u8) -> TxPayload {
    TxPayload::Purchase {
        amount,
        order_ref: Hash([order; 32]),
    }
}

/// Sum of all balances, widened so the check itself cannot overflow.
pub fn total_balances(state: &State) -> u128 {
    state
        .accounts()
        .values()
        .map(|account| u128::from(account.balance))
        .sum()
}
