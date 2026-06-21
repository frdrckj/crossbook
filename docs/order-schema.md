# Canonical EIP-712 order schema (source of truth)

This schema is shared, byte for byte, by the Rust core (`crossbook-core/src/eip712.rs`) and the Solidity library (`contracts/src/libraries/OrderLib.sol`). Neither side may drift from this document. The M2 parity test compares the two digests and gates everything downstream.

## Domain

```
EIP712Domain(string name,string version,uint256 chainId,address verifyingContract)
name              = "Crossbook"
version           = "1"
chainId           = <target chain id>           // 31337 on local Anvil, read at runtime, never hardcoded to 1
verifyingContract = <deployed CrossbookSettlement address>
```

## Order struct

```solidity
struct Order {
    address maker;             // signer; funds pulled from here
    address sellToken;         // ERC-20 the maker gives
    address buyToken;          // ERC-20 the maker receives
    uint256 sellAmount;        // amount of sellToken offered
    uint256 buyAmount;         // minimum buyToken required (limit price = buyAmount / sellAmount)
    uint256 validTo;           // unix timestamp expiry
    uint256 nonce;             // disambiguates otherwise identical orders; part of the order id
    bool    partiallyFillable; // if false, fill or kill (must fill the full sellAmount at once)
}
```

## Type hash

```
ORDER_TYPEHASH = keccak256(
  "Order(address maker,address sellToken,address buyToken,uint256 sellAmount,uint256 buyAmount,uint256 validTo,uint256 nonce,bool partiallyFillable)"
)
```

## Digest

```
digest = keccak256(0x1901 ++ domainSeparator ++ keccak256(abi.encode(
            ORDER_TYPEHASH, maker, sellToken, buyToken,
            sellAmount, buyAmount, validTo, nonce, partiallyFillable)))
recovered = ECDSA.recover(digest, signature)   // OpenZeppelin ECDSA rejects high-s and address(0); must equal order.maker
```

The EIP-712 digest is the order id. Fill and replay state is keyed by it (see the settlement contract).

This order shape follows CoW Protocol's sell amount and buy amount limit order model. Reusing that mental model is a deliberate signal for that application.

## Implementation rules (do not drift)

Rust declares the `Order` struct with Alloy's `sol!` macro, then derives the digest with `SolStruct::eip712_signing_hash(&domain)`. That keeps the Rust digest identical to Solidity for free.

`validTo` is a `uint256` on the wire. The ergonomic Rust type may store `valid_to: u64`, but the struct fed to the hash must encode it as `uint256`. Transcribe the `sol!` `Order` verbatim from the Solidity above and convert the `u64` to `U256` when building the hashing struct. Narrowing the encoded width silently breaks the digest.

Signatures use the 65 byte form `r ++ s ++ v` with `v` in {27, 28} and low `s`. Alloy signers already produce canonical low `s` signatures. The engine must apply the same acceptance rule as `ECDSA.recover`, so an order the engine admits never reverts the whole batch onchain.

All price comparisons (book ordering and the onchain limit check) use overflow safe cross multiplication, never a 256 bit product and never integer division. The ratio `b1/s1` versus `b2/s2` becomes `b1 * s2` versus `b2 * s1`, widened to 512 bits first. See `crossbook-core/src/price.rs`.

## Runtime domain wiring

OpenZeppelin's `EIP712` derives `chainId` and `verifyingContract` at runtime from `block.chainid` and `address(this)`. Its constructor takes only `name` and `version`. The Rust side must supply the same two values. After the Foundry deploy, capture the deployed address into engine config. At startup, read the live chain id from the provider rather than hardcoding it. Build the Alloy domain from those values, then read the contract's domain through ERC-5267 `eip712Domain()`, recompute the separator in Rust, and refuse to start if they differ.
