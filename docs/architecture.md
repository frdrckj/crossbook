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

## Batch auction

Crossbook can match in two modes, chosen by config and never both at once. Continuous mode crosses each order against the book the moment it arrives, at price time priority, the way a normal order book works. Batch mode instead collects orders over a window and, when the window closes, clears each token pair at one uniform price. The mode is set with `MATCHING_MODE` (continuous or batch) and the window length with `BATCH_INTERVAL`. Continuous mode is unchanged by any of this.

The batch auction is a call auction in the CoW style. Within a window, every order on the same pair that trades does so at a single clearing price, so no order gets a worse fill than its neighbour because it happened to arrive a moment later. Matching is buyer against seller directly, a coincidence of wants, with no external liquidity and no AMM in the loop.

### The clearing algorithm

The auction lives in `crossbook-core/auction.rs` and is a pure function of the collected orders. Same orders in, same result out, no clock and no I/O, the same discipline as the continuous matcher.

It works one token pair at a time. The two token addresses are sorted so the lower one is the canonical base and the higher one is the quote, which gives every pair a single orientation regardless of which side an order is written from. Each order becomes a bid (buys base, sells quote) or an ask (sells base, buys quote) with a limit price in quote per base. Asks are sorted ascending by limit, bids descending, and the two are walked together to find where the curves cross: the highest bids match the lowest asks while the best bid still meets the best ask.

The clearing price p\* is the midpoint of the marginal ask limit and the marginal bid limit, the overlap where the curves meet. Any price in that overlap clears the same orders at the same volume, so the executed volume is already maximal and the imbalance is already fixed. The midpoint is the deterministic, neutral tie break among the prices that all achieve that maximum. The documented rule is therefore: maximise executed volume first, then take the midpoint of the overlap. On extreme inputs where the midpoint arithmetic would not fit in 256 bits, the auction falls back to the marginal ask limit, which is also inside the overlap.

### One price and net zero, exactly, with integers

A uniform price only means something if every fill really executes at it. With integer token amounts that is not free: at an arbitrary rational price most base amounts have no exact quote counterpart. The auction solves this by quantising fills to a lot equal to the reduced denominator of p\*. Every filled base amount is a whole number of lots, so its quote leg, base divided by the denominator times the numerator, is an exact integer. The result is that each fill sits on the price exactly, and the pair nets to zero per token: the total base leaving the asks equals the total base reaching the bids, and the quote legs balance the same way. The contract's net flow check then passes with no dust left in the contract.

The short side of the book fills completely. The marginal order on the long side fills partially, and whatever does not fill, plus any order that did not clear at all, rolls into the next window at its reduced remaining size.

### Surplus

For each filled order the auction measures the price improvement over its own limit, in quote terms: an ask receives more quote than its minimum, a bid pays less quote than its maximum. The sum is the batch surplus, surfaced per pair through the API, the CLI, and the dashboard. For clean offsetting flow the batch captures the same total spread that continuous matching would, but it splits that spread evenly at the midpoint instead of handing it all to whichever side happened to be the taker, which is the fairness property a uniform price auction is supposed to have.

### Enforced on chain, not trusted

Settlement adds a `settleBatch` entrypoint beside `settle`. It shares the same internal verification, signatures, nonce, expiry, the cumulative fill bound, each maker's limit, and the net zero check, and then adds one more: every fill in a pair must execute at exactly that pair's clearing price, checked as an exact 512 bit ratio. It also rejects an empty batch, a non positive price, a duplicate price for a pair, and a price that cleared no volume, so the `BatchSettled` stream it emits per pair (carrying the clearing price and matched volume) is well formed.

It is worth being precise about what this does and does not guarantee. The chain enforces two things for a batch: one uniform price per pair, and each maker's own signed limit. Together these mean every fill in a pair trades at the same price and no maker is ever filled below its limit, even against a malicious solver. What the chain does not enforce is that the chosen price is the midpoint, or central, or that the batch cleared the maximal crossable volume. Those are properties of the off chain auction in `auction.rs`: the contract pins the price to be uniform and within limits, the matcher chooses which uniform price (the midpoint) and how much to clear. A maker is therefore protected to its limit on chain and protected to a fair midpoint by the matcher. The honest framing is uniform price plus limits on chain, fair price off chain.

### How it runs in the engine

In batch mode the single writer task buffers admitted orders instead of crossing them. A window driver closes the window on the interval, asks the core to clear the buffer, advances each order's remaining amount, and submits one `settleBatch` for the whole batch. Settlement emits the same Trade events the indexer already reads, so the read and feed paths are identical to continuous mode. The window countdown, the collected orders, and the last clearing per pair are exposed at `/batch` for the dashboard.
