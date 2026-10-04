//! `gridpool_*` methods. These names are not `commons_*` and the node does not call them.

use std::collections::HashSet;
use std::sync::Arc;

use async_trait::async_trait;
use blvm_node::module::inter_module::api::ModuleAPI;
use blvm_node::module::traits::{ModuleError, NodeAPI};
use tokio::sync::RwLock;

use super::GridState;
use crate::follow::save_state;
use crate::job::{job_after_payment, pay_published_list, published_job};
use crate::payout::coinbase_job;
use crate::reconcile::ShareProof;

pub struct GridPoolApi {
    pub state: Arc<RwLock<GridState>>,
    pub node_api: Arc<dyn NodeAPI>,
}

#[async_trait]
impl ModuleAPI for GridPoolApi {
    async fn handle_request(
        &self,
        method: &str,
        params: &[u8],
        _caller_module_id: &str,
    ) -> Result<Vec<u8>, ModuleError> {
        match method {
            "gridpool_get_coinbase_outputs" => {
                let request: serde_json::Value = if params.is_empty() {
                    serde_json::json!({})
                } else {
                    serde_json::from_slice(params)
                        .map_err(|error| ModuleError::OperationError(error.to_string()))?
                };
                let state = self.state.read().await;
                let job = published_job(
                    state.held.hold_closed,
                    &state.held.published,
                    &state.held.canon,
                );
                let addresses = if state.held.hold_closed {
                    &state.held.canon_addresses
                } else {
                    &state.held.published_addresses
                };
                let support = request["support_address"].as_str().map(str::to_string);
                let mut body = coinbase_job(
                    addresses,
                    request["subsidy_sats"].as_u64().unwrap_or(312_500_000),
                    request["total_slots"].as_u64().unwrap_or(300),
                    support.as_deref(),
                    request["network"].as_str().unwrap_or("mainnet"),
                )
                .map_err(ModuleError::OperationError)?;
                body["header_parent"] = serde_json::json!(job.header_parent);
                serde_json::to_vec(&body)
                    .map_err(|error| ModuleError::OperationError(error.to_string()))
            }
            "gridpool_close_hold" => {
                let mut state = self.state.write().await;
                state.held.hold_closed = true;
                let job = published_job(true, &state.held.published, &state.held.canon);
                state.held.published = job.proofs;
                state.held.published_addresses = state.held.canon_addresses.clone();
                persist(&state);
                serde_json::to_vec(&serde_json::json!({"hold_closed": true}))
                    .map_err(|error| ModuleError::OperationError(error.to_string()))
            }
            "gridpool_note_payment" => {
                let mut state = self.state.write().await;
                let remaining = pay_published_list(
                    &state.held.canon.proofs,
                    &state.held.published,
                    state.held.reserve_limit,
                );
                state.held.canon_addresses = keep_addresses(
                    &state.held.canon.proofs,
                    &state.held.canon_addresses,
                    &remaining,
                );
                state.held.canon.proofs = remaining.clone();
                state.held.published_addresses = keep_addresses(
                    &state.held.published,
                    &state.held.published_addresses,
                    &remaining,
                );
                state.held.published = job_after_payment(&remaining);
                state.held.hold_closed = false;
                persist(&state);
                serde_json::to_vec(&serde_json::json!({
                    "share_ids": remaining.iter().map(|proof| &proof.share_id).collect::<Vec<_>>(),
                }))
                .map_err(|error| ModuleError::OperationError(error.to_string()))
            }
            "gridpool_submit_block" => {
                let block: blvm_node::Block = bincode::deserialize(params)
                    .map_err(|error| ModuleError::OperationError(error.to_string()))?;
                let result = self.node_api.submit_block(block).await?;
                serde_json::to_vec(&serde_json::json!({ "result": format!("{result:?}") }))
                    .map_err(|error| ModuleError::OperationError(error.to_string()))
            }
            "gridpool_status" => {
                let state = self.state.read().await;
                serde_json::to_vec(&serde_json::json!({
                    "consensus_version": state.summary.consensus_version,
                    "network_id": state.summary.network_id,
                    "header_parent": state.held.canon.header_parent,
                    "tip_height": state.held.tip_height,
                }))
                .map_err(|error| ModuleError::OperationError(error.to_string()))
            }
            _ => Err(ModuleError::OperationError(format!(
                "unknown method {method}"
            ))),
        }
    }

    fn list_methods(&self) -> Vec<String> {
        vec![
            "gridpool_get_coinbase_outputs".into(),
            "gridpool_close_hold".into(),
            "gridpool_note_payment".into(),
            "gridpool_submit_block".into(),
            "gridpool_status".into(),
        ]
    }

    fn api_version(&self) -> u32 {
        1
    }
}

fn persist(state: &super::GridState) {
    if state.persist {
        if let Err(error) = save_state(&state.data_dir, &state.held) {
            tracing::warn!("gridpool state: {error}");
        }
    }
}

fn keep_addresses(proofs: &[ShareProof], addresses: &[String], kept: &[ShareProof]) -> Vec<String> {
    let wanted: HashSet<&str> = kept.iter().map(|proof| proof.share_id.as_str()).collect();
    proofs
        .iter()
        .zip(addresses)
        .filter(|(proof, _)| wanted.contains(proof.share_id.as_str()))
        .map(|(_, address)| address.clone())
        .collect()
}
