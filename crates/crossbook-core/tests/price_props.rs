//! Property tests for `cmp_limit`: it must agree with exact rational arithmetic
//! across the full U256 range (including operands near 2^256, where a naive
//! 256-bit multiply would wrap) and behave as a total order.

use alloy_primitives::U256;
use crossbook_core::price::cmp_limit;
use num_bigint::{BigInt, Sign};
use num_rational::BigRational;
use proptest::prelude::*;

fn nz(x: U256) -> U256 {
    if x.is_zero() {
        U256::from(1u64)
    } else {
        x
    }
}

fn to_ratio(b: U256, s: U256) -> BigRational {
    let bn = BigInt::from_bytes_be(Sign::Plus, &b.to_be_bytes::<32>());
    let sn = BigInt::from_bytes_be(Sign::Plus, &s.to_be_bytes::<32>());
    BigRational::new(bn, sn)
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(2000))]

    #[test]
    fn agrees_with_rational_oracle_and_is_antisymmetric(
        b1 in any::<[u8; 32]>(), s1 in any::<[u8; 32]>(),
        b2 in any::<[u8; 32]>(), s2 in any::<[u8; 32]>(),
    ) {
        let b1 = U256::from_be_bytes(b1);
        let b2 = U256::from_be_bytes(b2);
        let s1 = nz(U256::from_be_bytes(s1));
        let s2 = nz(U256::from_be_bytes(s2));

        let got = cmp_limit(b1, s1, b2, s2);
        let want = to_ratio(b1, s1).cmp(&to_ratio(b2, s2));
        prop_assert_eq!(got, want);

        // antisymmetry: swapping the operands reverses the result
        prop_assert_eq!(cmp_limit(b2, s2, b1, s1), got.reverse());
    }

    #[test]
    fn transitive_on_three(
        b1 in any::<[u8; 32]>(), s1 in any::<[u8; 32]>(),
        b2 in any::<[u8; 32]>(), s2 in any::<[u8; 32]>(),
        b3 in any::<[u8; 32]>(), s3 in any::<[u8; 32]>(),
    ) {
        use core::cmp::Ordering::*;
        let (b1, b2, b3) = (U256::from_be_bytes(b1), U256::from_be_bytes(b2), U256::from_be_bytes(b3));
        let (s1, s2, s3) = (nz(U256::from_be_bytes(s1)), nz(U256::from_be_bytes(s2)), nz(U256::from_be_bytes(s3)));
        // if a <= b and b <= c then a <= c
        let ab = cmp_limit(b1, s1, b2, s2);
        let bc = cmp_limit(b2, s2, b3, s3);
        if matches!(ab, Less | Equal) && matches!(bc, Less | Equal) {
            prop_assert_ne!(cmp_limit(b1, s1, b3, s3), Greater);
        }
    }
}
