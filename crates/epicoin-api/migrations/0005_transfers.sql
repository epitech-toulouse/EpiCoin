-- Transfers between students, allowed on chain since V2. The chain only sees the two
-- addresses and the amount; the optional message stays here, and is erased with the account
-- of either student.

ALTER TABLE chain_txs DROP CONSTRAINT chain_txs_kind_check;
ALTER TABLE chain_txs ADD CONSTRAINT chain_txs_kind_check
    CHECK (kind IN ('mint', 'purchase', 'refund', 'authority_update', 'transfer'));

-- One row per transfer, next to its chain transaction (actor = sender, subject = recipient).
CREATE TABLE transfers (
    tx_hash           TEXT PRIMARY KEY REFERENCES chain_txs (hash),
    sender_user_id    UUID REFERENCES users (id) ON DELETE SET NULL,
    recipient_user_id UUID REFERENCES users (id) ON DELETE SET NULL,
    amount            BIGINT NOT NULL CHECK (amount > 0),
    message           TEXT NOT NULL DEFAULT '',   -- free text: off chain only
    idempotency_key   TEXT,
    created_at        TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (sender_user_id, idempotency_key)
);

CREATE INDEX transfers_created_idx ON transfers (created_at DESC);
