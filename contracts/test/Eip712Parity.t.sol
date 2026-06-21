// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import {Test} from "forge-std/Test.sol";
import {Order, OrderLib} from "../src/libraries/OrderLib.sol";

/// Solidity half of the EIP-712 parity gate. OrderLib.sol must reproduce the
/// exact digests pinned by the Rust test (crates/crossbook-core/tests/eip712_parity.rs)
/// for the same vectors and the same fixed domain.
contract Eip712ParityTest is Test {
    bytes32 constant DOMAIN_TYPEHASH = keccak256(
        "EIP712Domain(string name,string version,uint256 chainId,address verifyingContract)"
    );
    address constant VERIFYING_CONTRACT = 0x5FbDB2315678afecb367f032d93F642f64180aa3;
    uint256 constant CHAIN_ID = 31337;

    address constant A11 = 0x1111111111111111111111111111111111111111;
    address constant AAA = 0xaAaAaAaaAaAaAaaAaAAAAAAAAaaaAaAaAaaAaaAa;
    address constant BBB = 0xbBbBBBBbbBBBbbbBbbBbbbbBBbBbbbbBbBbbBBbB;
    address constant FFF = 0xFFfFfFffFFfffFFfFFfFFFFFffFFFffffFfFFFfF;

    function domainSeparator() internal pure returns (bytes32) {
        return keccak256(
            abi.encode(
                DOMAIN_TYPEHASH,
                keccak256(bytes("Crossbook")),
                keccak256(bytes("1")),
                CHAIN_ID,
                VERIFYING_CONTRACT
            )
        );
    }

    function digest(Order memory o) internal pure returns (bytes32) {
        return keccak256(abi.encodePacked(hex"1901", domainSeparator(), OrderLib.hash(o)));
    }

    function test_TypeHashMatchesRust() public pure {
        assertEq(
            OrderLib.ORDER_TYPEHASH,
            0xd81e363e64f113b5ef986b1402957b28b98d171832da42cb5e62a904e2dcb564
        );
    }

    function test_BasicDigestMatchesRust() public pure {
        Order memory o = Order({
            maker: A11,
            sellToken: AAA,
            buyToken: BBB,
            sellAmount: 1000,
            buyAmount: 2000,
            validTo: 1_700_000_000,
            nonce: 1,
            partiallyFillable: true
        });
        assertEq(digest(o), 0x44dc657587daad14d16ff62051e3b82762be4ff115a8327090501c6a03e0983b);
    }

    function test_ZerosDigestMatchesRust() public pure {
        Order memory o = Order({
            maker: address(0),
            sellToken: address(0),
            buyToken: address(0),
            sellAmount: 0,
            buyAmount: 0,
            validTo: 0,
            nonce: 0,
            partiallyFillable: false
        });
        assertEq(digest(o), 0x0bd94b83e5f67d384bbbf2a21e3ae26e58caf51aedc1f5d51c8314b1c0a6c07b);
    }

    function test_MaxedDigestMatchesRust() public pure {
        Order memory o = Order({
            maker: FFF,
            sellToken: FFF,
            buyToken: FFF,
            sellAmount: type(uint256).max,
            buyAmount: type(uint256).max,
            validTo: type(uint64).max,
            nonce: type(uint256).max,
            partiallyFillable: true
        });
        assertEq(digest(o), 0x4dacc0230564d04ba8e4e12d03215c76ed2381eb868ed78b870c60064c9303e7);
    }

    function test_BasicFokDigestMatchesRust() public pure {
        Order memory o = Order({
            maker: A11,
            sellToken: AAA,
            buyToken: BBB,
            sellAmount: 1000,
            buyAmount: 2000,
            validTo: 1_700_000_000,
            nonce: 1,
            partiallyFillable: false
        });
        assertEq(digest(o), 0xe3b007a1c212696aa565ccd6daae20d439aa584796dd949c6e04f0f4b753a56a);
    }
}
