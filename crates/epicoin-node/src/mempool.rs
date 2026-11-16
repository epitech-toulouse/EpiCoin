//! Pending transactions, in arrival order.
//!
//! The mempool keeps a *pending state*: the chain state with every queued transaction
//! applied. A new transaction is validated against it, so a sender can queue several
//! transactions with consecutive nonces, and an invalid one is refused immediately with its
//! precise reason instead of silently disappearing later.

use std::collections::{HashSet, VecDeque};

use epicoin_core::{Address, Hash, State, Transaction, TxError};

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum MempoolError {
    #[error("mempool is full")]
    Full,
    #[error("invalid transaction: {0}")]
    Invalid(#[from] TxError),
}

pub struct Mempool {
    queue: VecDeque<(Hash, Transaction)>,
    hashes: HashSet<Hash>,
    pending: State,
    capacity: usize,
}

impl Mempool {
    pub fn new(state: State, capacity: usize) -> Self {
        Self {
            queue: VecDeque::new(),
            hashes: HashSet::new(),
            pending: state,
            capacity,
        }
    }

    pub fn len(&self) -> usize {
        self.queue.len()
    }

    pub fn is_empty(&self) -> bool {
        self.queue.is_empty()
    }

    pub fn contains(&self, hash: &Hash) -> bool {
        self.hashes.contains(hash)
    }

    pub fn get(&self, hash: &Hash) -> Option<&Transaction> {
        if !self.hashes.contains(hash) {
            return None;
        }
        self.queue.iter().find(|(h, _)| h == hash).map(|(_, tx)| tx)
    }

    /// The nonce the next transaction from `address` must carry, counting queued ones.
    pub fn pending_nonce(&self, address: &Address) -> u64 {
        self.pending.nonce(address)
    }

    /// The state as it will be once every queued transaction is included.
    pub fn pending_state(&self) -> &State {
        &self.pending
    }

    /// Validates `tx` on top of the queued transactions and queues it. Re-adding a queued
    /// transaction is a no-op.
    pub fn add(&mut self, tx: Transaction, now: u64) -> Result<Hash, MempoolError> {
        let hash = tx.hash();
        if self.hashes.contains(&hash) {
            return Ok(hash);
        }
        if self.queue.len() >= self.capacity {
            return Err(MempoolError::Full);
        }
        self.pending.apply_tx(&tx, now)?;
        self.hashes.insert(hash);
        self.queue.push_back((hash, tx));
        Ok(hash)
    }

    /// Picks up to `max` transactions for a block stamped `timestamp` on top of `base`.
    ///
    /// Transactions are re-validated in order; those that no longer pass (for instance
    /// because the block falls in another quota period) are returned separately. Nothing is
    /// removed: call [`Mempool::rebase`] once the block is committed.
    pub fn select(
        &self,
        base: &State,
        timestamp: u64,
        max: usize,
    ) -> (Vec<Transaction>, Vec<(Hash, TxError)>) {
        let mut scratch = base.clone();
        let mut selected = Vec::new();
        let mut rejected = Vec::new();
        for (hash, tx) in &self.queue {
            if selected.len() >= max {
                break;
            }
            match scratch.apply_tx(tx, timestamp) {
                Ok(()) => selected.push(tx.clone()),
                Err(error) => rejected.push((*hash, error)),
            }
        }
        (selected, rejected)
    }

    /// Restarts from the new chain `state`: forgets the transactions in `remove` (included
    /// or rejected), re-validates the others in order, and returns those that had to be
    /// dropped.
    pub fn rebase(
        &mut self,
        state: &State,
        remove: &HashSet<Hash>,
        now: u64,
    ) -> Vec<(Hash, TxError)> {
        let mut pending = state.clone();
        let mut dropped = Vec::new();
        let mut kept = VecDeque::with_capacity(self.queue.len());
        for (hash, tx) in self.queue.drain(..) {
            if remove.contains(&hash) {
                continue;
            }
            match pending.apply_tx(&tx, now) {
                Ok(()) => kept.push_back((hash, tx)),
                Err(error) => dropped.push((hash, error)),
            }
        }
        self.hashes = kept.iter().map(|(hash, _)| *hash).collect();
        self.queue = kept;
        self.pending = pending;
        dropped
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use epicoin_core::{AuthorityAction, Genesis, Keypair, ReasonCode, TxPayload};

    const CHAIN_ID: &str = "epicoin-test";
    const NOW: u64 = 1_000_001;

    struct Fixture {
        admin: Keypair,
        staff: Keypair,
        student: Keypair,
        state: State,
    }

    fn key(seed: u8) -> Keypair {
        Keypair::from_secret_bytes(&[seed; 32])
    }

    fn fixture() -> Fixture {
        let admin = key(2);
        let genesis = Genesis {
            chain_id: CHAIN_ID.into(),
            timestamp: 1_000_000,
            validators: vec![key(1).public_key()],
            admin: admin.address(),
            treasury: key(3).address(),
            quota_period_ms: 1_000,
        };
        Fixture {
            admin,
            staff: key(10),
            student: key(20),
            state: State::new(genesis).unwrap(),
        }
    }

    fn grant(f: &Fixture, nonce: u64, quota: u64) -> Transaction {
        Transaction::sign(
            CHAIN_ID,
            &f.admin,
            nonce,
            TxPayload::AuthorityUpdate {
                authority: f.staff.address(),
                action: AuthorityAction::Grant { quota },
            },
        )
    }

    fn mint(f: &Fixture, nonce: u64, amount: u64) -> Transaction {
        Transaction::sign(
            CHAIN_ID,
            &f.staff,
            nonce,
            TxPayload::Mint {
                to: f.student.address(),
                amount,
                reason: ReasonCode::Event,
            },
        )
    }

    #[test]
    fn queued_transactions_build_on_each_other() {
        let f = fixture();
        let mut pool = Mempool::new(f.state.clone(), 10);
        pool.add(grant(&f, 0, 500), NOW).unwrap();
        // Only valid because the grant is already queued.
        pool.add(mint(&f, 0, 100), NOW).unwrap();
        pool.add(mint(&f, 1, 100), NOW).unwrap();
        assert_eq!(pool.len(), 3);
        assert_eq!(pool.pending_nonce(&f.staff.address()), 2);
        assert_eq!(pool.pending_state().balance(&f.student.address()), 200);
    }

    #[test]
    fn invalid_transactions_are_refused_with_their_reason() {
        let f = fixture();
        let mut pool = Mempool::new(f.state.clone(), 10);
        assert_eq!(
            pool.add(mint(&f, 0, 100), NOW),
            Err(MempoolError::Invalid(TxError::NotAuthority))
        );
        pool.add(grant(&f, 0, 150), NOW).unwrap();
        pool.add(mint(&f, 0, 100), NOW).unwrap();
        assert!(matches!(
            pool.add(mint(&f, 1, 100), NOW),
            Err(MempoolError::Invalid(TxError::QuotaExceeded { .. }))
        ));
        assert!(matches!(
            pool.add(mint(&f, 5, 10), NOW),
            Err(MempoolError::Invalid(TxError::BadNonce { .. }))
        ));
        assert_eq!(pool.len(), 2);
    }

    #[test]
    fn duplicates_are_ignored_and_capacity_is_enforced() {
        let f = fixture();
        let mut pool = Mempool::new(f.state.clone(), 2);
        let g = grant(&f, 0, 500);
        let hash = pool.add(g.clone(), NOW).unwrap();
        assert_eq!(pool.add(g, NOW), Ok(hash));
        assert_eq!(pool.len(), 1);
        assert!(pool.contains(&hash));

        pool.add(mint(&f, 0, 1), NOW).unwrap();
        assert_eq!(pool.add(mint(&f, 1, 1), NOW), Err(MempoolError::Full));
    }

    #[test]
    fn select_respects_the_limit_and_keeps_order() {
        let f = fixture();
        let mut pool = Mempool::new(f.state.clone(), 10);
        let txs = vec![grant(&f, 0, 500), mint(&f, 0, 1), mint(&f, 1, 2)];
        for tx in &txs {
            pool.add(tx.clone(), NOW).unwrap();
        }
        let (selected, rejected) = pool.select(&f.state, NOW, 2);
        assert_eq!(selected, txs[..2]);
        assert!(rejected.is_empty());
        assert_eq!(pool.len(), 3, "select does not remove anything");
    }

    #[test]
    fn select_reports_transactions_that_no_longer_pass() {
        let f = fixture();
        let mut pool = Mempool::new(f.state.clone(), 10);
        pool.add(grant(&f, 0, 500), NOW).unwrap();
        let m = mint(&f, 0, 100);
        pool.add(m.clone(), NOW).unwrap();

        // Against a base state where the grant never happened, the mint is not valid.
        let mut base = f.state.clone();
        base.apply_tx(&grant(&f, 0, 50), NOW).unwrap();
        let (selected, rejected) = pool.select(&base, NOW, 10);
        assert!(selected.is_empty());
        assert_eq!(rejected.len(), 2);
        assert_eq!(rejected[1].0, m.hash());
        assert!(matches!(rejected[1].1, TxError::QuotaExceeded { .. }));
    }

    #[test]
    fn rebase_removes_included_and_revalidates_the_rest() {
        let f = fixture();
        let mut pool = Mempool::new(f.state.clone(), 10);
        let g = grant(&f, 0, 150);
        let m1 = mint(&f, 0, 100);
        let m2 = mint(&f, 1, 50);
        for tx in [&g, &m1, &m2] {
            pool.add(tx.clone(), NOW).unwrap();
        }

        // A block included the grant and the first mint.
        let mut chain_state = f.state.clone();
        chain_state.apply_tx(&g, NOW).unwrap();
        chain_state.apply_tx(&m1, NOW).unwrap();
        let included: HashSet<Hash> = [g.hash(), m1.hash()].into();
        let dropped = pool.rebase(&chain_state, &included, NOW);

        assert!(dropped.is_empty());
        assert_eq!(pool.len(), 1);
        assert!(pool.contains(&m2.hash()));
        assert!(!pool.contains(&g.hash()));
        assert_eq!(pool.pending_nonce(&f.staff.address()), 2);
    }

    #[test]
    fn rebase_drops_transactions_invalidated_by_the_new_state() {
        let f = fixture();
        let mut pool = Mempool::new(f.state.clone(), 10);
        pool.add(grant(&f, 0, 150), NOW).unwrap();
        let m = mint(&f, 0, 100);
        pool.add(m.clone(), NOW).unwrap();

        // The chain moved on without the grant: nothing queued is valid any more… except
        // the grant itself, which is re-validated first and still passes.
        let dropped = pool.rebase(&f.state, &HashSet::new(), NOW);
        assert!(dropped.is_empty());
        assert_eq!(pool.len(), 2);

        // Remove the grant: the mint has lost its authority.
        let g_hash = pool.queue[0].0;
        let dropped = pool.rebase(&f.state, &[g_hash].into(), NOW);
        assert_eq!(dropped, vec![(m.hash(), TxError::NotAuthority)]);
        assert!(pool.is_empty());
        assert_eq!(pool.pending_state(), &f.state);
    }
}
