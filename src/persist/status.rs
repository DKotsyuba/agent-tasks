//! Read-only provenance: the complete-operation oracle, call intent evidence and pending summaries.
//!
//! `effect_status` answers from the private journal and from engine commit trailers verified against
//! the commit tree and digests. It never trusts equal bytes: a file that merely looks like the
//! intended result is `Unknown`. Everything here is lock-free, bounded and infallible.
use super::{
    engine::{self, State},
    git,
    journal::{self, EntryPhase, IntentOutcome, Journal},
    policy::{EventClass, PendingFacts},
    receipt::{PendingRef, Phase},
};
use crate::store::{ABSOLUTE_CAP, EffectKind, Error, OperationId, Result, Store, Tracking};
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::time::Instant;

/// An assertion about one effect the caller can compute exactly: path, kind and body digest.
/// Generated effects are never listed here.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExpectedEffect {
    /// Owned relative path.
    pub relative: String,
    /// Expected effect kind.
    pub kind: EffectKind,
    /// SHA-256 of the expected new bytes; `None` for a removal.
    pub after_sha256: Option<String>,
}

/// Per-effect Git state at the time of the answer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EffectGit {
    /// No journal intent holds this effect (no repository or history-only evidence without a commit).
    Untracked,
    /// The owning intent is pending.
    Pending(PendingRef),
    /// The effect is in this commit.
    Committed {
        /// Full commit id.
        commit: String,
    },
    /// The owning intent has an unknown outcome.
    Unknown(PendingRef),
}

/// One attested effect of the operation, as recorded when it happened.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PathEffect {
    /// Owned relative path.
    pub relative: String,
    /// Effect kind.
    pub kind: EffectKind,
    /// SHA-256 of the replaced or removed bytes.
    pub before_sha256: Option<String>,
    /// SHA-256 of the effect's true new bytes.
    pub after_sha256: Option<String>,
    /// Matched one of the caller's assertions; false for a same-operation effect the caller did not predict.
    pub asserted: bool,
    /// `None` when not superseded; `Some(hash)` when later bytes replaced these in the same commit chain;
    /// `Some("-")` when the chain ended in a removal.
    pub superseded_into: Option<String>,
    /// State of this effect's own intent.
    pub git: EffectGit,
    /// This effect's intent was adopted by explicit recovery.
    pub adopted: bool,
}

/// The complete attested set for one operation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EffectReceipt {
    /// The operation identity.
    pub operation: OperationId,
    /// Every attested effect, asserted or not.
    pub paths: Vec<PathEffect>,
    /// True only when the journal was readable and the bounded history scan finished.
    pub complete: bool,
}

/// Why an operation's evidence is not proven.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnknownReason {
    /// A file equals the intended result but no record attests it.
    UnattestedEqualBytes,
    /// A prepared entry's file equals the planned bytes without confirmation.
    CrashAfterRename,
    /// The journal cannot be read.
    JournalUnavailable,
    /// The bounded history scan was cut short.
    HistoryScanIncomplete,
    /// There is no repository, so there is no journal or trailer evidence.
    NoRepository,
    /// A recorded effect disagrees with the commit tree.
    ConflictingRecord,
}

/// Why an operation is foreign.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ForeignReason {
    /// An asserted path is recorded for this operation with a different kind or digest.
    OperationMismatch,
}

/// Answer of the complete-operation oracle.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EffectStatus {
    /// Every assertion attested and nothing unproven; the full set is returned.
    Attested(EffectReceipt),
    /// Some assertions attested; the listed paths were never published.
    Partial {
        /// The attested part.
        attested: EffectReceipt,
        /// Asserted paths with no attested effect.
        missing: Vec<String>,
    },
    /// Nothing attested, nothing unproven and no look-alike bytes.
    NotPublished,
    /// Unproven rows: the attested part and the current digests of the unattested look-alikes.
    Unknown {
        /// The attested part.
        attested: EffectReceipt,
        /// Path and current SHA-256 (absent for a missing file) of each unproven look-alike.
        observed: Vec<(String, Option<String>)>,
        /// First reason.
        reason: UnknownReason,
    },
    /// An assertion conflicts with a recorded effect.
    Foreign {
        /// Reason.
        reason: ForeignReason,
    },
}

