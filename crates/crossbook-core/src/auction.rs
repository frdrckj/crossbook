//! Pure, deterministic per-pair uniform-price batch auction (a call auction).
//!
//! Continuous matching (`book.rs`) crosses each order the moment it arrives. The
//! batch auction instead works on a whole window of orders at once: when the
//! window closes, every executed order in a token pair trades at ONE uniform
//! clearing price. Matching buyers directly against sellers of the same pair at
//! that price, with no external liquidity, is the coincidence of wants.
//!
//! Algorithm, per token pair:
//! 1. Pick a canonical base and quote by sorting the two token addresses. Express
//!    each order as a bid (buys base, sells quote) or an ask (sells base, buys
//!    quote) with a limit price in quote per base.
//! 2. Sort asks ascending by limit and bids descending by limit, then walk both to
//!    find where the curves cross: the highest bids matched against the lowest
//!    asks while the bid limit is at or above the ask limit.
//! 3. The clearing price p* is the midpoint of the marginal ask and marginal bid
//!    limits (the price overlap). Any price in that overlap clears the same set at
//!    the same volume, so the volume is maximal and the imbalance is fixed; the
//!    midpoint is the deterministic, fair tie-break. If the midpoint cannot be
//!    represented in 256 bits (extreme inputs), fall back to the marginal ask
//!    limit, which is also in the overlap.
//! 4. To keep every fill at EXACTLY p* with integer amounts, fills are quantized to
//!    a lot equal to the reduced denominator of p*. Each filled base amount is a
//!    multiple of the lot, so its quote leg `base / den * num` is exact. The short
//!    side fills fully; the marginal order on the long side fills partially.
//!    Unmatched quantity is left for the next batch.
//! 5. Every fill nets to zero per token: total base out equals total base in, and
//!    total quote is `q* * p*` on both sides.
//! 6. Surplus is the total price improvement over the orders' limits, in quote.
//!
//! This is a pure function of the collected orders: same input, same output. No
//! async, no I/O, no clock.

use crate::price::{self, cmp_limit};
use crate::types::{OpenOrder, OrderHash};
use alloy_primitives::{Address, U256};
use std::cmp::Ordering;
use std::collections::BTreeMap;

/// One order's execution in a batch, at the pair's uniform clearing price.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AuctionFill {
    pub order_hash: OrderHash,
    pub sell_filled: U256,
    pub buy_filled: U256,
}

/// The clearing of one token pair in a batch.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AuctionResult {
    pub base: Address,
    pub quote: Address,
    /// Uniform clearing price p* = `clearing_num / clearing_den` (quote per base).
    pub clearing_num: U256,
    pub clearing_den: U256,
    /// Total base matched at p*.
    pub volume_base: U256,
    /// Total price improvement over the filled orders' limits, in quote units.
    pub surplus: U256,
    pub fills: Vec<AuctionFill>,
}

/// A pair side view: a bid or an ask with its limit as a quote-per-base ratio.
struct Side {
    hash: OrderHash,
    limit_num: U256,
    limit_den: U256,
    base: U256,
    seq: u64,
}

/// The uniform clearing price p* = `pn/pd` plus its lot size (the reduced
/// denominator), shared by both sides of a pair.
struct Clearing {
    pn: U256,
    pd: U256,
    lot: U256,
}

/// Run the batch auction over all collected orders. Returns one result per token
/// pair that cleared a non zero volume, ordered deterministically by token pair.
pub fn run_auction(orders: &[OpenOrder]) -> Vec<AuctionResult> {
    let mut pairs: BTreeMap<(Address, Address), (Vec<Side>, Vec<Side>)> = BTreeMap::new();

    for o in orders {
        // The batch clears in whole lots of the clearing price, so it can only
        // honor an exact fill or kill amount by accident. Fill or kill orders are
        // rejected at intake in batch mode; the core skips them defensively so its
        // output never asks the contract to partially fill one.
        if !o.order.partially_fillable {
            continue;
        }
        // Size each order by its unfilled quantity, so a remainder left from a
        // previous window rolls in at the right size. For a fresh order this is
        // the full sell amount.
        let rem = o.remaining_sell;
        if rem.is_zero() {
            continue;
        }
        let (st, bt) = (o.order.sell_token, o.order.buy_token);
        let (base, quote) = if st <= bt { (st, bt) } else { (bt, st) };
        let entry = pairs.entry((base, quote)).or_default();
        if st == base {
            // Ask: sells base, wants quote. Limit (min quote per base) = buy / sell.
            // Remaining base offered is the remaining sell amount.
            entry.0.push(Side {
                hash: o.hash,
                limit_num: o.order.buy_amount,
                limit_den: o.order.sell_amount,
                base: rem,
                seq: o.arrival_seq,
            });
        } else {
            // Bid: sells quote, wants base. Limit (max quote per base) = sell / buy.
            // Remaining base demanded = floor(remaining quote * buy / sell).
            let base_rem =
                price::cap_floor(rem, o.order.buy_amount, o.order.sell_amount, U256::MAX);
            if base_rem.is_zero() {
                continue;
            }
            entry.1.push(Side {
                hash: o.hash,
                limit_num: o.order.sell_amount,
                limit_den: o.order.buy_amount,
                base: base_rem,
                seq: o.arrival_seq,
            });
        }
    }

    let mut results = Vec::new();
    for ((base, quote), (mut asks, mut bids)) in pairs {
        if let Some(r) = clear_pair(base, quote, &mut asks, &mut bids) {
            results.push(r);
        }
    }
    results
}

