//! Adopt needs the saved family. A later adopt refreshes winner addresses.

use std::sync::{Arc, Mutex};

use blvm_gridpool::{
    decks_of, expected_winners_from_coinbase, poll_peer, BundleProof, FamilyId, FamilyState,
    LocalSync, NetworkSummary, PayoutMath, PeerBooks, PeerServer, PollSnapshot, ShareProof,
    StateBundle, VersionFields, WinnerRow, CONSENSUS_VERSION, HTTP_API_VERSION,
    STATE_BUNDLE_SCHEMA_VERSION,
};

fn fixture(name: &str) -> String {
    let path = format!("{}/tests/fixtures/{name}", env!("CARGO_MANIFEST_DIR"));
    std::fs::read_to_string(&path).unwrap_or_else(|error| panic!("{path}: {error}"))
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

fn family_of(json: &FamilyJson) -> FamilyState {
    FamilyState {
        family_id: FamilyId(json.family_id.clone()),
        consensus_version: json.consensus_version,
        network_id: json.network_id.clone(),
        predecessor_snapshot_id: json.predecessor_snapshot_id.clone(),
        boundary_block_hash: json.boundary_block_hash.clone(),
        boundary_block_height: json.boundary_block_height,
        payout_variant: json.payout_variant.clone(),
    }
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

fn local_for(
    bundle: &StateBundle,
    family: Option<FamilyState>,
    winners: Vec<WinnerRow>,
) -> LocalSync {
    let local: LockedLocal = serde_json::from_str(&fixture("locked-local.json")).unwrap();
    let decks = decks_of(&local.proofs);
    LocalSync {
        network_id: bundle.network_id.clone(),
        version: VersionFields {
            consensus_version: CONSENSUS_VERSION,
            state_bundle_schema_version: STATE_BUNDLE_SCHEMA_VERSION,
            http_api_version: HTTP_API_VERSION,
        },
        poll: PollSnapshot {
            winners_count: 0,
            on_deck_count: 0,
            current_round_number: 0,
            current_state_id: "local-before".into(),
            current_state_proof_count: 1,
            current_state_total_difficulty: 1.0,
        },
        candidate_state_id: String::new(),
        winners,
        slot_value_sats: 156_250_000,
        tip: local.tip,
        tip_height: local.height,
        parents: bundle.valid_parent_block_hashes.clone(),
        reconciled: shares(&decks),
        on_deck_shares: shares(&decks),
        on_deck: decks,
        family,
        paid_ids: local.paid_ids,
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

fn old_winner() -> WinnerRow {
    WinnerRow {
        value_sats: 1,
        address: "bc1q-old".into(),
        difficulty: 2.0,
    }
}

fn serve(bundle: &StateBundle) -> PeerServer {
    let mut remote = NetworkSummary::consensus_22(&bundle.network_id, "http://127.0.0.1");
    remote.current_state_id = bundle.state_id.clone();
    remote.candidate_state_id = String::new();
    let books = Arc::new(Mutex::new(PeerBooks {
        summary: remote,
        bundles: [(bundle.state_id.clone(), bundle.clone())]
            .into_iter()
            .collect(),
    }));
    PeerServer::spawn(books).expect("listen")
}

#[test]
fn adopt_without_a_saved_family_does_not_apply() {
    let bundle: StateBundle = serde_json::from_str(&fixture("locked-bundle.json")).unwrap();
    let server = serve(&bundle);
    let local = local_for(&bundle, None, vec![old_winner()]);
    let before = local.reconciled.clone();
    let outcome = poll_peer(&server.address, &local).expect("poll");
    assert!(outcome.family.is_none());
    assert_eq!(outcome.reconciled, before);
    assert_eq!(outcome.winners[0].address, "bc1q-old");
    assert_eq!(outcome.current_state_id, "local-before");
}

#[test]
fn adopt_with_the_saved_family_refreshes_winner_addresses() {
    let bundle: StateBundle = serde_json::from_str(&fixture("locked-bundle.json")).unwrap();
    let local_file: LockedLocal = serde_json::from_str(&fixture("locked-local.json")).unwrap();
    let server = serve(&bundle);
    let winners =
        expected_winners_from_coinbase(&bundle.boundary_decks()[0].coinbase_hex, "mainnet")
            .into_iter()
            .map(|winner| WinnerRow {
                value_sats: winner.value_sats,
                address: winner.address,
                difficulty: 2.0,
            })
            .collect::<Vec<_>>();
    assert!(!winners.is_empty());
    let local = local_for(
        &bundle,
        Some(family_of(&local_file.family)),
        winners.clone(),
    );
    let outcome = poll_peer(&server.address, &local).expect("poll");
    let want: Vec<&str> = bundle
        .winners_list
        .iter()
        .map(|winner| winner.address.as_str())
        .collect();
    let got: Vec<&str> = outcome
        .winners
        .iter()
        .map(|winner| winner.address.as_str())
        .collect();
    assert_eq!(got, want);
    assert_ne!(outcome.winners[0].difficulty, 2.0);
    assert_eq!(
        outcome.winners[0].difficulty,
        bundle.winners_list[0].difficulty
    );
    assert_eq!(
        outcome
            .family
            .as_ref()
            .map(|family| family.family_id.0.as_str()),
        Some(local_file.family.family_id.as_str())
    );
    assert_eq!(outcome.paid_ids, bundle.paid_snapshot_proof_ids);
}
