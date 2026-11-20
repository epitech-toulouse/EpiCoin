-- When each student last opened each page that can show a "!" for news (EPC received, an
-- order ready or refunded, a new challenge or a decision): news means "since then".
CREATE TABLE section_visits (
    user_id UUID NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    section TEXT NOT NULL CHECK (section IN ('balance', 'orders', 'challenges')),
    seen_at TIMESTAMPTZ NOT NULL,
    PRIMARY KEY (user_id, section)
);
