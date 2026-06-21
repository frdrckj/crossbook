// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import {ERC20} from "@openzeppelin/contracts/token/ERC20/ERC20.sol";
import {CrossbookSettlement} from "../../src/CrossbookSettlement.sol";
import {SignedOrder, Fill} from "../../src/interfaces/ISettlement.sol";

/// A plain mintable ERC20 for tests.
contract MockERC20 is ERC20 {
    constructor(string memory name_, string memory symbol_) ERC20(name_, symbol_) {}

    function mint(address to, uint256 amount) external {
        _mint(to, amount);
    }
}

/// An ERC20 that skims a 1% fee on every transfer between accounts. Breaks net
/// zero settlement, so settle must revert when one side uses it.
contract FeeOnTransferToken is ERC20 {
    constructor() ERC20("Fee", "FEE") {}

    function mint(address to, uint256 amount) external {
        _mint(to, amount);
    }

    function _update(address from, address to, uint256 value) internal override {
        if (from != address(0) && to != address(0) && value >= 100) {
            uint256 fee = value / 100;
            super._update(from, to, value - fee);
            super._update(from, address(0xdead), fee);
        } else {
            super._update(from, to, value);
        }
    }
}

/// An ERC20 that tries to re-enter settle on transfer. Any successful reentrancy
/// would be a double spend; the call must revert instead.
contract ReentrantToken is ERC20 {
    CrossbookSettlement public settlement;
    bool public armed;

    constructor() ERC20("Reentrant", "RE") {}

    function mint(address to, uint256 amount) external {
        _mint(to, amount);
    }

    function arm(CrossbookSettlement s) external {
        settlement = s;
        armed = true;
    }

    function _update(address from, address to, uint256 value) internal override {
        if (armed && address(settlement) != address(0) && from == address(settlement)) {
            // Re-enter during the send phase. Reverts (NotSolver / guard), which
            // bubbles up and reverts the whole settlement.
            settlement.settle(new SignedOrder[](0), new Fill[](0));
        }
        super._update(from, to, value);
    }
}
