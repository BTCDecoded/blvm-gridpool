//! Family id from `BootSnapshotReconciliation.ComputeFamilyId` on the pinned commit.
//!
//! Support rules live inside the payout-variant string. There is no second support field.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

/// `develop` tip confirmed at implementation time. Fourteen commits ahead of `v0.2.2-beta.2`.
pub const DEVELOP_PIN: &str = "be0f0b165181616b1b757c27219ed07a9f017e8c";

pub const FAMILY_DOMAIN: &[u8] = b"gridpool-msr-family-v22";

pub const CONSENSUS_VERSION: i32 = 22;

/// `PoolConfig.WinnersListSize` default on the pin.
pub const REFERENCE_WINNERS_LIST_SIZE: i32 = 299;

/// `PoolConfig.GridLabsSupportFeeEnabled` default on the pin.
pub const REFERENCE_SUPPORT_FEE_ENABLED: bool = true;

/// `PoolConfig.WorkSetReserveMultiplier` default on the pin.
pub const REFERENCE_RESERVE_MULTIPLIER: i32 = 3;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct FamilyId(pub String);

pub fn compute_family_id(
    consensus_version: i32,
    network: &str,
    predecessor_snapshot_id: &str,
    boundary_block_hash: &str,
    boundary_height: i64,
    payout_variant: &str,
) -> FamilyId {
    let mut buf = Vec::new();
    buf.extend_from_slice(FAMILY_DOMAIN);
    buf.extend_from_slice(&consensus_version.to_be_bytes());
    write_norm_string(&mut buf, network);
    write_norm_string(&mut buf, predecessor_snapshot_id);
    write_norm_string(&mut buf, boundary_block_hash);
    buf.extend_from_slice(&boundary_height.to_be_bytes());
    write_norm_string(&mut buf, payout_variant);
    let digest = Sha256::digest(&buf);
    FamilyId(hex_lower(&digest))
}

/// Local family a member is compared against. Field names follow `BootSnapshotFamilyState`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct FamilyState {
    pub family_id: FamilyId,
    pub consensus_version: i32,
    pub network_id: String,
    pub predecessor_snapshot_id: String,
    pub boundary_block_hash: String,
    pub boundary_block_height: i64,
    pub payout_variant: String,
}

/// Remote member. A member is in the family only when its fields match and its claimed id recomputes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FamilyMember {
    pub family_id: FamilyId,
    pub consensus_version: i32,
    pub network_id: String,
    pub predecessor_snapshot_id: String,
    pub boundary_block_hash: String,
    pub boundary_block_height: i64,
    pub payout_variant: String,
}

/// `MatchesFamily` on the pin. Identity fields compare trim-and-lowercase, then ordinal.
pub fn matches_family(family: &FamilyState, member: &FamilyMember) -> bool {
    family.consensus_version == member.consensus_version
        && family.boundary_block_height == member.boundary_block_height
        && eq_identity(&family.network_id, &member.network_id)
        && eq_identity(
            &family.predecessor_snapshot_id,
            &member.predecessor_snapshot_id,
        )
        && eq_identity(&family.boundary_block_hash, &member.boundary_block_hash)
        && eq_identity(&family.payout_variant, &member.payout_variant)
        && eq_identity(&family.family_id.0, &member.family_id.0)
        && eq_identity(
            &member.family_id.0,
            &compute_family_id(
                member.consensus_version,
                &member.network_id,
                &member.predecessor_snapshot_id,
                &member.boundary_block_hash,
                member.boundary_block_height,
                &member.payout_variant,
            )
            .0,
        )
}

fn eq_identity(left: &str, right: &str) -> bool {
    normalize_identity(left) == normalize_identity(right)
}

fn normalize_identity(value: &str) -> String {
    value.trim().to_lowercase()
}

/// Incompatible versions fail closed. A later GridPool version needs a new constant.
pub fn families_may_union(local: &FamilyId, remote: &FamilyId) -> bool {
    local == remote
}

/// `BuildPayoutVariantNoLock` on the pin. The caller supplies the slot counts.
pub fn payout_variant(
    consensus_version: i32,
    support_enabled: bool,
    shared_slots: i32,
    snapshot_slots: i32,
    reserve_limit: i32,
) -> String {
    let base = if support_enabled {
        "gridlabs-support-v1"
    } else {
        "fee-free"
    };
    if consensus_version >= CONSENSUS_VERSION {
        format!("{base}:shared={shared_slots}:snapshot={snapshot_slots}:reserve={reserve_limit}")
    } else {
        base.to_string()
    }
}

pub fn shared_winner_slot_count(winners_list_size: i32, support_enabled: bool) -> i32 {
    let support_slots = if support_enabled { 1 } else { 0 };
    (winners_list_size - support_slots).max(1)
}

pub fn snapshot_proof_slot_count(winners_list_size: i32) -> i32 {
    winners_list_size
}

pub fn work_set_reserve_limit(snapshot_slots: i32, multiplier: i32) -> i32 {
    let mult = multiplier.max(1);
    snapshot_slots.max(snapshot_slots.saturating_mul(mult))
}

pub fn total_payout_slot_count(winners_list_size: i32) -> i32 {
    winners_list_size + 1
}

/// Variant string produced by the pin's default `PoolConfig`, not a guessed dialect.
pub fn reference_payout_variant() -> String {
    let shared =
        shared_winner_slot_count(REFERENCE_WINNERS_LIST_SIZE, REFERENCE_SUPPORT_FEE_ENABLED);
    let snapshot = snapshot_proof_slot_count(REFERENCE_WINNERS_LIST_SIZE);
    let reserve = work_set_reserve_limit(snapshot, REFERENCE_RESERVE_MULTIPLIER);
    payout_variant(
        CONSENSUS_VERSION,
        REFERENCE_SUPPORT_FEE_ENABLED,
        shared,
        snapshot,
        reserve,
    )
}

fn write_norm_string(buf: &mut Vec<u8>, value: &str) {
    let normalized = normalize_identity(value);
    let bytes = normalized.as_bytes();
    let len = i32::try_from(bytes.len()).unwrap_or(i32::MAX);
    buf.extend_from_slice(&len.to_be_bytes());
    buf.extend_from_slice(bytes);
}

fn hex_lower(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push(HEX[(byte >> 4) as usize] as char);
        out.push(HEX[(byte & 0x0f) as usize] as char);
    }
    out
}
