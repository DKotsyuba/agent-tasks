//! Settlement and the Git engine: policy, reconciliation, exact-path commits, verification and receipts.
//!
//! Settlement runs while the caller holds the single root write lock (witnessed by `&LockGuard`) and
//! never takes it. It never fails the call: every path returns a plain [`GitReceipt`]. A commit is made
//! through a temporary index so the user's real index, staged changes and hooks, signing and
//! configuration are untouched until the commit has landed; then only this engine's own paths are
//! refreshed in the real index. Equal bytes never certify ownership: only journal entries do.
use super::{
    git,
    journal::{self, Entry, EntryPhase, Intent, IntentOutcome, Journal},
    policy::{Decision, DeferReason, Event, EventOutcome, PendingFacts, Policy, PolicyInput},
    receipt::{Attention, EarlierCommit, GitOutcome, GitReceipt, PendingRef, Phase, Reason},
};
use crate::store::{
    ABSOLUTE_CAP, Durability, EffectKind, LockGuard, Store, Tracking, UntrackedReason,
};
use std::{
    collections::{BTreeMap, BTreeSet},
    time::Instant,
};

#[cfg(test)]
std::thread_local! {
    /// One-shot read failure after a real successful commit, isolated to the test thread.
    static UNAVAILABLE_VERIFICATION_HEAD: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// Make only the next post-commit HEAD verification unavailable in this test thread.
#[cfg(test)]
pub(super) fn fail_next_verification_head() {
    UNAVAILABLE_VERIFICATION_HEAD.with(|fault| fault.set(true));
}

/// Serialized commit message budget: whole intents only, never a truncated trailer.
pub const MESSAGE_BUDGET: usize = 64 * 1024;

/// What this request's own publications say about the current call.
#[derive(Debug, Default)]
pub struct CallFacts {
    /// The journal intent of this call, when any eligible effect was tracked.
    pub intent: Option<String>,
    /// Tracked eligible-file effects.
    pub tracked: usize,
    /// Paths of eligible-file effects without a journal entry, in order.
    pub untracked: Vec<String>,
    /// Paths of tracked effects, in order.
    pub tracked_paths: Vec<String>,
    /// Directory and administrative effects.
    pub not_applicable: usize,
    /// Some event has an uncertain sync or an unconfirmed journal entry.
    pub uncertain: bool,
    /// The reason of the first untracked effect.
    pub first_untracked: Option<UntrackedReason>,
}

impl CallFacts {
    /// Eligible owned-file effects of the call, tracked or not.
    pub fn eligible(&self) -> usize {
        self.tracked + self.untracked.len()
    }
}

/// Derive the current call's facts from the request-local event ledger.
pub fn call_facts(store: &Store) -> CallFacts {
    let mut facts = CallFacts::default();
    for event in store.publications() {
        if event.durability == Durability::SyncUnknown {
            facts.uncertain = true;
        }
        match event.tracking {
            Tracking::NotApplicable => facts.not_applicable += 1,
            Tracking::Tracked => {
                facts.tracked += 1;
                facts.tracked_paths.push(event.relative);
                facts.intent = facts.intent.or(event.intent);
            }
            Tracking::UnknownAfterPublication => {
                facts.uncertain = true;
                facts.tracked += 1;
                facts.tracked_paths.push(event.relative);
                facts.intent = facts.intent.or(event.intent);
            }
            Tracking::Untracked(reason) => {
                facts.first_untracked.get_or_insert(reason);
                facts.untracked.push(event.relative);
            }
        }
    }
    facts
}

/// SHA-256 of the current file: `Ok(None)` when absent, `Err` when it cannot be read.
fn file_sha(store: &Store, relative: &str) -> Result<Option<String>, ()> {
    store
        .read_exact(relative, ABSOLUTE_CAP)
        .map(|o| o.map(|f| journal::sha256_hex(&f.bytes)))
        .map_err(|_| ())
}

/// Reconciled state of one open intent against the files on disk.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum State {
    /// Every entry is published, the call succeeded and the files match: eligible.
    Eligible,
    /// Published but the call was partial or unsettled: waits for explicit recovery.
    Held,
    /// A commit attempt started and has not resolved.
    Committing,
    /// An entry was prepared and the file now equals the planned bytes without confirmation.
    Unknown,
    /// A file differs from every recorded state: native drift.
    Drifted,
}

impl State {
    /// The public phase for receipts and pending summaries.
    pub fn phase(self) -> Phase {
        match self {
            Self::Eligible => Phase::Published,
            Self::Held => Phase::Held,
            Self::Committing => Phase::Committing,
            Self::Unknown => Phase::Unknown,
            Self::Drifted => Phase::Drifted,
        }
    }
}

/// Classify one open intent. `Prepared` entries whose file still equals the recorded before-bytes are
/// unpublished intents and are ignored here (settlement removes them); chain-aware drift is decided by
/// [`analyze`].
pub fn classify(store: &Store, intent: &Intent) -> State {
    if intent.committing_from.is_some() {
        return State::Committing;
    }
    for entry in &intent.entries {
        let Ok(now) = file_sha(store, &entry.path) else {
            return State::Unknown;
        };
        if entry.phase == EntryPhase::Prepared {
            if now == entry.after_sha256 && entry.kind != EffectKind::Removed
                || (entry.kind == EffectKind::Removed && now.is_none())
            {
                return State::Unknown;
            }
            if now != entry.before_sha256 {
                return State::Drifted;
            }
        }
    }
    if intent.outcome == IntentOutcome::Success {
        State::Eligible
    } else {
        State::Held
    }
}

/// Everything the commit step needs about the selection.
#[derive(Debug, Default)]
pub struct Selection {
    /// Indices of intents to commit, in journal order.
    pub selected: Vec<usize>,
    /// Why an intent was not selected.
    pub deferred: BTreeMap<usize, Reason>,
    /// Intents with native drift.
    pub drifted: BTreeSet<usize>,
}

