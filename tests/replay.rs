//! Captured bundles from boot-protocol `be0f0b16`, then the HTTP poll.

use std::sync::{Arc, Mutex};

use blvm_gridpool::{
    can_sync_state, decks_of, expected_winners_from_coinbase, http_get, import_candidate,
    import_locked_sibling, poll_peer, AdoptOutcome, BundleProof, CandidateImport, FamilyId,
    FamilyMember, FamilyState, LocalSync, LockedSiblingImport, NetworkSummary, PayoutMath,
    PeerBooks, PeerServer, PollOutcome, PollSnapshot, ShareProof, StateBundle, VersionFields,
    WinnerRow, CONSENSUS_VERSION, HTTP_API_VERSION, STATE_BUNDLE_SCHEMA_VERSION,
};

const RECONCILED: &[&str] = &["537c2cdb8dffd914b099b7a83423cafea4ede16152d42d6600272b89a5537c5f"];
const PAID: &[&str] = &["5ce703e346ed8dc65516d8d624c6184e199833f80a78006163b140a121cd8c67"];
const CANDIDATE_IDS: &[&str] = &[
    "4a2eecc90729efcb450f963f25f1cd438a15998af3904e1d4155dd2c7185372c",
    "537c2cdb8dffd914b099b7a83423cafea4ede16152d42d6600272b89a5537c5f",
];

fn fixture(name: &str) -> String {
    let path = format!("{}/tests/fixtures/{name}", env!("CARGO_MANIFEST_DIR"));
    std::fs::read_to_string(&path).unwrap_or_else(|error| panic!("{path}: {error}"))
}

fn bundle(name: &str) -> StateBundle {
    serde_json::from_str(&fixture(name)).unwrap_or_else(|error| panic!("{name}: {error}"))
}

fn version_of(bundle: &StateBundle) -> VersionFields {
    VersionFields {
        consensus_version: bundle.consensus_version,
        state_bundle_schema_version: bundle.state_bundle_schema_version,
        http_api_version: bundle.http_api_version,
    }
}

fn rows(bundle: &StateBundle) -> Vec<WinnerRow> {
    bundle
        .winners_list
        .iter()
        .map(|winner| WinnerRow {
            value_sats: winner.value,
            address: winner.address.clone(),
            difficulty: winner.difficulty,
        })
        .collect()
}

fn shares(proofs: &[blvm_gridpool::DeckProof]) -> Vec<ShareProof> {
    proofs
        .iter()
        .map(|proof| ShareProof {
            share_id: proof.share_id.clone(),
            difficulty: proof.difficulty,
        })
        .collect()
}

fn ids(proofs: &[ShareProof]) -> Vec<&str> {
    proofs.iter().map(|proof| proof.share_id.as_str()).collect()
}

