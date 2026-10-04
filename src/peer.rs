//! HTTP summary and state-bundle exchange. Peer-address gossip is a 404.

use std::collections::HashMap;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::family::{FamilyId, FamilyMember, FamilyState, CONSENSUS_VERSION};
use crate::import::{
    bootstrap_current, can_sync_state, import_candidate, import_locked_sibling,
    should_fetch_remote_current_state, AdoptOutcome, DeckProof, PayoutMath, PollSnapshot,
    VersionFields, WinnerRow, HTTP_API_VERSION, PEER_TRANSPORT_VERSION,
    STATE_BUNDLE_SCHEMA_VERSION,
};
use crate::reconcile::ShareProof;
use crate::verify::ExpectedWinner;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct NetworkSummary {
    pub node_id: String,
    pub self_endpoint: String,
    pub network_id: String,
    pub consensus_version: i32,
    pub protocol_version: i32,
    pub state_bundle_schema_version: i32,
    pub http_api_version: i32,
    pub peer_transport_version: i32,
    pub udp_relay_version: i32,
    pub current_state_id: String,
    pub candidate_state_id: String,
    pub current_tip_block_hash: String,
    #[serde(default)]
    pub current_tip_block_height: i64,
    pub current_round_number: i32,
    pub winners_count: i32,
    pub on_deck_count: i32,
    pub current_state_proof_count: i32,
    pub current_state_total_difficulty: f64,
    pub peers: Vec<PeerStatus>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "camelCase")]
pub struct PeerStatus {
    #[serde(default)]
    pub endpoint: String,
}

impl NetworkSummary {
    pub fn consensus_22(network_id: &str, self_endpoint: &str) -> Self {
        Self {
            node_id: "blvm-gridpool".into(),
            self_endpoint: self_endpoint.into(),
            network_id: network_id.into(),
            consensus_version: CONSENSUS_VERSION,
            protocol_version: CONSENSUS_VERSION,
            state_bundle_schema_version: STATE_BUNDLE_SCHEMA_VERSION,
            http_api_version: HTTP_API_VERSION,
            peer_transport_version: PEER_TRANSPORT_VERSION,
            udp_relay_version: 5,
            current_state_id: String::new(),
            candidate_state_id: String::new(),
            current_tip_block_hash: String::new(),
            current_tip_block_height: 0,
            current_round_number: 0,
            winners_count: 0,
            on_deck_count: 0,
            current_state_proof_count: 0,
            current_state_total_difficulty: 0.0,
            peers: Vec::new(),
        }
    }

    pub fn version_fields(&self) -> VersionFields {
        VersionFields {
            consensus_version: self.consensus_version,
            state_bundle_schema_version: self.state_bundle_schema_version,
            http_api_version: self.http_api_version,
        }
    }

    pub fn poll_snapshot(&self) -> PollSnapshot {
        PollSnapshot {
            winners_count: self.winners_count,
            on_deck_count: self.on_deck_count,
            current_round_number: self.current_round_number,
            current_state_id: self.current_state_id.clone(),
            current_state_proof_count: self.current_state_proof_count,
            current_state_total_difficulty: self.current_state_total_difficulty,
        }
    }
}

#[derive(Clone)]
pub struct PeerBooks {
    pub summary: NetworkSummary,
    pub bundles: HashMap<String, StateBundle>,
}

pub struct PeerServer {
    pub address: String,
    shutdown: Arc<Mutex<bool>>,
    thread: Option<JoinHandle<()>>,
}

impl PeerServer {
    pub fn spawn(books: Arc<Mutex<PeerBooks>>) -> std::io::Result<Self> {
        let listener = TcpListener::bind("127.0.0.1:0")?;
        let address = listener.local_addr()?.to_string();
        let shutdown = Arc::new(Mutex::new(false));
        let flag = Arc::clone(&shutdown);
        listener.set_nonblocking(true)?;
        let thread = thread::spawn(move || serve_loop(listener, books, flag));
        Ok(Self {
            address,
            shutdown,
            thread: Some(thread),
        })
    }

