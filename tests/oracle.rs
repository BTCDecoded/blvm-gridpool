//! Regression oracle for boot-protocol `develop` `be0f0b16`
//! `SnapshotReconciliationTests`, plus the adopt gate and handbook hazards
//! that file does not cover.

use blvm_gridpool::{
    activates_boundary, admit_direct_previous_parent, assign_slots, bind_context_proof,
    canon_after_local_block, compute_family_id, families_may_union, job_after_payment,
    local_winners_are_empty, matches_family, note_reorg, pay_published_list, payout_variant,
    peer_header_leaves_boundary_and_version, published_job, reconcile, reference_payout_variant,
    shared_slot_value_sats, shared_winner_slot_count, should_reconcile_snapshot_family,
    snapshot_proof_slot_count, total_payout_slot_count, try_retain_member_id,
    work_set_reserve_limit, ActivationSource, Canon, FamilyId, FamilyMember, FamilyState,
    LocalWinner, OpenFamily, ReorgError, ShareProof, CONSENSUS_VERSION, DEVELOP_PIN,
    MAX_RETAINED_MEMBER_SNAPSHOT_IDS, REFERENCE_RESERVE_MULTIPLIER, REFERENCE_SUPPORT_FEE_ENABLED,
    REFERENCE_WINNERS_LIST_SIZE,
};

const VECTOR_ID: &str = "6127d0d6ee591f1358584ef9d48fe86a9f1b5f672e011a8b926633c5db9f2a86";
const FEE_FREE: &str = "fee-free:shared=2:snapshot=2:reserve=6";
const SUPPORT_ON: &str = "gridlabs-support-v1:shared=1:snapshot=2:reserve=6";

fn boundary() -> String {
    format!("{}ab", "0".repeat(62))
}

fn vector_id() -> FamilyId {
    compute_family_id(22, "testnet", "snapshot-a", &boundary(), 100, FEE_FREE)
}

fn proof(id: &str, difficulty: f64) -> ShareProof {
    ShareProof {
        share_id: id.to_string(),
        difficulty,
    }
}

fn ids(proofs: &[ShareProof]) -> Vec<&str> {
    proofs.iter().map(|proof| proof.share_id.as_str()).collect()
}

#[test]
fn develop_pin_is_the_confirmed_tip() {
    assert_eq!(DEVELOP_PIN, "be0f0b165181616b1b757c27219ed07a9f017e8c");
    assert_eq!(CONSENSUS_VERSION, 22);
}

#[test]
fn family_id_matches_the_develop_vector() {
    assert_eq!(vector_id().0, VECTOR_ID);
    let family = family_state("testnet", "snapshot-a", &boundary(), 100, FEE_FREE);
    let member = family_member(
        " TestNet ",
        " Snapshot-A ",
        &boundary().to_uppercase(),
        100,
        FEE_FREE,
    );
    assert!(matches_family(&family, &member));
}

fn family_state(
    network: &str,
    predecessor: &str,
    boundary_hash: &str,
    height: i64,
    variant: &str,
) -> FamilyState {
    let id = compute_family_id(22, network, predecessor, boundary_hash, height, variant);
    FamilyState {
        family_id: id,
        consensus_version: 22,
        network_id: network.to_string(),
        predecessor_snapshot_id: predecessor.to_string(),
        boundary_block_hash: boundary_hash.to_string(),
        boundary_block_height: height,
        payout_variant: variant.to_string(),
    }
}

fn family_member(
    network: &str,
    predecessor: &str,
    boundary_hash: &str,
    height: i64,
    variant: &str,
) -> FamilyMember {
    let id = compute_family_id(22, network, predecessor, boundary_hash, height, variant);
    FamilyMember {
        family_id: id,
        consensus_version: 22,
        network_id: network.to_string(),
        predecessor_snapshot_id: predecessor.to_string(),
        boundary_block_hash: boundary_hash.to_string(),
        boundary_block_height: height,
        payout_variant: variant.to_string(),
    }
}

#[test]
fn each_isolated_family_input_changes_the_id() {
    let base = vector_id();
    let changed = [
        compute_family_id(21, "testnet", "snapshot-a", &boundary(), 100, FEE_FREE),
        compute_family_id(22, "mainnet", "snapshot-a", &boundary(), 100, FEE_FREE),
        compute_family_id(22, "testnet", "snapshot-b", &boundary(), 100, FEE_FREE),
        compute_family_id(
            22,
            "testnet",
            "snapshot-a",
            &format!("{}ac", "0".repeat(62)),
            100,
            FEE_FREE,
        ),
        compute_family_id(22, "testnet", "snapshot-a", &boundary(), 101, FEE_FREE),
        compute_family_id(22, "testnet", "snapshot-a", &boundary(), 100, SUPPORT_ON),
    ];
    for (index, family) in changed.iter().enumerate() {
        assert_ne!(family, &base);
        for other in changed.iter().skip(index + 1) {
            assert_ne!(family, other);
        }
    }
}

