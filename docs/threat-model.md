# Threat model

This is a stub. It is finished at M3 and M5. Treat it as a real deliverable, since the security writeup is the strongest part of this portfolio for a security focused DeFi team.

The plan is to make every entry concrete. For each threat, write a Foundry test against a deliberately broken variant that fails, then show the fixed contract passing.

Threats to cover, with their mitigations:

Signature replay. The nonce field, the `validTo` expiry, and the cumulative `filledSell` accounting prevent it. Fill state is updated before transfers (checks, effects, interactions).

Cross domain replay. An order signed for one chain id or verifying contract must revert on another.

Signature malleability. OpenZeppelin `ECDSA` rejects high `s` values and the zero address. The engine applies the same rule, so it never admits an order the contract will reject.

Self trade and wash trading. Documented policy plus a test.

Approval front running and griefing. A maker who calls `approve(0)` after admission should not be able to wedge the whole batch. The solver re-simulates against current chain state right before submission, and submits through a private path.

MEV and front running of the public settle transaction. Acknowledged. Private mempools and batch privacy are the mitigations.

Malicious or buggy solver. Bounded trust. The contract rechecks signatures, expiry, the fill bound, and the limit price, and requires net zero inventory. A leaked solver key is handled with a pause switch and a solver rotation function.

Reentrancy. A reentrancy guard plus the checks, effects, interactions order. Include a proof of concept that double spends with the order reversed, then the guarded version passing.

Stale allowance and nonstandard tokens. Validate at intake, revert safely at settlement, and restrict the venue to standard ERC-20 tokens.
