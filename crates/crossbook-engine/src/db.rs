//! Postgres access: admitted orders and indexed trades.
//!
//! uint256 values are stored as NUMERIC(78, 0) and carried in Rust as BigDecimal,
//! converted to and from U256 through their decimal string. Addresses and hashes
//! are fixed length BYTEA.

use alloy_primitives::{Address, B256, U256};
use anyhow::{Context, Result};
use sqlx::types::chrono::{DateTime, Utc};
use sqlx::types::BigDecimal;
use sqlx::PgPool;
use std::str::FromStr;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredOrder {
    pub hash: B256,
    pub maker: Address,
    pub sell_token: Address,
    pub buy_token: Address,
    pub sell_amount: U256,
    pub buy_amount: U256,
    pub valid_to: u64,
    pub nonce: U256,
    pub status: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TradeRow {
    pub tx_hash: B256,
    pub log_index: u32,
    pub maker: Address,
    pub sell_token: Address,
    pub buy_token: Address,
    pub sell_filled: U256,
    pub buy_filled: U256,
    pub order_hash: B256,
    pub block_number: u64,
    pub block_time: DateTime<Utc>,
}

/// Connect and run pending migrations.
pub async fn connect(url: &str) -> Result<PgPool> {
    let pool = PgPool::connect(url).await.context("connect postgres")?;
    sqlx::migrate!("./migrations")
        .run(&pool)
        .await
        .context("run migrations")?;
    Ok(pool)
}

pub async fn insert_order(pool: &PgPool, o: &StoredOrder) -> Result<()> {
    sqlx::query!(
        "INSERT INTO orders \
         (hash, maker, sell_token, buy_token, sell_amount, buy_amount, valid_to, nonce, status) \
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9) \
         ON CONFLICT (hash) DO NOTHING",
        o.hash.as_slice(),
        o.maker.as_slice(),
        o.sell_token.as_slice(),
        o.buy_token.as_slice(),
        to_numeric(o.sell_amount)?,
        to_numeric(o.buy_amount)?,
        o.valid_to as i64,
        to_numeric(o.nonce)?,
        o.status,
    )
    .execute(pool)
    .await
    .context("insert order")?;
    Ok(())
}

pub async fn set_order_status(pool: &PgPool, hash: B256, status: &str) -> Result<()> {
    sqlx::query!(
        "UPDATE orders SET status = $2 WHERE hash = $1",
        hash.as_slice(),
        status
    )
    .execute(pool)
    .await
    .context("set order status")?;
    Ok(())
}

pub async fn get_order(pool: &PgPool, hash: B256) -> Result<Option<StoredOrder>> {
    let row = sqlx::query!(
        "SELECT hash, maker, sell_token, buy_token, sell_amount, buy_amount, valid_to, nonce, status \
         FROM orders WHERE hash = $1",
        hash.as_slice()
    )
    .fetch_optional(pool)
    .await
    .context("get order")?;

    row.map(|r| {
        Ok(StoredOrder {
            hash: to_b256(&r.hash)?,
            maker: to_addr(&r.maker)?,
            sell_token: to_addr(&r.sell_token)?,
            buy_token: to_addr(&r.buy_token)?,
            sell_amount: from_numeric(&r.sell_amount)?,
            buy_amount: from_numeric(&r.buy_amount)?,
            valid_to: r.valid_to as u64,
            nonce: from_numeric(&r.nonce)?,
            status: r.status,
        })
    })
    .transpose()
}

pub async fn insert_trade(pool: &PgPool, t: &TradeRow) -> Result<()> {
    sqlx::query!(
        "INSERT INTO trades \
         (tx_hash, log_index, maker, sell_token, buy_token, sell_filled, buy_filled, order_hash, block_number, block_time) \
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10) \
         ON CONFLICT (tx_hash, log_index) DO NOTHING",
        t.tx_hash.as_slice(),
        t.log_index as i32,
        t.maker.as_slice(),
        t.sell_token.as_slice(),
        t.buy_token.as_slice(),
        to_numeric(t.sell_filled)?,
        to_numeric(t.buy_filled)?,
        t.order_hash.as_slice(),
        t.block_number as i64,
        t.block_time,
    )
    .execute(pool)
    .await
    .context("insert trade")?;
    Ok(())
}

pub async fn recent_trades(
    pool: &PgPool,
    sell_token: Address,
    buy_token: Address,
    limit: i64,
) -> Result<Vec<TradeRow>> {
    let rows = sqlx::query!(
        "SELECT tx_hash, log_index, maker, sell_token, buy_token, sell_filled, buy_filled, order_hash, block_number, block_time \
         FROM trades WHERE sell_token = $1 AND buy_token = $2 \
         ORDER BY block_number DESC LIMIT $3",
        sell_token.as_slice(),
        buy_token.as_slice(),
        limit
    )
    .fetch_all(pool)
    .await
    .context("recent trades")?;

    rows.into_iter()
        .map(|r| {
            Ok(TradeRow {
                tx_hash: to_b256(&r.tx_hash)?,
                log_index: r.log_index as u32,
                maker: to_addr(&r.maker)?,
                sell_token: to_addr(&r.sell_token)?,
                buy_token: to_addr(&r.buy_token)?,
                sell_filled: from_numeric(&r.sell_filled)?,
                buy_filled: from_numeric(&r.buy_filled)?,
                order_hash: to_b256(&r.order_hash)?,
                block_number: r.block_number as u64,
                block_time: r.block_time,
            })
        })
        .collect()
}

pub async fn get_cursor(pool: &PgPool) -> Result<Option<(u64, B256)>> {
    let row = sqlx::query!("SELECT last_block, last_block_hash FROM indexer_cursor WHERE id = 1")
        .fetch_optional(pool)
        .await
        .context("get cursor")?;
    row.map(|r| Ok((r.last_block as u64, to_b256(&r.last_block_hash)?)))
        .transpose()
}

pub async fn set_cursor(pool: &PgPool, last_block: u64, last_block_hash: B256) -> Result<()> {
    sqlx::query!(
        "INSERT INTO indexer_cursor (id, last_block, last_block_hash) VALUES (1, $1, $2) \
         ON CONFLICT (id) DO UPDATE SET last_block = $1, last_block_hash = $2",
        last_block as i64,
        last_block_hash.as_slice()
    )
    .execute(pool)
    .await
    .context("set cursor")?;
    Ok(())
}

fn to_numeric(x: U256) -> Result<BigDecimal> {
    BigDecimal::from_str(&x.to_string()).context("u256 to numeric")
}

fn from_numeric(x: &BigDecimal) -> Result<U256> {
    // NUMERIC(78, 0) has no fractional part; keep the integer digits only.
    let s = x.to_string();
    let int_part = s.split('.').next().unwrap_or("0");
    U256::from_str(int_part).context("numeric to u256")
}

fn to_addr(b: &[u8]) -> Result<Address> {
    (b.len() == 20)
        .then(|| Address::from_slice(b))
        .context("bad address length")
}

fn to_b256(b: &[u8]) -> Result<B256> {
    (b.len() == 32)
        .then(|| B256::from_slice(b))
        .context("bad hash length")
}
