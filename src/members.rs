//! `TryRetainMemberId` on the pinned commit. The cap is not inside `reconcile`.

pub const MAX_RETAINED_MEMBER_SNAPSHOT_IDS: usize = 64;

/// Record a member snapshot id. A blank id is ignored. A repeat does not consume
/// a slot. A new id past the cap is refused and no older id is evicted.
pub fn try_retain_member_id(retained: &mut Vec<String>, snapshot_id: &str) -> bool {
    if snapshot_id.trim().is_empty() {
        return true;
    }
    if retained
        .iter()
        .any(|id| id.eq_ignore_ascii_case(snapshot_id))
    {
        return true;
    }
    if retained.len() >= MAX_RETAINED_MEMBER_SNAPSHOT_IDS {
        return false;
    }
    retained.push(snapshot_id.to_string());
    true
}
