//! Guarded publication primitives: exact capped reads, no-clobber create, guarded replace and
//! removal, bounded directory creation and listing, and the typed request-local event ledger.
//!
//! These are additive to the original `publish` and `save`, which keep their signatures and now
//! delegate here. Eligible owned files are retained in the private pending journal before they are
//! touched; directories and built-in administrative files are [`Tracking::NotApplicable`].
//! Equal bytes never become ownership, and a failed journal never reverses a business publication.
#![allow(
    dead_code,
    reason = "Early primitives handoff: consumers in other Modules land later and this allowance is removed with them"
)]
use super::{Error, RECORD_CAP, Result, Store, TEMP_SEQUENCE, read_path, sync_parent};
use crate::persist::journal::{self, Admission, Spec, sha256_hex};
use serde::{Deserialize, Serialize};
use std::{
    fs::{self, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
    sync::atomic::Ordering,
};

/// Largest cap any caller may request; bounds memory regardless of the caller.
pub const ABSOLUTE_CAP: usize = 8 * 1024 * 1024;
/// Most directory levels one [`Store::ensure_parents`] call creates.
const PARENT_LEVELS: usize = 4;
/// Most entries one [`Store::list_dir`] call returns.
pub const LIST_CAP: usize = 4096;

/// Opaque caller-chosen identity of a multi-step operation, 1 to 64 characters of
/// `A-Z a-z 0-9 . _ : / -`. It is carried on events, in the journal and in commit trailers, grants
/// nothing and is never parsed. Callers derive it deterministically, never from an attempt counter.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct OperationId(String);

impl OperationId {
    /// Validate and wrap an identity.
    ///
    /// # Errors
    /// `invalid` when the text is empty, longer than 64 characters or uses a character outside the set.
    pub fn new(value: &str) -> Result<Self> {
        let ok = !value.is_empty()
            && value.len() <= 64
            && value.bytes().all(|b| {
                b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b':' | b'/' | b'-')
            });
        if ok {
            Ok(Self(value.to_owned()))
        } else {
            Err(super::invalid(
                "Operation identity must be 1 to 64 characters of letters, digits and . _ : / -.",
            ))
        }
    }

    /// The identity text.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Whether the caller needs a publication to be attested by the private journal.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Attest {
    /// Ordinary mutation: if the journal cannot admit the entry the write still happens and the event
    /// says it is untracked.
    Optional,
    /// The write happens only if its journal entry is retained first (needs an operation identity);
    /// otherwise `attestation_unavailable` and no effect.
    Required,
}

/// One guarded creation or replacement.
pub struct Publish<'a> {
    /// Owned relative path; no links, parents must exist.
    pub relative: &'a str,
    /// Exact bytes, any content, never normalized.
    pub bytes: &'a [u8],
    /// `None` creates without clobber; `Some` replaces only if the current bytes equal these.
    pub observed: Option<&'a [u8]>,
    /// Byte cap for this file, `bytes.len() <= cap <= ABSOLUTE_CAP`.
    pub cap: usize,
    /// Caller operation identity.
    pub operation: Option<&'a OperationId>,
    /// Attestation requirement; `Required` needs `operation`.
    pub attest: Attest,
}

/// One guarded removal.
pub struct Remove<'a> {
    /// Owned relative path.
    pub relative: &'a str,
    /// The file is removed only if its current bytes equal these.
    pub observed: &'a [u8],
    /// Byte cap used to read the current file.
    pub cap: usize,
    /// Caller operation identity.
    pub operation: Option<&'a OperationId>,
    /// Attestation requirement; `Required` needs `operation`.
    pub attest: Attest,
}

/// Exact observed owned file: bytes plus the root-bound version of exactly those bytes.
pub struct Observed {
    /// Owned relative path.
    pub relative: String,
    /// The file's exact bytes.
    pub bytes: Vec<u8>,
    /// Root-bound version of `bytes`.
    pub version: String,
}

/// Kind of one directory entry; links are reported, never followed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EntryKind {
    /// Regular file.
    File,
    /// Directory.
    Directory,
    /// Symbolic link.
    Symlink,
    /// Anything else.
    Other,
}

/// One directory entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DirEntry {
    /// File name (lossy UTF-8).
    pub name: String,
    /// Entry kind without following links.
    pub kind: EntryKind,
}

/// Bounded, non-recursive listing of one owned directory, sorted by name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DirListing {
    /// Entries kept, sorted by name.
    pub entries: Vec<DirEntry>,
    /// False when the cap cut the listing.
    pub complete: bool,
}

/// What kind of filesystem effect an event records.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum EffectKind {
    /// A new file.
    Created,
    /// A replaced file.
    Replaced,
    /// A removed file.
    Removed,
    /// A directory this call created.
    DirectoryCreated,
    /// A directory that already existed.
    DirectoryExisting,
}

/// Whether a visible effect reached durable storage.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Durability {
    /// The effect and its parent directory entry were synced.
    Durable,
    /// The effect is visible but syncing its parent failed; never claimed durable.
    SyncUnknown,
}

/// Why an eligible file effect has no journal entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum UntrackedReason {
    /// The root is not an independent repository.
    NoRepository,
    /// The journal had no room even after pruning verified committed intents.
    JournalFull,
    /// The journal could not be written.
    JournalUnavailable,
    /// The journal is unreadable or corrupt; it is kept as evidence and never rewritten.
    JournalCorrupt,
}

