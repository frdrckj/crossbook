//! HTTP and WebSocket API.

use crate::chain::Chain;
use crate::config::MatchingMode;
use crate::db::{self, StoredOrder, TradeRow};
use crate::engine_task::EngineHandle;
use crate::ingest::{self, OrderPayload};
use crate::reject::RejectReason;
use crate::settle::{self, AdmittedOrder};
use alloy_primitives::{Address, B256};
use alloy_sol_types::Eip712Domain;
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::{Html, IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use crossbook_core::types::{OrderHash, SubmitOutcome};
use metrics_exporter_prometheus::PrometheusHandle;
use serde::Serialize;
use serde_json::json;
use sqlx::PgPool;
use std::collections::HashMap;
use std::str::FromStr;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::sync::broadcast;

/// What the demo dashboard needs to build EIP-712 orders.
#[derive(Clone, Serialize)]
pub struct DemoConfig {
    pub chain_id: u64,
    pub settlement: String,
    pub token_a: Option<String>,
    pub token_b: Option<String>,
}

/// One pair's clearing in the most recent batch, for the dashboard.
#[derive(Clone, Serialize)]
pub struct ClearingView {
    pub base: String,
    pub quote: String,
    pub clearing_num: String,
    pub clearing_den: String,
    pub volume_base: String,
    pub surplus: String,
    pub fills: usize,
}

impl From<&crossbook_core::auction::AuctionResult> for ClearingView {
    fn from(r: &crossbook_core::auction::AuctionResult) -> Self {
        ClearingView {
            base: r.base.to_string(),
            quote: r.quote.to_string(),
            clearing_num: r.clearing_num.to_string(),
            clearing_den: r.clearing_den.to_string(),
            volume_base: r.volume_base.to_string(),
            surplus: r.surplus.to_string(),
            fills: r.fills.len(),
        }
    }
}

/// Live view of the batch window, updated by the window driver. In continuous
/// mode it just reports the mode and stays otherwise empty.
#[derive(Clone, Serialize, Default)]
pub struct BatchState {
    pub mode: String,
    pub interval_secs: u64,
    /// Unix seconds when the current window closes (0 in continuous mode).
    pub window_closes_at: u64,
    pub last_close_at: u64,
    pub last_results: Vec<ClearingView>,
    /// Number of multi token rings cleared in the last window.
    pub last_rings: u64,
}

/// Shared engine state. Cheap to clone (everything inside is shared).
#[derive(Clone)]
pub struct AppState {
    pub engine: EngineHandle,
    pub chain: Arc<Chain>,
    pub db: PgPool,
    pub domain: Arc<Eip712Domain>,
    pub admitted: Arc<Mutex<HashMap<OrderHash, AdmittedOrder>>>,
    pub seq: Arc<AtomicU64>,
    pub trades_tx: broadcast::Sender<TradeRow>,
    pub metrics: Arc<PrometheusHandle>,
    pub demo: DemoConfig,
    pub batch: Arc<Mutex<BatchState>>,
    pub mode: MatchingMode,
}

pub fn router(state: AppState) -> Router {
    Router::new()
        .route("/", get(index))
        .route("/config", get(config))
        .route("/health", get(health))
        .route("/metrics", get(metrics))
        .route("/orders", post(post_order))
        .route("/orders/{hash}", get(get_order).delete(cancel_order))
        .route("/book/{base}/{quote}", get(get_book))
        .route("/batch", get(get_batch))
        .route("/trades", get(get_trades))
        .route("/ws", get(ws_handler))
        .with_state(state)
}

async fn index() -> Html<&'static str> {
    Html(include_str!("../static/index.html"))
}

async fn config(State(st): State<AppState>) -> Json<DemoConfig> {
    Json(st.demo.clone())
}

async fn get_batch(State(st): State<AppState>) -> Response {
    let snapshot = st.batch.lock().ok().map(|s| s.clone()).unwrap_or_default();
    Json(snapshot).into_response()
}

async fn health() -> &'static str {
    "ok"
}

async fn metrics(State(st): State<AppState>) -> String {
    st.metrics.render()
}

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

#[derive(Serialize)]
struct Accepted {
    hash: String,
    status: String,
}

