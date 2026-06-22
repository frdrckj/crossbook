//! Property tests for the batch auction invariants.

use alloy_primitives::{Address, U256};
use crossbook_core::auction::run_auction;
use crossbook_core::price::cmp_limit;
use crossbook_core::types::{OpenOrder, Order};
use proptest::prelude::*;
use std::cmp::Ordering;
use std::collections::HashMap;

const A: u8 = 0x0A;
const B: u8 = 0x0B;

#[derive(Clone, Debug)]
struct Spec {
    sell_a: bool,
    sell: u64,
    buy: u64,
    fillable: bool,
}

fn spec() -> impl Strategy<Value = Spec> {
    // Mostly partially fillable, with a fill or kill minority to exercise the skip.
    (any::<bool>(), 1u64..2000, 1u64..2000, 0u8..4).prop_map(|(d, s, b, f)| Spec {
        sell_a: d,
        sell: s,
        buy: b,
        fillable: f != 0,
    })
}

fn build(specs: &[Spec]) -> (Vec<OpenOrder>, HashMap<[u8; 32], Order>) {
    let mut orders = Vec::new();
    let mut map = HashMap::new();
    for (i, s) in specs.iter().enumerate() {
        let (st, bt) = if s.sell_a { (A, B) } else { (B, A) };
        let mut h = [0u8; 32];
        h[0] = (i & 0xff) as u8;
        h[1] = ((i >> 8) & 0xff) as u8;
        let o = Order {
            maker: Address::repeat_byte(0xAA),
            sell_token: Address::repeat_byte(st),
            buy_token: Address::repeat_byte(bt),
            sell_amount: U256::from(s.sell),
            buy_amount: U256::from(s.buy),
            valid_to: u64::MAX,
            nonce: U256::from(i as u64),
            partially_fillable: s.fillable,
        };
        orders.push(OpenOrder::new(o.clone(), h, i as u64).unwrap());
        map.insert(h, o);
    }
    (orders, map)
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(1500))]

    #[test]
    fn auction_upholds_all_invariants(specs in proptest::collection::vec(spec(), 1..30)) {
        let (orders, map) = build(&specs);
        let results = run_auction(&orders);

        // (e) determinism, and independence of input ordering (arrival_seq decides ties)
        prop_assert_eq!(&results, &run_auction(&orders));
        let mut reversed = orders.clone();
        reversed.reverse();
        prop_assert_eq!(&results, &run_auction(&reversed));

        let base = Address::repeat_byte(A);
        for r in &results {
            prop_assert!(!r.volume_base.is_zero());
            let lot = r.clearing_den;

            let (mut base_in, mut base_out) = (U256::ZERO, U256::ZERO);
            let (mut quote_in, mut quote_out) = (U256::ZERO, U256::ZERO);
            let (mut ask_filled, mut bid_filled) = (U256::ZERO, U256::ZERO);

            for f in &r.fills {
                let o = &map[&f.order_hash];
                prop_assert!(!f.sell_filled.is_zero());
                prop_assert!(!f.buy_filled.is_zero());
                // (f) fill or kill orders never appear in a batch fill at all.
                prop_assert!(o.partially_fillable, "fill or kill order was matched in a batch");

                if o.sell_token == base {
                    // ask: sold base, received quote
                    // (a) executes at exactly the clearing price
                    prop_assert_eq!(
                        cmp_limit(f.buy_filled, f.sell_filled, r.clearing_num, r.clearing_den),
                        Ordering::Equal
                    );
                    // (b) limit respected: received >= its minimum
                    prop_assert_ne!(
                        cmp_limit(f.buy_filled, f.sell_filled, o.buy_amount, o.sell_amount),
                        Ordering::Less
                    );
                    base_out += f.sell_filled;
                    quote_in += f.buy_filled;
                    ask_filled += f.sell_filled;
                } else {
                    // bid: sold quote, received base
                    prop_assert_eq!(
                        cmp_limit(f.sell_filled, f.buy_filled, r.clearing_num, r.clearing_den),
                        Ordering::Equal
                    );
                    // (b) limit respected: paid <= its maximum
                    prop_assert_ne!(
                        cmp_limit(f.sell_filled, f.buy_filled, o.sell_amount, o.buy_amount),
                        Ordering::Greater
                    );
                    quote_out += f.sell_filled;
                    base_in += f.buy_filled;
                    bid_filled += f.buy_filled;
                }
            }

            // (c) conservation and net to zero
            prop_assert_eq!(base_in, base_out);
            prop_assert_eq!(quote_in, quote_out);
            prop_assert_eq!(ask_filled, r.volume_base);
            prop_assert_eq!(bid_filled, r.volume_base);

            // (d) maximal volume: the binding side is fully consumed and the
            // matched volume is the min of the two eligible sides. Fill or kill
            // orders do not participate, so they are excluded here too.
            let (mut ask_avail, mut bid_avail) = (U256::ZERO, U256::ZERO);
            for o in map.values() {
                if !o.partially_fillable {
                    continue;
                }
                if o.sell_token == base {
                    if cmp_limit(o.buy_amount, o.sell_amount, r.clearing_num, r.clearing_den)
                        != Ordering::Greater
                    {
                        ask_avail += (o.sell_amount / lot) * lot;
                    }
                } else if cmp_limit(o.sell_amount, o.buy_amount, r.clearing_num, r.clearing_den)
                    != Ordering::Less
                {
                    bid_avail += (o.buy_amount / lot) * lot;
                }
            }
            prop_assert_eq!(r.volume_base, ask_avail.min(bid_avail));
            prop_assert!(ask_avail == r.volume_base || bid_avail == r.volume_base);
        }
    }
}
