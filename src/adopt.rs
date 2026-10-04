//! `ShouldReconcileSnapshotFamily` on the pinned commit.
//!
//! Empty bootstrap is off unless the caller sets it. The regtest-only helper is
//! not the consensus default.

#[derive(Clone, Debug, PartialEq)]
pub struct LocalWinner {
    pub difficulty: f64,
    pub value_sats: u64,
}

/// Zero winners, or one winner with non-positive difficulty whose value equals
/// the one-slot shared payout (`GetSharedPayoutValueSatsNoLock`, which ignores
/// its count argument and divides by the total slot count).
pub fn local_winners_are_empty(winners: &[LocalWinner], slot_value_sats: u64) -> bool {
    winners.is_empty()
        || (winners.len() == 1
            && winners[0].difficulty <= 0.0
            && winners[0].value_sats == slot_value_sats)
}

pub fn should_reconcile_snapshot_family(
    has_local_active_family: bool,
    winners: &[LocalWinner],
    slot_value_sats: u64,
    allow_empty_snapshot_bootstrap: bool,
) -> bool {
    has_local_active_family
        && (!local_winners_are_empty(winners, slot_value_sats) || allow_empty_snapshot_bootstrap)
}
