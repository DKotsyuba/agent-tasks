//! The call-scoped Git receipt: plain serializable data built before any presentation.
//!
//! Outcome, commit, paths and reason describe THIS call's own publications and whole intent only.
//! Commits that the same settlement made for older calls appear in `earlier` as extra facts and never
//! change this call's outcome. A receipt survives renderer failure because it is plain data.
use serde::Serialize;

/// Outcome of THIS call's own publications.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum GitOutcome {
    /// Files are on disk; no Git attempt, no repository, or nothing eligible to commit.
    Saved,
    /// A commit holds this call's whole intent.
    Committed,
    /// Files are saved and pending; Git state, policy or call completeness kept this call uncommitted.
    Deferred,
    /// An attempt may or may not have landed for this call; never retried blindly.
    Unknown,
}

/// Progress of one pending intent as the reconciliation table sees it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum Phase {
    /// Entry written, publication not confirmed.
    Prepared,
    /// Published and eligible for a commit.
    Published,
    /// Partial or unsettled call; waits for explicit recovery.
    Held,
    /// A commit attempt started and its outcome is unresolved.
    Committing,
    /// Provenance unknown; only explicit recovery adopts it.
    Unknown,
    /// A native edit changed a published file.
    Drifted,
}

/// A recoverable pending intent.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PendingRef {
    /// Intent identity (`PG-` plus 24 hex characters).
    pub intent: String,
    /// Reconciled phase.
    pub phase: Phase,
    /// Number of path entries.
    pub paths: usize,
}

/// A commit this settlement also made for older calls.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct EarlierCommit {
    /// Full commit id.
    pub commit: String,
    /// Intents the commit holds.
    pub intents: usize,
    /// Paths the commit changed.
    pub paths: usize,
}

/// Why THIS call is deferred, or saved without an attempt.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum Reason {
    /// No independent repository with a commit.
    NoRepository,
    /// `.git` is a file (linked worktree or submodule) or the root is nested.
    NotIndependentRepository,
    /// Git could not run.
    GitUnavailable,
    /// A test policy deferred.
    PolicyDeferred,
    /// HEAD names no commit yet.
    UnbornHead,
    /// HEAD is detached.
    DetachedHead,
    /// A merge, rebase, cherry-pick, revert, bisect or am is in progress.
    OperationInProgress,
    /// The index has unmerged entries.
    UnmergedPaths,
    /// The index is locked.
    IndexLocked,
    /// The user staged a change on a path of the intent.
    ForeignStagingOnPath,
    /// Git started the commit but it did not complete (hook, signing or other rejection).
    CommitNotCompleted,
    /// The journal is full.
    JournalFull,
    /// The journal cannot be written.
    JournalUnavailable,
    /// The journal is corrupt.
    JournalCorrupt,
    /// The serialized commit message would exceed its budget.
    MessageBudget,
    /// A true no-op or nothing eligible: no commit is due.
    NoMutation,
    /// A partial or sync-uncertain call: held for explicit recovery.
    IncompleteOutcome,
    /// Some eligible file of this call is untracked, so the tracked rest is not committed alone.
    UntrackedSibling,
    /// Same-operation guard: an older held replacement of the same operation precedes this removal.
    RemovalBarrier,
    /// A business file or counter is untracked or ignored by the user's own rules.
    IgnoredBusinessSibling,
}

/// Things to look at even when committed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum Attention {
    /// The commit landed but refreshing the index failed.
    IndexRefreshFailed,
    /// The committed tree differs from the recorded bytes.
    TreeMismatch,
    /// A hook changed the worktree during the commit.
    HookChangedWorktree,
    /// A published file was changed natively.
    NativeDrift,
    /// Other pending work has an unknown outcome.
    UnknownPending,
    /// An eligible file was published without a journal entry.
    UntrackedMutation,
    /// A directory or other effect has an uncertain sync.
    SyncUncertain,
}

