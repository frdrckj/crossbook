//! Crossbook engine internals, exposed as a library so the binary and the tests
//! share them.

pub mod chain;
pub mod db;
pub mod engine_task;
pub mod ingest;
pub mod reject;
pub mod settle;
