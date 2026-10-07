//! Independent review: the persistent pinned reviewer, exact item verdicts, immutable findings and
//! loss-only reviewer replacement.
//!
//! Every function mutates a decoded [`CpRecord`] only after all checks passed, so a refusal never leaves
//! a half-changed record. Declared actor ids are observations, not authentication.
use super::record::{
    self, Acceptance, CpRecord, CpState, Disposition, Immersion, LostReport, MAX_FINDINGS,
    MAX_REVIEWS, REASON_CAP, ReviewRecord, ReviewerBinding,
};
use crate::{
    model::{self, Finding, FindingResolution, Verdict},
    store::{Error, Result},
};
use std::collections::BTreeSet;

/// One review as submitted by the independent reviewer.
#[derive(Clone, Debug)]
pub struct ReviewIn {
    /// Revision reviewed.
    pub revision: u32,
    /// Revision hash reviewed.
    pub content_hash: String,
    /// Verdict.
    pub verdict: Verdict,
    /// Summary, at most 512 bytes.
    pub summary: String,
    /// Every item the reviewer verified.
    pub verified_items: Vec<String>,
    /// Findings of this review.
    pub findings: Vec<Finding>,
    /// Resolutions of earlier findings.
    pub resolved: Vec<FindingResolution>,
}

/// Build a stable-code refusal.
fn refuse(code: &'static str, message: impl Into<String>) -> Error {
    Error::new(code, message)
}

/// The exact item set a reviewer must verify for acceptance: every preservation id, every action id and
/// every section id whose disposition is not `Kept`.
pub fn review_items(record: &CpRecord) -> Result<BTreeSet<String>> {
    let body = &record.revision()?.body;
    let mut items = BTreeSet::new();
    items.extend(body.preservation.iter().map(|p| p.id.clone()));
    items.extend(body.actions.iter().map(|a| a.id.clone()));
    items.extend(
        body.sections
            .iter()
            .filter(|s| !matches!(s.disposition, Disposition::Kept { .. }))
            .map(|s| s.id.clone()),
    );
    Ok(items)
}

/// Digest of a verified item set.
pub fn items_hash(items: &BTreeSet<String>) -> String {
    let parts: Vec<&[u8]> = items.iter().map(|i| i.as_bytes()).collect();
    record::digest_parts("agent-tasks/cp-items/v1", &parts)
}

/// Every `(review_index, finding_index)` already resolved by any review.
fn resolved_set(record: &CpRecord, extra: &[FindingResolution]) -> BTreeSet<(usize, usize)> {
    record
        .reviews
        .iter()
        .flat_map(|r| r.resolved.iter())
        .chain(extra.iter())
        .map(|r| (r.review_index, r.finding_index))
        .collect()
}

