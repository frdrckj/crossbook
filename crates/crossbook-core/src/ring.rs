//! Pure, deterministic ring clearing: a coincidence of wants across three tokens.
//!
//! The pair auction (`auction.rs`) matches a buyer of a token directly against a
//! seller of the same token. A ring goes one hop further: three orders that each
//! sell one token and buy the next form a cycle A -> B -> C -> A, and they can
//! all trade with no external liquidity even though no two of them share a pair.
//! This is the multi token coincidence of wants that an order book cannot express.
//!
//! A ring of three orders, with limits written as buy over sell (the minimum the
//! maker accepts), is profitable exactly when the product of the three limits is
//! at most one: `b1 b2 b3 <= s1 s2 s3`. Intuitively the value can flow all the way
//! around the cycle and come back without leaking.
//!
//! Clearing it with exact integers is the delicate part. Write each limit reduced
//! to lowest terms and pick a scale `t`, then set
//!   x_A = s1 s2 t,  x_B = b1 s2 t,  x_C = b1 b2 t.
//! Now `x_B / x_A = b1 / s1` and `x_C / x_B = b2 / s2` are exact, so o1 and o2
//! trade exactly at their limits, and o3 receives `x_A` for `x_C`, which clears its
//! limit precisely when the ring is profitable. Every token nets to zero: the base
//! each order sells is the base the next order in the cycle receives.
//!
//! `t` is taken as large as the three remaining amounts allow, so the ring clears
//! the most it can. The whole surplus lands on the closing order o3; splitting it
//! evenly across the ring would need a cube root and is left as future work. Like
//! the rest of the core this is pure: same orders in, same ring out, no clock and
//! no I/O.

use crate::auction::AuctionFill;
use crate::price::{self, cmp_limit};
use crate::types::{OpenOrder, OrderHash};
use alloy_primitives::{Address, U256};
use std::cmp::Ordering;

/// One cleared ring: the token cycle, the per order fills, and the surplus (in the
/// closing token) the ring captured over the makers' limits.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RingResult {
    /// The token cycle, e.g. [A, B, C] for A -> B -> C -> A.
    pub tokens: Vec<Address>,
    pub fills: Vec<AuctionFill>,
    pub surplus: U256,
}

/// An order viewed as a directed edge `sell -> buy` with a reduced limit.
struct Edge {
    hash: OrderHash,
    sell_token: Address,
    buy_token: Address,
    /// Reduced limit numerator and denominator (buy over sell), the minimum
    /// buy per sell the maker accepts.
    bn: U256,
    sd: U256,
    /// Remaining sell amount available.
    rem: U256,
    seq: u64,
}

/// Find and clear one profitable three token ring among the orders, if any. The
/// search is deterministic (orders are taken in arrival order) and returns the
/// first ring it can clear. Only partially fillable orders take part, since a ring
/// clears a scaled amount, not an exact signed amount.
pub fn find_ring(orders: &[OpenOrder]) -> Option<RingResult> {
    let mut edges: Vec<Edge> = orders
        .iter()
        .filter(|o| o.order.partially_fillable && !o.remaining_sell.is_zero())
        .map(|o| {
            let (bn, sd) = price::reduce(o.order.buy_amount, o.order.sell_amount);
            Edge {
                hash: o.hash,
                sell_token: o.order.sell_token,
                buy_token: o.order.buy_token,
                bn,
                sd,
                rem: o.remaining_sell,
                seq: o.arrival_seq,
            }
        })
        .collect();
    edges.sort_by_key(|e| e.seq);

    // Look for e1: A->B, e2: B->C, e3: C->A with A, B, C distinct.
    for e1 in &edges {
        for e2 in &edges {
            if e2.sell_token != e1.buy_token || e2.buy_token == e1.sell_token {
                continue;
            }
            for e3 in &edges {
                if e3.sell_token != e2.buy_token || e3.buy_token != e1.sell_token {
                    continue;
                }
                // Three distinct tokens form the cycle.
                if let Some(r) = clear_ring(e1, e2, e3) {
                    return Some(r);
                }
            }
        }
    }
    None
}

/// Clear the cycle e1: A->B, e2: B->C, e3: C->A at exact integer amounts, or None
/// if it is not profitable or the amounts do not fit in 256 bits.
fn clear_ring(e1: &Edge, e2: &Edge, e3: &Edge) -> Option<RingResult> {
    // Coefficients for x_A = s1 s2 t, x_B = b1 s2 t, x_C = b1 b2 t.
    let ca = e1.sd.checked_mul(e2.sd)?; // s1 s2
    let cb = e1.bn.checked_mul(e2.sd)?; // b1 s2
    let cc = e1.bn.checked_mul(e2.bn)?; // b1 b2
    if ca.is_zero() || cb.is_zero() || cc.is_zero() {
        return None;
    }

    // Largest scale the three remaining amounts allow.
    let t = (e1.rem / ca).min(e2.rem / cb).min(e3.rem / cc);
    if t.is_zero() {
        return None;
    }

    let xa = ca.checked_mul(t)?; // A: o1 sells, o3 buys
    let xb = cb.checked_mul(t)?; // B: o1 buys, o2 sells
    let xc = cc.checked_mul(t)?; // C: o2 buys, o3 sells

    // o1 and o2 trade exactly at their limits by construction; o3 must clear its
    // own limit, which holds exactly when the ring is profitable: x_A / x_C >= b3 / s3.
    if cmp_limit(xa, xc, e3.bn, e3.sd) == Ordering::Less {
        return None;
    }

    // o3's minimum acceptable A for the C it gives up; the rest is the ring surplus.
    let o3_min = price::cap_floor(xc, e3.bn, e3.sd, U256::MAX);
    let surplus = xa.saturating_sub(o3_min);

    Some(RingResult {
        tokens: vec![e1.sell_token, e2.sell_token, e3.sell_token],
        fills: vec![
            AuctionFill {
                order_hash: e1.hash,
                sell_filled: xa,
                buy_filled: xb,
            },
            AuctionFill {
                order_hash: e2.hash,
                sell_filled: xb,
                buy_filled: xc,
            },
            AuctionFill {
                order_hash: e3.hash,
                sell_filled: xc,
                buy_filled: xa,
            },
        ],
        surplus,
    })
}