/// One recorded effect found in the journal or in history.
struct Row {
    /// The effect.
    effect: PathEffect,
    /// Intent identity, for de-duplication.
    intent: String,
}

/// Parse `Agent-Tasks-Effect` trailers of one commit message into rows for `operation`.
fn rows_from_message(operation: &str, commit: &str, message: &str) -> Vec<(String, PathEffect)> {
    let mut rows = Vec::new();
    let mut intent = String::new();
    let mut adopted = std::collections::BTreeSet::new();
    for line in message.lines() {
        if let Some(id) = line.strip_prefix("Agent-Tasks-Adopted: ") {
            adopted.insert(id.trim().to_owned());
        }
    }
    for line in message.lines() {
        if let Some(id) = line.strip_prefix("Agent-Tasks-Intent: ") {
            intent = id.trim().to_owned();
        } else if let Some(rest) = line.strip_prefix("Agent-Tasks-Effect: ") {
            let parts: Vec<&str> = rest.split(' ').collect();
            if parts.len() < 5 || parts[0] != operation {
                continue;
            }
            let kind = match parts[1] {
                "created" => EffectKind::Created,
                "replaced" => EffectKind::Replaced,
                "removed" => EffectKind::Removed,
                _ => continue,
            };
            let digest = |s: &str| (s != "-").then(|| s.to_owned());
            rows.push((
                intent.clone(),
                PathEffect {
                    relative: decode_path(parts[2]),
                    kind,
                    before_sha256: digest(parts[3]),
                    after_sha256: digest(parts[4]),
                    asserted: false,
                    superseded_into: parts
                        .get(5)
                        .and_then(|p| p.strip_prefix("into="))
                        .map(str::to_owned),
                    git: EffectGit::Committed {
                        commit: commit.to_owned(),
                    },
                    adopted: adopted.contains(&intent),
                },
            ));
        }
    }
    rows
}

