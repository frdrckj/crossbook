//! Price comparison and fill arithmetic that cannot overflow.
//!
//! An order's price is the ratio buy_amount / sell_amount. To compare two ratios
//! b1/s1 and b2/s2 (with s1, s2 > 0) we cross multiply: b1*s2 against b2*s1. Those
//! products reach about 512 bits as the inputs approach 2^256, so a 256 bit
//! multiply would panic in debug and wrap in release. Everything here widens to
//! 512 bits first.

use alloy_primitives::{U256, U512};
use core::cmp::Ordering;

/// Compare the ratio `b1/s1` against `b2/s2`. Caller guarantees `s1 > 0` and `s2 > 0`.
pub fn cmp_limit(b1: U256, s1: U256, b2: U256, s2: U256) -> Ordering {
    // b1/s1 vs b2/s2 is the same as b1*s2 vs b2*s1, widened so it cannot overflow.
    let lhs: U512 = b1.widening_mul(s2);
    let rhs: U512 = b2.widening_mul(s1);
    lhs.cmp(&rhs)
}

#[inline]
fn to_u512(x: U256) -> U512 {
    // multiply-by-one widens without any From-impl assumptions.
    x.widening_mul(U256::from(1u64))
}

#[inline]
fn narrow(x: U512) -> U256 {
    // Caller guarantees x < 2^256, so the low 32 bytes carry the whole value.
    debug_assert!(x <= to_u512(U256::MAX), "narrow: value exceeds U256");
    let bytes = x.to_le_bytes::<64>();
    U256::from_le_slice(&bytes[..32])
}

/// ceil(a*b / d). Caller guarantees d > 0 and that the true result is below 2^256.
/// Rounds a maker's received amount up, in the maker's favor.
pub(crate) fn mul_div_ceil(a: U256, b: U256, d: U256) -> U256 {
    let num: U512 = a.widening_mul(b);
    let dd = to_u512(d);
    let q = num / dd;
    let q = if (num % dd).is_zero() {
        q
    } else {
        q + to_u512(U256::from(1u64))
    };
    narrow(q)
}

/// min(floor(a*b / d), cap). Caller guarantees d > 0. The result is at most cap,
/// so it always fits in U256 even when floor(a*b/d) would not.
pub(crate) fn cap_floor(a: U256, b: U256, d: U256, cap: U256) -> U256 {
    let q: U512 = a.widening_mul(b) / to_u512(d);
    let cap512 = to_u512(cap);
    narrow(if q < cap512 { q } else { cap512 })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn u(x: u64) -> U256 {
        U256::from(x)
    }

    #[test]
    fn equal_ratios_compare_equal() {
        assert_eq!(cmp_limit(u(2), u(4), u(1), u(2)), Ordering::Equal);
    }

    #[test]
    fn orders_distinct_ratios() {
        assert_eq!(cmp_limit(u(3), u(4), u(1), u(2)), Ordering::Greater);
        assert_eq!(cmp_limit(u(1), u(2), u(3), u(4)), Ordering::Less);
    }

    #[test]
    fn does_not_overflow_near_u256_max() {
        let max = U256::MAX;
        assert_eq!(cmp_limit(max, u(1), u(1), max), Ordering::Greater);
    }

    #[test]
    fn ceil_rounds_up_and_is_exact_when_divisible() {
        assert_eq!(mul_div_ceil(u(10), u(3), u(4)), u(8)); // 30/4 = 7.5 -> 8
        assert_eq!(mul_div_ceil(u(10), u(2), u(5)), u(4)); // 20/5 = 4 exact
        assert_eq!(mul_div_ceil(u(0), u(99), u(7)), u(0));
    }

    #[test]
    fn ceil_does_not_overflow_near_max() {
        let max = U256::MAX;
        assert_eq!(mul_div_ceil(max, max, max), max); // max*max/max = max
    }

    #[test]
    fn cap_floor_floors_then_caps() {
        assert_eq!(cap_floor(u(10), u(3), u(4), u(100)), u(7)); // floor(7.5)=7, <100
        assert_eq!(cap_floor(u(10), u(3), u(4), u(5)), u(5)); // capped at 5
    }

    #[test]
    fn cap_floor_does_not_overflow_when_uncapped_result_exceeds_u256() {
        let max = U256::MAX;
        // floor(max*max/1) is ~2^512; capped to max must not overflow.
        assert_eq!(cap_floor(max, max, u(1), max), max);
    }
}
