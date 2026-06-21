//! EIP-712 hashing for orders, the Rust half of the cross language parity gate.
//!
//! The `Order` struct is declared with Alloy's `sol!` macro so its typed data
//! encoding matches Solidity exactly. The digest is then derived with
//! `SolStruct::eip712_signing_hash`, which keeps the Rust digest identical to an
//! OpenZeppelin `EIP712` plus a hand written `OrderLib.sol`. This crate stays
//! pure: `alloy-sol-types` is a compile time crate, no providers or transports.
//!
//! `valid_to` is a `u64` in the ergonomic domain type but is encoded as
//! `uint256` here, exactly as the canonical schema requires.

use crate::types::Order as DomainOrder;
use alloy_primitives::{keccak256, Address, B256, U256};
use alloy_sol_types::{sol, Eip712Domain, SolStruct};

sol! {
    struct Order {
        address maker;
        address sellToken;
        address buyToken;
        uint256 sellAmount;
        uint256 buyAmount;
        uint256 validTo;
        uint256 nonce;
        bool partiallyFillable;
    }
}

/// The canonical EIP-712 type string. Single source of truth, mirrored in
/// `OrderLib.sol` and asserted equal to Alloy's encoding in the parity test.
pub const ORDER_TYPE_STRING: &str = "Order(address maker,address sellToken,address buyToken,uint256 sellAmount,uint256 buyAmount,uint256 validTo,uint256 nonce,bool partiallyFillable)";

impl From<&DomainOrder> for Order {
    fn from(o: &DomainOrder) -> Self {
        Order {
            maker: o.maker,
            sellToken: o.sell_token,
            buyToken: o.buy_token,
            sellAmount: o.sell_amount,
            buyAmount: o.buy_amount,
            validTo: U256::from(o.valid_to),
            nonce: o.nonce,
            partiallyFillable: o.partially_fillable,
        }
    }
}

/// `keccak256(ORDER_TYPE_STRING)`.
pub fn order_type_hash() -> B256 {
    keccak256(ORDER_TYPE_STRING.as_bytes())
}

/// The EIP-712 type string as Alloy encodes it (used to assert single sourcing).
pub fn alloy_encode_type() -> String {
    Order::eip712_encode_type().into_owned()
}

/// `hashStruct(order)`, domain independent.
pub fn struct_hash(order: &DomainOrder) -> B256 {
    Order::from(order).eip712_hash_struct()
}

/// The full EIP-712 digest: `keccak256(0x1901 ++ domainSeparator ++ hashStruct)`.
pub fn signing_hash(order: &DomainOrder, domain: &Eip712Domain) -> B256 {
    Order::from(order).eip712_signing_hash(domain)
}

/// Build the Crossbook EIP-712 domain for a chain and deployed contract.
pub fn crossbook_domain(chain_id: u64, verifying_contract: Address) -> Eip712Domain {
    Eip712Domain {
        name: Some("Crossbook".into()),
        version: Some("1".into()),
        chain_id: Some(U256::from(chain_id)),
        verifying_contract: Some(verifying_contract),
        salt: None,
    }
}