/// Turn a rejection into a counted HTTP response, the single place that shape lives.
fn rejected(reason: RejectReason) -> Response {
    metrics::counter!("crossbook_orders_rejected_total", "reason" => reason.label()).increment(1);
    let code = StatusCode::from_u16(reason.http_status()).unwrap_or(StatusCode::BAD_REQUEST);
    (
        code,
        Json(json!({ "error": reason.to_string(), "reason": reason.label() })),
    )
        .into_response()
}

async fn post_order(State(st): State<AppState>, Json(payload): Json<OrderPayload>) -> Response {
    // A uniform price batch clears in whole lots, so it cannot honor a fill or
    // kill order's exact amount. Reject it up front in batch mode.
    if st.mode == MatchingMode::Batch && !payload.partially_fillable {
        return rejected(RejectReason::FillOrKillNotInBatch);
    }

    let seq = st.seq.fetch_add(1, Ordering::Relaxed);
    let validated = match ingest::validate(&payload, &st.chain, &st.domain, now_secs(), seq).await {
        Ok(v) => v,
        Err(reason) => return rejected(reason),
    };
    metrics::counter!("crossbook_orders_admitted_total").increment(1);

    let hash = validated.open.hash;
    if let Ok(mut map) = st.admitted.lock() {
        map.insert(hash, validated.admitted.clone());
    }

    let stored = StoredOrder {
        hash: B256::from(hash),
        maker: payload.maker,
        sell_token: payload.sell_token,
        buy_token: payload.buy_token,
        sell_amount: payload.sell_amount,
        buy_amount: payload.buy_amount,
        valid_to: payload.valid_to,
        nonce: payload.nonce,
        status: "open".to_string(),
    };
    if let Err(e) = db::insert_order(&st.db, &stored).await {
        tracing::error!(error = ?e, "failed to persist order");
    }

    let result = match st.engine.submit(validated.open).await {
        Ok(r) => r,
        Err(_) => {
            return (
                StatusCode::SERVICE_UNAVAILABLE,
                Json(json!({ "error": "engine unavailable" })),
            )
                .into_response();
        }
    };

    let status = match result.outcome {
        SubmitOutcome::FullyFilled => "filled",
        SubmitOutcome::PartiallyFilled { .. } => "partial",
        SubmitOutcome::Resting => "open",
        SubmitOutcome::Killed => "killed",
    };

    if !result.fills.is_empty() {
        let map = st
            .admitted
            .lock()
            .ok()
            .map(|m| m.clone())
            .unwrap_or_default();
        match settle::to_settlement(&result.fills, &map) {
            Ok((signed, rows)) => {
                let chain = st.chain.clone();
                tokio::spawn(async move {
                    match chain.settle(signed, rows).await {
                        Ok(tx) => {
                            metrics::counter!("crossbook_settlements_total").increment(1);
                            tracing::info!(%tx, "settled batch");
                        }
                        Err(e) => tracing::error!(error = ?e, "settle submission failed"),
                    }
                });
            }
            Err(e) => tracing::error!(error = ?e, "failed to build settlement"),
        }
    }

    (
        StatusCode::OK,
        Json(Accepted {
            hash: B256::from(hash).to_string(),
            status: status.to_string(),
        }),
    )
        .into_response()
}

#[derive(Serialize)]
struct OrderView {
    hash: String,
    maker: String,
    sell_token: String,
    buy_token: String,
    sell_amount: String,
    buy_amount: String,
    valid_to: u64,
    nonce: String,
    status: String,
}

impl From<StoredOrder> for OrderView {
    fn from(o: StoredOrder) -> Self {
        OrderView {
            hash: o.hash.to_string(),
            maker: o.maker.to_string(),
            sell_token: o.sell_token.to_string(),
            buy_token: o.buy_token.to_string(),
            sell_amount: o.sell_amount.to_string(),
            buy_amount: o.buy_amount.to_string(),
            valid_to: o.valid_to,
            nonce: o.nonce.to_string(),
            status: o.status,
        }
    }
}

async fn get_order(State(st): State<AppState>, Path(hash): Path<String>) -> Response {
    let Ok(h) = B256::from_str(&hash) else {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({ "error": "bad hash" })),
        )
            .into_response();
    };
    match db::get_order(&st.db, h).await {
        Ok(Some(o)) => Json(OrderView::from(o)).into_response(),
        Ok(None) => (StatusCode::NOT_FOUND, Json(json!({ "error": "not found" }))).into_response(),
        Err(e) => {
            tracing::error!(error = ?e, "get order");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({ "error": "internal" })),
            )
                .into_response()
        }
    }
}