#[test]
fn support_lives_inside_the_variant_string() {
    let fee_free = compute_family_id(22, "testnet", "snapshot-a", &boundary(), 100, FEE_FREE);
    let support = compute_family_id(22, "testnet", "snapshot-a", &boundary(), 100, SUPPORT_ON);
    assert_ne!(fee_free, support);
    assert_eq!(payout_variant(22, false, 2, 2, 6), FEE_FREE);
    assert_eq!(payout_variant(22, true, 1, 2, 6), SUPPORT_ON);
}

#[test]
fn reference_variant_comes_from_pool_config_defaults() {
    let shared =
        shared_winner_slot_count(REFERENCE_WINNERS_LIST_SIZE, REFERENCE_SUPPORT_FEE_ENABLED);
    let snapshot = snapshot_proof_slot_count(REFERENCE_WINNERS_LIST_SIZE);
    let reserve = work_set_reserve_limit(snapshot, REFERENCE_RESERVE_MULTIPLIER);
    assert_eq!(shared, 298);
    assert_eq!(snapshot, 299);
    assert_eq!(reserve, 897);
    assert_eq!(total_payout_slot_count(REFERENCE_WINNERS_LIST_SIZE), 300);
    assert_eq!(
        reference_payout_variant(),
        "gridlabs-support-v1:shared=298:snapshot=299:reserve=897"
    );
    assert_ne!(reference_payout_variant(), FEE_FREE);
    assert_ne!(reference_payout_variant(), SUPPORT_ON);
}

#[test]
fn family_match_rejects_a_different_field_or_a_forged_id() {
    let family = family_state(
        "network-a",
        "predecessor-a",
        &padded("aa"),
        100,
        "variant-a",
    );
    let same = family_member(
        "network-a",
        "predecessor-a",
        &padded("aa"),
        100,
        "variant-a",
    );
    assert!(matches_family(&family, &same));
    let isolated = [
        family_member(
            "network-b",
            "predecessor-a",
            &padded("aa"),
            100,
            "variant-a",
        ),
        family_member(
            "network-a",
            "predecessor-b",
            &padded("aa"),
            100,
            "variant-a",
        ),
        family_member(
            "network-a",
            "predecessor-a",
            &padded("bb"),
            100,
            "variant-a",
        ),
        family_member(
            "network-a",
            "predecessor-a",
            &padded("aa"),
            101,
            "variant-a",
        ),
        family_member(
            "network-a",
            "predecessor-a",
            &padded("aa"),
            100,
            "variant-b",
        ),
    ];
    for member in &isolated {
        assert!(!matches_family(&family, member));
    }
    let mut forged = same;
    forged.family_id = FamilyId("deadbeef".to_string());
    assert!(!matches_family(&family, &forged));
}

fn padded(suffix: &str) -> String {
    format!("{}{suffix}", "0".repeat(64 - suffix.len()))
}

#[test]
fn rank_order_matches_the_published_vector() {
    let known = vec![proof("a", 10.0), proof("b", 20.0)];
    let incoming = vec![proof("c", 20.0), proof("d", 5.0)];
    let ranked = reconcile(&known, &incoming, &[], 4);
    assert_eq!(ids(&ranked), ["b", "c", "a", "d"]);
}

#[test]
fn fractional_difficulty_stays_in_the_sort() {
    let ranked = reconcile(&[proof("low", 10.4), proof("high", 10.5)], &[], &[], 4);
    assert_eq!(ids(&ranked), ["high", "low"]);
}

#[test]
fn union_is_commutative_idempotent_and_associative() {
    let a = proof("a", 10.0);
    let b = proof("b", 20.0);
    let c = proof("c", 20.0);
    let d = proof("d", 5.0);
    let ab = reconcile(&[a.clone()], &[b.clone()], &[], 4);
    let ba = reconcile(&[b.clone()], &[a.clone()], &[], 4);
    assert_eq!(ids(&ab), ids(&ba));
    assert_eq!(ids(&reconcile(&ab, &ab, &[], 4)), ids(&ab));
    let left = reconcile(
        &reconcile(&[a.clone()], &[b.clone()], &[], 4),
        &[c.clone(), d.clone()],
        &[],
        4,
    );
    let right = reconcile(&[a], &reconcile(&[b], &[c, d], &[], 4), &[], 4);
    assert_eq!(ids(&left), ids(&right));
    assert_eq!(ids(&left), ["b", "c", "a", "d"]);
}

