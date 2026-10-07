//! Pure apply decisions: kind specific post images, oracle classification and the removal gate.
//!
//! Nothing here performs I/O. The engine gathers typed facts through the adapter and these functions
//! decide whether an action is complete, whether current state has drifted and whether a destructive
//! effect may run. Equal bytes alone never certify ownership; only attested rows do.
use super::env::{
    DocFacts, DocState, EffectKindView, EffectRow, EventView, GitView, ReceiptView, StatusView,
    TrackingView,
};
use super::record::{Action, ActionKind, SourceObs};

/// Why an action's post image failed verification.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PostFail {
    /// The attested set is incomplete or lacks a required row, so the step stays partial.
    Incomplete(String),
    /// The body, or a body row, changed after the effect, even by another attested operation.
    Drift(String),
    /// The DOC metadata does not match the attested effect or the identity the action requires.
    Metadata(String),
}
impl PostFail {
    /// Stable error code of the failure.
    pub fn code(&self) -> &'static str {
        match self {
            Self::Incomplete(_) => "effect_incomplete",
            Self::Drift(_) => "post_apply_drift",
            Self::Metadata(_) => "metadata_mismatch",
        }
    }
    /// Bounded human explanation.
    pub fn reason(&self) -> &str {
        match self {
            Self::Incomplete(r) | Self::Drift(r) | Self::Metadata(r) => r,
        }
    }
}

/// Current facts a post image is judged against.
#[derive(Clone, Debug)]
pub struct PostFacts {
    /// Observation of the action path (the destination for a Move).
    pub dest: DocFacts,
    /// Observation of the Move source path.
    pub from: Option<DocFacts>,
    /// Observation by DOC identifier of the source document, for a Remove or Move of a managed source.
    pub by_id: Option<DocFacts>,
    /// True when the allocator and DOC identity invariants validate the destination identifier.
    pub allocation_ok: bool,
}

/// Find the receipt row of a path and kind.
fn row<'a>(
    receipt: &'a ReceiptView,
    relative: &str,
    kind: EffectKindView,
) -> Option<&'a EffectRow> {
    receipt
        .rows
        .iter()
        .find(|r| r.relative == relative && r.kind == kind)
}

/// Require a body row for the path: asserted, of the kind, not superseded and with the digest.
fn body_row(
    receipt: &ReceiptView,
    path: &str,
    kind: EffectKindView,
    after: Option<&str>,
) -> Result<(), PostFail> {
    let r = row(receipt, path, kind)
        .ok_or_else(|| PostFail::Incomplete(format!("{path}: no attested body effect.")))?;
    if r.superseded_into.is_some() {
        return Err(PostFail::Drift(format!(
            "{path}: a later publication replaced the attested effect."
        )));
    }
    if r.after.as_deref() != after {
        return Err(PostFail::Drift(format!(
            "{path}: the attested effect carries other bytes."
        )));
    }
    Ok(())
}

/// Require the record row, with an after digest equal to the current record bytes.
fn record_row(
    receipt: &ReceiptView,
    record_path: &str,
    kind: EffectKindView,
    current_sha: &str,
) -> Result<(), PostFail> {
    let r = row(receipt, record_path, kind).ok_or_else(|| {
        PostFail::Incomplete(format!("{record_path}: no attested metadata effect."))
    })?;
    if r.superseded_into.is_some() || r.after.as_deref() != Some(current_sha) {
        return Err(PostFail::Metadata(format!(
            "{record_path}: metadata changed after the attested effect."
        )));
    }
    Ok(())
}

