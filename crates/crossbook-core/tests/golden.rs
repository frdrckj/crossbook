//! Golden replay: a fixed input sequence must produce one exact, reproducible
//! output. Because the core is pure, replaying the same inputs always yields the
//! same fills.

use alloy_primitives::{Address, U256};
use crossbook_core::book::OrderBook;
use crossbook_core::types::{Fill, OpenOrder, Order, SubmitOutcome};

const A: u8 = 0x0A;
const B: u8 = 0x0B;

fn ord(seq: u64, hash: u8, st: u8, sa: u64, bt: u8, ba: u64, partial: bool) -> OpenOrder {
    let o = Order {
        maker: Address::repeat_byte(0xAA),
        sell_token: Address::repeat_byte(st),
        buy_token: Address::repeat_byte(bt),
        sell_amount: U256::from(sa),
        buy_amount: U256::from(ba),
        valid_to: u64::MAX,
        nonce: U256::from(seq),
        partially_fillable: partial,
    };
    OpenOrder::new(o, [hash; 32], seq).unwrap()
}

/// Submit the canonical scenario and return every fill plus the taker outcome.
fn run() -> (Vec<Fill>, SubmitOutcome) {
    let mut book = OrderBook::new();
    let mut fills = Vec::new();
    book.submit(ord(0, 1, A, 100, B, 100, true), &mut fills); // maker1, price 1.0
    book.submit(ord(1, 2, A, 100, B, 120, true), &mut fills); // maker2, price 1.2
    let outcome = book.submit(ord(2, 3, B, 300, A, 130, true), &mut fills); // taker sweeps both
    (fills, outcome)
}

#[test]
fn golden_two_level_sweep() {
    let (fills, outcome) = run();
    assert_eq!(
        outcome,
        SubmitOutcome::PartiallyFilled {
            remaining_sell: U256::from(80u64)
        }
    );
    assert_eq!(
        fills,
        vec![
            // best price (maker1, 1.0) first, at the maker's price
            Fill {
                maker_hash: [1; 32],
                taker_hash: [3; 32],
                sell_filled: U256::from(100u64),
                buy_filled: U256::from(100u64),
            },
            // then maker2 at 1.2; taker pays 120 B for 100 A
            Fill {
                maker_hash: [2; 32],
                taker_hash: [3; 32],
                sell_filled: U256::from(100u64),
                buy_filled: U256::from(120u64),
            },
        ]
    );
}

#[test]
fn replay_is_deterministic() {
    assert_eq!(run(), run());
}
