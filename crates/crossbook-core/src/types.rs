//! Domain types for the matching core.
//!
//! `partially_fillable` (signed, also enforced on chain) is the single source of
//! truth for fillability, and `valid_to` for expiry. There is deliberately no
//! stored `Side` or `TimeInForce`: a side is derived from the token pair, and
//! fill or kill is just `!partially_fillable`.

use crate::error::CoreError;
use alloy_primitives::{Address, U256};

/// EIP-712 digest of an order. Also the order id.
pub type OrderHash = [u8; 32];

/// A signed limit order in the CoW style sell amount / buy amount model.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Order {
    pub maker: Address,
    pub sell_token: Address,
    pub buy_token: Address,
    pub sell_amount: U256,
    pub buy_amount: U256,
    pub valid_to: u64,
    pub nonce: U256,
    pub partially_fillable: bool,
}

/// An order that has been signature verified and admitted to the book.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OpenOrder {
    pub order: Order,
    pub hash: OrderHash,
    /// Monotonic arrival order, assigned by the engine. The time priority tiebreaker.
    pub arrival_seq: u64,
    /// `sell_amount` minus the amount already filled.
    pub remaining_sell: U256,
}

impl OpenOrder {
    /// Build an admitted order, rejecting degenerate inputs so the matcher can
    /// assume `sell_amount > 0`, `buy_amount > 0`, and distinct tokens.
    pub fn new(order: Order, hash: OrderHash, arrival_seq: u64) -> Result<Self, CoreError> {
        if order.sell_amount.is_zero() || order.buy_amount.is_zero() {
            return Err(CoreError::ZeroAmount);
        }
        if order.sell_token == order.buy_token {
            return Err(CoreError::SameToken);
        }
        let remaining_sell = order.sell_amount;
        Ok(Self {
            order,
            hash,
            arrival_seq,
            remaining_sell,
        })
    }
}

/// One matched pair, from the resting maker's perspective. The taker's amounts
/// are the mirror: taker pays `buy_filled` of the maker's buy token and receives
/// `sell_filled` of the maker's sell token.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Fill {
    pub maker_hash: OrderHash,
    pub taker_hash: OrderHash,
    /// Amount of the maker's sell token transferred out of the maker.
    pub sell_filled: U256,
    /// Amount of the maker's buy token transferred in to the maker.
    pub buy_filled: U256,
}

/// How a submitted order resolved.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SubmitOutcome {
    FullyFilled,
    PartiallyFilled { remaining_sell: U256 },
    Resting,
    Killed,
}

/// The unit handed to the settlement layer.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Batch {
    pub orders: Vec<OpenOrder>,
    pub fills: Vec<Fill>,
}