/// Whether the private journal holds an entry for an effect.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Tracking {
    /// An entry was written before publication and confirmed after it.
    Tracked,
    /// An optional write proceeded without an entry; never certifiable later.
    Untracked(UntrackedReason),
    /// An entry was written but confirming it failed after the file became visible.
    UnknownAfterPublication,
    /// A directory or built-in administrative path that never receives a file journal entry.
    NotApplicable,
}

/// Plain record of one filesystem effect; contains no file content.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Publication {
    /// Owned relative path.
    pub relative: String,
    /// Effect kind.
    pub kind: EffectKind,
    /// Store version of the replaced or removed bytes; `None` for a create or a directory.
    pub before: Option<String>,
    /// Store version of the new bytes; `None` for a removal or a directory.
    pub after: Option<String>,
    /// Durability of the visible effect.
    pub durability: Durability,
    /// Caller operation identity as given.
    pub operation: Option<String>,
    /// Journal intent identity; `None` when untracked or not applicable.
    pub intent: Option<String>,
    /// Journal tracking state.
    pub tracking: Tracking,
}

/// Built-in administrative paths that never receive a file journal entry: ignored normalization
/// backups, lock files and this store's own publication temps. Decided from a fixed list, never by Git.
pub fn not_applicable(relative: &str) -> bool {
    let name = relative.rsplit('/').next().unwrap_or(relative);
    relative.starts_with(".agent-tasks/backups/")
        || relative.starts_with(".git/")
        || (relative.starts_with(".agent-tasks/") && name.ends_with(".lock"))
        || super::own_temp_name(name)
}

/// A staged temp file that is removed on drop unless it was placed.
pub(crate) struct Staged {
    /// Temp file path in the target's directory.
    temp: PathBuf,
    /// Device and inode of the staged file where the platform exposes them.
    pub witness: Option<[u64; 2]>,
}

impl Drop for Staged {
    /// Remove the temp file; after a successful rename it is already gone.
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.temp);
    }
}

/// Device and inode of `path`, where available.
#[cfg(unix)]
fn witness_of(path: &Path) -> Option<[u64; 2]> {
    use std::os::unix::fs::MetadataExt;
    fs::metadata(path).ok().map(|m| [m.dev(), m.ino()])
}

/// Device and inode of `path`, where available.
#[cfg(not(unix))]
fn witness_of(_path: &Path) -> Option<[u64; 2]> {
    None
}

impl Store {
    /// Refuse a primitive that runs without the cooperative write lock acquired through this Store.
    fn require_locked(&self) -> Result<()> {
        if self.state.locked.load(Ordering::SeqCst) {
            Ok(())
        } else {
            Err(Error::new(
                "not_locked",
                "The root write lock was not acquired through this Store in this call.",
            ))
        }
    }

    /// Append one typed event to this request's ledger.
    fn record(&self, event: Publication) {
        if let Ok(mut events) = self.state.events.lock() {
            events.push(event);
        }
    }

    /// Every typed event this request has produced, in order, including events of calls that later failed.
    pub fn publications(&self) -> Vec<Publication> {
        self.state
            .events
            .lock()
            .map(|events| events.clone())
            .unwrap_or_default()
    }

    /// Record a directory that legacy code already created as a not-applicable, durable event.
    pub(crate) fn record_directory(&self, relative: &str) {
        self.record(Publication {
            relative: relative.to_owned(),
            kind: EffectKind::DirectoryCreated,
            before: None,
            after: None,
            durability: Durability::Durable,
            operation: None,
            intent: None,
            tracking: Tracking::NotApplicable,
        });
    }

    /// The journal intent of this request, created on first use by `make`.
    pub(crate) fn call_intent_or_new(&self, make: impl FnOnce() -> String) -> String {
        match self.state.intent.lock() {
            Ok(mut slot) => slot.get_or_insert_with(make).clone(),
            Err(_) => make(),
        }
    }

    /// Read one owned regular file with a caller cap; `None` means absent. No filesystem effect.
    ///
    /// # Errors
    /// `capacity` above the cap, `file_type` for links and non-regular files, `invalid` for a cap above
    /// [`ABSOLUTE_CAP`], and the path errors of [`Store::path`].
    pub fn read_exact(&self, relative: &str, cap: usize) -> Result<Option<Observed>> {
        if cap > ABSOLUTE_CAP {
            return Err(Error::new(
                "capacity",
                "Requested cap exceeds the absolute cap.",
            ));
        }
        Ok(
            read_path(&self.path(relative)?, cap)?.map(|bytes| Observed {
                relative: relative.to_owned(),
                version: self.version(relative, Some(&bytes)),
                bytes,
            }),
        )
    }

