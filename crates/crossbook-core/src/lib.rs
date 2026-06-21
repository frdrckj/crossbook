//! # crossbook-core
//!
//! The pure matching core of Crossbook. By design this crate has **no async, no
//! I/O, no clock, and no rng** — matching is a deterministic function of
//! `(book_state, ordered_inputs) -> (new_state, trades)`. That purity is what
//! makes it fast (single-writer hot path) and testable (golden-replay + proptest).
//!
//! Milestone status: **M0 scaffold** — types, book, and matcher land in M1.

// ponytail: intentionally empty until M1. One smoke test below proves the
// crate + test harness compile in CI on the empty scaffold (M0 acceptance).
#[cfg(test)]
mod tests {
    #[test]
    fn scaffold_builds() {
        assert_eq!(2 + 2, 4);
    }
}
