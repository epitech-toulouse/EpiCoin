-- Knowledge shares ("Apprends-nous quelque chose !", formerly the separate Epicreds app).
--
-- A student teaches a staff member something; the staff member shows a short-lived QR code
-- and the student scans it to be paid. The payment is one `Mint` signed with the staff
-- member's key (reason `knowledge_share`), counted against their quota, and includes the
-- regularity bonus when the scan completes a streak tier. Topics, mentions, streaks, badges
-- and the ranking stay here: the chain only sees the mint. See `shares/mod.rs`.

-- The campus settings of the feature, edited by admins. A single row.
CREATE TABLE share_settings (
    id          SMALLINT PRIMARY KEY DEFAULT 1 CHECK (id = 1),
    standard    BIGINT NOT NULL DEFAULT 1000 CHECK (standard > 0),     -- suggested amount, units
    max_amount  BIGINT NOT NULL DEFAULT 10000,                         -- per share, units
    qr_seconds  INT NOT NULL DEFAULT 120 CHECK (qr_seconds BETWEEN 15 AND 600),
    time_zone   TEXT NOT NULL DEFAULT 'Europe/Paris',                  -- the campus calendar
    weekdays    SMALLINT[] NOT NULL DEFAULT '{1,2,3,4,5}',             -- ISO: 1 = Monday, 7 = Sunday
    closures    DATE[] NOT NULL DEFAULT '{}',                          -- days the campus is closed
    -- Streak tiers: reaching `days` consecutive working days pays `bonus` units, once.
    tiers       JSONB NOT NULL
                DEFAULT '[{"days": 5, "bonus": 2000}, {"days": 10, "bonus": 5000}, {"days": 20, "bonus": 10000}]',
    daily_limit INT CHECK (daily_limit > 0),                           -- shares per student and day
    updated_at  TIMESTAMPTZ NOT NULL DEFAULT now(),
    CHECK (standard <= max_amount)
);

INSERT INTO share_settings DEFAULT VALUES;

-- One row per QR code. A share is "claimed" when its payment transaction exists and the
-- chain did not refuse it; a refused payment leaves the QR code usable until it expires.
CREATE TABLE shares (
    id              UUID PRIMARY KEY,
    token_hash      BYTEA NOT NULL UNIQUE,         -- the secret of the QR code, hashed
    staff_user_id   UUID REFERENCES users (id) ON DELETE SET NULL,   -- validates and pays
    student_user_id UUID REFERENCES users (id) ON DELETE SET NULL,   -- scanned it
    amount          BIGINT NOT NULL CHECK (amount > 0),              -- units, fixed at creation
    standard        BIGINT NOT NULL,               -- the suggested amount at creation ("tip" badge)
    mention         TEXT,                          -- a code of `shares::MENTIONS`
    topic           TEXT NOT NULL DEFAULT '',      -- free text: off chain only
    expires_at      TIMESTAMPTZ NOT NULL,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT now(),
    invalidated_at  TIMESTAMPTZ,                   -- replaced by a newer QR code, or withdrawn
    claimed_at      TIMESTAMPTZ,
    claim_day       DATE,                          -- `claimed_at` on the campus calendar
    bonus           BIGINT NOT NULL DEFAULT 0 CHECK (bonus >= 0),    -- streak bonus, in the same mint
    streak_start    DATE,                          -- the streak whose tier the bonus pays
    streak_days     INT,
    tx_hash         TEXT REFERENCES chain_txs (hash),
    -- Cancelled by an admin: out of streaks, badges and ranking. The EPC stay (the chain
    -- cannot take them back).
    cancelled_at    TIMESTAMPTZ,
    cancelled_by    UUID REFERENCES users (id) ON DELETE SET NULL,
    cancel_reason   TEXT NOT NULL DEFAULT '',
    CHECK ((claimed_at IS NULL) = (tx_hash IS NULL)),
    CHECK ((claimed_at IS NULL) = (claim_day IS NULL)),
    CHECK ((bonus > 0) = (streak_start IS NOT NULL AND streak_days IS NOT NULL)),
    CHECK (cancelled_at IS NULL OR claimed_at IS NOT NULL)
);

CREATE INDEX shares_staff_idx ON shares (staff_user_id, created_at DESC);
CREATE INDEX shares_student_idx ON shares (student_user_id, claim_day);
CREATE INDEX shares_claimed_idx ON shares (claimed_at DESC) WHERE claimed_at IS NOT NULL;

-- "Légende du campus": the only badge given by hand, by an admin.
CREATE TABLE share_legends (
    user_id         UUID PRIMARY KEY REFERENCES users (id) ON DELETE CASCADE,
    granted_by      UUID REFERENCES users (id) ON DELETE SET NULL,
    granted_at      TIMESTAMPTZ NOT NULL DEFAULT now()
);

-- Students appear in the ranking unless they opt out.
ALTER TABLE users ADD COLUMN hidden_from_ranking BOOLEAN NOT NULL DEFAULT false;
