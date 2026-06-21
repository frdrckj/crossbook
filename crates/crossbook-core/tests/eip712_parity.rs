//! Rust half of the EIP-712 parity gate. Pins the digest for a set of vectors
//! (basic, all zero, all max, and a bool flip). The Foundry test
//! `contracts/test/Eip712Parity.t.sol` asserts OrderLib.sol produces the same
//! hex for the same vectors and domain.

use alloy_primitives::{address, b256, Address, B256, U256};
use alloy_sol_types::Eip712Domain;
use crossbook_core::eip712::{self, ORDER_TYPE_STRING};
use crossbook_core::types::Order;

const CHAIN_ID: u64 = 31337;

fn verifying_contract() -> Address {
    address!("5FbDB2315678afecb367f032d93F642f64180aa3")
}

fn domain() -> Eip712Domain {
    eip712::crossbook_domain(CHAIN_ID, verifying_contract())
}

fn vectors() -> Vec<(&'static str, Order)> {
    vec![
        (
            "basic",
            Order {
                maker: Address::repeat_byte(0x11),
                sell_token: Address::repeat_byte(0xAA),
                buy_token: Address::repeat_byte(0xBB),
                sell_amount: U256::from(1000u64),
                buy_amount: U256::from(2000u64),
                valid_to: 1_700_000_000,
                nonce: U256::from(1u64),
                partially_fillable: true,
            },
        ),
        (
            "zeros",
            Order {
                maker: Address::ZERO,
                sell_token: Address::ZERO,
                buy_token: Address::ZERO,
                sell_amount: U256::ZERO,
                buy_amount: U256::ZERO,
                valid_to: 0,
                nonce: U256::ZERO,
                partially_fillable: false,
            },
        ),
        (
            "maxed",
            Order {
                maker: Address::repeat_byte(0xFF),
                sell_token: Address::repeat_byte(0xFF),
                buy_token: Address::repeat_byte(0xFF),
                sell_amount: U256::MAX,
                buy_amount: U256::MAX,
                valid_to: u64::MAX,
                nonce: U256::MAX,
                partially_fillable: true,
            },
        ),
        (
            "basic_fok",
            Order {
                maker: Address::repeat_byte(0x11),
                sell_token: Address::repeat_byte(0xAA),
                buy_token: Address::repeat_byte(0xBB),
                sell_amount: U256::from(1000u64),
                buy_amount: U256::from(2000u64),
                valid_to: 1_700_000_000,
                nonce: U256::from(1u64),
                partially_fillable: false,
            },
        ),
    ]
}

/// Pinned digests. The Foundry test `contracts/test/Eip712Parity.t.sol` asserts
/// OrderLib.sol reproduces these exact values for the same vectors and domain.
fn expected(name: &str) -> B256 {
    match name {
        "basic" => b256!("44dc657587daad14d16ff62051e3b82762be4ff115a8327090501c6a03e0983b"),
        "zeros" => b256!("0bd94b83e5f67d384bbbf2a21e3ae26e58caf51aedc1f5d51c8314b1c0a6c07b"),
        "maxed" => b256!("4dacc0230564d04ba8e4e12d03215c76ed2381eb868ed78b870c60064c9303e7"),
        "basic_fok" => b256!("e3b007a1c212696aa565ccd6daae20d439aa584796dd949c6e04f0f4b753a56a"),
        other => panic!("unknown vector {other}"),
    }
}

#[test]
fn type_string_is_single_sourced() {
    // Alloy's encoding must equal the canonical string byte for byte.
    assert_eq!(eip712::alloy_encode_type(), ORDER_TYPE_STRING);
}

#[test]
fn type_hash_is_pinned() {
    assert_eq!(
        eip712::order_type_hash(),
        b256!("d81e363e64f113b5ef986b1402957b28b98d171832da42cb5e62a904e2dcb564")
    );
}

#[test]
fn digests_match_pinned_vectors() {
    let d = domain();
    for (name, o) in vectors() {
        assert_eq!(
            eip712::signing_hash(&o, &d),
            expected(name),
            "vector {name}"
        );
    }
}

#[test]
fn bool_flip_changes_digest() {
    let d = domain();
    let v = vectors();
    let basic = v
        .iter()
        .find(|(n, _)| *n == "basic")
        .map(|(_, o)| o)
        .unwrap();
    let fok = v
        .iter()
        .find(|(n, _)| *n == "basic_fok")
        .map(|(_, o)| o)
        .unwrap();
    assert_ne!(
        eip712::signing_hash(basic, &d),
        eip712::signing_hash(fok, &d)
    );
}
