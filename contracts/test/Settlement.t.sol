// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import {Test} from "forge-std/Test.sol";
import {Pausable} from "@openzeppelin/contracts/utils/Pausable.sol";
import {Ownable} from "@openzeppelin/contracts/access/Ownable.sol";
import {IERC20} from "@openzeppelin/contracts/token/ERC20/IERC20.sol";

import {CrossbookSettlement} from "../src/CrossbookSettlement.sol";
import {Order} from "../src/libraries/OrderLib.sol";
import {SignedOrder, Fill} from "../src/interfaces/ISettlement.sol";
import {MockERC20, FeeOnTransferToken, ReentrantToken} from "./mocks/Mocks.sol";

contract SettlementTest is Test {
    CrossbookSettlement settlement;
    MockERC20 tokenA;
    MockERC20 tokenB;

    address solver = address(0x5011E5);
    uint256 pkA = 0xA11CE;
    uint256 pkB = 0xB0B;
    address makerA;
    address makerB;

    uint256 constant AMT = 100e18;
    uint256 future;

    function setUp() public {
        settlement = new CrossbookSettlement(solver); // owner = this
        tokenA = new MockERC20("TokenA", "A");
        tokenB = new MockERC20("TokenB", "B");
        makerA = vm.addr(pkA);
        makerB = vm.addr(pkB);
        future = block.timestamp + 1 days;

        tokenA.mint(makerA, AMT);
        tokenB.mint(makerB, AMT);
        vm.prank(makerA);
        tokenA.approve(address(settlement), type(uint256).max);
        vm.prank(makerB);
        tokenB.approve(address(settlement), type(uint256).max);
    }

    // ---- helpers ----

    function mkOrder(
        address maker,
        address sellToken,
        uint256 sellAmount,
        address buyToken,
        uint256 buyAmount,
        bool partiallyFillable
    ) internal view returns (Order memory) {
        return Order({
            maker: maker,
            sellToken: sellToken,
            buyToken: buyToken,
            sellAmount: sellAmount,
            buyAmount: buyAmount,
            validTo: future,
            nonce: 1,
            partiallyFillable: partiallyFillable
        });
    }

    function sign(uint256 pk, Order memory o) internal view returns (SignedOrder memory) {
        bytes32 digest = settlement.orderHash(o);
        (uint8 v, bytes32 r, bytes32 s) = vm.sign(pk, digest);
        return SignedOrder({order: o, signature: abi.encodePacked(r, s, v)});
    }

    /// A balanced two order cross: A sells `aSell` for `aBuy`, B mirrors it.
    function balancedBatch(uint256 aSell, uint256 aBuy)
        internal
        view
        returns (SignedOrder[] memory orders, Fill[] memory fills)
    {
        Order memory o0 = mkOrder(makerA, address(tokenA), aSell, address(tokenB), aBuy, true);
        Order memory o1 = mkOrder(makerB, address(tokenB), aBuy, address(tokenA), aSell, true);
        orders = new SignedOrder[](2);
        orders[0] = sign(pkA, o0);
        orders[1] = sign(pkB, o1);
        fills = new Fill[](2);
        fills[0] = Fill({orderIndex: 0, sellFilled: aSell, buyFilled: aBuy});
        fills[1] = Fill({orderIndex: 1, sellFilled: aBuy, buyFilled: aSell});
    }

    // ---- happy path ----

    function test_SettlesBalancedCross() public {
        (SignedOrder[] memory orders, Fill[] memory fills) = balancedBatch(AMT, AMT);
        vm.prank(solver);
        settlement.settle(orders, fills);

        assertEq(tokenA.balanceOf(makerA), 0);
        assertEq(tokenB.balanceOf(makerA), AMT);
        assertEq(tokenA.balanceOf(makerB), AMT);
        assertEq(tokenB.balanceOf(makerB), 0);
        // contract holds zero inventory
        assertEq(tokenA.balanceOf(address(settlement)), 0);
        assertEq(tokenB.balanceOf(address(settlement)), 0);
        // fill accounting recorded
        assertEq(settlement.filledSell(settlement.orderHash(orders[0].order)), AMT);
    }

    function test_PartialFillAccumulates() public {
        // makerA sells 100 A for 100 B, partially fillable; fill half twice.
        Order memory oA = mkOrder(makerA, address(tokenA), AMT, address(tokenB), AMT, true);
        Order memory oB = mkOrder(makerB, address(tokenB), AMT, address(tokenA), AMT, true);
        bytes32 hA = settlement.orderHash(oA);

        SignedOrder[] memory orders = new SignedOrder[](2);
        orders[0] = sign(pkA, oA);
        orders[1] = sign(pkB, oB);

        Fill[] memory fills = new Fill[](2);
        fills[0] = Fill({orderIndex: 0, sellFilled: AMT / 2, buyFilled: AMT / 2});
        fills[1] = Fill({orderIndex: 1, sellFilled: AMT / 2, buyFilled: AMT / 2});

        vm.prank(solver);
        settlement.settle(orders, fills);
        assertEq(settlement.filledSell(hA), AMT / 2);

        vm.prank(solver);
        settlement.settle(orders, fills);
        assertEq(settlement.filledSell(hA), AMT);
    }

    // ---- revert paths ----

    function test_RevertWhen_NotSolver() public {
        (SignedOrder[] memory orders, Fill[] memory fills) = balancedBatch(AMT, AMT);
        vm.expectRevert(CrossbookSettlement.NotSolver.selector);
        settlement.settle(orders, fills); // msg.sender = test, not solver
    }

    function test_RevertWhen_BadSignature() public {
        (SignedOrder[] memory orders, Fill[] memory fills) = balancedBatch(AMT, AMT);
        orders[0].signature = sign(pkB, orders[0].order).signature; // wrong signer
        vm.prank(solver);
        vm.expectRevert(CrossbookSettlement.InvalidSignature.selector);
        settlement.settle(orders, fills);
    }

    function test_RevertWhen_Expired() public {
        (SignedOrder[] memory orders, Fill[] memory fills) = balancedBatch(AMT, AMT);
        vm.warp(future + 1);
        vm.prank(solver);
        vm.expectRevert(CrossbookSettlement.OrderExpired.selector);
        settlement.settle(orders, fills);
    }

    function test_RevertWhen_Overfill() public {
        (SignedOrder[] memory orders, Fill[] memory fills) = balancedBatch(AMT, AMT);
        fills[0].sellFilled = AMT + 1; // more than signed
        vm.prank(solver);
        vm.expectRevert(CrossbookSettlement.Overfill.selector);
        settlement.settle(orders, fills);
    }

    function test_RevertWhen_PartialOnFillOrKill() public {
        Order memory oA = mkOrder(makerA, address(tokenA), AMT, address(tokenB), AMT, false); // FOK
        Order memory oB = mkOrder(makerB, address(tokenB), AMT, address(tokenA), AMT, true);
        SignedOrder[] memory orders = new SignedOrder[](2);
        orders[0] = sign(pkA, oA);
        orders[1] = sign(pkB, oB);
        Fill[] memory fills = new Fill[](2);
        fills[0] = Fill({orderIndex: 0, sellFilled: AMT / 2, buyFilled: AMT / 2}); // partial on a FOK
        fills[1] = Fill({orderIndex: 1, sellFilled: AMT / 2, buyFilled: AMT / 2});
        vm.prank(solver);
        vm.expectRevert(CrossbookSettlement.PartialFillNotAllowed.selector);
        settlement.settle(orders, fills);
    }

    function test_RevertWhen_BelowLimitPrice() public {
        (SignedOrder[] memory orders, Fill[] memory fills) = balancedBatch(AMT, AMT);
        fills[0].buyFilled = AMT - 1; // maker A gets one wei less than its limit
        vm.prank(solver);
        vm.expectRevert(CrossbookSettlement.LimitPriceViolated.selector);
        settlement.settle(orders, fills);
    }

    function test_RevertWhen_InventoryNotZero() public {
        // A single order whose buyToken has no matching inflow: net is unbalanced.
        Order memory oA = mkOrder(makerA, address(tokenA), AMT, address(tokenB), AMT, true);
        SignedOrder[] memory orders = new SignedOrder[](1);
        orders[0] = sign(pkA, oA);
        Fill[] memory fills = new Fill[](1);
        fills[0] = Fill({orderIndex: 0, sellFilled: AMT, buyFilled: AMT});
        // sellToken (tokenA) is the first token in the net list: inflow AMT, outflow 0.
        vm.prank(solver);
        vm.expectRevert(
            abi.encodeWithSelector(CrossbookSettlement.InventoryNotZero.selector, address(tokenA))
        );
        settlement.settle(orders, fills);
    }

    function test_RevertWhen_OrderCancelled() public {
        (SignedOrder[] memory orders, Fill[] memory fills) = balancedBatch(AMT, AMT);
        vm.prank(makerA);
        settlement.cancel(orders[0].order);
        vm.prank(solver);
        vm.expectRevert(CrossbookSettlement.OrderIsCancelled.selector);
        settlement.settle(orders, fills);
    }

    function test_RevertWhen_CancelByNonMaker() public {
        Order memory oA = mkOrder(makerA, address(tokenA), AMT, address(tokenB), AMT, true);
        vm.prank(makerB);
        vm.expectRevert(CrossbookSettlement.NotMaker.selector);
        settlement.cancel(oA);
    }

    function test_RevertWhen_Paused() public {
        (SignedOrder[] memory orders, Fill[] memory fills) = balancedBatch(AMT, AMT);
        settlement.pause(); // owner = this
        vm.prank(solver);
        vm.expectRevert(Pausable.EnforcedPause.selector);
        settlement.settle(orders, fills);
    }

    function test_RevertWhen_FeeOnTransferToken() public {
        FeeOnTransferToken fee = new FeeOnTransferToken();
        // makerA sells fee token, makerB sells tokenB.
        fee.mint(makerA, AMT);
        vm.prank(makerA);
        fee.approve(address(settlement), type(uint256).max);

        Order memory oA = mkOrder(makerA, address(fee), AMT, address(tokenB), AMT, true);
        Order memory oB = mkOrder(makerB, address(tokenB), AMT, address(fee), AMT, true);
        SignedOrder[] memory orders = new SignedOrder[](2);
        orders[0] = sign(pkA, oA);
        orders[1] = sign(pkB, oB);
        Fill[] memory fills = new Fill[](2);
        fills[0] = Fill({orderIndex: 0, sellFilled: AMT, buyFilled: AMT});
        fills[1] = Fill({orderIndex: 1, sellFilled: AMT, buyFilled: AMT});

        // Net accounting passes on stated amounts, but the pull yields less than
        // AMT (fee skimmed), so sending AMT of the fee token reverts.
        vm.prank(solver);
        vm.expectRevert();
        settlement.settle(orders, fills);
    }

    function test_ReentrancyIsBlocked() public {
        ReentrantToken re = new ReentrantToken();
        re.mint(makerB, AMT);
        vm.prank(makerB);
        re.approve(address(settlement), type(uint256).max);
        re.arm(settlement);

        Order memory oA = mkOrder(makerA, address(tokenA), AMT, address(re), AMT, true);
        Order memory oB = mkOrder(makerB, address(re), AMT, address(tokenA), AMT, true);
        SignedOrder[] memory orders = new SignedOrder[](2);
        orders[0] = sign(pkA, oA);
        orders[1] = sign(pkB, oB);
        Fill[] memory fills = new Fill[](2);
        fills[0] = Fill({orderIndex: 0, sellFilled: AMT, buyFilled: AMT});
        fills[1] = Fill({orderIndex: 1, sellFilled: AMT, buyFilled: AMT});

        // The reentrant call during the send phase reverts and bubbles up.
        vm.prank(solver);
        vm.expectRevert();
        settlement.settle(orders, fills);
    }

    // ---- admin ----

    function test_SetSolverRotatesAccess() public {
        address newSolver = address(0xBEEF);
        settlement.setSolver(newSolver);
        (SignedOrder[] memory orders, Fill[] memory fills) = balancedBatch(AMT, AMT);
        vm.prank(solver);
        vm.expectRevert(CrossbookSettlement.NotSolver.selector);
        settlement.settle(orders, fills);
        vm.prank(newSolver);
        settlement.settle(orders, fills); // new solver works
    }

    function test_RevertWhen_SetSolverByNonOwner() public {
        vm.prank(makerA);
        vm.expectRevert(abi.encodeWithSelector(Ownable.OwnableUnauthorizedAccount.selector, makerA));
        settlement.setSolver(makerA);
    }

    // ---- fuzz ----

    function testFuzz_BalancedCrossConservesAndLeavesNoInventory(uint256 aSell, uint256 aBuy)
        public
    {
        aSell = bound(aSell, 1, 1e30);
        aBuy = bound(aBuy, 1, 1e30);
        // mint more to cover the fuzzed amounts
        tokenA.mint(makerA, aSell);
        tokenB.mint(makerB, aBuy);

        (SignedOrder[] memory orders, Fill[] memory fills) = balancedBatch(aSell, aBuy);
        uint256 a0 = tokenA.balanceOf(makerA);
        uint256 b1 = tokenB.balanceOf(makerB);

        vm.prank(solver);
        settlement.settle(orders, fills);

        assertEq(tokenA.balanceOf(makerA), a0 - aSell);
        assertEq(tokenB.balanceOf(makerA), aBuy);
        assertEq(tokenA.balanceOf(makerB), aSell);
        assertEq(tokenB.balanceOf(makerB), b1 - aBuy);
        assertEq(tokenA.balanceOf(address(settlement)), 0);
        assertEq(tokenB.balanceOf(address(settlement)), 0);
    }
}
