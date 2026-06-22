//! The single writer engine task.
//!
//! One task owns the OrderBook and is the only code that touches it. Other tasks
//! talk to it over a bounded command channel, each command carrying a oneshot
//! sender for its reply. The read path (book snapshots) does not go through the
//! channel: the writer publishes an immutable snapshot into a watch channel that
//! readers load without locking the writer.

use crate::config::MatchingMode;
use crossbook_core::auction::{run_auction, AuctionFill, AuctionResult};
use crossbook_core::book::OrderBook;
use crossbook_core::ring::{find_ring, RingResult};
use crossbook_core::types::{Fill, OpenOrder, OrderHash, SubmitOutcome};
use std::sync::Arc;
use tokio::sync::{mpsc, oneshot, watch};

/// An immutable view of the resting book, cheap to clone.
pub type BookView = Arc<Vec<OpenOrder>>;

/// What a closed batch window produced: the per pair clearings and any multi
/// token rings extracted from what the pairs left behind.
#[derive(Debug, Default)]
pub struct CloseOutcome {
    pub pairs: Vec<AuctionResult>,
    pub rings: Vec<RingResult>,
}

#[derive(Debug)]
pub struct SubmitResult {
    pub outcome: SubmitOutcome,
    pub fills: Vec<Fill>,
}

#[derive(Debug, PartialEq, Eq)]
pub enum EngineError {
    /// The command queue is full. The API layer turns this into a 503.
    Backpressure,
    /// The engine task is gone.
    Closed,
}

enum Command {
    Submit {
        order: Box<OpenOrder>,
        reply: oneshot::Sender<SubmitResult>,
    },
    Cancel {
        hash: OrderHash,
        reply: oneshot::Sender<bool>,
    },
    CloseBatch {
        now: u64,
        reply: oneshot::Sender<CloseOutcome>,
    },
}

/// A cloneable handle to the engine task.
#[derive(Clone)]
pub struct EngineHandle {
    tx: mpsc::Sender<Command>,
    book: watch::Receiver<BookView>,
}

impl EngineHandle {
    /// Submit an order, awaiting a free slot if the queue is full.
    pub async fn submit(&self, order: OpenOrder) -> Result<SubmitResult, EngineError> {
        let (reply, rx) = oneshot::channel();
        self.tx
            .send(Command::Submit {
                order: Box::new(order),
                reply,
            })
            .await
            .map_err(|_| EngineError::Closed)?;
        rx.await.map_err(|_| EngineError::Closed)
    }

    /// Submit without blocking. Returns Backpressure when the queue is full, so
    /// the caller can reject the request instead of stalling the writer.
    pub fn try_submit(
        &self,
        order: OpenOrder,
    ) -> Result<oneshot::Receiver<SubmitResult>, EngineError> {
        let (reply, rx) = oneshot::channel();
        self.tx
            .try_send(Command::Submit {
                order: Box::new(order),
                reply,
            })
            .map_err(|e| match e {
                mpsc::error::TrySendError::Full(_) => EngineError::Backpressure,
                mpsc::error::TrySendError::Closed(_) => EngineError::Closed,
            })?;
        Ok(rx)
    }

    /// Cancel a resting order by hash. Returns whether it was present.
    pub async fn cancel(&self, hash: OrderHash) -> Result<bool, EngineError> {
        let (reply, rx) = oneshot::channel();
        self.tx
            .send(Command::Cancel { hash, reply })
            .await
            .map_err(|_| EngineError::Closed)?;
        rx.await.map_err(|_| EngineError::Closed)
    }

    /// Close the current batch window: clear the collected orders at a uniform
    /// price per pair, extract any multi token rings from what remains, advance the
    /// remaining quantities, and return both. In continuous mode there is no
    /// buffer, so this returns an empty outcome.
    pub async fn close_batch(&self, now: u64) -> Result<CloseOutcome, EngineError> {
        let (reply, rx) = oneshot::channel();
        self.tx
            .send(Command::CloseBatch { now, reply })
            .await
            .map_err(|_| EngineError::Closed)?;
        rx.await.map_err(|_| EngineError::Closed)
    }

