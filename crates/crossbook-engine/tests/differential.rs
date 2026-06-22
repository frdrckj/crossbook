//! Cross implementation differential fuzzing.
//!
//! The Rust core decides what crosses and at what amounts; the Solidity contract
//! independently re-verifies every signature, limit, uniform price, and net flow.
//! If they ever disagree, a batch the core produced would revert on chain. This
//! test drives many random batches through both: it generates random partially
//! fillable orders over a small token set, clears them with the real `run_auction`
//! and `find_ring`, builds the exact settlement calldata the engine would, and
//! submits it to a live contract on Anvil. Every batch the core emits must settle,
//! and the contract must hold zero inventory after each one.
//!
//! Gated behind the `e2e` feature; needs a running Anvil (RPC_URL). Run with
//! `just e2e` or directly with the feature enabled.
#![cfg(feature = "e2e")]

use alloy::network::EthereumWallet;
use alloy::primitives::U256;
use alloy::providers::{Provider, ProviderBuilder};
use alloy::signers::local::PrivateKeySigner;
use alloy::signers::SignerSync;
use alloy::sol;
use crossbook_core::auction::{run_auction, AuctionFill};
use crossbook_core::eip712;
use crossbook_core::ring::find_ring;
use crossbook_core::types::{OpenOrder, Order};
use crossbook_engine::chain::Chain;
use crossbook_engine::settle::{self, AdmittedOrder};
use std::collections::HashMap;

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

const SOLVER: &str = "0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80";
const MAKERS: [&str; 3] = [
    "0x59c6995e998f97a5a0044966f0945389dc9e86dae88c7a8412f4603b6b78690d",
    "0x5de4111afa1a4b94908f83103eb1f1706367c2e68ca870fc3fb9a804cdab365a",
    "0x7c852118294e51e653712a81e05800f419141751be58f605c371e15141b007a6",
];
const VALID_TO: u64 = 4_000_000_000;
const ITERATIONS: u64 = 30;

/// A small deterministic PRNG, so a failure reproduces from the same seed.
struct Lcg(u64);
impl Lcg {
    fn next(&mut self) -> u64 {
        self.0 = self
            .0
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        self.0 >> 16
    }
    fn range(&mut self, lo: u64, hi: u64) -> u64 {
        lo + self.next() % (hi - lo)
    }
}

fn provider_for(key: &str, rpc: &str) -> impl Provider + Clone {
    let signer: PrivateKeySigner = key.parse().unwrap();
    ProviderBuilder::new()
        .wallet(EthereumWallet::from(signer))
        .connect_http(rpc.parse().unwrap())
}

fn apply(orders: &mut [OpenOrder], fills: &[AuctionFill]) {
    for f in fills {
        if let Some(o) = orders.iter_mut().find(|o| o.hash == f.order_hash) {
            o.remaining_sell = o.remaining_sell.saturating_sub(f.sell_filled);
        }
    }
}

