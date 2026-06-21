# Threat model

The settlement contract is the trust backstop, not the engine. The engine is a
convenience: it matches orders and submits batches, but it cannot move funds
against a maker's signed limits, because the contract rechecks everything on
chain. Each threat below names the mitigation and the test that demonstrates it,
so the claims are backed by code rather than prose.

Contract tests live in `contracts/test/Settlement.t.sol` and
`contracts/test/Settlement.invariant.t.sol`. Engine and matcher tests live under
`crates/`.

## Forged or wrong signature

A batch can only move a maker's funds if the order recovers to that maker. The
contract recomputes the EIP-712 digest and uses OpenZeppelin `ECDSA.recover`,
requiring the recovered address to equal `order.maker`.

- `Settlement.t.sol::test_RevertWhen_BadSignature`
- `ingest::tests::tampered_signature_is_rejected`
- `ingest::tests::signature_by_another_key_is_rejected`

The Rust and Solidity digests are proven byte identical by the parity gate
(`crates/crossbook-core/tests/eip712_parity.rs` and
`contracts/test/Eip712Parity.t.sol`), so an order the engine admits is the same
order the contract verifies.

## Signature malleability

OpenZeppelin `ECDSA.recover` rejects high `s` values and the zero address, so a
malleated signature does not recover to a different valid signer. The engine
applies the same acceptance rule, so it never admits an order the contract would
reject.

## Expiry

Every order carries `validTo`. The contract requires `block.timestamp <= validTo`.

- `Settlement.t.sol::test_RevertWhen_Expired`
- `ingest::tests::static_checks_catch_expiry_and_degenerate_orders`

## Replay and overfill

Fill state is cumulative, keyed by the order hash (`filledSell[orderHash]`, the
CoW filledAmount pattern), and updated before any transfer (checks, effects,
interactions). A fully filled order cannot fill again, a partially filled order
cannot exceed its `sellAmount`, and a fill or kill order fills exactly once.

- `Settlement.t.sol::test_RevertWhen_Overfill`
- `Settlement.t.sol::test_PartialFillAccumulates`
- `Settlement.t.sol::test_RevertWhen_PartialOnFillOrKill`
- the invariant suite asserts cumulative fills never exceed `sellAmount`

A maker can also invalidate an order with `cancel`, after which it cannot settle.

- `Settlement.t.sol::test_RevertWhen_OrderCancelled`

## A solver moving funds against a maker's limit

This is the central risk and the reason the contract exists. For every fill the
contract requires `buyFilled * sellAmount >= sellFilled * buyAmount`, cross
multiplied and widened to 512 bits so it cannot overflow or spuriously revert on
extreme amounts. A fill one wei below the maker's limit reverts the whole batch.

- `Settlement.t.sol::test_RevertWhen_BelowLimitPrice`
- the fuzz test asserts the realized price respects the limit across random amounts
- the matcher upholds the same limit cumulatively, with rounding in the maker's
  favor (`crates/crossbook-core/tests/invariants.rs`)

## The contract holding or stealing inventory

The contract is non custodial. It tracks net flow per token during a settlement
and requires every touched token to net to zero, so it receives exactly what it
sends and never keeps a balance.

- `Settlement.t.sol::test_RevertWhen_InventoryNotZero`
- `Settlement.invariant.t.sol::invariant_SettlementHoldsNoInventory` (zero balance
  held across 8192 random settlements)

## Reentrancy

`settle` is `nonReentrant` and follows checks, effects, interactions. A malicious
token that tries to reenter during the transfer phase causes the whole batch to
revert.

- `Settlement.t.sol::test_ReentrancyIsBlocked`

## Fee on transfer and nonstandard tokens

The venue targets standard ERC-20 tokens. A fee on transfer token delivers less
than stated, which breaks the net zero requirement and reverts the batch cleanly
rather than leaving the contract short.

- `Settlement.t.sol::test_RevertWhen_FeeOnTransferToken`

## Solver key compromise

The solver is a single permissioned address in the MVP, which is a centralization
point stated honestly. The owner can rotate the solver and can pause settlement
to respond to a key compromise. Maker funds at risk are bounded by their
outstanding allowance, since the contract only ever pulls up to what was approved.

- `Settlement.t.sol::test_SetSolverRotatesAccess`
- `Settlement.t.sol::test_RevertWhen_Paused`
- `Settlement.t.sol::test_RevertWhen_SetSolverByNonOwner`

## What the solver can and cannot do

Can: pick the execution price within each maker's limit, capture the spread,
choose which orders to include, and choose ordering. Cannot: forge signatures,
exceed `validTo`, replay or overfill, settle below a maker's limit, or strand the
contract with inventory. The contract enforces every item in the second list.

## Acknowledged, not yet fully mitigated

- **MEV and front running of the public settle transaction.** The settle call is
  public on chain and can be observed. Production mitigations are private mempools
  or bundle submission and batch privacy.
- **Intake versus settlement timing.** Off chain intake validation (balance and
  allowance) is a best effort admission hint, separate in time from the on chain
  pull. Wholesale revert keeps funds safe, but a maker who revokes an allowance
  after admission can make a batch revert. The intended mitigation is for the
  solver to re simulate the batch against current chain state immediately before
  submission and to drop or rematch orders that no longer validate.
- **Cross domain replay.** The digest binds the chain id and the verifying
  contract, so an order signed for one deployment does not recover on another. A
  dedicated negative test for this is a worthwhile addition.
- **Reorgs.** The indexer stores the last block hash so a reorg can be detected.
  Rollback is left for production, since the MVP targets a local Anvil that does
  not reorg.
