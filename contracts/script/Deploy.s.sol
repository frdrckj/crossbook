// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import {Script, console} from "forge-std/Script.sol";
import {CrossbookSettlement} from "../src/CrossbookSettlement.sol";

/// Deploys CrossbookSettlement. The deployer becomes the owner; the solver is
/// derived from SOLVER_PRIVATE_KEY (falling back to the deployer).
contract Deploy is Script {
    function run() external returns (CrossbookSettlement settlement) {
        uint256 deployerPk = vm.envUint("DEPLOYER_PRIVATE_KEY");
        uint256 solverPk = vm.envOr("SOLVER_PRIVATE_KEY", deployerPk);
        address solver = vm.addr(solverPk);

        vm.startBroadcast(deployerPk);
        settlement = new CrossbookSettlement(solver);
        vm.stopBroadcast();

        console.log("CrossbookSettlement deployed at", address(settlement));
        console.log("solver", solver);
    }
}
