//! Property tests for the matcher invariants. A random sequence of orders is
//! submitted, then every invariant from the spec is checked over the resulting
//! fills and the final book.

use alloy_primitives::{Address, U256};
use crossbook_core::book::OrderBook;
use crossbook_core::price::cmp_limit;
use crossbook_core::types::{OpenOrder, Order};
use proptest::prelude::*;
use std::cmp::Ordering;
use std::collections::HashMap;

fn token(i: u8) -> Address {
    Address::repeat_byte(i)
}

#[derive(Clone, Debug)]
struct Spec {
    sell_token: u8,
    buy_token: u8,
    sell: u64,
    buy: u64,
    partial: bool,
}

fn spec() -> impl Strategy<Value = Spec> {
    // Two tokens (1 and 2). `dir` chooses which side the order sells.
    (
        any::<bool>(),
        1u64..1_000_000u64,
        1u64..1_000_000u64,
        any::<bool>(),
    )
        .prop_map(|(dir, sell, buy, partial)| {
            let (s, b) = if dir { (1u8, 2u8) } else { (2u8, 1u8) };
            Spec {
                sell_token: s,
                buy_token: b,
                sell,
                buy,
                partial,
            }
        })
}

fn hash_of(i: usize) -> [u8; 32] {
    let mut h = [0u8; 32];
    h[0] = (i & 0xff) as u8;
    h[1] = ((i >> 8) & 0xff) as u8;
    h
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(500))]

    #[test]
    fn matcher_upholds_all_invariants(specs in proptest::collection::vec(spec(), 1..40)) {
        let mut book = OrderBook::new();
        let mut fills = Vec::new();
        let mut orders: HashMap<[u8; 32], Order> = HashMap::new();

        for (i, s) in specs.iter().enumerate() {
            let o = Order {
                maker: token(0xAA),
                sell_token: token(s.sell_token),
                buy_token: token(s.buy_token),
                sell_amount: U256::from(s.sell),
                buy_amount: U256::from(s.buy),
                valid_to: u64::MAX,
                nonce: U256::from(i as u64),
                partially_fillable: s.partial,
            };
            let oo = OpenOrder::new(o.clone(), hash_of(i), i as u64).expect("valid order");
            orders.insert(hash_of(i), o);
            book.submit(oo, &mut fills);
        }

        // Accumulate cumulative fills per order, in that order's own sell/buy tokens.
        let mut filled_sell: HashMap<[u8; 32], U256> = HashMap::new();
        let mut filled_buy: HashMap<[u8; 32], U256> = HashMap::new();
        for f in &fills {
            let m = &orders[&f.maker_hash];
            let t = &orders[&f.taker_hash];
            // Conservation: a fill is always between opposite directed orders.
            prop_assert_eq!(m.sell_token, t.buy_token);
            prop_assert_eq!(m.buy_token, t.sell_token);
            // Amounts must be positive (no empty fills emitted).
            prop_assert!(!f.sell_filled.is_zero());
            prop_assert!(!f.buy_filled.is_zero());

            *filled_sell.entry(f.maker_hash).or_insert(U256::ZERO) += f.sell_filled;
            *filled_buy.entry(f.maker_hash).or_insert(U256::ZERO) += f.buy_filled;
            // The taker sells the maker's buy token and receives the maker's sell token.
            *filled_sell.entry(f.taker_hash).or_insert(U256::ZERO) += f.buy_filled;
            *filled_buy.entry(f.taker_hash).or_insert(U256::ZERO) += f.sell_filled;
        }

        for (hash, o) in &orders {
            let fs = filled_sell.get(hash).copied().unwrap_or(U256::ZERO);
            let fb = filled_buy.get(hash).copied().unwrap_or(U256::ZERO);

            // Never sell more than offered.
            prop_assert!(fs <= o.sell_amount);

            // Cumulative limit respected, rounded in the order's favor:
            // filled_buy / filled_sell >= buy_amount / sell_amount.
            if !fs.is_zero() {
                prop_assert_ne!(
                    cmp_limit(fb, fs, o.buy_amount, o.sell_amount),
                    Ordering::Less
                );
            }

            // Fill or kill is all or nothing.
            if !o.partially_fillable {
                prop_assert!(fs.is_zero() || fs == o.sell_amount);
            }
        }

        // Maximal matching: nothing crossable is left unmatched.
        prop_assert!(!book.crossable_fill_exists());

        // Exact remaining accounting, and fill or kill orders never rest.
        for o in book.resting_orders() {
            let fs = filled_sell.get(&o.hash).copied().unwrap_or(U256::ZERO);
            prop_assert_eq!(o.remaining_sell, o.order.sell_amount - fs);
            prop_assert!(o.order.partially_fillable);
        }
    }
}
