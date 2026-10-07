//! Private pending journal: write-ahead provenance for eligible owned-file publications.
//!
//! The journal is one closed YAML file inside the documentation repository's Git directory
//! (`.git/agent-tasks/pending.yaml`), so it is untracked by definition and local to the clone. It is
//! bounded to [`JOURNAL_CAP`] serialized bytes, [`MAX_INTENTS`] open intents and [`MAX_ENTRIES`] path
//! entries. Admission never discards, reorders or rewrites old entries except by pruning intents
//! that are already committed and verified. Equal bytes are never ownership: only an entry written
//! here before a publication names that publication.
use crate::store::{self, EffectKind, Error, Result, Store, UntrackedReason};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

/// Journal location relative to the documentation root.
pub const JOURNAL_PATH: &str = ".git/agent-tasks/pending.yaml";
/// Directory that holds the journal, created on first admission.
const JOURNAL_DIR: &str = ".git/agent-tasks";
/// Largest serialized journal, measured on the actual bytes before every write.
pub const JOURNAL_CAP: usize = 256 * 1024;
/// Most open intents the journal holds.
pub const MAX_INTENTS: usize = 64;
/// Most path entries across all intents.
pub const MAX_ENTRIES: usize = 256;
/// Journal schema revision understood by this build.
const SCHEMA: u32 = 1;

/// How the call that wrote an intent ended, stamped at settlement.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum IntentOutcome {
    /// The call never reached settlement (for example a crash); never auto-committed.
    Unsettled,
    /// The call returned success with every eligible effect tracked and certain.
    Success,
    /// The call failed after publishing, or one of its effects is uncertain or untracked.
    Partial,
}

/// Progress of one path entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum EntryPhase {
    /// Written before the publication; the file may or may not have become visible.
    Prepared,
    /// The publication was confirmed after it happened.
    Published,
}

/// One owned-file effect recorded before it was performed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Entry {
    /// Owned relative path of the file.
    pub path: String,
    /// Create, replace or remove.
    pub kind: EffectKind,
    /// Caller operation identity, when the caller supplied one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub operation: Option<String>,
    /// Entry progress.
    pub phase: EntryPhase,
    /// Store version of the bytes this effect replaced or removed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub before_version: Option<String>,
    /// SHA-256 of the bytes this effect replaced or removed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub before_sha256: Option<String>,
    /// SHA-256 of the planned new bytes; absent for a removal.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub after_sha256: Option<String>,
    /// Length of the planned new bytes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub after_len: Option<u64>,
    /// Store version of the new bytes, set when the publication is confirmed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub after_version: Option<String>,
    /// Device and inode of the staged file that became the target (Unix); evidence only.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub witness: Option<[u64; 2]>,
    /// Intent that replaced this entry's path later inside a contiguous chain, if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub superseded_by: Option<String>,
}

/// All tracked publications of one handler call.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Intent {
    /// `PG-` plus 24 lowercase hex characters.
    pub id: String,
    /// RFC 3339 creation time, evidence only.
    pub created_at: String,
    /// Event class name once the call settled.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub class: Option<String>,
    /// Advisory canonical references of affected records, up to 16.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub refs: Vec<String>,
    /// Call outcome stamp.
    pub outcome: IntentOutcome,
    /// Recovery adopted this intent after explicit authorization.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub adopted: bool,
    /// HEAD recorded when a commit attempt started, so a lost reply can be reconciled.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub committing_from: Option<String>,
    /// Commit that holds this intent once committed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub committed: Option<String>,
    /// Path entries in publication order.
    pub entries: Vec<Entry>,
}

/// The whole journal file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Journal {
    /// Schema revision, currently 1.
    pub schema_version: u32,
    /// Open intents, oldest first.
    pub intents: Vec<Intent>,
}

impl Default for Journal {
    /// An empty journal at the current schema.
    fn default() -> Self {
        Self {
            schema_version: SCHEMA,
            intents: Vec::new(),
        }
    }
}

/// Everything admission needs to know about one planned effect.
pub struct Spec<'a> {
    /// Owned relative path.
    pub relative: &'a str,
    /// Create, replace or remove.
    pub kind: EffectKind,
    /// Caller operation identity.
    pub operation: Option<&'a str>,
    /// Store version of the bytes being replaced or removed.
    pub before_version: Option<String>,
    /// SHA-256 of the bytes being replaced or removed.
    pub before_sha256: Option<String>,
    /// SHA-256 of the planned new bytes.
    pub after_sha256: Option<String>,
    /// Length of the planned new bytes.
    pub after_len: Option<u64>,
    /// Device and inode of the staged file.
    pub witness: Option<[u64; 2]>,
}

/// Result of asking the journal to retain one planned effect.
pub enum Admission {
    /// The entry is durable in phase `Prepared` under this intent.
    Tracked(String),
    /// The effect proceeds without an entry for this reason; it can never be certified later.
    Untracked(UntrackedReason),
}

