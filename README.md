# Crossbook

> A non-custodial hybrid decentralized exchange: traders sign orders off-chain as
> gasless intents, a Rust matching engine crosses them, and an on-chain settlement
> contract verifies the signatures and swaps the tokens atomically.

Funds never leave the trader's wallet until execution. Makers grant the settlement
contract an ERC-20 allowance once; the contract pulls tokens only at settlement, after
independently re-verifying every signature, nonce, and expiry on-chain. The matching
core is a pure, single-writer, deterministic engine (LMAX-style hot path) — no async,
no I/O, no clock — which makes it both fast and exhaustively testable.

## Status

🚧 **M0 — scaffolding.** Built milestone-by-milestone; see the roadmap below.

| Milestone | Scope | State |
| --- | --- | --- |
| M0 | Workspace, CI, Foundry skeleton, devnet compose | ✅ |
| M1 | Pure matching core (types, book, matcher) + proptest + criterion | ⬜ |
| M2 | EIP-712 Rust↔Solidity digest parity (gate) | ⬜ |
| M3 | `CrossbookSettlement.sol` + Foundry unit/fuzz/invariant suites | ⬜ |
| M4 | Engine service (axum REST/WS, ingest, settle, indexer, metrics) | ⬜ |
| M5 | CLI + end-to-end + full README/threat-model/benchmarks | ⬜ |
| M6 | Stretch: batch auction, uniform clearing price (→ CoW) | ⬜ |
| M7 | Stretch: perps margin + liquidation engine (→ Curated) | ⬜ |

## Stack

Rust (Tokio) · [Alloy](https://github.com/alloy-rs/alloy) (not ethers-rs) · axum ·
sqlx + PostgreSQL · Foundry / Solidity `^0.8.24` + OpenZeppelin · proptest · criterion ·
tracing + Prometheus.

## Layout

```
crates/crossbook-core     pure matching engine (no I/O, no async)
crates/crossbook-engine   Tokio service: REST/WS, ingest, settle, indexer
crates/crossbook-cli      client: approve, sign, submit, query
contracts/                Foundry project: CrossbookSettlement + OrderLib
docs/                     architecture · order-schema · threat-model · benchmarks
```

## Develop

```sh
just            # list tasks
just check      # fmt + clippy + cargo test + forge test  (the CI gate)
just dev        # docker compose up: Postgres + Anvil
just bench      # matching-core microbenchmarks (M1+)
```

Requires the Rust stable toolchain, [Foundry](https://book.getfoundry.sh/), `just`, and
Docker. Copy `.env.example` to `.env` for local devnet keys (Anvil test keys only).

## License

MIT.
