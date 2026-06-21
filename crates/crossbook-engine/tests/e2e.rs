//! End to end: deploy the contracts and two tokens on Anvil, start the engine,
//! post two signed orders that cross over HTTP, and assert the batch settles on
//! chain, lands in Postgres, and is broadcast to subscribers.
//!
//! Gated behind the `e2e` feature because it reads compiled contract artifacts.
//! Requires a running Anvil (RPC_URL) and Postgres (DATABASE_URL). Run it with
//! `just e2e`.
#![cfg(feature = "e2e")]

use alloy::network::EthereumWallet;
use alloy::primitives::{Bytes, U256};
use alloy::providers::{Provider, ProviderBuilder};
use alloy::signers::local::PrivateKeySigner;
use alloy::signers::SignerSync;
use alloy::sol;
use crossbook_core::eip712;
use crossbook_core::types::Order;
use crossbook_engine::ingest::OrderPayload;
use crossbook_engine::{api, chain::Chain, db, engine_task, indexer};
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

#[tokio::test]
async fn full_flow_settles_indexes_and_broadcasts() {
    let (Ok(rpc), Ok(database_url)) = (std::env::var("RPC_URL"), std::env::var("DATABASE_URL"))
    else {
        eprintln!("skipping e2e: set RPC_URL and DATABASE_URL (just dev)");
        return;
    };

    let solver_signer: PrivateKeySigner = SOLVER.parse().unwrap();
    let maker_a: PrivateKeySigner = MAKER_A.parse().unwrap();
    let maker_b: PrivateKeySigner = MAKER_B.parse().unwrap();

    // Deploy two tokens and the settlement contract.
    let deployer = provider_for(SOLVER, &rpc);
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

    // Fund and approve both makers.
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
    MockERC20::new(ta, &provider_for(MAKER_A, &rpc))
        .approve(settlement_addr, U256::MAX)
        .send()
        .await
        .unwrap()
        .get_receipt()
        .await
        .unwrap();
    MockERC20::new(tb, &provider_for(MAKER_B, &rpc))
        .approve(settlement_addr, U256::MAX)
        .send()
        .await
        .unwrap()
        .get_receipt()
        .await
        .unwrap();

    // Engine state.
    let pool = db::connect(&database_url).await.unwrap();
    for table in ["trades", "orders", "indexer_cursor"] {
        sqlx::query(&format!("DELETE FROM {table}"))
            .execute(&pool)
            .await
            .unwrap();
    }
    let chain = Arc::new(
        Chain::connect(&rpc, SOLVER.parse().unwrap(), settlement_addr)
            .await
            .unwrap(),
    );
    let chain_id = chain.chain_id().await.unwrap();
    let domain = eip712::crossbook_domain(chain_id, settlement_addr);
    let engine = engine_task::spawn(256);
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
        domain: Arc::new(domain.clone()),
        admitted: Arc::new(Mutex::new(HashMap::new())),
        seq: Arc::new(AtomicU64::new(0)),
        trades_tx,
        metrics,
        demo: api::DemoConfig {
            chain_id,
            settlement: settlement_addr.to_string(),
            token_a: Some(ta.to_string()),
            token_b: Some(tb.to_string()),
        },
    };
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, api::router(state)).await.unwrap();
    });

    // Two crossing orders.
    let order_a = Order {
        maker: maker_a.address(),
        sell_token: ta,
        buy_token: tb,
        sell_amount: amt,
        buy_amount: amt,
        valid_to: VALID_TO,
        nonce: U256::from(1u64),
        partially_fillable: true,
    };
    let order_b = Order {
        maker: maker_b.address(),
        sell_token: tb,
        buy_token: ta,
        sell_amount: amt,
        buy_amount: amt,
        valid_to: VALID_TO,
        nonce: U256::from(1u64),
        partially_fillable: true,
    };
    let payload_a = sign_payload(MAKER_A, &order_a, &domain);
    let payload_b = sign_payload(MAKER_B, &order_b, &domain);

    let http = reqwest::Client::new();
    let base = format!("http://{addr}");

    let r1 = http
        .post(format!("{base}/orders"))
        .json(&payload_a)
        .send()
        .await
        .unwrap();
    assert!(
        r1.status().is_success(),
        "post A failed: {}",
        r1.text().await.unwrap()
    );

    let r2 = http
        .post(format!("{base}/orders"))
        .json(&payload_b)
        .send()
        .await
        .unwrap();
    assert!(
        r2.status().is_success(),
        "post B failed: {}",
        r2.text().await.unwrap()
    );

    // Poll the trades endpoint until the settlement is indexed.
    let mut indexed = false;
    for _ in 0..30 {
        let body: serde_json::Value = http
            .get(format!("{base}/trades?base={ta}&quote={tb}&limit=10"))
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        if body.as_array().map(|a| !a.is_empty()).unwrap_or(false) {
            indexed = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
    assert!(indexed, "trade was not indexed within the timeout");

    // The trade was broadcast to subscribers.
    let broadcast = tokio::time::timeout(Duration::from_secs(5), ws_rx.recv()).await;
    assert!(
        broadcast.is_ok() && broadcast.unwrap().is_ok(),
        "no trade broadcast"
    );

    // On chain, maker A received token B.
    let bal = MockERC20::new(tb, &deployer)
        .balanceOf(maker_a.address())
        .call()
        .await
        .unwrap();
    assert_eq!(bal, amt, "maker A should have received token B");
}