fn clear_pair(
    base: Address,
    quote: Address,
    asks: &mut [Side],
    bids: &mut [Side],
) -> Option<AuctionResult> {
    if asks.is_empty() || bids.is_empty() {
        return None;
    }
    // Asks ascending by limit, bids descending, ties broken by arrival order.
    asks.sort_by(|a, b| {
        cmp_limit(a.limit_num, a.limit_den, b.limit_num, b.limit_den).then(a.seq.cmp(&b.seq))
    });
    bids.sort_by(|a, b| {
        cmp_limit(b.limit_num, b.limit_den, a.limit_num, a.limit_den).then(a.seq.cmp(&b.seq))
    });

    let (mi, mj) = find_cross(asks, bids)?;

    // p* = midpoint of the marginal ask and marginal bid limits (the overlap).
    let (pn, pd) = price::midpoint(
        asks[mi].limit_num,
        asks[mi].limit_den,
        bids[mj].limit_num,
        bids[mj].limit_den,
    )
    .unwrap_or_else(|| price::reduce(asks[mi].limit_num, asks[mi].limit_den));
    let c = Clearing { pn, pd, lot: pd };

    // Cap the matched base so the quote leg `base / lot * pn` can never exceed 256
    // bits: the largest base whose quote fits is floor(MAX / pn) lots. Both sides
    // share the cap, so the pair still nets to zero.
    let quote_cap = (U256::MAX / c.pn) * c.lot;

    // Eligible base per side at p*, floored to whole lots, then capped.
    let ask_avail = eligible_lots(asks, &c, true);
    let bid_avail = eligible_lots(bids, &c, false);
    let qstar = ask_avail.min(bid_avail).min(quote_cap);
    if qstar.is_zero() {
        return None;
    }

    let (mut fills, ask_surplus) = allocate(asks, qstar, &c, true);
    let (bid_fills, bid_surplus) = allocate(bids, qstar, &c, false);
    fills.extend(bid_fills);
    let surplus = ask_surplus.saturating_add(bid_surplus);

    Some(AuctionResult {
        base,
        quote,
        clearing_num: pn,
        clearing_den: pd,
        volume_base: qstar,
        surplus,
        fills,
    })
}

/// Greedy two-pointer: match the highest bids against the lowest asks while the
/// bid limit is at or above the ask limit. Returns the indices of the marginal
/// (last matched) ask and bid, or None if nothing crosses.
fn find_cross(asks: &[Side], bids: &[Side]) -> Option<(usize, usize)> {
    let (mut i, mut j) = (0usize, 0usize);
    let mut ra = asks[0].base;
    let mut rb = bids[0].base;
    let mut matched = None;
    while i < asks.len() && j < bids.len() {
        if cmp_limit(
            bids[j].limit_num,
            bids[j].limit_den,
            asks[i].limit_num,
            asks[i].limit_den,
        ) == Ordering::Less
        {
            break;
        }
        matched = Some((i, j));
        let t = ra.min(rb);
        ra -= t;
        rb -= t;
        if ra.is_zero() {
            i += 1;
            if i < asks.len() {
                ra = asks[i].base;
            }
        }
        if rb.is_zero() {
            j += 1;
            if j < bids.len() {
                rb = bids[j].base;
            }
        }
    }
    matched
}

/// True if this side's limit qualifies at p* (ask limit <= p*, bid limit >= p*).
fn eligible(s: &Side, pn: U256, pd: U256, is_ask: bool) -> bool {
    let c = cmp_limit(s.limit_num, s.limit_den, pn, pd);
    if is_ask {
        c != Ordering::Greater
    } else {
        c != Ordering::Less
    }
}

/// Total eligible base, floored to whole lots, saturating on absurd totals. This
/// is the divisible upper bound; fill or kill is reconciled separately.
fn eligible_lots(side: &[Side], c: &Clearing, is_ask: bool) -> U256 {
    let mut total = U256::ZERO;
    for s in side {
        if !eligible(s, c.pn, c.pd, is_ask) {
            break; // sides are sorted, so the rest are ineligible too
        }
        total = total.saturating_add((s.base / c.lot) * c.lot);
    }
    total
}

/// Fill up to `qstar` base from a side in priority order, in whole lots, all at
/// p*. Returns the fills and the surplus (in quote) this side captured.
fn allocate(side: &[Side], qstar: U256, c: &Clearing, is_ask: bool) -> (Vec<AuctionFill>, U256) {
    let mut fills = Vec::new();
    let mut surplus = U256::ZERO;
    let mut remaining = qstar;
    for s in side {
        if remaining.is_zero() {
            break;
        }
        if !eligible(s, c.pn, c.pd, is_ask) {
            break;
        }
        let avail = (s.base / c.lot) * c.lot;
        let take = avail.min(remaining);
        if take.is_zero() {
            continue;
        }
        let k = take / c.lot; // exact, take is a multiple of lot
        let quote = match k.checked_mul(c.pn) {
            Some(q) => q,
            None => continue, // quote leg would exceed 256 bits; skip this order
        };
        remaining -= take;

        // limit value for `take` base = take * limit_num / limit_den (floored).
        let limit_quote = price::cap_floor(take, s.limit_num, s.limit_den, U256::MAX);
        if is_ask {
            // ask receives quote >= its minimum; improvement = quote - minimum.
            surplus = surplus.saturating_add(quote.saturating_sub(limit_quote));
            fills.push(AuctionFill {
                order_hash: s.hash,
                sell_filled: take,
                buy_filled: quote,
            });
        } else {
            // bid pays quote <= its maximum; improvement = maximum - quote.
            surplus = surplus.saturating_add(limit_quote.saturating_sub(quote));
            fills.push(AuctionFill {
                order_hash: s.hash,
                sell_filled: quote,
                buy_filled: take,
            });
        }
    }
    (fills, surplus)
}
