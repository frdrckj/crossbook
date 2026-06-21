// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import {Test} from "forge-std/Test.sol";
import {CrossbookSettlement} from "../src/CrossbookSettlement.sol";
import {Order} from "../src/libraries/OrderLib.sol";
import {SignedOrder, Fill} from "../src/interfaces/ISettlement.sol";
import {MockERC20} from "./mocks/Mocks.sol";

/// Drives random balanced settlements. Every settlement nets to zero by
/// construction, so the contract must never accumulate inventory.
contract SettlementHandler is Test {
    CrossbookSettlement settlement;
    MockERC20 tokenA;
    MockERC20 tokenB;
    address solver;

    uint256 pkA = 0xA11CE;
    uint256 pkB = 0xB0B;
    address makerA;
    address makerB;
    uint256 nonce;

    constructor(CrossbookSettlement s, MockERC20 a, MockERC20 b, address solver_) {
        settlement = s;
        tokenA = a;
        tokenB = b;
        solver = solver_;
        makerA = vm.addr(pkA);
        makerB = vm.addr(pkB);
        vm.prank(makerA);
        tokenA.approve(address(s), type(uint256).max);
        vm.prank(makerB);
        tokenB.approve(address(s), type(uint256).max);
    }

    function settleRandom(uint256 aSell, uint256 aBuy) external {
        aSell = bound(aSell, 1, 1e30);
        aBuy = bound(aBuy, 1, 1e30);
        nonce++;
        tokenA.mint(makerA, aSell);
        tokenB.mint(makerB, aBuy);

        Order memory o0 = Order(
            makerA, address(tokenA), address(tokenB), aSell, aBuy, block.timestamp + 1, nonce, true
        );
        Order memory o1 = Order(
            makerB, address(tokenB), address(tokenA), aBuy, aSell, block.timestamp + 1, nonce, true
        );

        SignedOrder[] memory orders = new SignedOrder[](2);
        orders[0] = _sign(pkA, o0);
        orders[1] = _sign(pkB, o1);
        Fill[] memory fills = new Fill[](2);
        fills[0] = Fill(0, aSell, aBuy);
        fills[1] = Fill(1, aBuy, aSell);

        vm.prank(solver);
        settlement.settle(orders, fills);
    }

    function _sign(uint256 pk, Order memory o) internal view returns (SignedOrder memory) {
        bytes32 d = settlement.orderHash(o);
        (uint8 v, bytes32 r, bytes32 s) = vm.sign(pk, d);
        return SignedOrder(o, abi.encodePacked(r, s, v));
    }
}

contract SettlementInvariantTest is Test {
    CrossbookSettlement settlement;
    MockERC20 tokenA;
    MockERC20 tokenB;
    SettlementHandler handler;
    address solver = address(0x5011E5);

    function setUp() public {
        settlement = new CrossbookSettlement(solver);
        tokenA = new MockERC20("TokenA", "A");
        tokenB = new MockERC20("TokenB", "B");
        handler = new SettlementHandler(settlement, tokenA, tokenB, solver);
        targetContract(address(handler));
    }

    /// The settlement contract is non custodial: it must hold zero inventory of
    /// any token after any sequence of settlements.
    function invariant_SettlementHoldsNoInventory() public view {
        assertEq(tokenA.balanceOf(address(settlement)), 0);
        assertEq(tokenB.balanceOf(address(settlement)), 0);
    }
}
