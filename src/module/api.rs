//! `gridpool_*` methods. These names are not `commons_*` and the node does not call them.

use std::collections::HashSet;
use std::sync::Arc;

use async_trait::async_trait;
use blvm_node::module::inter_module::api::ModuleAPI;
use blvm_node::module::traits::{ModuleError, NodeAPI};
use tokio::sync::RwLock;

use super::GridState;
use crate::job::{job_after_payment, pay_published_list, published_job};
use crate::payout::coinbase_outputs;
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
                let job = published_job(state.hold_closed, &state.published, &state.canon);
                let addresses = if state.hold_closed {
                    &state.canon_addresses
                } else {
                    &state.published_addresses
                };
                let support = request["support_address"].as_str();
                let rows = coinbase_outputs(
                    addresses,
                    request["subsidy_sats"].as_u64().unwrap_or(312_500_000),
                    request["fee_sats"].as_u64().unwrap_or(0),
                    request["total_slots"].as_u64().unwrap_or(300),
                    support,
                    request["network"].as_str().unwrap_or("mainnet"),
                )
                .map_err(|error| ModuleError::OperationError(error))?;
                serde_json::to_vec(&serde_json::json!({
                    "header_parent": job.header_parent,
                    "outputs": rows,
                }))
                .map_err(|error| ModuleError::OperationError(error.to_string()))
            }
            "gridpool_close_hold" => {
                let mut state = self.state.write().await;
                state.hold_closed = true;
                let job = published_job(true, &state.published, &state.canon);
                state.published = job.proofs;
                state.published_addresses = state.canon_addresses.clone();
                serde_json::to_vec(&serde_json::json!({"hold_closed": true}))
                    .map_err(|error| ModuleError::OperationError(error.to_string()))
            }
            "gridpool_note_payment" => {
                let mut state = self.state.write().await;
                let remaining =
                    pay_published_list(&state.canon.proofs, &state.published, state.reserve_limit);
                state.canon_addresses =
                    keep_addresses(&state.canon.proofs, &state.canon_addresses, &remaining);
                state.canon.proofs = remaining.clone();
                state.published_addresses =
                    keep_addresses(&state.published, &state.published_addresses, &remaining);
                state.published = job_after_payment(&remaining);
                state.hold_closed = false;
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
                    "header_parent": state.canon.header_parent,
                    "tip_height": state.summary.current_tip_block_height,
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

fn keep_addresses(proofs: &[ShareProof], addresses: &[String], kept: &[ShareProof]) -> Vec<String> {
    let wanted: HashSet<&str> = kept.iter().map(|proof| proof.share_id.as_str()).collect();
    proofs
        .iter()
        .zip(addresses)
        .filter(|(proof, _)| wanted.contains(proof.share_id.as_str()))
        .map(|(_, address)| address.clone())
        .collect()
}
