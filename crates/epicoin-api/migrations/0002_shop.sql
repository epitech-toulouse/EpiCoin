-- The shop: products, orders and their link to on-chain transactions.

CREATE TABLE products (
    id          UUID PRIMARY KEY,
    name        TEXT NOT NULL,
    description TEXT NOT NULL DEFAULT '',
    image_file  TEXT,                                 -- file name in the upload directory
    price       BIGINT NOT NULL CHECK (price > 0),    -- in units (1 EPC = 100)
    stock       INT NOT NULL CHECK (stock >= 0),      -- can never go negative
    active      BOOLEAN NOT NULL DEFAULT true,
    created_at  TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at  TIMESTAMPTZ NOT NULL DEFAULT now()
);

-- Lifecycle:
--   awaiting_payment -> pending -> ready -> delivered
--                    \-> failed            (the Purchase was refused: stock given back)
--   pending | ready  -> refunding -> cancelled   (Refund included: stock given back)
CREATE TABLE orders (
    id              UUID PRIMARY KEY,
    user_id         UUID REFERENCES users (id) ON DELETE SET NULL,
    idempotency_key TEXT NOT NULL,
    status          TEXT NOT NULL CHECK (status IN
        ('awaiting_payment', 'failed', 'pending', 'ready', 'delivered', 'refunding', 'cancelled')),
    total           BIGINT NOT NULL CHECK (total > 0),
    failure         TEXT,                             -- why the payment failed (error code)
    created_at      TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at      TIMESTAMPTZ NOT NULL DEFAULT now(),
    -- A retried or double-clicked checkout maps to the same order.
    UNIQUE (user_id, idempotency_key)
);

CREATE INDEX orders_user_idx ON orders (user_id, created_at DESC);
CREATE INDEX orders_status_idx ON orders (status, created_at);

CREATE TABLE order_items (
    order_id     UUID NOT NULL REFERENCES orders (id) ON DELETE CASCADE,
    product_id   UUID NOT NULL REFERENCES products (id),
    product_name TEXT NOT NULL,                       -- as it was when ordered
    unit_price   BIGINT NOT NULL CHECK (unit_price > 0),
    quantity     INT NOT NULL CHECK (quantity > 0),
    PRIMARY KEY (order_id, product_id)
);

-- The Purchase and Refund transactions of an order.
ALTER TABLE chain_txs ADD COLUMN order_id UUID REFERENCES orders (id) ON DELETE SET NULL;
CREATE INDEX chain_txs_order_idx ON chain_txs (order_id);