    /// Write `bytes` to an exclusive temp file next to the target and sync it.
    pub(crate) fn stage(&self, target: &Path, bytes: &[u8]) -> Result<Staged> {
        let parent = target
            .parent()
            .ok_or_else(|| Error::new("path", "Record has no parent."))?;
        let name = target
            .file_name()
            .and_then(|s| s.to_str())
            .ok_or_else(|| Error::new("path", "Invalid record name."))?;
        let temp = parent.join(format!(
            ".{name}.tmp-{}-{}",
            std::process::id(),
            TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed)
        ));
        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = options
            .open(&temp)
            .map_err(|_| Error::new("io", "Cannot exclusively create publication temp."))?;
        let staged = Staged {
            witness: witness_of(&temp),
            temp,
        };
        file.write_all(bytes)
            .and_then(|_| file.sync_all())
            .map_err(|_| Error::new("io", "Cannot write/sync candidate; work not published."))?;
        Ok(staged)
    }

    /// Recheck the observed bytes, then link (create) or rename (replace) the staged file into place.
    ///
    /// Returns whether the parent sync succeeded; a failed sync leaves a visible publication.
    pub(crate) fn place(
        &self,
        staged: &Staged,
        relative: &str,
        observed: Option<&[u8]>,
        cap: usize,
    ) -> Result<Durability> {
        let path = self.path(relative)?;
        if read_path(&path, cap.max(RECORD_CAP))?.as_deref() != observed {
            return Err(Error::new(
                "stale",
                "Observed bytes changed; work not published. Read context before retrying.",
            ));
        }
        if observed.is_some() {
            fs::rename(&staged.temp, &path)
                .map_err(|_| Error::new("io", "Atomic replacement failed."))?;
        } else {
            fs::hard_link(&staged.temp, &path).map_err(|_| {
                Error::new(
                    "publication",
                    "No-clobber publication failed; no overwrite fallback.",
                )
            })?;
        }
        let parent = path.parent().unwrap_or(&self.root);
        Ok(if sync_parent(parent).is_ok() {
            Durability::Durable
        } else {
            Durability::SyncUnknown
        })
    }

    /// Publish without events or journal; used for the journal file itself.
    pub(crate) fn publish_raw(
        &self,
        relative: &str,
        bytes: &[u8],
        observed: Option<&[u8]>,
        cap: usize,
    ) -> Result<()> {
        let path = self.path(relative)?;
        let staged = self.stage(&path, bytes)?;
        self.place(&staged, relative, observed, cap).map(|_| ())
    }

    /// Create or replace one owned file with guarded, capped, journaled publication.
    ///
    /// Create is no-clobber; replace succeeds only while the current bytes equal `observed`. Eligible
    /// files are retained in the private journal first. With [`Attest::Optional`] a journal that cannot
    /// admit the entry never stops the write (the event is `Untracked`); with [`Attest::Required`] the
    /// call refuses with `attestation_unavailable` before any effect. A failure to confirm the entry after
    /// the file became visible is `UnknownAfterPublication` (optional) or `attestation_unknown` (required).
    ///
    /// # Errors
    /// `not_locked`, `capacity`, `invalid`, `stale`, `operation_repeat`, `attestation_unavailable`,
    /// `attestation_unknown`, `durability_unknown` (the event is present with `SyncUnknown`) and the
    /// path, I/O and publication errors of the original `publish`.
    pub fn publish_with(
        &self,
        request: Publish<'_>,
        effects: &mut Vec<String>,
    ) -> Result<Publication> {
        self.require_locked()?;
        self.publish_inner(request, effects)
    }

    /// Shared body of [`Store::publish_with`] and the legacy `publish`, which does not require the lock token.
    pub(crate) fn publish_inner(
        &self,
        request: Publish<'_>,
        effects: &mut Vec<String>,
    ) -> Result<Publication> {
        let Publish {
            relative,
            bytes,
            observed,
            cap,
            operation,
            attest,
        } = request;
        if cap > ABSOLUTE_CAP || bytes.len() > cap {
            return Err(Error::new("capacity", "Content exceeds its byte cap."));
        }
        if attest == Attest::Required && operation.is_none() {
            return Err(super::invalid(
                "Required attestation needs an operation identity.",
            ));
        }
        let path = self.path(relative)?;
        let current = read_path(&path, cap.max(RECORD_CAP))?;
        if current.as_deref() != observed {
            return Err(Error::new(
                "stale",
                "Observed bytes changed; work not published. Read context before retrying.",
            ));
        }
        let eligible = !not_applicable(relative);
        if let (true, Some(op)) = (eligible, operation)
            && journal::already_published(self, op.as_str(), relative)
        {
            return Err(Error::new(
                "operation_repeat",
                "This operation already published this path; ask for its effect status.",
            ));
        }
        let after_sha = sha256_hex(bytes);
        let staged = self.stage(&path, bytes)?;
        let kind = if observed.is_some() {
            EffectKind::Replaced
        } else {
            EffectKind::Created
        };
        let (mut tracking, mut intent) = (Tracking::NotApplicable, None);
        if eligible {
            let spec = Spec {
                relative,
                kind,
                operation: operation.map(OperationId::as_str),
                before_version: observed.map(|b| self.version(relative, Some(b))),
                before_sha256: observed.map(sha256_hex),
                after_sha256: Some(after_sha.clone()),
                after_len: Some(bytes.len() as u64),
                witness: staged.witness,
            };
            match journal::admit(self, &spec) {
                Admission::Tracked(id) => {
                    tracking = Tracking::Tracked;
                    intent = Some(id);
                }
                Admission::Untracked(_) if attest == Attest::Required => {
                    return Err(Error::new(
                        "attestation_unavailable",
                        "The pending journal cannot retain this operation; nothing was written.",
                    ));
                }
                Admission::Untracked(reason) => tracking = Tracking::Untracked(reason),
            }
        }
        let durability = match self.place(&staged, relative, observed, cap) {
            Ok(durability) => durability,
            Err(e) => {
                if let Some(id) = &intent {
                    journal::abandon(self, id, relative, Some(&after_sha));
                }
                return Err(e);
            }
        };
        effects.push(format!("Published {relative}."));
        let after = Some(self.version(relative, Some(bytes)));
        if let Some(id) = &intent
            && journal::mark_published(self, id, relative, Some(&after_sha), after.clone()).is_err()
        {
            tracking = Tracking::UnknownAfterPublication;
        }
        let event = Publication {
            relative: relative.to_owned(),
            kind,
            before: observed.map(|b| self.version(relative, Some(b))),
            after,
            durability,
            operation: operation.map(|o| o.as_str().to_owned()),
            intent,
            tracking,
        };
        self.record(event.clone());
        if tracking == Tracking::UnknownAfterPublication && attest == Attest::Required {
            return Err(Error::new(
                "attestation_unknown",
                format!(
                    "{relative} is visibly published; the journal could not confirm it. Inspect effect status; do not replay."
                ),
            ));
        }
        if durability == Durability::SyncUnknown {
            return Err(Error::new(
                "durability_unknown",
                format!(
                    "{relative} is visibly published; directory sync failed. Inspect current context; do not blindly replay."
                ),
            ));
        }
        Ok(event)
    }

    /// Remove one owned file only when its current bytes equal `request.observed`.
    ///
    /// The file is atomically detached to a hidden sibling, compared again, then unlinked, so a late
    /// native edit is never deleted. A changed detached file is renamed back with no-clobber semantics
    /// (`removal_conflict`); if the original name was taken meanwhile the sibling is left in place and
    /// named in the error. Journal admission follows the same rules as [`Store::publish_with`].
    ///
    /// # Errors
    /// `not_locked`, `capacity`, `invalid`, `stale`, `removal_conflict`, `attestation_unavailable`,
    /// `attestation_unknown`, `durability_unknown` and path or I/O errors.
    pub fn remove(&self, request: Remove<'_>, effects: &mut Vec<String>) -> Result<Publication> {
        self.require_locked()?;
        let Remove {
            relative,
            observed,
            cap,
            operation,
            attest,
        } = request;
        if cap > ABSOLUTE_CAP {
            return Err(Error::new(
                "capacity",
                "Requested cap exceeds the absolute cap.",
            ));
        }
        if attest == Attest::Required && operation.is_none() {
            return Err(super::invalid(
                "Required attestation needs an operation identity.",
            ));
        }
        let path = self.path(relative)?;
        if read_path(&path, cap.max(RECORD_CAP))?.as_deref() != Some(observed) {
            return Err(Error::new(
                "stale",
                "Observed bytes changed; nothing was removed. Read context before retrying.",
            ));
        }
        let before_sha = sha256_hex(observed);
        let eligible = !not_applicable(relative);
        let (mut tracking, mut intent) = (Tracking::NotApplicable, None);
        if eligible {
            let spec = Spec {
                relative,
                kind: EffectKind::Removed,
                operation: operation.map(OperationId::as_str),
                before_version: Some(self.version(relative, Some(observed))),
                before_sha256: Some(before_sha),
                after_sha256: None,
                after_len: None,
                witness: None,
            };
            match journal::admit(self, &spec) {
                Admission::Tracked(id) => {
                    tracking = Tracking::Tracked;
                    intent = Some(id);
                }
                Admission::Untracked(_) if attest == Attest::Required => {
                    return Err(Error::new(
                        "attestation_unavailable",
                        "The pending journal cannot retain this operation; nothing was removed.",
                    ));
                }
                Admission::Untracked(reason) => tracking = Tracking::Untracked(reason),
            }
        }
        let abandon = |this: &Self| {
            if let Some(id) = &intent {
                journal::abandon(this, id, relative, None);
            }
        };
        let parent = path.parent().unwrap_or(&self.root).to_path_buf();
        let name = path
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("record")
            .to_owned();
        let detached = parent.join(format!(
            ".{name}.rm-{}-{}",
            std::process::id(),
            TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed)
        ));
        if fs::rename(&path, &detached).is_err() {
            abandon(self);
            return Err(Error::new("io", "Cannot detach the file for removal."));
        }
        if read_path(&detached, cap.max(RECORD_CAP))?.as_deref() != Some(observed) {
            return match fs::hard_link(&detached, &path) {
                Ok(()) => {
                    let _ = fs::remove_file(&detached);
                    abandon(self);
                    Err(Error::new(
                        "removal_conflict",
                        "The file changed during removal; it was put back and nothing was removed.",
                    ))
                }
                Err(_) => Err(Error::new(
                    "removal_conflict",
                    format!(
                        "The file changed during removal and its name was taken; it remains at {}.",
                        detached
                            .file_name()
                            .and_then(|s| s.to_str())
                            .unwrap_or("a hidden sibling")
                    ),
                )),
            };
        }
        if fs::remove_file(&detached).is_err() {
            abandon(self);
            return Err(Error::new("io", "Cannot unlink the detached file."));
        }
        let durability = if sync_parent(&parent).is_ok() {
            Durability::Durable
        } else {
            Durability::SyncUnknown
        };
        effects.push(format!("Removed {relative}."));
        if let Some(id) = &intent {
            let version = None;
            if journal::mark_published(self, id, relative, None, version).is_err() {
                tracking = Tracking::UnknownAfterPublication;
            }
        }
        let event = Publication {
            relative: relative.to_owned(),
            kind: EffectKind::Removed,
            before: Some(self.version(relative, Some(observed))),
            after: None,
            durability,
            operation: operation.map(|o| o.as_str().to_owned()),
            intent,
            tracking,
        };
        self.record(event.clone());
        if tracking == Tracking::UnknownAfterPublication && attest == Attest::Required {
            return Err(Error::new(
                "attestation_unknown",
                format!(
                    "{relative} is removed; the journal could not confirm it. Inspect effect status; do not replay."
                ),
            ));
        }
        if durability == Durability::SyncUnknown {
            return Err(Error::new(
                "durability_unknown",
                format!(
                    "{relative} is removed; directory sync failed. Inspect current context; do not blindly replay."
                ),
            ));
        }
        Ok(event)
    }

    /// Create exactly one missing directory whose parent exists.
    ///
    /// An existing directory is a no-op event of kind `DirectoryExisting`; a file or link at the path
    /// refuses. Directory events are always [`Tracking::NotApplicable`].
    ///
    /// # Errors
    /// `not_locked`, `file_type`, `path`, `io` and `durability_unknown` (the event is present).
    pub fn create_dir(&self, relative: &str, effects: &mut Vec<String>) -> Result<Publication> {
        self.require_locked()?;
        let path = self.path(relative)?;
        let event = |kind, durability| Publication {
            relative: relative.to_owned(),
            kind,
            before: None,
            after: None,
            durability,
            operation: None,
            intent: None,
            tracking: Tracking::NotApplicable,
        };
        match fs::symlink_metadata(&path) {
            Ok(m) if m.is_dir() && !m.file_type().is_symlink() => {
                let existing = event(EffectKind::DirectoryExisting, Durability::Durable);
                self.record(existing.clone());
                return Ok(existing);
            }
            Ok(_) => {
                return Err(Error::new(
                    "file_type",
                    "A file or link occupies the directory path.",
                ));
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => (),
            Err(_) => return Err(Error::new("io", "Cannot inspect the directory path.")),
        }
        let parent = path.parent().unwrap_or(&self.root);
        if !parent.is_dir() {
            return Err(Error::new("path", "The parent directory does not exist."));
        }
        fs::create_dir(&path).map_err(|_| Error::new("io", "Cannot create the directory."))?;
        effects.push(format!("Created directory {relative}/."));
        let durability = if sync_parent(parent).is_ok() {
            Durability::Durable
        } else {
            Durability::SyncUnknown
        };
        let created = event(EffectKind::DirectoryCreated, durability);
        self.record(created.clone());
        if durability == Durability::SyncUnknown {
            return Err(Error::new(
                "durability_unknown",
                format!(
                    "{relative}/ is visibly created; directory sync failed. Inspect current context."
                ),
            ));
        }
        Ok(created)
    }

    /// Create the missing ancestors of `relative` below the root, at most four levels, each disclosed.
    ///
    /// # Errors
    /// `capacity` when more than four levels are missing, plus the errors of [`Store::create_dir`].
    pub fn ensure_parents(
        &self,
        relative: &str,
        effects: &mut Vec<String>,
    ) -> Result<Vec<Publication>> {
        self.require_locked()?;
        let parts: Vec<&str> = relative.split('/').collect();
        let mut missing = Vec::new();
        let mut prefix = String::new();
        for part in &parts[..parts.len().saturating_sub(1)] {
            if !prefix.is_empty() {
                prefix.push('/');
            }
            prefix.push_str(part);
            if !self.path(&prefix)?.exists() {
                missing.push(prefix.clone());
            }
        }
        if missing.len() > PARENT_LEVELS {
            return Err(Error::new(
                "capacity",
                "More than four parent directories are missing.",
            ));
        }
        missing
            .iter()
            .map(|dir| self.create_dir(dir, effects))
            .collect()
    }

    /// Bounded, non-recursive, name-sorted listing of one owned directory.
    ///
    /// An absent directory lists as empty and complete. Links are reported as links and never followed.
    ///
    /// # Errors
    /// `invalid` for a cap above [`LIST_CAP`], `file_type` when a file or link occupies the path, `io`.
    pub fn list_dir(&self, relative: &str, cap: usize) -> Result<DirListing> {
        if cap > LIST_CAP {
            return Err(super::invalid(
                "Listing cap exceeds the directory listing limit.",
            ));
        }
        let path = self.path(relative)?;
        match fs::symlink_metadata(&path) {
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                return Ok(DirListing {
                    entries: Vec::new(),
                    complete: true,
                });
            }
            Err(_) => return Err(Error::new("io", "Cannot inspect the directory.")),
            Ok(m) if !m.is_dir() || m.file_type().is_symlink() => {
                return Err(Error::new(
                    "file_type",
                    "A file or link occupies the directory path.",
                ));
            }
            Ok(_) => (),
        }
        let mut entries = Vec::new();
        let mut complete = true;
        for item in
            fs::read_dir(&path).map_err(|_| Error::new("io", "Cannot list the directory."))?
        {
            if entries.len() >= cap {
                complete = false;
                break;
            }
            let item = item.map_err(|_| Error::new("io", "Cannot read a directory entry."))?;
            let kind = match item.file_type() {
                Ok(t) if t.is_symlink() => EntryKind::Symlink,
                Ok(t) if t.is_dir() => EntryKind::Directory,
                Ok(t) if t.is_file() => EntryKind::File,
                _ => EntryKind::Other,
            };
            entries.push(DirEntry {
                name: item.file_name().to_string_lossy().into_owned(),
                kind,
            });
        }
        entries.sort_by(|a, b| a.name.cmp(&b.name));
        Ok(DirListing { entries, complete })
    }
}

