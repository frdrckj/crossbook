// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import {Test} from "forge-std/Test.sol";

/// ponytail: M0 smoke test — proves the Foundry toolchain runs green on the
/// empty scaffold. Real settlement tests replace this in M3.
contract ScaffoldTest is Test {
    function test_Scaffold() public pure {
        assertEq(uint256(2) + 2, 4);
    }
}
