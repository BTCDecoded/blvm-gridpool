//! Held miner job. There is no clock in this function.
//!
//! The published payout list stays until the caller closes the hold. `canon`
//! is only read. A later sibling updates canon through `reconcile`, not here.
//! The header parent during a hold is the new locally validated block.

use serde::{Deserialize, Serialize};

use crate::reconcile::{reconcile, ShareProof};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Default)]
pub struct Canon {
    pub proofs: Vec<ShareProof>,
    pub header_parent: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct PublishedJob {
    pub proofs: Vec<ShareProof>,
    pub header_parent: String,
}

pub fn published_job(
    hold_closed: bool,
    last_published: &[ShareProof],
    canon: &Canon,
) -> PublishedJob {
    PublishedJob {
        proofs: if hold_closed {
            canon.proofs.clone()
        } else {
            last_published.to_vec()
        },
        header_parent: canon.header_parent.clone(),
    }
}

/// After a GridPool payment, the next job is the remaining reserve.
pub fn job_after_payment(remaining_reserve: &[ShareProof]) -> Vec<ShareProof> {
    remaining_reserve.to_vec()
}

/// A new locally validated block updates the header parent and keeps the unpaid reserve.
pub fn canon_after_local_block(reserve: &[ShareProof], header_parent: &str) -> Canon {
    Canon {
        proofs: reserve.to_vec(),
        header_parent: header_parent.to_string(),
    }
}

/// A found block pays the stored list that was hashed. Unpaid proofs stay in the reserve.
pub fn pay_published_list(
    canon_proofs: &[ShareProof],
    published: &[ShareProof],
    reserve_limit: usize,
) -> Vec<ShareProof> {
    let paid_ids = published
        .iter()
        .map(|proof| proof.share_id.clone())
        .collect::<Vec<_>>();
    reconcile(canon_proofs, &[], &paid_ids, reserve_limit)
}
