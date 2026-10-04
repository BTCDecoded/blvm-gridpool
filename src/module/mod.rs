//! Node process for GridPool. Present only with `--features module`.
//!
//! `getblocktemplate` is not registered here. Commons keeps that call.

pub mod api;

use std::path::PathBuf;
use std::sync::Arc;

use blvm_node::module::ipc::protocol::{EventMessage, EventPayload};
use blvm_sdk::module::prelude::*;
use blvm_sdk_macros::module;
use serde_json::Value;
use tokio::sync::RwLock;

use crate::bitcoin::display_hash;
use crate::follow::{
    absorb_poll, apply_poll, load_config, load_state, poll_open_hold, save_state, Follower,
};
use crate::job::{canon_after_local_block, published_job};
use crate::peer::{NetworkSummary, PeerBooks, PeerServer};

use self::api::GridPoolApi;

#[derive(Clone)]
pub struct GridPoolModule {
    state: Arc<RwLock<GridState>>,
    node_api: Arc<dyn blvm_node::module::traits::NodeAPI>,
    _server: Arc<std::sync::Mutex<PeerServer>>,
}

pub struct GridState {
    pub held: Follower,
    pub summary: NetworkSummary,
    pub books: Arc<std::sync::Mutex<PeerBooks>>,
    pub data_dir: PathBuf,
    /// False when `state.json` could not be read. Do not overwrite that file.
    pub persist: bool,
}

impl GridPoolModule {
    pub fn new(
        data_dir: PathBuf,
        node_api: Arc<dyn blvm_node::module::traits::NodeAPI>,
    ) -> std::io::Result<Self> {
        let config = if data_dir.join("config.toml").exists() {
            match load_config(&data_dir) {
                Ok(config) => Some(config),
                Err(error) => {
                    tracing::warn!("gridpool config: {error}");
                    None
                }
            }
        } else {
            None
        };
        let network = config
            .as_ref()
            .map(|config| config.network_id.as_str())
            .filter(|network| !network.trim().is_empty())
            .unwrap_or("mainnet");
        let (mut held, persist) = match load_state(&data_dir) {
            Ok(Some(loaded)) => (loaded, true),
            Ok(None) => (Follower::fresh(network), true),
            Err(error) => {
                tracing::warn!("gridpool state: {error}");
                (Follower::fresh(network), false)
            }
        };
        crate::follow::apply_saved_config(&mut held, config.as_ref());
        let snapshot = held.clone();
        let polled = std::thread::spawn(move || poll_open_hold(&snapshot)).join();
        if let Ok(result) = polled {
            absorb_poll(&mut held, result);
        }
        let mut summary = NetworkSummary::consensus_22(&held.network_id, "http://127.0.0.1");
        summary.current_tip_block_hash = held.tip_hash.clone();
        summary.current_tip_block_height = held.tip_height;
        summary.current_state_id = held.current_state_id.clone();
        summary.candidate_state_id = held.candidate_state_id.clone();
        let books = Arc::new(std::sync::Mutex::new(PeerBooks {
            summary: summary.clone(),
            bundles: std::collections::HashMap::new(),
        }));
        let server = PeerServer::spawn(Arc::clone(&books))?;
        let mut state = GridState {
            held,
            summary,
            books,
            data_dir,
            persist,
        };
        publish_summary_and_save(&mut state);
        Ok(Self {
            state: Arc::new(RwLock::new(state)),
            node_api,
            _server: Arc::new(std::sync::Mutex::new(server)),
        })
    }

    pub fn api(&self) -> GridPoolApi {
        GridPoolApi {
            state: Arc::clone(&self.state),
            node_api: Arc::clone(&self.node_api),
        }
    }