/// Reverse of the trailer path encoding.
pub(super) fn decode_path(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%'
            && i + 2 < bytes.len()
            && let Ok(v) = u8::from_str_radix(&text[i + 1..i + 3], 16)
        {
            out.push(v);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Current SHA-256 of a file, `Ok(None)` when absent.
fn sha_now(store: &Store, relative: &str) -> std::result::Result<Option<String>, ()> {
    store
        .read_exact(relative, ABSOLUTE_CAP)
        .map(|o| o.map(|f| journal::sha256_hex(&f.bytes)))
        .map_err(|_| ())
}

/// Journal rows for `operation`, the unproven crash rows and whether the journal was readable.
fn journal_rows(
    store: &Store,
    operation: &str,
    journal: &Journal,
) -> (Vec<Row>, Vec<(String, Option<String>)>) {
    let mut rows = Vec::new();
    let mut unproven = Vec::new();
    for intent in &journal.intents {
        let state = if intent.committed.is_some() {
            None
        } else {
            Some(engine::classify(store, intent))
        };
        for entry in intent
            .entries
            .iter()
            .filter(|e| e.operation.as_deref() == Some(operation))
        {
            if entry.phase == EntryPhase::Prepared {
                let now = sha_now(store, &entry.path).ok();
                let planned = now.as_ref() == Some(&entry.after_sha256);
                if planned && entry.kind != EffectKind::Removed
                    || (entry.kind == EffectKind::Removed && now == Some(None))
                {
                    unproven.push((entry.path.clone(), now.flatten()));
                }
                continue;
            }
            let git = match (&intent.committed, state) {
                (Some(commit), _) => EffectGit::Committed {
                    commit: commit.clone(),
                },
                (None, Some(State::Unknown)) => EffectGit::Unknown(PendingRef {
                    intent: intent.id.clone(),
                    phase: Phase::Unknown,
                    paths: intent.entries.len(),
                }),
                (None, Some(s)) => EffectGit::Pending(PendingRef {
                    intent: intent.id.clone(),
                    phase: s.phase(),
                    paths: intent.entries.len(),
                }),
                (None, None) => EffectGit::Untracked,
            };
            let superseded_into = entry.superseded_by.as_ref().and_then(|by| {
                // The superseding image is recorded with the mark, so pruning its intent loses nothing.
                if let Some((_, digest)) = by.split_once(':') {
                    return Some(digest.to_owned());
                }
                journal
                    .intents
                    .iter()
                    .flat_map(|i| i.entries.iter())
                    .rfind(|e| e.path == entry.path && e.phase == EntryPhase::Published)
                    .map(|e| e.after_sha256.clone().unwrap_or_else(|| "-".to_owned()))
            });
            rows.push(Row {
                intent: intent.id.clone(),
                effect: PathEffect {
                    relative: entry.path.clone(),
                    kind: entry.kind,
                    before_sha256: entry.before_sha256.clone(),
                    after_sha256: entry.after_sha256.clone(),
                    asserted: false,
                    superseded_into,
                    git,
                    adopted: intent.adopted,
                },
            });
        }
    }
    (rows, unproven)
}

/// Most commits one history scan reads before it reports itself incomplete.
const HISTORY_COMMITS: usize = 64;

/// Bounded history scan: rows for `operation` from commits reachable from HEAD, verified against the
/// commit tree. Returns `(rows, conflicting, complete)`.
fn history_rows(store: &Store, operation: &str) -> (Vec<Row>, bool, bool) {
    let deadline = Instant::now() + git::SETTLEMENT_TIME;
    let out = git::run(
        &store.root,
        &[
            "log",
            "--fixed-strings",
            "--grep",
            &format!("Agent-Tasks-Effect: {operation} "),
            "-n",
            "65",
            "--format=%H%x1f%B%x1e",
        ],
        None,
        deadline,
    );
    let Ok(out) = out else {
        return (Vec::new(), false, false);
    };
    if !out.success() {
        return (Vec::new(), false, false);
    }
    let mut complete = !out.truncated;
    let mut conflicting = false;
    let mut rows = Vec::new();
    let text = String::from_utf8_lossy(&out.stdout).into_owned();
    let records: Vec<&str> = text
        .split('\u{1e}')
        .filter(|r| !r.trim().is_empty())
        .collect();
    // One record beyond the bound proves that older history was omitted.
    if records.len() > HISTORY_COMMITS {
        complete = false;
    }
    for record in records.into_iter().take(HISTORY_COMMITS) {
        let Some((commit, message)) = record.trim_start().split_once('\u{1f}') else {
            continue;
        };
        for (intent, mut effect) in rows_from_message(operation, commit.trim(), message) {
            let expected = match (&effect.superseded_into, effect.kind) {
                (Some(into), _) => (into != "-").then(|| into.clone()),
                (None, EffectKind::Removed) => None,
                (None, _) => effect.after_sha256.clone(),
            };
            // A failed read is never "absent" and never a match: it makes the scan incomplete.
            let ok = match super::verify::path_matches(
                store,
                commit.trim(),
                &effect.relative,
                expected.as_deref(),
                deadline,
            ) {
                super::verify::Verdict::Exact => true,
                super::verify::Verdict::Mismatch => false,
                super::verify::Verdict::Unreadable => {
                    complete = false;
                    continue;
                }
            };
            if ok {
                effect.git = EffectGit::Committed {
                    commit: commit.trim().to_owned(),
                };
                rows.push(Row { effect, intent });
            } else {
                conflicting = true;
            }
        }
    }
    (rows, conflicting, complete)
}

/// The complete-operation oracle: what M-004's own evidence attests for `operation`.
///
/// `expected` holds assertions about effects the caller can compute exactly; generated same-operation
/// effects are returned unasserted and are neither foreign nor certified by equal bytes. Combination:
/// any foreign assertion wins; else any unproven row, look-alike or incomplete scan gives `Unknown`;
/// else all assertions satisfied gives `Attested`; some gives `Partial`; none gives `NotPublished`.
pub fn effect_status(
    store: &Store,
    operation: &OperationId,
    expected: &[ExpectedEffect],
) -> EffectStatus {
    let op = operation.as_str();
    let (journal, readable) = match journal::load(store) {
        Ok((journal, _)) => (journal, true),
        Err(super::super::store::UntrackedReason::NoRepository) => {
            return no_repository(store, operation, expected);
        }
        Err(_) => (Journal::default(), false),
    };
    let (mut rows, unproven) = journal_rows(store, op, &journal);
    let (history, conflicting, scanned) = history_rows(store, op);
    for h in history {
        if !rows
            .iter()
            .any(|r| r.intent == h.intent && r.effect.relative == h.effect.relative)
        {
            rows.push(h);
        }
    }
    let complete = readable && scanned;
    let mut foreign = false;
    let mut lookalikes: Vec<(String, Option<String>)> = Vec::new();
    let mut missing = Vec::new();
    let mut satisfied = 0usize;
    for want in expected {
        let on_path: Vec<usize> = rows
            .iter()
            .enumerate()
            .filter(|(_, r)| r.effect.relative == want.relative)
            .map(|(n, _)| n)
            .collect();
        match on_path.last() {
            Some(&n) => {
                let have = &rows[n].effect;
                if have.kind == want.kind && have.after_sha256 == want.after_sha256 {
                    satisfied += 1;
                    rows[n].effect.asserted = true;
                } else {
                    foreign = true;
                }
            }
            None => {
                missing.push(want.relative.clone());
                if unproven.iter().all(|(p, _)| *p != want.relative)
                    && let Ok(now) = sha_now(store, &want.relative)
                    && now == want.after_sha256
                {
                    lookalikes.push((want.relative.clone(), now));
                }
            }
        }
    }
    let receipt = EffectReceipt {
        operation: operation.clone(),
        paths: rows.iter().map(|r| r.effect.clone()).collect(),
        complete,
    };
    if foreign {
        return EffectStatus::Foreign {
            reason: ForeignReason::OperationMismatch,
        };
    }
    let mut observed = unproven.clone();
    observed.extend(lookalikes.iter().cloned());
    let reason = if !readable {
        Some(UnknownReason::JournalUnavailable)
    } else if !unproven.is_empty() {
        Some(UnknownReason::CrashAfterRename)
    } else if conflicting {
        Some(UnknownReason::ConflictingRecord)
    } else if !lookalikes.is_empty() {
        Some(UnknownReason::UnattestedEqualBytes)
    } else if !scanned {
        Some(UnknownReason::HistoryScanIncomplete)
    } else {
        None
    };
    if let Some(reason) = reason {
        return EffectStatus::Unknown {
            attested: receipt,
            observed,
            reason,
        };
    }
    if expected.is_empty() {
        return if receipt.paths.is_empty() {
            EffectStatus::NotPublished
        } else {
            EffectStatus::Attested(receipt)
        };
    }
    if satisfied == expected.len() {
        EffectStatus::Attested(receipt)
    } else if satisfied == 0 {
        EffectStatus::NotPublished
    } else {
        EffectStatus::Partial {
            attested: receipt,
            missing,
        }
    }
}

/// Without a repository there is no journal and no trailer: equal bytes are unknown, otherwise unpublished.
fn no_repository(
    store: &Store,
    operation: &OperationId,
    expected: &[ExpectedEffect],
) -> EffectStatus {
    let observed: Vec<(String, Option<String>)> = expected
        .iter()
        .filter_map(|want| {
            let now = sha_now(store, &want.relative).ok()?;
            (now == want.after_sha256).then(|| (want.relative.clone(), now))
        })
        .collect();
    if observed.is_empty() {
        return EffectStatus::NotPublished;
    }
    EffectStatus::Unknown {
        attested: EffectReceipt {
            operation: operation.clone(),
            paths: Vec::new(),
            complete: true,
        },
        observed,
        reason: UnknownReason::NoRepository,
    }
}

/// The receipt for effects this request just published under `operation`.
///
/// # Errors
/// `not_found` when the operation has no event in this request.
pub fn receipt(store: &Store, operation: &OperationId) -> Result<EffectReceipt> {
    let events: Vec<_> = store
        .publications()
        .into_iter()
        .filter(|e| e.operation.as_deref() == Some(operation.as_str()))
        .collect();
    if events.is_empty() {
        return Err(Error::new(
            "not_found",
            "This request published nothing under that operation.",
        ));
    }
    let (journal, _) = journal::load(store).unwrap_or_default();
    let (rows, _) = journal_rows(store, operation.as_str(), &journal);
    let mut paths = Vec::new();
    for event in &events {
        if matches!(event.tracking, Tracking::NotApplicable) {
            continue;
        }
        if let Some(row) = rows.iter().rfind(|r| {
            r.effect.relative == event.relative && Some(&r.intent) == event.intent.as_ref()
        }) {
            paths.push(row.effect.clone());
        } else {
            paths.push(PathEffect {
                relative: event.relative.clone(),
                kind: event.kind,
                before_sha256: None,
                after_sha256: None,
                asserted: false,
                superseded_into: None,
                git: EffectGit::Untracked,
                adopted: false,
            });
        }
    }
    Ok(EffectReceipt {
        operation: operation.clone(),
        paths,
        complete: true,
    })
}

/// Membership of the current call's publications, derived from `store.publications()`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CallIntents {
    /// Distinct intent identities in publication order.
    pub intents: Vec<String>,
    /// Tracked eligible-file effects.
    pub tracked: usize,
    /// Eligible-file effects without a journal entry.
    pub untracked: usize,
    /// Effects with an uncertain sync or unconfirmed journal entry, directories included.
    pub uncertain: usize,
    /// Directory and administrative effects; they never count as tracked or untracked.
    pub not_applicable: usize,
    /// At least one eligible file, every eligible file tracked, and no uncertain event.
    pub all_tracked: bool,
}

/// Summarize the current call's own events; pure and lock-free.
pub fn call_intents(store: &Store) -> CallIntents {
    let mut summary = CallIntents {
        intents: Vec::new(),
        tracked: 0,
        untracked: 0,
        uncertain: 0,
        not_applicable: 0,
        all_tracked: false,
    };
    for event in store.publications() {
        if event.durability == crate::store::Durability::SyncUnknown {
            summary.uncertain += 1;
        }
        match event.tracking {
            Tracking::NotApplicable => summary.not_applicable += 1,
            Tracking::Tracked => {
                summary.tracked += 1;
                if let Some(id) = event.intent.filter(|id| !summary.intents.contains(id)) {
                    summary.intents.push(id);
                }
            }
            Tracking::UnknownAfterPublication => {
                summary.uncertain += 1;
                summary.tracked += 1;
            }
            Tracking::Untracked(_) => summary.untracked += 1,
        }
    }
    summary.all_tracked = summary.tracked > 0 && summary.untracked == 0 && summary.uncertain == 0;
    summary
}

/// One effect of a journal intent.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IntentEffect {
    /// Owned relative path.
    pub relative: String,
    /// Effect kind.
    pub kind: EffectKind,
    /// Operation identity.
    pub operation: Option<String>,
    /// SHA-256 of the planned new bytes.
    pub after_sha256: Option<String>,
    /// Commit holding this effect once committed.
    pub committed: Option<String>,
}

