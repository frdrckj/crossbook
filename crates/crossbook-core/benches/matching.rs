//! Matching core microbenchmarks: throughput and per submit latency. These
//! measure the pure matcher only, not end to end settlement.

use alloy_primitives::{Address, U256};
use criterion::{criterion_group, criterion_main, BatchSize, Criterion, Throughput};
use crossbook_core::book::OrderBook;
use crossbook_core::types::{Fill, OpenOrder, Order};
use std::hint::black_box;

const A: u8 = 0x0A;
const B: u8 = 0x0B;

fn ord(seq: u64, st: u8, sa: u64, bt: u8, ba: u64) -> OpenOrder {
    let mut hash = [0u8; 32];
    hash[0..8].copy_from_slice(&seq.to_le_bytes());
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
    OpenOrder::new(o, hash, seq).expect("valid order")
}

/// A book of `depth` makers selling A for B at price 1.0, plus a warm out buffer.
fn resting_book(depth: u64) -> (OrderBook, Vec<Fill>) {
    let mut book = OrderBook::new();
    let mut out = Vec::with_capacity(256);
    for i in 0..depth {
        book.submit(ord(i, A, 1, B, 1), &mut out);
    }
    out.clear();
    (book, out)
}

fn bench_matching(c: &mut Criterion) {
    let mut group = c.benchmark_group("matching");
    group.throughput(Throughput::Elements(1));

    // A taker that crosses 32 resting makers in one submit.
    group.bench_function("submit_crossing_taker", |b| {
        b.iter_batched(
            || resting_book(2000),
            |(mut book, mut out)| {
                out.clear();
                let taker = ord(1_000_000, B, 32, A, 32);
                black_box(book.submit(black_box(taker), &mut out));
            },
            BatchSize::SmallInput,
        );
    });

    // A taker that does not cross and simply rests.
    group.bench_function("submit_resting", |b| {
        b.iter_batched(
            || resting_book(2000),
            |(mut book, mut out)| {
                out.clear();
                // sells A for B too, so it joins the same side instead of crossing
                let resting = ord(2_000_000, A, 5, B, 9);
                black_box(book.submit(black_box(resting), &mut out));
            },
            BatchSize::SmallInput,
        );
    });

    group.finish();
}

criterion_group!(benches, bench_matching);
criterion_main!(benches);
