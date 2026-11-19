//! Property tests: whatever sequence of transactions is thrown at the state machine — valid
//! or not — the supply invariant holds, rejections change nothing, and replays are
//! deterministic.

mod common;

use common::*;
use epicoin_core::{
    AuthorityAction, Chain, Hash, Keypair, ReasonCode, State, Transaction, TxError, TxPayload,
};
use proptest::prelude::*;

/// What a generated step tries to do. Indices are reduced modulo the relevant list, so every
/// generated value is meaningful.
#[derive(Debug, Clone)]
enum Action {
    Grant {
        staff: usize,
        quota: u64,
    },
    Revoke {
        staff: usize,
    },
    Mint {
        to: usize,
        amount: u64,
        reason: usize,
    },
    Purchase {
        amount: u64,
        order: u8,
    },
    Refund {
        purchase: usize,
    },
    Transfer {
        to: usize,
        amount: u64,
    },
}

#[derive(Debug, Clone)]
struct Step {
    action: Action,
    /// `None`: the signer the rules expect. `Some(i)`: actor `i`, whoever that is.
    signer: Option<usize>,
    /// Added to the correct nonce; non-zero makes the transaction invalid.
    nonce_skew: u64,
    /// Milliseconds elapsed before this step (may cross quota periods).
    wait: u64,
}

fn amount() -> impl Strategy<Value = u64> {
    prop_oneof![
        8 => 1u64..=800,
        1 => Just(0u64),
        1 => Just(u64::MAX),
        1 => (u64::MAX - 2_000)..=u64::MAX,
    ]
}

fn action() -> impl Strategy<Value = Action> {
    prop_oneof![
        3 => (any::<usize>(), prop_oneof![4 => 0u64..=3_000, 1 => Just(u64::MAX)])
            .prop_map(|(staff, quota)| Action::Grant { staff, quota }),
        1 => any::<usize>().prop_map(|staff| Action::Revoke { staff }),
        6 => (any::<usize>(), amount(), any::<usize>())
            .prop_map(|(to, amount, reason)| Action::Mint { to, amount, reason }),
        5 => (amount(), any::<u8>()).prop_map(|(amount, order)| Action::Purchase { amount, order }),
        3 => any::<usize>().prop_map(|purchase| Action::Refund { purchase }),
        4 => (any::<usize>(), amount()).prop_map(|(to, amount)| Action::Transfer { to, amount }),
    ]
}

fn step() -> impl Strategy<Value = Step> {
    (
        action(),
        prop_oneof![4 => Just(None), 1 => any::<usize>().prop_map(Some)],
        prop_oneof![9 => Just(0u64), 1 => 1u64..3],
        prop_oneof![3 => 0u64..1_000, 1 => 0u64..(2 * PERIOD_MS)],
    )
        .prop_map(|(action, signer, nonce_skew, wait)| Step {
            action,
            signer,
            nonce_skew,
            wait,
        })
}

/// An active authority when there is one, so that mints regularly get past the first check.
fn minter<'a>(w: &'a World, state: &State, picker: usize) -> &'a Keypair {
    let active: Vec<&Keypair> = w
        .staff
        .iter()
        .filter(|k| {
            state
                .authorities()
                .get(&k.address())
                .is_some_and(|a| a.active)
        })
        .collect();
    if active.is_empty() {
        &w.staff[picker % w.staff.len()]
    } else {
        active[picker % active.len()]
    }
}

/// The student most likely to afford a purchase.
fn richest_student<'a>(w: &'a World, state: &State) -> &'a Keypair {
    w.students
        .iter()
        .max_by_key(|k| state.balance(&k.address()))
        .expect("the world has students")
}

/// Turns a step into a signed transaction against the current state. The expected signer is
/// chosen so that a good share of the generated transactions is accepted; `step.signer` and
/// `step.nonce_skew` then deliberately break some of them.
fn build(w: &World, state: &State, purchases: &[Hash], picker: usize, step: &Step) -> Transaction {
    let actors = w.actors();
    let (expected_signer, payload) = match &step.action {
        Action::Grant { staff, quota } => (
            &w.admin,
            TxPayload::AuthorityUpdate {
                authority: w.staff[staff % w.staff.len()].address(),
                action: AuthorityAction::Grant { quota: *quota },
            },
        ),
        Action::Revoke { staff } => (
            &w.admin,
            TxPayload::AuthorityUpdate {
                authority: w.staff[staff % w.staff.len()].address(),
                action: AuthorityAction::Revoke,
            },
        ),
        Action::Mint { to, amount, reason } => (
            minter(w, state, picker),
            TxPayload::Mint {
                // Mostly students, sometimes anyone (including the minter itself).
                to: if to % 4 == 0 {
                    actors[(to / 4) % actors.len()].address()
                } else {
                    w.students[(to / 4) % w.students.len()].address()
                },
                amount: *amount,
                reason: ReasonCode::ALL[reason % ReasonCode::ALL.len()],
            },
        ),
        Action::Purchase { amount, order } => (
            richest_student(w, state),
            TxPayload::Purchase {
                amount: *amount,
                order_ref: Hash([*order; 32]),
            },
        ),
        Action::Refund { purchase } => (
            &w.treasury,
            TxPayload::Refund {
                purchase: if purchases.is_empty() {
                    Hash([0xee; 32])
                } else {
                    purchases[purchase % purchases.len()]
                },
            },
        ),
        Action::Transfer { to, amount } => (
            &w.students[picker % w.students.len()],
            TxPayload::Transfer {
                to: actors[to % actors.len()].address(),
                amount: *amount,
            },
        ),
    };
    let signer = match step.signer {
        None => expected_signer,
        Some(i) => actors[i % actors.len()],
    };
    let nonce = state.nonce(&signer.address()) + step.nonce_skew;
    Transaction::sign(CHAIN_ID, signer, nonce, payload)
}