/// Verify the kind specific post image of one action against current facts and its attested receipt.
///
/// The receipt must be `complete`. A body row that was superseded, or a body that differs now, is
/// `Drift` whoever changed it. Metadata that disagrees with the attested record effect or the required
/// identity is `Metadata`. A body without its required metadata is never complete.
pub fn verify_post(
    action: &Action,
    source: Option<&SourceObs>,
    facts: &PostFacts,
    receipt: &ReceiptView,
) -> Result<(), PostFail> {
    if !receipt.complete {
        return Err(PostFail::Incomplete(
            "The attested set is incomplete.".into(),
        ));
    }
    let path = action.path.as_str();
    let staged = action.staged_sha256.as_deref();
    match action.kind {
        ActionKind::Create | ActionKind::Replace => {
            let kind = if action.kind == ActionKind::Create {
                EffectKindView::Created
            } else {
                EffectKindView::Replaced
            };
            body_row(receipt, path, kind, staged)?;
            if facts.dest.body_sha256().as_deref() != staged {
                return Err(PostFail::Drift(format!(
                    "{path}: the body differs from the candidate."
                )));
            }
            let managed_before = source.is_some_and(|s| s.managed);
            if facts.dest.state != DocState::Managed {
                return Err(PostFail::Incomplete(format!(
                    "{path}: the document is not managed yet."
                )));
            }
            let rec = facts.dest.record.as_ref().ok_or_else(|| {
                PostFail::Incomplete(format!("{path}: the DOC record is missing."))
            })?;
            if rec.bound_path != path || Some(rec.body_sha256.as_str()) != staged {
                return Err(PostFail::Metadata(format!(
                    "{path}: the record binds other bytes or a path."
                )));
            }
            if managed_before {
                let s =
                    source.ok_or_else(|| PostFail::Metadata("Missing frozen source.".into()))?;
                if s.doc_id.as_deref() != Some(rec.id.as_str()) {
                    return Err(PostFail::Metadata(format!(
                        "{path}: the DOC identity changed."
                    )));
                }
                if Some(rec.revision) != s.record_revision.map(|r| r + 1) {
                    return Err(PostFail::Metadata(format!(
                        "{path}: unexpected record revision."
                    )));
                }
                record_row(receipt, &rec.path, EffectKindView::Replaced, &rec.sha256)?;
            } else {
                if rec.revision != 1 {
                    return Err(PostFail::Metadata(format!(
                        "{path}: a new record starts at revision one."
                    )));
                }
                if !facts.allocation_ok {
                    return Err(PostFail::Metadata(format!(
                        "{path}: the DOC identifier is not validly allocated."
                    )));
                }
                record_row(receipt, &rec.path, EffectKindView::Created, &rec.sha256)?;
            }
            Ok(())
        }
        ActionKind::Move => {
            let s = source.ok_or_else(|| PostFail::Metadata("Missing frozen source.".into()))?;
            let from = action.from.as_deref().unwrap_or_default();
            body_row(
                receipt,
                path,
                EffectKindView::Created,
                Some(s.sha256.as_str()),
            )?;
            if row(receipt, from, EffectKindView::Removed).is_none() {
                return Err(PostFail::Incomplete(format!(
                    "{from}: no attested source removal."
                )));
            }
            if facts
                .from
                .as_ref()
                .is_none_or(|f| f.state != DocState::Absent)
            {
                return Err(PostFail::Drift(format!(
                    "{from}: the source path is not absent."
                )));
            }
            if facts.dest.body_sha256().as_deref() != Some(s.sha256.as_str()) {
                return Err(PostFail::Drift(format!(
                    "{path}: the moved body differs from the original."
                )));
            }
            if s.managed {
                let rec = facts
                    .dest
                    .record
                    .as_ref()
                    .filter(|_| facts.dest.state == DocState::Managed)
                    .ok_or_else(|| {
                        PostFail::Incomplete(format!("{path}: the moved record is missing."))
                    })?;
                if s.doc_id.as_deref() != Some(rec.id.as_str()) {
                    return Err(PostFail::Metadata(format!(
                        "{path}: the destination does not carry the source DOC id."
                    )));
                }
                if rec.bound_path != path || rec.body_sha256 != s.sha256 {
                    return Err(PostFail::Metadata(format!(
                        "{path}: the record binds another path or bytes."
                    )));
                }
                if Some(rec.revision) != s.record_revision.map(|r| r + 1) {
                    return Err(PostFail::Metadata(format!(
                        "{path}: unexpected record revision."
                    )));
                }
                record_row(receipt, &rec.path, EffectKindView::Replaced, &rec.sha256)?;
            } else if facts.dest.state != DocState::Unmanaged || facts.dest.record.is_some() {
                return Err(PostFail::Metadata(format!(
                    "{path}: an unmanaged move must not gain a record."
                )));
            }
            Ok(())
        }
        ActionKind::Remove => {
            let s = source.ok_or_else(|| PostFail::Metadata("Missing frozen source.".into()))?;
            body_row(receipt, path, EffectKindView::Removed, None)?;
            if facts.dest.body.is_some() {
                return Err(PostFail::Drift(format!("{path}: a body is present again.")));
            }
            if facts.dest.state != DocState::Absent {
                return Err(PostFail::Incomplete(format!(
                    "{path}: an active record still claims the path."
                )));
            }
            if s.managed {
                let id = s.doc_id.as_deref().unwrap_or_default();
                let by_id = facts.by_id.as_ref().ok_or_else(|| {
                    PostFail::Incomplete(format!("{id}: the retired record was not observed."))
                })?;
                let rec = by_id
                    .record
                    .as_ref()
                    .ok_or_else(|| PostFail::Incomplete(format!("{id}: no retired record.")))?;
                if !by_id.retired || by_id.state != DocState::Retired {
                    return Err(PostFail::Incomplete(format!(
                        "{id}: the record is not retired."
                    )));
                }
                if rec.id != id || rec.bound_path != path || rec.body_sha256 != s.sha256 {
                    return Err(PostFail::Metadata(format!(
                        "{id}: the retired record disagrees with the original."
                    )));
                }
                record_row(receipt, &rec.path, EffectKindView::Replaced, &rec.sha256)?;
            } else if facts.by_id.as_ref().is_some_and(|f| f.record.is_some()) {
                return Err(PostFail::Metadata(format!(
                    "{path}: an unmanaged remove must not gain a record."
                )));
            }
            Ok(())
        }
    }
}

