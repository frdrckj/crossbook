//! The order book and the continuous price then time priority matcher.
//!
//! Orders are grouped by directed pair (sell_token, buy_token), and an incoming
//! order matches against the opposite pair. Resting orders sit in a price ordered
//! map of FIFO queues, so the best price matches first and equal prices match in
//! arrival order.
//!
//! Trades execute at the resting maker's price, and the maker's received amount
//! is rounded up in its favor. A fill is only produced when it also respects the
//! taker's signed limit, so a maker whose remaining amount is too small to clear
//! at a ratio the taker accepts is skipped rather than matched, leaving its dust
//! to rest.
//!
//! Matching the incoming order only as a taker at resting maker prices is not
//! quite enough to leave a fixpoint: a resting order can still cross the incoming
//! order at the incoming order's price, a case integer rounding at the maker price
//! makes unreachable from the taker side. So once the incoming order would rest,
//! a maker pass lets any resting order that still crosses it take from it at its
//! price, which guarantees the book has no crossable fill left. The matcher does
//! no I/O and keeps no clock.

use crate::price::{self, cmp_limit};
use crate::types::{Fill, OpenOrder, Order, OrderHash, SubmitOutcome};
use alloy_primitives::{Address, U256};
use std::cmp::Ordering;
use std::collections::{BTreeMap, HashMap, VecDeque};

/// A maker's limit price as the ratio `buy / sell`, ordered and compared by exact
/// cross multiplication so that equal ratios collapse to a single price level.
#[derive(Clone, Copy, Debug)]
struct Price {
    buy: U256,
    sell: U256,
}

impl PartialEq for Price {
    fn eq(&self, other: &Self) -> bool {
        cmp_limit(self.buy, self.sell, other.buy, other.sell) == Ordering::Equal
    }
}
impl Eq for Price {}
impl Ord for Price {
    fn cmp(&self, other: &Self) -> Ordering {
        cmp_limit(self.buy, self.sell, other.buy, other.sell)
    }
}
impl PartialOrd for Price {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

type Side = BTreeMap<Price, VecDeque<OpenOrder>>;

/// Outcome of planning a fill against one maker.
enum Step {
    /// A valid fill: maker sends `y` of its sell token, taker pays `x`.
    Trade { y: U256, x: U256 },
    /// This maker cannot fill at a ratio the taker accepts (its remaining is too
    /// small). Skip it and try the next maker.
    Skip,
    /// The taker can no longer afford even one unit at this price (or any pricier
    /// maker). Stop matching.
    Stop,
}

/// Plan a single pairwise fill at the maker's price. `y` is the maker's sell token
/// sent out (the taker receives it) and `x` is the taker's payment (the maker's
/// buy token received), rounded up in the maker's favor.
fn plan_step(maker: &Order, m_remaining: U256, taker: &Order, t_remaining: U256) -> Step {
    // y is bounded by the maker's remaining and by what the taker's budget buys
    // at the maker price: y <= floor(t_remaining * maker.sell / maker.buy).
    let y = price::cap_floor(
        t_remaining,
        maker.sell_amount,
        maker.buy_amount,
        m_remaining,
    );
    if y.is_zero() {
        // Budget cannot buy a whole unit here; every later (pricier) maker is worse.
        return Step::Stop;
    }
    // x = ceil(y * maker.buy / maker.sell): the maker receives at least its limit.
    let x = price::mul_div_ceil(y, maker.buy_amount, maker.sell_amount);
    // The taker's signed limit must hold too: taker gets y for paying x, so it
    // needs y/x >= taker.buy/taker.sell.
    if cmp_limit(y, x, taker.buy_amount, taker.sell_amount) == Ordering::Less {
        return Step::Skip;
    }
    Step::Trade { y, x }
}

#[derive(Default)]
pub struct OrderBook {
    /// directed pair (sell_token, buy_token) -> price levels -> FIFO queue
    sides: HashMap<(Address, Address), Side>,
    /// order hash -> the directed pair it rests in (for cancel)
    locator: HashMap<OrderHash, (Address, Address)>,
    /// Reused planning buffer so the steady state match path does not allocate
    /// per submit. Taken out with `mem::take` during a submit, then restored.
    scratch: Vec<(OrderHash, U256, U256)>,
}

impl OrderBook {
    pub fn new() -> Self {
        Self::default()
    }

