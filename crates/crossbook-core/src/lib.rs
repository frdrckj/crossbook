//! The pure matching core of Crossbook.
//!
//! The crate does no I/O and keeps no clock. Matching is a deterministic function
//! of the book state and the ordered inputs, which is what makes the single writer
//! hot path fast and the engine easy to test with golden replays and property
//! tests.

pub mod auction;
pub mod book;
pub mod eip712;
pub mod error;
pub mod price;
pub mod types;
