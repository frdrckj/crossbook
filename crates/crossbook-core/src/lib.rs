//! # crossbook-core
//!
//! The pure matching core of Crossbook. By design this crate has **no async, no
//! I/O, no clock, and no rng** — matching is a deterministic function of
//! `(book_state, ordered_inputs) -> (new_state, trades)`. That purity is what
//! makes it fast (single-writer hot path) and testable (golden-replay + proptest).
//!
//! Milestone status: M1, pure matching core under construction.

pub mod book;
pub mod error;
pub mod price;
pub mod types;