    async fn note_block(
        &self,
        block_hash: [u8; 32],
        height: u64,
        api: &Arc<dyn blvm_node::module::traits::NodeAPI>,
    ) -> Result<(), ModuleError> {
        let found = api.get_block(&block_hash).await?;
        if found.is_none() {
            return Ok(());
        }
        let parent = display_hash(&block_hash[..]);
        let mut state = self.state.write().await;
        state.held.canon = canon_after_local_block(&state.held.canon.proofs, &parent);
        state.held.tip_hash = parent.clone();
        state.held.tip_height = height as i64;
        if !parent.is_empty() && !state.held.parents.iter().any(|known| known == &parent) {
            state.held.parents.push(parent);
        }
        let job = published_job(
            state.held.hold_closed,
            &state.held.published,
            &state.held.canon,
        );
        if state.held.hold_closed {
            state.held.published = job.proofs;
            state.held.published_addresses = state.held.canon_addresses.clone();
        }
        publish_summary_and_save(&mut state);
        Ok(())
    }

    async fn poll_sibling(&self) {
        let snapshot = {
            let state = self.state.read().await;
            state.held.clone()
        };
        if snapshot.hold_closed || snapshot.peer_url.trim().is_empty() {
            return;
        }
        let polled = tokio::task::spawn_blocking(move || poll_open_hold(&snapshot)).await;
        let outcome = match polled {
            Ok(Ok(Some(outcome))) => outcome,
            Ok(Ok(None)) => return,
            Ok(Err(error)) => {
                tracing::warn!("gridpool poll: {error}");
                return;
            }
            Err(error) => {
                tracing::warn!("gridpool poll thread: {error}");
                return;
            }
        };
        let mut state = self.state.write().await;
        if state.held.hold_closed {
            return;
        }
        if apply_poll(&mut state.held, &outcome) {
            publish_summary_and_save(&mut state);
        }
    }
}

fn publish_summary_and_save(state: &mut GridState) {
    state.summary.network_id = state.held.network_id.clone();
    state.summary.current_tip_block_hash = state.held.tip_hash.clone();
    state.summary.current_tip_block_height = state.held.tip_height;
    state.summary.current_state_id = state.held.current_state_id.clone();
    state.summary.candidate_state_id = state.held.candidate_state_id.clone();
    state.summary.winners_count = state.held.winners.len() as i32;
    state.summary.on_deck_count = state.held.on_deck.len() as i32;
    state.summary.current_state_proof_count = state.held.canon.proofs.len() as i32;
    state.summary.current_state_total_difficulty = state
        .held
        .canon
        .proofs
        .iter()
        .map(|proof| proof.difficulty)
        .sum();
    if let Ok(mut books) = state.books.lock() {
        books.summary = state.summary.clone();
    }
    if state.persist {
        if let Err(error) = save_state(&state.data_dir, &state.held) {
            tracing::warn!("gridpool state: {error}");
        }
    }
}

#[module]
impl GridPoolModule {
    #[on_event(NewBlock)]
    async fn on_new_block(
        &self,
        event: &EventMessage,
        ctx: &InvocationContext,
    ) -> Result<(), ModuleError> {
        let EventPayload::NewBlock { block_hash, height } = event.payload else {
            return Ok(());
        };
        let Some(api) = ctx.node_api() else {
            return Err(ModuleError::OperationError(
                "gridpool node api missing".into(),
            ));
        };
        self.note_block(block_hash, height, &api).await?;
        self.poll_sibling().await;
        Ok(())
    }

    #[command]
    fn status(&self, _ctx: &InvocationContext) -> Result<String, ModuleError> {
        let state = self
            .state
            .try_read()
            .map_err(|_| ModuleError::OperationError("gridpool busy".into()))?;
        Ok(format!(
            "consensus={} parent={} reserve={} published={}\n",
            state.summary.consensus_version,
            state.held.canon.header_parent,
            state.held.canon.proofs.len(),
            state.held.published.len()
        ))
    }

    #[rpc_method(name = "gridpool_status")]
    fn rpc_status(
        &self,
        _params: &Value,
        _db: &std::sync::Arc<dyn blvm_node::storage::database::Database>,
    ) -> Result<Value, ModuleError> {
        let state = self
            .state
            .try_read()
            .map_err(|_| ModuleError::OperationError("gridpool busy".into()))?;
        Ok(serde_json::json!({
            "consensus_version": state.summary.consensus_version,
            "header_parent": state.held.canon.header_parent,
            "reserve": state.held.canon.proofs.len(),
        }))
    }
}
