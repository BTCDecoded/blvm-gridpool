//! Saved job and one sibling poll.
//!
//! `config.toml` is the sibling base URL and network id. No URL means do not
//! poll. `state.json` is the held job. A closed hold is not polled: the live
//! list is canon, and a poll would change the list about to be paid.

use std::io;
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::family::{
    snapshot_proof_slot_count, work_set_reserve_limit, FamilyState, CONSENSUS_VERSION,
    REFERENCE_RESERVE_MULTIPLIER, REFERENCE_SUPPORT_FEE_ENABLED, REFERENCE_WINNERS_LIST_SIZE,
};
use crate::import::{
    DeckProof, PayoutMath, PollSnapshot, VersionFields, WinnerRow, HTTP_API_VERSION,
    STATE_BUNDLE_SCHEMA_VERSION,
};
use crate::job::Canon;
use crate::payout::shared_slot_value_sats;
use crate::peer::{poll_peer, LocalSync, PollOutcome};
use crate::reconcile::ShareProof;

const SUBSIDY_SATS: u64 = 312_500_000;
const TOTAL_SLOTS: u64 = 300;

fn default_network() -> String {
    "mainnet".into()
}

fn default_reserve_limit() -> usize {
    work_set_reserve_limit(
        snapshot_proof_slot_count(REFERENCE_WINNERS_LIST_SIZE),
        REFERENCE_RESERVE_MULTIPLIER,
    ) as usize
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct GridPoolConfig {
    #[serde(default)]
    pub peer_url: String,
    #[serde(default = "default_network")]
    pub network_id: String,
}

impl Default for GridPoolConfig {
    fn default() -> Self {
        Self {
            peer_url: String::new(),
            network_id: default_network(),
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Follower {
    #[serde(default)]
    pub peer_url: String,
    #[serde(default = "default_network")]
    pub network_id: String,
    #[serde(default)]
    pub canon: Canon,
    #[serde(default)]
    pub published: Vec<ShareProof>,
    #[serde(default)]
    pub canon_addresses: Vec<String>,
    #[serde(default)]
    pub published_addresses: Vec<String>,
    #[serde(default)]
    pub hold_closed: bool,
    #[serde(default)]
    pub family: Option<FamilyState>,
    #[serde(default)]
    pub paid_ids: Vec<String>,
    #[serde(default)]
    pub tip_hash: String,
    #[serde(default)]
    pub tip_height: i64,
    #[serde(default)]
    pub winners: Vec<WinnerRow>,
    #[serde(default)]
    pub on_deck: Vec<DeckProof>,
    #[serde(default)]
    pub current_state_id: String,
    #[serde(default)]
    pub candidate_state_id: String,
    #[serde(default)]
    pub parents: Vec<String>,
    #[serde(default = "default_reserve_limit")]
    pub reserve_limit: usize,
}

impl Default for Follower {
    fn default() -> Self {
        Self::fresh(&default_network())
    }
}

impl Follower {
    pub fn fresh(network_id: &str) -> Self {
        Self {
            peer_url: String::new(),
            network_id: network_id.to_string(),
            canon: Canon::default(),
            published: Vec::new(),
            canon_addresses: Vec::new(),
            published_addresses: Vec::new(),
            hold_closed: false,
            family: None,
            paid_ids: Vec::new(),
            tip_hash: String::new(),
            tip_height: 0,
            winners: Vec::new(),
            on_deck: Vec::new(),
            current_state_id: String::new(),
            candidate_state_id: String::new(),
            parents: Vec::new(),
            reserve_limit: default_reserve_limit(),
        }
    }

    pub fn local_sync(&self) -> LocalSync {
        let parents = if self.parents.is_empty() && !self.tip_hash.is_empty() {
            vec![self.tip_hash.clone()]
        } else {
            self.parents.clone()
        };
        LocalSync {
            network_id: self.network_id.clone(),
            version: VersionFields {
                consensus_version: CONSENSUS_VERSION,
                state_bundle_schema_version: STATE_BUNDLE_SCHEMA_VERSION,
                http_api_version: HTTP_API_VERSION,
            },
            poll: PollSnapshot {
                winners_count: self.winners.len() as i32,
                on_deck_count: self.on_deck.len() as i32,
                current_round_number: 0,
                current_state_id: self.current_state_id.clone(),
                current_state_proof_count: self.canon.proofs.len() as i32,
                current_state_total_difficulty: self
                    .canon
                    .proofs
                    .iter()
                    .map(|proof| proof.difficulty)
                    .sum(),
            },
            candidate_state_id: self.candidate_state_id.clone(),
            winners: self.winners.clone(),
            slot_value_sats: shared_slot_value_sats(SUBSIDY_SATS, TOTAL_SLOTS),
            tip: self.tip_hash.clone(),
            tip_height: self.tip_height,
            parents,
            reconciled: self.canon.proofs.clone(),
            on_deck_shares: self
                .on_deck
                .iter()
                .map(|proof| ShareProof {
                    share_id: proof.share_id.clone(),
                    difficulty: proof.difficulty,
                })
                .collect(),
            on_deck: self.on_deck.clone(),
            family: self.family.clone(),
            paid_ids: self.paid_ids.clone(),
            math: PayoutMath {
                subsidy_sats: SUBSIDY_SATS,
                total_slots: TOTAL_SLOTS,
                shared_slots: TOTAL_SLOTS.saturating_sub(1) as usize,
                support_address: None,
                network: self.network_id.clone(),
            },
            support_enabled: REFERENCE_SUPPORT_FEE_ENABLED,
            reserve_limit: self.reserve_limit,
        }
    }
}

/// Host and port for `poll_peer`. An empty URL does not poll.
pub fn peer_address(peer_url: &str) -> Option<String> {
    let url = peer_url.trim();
    if url.is_empty() {
        return None;
    }
    let rest = url
        .strip_prefix("https://")
        .or_else(|| url.strip_prefix("http://"))
        .unwrap_or(url);
    let host = rest.split('/').next().unwrap_or("").trim();
    if host.is_empty() {
        return None;
    }
    if host.contains(':') {
        Some(host.to_string())
    } else {
        Some(format!("{host}:80"))
    }
}

pub fn load_config(dir: &Path) -> io::Result<GridPoolConfig> {
    let path = dir.join("config.toml");
    if !path.exists() {
        return Ok(GridPoolConfig::default());
    }
    let text = std::fs::read_to_string(path)?;
    toml::from_str(&text).map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))
}

/// `Ok(None)` when the file is absent. A parse error leaves the file in place.
pub fn load_state(dir: &Path) -> io::Result<Option<Follower>> {
    let path = dir.join("state.json");
    if !path.exists() {
        return Ok(None);
    }
    let text = std::fs::read_to_string(path)?;
    serde_json::from_str(&text)
        .map(Some)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))
}

