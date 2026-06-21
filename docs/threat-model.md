# Threat Model

> Stub — completed at M3/M5. This is a primary portfolio artifact for security-conscious
> DeFi teams; treat it as a deliverable, not an afterthought.

Threats to analyze, with mitigations:

- **Signature replay** — nonces + `validTo` expiry; nonces marked used before transfers.
- **MEV / front-running of the public `settle` tx** — acknowledge; note private mempools
  / batch privacy as mitigations.
- **Malicious or buggy solver** — on-chain re-verification of signatures, nonces, expiry,
  and each maker's limit price; net-zero inventory check.
- **Reentrancy** — `ReentrancyGuard` + checks-effects-interactions.
- **Griefing via stale allowances / fee-on-transfer tokens** — validation at intake +
  revert-safe settlement; document token assumptions.
