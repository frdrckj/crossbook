//! Proves the no allocation claim: once the reused buffers are warm, a submit
//! that crosses existing liquidity (without resting a remainder) performs zero
//! heap allocations on the match path. A counting global allocator measures it.

use alloy_primitives::{Address, U256};
use crossbook_core::book::OrderBook;
use crossbook_core::types::{OpenOrder, Order};
use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicUsize, Ordering};

static ALLOCS: AtomicUsize = AtomicUsize::new(0);

struct Counting;
unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, l: Layout) -> *mut u8 {
        ALLOCS.fetch_add(1, Ordering::Relaxed);
        System.alloc(l)
    }
    unsafe fn dealloc(&self, p: *mut u8, l: Layout) {
        System.dealloc(p, l)
    }
}

#[global_allocator]
static GLOBAL: Counting = Counting;

const A: u8 = 0x0A;
const B: u8 = 0x0B;

fn ord(seq: u64, hash: [u8; 32], st: u8, sa: u64, bt: u8, ba: u64) -> OpenOrder {
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
    OpenOrder::new(o, hash, seq).unwrap()
}

fn hash(i: u64) -> [u8; 32] {
    let mut h = [0u8; 32];
    h[0..8].copy_from_slice(&i.to_le_bytes());
    h
}

#[test]
fn hot_match_path_does_not_allocate() {
    let mut book = OrderBook::new();
    let mut out = Vec::with_capacity(128);

    // Rest a deep book of makers selling A for B at price 1.0.
    for i in 0..2000u64 {
        book.submit(ord(i, hash(i), A, 1, B, 1), &mut out);
    }
    out.clear();

    // Warm up the reused buffers: a taker that fully fills against 16 makers.
    let warm = ord(10_001, hash(10_001), B, 16, A, 16);
    book.submit(warm, &mut out);
    out.clear();

    // Measured: another taker that fully fills against 16 makers. No remainder
    // rests, the out buffer has capacity, and the scratch buffer is warm, so the
    // match path must not touch the allocator.
    let taker = ord(10_002, hash(10_002), B, 16, A, 16);
    let before = ALLOCS.load(Ordering::Relaxed);
    let outcome = book.submit(taker, &mut out);
    let after = ALLOCS.load(Ordering::Relaxed);

    assert_eq!(out.len(), 16, "should have produced 16 fills");
    assert_eq!(
        after - before,
        0,
        "hot match path allocated {} times",
        after - before
    );
    // touch outcome so it is not optimized away
    assert!(matches!(
        outcome,
        crossbook_core::types::SubmitOutcome::FullyFilled
    ));
}
