//! Engine configuration, read from the environment.

use alloy_primitives::{Address, B256};
use anyhow::{Context, Result};
use std::net::SocketAddr;
use std::str::FromStr;

#[derive(Debug, Clone)]
pub struct Config {
    pub database_url: String,
    pub rpc_url: String,
    pub settlement: Address,
    pub solver_key: B256,
    pub bind: SocketAddr,
}

impl Config {
    pub fn from_env() -> Result<Self> {
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
        })
    }
}

fn env(key: &str) -> Result<String> {
    std::env::var(key).with_context(|| format!("missing env {key}"))
}
