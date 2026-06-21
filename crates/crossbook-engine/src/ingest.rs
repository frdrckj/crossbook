//! Order intake and validation. Turns a signed order payload into an admitted
//! order, or into a typed RejectReason. The signature recovery and static checks
//! are pure; balance and allowance are read from chain.

use crate::chain::Chain;
use crate::reject::RejectReason;
use crate::settle::AdmittedOrder;
use alloy_primitives::{Address, Bytes, Signature, B256, U256};
use alloy_sol_types::Eip712Domain;
use crossbook_core::eip712;
use crossbook_core::types::{OpenOrder, Order};
use serde::{Deserialize, Serialize};

/// The wire form of a signed order, as posted to the engine.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OrderPayload {
    pub maker: Address,
    pub sell_token: Address,
    pub buy_token: Address,
    pub sell_amount: U256,
    pub buy_amount: U256,
    pub valid_to: u64,
    pub nonce: U256,
    pub partially_fillable: bool,
    pub signature: Bytes,
}

/// A validated order, ready to feed the matcher and later settle.
pub struct Validated {
    pub open: OpenOrder,
    pub admitted: AdmittedOrder,
}

impl OrderPayload {
    fn to_order(&self) -> Order {
        Order {
            maker: self.maker,
            sell_token: self.sell_token,
            buy_token: self.buy_token,
            sell_amount: self.sell_amount,
            buy_amount: self.buy_amount,
            valid_to: self.valid_to,
            nonce: self.nonce,
            partially_fillable: self.partially_fillable,
        }
    }
}

fn check_static(o: &Order, now: u64) -> Result<(), RejectReason> {
    if o.sell_amount.is_zero() || o.buy_amount.is_zero() || o.sell_token == o.buy_token {
        return Err(RejectReason::Malformed);
    }
    if now > o.valid_to {
        return Err(RejectReason::Expired);
    }
    Ok(())
}

/// Recover the signer from the EIP-712 digest and require it to be the maker.
/// Returns the digest (the order id) on success.
fn check_signature(
    payload: &OrderPayload,
    order: &Order,
    domain: &Eip712Domain,
) -> Result<B256, RejectReason> {
    let digest = eip712::signing_hash(order, domain);
    let sig =
        Signature::try_from(payload.signature.as_ref()).map_err(|_| RejectReason::BadSignature)?;
    let recovered = sig
        .recover_address_from_prehash(&digest)
        .map_err(|_| RejectReason::BadSignature)?;
    if recovered != order.maker {
        return Err(RejectReason::BadSignature);
    }
    Ok(digest)
}

/// Full intake: static checks, signature, then balance and allowance from chain.
pub async fn validate(
    payload: &OrderPayload,
    chain: &Chain,
    domain: &Eip712Domain,
    now: u64,
    arrival_seq: u64,
) -> Result<Validated, RejectReason> {
    let order = payload.to_order();
    check_static(&order, now)?;
    let digest = check_signature(payload, &order, domain)?;

    let balance = chain
        .balance_of(order.sell_token, order.maker)
        .await
        .map_err(|_| RejectReason::UnsupportedToken)?;
    if balance < order.sell_amount {
        return Err(RejectReason::InsufficientBalance);
    }
    let allowance = chain
        .allowance(order.sell_token, order.maker)
        .await
        .map_err(|_| RejectReason::UnsupportedToken)?;
    if allowance < order.sell_amount {
        return Err(RejectReason::InsufficientAllowance);
    }

    let open = OpenOrder::new(order.clone(), digest.0, arrival_seq)
        .map_err(|_| RejectReason::Malformed)?;
    let admitted = AdmittedOrder {
        order,
        signature: payload.signature.to_vec(),
    };
    Ok(Validated { open, admitted })
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloy::signers::local::PrivateKeySigner;
    use alloy::signers::SignerSync;

    fn domain() -> Eip712Domain {
        eip712::crossbook_domain(31337, Address::repeat_byte(0x42))
    }

    fn unsigned(maker: Address) -> OrderPayload {
        OrderPayload {
            maker,
            sell_token: Address::repeat_byte(0x0A),
            buy_token: Address::repeat_byte(0x0B),
            sell_amount: U256::from(100u64),
            buy_amount: U256::from(200u64),
            valid_to: 1_700_000_000,
            nonce: U256::from(1u64),
            partially_fillable: true,
            signature: Bytes::new(),
        }
    }

    fn signed(signer: &PrivateKeySigner) -> OrderPayload {
        let mut p = unsigned(signer.address());
        let order = p.to_order();
        let digest = eip712::signing_hash(&order, &domain());
        let sig = signer.sign_hash_sync(&digest).unwrap();
        p.signature = Bytes::from(sig.as_bytes().to_vec());
        p
    }

    #[test]
    fn valid_signature_recovers_to_maker() {
        let signer = PrivateKeySigner::random();
        let p = signed(&signer);
        let order = p.to_order();
        assert!(check_signature(&p, &order, &domain()).is_ok());
    }

    #[test]
    fn tampered_signature_is_rejected() {
        let signer = PrivateKeySigner::random();
        let mut p = signed(&signer);
        let mut bytes = p.signature.to_vec();
        bytes[10] ^= 0xff;
        p.signature = Bytes::from(bytes);
        let order = p.to_order();
        assert_eq!(
            check_signature(&p, &order, &domain()),
            Err(RejectReason::BadSignature)
        );
    }

    #[test]
    fn signature_by_another_key_is_rejected() {
        let signer = PrivateKeySigner::random();
        let other = PrivateKeySigner::random();
        let mut p = signed(&signer);
        p.maker = other.address(); // claims a different maker than signed
        let order = p.to_order();
        assert_eq!(
            check_signature(&p, &order, &domain()),
            Err(RejectReason::BadSignature)
        );
    }

    #[test]
    fn static_checks_catch_expiry_and_degenerate_orders() {
        let signer = PrivateKeySigner::random();
        let order = unsigned(signer.address()).to_order();
        assert!(check_static(&order, 1_600_000_000).is_ok());
        assert_eq!(
            check_static(&order, 1_700_000_001),
            Err(RejectReason::Expired)
        );

        let mut zero = order.clone();
        zero.sell_amount = U256::ZERO;
        assert_eq!(check_static(&zero, 0), Err(RejectReason::Malformed));

        let mut same = order.clone();
        same.buy_token = same.sell_token;
        assert_eq!(check_static(&same, 0), Err(RejectReason::Malformed));
    }
}