/// Read-only evidence for one journal intent.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IntentView {
    /// Intent identity.
    pub intent: String,
    /// Event class name once settled.
    pub class: Option<String>,
    /// Outcome stamp.
    pub outcome: IntentOutcome,
    /// Reconciled phase.
    pub phase: Phase,
    /// Up to 256 effects.
    pub effects: Vec<IntentEffect>,
}

/// Evidence for one intent, or `None` when it is unknown or already pruned.
pub fn intent_view(store: &Store, intent: &str) -> Option<IntentView> {
    let (journal, _) = journal::load(store).ok()?;
    let found = journal.intents.iter().find(|i| i.id == intent)?;
    Some(IntentView {
        intent: found.id.clone(),
        class: found.class.clone(),
        outcome: found.outcome,
        phase: if found.committed.is_some() {
            Phase::Published
        } else {
            engine::classify(store, found).phase()
        },
        effects: found
            .entries
            .iter()
            .take(256)
            .map(|e| IntentEffect {
                relative: e.path.clone(),
                kind: e.kind,
                operation: e.operation.clone(),
                after_sha256: e.after_sha256.clone(),
                committed: found.committed.clone(),
            })
            .collect(),
    })
}

/// Lock-free summary of pending Git work for context and status.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PendingSummary {
    /// False when the journal or repository could not be fully read.
    pub complete: bool,
    /// Exact observed snapshot, see [`pending_version`].
    pub version: String,
    /// Aggregate facts.
    pub facts: PendingFacts,
    /// Up to 16 pending intents, oldest first.
    pub refs: Vec<PendingRef>,
    /// Named read problems.
    pub warnings: Vec<String>,
}

