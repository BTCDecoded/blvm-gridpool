//! `Reconcile` on the pinned commit.
//!
//! Paid ids and grouped share ids compare ordinal-ignore-case. Ranking ties break
//! on ordinal (case-sensitive) share id. Difficulty stays a float.

use std::cmp::Ordering;

#[derive(Clone, Debug, PartialEq)]
pub struct ShareProof {
    pub share_id: String,
    pub difficulty: f64,
}

/// Concatenate `known` and `incoming`, drop blank and paid ids, keep the best
/// copy of each share id, then rank and keep `reserve_limit` proofs.
///
/// A limit of 0 keeps nothing. The same paid id twice removes that proof once.
pub fn reconcile(
    known: &[ShareProof],
    incoming: &[ShareProof],
    paid_ids: &[String],
    reserve_limit: usize,
) -> Vec<ShareProof> {
    if reserve_limit == 0 {
        return Vec::new();
    }
    let mut best: Vec<ShareProof> = Vec::new();
    for proof in known.iter().chain(incoming.iter()) {
        if is_blank(&proof.share_id) || is_paid(paid_ids, &proof.share_id) {
            continue;
        }
        if let Some(existing) = best
            .iter_mut()
            .find(|kept| kept.share_id.eq_ignore_ascii_case(&proof.share_id))
        {
            if proof_beats(proof, existing) {
                *existing = proof.clone();
            }
        } else {
            best.push(proof.clone());
        }
    }
    best.sort_by(rank_cmp);
    best.truncate(reserve_limit);
    best
}

fn is_blank(share_id: &str) -> bool {
    share_id.trim().is_empty()
}

fn is_paid(paid_ids: &[String], share_id: &str) -> bool {
    paid_ids
        .iter()
        .any(|paid| !paid.trim().is_empty() && paid.eq_ignore_ascii_case(share_id))
}

/// Higher difficulty wins. Equal difficulty keeps the ordinal-smaller share id.
fn proof_beats(candidate: &ShareProof, current: &ShareProof) -> bool {
    match diff_cmp(candidate.difficulty, current.difficulty) {
        Ordering::Greater => true,
        Ordering::Less => false,
        Ordering::Equal => candidate.share_id.as_str() < current.share_id.as_str(),
    }
}

fn rank_cmp(left: &ShareProof, right: &ShareProof) -> Ordering {
    match diff_cmp(left.difficulty, right.difficulty) {
        Ordering::Equal => left.share_id.as_str().cmp(right.share_id.as_str()),
        other => other.reverse(),
    }
}

fn diff_cmp(left: f64, right: f64) -> Ordering {
    match (left.is_nan(), right.is_nan()) {
        (true, true) => Ordering::Equal,
        (true, false) => Ordering::Less,
        (false, true) => Ordering::Greater,
        (false, false) => left.partial_cmp(&right).unwrap_or(Ordering::Equal),
    }
}