/// Decide which candidate intents may be committed together, with chain-aware drift detection.
///
/// Candidates are intents stamped `Success` with every entry published. For every path the entries of all
/// open intents form a chain in journal order that must be contiguous (each `before` equals the previous
/// `after`) and must end at the current bytes. A later entry on a path that belongs to a non-candidate
/// (held or unknown) intent blocks the candidates before it; a held intent before them does not.
/// Exclusions propagate to every candidate that shares a path with an excluded one.
pub fn analyze(store: &Store, journal: &Journal, current: Option<&str>) -> Selection {
    let mut sel = Selection::default();
    let states: Vec<State> = journal
        .intents
        .iter()
        .map(|i| {
            if i.committed.is_some() {
                State::Held
            } else {
                classify(store, i)
            }
        })
        .collect();
    let candidate =
        |n: usize| journal.intents[n].committed.is_none() && states[n] == State::Eligible;
    let mut chains: BTreeMap<&str, Vec<(usize, &Entry)>> = BTreeMap::new();
    for (n, intent) in journal.intents.iter().enumerate() {
        if intent.committed.is_some() {
            continue;
        }
        for entry in &intent.entries {
            if entry.phase == EntryPhase::Published && entry.superseded_by.is_none() {
                chains
                    .entry(entry.path.as_str())
                    .or_default()
                    .push((n, entry));
            }
        }
    }
    let mut excluded: BTreeMap<usize, Reason> = BTreeMap::new();
    for (path, chain) in &chains {
        let mut broken = false;
        for pair in chain.windows(2) {
            if pair[1].1.before_sha256 != pair[0].1.after_sha256 {
                broken = true;
            }
        }
        if let Some((_, last)) = chain.last() {
            match file_sha(store, path) {
                Ok(now) if now == last.after_sha256 => (),
                _ => broken = true,
            }
        }
        let first_candidate = chain.iter().position(|(n, _)| candidate(*n));
        let held_after =
            first_candidate.is_some_and(|f| chain[f..].iter().any(|(n, _)| !candidate(*n)));
        for (n, _) in chain {
            if broken {
                sel.drifted.insert(*n);
                excluded.entry(*n).or_insert(Reason::IncompleteOutcome);
            } else if held_after && candidate(*n) {
                excluded.entry(*n).or_insert(Reason::IncompleteOutcome);
            }
        }
    }
    for (n, state) in states.iter().enumerate() {
        if *state == State::Drifted {
            sel.drifted.insert(n);
            excluded.entry(n).or_insert(Reason::IncompleteOutcome);
        }
    }
    close_over_paths(
        journal,
        &chains_to_paths(&chains),
        &mut excluded,
        &candidate,
    );
    let _ = current;
    for n in 0..journal.intents.len() {
        if !candidate(n) {
            continue;
        }
        match excluded.get(&n) {
            Some(reason) => {
                sel.deferred.insert(n, *reason);
            }
            None => sel.selected.push(n),
        }
    }
    sel
}

/// Paths touched by each intent index, from the chain map.
fn chains_to_paths(
    chains: &BTreeMap<&str, Vec<(usize, &Entry)>>,
) -> BTreeMap<usize, BTreeSet<String>> {
    let mut paths: BTreeMap<usize, BTreeSet<String>> = BTreeMap::new();
    for (path, chain) in chains {
        for (n, _) in chain {
            paths.entry(*n).or_default().insert((*path).to_owned());
        }
    }
    paths
}

/// Propagate exclusions to every candidate that shares a path with an excluded candidate, to a fixpoint.
pub fn close_over_paths(
    journal: &Journal,
    paths: &BTreeMap<usize, BTreeSet<String>>,
    excluded: &mut BTreeMap<usize, Reason>,
    candidate: &dyn Fn(usize) -> bool,
) {
    loop {
        let mut grew = false;
        for n in 0..journal.intents.len() {
            if !candidate(n) || excluded.contains_key(&n) {
                continue;
            }
            let Some(mine) = paths.get(&n) else { continue };
            let hit = excluded.iter().find_map(|(m, reason)| {
                (paths.get(m).is_some_and(|theirs| !theirs.is_disjoint(mine))).then_some(*reason)
            });
            if let Some(reason) = hit {
                excluded.insert(n, reason);
                grew = true;
            }
        }
        if !grew {
            break;
        }
    }
}