#[test]
fn omission_cannot_delete_a_known_proof() {
    let known = vec![proof("a", 30.0), proof("b", 20.0), proof("c", 10.0)];
    let omitted = reconcile(&known, &[proof("a", 30.0)], &[], 3);
    assert_eq!(ids(&omitted), ["a", "b", "c"]);
}

#[test]
fn paying_an_id_twice_removes_it_once() {
    let known = vec![proof("a", 30.0), proof("b", 20.0), proof("c", 10.0)];
    let paid = reconcile(&known, &known, &["b".to_string(), "b".to_string()], 3);
    assert_eq!(ids(&paid), ["a", "c"]);
    let folded = reconcile(&known, &[], &["B".to_string()], 3);
    assert_eq!(ids(&folded), ["a", "c"]);
    let untrimmed = reconcile(&[proof("b", 1.0)], &[], &[" b ".to_string()], 4);
    assert_eq!(ids(&untrimmed), ["b"]);
}

#[test]
fn duplicate_ids_keep_the_higher_difficulty_and_limit_zero_keeps_nothing() {
    let tied = reconcile(&[proof("b", 20.0)], &[proof("B", 20.0)], &[], 4);
    assert_eq!(tied[0].share_id, "B");
    let ranked = reconcile(&[proof("a", 5.0)], &[proof("A", 9.0)], &[], 4);
    assert_eq!(ranked.len(), 1);
    assert_eq!(ranked[0].share_id, "A");
    assert_eq!(ranked[0].difficulty, 9.0);
    assert!(reconcile(&[proof("a", 5.0)], &[], &[], 0).is_empty());
    assert!(reconcile(&[proof("  ", 5.0)], &[], &[], 4).is_empty());
}

#[test]
fn member_snapshot_cap_retains_sixty_four() {
    let mut retained = Vec::new();
    let mut dropped = 0;
    for index in 0..10_000 {
        let id = format!("snapshot-{index}");
        if !try_retain_member_id(&mut retained, &id) {
            dropped += 1;
        }
    }
    assert_eq!(retained.len(), MAX_RETAINED_MEMBER_SNAPSHOT_IDS);
    assert_eq!(dropped, 10_000 - MAX_RETAINED_MEMBER_SNAPSHOT_IDS);
    assert!(try_retain_member_id(&mut retained, "snapshot-0"));
    assert!(try_retain_member_id(&mut retained, "SNAPSHOT-0"));
    assert_eq!(retained.len(), MAX_RETAINED_MEMBER_SNAPSHOT_IDS);
    assert!(try_retain_member_id(&mut retained, "   "));
    assert!(!try_retain_member_id(&mut retained, "snapshot-new"));
    assert_eq!(retained[0], "snapshot-0");
    let sibling = reconcile(&[proof("sibling", 10.0)], &[], &[], 897);
    assert_eq!(ids(&sibling), ["sibling"]);
}

#[test]
fn empty_local_state_does_not_adopt_a_remote_sibling() {
    let slot = shared_slot_value_sats(3_125_000_000, 300);
    let empty = Vec::new();
    let genesis = vec![LocalWinner {
        difficulty: 0.0,
        value_sats: slot,
    }];
    assert!(local_winners_are_empty(&empty, slot));
    assert!(local_winners_are_empty(&genesis, slot));
    assert!(!should_reconcile_snapshot_family(
        false, &genesis, slot, false
    ));
    assert!(!should_reconcile_snapshot_family(true, &empty, slot, false));
    assert!(!should_reconcile_snapshot_family(
        true, &genesis, slot, false
    ));
    assert!(should_reconcile_snapshot_family(true, &genesis, slot, true));
    let real = vec![LocalWinner {
        difficulty: 20.0,
        value_sats: slot,
    }];
    assert!(should_reconcile_snapshot_family(true, &real, slot, false));
}

#[test]
fn incompatible_versions_and_predecessors_do_not_union() {
    let local = vector_id();
    let other_version = compute_family_id(23, "testnet", "snapshot-a", &boundary(), 100, FEE_FREE);
    let other_predecessor =
        compute_family_id(22, "testnet", "snapshot-b", &boundary(), 100, FEE_FREE);
    assert!(!families_may_union(&local, &other_version));
    assert!(!families_may_union(&local, &other_predecessor));
    assert!(families_may_union(&local, &local));
}

#[test]
fn direct_ingress_after_the_boundary_is_rejected() {
    assert!(admit_direct_previous_parent(false));
    assert!(!admit_direct_previous_parent(true));
    assert!(activates_boundary(ActivationSource::LocalValidatedBlock));
    assert!(!activates_boundary(ActivationSource::PeerHeader));
}