    /// Insert a new admitted order and immediately match it against the book.
    /// Produced trades are appended to `out` (the caller reuses one buffer, so
    /// the steady state match path does not allocate per submit).
    pub fn submit(&mut self, order: OpenOrder, out: &mut Vec<Fill>) -> SubmitOutcome {
        let opp_key = (order.order.buy_token, order.order.sell_token);
        let mut t_remaining = order.remaining_sell;

        // Phase 1: plan (read only). Walk the opposite side, cheapest first.
        // Reuse the scratch buffer (taken out so `self.sides` can be borrowed).
        let mut plan = std::mem::take(&mut self.scratch);
        plan.clear();
        if let Some(side) = self.sides.get(&opp_key) {
            'outer: for (price, queue) in side.iter() {
                // Cross check at the level: maker price (buy/sell) must be at or
                // below the taker's max price (sell/buy). Levels are ascending,
                // so once one is too expensive, all the rest are too.
                if cmp_limit(
                    price.buy,
                    price.sell,
                    order.order.sell_amount,
                    order.order.buy_amount,
                ) == Ordering::Greater
                {
                    break;
                }
                for maker in queue.iter() {
                    if t_remaining.is_zero() {
                        break 'outer;
                    }
                    match plan_step(
                        &maker.order,
                        maker.remaining_sell,
                        &order.order,
                        t_remaining,
                    ) {
                        Step::Stop => break 'outer,
                        Step::Skip => continue,
                        Step::Trade { y, x } => {
                            plan.push((maker.hash, y, x));
                            t_remaining -= x;
                        }
                    }
                }
            }
        }

        // Fill or kill: must complete in full as a taker or touch nothing.
        if !order.order.partially_fillable && !t_remaining.is_zero() {
            plan.clear();
            self.scratch = plan;
            return SubmitOutcome::Killed;
        }

        // Phase 2: commit the taker fills. Consume each planned maker by hash
        // (skipped makers remain in place, so front popping would not line up).
        let filled_as_taker = !plan.is_empty();
        for (maker_hash, y, x) in &plan {
            self.consume(&opp_key, maker_hash, *y);
            out.push(Fill {
                maker_hash: *maker_hash,
                taker_hash: order.hash,
                sell_filled: *y,
                buy_filled: *x,
            });
        }
        plan.clear();

        if t_remaining.is_zero() {
            self.scratch = plan;
            return SubmitOutcome::FullyFilled;
        }

        // Phase 3, maker pass. The order is about to rest, so it becomes a maker.
        // Resting orders on the opposite side may still cross it at its own price,
        // a locked cross the taker phase cannot reach because it executes only at
        // resting maker prices and integer rounding there can tip the fill past
        // the incoming order's limit. Let those resting orders take from it now, so
        // the book stays a fixpoint with no crossable fill left.
        if let Some(side) = self.sides.get(&opp_key) {
            'maker: for (_p, queue) in side.iter() {
                for r in queue.iter() {
                    if t_remaining.is_zero() {
                        break 'maker;
                    }
                    // The incoming order is the maker (remaining t_remaining); the
                    // resting order r is the taker. Each taker has its own budget,
                    // so one that cannot take a whole unit is skipped, not a stop.
                    if let Step::Trade { y, x } =
                        plan_step(&order.order, t_remaining, &r.order, r.remaining_sell)
                    {
                        plan.push((r.hash, y, x));
                        t_remaining -= y;
                    }
                }
            }
        }
        let filled_as_maker = !plan.is_empty();
        for (taker_hash, y, x) in &plan {
            // The incoming order (maker) sends `y` of its sell token; the resting
            // taker pays `x` of its own sell token, which is the maker's buy token.
            self.consume(&opp_key, taker_hash, *x);
            out.push(Fill {
                maker_hash: order.hash,
                taker_hash: *taker_hash,
                sell_filled: *y,
                buy_filled: *x,
            });
        }
        plan.clear();
        self.scratch = plan;

