// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import {Test} from "forge-std/Test.sol";

import {CrossbookSettlement} from "../src/CrossbookSettlement.sol";
import {Order} from "../src/libraries/OrderLib.sol";
import {ISettlement, SignedOrder, Fill, ClearingPrice} from "../src/interfaces/ISettlement.sol";
import {MockERC20} from "./mocks/Mocks.sol";

/// Tests for the batch auction entrypoint: a uniform clearing price per pair,
/// enforced on chain, the BatchSettled event, and that the regular settle checks
/// still apply.
contract SettlementBatchTest is Test {
    CrossbookSettlement settlement;
    MockERC20 tokenA;
    MockERC20 tokenB;

    address solver = address(0x5011E5);
    uint256 pkA = 0xA11CE;
    uint256 pkB = 0xB0B;
    address makerA;
    address makerB;

    uint256 future;

    // Derived per batch and kept in storage to keep `uniformBatch` off the stack.
    address base;
    address quote;
    uint256 quoteAmt;
    address askMaker;
    address bidMaker;

    function setUp() public {
        settlement = new CrossbookSettlement(solver);
        tokenA = new MockERC20("TokenA", "A");
        tokenB = new MockERC20("TokenB", "B");
        makerA = vm.addr(pkA);
        makerB = vm.addr(pkB);
        future = block.timestamp + 1 days;
    }

    function mkOrder(
        address maker,
        address sellToken,
        uint256 sellAmount,
        address buyToken,
        uint256 buyAmount
    ) internal view returns (Order memory) {
        return Order({
            maker: maker,
            sellToken: sellToken,
            buyToken: buyToken,
            sellAmount: sellAmount,
            buyAmount: buyAmount,
            validTo: future,
            nonce: 1,
            partiallyFillable: true
        });
    }

    function sign(uint256 pk, Order memory o) internal view returns (SignedOrder memory) {
        bytes32 digest = settlement.orderHash(o);
        (uint8 v, bytes32 r, bytes32 s) = vm.sign(pk, digest);
        return SignedOrder({order: o, signature: abi.encodePacked(r, s, v)});
    }

    /// A clean coincidence of wants at the uniform price num/den (quote per base).
    /// The ask sells `baseAmt` base, the bid pays `baseAmt*num/den` quote; both
    /// makers are funded and approved, and the canonical base is the lower address.
    /// Derived values land in storage (base, quote, quoteAmt, askMaker, bidMaker).
    function uniformBatch(uint256 baseAmt, uint256 num, uint256 den)
        internal
        returns (SignedOrder[] memory orders, Fill[] memory fills, ClearingPrice[] memory prices)
    {
        (base, quote) = address(tokenA) < address(tokenB)
            ? (address(tokenA), address(tokenB))
            : (address(tokenB), address(tokenA));
        quoteAmt = baseAmt * num / den;
        // The ask maker holds the base token; the bid maker holds the quote token.
        askMaker = base == address(tokenA) ? makerA : makerB;
        bidMaker = base == address(tokenA) ? makerB : makerA;
        uint256 askPk = base == address(tokenA) ? pkA : pkB;
        uint256 bidPk = base == address(tokenA) ? pkB : pkA;

        MockERC20(base).mint(askMaker, baseAmt);
        MockERC20(quote).mint(bidMaker, quoteAmt);
        vm.prank(askMaker);
        MockERC20(base).approve(address(settlement), type(uint256).max);
        vm.prank(bidMaker);
        MockERC20(quote).approve(address(settlement), type(uint256).max);

        orders = new SignedOrder[](2);
        orders[0] = sign(askPk, mkOrder(askMaker, base, baseAmt, quote, quoteAmt));
        orders[1] = sign(bidPk, mkOrder(bidMaker, quote, quoteAmt, base, baseAmt));

        fills = new Fill[](2);
        fills[0] = Fill({orderIndex: 0, sellFilled: baseAmt, buyFilled: quoteAmt});
        fills[1] = Fill({orderIndex: 1, sellFilled: quoteAmt, buyFilled: baseAmt});

        prices = new ClearingPrice[](1);
        prices[0] = ClearingPrice({base: base, quote: quote, num: num, den: den});
    }

    function test_BatchSettlesAtUniformPrice() public {
        uint256 baseAmt = 100e18;
        (SignedOrder[] memory orders, Fill[] memory fills, ClearingPrice[] memory prices) =
            uniformBatch(baseAmt, 3, 2); // price 1.5

        vm.prank(solver);
        settlement.settleBatch(orders, fills, prices);

        // Ask received quote, bid received base, both at the single price.
        assertEq(MockERC20(quote).balanceOf(askMaker), quoteAmt);
        assertEq(MockERC20(base).balanceOf(bidMaker), baseAmt);
        // No leftover inventory in the contract.
        assertEq(MockERC20(base).balanceOf(address(settlement)), 0);
        assertEq(MockERC20(quote).balanceOf(address(settlement)), 0);
        assertEq(settlement.filledSell(settlement.orderHash(orders[0].order)), baseAmt);
    }

    function test_BatchEmitsBatchSettled() public {
        uint256 baseAmt = 100e18;
        (SignedOrder[] memory orders, Fill[] memory fills, ClearingPrice[] memory prices) =
            uniformBatch(baseAmt, 3, 2);

        vm.expectEmit(true, true, false, true, address(settlement));
        emit ISettlement.BatchSettled(base, quote, 3, 2, baseAmt);
        vm.prank(solver);
        settlement.settleBatch(orders, fills, prices);
    }

    function test_RevertWhen_FillDeviatesFromClearingPrice() public {
        (SignedOrder[] memory orders, Fill[] memory fills, ClearingPrice[] memory prices) =
            uniformBatch(100e18, 3, 2);
        // Nudge the ask's received quote off the clearing ratio.
        fills[0].buyFilled = fills[0].buyFilled - 1;
        vm.prank(solver);
        vm.expectRevert(CrossbookSettlement.UniformPriceViolated.selector);
        settlement.settleBatch(orders, fills, prices);
    }

    function test_RevertWhen_ClearingPriceMissing() public {
        (SignedOrder[] memory orders, Fill[] memory fills,) = uniformBatch(100e18, 3, 2);
        ClearingPrice[] memory empty = new ClearingPrice[](0);
        vm.prank(solver);
        vm.expectRevert(CrossbookSettlement.ClearingPriceMissing.selector);
        settlement.settleBatch(orders, fills, empty);
    }

    function test_RevertWhen_NotSolver() public {
        (SignedOrder[] memory orders, Fill[] memory fills, ClearingPrice[] memory prices) =
            uniformBatch(100e18, 3, 2);
        vm.expectRevert(CrossbookSettlement.NotSolver.selector);
        settlement.settleBatch(orders, fills, prices);
    }

    function test_RevertWhen_EmptyBatch() public {
        (SignedOrder[] memory orders,, ClearingPrice[] memory prices) = uniformBatch(100e18, 3, 2);
        Fill[] memory empty = new Fill[](0);
        vm.prank(solver);
        vm.expectRevert(CrossbookSettlement.EmptyBatch.selector);
        settlement.settleBatch(orders, empty, prices);
    }

    function test_RevertWhen_DegenerateClearingPrice() public {
        (SignedOrder[] memory orders, Fill[] memory fills, ClearingPrice[] memory prices) =
            uniformBatch(100e18, 3, 2);
        prices[0].den = 0; // not a valid price
        vm.prank(solver);
        vm.expectRevert(CrossbookSettlement.InvalidClearingPrice.selector);
        settlement.settleBatch(orders, fills, prices);
    }

    function test_RevertWhen_DuplicateClearingPrice() public {
        (SignedOrder[] memory orders, Fill[] memory fills, ClearingPrice[] memory prices) =
            uniformBatch(100e18, 3, 2);
        ClearingPrice[] memory dup = new ClearingPrice[](2);
        dup[0] = prices[0];
        dup[1] = prices[0]; // same pair twice
        vm.prank(solver);
        vm.expectRevert(CrossbookSettlement.DuplicateClearingPrice.selector);
        settlement.settleBatch(orders, fills, dup);
    }

    function test_RevertWhen_StaleClearingPrice() public {
        (SignedOrder[] memory orders, Fill[] memory fills, ClearingPrice[] memory prices) =
            uniformBatch(100e18, 3, 2);
        ClearingPrice[] memory extra = new ClearingPrice[](2);
        extra[0] = prices[0];
        // A price for a pair no fill references: it cleared no volume.
        extra[1] = ClearingPrice({base: address(0x1111), quote: address(0x2222), num: 1, den: 1});
        vm.prank(solver);
        vm.expectRevert(CrossbookSettlement.UnusedClearingPrice.selector);
        settlement.settleBatch(orders, fills, extra);
    }

    function test_RevertWhen_BatchOrderExpired() public {
        (SignedOrder[] memory orders, Fill[] memory fills, ClearingPrice[] memory prices) =
            uniformBatch(100e18, 3, 2);
        // The uniform price check passes; the inherited settle checks still apply.
        vm.warp(future + 1);
        vm.prank(solver);
        vm.expectRevert(CrossbookSettlement.OrderExpired.selector);
        settlement.settleBatch(orders, fills, prices);
    }

    function testFuzz_BatchUniformPriceLeavesNoInventory(uint256 baseAmt, uint256 num, uint256 den)
        public
    {
        // Keep the quote leg exact so a single integer price clears both sides.
        den = bound(den, 1, 1e6);
        num = bound(num, 1, 1e6);
        baseAmt = bound(baseAmt, 1, 1e24);
        baseAmt = baseAmt - (baseAmt % den) + den; // a multiple of den, so quote is exact

        (SignedOrder[] memory orders, Fill[] memory fills, ClearingPrice[] memory prices) =
            uniformBatch(baseAmt, num, den);

        vm.prank(solver);
        settlement.settleBatch(orders, fills, prices);

        assertEq(MockERC20(base).balanceOf(address(settlement)), 0);
        assertEq(MockERC20(quote).balanceOf(address(settlement)), 0);
    }
}
