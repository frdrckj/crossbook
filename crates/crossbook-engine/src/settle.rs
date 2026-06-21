//! Translate the matcher's output into settlement calldata.
//!
//! The core produces pairwise fills (a maker hash, a taker hash, and the amounts
//! from the maker's side). The contract wants a deduped list of signed orders and
//! per order fill rows. Each pairwise fill becomes two rows: the maker's, and the
//! taker's mirror (the taker sends what the maker received and receives what the
//! maker sent).

use crate::chain::Settlement;
use alloy::primitives::{Bytes, U256};
use anyhow::{bail, Result};
use crossbook_core::types::{Fill as CoreFill, Order as CoreOrder, OrderHash};
use std::collections::HashMap;

/// An order admitted by the engine, kept with its signature so it can be settled.
#[derive(Clone, Debug)]
pub struct AdmittedOrder {
    pub order: CoreOrder,
    pub signature: Vec<u8>,
}

fn to_sol_order(o: &CoreOrder) -> Settlement::Order {
    Settlement::Order {
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

fn index_of(
    hash: &OrderHash,
    index: &mut HashMap<OrderHash, usize>,
    signed: &mut Vec<Settlement::SignedOrder>,
    admitted: &HashMap<OrderHash, AdmittedOrder>,
) -> Result<usize> {
    if let Some(i) = index.get(hash) {
        return Ok(*i);
    }
    let Some(a) = admitted.get(hash) else {
        bail!("missing admitted order for a fill");
    };
    let i = signed.len();
    signed.push(Settlement::SignedOrder {
        order: to_sol_order(&a.order),
        signature: Bytes::from(a.signature.clone()),
    });
    index.insert(*hash, i);
    Ok(i)
}

/// Build the `settle` arguments from core fills and the admitted orders they
/// reference. Errors if a referenced order is not known.
pub fn to_settlement(
    fills: &[CoreFill],
    admitted: &HashMap<OrderHash, AdmittedOrder>,
) -> Result<(Vec<Settlement::SignedOrder>, Vec<Settlement::Fill>)> {
    let mut index = HashMap::new();
    let mut signed = Vec::new();
    let mut rows = Vec::with_capacity(fills.len() * 2);

    for f in fills {
        let maker = index_of(&f.maker_hash, &mut index, &mut signed, admitted)?;
        let taker = index_of(&f.taker_hash, &mut index, &mut signed, admitted)?;
        rows.push(Settlement::Fill {
            orderIndex: U256::from(maker),
            sellFilled: f.sell_filled,
            buyFilled: f.buy_filled,
        });
        rows.push(Settlement::Fill {
            orderIndex: U256::from(taker),
            sellFilled: f.buy_filled,
            buyFilled: f.sell_filled,
        });
    }
    Ok((signed, rows))
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloy::primitives::Address;
    use crossbook_core::types::Order;

    fn order(st: u8, sa: u64, bt: u8, ba: u64) -> Order {
        Order {
            maker: Address::repeat_byte(0xAA),
            sell_token: Address::repeat_byte(st),
            buy_token: Address::repeat_byte(bt),
            sell_amount: U256::from(sa),
            buy_amount: U256::from(ba),
            valid_to: 1,
            nonce: U256::from(1u64),
            partially_fillable: true,
        }
    }

    #[test]
    fn pairwise_fill_becomes_maker_and_mirrored_taker_rows() {
        let mut admitted = HashMap::new();
        admitted.insert(
            [1u8; 32],
            AdmittedOrder {
                order: order(0x0A, 100, 0x0B, 120),
                signature: vec![0xAA],
            },
        );
        admitted.insert(
            [2u8; 32],
            AdmittedOrder {
                order: order(0x0B, 120, 0x0A, 100),
                signature: vec![0xBB],
            },
        );

        let fills = vec![CoreFill {
            maker_hash: [1u8; 32],
            taker_hash: [2u8; 32],
            sell_filled: U256::from(100u64),
            buy_filled: U256::from(120u64),
        }];

        let (signed, rows) = to_settlement(&fills, &admitted).unwrap();
        assert_eq!(signed.len(), 2);
        assert_eq!(rows.len(), 2);

        // maker row keeps the amounts as given
        assert_eq!(rows[0].orderIndex, U256::from(0u64));
        assert_eq!(rows[0].sellFilled, U256::from(100u64));
        assert_eq!(rows[0].buyFilled, U256::from(120u64));

        // taker row mirrors them
        assert_eq!(rows[1].orderIndex, U256::from(1u64));
        assert_eq!(rows[1].sellFilled, U256::from(120u64));
        assert_eq!(rows[1].buyFilled, U256::from(100u64));
    }

    #[test]
    fn missing_order_is_an_error() {
        let admitted = HashMap::new();
        let fills = vec![CoreFill {
            maker_hash: [1u8; 32],
            taker_hash: [2u8; 32],
            sell_filled: U256::from(1u64),
            buy_filled: U256::from(1u64),
        }];
        assert!(to_settlement(&fills, &admitted).is_err());
    }
}
