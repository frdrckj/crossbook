//! Crossbook engine entrypoint. Wires the database, the chain client, the single
//! writer matching task, the indexer, and the axum API together.

use anyhow::{Context, Result};
use crossbook_core::eip712;
use crossbook_engine::{api, chain::Chain, config::Config, db, engine_task, indexer};
use metrics_exporter_prometheus::PrometheusBuilder;
use std::collections::HashMap;
use std::sync::atomic::AtomicU64;
use std::sync::{Arc, Mutex};
use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()))
        .init();

    let cfg = Config::from_env()?;
    let metrics = Arc::new(
        PrometheusBuilder::new()
            .install_recorder()
            .context("install metrics recorder")?,
    );

    let db = db::connect(&cfg.database_url).await?;
    let chain = Chain::connect(&cfg.rpc_url, cfg.solver_key, cfg.settlement).await?;
    let chain_id = chain.chain_id().await?;
    let domain = eip712::crossbook_domain(chain_id, cfg.settlement);
    let chain = Arc::new(chain);

    let engine = engine_task::spawn(1024);
    let (trades_tx, _) = tokio::sync::broadcast::channel(1024);

    tokio::spawn(indexer::run(chain.clone(), db.clone(), trades_tx.clone()));

    let state = api::AppState {
        engine,
        chain,
        db,
        domain: Arc::new(domain),
        admitted: Arc::new(Mutex::new(HashMap::new())),
        seq: Arc::new(AtomicU64::new(0)),
        trades_tx,
        metrics,
    };

    let listener = tokio::net::TcpListener::bind(cfg.bind)
        .await
        .context("bind")?;
    tracing::info!(addr = %cfg.bind, chain_id, "crossbook engine listening");
    axum::serve(listener, api::router(state))
        .await
        .context("serve")?;
    Ok(())
}
