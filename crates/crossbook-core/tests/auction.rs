//! Golden and behavior tests for the batch auction.

use alloy_primitives::{Address, U256};
use crossbook_core::auction::{run_auction, AuctionFill};
use crossbook_core::types::{OpenOrder, Order};

const A: u8 = 0x0A; // base (lower address)
const B: u8 = 0x0B; // quote

fn ord(seq: u64, hash: u8, st: u8, sa: u64, bt: u8, ba: u64) -> OpenOrder {
    let o = Order {
        maker: Address::repeat_byte(0xAA),
        sell_token: Address::repeat_byte(st),
        buy_token: Address::repeat_byte(bt),
        sell_amount: U256::from(sa),
        buy_amount: U256::from(ba),
        valid_to: u64::MAX,
        nonce: U256::from(seq),
        partially_fillable: true,
    };
    OpenOrder::new(o, [hash; 32], seq).unwrap()
}

fn u(x: u64) -> U256 {
    U256::from(x)
}

#[test]
fn coincidence_of_wants_clears_at_the_midpoint() {
    // Ask: sells 100 base, wants >= 180 quote (limit 1.8).
    // Bid: sells 200 quote, wants 100 base (limit 2.0, pays up to 200).
    // Overlap [1.8, 2.0], midpoint 1.9 = 19/10. Both fully fill 100 base at 1.9.
    let ask = ord(0, 1, A, 100, B, 180);
    let bid = ord(1, 2, B, 200, A, 100);

    let results = run_auction(&[ask, bid]);
    assert_eq!(results.len(), 1);
    let r = &results[0];

    assert_eq!(r.base, Address::repeat_byte(A));
    assert_eq!(r.quote, Address::repeat_byte(B));
    assert_eq!((r.clearing_num, r.clearing_den), (u(19), u(10))); // 1.9
    assert_eq!(r.volume_base, u(100));
    assert_eq!(r.surplus, u(20)); // 10 to the ask, 10 to the bid

    // Ask sells 100 base, receives 190 quote at 1.9.
    assert!(r.fills.contains(&AuctionFill {
        order_hash: [1; 32],
        sell_filled: u(100),
        buy_filled: u(190),
    }));
    // Bid sells 190 quote, receives 100 base at 1.9.
    assert!(r.fills.contains(&AuctionFill {
        order_hash: [2; 32],
        sell_filled: u(190),
        buy_filled: u(100),
    }));

    // Net to zero per token.
    let mut base_net = 0i128;
    let mut quote_net = 0i128;
    for f in &r.fills {
        // by hash: ask (1) sells base, bid (2) sells quote
        if f.order_hash == [1; 32] {
            base_net -= f.sell_filled.to::<u128>() as i128;
            quote_net += f.buy_filled.to::<u128>() as i128;
        } else {
            quote_net -= f.sell_filled.to::<u128>() as i128;
            base_net += f.buy_filled.to::<u128>() as i128;
        }
    }
    assert_eq!(base_net, 0);
    assert_eq!(quote_net, 0);
}

#[test]
fn deterministic() {
    let orders = vec![
        ord(0, 1, A, 100, B, 180),
        ord(1, 2, B, 200, A, 100),
        ord(2, 3, A, 50, B, 95),
        ord(3, 4, B, 120, A, 60),
    ];
    assert_eq!(run_auction(&orders), run_auction(&orders));
}

#[test]
fn no_cross_clears_nothing() {
    // Ask wants 200 quote per 100 base (limit 2.0); bid pays at most 1.0. No overlap.
    let ask = ord(0, 1, A, 100, B, 200);
    let bid = ord(1, 2, B, 100, A, 100);
    assert!(run_auction(&[ask, bid]).is_empty());
}

#[test]
fn one_sided_book_clears_nothing() {
    let only_asks = vec![ord(0, 1, A, 100, B, 100), ord(1, 2, A, 50, B, 60)];
    assert!(run_auction(&only_asks).is_empty());
}

#[test]
fn sizes_orders_by_remaining_quantity() {
    // The ask has only 40 of its 100 base left from a previous window; the bid
    // wants 100 base at 2.0. Only the 40 remaining can clear.
    let mut ask = ord(0, 1, A, 100, B, 180); // limit 1.8
    ask.remaining_sell = u(40);
    let bid = ord(1, 2, B, 200, A, 100); // limit 2.0

    let results = run_auction(&[ask, bid]);
    assert_eq!(results.len(), 1);
    let r = &results[0];
    assert_eq!(r.volume_base, u(40));
    assert_eq!((r.clearing_num, r.clearing_den), (u(19), u(10)));
}

/// Quote-unit surplus of one order given its cumulative fill, base token = A.
/// Asks (sell A) measure how much more quote they got than their minimum; bids
/// (sell B) measure how much less quote they paid than their maximum.
fn surplus(o: &Order, filled_sell: u128, filled_buy: u128) -> u128 {
    let sell = o.sell_amount.to::<u128>();
    let buy = o.buy_amount.to::<u128>();
    if o.sell_token == Address::repeat_byte(A) {
        let min_quote = filled_sell * buy / sell; // floor
        filled_buy.saturating_sub(min_quote)
    } else {
        let max_quote = filled_buy * sell / buy; // floor
        max_quote.saturating_sub(filled_sell)
    }
}

#[test]
fn batch_surplus_at_least_continuous_for_offsetting_flow() {
    // Offsetting pair: ask limit 1.8, bid limit 2.0, both 100 base.
    let ask = ord(0, 1, A, 100, B, 180);
    let bid = ord(1, 2, B, 200, A, 100);

    // Batch surplus.
    let batch: u128 = run_auction(&[ask.clone(), bid.clone()])
        .iter()
        .map(|r| r.surplus.to::<u128>())
        .sum();

    // Continuous surplus: run the book in arrival order, accumulate per order.
    use crossbook_core::book::OrderBook;
    let mut book = OrderBook::new();
    let mut fills = Vec::new();
    book.submit(ask.clone(), &mut fills);
    book.submit(bid.clone(), &mut fills);
    let mut fs: std::collections::HashMap<[u8; 32], (u128, u128)> =
        std::collections::HashMap::new();
    for f in &fills {
        let e = fs.entry(f.maker_hash).or_default();
        e.0 += f.sell_filled.to::<u128>();
        e.1 += f.buy_filled.to::<u128>();
        let e = fs.entry(f.taker_hash).or_default();
        e.0 += f.buy_filled.to::<u128>(); // taker sells the maker's buy token
        e.1 += f.sell_filled.to::<u128>();
    }
    let continuous = surplus(
        &ask.order,
        fs.get(&[1; 32]).map_or(0, |x| x.0),
        fs.get(&[1; 32]).map_or(0, |x| x.1),
    ) + surplus(
        &bid.order,
        fs.get(&[2; 32]).map_or(0, |x| x.0),
        fs.get(&[2; 32]).map_or(0, |x| x.1),
    );

    // For clean offsetting flow both capture the same total spread (here 20). The
    // batch redistributes it uniformly at the midpoint (10 to each side) instead
    // of giving it all to the taker, which is the fairness property of a uniform
    // price auction. The invariant we assert is that batch never captures less.
    assert!(
        batch >= continuous,
        "batch {batch} < continuous {continuous}"
    );
    assert_eq!(batch, 20);
    assert_eq!(continuous, 20);
}
