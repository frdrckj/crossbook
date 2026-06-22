//! End to end against Anvil and Postgres: deploy the contracts and two tokens,
//! start the engine, post signed orders over HTTP, and assert they settle on
//! chain, land in Postgres, and are broadcast to subscribers. One test exercises
//! the continuous path, the other the batch auction path.
//!
//! Gated behind the `e2e` feature because it reads compiled contract artifacts.
//! Requires a running Anvil (RPC_URL) and Postgres (DATABASE_URL). Run it with
//! `just e2e`.
#![cfg(feature = "e2e")]

use alloy::network::EthereumWallet;
use alloy::primitives::{Address, Bytes, U256};
use alloy::providers::{Provider, ProviderBuilder};
use alloy::signers::local::PrivateKeySigner;
use alloy::signers::SignerSync;
use alloy::sol;
use crossbook_core::eip712;
use crossbook_core::types::Order;
use crossbook_engine::config::MatchingMode;
use crossbook_engine::ingest::OrderPayload;
use crossbook_engine::{api, batch, chain::Chain, db, engine_task, indexer};
use std::collections::HashMap;
use std::sync::atomic::AtomicU64;
use std::sync::{Arc, Mutex};
use std::time::Duration;

sol!(
    #[sol(rpc)]
    CrossbookSettlement,
    "../../contracts/out/CrossbookSettlement.sol/CrossbookSettlement.json"
);
sol!(
    #[sol(rpc)]
    MockERC20,
    "../../contracts/out/Mocks.sol/MockERC20.json"
);

// Anvil's well known dev keys.
const SOLVER: &str = "0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80";
const MAKER_A: &str = "0x59c6995e998f97a5a0044966f0945389dc9e86dae88c7a8412f4603b6b78690d";
const MAKER_B: &str = "0x5de4111afa1a4b94908f83103eb1f1706367c2e68ca870fc3fb9a804cdab365a";

const AMT: u64 = 1_000_000;
const VALID_TO: u64 = 4_000_000_000;

fn provider_for(key: &str, rpc: &str) -> impl Provider + Clone {
    let signer: PrivateKeySigner = key.parse().unwrap();
    ProviderBuilder::new()
        .wallet(EthereumWallet::from(signer))
        .connect_http(rpc.parse().unwrap())
}

fn sign_payload(key: &str, order: &Order, domain: &alloy_sol_types::Eip712Domain) -> OrderPayload {
    let signer: PrivateKeySigner = key.parse().unwrap();
    let digest = eip712::signing_hash(order, domain);
    let sig = signer.sign_hash_sync(&digest).unwrap();
    OrderPayload {
        maker: order.maker,
        sell_token: order.sell_token,
        buy_token: order.buy_token,
        sell_amount: order.sell_amount,
        buy_amount: order.buy_amount,
        valid_to: order.valid_to,
        nonce: order.nonce,
        partially_fillable: order.partially_fillable,
        signature: Bytes::from(sig.as_bytes().to_vec()),
    }
}

/// What `deploy_and_fund` hands back: the deployed addresses and the matching
/// EIP-712 domain. Providers are cheap, so callers rebuild them from keys.
struct Deployed {
    ta: Address,
    tb: Address,
    settlement: Address,
    domain: alloy_sol_types::Eip712Domain,
}

/// Deploy two tokens and the settlement contract, fund and approve both makers,
/// and clear the engine tables. Shared by both flows.
async fn deploy_and_fund(rpc: &str, database_url: &str) -> Deployed {
    let solver_signer: PrivateKeySigner = SOLVER.parse().unwrap();
    let maker_a: PrivateKeySigner = MAKER_A.parse().unwrap();
    let maker_b: PrivateKeySigner = MAKER_B.parse().unwrap();

    let deployer = provider_for(SOLVER, rpc);
    let token_a = MockERC20::deploy(&deployer, "TokenA".into(), "A".into())
        .await
        .unwrap();
    let token_b = MockERC20::deploy(&deployer, "TokenB".into(), "B".into())
        .await
        .unwrap();
    let settlement = CrossbookSettlement::deploy(&deployer, solver_signer.address())
        .await
        .unwrap();
    let ta = *token_a.address();
    let tb = *token_b.address();
    let settlement_addr = *settlement.address();

    let amt = U256::from(AMT);
    token_a
        .mint(maker_a.address(), amt)
        .send()
        .await
        .unwrap()
        .get_receipt()
        .await
        .unwrap();
    token_b
        .mint(maker_b.address(), amt)
        .send()
        .await
        .unwrap()
        .get_receipt()
        .await
        .unwrap();
    MockERC20::new(ta, &provider_for(MAKER_A, rpc))
        .approve(settlement_addr, U256::MAX)
        .send()
        .await
        .unwrap()
        .get_receipt()
        .await
        .unwrap();
    MockERC20::new(tb, &provider_for(MAKER_B, rpc))
        .approve(settlement_addr, U256::MAX)
        .send()
        .await
        .unwrap()
        .get_receipt()
        .await
        .unwrap();

    let pool = db::connect(database_url).await.unwrap();
    for table in ["trades", "orders", "indexer_cursor"] {
        sqlx::query(&format!("DELETE FROM {table}"))
            .execute(&pool)
            .await
            .unwrap();
    }

    let chain_id = deployer.get_chain_id().await.unwrap();
    let domain = eip712::crossbook_domain(chain_id, settlement_addr);
    Deployed {
        ta,
        tb,
        settlement: settlement_addr,
        domain,
    }
}

