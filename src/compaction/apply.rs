//! Guarded, interruption-safe application of an accepted proposal.
//!
//! Order is Create, Move, Replace, a fresh coverage verification, then Remove. Every action carries a
//! deterministic operation identity, and recovery asks the complete operation oracle first: an
//! attested effect is adopted only after its kind specific post image verifies, equal bytes never
//! certify ownership, later external or other-operation changes block the proposal and no removal of
//! any kind runs until every replacement is committed or belongs to the current whole atomic intent.
use super::{
    env::{
        DocOp, DocState, EffectKindView, EffectRow, Env, Expected, GitView, LocatorView,
        OriginalItem, ReceiptView,
    },
    gate::{self, Class, PostFacts, PostFail, Replacement},
    inventory,
    ops::{self, Outcome},
    record::{
        self, Action, ActionKind, ActionState, ApplyPhase, ApplyProgress, BlockedInfo, CpRecord,
        CpState, PubCopy, SourceObs, StoredOriginal,
    },
    validate,
};
use crate::store::{self, Error, Result};
use std::collections::{BTreeMap, BTreeSet};

/// Build a stable-code refusal.
fn refuse(code: &'static str, message: impl Into<String>) -> Error {
    Error::new(code, message)
}

/// Assertions about the computable body effects of one action, never about generated metadata.
pub fn expected_effects(action: &Action, source: Option<&SourceObs>) -> Vec<Expected> {
    match action.kind {
        ActionKind::Create => vec![Expected {
            relative: action.path.clone(),
            kind: EffectKindView::Created,
            after: action.staged_sha256.clone(),
        }],
        ActionKind::Replace => vec![Expected {
            relative: action.path.clone(),
            kind: EffectKindView::Replaced,
            after: action.staged_sha256.clone(),
        }],
        ActionKind::Move => vec![
            Expected {
                relative: action.path.clone(),
                kind: EffectKindView::Created,
                after: source.map(|s| s.sha256.clone()),
            },
            Expected {
                relative: action.from.clone().unwrap_or_default(),
                kind: EffectKindView::Removed,
                after: None,
            },
        ],
        ActionKind::Remove => vec![Expected {
            relative: action.path.clone(),
            kind: EffectKindView::Removed,
            after: None,
        }],
    }
}

/// The frozen source an action works on: its own path, or the Move source.
fn source_of<'a>(body: &'a record::ProposalBody, a: &Action) -> Option<&'a SourceObs> {
    let path = if a.kind == ActionKind::Move {
        a.from.as_deref()?
    } else {
        a.path.as_str()
    };
    body.sources.iter().find(|s| s.path == path)
}

/// Lowercase label of an effect kind for the stored publication copies.
fn kind_label(kind: EffectKindView) -> &'static str {
    match kind {
        EffectKindView::Created => "created",
        EffectKindView::Replaced => "replaced",
        EffectKindView::Removed => "removed",
    }
}

/// Convert attested effect rows into the convenience publication copies kept on an action; the provenance journal stays authoritative.
fn copies(operation: &str, rows: &[EffectRow]) -> Vec<PubCopy> {
    rows.iter()
        .map(|r| PubCopy {
            relative: r.relative.clone(),
            kind: kind_label(r.kind).to_owned(),
            before: r.before.clone(),
            after: r.after.clone(),
            operation: Some(operation.to_owned()),
            intent: match &r.git {
                GitView::Pending(i) | GitView::Held(i) | GitView::Unknown(i) => Some(i.clone()),
                _ => None,
            },
        })
        .collect()
}

/// Gather the current facts a post image is judged against.
fn post_facts(env: &dyn Env, body: &record::ProposalBody, a: &Action) -> Result<PostFacts> {
    let dest = env.observe(&a.path)?;
    let from = match (&a.kind, &a.from) {
        (ActionKind::Move, Some(f)) => Some(env.observe(f)?),
        _ => None,
    };
    let source = source_of(body, a);
    let by_id = match (a.kind, source.and_then(|s| s.doc_id.as_deref())) {
        (ActionKind::Remove, Some(id)) => Some(env.observe_id(id)?),
        _ => None,
    };
    let allocation_ok = match &dest.record {
        Some(r) if matches!(a.kind, ActionKind::Create | ActionKind::Replace) => {
            env.allocation_valid(&r.id)?
        }
        _ => true,
    };
    Ok(PostFacts {
        dest,
        from,
        by_id,
        allocation_ok,
    })
}

