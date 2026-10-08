//! Explicit recovery of pending Git work: reconcile, retry, adopt, release and preserve.
//!
//! Recovery runs under the dispatcher's single root lock (witnessed by `&LockGuard`) and never takes it.
//! It acts only on the intents or exact byte identities the caller names, never replays a business
//! mutation, and checks the observed pending snapshot version before any effect.
use super::{
    engine,
    journal::{self, Entry, EntryPhase, Intent, IntentOutcome},
    locator::{self, Item, Locator, Proof},
    policy::{Event, EventClass, EventOutcome},
    receipt::{GitOutcome, GitReceipt, Reason},
    status,
};
use crate::store::{
    ABSOLUTE_CAP, EffectKind, Error, LockGuard, OperationId, Result, Store, UntrackedReason,
};
use std::time::Instant;

/// One file the caller authorizes to be committed with exactly these bytes (owned wire form).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreserveItem {
    /// Owned relative path.
    pub relative: String,
    /// SHA-256 of the authorized bytes.
    pub sha256: String,
    /// Length of the authorized bytes.
    pub len: u64,
}

/// The exact bytes the caller authorizes for one file.
#[derive(Debug, Clone, Copy)]
pub struct PreservePath<'a> {
    /// Owned relative path.
    pub relative: &'a str,
    /// Lowercase SHA-256 of the authorized bytes.
    pub sha256: &'a str,
    /// Length of the authorized bytes.
    pub len: u64,
}

/// A preservation request: up to 32 files and an optional operation identity carried into the trailers.
#[derive(Debug, Clone, Copy)]
pub struct PreserveRequest<'a> {
    /// Files to commit.
    pub paths: &'a [PreservePath<'a>],
    /// Caller operation identity for the preserved effects.
    pub operation: Option<&'a OperationId>,
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
    Preserve(Vec<PreserveItem>),
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

/// Run the explicit `action` for `store` under its caller-owned root `guard`, without business replay.
///
/// `expected_version` is the exact pending observation; stale state refuses before effects. The
/// report carries the resulting pending version and receipt. Half the command budget is reserved
/// from the shared settlement maximum for receipt rendering and the wire reply; verification and
/// output draining stay inside the remaining deadline. Unconfirmed commits remain recoverable.
///
/// # Errors
/// `not_locked`, `stale` (the pending snapshot changed), `invalid_data` (limits or unknown names),
/// `recovery_blocked` and `preserve_blocked` for refusal or unknown commit proof. An unknown attempt
/// may have landed; reconcile it before retrying. Messages retain pending facts.
pub fn recover(
    store: &Store,
    guard: &LockGuard,
    expected_version: &str,
    action: Action,
) -> Result<Report> {
    recover_by(
        store,
        guard,
        expected_version,
        action,
        Instant::now() + super::git::SETTLEMENT_TIME - super::git::COMMAND_TIME / 2,
    )
}

