-- EpiCoin off-chain data.
--
-- GDPR: every piece of personal data lives here and only here. Deleting a `users` row
-- removes the person, their wallet key and their sessions; what remains (chain_txs,
-- audit_log) only keeps anonymous references.

CREATE TABLE users (
    id            UUID PRIMARY KEY,
    email         TEXT NOT NULL UNIQUE,          -- always stored lowercase
    first_name    TEXT NOT NULL,
    last_name     TEXT NOT NULL,
    promo         TEXT NOT NULL DEFAULT '',
    role          TEXT NOT NULL CHECK (role IN ('student', 'staff', 'shop_manager', 'admin')),
    password_hash TEXT,                          -- Argon2id PHC string; NULL until activation
    mint_quota    BIGINT CHECK (mint_quota >= 0), -- staff only, in units; NULL = default quota
    activated_at  TIMESTAMPTZ,
    disabled_at   TIMESTAMPTZ,
    created_at    TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX users_role_idx ON users (role);

-- External identities, for a future Microsoft 365 (OIDC) login. Not used in V1.
CREATE TABLE user_identities (
    provider   TEXT NOT NULL,
    subject    TEXT NOT NULL,
    user_id    UUID NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (provider, subject)
);

-- Custodial wallets: one ed25519 key per user, encrypted at rest with the master key.
CREATE TABLE wallets (
    user_id          UUID PRIMARY KEY REFERENCES users (id) ON DELETE CASCADE,
    address          TEXT NOT NULL UNIQUE,       -- hex
    public_key       TEXT NOT NULL,              -- hex
    encrypted_secret BYTEA NOT NULL,             -- XChaCha20-Poly1305: nonce || ciphertext
    key_id           TEXT NOT NULL,              -- fingerprint of the master key used
    created_at       TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TABLE sessions (
    token_hash BYTEA PRIMARY KEY,                -- hash of the cookie value
    user_id    UUID NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    csrf_token TEXT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    expires_at TIMESTAMPTZ NOT NULL
);

CREATE INDEX sessions_user_idx ON sessions (user_id);

-- Single-use tokens sent by email: account activation and password reset.
CREATE TABLE auth_tokens (
    token_hash BYTEA PRIMARY KEY,
    user_id    UUID NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    purpose    TEXT NOT NULL CHECK (purpose IN ('invite', 'reset')),
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    expires_at TIMESTAMPTZ NOT NULL,
    used_at    TIMESTAMPTZ
);

CREATE INDEX auth_tokens_user_idx ON auth_tokens (user_id, purpose);

-- A group of mints triggered by one staff action.
CREATE TABLE mint_batches (
    id              UUID PRIMARY KEY,
    actor_user_id   UUID REFERENCES users (id) ON DELETE SET NULL,
    idempotency_key TEXT,
    amount          BIGINT NOT NULL CHECK (amount > 0),  -- per recipient, in units
    recipient_count INT NOT NULL CHECK (recipient_count > 0),
    reason_code     TEXT NOT NULL,
    comment         TEXT NOT NULL DEFAULT '',            -- free text: off-chain only
    created_at      TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (actor_user_id, idempotency_key)
);

-- Every transaction signed by the API, stored *before* it is sent to the node. This is
-- both the outbox that makes submissions retry-safe and the readable mirror of the chain
-- used for histories and statistics.
CREATE TABLE chain_txs (
    hash             TEXT PRIMARY KEY,           -- transaction hash, hex
    kind             TEXT NOT NULL CHECK (kind IN ('mint', 'purchase', 'refund', 'authority_update')),
    signer_address   TEXT NOT NULL,
    nonce            BIGINT NOT NULL,
    raw              BYTEA NOT NULL,             -- canonical signed bytes, resubmitted as is
    status           TEXT NOT NULL CHECK (status IN ('pending', 'included', 'failed')),
    error            TEXT,                       -- why it failed: state-machine error as JSON
    block_height     BIGINT,
    block_time       TIMESTAMPTZ,
    actor_user_id    UUID REFERENCES users (id) ON DELETE SET NULL,  -- who triggered it
    subject_user_id  UUID REFERENCES users (id) ON DELETE SET NULL,  -- whose balance changes
    amount           BIGINT,
    reason_code      TEXT,
    batch_id         UUID REFERENCES mint_batches (id) ON DELETE SET NULL,
    created_at       TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at       TIMESTAMPTZ NOT NULL DEFAULT now()
);

-- A nonce is used at most once per signer, except by transactions that failed.
CREATE UNIQUE INDEX chain_txs_signer_nonce_idx ON chain_txs (signer_address, nonce)
    WHERE status <> 'failed';
CREATE INDEX chain_txs_pending_idx ON chain_txs (signer_address, nonce) WHERE status = 'pending';
CREATE INDEX chain_txs_subject_idx ON chain_txs (subject_user_id, created_at DESC);
CREATE INDEX chain_txs_actor_idx ON chain_txs (actor_user_id, created_at DESC);
CREATE INDEX chain_txs_batch_idx ON chain_txs (batch_id);

-- Who did what, when. `details` never contains personal data: only ids, counts and codes.
CREATE TABLE audit_log (
    id            BIGSERIAL PRIMARY KEY,
    at            TIMESTAMPTZ NOT NULL DEFAULT now(),
    actor_user_id UUID REFERENCES users (id) ON DELETE SET NULL,
    action        TEXT NOT NULL,
    target_type   TEXT,
    target_id     TEXT,
    details       JSONB NOT NULL DEFAULT '{}'
);

CREATE INDEX audit_log_at_idx ON audit_log (at DESC);
