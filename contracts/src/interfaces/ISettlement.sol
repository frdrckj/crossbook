// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import {Order} from "../libraries/OrderLib.sol";

/// @notice An order plus its EIP-712 signature by order.maker.
struct SignedOrder {
    Order order;
    bytes signature;
}

/// @notice One fill, from the maker's perspective. `orderIndex` points into the
/// SignedOrder[] passed to settle.
struct Fill {
    uint256 orderIndex;
    uint256 sellFilled; // sellToken pulled from this maker
    uint256 buyFilled; // buyToken delivered to this maker
}

/// @notice The uniform clearing price for one token pair in a batch, expressed as
/// the rational `num / den` in quote per base. `base` is the lower token address.
struct ClearingPrice {
    address base;
    address quote;
    uint256 num;
    uint256 den;
}

interface ISettlement {
    event Trade(
        address indexed maker,
        address indexed sellToken,
        address indexed buyToken,
        uint256 sellFilled,
        uint256 buyFilled,
        bytes32 orderHash
    );
    event Settlement(address indexed solver, uint256 tradeCount);
    event BatchSettled(
        address indexed base,
        address indexed quote,
        uint256 clearingNum,
        uint256 clearingDen,
        uint256 volumeBase
    );
    event SolverUpdated(address indexed solver);
    event OrderInvalidated(bytes32 indexed orderHash, address indexed maker);

    /// @notice Verify each order's signature, expiry, cumulative fill bound, and
    /// limit price, then execute the fills atomically. Reverts wholesale on any
    /// failure. Restricted to the registered solver.
    function settle(SignedOrder[] calldata orders, Fill[] calldata fills) external;

    /// @notice Like settle, but additionally asserts that every fill in a token
    /// pair executes at that pair's single uniform clearing price, then emits a
    /// BatchSettled event per pair. The batch auction's correctness is enforced on
    /// chain, not merely trusted from the solver.
    function settleBatch(
        SignedOrder[] calldata orders,
        Fill[] calldata fills,
        ClearingPrice[] calldata prices
    ) external;

    /// @notice Maker invalidates one of their own orders.
    function cancel(Order calldata order) external;

    function filledSell(bytes32 orderHash) external view returns (uint256);
}