/// Validate one review against the record without changing it.
///
/// Returns the verified item set digest for an acceptance. Codes: `not_reviewable`, `stale_revision`,
/// `self_review`, `reviewer_pinned`, `reviewer_gap`, `review_incomplete`, `capacity`, `invalid_arguments`.
pub fn check_review(record: &CpRecord, actor: &str, input: &ReviewIn) -> Result<Option<String>> {
    if !matches!(record.state, CpState::Proposed | CpState::ChangesRequested) {
        return Err(refuse(
            "not_reviewable",
            "Only a proposed or changes requested revision can be reviewed.",
        ));
    }
    let current = record.revision()?;
    if input.revision != record.current || input.content_hash != current.content_hash {
        return Err(refuse(
            "stale_revision",
            "The review names a revision or hash that is not current.",
        ));
    }
    if record.authors().contains(&actor) {
        return Err(refuse(
            "self_review",
            "An author of any revision cannot review it.",
        ));
    }
    if let Some(binding) = &record.reviewer {
        if binding.lost.is_some() {
            return Err(refuse(
                "reviewer_gap",
                "The reviewer was reported lost; a replacement must immerse first.",
            ));
        }
        if binding.agent_id != actor {
            return Err(refuse(
                "reviewer_pinned",
                "Only the pinned reviewer may review this proposal.",
            ));
        }
        if binding
            .immersion
            .as_ref()
            .is_some_and(|i| !i.gaps.is_empty())
        {
            return Err(refuse(
                "reviewer_gap",
                "Replacement context restoration reports gaps that block review.",
            ));
        }
    }
    model::text(&input.summary, REASON_CAP)
        .map_err(|e| Error::new("invalid_arguments", format!("summary: {e}")))?;
    if record.reviews.len() >= MAX_REVIEWS {
        return Err(refuse("capacity", "At most 16 reviews are retained."));
    }
    let existing: usize = record.reviews.iter().map(|r| r.findings.len()).sum();
    if existing + input.findings.len() > MAX_FINDINGS {
        return Err(refuse("capacity", "At most 64 findings are retained."));
    }
    for f in &input.findings {
        model::text(&f.text, 256)
            .map_err(|e| Error::new("invalid_arguments", format!("findings: {e}")))?;
    }
    let mut seen = BTreeSet::new();
    for r in &input.resolved {
        model::text(&r.summary, REASON_CAP)
            .map_err(|e| Error::new("invalid_arguments", format!("resolved_findings: {e}")))?;
        let exists = record
            .reviews
            .get(r.review_index)
            .is_some_and(|rev| r.finding_index < rev.findings.len());
        if !exists || !seen.insert((r.review_index, r.finding_index)) {
            return Err(refuse(
                "invalid_arguments",
                "resolved_findings: unknown or repeated finding address.",
            ));
        }
        if resolved_set(record, &[]).contains(&(r.review_index, r.finding_index)) {
            return Err(refuse(
                "invalid_arguments",
                "resolved_findings: finding already resolved.",
            ));
        }
    }
    match input.verdict {
        Verdict::ChangesRequested => {
            if input.findings.is_empty() {
                return Err(refuse(
                    "review_incomplete",
                    "Changes requested needs at least one finding.",
                ));
            }
            Ok(None)
        }
        Verdict::Accepted => {
            let expected = review_items(record)?;
            let given: BTreeSet<String> = input.verified_items.iter().cloned().collect();
            if given.len() != input.verified_items.len() || given != expected {
                return Err(refuse(
                    "review_incomplete",
                    "Acceptance needs a verdict for exactly every item.",
                ));
            }
            if input.findings.iter().any(|f| f.must_fix) {
                return Err(refuse(
                    "review_incomplete",
                    "Acceptance cannot carry a must fix finding.",
                ));
            }
            let resolved = resolved_set(record, &input.resolved);
            for (ri, rev) in record.reviews.iter().enumerate() {
                for (fi, f) in rev.findings.iter().enumerate() {
                    if f.must_fix && !resolved.contains(&(ri, fi)) {
                        return Err(refuse(
                            "review_incomplete",
                            "An earlier must fix finding is unresolved.",
                        ));
                    }
                }
            }
            Ok(Some(items_hash(&expected)))
        }
    }
}

/// Append a checked review, pin the reviewer on first use and set the record state.
///
/// `acceptance` carries the freshly computed digests for an accepting review and must be `Some` exactly
/// then. The review is immutable once appended; revise clears only the pointer, never the review.
pub fn append_review(
    record: &mut CpRecord,
    actor: &str,
    input: &ReviewIn,
    items_hash: Option<String>,
    acceptance: Option<Acceptance>,
    at: &str,
) -> Result<()> {
    if record.reviewer.is_none() {
        record.reviewer = Some(ReviewerBinding {
            agent_id: actor.to_owned(),
            pinned_at: at.to_owned(),
            predecessors: Vec::new(),
            lost: None,
            immersion: None,
        });
    }
    record.reviews.push(ReviewRecord {
        revision: input.revision,
        content_hash: input.content_hash.clone(),
        reviewer: actor.to_owned(),
        verdict: input.verdict,
        summary: input.summary.clone(),
        items_hash,
        findings: input.findings.clone(),
        resolved: input.resolved.clone(),
        acceptance,
        at: at.to_owned(),
    });
    match input.verdict {
        Verdict::Accepted => {
            record.state = CpState::Accepted;
            record.accepted_review = Some(record.reviews.len() - 1);
        }
        Verdict::ChangesRequested => {
            record.state = CpState::ChangesRequested;
            record.accepted_review = None;
        }
    }
    Ok(())
}

