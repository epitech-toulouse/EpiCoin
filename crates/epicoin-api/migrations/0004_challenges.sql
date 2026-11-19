-- Challenges: a student who succeeds gets EPC at once, minted with the creator's key and
-- counted against the creator's quota (see `challenges.rs`).
--
-- Two modes: `answer` (the student types the answer, checked against its hash, and a right
-- answer is paid immediately) and `review` (the student explains how they did it and the
-- creator or an admin decides). Titles, descriptions, explanations and notes stay here:
-- the chain only sees a `Mint` with the reason code `challenge`.

CREATE TABLE challenges (
    id              UUID PRIMARY KEY,
    creator_user_id UUID REFERENCES users (id) ON DELETE SET NULL,  -- signs and pays the rewards
    title           TEXT NOT NULL,
    description     TEXT NOT NULL DEFAULT '',
    reward          BIGINT NOT NULL CHECK (reward > 0),             -- per winner, in units
    mode            TEXT NOT NULL CHECK (mode IN ('answer', 'review')),
    answer_hash     TEXT,                    -- Argon2id PHC string of the normalized answer
    cohorts         TEXT[] NOT NULL DEFAULT '{}',                   -- empty: every student
    max_winners     INT CHECK (max_winners > 0),                    -- NULL: no limit
    closes_at       TIMESTAMPTZ,                                    -- NULL: no deadline
    closed_at       TIMESTAMPTZ,                                    -- closed by hand
    created_at      TIMESTAMPTZ NOT NULL DEFAULT now(),
    CHECK ((mode = 'answer') = (answer_hash IS NOT NULL))
);

CREATE INDEX challenges_creator_idx ON challenges (creator_user_id);

-- One row per student and challenge. `approved` exactly when the reward transaction is
-- recorded: both are written in the same SQL transaction. If the chain refuses that
-- transaction, the student counts as unpaid and can be paid again.
CREATE TABLE challenge_submissions (
    id               UUID PRIMARY KEY,
    challenge_id     UUID NOT NULL REFERENCES challenges (id) ON DELETE CASCADE,
    student_user_id  UUID NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    status           TEXT NOT NULL CHECK (status IN ('pending', 'approved', 'rejected')),
    proof            TEXT NOT NULL DEFAULT '',   -- review mode: how the student succeeded
    review_note      TEXT NOT NULL DEFAULT '',   -- why it was rejected
    reviewer_user_id UUID REFERENCES users (id) ON DELETE SET NULL,
    tx_hash          TEXT REFERENCES chain_txs (hash),
    submitted_at     TIMESTAMPTZ NOT NULL DEFAULT now(),
    decided_at       TIMESTAMPTZ,
    UNIQUE (challenge_id, student_user_id),
    CHECK ((status = 'approved') = (tx_hash IS NOT NULL))
);

CREATE INDEX challenge_submissions_pending_idx ON challenge_submissions (challenge_id)
    WHERE status = 'pending';
