//! Behavior tests for the continuous matcher.

use alloy_primitives::{Address, U256};
use crossbook_core::book::OrderBook;
use crossbook_core::types::{OpenOrder, Order, SubmitOutcome};

fn addr(b: u8) -> Address {
    Address::repeat_byte(b)
}

const A: u8 = 0x0A; // token A
const B: u8 = 0x0B; // token B

/// Build an admitted order. `hash` byte distinguishes orders; `seq` is arrival order.
fn order(
    seq: u64,
    hash: u8,
    sell_token: u8,
    sell_amount: u64,
    buy_token: u8,
    buy_amount: u64,
    partially_fillable: bool,
) -> OpenOrder {
    let o = Order {
        maker: addr(0xAA),
        sell_token: addr(sell_token),
        buy_token: addr(buy_token),
        sell_amount: U256::from(sell_amount),
        buy_amount: U256::from(buy_amount),
        valid_to: u64::MAX,
        nonce: U256::from(seq),
        partially_fillable,
    };
    OpenOrder::new(o, [hash; 32], seq).expect("valid test order")
}

fn u(x: u64) -> U256 {
    U256::from(x)
}

#[test]
fn lone_order_rests() {
    let mut book = OrderBook::new();
    let mut fills = Vec::new();
    let outcome = book.submit(order(0, 1, A, 100, B, 100, true), &mut fills);
    assert_eq!(outcome, SubmitOutcome::Resting);
    assert!(fills.is_empty());
    assert_eq!(book.resting_count(), 1);
    assert!(book.contains(&[1; 32]));
}

#[test]
fn same_side_orders_do_not_cross() {
    let mut book = OrderBook::new();
    let mut fills = Vec::new();
    book.submit(order(0, 1, A, 100, B, 100, true), &mut fills);
    let outcome = book.submit(order(1, 2, A, 50, B, 60, true), &mut fills);
    assert_eq!(outcome, SubmitOutcome::Resting);
    assert!(fills.is_empty());
    assert_eq!(book.resting_count(), 2);
}

#[test]
fn exact_opposite_orders_fully_fill() {
    let mut book = OrderBook::new();
    let mut fills = Vec::new();
    // maker sells 100 A for 100 B
    book.submit(order(0, 1, A, 100, B, 100, true), &mut fills);
    // taker sells 100 B for 100 A
    let outcome = book.submit(order(1, 2, B, 100, A, 100, true), &mut fills);

    assert_eq!(outcome, SubmitOutcome::FullyFilled);
    assert_eq!(fills.len(), 1);
    let f = fills[0];
    assert_eq!(f.maker_hash, [1; 32]);
    assert_eq!(f.taker_hash, [2; 32]);
    assert_eq!(f.sell_filled, u(100)); // maker's A out
    assert_eq!(f.buy_filled, u(100)); // maker's B in
    assert_eq!(book.resting_count(), 0); // both gone
}

#[test]
fn taker_smaller_than_maker_leaves_maker_resting() {
    let mut book = OrderBook::new();
    let mut fills = Vec::new();
    book.submit(order(0, 1, A, 100, B, 100, true), &mut fills); // maker 100 A / 100 B
    let outcome = book.submit(order(1, 2, B, 40, A, 40, true), &mut fills); // taker 40 B / 40 A

    assert_eq!(outcome, SubmitOutcome::FullyFilled);
    assert_eq!(fills.len(), 1);
    assert_eq!(fills[0].sell_filled, u(40));
    assert_eq!(fills[0].buy_filled, u(40));
    assert_eq!(book.resting_count(), 1); // maker remainder rests
    assert!(book.contains(&[1; 32]));
}

#[test]
fn taker_larger_than_maker_partially_fills_and_rests() {
    let mut book = OrderBook::new();
    let mut fills = Vec::new();
    book.submit(order(0, 1, A, 40, B, 40, true), &mut fills); // maker 40 A / 40 B
    let outcome = book.submit(order(1, 2, B, 100, A, 100, true), &mut fills); // taker 100 B / 100 A

    assert_eq!(
        outcome,
        SubmitOutcome::PartiallyFilled {
            remaining_sell: u(60)
        }
    );
    assert_eq!(fills.len(), 1);
    assert_eq!(fills[0].sell_filled, u(40)); // maker A out
    assert_eq!(fills[0].buy_filled, u(40)); // maker B in
    assert_eq!(book.resting_count(), 1); // taker remainder rests, maker gone
    assert!(book.contains(&[2; 32]));
    assert!(!book.contains(&[1; 32]));
}

