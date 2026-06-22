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

## Batch auction

Batch mode adds the `settleBatch` entrypoint, which shares `settle`'s internal verification and adds a uniform price per pair. Everything above still holds for it, since it routes through the same `_settle`. The items here are specific to the batch path, and a security review of it produced them, so they name what is enforced, what is only a solver policy, and where the honest gaps are.

### Uniform price per pair, enforced on chain

Every fill in a token pair must execute at exactly that pair's clearing price, checked as an exact 512 bit ratio in `_assertUniformPrice` before any funds move. The price set is also validated: an empty batch, a non positive price, a duplicate price for a pair, and a price that cleared no volume all revert, so the `BatchSettled` stream is well formed and a solver cannot smuggle a second unenforced price for a pair.

- `SettlementBatch.t.sol::test_RevertWhen_FillDeviatesFromClearingPrice`
- `SettlementBatch.t.sol::test_RevertWhen_EmptyBatch`, `test_RevertWhen_DegenerateClearingPrice`, `test_RevertWhen_DuplicateClearingPrice`, `test_RevertWhen_StaleClearingPrice`, `test_RevertWhen_ClearingPriceMissing`

### What the chain enforces versus what the solver chooses

This is the most important honesty point for the batch. The chain enforces two things: one uniform price per pair, and each maker's own signed limit (the same `_ge512` check as `settle`). It does not enforce that the price is the midpoint, that it is central in the overlap, or that the batch cleared the maximal crossable volume. Those are properties of the off chain matcher in `auction.rs`. So against a fully malicious solver the worst case for a maker is bounded but real: it can be denied the surplus a fair midpoint would have given it, or left out of a thinner clearing, but it can never be filled below its own signed limit and the contract can never be left holding inventory. Fair price is an off chain property, uniform price and limits are on chain.

- per maker limit floor: `Settlement.t.sol::test_RevertWhen_BelowLimitPrice`
- off chain midpoint and maximal volume: `crates/crossbook-core/tests/auction_props.rs::auction_upholds_all_invariants`
- the structural fix for solver discretion (must demonstrate maximality or be beaten) only arrives under competing solvers, which is out of scope and noted in the solver section above

### Fill or kill is not supported in batch mode

A uniform price batch clears in whole lots of the clearing price, so it cannot in general honor a fill or kill order's exact signed amount (the contract requires `sellFilled == sellAmount` for such an order). Rather than silently mishandle it, the engine rejects fill or kill orders at intake in batch mode, and the matcher skips them defensively so its output never asks the contract to partially fill one. Partially fillable orders are the supported shape for batch matching.

- intake rejection: `RejectReason::FillOrKillNotInBatch`
- core skip: `crates/crossbook-core/tests/auction.rs::fill_or_kill_orders_do_not_participate_in_the_batch` and the `auction_props` invariant that no fill or kill order ever appears in a batch fill

### Net zero is global across the batch, by design

The contract's net flow check sums inflow and outflow per token across the whole `settleBatch`, not per pair. Honest batches are per pair balanced, since the matcher clears each pair independently, so they satisfy the global check trivially. The global scope is deliberate: it is what will let a future multi token ring (token A to B to C to A) net to zero across three tokens that no single pair balances. A consequence to state plainly: a multi pair batch that shares a token can net to zero globally while individual pairs are imbalanced. No maker is filled below its limit, so this moves no funds against anyone, but the per pair self contained framing is an off chain matcher property, not an on chain one.

### Optimistic settlement and reconciliation drift

In batch mode the engine decrements each order's remaining amount and drops fully filled orders from its buffer at window close, before the `settleBatch` transaction confirms, and the submission is fire and forget (a failure is only logged). If that transaction reverts or never lands, on chain `filledSell` never moved but the engine has already debited its view, so the maker is under offered, or dropped, in the next window until it reposts. There is no channel feeding the indexed events back into the engine to correct this. Funds stay safe, since the contract's cumulative `filledSell` cap, per maker limit, and net zero checks mean a stale over offer simply reverts the next batch rather than mis settling, so this is a liveness and consistency gap, not a loss of funds. The intended fix is to make the indexed `BatchSettled` and `Trade` events the source of truth: build the batch without mutating remainders, and apply the decrement only on confirmation.

- not yet mitigated; no test. Related to but distinct from the intake versus settlement timing item below.

### Whole batch revert denies the window

The engine submits one `settleBatch` per window covering every pair that cleared. So a single un settleable order, one that expired between window close and mining, was cancelled on chain directly, or revoked its allowance, reverts the whole window and denies every other pair too. Funds stay safe (wholesale revert), but the window is lost. The fix is the re simulation already proposed under intake versus settlement timing, plus optionally one `settleBatch` per pair so a bad pair cannot starve the others.

- `SettlementBatch.t.sol::test_RevertWhen_BatchOrderExpired`

### Off chain cancel is best effort in batch

A maker cancel in batch mode drops the order from the in memory buffer and flips its database status, but it does not call the contract's authoritative `cancel` and does not prune the admitted signature cache. A cancel that races an already closed window can still settle, because the carried signature is intact and the on chain `cancelled` map was never set. The only authoritative cancel is the on chain one. The fix is for a maker cancel to submit that transaction, or for the engine to freeze the included set at window close.

- on chain authority: `Settlement.t.sol::test_RevertWhen_OrderCancelled`

### Resource growth and gas

These are availability concerns, not fund safety. The admitted signature map and the batch buffer are insert and roll only, with no eviction, so they grow with order flow. Remainders below one clearing lot never fill and never leave the buffer. And `settleBatch` has no cap on fills or pairs and is roughly order fills times prices, so a large enough window can exceed the block gas limit and stall settlement. None of these can move funds, but each should be bounded for production: prune admitted entries on confirmation or expiry, evict sub lot dust, cap the batch size or split it one transaction per pair, and index the price lookup.

### Dashboard numbers are an operator view, not proof

The `/batch` endpoint serves the engine's in memory clearing price, volume, and surplus, published before the settle transaction is even sent, and the indexer consumes only `Trade` events and ignores `BatchSettled`. Surplus has no on chain existence at all. So the displayed numbers are the matcher's self report, useful for watching the system, not a verified record of what settled. Reconciling them against `BatchSettled` is the fix.

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