        if t_remaining.is_zero() {
            return SubmitOutcome::FullyFilled;
        }
        let mut resting = order;
        resting.remaining_sell = t_remaining;
        self.insert_resting(resting);
        if filled_as_taker || filled_as_maker {
            SubmitOutcome::PartiallyFilled {
                remaining_sell: t_remaining,
            }
        } else {
            SubmitOutcome::Resting
        }
    }

    /// Remove a resting order by hash. Returns true if it was present.
    pub fn cancel(&mut self, order_hash: &OrderHash) -> bool {
        let Some(key) = self.locator.remove(order_hash) else {
            return false;
        };
        let Some(side) = self.sides.get_mut(&key) else {
            return false;
        };
        let mut empty_price = None;
        let mut found = false;
        for (price, queue) in side.iter_mut() {
            if let Some(pos) = queue.iter().position(|o| &o.hash == order_hash) {
                queue.remove(pos);
                found = true;
                if queue.is_empty() {
                    empty_price = Some(*price);
                }
                break;
            }
        }
        if let Some(p) = empty_price {
            side.remove(&p);
        }
        found
    }

    /// Whether an order with this hash is currently resting.
    pub fn contains(&self, order_hash: &OrderHash) -> bool {
        self.locator.contains_key(order_hash)
    }

    /// Total number of resting orders across all pairs.
    pub fn resting_count(&self) -> usize {
        self.sides
            .values()
            .flat_map(|s| s.values())
            .map(|q| q.len())
            .sum()
    }

    /// All resting orders, in no particular order. Clones; intended for snapshots
    /// and test assertions.
    pub fn resting_orders(&self) -> Vec<OpenOrder> {
        self.sides
            .values()
            .flat_map(|s| s.values())
            .flatten()
            .cloned()
            .collect()
    }

    /// True if any resting order, submitted again as a taker, would still produce
    /// a fill against the book. The matcher must leave a book where this is false
    /// (a fixpoint), so it doubles as the maximal matching assertion.
    pub fn crossable_fill_exists(&self) -> bool {
        // O(n^2) over resting orders, which is fine for assertions and small
        // books. A per pair best versus best check would be cheaper if needed.
        let all: Vec<&OpenOrder> = self
            .sides
            .values()
            .flat_map(|s| s.values())
            .flatten()
            .collect();
        for taker in &all {
            for maker in &all {
                if taker.hash == maker.hash {
                    continue;
                }
                if maker.order.sell_token == taker.order.buy_token
                    && maker.order.buy_token == taker.order.sell_token
                {
                    if let Step::Trade { .. } = plan_step(
                        &maker.order,
                        maker.remaining_sell,
                        &taker.order,
                        taker.remaining_sell,
                    ) {
                        return true;
                    }
                }
            }
        }
        false
    }

    fn insert_resting(&mut self, o: OpenOrder) {
        let key = (o.order.sell_token, o.order.buy_token);
        let p = Price {
            buy: o.order.buy_amount,
            sell: o.order.sell_amount,
        };
        self.locator.insert(o.hash, key);
        self.sides
            .entry(key)
            .or_default()
            .entry(p)
            .or_default()
            .push_back(o);
    }

    /// Reduce the resting order `hash` on side `key` by `y`, removing it (and an
    /// emptied level) when fully consumed.
    fn consume(&mut self, key: &(Address, Address), hash: &OrderHash, y: U256) {
        let Some(side) = self.sides.get_mut(key) else {
            return;
        };
        let mut empty_price = None;
        let mut remove_hash = None;
        for (price, queue) in side.iter_mut() {
            if let Some(pos) = queue.iter().position(|o| &o.hash == hash) {
                let maker = &mut queue[pos];
                maker.remaining_sell -= y;
                if maker.remaining_sell.is_zero() {
                    queue.remove(pos);
                    remove_hash = Some(*hash);
                    if queue.is_empty() {
                        empty_price = Some(*price);
                    }
                }
                break;
            }
        }
        if let Some(p) = empty_price {
            side.remove(&p);
        }
        if let Some(h) = remove_hash {
            self.locator.remove(&h);
        }
    }
}
