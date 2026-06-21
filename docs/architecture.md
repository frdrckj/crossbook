# Architecture

> Stub — fleshed out alongside the build. Captures the design decisions that are the
> "signal" of this project.

Crossbook has four planes; the matching core is pure and deterministic, everything with
I/O lives outside it.

Key decisions to document here as they are implemented:

- **Non-custodial / allowance-pull** — traders never deposit; settlement pulls at execution.
- **Pure, single-writer matching core** — LMAX-style hot path; deterministic function.
- **Determinism + event sourcing** — recorded inputs replay to identical output.
- **On-chain re-verification** — the contract does not trust the engine.
- **Schema parity is sacred** — Rust and Solidity EIP-712 digests are byte-identical.
- **Trust assumption, stated honestly** — MVP uses a single permissioned solver; the
  decentralized path (competing solvers, on-chain limit/surplus checks) is sketched but
  out of scope.