/// The two offsetting orders both flows use: A sells ta for tb, B sells tb for
/// ta, equal amounts, so they cross one to one.
fn offsetting_orders(d: &Deployed) -> (OrderPayload, OrderPayload) {
    let maker_a: PrivateKeySigner = MAKER_A.parse().unwrap();
    let maker_b: PrivateKeySigner = MAKER_B.parse().unwrap();
    let amt = U256::from(AMT);
    let order_a = Order {
        maker: maker_a.address(),
        sell_token: d.ta,
        buy_token: d.tb,
        sell_amount: amt,
        buy_amount: amt,
        valid_to: VALID_TO,
        nonce: U256::from(1u64),
        partially_fillable: true,
    };
    let order_b = Order {
        maker: maker_b.address(),
        sell_token: d.tb,
        buy_token: d.ta,
        sell_amount: amt,
        buy_amount: amt,
        valid_to: VALID_TO,
        nonce: U256::from(1u64),
        partially_fillable: true,
    };
    (
        sign_payload(MAKER_A, &order_a, &d.domain),
        sign_payload(MAKER_B, &order_b, &d.domain),
    )
}

async fn post_order(http: &reqwest::Client, base: &str, payload: &OrderPayload, label: &str) {
    let r = http
        .post(format!("{base}/orders"))
        .json(payload)
        .send()
        .await
        .unwrap();
    assert!(
        r.status().is_success(),
        "post {label} failed: {}",
        r.text().await.unwrap()
    );
}

async fn poll_trades_indexed(http: &reqwest::Client, base: &str, d: &Deployed) -> bool {
    for _ in 0..30 {
        let body: serde_json::Value = http
            .get(format!(
                "{base}/trades?base={}&quote={}&limit=10",
                d.ta, d.tb
            ))
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        if body.as_array().map(|a| !a.is_empty()).unwrap_or(false) {
            return true;
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
    false
}

#[tokio::test]
async fn continuous_flow_settles_indexes_and_broadcasts() {
    let (Ok(rpc), Ok(database_url)) = (std::env::var("RPC_URL"), std::env::var("DATABASE_URL"))
    else {
        eprintln!("skipping e2e: set RPC_URL and DATABASE_URL (just dev)");
        return;
    };
    let d = deploy_and_fund(&rpc, &database_url).await;
    let pool = db::connect(&database_url).await.unwrap();
    let chain = Arc::new(
        Chain::connect(&rpc, SOLVER.parse().unwrap(), d.settlement)
            .await
            .unwrap(),
    );
    let chain_id = chain.chain_id().await.unwrap();

    let engine = engine_task::spawn(256, MatchingMode::Continuous);
    let (trades_tx, mut ws_rx) = {
        let (tx, _) = tokio::sync::broadcast::channel(256);
        let rx = tx.subscribe();
        (tx, rx)
    };
    let metrics = Arc::new(
        metrics_exporter_prometheus::PrometheusBuilder::new()
            .build_recorder()
            .handle(),
    );
    tokio::spawn(indexer::run(chain.clone(), pool.clone(), trades_tx.clone()));

    let state = api::AppState {
        engine,
        chain,
        db: pool.clone(),
        domain: Arc::new(d.domain.clone()),
        admitted: Arc::new(Mutex::new(HashMap::new())),
        seq: Arc::new(AtomicU64::new(0)),
        trades_tx,
        metrics,
        demo: api::DemoConfig {
            chain_id,
            settlement: d.settlement.to_string(),
            token_a: Some(d.ta.to_string()),
            token_b: Some(d.tb.to_string()),
        },
        batch: Arc::new(Mutex::new(api::BatchState::default())),
    };
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, api::router(state)).await.unwrap();
    });

    let (payload_a, payload_b) = offsetting_orders(&d);
    let http = reqwest::Client::new();
    let base = format!("http://{addr}");
    post_order(&http, &base, &payload_a, "A").await;
    post_order(&http, &base, &payload_b, "B").await;

    assert!(
        poll_trades_indexed(&http, &base, &d).await,
        "trade was not indexed within the timeout"
    );

    let broadcast = tokio::time::timeout(Duration::from_secs(5), ws_rx.recv()).await;
    assert!(
        broadcast.is_ok() && broadcast.unwrap().is_ok(),
        "no trade broadcast"
    );

    let maker_a: PrivateKeySigner = MAKER_A.parse().unwrap();
    let bal = MockERC20::new(d.tb, &provider_for(SOLVER, &rpc))
        .balanceOf(maker_a.address())
        .call()
        .await
        .unwrap();
    assert_eq!(bal, U256::from(AMT), "maker A should have received token B");
}

