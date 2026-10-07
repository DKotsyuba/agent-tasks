//! Committed-bytes proof and stable locators.
//!
//! A pending, deferred, ignored or working-tree file is never proof. The proof names a commit that is
//! reachable from the attached branch HEAD and holds blob bytes whose SHA-256 and length equal the
//! caller's item. All Git is read-only, bounded and never fetches; an unreadable or oversize object is
//! `Unknown`, never `Committed`.
use super::{git, journal::sha256_hex, proof::reachable};
use crate::store::{Error, Result, Store};
use sha2::{Digest, Sha256};
use std::time::Instant;

/// Longest locator wire form.
pub const LOCATOR_CAP: usize = 256;

/// A committed object that holds exact bytes, reachable from the current branch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Locator {
    /// Full commit id.
    pub commit: String,
    /// Full blob id.
    pub blob: String,
    /// Owned relative path.
    pub relative: String,
    /// SHA-256 of the bytes.
    pub sha256: String,
    /// Length of the bytes.
    pub len: u64,
}

/// A parsed locator wire form; the digest and length are re-derived by reading the object.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LocatorRef {
    /// Full commit id.
    pub commit: String,
    /// Full blob id.
    pub blob: String,
    /// Owned relative path.
    pub relative: String,
}

/// Percent-encode every byte outside `A-Za-z0-9._/-`.
fn encode_path(path: &str) -> String {
    super::engine::encode_path(path)
}

/// Reverse of [`encode_path`].
fn decode_path(text: &str) -> Option<String> {
    let bytes = text.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' {
            let hex = text.get(i + 1..i + 3)?;
            out.push(u8::from_str_radix(hex, 16).ok()?);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8(out).ok()
}

/// Whether `text` is a lowercase object id of the two Git hash lengths.
fn object_id(text: &str) -> bool {
    (text.len() == 40 || text.len() == 64)
        && text
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

impl Locator {
    /// Stable wire form `git1:<commit>:<blob>:<path with %XX>` of at most 256 bytes.
    ///
    /// # Errors
    /// `locator_too_long` rather than truncating.
    pub fn encode(&self) -> Result<String> {
        let text = format!(
            "git1:{}:{}:{}",
            self.commit,
            self.blob,
            encode_path(&self.relative)
        );
        if text.len() > LOCATOR_CAP {
            return Err(Error::new(
                "locator_too_long",
                "The locator would exceed 256 bytes.",
            ));
        }
        Ok(text)
    }

    /// Parse a wire form.
    ///
    /// # Errors
    /// `invalid_data` for any other shape.
    pub fn parse(text: &str) -> Result<LocatorRef> {
        let bad = || crate::store::invalid("Not a committed-bytes locator.");
        let rest = text.strip_prefix("git1:").ok_or_else(bad)?;
        let mut parts = rest.splitn(3, ':');
        let (commit, blob, path) = (
            parts.next().ok_or_else(bad)?,
            parts.next().ok_or_else(bad)?,
            parts.next().ok_or_else(bad)?,
        );
        if !object_id(commit) || !object_id(blob) || path.is_empty() {
            return Err(bad());
        }
        Ok(LocatorRef {
            commit: commit.to_owned(),
            blob: blob.to_owned(),
            relative: decode_path(path).ok_or_else(bad)?,
        })
    }
}

/// One original to prove.
#[derive(Debug, Clone)]
pub struct Item<'a> {
    /// Owned relative path.
    pub relative: &'a str,
    /// SHA-256 of the bytes to prove.
    pub sha256: &'a str,
    /// Length of the bytes to prove.
    pub len: u64,
    /// Prove in this commit instead of the last commit touching the path.
    pub at: Option<&'a str>,
}

/// Why bytes are not proven committed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NotCommittedReason {
    /// No commit holds the path.
    Untracked,
    /// The path is untracked and ignored by the user's rules.
    Ignored,
    /// HEAD holds different bytes while the working file equals the item.
    Dirty,
    /// HEAD names no commit.
    UnbornHead,
    /// HEAD is detached.
    DetachedHead,
    /// `.git` is a file or the root is nested.
    NotIndependentRepository,
    /// No repository.
    NoRepository,
}

/// Result of proving one original.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Proof {
    /// The bytes hash to the item's digest in a commit reachable from the attached HEAD.
    Committed(Locator),
    /// Not proven committed, with the reason.
    NotCommitted(NotCommittedReason),
    /// Tracked, but the committed bytes differ and the working file differs from the item too.
    Mismatch {
        /// SHA-256 of the committed bytes.
        committed_sha256: String,
    },
    /// A timeout, oversize or unreadable object; never treated as proof.
    Unknown(String),
}

