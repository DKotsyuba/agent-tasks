//! Explicit recovery of pending Git work: reconcile, retry, adopt, release and preserve.
//!
//! Recovery runs under the dispatcher's single root lock (witnessed by `&LockGuard`) and never takes it.
//! It acts only on the intents or exact byte identities the caller names, never replays a business
//! mutation, and checks the observed pending snapshot version before any effect.
use super::{
    engine,
    journal::{self, Entry, EntryPhase, Intent, IntentOutcome},
    locator::{self, Item, Proof},
    policy::{Event, EventClass, EventOutcome},
    receipt::{GitOutcome, GitReceipt, Reason},
    status,
};
use crate::store::{ABSOLUTE_CAP, EffectKind, Error, LockGuard, Result, Store};
use std::time::Instant;

/// One file the caller authorizes to be committed with exactly these bytes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreservePath {
    /// Owned relative path.
    pub relative: String,
    /// SHA-256 of the authorized bytes.
    pub sha256: String,
    /// Length of the authorized bytes.
    pub len: u64,
}

/// A recovery request over explicitly selected pending work.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    /// Resolve unknown and committing intents from HEAD, trailers and the files; read-only for Git.
    Reconcile(Vec<String>),
    /// Commit the named published or held intents together, without a policy decision.
    Retry(Vec<String>),
    /// Treat the named unknown intents with matching bytes as published, after explicit authorization.
    Adopt(Vec<String>),
    /// Stop tracking the named intents without committing; files are untouched.
    Release(Vec<String>),
    /// Commit exactly the authorized bytes of the named files.
    Preserve(Vec<PreservePath>),
}

/// What a recovery did, as plain data.
#[derive(Debug, Clone)]
pub struct Report {
    /// Whether anything was committed, released, adopted or reconciled.
    pub changed: bool,
    /// Plain change lines, at most two are shown by the host.
    pub lines: Vec<String>,
    /// The explicit receipt of the recovery.
    pub receipt: GitReceipt,
    /// The pending snapshot after the operation.
    pub version: String,
}

/// Most intents one request may name.
pub const MAX_INTENTS: usize = 16;
/// Most paths one preservation may name.
pub const MAX_PATHS: usize = 32;

/// Run one recovery action under the caller's root lock.
///
/// # Errors
/// `not_locked`, `stale` (the pending snapshot changed), `invalid_data` (limits or unknown names),
/// `recovery_blocked` and `preserve_blocked` (nothing was committed; the message names pending facts).
pub fn recover(
    store: &Store,
    guard: &LockGuard,
    expected_version: &str,
    action: Action,
) -> Result<Report> {
    if !guard.is_for(store) {
        return Err(Error::new(
            "not_locked",
            "The root write lock was not acquired through this Store.",
        ));
    }
    if status::pending_version(store) != expected_version {
        return Err(Error::new(
            "stale",
            "Pending state changed; read the pending facts again before recovering.",
        ));
    }
    let deadline = Instant::now() + super::git::SETTLEMENT_TIME;
    let mut lines = Vec::new();
    let mut receipt = GitReceipt::saved_only();
    let changed = match action {
        Action::Reconcile(ids) => {
            check_ids(store, &ids, false)?;
            engine::discard_unpublished(store);
            engine::reconcile_committing(store, deadline);
            lines.push(format!(
                "Reconciled {} intent(s) against HEAD and the files.",
                ids.len()
            ));
            true
        }
        Action::Release(ids) => {
            check_ids(store, &ids, true)?;
            edit(store, |j| {
                j.intents
                    .retain(|i| !ids.contains(&i.id) || i.committed.is_some())
            })?;
            lines.push(format!(
                "Released {} intent(s); files were left untouched.",
                ids.len()
            ));
            true
        }
        Action::Adopt(ids) => {
            check_ids(store, &ids, true)?;
            adopt(store, &ids, &mut lines)?;
            true
        }
        Action::Retry(ids) => {
            check_ids(store, &ids, true)?;
            retry(store, &ids, deadline, &mut receipt, &mut lines)?;
            receipt.outcome == GitOutcome::Committed
        }
        Action::Preserve(paths) => {
            preserve(store, &paths, deadline, &mut receipt, &mut lines)?;
            true
        }
    };
    receipt.pending = engine::pending_refs(store, None);
    Ok(Report {
        changed,
        lines,
        receipt,
        version: status::pending_version(store),
    })
}

/// Validate the named intents: bounded count and, when `must_exist`, all present and uncommitted.
fn check_ids(store: &Store, ids: &[String], must_exist: bool) -> Result<()> {
    if ids.is_empty() || ids.len() > MAX_INTENTS {
        return Err(crate::store::invalid(
            "Name between one and sixteen pending intents.",
        ));
    }
    let (journal, _) = journal::load(store)
        .map_err(|_| Error::new("recovery_blocked", "The pending journal is not readable."))?;
    for id in ids {
        let found = journal.intents.iter().find(|i| i.id == *id);
        if must_exist && found.is_none_or(|i| i.committed.is_some()) {
            return Err(Error::new(
                "recovery_blocked",
                format!("Intent {} is not pending.", crate::store::safe(id, 40)),
            ));
        }
    }
    Ok(())
}