#[tokio::test]
async fn core_output_always_settles_on_chain() {
    let Ok(rpc) = std::env::var("RPC_URL") else {
        eprintln!("skipping differential: set RPC_URL (just dev)");
        return;
    };

    let solver: PrivateKeySigner = SOLVER.parse().unwrap();
    let makers: Vec<PrivateKeySigner> = MAKERS.iter().map(|k| k.parse().unwrap()).collect();
    let deployer = provider_for(SOLVER, &rpc);

    // Three tokens and the settlement contract.
    let mut tokens = Vec::new();
    for sym in ["A", "B", "C"] {
        let t = MockERC20::deploy(&deployer, sym.into(), sym.into())
            .await
            .unwrap();
        tokens.push(*t.address());
    }
    let settlement = CrossbookSettlement::deploy(&deployer, solver.address())
        .await
        .unwrap();
    let settlement_addr = *settlement.address();

    // Fund every maker with a large balance of every token and approve settlement.
    let big = U256::from(10u128.pow(30));
    for m in &makers {
        for &tok in &tokens {
            MockERC20::new(tok, &deployer)
                .mint(m.address(), big)
                .send()
                .await
                .unwrap()
                .get_receipt()
                .await
                .unwrap();
        }
    }
    for (i, key) in MAKERS.iter().enumerate() {
        let p = provider_for(key, &rpc);
        let _ = i;
        for &tok in &tokens {
            MockERC20::new(tok, &p)
                .approve(settlement_addr, U256::MAX)
                .send()
                .await
                .unwrap()
                .get_receipt()
                .await
                .unwrap();
        }
    }

    let chain_id = deployer.get_chain_id().await.unwrap();
    let domain = eip712::crossbook_domain(chain_id, settlement_addr);
    let chain = Chain::connect(&rpc, SOLVER.parse().unwrap(), settlement_addr)
        .await
        .unwrap();

    let mut rng = Lcg(0x1234_5678_9abc_def0);
    let mut nonce: u64 = 0;
    let mut settled_pairs = 0u64;
    let mut settled_rings = 0u64;

    for iter in 0..ITERATIONS {
        let k = rng.range(4, 10);
        let mut orders = Vec::new();
        let mut admitted: HashMap<[u8; 32], AdmittedOrder> = HashMap::new();

        for seq in 0..k {
            let mi = (rng.range(0, 3)) as usize;
            let st = (rng.range(0, 3)) as usize;
            let bt = (st + 1 + (rng.range(0, 2)) as usize) % 3; // distinct token
            nonce += 1;
            // Limit = x/100 with x in [85, 115], an exact small denominator ratio
            // near 1.0, and amounts far larger than any lot, so opposite orders on
            // a pair and three token cycles cross and clear often, across a spread
            // of distinct prices.
            let q = rng.range(1_000_000, 100_000_000);
            let x = rng.range(85, 116);
            let order = Order {
                maker: makers[mi].address(),
                sell_token: tokens[st],
                buy_token: tokens[bt],
                sell_amount: U256::from(q * 100),
                buy_amount: U256::from(q * x),
                valid_to: VALID_TO,
                nonce: U256::from(nonce),
                partially_fillable: true,
            };
            let digest = eip712::signing_hash(&order, &domain);
            let sig = makers[mi].sign_hash_sync(&digest).unwrap();
            let hash: [u8; 32] = digest.into();
            admitted.insert(
                hash,
                AdmittedOrder {
                    order: order.clone(),
                    signature: sig.as_bytes().to_vec(),
                },
            );
            orders.push(OpenOrder::new(order, hash, seq).unwrap());
        }

        // Clear exactly as the engine's window does: pairs first, then rings on
        // what remains.
        let pairs = run_auction(&orders);
        for r in &pairs {
            apply(&mut orders, &r.fills);
        }
        let mut rings = Vec::new();
        for _ in 0..orders.len() {
            match find_ring(&orders) {
                Some(r) => {
                    apply(&mut orders, &r.fills);
                    rings.push(r);
                }
                None => break,
            }
        }

        // Every batch the core produced must settle. A revert here is a core or
        // contract disagreement, the exact bug this test exists to catch.
        if !pairs.is_empty() {
            let (signed, fills, prices) = settle::to_batch_settlement(&pairs, &admitted).unwrap();
            chain
                .settle_batch(signed, fills, prices)
                .await
                .unwrap_or_else(|e| panic!("iter {iter}: core batch rejected on chain: {e}"));
            settled_pairs += pairs.len() as u64;
        }
        if !rings.is_empty() {
            let (signed, fills) = settle::to_ring_settlement(&rings, &admitted).unwrap();
            chain
                .settle(signed, fills)
                .await
                .unwrap_or_else(|e| panic!("iter {iter}: core ring rejected on chain: {e}"));
            settled_rings += rings.len() as u64;
        }

        // Non custody holds across every random settlement.
        for &tok in &tokens {
            let bal: U256 = MockERC20::new(tok, &deployer)
                .balanceOf(settlement_addr)
                .call()
                .await
                .unwrap();
            assert_eq!(
                bal,
                U256::ZERO,
                "iter {iter}: contract held inventory of {tok}"
            );
        }
    }

    // The fuzz is only meaningful if it actually cleared something.
    assert!(
        settled_pairs + settled_rings > 0,
        "differential fuzz cleared nothing; check the generator"
    );
    eprintln!("differential: settled {settled_pairs} pair clearings and {settled_rings} rings over {ITERATIONS} batches");
}
