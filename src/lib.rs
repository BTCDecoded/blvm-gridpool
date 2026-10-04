//! GridPool consensus library.
//!
//! Pinned to boot-protocol `develop` `be0f0b165181616b1b757c27219ed07a9f017e8c`
//! (2026-09-13). The pin is a constant, not a branch followed at build time.
//! Stratum asks this module for coinbase rows when Commons is not loaded.
//! `getblocktemplate` stays on Commons.

mod adopt;
mod bitcoin;
mod boundary;
mod family;
mod follow;
mod import;
mod job;
mod members;
mod payout;
mod peer;
mod reconcile;
mod reorg;
mod verify;

#[cfg(feature = "module")]
pub mod module;

pub use adopt::{local_winners_are_empty, should_reconcile_snapshot_family, LocalWinner};
pub use bitcoin::{address_to_script, decode_hex, double_sha256, encode_hex};
pub use boundary::{
    activates_boundary, admit_direct_previous_parent, bind_context_proof,
    peer_header_leaves_boundary_and_version, ActivationSource, ContextBinding, OpenFamily,
};
pub use family::{
    compute_family_id, families_may_union, matches_family, payout_variant,
    reference_payout_variant, shared_winner_slot_count, snapshot_proof_slot_count,
    total_payout_slot_count, work_set_reserve_limit, FamilyId, FamilyMember, FamilyState,
    CONSENSUS_VERSION, DEVELOP_PIN, FAMILY_DOMAIN, REFERENCE_RESERVE_MULTIPLIER,
    REFERENCE_SUPPORT_FEE_ENABLED, REFERENCE_WINNERS_LIST_SIZE,
};
pub use follow::{
    absorb_poll, apply_poll, apply_saved_config, load_config, load_state, peer_address,
    poll_open_hold, save_state, Follower, GridPoolConfig,
};
pub use import::{
    bootstrap_current, can_sync_state, compute_candidate_state_id, compute_state_id,
    import_candidate, import_locked_sibling, merge_candidate_reserve, parent_sets_overlap,
    payouts_from_proofs, should_fetch_remote_current_state, winners_match, AdoptOutcome, DeckProof,
    ImportError, PayoutMath, PollSnapshot, VersionFields, WinnerRow, HTTP_API_VERSION,
    PEER_TRANSPORT_VERSION, STATE_BUNDLE_SCHEMA_VERSION,
};
pub use job::{
    canon_after_local_block, job_after_payment, pay_published_list, published_job, Canon,
    PublishedJob,
};
pub use members::{try_retain_member_id, MAX_RETAINED_MEMBER_SNAPSHOT_IDS};
pub use payout::{
    assign_slots, coinbase_job, coinbase_outputs, shared_slot_value_sats, slot0_value_sats,
    CoinbaseRow, PayoutSlots,
};
pub use peer::{
    decks_of, http_get, poll_peer, BundleProof, LocalSync, NetworkSummary, PeerBooks, PeerServer,
    PollOutcome, StateBundle,
};
pub use reconcile::{reconcile, ShareProof};
pub use reorg::{note_reorg, ReorgError};
pub use verify::{
    coinbase_txid_hex, expected_winners_from_coinbase, validate_share, ExpectedWinner,
    ShareSubmission, ShareValidation,
};
