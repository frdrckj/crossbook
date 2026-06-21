# Crossbook

Crossbook is a noncustodial hybrid decentralized exchange. Traders sign orders offchain as gasless intents, a Rust matching engine crosses them, and an onchain settlement contract checks the signatures and swaps the tokens in one atomic transaction.

Funds stay in the trader's wallet until the moment of execution. A maker grants the settlement contract an ERC-20 allowance once, and the contract pulls tokens only when it settles, after it has independently rechecked every signature, nonce, expiry, and limit price onchain. The matching core is pure and deterministic. It has no async, no I/O, and no clock, and a single writer task owns it. That is what makes it both fast and easy to test exhaustively.

## Status

M0 (scaffolding) is done. The project is built one milestone at a time.

| Milestone | Scope | State |
| --- | --- | --- |
| M0 | Workspace, CI, Foundry skeleton, local devnet | done |
| M1 | Pure matching core (types, book, matcher) with property tests and benchmarks | in progress |
| M2 | EIP-712 digest parity between Rust and Solidity (the gate) | todo |
| M3 | CrossbookSettlement.sol with unit, fuzz, and invariant suites | todo |
| M4 | Engine service (axum REST and WebSocket, ingest, settle, indexer, metrics) | todo |
| M5 | CLI, end to end test, and the full README, threat model, and benchmarks | todo |
| M6 | Stretch: batch auction with a uniform clearing price (toward CoW) | todo |
| M7 | Stretch: perps margin and liquidation risk engine (toward Curated) | todo |

## Stack

Rust on Tokio. Alloy for Ethereum, not `ethers-rs`, which is deprecated. axum for the API. sqlx with PostgreSQL. Foundry and Solidity 0.8.24 with OpenZeppelin. proptest and criterion for tests and benchmarks. tracing with a Prometheus metrics endpoint.

## Layout

```
crates/crossbook-core     pure matching engine (no I/O, no async)
crates/crossbook-engine   Tokio service: REST and WebSocket, ingest, settle, indexer
crates/crossbook-cli      client: approve, sign, submit, query
contracts/                Foundry project: CrossbookSettlement and OrderLib
docs/                     architecture, order schema, threat model, benchmarks
```

## Develop

```sh
just            # list tasks
just check      # fmt, clippy, cargo test, forge test (the CI gate)
just dev        # docker compose up: Postgres and Anvil
just bench      # matching core benchmarks (M1 onward)
```

You need the Rust stable toolchain, Foundry, `just`, and Docker. Copy `.env.example` to `.env` for local devnet keys. Those are Anvil test keys only.

## License

MIT.