/// [`recover`] with an explicit deadline for the whole recovery, so tests can bound a stuck hook.
///
/// # Errors
/// The same as [`recover`].
pub fn recover_by(
    store: &Store,
    guard: &LockGuard,
    expected_version: &str,
    action: Action,
    deadline: Instant,
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
            receipt.commit.is_some()
        }
        Action::Preserve(items) => {
            let paths: Vec<PreservePath<'_>> = items
                .iter()
                .map(|p| PreservePath {
                    relative: &p.relative,
                    sha256: &p.sha256,
                    len: p.len,
                })
                .collect();
            let locators = preserve_core(store, &paths, None, deadline, &mut receipt)?;
            for locator in locators.iter().take(2) {
                if let Ok(text) = locator.encode() {
                    lines.push(format!("Preserved {} at {text}", locator.relative));
                }
            }
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
///
/// The whole connected chain must be named: a recovery that would split paths shared with an unnamed
/// intent is refused, naming the rest. `ids` identifies previously validated open intents in the
/// locked `store`; `deadline` bounds Git, and `receipt` plus `lines` receive only this recovery's facts.
/// Superseded entries are skipped and reported, never restored or certified as completed. If every
/// named entry is superseded, refuse without changing Git or the journal and name the first path.
/// Partial selection is reported; unknown outcomes require reconciliation before another attempt.
fn retry(
    store: &Store,
    ids: &[String],
    deadline: Instant,
    receipt: &mut GitReceipt,
    lines: &mut Vec<String>,
) -> Result<()> {
    let extras = engine::connected_extras(store, ids);
    if !extras.is_empty() {
        let named: Vec<&str> = extras.iter().take(4).map(String::as_str).collect();
        return Err(Error::new(
            "recovery_blocked",
            format!(
                "Intents sharing files with the named ones must be named too: {}. A split chain is never committed.",
                named.join(", ")
            ),
        ));
    }
    let (journal, _) = journal::load(store).map_err(|_| {
        Error::new(
            "recovery_blocked",
            "The pending journal is unavailable; nothing was committed.",
        )
    })?;
    let entries: Vec<_> = journal
        .intents
        .iter()
        .filter(|i| ids.contains(&i.id) && i.committed.is_none())
        .flat_map(|i| &i.entries)
        .collect();
    let skipped: Vec<_> = entries
        .iter()
        .filter(|e| e.superseded_by.is_some())
        .collect();
    if !skipped.is_empty() {
        if skipped.len() == entries.len() {
            return Err(Error::new(
                "recovery_blocked",
                format!(
                    "Skipped all {} superseded path(s); first: {}. Nothing was committed; held work stays pending. Use release to stop tracking it.",
                    skipped.len(),
                    skipped[0].path
                ),
            ));
        }
        lines.push(format!(
            "Skipped {} superseded path(s); kept their committed successors.",
            skipped.len()
        ));
    }
    let event = Event {
        class: EventClass::Recovery,
        refs: Vec::new(),
        operation: None,
        outcome: EventOutcome::Success,
    };
    engine::run_engine(store, "", &event, receipt, deadline, ids);
    match (receipt.outcome, receipt.commit.is_some()) {
        (GitOutcome::Committed, _) => {
            lines.push("Committed the named intents as one composition.".into());
        }
        (GitOutcome::Unknown, _) => {
            return Err(Error::new(
                "recovery_blocked",
                "The commit outcome is unknown; the named intents stay pending. Reconcile before retrying.",
            ));
        }
        (_, true) => lines.push(format!(
            "Committed only part of the named intents ({:?}); the rest stay pending.",
            receipt.reason
        )),
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
///
/// The intent is kept whenever a commit may have landed, so a later reconcile can prove or release it;
/// it is dropped only when Git reported that nothing was committed.
fn preserve_core(
    store: &Store,
    paths: &[PreservePath<'_>],
    operation: Option<&OperationId>,
    deadline: Instant,
    receipt: &mut GitReceipt,
) -> Result<Vec<Locator>> {
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
            .read_exact(want.relative, ABSOLUTE_CAP)?
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
            .rfind(|e| e.path == want.relative && e.superseded_by.is_none())
            .and_then(|e| e.after_sha256.clone());
        entries.push(Entry {
            path: want.relative.to_owned(),
            kind: EffectKind::Replaced,
            operation: operation.map(|o| o.as_str().to_owned()),
            phase: EntryPhase::Published,
            before_version: None,
            before_sha256: previous,
            after_sha256: Some(want.sha256.to_owned()),
            after_len: Some(want.len),
            after_version: Some(found.version),
            witness: None,
            superseded_by: None,
        });
    }
    let id = journal::new_intent_id(store);
    // The synthetic intent needs journal room like any admission: verified committed intents may be
    // pruned for it, open ones never are, so a journal full of open work refuses honestly.
    let (mut open, observed) = journal::load(store)
        .map_err(|_| Error::new("preserve_blocked", "The pending journal is not readable."))?;
    let intent = Intent {
        id: id.clone(),
        created_at: crate::store::now(),
        class: Some("recovery".into()),
        refs: Vec::new(),
        outcome: IntentOutcome::Partial,
        adopted: true,
        committing_from: None,
        committed: None,
        entries,
    };
    open.intents.push(intent.clone());
    let mut saved = journal::save(store, &open, observed.as_deref());
    if saved == Err(UntrackedReason::JournalFull) {
        let (mut pruned, observed) = journal::load(store)
            .map_err(|_| Error::new("preserve_blocked", "The pending journal is not readable."))?;
        if journal::prune(store, &mut pruned) {
            pruned.intents.push(intent);
            saved = journal::save(store, &pruned, observed.as_deref());
        }
    }
    saved.map_err(|reason| {
        Error::new(
            "preserve_blocked",
            match reason {
                UntrackedReason::JournalFull => {
                    "The pending journal is full of open intents; retry, adopt or release some first. Nothing was committed."
                }
                _ => "The pending journal cannot be written. Nothing was committed.",
            },
        )
    })?;
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
    match receipt.outcome {
        GitOutcome::Committed => (),
        GitOutcome::Unknown => {
            return Err(Error::new(
                "preserve_blocked",
                format!(
                    "The commit outcome is unknown; intent {id} stays pending. Reconcile before retrying."
                ),
            ));
        }
        _ => {
            let reason = receipt.reason.unwrap_or(Reason::CommitNotCompleted);
            let _ = edit(store, |j| j.intents.retain(|i| i.id != id));
            return Err(Error::new(
                "preserve_blocked",
                format!("Nothing was committed: {reason:?}."),
            ));
        }
    }
    let mut locators = Vec::new();
    for want in paths {
        match locator::committed_original(
            store,
            &Item {
                relative: want.relative,
                sha256: want.sha256,
                len: want.len,
                at: None,
            },
        ) {
            Proof::Committed(l) => locators.push(l),
            _ => {
                return Err(Error::new(
                    "preserve_blocked",
                    format!(
                        "The commit landed but {} could not be proven committed; inspect the pending state.",
                        crate::store::safe(want.relative, 120)
                    ),
                ));
            }
        }
    }
    Ok(locators)
}

/// Commit exactly the authorized bytes of the named files and return a locator for each.
///
/// Allowed for unattested and untracked files because the caller is the explicit authorization; the
/// commit trailers mark the intent adopted and carry the request's operation identity. Never uses the
/// policy and never defers: the caller needs the proof. Like [`recover`], its Git cutoff reserves
/// half the shared command budget for the caller's receipt and wire reply.
///
/// # Errors
/// `not_locked`, `stale` (a file's bytes changed or vanished), `invalid_data` (limits) and
/// `preserve_blocked` (nothing committed, or the outcome is unknown and the intent stays pending).
pub fn preserve(
    store: &Store,
    guard: &LockGuard,
    request: PreserveRequest<'_>,
) -> Result<Vec<Locator>> {
    if !guard.is_for(store) {
        return Err(Error::new(
            "not_locked",
            "The root write lock was not acquired through this Store.",
        ));
    }
    preserve_core(
        store,
        request.paths,
        request.operation,
        Instant::now() + super::git::SETTLEMENT_TIME - super::git::COMMAND_TIME / 2,
        &mut GitReceipt::saved_only(),
    )
}