#[tokio::test]
async fn batch_flow_settles_at_a_uniform_price() {
    let (Ok(rpc), Ok(database_url)) = (std::env::var("RPC_URL"), std::env::var("DATABASE_URL"))
    else {
        eprintln!("skipping e2e: set RPC_URL and DATABASE_URL (just dev)");
        return;
    };
    let d = deploy_and_fund(&rpc, &database_url).await;
    let pool = db::connect(&database_url).await.unwrap();
    let chain = Arc::new(
        Chain::connect(&rpc, SOLVER.parse().unwrap(), d.settlement)
            .await
            .unwrap(),
    );
    let chain_id = chain.chain_id().await.unwrap();

    let engine = engine_task::spawn(256, MatchingMode::Batch);
    let (trades_tx, _) = tokio::sync::broadcast::channel(256);
    let metrics = Arc::new(
        metrics_exporter_prometheus::PrometheusBuilder::new()
            .build_recorder()
            .handle(),
    );
    let admitted = Arc::new(Mutex::new(HashMap::new()));
    let batch_state = Arc::new(Mutex::new(api::BatchState {
        mode: "batch".to_string(),
        interval_secs: 1,
        ..Default::default()
    }));

    tokio::spawn(indexer::run(chain.clone(), pool.clone(), trades_tx.clone()));
    tokio::spawn(batch::run_window(
        engine.clone(),
        chain.clone(),
        admitted.clone(),
        batch_state.clone(),
        Duration::from_secs(1),
    ));

    let state = api::AppState {
        engine,
        chain: chain.clone(),
        db: pool.clone(),
        domain: Arc::new(d.domain.clone()),
        admitted,
        seq: Arc::new(AtomicU64::new(0)),
        trades_tx,
        metrics,
        demo: api::DemoConfig {
            chain_id,
            settlement: d.settlement.to_string(),
            token_a: Some(d.ta.to_string()),
            token_b: Some(d.tb.to_string()),
        },
        batch: batch_state,
    };
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, api::router(state)).await.unwrap();
    });

    let (payload_a, payload_b) = offsetting_orders(&d);
    let http = reqwest::Client::new();
    let base = format!("http://{addr}");
    post_order(&http, &base, &payload_a, "A").await;
    post_order(&http, &base, &payload_b, "B").await;

    // The window driver clears and settles within a couple of intervals.
    assert!(
        poll_trades_indexed(&http, &base, &d).await,
        "batch trade was not indexed within the timeout"
    );

    // Exactly one pair cleared, so exactly one BatchSettled event, at a one to
    // one uniform price for the full amount.
    let reader = provider_for(SOLVER, &rpc);
    let settlement = CrossbookSettlement::new(d.settlement, &reader);
    let events = settlement
        .BatchSettled_filter()
        .from_block(0)
        .query()
        .await
        .unwrap();
    assert_eq!(events.len(), 1, "expected exactly one BatchSettled event");
    let (ev, _) = &events[0];
    assert_eq!(ev.clearingNum, ev.clearingDen, "price should be one to one");
    assert_eq!(ev.volumeBase, U256::from(AMT), "full amount should clear");

    // The dashboard endpoint reflects the clearing.
    let batch_view: serde_json::Value = http
        .get(format!("{base}/batch"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(batch_view["mode"], "batch");
    assert!(
        batch_view["last_results"]
            .as_array()
            .map(|a| !a.is_empty())
            .unwrap_or(false),
        "dashboard should show the last batch results"
    );

    // On chain, maker A received token B at the uniform price.
    let maker_a: PrivateKeySigner = MAKER_A.parse().unwrap();
    let bal = MockERC20::new(d.tb, &provider_for(SOLVER, &rpc))
        .balanceOf(maker_a.address())
        .call()
        .await
        .unwrap();
    assert_eq!(bal, U256::from(AMT), "maker A should have received token B");
}