/// One apply run: the mutable proposal, the classified actions and the current call's bookkeeping.
struct Run<'a> {
    env: &'a dyn Env,
    actor: &'a str,
    rec: CpRecord,
    observed: Vec<u8>,
    body: record::ProposalBody,
    blobs: Vec<(String, Vec<u8>)>,
    effects: &'a mut Vec<String>,
    classes: BTreeMap<String, Class>,
    older: Vec<(String, Replacement)>,
    executed: BTreeSet<String>,
    expected_incoming: String,
}

impl Run<'_> {
    /// Deterministic operation identity of one action in the current revision.
    fn op(&self, a: &Action) -> String {
        record::action_operation(&self.rec.id, self.rec.current, &a.id)
    }

    /// Persist the current action states and a stop condition, then build the error to return.
    ///
    /// A record is written only when this call already published something; a refusal before any effect
    /// leaves the record untouched and settles nothing.
    fn stop(
        &mut self,
        code: &'static str,
        reason: impl Into<String>,
        action: Option<&str>,
    ) -> Error {
        let reason = store::safe(&reason.into(), 400);
        if !self.env.call_events().is_empty() {
            if let Some(id) = action
                && let Some(a) = self
                    .body
                    .actions
                    .iter_mut()
                    .find(|a| a.id == id && a.state == ActionState::Pending)
            {
                a.state = ActionState::Blocked;
            }
            if let Ok(rev) = self.rec.revision_mut() {
                rev.body.actions = self.body.actions.clone();
            }
            self.rec.state = CpState::Blocked;
            if let Some(progress) = self.rec.apply.as_mut() {
                progress.blocked = Some(BlockedInfo {
                    kind: code.to_owned(),
                    action: action.map(str::to_owned),
                    reason: reason.clone(),
                });
            }
            let observed = self.observed.clone();
            if let Err(e) =
                ops::save_record(self.env, &mut self.rec, Some(&observed), true, self.effects)
            {
                return e;
            }
            self.effects.push(format!(
                "Blocked {code} at {}.",
                action.unwrap_or("proposal")
            ));
        }
        Error::new(code, reason)
    }

    /// Verify one action's post image against its receipt and current facts.
    fn verify(&self, a: &Action, receipt: &ReceiptView) -> std::result::Result<(), PostFail> {
        let facts = post_facts(self.env, &self.body, a).map_err(|e| {
            PostFail::Incomplete(format!("Cannot observe current state: {}", e.message))
        })?;
        gate::verify_post(a, source_of(&self.body, a), &facts, receipt)
    }

    /// Ask the oracle about one action.
    fn status(&self, a: &Action) -> Class {
        let expected = expected_effects(a, source_of(&self.body, a));
        gate::classify(self.env.effect_status(&self.op(a), &expected))
    }

    /// Mark one action applied and keep copies of its attested effects.
    fn set_applied(&mut self, index: usize, receipt: &ReceiptView) {
        let op = self.op(&self.body.actions[index].clone());
        let action = &mut self.body.actions[index];
        action.state = ActionState::Applied;
        action.publications = copies(&op, &receipt.rows);
    }

    /// Decide whether a destructive effect may run now.
    fn removal_barrier(&mut self, action: &str) -> Result<()> {
        let mut replacements = self.older.clone();
        replacements.extend(
            self.executed
                .iter()
                .map(|id| (id.clone(), Replacement::Current)),
        );
        let current_ok = gate::current_eligible(&self.env.call_events());
        gate::removal_gate(&replacements, current_ok)
            .map_err(|block| self.stop("replacements_not_committed", block.message(), Some(action)))
    }

    /// The document operation that carries one action forward.
    fn doc_op(&self, a: &Action) -> Result<DocOp> {
        let source = source_of(&self.body, a);
        let blob = || {
            self.blobs
                .iter()
                .find(|(id, _)| id == &a.id)
                .map(|(_, b)| b.clone())
                .ok_or_else(|| refuse("invalid_data", "Staged candidate is missing."))
        };
        let base = a.base_version.clone().unwrap_or_default();
        Ok(match a.kind {
            ActionKind::Create | ActionKind::Replace => DocOp::Save {
                path: a.path.clone(),
                body: blob()?,
                purpose: a.purpose.clone(),
                expected: base,
            },
            ActionKind::Move => DocOp::Relocate {
                from: a.from.clone().unwrap_or_default(),
                to: a.path.clone(),
                expected: source.map(|s| s.version.clone()).unwrap_or_default(),
                expected_to: base,
                basis: source.and_then(|s| s.move_basis.clone()).ok_or_else(|| {
                    refuse("invalid_data", "The original move basis was not captured.")
                })?,
            },
            ActionKind::Remove => DocOp::Remove {
                path: a.path.clone(),
                expected: base,
            },
        })
    }

    /// Run one document operation, then re-ask the oracle and require the full post image.
    fn carry(&mut self, index: usize, op: &DocOp) -> Result<()> {
        let a = self.body.actions[index].clone();
        let operation = self.op(&a);
        if matches!(a.kind, ActionKind::Move | ActionKind::Remove) {
            self.removal_barrier(&a.id)?;
        }
        if let Err(e) = self.env.doc_op(&operation, self.actor, op, self.effects) {
            return Err(self.stop(e.code, e.message, Some(&a.id)));
        }
        self.executed.insert(a.id.clone());
        match self.status(&a) {
            Class::Attested(receipt) => match self.verify(&a, &receipt) {
                Ok(()) => {
                    self.set_applied(index, &receipt);
                    Ok(())
                }
                Err(f) => Err(self.stop(f.code(), f.reason().to_owned(), Some(&a.id))),
            },
            _ => Err(self.stop(
                "effect_incomplete",
                "The operation was not fully attested after it ran.",
                Some(&a.id),
            )),
        }
    }

    /// Check the original preimages of an action the oracle reports not published.
    fn preimage(&mut self, a: &Action) -> Result<()> {
        let stale = |this: &mut Self, p: &str| {
            this.stop(
                "stale_source",
                format!("{p}: changed since the proposal."),
                Some(&a.id),
            )
        };
        let source = source_of(&self.body, a).cloned();
        match a.kind {
            ActionKind::Create => {
                let d = self.env.observe(&a.path)?;
                if Some(&d.version) != a.base_version.as_ref() {
                    return Err(stale(self, &a.path));
                }
            }
            ActionKind::Replace | ActionKind::Remove => {
                let d = self.env.observe(&a.path)?;
                let same = Some(&d.version) == a.base_version.as_ref()
                    && source
                        .as_ref()
                        .is_some_and(|s| d.body_sha256().as_deref() == Some(s.sha256.as_str()));
                if !same {
                    return Err(stale(self, &a.path));
                }
            }
            ActionKind::Move => {
                let from = a.from.clone().unwrap_or_default();
                let f = self.env.observe(&from)?;
                if source.as_ref().is_none_or(|s| s.version != f.version) {
                    return Err(stale(self, &from));
                }
                let t = self.env.observe(&a.path)?;
                if Some(&t.version) != a.base_version.as_ref() {
                    return Err(stale(self, &a.path));
                }
            }
        }
        Ok(())
    }

    /// Drive one pending action to its verified post image or stop.
    fn drive(&mut self, index: usize) -> Result<()> {
        let a = self.body.actions[index].clone();
        let class = self
            .classes
            .get(&a.id)
            .cloned()
            .unwrap_or(Class::NotPublished);
        match class {
            Class::Unknown(r) => Err(self.stop("effect_unknown", r, Some(&a.id))),
            Class::Foreign(r) => Err(self.stop("effect_foreign", r, Some(&a.id))),
            Class::NotPublished => {
                self.preimage(&a)?;
                let op = self.doc_op(&a)?;
                self.carry(index, &op)
            }
            Class::Attested(receipt) | Class::Partial(receipt, _) => {
                match self.verify(&a, &receipt) {
                    Ok(()) => {
                        self.set_applied(index, &receipt);
                        Ok(())
                    }
                    Err(PostFail::Incomplete(reason)) => {
                        self.roll_forward(index, &receipt, &reason)
                    }
                    Err(f) => Err(self.stop(f.code(), f.reason().to_owned(), Some(&a.id))),
                }
            }
        }
    }

    /// Roll a partially published action forward through the document owner's documented recovery step,
    /// only when the oracle attested the body for this operation.
    fn roll_forward(&mut self, index: usize, receipt: &ReceiptView, why: &str) -> Result<()> {
        let a = self.body.actions[index].clone();
        let body_attested = receipt
            .rows
            .iter()
            .any(|r| r.relative == a.path && r.asserted);
        let op = match a.kind {
            ActionKind::Create | ActionKind::Replace => {
                if !body_attested {
                    return Err(self.stop("effect_incomplete", why.to_owned(), Some(&a.id)));
                }
                let current = self.env.observe(&a.path)?;
                if current.state == DocState::Managed {
                    return Err(self.stop("metadata_mismatch", why.to_owned(), Some(&a.id)));
                }
                DocOp::Adopt {
                    path: a.path.clone(),
                    purpose: a.purpose.clone(),
                    expected: current.version,
                }
            }
            ActionKind::Move => self.doc_op(&a)?,
            ActionKind::Remove => {
                let current = self.env.observe(&a.path)?;
                DocOp::Remove {
                    path: a.path.clone(),
                    expected: current.version,
                }
            }
        };
        self.carry(index, &op)
    }

    /// Whether the receipt rows of an earlier call are all committed, with body rows proven at their commit.
    fn older_state(&self, a: &Action, receipt: &ReceiptView) -> Replacement {
        let intents = gate::uncommitted_intents(receipt);
        let Some(rows) = gate::committed_commits(receipt) else {
            return Replacement::Older {
                committed: false,
                intents,
            };
        };
        let source = source_of(&self.body, a);
        let items: Vec<OriginalItem> = rows
            .iter()
            .filter_map(|(relative, commit, digest)| {
                let digest = digest.clone()?;
                let len = if *relative == a.path {
                    a.staged_len.or_else(|| source.map(|s| s.len))?
                } else {
                    return None;
                };
                Some(OriginalItem {
                    relative: relative.clone(),
                    sha256: digest,
                    len,
                    at: Some(commit.clone()),
                })
            })
            .collect();
        let proven = items.is_empty() || self.env.verify_committed(&items).is_ok();
        Replacement::Older {
            committed: proven,
            intents,
        }
    }
}

