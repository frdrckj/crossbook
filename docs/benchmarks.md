# Benchmarks

These measure the pure matching core only, not end to end settlement. They are
criterion microbenchmarks of `OrderBook::submit`. Network, signature checks,
database writes, and chain submission are all out of scope here.

Reproduce with `just bench` (or `cargo bench -p crossbook-core`).

## Method

- Machine: Apple M4, 10 cores, macOS 26.5.
- Toolchain: rustc 1.94.1, release profile (thin LTO, one codegen unit).
- criterion with a 1 second warm up and a 4 second measurement window.
- Two scenarios, each starting from a book of 2000 resting makers at one price:
  - `submit_crossing_taker`: a taker that crosses and fully fills 32 makers in a
    single submit.
  - `submit_resting`: an order that does not cross and simply rests.

The reused fill buffer means the crossing path does no per submit allocation; a
separate test (`tests/allocation.rs`) asserts zero allocations on that path with a
counting allocator.

## Results (as of 2026-06)

| Scenario | Median per submit | Notes |
| --- | --- | --- |
| Crossing taker (32 fills) | about 5.3 microseconds | about 165 nanoseconds per fill, including the 512 bit price arithmetic |
| Resting insert | about 356 nanoseconds | roughly 2.8 million resting submits per second |

The crossing path sustains on the order of 6 million fills per second on this
machine. Numbers vary with hardware; rerun `just bench` for your own.

## Settlement gas

From the Foundry gas report (`forge test --gas-report`) for a two order balanced
clearing:

| Entry point | Gas | Notes |
| --- | --- | --- |
| `settle` | about 165k | continuous mode, one balanced pair |
| `settleBatch` | about 201k | the same pair, plus the uniform price assertion, the price set validation, and the BatchSettled event, about 36k more |

A ring settles through `settle`, so it carries the same per fill cost as a
continuous settlement of the same number of legs.

## Differential fuzz

`just e2e` runs a cross implementation differential test
(`crates/crossbook-engine/tests/differential.rs`): it generates random partially
fillable orders over three tokens, clears them with the real `run_auction` and
`find_ring`, and submits the exact settlement calldata to a live contract on
Anvil. A representative run cleared 27 pair clearings and 15 rings across 30
random batches, every one accepted on chain, with the contract holding zero
inventory after each. A batch the core produces that the contract would reject is
a differential bug, which this test exists to catch.