pub fn save_state(dir: &Path, follower: &Follower) -> io::Result<()> {
    let path = dir.join("state.json");
    let text = serde_json::to_string_pretty(follower)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
    std::fs::write(path, text)
}

/// `config.toml` supplies the sibling URL and network. A missing file does not
/// poll and does not replace the network stored with the job.
pub fn apply_saved_config(held: &mut Follower, config: Option<&GridPoolConfig>) {
    let Some(config) = config else {
        held.peer_url.clear();
        return;
    };
    held.peer_url = config.peer_url.clone();
    if !config.network_id.trim().is_empty() {
        held.network_id = config.network_id.clone();
    }
}

/// Poll only while the hold is open and a sibling URL is configured.
pub fn poll_open_hold(follower: &Follower) -> io::Result<Option<PollOutcome>> {
    if follower.hold_closed {
        return Ok(None);
    }
    let Some(address) = peer_address(&follower.peer_url) else {
        return Ok(None);
    };
    poll_peer(&address, &follower.local_sync()).map(Some)
}

/// Apply a poll that imported something. A closed hold and an empty import do nothing.
/// The first import copies proofs and winner addresses into the published job.
/// A later open hold updates canon and the canon addresses only.
pub fn apply_poll(store: &mut Follower, outcome: &PollOutcome) -> bool {
    if store.hold_closed {
        return false;
    }
    let addresses: Vec<String> = outcome
        .winners
        .iter()
        .map(|winner| winner.address.clone())
        .collect();
    let same = outcome.reconciled == store.canon.proofs
        && addresses == store.canon_addresses
        && outcome.winners == store.winners
        && outcome.on_deck == store.on_deck
        && outcome.current_state_id == store.current_state_id
        && outcome.candidate_state_id == store.candidate_state_id
        && outcome.paid_ids == store.paid_ids
        && outcome.family == store.family
        && (!store.published.is_empty() || outcome.reconciled.is_empty());
    if same {
        return false;
    }
    store.winners = outcome.winners.clone();
    store.canon.proofs = outcome.reconciled.clone();
    store.canon_addresses = addresses;
    store.on_deck = outcome.on_deck.clone();
    store.current_state_id = outcome.current_state_id.clone();
    store.candidate_state_id = outcome.candidate_state_id.clone();
    store.family = outcome.family.clone();
    store.paid_ids = outcome.paid_ids.clone();
    if store.published.is_empty() {
        store.published = store.canon.proofs.clone();
        store.published_addresses = store.canon_addresses.clone();
    }
    true
}

