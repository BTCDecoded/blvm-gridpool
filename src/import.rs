//! The two imports `BootPeerSyncService` performs after a summary poll.
//!
//! A candidate bundle is trimmed with `SortAndTrimProofs` (timestamp tie-break).
//! A locked sibling bundle uses `Reconcile` (ordinal share-id tie-break).

use std::cmp::Ordering;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::bitcoin::{
    address_to_script, encode_hex, hashes_equivalent, normalize_hex, reverse_hex,
};
use crate::family::{matches_family, FamilyMember, FamilyState, CONSENSUS_VERSION};
use crate::payout::shared_slot_value_sats;
use crate::reconcile::{reconcile, ShareProof};
use crate::verify::{validate_share, ExpectedWinner, ShareSubmission};

pub const HTTP_API_VERSION: i32 = 1;
pub const STATE_BUNDLE_SCHEMA_VERSION: i32 = 3;
pub const PEER_TRANSPORT_VERSION: i32 = 2;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DeckProof {
    pub share_id: String,
    pub difficulty: f64,
    pub script_pub_key_hex: String,
    pub prev_block_hash: String,
    pub timestamp_millis: i64,
    pub miner_address: String,
    pub header_hex: String,
    pub coinbase_hex: String,
    pub merkle_path: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct WinnerRow {
    pub value_sats: u64,
    pub address: String,
    pub difficulty: f64,
}

#[derive(Clone, Debug)]
pub struct PayoutMath {
    pub subsidy_sats: u64,
    pub total_slots: u64,
    pub shared_slots: usize,
    pub support_address: Option<String>,
    pub network: String,
}

#[derive(Clone, Debug, PartialEq)]
pub enum AdoptOutcome {
    Unchanged,
    Reconciled {
        reconciled: Vec<ShareProof>,
        on_deck: Vec<ShareProof>,
    },
}

#[derive(Clone, Debug, PartialEq)]
pub struct VersionFields {
    pub consensus_version: i32,
    pub state_bundle_schema_version: i32,
    pub http_api_version: i32,
}

#[derive(Clone, Debug, PartialEq)]
pub struct PollSnapshot {
    pub winners_count: i32,
    pub on_deck_count: i32,
    pub current_round_number: i32,
    pub current_state_id: String,
    pub current_state_proof_count: i32,
    pub current_state_total_difficulty: f64,
}

pub fn can_sync_state(
    local: &VersionFields,
    remote: &VersionFields,
    local_network_id: &str,
    remote_network_id: &str,
) -> bool {
    local_network_id
        .trim()
        .eq_ignore_ascii_case(remote_network_id.trim())
        && local.consensus_version == remote.consensus_version
        && local.state_bundle_schema_version == remote.state_bundle_schema_version
        && (remote.http_api_version == 0 || remote.http_api_version == local.http_api_version)
}

pub fn should_fetch_remote_current_state(local: &PollSnapshot, remote: &PollSnapshot) -> bool {
    if local.winners_count == 0 || (local.winners_count == 1 && local.on_deck_count == 0) {
        return true;
    }
    if remote.current_round_number > local.current_round_number {
        return true;
    }
    if remote.current_round_number < local.current_round_number {
        return false;
    }
    if !remote
        .current_state_id
        .eq_ignore_ascii_case(&local.current_state_id)
        && local.current_state_proof_count == 0
        && remote.current_state_proof_count > 0
    {
        return true;
    }
    const EPSILON: f64 = 0.000_000_1;
    if remote.current_state_total_difficulty > local.current_state_total_difficulty + EPSILON {
        return true;
    }
    if remote.current_state_total_difficulty + EPSILON < local.current_state_total_difficulty {
        return false;
    }
    remote.current_state_id.as_str() > local.current_state_id.as_str()
}

/// `MergeCandidateProofsIntoCanonicalReserveNoLock` then `SortAndTrimProofs`.
pub fn merge_candidate_reserve(
    local: &[DeckProof],
    remote: &[DeckProof],
    current_tip: Option<&str>,
    limit: usize,
) -> Vec<DeckProof> {
    let mut merged = local.to_vec();
    for proof in remote {
        if should_merge(proof, local, current_tip) {
            merged.push(proof.clone());
        }
    }
    sort_and_trim(&merged, limit)
}

fn should_merge(proof: &DeckProof, local: &[DeckProof], current_tip: Option<&str>) -> bool {
    if proof.share_id.trim().is_empty() {
        return false;
    }
    if local
        .iter()
        .any(|known| known.share_id.eq_ignore_ascii_case(&proof.share_id))
    {
        return true;
    }
    match current_tip {
        None => true,
        Some(tip) if tip.trim().is_empty() => true,
        Some(tip) => hashes_equivalent(&proof.prev_block_hash, tip),
    }
}

pub fn sort_and_trim(proofs: &[DeckProof], limit: usize) -> Vec<DeckProof> {
    if limit == 0 {
        return Vec::new();
    }
    let mut indexed: Vec<(usize, DeckProof)> = proofs
        .iter()
        .cloned()
        .enumerate()
        .filter(|(_, proof)| !proof.share_id.trim().is_empty())
        .collect();
    let mut groups: Vec<Vec<(usize, DeckProof)>> = Vec::new();
    for item in indexed.drain(..) {
        if let Some(group) = groups
            .iter_mut()
            .find(|group| group[0].1.share_id.eq_ignore_ascii_case(&item.1.share_id))
        {
            group.push(item);
        } else {
            groups.push(vec![item]);
        }
    }
    let mut best: Vec<DeckProof> = groups
        .into_iter()
        .map(|mut group| {
            group.sort_by(|left, right| {
                match diff_cmp(left.1.difficulty, right.1.difficulty) {
                    Ordering::Equal => {}
                    other => return other.reverse(),
                }
                match left.1.timestamp_millis.cmp(&right.1.timestamp_millis) {
                    Ordering::Equal => left.0.cmp(&right.0),
                    other => other,
                }
            });
            group.remove(0).1
        })
        .collect();
    best.sort_by(
        |left, right| match diff_cmp(left.difficulty, right.difficulty) {
            Ordering::Equal => left.share_id.as_str().cmp(right.share_id.as_str()),
            other => other.reverse(),
        },
    );
    best.truncate(limit);
    best
}

pub fn compute_state_id(
    consensus_version: i32,
    network_id: &str,
    block_hash: Option<&str>,
    payout_variant: &str,
    shares: &[DeckProof],
) -> String {
    digest_state(
        "boot-protocol-state",
        consensus_version,
        network_id,
        normalize_block_hash(block_hash.unwrap_or("")),
        payout_variant,
        shares,
    )
}

pub fn compute_candidate_state_id(
    consensus_version: i32,
    network_id: &str,
    current_state_id: &str,
    payout_variant: &str,
    shares: &[DeckProof],
) -> String {
    digest_state(
        "boot-protocol-candidate-state",
        consensus_version,
        network_id,
        Some(current_state_id.to_string()).filter(|id| !id.is_empty()),
        payout_variant,
        shares,
    )
}

fn digest_state(
    domain: &str,
    consensus_version: i32,
    network_id: &str,
    line: Option<String>,
    payout_variant: &str,
    shares: &[DeckProof],
) -> String {
    let mut body = String::new();
    body.push_str(domain);
    body.push('\n');
    body.push_str(&consensus_version.to_string());
    body.push('\n');
    body.push_str(network_id);
    body.push('\n');
    body.push_str(line.as_deref().unwrap_or(""));
    body.push('\n');
    body.push_str(payout_variant);
    body.push('\n');
    let mut ordered = shares.to_vec();
    ordered.sort_by(
        |left, right| match diff_cmp(left.difficulty, right.difficulty) {
            Ordering::Equal => left.share_id.as_str().cmp(right.share_id.as_str()),
            other => other.reverse(),
        },
    );
    for (index, share) in ordered.iter().enumerate() {
        body.push_str(&format!(
            "{index}|{}|{}|{}\n",
            normalize_hex(&share.script_pub_key_hex),
            difficulty_round_trip(share.difficulty),
            share.share_id
        ));
    }
    encode_hex(&Sha256::digest(body.as_bytes()))
}

fn normalize_block_hash(hash: &str) -> Option<String> {
    let normalized = normalize_hex(hash);
    if normalized.len() != 64 || !normalized.chars().all(|c| c.is_ascii_hexdigit()) {
        return None;
    }
    let reversed = reverse_hex(&normalized);
    let leading = |hex: &str| hex.chars().take_while(|c| *c == '0').count();
    Some(if leading(&normalized) > leading(&reversed) {
        normalized
    } else {
        reversed
    })
}

fn difficulty_round_trip(value: f64) -> String {
    if !value.is_finite() {
        return "NaN".into();
    }
    if value == 0.0 {
        return "0".into();
    }
    if value.fract() == 0.0 && value.abs() < i64::MAX as f64 {
        return format!("{}", value as i64);
    }
    let rendered = format!("{value:?}");
    let Some((mantissa, exponent)) = rendered.split_once(['e', 'E']) else {
        return rendered;
    };
    let exponent: i32 = exponent.parse().unwrap_or(0);
    format!("{mantissa}E{exponent:+03}")
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ImportError {
    VersionMismatch,
    StateIdMismatch,
    FamilyMismatch,
    WinnersMismatch,
    InvalidProof,
}

pub fn winners_match(expected: &[WinnerRow], actual: &[WinnerRow], network: &str) -> bool {
    if expected.len() != actual.len() {
        return false;
    }
    expected.iter().zip(actual).all(|(left, right)| {
        left.value_sats == right.value_sats
            && (left.difficulty - right.difficulty).abs() <= 1e-7
            && scripts_match(&left.address, &right.address, network)
    })
}

pub fn payouts_from_proofs(proofs: &[DeckProof], math: &PayoutMath) -> Vec<WinnerRow> {
    let mut ordered = proofs.to_vec();
    ordered.sort_by(|left, right| {
        diff_cmp(right.difficulty, left.difficulty).then_with(|| left.share_id.cmp(&right.share_id))
    });
    ordered.truncate(math.shared_slots);
    let value = shared_slot_value_sats(math.subsidy_sats, math.total_slots);
    let mut rows = Vec::new();
    if let Some(address) = &math.support_address {
        rows.push(WinnerRow {
            value_sats: value,
            address: address.clone(),
            difficulty: 0.0,
        });
    }
    rows.extend(ordered.into_iter().map(|proof| WinnerRow {
        value_sats: value,
        address: proof.miner_address,
        difficulty: proof.difficulty,
    }));
    rows
}

pub fn parent_sets_overlap(local: &[String], remote: &[String]) -> bool {
    let local: Vec<&String> = local
        .iter()
        .filter(|hash| !hash.trim().is_empty())
        .collect();
    let remote: Vec<&String> = remote
        .iter()
        .filter(|hash| !hash.trim().is_empty())
        .collect();
    local.is_empty()
        || remote.is_empty()
        || local
            .iter()
            .any(|left| remote.iter().any(|right| hashes_equivalent(left, right)))
}

pub fn bootstrap_current(
    winners: &[WinnerRow],
    work_set: &[DeckProof],
    observed_tip: &str,
    limit: usize,
    verifier_winners: &[ExpectedWinner],
    parents: &[String],
    math: &PayoutMath,
) -> Result<(Vec<WinnerRow>, Vec<DeckProof>), ImportError> {
    if observed_tip.trim().is_empty() || winners.is_empty() {
        return Err(ImportError::InvalidProof);
    }
    let validated = validate_imported(work_set, verifier_winners, parents, &math.network)?;
    Ok((winners.to_vec(), sort_and_trim(&validated, limit)))
}

pub fn import_candidate(
    local_version: &VersionFields,
    remote_version: &VersionFields,
    local_network_id: &str,
    remote_network_id: &str,
    current_state_id: &str,
    claimed_state_id: &str,
    payout_variant: &str,
    local: &[DeckProof],
    remote: &[DeckProof],
    current_tip: Option<&str>,
    limit: usize,
    verifier_winners: &[ExpectedWinner],
    claimed_winners: &[WinnerRow],
    math: &PayoutMath,
    local_parents: &[String],
    remote_parents: &[String],
) -> Result<Vec<DeckProof>, ImportError> {
    if !can_sync_state(
        local_version,
        remote_version,
        local_network_id,
        remote_network_id,
    ) {
        return Err(ImportError::VersionMismatch);
    }
    let validation_parents = merged_parents(local_parents, remote_parents);
    if !parent_sets_overlap(local_parents, remote_parents)
        || !tip_is_compatible(current_tip, &validation_parents)
    {
        return Err(ImportError::InvalidProof);
    }
    let validated =
        validate_imported(remote, verifier_winners, &validation_parents, &math.network)?;
    let expected = compute_candidate_state_id(
        remote_version.consensus_version,
        remote_network_id,
        current_state_id,
        payout_variant,
        &validated,
    );
    if !expected.eq_ignore_ascii_case(claimed_state_id) {
        return Err(ImportError::StateIdMismatch);
    }
    if !winners_match(
        &payouts_from_proofs(&validated, math),
        claimed_winners,
        &math.network,
    ) {
        return Err(ImportError::WinnersMismatch);
    }
    Ok(merge_candidate_reserve(
        local,
        &validated,
        current_tip,
        limit,
    ))
}

pub fn import_locked_sibling(
    family: &FamilyState,
    member: &FamilyMember,
    local_reconciled: &[ShareProof],
    boundary: &[DeckProof],
    paid_ids: &[String],
    on_deck: &[ShareProof],
    reserve_limit: usize,
    math: &PayoutMath,
    network_id: &str,
    payout_variant: &str,
    claimed_snapshot_id: &str,
    active_snapshot_id: &str,
    local_tip: &str,
    local_tip_height: i64,
    local_support_enabled: bool,
    remote_support_enabled: bool,
    verifier_winners: &[ExpectedWinner],
    claimed_winners: &[WinnerRow],
    parents: &[String],
) -> Result<AdoptOutcome, ImportError> {
    if family.consensus_version != CONSENSUS_VERSION
        || !matches_family(family, member)
        || !payout_variant.eq_ignore_ascii_case(&member.payout_variant)
        || local_support_enabled != remote_support_enabled
        || !hashes_equivalent(&member.boundary_block_hash, local_tip)
        || member.boundary_block_height != local_tip_height
    {
        return Err(ImportError::FamilyMismatch);
    }
    if boundary.iter().all(|proof| {
        local_reconciled
            .iter()
            .any(|known| known.share_id.eq_ignore_ascii_case(&proof.share_id))
    }) {
        let mut known: Vec<ShareProof> = local_reconciled
            .iter()
            .filter(|proof| {
                boundary
                    .iter()
                    .any(|remote| remote.share_id.eq_ignore_ascii_case(&proof.share_id))
            })
            .cloned()
            .collect();
        known.sort_by(|left, right| {
            diff_cmp(right.difficulty, left.difficulty)
                .then_with(|| left.share_id.cmp(&right.share_id))
        });
        known.truncate(reserve_limit);
        let snapshot = decks_for_shares(&known, boundary, math.shared_slots);
        return check_member_snapshot(
            family,
            &snapshot,
            network_id,
            payout_variant,
            &member.boundary_block_hash,
            claimed_snapshot_id,
            active_snapshot_id,
            math,
            claimed_winners,
        )
        .map(|()| AdoptOutcome::Unchanged);
    }
    let validated = validate_imported(boundary, verifier_winners, parents, &math.network)?;
    let boundary_shares = share_proofs(&validated);
    let member_reserve = reconcile(&[], &boundary_shares, paid_ids, reserve_limit);
    let member_snapshot = decks_for_shares(&member_reserve, &validated, math.shared_slots);
    check_member_snapshot(
        family,
        &member_snapshot,
        network_id,
        payout_variant,
        &member.boundary_block_hash,
        claimed_snapshot_id,
        active_snapshot_id,
        math,
        claimed_winners,
    )?;
    let reconciled = reconcile(local_reconciled, &boundary_shares, paid_ids, reserve_limit);
    let on_deck = reconcile(on_deck, &reconciled, paid_ids, reserve_limit);
    Ok(AdoptOutcome::Reconciled {
        reconciled,
        on_deck,
    })
}

fn check_member_snapshot(
    family: &FamilyState,
    snapshot: &[DeckProof],
    network_id: &str,
    payout_variant: &str,
    boundary_hash: &str,
    claimed_snapshot_id: &str,
    active_snapshot_id: &str,
    math: &PayoutMath,
    claimed_winners: &[WinnerRow],
) -> Result<(), ImportError> {
    let expected = compute_state_id(
        family.consensus_version,
        network_id,
        Some(boundary_hash),
        payout_variant,
        snapshot,
    );
    if !expected.eq_ignore_ascii_case(claimed_snapshot_id)
        || !claimed_snapshot_id.eq_ignore_ascii_case(active_snapshot_id)
    {
        return Err(ImportError::StateIdMismatch);
    }
    if !winners_match(
        &payouts_from_proofs(snapshot, math),
        claimed_winners,
        &math.network,
    ) {
        return Err(ImportError::WinnersMismatch);
    }
    Ok(())
}

fn validate_imported(
    proofs: &[DeckProof],
    verifier_winners: &[ExpectedWinner],
    parents: &[String],
    network: &str,
) -> Result<Vec<DeckProof>, ImportError> {
    let mut ordered = proofs.to_vec();
    ordered.sort_by(|left, right| {
        diff_cmp(right.difficulty, left.difficulty).then_with(|| left.share_id.cmp(&right.share_id))
    });
    let mut validated = Vec::with_capacity(ordered.len());
    for proof in ordered {
        if proof.header_hex.trim().is_empty() {
            return Err(ImportError::InvalidProof);
        }
        let result = validate_share(
            &ShareSubmission {
                header_hex: proof.header_hex.clone(),
                coinbase_hex: proof.coinbase_hex.clone(),
                merkle_path: proof.merkle_path.clone(),
                prev_block_hash: proof.prev_block_hash.clone(),
                username: proof.miner_address.clone(),
                expected_share_id: Some(proof.share_id.clone()),
            },
            verifier_winners,
            parents,
            network,
        );
        if !result.valid {
            return Err(ImportError::InvalidProof);
        }
        validated.push(DeckProof {
            share_id: result.share_id,
            difficulty: result.difficulty,
            script_pub_key_hex: result.script_pub_key_hex,
            prev_block_hash: result.prev_block_hash,
            timestamp_millis: proof.timestamp_millis,
            miner_address: result.miner_address,
            header_hex: proof.header_hex,
            coinbase_hex: proof.coinbase_hex,
            merkle_path: proof.merkle_path,
        });
    }
    Ok(validated)
}

fn decks_for_shares(
    shares: &[ShareProof],
    source: &[DeckProof],
    shared_slots: usize,
) -> Vec<DeckProof> {
    shares
        .iter()
        .take(shared_slots)
        .filter_map(|proof| {
            source
                .iter()
                .find(|deck| deck.share_id.eq_ignore_ascii_case(&proof.share_id))
                .cloned()
        })
        .collect()
}

fn share_proofs(proofs: &[DeckProof]) -> Vec<ShareProof> {
    proofs
        .iter()
        .map(|proof| ShareProof {
            share_id: proof.share_id.clone(),
            difficulty: proof.difficulty,
        })
        .collect()
}

fn merged_parents(local: &[String], remote: &[String]) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for hash in local.iter().chain(remote) {
        if hash.trim().is_empty() {
            continue;
        }
        if !out.iter().any(|existing| hashes_equivalent(existing, hash)) {
            out.push(hash.clone());
        }
    }
    out
}

fn tip_is_compatible(tip: Option<&str>, parents: &[String]) -> bool {
    match tip.map(str::trim).filter(|tip| !tip.is_empty()) {
        None => true,
        Some(tip) => parents.iter().any(|hash| hashes_equivalent(hash, tip)),
    }
}

fn scripts_match(left: &str, right: &str, network: &str) -> bool {
    match (
        address_to_script(left, network),
        address_to_script(right, network),
    ) {
        (Ok(left), Ok(right)) => encode_hex(&left).eq_ignore_ascii_case(&encode_hex(&right)),
        _ => false,
    }
}

fn diff_cmp(left: f64, right: f64) -> Ordering {
    match (left.is_nan(), right.is_nan()) {
        (true, true) => Ordering::Equal,
        (true, false) => Ordering::Less,
        (false, true) => Ordering::Greater,
        (false, false) => left.partial_cmp(&right).unwrap_or(Ordering::Equal),
    }
}
