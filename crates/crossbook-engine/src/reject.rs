//! Why an order is turned away at intake. One enum is the single source of truth,
//! so the REST error, the metric label, and the validation code never drift. Each
//! variant maps to one HTTP status and one low cardinality metric label.

use thiserror::Error;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum RejectReason {
    #[error("bad signature")]
    BadSignature,
    #[error("order expired")]
    Expired,
    #[error("order cancelled or already fully filled")]
    Cancelled,
    #[error("fill exceeds the remaining amount")]
    Overfill,
    #[error("insufficient balance")]
    InsufficientBalance,
    #[error("insufficient allowance")]
    InsufficientAllowance,
    #[error("unknown token pair")]
    UnknownPair,
    #[error("unsupported token")]
    UnsupportedToken,
    #[error("fill or kill orders are not supported in batch mode")]
    FillOrKillNotInBatch,
    #[error("malformed order")]
    Malformed,
}

impl RejectReason {
    /// Every variant, for tests and for registering the metric label set.
    pub const ALL: [RejectReason; 10] = [
        RejectReason::BadSignature,
        RejectReason::Expired,
        RejectReason::Cancelled,
        RejectReason::Overfill,
        RejectReason::InsufficientBalance,
        RejectReason::InsufficientAllowance,
        RejectReason::UnknownPair,
        RejectReason::UnsupportedToken,
        RejectReason::FillOrKillNotInBatch,
        RejectReason::Malformed,
    ];

    /// A stable label for the rejected orders counter. Never a free form string,
    /// so the metric stays bounded in cardinality.
    pub fn label(self) -> &'static str {
        match self {
            RejectReason::BadSignature => "bad_signature",
            RejectReason::Expired => "expired",
            RejectReason::Cancelled => "cancelled",
            RejectReason::Overfill => "overfill",
            RejectReason::InsufficientBalance => "insufficient_balance",
            RejectReason::InsufficientAllowance => "insufficient_allowance",
            RejectReason::UnknownPair => "unknown_pair",
            RejectReason::UnsupportedToken => "unsupported_token",
            RejectReason::FillOrKillNotInBatch => "fok_not_in_batch",
            RejectReason::Malformed => "malformed",
        }
    }

    /// The HTTP status to return for this rejection.
    pub fn http_status(self) -> u16 {
        match self {
            RejectReason::BadSignature | RejectReason::Malformed => 400,
            RejectReason::UnknownPair | RejectReason::UnsupportedToken => 400,
            RejectReason::FillOrKillNotInBatch => 400,
            RejectReason::Expired | RejectReason::Cancelled | RejectReason::Overfill => 409,
            RejectReason::InsufficientBalance | RejectReason::InsufficientAllowance => 422,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn labels_unique_and_statuses_in_range() {
        let mut labels = HashSet::new();
        for r in RejectReason::ALL {
            assert!(labels.insert(r.label()), "duplicate label {}", r.label());
            assert!(
                (400..=499).contains(&r.http_status()),
                "{r:?} status out of range"
            );
        }
        assert_eq!(labels.len(), RejectReason::ALL.len());
    }
}
