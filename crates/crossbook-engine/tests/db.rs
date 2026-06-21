//! Database round trip tests. Skipped when DATABASE_URL is not set, so the suite
//! still passes without a live Postgres. Run them with `just dev` up.

use alloy_primitives::{Address, B256, U256};
use crossbook_engine::db::{self, StoredOrder, TradeRow};
use sqlx::types::chrono::{DateTime, Utc};

fn ts() -> DateTime<Utc> {
    DateTime::<Utc>::from_timestamp(1_700_000_000, 0).unwrap()
}

#[tokio::test]
async fn order_trade_and_cursor_round_trip() {
    let Ok(url) = std::env::var("DATABASE_URL") else {
        eprintln!("skipping db test: DATABASE_URL not set");
        return;
    };
    let pool = db::connect(&url).await.unwrap();

    // Clean any rows from a prior run so the test is idempotent.
    sqlx::query("DELETE FROM orders WHERE hash = $1")
        .bind(&[0x42u8; 32][..])
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("DELETE FROM trades WHERE tx_hash = $1")
        .bind(&[0x99u8; 32][..])
        .execute(&pool)
        .await
        .unwrap();

    // Order round trip, including U256::MAX to exercise the NUMERIC conversion.
    let order = StoredOrder {
        hash: B256::repeat_byte(0x42),
        maker: Address::repeat_byte(0x01),
        sell_token: Address::repeat_byte(0x02),
        buy_token: Address::repeat_byte(0x03),
        sell_amount: U256::MAX,
        buy_amount: U256::from(1000u64),
        valid_to: 1_700_000_000,
        nonce: U256::from(7u64),
        status: "open".to_string(),
    };
    db::insert_order(&pool, &order).await.unwrap();
    assert_eq!(
        db::get_order(&pool, order.hash).await.unwrap().as_ref(),
        Some(&order)
    );

    db::set_order_status(&pool, order.hash, "filled")
        .await
        .unwrap();
    assert_eq!(
        db::get_order(&pool, order.hash)
            .await
            .unwrap()
            .unwrap()
            .status,
        "filled"
    );

    // Trade round trip.
    let trade = TradeRow {
        tx_hash: B256::repeat_byte(0x99),
        log_index: 0,
        maker: Address::repeat_byte(0x01),
        sell_token: Address::repeat_byte(0x02),
        buy_token: Address::repeat_byte(0x03),
        sell_filled: U256::MAX,
        buy_filled: U256::from(1000u64),
        order_hash: order.hash,
        block_number: 123,
        block_time: ts(),
    };
    db::insert_trade(&pool, &trade).await.unwrap();
    let trades = db::recent_trades(
        &pool,
        Address::repeat_byte(0x02),
        Address::repeat_byte(0x03),
        10,
    )
    .await
    .unwrap();
    let found = trades.iter().find(|t| t.tx_hash == trade.tx_hash).unwrap();
    assert_eq!(found.sell_filled, U256::MAX);
    assert_eq!(found, &trade);

    // Cursor round trip.
    db::set_cursor(&pool, 500, B256::repeat_byte(0xAB))
        .await
        .unwrap();
    assert_eq!(
        db::get_cursor(&pool).await.unwrap(),
        Some((500, B256::repeat_byte(0xAB)))
    );
}