/// Items proving every changed original committed: body and, when managed, its DOC record.
fn original_items(body: &record::ProposalBody) -> Vec<OriginalItem> {
    let mut items = Vec::new();
    for s in &body.sources {
        items.push(OriginalItem {
            relative: s.path.clone(),
            sha256: s.sha256.clone(),
            len: s.len,
            at: None,
        });
        if let (Some(p), Some(h), Some(l)) = (&s.record_path, &s.record_sha256, s.record_len) {
            items.push(OriginalItem {
                relative: p.clone(),
                sha256: h.clone(),
                len: l,
                at: None,
            });
        }
    }
    items
}

/// Match proven committed locators to the frozen original identities they prove.
fn stored(items: &[OriginalItem], located: Vec<LocatorView>) -> Vec<StoredOriginal> {
    located
        .into_iter()
        .filter_map(|l| {
            let item = items.iter().find(|i| i.relative == l.relative)?;
            Some(StoredOriginal {
                relative: l.relative,
                sha256: item.sha256.clone(),
                len: item.len,
                locator: l.locator,
                commit: l.commit,
            })
        })
        .collect()
}

/// Read-only report for an already applied proposal: honest current drift, never a verified claim.
fn applied_repeat(env: &dyn Env, rec: &CpRecord, version: String) -> Result<Outcome> {
    let revision = rec.revision()?;
    let mut out = Outcome::of(rec, version, false);
    let mut drift = Vec::new();
    for a in &revision.body.actions {
        let expected = expected_effects(a, source_of(&revision.body, a));
        let op = record::action_operation(&rec.id, rec.current, &a.id);
        let ok = match gate::classify(env.effect_status(&op, &expected)) {
            Class::Attested(receipt) => post_facts(env, &revision.body, a).ok().is_some_and(|f| {
                gate::verify_post(a, source_of(&revision.body, a), &f, &receipt).is_ok()
            }),
            _ => false,
        };
        if !ok {
            drift.push(a.path.clone());
        }
    }
    if drift.is_empty() {
        out.notes
            .push("Applied historically; no drift observed at this read.".into());
    } else {
        out.notes.push(store::safe(
            &format!("post_apply_drift observed at: {}", drift.join(", ")),
            190,
        ));
    }
    out.notes.truncate(8);
    Ok(out)
}

