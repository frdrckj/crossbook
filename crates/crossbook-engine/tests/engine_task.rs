//! Tests for the single writer engine task: submit, cancel, snapshot, and that
//! concurrent submits from many tasks all land.

use alloy_primitives::{Address, U256};
use crossbook_core::types::{OpenOrder, Order, SubmitOutcome};
use crossbook_engine::config::MatchingMode;
use crossbook_engine::engine_task;

const A: u8 = 0x0A;
const B: u8 = 0x0B;
const C: u8 = 0x0C;

fn order(seq: u64, hash: u8, st: u8, sa: u64, bt: u8, ba: u64) -> OpenOrder {
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

#[tokio::test]
async fn submit_rests_then_crosses_and_snapshot_tracks_it() {
    let engine = engine_task::spawn(64, MatchingMode::Continuous);

    let r0 = engine.submit(order(0, 1, A, 100, B, 100)).await.unwrap();
    assert_eq!(r0.outcome, SubmitOutcome::Resting);
    assert!(r0.fills.is_empty());
    assert_eq!(engine.snapshot().len(), 1);

    let r1 = engine.submit(order(1, 2, B, 100, A, 100)).await.unwrap();
    assert_eq!(r1.outcome, SubmitOutcome::FullyFilled);
    assert_eq!(r1.fills.len(), 1);
    assert_eq!(engine.snapshot().len(), 0);
}

#[tokio::test]
async fn cancel_removes_a_resting_order() {
    let engine = engine_task::spawn(64, MatchingMode::Continuous);
    engine.submit(order(0, 1, A, 100, B, 100)).await.unwrap();
    assert!(engine.cancel([1; 32]).await.unwrap());
    assert_eq!(engine.snapshot().len(), 0);
    assert!(!engine.cancel([1; 32]).await.unwrap());
}

#[tokio::test]
async fn batch_mode_collects_then_clears_on_close() {
    let engine = engine_task::spawn(64, MatchingMode::Batch);

    // Offsetting one to one orders just rest into the buffer; nothing fills yet.
    let ask = engine.submit(order(0, 1, A, 100, B, 100)).await.unwrap();
    let bid = engine.submit(order(1, 2, B, 100, A, 100)).await.unwrap();
    assert_eq!(ask.outcome, SubmitOutcome::Resting);
    assert!(ask.fills.is_empty());
    assert_eq!(bid.outcome, SubmitOutcome::Resting);
    assert_eq!(engine.snapshot().len(), 2); // both collected, awaiting the window

    // Closing the window clears the pair at one to one and empties the buffer.
    let outcome = engine.close_batch(u64::MAX - 1).await.unwrap();
    assert_eq!(outcome.pairs.len(), 1);
    assert_eq!(outcome.pairs[0].volume_base, U256::from(100u64));
    assert!(outcome.rings.is_empty());
    assert_eq!(engine.snapshot().len(), 0); // both fully spent, nothing rolls over
}

#[tokio::test]
async fn batch_mode_rolls_unmatched_remainder() {
    let engine = engine_task::spawn(64, MatchingMode::Batch);
    // Ask offers 100 base, bid only wants 40 base, both at one to one. 60 base of
    // the ask should roll into the next window; the bid spends fully and leaves.
    engine.submit(order(0, 1, A, 100, B, 100)).await.unwrap();
    engine.submit(order(1, 2, B, 40, A, 40)).await.unwrap();

    let outcome = engine.close_batch(u64::MAX - 1).await.unwrap();
    assert_eq!(outcome.pairs[0].volume_base, U256::from(40u64));
    let rest = engine.snapshot();
    assert_eq!(rest.len(), 1);
    assert_eq!(rest[0].hash, [1; 32]);
    assert_eq!(rest[0].remaining_sell, U256::from(60u64));
}

#[tokio::test]
async fn batch_mode_clears_a_three_token_ring() {
    let engine = engine_task::spawn(64, MatchingMode::Batch);
    // A -> B -> C -> A: no two orders share a pair, but the ring crosses.
    engine.submit(order(0, 1, A, 100, B, 90)).await.unwrap();
    engine.submit(order(1, 2, B, 90, C, 80)).await.unwrap();
    engine.submit(order(2, 3, C, 80, A, 70)).await.unwrap();

    let outcome = engine.close_batch(u64::MAX - 1).await.unwrap();
    assert!(outcome.pairs.is_empty()); // nothing clears as a pair
    assert_eq!(outcome.rings.len(), 1);
    assert_eq!(outcome.rings[0].tokens.len(), 3);
}

#[tokio::test]
async fn concurrent_submits_all_land() {
    let engine = engine_task::spawn(256, MatchingMode::Continuous);
    // 50 makers on the same side, submitted from concurrent tasks. None cross, so
    // all rest. The single writer serializes them safely.
    let mut handles = Vec::new();
    for i in 0..50u64 {
        let e = engine.clone();
        handles.push(tokio::spawn(async move {
            let hash = (i + 1) as u8;
            e.submit(order(i, hash, A, 10 + i, B, 10)).await.unwrap()
        }));
    }
    for h in handles {
        h.await.unwrap();
    }
    assert_eq!(engine.snapshot().len(), 50);
}
