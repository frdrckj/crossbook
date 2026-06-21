// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import {EIP712} from "@openzeppelin/contracts/utils/cryptography/EIP712.sol";
import {ECDSA} from "@openzeppelin/contracts/utils/cryptography/ECDSA.sol";
import {ReentrancyGuard} from "@openzeppelin/contracts/utils/ReentrancyGuard.sol";
import {Ownable} from "@openzeppelin/contracts/access/Ownable.sol";
import {Pausable} from "@openzeppelin/contracts/utils/Pausable.sol";
import {SafeERC20} from "@openzeppelin/contracts/token/ERC20/utils/SafeERC20.sol";
import {IERC20} from "@openzeppelin/contracts/token/ERC20/IERC20.sol";

import {Order, OrderLib} from "./libraries/OrderLib.sol";
import {ISettlement, SignedOrder, Fill} from "./interfaces/ISettlement.sol";

/// @title CrossbookSettlement
/// @notice Allowance pull settlement for signed intents. The contract does not
/// trust the solver: it independently rechecks every signature, expiry, the
/// cumulative fill bound, and each maker's limit price on chain, and it requires
/// every touched token to net to zero so it never holds inventory.
contract CrossbookSettlement is ISettlement, EIP712, ReentrancyGuard, Ownable, Pausable {
    using SafeERC20 for IERC20;
    using OrderLib for Order;

    /// @notice The single permissioned solver allowed to call settle (MVP trust
    /// assumption). Rotatable by the owner.
    address public solver;

    /// @notice Cumulative sellToken filled per order hash (the CoW filledAmount pattern).
    mapping(bytes32 => uint256) public filledSell;

    /// @notice Orders invalidated by their maker.
    mapping(bytes32 => bool) public cancelled;

    error NotSolver();
    error InvalidSignature();
    error OrderExpired();
    error OrderIsCancelled();
    error Overfill();
    error PartialFillNotAllowed();
    error LimitPriceViolated();
    error InventoryNotZero(address token);
    error NotMaker();

    modifier onlySolver() {
        if (msg.sender != solver) revert NotSolver();
        _;
    }

    constructor(address solver_) EIP712("Crossbook", "1") Ownable(msg.sender) {
        solver = solver_;
    }

    function setSolver(address newSolver) external onlyOwner {
        solver = newSolver;
        emit SolverUpdated(newSolver);
    }

    function pause() external onlyOwner {
        _pause();
    }

    function unpause() external onlyOwner {
        _unpause();
    }

    /// @notice The EIP-712 digest (and order id) for an order under this domain.
    function orderHash(Order calldata order) external view returns (bytes32) {
        return _hashTypedDataV4(order.hash());
    }

    /// @inheritdoc ISettlement
    function cancel(Order calldata order) external {
        if (msg.sender != order.maker) revert NotMaker();
        bytes32 h = _hashTypedDataV4(order.hash());
        cancelled[h] = true;
        emit OrderInvalidated(h, msg.sender);
    }

    /// @inheritdoc ISettlement
    function settle(SignedOrder[] calldata orders, Fill[] calldata fills)
        external
        onlySolver
        whenNotPaused
        nonReentrant
    {
        uint256 nFills = fills.length;

        // Verify every order once and cache its hash.
        bytes32[] memory hashes = new bytes32[](orders.length);
        for (uint256 i = 0; i < orders.length; i++) {
            Order calldata o = orders[i].order;
            bytes32 h = _hashTypedDataV4(o.hash());
            if (ECDSA.recover(h, orders[i].signature) != o.maker) revert InvalidSignature();
            if (block.timestamp > o.validTo) revert OrderExpired();
            if (cancelled[h]) revert OrderIsCancelled();
            hashes[i] = h;
        }

        // Net flow accounting in memory: per token, total pulled in must equal
        // total sent out. Bounds distinct tokens at 2 per fill.
        address[] memory toks = new address[](nFills * 2);
        uint256[] memory inflow = new uint256[](nFills * 2);
        uint256[] memory outflow = new uint256[](nFills * 2);
        uint256 nToks = 0;

        // Checks and effects (no external calls yet, CEI preserved).
        for (uint256 j = 0; j < nFills; j++) {
            Fill calldata f = fills[j];
            Order calldata o = orders[f.orderIndex].order;
            bytes32 h = hashes[f.orderIndex];

            uint256 newFilled = filledSell[h] + f.sellFilled; // checked add
            if (newFilled > o.sellAmount) revert Overfill();
            if (!o.partiallyFillable && (filledSell[h] != 0 || f.sellFilled != o.sellAmount)) {
                revert PartialFillNotAllowed();
            }
            // Limit price: buyFilled / sellFilled >= buyAmount / sellAmount.
            // Cross multiplied and widened to 512 bits so it cannot overflow.
            if (!_ge512(f.buyFilled, o.sellAmount, f.sellFilled, o.buyAmount)) {
                revert LimitPriceViolated();
            }
            filledSell[h] = newFilled;

            uint256 idx;
            (idx, nToks) = _index(toks, nToks, o.sellToken);
            inflow[idx] += f.sellFilled;
            (idx, nToks) = _index(toks, nToks, o.buyToken);
            outflow[idx] += f.buyFilled;
        }

        // Net zero: the contract receives exactly what it sends, per token.
        for (uint256 k = 0; k < nToks; k++) {
            if (inflow[k] != outflow[k]) revert InventoryNotZero(toks[k]);
        }

        // Interactions: pull all, then send all. With net zero per token the
        // pulls cover the sends and nothing remains. Fee on transfer tokens pull
        // less than stated and make a later send revert (clean wholesale revert).
        for (uint256 j = 0; j < nFills; j++) {
            Fill calldata f = fills[j];
            Order calldata o = orders[f.orderIndex].order;
            IERC20(o.sellToken).safeTransferFrom(o.maker, address(this), f.sellFilled);
        }
        for (uint256 j = 0; j < nFills; j++) {
            Fill calldata f = fills[j];
            Order calldata o = orders[f.orderIndex].order;
            IERC20(o.buyToken).safeTransfer(o.maker, f.buyFilled);
            emit Trade(
                o.maker, o.sellToken, o.buyToken, f.sellFilled, f.buyFilled, hashes[f.orderIndex]
            );
        }

        emit Settlement(msg.sender, nFills);
    }

    /// @dev Linear find or insert of `t` in `toks`, returning its index and the
    /// updated count. Distinct token count is small (<= 2 per fill).
    function _index(address[] memory toks, uint256 nToks, address t)
        private
        pure
        returns (uint256 idx, uint256 newN)
    {
        for (uint256 k = 0; k < nToks; k++) {
            if (toks[k] == t) return (k, nToks);
        }
        toks[nToks] = t;
        return (nToks, nToks + 1);
    }

    /// @dev Returns `a*b >= c*d` using full 512 bit products, so it never
    /// overflows or spuriously reverts on extreme amounts.
    function _ge512(uint256 a, uint256 b, uint256 c, uint256 d) private pure returns (bool) {
        (uint256 hi1, uint256 lo1) = _mul512(a, b);
        (uint256 hi2, uint256 lo2) = _mul512(c, d);
        if (hi1 != hi2) return hi1 > hi2;
        return lo1 >= lo2;
    }

    /// @dev Full width 512 bit product (hi, lo) of a*b.
    function _mul512(uint256 a, uint256 b) private pure returns (uint256 hi, uint256 lo) {
        assembly {
            let mm := mulmod(a, b, not(0))
            lo := mul(a, b)
            hi := sub(sub(mm, lo), lt(mm, lo))
        }
    }
}