    pub fn stop(&mut self) {
        if let Ok(mut flag) = self.shutdown.lock() {
            *flag = true;
        }
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

impl Drop for PeerServer {
    fn drop(&mut self) {
        self.stop();
    }
}

fn serve_loop(listener: TcpListener, books: Arc<Mutex<PeerBooks>>, shutdown: Arc<Mutex<bool>>) {
    while !shutdown.lock().map(|flag| *flag).unwrap_or(true) {
        match listener.accept() {
            Ok((stream, _)) => handle_client(stream, &books),
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                thread::sleep(Duration::from_millis(5));
            }
            Err(_) => break,
        }
    }
}

fn handle_client(mut stream: TcpStream, books: &Arc<Mutex<PeerBooks>>) {
    let _ = stream.set_read_timeout(Some(Duration::from_secs(2)));
    let mut buf = [0u8; 4096];
    let Ok(n) = stream.read(&mut buf) else {
        return;
    };
    let request = String::from_utf8_lossy(&buf[..n]);
    let path = request
        .lines()
        .next()
        .and_then(|line| line.split_whitespace().nth(1))
        .unwrap_or("/");
    let path = path.split('?').next().unwrap_or(path);
    let books = books.lock().ok();
    let (status, body) = if path == "/api/network/summary" {
        let summary = books
            .as_ref()
            .map(|books| serde_json::to_string(&books.summary).unwrap_or_else(|_| "{}".into()))
            .unwrap_or_else(|| "{}".into());
        (200, summary)
    } else if let Some(id) = path.strip_prefix("/api/network/state/") {
        match books.as_ref().and_then(|books| books.bundles.get(id)) {
            Some(body) => (
                200,
                serde_json::to_string(body).unwrap_or_else(|_| "{}".into()),
            ),
            None => (404, String::new()),
        }
    } else if path.starts_with("/api/network/peer-addresses") {
        (404, String::new())
    } else {
        (404, String::new())
    };
    let reason = if status == 200 { "OK" } else { "Not Found" };
    let response = format!(
        "HTTP/1.1 {status} {reason}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    let _ = stream.write_all(response.as_bytes());
}

pub fn http_get(address: &str, path: &str) -> std::io::Result<(u16, String)> {
    let mut stream = TcpStream::connect(address)?;
    let _ = stream.set_read_timeout(Some(Duration::from_secs(2)));
    let request = format!("GET {path} HTTP/1.1\r\nHost: {address}\r\nConnection: close\r\n\r\n");
    stream.write_all(request.as_bytes())?;
    let mut response = String::new();
    stream.read_to_string(&mut response)?;
    let status = response
        .split_whitespace()
        .nth(1)
        .and_then(|code| code.parse().ok())
        .unwrap_or(0);
    let body = response
        .split_once("\r\n\r\n")
        .map(|(_, body)| body.to_string())
        .unwrap_or_default();
    Ok((status, body))
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Default)]
#[serde(rename_all = "camelCase")]
pub struct StateBundle {
    #[serde(default)]
    pub state_id: String,
    #[serde(default)]
    pub consensus_version: i32,
    #[serde(default)]
    pub state_bundle_schema_version: i32,
    #[serde(default)]
    pub http_api_version: i32,
    #[serde(default)]
    pub network_id: String,
    #[serde(default)]
    pub payout_variant: String,
    #[serde(default)]
    pub active_snapshot_id: String,
    #[serde(default)]
    pub support_fee_enabled: bool,
    #[serde(default, deserialize_with = "empty_if_null")]
    pub parent_block_hash: String,
    #[serde(default)]
    pub valid_parent_block_hashes: Vec<String>,
    #[serde(default)]
    pub winners_list: Vec<BundleWinner>,
    #[serde(default)]
    pub proof_winners_list: Vec<BundleWinner>,
    #[serde(default)]
    pub share_proofs: Vec<BundleProof>,
    #[serde(default)]
    pub work_set_proofs: Vec<BundleProof>,
    #[serde(default)]
    pub paid_snapshot_proof_ids: Vec<String>,
    #[serde(default)]
    pub snapshot_family_member: Option<BundleMember>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct BundleWinner {
    #[serde(default)]
    pub value: u64,
    #[serde(default)]
    pub address: String,
    #[serde(default)]
    pub difficulty: f64,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct BundleProof {
    #[serde(default)]
    pub share_id: String,
    #[serde(default)]
    pub difficulty: f64,
    #[serde(default)]
    pub script_pub_key_hex: String,
    #[serde(default)]
    pub prev_block_hash: String,
    #[serde(default)]
    pub miner_address: String,
    #[serde(default)]
    pub header_hex: String,
    #[serde(default)]
    pub coinbase_hex: String,
    #[serde(default)]
    pub merkle_path: Vec<String>,
    #[serde(default)]
    pub timestamp: Option<serde_json::Value>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct BundleMember {
    #[serde(default)]
    pub family_id: String,
    #[serde(default)]
    pub consensus_version: i32,
    #[serde(default)]
    pub network_id: String,
    #[serde(default)]
    pub predecessor_snapshot_id: String,
    #[serde(default)]
    pub boundary_block_hash: String,
    #[serde(default)]
    pub boundary_block_height: i64,
    #[serde(default)]
    pub payout_variant: String,
    #[serde(default)]
    pub snapshot_id: String,
    #[serde(default)]
    pub boundary_reserve_proofs: Vec<BundleProof>,
}

#[derive(Clone, Debug)]
pub struct LocalSync {
    pub network_id: String,
    pub version: VersionFields,
    pub poll: PollSnapshot,
    pub candidate_state_id: String,
    pub winners: Vec<WinnerRow>,
    pub slot_value_sats: u64,
    pub tip: String,
    pub tip_height: i64,
    pub parents: Vec<String>,
    pub reconciled: Vec<ShareProof>,
    pub on_deck_shares: Vec<ShareProof>,
    pub on_deck: Vec<DeckProof>,
    pub family: Option<FamilyState>,
    pub paid_ids: Vec<String>,
    pub math: PayoutMath,
    pub support_enabled: bool,
    pub reserve_limit: usize,
}

#[derive(Clone, Debug, PartialEq)]
pub struct PollOutcome {
    pub stopped: bool,
    pub current_state_id: String,
    pub candidate_state_id: String,
    pub winners: Vec<WinnerRow>,
    pub reconciled: Vec<ShareProof>,
    pub on_deck: Vec<DeckProof>,
    /// Family member of the current bundle when bootstrap or adopt ran.
    pub family: Option<FamilyState>,
    /// Paid ids of that same bundle. A poll that does not adopt keeps the local list.
    pub paid_ids: Vec<String>,
}

pub fn poll_peer(address: &str, local: &LocalSync) -> std::io::Result<PollOutcome> {
    let mut outcome = PollOutcome {
        stopped: true,
        current_state_id: local.poll.current_state_id.clone(),
        candidate_state_id: local.candidate_state_id.clone(),
        winners: local.winners.clone(),
        reconciled: local.reconciled.clone(),
        on_deck: local.on_deck.clone(),
        family: local.family.clone(),
        paid_ids: local.paid_ids.clone(),
    };
    let (status, body) = http_get(address, "/api/network/summary")?;
    if status != 200 {
        return Ok(outcome);
    }
    let remote: NetworkSummary = match serde_json::from_str(&body) {
        Ok(summary) => summary,
        Err(_) => return Ok(outcome),
    };
    if !can_sync_state(
        &local.version,
        &remote.version_fields(),
        &local.network_id,
        &remote.network_id,
    ) {
        return Ok(outcome);
    }
    let _ = http_get(address, "/api/network/peer-addresses?limit=10");
    if !remote.current_state_id.trim().is_empty()
        && !remote
            .current_state_id
            .eq_ignore_ascii_case(&outcome.current_state_id)
        && should_fetch_remote_current_state(&local.poll, &remote.poll_snapshot())
    {
        if let Ok((200, body)) = http_get(
            address,
            &format!("/api/network/state/{}", remote.current_state_id),
        ) {
            if let Ok(bundle) = serde_json::from_str::<StateBundle>(&body) {
                let empty = crate::adopt::local_winners_are_empty(
                    &outcome
                        .winners
                        .iter()
                        .map(|row| crate::adopt::LocalWinner {
                            difficulty: row.difficulty,
                            value_sats: row.value_sats,
                        })
                        .collect::<Vec<_>>(),
                    local.slot_value_sats,
                );
                let mut bootstrapped = false;
                if empty {
                    let work = candidate_proofs(&bundle);
                    if let Ok((winners, trimmed)) = bootstrap_current(
                        &winner_rows(&bundle.winners_list),
                        &work,
                        &remote.current_tip_block_hash,
                        local.reserve_limit,
                        &verifier_winners(&bundle, &outcome.winners),
                        &bundle_parents(&bundle, &work),
                        &local.math,
                    ) {
                        outcome.winners = winners;
                        outcome.on_deck = trimmed;
                        outcome.current_state_id = bundle.state_id.clone();
                        remember_current_bundle(&mut outcome, &bundle);
                        bootstrapped = true;
                    }
                }
                if !bootstrapped {
                    if let Some(adopted) = adopt_bundle(
                        &bundle,
                        local,
                        &outcome,
                        &local.on_deck_shares,
                        &local.paid_ids,
                    ) {
                        match adopted {
                            AdoptOutcome::Unchanged => {}
                            AdoptOutcome::Reconciled {
                                reconciled,
                                on_deck,
                            } => {
                                outcome.on_deck = decks_for_shares(
                                    &outcome.on_deck,
                                    &bundle.boundary_decks(),
                                    &on_deck,
                                );
                                outcome.reconciled = reconciled;
                                outcome.winners = winner_rows(&bundle.winners_list);
                                outcome.current_state_id = bundle.state_id.clone();
                            }
                        }
                        remember_current_bundle(&mut outcome, &bundle);
                    }
                }
            }
        }
    }
    if !outcome
        .current_state_id
        .eq_ignore_ascii_case(&remote.current_state_id)
    {
        return Ok(outcome);
    }
    if remote.candidate_state_id.trim().is_empty()
        || remote
            .candidate_state_id
            .eq_ignore_ascii_case(&outcome.candidate_state_id)
    {
        outcome.stopped = false;
        return Ok(outcome);
    }
    if let Ok((200, body)) = http_get(
        address,
        &format!("/api/network/state/{}", remote.candidate_state_id),
    ) {
        if let Ok(bundle) = serde_json::from_str::<StateBundle>(&body) {
            let remote_proofs = candidate_proofs(&bundle);
            let parents = bundle_parents(&bundle, &remote_proofs);
            if let Ok(merged) = import_candidate(
                &local.version,
                &bundle_version(&bundle),
                &local.network_id,
                &bundle.network_id,
                &outcome.current_state_id,
                &bundle.state_id,
                &bundle.payout_variant,
                &outcome.on_deck,
                &remote_proofs,
                Some(&local.tip),
                local.reserve_limit,
                &verifier_winners(&bundle, &outcome.winners),
                &winner_rows(&bundle.winners_list),
                &local.math,
                &local.parents,
                &parents,
            ) {
                outcome.on_deck = merged;
                outcome.candidate_state_id = bundle.state_id;
                outcome.stopped = false;
            }
        }
    }
    Ok(outcome)
}

fn remember_current_bundle(outcome: &mut PollOutcome, bundle: &StateBundle) {
    outcome.paid_ids = bundle.paid_snapshot_proof_ids.clone();
    let Some(member) = bundle.snapshot_family_member.as_ref() else {
        return;
    };
    if member.family_id.trim().is_empty() {
        return;
    }
    outcome.family = Some(FamilyState {
        family_id: FamilyId(member.family_id.clone()),
        consensus_version: member.consensus_version,
        network_id: member.network_id.clone(),
        predecessor_snapshot_id: member.predecessor_snapshot_id.clone(),
        boundary_block_hash: member.boundary_block_hash.clone(),
        boundary_block_height: member.boundary_block_height,
        payout_variant: member.payout_variant.clone(),
    });
}

fn adopt_bundle(
    bundle: &StateBundle,
    local: &LocalSync,
    outcome: &PollOutcome,
    on_deck_shares: &[ShareProof],
    paid_ids: &[String],
) -> Option<AdoptOutcome> {
    let member = bundle.snapshot_family_member.as_ref()?;
    let family = local.family.as_ref()?;
    let boundary = member
        .boundary_reserve_proofs
        .iter()
        .map(deck_from_bundle)
        .collect::<Vec<_>>();
    let mut parents = bundle_parents(bundle, &boundary);
    parents.push(local.tip.clone());
    import_locked_sibling(
        family,
        &FamilyMember {
            family_id: FamilyId(member.family_id.clone()),
            consensus_version: member.consensus_version,
            network_id: member.network_id.clone(),
            predecessor_snapshot_id: member.predecessor_snapshot_id.clone(),
            boundary_block_hash: member.boundary_block_hash.clone(),
            boundary_block_height: member.boundary_block_height,
            payout_variant: member.payout_variant.clone(),
        },
        &outcome.reconciled,
        &boundary,
        if bundle.paid_snapshot_proof_ids.is_empty() {
            paid_ids
        } else {
            &bundle.paid_snapshot_proof_ids
        },
        on_deck_shares,
        local.reserve_limit,
        &local.math,
        &bundle.network_id,
        &bundle.payout_variant,
        &member.snapshot_id,
        &bundle.active_snapshot_id,
        &local.tip,
        local.tip_height,
        local.support_enabled,
        bundle.support_fee_enabled,
        &verifier_winners(bundle, &outcome.winners),
        &winner_rows(&bundle.winners_list),
        &parents,
    )
    .ok()
}

impl StateBundle {
    pub fn candidate_decks(&self) -> Vec<DeckProof> {
        candidate_proofs(self)
    }

    pub fn boundary_decks(&self) -> Vec<DeckProof> {
        self.snapshot_family_member
            .as_ref()
            .map(|member| {
                member
                    .boundary_reserve_proofs
                    .iter()
                    .map(deck_from_bundle)
                    .collect()
            })
            .unwrap_or_default()
    }
}

pub fn decks_of(proofs: &[BundleProof]) -> Vec<DeckProof> {
    proofs.iter().map(deck_from_bundle).collect()
}

fn decks_for_shares(
    previous: &[DeckProof],
    incoming: &[DeckProof],
    shares: &[ShareProof],
) -> Vec<DeckProof> {
    shares
        .iter()
        .filter_map(|share| {
            previous
                .iter()
                .chain(incoming)
                .find(|deck| deck.share_id.eq_ignore_ascii_case(&share.share_id))
                .cloned()
        })
        .collect()
}

fn candidate_proofs(bundle: &StateBundle) -> Vec<DeckProof> {
    let source = if bundle.work_set_proofs.is_empty() {
        &bundle.share_proofs
    } else {
        &bundle.work_set_proofs
    };
    source.iter().map(deck_from_bundle).collect()
}

fn bundle_parents(bundle: &StateBundle, proofs: &[DeckProof]) -> Vec<String> {
    let mut parents = bundle.valid_parent_block_hashes.clone();
    if !bundle.parent_block_hash.trim().is_empty() {
        parents.push(bundle.parent_block_hash.clone());
    }
    for proof in proofs {
        if !proof.prev_block_hash.trim().is_empty() {
            parents.push(proof.prev_block_hash.clone());
        }
    }
    parents
}

fn bundle_version(bundle: &StateBundle) -> VersionFields {
    VersionFields {
        consensus_version: bundle.consensus_version,
        state_bundle_schema_version: bundle.state_bundle_schema_version,
        http_api_version: bundle.http_api_version,
    }
}

fn winner_rows(winners: &[BundleWinner]) -> Vec<WinnerRow> {
    winners
        .iter()
        .map(|winner| WinnerRow {
            value_sats: winner.value,
            address: winner.address.clone(),
            difficulty: winner.difficulty,
        })
        .collect()
}

fn verifier_winners(bundle: &StateBundle, local: &[WinnerRow]) -> Vec<ExpectedWinner> {
    let source = if bundle.proof_winners_list.is_empty() {
        local
            .iter()
            .map(|row| ExpectedWinner {
                value_sats: row.value_sats,
                address: row.address.clone(),
            })
            .collect()
    } else {
        bundle
            .proof_winners_list
            .iter()
            .map(|row| ExpectedWinner {
                value_sats: row.value,
                address: row.address.clone(),
            })
            .collect()
    };
    source
}

fn deck_from_bundle(proof: &BundleProof) -> DeckProof {
    DeckProof {
        share_id: proof.share_id.clone(),
        difficulty: proof.difficulty,
        script_pub_key_hex: proof.script_pub_key_hex.clone(),
        prev_block_hash: proof.prev_block_hash.clone(),
        timestamp_millis: proof.timestamp.as_ref().map(unix_millis).unwrap_or(0),
        miner_address: proof.miner_address.clone(),
        header_hex: proof.header_hex.clone(),
        coinbase_hex: proof.coinbase_hex.clone(),
        merkle_path: proof.merkle_path.clone(),
    }
}

fn unix_millis(value: &serde_json::Value) -> i64 {
    match value {
        serde_json::Value::Number(number) => number.as_i64().unwrap_or(0),
        serde_json::Value::String(text) => parse_rfc3339_millis(text).unwrap_or(0),
        _ => 0,
    }
}

fn parse_rfc3339_millis(text: &str) -> Option<i64> {
    let text = text.trim().trim_end_matches('Z');
    let (date, time) = text.split_once('T')?;
    let mut date_parts = date.split('-');
    let year: i32 = date_parts.next()?.parse().ok()?;
    let month: u32 = date_parts.next()?.parse().ok()?;
    let day: u32 = date_parts.next()?.parse().ok()?;
    let (clock, fraction) = time.split_once('.').unwrap_or((time, ""));
    let mut clock_parts = clock.split(':');
    let hour: u32 = clock_parts.next()?.parse().ok()?;
    let minute: u32 = clock_parts.next()?.parse().ok()?;
    let second: u32 = clock_parts.next()?.parse().ok()?;
    let mut millis = 0i64;
    if !fraction.is_empty() {
        let digits: String = fraction.chars().take(3).collect();
        millis = format!("{digits:0<3}").parse().unwrap_or(0);
    }
    let days = days_from_civil(year, month, day)?;
    Some(
        days * 86_400_000
            + hour as i64 * 3_600_000
            + minute as i64 * 60_000
            + second as i64 * 1000
            + millis,
    )
}

fn empty_if_null<'de, D>(deserializer: D) -> Result<String, D::Error>
where
    D: serde::Deserializer<'de>,
{
    Ok(Option::<String>::deserialize(deserializer)?.unwrap_or_default())
}

fn days_from_civil(year: i32, month: u32, day: u32) -> Option<i64> {
    if !(1..=12).contains(&month) || !(1..=31).contains(&day) {
        return None;
    }
    let year = if month <= 2 { year - 1 } else { year };
    let era = if year >= 0 { year } else { year - 399 } / 400;
    let yoe = (year - era * 400) as u64;
    let month = if month > 2 { month - 3 } else { month + 9 };
    let doy = (153 * month + 2) / 5 + day - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy as u64;
    Some(era as i64 * 146097 + doe as i64 - 719468)
}