fn deck_ids(proofs: &[blvm_gridpool::DeckProof]) -> Vec<&str> {
    proofs.iter().map(|proof| proof.share_id.as_str()).collect()
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct LockedLocal {
    tip: String,
    height: i64,
    paid_ids: Vec<String>,
    proofs: Vec<BundleProof>,
    family: FamilyJson,
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct FamilyJson {
    family_id: String,
    consensus_version: i32,
    network_id: String,
    predecessor_snapshot_id: String,
    boundary_block_hash: String,
    boundary_block_height: i64,
    payout_variant: String,
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct CandidateLocal {
    tip: String,
    proofs: Vec<BundleProof>,
}

fn family_from(json: &FamilyJson) -> (FamilyState, FamilyMember) {
    let id = FamilyId(json.family_id.clone());
    let state = FamilyState {
        family_id: id.clone(),
        consensus_version: json.consensus_version,
        network_id: json.network_id.clone(),
        predecessor_snapshot_id: json.predecessor_snapshot_id.clone(),
        boundary_block_hash: json.boundary_block_hash.clone(),
        boundary_block_height: json.boundary_block_height,
        payout_variant: json.payout_variant.clone(),
    };
    let member = FamilyMember {
        family_id: id,
        consensus_version: json.consensus_version,
        network_id: json.network_id.clone(),
        predecessor_snapshot_id: json.predecessor_snapshot_id.clone(),
        boundary_block_hash: json.boundary_block_hash.clone(),
        boundary_block_height: json.boundary_block_height,
        payout_variant: json.payout_variant.clone(),
    };
    (state, member)
}

fn locked_math() -> PayoutMath {
    PayoutMath {
        subsidy_sats: 312_500_000,
        total_slots: 2,
        shared_slots: 1,
        support_address: None,
        network: "mainnet".into(),
    }
}

fn adopt(
    bundle: &StateBundle,
    local: &LockedLocal,
    family: &FamilyState,
    member: &FamilyMember,
    claimed_snapshot: &str,
    claimed_winners: &[WinnerRow],
) -> Result<AdoptOutcome, blvm_gridpool::ImportError> {
    let boundary = bundle.boundary_decks();
    let local_decks = decks_of(&local.proofs);
    let winners = expected_winners_from_coinbase(&boundary[0].coinbase_hex, "mainnet");
    let math = locked_math();
    let local_shares = shares(&local_decks);
    import_locked_sibling(&LockedSiblingImport {
        family,
        member,
        local_reconciled: &local_shares,
        boundary: &boundary,
        paid_ids: &local.paid_ids,
        on_deck: &local_shares,
        reserve_limit: 3,
        math: &math,
        network_id: &bundle.network_id,
        payout_variant: &bundle.payout_variant,
        claimed_snapshot_id: claimed_snapshot,
        active_snapshot_id: &bundle.active_snapshot_id,
        local_tip: &local.tip,
        local_tip_height: local.height,
        local_support_enabled: false,
        remote_support_enabled: bundle.support_fee_enabled,
        verifier_winners: &winners,
        claimed_winners,
        parents: &bundle.valid_parent_block_hashes,
    })
}

#[test]
fn locked_sibling_matches_the_captured_order_and_paid_ids() {
    let bundle = bundle("locked-bundle.json");
    let local: LockedLocal = serde_json::from_str(&fixture("locked-local.json")).unwrap();
    let (family, member) = family_from(&local.family);
    let member_id = bundle
        .snapshot_family_member
        .as_ref()
        .unwrap()
        .snapshot_id
        .clone();
    let adopted = adopt(
        &bundle,
        &local,
        &family,
        &member,
        &member_id,
        &rows(&bundle),
    )
    .expect("adopt");
    let AdoptOutcome::Reconciled {
        reconciled,
        on_deck,
    } = adopted
    else {
        panic!("first adopt reconciles");
    };
    assert_eq!(ids(&reconciled), RECONCILED);
    assert_eq!(ids(&on_deck), RECONCILED);
    assert_eq!(local.paid_ids, PAID);
    assert!(!PAID
        .iter()
        .any(|paid| reconciled.iter().any(|proof| proof.share_id == *paid)));

    let again_math = locked_math();
    let again_boundary = bundle.boundary_decks();
    let again_winners = expected_winners_from_coinbase(&again_boundary[0].coinbase_hex, "mainnet");
    let again_rows = rows(&bundle);
    let again = import_locked_sibling(&LockedSiblingImport {
        family: &family,
        member: &member,
        local_reconciled: &reconciled,
        boundary: &again_boundary,
        paid_ids: &local.paid_ids,
        on_deck: &on_deck,
        reserve_limit: 3,
        math: &again_math,
        network_id: &bundle.network_id,
        payout_variant: &bundle.payout_variant,
        claimed_snapshot_id: &member_id,
        active_snapshot_id: &bundle.active_snapshot_id,
        local_tip: &local.tip,
        local_tip_height: local.height,
        local_support_enabled: false,
        remote_support_enabled: bundle.support_fee_enabled,
        verifier_winners: &again_winners,
        claimed_winners: &again_rows,
        parents: &bundle.valid_parent_block_hashes,
    })
    .expect("second adopt");
    match again {
        AdoptOutcome::Unchanged => {}
        AdoptOutcome::Reconciled {
            reconciled: next, ..
        } => assert_eq!(ids(&next), RECONCILED),
    }

    let forged = adopt(
        &bundle,
        &local,
        &family,
        &member,
        "deadbeef",
        &rows(&bundle),
    );
    assert!(matches!(
        forged,
        Err(blvm_gridpool::ImportError::StateIdMismatch)
    ));
    let mut mismatched = rows(&bundle);
    mismatched[0].value_sats = 1;
    let winners = adopt(&bundle, &local, &family, &member, &member_id, &mismatched);
    assert!(matches!(
        winners,
        Err(blvm_gridpool::ImportError::WinnersMismatch)
    ));
    let mut other = member.clone();
    other.predecessor_snapshot_id = "other-family".into();
    let family_miss = adopt(&bundle, &local, &family, &other, &member_id, &rows(&bundle));
    assert!(matches!(
        family_miss,
        Err(blvm_gridpool::ImportError::FamilyMismatch)
    ));
}

#[test]
fn candidate_merge_matches_the_captured_ids_without_reconcile() {
    let bundle = bundle("candidate-bundle.json");
    let local: CandidateLocal = serde_json::from_str(&fixture("candidate-local.json")).unwrap();
    let local_decks = decks_of(&local.proofs);
    let remote = bundle.candidate_decks();
    let math = PayoutMath {
        subsidy_sats: 312_500_000,
        total_slots: 5,
        shared_slots: 4,
        support_address: None,
        network: "mainnet".into(),
    };
    let winners = expected_winners_from_coinbase(&remote[0].coinbase_hex, "mainnet");
    let local_version = version_of(&bundle);
    let remote_version = version_of(&bundle);
    let claimed_rows = rows(&bundle);
    let local_parents = [local.tip.clone()];
    let remote_parents = remote
        .iter()
        .map(|proof| proof.prev_block_hash.clone())
        .collect::<Vec<_>>();
    let merged = import_candidate(&CandidateImport {
        local_version: &local_version,
        remote_version: &remote_version,
        local_network_id: &bundle.network_id,
        remote_network_id: &bundle.network_id,
        current_state_id: "seed-current",
        claimed_state_id: &bundle.state_id,
        payout_variant: &bundle.payout_variant,
        local: &local_decks,
        remote: &remote,
        current_tip: Some(&local.tip),
        limit: 12,
        verifier_winners: &winners,
        claimed_winners: &claimed_rows,
        math: &math,
        local_parents: &local_parents,
        remote_parents: &remote_parents,
    })
    .expect("candidate");
    assert_eq!(deck_ids(&merged), CANDIDATE_IDS);
    let kept = merged
        .iter()
        .find(|proof| proof.share_id == CANDIDATE_IDS[0])
        .unwrap();
    assert_eq!(kept.timestamp_millis, 1000);
}

#[test]
fn http_summary_syncs_and_peer_addresses_are_not_served() {
    let mut summary = NetworkSummary::consensus_22("testnet", "http://127.0.0.1");
    summary.current_state_id = "locked-1".into();
    summary.candidate_state_id = "candidate-1".into();
    summary.winners_count = 2;
    let books = Arc::new(Mutex::new(PeerBooks {
        summary: summary.clone(),
        bundles: [(
            "locked-1".into(),
            StateBundle {
                state_id: "locked-1".into(),
                ..StateBundle::default()
            },
        )]
        .into_iter()
        .collect(),
    }));
    let server = PeerServer::spawn(Arc::clone(&books)).expect("listen");
    let (status, body) = http_get(&server.address, "/api/network/summary").expect("summary");
    assert_eq!(status, 200);
    let remote: NetworkSummary = serde_json::from_str(&body).expect("json");
    assert!(can_sync_state(
        &summary.version_fields(),
        &remote.version_fields(),
        &summary.network_id,
        &remote.network_id
    ));
    assert!(remote.peers.is_empty());
    let (missing, _) =
        http_get(&server.address, "/api/network/peer-addresses?limit=10").expect("gossip");
    assert_eq!(missing, 404);
    let (bundle_status, bundle_body) =
        http_get(&server.address, "/api/network/state/locked-1").expect("state");
    assert_eq!(bundle_status, 200);
    assert!(bundle_body.contains("locked-1"));
    let _ = (
        CONSENSUS_VERSION,
        HTTP_API_VERSION,
        STATE_BUNDLE_SCHEMA_VERSION,
    );
}

fn local_sync(current_id: &str, candidate_id: &str) -> LocalSync {
    LocalSync {
        network_id: "testnet".into(),
        version: VersionFields {
            consensus_version: CONSENSUS_VERSION,
            state_bundle_schema_version: STATE_BUNDLE_SCHEMA_VERSION,
            http_api_version: HTTP_API_VERSION,
        },
        poll: PollSnapshot {
            winners_count: 2,
            on_deck_count: 1,
            current_round_number: 4,
            current_state_id: current_id.into(),
            current_state_proof_count: 1,
            current_state_total_difficulty: 20.0,
        },
        candidate_state_id: candidate_id.into(),
        winners: Vec::new(),
        slot_value_sats: 156_250_000,
        tip: String::new(),
        tip_height: 0,
        parents: Vec::new(),
        reconciled: Vec::new(),
        on_deck_shares: Vec::new(),
        on_deck: vec![blvm_gridpool::DeckProof {
            share_id: "local-deck".into(),
            difficulty: 1.0,
            script_pub_key_hex: String::new(),
            prev_block_hash: String::new(),
            miner_address: String::new(),
            header_hex: String::new(),
            coinbase_hex: String::new(),
            merkle_path: Vec::new(),
            timestamp_millis: 1,
        }],
        family: None,
        paid_ids: Vec::new(),
        math: PayoutMath {
            subsidy_sats: 312_500_000,
            total_slots: 2,
            shared_slots: 1,
            support_address: None,
            network: "mainnet".into(),
        },
        support_enabled: false,
        reserve_limit: 3,
    }
}

#[test]
fn incompatible_summary_and_a_different_current_id_import_nothing() {
    let mut remote = NetworkSummary::consensus_22("testnet", "http://127.0.0.1");
    remote.consensus_version = 21;
    remote.current_state_id = "other-current".into();
    remote.candidate_state_id = "other-candidate".into();
    let books = Arc::new(Mutex::new(PeerBooks {
        summary: remote,
        bundles: std::collections::HashMap::new(),
    }));
    let server = PeerServer::spawn(Arc::clone(&books)).expect("listen");
    let local = local_sync("seed-current", "seed-candidate");
    let outcome = poll_peer(&server.address, &local).expect("poll");
    assert!(outcome.stopped);
    assert_eq!(outcome, unchanged(&local));

    books.lock().unwrap().summary.consensus_version = CONSENSUS_VERSION;
    books.lock().unwrap().summary.current_round_number = 1;
    books.lock().unwrap().summary.current_state_total_difficulty = 1.0;
    let still = poll_peer(&server.address, &local).expect("poll");
    assert!(still.stopped);
    assert_eq!(still.current_state_id, "seed-current");
    assert_eq!(still.candidate_state_id, "seed-candidate");
    assert_eq!(deck_ids(&still.on_deck), ["local-deck"]);
}

fn unchanged(local: &LocalSync) -> PollOutcome {
    PollOutcome {
        stopped: true,
        current_state_id: local.poll.current_state_id.clone(),
        candidate_state_id: local.candidate_state_id.clone(),
        winners: local.winners.clone(),
        reconciled: local.reconciled.clone(),
        on_deck: local.on_deck.clone(),
        family: local.family.clone(),
        paid_ids: local.paid_ids.clone(),
    }
}

#[test]
fn poll_imports_the_captured_candidate_only_after_the_current_ids_match() {
    let bundle = bundle("candidate-bundle.json");
    let local_file: CandidateLocal =
        serde_json::from_str(&fixture("candidate-local.json")).unwrap();
    let mut remote = NetworkSummary::consensus_22("testnet", "http://127.0.0.1");
    remote.current_state_id = "seed-current".into();
    remote.candidate_state_id = bundle.state_id.clone();
    remote.winners_count = 3;
    remote.current_tip_block_hash = local_file.tip.clone();
    let books = Arc::new(Mutex::new(PeerBooks {
        summary: remote,
        bundles: [(bundle.state_id.clone(), bundle.clone())]
            .into_iter()
            .collect(),
    }));
    let server = PeerServer::spawn(Arc::clone(&books)).expect("listen");
    let mut local = local_sync("seed-current", "");
    local.tip = local_file.tip.clone();
    local.parents = vec![local_file.tip.clone()];
    local.on_deck = decks_of(&local_file.proofs);
    local.reserve_limit = 12;
    local.math.total_slots = 5;
    local.math.shared_slots = 4;
    local.winners =
        expected_winners_from_coinbase(&bundle.candidate_decks()[0].coinbase_hex, "mainnet")
            .into_iter()
            .map(|winner| WinnerRow {
                value_sats: winner.value_sats,
                address: winner.address,
                difficulty: 0.0,
            })
            .collect();
    let outcome = poll_peer(&server.address, &local).expect("poll");
    assert!(!outcome.stopped);
    assert_eq!(deck_ids(&outcome.on_deck), CANDIDATE_IDS);
    assert_eq!(
        outcome
            .on_deck
            .iter()
            .find(|proof| proof.share_id == CANDIDATE_IDS[0])
            .unwrap()
            .timestamp_millis,
        1000
    );
}
