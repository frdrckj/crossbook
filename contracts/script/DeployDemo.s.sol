// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import {Script, console} from "forge-std/Script.sol";
import {CrossbookSettlement} from "../src/CrossbookSettlement.sol";
import {MockERC20} from "../test/mocks/Mocks.sol";

/// Deploys the settlement contract and two demo tokens, then funds and approves
/// two makers, so the dashboard has something to trade. Prints the addresses for
/// the demo script to capture.
contract DeployDemo is Script {
    function run() external {
        uint256 deployerKey = vm.envUint("DEPLOYER_PRIVATE_KEY");
        uint256 makerAKey = vm.envUint("MAKER_A_KEY");
        uint256 makerBKey = vm.envUint("MAKER_B_KEY");
        address makerA = vm.addr(makerAKey);
        address makerB = vm.addr(makerBKey);
        uint256 mint = 1e24;

        vm.startBroadcast(deployerKey);
        CrossbookSettlement settlement = new CrossbookSettlement(vm.addr(deployerKey));
        MockERC20 tokenA = new MockERC20("Token A", "A");
        MockERC20 tokenB = new MockERC20("Token B", "B");
        tokenA.mint(makerA, mint);
        tokenB.mint(makerB, mint);
        vm.stopBroadcast();

        vm.startBroadcast(makerAKey);
        tokenA.approve(address(settlement), type(uint256).max);
        vm.stopBroadcast();

        vm.startBroadcast(makerBKey);
        tokenB.approve(address(settlement), type(uint256).max);
        vm.stopBroadcast();

        console.log(string.concat("SETTLEMENT_ADDRESS=", vm.toString(address(settlement))));
        console.log(string.concat("DEMO_TOKEN_A=", vm.toString(address(tokenA))));
        console.log(string.concat("DEMO_TOKEN_B=", vm.toString(address(tokenB))));
    }
}
