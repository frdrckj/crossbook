//! The single writer engine task.
//!
//! One task owns the OrderBook and is the only code that touches it. Other tasks
//! talk to it over a bounded command channel, each command carrying a oneshot
//! sender for its reply. The read path (book snapshots) does not go through the
//! channel: the writer publishes an immutable snapshot into a watch channel that
//! readers load without locking the writer.

use crossbook_core::book::OrderBook;
use crossbook_core::types::{Fill, OpenOrder, OrderHash, SubmitOutcome};
use std::sync::Arc;
use tokio::sync::{mpsc, oneshot, watch};

/// An immutable view of the resting book, cheap to clone.
pub type BookView = Arc<Vec<OpenOrder>>;

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

    /// The current resting book, read without locking the writer.
    pub fn snapshot(&self) -> BookView {
        self.book.borrow().clone()
    }
}

/// Spawn the writer task with a bounded command queue, returning a handle.
pub fn spawn(capacity: usize) -> EngineHandle {
    let (tx, mut rx) = mpsc::channel::<Command>(capacity);
    let (book_tx, book_rx) = watch::channel::<BookView>(Arc::new(Vec::new()));

    tokio::spawn(async move {
        let mut book = OrderBook::new();
        let mut fills = Vec::new();
        while let Some(cmd) = rx.recv().await {
            match cmd {
                Command::Submit { order, reply } => {
                    fills.clear();
                    let outcome = book.submit(*order, &mut fills);
                    let result = SubmitResult {
                        outcome,
                        fills: fills.clone(),
                    };
                    // Publish the snapshot before replying, so a caller that has
                    // its reply always sees an up to date book.
                    let _ = book_tx.send(Arc::new(book.resting_orders()));
                    let _ = reply.send(result);
                }
                Command::Cancel { hash, reply } => {
                    let removed = book.cancel(&hash);
                    let _ = book_tx.send(Arc::new(book.resting_orders()));
                    let _ = reply.send(removed);
                }
            }
        }
    });

    EngineHandle { tx, book: book_rx }
}