/// Runs the steps on a fresh state. Returns the final state, each transaction with its
/// timestamp, and each outcome.
#[allow(clippy::type_complexity)]
fn run(
    w: &World,
    steps: &[Step],
) -> Result<(State, Vec<(Transaction, u64)>, Vec<Result<(), TxError>>), TestCaseError> {
    let mut state = w.state();
    let mut now = T0;
    let mut purchases = Vec::new();
    let mut txs = Vec::new();
    let mut outcomes = Vec::new();

    for (i, step) in steps.iter().enumerate() {
        now += step.wait;
        let tx = build(w, &state, &purchases, i, step);
        let before = state.clone();
        let outcome = state.apply_tx(&tx, now);

        match &outcome {
            Ok(()) => {
                prop_assert_eq!(state.nonce(&tx.from), before.nonce(&tx.from) + 1);
                if matches!(tx.payload, TxPayload::Purchase { .. }) {
                    purchases.push(tx.hash());
                }
            }
            Err(_) => {
                prop_assert_eq!(&state, &before, "a rejected transaction changed the state");
                prop_assert_eq!(state.root(), before.root());
            }
        }

        // The supply invariant, after every single step.
        prop_assert_eq!(
            total_balances(&state),
            u128::from(state.total_minted()) - u128::from(state.total_burned())
        );
        prop_assert!(state.total_minted() >= before.total_minted());

        txs.push((tx, now));
        outcomes.push(outcome);
    }
    Ok((state, txs, outcomes))
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(256))]

    /// sum(balances) == total_minted - total_burned, always; rejections are no-ops.
    #[test]
    fn supply_invariant_holds(steps in prop::collection::vec(step(), 1..60)) {
        let w = World::new();
        let (state, _, outcomes) = run(&w, &steps)?;
        prop_assert_eq!(outcomes.len(), steps.len());
        prop_assert_eq!(
            u128::from(state.circulating_supply()),
            total_balances(&state)
        );
    }

    /// Replaying the same transactions gives the same outcomes and the same state root.
    #[test]
    fn replay_is_deterministic(steps in prop::collection::vec(step(), 1..60)) {
        let w = World::new();
        let (state, txs, outcomes) = run(&w, &steps)?;

        let mut replayed = w.state();
        for ((tx, at), expected) in txs.iter().zip(&outcomes) {
            prop_assert_eq!(&replayed.apply_tx(tx, *at), expected);
        }
        prop_assert_eq!(replayed.root(), state.root());
        prop_assert_eq!(&replayed, &state);

        // And the state survives a canonical encode/decode with the same root.
        let decoded: State = borsh::from_slice(&borsh::to_vec(&state).unwrap()).unwrap();
        prop_assert_eq!(decoded.root(), state.root());
    }

    /// The accepted transactions, packed into blocks and replayed by an independent chain
    /// (a replica), lead to the same state root.
    #[test]
    fn blocks_replay_to_the_same_state_root(
        steps in prop::collection::vec(step(), 1..40),
        block_size in 1usize..6,
    ) {
        let w = World::new();
        let (_, txs, outcomes) = run(&w, &steps)?;
        let accepted: Vec<&(Transaction, u64)> = txs
            .iter()
            .zip(&outcomes)
            .filter(|(_, outcome)| outcome.is_ok())
            .map(|(tx, _)| tx)
            .collect();

        let mut producer = Chain::new(w.genesis.clone()).unwrap();
        let mut blocks = Vec::new();
        for chunk in accepted.chunks(block_size) {
            // Stamp the block with its first transaction's time; later ones in the chunk may
            // then be refused (quota period), so keep only what the block rules accept.
            let timestamp = chunk[0].1.max(producer.tip().timestamp + 1);
            let mut scratch = producer.state().clone();
            let valid: Vec<Transaction> = chunk
                .iter()
                .filter(|(tx, _)| scratch.apply_tx(tx, timestamp).is_ok())
                .map(|(tx, _)| tx.clone())
                .collect();
            let block = producer.build_block(valid, timestamp, &w.validator).unwrap();
            producer.apply_block(&block).unwrap();
            blocks.push(block);
        }

        let mut replica = Chain::new(w.genesis.clone()).unwrap();
        for block in &blocks {
            prop_assert_eq!(replica.apply_block(block), Ok(()));
        }
        prop_assert_eq!(replica.state().root(), producer.state().root());
        prop_assert_eq!(replica.tip(), producer.tip());
        prop_assert_eq!(
            total_balances(replica.state()),
            u128::from(replica.state().circulating_supply())
        );
    }
}