/// Load, change and save the journal.
fn edit(store: &Store, change: impl FnOnce(&mut journal::Journal)) -> Result<()> {
    let (mut journal, observed) = journal::load(store)
        .map_err(|_| Error::new("recovery_blocked", "The pending journal is not readable."))?;
    change(&mut journal);
    journal::save(store, &journal, observed.as_deref())
        .map_err(|_| Error::new("recovery_blocked", "The pending journal cannot be written."))
}

/// Promote unknown intents whose files equal the planned bytes to published, adopted and held.
fn adopt(store: &Store, ids: &[String], lines: &mut Vec<String>) -> Result<()> {
    let (journal, _) = journal::load(store)
        .map_err(|_| Error::new("recovery_blocked", "The pending journal is not readable."))?;
    for id in ids {
        let intent = journal.intents.iter().find(|i| i.id == *id);
        if intent.is_none_or(|i| engine::classify(store, i) != engine::State::Unknown) {
            return Err(Error::new(
                "recovery_blocked",
                format!(
                    "Intent {} is not unknown with matching bytes; drifted work is never adopted.",
                    crate::store::safe(id, 40)
                ),
            ));
        }
    }
    edit(store, |j| {
        for intent in j.intents.iter_mut().filter(|i| ids.contains(&i.id)) {
            for entry in &mut intent.entries {
                entry.phase = EntryPhase::Published;
            }
            intent.adopted = true;
            intent.outcome = IntentOutcome::Partial;
        }
    })?;
    lines.push(format!(
        "Adopted {} unknown intent(s) as published; commit them with retry.",
        ids.len()
    ));
    Ok(())
}

/// Commit the named intents as one composition through the engine rules.
fn retry(
    store: &Store,
    ids: &[String],
    deadline: Instant,
    receipt: &mut GitReceipt,
    lines: &mut Vec<String>,
) -> Result<()> {
    let event = Event {
        class: EventClass::Recovery,
        refs: Vec::new(),
        operation: None,
        outcome: EventOutcome::Success,
    };
    engine::run_engine(store, "", &event, receipt, deadline, ids);
    match receipt.outcome {
        GitOutcome::Committed => {
            lines.push("Committed the named intents as one composition.".into())
        }
        _ => {
            return Err(Error::new(
                "recovery_blocked",
                format!(
                    "Nothing was committed: {:?}. Pending work stays recoverable.",
                    receipt.reason
                ),
            ));
        }
    }
    Ok(())
}

/// Commit exactly the authorized bytes of the named files through a synthetic adopted intent.
fn preserve(
    store: &Store,
    paths: &[PreservePath],
    deadline: Instant,
    receipt: &mut GitReceipt,
    lines: &mut Vec<String>,
) -> Result<()> {
    if paths.is_empty() || paths.len() > MAX_PATHS {
        return Err(crate::store::invalid(
            "Name between one and thirty-two files to preserve.",
        ));
    }
    let (journal, _) = journal::load(store)
        .map_err(|_| Error::new("preserve_blocked", "The pending journal is not readable."))?;
    let mut entries = Vec::new();
    for want in paths {
        let found = store
            .read_exact(&want.relative, ABSOLUTE_CAP)?
            .ok_or_else(|| Error::new("stale", "A named file is absent."))?;
        if journal::sha256_hex(&found.bytes) != want.sha256 || found.bytes.len() as u64 != want.len
        {
            return Err(Error::new(
                "stale",
                "A named file no longer has the authorized bytes.",
            ));
        }
        let previous = journal
            .intents
            .iter()
            .filter(|i| i.committed.is_none())
            .flat_map(|i| i.entries.iter())
            .rfind(|e| e.path == want.relative)
            .and_then(|e| e.after_sha256.clone());
        entries.push(Entry {
            path: want.relative.clone(),
            kind: EffectKind::Replaced,
            operation: None,
            phase: EntryPhase::Published,
            before_version: None,
            before_sha256: previous,
            after_sha256: Some(want.sha256.clone()),
            after_len: Some(want.len),
            after_version: Some(found.version),
            witness: None,
            superseded_by: None,
        });
    }
    let id = journal::new_intent_id(store);
    edit(store, |j| {
        j.intents.push(Intent {
            id: id.clone(),
            created_at: crate::store::now(),
            class: Some("recovery".into()),
            refs: Vec::new(),
            outcome: IntentOutcome::Partial,
            adopted: true,
            committing_from: None,
            committed: None,
            entries,
        })
    })
    .map_err(|e| Error::new("preserve_blocked", e.message))?;
    let event = Event {
        class: EventClass::Recovery,
        refs: Vec::new(),
        operation: None,
        outcome: EventOutcome::Success,
    };
    engine::run_engine(
        store,
        &id,
        &event,
        receipt,
        deadline,
        std::slice::from_ref(&id),
    );
    if receipt.outcome != GitOutcome::Committed {
        let reason = receipt.reason.unwrap_or(Reason::CommitNotCompleted);
        let _ = edit(store, |j| j.intents.retain(|i| i.id != id));
        return Err(Error::new(
            "preserve_blocked",
            format!("Nothing was committed: {reason:?}."),
        ));
    }
    for want in paths {
        if let Proof::Committed(l) = locator::committed_original(
            store,
            &Item {
                relative: &want.relative,
                sha256: &want.sha256,
                len: want.len,
                at: None,
            },
        ) && let Ok(text) = l.encode()
        {
            lines.push(format!("Preserved {} at {text}", want.relative));
        }
    }
    Ok(())
}