/// Percent-encode every byte outside `A-Za-z0-9._/-` so spaces, colons and non-ASCII never split fields.
pub fn encode_path(path: &str) -> String {
    let mut out = String::with_capacity(path.len());
    for b in path.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'/' | b'-') {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

/// Trailer lines for one intent, with `into=` on effects replaced later inside the same commit.
fn trailers(journal: &Journal, selected: &[usize], n: usize) -> String {
    let intent = &journal.intents[n];
    let mut text = format!("Agent-Tasks-Intent: {}\n", intent.id);
    for entry in &intent.entries {
        let later = selected
            .iter()
            .filter(|m| **m > n)
            .flat_map(|m| journal.intents[*m].entries.iter())
            .rfind(|e| e.path == entry.path);
        let kind = match entry.kind {
            EffectKind::Created => "created",
            EffectKind::Replaced => "replaced",
            _ => "removed",
        };
        text.push_str(&format!(
            "Agent-Tasks-Effect: {} {kind} {} {} {}",
            entry.operation.as_deref().unwrap_or("-"),
            encode_path(&entry.path),
            entry.before_sha256.as_deref().unwrap_or("-"),
            entry.after_sha256.as_deref().unwrap_or("-"),
        ));
        if let Some(later) = later {
            text.push_str(&format!(
                " into={}",
                later.after_sha256.as_deref().unwrap_or("-")
            ));
        } else if let Some((_, digest)) = entry
            .superseded_by
            .as_deref()
            .and_then(|s| s.split_once(':'))
        {
            // A later commit already holds this path; its recorded image is the chain end.
            text.push_str(&format!(" into={digest}"));
        }
        text.push('\n');
    }
    if intent.adopted {
        text.push_str(&format!("Agent-Tasks-Adopted: {}\n", intent.id));
    }
    text
}

/// Repository facts needed before any commit attempt.
#[derive(Debug)]
pub struct RepoState {
    /// HEAD commit id the attempt builds on.
    pub parent: String,
}

/// Inspect the repository without writing; the first unsuitable state is the deferral reason.
pub fn inspect(store: &Store, deadline: Instant) -> Result<RepoState, Reason> {
    let git_dir = store.root.join(".git");
    match std::fs::symlink_metadata(&git_dir) {
        Ok(m) if m.is_dir() && !m.file_type().is_symlink() => (),
        Ok(_) => return Err(Reason::NotIndependentRepository),
        Err(_) => return Err(Reason::NoRepository),
    }
    let top = git::read_text(&store.root, &["rev-parse", "--show-toplevel"], deadline)
        .ok_or(Reason::GitUnavailable)?;
    if std::fs::canonicalize(&top).ok() != std::fs::canonicalize(&store.root).ok() {
        return Err(Reason::NotIndependentRepository);
    }
    let head = std::fs::read_to_string(git_dir.join("HEAD")).map_err(|_| Reason::NoRepository)?;
    if head.trim().strip_prefix("ref: ").is_none() {
        return Err(Reason::DetachedHead);
    }
    let parent = git::read_text(
        &store.root,
        &["rev-parse", "--verify", "-q", "HEAD"],
        deadline,
    )
    .ok_or(Reason::UnbornHead)?;
    for marker in [
        "MERGE_HEAD",
        "CHERRY_PICK_HEAD",
        "REVERT_HEAD",
        "BISECT_LOG",
        "rebase-merge",
        "rebase-apply",
    ] {
        if git_dir.join(marker).exists() {
            return Err(Reason::OperationInProgress);
        }
    }
    if git_dir.join("index.lock").exists() {
        return Err(Reason::IndexLocked);
    }
    match git::run(&store.root, &["ls-files", "-u", "-z"], None, deadline) {
        Ok(o) if o.success() && o.stdout.is_empty() => (),
        Ok(o) if o.success() => return Err(Reason::UnmergedPaths),
        _ => return Err(Reason::GitUnavailable),
    }
    Ok(RepoState { parent })
}

/// Split NUL-separated Git output into path strings.
fn nul_list(bytes: &[u8]) -> BTreeSet<String> {
    bytes
        .split(|b| *b == 0)
        .filter(|s| !s.is_empty())
        .map(|s| String::from_utf8_lossy(s).into_owned())
        .collect()
}

/// Paths staged in the real index that differ from HEAD.
fn staged_paths(store: &Store, deadline: Instant) -> Option<BTreeSet<String>> {
    let out = git::run(
        &store.root,
        &["diff", "--cached", "--name-only", "-z"],
        None,
        deadline,
    )
    .ok()?;
    out.success().then(|| nul_list(&out.stdout))
}

/// Untracked paths among `paths` that the user's own ignore rules would refuse to stage.
fn ignored_among(store: &Store, paths: &[String], deadline: Instant) -> Option<BTreeSet<String>> {
    let stdin: Vec<u8> = paths.iter().flat_map(|p| p.bytes().chain([0])).collect();
    let out = git::run(
        &store.root,
        &["check-ignore", "-z", "--stdin"],
        Some(&stdin),
        deadline,
    )
    .ok()?;
    matches!(out.code, Some(0) | Some(1)).then(|| nul_list(&out.stdout))
}

/// Result of one commit attempt for a set of intents.
pub enum Attempt {
    /// A commit landed with this id; attention flags describe verification and index refresh.
    Landed(String, Vec<Attention>),
    /// Git refused or did not complete; nothing landed.
    NotCompleted,
    /// A commit landed but its content, parent or trailers differ from the journal: not certified.
    Unverified(String, Vec<Attention>),
    /// The outcome is unknown (timeout or lost reply).
    Unknown,
}

/// Run `git` with an alternative index file.
fn run_indexed(
    store: &Store,
    index: &std::path::Path,
    args: &[&str],
    stdin: Option<&[u8]>,
    deadline: Instant,
) -> Result<git::Output, git::RunError> {
    git::run_env(
        &store.root,
        args,
        stdin,
        deadline,
        &[("GIT_INDEX_FILE", &index.to_string_lossy())],
    )
}

/// One path's exact state to stage: raw bytes stored without filters, or a removal.
struct Staged {
    /// Owned relative path.
    path: String,
    /// File mode, kept from HEAD when the path exists there.
    mode: String,
    /// Object id of the exact bytes; `None` removes the path.
    oid: Option<String>,
}

/// Store the exact recorded bytes of every path as raw blobs and describe the index entries to write.
///
/// Each file is read, compared with the recorded digest and hashed from the same bytes, so a late
/// native edit or a clean filter can never change what is committed. `Err` means nothing can be staged.
fn stage_exact(
    store: &Store,
    parent: &str,
    paths: &[String],
    expected: &BTreeMap<String, Option<String>>,
    deadline: Instant,
) -> Result<Vec<Staged>, ()> {
    let mut staged = Vec::new();
    for path in paths {
        let want = expected.get(path).ok_or(())?;
        let found = store.read_exact(path, ABSOLUTE_CAP).map_err(|_| ())?;
        match (want, found) {
            (None, None) => staged.push(Staged {
                path: path.clone(),
                mode: "0".into(),
                oid: None,
            }),
            (Some(digest), Some(file)) if journal::sha256_hex(&file.bytes) == *digest => {
                let mode = match super::verify::tree_entry(store, parent, path, deadline)? {
                    Some((mode, _)) => mode,
                    None => "100644".to_owned(),
                };
                let oid =
                    super::verify::raw_blob_id(store, &file.bytes, true, deadline).ok_or(())?;
                staged.push(Staged {
                    path: path.clone(),
                    mode,
                    oid: Some(oid),
                });
            }
            _ => return Err(()),
        }
    }
    Ok(staged)
}

/// `update-index --index-info -z` input for the staged paths.
fn index_info(staged: &[Staged], oid_len: usize) -> Vec<u8> {
    let zero = "0".repeat(oid_len);
    staged
        .iter()
        .flat_map(|s| {
            let oid = s.oid.as_deref().unwrap_or(&zero);
            format!("{} {oid}\t{}\0", s.mode, s.path).into_bytes()
        })
        .collect()
}

/// Commit exactly `paths` with `message` through a temporary index, then refresh only those paths in the
/// real index. The user's configuration, hooks and signing apply; nothing is forced.
///
/// The commit stages the exact recorded bytes (no clean filters) and is certified only when its parent,
/// changed set, trailers and every path's object id equal what was staged. An unreadable HEAD after
/// writing returns [`Attempt::Unknown`] and retains reconciliation evidence; a readable but mismatched
/// commit returns [`Attempt::Unverified`], never as landed.
pub fn commit_paths(
    store: &Store,
    parent: &str,
    paths: &[String],
    message: &str,
    expected: &BTreeMap<String, Option<String>>,
    deadline: Instant,
) -> Attempt {
    let dir = store.root.join(".git/agent-tasks");
    let _ = std::fs::create_dir(&dir);
    let index = dir.join(format!(
        "index.{}-{}",
        std::process::id(),
        journal::new_intent_id(store)
    ));
    let cleanup = |index: &std::path::Path| {
        let _ = std::fs::remove_file(index);
        let mut lock = index.as_os_str().to_owned();
        lock.push(".lock");
        let _ = std::fs::remove_file(std::path::PathBuf::from(lock));
    };
    let Ok(staged) = stage_exact(store, parent, paths, expected, deadline) else {
        return Attempt::NotCompleted;
    };
    let info = index_info(&staged, parent.len());
    let prepared = run_indexed(store, &index, &["read-tree", "HEAD"], None, deadline)
        .is_ok_and(|o| o.success())
        && run_indexed(
            store,
            &index,
            &["update-index", "-z", "--index-info"],
            Some(&info),
            deadline,
        )
        .is_ok_and(|o| o.success());
    if !prepared {
        cleanup(&index);
        return Attempt::NotCompleted;
    }
    if run_indexed(
        store,
        &index,
        &["diff", "--cached", "--quiet", "HEAD"],
        None,
        deadline,
    )
    .is_ok_and(|o| o.code == Some(0))
    {
        cleanup(&index);
        return Attempt::NotCompleted;
    }
    let result = run_indexed(
        store,
        &index,
        &["commit", "--quiet", "-F", "-"],
        Some(message.as_bytes()),
        deadline,
    );
    cleanup(&index);
    // The commit deadline may be spent by a stuck hook; the verification reads get their own short one.
    let verify = Instant::now() + std::time::Duration::from_secs(5);
    let head = git::read_text(
        &store.root,
        &["rev-parse", "--verify", "-q", "HEAD"],
        verify,
    );
    #[cfg(test)]
    let head = if UNAVAILABLE_VERIFICATION_HEAD.with(|fault| fault.replace(false)) {
        None
    } else {
        head
    };
    match result {
        Ok(out) if out.success() => (),
        Ok(_) if head.as_deref() == Some(parent) => return Attempt::NotCompleted,
        Err(git::RunError::Unavailable) => return Attempt::NotCompleted,
        _ if head.as_deref() == Some(parent) || head.is_none() => return Attempt::Unknown,
        _ => (),
    }
    // A successful write without readable HEAD proof is uncertain, not evidence that no commit
    // landed. Keep the journal's committing_from so explicit reconciliation can find that commit.
    if head.is_none() {
        return Attempt::Unknown;
    }
    let deadline = deadline.max(verify);
    let Some(new) = head.filter(|h| h != parent) else {
        return Attempt::NotCompleted;
    };
    let mut attention = Vec::new();
    let mut proven = true;
    if git::read_text(
        &store.root,
        &["rev-parse", "--verify", "-q", &format!("{new}^")],
        deadline,
    )
    .as_deref()
        != Some(parent)
    {
        proven = false;
    }
    let changed = git::run(
        &store.root,
        &[
            "diff-tree",
            "-r",
            "-z",
            "--no-renames",
            "--name-only",
            "--no-commit-id",
            &new,
        ],
        None,
        deadline,
    )
    .ok()
    .filter(|o| o.success() && !o.truncated)
    .map(|o| nul_list(&o.stdout));
    let wanted: BTreeSet<String> = paths.iter().cloned().collect();
    if !changed.as_ref().is_some_and(|c| c.is_subset(&wanted)) {
        proven = false;
    }
    for s in &staged {
        match super::verify::tree_entry(store, &new, &s.path, deadline) {
            Ok(entry) if entry.as_ref().map(|(_, oid)| oid) == s.oid.as_ref() => (),
            _ => proven = false,
        }
        if file_sha(store, &s.path).ok().as_ref() != expected.get(&s.path) {
            attention.push(Attention::HookChangedWorktree);
        }
    }
    let committed_message =
        git::read_text(&store.root, &["log", "-1", "--format=%B", &new], deadline)
            .unwrap_or_default();
    let present: BTreeSet<&str> = committed_message.lines().collect();
    if !message
        .lines()
        .filter(|l| l.starts_with("Agent-Tasks-"))
        .all(|l| present.contains(l))
    {
        proven = false;
    }
    if !proven {
        attention.push(Attention::TreeMismatch);
        attention.sort_by_key(|a| format!("{a:?}"));
        attention.dedup();
        return Attempt::Unverified(new, attention);
    }
    if !run_refresh(store, &info, deadline) {
        attention.push(Attention::IndexRefreshFailed);
    }
    attention.sort_by_key(|a| format!("{a:?}"));
    attention.dedup();
    Attempt::Landed(new, attention)
}

/// Refresh only this engine's own paths in the real index after a landed commit, to the committed blobs.
fn run_refresh(store: &Store, info: &[u8], deadline: Instant) -> bool {
    git::run(
        &store.root,
        &["update-index", "-z", "--index-info"],
        Some(info),
        deadline,
    )
    .is_ok_and(|o| o.success())
}

/// Build the commit message for the selected intents of one commit.
fn build_message(journal: &Journal, selected: &[usize], class: &str, refs: &[String]) -> String {
    let mut text = format!("docs: record {class} mutation\n\n");
    for r in refs.iter().take(16) {
        text.push_str(&format!("Ref: {r}\n"));
    }
    if !refs.is_empty() {
        text.push('\n');
    }
    for n in selected {
        text.push_str(&trailers(journal, selected, *n));
    }
    text
}

/// Map an untracked reason to the receipt reason.
fn untracked_reason(reason: UntrackedReason) -> Reason {
    match reason {
        UntrackedReason::NoRepository => Reason::NoRepository,
        UntrackedReason::JournalFull => Reason::JournalFull,
        UntrackedReason::JournalUnavailable => Reason::JournalUnavailable,
        UntrackedReason::JournalCorrupt => Reason::JournalCorrupt,
    }
}

/// Up to sixteen pending references, the current call's intent first.
pub fn pending_refs(store: &Store, current: Option<&str>) -> Vec<PendingRef> {
    let Ok((journal, _)) = journal::load(store) else {
        return Vec::new();
    };
    let mut refs: Vec<PendingRef> = journal
        .intents
        .iter()
        .filter(|i| i.committed.is_none())
        .map(|i| PendingRef {
            intent: i.id.clone(),
            phase: classify(store, i).phase(),
            paths: i.entries.len(),
        })
        .collect();
    refs.sort_by_key(|r| Some(r.intent.as_str()) != current);
    refs.truncate(16);
    refs
}

/// Pending facts for the policy and for read-only summaries.
pub fn pending_facts(store: &Store) -> PendingFacts {
    let Ok((journal, _)) = journal::load(store) else {
        return PendingFacts::default();
    };
    let mut facts = PendingFacts::default();
    for intent in journal.intents.iter().filter(|i| i.committed.is_none()) {
        facts.intents += 1;
        facts.paths += intent.entries.len();
        match classify(store, intent) {
            State::Unknown => facts.unknown += 1,
            State::Drifted => facts.drifted += 1,
            _ => (),
        }
        if let Ok(when) = chrono::DateTime::parse_from_rfc3339(&intent.created_at) {
            let secs = u64::try_from(when.timestamp()).ok();
            facts.oldest_unix_secs = match (facts.oldest_unix_secs, secs) {
                (Some(a), Some(b)) => Some(a.min(b)),
                (a, b) => a.or(b),
            };
        }
    }
    facts
}

/// Drop `Prepared` entries whose file still equals the recorded before-bytes (unpublished intents).
pub fn discard_unpublished(store: &Store) {
    let Ok((mut journal, observed)) = journal::load(store) else {
        return;
    };
    let mut changed = false;
    for intent in &mut journal.intents {
        let before = intent.entries.len();
        intent.entries.retain(|e| {
            e.phase != EntryPhase::Prepared
                || file_sha(store, &e.path)
                    .ok()
                    .is_none_or(|now| now != e.before_sha256)
                || e.after_sha256 == e.before_sha256
        });
        changed |= intent.entries.len() != before;
    }
    journal
        .intents
        .retain(|i| !i.entries.is_empty() || i.committed.is_some());
    if changed {
        let _ = journal::save(store, &journal, observed.as_deref());
    }
}

/// Resolve intents whose commit attempt was interrupted, using HEAD and the intent trailer.
pub fn reconcile_committing(store: &Store, deadline: Instant) {
    let Ok((mut journal, observed)) = journal::load(store) else {
        return;
    };
    let mut changed = false;
    let head = git::read_text(
        &store.root,
        &["rev-parse", "--verify", "-q", "HEAD"],
        deadline,
    );
    for intent in &mut journal.intents {
        let Some(from) = intent.committing_from.clone() else {
            continue;
        };
        if head.as_deref() == Some(from.as_str()) {
            intent.committing_from = None;
            changed = true;
        } else if let Some(found) = git::read_text(
            &store.root,
            &[
                "log",
                "--fixed-strings",
                "--grep",
                &format!("Agent-Tasks-Intent: {}", intent.id),
                "-n",
                "1",
                "--format=%H",
                &format!("{from}..HEAD"),
            ],
            deadline,
        )
        .filter(|s| !s.is_empty())
        {
            // A trailer match alone is not proof: the commit must hold what the journal recorded.
            if super::verify::intent_in_commit(store, &found, intent, deadline)
                == super::verify::Verdict::Exact
            {
                intent.committed = Some(found);
                intent.committing_from = None;
                changed = true;
            }
        }
    }
    if changed {
        let _ = journal::save(store, &journal, observed.as_deref());
    }
}

/// Stamp this call's intent with its class, references and effective outcome before any Git step.
fn stamp(store: &Store, intent: &str, event: &Event, outcome: IntentOutcome) -> bool {
    let Ok((mut journal, observed)) = journal::load(store) else {
        return false;
    };
    let Some(target) = journal.intents.iter_mut().find(|i| i.id == intent) else {
        return false;
    };
    target.class = Some(event.class.name().to_owned());
    target.refs = event.refs.iter().take(16).cloned().collect();
    target.outcome = outcome;
    journal::save(store, &journal, observed.as_deref()).is_ok()
}

/// Evaluate the policy and, if it says commit, run the Git engine; always returns a receipt.
///
/// `guard` proves the root write lock for this store is held; settlement never takes it. The
/// receipt's outcome, commit and paths are derived only from this call's own publications and intent.
pub fn settle(store: &Store, guard: &LockGuard, event: &Event, policy: &dyn Policy) -> GitReceipt {
    settle_by(
        store,
        guard,
        event,
        policy,
        Instant::now() + git::SETTLEMENT_TIME,
    )
}

/// [`settle`] with an explicit deadline for the whole settlement, so tests can bound a stuck hook.
pub fn settle_by(
    store: &Store,
    guard: &LockGuard,
    event: &Event,
    policy: &dyn Policy,
    deadline: Instant,
) -> GitReceipt {
    let mut receipt = GitReceipt::saved_only();
    if !guard.is_for(store) {
        receipt.reason = Some(Reason::JournalUnavailable);
        return receipt;
    }
    let facts = call_facts(store);
    receipt.untracked = facts.untracked.iter().take(16).cloned().collect();
    if !facts.untracked.is_empty() {
        receipt.attention.push(Attention::UntrackedMutation);
    }
    if facts.uncertain {
        receipt.attention.push(Attention::SyncUncertain);
    }
    let finish = |mut r: GitReceipt, intent: Option<&str>| {
        r.pending = pending_refs(store, intent);
        r
    };
    if facts.eligible() == 0 {
        receipt.reason = Some(Reason::NoMutation);
        return finish(receipt, None);
    }
    let Some(intent) = facts.intent.clone() else {
        receipt.reason = facts.first_untracked.map(untracked_reason);
        return finish(receipt, None);
    };
    discard_unpublished(store);
    let incomplete =
        event.outcome != EventOutcome::Success || !facts.untracked.is_empty() || facts.uncertain;
    let outcome = if incomplete {
        IntentOutcome::Partial
    } else {
        IntentOutcome::Success
    };
    if !stamp(store, &intent, event, outcome) {
        receipt.outcome = GitOutcome::Deferred;
        receipt.reason = Some(Reason::JournalUnavailable);
        receipt.paths = facts.tracked_paths.clone();
        receipt.attention.push(Attention::UnknownPending);
        return finish(receipt, Some(&intent));
    }
    receipt.paths = facts.tracked_paths.clone();
    let pending = pending_facts(store);
    let decision = policy.decide(&PolicyInput {
        event,
        pending: &pending,
        tracked: facts.tracked,
        sync_uncertain: facts.uncertain,
        untracked: facts.untracked.len(),
    });
    match decision {
        Decision::Skip => {
            receipt.outcome = GitOutcome::Saved;
            receipt.paths.clear();
            return finish(receipt, Some(&intent));
        }
        Decision::Defer(why) => {
            receipt.outcome = GitOutcome::Deferred;
            receipt.reason = Some(if !facts.untracked.is_empty() {
                Reason::UntrackedSibling
            } else if why == DeferReason::PolicyDeferred {
                Reason::PolicyDeferred
            } else {
                Reason::IncompleteOutcome
            });
            return finish(receipt, Some(&intent));
        }
        Decision::Commit => (),
    }
    run_engine(store, &intent, event, &mut receipt, deadline, &[]);
    finish(receipt, Some(&intent))
}

/// Paths each intent still has to commit: entries no later commit has already superseded.
fn live_paths(journal: &Journal) -> BTreeMap<usize, BTreeSet<String>> {
    journal
        .intents
        .iter()
        .enumerate()
        .map(|(n, i)| {
            (
                n,
                i.entries
                    .iter()
                    .filter(|e| e.superseded_by.is_none())
                    .map(|e| e.path.clone())
                    .collect(),
            )
        })
        .collect()
}

/// Open intents of `ids` plus every open intent connected to them through a shared live path.
///
/// A recovery that names only part of such a chain would commit a split of it, so callers refuse and
/// name the rest. Returns the identities connected to the named ones that were not named.
pub fn connected_extras(store: &Store, ids: &[String]) -> Vec<String> {
    let Ok((journal, _)) = journal::load(store) else {
        return Vec::new();
    };
    let paths = live_paths(&journal);
    let open = |n: usize| journal.intents[n].committed.is_none();
    let mut members: BTreeSet<usize> = journal
        .intents
        .iter()
        .enumerate()
        .filter(|(n, i)| open(*n) && ids.contains(&i.id))
        .map(|(n, _)| n)
        .collect();
    loop {
        let grew: Vec<usize> = (0..journal.intents.len())
            .filter(|n| open(*n) && !members.contains(n))
            .filter(|n| members.iter().any(|m| !paths[m].is_disjoint(&paths[n])))
            .collect();
        if grew.is_empty() {
            break;
        }
        members.extend(grew);
    }
    members
        .into_iter()
        .map(|n| journal.intents[n].id.clone())
        .filter(|id| !ids.contains(id))
        .collect()
}

/// Intents in `set` that remove a path under an operation identity while an older open intent outside
/// `set` still holds a replacement written under the same identity.
///
/// This is the engine's limited removal guard: the original must stay in history until the replacement
/// that justifies the removal is committed (or in the same commit).
fn barrier_hits(journal: &Journal, set: &BTreeSet<usize>) -> Vec<usize> {
    set.iter()
        .copied()
        .filter(|n| {
            journal.intents[*n]
                .entries
                .iter()
                .filter(|e| e.kind == EffectKind::Removed && e.superseded_by.is_none())
                .filter_map(|e| e.operation.as_deref())
                .any(|op| {
                    (0..*n).any(|m| {
                        journal.intents[m].committed.is_none()
                            && !set.contains(&m)
                            && journal.intents[m].entries.iter().any(|e| {
                                e.kind != EffectKind::Removed && e.operation.as_deref() == Some(op)
                            })
                    })
                })
        })
        .collect()
}

/// Remove barrier hits and everything connected to them from `set`, recording the reason.
fn apply_barrier(
    journal: &Journal,
    paths: &BTreeMap<usize, BTreeSet<String>>,
    set: &mut BTreeSet<usize>,
    excluded: &mut BTreeMap<usize, Reason>,
) {
    loop {
        let hits = barrier_hits(journal, set);
        if hits.is_empty() {
            return;
        }
        let mut doomed: BTreeSet<usize> = hits.into_iter().collect();
        loop {
            let more: Vec<usize> = set
                .iter()
                .copied()
                .filter(|n| !doomed.contains(n))
                .filter(|n| doomed.iter().any(|d| !paths[d].is_disjoint(&paths[n])))
                .collect();
            if more.is_empty() {
                break;
            }
            doomed.extend(more);
        }
        for n in doomed {
            set.remove(&n);
            excluded.insert(n, Reason::RemovalBarrier);
        }
    }
}

/// Select, commit and verify, then fill the call-scoped part of `receipt`.
pub fn run_engine(
    store: &Store,
    current: &str,
    event: &Event,
    receipt: &mut GitReceipt,
    deadline: Instant,
    forced: &[String],
) {
    receipt.outcome = GitOutcome::Deferred;
    let repo = match inspect(store, deadline) {
        Ok(repo) => repo,
        Err(reason) => {
            receipt.reason = Some(reason);
            return;
        }
    };
    reconcile_committing(store, deadline);
    let Ok((mut journal, observed)) = journal::load(store) else {
        receipt.reason = Some(Reason::JournalUnavailable);
        return;
    };
    let mut originals: Vec<(String, IntentOutcome, bool)> = Vec::new();
    for intent in journal
        .intents
        .iter_mut()
        .filter(|i| forced.contains(&i.id) && i.committed.is_none())
    {
        originals.push((intent.id.clone(), intent.outcome, intent.adopted));
        if intent.outcome != IntentOutcome::Success {
            intent.adopted = true;
        }
        intent.outcome = IntentOutcome::Success;
    }
    let selection = analyze(store, &journal, Some(current));
    let candidate = |n: usize| selection_candidate(&journal, n);
    let all_paths = live_paths(&journal);
    let mut excluded: BTreeMap<usize, Reason> = selection.deferred.clone();
    let selected_paths: Vec<String> = selection
        .selected
        .iter()
        .flat_map(|n| all_paths[n].iter().cloned())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();
    if !selected_paths.is_empty() {
        let (Some(staged), Some(ignored)) = (
            staged_paths(store, deadline),
            ignored_among(store, &selected_paths, deadline),
        ) else {
            receipt.reason = Some(Reason::GitUnavailable);
            return;
        };
        for n in &selection.selected {
            let touched = &all_paths[n];
            if !touched.is_disjoint(&staged) {
                excluded.entry(*n).or_insert(Reason::ForeignStagingOnPath);
            } else if !touched.is_disjoint(&ignored) {
                excluded.entry(*n).or_insert(Reason::IgnoredBusinessSibling);
            }
        }
        close_over_paths(&journal, &all_paths, &mut excluded, &candidate);
    }
    let mut live: BTreeSet<usize> = selection
        .selected
        .iter()
        .copied()
        .filter(|n| !excluded.contains_key(n))
        .collect();
    apply_barrier(&journal, &all_paths, &mut live, &mut excluded);
    let order: Vec<usize> = {
        let mut order: Vec<usize> = live
            .iter()
            .copied()
            .filter(|n| forced.is_empty() || forced.contains(&journal.intents[*n].id))
            .collect();
        order.sort_by_key(|n| journal.intents[*n].id != current);
        order
    };
    // Components over shared paths, current call's first, within the message budget.
    let mut chosen: Vec<usize> = Vec::new();
    let mut used = 0usize;
    for n in &order {
        if chosen.contains(n) {
            continue;
        }
        let mut component = vec![*n];
        loop {
            let more: Vec<usize> = order
                .iter()
                .copied()
                .filter(|m| {
                    !component.contains(m)
                        && component
                            .iter()
                            .any(|c| !all_paths[c].is_disjoint(&all_paths[m]))
                })
                .collect();
            if more.is_empty() {
                break;
            }
            component.extend(more);
        }
        component.sort_unstable();
        let size: usize = component
            .iter()
            .map(|c| trailers(&journal, &component, *c).len())
            .sum();
        if used + size + 1024 > MESSAGE_BUDGET {
            for c in &component {
                excluded.entry(*c).or_insert(Reason::MessageBudget);
            }
            continue;
        }
        used += size;
        chosen.extend(component);
    }
    // A budget exclusion may have dropped the replacement a removal waits for.
    let mut chosen_set: BTreeSet<usize> = chosen.iter().copied().collect();
    apply_barrier(&journal, &all_paths, &mut chosen_set, &mut excluded);
    chosen.retain(|n| chosen_set.contains(n));
    chosen.sort_unstable();
    chosen.dedup();
    let this_index = journal.intents.iter().position(|i| i.id == current);
    if let Some(n) = this_index
        && !chosen.contains(&n)
    {
        receipt.reason = excluded
            .get(&n)
            .or(selection.deferred.get(&n))
            .copied()
            .or(Some(Reason::IncompleteOutcome));
        if selection.drifted.contains(&n) {
            receipt.attention.push(Attention::NativeDrift);
        }
    }
    let recovering = !forced.is_empty();
    if chosen.is_empty() {
        if recovering {
            receipt.reason = forced
                .iter()
                .filter_map(|id| journal.intents.iter().position(|i| i.id == *id))
                .find_map(|n| excluded.get(&n).or(selection.deferred.get(&n)).copied())
                .or(Some(Reason::IncompleteOutcome));
        }
        return;
    }
    let paths: Vec<String> = chosen
        .iter()
        .flat_map(|n| all_paths[n].iter().cloned())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();
    if paths.is_empty() {
        receipt.reason = Some(Reason::IncompleteOutcome);
        return;
    }
    let mut expected: BTreeMap<String, Option<String>> = BTreeMap::new();
    for n in &chosen {
        for e in journal.intents[*n]
            .entries
            .iter()
            .filter(|e| e.superseded_by.is_none())
        {
            expected.insert(e.path.clone(), e.after_sha256.clone());
        }
    }
    let message = build_message(&journal, &chosen, event.class.name(), &event.refs);
    for n in &chosen {
        journal.intents[*n].committing_from = Some(repo.parent.clone());
    }
    if journal::save(store, &journal, observed.as_deref()).is_err() {
        receipt.reason = Some(Reason::JournalUnavailable);
        return;
    }
    let attempt = commit_paths(store, &repo.parent, &paths, &message, &expected, deadline);
    let Ok((mut after, observed)) = journal::load(store) else {
        receipt.outcome = GitOutcome::Unknown;
        return;
    };
    match attempt {
        Attempt::Landed(commit, attention) => {
            for n in &chosen {
                after.intents[*n].committed = Some(commit.clone());
                after.intents[*n].committing_from = None;
            }
            // Each committed path now holds the last chosen image; older open writes to it are superseded.
            let mut winners: BTreeMap<&str, (String, String)> = BTreeMap::new();
            for n in &chosen {
                for e in journal.intents[*n]
                    .entries
                    .iter()
                    .filter(|e| e.superseded_by.is_none())
                {
                    winners.insert(
                        e.path.as_str(),
                        (
                            journal.intents[*n].id.clone(),
                            e.after_sha256.clone().unwrap_or_else(|| "-".to_owned()),
                        ),
                    );
                }
            }
            for intent in after.intents.iter_mut().filter(|i| i.committed.is_none()) {
                for entry in &mut intent.entries {
                    if entry.superseded_by.is_none()
                        && let Some((id, digest)) = winners.get(entry.path.as_str())
                    {
                        entry.superseded_by = Some(format!("{id}:{digest}"));
                    }
                }
            }
            let _ = journal::save(store, &after, observed.as_deref());
            let chosen_ids: BTreeSet<&String> =
                chosen.iter().map(|n| &journal.intents[*n].id).collect();
            let all_named = recovering && forced.iter().all(|id| chosen_ids.contains(id));
            let mine =
                all_named || (!recovering && this_index.is_some_and(|n| chosen.contains(&n)));
            let older = if recovering {
                0
            } else {
                chosen.iter().filter(|n| Some(**n) != this_index).count()
            };
            let older_paths: usize = chosen
                .iter()
                .filter(|n| Some(**n) != this_index)
                .map(|n| journal.intents[*n].entries.len())
                .sum();
            if recovering {
                receipt.paths = paths.clone();
                receipt.commit = Some(commit.clone());
                if !all_named {
                    receipt.reason = forced
                        .iter()
                        .filter(|id| !chosen_ids.contains(id))
                        .filter_map(|id| journal.intents.iter().position(|i| i.id == *id))
                        .find_map(|n| excluded.get(&n).or(selection.deferred.get(&n)).copied())
                        .or(Some(Reason::IncompleteOutcome));
                }
            }
            if mine {
                receipt.outcome = GitOutcome::Committed;
                receipt.commit = Some(commit.clone());
                receipt.reason = None;
            }
            if older > 0 {
                receipt.earlier.push(EarlierCommit {
                    commit,
                    intents: older,
                    paths: older_paths,
                });
            }
            receipt.attention.extend(attention);
        }
        Attempt::Unverified(_, attention) => {
            receipt.outcome = GitOutcome::Unknown;
            receipt.reason = None;
            receipt.attention.extend(attention);
            receipt.attention.push(Attention::UnknownPending);
        }
        Attempt::NotCompleted => {
            for n in &chosen {
                after.intents[*n].committing_from = None;
            }
            for (id, outcome, adopted) in &originals {
                if let Some(i) = after.intents.iter_mut().find(|i| i.id == *id) {
                    i.outcome = *outcome;
                    i.adopted = *adopted;
                }
            }
            let _ = journal::save(store, &after, observed.as_deref());
            receipt.reason = Some(Reason::CommitNotCompleted);
        }
        Attempt::Unknown => {
            receipt.outcome = GitOutcome::Unknown;
            receipt.reason = None;
            receipt.attention.push(Attention::UnknownPending);
        }
    }
}

/// Whether intent `n` is a commit candidate (successful and fully published).
fn selection_candidate(journal: &Journal, n: usize) -> bool {
    let i = &journal.intents[n];
    i.committed.is_none()
        && i.committing_from.is_none()
        && i.outcome == IntentOutcome::Success
        && i.entries.iter().all(|e| e.phase == EntryPhase::Published)
}

/// Convenience for dispatchers: settle after `result`, keep `result` unchanged and attach the receipt.
///
/// The dispatcher builds `event` with the outcome it judged; the result itself is returned untouched, so
/// a business error is never replaced by a Git outcome and a Git failure never fails a successful call.
pub fn settled<T>(
    store: &Store,
    guard: &LockGuard,
    event: Event,
    policy: &dyn Policy,
    result: crate::store::Result<T>,
) -> (crate::store::Result<T>, GitReceipt) {
    let receipt = settle(store, guard, &event, policy);
    (result, receipt)
}