/// Plain data describing what Git did for one call.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct GitReceipt {
    /// Outcome of THIS call's own publications.
    pub outcome: GitOutcome,
    /// Commit holding this call's intent; `None` otherwise.
    pub commit: Option<String>,
    /// This call's exact paths: in `commit` when committed, else the pending ones.
    pub paths: Vec<String>,
    /// Up to 16 paths this call published without a journal entry.
    pub untracked: Vec<String>,
    /// Why this call is deferred, or saved without an attempt.
    pub reason: Option<Reason>,
    /// Attention flags.
    pub attention: Vec<Attention>,
    /// Up to 16 recoverable pending intents, this call's first.
    pub pending: Vec<PendingRef>,
    /// Up to 4 commits this settlement also made for older calls.
    pub earlier: Vec<EarlierCommit>,
}

impl GitReceipt {
    /// A receipt for a call with nothing to commit and no Git attempt.
    pub fn saved_only() -> Self {
        Self {
            outcome: GitOutcome::Saved,
            commit: None,
            paths: Vec::new(),
            untracked: Vec::new(),
            reason: None,
            attention: Vec::new(),
            pending: Vec::new(),
            earlier: Vec::new(),
        }
    }

    /// At most six plain English lines of at most 200 bytes, this call's own line first and at most one
    /// line for older commits last. Paths are shown only as a count and the first path.
    pub fn lines(&self) -> Vec<String> {
        let first = self
            .paths
            .first()
            .map_or(String::new(), |p| format!(" First: {p}."));
        let mut lines = vec![match self.outcome {
            GitOutcome::Saved => "Git: saved on disk; nothing was committed for this call.".to_owned(),
            GitOutcome::Committed => format!(
                "Git: committed {} with {} path(s).{first}",
                self.commit.as_deref().map_or("", |c| &c[..c.len().min(12)]),
                self.paths.len()
            ),
            GitOutcome::Deferred => format!("Git: saved and pending; {} path(s) not committed.{first}", self.paths.len()),
            GitOutcome::Unknown => "Git: outcome unknown; saved files are intact. Inspect pending state; do not replay.".to_owned(),
        }];
        if let Some(reason) = self.reason {
            lines.push(format!("Reason: {reason:?}."));
        }
        if !self.untracked.is_empty() {
            lines.push(format!(
                "Untracked: {} eligible file(s) were saved without a journal entry. First: {}.",
                self.untracked.len(),
                self.untracked[0]
            ));
        }
        if !self.attention.is_empty() {
            lines.push(format!("Attention: {:?}.", self.attention));
        }
        if !self.pending.is_empty() {
            lines.push(format!(
                "Pending: {} intent(s). First: {} ({:?}).",
                self.pending.len(),
                self.pending[0].intent,
                self.pending[0].phase
            ));
        }
        if let Some(e) = self.earlier.first() {
            lines.push(format!(
                "Also committed {} for {} earlier call(s), {} path(s).",
                &e.commit[..e.commit.len().min(12)],
                e.intents,
                e.paths
            ));
        }
        lines
            .into_iter()
            .map(|l| crate::store::safe(&l, 200).trim_matches('"').to_owned())
            .collect()
    }
}

/// Receipt regressions.
#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "Explicit isolated assertions"
)]
mod tests {
    use super::*;

    /// A committed receipt names its own commit first and older commits last, within six short lines.
    #[test]
    fn lines_are_bounded_and_ordered() {
        let mut receipt = GitReceipt::saved_only();
        receipt.outcome = GitOutcome::Committed;
        receipt.commit = Some("a".repeat(40));
        receipt.paths = vec!["decisions/D-001.yaml".into(), "x".repeat(400)];
        receipt.attention = vec![Attention::SyncUncertain];
        receipt.earlier = vec![EarlierCommit {
            commit: "b".repeat(40),
            intents: 2,
            paths: 3,
        }];
        let lines = receipt.lines();
        assert!(lines[0].starts_with("Git: committed aaaaaaaaaaaa with 2 path(s)."));
        assert!(
            lines
                .last()
                .unwrap()
                .starts_with("Also committed bbbbbbbbbbbb")
        );
        assert!(lines.len() <= 6 && lines.iter().all(|l| l.len() <= 200));
    }

    /// A saved-only receipt claims nothing about Git.
    #[test]
    fn saved_only_claims_no_commit() {
        let receipt = GitReceipt::saved_only();
        assert_eq!(
            (receipt.outcome, receipt.commit.clone()),
            (GitOutcome::Saved, None)
        );
        assert_eq!(receipt.lines().len(), 1);
    }
}
