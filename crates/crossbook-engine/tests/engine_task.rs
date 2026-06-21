//! Tests for the single writer engine task: submit, cancel, snapshot, and that
//! concurrent submits from many tasks all land.

use alloy_primitives::{Address, U256};
use crossbook_core::types::{OpenOrder, Order, SubmitOutcome};
use crossbook_engine::engine_task;

const A: u8 = 0x0A;
const B: u8 = 0x0B;

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
    let engine = engine_task::spawn(64);

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
    let engine = engine_task::spawn(64);
    engine.submit(order(0, 1, A, 100, B, 100)).await.unwrap();
    assert!(engine.cancel([1; 32]).await.unwrap());
    assert_eq!(engine.snapshot().len(), 0);
    assert!(!engine.cancel([1; 32]).await.unwrap());
}

#[tokio::test]
async fn concurrent_submits_all_land() {
    let engine = engine_task::spawn(256);
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
