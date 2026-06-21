//! Crossbook engine: the Tokio service that ingests signed orders, drives the
//! single writer matching core, submits settlement batches, and indexes events.

use anyhow::Result;

#[tokio::main]
async fn main() -> Result<()> {
    println!("crossbook-engine: not implemented yet");
    Ok(())
}