/// What the oracle says about one action, reduced to the engine's decision classes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Class {
    /// Nothing attested and nothing equal to the intended result.
    NotPublished,
    /// Every assertion attested.
    Attested(ReceiptView),
    /// Some assertions attested; the listed paths were never published.
    Partial(ReceiptView, Vec<String>),
    /// Unproven, conflicting or truncated evidence: blocks, with the attested part for display.
    Unknown(String),
    /// A recorded operation conflict: blocks.
    Foreign(String),
}

/// Classify one oracle answer.
pub fn classify(status: StatusView) -> Class {
    match status {
        StatusView::NotPublished => Class::NotPublished,
        StatusView::Attested(r) => Class::Attested(r),
        StatusView::Partial { attested, missing } => Class::Partial(attested, missing),
        StatusView::Unknown { reason, .. } => Class::Unknown(reason),
        StatusView::Foreign { reason } => Class::Foreign(reason),
    }
}

/// Where one replacement's effects live relative to the current call.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Replacement {
    /// Published by an earlier call; `committed` is true only when every effect row is committed in a
    /// reachable commit and its final bytes were proven there; `intents` names the pending ones.
    Older {
        /// All effect rows proven committed.
        committed: bool,
        /// Intents still held, pending or unknown.
        intents: Vec<String>,
    },
    /// Published by this same handler call and a member of its whole intent.
    Current,
}

/// A removal blocked because a replacement is not durable.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GateBlock {
    /// Actions whose replacement is not committed.
    pub actions: Vec<String>,
    /// Intents to retry explicitly, deduplicated.
    pub intents: Vec<String>,
}
impl GateBlock {
    /// Bounded message naming the actions and the explicit recovery.
    pub fn message(&self) -> String {
        let intents = if self.intents.is_empty() {
            "none listed".to_owned()
        } else {
            self.intents.join(", ")
        };
        format!(
            "Replacements of {} are not committed (intents: {intents}). Run git_recovery Retry for them, then apply again.",
            self.actions.join(", ")
        )
    }
}

/// Whether the current call's whole intent is eligible to commit atomically.
///
/// Directory, ignored backup, lock and temp effects are `NotApplicable`: they never count as tracked or
/// untracked. A durability uncertainty on any event, including a necessary directory, still makes the
/// call sync uncertain and therefore not eligible.
pub fn current_eligible(events: &[EventView]) -> bool {
    events
        .iter()
        .all(|e| e.durable && !matches!(e.tracking, TrackingView::Untracked))
}

/// Decide whether a destructive effect may run.
///
/// Every Create, Replace and Move replacement of the proposal must be committed from an older intent or
/// be published by the current call whose whole intent is eligible. Anything else blocks before the
/// removal with the pending intents named; a fresh call whose own effects form the current intent is
/// never blocked by its own uncommitted state.
pub fn removal_gate(
    replacements: &[(String, Replacement)],
    current_ok: bool,
) -> Result<(), GateBlock> {
    let mut actions = Vec::new();
    let mut intents = Vec::new();
    for (id, r) in replacements {
        match r {
            Replacement::Older {
                committed: true, ..
            } => {}
            Replacement::Older { intents: held, .. } => {
                actions.push(id.clone());
                intents.extend(held.iter().cloned());
            }
            Replacement::Current if current_ok => {}
            Replacement::Current => actions.push(id.clone()),
        }
    }
    if actions.is_empty() {
        return Ok(());
    }
    intents.sort();
    intents.dedup();
    Err(GateBlock { actions, intents })
}

/// Intents of rows that are not committed (held, pending or unknown), for the explicit recovery.
pub fn uncommitted_intents(receipt: &ReceiptView) -> Vec<String> {
    let mut v: Vec<String> = receipt
        .rows
        .iter()
        .filter_map(|r| match &r.git {
            GitView::Pending(i) | GitView::Held(i) | GitView::Unknown(i) => Some(i.clone()),
            _ => None,
        })
        .collect();
    v.sort();
    v.dedup();
    v
}

/// The commits that hold the final image of every non-removal row, or `None` when any row is not
/// committed. Rows superseded into later bytes are judged by their committed final image elsewhere.
pub fn committed_commits(receipt: &ReceiptView) -> Option<Vec<(String, String, Option<String>)>> {
    let mut out = Vec::new();
    for r in &receipt.rows {
        match &r.git {
            GitView::Committed(commit) => {
                let digest = r
                    .superseded_into
                    .clone()
                    .filter(|d| d != "-")
                    .or_else(|| r.after.clone());
                out.push((r.relative.clone(), commit.clone(), digest));
            }
            _ => return None,
        }
    }
    Some(out)
}
