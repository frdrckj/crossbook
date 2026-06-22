//! Golden and property tests for three token ring clearing.

use alloy_primitives::{Address, U256};
use crossbook_core::price::cmp_limit;
use crossbook_core::ring::find_ring;
use crossbook_core::types::{OpenOrder, Order};
use proptest::prelude::*;
use std::cmp::Ordering;
use std::collections::HashMap;

const A: u8 = 0x0A;
const B: u8 = 0x0B;
const C: u8 = 0x0C;

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
fn clears_a_profitable_three_token_ring() {
    // A -> B -> C -> A, limits 0.9, 0.889, 0.875; product 0.7, so the ring is
    // profitable and the closing order o3 captures the surplus.
    let o1 = ord(0, 1, A, 100, B, 90);
    let o2 = ord(1, 2, B, 90, C, 80);
    let o3 = ord(2, 3, C, 80, A, 70);

    let r = find_ring(&[o1, o2, o3]).expect("ring should clear");
    assert_eq!(
        r.tokens,
        vec![
            Address::repeat_byte(A),
            Address::repeat_byte(B),
            Address::repeat_byte(C)
        ]
    );

    // o1 trades 90 A for 81 B (exactly its 0.9 limit).
    assert_eq!(by_hash(&r.fills, 1), (u(90), u(81)));
    // o2 trades 81 B for 72 C (exactly its 8/9 limit).
    assert_eq!(by_hash(&r.fills, 2), (u(81), u(72)));
    // o3 trades 72 C for 90 A (its minimum was 63, so it keeps the 27 surplus).
    assert_eq!(by_hash(&r.fills, 3), (u(72), u(90)));
    assert_eq!(r.surplus, u(27));

    // Net to zero per token across the ring.
    assert_net_zero(&r.fills);
}

#[test]
fn rejects_an_unprofitable_ring() {
    // Each limit 1.1; product 1.331 > 1, so no consistent price clears it.
    let o1 = ord(0, 1, A, 1000, B, 1100);
    let o2 = ord(1, 2, B, 1000, C, 1100);
    let o3 = ord(2, 3, C, 1000, A, 1100);
    assert!(find_ring(&[o1, o2, o3]).is_none());
}

#[test]
fn no_ring_without_a_full_cycle() {
    // A -> B and B -> C, but nothing closes C -> A.
    let o1 = ord(0, 1, A, 100, B, 90);
    let o2 = ord(1, 2, B, 90, C, 80);
    assert!(find_ring(&[o1, o2]).is_none());
}

/// The (sell_filled, buy_filled) of the fill for the order with this hash byte.
fn by_hash(fills: &[crossbook_core::auction::AuctionFill], h: u8) -> (U256, U256) {
    let f = fills
        .iter()
        .find(|f| f.order_hash == [h; 32])
        .expect("fill present");
    (f.sell_filled, f.buy_filled)
}

fn assert_net_zero(fills: &[crossbook_core::auction::AuctionFill]) {
    // Hash byte -> (sell_token, buy_token) for the golden ring.
    let toks: HashMap<u8, (u8, u8)> = [(1u8, (A, B)), (2, (B, C)), (3, (C, A))]
        .into_iter()
        .collect();
    for token in [A, B, C] {
        let mut net = 0i128;
        for f in fills {
            let h = f.order_hash[0];
            let (st, bt) = toks[&h];
            if st == token {
                net -= f.sell_filled.to::<u128>() as i128;
            }
            if bt == token {
                net += f.buy_filled.to::<u128>() as i128;
            }
        }
        assert_eq!(net, 0, "token {token:#x} did not net to zero");
    }
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(2000))]

    #[test]
    fn ring_clearing_upholds_invariants(
        s1 in 1u64..10_000, b1 in 1u64..10_000,
        s2 in 1u64..10_000, b2 in 1u64..10_000,
        s3 in 1u64..10_000, b3 in 1u64..10_000,
    ) {
        let orders = vec![
            ord(0, 1, A, s1, B, b1), // A -> B
            ord(1, 2, B, s2, C, b2), // B -> C
            ord(2, 3, C, s3, A, b3), // C -> A
        ];
        let mut map = HashMap::new();
        for o in &orders {
            map.insert(o.hash, o.order.clone());
        }

        let result = find_ring(&orders);
        prop_assert_eq!(&result, &find_ring(&orders)); // deterministic

        if let Some(r) = result {
            prop_assert_eq!(r.fills.len(), 3);

            // Per token net to zero, and each order honors its own limit.
            let mut net: HashMap<Address, i128> = HashMap::new();
            for f in &r.fills {
                let o = &map[&f.order_hash];
                prop_assert!(!f.sell_filled.is_zero() && !f.buy_filled.is_zero());
                prop_assert!(f.sell_filled <= o.sell_amount); // never over the signed amount
                // received / sold >= buy / sell (the maker's limit)
                prop_assert_ne!(
                    cmp_limit(f.buy_filled, f.sell_filled, o.buy_amount, o.sell_amount),
                    Ordering::Less
                );
                *net.entry(o.sell_token).or_default() -= f.sell_filled.to::<u128>() as i128;
                *net.entry(o.buy_token).or_default() += f.buy_filled.to::<u128>() as i128;
            }
            for (_, v) in net {
                prop_assert_eq!(v, 0);
            }
        }
    }
}
