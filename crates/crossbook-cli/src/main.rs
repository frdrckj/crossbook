//! Crossbook client. Approve the settlement contract, sign and submit orders,
//! and query the book, trades, and order status.

use alloy::network::EthereumWallet;
use alloy::primitives::{Address, Bytes, U256};
use alloy::providers::{Provider, ProviderBuilder};
use alloy::signers::local::PrivateKeySigner;
use alloy::signers::SignerSync;
use alloy::sol;
use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use crossbook_core::eip712;
use crossbook_core::types::Order;
use serde::Serialize;
use std::time::{SystemTime, UNIX_EPOCH};

sol! {
    #[sol(rpc)]
    contract Erc20 {
        function approve(address spender, uint256 amount) external returns (bool);
    }
}

#[derive(Parser)]
#[command(name = "crossbook", about = "Crossbook client")]
struct Cli {
    #[arg(
        long,
        env = "CROSSBOOK_ENGINE",
        default_value = "http://localhost:8080"
    )]
    engine: String,
    #[arg(long, env = "RPC_URL", default_value = "http://localhost:8545")]
    rpc: String,
    #[arg(long, env = "SETTLEMENT_ADDRESS")]
    settlement: Option<Address>,
    #[arg(long, env = "PRIVATE_KEY")]
    key: Option<String>,
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Approve the settlement contract to pull a token.
    Approve {
        #[arg(long)]
        token: Address,
        #[arg(long)]
        amount: Option<U256>,
    },
    /// Sign an order and submit it to the engine.
    Submit {
        #[arg(long)]
        sell_token: Address,
        #[arg(long)]
        sell_amount: U256,
        #[arg(long)]
        buy_token: Address,
        #[arg(long)]
        buy_amount: U256,
        #[arg(long)]
        valid_to: Option<u64>,
        #[arg(long)]
        nonce: Option<U256>,
        #[arg(long)]
        partial: bool,
    },
    /// Show the in memory book for a token pair.
    Book {
        #[arg(long)]
        base: Address,
        #[arg(long)]
        quote: Address,
    },
    /// Show recent trades for a token pair.
    Trades {
        #[arg(long)]
        base: Address,
        #[arg(long)]
        quote: Address,
        #[arg(long, default_value_t = 50)]
        limit: i64,
    },
    /// Look up an order by hash.
    Status {
        #[arg(long)]
        hash: String,
    },
}

#[derive(Serialize)]
struct OrderPayload {
    maker: Address,
    sell_token: Address,
    buy_token: Address,
    sell_amount: U256,
    buy_amount: U256,
    valid_to: u64,
    nonce: U256,
    partially_fillable: bool,
    signature: Bytes,
}

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

async fn get_json(http: &reqwest::Client, url: String) -> Result<()> {
    let body: serde_json::Value = http.get(url).send().await?.json().await?;
    println!("{}", serde_json::to_string_pretty(&body)?);
    Ok(())
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    let http = reqwest::Client::new();

    match cli.cmd {
        Cmd::Book { base, quote } => {
            get_json(&http, format!("{}/book/{base}/{quote}", cli.engine)).await?;
        }
        Cmd::Trades { base, quote, limit } => {
            get_json(
                &http,
                format!(
                    "{}/trades?base={base}&quote={quote}&limit={limit}",
                    cli.engine
                ),
            )
            .await?;
        }
        Cmd::Status { hash } => {
            get_json(&http, format!("{}/orders/{hash}", cli.engine)).await?;
        }
        Cmd::Approve { token, amount } => {
            let settlement = cli.settlement.context("--settlement is required")?;
            let signer: PrivateKeySigner = cli
                .key
                .context("--key is required")?
                .parse()
                .context("parse key")?;
            let provider = ProviderBuilder::new()
                .wallet(EthereumWallet::from(signer))
                .connect_http(cli.rpc.parse()?);
            let erc20 = Erc20::new(token, &provider);
            let receipt = erc20
                .approve(settlement, amount.unwrap_or(U256::MAX))
                .send()
                .await?
                .get_receipt()
                .await?;
            println!("approved, tx {}", receipt.transaction_hash);
        }
        Cmd::Submit {
            sell_token,
            sell_amount,
            buy_token,
            buy_amount,
            valid_to,
            nonce,
            partial,
        } => {
            let settlement = cli.settlement.context("--settlement is required")?;
            let signer: PrivateKeySigner = cli
                .key
                .context("--key is required")?
                .parse()
                .context("parse key")?;
            let provider = ProviderBuilder::new().connect_http(cli.rpc.parse()?);
            let chain_id = provider.get_chain_id().await?;

            let order = Order {
                maker: signer.address(),
                sell_token,
                buy_token,
                sell_amount,
                buy_amount,
                valid_to: valid_to.unwrap_or_else(|| now() + 3600),
                nonce: nonce.unwrap_or_else(|| U256::from(now())),
                partially_fillable: partial,
            };
            let domain = eip712::crossbook_domain(chain_id, settlement);
            let digest = eip712::signing_hash(&order, &domain);
            let sig = signer.sign_hash_sync(&digest)?;

            let payload = OrderPayload {
                maker: order.maker,
                sell_token: order.sell_token,
                buy_token: order.buy_token,
                sell_amount: order.sell_amount,
                buy_amount: order.buy_amount,
                valid_to: order.valid_to,
                nonce: order.nonce,
                partially_fillable: order.partially_fillable,
                signature: Bytes::from(sig.as_bytes().to_vec()),
            };
            let resp = http
                .post(format!("{}/orders", cli.engine))
                .json(&payload)
                .send()
                .await?;
            println!("{}\n{}", resp.status(), resp.text().await?);
        }
    }
    Ok(())
}