#[test]
fn fill_or_kill_unfillable_is_killed_and_changes_nothing() {
    let mut book = OrderBook::new();
    let mut fills = Vec::new();
    book.submit(order(0, 1, A, 100, B, 100, true), &mut fills); // maker 100 A / 100 B
                                                                // FOK taker wants 200 A for 200 B but only 100 available
    let outcome = book.submit(order(1, 2, B, 200, A, 200, false), &mut fills);

    assert_eq!(outcome, SubmitOutcome::Killed);
    assert!(fills.is_empty());
    assert_eq!(book.resting_count(), 1); // maker untouched
    assert!(book.contains(&[1; 32]));
    assert!(!book.contains(&[2; 32]));
}

#[test]
fn fill_or_kill_fully_fillable_executes() {
    let mut book = OrderBook::new();
    let mut fills = Vec::new();
    book.submit(order(0, 1, A, 200, B, 200, true), &mut fills); // maker 200 A / 200 B
    let outcome = book.submit(order(1, 2, B, 100, A, 100, false), &mut fills); // FOK 100/100

    assert_eq!(outcome, SubmitOutcome::FullyFilled);
    assert_eq!(fills.len(), 1);
    assert_eq!(fills[0].sell_filled, u(100));
    assert_eq!(book.resting_count(), 1); // maker remainder rests
}

#[test]
fn best_price_matches_first() {
    let mut book = OrderBook::new();
    let mut fills = Vec::new();
    // two makers selling A; maker cheap wants 90 B for 100 A, maker pricey wants 110 B
    book.submit(order(0, 1, A, 100, B, 110, true), &mut fills); // pricey (1.10)
    book.submit(order(1, 2, A, 100, B, 90, true), &mut fills); // cheap (0.90)
                                                               // taker sells 90 B to buy A; should hit the cheap maker (hash 2) first
    let outcome = book.submit(order(2, 3, B, 90, A, 100, true), &mut fills);

    assert_eq!(outcome, SubmitOutcome::FullyFilled);
    assert_eq!(fills.len(), 1);
    assert_eq!(fills[0].maker_hash, [2; 32]); // cheap maker matched
    assert_eq!(fills[0].sell_filled, u(100)); // 90 B buys all 100 A at 0.9
    assert_eq!(fills[0].buy_filled, u(90));
}

#[test]
fn equal_price_matches_in_arrival_order() {
    let mut book = OrderBook::new();
    let mut fills = Vec::new();
    // two makers, same price (1.0), different arrival
    book.submit(order(0, 1, A, 50, B, 50, true), &mut fills); // first
    book.submit(order(1, 2, A, 50, B, 50, true), &mut fills); // second
                                                              // taker buys 50 A; must hit the first maker (hash 1)
    let outcome = book.submit(order(2, 3, B, 50, A, 50, true), &mut fills);

    assert_eq!(outcome, SubmitOutcome::FullyFilled);
    assert_eq!(fills.len(), 1);
    assert_eq!(fills[0].maker_hash, [1; 32]);
    assert!(book.contains(&[2; 32])); // second maker still resting
}

#[test]
fn taker_gets_price_improvement_at_maker_price() {
    let mut book = OrderBook::new();
    let mut fills = Vec::new();
    book.submit(order(0, 1, A, 100, B, 100, true), &mut fills); // maker 100 A / 100 B (price 1)
                                                                // taker willing to pay up to 100 B for just 50 A (generous); executes at maker price
    let outcome = book.submit(order(1, 2, B, 100, A, 50, true), &mut fills);

    assert_eq!(outcome, SubmitOutcome::FullyFilled);
    assert_eq!(fills.len(), 1);
    assert_eq!(fills[0].sell_filled, u(100)); // taker receives all 100 A, not just 50
    assert_eq!(fills[0].buy_filled, u(100));
}

#[test]
fn cancel_removes_resting_order() {
    let mut book = OrderBook::new();
    let mut fills = Vec::new();
    book.submit(order(0, 1, A, 100, B, 100, true), &mut fills);
    assert!(book.cancel(&[1; 32]));
    assert_eq!(book.resting_count(), 0);
    assert!(!book.contains(&[1; 32]));
    assert!(!book.cancel(&[1; 32])); // already gone
}
