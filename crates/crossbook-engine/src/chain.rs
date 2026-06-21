//! Ethereum access: the settlement contract bindings, plus the provider and
//! signer the engine uses to read balances, submit settlements, and read events.

use alloy::network::EthereumWallet;
use alloy::primitives::{Address, B256, U256};
use alloy::providers::{DynProvider, Provider, ProviderBuilder};
use alloy::rpc::types::Log;
use alloy::signers::local::PrivateKeySigner;
use alloy::sol;
use anyhow::{Context, Result};

sol! {
    #[sol(rpc)]
    contract Settlement {
        struct Order {
            address maker;
            address sellToken;
            address buyToken;
            uint256 sellAmount;
            uint256 buyAmount;
            uint256 validTo;
            uint256 nonce;
            bool partiallyFillable;
        }
        struct SignedOrder {
            Order order;
            bytes signature;
        }
        struct Fill {
            uint256 orderIndex;
            uint256 sellFilled;
            uint256 buyFilled;
        }

        event Trade(
            address indexed maker,
            address indexed sellToken,
            address indexed buyToken,
            uint256 sellFilled,
            uint256 buyFilled,
            bytes32 orderHash
        );

        function settle(SignedOrder[] orders, Fill[] fills) external;
        function orderHash(Order order) external view returns (bytes32);
        function filledSell(bytes32 orderHash) external view returns (uint256);
    }
}

sol! {
    #[sol(rpc)]
    contract Erc20 {
        function balanceOf(address account) external view returns (uint256);
        function allowance(address owner, address spender) external view returns (uint256);
    }
}

/// A connected client: a provider carrying the solver wallet, plus the deployed
/// settlement address.
#[derive(Clone)]
pub struct Chain {
    provider: DynProvider,
    pub settlement: Address,
}

impl Chain {
    pub async fn connect(rpc_url: &str, solver_key: B256, settlement: Address) -> Result<Self> {
        let signer = PrivateKeySigner::from_bytes(&solver_key).context("parse solver key")?;
        let provider = ProviderBuilder::new()
            .wallet(EthereumWallet::from(signer))
            .connect_http(rpc_url.parse().context("parse rpc url")?)
            .erased();
        Ok(Self {
            provider,
            settlement,
        })
    }

    pub fn provider(&self) -> &DynProvider {
        &self.provider
    }

    pub async fn chain_id(&self) -> Result<u64> {
        self.provider.get_chain_id().await.context("get chain id")
    }

    pub async fn latest_block(&self) -> Result<u64> {
        self.provider
            .get_block_number()
            .await
            .context("get block number")
    }

    /// Timestamp (unix seconds) and hash of a block by number.
    pub async fn block_info(&self, number: u64) -> Result<(u64, B256)> {
        let block = self
            .provider
            .get_block_by_number(alloy::eips::BlockNumberOrTag::Number(number))
            .await
            .context("get block")?
            .context("block not found")?;
        Ok((block.header.timestamp, block.header.hash))
    }

    pub async fn balance_of(&self, token: Address, owner: Address) -> Result<U256> {
        let erc20 = Erc20::new(token, &self.provider);
        erc20.balanceOf(owner).call().await.context("balanceOf")
    }

    pub async fn allowance(&self, token: Address, owner: Address) -> Result<U256> {
        let erc20 = Erc20::new(token, &self.provider);
        erc20
            .allowance(owner, self.settlement)
            .call()
            .await
            .context("allowance")
    }

    /// Submit a settlement batch and wait for the receipt. Returns the tx hash.
    pub async fn settle(
        &self,
        orders: Vec<Settlement::SignedOrder>,
        fills: Vec<Settlement::Fill>,
    ) -> Result<B256> {
        let contract = Settlement::new(self.settlement, &self.provider);
        let pending = contract
            .settle(orders, fills)
            .send()
            .await
            .context("send settle tx")?;
        let receipt = pending
            .get_receipt()
            .await
            .context("await settle receipt")?;
        Ok(receipt.transaction_hash)
    }

    /// All Trade events between two blocks, inclusive, with their logs.
    pub async fn trades_in_range(
        &self,
        from: u64,
        to: u64,
    ) -> Result<Vec<(Settlement::Trade, Log)>> {
        let contract = Settlement::new(self.settlement, &self.provider);
        contract
            .Trade_filter()
            .from_block(from)
            .to_block(to)
            .query()
            .await
            .context("query Trade events")
    }
}
