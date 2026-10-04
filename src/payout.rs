//! Slot arithmetic from handbook V2.1 and `GetSharedPayoutValueSatsNoLock`.
//!
//! Each filled slot is `subsidy / max(2, total_slots)`. Slot 0 also receives
//! transaction fees and `subsidy % total_slots`. Unfilled conceptual slots are
//! not assigned. Addresses and the support output are caller inputs.

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PayoutSlots {
    pub slot_value_sats: u64,
    pub slot0_sats: u64,
    pub support_sats: Option<u64>,
    pub shared_sats: Vec<u64>,
}

pub fn shared_slot_value_sats(subsidy_sats: u64, total_slots: u64) -> u64 {
    let slots = total_slots.max(2);
    subsidy_sats / slots
}

pub fn slot0_value_sats(subsidy_sats: u64, fee_sats: u64, total_slots: u64) -> u64 {
    let slots = total_slots.max(2);
    let slot = subsidy_sats / slots;
    let remainder = subsidy_sats % slots;
    slot + fee_sats + remainder
}

#[cfg(test)]
mod tests {
    use super::coinbase_outputs;

    #[test]
    fn coinbase_rows_use_slot_zero_then_support_then_shared() {
        let rows = coinbase_outputs(
            &[
                "bc1qce93hy5rhg02s6aeu7mfdvxg76x66pqqtrvzs3".into(),
                "bc1qrwsx8fs0l6z7ugp5cvzy6lhss7jlyru3kg9s8y".into(),
            ],
            312_500_000,
            1_000,
            300,
            Some("bc1qrwsx8fs0l6z7ugp5cvzy6lhss7jlyru3kg9s8y"),
            "mainnet",
        )
        .expect("rows");
        assert_eq!(rows.len(), 3);
        assert_eq!(rows[0].value_sats, 1_041_666 + 1_000 + 200);
        assert_eq!(rows[1].value_sats, 1_041_666);
        assert_eq!(rows[2].value_sats, 1_041_666);
        assert!(rows[0].script_hex.starts_with("0014"));
    }
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize)]
pub struct CoinbaseRow {
    #[serde(rename = "value")]
    pub value_sats: u64,
    pub address: String,
    #[serde(rename = "script")]
    pub script_hex: String,
}

/// Published-job coinbase. The first proof is slot 0 and receives fees and the
/// subsidy remainder. A support address, when present, is the next row. Later
/// proofs each receive one shared slot.
pub fn coinbase_outputs(
    addresses: &[String],
    subsidy_sats: u64,
    fee_sats: u64,
    total_slots: u64,
    support_address: Option<&str>,
    network: &str,
) -> Result<Vec<CoinbaseRow>, String> {
    let slots = assign_slots(
        subsidy_sats,
        fee_sats,
        total_slots,
        support_address.is_some(),
        addresses.len().saturating_sub(1),
    );
    let mut rows = Vec::new();
    if let Some(address) = addresses.first() {
        rows.push(coinbase_row(slots.slot0_sats, address, network)?);
    }
    if let Some(address) = support_address {
        rows.push(coinbase_row(
            slots.support_sats.unwrap_or(slots.slot_value_sats),
            address,
            network,
        )?);
    }
    for (address, value) in addresses.iter().skip(1).zip(slots.shared_sats) {
        rows.push(coinbase_row(value, address, network)?);
    }
    Ok(rows)
}

fn coinbase_row(value_sats: u64, address: &str, network: &str) -> Result<CoinbaseRow, String> {
    let script = crate::bitcoin::address_to_script(address, network)?;
    Ok(CoinbaseRow {
        value_sats,
        address: address.to_string(),
        script_hex: crate::bitcoin::encode_hex(&script),
    })
}

/// `shared_count` is the number of proofs selected for shared slots, already
/// capped by the caller (`SharedWinnerSlotCount` when support is on).
pub fn assign_slots(
    subsidy_sats: u64,
    fee_sats: u64,
    total_slots: u64,
    include_support: bool,
    shared_count: usize,
) -> PayoutSlots {
    let slot_value_sats = shared_slot_value_sats(subsidy_sats, total_slots);
    PayoutSlots {
        slot_value_sats,
        slot0_sats: slot0_value_sats(subsidy_sats, fee_sats, total_slots),
        support_sats: include_support.then_some(slot_value_sats),
        shared_sats: vec![slot_value_sats; shared_count],
    }
}