/// The branch name or detached state with HEAD's object id, from the filesystem only.
fn head_state(store: &Store) -> String {
    let git_dir = store.root.join(".git");
    let Ok(head) = std::fs::read_to_string(git_dir.join("HEAD")) else {
        return "none".into();
    };
    match head.trim().strip_prefix("ref: ") {
        None => format!("detached:{}", head.trim()),
        Some(name) => {
            let oid = std::fs::read_to_string(git_dir.join(name))
                .ok()
                .map(|s| s.trim().to_owned())
                .or_else(|| {
                    std::fs::read_to_string(git_dir.join("packed-refs"))
                        .ok()
                        .and_then(|p| {
                            p.lines().find_map(|l| {
                                l.split_once(' ')
                                    .filter(|(_, n)| *n == name)
                                    .map(|(o, _)| o.to_owned())
                            })
                        })
                })
                .unwrap_or_else(|| "unborn".into());
            format!("branch:{name}:{oid}")
        }
    }
}

/// Snapshot digest over the immutable observations recovery cares about: the journal file's version,
/// the attached branch or detached state with HEAD's id, and the current version of every journaled
/// path. Nothing here is time- or offset-based.
pub fn pending_version(store: &Store) -> String {
    let mut digest = Sha256::new();
    let bytes = store
        .read_exact(journal::JOURNAL_PATH, journal::JOURNAL_CAP)
        .ok()
        .flatten();
    digest.update(b"agent-tasks/pending/v1");
    digest.update(
        bytes
            .as_ref()
            .map_or("absent".to_owned(), |b| b.version.clone())
            .as_bytes(),
    );
    digest.update(head_state(store).as_bytes());
    if let Ok((journal, _)) = journal::load(store) {
        let mut paths: Vec<&str> = journal
            .intents
            .iter()
            .flat_map(|i| i.entries.iter().map(|e| e.path.as_str()))
            .collect();
        paths.sort_unstable();
        paths.dedup();
        for path in paths {
            let version = store
                .read_exact(path, ABSOLUTE_CAP)
                .ok()
                .flatten()
                .map(|f| f.version);
            digest.update(path.as_bytes());
            digest.update(version.unwrap_or_else(|| "absent".into()).as_bytes());
        }
    }
    format!("{:x}", digest.finalize())
}

/// Read-only pending summary; never locks, never commits, unreadable evidence is `complete = false`.
pub fn pending(store: &Store) -> PendingSummary {
    let mut warnings = Vec::new();
    let complete = match journal::load(store) {
        Ok(_) => true,
        Err(super::super::store::UntrackedReason::NoRepository) => true,
        Err(reason) => {
            warnings.push(format!("Pending journal is not readable: {reason:?}."));
            false
        }
    };
    let mut refs = engine::pending_refs(store, None);
    refs.reverse();
    PendingSummary {
        complete,
        version: pending_version(store),
        facts: engine::pending_facts(store),
        refs,
        warnings,
    }
}

/// Keep the event class name type referenced for dispatchers that map classes to journal names.
pub fn class_name(class: EventClass) -> &'static str {
    class.name()
}
