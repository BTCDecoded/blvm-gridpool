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
use crate::job::{canon_after_local_block, published_job, Canon};
use crate::peer::{NetworkSummary, PeerBooks, PeerServer};
use crate::reconcile::ShareProof;

use self::api::GridPoolApi;

#[derive(Clone)]
pub struct GridPoolModule {
    state: Arc<RwLock<GridState>>,
    node_api: Arc<dyn blvm_node::module::traits::NodeAPI>,
    _server: Arc<std::sync::Mutex<PeerServer>>,
}

pub struct GridState {
    pub canon: Canon,
    pub published: Vec<ShareProof>,
    pub canon_addresses: Vec<String>,
    pub published_addresses: Vec<String>,
    pub hold_closed: bool,
    pub reserve_limit: usize,
    pub summary: NetworkSummary,
    pub books: Arc<std::sync::Mutex<PeerBooks>>,
}

impl GridPoolModule {
    pub fn new(
        data_dir: PathBuf,
        node_api: Arc<dyn blvm_node::module::traits::NodeAPI>,
    ) -> std::io::Result<Self> {
        let summary = NetworkSummary::consensus_22("mainnet", "http://127.0.0.1");
        let books = Arc::new(std::sync::Mutex::new(PeerBooks {
            summary: summary.clone(),
            bundles: std::collections::HashMap::new(),
        }));
        let server = PeerServer::spawn(Arc::clone(&books))?;
        let _ = data_dir;
        Ok(Self {
            state: Arc::new(RwLock::new(GridState {
                canon: Canon {
                    proofs: Vec::new(),
                    header_parent: String::new(),
                },
                published: Vec::new(),
                canon_addresses: Vec::new(),
                published_addresses: Vec::new(),
                hold_closed: false,
                reserve_limit: crate::family::work_set_reserve_limit(
                    crate::family::snapshot_proof_slot_count(
                        crate::family::REFERENCE_WINNERS_LIST_SIZE,
                    ),
                    crate::family::REFERENCE_RESERVE_MULTIPLIER,
                ) as usize,
                summary,
                books,
            })),
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
        state.canon = canon_after_local_block(&state.canon.proofs, &parent);
        state.summary.current_tip_block_hash = parent;
        state.summary.current_tip_block_height = height as i64;
        let job = published_job(state.hold_closed, &state.published, &state.canon);
        if state.hold_closed {
            state.published = job.proofs;
            state.published_addresses = state.canon_addresses.clone();
        }
        if let Ok(mut books) = state.books.lock() {
            books.summary = state.summary.clone();
        }
        Ok(())
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
        self.note_block(block_hash, height, &api).await
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
            state.canon.header_parent,
            state.canon.proofs.len(),
            state.published.len()
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
            "header_parent": state.canon.header_parent,
            "reserve": state.canon.proofs.len(),
        }))
    }
}
