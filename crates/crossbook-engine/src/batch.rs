//! The batch window driver.
//!
//! In batch mode a single task closes the window on a fixed interval, asks the
//! engine to clear the collected orders at a uniform price per pair, and submits
//! one `settleBatch` per run. Settlement emits Trade events that the indexer
//! persists, exactly as in continuous mode, so the read path is unchanged. The
//! latest clearing is published into `BatchState` for the dashboard.

use crate::api::{BatchState, ClearingView};
use crate::chain::Chain;
use crate::engine_task::EngineHandle;
use crate::settle::{self, AdmittedOrder};
use crossbook_core::types::OrderHash;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Run the window loop forever. Returns only if the engine task is gone.
pub async fn run_window(
    engine: EngineHandle,
    chain: Arc<Chain>,
    admitted: Arc<Mutex<HashMap<OrderHash, AdmittedOrder>>>,
    state: Arc<Mutex<BatchState>>,
    interval: Duration,
) {
    let mut ticker = tokio::time::interval(interval);
    ticker.tick().await; // the first tick fires immediately; skip it

    loop {
        if let Ok(mut s) = state.lock() {
            s.window_closes_at = now_secs() + interval.as_secs();
        }
        ticker.tick().await;

        let now = now_secs();
        let outcome = match engine.close_batch(now).await {
            Ok(o) => o,
            Err(_) => {
                tracing::error!("engine task gone, stopping batch window");
                return;
            }
        };

        if let Ok(mut s) = state.lock() {
            s.last_close_at = now;
            s.last_results = outcome.pairs.iter().map(ClearingView::from).collect();
            s.last_rings = outcome.rings.len() as u64;
        }
        if outcome.pairs.is_empty() && outcome.rings.is_empty() {
            continue;
        }

        let map = admitted.lock().ok().map(|m| m.clone()).unwrap_or_default();

        // Pairs settle through settleBatch with their uniform price assertion.
        if !outcome.pairs.is_empty() {
            match settle::to_batch_settlement(&outcome.pairs, &map) {
                Ok((signed, fills, prices)) => {
                    let pairs = outcome.pairs.len();
                    let chain = chain.clone();
                    tokio::spawn(async move {
                        match chain.settle_batch(signed, fills, prices).await {
                            Ok(tx) => {
                                metrics::counter!("crossbook_batches_total").increment(1);
                                tracing::info!(%tx, pairs, "settled batch auction");
                            }
                            Err(e) => tracing::error!(error = ?e, "batch settle submission failed"),
                        }
                    });
                }
                Err(e) => tracing::error!(error = ?e, "failed to build batch settlement"),
            }
        }

        // Rings net to zero across their own tokens and respect every limit, so
        // they settle through the plain settle path with no uniform price.
        if !outcome.rings.is_empty() {
            match settle::to_ring_settlement(&outcome.rings, &map) {
                Ok((signed, fills)) => {
                    let rings = outcome.rings.len();
                    let chain = chain.clone();
                    tokio::spawn(async move {
                        match chain.settle(signed, fills).await {
                            Ok(tx) => {
                                metrics::counter!("crossbook_rings_total").increment(1);
                                tracing::info!(%tx, rings, "settled token rings");
                            }
                            Err(e) => tracing::error!(error = ?e, "ring settle submission failed"),
                        }
                    });
                }
                Err(e) => tracing::error!(error = ?e, "failed to build ring settlement"),
            }
        }
    }
}
