//! Engine configuration, read from the environment.

use alloy_primitives::{Address, B256};
use anyhow::{bail, Context, Result};
use std::net::SocketAddr;
use std::str::FromStr;
use std::time::Duration;

/// How the engine matches orders.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MatchingMode {
    /// Cross each order against the book the moment it arrives (the default).
    Continuous,
    /// Collect orders over a window and clear each pair at one uniform price.
    Batch,
}

#[derive(Debug, Clone)]
pub struct Config {
    pub database_url: String,
    pub rpc_url: String,
    pub settlement: Address,
    pub solver_key: B256,
    pub bind: SocketAddr,
    pub matching_mode: MatchingMode,
    /// Length of a batch window. Only used in batch mode.
    pub batch_interval: Duration,
}

impl Config {
    pub fn from_env() -> Result<Self> {
        let matching_mode = match std::env::var("MATCHING_MODE")
            .unwrap_or_else(|_| "continuous".to_string())
            .to_ascii_lowercase()
            .as_str()
        {
            "continuous" => MatchingMode::Continuous,
            "batch" => MatchingMode::Batch,
            other => bail!("MATCHING_MODE must be continuous or batch, got {other}"),
        };
        let batch_interval = Duration::from_secs(
            std::env::var("BATCH_INTERVAL")
                .ok()
                .and_then(|s| s.parse().ok())
                .unwrap_or(5),
        );
        Ok(Self {
            database_url: env("DATABASE_URL")?,
            rpc_url: std::env::var("RPC_URL")
                .unwrap_or_else(|_| "http://localhost:8545".to_string()),
            settlement: Address::from_str(&env("SETTLEMENT_ADDRESS")?)
                .context("parse SETTLEMENT_ADDRESS")?,
            solver_key: B256::from_str(&env("SOLVER_PRIVATE_KEY")?)
                .context("parse SOLVER_PRIVATE_KEY")?,
            bind: std::env::var("BIND")
                .unwrap_or_else(|_| "0.0.0.0:8080".to_string())
                .parse()
                .context("parse BIND")?,
            matching_mode,
            batch_interval,
        })
    }
}

fn env(key: &str) -> Result<String> {
    std::env::var(key).with_context(|| format!("missing env {key}"))
}