/// Lowercase hexadecimal SHA-256 of `bytes`.
pub fn sha256_hex(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

/// Whether HEAD already names a commit: a detached object id, or a branch ref that exists loose or packed.
fn head_born(git: &std::path::Path) -> bool {
    let Ok(head) = std::fs::read_to_string(git.join("HEAD")) else {
        return false;
    };
    match head.trim().strip_prefix("ref: ") {
        None => !head.trim().is_empty(),
        Some(reference) => {
            git.join(reference).is_file()
                || std::fs::read_to_string(git.join("packed-refs")).is_ok_and(|packed| {
                    packed
                        .lines()
                        .any(|l| l.split_once(' ').is_some_and(|(_, name)| name == reference))
                })
        }
    }
}

/// Whether the root holds an independent repository, with at least one commit, whose Git directory
/// can hold the journal. Before the first commit (for example during registration) nothing is
/// tracked, because the bootstrap commit is made by registration itself and no settlement follows.
fn repository(store: &Store) -> std::result::Result<(), UntrackedReason> {
    let git = store.root.join(".git");
    match std::fs::symlink_metadata(&git) {
        Ok(m) if m.is_dir() && !m.file_type().is_symlink() && head_born(&git) => Ok(()),
        _ => Err(UntrackedReason::NoRepository),
    }
}

/// Read and decode the journal, with its exact bytes for guarded replacement.
///
/// An absent file is an empty journal. A corrupt or oversize file is [`UntrackedReason::JournalCorrupt`]
/// and is left in place as evidence; I/O trouble is [`UntrackedReason::JournalUnavailable`].
pub fn load(store: &Store) -> std::result::Result<(Journal, Option<Vec<u8>>), UntrackedReason> {
    repository(store)?;
    match store.read_exact(JOURNAL_PATH, JOURNAL_CAP) {
        Ok(None) => Ok((Journal::default(), None)),
        Ok(Some(found)) => store::decode::<Journal>(&found.bytes)
            .ok()
            .filter(|j| j.schema_version == SCHEMA)
            .map(|j| (j, Some(found.bytes)))
            .ok_or(UntrackedReason::JournalCorrupt),
        Err(e) if e.code == "capacity" || e.code == "file_type" => {
            Err(UntrackedReason::JournalCorrupt)
        }
        Err(_) => Err(UntrackedReason::JournalUnavailable),
    }
}

/// Number of path entries across every intent.
fn entry_count(journal: &Journal) -> usize {
    journal.intents.iter().map(|i| i.entries.len()).sum()
}

/// Serialize and atomically replace the journal.
///
/// Refuses with [`UntrackedReason::JournalFull`] when the serialized bytes, the intent count or the
/// entry count would exceed their bounds; the previous file is untouched on those refusals. A
/// replacement that became visible but whose directory sync is unconfirmed is
/// [`UntrackedReason::JournalUnavailable`], so admission never treats it as retained and a required
/// attestation refuses before any business byte is written.
pub fn save(
    store: &Store,
    journal: &Journal,
    observed: Option<&[u8]>,
) -> std::result::Result<(), UntrackedReason> {
    if journal.intents.len() > MAX_INTENTS || entry_count(journal) > MAX_ENTRIES {
        return Err(UntrackedReason::JournalFull);
    }
    let bytes = store::encode(journal).map_err(|_| UntrackedReason::JournalUnavailable)?;
    if bytes.len() > JOURNAL_CAP {
        return Err(UntrackedReason::JournalFull);
    }
    let dir = store
        .path(JOURNAL_DIR)
        .map_err(|_| UntrackedReason::JournalUnavailable)?;
    if !dir.exists() {
        std::fs::create_dir(&dir).map_err(|_| UntrackedReason::JournalUnavailable)?;
    }
    store
        .publish_raw(JOURNAL_PATH, &bytes, observed, JOURNAL_CAP)
        .map_err(|_| UntrackedReason::JournalUnavailable)
}

/// Generate a fresh intent identity: `PG-` plus 24 hex characters from the root, process, clock and counter.
pub fn new_intent_id(store: &Store) -> String {
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(1);
    let mut digest = Sha256::new();
    digest.update(store.root.as_os_str().as_encoded_bytes());
    digest.update(std::process::id().to_le_bytes());
    digest.update(store::now().as_bytes());
    digest.update(COUNTER.fetch_add(1, Ordering::Relaxed).to_le_bytes());
    let hex = format!("{:x}", digest.finalize());
    format!("PG-{}", &hex[..24])
}

/// Retain one planned effect as a `Prepared` entry before it is performed.
///
/// All admissions of one request share one intent (`call_intent`). Full journals first drop intents that
/// are committed and verified (see [`prune`]); if there is still no room the effect is untracked.
pub fn admit(store: &Store, spec: &Spec<'_>) -> Admission {
    let (journal, observed) = match load(store) {
        Ok(found) => found,
        Err(reason) => return Admission::Untracked(reason),
    };
    let id = store.call_intent_or_new(|| new_intent_id(store));
    let entry = Entry {
        path: spec.relative.to_owned(),
        kind: spec.kind,
        operation: spec.operation.map(str::to_owned),
        phase: EntryPhase::Prepared,
        before_version: spec.before_version.clone(),
        before_sha256: spec.before_sha256.clone(),
        after_sha256: spec.after_sha256.clone(),
        after_len: spec.after_len,
        after_version: None,
        witness: spec.witness,
        superseded_by: None,
    };
    let attempt = |journal: &mut Journal| {
        match journal.intents.iter_mut().find(|i| i.id == id) {
            Some(intent) => intent.entries.push(entry.clone()),
            None => journal.intents.push(Intent {
                id: id.clone(),
                created_at: store::now(),
                class: None,
                refs: Vec::new(),
                outcome: IntentOutcome::Unsettled,
                adopted: false,
                committing_from: None,
                committed: None,
                entries: vec![entry.clone()],
            }),
        }
        save(store, journal, observed.as_deref())
    };
    let mut candidate = journal.clone();
    match attempt(&mut candidate) {
        Ok(()) => return Admission::Tracked(id),
        Err(UntrackedReason::JournalFull) => (),
        Err(reason) => return Admission::Untracked(reason),
    }
    let mut pruned = journal.clone();
    if !prune(store, &mut pruned) {
        return Admission::Untracked(UntrackedReason::JournalFull);
    }
    match attempt(&mut pruned) {
        Ok(()) => Admission::Tracked(id),
        Err(reason) => Admission::Untracked(reason),
    }
}

/// Drop committed intents, oldest first, until at least one intent is gone; true if any was dropped.
///
/// Only intents with a recorded commit are eligible, and each is dropped only after its commit is
/// verified reachable from the attached HEAD (its trailers keep the operation evidence). Intents in any
/// other state are never pruned.
pub fn prune(store: &Store, journal: &mut Journal) -> bool {
    let before = journal.intents.len();
    journal.intents.retain(|intent| {
        !intent
            .committed
            .as_deref()
            .is_some_and(|commit| crate::persist::proof::reachable(store, commit))
    });
    journal.intents.len() < before
}

/// Confirm the latest `Prepared` entry for `relative` with this planned digest as `Published`.
///
/// # Errors
/// An I/O or encoding failure; the business publication has already happened and is not reversed.
pub fn mark_published(
    store: &Store,
    intent: &str,
    relative: &str,
    after_sha256: Option<&str>,
    after_version: Option<String>,
) -> Result<()> {
    let (mut journal, observed) = load(store).map_err(|_| {
        Error::new(
            "journal",
            "Pending journal is unavailable after publication.",
        )
    })?;
    let entry = journal
        .intents
        .iter_mut()
        .find(|i| i.id == intent)
        .and_then(|i| {
            i.entries.iter_mut().rev().find(|e| {
                e.phase == EntryPhase::Prepared
                    && e.path == relative
                    && e.after_sha256.as_deref() == after_sha256
            })
        })
        .ok_or_else(|| Error::new("journal", "Prepared journal entry is missing."))?;
    entry.phase = EntryPhase::Published;
    entry.after_version = after_version;
    save(store, &journal, observed.as_deref())
        .map_err(|_| Error::new("journal", "Cannot confirm the publication in the journal."))
}

/// Remove a `Prepared` entry whose publication was refused, and its intent if it becomes empty.
///
/// Best effort: a failure leaves a `Prepared` entry that reconciliation reports as an unpublished
/// intent and discards at the next write scope.
pub fn abandon(store: &Store, intent: &str, relative: &str, after_sha256: Option<&str>) {
    let Ok((mut journal, observed)) = load(store) else {
        return;
    };
    if let Some(i) = journal.intents.iter_mut().find(|i| i.id == intent)
        && let Some(at) = i.entries.iter().rposition(|e| {
            e.phase == EntryPhase::Prepared
                && e.path == relative
                && e.after_sha256.as_deref() == after_sha256
        })
    {
        i.entries.remove(at);
    }
    journal.intents.retain(|i| !i.entries.is_empty());
    let _ = save(store, &journal, observed.as_deref());
}

/// Whether this journal already holds a confirmed effect for the same operation and path.
///
/// This is the cheap idempotency guard behind `operation_repeat`; it never reads Git history.
pub fn already_published(store: &Store, operation: &str, relative: &str) -> bool {
    load(store).is_ok_and(|(journal, _)| {
        journal.intents.iter().any(|i| {
            i.entries.iter().any(|e| {
                e.phase == EntryPhase::Published
                    && e.path == relative
                    && e.operation.as_deref() == Some(operation)
            })
        })
    })
}
