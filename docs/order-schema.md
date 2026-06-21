# Canonical EIP-712 Order Schema (source of truth)

This schema is shared, byte-for-byte, by the Rust core (`crossbook-core/src/eip712.rs`)
and the Solidity library (`contracts/src/libraries/OrderLib.sol`). **Neither side may
drift from this document.** The M2 cross-language parity test gates everything downstream.

## Domain

```
EIP712Domain(string name,string version,uint256 chainId,address verifyingContract)
name              = "Crossbook"
version           = "1"
chainId           = <target chain id>           # 31337 on local Anvil
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
    uint256 nonce;             // replay protection (unique per maker)
    bool    partiallyFillable; // if false, fill-or-kill
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
recovered = ecrecover(digest, signature)   // MUST equal order.maker
```

## Implementation notes

- **Rust:** declare `Order` via Alloy's `sol!` macro, then derive the digest with
  `SolStruct::eip712_signing_hash(&domain)`. This keeps the Rust digest identical to
  Solidity for free.
- **`validTo` is `uint256` on the wire** (EIP-712 encodes it as a 32-byte word). The
  Rust domain type may store it as `u64` internally, but it MUST be encoded as `uint256`
  in the hash. Do not let the Rust type narrow the encoded width.
- Signature handling must reject malleable / high-`s` signatures (use OZ `ECDSA.recover`
  on-chain, which enforces this).

> This order shape intentionally mirrors CoW Protocol's sell/buy-amount limit-order
> model — a deliberate signal for that application.
