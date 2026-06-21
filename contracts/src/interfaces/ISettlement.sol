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
    event SolverUpdated(address indexed solver);
    event OrderInvalidated(bytes32 indexed orderHash, address indexed maker);

    /// @notice Verify each order's signature, expiry, cumulative fill bound, and
    /// limit price, then execute the fills atomically. Reverts wholesale on any
    /// failure. Restricted to the registered solver.
    function settle(SignedOrder[] calldata orders, Fill[] calldata fills) external;

    /// @notice Maker invalidates one of their own orders.
    function cancel(Order calldata order) external;

    function filledSell(bytes32 orderHash) external view returns (uint256);
}
