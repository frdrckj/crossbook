-- Off chain orders the engine has admitted. Useful for status queries.
CREATE TABLE orders (
    hash         BYTEA PRIMARY KEY,
    maker        BYTEA NOT NULL,
    sell_token   BYTEA NOT NULL,
    buy_token    BYTEA NOT NULL,
    sell_amount  NUMERIC(78, 0) NOT NULL,
    buy_amount   NUMERIC(78, 0) NOT NULL,
    valid_to     BIGINT NOT NULL,
    nonce        NUMERIC(78, 0) NOT NULL,
    status       TEXT NOT NULL,
    created_at   TIMESTAMPTZ NOT NULL DEFAULT now()
);

-- Trades reconstructed from on chain Trade events, the source of truth.
CREATE TABLE trades (
    tx_hash       BYTEA NOT NULL,
    log_index     INTEGER NOT NULL,
    maker         BYTEA NOT NULL,
    sell_token    BYTEA NOT NULL,
    buy_token     BYTEA NOT NULL,
    sell_filled   NUMERIC(78, 0) NOT NULL,
    buy_filled    NUMERIC(78, 0) NOT NULL,
    order_hash    BYTEA NOT NULL,
    block_number  BIGINT NOT NULL,
    block_time    TIMESTAMPTZ NOT NULL,
    PRIMARY KEY (tx_hash, log_index)
);

CREATE INDEX trades_pair_idx ON trades (sell_token, buy_token, block_number DESC);

-- Indexer progress. Stores the last block hash so a reorg can be detected.
CREATE TABLE indexer_cursor (
    id              INTEGER PRIMARY KEY DEFAULT 1,
    last_block      BIGINT NOT NULL,
    last_block_hash BYTEA NOT NULL,
    CHECK (id = 1)
);
