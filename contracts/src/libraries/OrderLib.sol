// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

/// @notice The signed order. Must stay byte for byte identical to the Rust
/// domain type and the canonical schema in docs/order-schema.md.
struct Order {
    address maker;
    address sellToken;
    address buyToken;
    uint256 sellAmount;
    uint256 buyAmount;
    uint256 validTo;
    uint256 nonce;
    bool partiallyFillable;
}

/// @notice EIP-712 type hash and struct hashing for Order. The Solidity twin of
/// crossbook-core/src/eip712.rs.
library OrderLib {
    bytes32 internal constant ORDER_TYPEHASH = keccak256(
        "Order(address maker,address sellToken,address buyToken,uint256 sellAmount,uint256 buyAmount,uint256 validTo,uint256 nonce,bool partiallyFillable)"
    );

    /// @dev hashStruct(order) = keccak256(abi.encode(typeHash, fields...)).
    function hash(Order memory order) internal pure returns (bytes32) {
        return keccak256(
            abi.encode(
                ORDER_TYPEHASH,
                order.maker,
                order.sellToken,
                order.buyToken,
                order.sellAmount,
                order.buyAmount,
                order.validTo,
                order.nonce,
                order.partiallyFillable
            )
        );
    }
}
