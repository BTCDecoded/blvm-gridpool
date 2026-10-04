//! `BootShareVerifier.ValidateShare` on the pinned commit.
//!
//! Parent-set overlap against the caller's expected parents is inside this
//! function, matching `ValidateCore`. Import-level state id and winners-list
//! checks stay outside it. `reconcile` still sees only share id and difficulty.

use sha2::{Digest, Sha256};

use crate::bitcoin::{
    address_to_script, decode_compact_target, decode_hex, display_hash, double_sha256, encode_hex,
    hashes_equivalent, header_difficulty, normalize_hex, parse_outputs, script_to_address,
    transaction_id,
};

#[derive(Clone, Debug, PartialEq)]
pub struct ShareSubmission {
    pub header_hex: String,
    pub coinbase_hex: String,
    pub merkle_path: Vec<String>,
    pub prev_block_hash: String,
    pub username: String,
    pub expected_share_id: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExpectedWinner {
    pub value_sats: u64,
    pub address: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ShareValidation {
    pub valid: bool,
    pub reason: Option<String>,
    pub share_id: String,
    pub miner_address: String,
    pub script_pub_key_hex: String,
    pub prev_block_hash: String,
    pub difficulty: f64,
}

pub fn coinbase_txid_hex(coinbase_hex: &str) -> Result<String, String> {
    let bytes = decode_hex(coinbase_hex)?;
    Ok(encode_hex(&transaction_id(&bytes)?))
}

pub fn expected_winners_from_coinbase(coinbase_hex: &str, network: &str) -> Vec<ExpectedWinner> {
    let Ok(bytes) = decode_hex(coinbase_hex) else {
        return Vec::new();
    };
    let Ok(outputs) = parse_outputs(&bytes) else {
        return Vec::new();
    };
    outputs
        .into_iter()
        .skip(1)
        .filter_map(|output| {
            let address = script_to_address(&output.script, network)?;
            Some(ExpectedWinner {
                value_sats: output.value_sats,
                address,
            })
        })
        .collect()
}

pub fn validate_share(
    share: &ShareSubmission,
    expected_winners: &[ExpectedWinner],
    expected_prev_block_hashes: &[String],
    network: &str,
) -> ShareValidation {
    match validate_core(share, expected_winners, expected_prev_block_hashes, network) {
        Ok(result) => result,
        Err(reason) => ShareValidation {
            valid: false,
            reason: Some(reason),
            share_id: String::new(),
            miner_address: String::new(),
            script_pub_key_hex: String::new(),
            prev_block_hash: String::new(),
            difficulty: 0.0,
        },
    }
}

fn validate_core(
    share: &ShareSubmission,
    expected_winners: &[ExpectedWinner],
    expected_prev_block_hashes: &[String],
    network: &str,
) -> Result<ShareValidation, String> {
    let header_hex = normalize_hex(&share.header_hex);
    if header_hex.len() != 160 {
        return Err("Header must be exactly 80 bytes.".into());
    }
    let coinbase_hex = normalize_hex(&share.coinbase_hex);
    if coinbase_hex.is_empty() || coinbase_hex.len() % 2 != 0 {
        return Err("Coinbase transaction hex is invalid.".into());
    }
    let header =
        decode_hex(&header_hex).map_err(|_| "Share payload is not valid hex.".to_string())?;
    let coinbase =
        decode_hex(&coinbase_hex).map_err(|_| "Share payload is not valid hex.".to_string())?;
    let mut branches = Vec::new();
    let mut merkle_hex = Vec::new();
    for branch in &share.merkle_path {
        let normalized = normalize_hex(branch);
        if normalized.len() != 64 {
            return Err("Merkle path entries must be 32-byte hashes.".into());
        }
        let bytes =
            decode_hex(&normalized).map_err(|_| "Share payload is not valid hex.".to_string())?;
        branches.push(bytes);
        merkle_hex.push(normalized);
    }
    let actual_prev = display_hash(&header[4..36]);
    if !share.prev_block_hash.trim().is_empty()
        && !hashes_equivalent(&share.prev_block_hash, &actual_prev)
    {
        return Err("Prev block hash does not match the submitted header.".into());
    }
    let expected_parents: Vec<&String> = expected_prev_block_hashes
        .iter()
        .filter(|hash| !hash.trim().is_empty())
        .collect();
    if !expected_parents.is_empty()
        && !expected_parents
            .iter()
            .any(|hash| hashes_equivalent(hash, &actual_prev))
    {
        return Err(format!(
            "Share builds on the wrong parent block ({actual_prev})."
        ));
    }
    let coinbase_hash = transaction_id(&coinbase)?;
    let header_merkle = &header[36..68];
    let computed = merkle_root(&coinbase_hash, &branches);
    if computed.as_slice() != header_merkle {
        let reversed: Vec<Vec<u8>> = branches
            .iter()
            .map(|branch| {
                let mut copy = branch.clone();
                copy.reverse();
                copy
            })
            .collect();
        let alternate = merkle_root(&coinbase_hash, &reversed);
        if alternate.as_slice() != header_merkle {
            return Err("Coinbase transaction does not match the header merkle root.".into());
        }
        branches = reversed;
        merkle_hex = branches.iter().map(|branch| encode_hex(branch)).collect();
    }
    let _ = merkle_hex;
    let outputs = parse_outputs(&coinbase)?;
    if outputs.is_empty() {
        return Err("Coinbase payout list is too short.".into());
    }
    let miner_address = script_to_address(&outputs[0].script, network)
        .ok_or_else(|| "Slot 0 payout address is not a supported standard script.".to_string())?;
    let script_pub_key_hex = encode_hex(&outputs[0].script);
    validate_payout_outputs(&outputs, expected_winners, network)?;
    let header_hash = double_sha256(&header);
    let compact = u32::from_le_bytes(header[72..76].try_into().unwrap());
    let target = decode_compact_target(compact);
    if target == num_bigint::BigUint::from(0u8) {
        return Err("Header target is invalid.".into());
    }
    let difficulty = header_difficulty(&header_hash);
    let share_id = compute_share_id(&header_hex, &coinbase_hex);
    let legacy = compute_legacy_share_id(&header_hex, &coinbase_hex, &miner_address);
    if let Some(expected) = &share.expected_share_id {
        let expected = normalize_hex(expected);
        if !expected.is_empty()
            && !expected.eq_ignore_ascii_case(&share_id)
            && !expected.eq_ignore_ascii_case(&legacy)
        {
            return Err("Share identifier mismatch.".into());
        }
    }
    Ok(ShareValidation {
        valid: true,
        reason: None,
        share_id,
        miner_address,
        script_pub_key_hex,
        prev_block_hash: actual_prev,
        difficulty,
    })
}

fn merkle_root(coinbase_hash: &[u8; 32], branches: &[Vec<u8>]) -> [u8; 32] {
    let mut current = *coinbase_hash;
    for branch in branches {
        let mut joined = current.to_vec();
        joined.extend_from_slice(branch);
        current = double_sha256(&joined);
    }
    current
}

fn compute_share_id(header_hex: &str, coinbase_hex: &str) -> String {
    let digest = Sha256::digest(format!("{header_hex}|{coinbase_hex}").as_bytes());
    encode_hex(&digest)
}

fn compute_legacy_share_id(header_hex: &str, coinbase_hex: &str, miner_address: &str) -> String {
    let digest = Sha256::digest(format!("{header_hex}|{coinbase_hex}|{miner_address}").as_bytes());
    encode_hex(&digest)
}

fn validate_payout_outputs(
    outputs: &[crate::bitcoin::TxOutput],
    expected_winners: &[ExpectedWinner],
    network: &str,
) -> Result<(), String> {
    let legacy = winner_scripts(expected_winners, network, false)?;
    let compressed = winner_scripts(expected_winners, network, true)?;
    let winners: Vec<&crate::bitcoin::TxOutput> = outputs
        .iter()
        .skip(1)
        .filter(|output| output.value_sats > 0)
        .collect();
    if matches_outputs(&winners, &legacy)
        || matches_outputs(&winners, &compressed)
        || matches_aggregated(outputs, &compressed)
    {
        return Ok(());
    }
    let positive = outputs
        .iter()
        .filter(|output| output.value_sats > 0)
        .count();
    if positive == 1 {
        return Err("Coinbase appears to use a non-Boot single-recipient template (likely stale or solo fallback work).".into());
    }
    let truncated = count_truncated(&winners, &compressed);
    if truncated > 0 {
        return Err(format!(
            "Coinbase appears truncated by miner firmware/DATUM coinbase-size selection; matched {truncated} of {} required GridPool payout outputs.",
            compressed.len()
        ));
    }
    Err("Coinbase winners payouts do not match the required Boot outputs.".into())
}

struct WinnerScript {
    value_sats: u64,
    script: Vec<u8>,
}

fn winner_scripts(
    winners: &[ExpectedWinner],
    network: &str,
    compress: bool,
) -> Result<Vec<WinnerScript>, String> {
    let mut out = Vec::new();
    for winner in winners {
        let script = address_to_script(&winner.address, network)?;
        if compress {
            if let Some(existing) = out
                .iter_mut()
                .find(|item: &&mut WinnerScript| item.script == script)
            {
                existing.value_sats = existing.value_sats.saturating_add(winner.value_sats);
                continue;
            }
        }
        out.push(WinnerScript {
            value_sats: winner.value_sats,
            script,
        });
    }
    Ok(out)
}

fn matches_outputs(actual: &[&crate::bitcoin::TxOutput], expected: &[WinnerScript]) -> bool {
    if actual.len() < expected.len() {
        return false;
    }
    expected.iter().enumerate().all(|(index, expected)| {
        actual[index].value_sats == expected.value_sats && actual[index].script == expected.script
    })
}

fn matches_aggregated(actual: &[crate::bitcoin::TxOutput], expected: &[WinnerScript]) -> bool {
    let mut actual_totals: Vec<(Vec<u8>, u64)> = Vec::new();
    for output in actual.iter().filter(|output| output.value_sats > 0) {
        add_total(&mut actual_totals, output.script.clone(), output.value_sats);
    }
    let mut expected_totals: Vec<(Vec<u8>, u64)> = Vec::new();
    for output in expected {
        add_total(
            &mut expected_totals,
            output.script.clone(),
            output.value_sats,
        );
    }
    let mut scripts: Vec<Vec<u8>> = actual_totals
        .iter()
        .chain(expected_totals.iter())
        .map(|(script, _)| script.clone())
        .collect();
    scripts.sort();
    scripts.dedup();
    let mut saw_slot_zero_residual = false;
    for script in scripts {
        let actual_value = total_of(&actual_totals, &script);
        let expected_value = total_of(&expected_totals, &script);
        if actual_value < expected_value {
            return false;
        }
        if actual_value > expected_value {
            if saw_slot_zero_residual {
                return false;
            }
            saw_slot_zero_residual = true;
        }
    }
    saw_slot_zero_residual
}

fn add_total(totals: &mut Vec<(Vec<u8>, u64)>, script: Vec<u8>, value: u64) {
    if let Some((_, existing)) = totals.iter_mut().find(|(key, _)| key == &script) {
        *existing = existing.saturating_add(value);
    } else {
        totals.push((script, value));
    }
}

fn total_of(totals: &[(Vec<u8>, u64)], script: &[u8]) -> u64 {
    totals
        .iter()
        .find(|(key, _)| key == script)
        .map(|(_, value)| *value)
        .unwrap_or(0)
}

fn count_truncated(actual: &[&crate::bitcoin::TxOutput], expected: &[WinnerScript]) -> usize {
    let positive: Vec<_> = actual
        .iter()
        .copied()
        .filter(|output| output.value_sats > 0)
        .collect();
    if positive.is_empty() || positive.len() >= expected.len() {
        return 0;
    }
    for (index, output) in positive.iter().enumerate() {
        if output.value_sats != expected[index].value_sats
            || output.script != expected[index].script
        {
            return 0;
        }
    }
    positive.len()
}
