# Architecture

This is a stub. It grows alongside the build and records the design decisions that are the real signal of the project.

Crossbook has four planes. The matching core is pure and deterministic. Everything that does I/O lives outside it.

Decisions to write up here as they land:

Noncustodial allowance pull. Traders never deposit into the engine. They approve the settlement contract once, and the contract pulls tokens at execution. Scope is standard ERC-20 tokens. Fee on transfer and rebasing tokens break net zero settlement, so the engine rejects them at intake and the contract would revert through its net flow check.

Pure single writer matching core. The hot path is one task that owns the book. It is a deterministic function of the book state and the ordered inputs. This is the LMAX style design.

Determinism and event sourcing. Because the core is pure, a recorded input sequence replays to identical output. That powers the golden replay tests and makes the engine easy to debug.

Onchain re-verification. The contract does not trust the engine. It rechecks signatures, the nonce field, expiry, and each order's limit price, so a buggy or malicious matcher cannot move funds against a maker's signed limits.

Schema parity. The Rust and Solidity EIP-712 digests are byte for byte identical, and the M2 test proves it before any settlement work begins.

Honest trust assumption. The MVP uses a single permissioned solver. The threat model spells out exactly what that solver can and cannot do. The decentralized path, with competing solvers and onchain surplus checks, is sketched but out of scope.

Reorg handling for the indexer. Index a few confirmations behind the head, store the last block hash, and roll back on a mismatch.