    /// The current resting book, read without locking the writer.
    pub fn snapshot(&self) -> BookView {
        self.book.borrow().clone()
    }
}

/// Spawn the writer task with a bounded command queue, returning a handle.
///
/// In continuous mode the task crosses each order against the book on arrival. In
/// batch mode it instead buffers orders and clears them on `close_batch`. The two
/// modes never run at once; the snapshot exposes the resting book in continuous
/// mode and the collected orders in batch mode.
pub fn spawn(capacity: usize, mode: MatchingMode) -> EngineHandle {
    let (tx, mut rx) = mpsc::channel::<Command>(capacity);
    let (book_tx, book_rx) = watch::channel::<BookView>(Arc::new(Vec::new()));

    tokio::spawn(async move {
        let mut book = OrderBook::new();
        let mut buffer: Vec<OpenOrder> = Vec::new();
        let mut fills = Vec::new();
        while let Some(cmd) = rx.recv().await {
            match cmd {
                Command::Submit { order, reply } => match mode {
                    MatchingMode::Continuous => {
                        fills.clear();
                        let outcome = book.submit(*order, &mut fills);
                        let result = SubmitResult {
                            outcome,
                            fills: fills.clone(),
                        };
                        // Publish the snapshot before replying, so a caller that
                        // has its reply always sees an up to date book.
                        let _ = book_tx.send(Arc::new(book.resting_orders()));
                        let _ = reply.send(result);
                    }
                    MatchingMode::Batch => {
                        // Hold the order until the window closes; nothing fills yet.
                        buffer.push(*order);
                        let _ = book_tx.send(Arc::new(buffer.clone()));
                        let _ = reply.send(SubmitResult {
                            outcome: SubmitOutcome::Resting,
                            fills: Vec::new(),
                        });
                    }
                },
                Command::Cancel { hash, reply } => {
                    let removed = match mode {
                        MatchingMode::Continuous => book.cancel(&hash),
                        MatchingMode::Batch => {
                            let before = buffer.len();
                            buffer.retain(|o| o.hash != hash);
                            buffer.len() != before
                        }
                    };
                    let snapshot = match mode {
                        MatchingMode::Continuous => book.resting_orders(),
                        MatchingMode::Batch => buffer.clone(),
                    };
                    let _ = book_tx.send(Arc::new(snapshot));
                    let _ = reply.send(removed);
                }
                Command::CloseBatch { now, reply } => {
                    // Drop expired orders, clear each pair, then look for multi
                    // token rings in what the pairs left behind, then advance the
                    // remaining amounts.
                    buffer.retain(|o| o.order.valid_to > now);
                    let pairs = run_auction(&buffer);
                    for r in &pairs {
                        apply_fills(&mut buffer, &r.fills);
                    }
                    let mut rings = Vec::new();
                    // Bounded by the buffer size: each ring consumes some quantity.
                    for _ in 0..buffer.len() {
                        match find_ring(&buffer) {
                            Some(r) => {
                                apply_fills(&mut buffer, &r.fills);
                                rings.push(r);
                            }
                            None => break,
                        }
                    }
                    buffer.retain(|o| !o.remaining_sell.is_zero());
                    let _ = book_tx.send(Arc::new(buffer.clone()));
                    let _ = reply.send(CloseOutcome { pairs, rings });
                }
            }
        }
    });

    EngineHandle { tx, book: book_rx }
}

/// Subtract each fill's sell amount from its order's remaining, so a partly filled
/// order rolls into the next window at its reduced size.
fn apply_fills(buffer: &mut [OpenOrder], fills: &[AuctionFill]) {
    for f in fills {
        if let Some(o) = buffer.iter_mut().find(|o| o.hash == f.order_hash) {
            o.remaining_sell = o.remaining_sell.saturating_sub(f.sell_filled);
        }
    }
}
