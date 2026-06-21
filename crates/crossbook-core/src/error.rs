//! Errors for the pure core. Admitted orders are validated once at construction
//! so the matcher can assume well formed inputs and stay panic free.

use thiserror::Error;

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum CoreError {
    #[error("sell_amount and buy_amount must both be non-zero")]
    ZeroAmount,
    #[error("sell_token and buy_token must differ")]
    SameToken,
}
