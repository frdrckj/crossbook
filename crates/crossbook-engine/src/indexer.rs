//! The chain indexer. Reads Trade events into Postgres and fans them out to
//! WebSocket subscribers, advancing a cursor so restarts resume. It records the
//! last block hash so a reorg can be detected; rollback is a production concern,
//! since the MVP targets a local Anvil that does not reorg.

use crate::chain::Chain;
use crate::db::{self, TradeRow};
use sqlx::types::chrono::{DateTime, Utc};
use sqlx::PgPool;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::broadcast;

const POLL: Duration = Duration::from_millis(1000);

pub async fn run(chain: Arc<Chain>, db: PgPool, trades_tx: broadcast::Sender<TradeRow>) {
    loop {
        if let Err(e) = tick(&chain, &db, &trades_tx).await {
            tracing::warn!(error = ?e, "indexer tick failed");
        }
        tokio::time::sleep(POLL).await;
    }
}

async fn tick(
    chain: &Chain,
    db: &PgPool,
    trades_tx: &broadcast::Sender<TradeRow>,
) -> anyhow::Result<()> {
    let head = chain.latest_block().await?;
    let from = match db::get_cursor(db).await? {
        Some((last, _)) => last + 1,
        None => 0,
    };
    if from > head {
        return Ok(());
    }

    let logs = chain.trades_in_range(from, head).await?;
    let mut times: HashMap<u64, DateTime<Utc>> = HashMap::new();

    for (event, log) in logs {
        let block_number = log.block_number.unwrap_or_default();
        let block_time = match times.get(&block_number) {
            Some(t) => *t,
            None => {
                let (ts, _) = chain.block_info(block_number).await?;
                let t = DateTime::<Utc>::from_timestamp(ts as i64, 0).unwrap_or_default();
                times.insert(block_number, t);
                t
            }
        };

        let row = TradeRow {
            tx_hash: log.transaction_hash.unwrap_or_default(),
            log_index: log.log_index.unwrap_or_default() as u32,
            maker: event.maker,
            sell_token: event.sellToken,
            buy_token: event.buyToken,
            sell_filled: event.sellFilled,
            buy_filled: event.buyFilled,
            order_hash: event.orderHash,
            block_number,
            block_time,
        };
        db::insert_trade(db, &row).await?;
        let _ = trades_tx.send(row);
    }

    let (_, head_hash) = chain.block_info(head).await?;
    db::set_cursor(db, head, head_hash).await?;
    Ok(())
}