/// Prove one original by digest and length. Read-only, bounded, infallible.
pub fn committed_original(store: &Store, item: &Item<'_>) -> Proof {
    let deadline = Instant::now() + git::COMMAND_TIME;
    let git_dir = store.root.join(".git");
    match std::fs::symlink_metadata(&git_dir) {
        Ok(m) if m.is_dir() && !m.file_type().is_symlink() => (),
        Ok(_) => return Proof::NotCommitted(NotCommittedReason::NotIndependentRepository),
        Err(_) => return Proof::NotCommitted(NotCommittedReason::NoRepository),
    }
    let head = std::fs::read_to_string(git_dir.join("HEAD")).unwrap_or_default();
    if head.trim().strip_prefix("ref: ").is_none() {
        return Proof::NotCommitted(NotCommittedReason::DetachedHead);
    }
    let run = |args: &[&str]| git::read_text(&store.root, args, deadline);
    if run(&["rev-parse", "--verify", "-q", "HEAD"]).is_none() {
        return Proof::NotCommitted(NotCommittedReason::UnbornHead);
    }
    let commit = match item.at {
        Some(at) if object_id(at) && reachable(store, at) => at.to_owned(),
        Some(_) => return Proof::NotCommitted(NotCommittedReason::Untracked),
        None => match run(&["rev-list", "-n", "1", "HEAD", "--", item.relative]) {
            Some(c) if object_id(&c) => c,
            _ => return untracked_or_ignored(store, item.relative, deadline),
        },
    };
    let object = format!("{commit}:{}", item.relative);
    let Some(blob) = run(&["rev-parse", "--verify", "-q", &object]).filter(|b| object_id(b)) else {
        return untracked_or_ignored(store, item.relative, deadline);
    };
    let size: u64 = match run(&["cat-file", "-s", &blob]).and_then(|s| s.parse().ok()) {
        Some(n) => n,
        None => return Proof::Unknown("The committed object cannot be read.".into()),
    };
    let committed_sha = if size > git::STDOUT_CAP as u64 {
        return Proof::Unknown("The committed object is larger than the bounded read.".into());
    } else {
        match git::run(&store.root, &["cat-file", "blob", &blob], None, deadline) {
            Ok(out) if out.success() && !out.truncated => {
                format!("{:x}", Sha256::digest(&out.stdout))
            }
            _ => return Proof::Unknown("The committed object cannot be read.".into()),
        }
    };
    if committed_sha == item.sha256 && size == item.len {
        return Proof::Committed(Locator {
            commit,
            blob,
            relative: item.relative.to_owned(),
            sha256: committed_sha,
            len: size,
        });
    }
    let working = store
        .read_exact(item.relative, crate::store::ABSOLUTE_CAP)
        .ok()
        .flatten()
        .map(|f| sha256_hex(&f.bytes));
    if working.as_deref() == Some(item.sha256) {
        Proof::NotCommitted(NotCommittedReason::Dirty)
    } else {
        Proof::Mismatch {
            committed_sha256: committed_sha,
        }
    }
}

/// A path no commit holds is ignored when the user's rules say so, otherwise untracked.
fn untracked_or_ignored(store: &Store, relative: &str, deadline: Instant) -> Proof {
    let ignored = git::run(
        &store.root,
        &["check-ignore", "-q", "--", relative],
        None,
        deadline,
    )
    .is_ok_and(|o| o.code == Some(0));
    Proof::NotCommitted(if ignored {
        NotCommittedReason::Ignored
    } else {
        NotCommittedReason::Untracked
    })
}

/// Prove several originals.
///
/// # Errors
/// `not_committed`, naming every relative path that is not `Committed`.
pub fn verify_committed(store: &Store, items: &[Item<'_>]) -> Result<Vec<Locator>> {
    let mut locators = Vec::new();
    let mut missing = Vec::new();
    for item in items {
        match committed_original(store, item) {
            Proof::Committed(locator) => locators.push(locator),
            _ => missing.push(item.relative.to_owned()),
        }
    }
    if missing.is_empty() {
        Ok(locators)
    } else {
        Err(Error::new(
            "not_committed",
            format!(
                "Not provably committed: {}.",
                crate::store::safe(&missing.join(", "), 400)
            ),
        ))
    }
}

/// The shape the knowledge history proof needs: `Some(locator)` only for exact committed bytes.
///
/// `version` must equal `store.version(relative, Some(bytes))`.
///
/// # Errors
/// `invalid_data` for a mismatching version and `locator_too_long` when the wire form would exceed 256 bytes.
pub fn locate_committed(
    store: &Store,
    relative: &str,
    version: &str,
    bytes: &[u8],
) -> Result<Option<String>> {
    if store.version(relative, Some(bytes)) != version {
        return Err(crate::store::invalid(
            "The version does not belong to those bytes.",
        ));
    }
    let digest = sha256_hex(bytes);
    match committed_original(
        store,
        &Item {
            relative,
            sha256: &digest,
            len: bytes.len() as u64,
            at: None,
        },
    ) {
        Proof::Committed(locator) => locator.encode().map(Some),
        _ => Ok(None),
    }
}

/// Read the exact bytes back from a locator after checking its commit is still reachable.
///
/// # Errors
/// `not_committed` when it is unreachable or does not hold that blob at that path, `capacity` above `cap`.
pub fn read_committed(store: &Store, locator: &str, cap: usize) -> Result<Vec<u8>> {
    let parsed = Locator::parse(locator)?;
    let deadline = Instant::now() + git::COMMAND_TIME;
    let gone = || {
        Error::new(
            "not_committed",
            "The locator is no longer reachable from the current branch.",
        )
    };
    if !reachable(store, &parsed.commit) {
        return Err(gone());
    }
    let held = git::read_text(
        &store.root,
        &[
            "rev-parse",
            "--verify",
            "-q",
            &format!("{}:{}", parsed.commit, parsed.relative),
        ],
        deadline,
    );
    if held.as_deref() != Some(parsed.blob.as_str()) {
        return Err(gone());
    }
    if cap > git::STDOUT_CAP {
        return Err(Error::new(
            "capacity",
            "The bounded read cannot exceed one mebibyte.",
        ));
    }
    let out = git::run(
        &store.root,
        &["cat-file", "blob", &parsed.blob],
        None,
        deadline,
    )
    .map_err(|_| gone())?;
    if !out.success() || out.truncated || out.stdout.len() > cap {
        return Err(Error::new(
            "capacity",
            "The committed object exceeds the requested cap.",
        ));
    }
    Ok(out.stdout)
}