async fn cancel_order(State(st): State<AppState>, Path(hash): Path<String>) -> Response {
    let Ok(h) = B256::from_str(&hash) else {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({ "error": "bad hash" })),
        )
            .into_response();
    };
    let removed = st.engine.cancel(h.0).await.unwrap_or(false);
    if removed {
        let _ = db::set_order_status(&st.db, h, "cancelled").await;
    }
    Json(json!({ "cancelled": removed })).into_response()
}

#[derive(Serialize)]
struct BookOrder {
    hash: String,
    sell_amount: String,
    buy_amount: String,
    remaining_sell: String,
}

#[derive(Serialize)]
struct BookView {
    asks: Vec<BookOrder>,
    bids: Vec<BookOrder>,
}

async fn get_book(
    State(st): State<AppState>,
    Path((base, quote)): Path<(String, String)>,
) -> Response {
    let (Ok(base), Ok(quote)) = (Address::from_str(&base), Address::from_str(&quote)) else {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({ "error": "bad token address" })),
        )
            .into_response();
    };
    let mut view = BookView {
        asks: Vec::new(),
        bids: Vec::new(),
    };
    for o in st.engine.snapshot().iter() {
        let entry = BookOrder {
            hash: B256::from(o.hash).to_string(),
            sell_amount: o.order.sell_amount.to_string(),
            buy_amount: o.order.buy_amount.to_string(),
            remaining_sell: o.remaining_sell.to_string(),
        };
        if o.order.sell_token == base && o.order.buy_token == quote {
            view.asks.push(entry);
        } else if o.order.sell_token == quote && o.order.buy_token == base {
            view.bids.push(entry);
        }
    }
    Json(view).into_response()
}

#[derive(serde::Deserialize)]
struct TradesQuery {
    base: String,
    quote: String,
    limit: Option<i64>,
}

#[derive(Serialize)]
struct TradeView {
    tx_hash: String,
    log_index: u32,
    maker: String,
    sell_token: String,
    buy_token: String,
    sell_filled: String,
    buy_filled: String,
    order_hash: String,
    block_number: u64,
    block_time: String,
}

impl From<TradeRow> for TradeView {
    fn from(t: TradeRow) -> Self {
        TradeView {
            tx_hash: t.tx_hash.to_string(),
            log_index: t.log_index,
            maker: t.maker.to_string(),
            sell_token: t.sell_token.to_string(),
            buy_token: t.buy_token.to_string(),
            sell_filled: t.sell_filled.to_string(),
            buy_filled: t.buy_filled.to_string(),
            order_hash: t.order_hash.to_string(),
            block_number: t.block_number,
            block_time: t.block_time.to_rfc3339(),
        }
    }
}

async fn get_trades(State(st): State<AppState>, Query(q): Query<TradesQuery>) -> Response {
    let (Ok(base), Ok(quote)) = (Address::from_str(&q.base), Address::from_str(&q.quote)) else {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({ "error": "bad token address" })),
        )
            .into_response();
    };
    let limit = q.limit.unwrap_or(50).clamp(1, 500);
    match db::recent_trades(&st.db, base, quote, limit).await {
        Ok(rows) => Json(rows.into_iter().map(TradeView::from).collect::<Vec<_>>()).into_response(),
        Err(e) => {
            tracing::error!(error = ?e, "get trades");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({ "error": "internal" })),
            )
                .into_response()
        }
    }
}

async fn ws_handler(State(st): State<AppState>, ws: WebSocketUpgrade) -> Response {
    ws.on_upgrade(move |socket| ws_feed(socket, st.trades_tx.subscribe()))
}

async fn ws_feed(mut socket: WebSocket, mut rx: broadcast::Receiver<TradeRow>) {
    loop {
        tokio::select! {
            msg = rx.recv() => match msg {
                Ok(trade) => {
                    let text = serde_json::to_string(&TradeView::from(trade)).unwrap_or_default();
                    if socket.send(Message::Text(text.into())).await.is_err() {
                        break;
                    }
                }
                Err(broadcast::error::RecvError::Lagged(_)) => continue,
                Err(broadcast::error::RecvError::Closed) => break,
            },
            incoming = socket.recv() => match incoming {
                Some(Ok(_)) => {}
                _ => break,
            }
        }
    }
}