/// Real-filesystem and real-Git regressions for the publication primitives.
#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "Explicit isolated fixture assertions"
)]
mod tests {
    use super::*;
    use crate::persist::{
        journal::{self, EntryPhase, Journal},
        testing::GitFixture,
    };

    /// Build an ordinary optional create/replace request with the record cap and no operation.
    fn request<'a>(relative: &'a str, bytes: &'a [u8], observed: Option<&'a [u8]>) -> Publish<'a> {
        Publish {
            relative,
            bytes,
            observed,
            cap: RECORD_CAP,
            operation: None,
            attest: Attest::Optional,
        }
    }

    /// A plain temporary root (no repository) with the owned directories prepared and locked.
    fn plain() -> (tempfile::TempDir, Store, super::super::LockGuard) {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::from_root(dir.path()).unwrap();
        store.prepare(&mut Vec::new()).unwrap();
        let guard = store.lock(true, &mut Vec::new()).unwrap().unwrap();
        (dir, store, guard)
    }

    /// Operation identities accept the documented alphabet and refuse everything else.
    #[test]
    fn operation_identity_grammar() {
        assert!(OperationId::new("cp:CP-001:r1:A01").is_ok());
        assert!(OperationId::new("a/b.c_d-e").is_ok());
        for bad in ["", "has space", "semi;colon", &"x".repeat(65), "é"] {
            assert!(OperationId::new(bad).is_err(), "{bad:?}");
        }
    }

    /// Primitives refuse without the root lock acquired through the same Store, and work with it.
    #[test]
    fn primitives_need_the_root_lock_from_the_same_store() {
        let (_dir, store, guard) = plain();
        assert!(guard.is_for(&store));
        drop(guard);
        let error = store
            .publish_with(request("modules/x.yaml", b"a", None), &mut Vec::new())
            .unwrap_err();
        assert_eq!(error.code, "not_locked");
        assert_eq!(
            store.create_dir("docs", &mut Vec::new()).unwrap_err().code,
            "not_locked"
        );
        let _guard = store.lock(true, &mut Vec::new()).unwrap().unwrap();
        store
            .publish_with(request("modules/x.yaml", b"a", None), &mut Vec::new())
            .unwrap();
        let other = Store::from_root(store.root.as_path()).unwrap();
        assert!(!_guard.is_for(&other));
    }

    /// Create is no-clobber, replace needs the exact bytes, caps are enforced, nothing is saved on refusal.
    #[test]
    fn create_replace_stale_and_cap() {
        let (_dir, store, _guard) = plain();
        let mut fx = Vec::new();
        store
            .publish_with(request("a.md", b"one", None), &mut fx)
            .unwrap();
        assert_eq!(fx, ["Published a.md."]);
        assert_eq!(
            store
                .publish_with(request("a.md", b"two", None), &mut fx)
                .unwrap_err()
                .code,
            "stale"
        );
        assert_eq!(
            store
                .publish_with(request("a.md", b"two", Some(b"other")), &mut fx)
                .unwrap_err()
                .code,
            "stale"
        );
        let replaced = store
            .publish_with(request("a.md", b"two", Some(b"one")), &mut fx)
            .unwrap();
        assert_eq!(replaced.kind, EffectKind::Replaced);
        assert_eq!(std::fs::read(store.root.join("a.md")).unwrap(), b"two");
        let mut big = request("b.md", &[0u8; 11], None);
        big.cap = 10;
        assert_eq!(
            store.publish_with(big, &mut fx).unwrap_err().code,
            "capacity"
        );
        assert!(!store.root.join("b.md").exists());
        let mut wide = request("c.md", b"x", None);
        wide.cap = ABSOLUTE_CAP + 1;
        assert_eq!(
            store.publish_with(wide, &mut fx).unwrap_err().code,
            "capacity"
        );
        let mut required = request("d.md", b"x", None);
        required.attest = Attest::Required;
        assert_eq!(
            store.publish_with(required, &mut fx).unwrap_err().code,
            "invalid_data"
        );
        assert_eq!(store.publications().len(), 2);
    }

    /// Without a repository an optional write is saved and untracked; a required write refuses first.
    #[test]
    fn untracked_optional_and_refused_required_without_repository() {
        let (_dir, store, _guard) = plain();
        let mut fx = Vec::new();
        let event = store
            .publish_with(request("a.md", b"one", None), &mut fx)
            .unwrap();
        assert_eq!(
            event.tracking,
            Tracking::Untracked(UntrackedReason::NoRepository)
        );
        assert!(event.intent.is_none());
        let op = OperationId::new("op:1").unwrap();
        let mut required = request("b.md", b"two", None);
        required.operation = Some(&op);
        required.attest = Attest::Required;
        let error = store.publish_with(required, &mut fx).unwrap_err();
        assert_eq!(error.code, "attestation_unavailable");
        assert!(!store.root.join("b.md").exists());
    }

    /// A tracked write leaves a Published journal entry with the planned digest and one shared intent.
    #[test]
    fn tracked_publication_is_journaled_before_and_confirmed_after() {
        let f = GitFixture::new();
        let _guard = f.lock();
        let mut fx = Vec::new();
        let first = f
            .store
            .publish_with(request("modules/M-001.yaml", b"one", None), &mut fx)
            .unwrap();
        let second = f
            .store
            .publish_with(request("modules/M-002.yaml", b"two", None), &mut fx)
            .unwrap();
        assert_eq!(first.tracking, Tracking::Tracked);
        assert_eq!(first.intent, second.intent);
        let (j, _) = journal::load(&f.store).unwrap();
        assert_eq!(j.intents.len(), 1);
        let entries = &j.intents[0].entries;
        assert_eq!(entries.len(), 2);
        assert!(entries.iter().all(|e| e.phase == EntryPhase::Published));
        assert_eq!(
            entries[0].after_sha256.as_deref(),
            Some(journal::sha256_hex(b"one").as_str())
        );
        assert_eq!(f.store.publications().len(), 2);
    }

    /// Directories, ignored backups and locks are not applicable: truthful events, never journaled.
    #[test]
    fn directories_and_admin_paths_are_not_applicable() {
        let f = GitFixture::new();
        let _guard = f.lock();
        let mut fx = Vec::new();
        let dir = f.store.create_dir("decisions", &mut fx).unwrap();
        assert_eq!(
            (dir.kind, dir.tracking),
            (EffectKind::DirectoryCreated, Tracking::NotApplicable)
        );
        assert_eq!(
            f.store.create_dir("decisions", &mut fx).unwrap().kind,
            EffectKind::DirectoryExisting
        );
        let backup = f.store.publish_with(
            request(".agent-tasks/backups/x.yaml", b"old", None),
            &mut fx,
        );
        assert_eq!(
            backup.unwrap_err().code,
            "io",
            "backups/ does not exist yet"
        );
        f.store.create_dir(".agent-tasks/backups", &mut fx).unwrap();
        let backup = f
            .store
            .publish_with(
                request(".agent-tasks/backups/x.yaml", b"old", None),
                &mut fx,
            )
            .unwrap();
        assert_eq!(backup.tracking, Tracking::NotApplicable);
        let record = f
            .store
            .publish_with(request("decisions/D-001.yaml", b"new", None), &mut fx)
            .unwrap();
        assert_eq!(record.tracking, Tracking::Tracked);
        let (j, _) = journal::load(&f.store).unwrap();
        let paths: Vec<_> = j
            .intents
            .iter()
            .flat_map(|i| i.entries.iter().map(|e| e.path.as_str()))
            .collect();
        assert_eq!(paths, ["decisions/D-001.yaml"]);
    }

    /// With a journal that has no room an optional write proceeds untracked and leaves old entries
    /// byte-identical; a required write refuses before any effect.
    #[test]
    fn full_journal_never_stops_optional_writes_and_blocks_required_ones() {
        let f = GitFixture::new();
        let _guard = f.lock();
        let mut full = Journal::default();
        for n in 0..journal::MAX_INTENTS {
            full.intents.push(journal::Intent {
                id: format!("PG-{n:024x}"),
                created_at: "2026-10-07T00:00:00.000Z".into(),
                class: None,
                refs: Vec::new(),
                outcome: journal::IntentOutcome::Success,
                adopted: false,
                committing_from: None,
                committed: None,
                entries: vec![journal::Entry {
                    path: format!("modules/M-{n:03}.yaml"),
                    kind: EffectKind::Created,
                    operation: None,
                    phase: EntryPhase::Published,
                    before_version: None,
                    before_sha256: None,
                    after_sha256: Some(journal::sha256_hex(b"x")),
                    after_len: Some(1),
                    after_version: None,
                    witness: None,
                    superseded_by: None,
                }],
            });
        }
        journal::save(&f.store, &full, None).unwrap();
        let before = std::fs::read(f.store.root.join(journal::JOURNAL_PATH)).unwrap();
        let mut fx = Vec::new();
        let event = f
            .store
            .publish_with(request("a.md", b"one", None), &mut fx)
            .unwrap();
        assert_eq!(
            event.tracking,
            Tracking::Untracked(UntrackedReason::JournalFull)
        );
        assert_eq!(std::fs::read(f.store.root.join("a.md")).unwrap(), b"one");
        let op = OperationId::new("op:1").unwrap();
        let mut required = request("b.md", b"two", None);
        required.operation = Some(&op);
        required.attest = Attest::Required;
        assert_eq!(
            f.store.publish_with(required, &mut fx).unwrap_err().code,
            "attestation_unavailable"
        );
        assert!(!f.store.root.join("b.md").exists());
        assert_eq!(
            std::fs::read(f.store.root.join(journal::JOURNAL_PATH)).unwrap(),
            before
        );
    }

    /// A corrupt journal is kept as evidence and never rewritten; optional writes continue untracked.
    #[test]
    fn corrupt_journal_is_kept_and_does_not_stop_writes() {
        let f = GitFixture::new();
        let _guard = f.lock();
        std::fs::create_dir_all(f.store.root.join(".git/agent-tasks")).unwrap();
        std::fs::write(f.store.root.join(journal::JOURNAL_PATH), b"not: [valid").unwrap();
        let event = f
            .store
            .publish_with(request("a.md", b"one", None), &mut Vec::new())
            .unwrap();
        assert_eq!(
            event.tracking,
            Tracking::Untracked(UntrackedReason::JournalCorrupt)
        );
        assert_eq!(
            std::fs::read(f.store.root.join(journal::JOURNAL_PATH)).unwrap(),
            b"not: [valid"
        );
    }

    /// The same operation cannot publish the same path twice while the journal still holds it.
    #[test]
    fn repeated_operation_and_path_is_refused() {
        let f = GitFixture::new();
        let _guard = f.lock();
        let op = OperationId::new("op:1").unwrap();
        let mut first = request("a.md", b"one", None);
        first.operation = Some(&op);
        first.attest = Attest::Required;
        f.store.publish_with(first, &mut Vec::new()).unwrap();
        let mut again = request("a.md", b"two", Some(b"one"));
        again.operation = Some(&op);
        assert_eq!(
            f.store
                .publish_with(again, &mut Vec::new())
                .unwrap_err()
                .code,
            "operation_repeat"
        );
        assert_eq!(std::fs::read(f.store.root.join("a.md")).unwrap(), b"one");
    }

    /// Removal needs the exact bytes, is journaled, and a stale attempt removes nothing.
    #[test]
    fn remove_is_guarded_and_journaled() {
        let f = GitFixture::new();
        let _guard = f.lock();
        let mut fx = Vec::new();
        f.store
            .publish_with(request("a.md", b"one", None), &mut fx)
            .unwrap();
        let wrong = Remove {
            relative: "a.md",
            observed: b"other",
            cap: RECORD_CAP,
            operation: None,
            attest: Attest::Optional,
        };
        assert_eq!(f.store.remove(wrong, &mut fx).unwrap_err().code, "stale");
        assert!(f.store.root.join("a.md").exists());
        let event = f
            .store
            .remove(
                Remove {
                    relative: "a.md",
                    observed: b"one",
                    cap: RECORD_CAP,
                    operation: None,
                    attest: Attest::Optional,
                },
                &mut fx,
            )
            .unwrap();
        assert_eq!(
            (event.kind, event.tracking),
            (EffectKind::Removed, Tracking::Tracked)
        );
        assert!(!f.store.root.join("a.md").exists());
        assert!(fx.contains(&"Removed a.md.".to_owned()));
        let leftovers: Vec<_> = std::fs::read_dir(&f.store.root)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .filter(|n| super::super::own_temp_name(n))
            .collect();
        assert!(leftovers.is_empty(), "{leftovers:?}");
    }

    /// A visible publication whose parent sync fails reports durability unknown and keeps the event.
    #[test]
    fn failed_parent_sync_is_reported_not_durable() {
        let (_dir, store, _guard) = plain();
        super::super::fail_next_directory_sync();
        let error = store
            .publish_with(request("a.md", b"one", None), &mut Vec::new())
            .unwrap_err();
        assert_eq!(error.code, "durability_unknown");
        assert_eq!(std::fs::read(store.root.join("a.md")).unwrap(), b"one");
        assert_eq!(store.publications()[0].durability, Durability::SyncUnknown);
        super::super::fail_next_directory_sync();
        let error = store.create_dir("docs", &mut Vec::new()).unwrap_err();
        assert_eq!(error.code, "durability_unknown");
        assert_eq!(store.publications()[1].durability, Durability::SyncUnknown);
    }

    /// Parent creation is bounded to four levels and listing is sorted, bounded and link-aware.
    #[test]
    fn parents_and_listing_are_bounded() {
        let (_dir, store, _guard) = plain();
        let mut fx = Vec::new();
        let made = store.ensure_parents("docs/a/b/file.md", &mut fx).unwrap();
        assert_eq!(made.len(), 3);
        assert_eq!(
            store
                .ensure_parents("a/b/c/d/e/f.md", &mut fx)
                .unwrap_err()
                .code,
            "capacity"
        );
        std::fs::write(store.root.join("docs/z.md"), b"z").unwrap();
        #[cfg(unix)]
        std::os::unix::fs::symlink("z.md", store.root.join("docs/link.md")).unwrap();
        let listing = store.list_dir("docs", 10).unwrap();
        let names: Vec<_> = listing.entries.iter().map(|e| e.name.as_str()).collect();
        assert!(listing.complete);
        assert_eq!(names.first(), Some(&"a"));
        #[cfg(unix)]
        assert!(
            listing
                .entries
                .iter()
                .any(|e| e.name == "link.md" && e.kind == EntryKind::Symlink)
        );
        assert!(!store.list_dir("docs", 1).unwrap().complete);
        assert!(store.list_dir("absent", 10).unwrap().entries.is_empty());
        assert_eq!(
            store.list_dir("docs/z.md", 10).unwrap_err().code,
            "file_type"
        );
        assert_eq!(
            store.list_dir("docs", LIST_CAP + 1).unwrap_err().code,
            "invalid_data"
        );
    }

    /// The shared inventory keeps the work directories exact and gives new homes the stem rule.
    #[test]
    fn inventory_grammar_for_work_and_new_homes() {
        let (_dir, store, _guard) = plain();
        let root = &store.root;
        std::fs::write(root.join("epics/.E-001.yaml.tmp-1-1"), b"").unwrap();
        let epics = store.kind_inventory("epics", "E-").unwrap();
        assert!(
            !epics.complete,
            "an orphan epic temp stays incomplete exactly as before"
        );
        std::fs::write(root.join("modules/.M-001.yaml.tmp-1-1"), b"").unwrap();
        let modules = store.kind_inventory("modules", "M-").unwrap();
        assert!(modules.complete && modules.warnings.len() == 1);
        std::fs::create_dir(root.join("decisions")).unwrap();
        std::fs::write(root.join("decisions/D-001.yaml"), b"").unwrap();
        std::fs::write(root.join("decisions/.D-002.yaml.tmp-3-4"), b"").unwrap();
        std::fs::write(root.join("decisions/.D-003.yaml.rm-3-5"), b"").unwrap();
        let home = store.kind_inventory("decisions", "D-").unwrap();
        assert!(home.complete);
        assert_eq!(home.ids, ["D-001"]);
        assert_eq!(home.warnings.len(), 2);
        std::fs::write(root.join("decisions/.R-002.yaml.tmp-3-4"), b"").unwrap();
        assert!(!store.kind_inventory("decisions", "D-").unwrap().complete);
        assert!(super::super::own_temp_name(".notes.md.tmp-12-3"));
        assert!(!super::super::own_temp_name(".foreign.swp"));
        assert!(!super::super::own_temp_name(".x.tmp-a-3"));
    }
}
