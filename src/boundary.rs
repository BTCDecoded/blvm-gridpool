//! Handbook boundary rules that `SnapshotReconciliationTests` does not cover.
//!
//! CONS-001: a new previous-parent proof by direct ingress after the local
//! boundary does not enter. CONS-010: a peer header does not activate a boundary.
//! A context proof stays with the snapshot it committed.

use crate::reconcile::ShareProof;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ActivationSource {
    LocalValidatedBlock,
    PeerHeader,
}

#[derive(Clone, Debug, PartialEq)]
pub struct OpenFamily {
    pub family_id: String,
    pub reserve: Vec<ShareProof>,
}

pub fn activates_boundary(source: ActivationSource) -> bool {
    matches!(source, ActivationSource::LocalValidatedBlock)
}

/// A peer header does not activate a boundary or a consensus-version change.
pub fn peer_header_leaves_boundary_and_version(
    boundary_sealed: bool,
    consensus_version: i32,
) -> (bool, i32) {
    (boundary_sealed, consensus_version)
}

pub fn admit_direct_previous_parent(local_boundary_sealed: bool) -> bool {
    !local_boundary_sealed
}

/// A context proof stays with the snapshot it committed and does not change the open family.
#[derive(Clone, Debug, PartialEq)]
pub struct ContextBinding {
    pub committed_snapshot_id: String,
    pub proof: ShareProof,
}

pub fn bind_context_proof(
    open: &OpenFamily,
    committed_snapshot_id: &str,
    proof: &ShareProof,
) -> (OpenFamily, ContextBinding) {
    (
        open.clone(),
        ContextBinding {
            committed_snapshot_id: committed_snapshot_id.to_string(),
            proof: proof.clone(),
        },
    )
}
