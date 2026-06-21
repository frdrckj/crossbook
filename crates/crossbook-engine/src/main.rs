//! Crossbook engine — the Tokio service that ingests signed orders, drives the
//! single-writer matching core, submits settlement batches, and indexes events.
//!
//! Milestone status: **M0 scaffold**.

use anyhow::Result;

#[tokio::main]
async fn main() -> Result<()> {
    println!("crossbook-engine: M0 scaffold — service lands in M4");
    Ok(())
}