/// A poll error, or a summary that imports nothing, leaves the saved job unchanged.
pub fn absorb_poll(store: &mut Follower, result: io::Result<Option<PollOutcome>>) {
    let Ok(Some(outcome)) = result else {
        return;
    };
    apply_poll(store, &outcome);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn proof(id: &str) -> ShareProof {
        ShareProof {
            share_id: id.into(),
            difficulty: 1.0,
        }
    }

    fn winner(address: &str) -> WinnerRow {
        WinnerRow {
            value_sats: 1,
            address: address.into(),
            difficulty: 1.0,
        }
    }

    fn outcome_from(store: &Follower) -> PollOutcome {
        PollOutcome {
            stopped: true,
            current_state_id: store.current_state_id.clone(),
            candidate_state_id: store.candidate_state_id.clone(),
            winners: store.winners.clone(),
            reconciled: store.canon.proofs.clone(),
            on_deck: store.on_deck.clone(),
            family: store.family.clone(),
            paid_ids: store.paid_ids.clone(),
        }
    }

    #[test]
    fn restart_keeps_the_published_ids() {
        let dir = std::env::temp_dir().join(format!(
            "gridpool-restart-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir(&dir).unwrap();
        let mut saved = Follower::fresh("mainnet");
        saved.published = vec![proof("published-share")];
        saved.published_addresses = vec!["bc1q-published".into()];
        saved.canon.proofs = vec![proof("canon-share")];
        saved.canon.header_parent = "parent".into();
        saved.canon_addresses = vec!["bc1q-canon".into()];
        saved.hold_closed = false;
        saved.paid_ids = vec!["paid-share".into()];
        saved.tip_hash = "tip".into();
        saved.tip_height = 10;
        save_state(&dir, &saved).unwrap();
        let loaded = load_state(&dir).unwrap().unwrap();
        assert_eq!(
            loaded
                .published
                .iter()
                .map(|proof| proof.share_id.as_str())
                .collect::<Vec<_>>(),
            ["published-share"]
        );
        assert_eq!(loaded.published_addresses, ["bc1q-published"]);
        assert_eq!(loaded.canon.header_parent, "parent");
        assert_eq!(loaded.paid_ids, ["paid-share"]);
        assert_eq!(loaded.tip_hash, "tip");
        assert_eq!(loaded.tip_height, 10);
        let _ = std::fs::remove_file(dir.join("state.json"));
        let _ = std::fs::remove_file(dir.join("config.toml"));
        let _ = std::fs::remove_dir(&dir);
    }

    #[test]
    fn second_open_poll_does_not_replace_published_ids() {
        let mut store = Follower::fresh("mainnet");
        store.published = vec![proof("published-share")];
        store.published_addresses = vec!["bc1q-published".into()];
        store.canon.proofs = vec![proof("published-share")];
        store.canon_addresses = vec!["bc1q-published".into()];
        store.winners = vec![winner("bc1q-published")];
        store.current_state_id = "first".into();
        let mut outcome = outcome_from(&store);
        outcome.reconciled = vec![proof("canon-share")];
        outcome.winners = vec![winner("bc1q-canon")];
        outcome.current_state_id = "second".into();
        assert!(apply_poll(&mut store, &outcome));
        assert_eq!(store.published[0].share_id, "published-share");
        assert_eq!(store.published_addresses, ["bc1q-published"]);
        assert_eq!(store.canon.proofs[0].share_id, "canon-share");
        assert_eq!(store.canon_addresses, ["bc1q-canon"]);
    }

    #[test]
    fn first_poll_copies_proofs_and_winner_addresses_into_the_published_job() {
        let mut store = Follower::fresh("mainnet");
        let mut outcome = outcome_from(&store);
        outcome.reconciled = vec![proof("one"), proof("two")];
        outcome.winners = vec![winner("bc1q-one"), winner("bc1q-two")];
        outcome.current_state_id = "boot".into();
        assert!(apply_poll(&mut store, &outcome));
        assert_eq!(
            store
                .published
                .iter()
                .map(|proof| proof.share_id.as_str())
                .collect::<Vec<_>>(),
            ["one", "two"]
        );
        assert_eq!(store.published_addresses, ["bc1q-one", "bc1q-two"]);
        assert_eq!(store.canon_addresses, store.published_addresses);
    }

    #[test]
    fn closed_hold_does_not_poll_or_apply() {
        let mut store = Follower::fresh("mainnet");
        store.hold_closed = true;
        store.peer_url = "http://127.0.0.1:1".into();
        store.published = vec![proof("held")];
        store.canon.proofs = vec![proof("held")];
        assert!(poll_open_hold(&store).unwrap().is_none());
        let mut outcome = outcome_from(&store);
        outcome.reconciled = vec![proof("other")];
        outcome.winners = vec![winner("bc1q-other")];
        assert!(!apply_poll(&mut store, &outcome));
        assert_eq!(store.canon.proofs[0].share_id, "held");
        assert_eq!(store.published[0].share_id, "held");
    }

    #[test]
    fn missing_url_does_not_poll() {
        let store = Follower::fresh("mainnet");
        assert!(poll_open_hold(&store).unwrap().is_none());
        let dir = std::env::temp_dir().join(format!(
            "gridpool-config-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir(&dir).unwrap();
        assert!(load_config(&dir).unwrap().peer_url.is_empty());
        std::fs::write(
            dir.join("config.toml"),
            "peer_url = \"http://127.0.0.1:9\"\nnetwork_id = \"testnet\"\n",
        )
        .unwrap();
        let config = load_config(&dir).unwrap();
        assert_eq!(config.peer_url, "http://127.0.0.1:9");
        assert_eq!(config.network_id, "testnet");
        assert_eq!(
            peer_address(&config.peer_url).as_deref(),
            Some("127.0.0.1:9")
        );
        let _ = std::fs::remove_file(dir.join("config.toml"));
        let _ = std::fs::remove_dir(&dir);
    }

    #[test]
    fn a_poll_error_leaves_the_published_ids() {
        let mut store = Follower::fresh("mainnet");
        store.peer_url = "http://127.0.0.1:1".into();
        store.published = vec![proof("kept")];
        let result = poll_open_hold(&store);
        assert!(result.is_err());
        absorb_poll(&mut store, result);
        assert_eq!(store.published[0].share_id, "kept");
    }

    #[test]
    fn a_missing_config_keeps_the_saved_network() {
        let mut held = Follower::fresh("testnet");
        held.peer_url = "http://127.0.0.1:9".into();
        apply_saved_config(&mut held, None);
        assert!(held.peer_url.is_empty());
        assert_eq!(held.network_id, "testnet");
        apply_saved_config(
            &mut held,
            Some(&GridPoolConfig {
                peer_url: "http://127.0.0.1:9".into(),
                network_id: "mainnet".into(),
            }),
        );
        assert_eq!(held.peer_url, "http://127.0.0.1:9");
        assert_eq!(held.network_id, "mainnet");
    }
}