/// Apply an accepted proposal, or resume an interrupted or blocked application.
///
/// Returns `Ok` only when the proposal ended `Applied` or was already applied. Any block after a
/// publication returns `Err` with a stable code after recording the stop, so the dispatcher settles the
/// call as partial and the production policy holds it. A refusal before any effect changes nothing.
pub fn apply(
    env: &dyn Env,
    actor: &str,
    version: &str,
    cp: &str,
    effects: &mut Vec<String>,
) -> Result<Outcome> {
    ops::require_actor(actor)?;
    let loaded = ops::load(env, cp, version)?;
    let rec = loaded.value;
    match rec.state {
        CpState::Applied => return applied_repeat(env, &rec, loaded.version),
        CpState::Accepted | CpState::Applying | CpState::Blocked => {}
        CpState::Withdrawn | CpState::AbandonedPartial => {
            return Err(refuse("already_withdrawn", "The proposal was withdrawn."));
        }
        CpState::Proposed | CpState::ChangesRequested => {
            return Err(refuse(
                "not_accepted",
                "Only an accepted revision can be applied.",
            ));
        }
    }
    let review = rec.acceptance_current().ok_or_else(|| {
        refuse(
            "not_accepted",
            "The accepted review no longer binds the current revision.",
        )
    })?;
    let acceptance = review
        .acceptance
        .clone()
        .ok_or_else(|| refuse("not_accepted", "The accepting review carries no digests."))?;
    let body = rec.revision()?.body.clone();
    let blobs = ops::load_blobs(env, &rec)?;
    let first = rec.state == CpState::Accepted;

    // Claims and pending provenance.
    let scan = inventory::scan(env.store())?;
    if !scan.complete {
        return Err(refuse(
            "inventory",
            "The compaction home has unreadable or foreign entries.",
        ));
    }
    let touched = validate::touched_paths(&body);
    for other in &scan.records {
        let r = &other.value;
        if r.id != rec.id
            && !r.state.terminal()
            && let Some(path) = r
                .revision()
                .ok()
                .map(|x| validate::touched_paths(&x.body))
                .and_then(|t| t.intersection(&touched).next().cloned())
        {
            return Err(refuse("path_claimed", format!("{path}: held by {}.", r.id)));
        }
    }
    for p in env.pending() {
        let risky = matches!(p.phase.as_str(), "unknown" | "drifted");
        if risky && p.paths.iter().any(|x| touched.contains(x)) {
            return Err(refuse(
                "effect_unknown",
                format!(
                    "Pending intent {} is {} on a touched path.",
                    p.intent, p.phase
                ),
            ));
        }
    }
    if first {
        let current = validate::current_source_lines(env, &body)?;
        if current != validate::frozen_source_lines(&body)
            || validate::sources_digest(&current) != acceptance.sources_digest
        {
            return Err(refuse(
                "stale_source",
                "A source or destination observation changed since acceptance.",
            ));
        }
        if validate::overlay_digest(&body) != acceptance.overlay_digest {
            return Err(refuse(
                "not_accepted",
                "The overlay no longer matches the accepted digest.",
            ));
        }
    }
    let cov = validate::coverage(env, &body, &blobs, !first)?;
    if cov.incoming_digest != acceptance.incoming_digest {
        return Err(refuse(
            "incoming_changed",
            "Incoming references changed since acceptance.",
        ));
    }

    // Originals: prove once before any effect; on resume re-verify the stored locators only.
    let originals = if first {
        let items = original_items(&body);
        let located = env.verify_committed(&items).map_err(|e| {
            refuse(
                "originals_not_committed",
                format!("Originals are not committed: {}", e.message),
            )
        })?;
        stored(&items, located)
    } else {
        let stored_items: Vec<OriginalItem> = rec
            .apply
            .as_ref()
            .map(|p| {
                p.originals
                    .iter()
                    .map(|o| OriginalItem {
                        relative: o.relative.clone(),
                        sha256: o.sha256.clone(),
                        len: o.len,
                        at: Some(o.commit.clone()),
                    })
                    .collect()
            })
            .unwrap_or_default();
        if !stored_items.is_empty() {
            env.verify_committed(&stored_items).map_err(|e| {
                refuse(
                    "originals_not_committed",
                    format!("A stored original no longer verifies: {}", e.message),
                )
            })?;
        }
        rec.apply
            .as_ref()
            .map(|p| p.originals.clone())
            .unwrap_or_default()
    };

    let mut run = Run {
        env,
        actor,
        rec,
        observed: loaded.bytes,
        body,
        blobs,
        effects,
        classes: BTreeMap::new(),
        older: Vec::new(),
        executed: BTreeSet::new(),
        expected_incoming: acceptance.incoming_digest,
    };
    if first {
        run.rec.state = CpState::Applying;
        run.rec.apply = Some(ApplyProgress {
            attempts: 1,
            phase: ApplyPhase::Started,
            originals,
            blocked: None,
        });
        let observed = run.observed.clone();
        let new_version = ops::save_record(env, &mut run.rec, Some(&observed), false, run.effects)?;
        run.observed = env
            .store()
            .bytes(&record::record_path(&run.rec.id))?
            .ok_or_else(|| refuse("io", "The record vanished after publication."))?;
        let _ = new_version;
    } else if let Some(p) = run.rec.apply.as_mut() {
        p.attempts += 1;
        p.blocked = None;
    }

    // Classification pass over every action, applied ones included, before anything runs.
    let actions = run.body.actions.clone();
    for a in &actions {
        let class = run.status(a);
        if a.state == ActionState::Applied {
            match &class {
                Class::Attested(receipt) => {
                    if let Err(f) = run.verify(a, receipt) {
                        return Err(run.stop(f.code(), f.reason().to_owned(), Some(&a.id)));
                    }
                }
                _ => {
                    return Err(run.stop(
                        "post_apply_drift",
                        format!("{}: an applied action is no longer attested.", a.path),
                        Some(&a.id),
                    ));
                }
            }
        }
        if a.kind != ActionKind::Remove
            && let Class::Attested(r) | Class::Partial(r, _) = &class
        {
            let state = run.older_state(a, r);
            run.older.push((a.id.clone(), state));
        }
        run.classes.insert(a.id.clone(), class);
    }

    // Phases: Create, Move, Replace, verification, Remove.
    for phase in [ActionKind::Create, ActionKind::Move, ActionKind::Replace] {
        for i in 0..run.body.actions.len() {
            if run.body.actions[i].kind == phase
                && run.body.actions[i].state != ActionState::Applied
            {
                run.drive(i)?;
            }
        }
    }
    if let Some(p) = run.rec.apply.as_mut() {
        p.phase = ApplyPhase::Verifying;
    }
    let pending_removals = run
        .body
        .actions
        .iter()
        .any(|a| a.kind == ActionKind::Remove && a.state != ActionState::Applied);
    if pending_removals {
        let mut view = record::ProposalBody {
            actions: run.body.actions.clone(),
            ..run.body.clone()
        };
        view.actions = run.body.actions.clone();
        let fresh = validate::coverage(env, &view, &run.blobs, true);
        match fresh {
            Ok(c) if c.incoming_digest == run.expected_incoming => {}
            Ok(_) => {
                return Err(run.stop(
                    "incoming_changed",
                    "Incoming references changed during application.",
                    None,
                ));
            }
            Err(e) => return Err(run.stop(e.code, e.message, None)),
        }
        if let Some(p) = run.rec.apply.as_mut() {
            p.phase = ApplyPhase::Removing;
        }
        for i in 0..run.body.actions.len() {
            if run.body.actions[i].kind == ActionKind::Remove
                && run.body.actions[i].state != ActionState::Applied
            {
                run.drive(i)?;
            }
        }
    }

    // Everything applied: finish with a terminal write.
    run.rec.revision_mut()?.body.actions = run.body.actions.clone();
    run.rec.state = CpState::Applied;
    if let Some(p) = run.rec.apply.as_mut() {
        p.phase = ApplyPhase::Done;
        p.blocked = None;
    }
    let observed = run.observed.clone();
    let version = ops::save_record(env, &mut run.rec, Some(&observed), true, run.effects)?;
    Ok(Outcome::of(&run.rec, version, true))
}