#[test]
fn context_proof_does_not_change_the_open_family() {
    let open = OpenFamily {
        family_id: VECTOR_ID.to_string(),
        reserve: vec![proof("a", 10.0)],
    };
    let (after, binding) = bind_context_proof(&open, "older-snapshot", &proof("ctx", 4.0));
    assert_eq!(after, open);
    assert_eq!(binding.committed_snapshot_id, "older-snapshot");
    assert_eq!(binding.proof.share_id, "ctx");
}

#[test]
fn peer_header_does_not_change_boundary_or_version() {
    assert_eq!(
        peer_header_leaves_boundary_and_version(false, CONSENSUS_VERSION),
        (false, CONSENSUS_VERSION)
    );
}

#[test]
fn a_new_block_keeps_the_unpaid_reserve() {
    let reserve = vec![proof("a", 10.0), proof("b", 20.0)];
    let canon = canon_after_local_block(&reserve, "block-2");
    assert_eq!(ids(&canon.proofs), ["a", "b"]);
    assert_eq!(canon.header_parent, "block-2");
}

#[test]
fn found_block_pays_the_published_list_and_the_next_job_is_the_remainder() {
    let published = vec![proof("a", 10.0)];
    let canon = reconcile(&published, &[proof("sibling", 30.0)], &[], 4);
    let remaining = pay_published_list(&canon, &published, 4);
    assert_eq!(ids(&remaining), ["sibling"]);
    let next = job_after_payment(&remaining);
    assert_eq!(ids(&next), ["sibling"]);
    assert_ne!(ids(&next), ids(&published));
}

#[test]
fn support_off_does_not_union_with_the_reference_variant() {
    let reference = compute_family_id(
        22,
        "testnet",
        "snapshot-a",
        &boundary(),
        100,
        &reference_payout_variant(),
    );
    let fee_free = compute_family_id(22, "testnet", "snapshot-a", &boundary(), 100, FEE_FREE);
    assert!(!families_may_union(&reference, &fee_free));
}

#[test]
fn peer_count_does_not_choose_the_sibling() {
    let many_weak = (0..10)
        .map(|index| proof(&format!("w{index}"), 1.0))
        .collect::<Vec<_>>();
    let one_strong = vec![proof("strong", 100.0)];
    let ranked = reconcile(&many_weak, &one_strong, &[], 1);
    assert_eq!(ids(&ranked), ["strong"]);
}

#[test]
fn paid_once_removes_only_the_mined_ids() {
    let reserve = vec![
        proof("a", 10.0),
        proof("b", 20.0),
        proof("sibling", 15.0),
        proof("keep", 8.0),
    ];
    let mined = vec!["b".to_string(), "sibling".to_string()];
    let remaining = reconcile(&reserve, &[], &mined, 897);
    assert_eq!(ids(&remaining), ["a", "keep"]);
    let next = job_after_payment(&remaining);
    assert_eq!(ids(&next), ["a", "keep"]);
    assert!(!next
        .iter()
        .any(|proof| mined.iter().any(|id| id == &proof.share_id)));
}

#[test]
fn two_block_reorg_is_unsupported() {
    assert_eq!(note_reorg(2), Err(ReorgError::UnsupportedTwoBlock));
    assert_eq!(note_reorg(1), Ok(()));
}

#[test]
fn payout_slots_keep_the_integer_remainder_on_slot_zero() {
    let slots = assign_slots(3_001, 50, 300, true, 2);
    assert_eq!(slots.slot_value_sats, 10);
    assert_eq!(slots.slot0_sats, 10 + 50 + 1);
    assert_eq!(slots.support_sats, Some(10));
    assert_eq!(slots.shared_sats, vec![10, 10]);
    let support_off = assign_slots(3_000, 0, 300, false, 1);
    assert_eq!(support_off.support_sats, None);
}

#[test]
fn a_sibling_admitted_after_publish_still_changes_canon() {
    let published = vec![proof("a", 10.0)];
    let mut canon = Canon {
        proofs: published.clone(),
        header_parent: "block-1".to_string(),
    };
    canon.proofs = reconcile(&canon.proofs, &[proof("sibling", 30.0)], &[], 4);
    canon.header_parent = "block-2".to_string();
    let held = published_job(false, &published, &canon);
    assert_eq!(ids(&held.proofs), ["a"]);
    assert_eq!(held.header_parent, "block-2");
    assert_eq!(ids(&canon.proofs), ["sibling", "a"]);
    let closed = published_job(true, &published, &canon);
    assert_eq!(ids(&closed.proofs), ["sibling", "a"]);
}

#[test]
fn two_published_lists_as_predecessors_do_not_union() {
    let left = compute_family_id(22, "testnet", "job-list-a", &boundary(), 100, FEE_FREE);
    let right = compute_family_id(22, "testnet", "job-list-b", &boundary(), 100, FEE_FREE);
    assert_ne!(left, right);
    assert!(!families_may_union(&left, &right));
}