/// Report the pinned reviewer lost and unable to continue.
///
/// Requires `lost` and `unrecoverable` both true and an observation of inability to continue or resume;
/// a timeout or preference never qualifies. Only the reporter differs from the lost reviewer.
pub fn report_lost(
    record: &mut CpRecord,
    actor: &str,
    lost: Option<bool>,
    unrecoverable: Option<bool>,
    observation: Option<&str>,
    at: &str,
) -> Result<()> {
    if record.state.terminal() {
        return Err(refuse(
            "not_reviewable",
            "A finished proposal has no reviewer to replace.",
        ));
    }
    let binding = record.reviewer.as_mut().ok_or_else(|| {
        refuse(
            "reviewer_gap",
            "No reviewer is pinned, so none can be lost.",
        )
    })?;
    if binding.lost.is_some() {
        return Err(refuse(
            "reviewer_gap",
            "The reviewer is already reported lost.",
        ));
    }
    if lost != Some(true) || unrecoverable != Some(true) {
        return Err(refuse(
            "invalid_arguments",
            "lost and unrecoverable must both be true after observed loss.",
        ));
    }
    let observation =
        observation.ok_or_else(|| Error::new("invalid_arguments", "observation: required"))?;
    model::text(observation, REASON_CAP)
        .map_err(|e| Error::new("invalid_arguments", format!("observation: {e}")))?;
    if binding.agent_id == actor {
        return Err(refuse(
            "self_review",
            "The lost reviewer cannot report its own loss.",
        ));
    }
    binding.lost = Some(LostReport {
        agent_id: binding.agent_id.clone(),
        reported_by: actor.to_owned(),
        observation: observation.to_owned(),
        at: at.to_owned(),
    });
    Ok(())
}

/// Immerse a replacement reviewer after reported loss, or refresh its own gap report.
///
/// The replacement must be neither an author nor a predecessor. Any gap blocks review until a later
/// immersion by the same replacement reports none. Predecessors, reviews and findings remain.
pub fn immerse(
    record: &mut CpRecord,
    actor: &str,
    understanding: Option<&str>,
    sources: &[String],
    unfinished: &[String],
    gaps: &[String],
    at: &str,
) -> Result<()> {
    if record.state.terminal() {
        return Err(refuse(
            "not_reviewable",
            "A finished proposal has no reviewer to replace.",
        ));
    }
    let authors: Vec<String> = record.authors().iter().map(|a| (*a).to_owned()).collect();
    let binding = record
        .reviewer
        .as_mut()
        .ok_or_else(|| refuse("reviewer_gap", "No reviewer is pinned."))?;
    if authors.iter().any(|a| a == actor) {
        return Err(refuse(
            "self_review",
            "An author cannot become the reviewer.",
        ));
    }
    let understanding =
        understanding.ok_or_else(|| Error::new("invalid_arguments", "understanding: required"))?;
    model::text(understanding, REASON_CAP)
        .map_err(|e| Error::new("invalid_arguments", format!("understanding: {e}")))?;
    model::strings(sources, 256, false)
        .map_err(|e| Error::new("invalid_arguments", format!("sources: {e}")))?;
    model::strings(unfinished, 256, false)
        .map_err(|e| Error::new("invalid_arguments", format!("unfinished: {e}")))?;
    model::strings(gaps, 256, false)
        .map_err(|e| Error::new("invalid_arguments", format!("gaps: {e}")))?;
    let immersion = Immersion {
        agent_id: actor.to_owned(),
        understanding: understanding.to_owned(),
        sources: sources.to_vec(),
        unfinished: unfinished.to_vec(),
        gaps: gaps.to_vec(),
        at: at.to_owned(),
    };
    if let Some(lost) = binding.lost.take() {
        if lost.agent_id == actor || binding.predecessors.iter().any(|p| p.agent_id == actor) {
            binding.lost = Some(lost);
            return Err(refuse(
                "self_review",
                "A lost or earlier reviewer cannot replace itself.",
            ));
        }
        binding.predecessors.push(lost);
        binding.agent_id = actor.to_owned();
        binding.immersion = Some(immersion);
        return Ok(());
    }
    if binding.agent_id == actor && binding.immersion.is_some() {
        binding.immersion = Some(immersion);
        return Ok(());
    }
    Err(refuse(
        "reviewer_gap",
        "No loss was reported, so no replacement may immerse.",
    ))
}
