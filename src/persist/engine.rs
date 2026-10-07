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
            if entry.phase == EntryPhase::Published {
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

/// Commit exactly `paths` with `message` through a temporary index, then refresh only those paths in the
/// real index. The user's configuration, hooks and signing apply; nothing is forced.
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
    let stdin: Vec<u8> = paths.iter().flat_map(|p| p.bytes().chain([0])).collect();
    let prepared = run_indexed(store, &index, &["read-tree", "HEAD"], None, deadline)
        .is_ok_and(|o| o.success())
        && run_indexed(
            store,
            &index,
            &["update-index", "--add", "--remove", "-z", "--stdin"],
            Some(&stdin),
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
    // The commit deadline may be spent by a stuck hook; the verification read gets its own short one.
    let verify = Instant::now() + std::time::Duration::from_secs(5);
    let head = git::read_text(
        &store.root,
        &["rev-parse", "--verify", "-q", "HEAD"],
        verify,
    );
    match result {
        Ok(out) if out.success() => (),
        Ok(_) if head.as_deref() == Some(parent) => return Attempt::NotCompleted,
        Err(git::RunError::Unavailable) => return Attempt::NotCompleted,
        _ if head.as_deref() == Some(parent) || head.is_none() => return Attempt::Unknown,
        _ => (),
    }
    let deadline = deadline.max(verify);
    let Some(new) = head.filter(|h| h != parent) else {
        return Attempt::NotCompleted;
    };
    let mut attention = Vec::new();
    if git::read_text(
        &store.root,
        &["rev-parse", "--verify", "-q", &format!("{new}^")],
        deadline,
    )
    .as_deref()
        != Some(parent)
    {
        attention.push(Attention::TreeMismatch);
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
    .filter(git::Output::success)
    .map(|o| nul_list(&o.stdout));
    if changed.as_ref() != Some(&paths.iter().cloned().collect()) {
        attention.push(Attention::TreeMismatch);
    }
    for (path, digest) in expected {
        let committed = git::read_text(
            &store.root,
            &["rev-parse", "--verify", "-q", &format!("{new}:{path}")],
            deadline,
        );
        let matches = match digest {
            None => committed.is_none(),
            Some(_) => {
                let raw = git::read_text(
                    &store.root,
                    &["hash-object", "--no-filters", "--", path],
                    deadline,
                );
                committed.is_some() && committed == raw
            }
        };
        if !matches {
            attention.push(Attention::TreeMismatch);
        }
        if file_sha(store, path).ok().as_ref() != Some(digest) {
            attention.push(Attention::HookChangedWorktree);
        }
    }
    if !run_refresh(store, &stdin, deadline) {
        attention.push(Attention::IndexRefreshFailed);
    }
    attention.sort_by_key(|a| format!("{a:?}"));
    attention.dedup();
    Attempt::Landed(new, attention)
}

/// Refresh only this engine's own paths in the real index after a landed commit.
fn run_refresh(store: &Store, stdin: &[u8], deadline: Instant) -> bool {
    git::run(
        &store.root,
        &["update-index", "--add", "--remove", "-z", "--stdin"],
        Some(stdin),
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
            intent.committed = Some(found);
            intent.committing_from = None;
            changed = true;
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
    let mut selection = analyze(store, &journal, Some(current));
    let candidate = |n: usize| selection_candidate(&journal, n);
    let all_paths: BTreeMap<usize, BTreeSet<String>> = journal
        .intents
        .iter()
        .enumerate()
        .map(|(n, i)| (n, i.entries.iter().map(|e| e.path.clone()).collect()))
        .collect();
    let mut excluded: BTreeMap<usize, Reason> = selection.deferred.clone();
    let selected_paths: Vec<String> = selection
        .selected
        .iter()
        .flat_map(|n| journal.intents[*n].entries.iter().map(|e| e.path.clone()))
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
    let order: Vec<usize> = {
        let mut order: Vec<usize> = selection
            .selected
            .iter()
            .copied()
            .filter(|n| !excluded.contains_key(n))
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
    if chosen.is_empty() {
        return;
    }
    let paths: Vec<String> = chosen
        .iter()
        .flat_map(|n| journal.intents[*n].entries.iter().map(|e| e.path.clone()))
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();
    let mut expected: BTreeMap<String, Option<String>> = BTreeMap::new();
    for n in &chosen {
        for e in &journal.intents[*n].entries {
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
            let chosen_ids: BTreeSet<String> = chosen
                .iter()
                .map(|n| after.intents[*n].id.clone())
                .collect();
            for intent in after.intents.iter_mut().filter(|i| i.committed.is_none()) {
                let winner = chosen_ids.iter().next().cloned();
                for entry in &mut intent.entries {
                    if paths.contains(&entry.path) && entry.superseded_by.is_none() {
                        entry.superseded_by = winner.clone();
                    }
                }
            }
            let _ = journal::save(store, &after, observed.as_deref());
            // An explicit recovery names its intents; they are the call, not "older" work.
            let recovering = !forced.is_empty();
            let mine = recovering || this_index.is_some_and(|n| chosen.contains(&n));
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
    let _ = &mut selection;
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
