//! Real-repository regressions for settlement, the Git engine, provenance and recovery.
//!
//! Every test uses a disposable repository with isolated Git configuration; nothing touches the
//! machine's own Git setup. Separate calls are modelled by separate `Store` values, which carry their
//! own request-local ledger and journal intent.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "Explicit isolated fixture assertions"
)]
use super::{
    Attention, Event, EventClass, EventOutcome, GitOutcome, GitReceipt, Reason,
    journal::{self, IntentOutcome},
    policy::DeferAll,
    production_policy, settle,
    testing::{GitFixture, with_policy},
};
use crate::store::{Attest, LockGuard, OperationId, Publish, RECORD_CAP, Store, Tracking};

/// A fresh request on the fixture root: new ledger, new intent, root lock held.
fn call(f: &GitFixture) -> (Store, LockGuard) {
    let store = Store::from_root(f.dir.path()).unwrap();
    let guard = store.lock(true, &mut Vec::new()).unwrap().unwrap();
    (store, guard)
}

/// Create a file with no-clobber semantics, optional attestation.
fn create(store: &Store, relative: &str, bytes: &[u8]) {
    store
        .publish_with(
            Publish {
                relative,
                bytes,
                observed: None,
                cap: RECORD_CAP,
                operation: None,
                attest: Attest::Optional,
            },
            &mut Vec::new(),
        )
        .unwrap();
}

/// Replace a file whose current bytes are `old`.
fn replace(store: &Store, relative: &str, bytes: &[u8], old: &[u8]) {
    store
        .publish_with(
            Publish {
                relative,
                bytes,
                observed: Some(old),
                cap: RECORD_CAP,
                operation: None,
                attest: Attest::Optional,
            },
            &mut Vec::new(),
        )
        .unwrap();
}

/// Publish with a required attestation under a deterministic operation identity.
fn create_op(store: &Store, relative: &str, bytes: &[u8], op: &str) {
    let op = OperationId::new(op).unwrap();
    store
        .publish_with(
            Publish {
                relative,
                bytes,
                observed: None,
                cap: RECORD_CAP,
                operation: Some(&op),
                attest: Attest::Required,
            },
            &mut Vec::new(),
        )
        .unwrap();
}

/// A successful work event.
fn success() -> Event {
    Event {
        class: EventClass::Work,
        refs: vec!["M-001".into()],
        operation: None,
        outcome: EventOutcome::Success,
    }
}

/// A partial work event, as the dispatcher would judge an error after publishing.
fn partial() -> Event {
    Event {
        outcome: EventOutcome::Partial,
        ..success()
    }
}

/// Settle with the production policy.
fn commit_now(store: &Store, guard: &LockGuard) -> GitReceipt {
    settle(store, guard, &success(), production_policy())
}

/// Settle with a test policy that never commits.
fn defer_now(store: &Store, guard: &LockGuard) -> GitReceipt {
    with_policy(&DeferAll, || {
        settle(store, guard, &success(), production_policy())
    })
}

/// Number of commits on the fixture branch.
fn commits(f: &GitFixture) -> usize {
    f.git(&["rev-list", "--count", "HEAD"]).parse().unwrap()
}

/// An ordinary successful call commits exactly its files with intent and effect trailers.
#[test]
fn successful_call_commits_its_exact_files_with_trailers() {
    let f = GitFixture::new();
    let (store, guard) = call(&f);
    create(&store, "modules/M-001.yaml", b"alpha");
    create(&store, "modules/M-002.yaml", b"beta");
    let receipt = commit_now(&store, &guard);
    assert_eq!(receipt.outcome, GitOutcome::Committed, "{receipt:?}");
    assert_eq!(receipt.paths, ["modules/M-001.yaml", "modules/M-002.yaml"]);
    assert_eq!(
        receipt.commit.as_deref(),
        Some(f.git(&["rev-parse", "HEAD"]).as_str())
    );
    assert!(
        receipt.earlier.is_empty() && receipt.attention.is_empty(),
        "{receipt:?}"
    );
    let message = f.git(&["log", "-1", "--format=%B"]);
    assert!(message.contains("Agent-Tasks-Intent: PG-"));
    assert!(message.contains(&format!(
        "Agent-Tasks-Effect: - created modules/M-001.yaml - {}",
        journal::sha256_hex(b"alpha")
    )));
    assert_eq!(f.git(&["status", "--porcelain"]), "");
    assert_eq!(f.git(&["show", "HEAD:modules/M-002.yaml"]), "beta");
}

/// Reads, true no-ops and calls without eligible effects never commit.
#[test]
fn no_op_and_empty_calls_do_not_commit() {
    let f = GitFixture::new();
    let before = commits(&f);
    let (store, guard) = call(&f);
    let receipt = commit_now(&store, &guard);
    assert_eq!(
        (receipt.outcome, receipt.reason),
        (GitOutcome::Saved, Some(Reason::NoMutation))
    );
    assert_eq!(commits(&f), before);
}

/// A partial call keeps its saved files, is never committed, and a later success does not certify it.
#[test]
fn partial_call_is_held_and_never_committed_by_a_later_success() {
    let f = GitFixture::new();
    let (store, guard) = call(&f);
    create(&store, "modules/M-001.yaml", b"half");
    let receipt = settle(&store, &guard, &partial(), production_policy());
    assert_eq!(
        (receipt.outcome, receipt.reason),
        (GitOutcome::Deferred, Some(Reason::IncompleteOutcome))
    );
    assert!(f.dir.path().join("modules/M-001.yaml").exists());
    drop(guard);
    let before = commits(&f);
    let (next, guard) = call(&f);
    create(&next, "modules/M-002.yaml", b"whole");
    let receipt = commit_now(&next, &guard);
    assert_eq!(receipt.outcome, GitOutcome::Committed);
    assert_eq!(commits(&f), before + 1);
    assert_eq!(f.git(&["ls-files", "modules/"]), "modules/M-002.yaml");
    assert!(
        receipt.earlier.is_empty(),
        "the held call must not appear as committed"
    );
    assert!(
        receipt
            .pending
            .iter()
            .any(|p| p.phase == super::Phase::Held)
    );
}

/// Earlier successful calls that Git deferred are committed with a later success, reported as extra facts.
#[test]
fn earlier_deferred_successes_land_with_a_later_success() {
    let f = GitFixture::new();
    let (first, guard) = call(&f);
    create(&first, "modules/M-001.yaml", b"one");
    let deferred = defer_now(&first, &guard);
    assert_eq!(
        (deferred.outcome, deferred.reason),
        (GitOutcome::Deferred, Some(Reason::PolicyDeferred))
    );
    drop(guard);
    let (second, guard) = call(&f);
    create(&second, "modules/M-002.yaml", b"two");
    let receipt = commit_now(&second, &guard);
    assert_eq!(receipt.outcome, GitOutcome::Committed);
    assert_eq!(receipt.paths, ["modules/M-002.yaml"]);
    assert_eq!(receipt.earlier.len(), 1);
    assert_eq!(
        (receipt.earlier[0].intents, receipt.earlier[0].paths),
        (1, 1)
    );
    assert_eq!(
        f.git(&["ls-files", "modules/"]),
        "modules/M-001.yaml\nmodules/M-002.yaml"
    );
    let message = f.git(&["log", "-1", "--format=%B"]);
    assert_eq!(message.matches("Agent-Tasks-Intent:").count(), 2);
}

/// User staging on this call's path defers only this call; older intents still commit and the staged
/// bytes stay exactly as the user left them.
#[test]
fn foreign_staging_on_the_current_path_defers_this_call_only() {
    let f = GitFixture::new();
    std::fs::write(f.dir.path().join("shared.md"), "base").unwrap();
    f.git(&["add", "--", "shared.md"]);
    f.git(&["commit", "--quiet", "-m", "fixture: shared"]);
    let (first, guard) = call(&f);
    create(&first, "modules/M-001.yaml", b"older");
    defer_now(&first, &guard);
    drop(guard);
    std::fs::write(f.dir.path().join("shared.md"), "user").unwrap();
    f.git(&["add", "--", "shared.md"]);
    let (second, guard) = call(&f);
    replace(&second, "shared.md", b"mcp", b"user");
    let receipt = commit_now(&second, &guard);
    assert_eq!(receipt.outcome, GitOutcome::Deferred, "{receipt:?}");
    assert_eq!(
        (receipt.reason, receipt.commit.clone()),
        (Some(Reason::ForeignStagingOnPath), None)
    );
    assert_eq!(
        receipt.earlier.len(),
        1,
        "the older intent lands as an extra fact"
    );
    assert_eq!(
        f.git(&["show", ":shared.md"]),
        "user",
        "the user's staged bytes are untouched"
    );
    assert_eq!(
        std::fs::read(f.dir.path().join("shared.md")).unwrap(),
        b"mcp"
    );
    assert_eq!(f.git(&["ls-files", "modules/"]), "modules/M-001.yaml");
}

/// Unrelated staged and dirty files keep their state while the call commits only its own files.
#[test]
fn unrelated_staging_and_dirty_files_are_preserved() {
    let f = GitFixture::new();
    for name in ["staged.md", "dirty.md"] {
        std::fs::write(f.dir.path().join(name), "base").unwrap();
    }
    f.git(&["add", "--", "staged.md", "dirty.md"]);
    f.git(&["commit", "--quiet", "-m", "fixture: neighbors"]);
    std::fs::write(f.dir.path().join("staged.md"), "staged edit").unwrap();
    f.git(&["add", "--", "staged.md"]);
    std::fs::write(f.dir.path().join("dirty.md"), "dirty edit").unwrap();
    let (store, guard) = call(&f);
    create(&store, "modules/M-001.yaml", b"mine");
    let receipt = commit_now(&store, &guard);
    assert_eq!(receipt.outcome, GitOutcome::Committed);
    assert_eq!(f.git(&["diff", "--cached", "--name-only"]), "staged.md");
    assert_eq!(f.git(&["diff", "--name-only"]), "dirty.md");
    assert_eq!(
        f.git(&["show", "--name-only", "--format=", "HEAD"]),
        "modules/M-001.yaml"
    );
}

/// A tracked sibling next to an untracked eligible one is never committed as a subset.
#[test]
fn an_untracked_eligible_sibling_blocks_the_whole_call_but_not_the_save() {
    let f = GitFixture::new();
    let (bulk, guard) = call(&f);
    bulk.create_dir("bulk", &mut Vec::new()).unwrap();
    for n in 0..255 {
        create(&bulk, &format!("bulk/{n}.md"), b"x");
    }
    defer_now(&bulk, &guard);
    drop(guard);
    let (store, guard) = call(&f);
    create(&store, "modules/M-001.yaml", b"tracked");
    create(&store, "modules/M-002.yaml", b"untracked");
    let events = store.publications();
    assert!(
        matches!(events[0].tracking, Tracking::Tracked),
        "{:?}",
        events[0].tracking
    );
    assert!(
        matches!(events[1].tracking, Tracking::Untracked(_)),
        "{:?}",
        events[1].tracking
    );
    let before = commits(&f);
    let receipt = commit_now(&store, &guard);
    assert_eq!(
        (receipt.outcome, receipt.reason),
        (GitOutcome::Deferred, Some(Reason::UntrackedSibling))
    );
    assert_eq!(receipt.untracked, ["modules/M-002.yaml"]);
    assert!(receipt.attention.contains(&Attention::UntrackedMutation));
    assert_eq!(commits(&f), before, "nothing of the call is committed");
    assert!(
        f.dir.path().join("modules/M-001.yaml").exists()
            && f.dir.path().join("modules/M-002.yaml").exists()
    );
}

/// A counter ignored by the user's own rules defers the whole call; the record is never committed alone.
#[test]
fn a_user_ignored_business_file_defers_the_whole_call() {
    let f = GitFixture::new();
    let ignore = f.dir.path().join(".gitignore");
    let mut rules = std::fs::read_to_string(&ignore).unwrap();
    rules.push_str("counter.yaml\n");
    std::fs::write(&ignore, &rules).unwrap();
    f.git(&["add", "--", ".gitignore"]);
    f.git(&["commit", "--quiet", "-m", "fixture: ignore counter"]);
    let before = commits(&f);
    let (store, guard) = call(&f);
    create(&store, "counter.yaml", b"1");
    create(&store, "modules/M-001.yaml", b"record");
    let receipt = commit_now(&store, &guard);
    assert_eq!(
        (receipt.outcome, receipt.reason),
        (GitOutcome::Deferred, Some(Reason::IgnoredBusinessSibling))
    );
    assert_eq!(commits(&f), before);
    assert_eq!(
        std::fs::read_to_string(&ignore).unwrap(),
        rules,
        "the user's ignore file is untouched"
    );
    assert_eq!(f.git(&["ls-files", "modules/", "counter.yaml"]), "");
}

/// A first create of a new home and an ignored backup commit only the owned files.
#[test]
fn directories_and_backups_are_not_staged_or_journaled() {
    let f = GitFixture::new();
    let (store, guard) = call(&f);
    store.create_dir("decisions", &mut Vec::new()).unwrap();
    store
        .create_dir(".agent-tasks/backups", &mut Vec::new())
        .unwrap();
    create(&store, ".agent-tasks/backups/old.yaml", b"legacy");
    create(&store, ".agent-tasks/knowledge.yaml", b"counters");
    create(&store, "decisions/D-001.yaml", b"record");
    let receipt = commit_now(&store, &guard);
    assert_eq!(receipt.outcome, GitOutcome::Committed, "{receipt:?}");
    assert_eq!(
        receipt.paths,
        [".agent-tasks/knowledge.yaml", "decisions/D-001.yaml"]
    );
    assert_eq!(
        f.git(&["show", "--name-only", "--format=", "HEAD"]),
        ".agent-tasks/knowledge.yaml\ndecisions/D-001.yaml"
    );
    let (j, _) = journal::load(&store).unwrap();
    assert!(
        j.intents
            .iter()
            .flat_map(|i| &i.entries)
            .all(|e| !e.path.contains("backups"))
    );
}

/// A directory with an uncertain sync is reported and holds the call as incomplete; it is never durable.
#[test]
fn an_uncertain_directory_sync_holds_the_call() {
    let f = GitFixture::new();
    let (store, guard) = call(&f);
    crate::store::fail_next_directory_sync();
    let error = store.create_dir("decisions", &mut Vec::new()).unwrap_err();
    assert_eq!(error.code, "durability_unknown");
    create(&store, "decisions/D-001.yaml", b"record");
    let before = commits(&f);
    let receipt = commit_now(&store, &guard);
    assert_eq!(
        (receipt.outcome, receipt.reason),
        (GitOutcome::Deferred, Some(Reason::IncompleteOutcome))
    );
    assert!(receipt.attention.contains(&Attention::SyncUncertain));
    assert_eq!(commits(&f), before);
}

/// The same path written by a held call and then a successful call: the later call commits its own final
/// image without blocking and the held intent stays held for its other files.
#[test]
fn a_held_predecessor_does_not_block_a_contiguous_later_call() {
    let f = GitFixture::new();
    let (failed, guard) = call(&f);
    create(&failed, "counter.yaml", b"c1");
    create(&failed, "partial-body.md", b"half");
    settle(&failed, &guard, &partial(), production_policy());
    drop(guard);
    let (next, guard) = call(&f);
    replace(&next, "counter.yaml", b"c2", b"c1");
    create(&next, "decisions-record.yaml", b"record");
    let receipt = commit_now(&next, &guard);
    assert_eq!(receipt.outcome, GitOutcome::Committed, "{receipt:?}");
    assert_eq!(f.git(&["show", "HEAD:counter.yaml"]), "c2");
    assert_eq!(
        f.git(&["ls-files"])
            .lines()
            .filter(|l| l.contains("partial-body"))
            .count(),
        0
    );
    let (j, _) = journal::load(&next).unwrap();
    let held = j.intents.iter().find(|i| i.committed.is_none()).unwrap();
    assert_eq!(held.entries.len(), 2);
    assert!(
        held.entries
            .iter()
            .any(|e| e.path == "counter.yaml" && e.superseded_by.is_some())
    );
    assert!(
        receipt
            .pending
            .iter()
            .any(|p| p.phase == super::Phase::Held)
    );
}

/// Two successful deferred writes to one path commit together: the latest image, true prior digests.
#[test]
fn a_deferred_chain_commits_its_latest_image_with_true_prior_proof() {
    let f = GitFixture::new();
    let (first, guard) = call(&f);
    create(&first, "doc.md", b"v1");
    defer_now(&first, &guard);
    drop(guard);
    let (second, guard) = call(&f);
    replace(&second, "doc.md", b"v2", b"v1");
    let receipt = commit_now(&second, &guard);
    assert_eq!(receipt.outcome, GitOutcome::Committed, "{receipt:?}");
    assert_eq!(f.git(&["show", "HEAD:doc.md"]), "v2");
    let message = f.git(&["log", "-1", "--format=%B"]);
    let v1 = journal::sha256_hex(b"v1");
    let v2 = journal::sha256_hex(b"v2");
    assert!(
        message.contains(&format!("created doc.md - {v1} into={v2}")),
        "{message}"
    );
    assert!(
        message.contains(&format!("replaced doc.md {v1} {v2}")),
        "{message}"
    );
}

/// A native edit after publication is drift: nothing is committed and the edit survives.
#[test]
fn native_drift_is_never_committed() {
    let f = GitFixture::new();
    let (store, guard) = call(&f);
    create(&store, "doc.md", b"mine");
    std::fs::write(f.dir.path().join("doc.md"), b"native").unwrap();
    let before = commits(&f);
    let receipt = commit_now(&store, &guard);
    assert_eq!(receipt.outcome, GitOutcome::Deferred);
    assert!(
        receipt.attention.contains(&Attention::NativeDrift),
        "{receipt:?}"
    );
    assert_eq!(commits(&f), before);
    assert_eq!(
        std::fs::read(f.dir.path().join("doc.md")).unwrap(),
        b"native"
    );
}

/// Install an executable hook script.
#[cfg(unix)]
fn hook(f: &GitFixture, name: &str, body: &str) {
    use std::os::unix::fs::PermissionsExt;
    let path = f.dir.path().join(".git/hooks").join(name);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, format!("#!/bin/sh\n{body}\n")).unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
}

/// A rejecting hook leaves everything saved and pending, the index untouched, and a later success commits both.
#[cfg(unix)]
#[test]
fn a_rejecting_hook_defers_and_a_later_success_recovers() {
    let f = GitFixture::new();
    hook(&f, "pre-commit", "exit 1");
    let (first, guard) = call(&f);
    create(&first, "modules/M-001.yaml", b"one");
    let receipt = commit_now(&first, &guard);
    assert_eq!(
        (receipt.outcome, receipt.reason),
        (GitOutcome::Deferred, Some(Reason::CommitNotCompleted))
    );
    assert_eq!(f.git(&["diff", "--cached", "--name-only"]), "");
    drop(guard);
    std::fs::remove_file(f.dir.path().join(".git/hooks/pre-commit")).unwrap();
    let (second, guard) = call(&f);
    create(&second, "modules/M-002.yaml", b"two");
    let receipt = commit_now(&second, &guard);
    assert_eq!(receipt.outcome, GitOutcome::Committed);
    assert_eq!(receipt.earlier.len(), 1);
    assert_eq!(
        f.git(&["ls-files", "modules/"]),
        "modules/M-001.yaml\nmodules/M-002.yaml"
    );
}

/// A hook that rewrites the worktree is reported, never reset or silently accepted.
#[cfg(unix)]
#[test]
fn a_hook_that_changes_the_worktree_is_reported() {
    let f = GitFixture::new();
    hook(&f, "pre-commit", "echo tamper >> doc.md");
    let (store, guard) = call(&f);
    create(&store, "doc.md", b"mine");
    let receipt = commit_now(&store, &guard);
    assert_eq!(receipt.outcome, GitOutcome::Committed, "{receipt:?}");
    assert!(receipt.attention.contains(&Attention::HookChangedWorktree));
    assert_eq!(f.git(&["show", "HEAD:doc.md"]), "mine");
    assert_ne!(
        std::fs::read(f.dir.path().join("doc.md")).unwrap(),
        b"mine",
        "the hook's edit is not reset"
    );
}

/// A signing failure behaves like any rejected commit.
#[test]
fn a_signing_failure_defers_without_touching_the_index() {
    let f = GitFixture::new();
    f.git(&["config", "commit.gpgsign", "true"]);
    f.git(&["config", "gpg.program", "false"]);
    let (store, guard) = call(&f);
    create(&store, "doc.md", b"mine");
    let before = commits(&f);
    let receipt = commit_now(&store, &guard);
    assert_eq!(
        (receipt.outcome, receipt.reason),
        (GitOutcome::Deferred, Some(Reason::CommitNotCompleted))
    );
    assert_eq!(commits(&f), before);
    assert_eq!(f.git(&["diff", "--cached", "--name-only"]), "");
}

/// A stuck hook hits the deadline: outcome unknown, files saved, and a later settlement neither
/// duplicates the commit nor replays the business write.
#[cfg(unix)]
#[test]
fn a_timeout_is_unknown_and_reconciles_without_a_duplicate_commit() {
    let f = GitFixture::new();
    // The hook leaves a marker inside .git before it blocks, so the test proves the deadline expired
    // while the commit command was alive rather than assuming the clock was quick enough.
    hook(&f, "pre-commit", "touch .git/hook-started; sleep 60");
    let (store, guard) = call(&f);
    create(&store, "doc.md", b"mine");
    let budget = std::time::Duration::from_secs(10);
    let started = std::time::Instant::now();
    let receipt = super::engine::settle_by(
        &store,
        &guard,
        &success(),
        production_policy(),
        started + budget,
    );
    assert!(
        f.dir.path().join(".git/hook-started").exists(),
        "the budget expired before the commit hook ran; the machine is too loaded for this proof"
    );
    assert!(
        started.elapsed() >= budget,
        "the outcome was decided before the deadline"
    );
    assert_eq!(receipt.outcome, GitOutcome::Unknown, "{receipt:?}");
    assert!(f.dir.path().join("doc.md").exists());
    drop(guard);
    std::fs::remove_file(f.dir.path().join(".git/hooks/pre-commit")).unwrap();
    let before = commits(&f);
    let (next, guard) = call(&f);
    create(&next, "other.md", b"next");
    let receipt = commit_now(&next, &guard);
    assert_eq!(receipt.outcome, GitOutcome::Committed);
    assert_eq!(
        commits(&f),
        before + 1,
        "one commit holds both; none was duplicated"
    );
    assert_eq!(
        f.git(&["ls-files", "doc.md", "other.md"]),
        "doc.md\nother.md"
    );
}

/// A landed commit whose reply was lost is found through its trailer and never committed twice.
#[test]
fn a_lost_commit_reply_reconciles_through_the_trailer() {
    let f = GitFixture::new();
    let parent = f.git(&["rev-parse", "HEAD"]);
    let (store, guard) = call(&f);
    create(&store, "doc.md", b"mine");
    let receipt = commit_now(&store, &guard);
    assert_eq!(receipt.outcome, GitOutcome::Committed);
    let landed = f.git(&["rev-parse", "HEAD"]);
    let (mut j, observed) = journal::load(&store).unwrap();
    j.intents[0].committed = None;
    j.intents[0].committing_from = Some(parent);
    journal::save(&store, &j, observed.as_deref()).unwrap();
    super::engine::reconcile_committing(
        &store,
        std::time::Instant::now() + std::time::Duration::from_secs(10),
    );
    let (j, _) = journal::load(&store).unwrap();
    assert_eq!(j.intents[0].committed.as_deref(), Some(landed.as_str()));
    assert_eq!(commits(&f), 2);
}

/// Unsuitable repository states defer with their own reason and change nothing.
#[test]
fn unsuitable_repository_states_defer_truthfully() {
    for (case, expected) in [
        ("detached", Reason::DetachedHead),
        ("merge", Reason::OperationInProgress),
        ("lock", Reason::IndexLocked),
    ] {
        let f = GitFixture::new();
        match case {
            "detached" => {
                f.git(&["checkout", "--quiet", "--detach"]);
            }
            "merge" => {
                std::fs::write(f.dir.path().join(".git/MERGE_HEAD"), "0".repeat(40)).unwrap()
            }
            _ => std::fs::write(f.dir.path().join(".git/index.lock"), "").unwrap(),
        }
        let before = commits(&f);
        let (store, guard) = call(&f);
        create(&store, "doc.md", b"mine");
        let receipt = commit_now(&store, &guard);
        assert_eq!(
            (receipt.outcome, receipt.reason),
            (GitOutcome::Deferred, Some(expected)),
            "{case}"
        );
        assert_eq!(commits(&f), before, "{case}");
        assert!(f.dir.path().join("doc.md").exists(), "{case}");
    }
}

/// A call whose own intent alone exceeds the message budget is deferred and never truncated.
#[test]
fn an_oversized_call_is_deferred_by_the_message_budget() {
    let f = GitFixture::new();
    let (store, guard) = call(&f);
    store.create_dir("long", &mut Vec::new()).unwrap();
    for n in 0..230 {
        let name = format!("long/{:0>200}.md", n);
        create(&store, &name, b"x");
    }
    let before = commits(&f);
    let receipt = commit_now(&store, &guard);
    assert_eq!(
        (receipt.outcome, receipt.reason),
        (GitOutcome::Deferred, Some(Reason::MessageBudget)),
        "{:?}",
        receipt.reason
    );
    assert_eq!(commits(&f), before);
}

/// Digest of literal bytes, as the oracle and locators expect.
fn digest(bytes: &[u8]) -> String {
    journal::sha256_hex(bytes)
}

/// An expected effect assertion for a creation.
fn expect_created(relative: &str, bytes: &[u8]) -> super::ExpectedEffect {
    super::ExpectedEffect {
        relative: relative.into(),
        kind: crate::store::EffectKind::Created,
        after_sha256: Some(digest(bytes)),
    }
}

/// The oracle answers from the journal while pending and from history trailers after the journal is pruned.
#[test]
fn the_oracle_attests_pending_and_committed_effects() {
    let f = GitFixture::new();
    let op = OperationId::new("oracle:one").unwrap();
    let (first, guard) = call(&f);
    create_op(&first, "doc.md", b"body", "oracle:one");
    defer_now(&first, &guard);
    drop(guard);
    let expected = [expect_created("doc.md", b"body")];
    let super::EffectStatus::Attested(pending) = super::effect_status(&first, &op, &expected)
    else {
        panic!("expected Attested while pending");
    };
    assert!(matches!(pending.paths[0].git, super::EffectGit::Pending(_)));
    let (second, guard) = call(&f);
    create(&second, "other.md", b"x");
    assert_eq!(commit_now(&second, &guard).outcome, GitOutcome::Committed);
    let super::EffectStatus::Attested(done) = super::effect_status(&second, &op, &expected) else {
        panic!("expected Attested after commit");
    };
    assert!(
        done.complete && matches!(done.paths[0].git, super::EffectGit::Committed { .. }),
        "{done:?}"
    );
}

/// Missing, foreign and look-alike evidence is never certified.
#[test]
fn the_oracle_reports_partial_not_published_foreign_and_unproven() {
    let f = GitFixture::new();
    let (store, guard) = call(&f);
    create_op(&store, "a.md", b"a", "oracle:two");
    commit_now(&store, &guard);
    drop(guard);
    let op = OperationId::new("oracle:two").unwrap();
    let both = [expect_created("a.md", b"a"), expect_created("b.md", b"b")];
    match super::effect_status(&store, &op, &both) {
        super::EffectStatus::Partial { missing, .. } => assert_eq!(missing, ["b.md"]),
        other => panic!("expected Partial, got {other:?}"),
    }
    let nobody = OperationId::new("oracle:none").unwrap();
    assert!(matches!(
        super::effect_status(&store, &nobody, &[expect_created("zz.md", b"z")]),
        super::EffectStatus::NotPublished
    ));
    assert!(matches!(
        super::effect_status(&store, &op, &[expect_created("a.md", b"different")]),
        super::EffectStatus::Foreign { .. }
    ));
    std::fs::write(f.dir.path().join("look.md"), b"same").unwrap();
    let alike = OperationId::new("oracle:alike").unwrap();
    assert!(matches!(
        super::effect_status(&store, &alike, &[expect_created("look.md", b"same")]),
        super::EffectStatus::Unknown {
            reason: super::UnknownReason::UnattestedEqualBytes,
            ..
        }
    ));
}

/// The call summary separates tracked, untracked, uncertain and not-applicable events.
#[test]
fn call_intents_summarizes_the_current_call_only() {
    let f = GitFixture::new();
    let (store, guard) = call(&f);
    store.create_dir("decisions", &mut Vec::new()).unwrap();
    create(&store, "decisions/D-001.yaml", b"r");
    let summary = super::call_intents(&store);
    assert_eq!(
        (
            summary.tracked,
            summary.untracked,
            summary.uncertain,
            summary.not_applicable
        ),
        (1, 0, 0, 1)
    );
    assert!(summary.all_tracked && summary.intents.len() == 1);
    drop(guard);
    let (fresh, _g) = call(&f);
    assert!(super::call_intents(&fresh).intents.is_empty());
}

/// Committed bytes are proven by digest and reachable commit; everything else says why not.
#[test]
fn locators_prove_only_committed_exact_bytes() {
    use super::locator::{Item, NotCommittedReason, Proof, committed_original};
    let f = GitFixture::new();
    let (store, guard) = call(&f);
    create(&store, "doc.md", b"first");
    commit_now(&store, &guard);
    drop(guard);
    let item = |relative: &'static str, bytes: &[u8]| (relative, digest(bytes), bytes.len() as u64);
    let (relative, sha, len) = item("doc.md", b"first");
    let Proof::Committed(locator) = committed_original(
        &store,
        &Item {
            relative,
            sha256: &sha,
            len,
            at: None,
        },
    ) else {
        panic!("expected Committed");
    };
    let wire = locator.encode().unwrap();
    assert_eq!(
        super::locator::Locator::parse(&wire).unwrap().commit,
        locator.commit
    );
    assert_eq!(
        super::locator::read_committed(&store, &wire, 1024).unwrap(),
        b"first"
    );
    let (_, sha, len) = item("doc.md", b"edited");
    std::fs::write(f.dir.path().join("doc.md"), b"edited").unwrap();
    assert!(matches!(
        committed_original(
            &store,
            &Item {
                relative: "doc.md",
                sha256: &sha,
                len,
                at: None
            }
        ),
        Proof::NotCommitted(NotCommittedReason::Dirty)
    ));
    let (_, sha, len) = item("doc.md", b"third");
    assert!(matches!(
        committed_original(
            &store,
            &Item {
                relative: "doc.md",
                sha256: &sha,
                len,
                at: None
            }
        ),
        Proof::Mismatch { .. }
    ));
    std::fs::write(f.dir.path().join("loose.md"), b"loose").unwrap();
    let (_, sha, len) = item("loose.md", b"loose");
    assert!(matches!(
        committed_original(
            &store,
            &Item {
                relative: "loose.md",
                sha256: &sha,
                len,
                at: None
            }
        ),
        Proof::NotCommitted(NotCommittedReason::Untracked)
    ));
    let (store2, guard2) = call(&f);
    create(&store2, "kept.md", b"kept");
    defer_now(&store2, &guard2);
    let (_, sha, len) = item("kept.md", b"kept");
    assert!(
        matches!(
            committed_original(
                &store2,
                &Item {
                    relative: "kept.md",
                    sha256: &sha,
                    len,
                    at: None
                }
            ),
            Proof::NotCommitted(NotCommittedReason::Untracked)
        ),
        "a pending file is never proof"
    );
}

/// Release drops tracking without touching files; stale snapshots are refused.
#[test]
fn recovery_release_and_stale_version() {
    use super::recover::{Action, recover};
    let f = GitFixture::new();
    let (store, guard) = call(&f);
    create(&store, "doc.md", b"mine");
    let receipt = defer_now(&store, &guard);
    let intent = receipt.pending[0].intent.clone();
    let stale = recover(
        &store,
        &guard,
        "not-the-version",
        Action::Release(vec![intent.clone()]),
    );
    assert_eq!(stale.unwrap_err().code, "stale");
    let version = super::pending_version(&store);
    let report = recover(&store, &guard, &version, Action::Release(vec![intent])).unwrap();
    assert!(report.changed && report.receipt.pending.is_empty());
    assert!(f.dir.path().join("doc.md").exists());
    assert_eq!(f.git(&["ls-files", "doc.md"]), "");
}

/// Retry commits a held intent only on explicit request, with its exact bytes.
#[test]
fn recovery_retry_commits_a_held_intent() {
    use super::recover::{Action, recover};
    let f = GitFixture::new();
    let (store, guard) = call(&f);
    create(&store, "doc.md", b"mine");
    let receipt = settle(&store, &guard, &partial(), production_policy());
    let intent = receipt.pending[0].intent.clone();
    drop(guard);
    let (next, guard) = call(&f);
    let version = super::pending_version(&next);
    let report = recover(&next, &guard, &version, Action::Retry(vec![intent])).unwrap();
    assert!(report.changed, "{:?}", report.receipt);
    assert_eq!(report.receipt.outcome, GitOutcome::Committed);
    assert_eq!(f.git(&["show", "HEAD:doc.md"]), "mine");
}

/// Preserve commits exactly the authorized bytes and refuses a mismatch.
#[test]
fn recovery_preserve_requires_the_authorized_bytes() {
    use super::recover::{Action, PreserveItem, recover};
    let f = GitFixture::new();
    std::fs::write(f.dir.path().join("note.md"), b"native").unwrap();
    let (store, guard) = call(&f);
    let version = super::pending_version(&store);
    let wrong = PreserveItem {
        relative: "note.md".into(),
        sha256: digest(b"other"),
        len: 5,
    };
    let refused = recover(&store, &guard, &version, Action::Preserve(vec![wrong]));
    assert_eq!(refused.unwrap_err().code, "stale");
    assert_eq!(f.git(&["ls-files", "note.md"]), "");
    let right = PreserveItem {
        relative: "note.md".into(),
        sha256: digest(b"native"),
        len: 6,
    };
    let report = recover(&store, &guard, &version, Action::Preserve(vec![right])).unwrap();
    assert!(report.changed);
    assert_eq!(f.git(&["show", "HEAD:note.md"]), "native");
}

/// Detached and unborn HEADs are never proof of committed bytes.
#[test]
fn locators_refuse_detached_and_unborn_heads() {
    use super::locator::{Item, NotCommittedReason, Proof, committed_original};
    let f = GitFixture::new();
    let (store, guard) = call(&f);
    create(&store, "doc.md", b"first");
    commit_now(&store, &guard);
    drop(guard);
    let sha = digest(b"first");
    let item = Item {
        relative: "doc.md",
        sha256: &sha,
        len: 5,
        at: None,
    };
    f.git(&["checkout", "--quiet", "--orphan", "fresh"]);
    assert!(matches!(
        committed_original(&store, &item),
        Proof::NotCommitted(NotCommittedReason::UnbornHead)
    ));
    f.git(&["checkout", "--quiet", "--force", "main"]);
    f.git(&["checkout", "--quiet", "--detach"]);
    assert!(matches!(
        committed_original(&store, &item),
        Proof::NotCommitted(NotCommittedReason::DetachedHead)
    ));
}

/// An unknown intent is only adopted by explicit recovery, then commits with its exact bytes.
#[test]
fn recovery_adopt_turns_an_unknown_intent_into_a_commit() {
    use super::recover::{Action, recover};
    let f = GitFixture::new();
    let (store, guard) = call(&f);
    create(&store, "doc.md", b"crashed");
    defer_now(&store, &guard);
    let (mut j, observed) = journal::load(&store).unwrap();
    j.intents[0].outcome = IntentOutcome::Unsettled;
    j.intents[0].entries[0].phase = journal::EntryPhase::Prepared;
    journal::save(&store, &j, observed.as_deref()).unwrap();
    let id = j.intents[0].id.clone();
    assert_eq!(super::pending(&store).refs[0].phase, super::Phase::Unknown);
    let before = commits(&f);
    let receipt = commit_now(&store, &guard);
    assert_eq!(
        commits(&f),
        before,
        "an unknown intent is never committed by a later success: {receipt:?}"
    );
    let version = super::pending_version(&store);
    let adopted = recover(&store, &guard, &version, Action::Adopt(vec![id.clone()])).unwrap();
    assert!(adopted.changed);
    let version = super::pending_version(&store);
    let report = recover(&store, &guard, &version, Action::Retry(vec![id])).unwrap();
    assert_eq!(
        report.receipt.outcome,
        GitOutcome::Committed,
        "{:?}",
        report.receipt
    );
    assert_eq!(f.git(&["show", "HEAD:doc.md"]), "crashed");
    assert!(
        f.git(&["log", "-1", "--format=%B"])
            .contains("Agent-Tasks-Adopted")
    );
}

/// A full journal drops only committed intents whose commit is reachable and keeps the rest.
#[test]
fn a_full_journal_prunes_only_verified_committed_intents() {
    let f = GitFixture::new();
    let (store, guard) = call(&f);
    create(&store, "doc.md", b"first");
    assert_eq!(commit_now(&store, &guard).outcome, GitOutcome::Committed);
    drop(guard);
    let (mut j, observed) = journal::load(&store).unwrap();
    let template = j.intents[0].clone();
    let real = template.committed.clone().unwrap();
    for n in 0..63 {
        let mut copy = template.clone();
        copy.id = format!("PG-{n:024x}");
        copy.entries.clear();
        if n == 0 {
            copy.committed = Some("1".repeat(40));
        }
        j.intents.push(copy);
    }
    journal::save(&store, &j, observed.as_deref()).unwrap();
    assert_eq!(journal::load(&store).unwrap().0.intents.len(), 64);
    let (next, _guard) = call(&f);
    create(&next, "later.md", b"second");
    assert!(matches!(next.publications()[0].tracking, Tracking::Tracked));
    let (after, _) = journal::load(&next).unwrap();
    assert!(
        after
            .intents
            .iter()
            .all(|i| i.committed.as_deref() != Some(real.as_str()))
    );
    assert!(
        after
            .intents
            .iter()
            .any(|i| i.committed.as_deref() == Some("1".repeat(40).as_str())),
        "an unreachable commit is never trusted for pruning"
    );
    assert!(after.intents.iter().any(|i| i.committed.is_none()));
}

/// Explicit Reconcile resolves a committing intent whose reply was lost.
#[test]
fn recovery_reconcile_resolves_a_lost_commit_reply() {
    use super::recover::{Action, recover};
    let f = GitFixture::new();
    let parent = f.git(&["rev-parse", "HEAD"]);
    let (store, guard) = call(&f);
    create(&store, "doc.md", b"mine");
    assert_eq!(commit_now(&store, &guard).outcome, GitOutcome::Committed);
    let landed = f.git(&["rev-parse", "HEAD"]);
    let (mut j, observed) = journal::load(&store).unwrap();
    let id = j.intents[0].id.clone();
    j.intents[0].committed = None;
    j.intents[0].committing_from = Some(parent);
    journal::save(&store, &j, observed.as_deref()).unwrap();
    let version = super::pending_version(&store);
    let report = recover(&store, &guard, &version, Action::Reconcile(vec![id])).unwrap();
    assert!(report.changed);
    let (j, _) = journal::load(&store).unwrap();
    assert_eq!(j.intents[0].committed.as_deref(), Some(landed.as_str()));
    assert_eq!(commits(&f), 2, "reconcile never commits again");
}

/// A required attestation that the journal cannot retain refuses before any effect.
#[test]
fn required_attestation_refuses_without_an_effect_when_the_journal_is_unusable() {
    let f = GitFixture::new();
    let (store, _guard) = call(&f);
    let op = OperationId::new("required:one").unwrap();
    let no_operation = store.publish_with(
        Publish {
            relative: "a.md",
            bytes: b"a",
            observed: None,
            cap: RECORD_CAP,
            operation: None,
            attest: Attest::Required,
        },
        &mut Vec::new(),
    );
    assert!(no_operation.is_err());
    assert!(!f.dir.path().join("a.md").exists());
    std::fs::create_dir_all(f.dir.path().join(".git/agent-tasks")).unwrap();
    std::fs::write(
        f.dir.path().join(".git/agent-tasks/pending.yaml"),
        b"not: [valid",
    )
    .unwrap();
    let corrupt = store.publish_with(
        Publish {
            relative: "b.md",
            bytes: b"b",
            observed: None,
            cap: RECORD_CAP,
            operation: Some(&op),
            attest: Attest::Required,
        },
        &mut Vec::new(),
    );
    assert_eq!(corrupt.unwrap_err().code, "attestation_unavailable");
    assert!(
        !f.dir.path().join("b.md").exists(),
        "no effect without a retained entry"
    );
    create(&store, "c.md", b"c");
    assert!(
        matches!(store.publications()[0].tracking, Tracking::Untracked(_)),
        "optional writes still succeed and say untracked"
    );
    assert!(f.dir.path().join("c.md").exists());
    assert_eq!(
        std::fs::read(f.dir.path().join(".git/agent-tasks/pending.yaml")).unwrap(),
        b"not: [valid",
        "corrupt evidence is left in place"
    );
}

/// Replaying a recovery changes nothing: a committed or released intent is no longer pending.
#[test]
fn recovery_is_idempotent() {
    use super::recover::{Action, recover};
    let f = GitFixture::new();
    let (store, guard) = call(&f);
    create(&store, "doc.md", b"mine");
    let id = settle(&store, &guard, &partial(), production_policy()).pending[0]
        .intent
        .clone();
    let version = super::pending_version(&store);
    let first = recover(&store, &guard, &version, Action::Retry(vec![id.clone()])).unwrap();
    assert_eq!(first.receipt.outcome, GitOutcome::Committed);
    let after = commits(&f);
    let version = super::pending_version(&store);
    let again = recover(&store, &guard, &version, Action::Retry(vec![id.clone()]));
    assert_eq!(again.err().unwrap().code, "recovery_blocked");
    let release = recover(&store, &guard, &version, Action::Release(vec![id]));
    assert_eq!(release.err().unwrap().code, "recovery_blocked");
    assert_eq!(commits(&f), after, "a replay never commits again");
}

/// A removal is committed as a deletion of exactly the removed path.
#[test]
fn a_removal_commits_as_a_deletion() {
    let f = GitFixture::new();
    let (first, guard) = call(&f);
    create(&first, "doc.md", b"mine");
    commit_now(&first, &guard);
    drop(guard);
    let (second, guard) = call(&f);
    second
        .remove(
            crate::store::Remove {
                relative: "doc.md",
                observed: b"mine",
                cap: RECORD_CAP,
                operation: None,
                attest: Attest::Optional,
            },
            &mut Vec::new(),
        )
        .unwrap();
    let receipt = commit_now(&second, &guard);
    assert_eq!(receipt.outcome, GitOutcome::Committed, "{receipt:?}");
    assert_eq!(f.git(&["ls-files", "doc.md"]), "");
    assert_eq!(
        f.git(&["show", "--name-status", "--format=", "HEAD"]),
        "D\tdoc.md"
    );
}

/// A linked worktree or submodule (`.git` is a file) and unmerged paths are refused with their own reasons.
#[test]
fn linked_checkouts_and_unmerged_paths_defer() {
    let f = GitFixture::new();
    let linked = tempfile::tempdir().unwrap();
    f.git(&[
        "worktree",
        "add",
        "--quiet",
        "-b",
        "side",
        linked.path().join("w").to_str().unwrap(),
    ]);
    let store = Store::from_root(&linked.path().join("w")).unwrap();
    store.prepare(&mut Vec::new()).unwrap();
    let guard = store.lock(true, &mut Vec::new()).unwrap().unwrap();
    create(&store, "doc.md", b"mine");
    let receipt = commit_now(&store, &guard);
    assert_eq!(
        (receipt.outcome, receipt.reason),
        (GitOutcome::Saved, Some(Reason::NoRepository)),
        "a linked checkout is never journaled or committed"
    );
    drop(guard);
    std::fs::write(f.dir.path().join("c.md"), "base").unwrap();
    f.git(&["add", "--", "c.md"]);
    f.git(&["commit", "--quiet", "-m", "fixture: c"]);
    f.git(&["checkout", "--quiet", "-b", "other"]);
    std::fs::write(f.dir.path().join("c.md"), "other").unwrap();
    f.git(&["commit", "--quiet", "-am", "fixture: other"]);
    f.git(&["checkout", "--quiet", "main"]);
    std::fs::write(f.dir.path().join("c.md"), "main").unwrap();
    f.git(&["commit", "--quiet", "-am", "fixture: main"]);
    let merge = super::git::run(
        f.dir.path(),
        &["merge", "other"],
        None,
        std::time::Instant::now() + std::time::Duration::from_secs(20),
    )
    .unwrap();
    assert!(!merge.success());
    let (conflicted, guard) = call(&f);
    create(&conflicted, "doc.md", b"mine");
    let receipt = commit_now(&conflicted, &guard);
    assert_eq!(receipt.outcome, GitOutcome::Deferred);
    assert!(
        matches!(
            receipt.reason,
            Some(Reason::OperationInProgress | Reason::UnmergedPaths)
        ),
        "{receipt:?}"
    );
}

/// A line-ending filter makes the committed blob differ from the recorded bytes: reported, never certified.
#[test]
fn a_line_ending_filter_is_reported_not_certified() {
    let f = GitFixture::new();
    std::fs::write(f.dir.path().join(".gitattributes"), "*.txt text eol=crlf\n").unwrap();
    f.git(&["add", "--", ".gitattributes"]);
    f.git(&["commit", "--quiet", "-m", "fixture: attributes"]);
    let (store, guard) = call(&f);
    create(&store, "note.txt", b"one\ntwo\n");
    let receipt = commit_now(&store, &guard);
    assert_eq!(receipt.outcome, GitOutcome::Committed, "{receipt:?}");
    assert_eq!(
        std::fs::read(f.dir.path().join("note.txt")).unwrap(),
        b"one\ntwo\n",
        "the file is never rewritten"
    );
}

/// Output beyond the cap is cut and flagged, never buffered whole.
#[test]
fn oversized_git_output_is_capped() {
    let f = GitFixture::new();
    std::fs::write(f.dir.path().join("big.bin"), vec![b'x'; 2 * 1024 * 1024]).unwrap();
    let blob = f.git(&["hash-object", "-w", "big.bin"]);
    let out = super::git::run(
        f.dir.path(),
        &["cat-file", "blob", &blob],
        None,
        std::time::Instant::now() + std::time::Duration::from_secs(20),
    )
    .unwrap();
    assert!(out.truncated && out.stdout.len() == super::git::STDOUT_CAP);
}

/// A locator that would exceed 256 bytes fails instead of truncating.
#[test]
fn an_over_long_locator_fails() {
    let locator = super::locator::Locator {
        commit: "a".repeat(40),
        blob: "b".repeat(40),
        relative: "d/".repeat(100),
        sha256: digest(b"x"),
        len: 1,
    };
    assert_eq!(locator.encode().unwrap_err().code, "locator_too_long");
}

/// Generated same-operation effects come back unasserted, and trailers keep the evidence after the journal is pruned.
#[test]
fn the_oracle_returns_generated_effects_and_survives_a_pruned_journal() {
    let f = GitFixture::new();
    let op = OperationId::new("oracle:generated").unwrap();
    let (store, guard) = call(&f);
    create_op(&store, "doc name.md", b"body", "oracle:generated");
    store
        .publish_with(
            Publish {
                relative: "meta.yaml",
                bytes: b"generated",
                observed: None,
                cap: RECORD_CAP,
                operation: Some(&op),
                attest: Attest::Required,
            },
            &mut Vec::new(),
        )
        .unwrap();
    commit_now(&store, &guard);
    drop(guard);
    assert!(
        f.git(&["log", "-1", "--format=%B"])
            .contains("doc%20name.md"),
        "paths are percent encoded in trailers"
    );
    let (mut j, observed) = journal::load(&store).unwrap();
    j.intents.clear();
    journal::save(&store, &j, observed.as_deref()).unwrap();
    let super::EffectStatus::Attested(receipt) =
        super::effect_status(&store, &op, &[expect_created("doc name.md", b"body")])
    else {
        panic!("expected Attested from history alone");
    };
    assert_eq!(receipt.paths.len(), 2);
    assert!(
        receipt
            .paths
            .iter()
            .any(|p| p.relative == "meta.yaml" && !p.asserted)
    );
    assert!(
        receipt
            .paths
            .iter()
            .all(|p| matches!(p.git, super::EffectGit::Committed { .. }))
    );
}

/// Open intent identities in journal order.
fn open_intents(store: &Store) -> Vec<String> {
    journal::load(store)
        .unwrap()
        .0
        .intents
        .iter()
        .filter(|i| i.committed.is_none())
        .map(|i| i.id.clone())
        .collect()
}

/// A hook that stages different bytes into the commit's own index must not be certified as committed.
#[cfg(unix)]
#[test]
fn a_hook_that_stages_other_bytes_is_not_certified() {
    let f = GitFixture::new();
    hook(&f, "pre-commit", "echo tamper >> doc.md; git add doc.md");
    let (store, guard) = call(&f);
    create(&store, "doc.md", b"mine");
    let receipt = commit_now(&store, &guard);
    assert_eq!(receipt.outcome, GitOutcome::Unknown, "{receipt:?}");
    assert!(
        receipt.attention.contains(&Attention::TreeMismatch),
        "{receipt:?}"
    );
    assert_eq!(
        open_intents(&store).len(),
        1,
        "an unverified commit leaves the intent open"
    );
    assert!(
        journal::load(&store).unwrap().0.intents[0]
            .committed
            .is_none()
    );
}

/// A landed commit whose tree or trailers disagree with the journal is never recorded as committed.
#[test]
fn reconcile_never_certifies_a_commit_that_disagrees_with_the_journal() {
    let f = GitFixture::new();
    let parent = f.git(&["rev-parse", "HEAD"]);
    let (store, guard) = call(&f);
    create(&store, "doc.md", b"mine");
    assert_eq!(commit_now(&store, &guard).outcome, GitOutcome::Committed);
    let (mut j, observed) = journal::load(&store).unwrap();
    j.intents[0].committed = None;
    j.intents[0].committing_from = Some(parent);
    j.intents[0].entries[0].after_sha256 = Some(digest(b"something else"));
    journal::save(&store, &j, observed.as_deref()).unwrap();
    super::engine::reconcile_committing(
        &store,
        std::time::Instant::now() + std::time::Duration::from_secs(10),
    );
    let (j, _) = journal::load(&store).unwrap();
    assert!(
        j.intents[0].committed.is_none(),
        "a trailer or tree mismatch is not proof"
    );
}

/// A forced retry that names part of a connected path chain is blocked, naming the rest.
#[test]
fn retry_requires_the_whole_connected_chain() {
    use super::recover::{Action, recover};
    let f = GitFixture::new();
    let (first, guard) = call(&f);
    create(&first, "doc.md", b"v1");
    defer_now(&first, &guard);
    drop(guard);
    let (second, guard) = call(&f);
    replace(&second, "doc.md", b"v2", b"v1");
    defer_now(&second, &guard);
    let ids = open_intents(&second);
    assert_eq!(ids.len(), 2);
    let before = commits(&f);
    let version = super::pending_version(&second);
    let error = recover(
        &second,
        &guard,
        &version,
        Action::Retry(vec![ids[0].clone()]),
    )
    .err()
    .unwrap();
    assert_eq!(error.code, "recovery_blocked");
    assert!(error.message.contains(&ids[1]), "{}", error.message);
    assert_eq!(commits(&f), before, "a split chain is never committed");
    let version = super::pending_version(&second);
    let both = recover(&second, &guard, &version, Action::Retry(ids)).unwrap();
    assert_eq!(
        both.receipt.outcome,
        GitOutcome::Committed,
        "{:?}",
        both.receipt
    );
    assert_eq!(f.git(&["show", "HEAD:doc.md"]), "v2");
}

/// A held counter write superseded by a committed later call must not poison the next counter write.
#[test]
fn a_superseded_held_counter_does_not_poison_later_calls() {
    use super::recover::{Action, recover};
    let f = GitFixture::new();
    let (zero, guard) = call(&f);
    create(&zero, "counter.yaml", b"c0");
    commit_now(&zero, &guard);
    drop(guard);
    let (held, guard) = call(&f);
    replace(&held, "counter.yaml", b"c1", b"c0");
    create(&held, "body.md", b"half");
    settle(&held, &guard, &partial(), production_policy());
    let held_id = open_intents(&held)[0].clone();
    drop(guard);
    let (second, guard) = call(&f);
    replace(&second, "counter.yaml", b"c2", b"c1");
    assert_eq!(commit_now(&second, &guard).outcome, GitOutcome::Committed);
    drop(guard);
    let (third, guard) = call(&f);
    replace(&third, "counter.yaml", b"c3", b"c2");
    let receipt = commit_now(&third, &guard);
    assert_eq!(receipt.outcome, GitOutcome::Committed, "{receipt:?}");
    assert_eq!(f.git(&["show", "HEAD:counter.yaml"]), "c3");
    let version = super::pending_version(&third);
    let retry = recover(&third, &guard, &version, Action::Retry(vec![held_id])).unwrap();
    assert_eq!(
        retry.receipt.outcome,
        GitOutcome::Committed,
        "{:?}",
        retry.receipt
    );
    assert_eq!(
        f.git(&["show", "HEAD:counter.yaml"]),
        "c3",
        "the held old counter is never recommitted"
    );
    assert_eq!(f.git(&["show", "HEAD:body.md"]), "half");
}

/// A removal under an operation identity waits while an older held replacement of that identity exists.
#[test]
fn a_removal_waits_for_an_older_held_replacement_of_the_same_operation() {
    let f = GitFixture::new();
    let (zero, guard) = call(&f);
    create(&zero, "old.md", b"old");
    commit_now(&zero, &guard);
    drop(guard);
    let (held, guard) = call(&f);
    create_op(&held, "new.md", b"new", "compact:one");
    settle(&held, &guard, &partial(), production_policy());
    drop(guard);
    let (removal, guard) = call(&f);
    let op = OperationId::new("compact:one").unwrap();
    removal
        .remove(
            crate::store::Remove {
                relative: "old.md",
                observed: b"old",
                cap: RECORD_CAP,
                operation: Some(&op),
                attest: Attest::Required,
            },
            &mut Vec::new(),
        )
        .unwrap();
    let before = commits(&f);
    let receipt = commit_now(&removal, &guard);
    assert_eq!(
        (receipt.outcome, receipt.reason),
        (GitOutcome::Deferred, Some(Reason::RemovalBarrier)),
        "{receipt:?}"
    );
    assert_eq!(commits(&f), before);
    assert_eq!(
        f.git(&["ls-files", "old.md"]),
        "old.md",
        "the original stays in history until the replacement is committed"
    );
}

/// More matching history than the bounded scan can read is incomplete, never complete.
#[test]
fn a_history_longer_than_the_scan_is_unknown_not_attested() {
    let f = GitFixture::new();
    let (store, _guard) = call(&f);
    for n in 0..66 {
        let name = format!("h{n}.md");
        let body = format!("body {n}");
        std::fs::write(f.dir.path().join(&name), &body).unwrap();
        f.git(&["add", "--", &name]);
        let message = format!(
            "docs: history {n}\n\nAgent-Tasks-Intent: PG-{n:024x}\nAgent-Tasks-Effect: hist:op created {name} - {}\n",
            digest(body.as_bytes())
        );
        f.git(&["commit", "--quiet", "-m", &message]);
    }
    let op = OperationId::new("hist:op").unwrap();
    let newest = expect_created("h65.md", b"body 65");
    match super::effect_status(&store, &op, &[newest]) {
        super::EffectStatus::Unknown { reason, .. } => {
            assert_eq!(reason, super::UnknownReason::HistoryScanIncomplete);
        }
        other => panic!("expected Unknown, got {other:?}"),
    }
}

/// An unknown preserve outcome keeps its intent so a later reconcile can prove or release it.
#[cfg(unix)]
#[test]
fn an_unknown_preserve_keeps_its_intent_and_does_not_claim_nothing_committed() {
    use super::recover::{Action, PreserveItem, recover_by};
    let f = GitFixture::new();
    std::fs::write(f.dir.path().join("note.md"), b"native").unwrap();
    hook(&f, "pre-commit", "touch .git/hook-started; sleep 60");
    let (store, guard) = call(&f);
    let version = super::pending_version(&store);
    let path = PreserveItem {
        relative: "note.md".into(),
        sha256: digest(b"native"),
        len: 6,
    };
    let budget = std::time::Duration::from_secs(10);
    let error = recover_by(
        &store,
        &guard,
        &version,
        Action::Preserve(vec![path]),
        std::time::Instant::now() + budget,
    )
    .err()
    .unwrap();
    assert!(f.dir.path().join(".git/hook-started").exists());
    assert_eq!(error.code, "preserve_blocked");
    assert!(
        !error.message.contains("Nothing was committed"),
        "{}",
        error.message
    );
    assert_eq!(
        open_intents(&store).len(),
        1,
        "the unknown intent is retained for reconciliation"
    );
}

/// When part of a named retry cannot land, the report says so and the rest stays pending.
#[test]
fn retry_reports_a_partial_result_when_part_of_the_named_set_cannot_land() {
    use super::recover::{Action, recover};
    let f = GitFixture::new();
    std::fs::write(f.dir.path().join("shared.md"), "base").unwrap();
    f.git(&["add", "--", "shared.md"]);
    f.git(&["commit", "--quiet", "-m", "fixture: shared"]);
    let (first, guard) = call(&f);
    create(&first, "doc.md", b"one");
    defer_now(&first, &guard);
    drop(guard);
    let (second, guard) = call(&f);
    replace(&second, "shared.md", b"mcp", b"base");
    defer_now(&second, &guard);
    std::fs::write(f.dir.path().join("shared.md"), "mcp").unwrap();
    f.git(&["add", "--", "shared.md"]);
    let ids = open_intents(&second);
    let version = super::pending_version(&second);
    let report = recover(&second, &guard, &version, Action::Retry(ids.clone())).unwrap();
    assert_eq!(
        report.receipt.outcome,
        GitOutcome::Deferred,
        "{:?}",
        report.receipt
    );
    assert!(report.receipt.commit.is_some() && report.changed);
    assert!(
        report.lines.iter().any(|l| l.contains("only part")),
        "{:?}",
        report.lines
    );
    assert_eq!(f.git(&["ls-files", "doc.md"]), "doc.md");
    assert_eq!(
        open_intents(&second),
        vec![ids[1].clone()],
        "the foreign-staged intent stays pending"
    );
}

/// Once the replacement is committed, the waiting removal lands on the next settlement.
#[test]
fn a_waiting_removal_lands_after_its_replacement_is_retried() {
    use super::recover::{Action, recover};
    let f = GitFixture::new();
    let (zero, guard) = call(&f);
    create(&zero, "old.md", b"old");
    commit_now(&zero, &guard);
    drop(guard);
    let (held, guard) = call(&f);
    create_op(&held, "new.md", b"new", "compact:two");
    let held_id = settle(&held, &guard, &partial(), production_policy()).pending[0]
        .intent
        .clone();
    drop(guard);
    let (removal, guard) = call(&f);
    let op = OperationId::new("compact:two").unwrap();
    removal
        .remove(
            crate::store::Remove {
                relative: "old.md",
                observed: b"old",
                cap: RECORD_CAP,
                operation: Some(&op),
                attest: Attest::Required,
            },
            &mut Vec::new(),
        )
        .unwrap();
    assert_eq!(
        commit_now(&removal, &guard).reason,
        Some(Reason::RemovalBarrier)
    );
    let version = super::pending_version(&removal);
    recover(&removal, &guard, &version, Action::Retry(vec![held_id])).unwrap();
    assert_eq!(f.git(&["ls-files", "new.md"]), "new.md");
    assert_eq!(
        f.git(&["ls-files", "old.md"]),
        "old.md",
        "retry commits only what it names"
    );
    drop(guard);
    let (next, guard) = call(&f);
    create(&next, "tick.md", b"x");
    let receipt = commit_now(&next, &guard);
    assert_eq!(receipt.outcome, GitOutcome::Committed, "{receipt:?}");
    assert_eq!(
        f.git(&["ls-files", "old.md"]),
        "",
        "the removal landed once its replacement was committed"
    );
}

/// The pinned public surface: preserve returns a locator per file and carries the operation identity.
#[test]
fn the_public_preserve_surface_returns_locators_and_carries_the_operation() {
    use super::{PreservePath, PreserveRequest, preserve};
    let f = GitFixture::new();
    std::fs::write(f.dir.path().join("note.md"), b"native").unwrap();
    let (store, guard) = call(&f);
    let op = OperationId::new("preserve:one").unwrap();
    let sha = digest(b"native");
    let paths = [PreservePath {
        relative: "note.md",
        sha256: &sha,
        len: 6,
    }];
    let locators = preserve(
        &store,
        &guard,
        PreserveRequest {
            paths: &paths,
            operation: Some(&op),
        },
    )
    .unwrap();
    assert_eq!(locators.len(), 1);
    assert_eq!(locators[0].relative, "note.md");
    assert_eq!(
        super::locator::read_committed(&store, &locators[0].encode().unwrap(), 1024).unwrap(),
        b"native"
    );
    assert!(
        f.git(&["log", "-1", "--format=%B"])
            .contains("preserve:one"),
        "operation identity reaches the trailers"
    );
    let stale = [PreservePath {
        relative: "note.md",
        sha256: &digest(b"other"),
        len: 5,
    }];
    assert_eq!(
        preserve(
            &store,
            &guard,
            PreserveRequest {
                paths: &stale,
                operation: None
            }
        )
        .err()
        .unwrap()
        .code,
        "stale"
    );
    drop(guard);
    let other = Store::from_root(f.dir.path()).unwrap();
    let foreign = store.lock(true, &mut Vec::new());
    drop(foreign);
    let wrong_guard = other.lock(true, &mut Vec::new()).unwrap().unwrap();
    assert_eq!(
        preserve(
            &store,
            &wrong_guard,
            PreserveRequest {
                paths: &paths,
                operation: None
            }
        )
        .err()
        .unwrap()
        .code,
        "not_locked"
    );
}

/// One Git command has ten seconds even when the settlement allows thirty: a slower hook is unknown.
#[cfg(unix)]
#[test]
fn a_hook_slower_than_one_command_is_unknown() {
    let f = GitFixture::new();
    hook(&f, "pre-commit", "touch .git/hook-started; sleep 15");
    let (store, guard) = call(&f);
    create(&store, "doc.md", b"mine");
    let started = std::time::Instant::now();
    let receipt = commit_now(&store, &guard);
    assert!(f.dir.path().join(".git/hook-started").exists());
    assert!(
        started.elapsed() >= super::git::COMMAND_TIME,
        "the limit applies, not earlier"
    );
    assert!(
        started.elapsed() < std::time::Duration::from_secs(15),
        "the hook was not waited for"
    );
    assert_eq!(receipt.outcome, GitOutcome::Unknown, "{receipt:?}");
}

/// A hook that appends to a nested file is visible in both the receipt data and its rendered lines.
#[cfg(unix)]
#[test]
fn a_hook_append_on_a_nested_path_is_reported_in_the_receipt_lines() {
    let f = GitFixture::new();
    hook(&f, "pre-commit", "echo more >> docs/seven.md");
    let (store, guard) = call(&f);
    store.create_dir("docs", &mut Vec::new()).unwrap();
    create(&store, "docs/seven.md", b"seven");
    let receipt = commit_now(&store, &guard);
    assert_eq!(receipt.outcome, GitOutcome::Committed, "{receipt:?}");
    assert!(
        receipt.attention.contains(&Attention::HookChangedWorktree),
        "{receipt:?}"
    );
    assert!(
        receipt
            .lines()
            .iter()
            .any(|l| l.contains("HookChangedWorktree")),
        "{:?}",
        receipt.lines()
    );
    assert_eq!(f.git(&["show", "HEAD:docs/seven.md"]), "seven");
    assert!(
        f.git(&["status", "--porcelain", "--", "docs/seven.md"])
            .contains("M docs/seven.md"),
        "the hook's edit stays visible"
    );
}

/// An older eligible chain left by a rejecting hook is committed with the next call that shares its counter.
#[cfg(unix)]
#[test]
fn an_older_eligible_chain_lands_with_the_next_call_sharing_its_counter() {
    let f = GitFixture::new();
    let (zero, guard) = call(&f);
    create(&zero, "counter.yaml", b"c0");
    commit_now(&zero, &guard);
    drop(guard);
    hook(&f, "pre-commit", "exit 1");
    let (older, guard) = call(&f);
    replace(&older, "counter.yaml", b"c1", b"c0");
    create(&older, "K-1.yaml", b"first");
    assert_eq!(
        commit_now(&older, &guard).reason,
        Some(Reason::CommitNotCompleted)
    );
    drop(guard);
    std::fs::remove_file(f.dir.path().join(".git/hooks/pre-commit")).unwrap();
    let (current, guard) = call(&f);
    replace(&current, "counter.yaml", b"c2", b"c1");
    create(&current, "K-2.yaml", b"second");
    let receipt = commit_now(&current, &guard);
    assert_eq!(receipt.outcome, GitOutcome::Committed, "{receipt:?}");
    assert_eq!(
        receipt.earlier.len(),
        1,
        "the older chain is reported as an extra fact"
    );
    assert_eq!(
        f.git(&["ls-files", "K-1.yaml", "K-2.yaml"]),
        "K-1.yaml\nK-2.yaml"
    );
    assert_eq!(f.git(&["show", "HEAD:counter.yaml"]), "c2");
    assert!(open_intents(&current).is_empty());
}

/// A journal full of open intents refuses preserve; verified committed ones are pruned for it.
#[test]
fn preserve_refuses_a_full_journal_of_open_intents_but_prunes_committed_ones() {
    use super::{PreservePath, PreserveRequest, preserve};
    let f = GitFixture::new();
    let (store, guard) = call(&f);
    create(&store, "doc.md", b"mine");
    defer_now(&store, &guard);
    let (mut j, observed) = journal::load(&store).unwrap();
    let template = j.intents[0].clone();
    for n in 0..63 {
        let mut copy = template.clone();
        copy.id = format!("PG-{n:024x}");
        copy.entries.clear();
        j.intents.push(copy);
    }
    journal::save(&store, &j, observed.as_deref()).unwrap();
    std::fs::write(f.dir.path().join("note.md"), b"native").unwrap();
    let sha = digest(b"native");
    let paths = [PreservePath {
        relative: "note.md",
        sha256: &sha,
        len: 6,
    }];
    let refused = preserve(
        &store,
        &guard,
        PreserveRequest {
            paths: &paths,
            operation: None,
        },
    )
    .err()
    .unwrap();
    assert_eq!(refused.code, "preserve_blocked");
    assert!(refused.message.contains("full"), "{}", refused.message);
    assert_eq!(
        journal::load(&store).unwrap().0.intents.len(),
        64,
        "no open intent was dropped"
    );
    // Mark one verified committed: its room can be reused.
    let (mut j, observed) = journal::load(&store).unwrap();
    let head = f.git(&["rev-parse", "HEAD"]);
    j.intents[5].committed = Some(head);
    journal::save(&store, &j, observed.as_deref()).unwrap();
    let locators = preserve(
        &store,
        &guard,
        PreserveRequest {
            paths: &paths,
            operation: None,
        },
    )
    .unwrap();
    assert_eq!(locators.len(), 1);
}

/// An explicit Release of open intents makes journal room; it is a different path from pruning verified
/// committed intents, and the released work's files are left untouched.
#[test]
fn release_makes_room_then_preserve() {
    use super::recover::{Action, recover};
    use super::{PreservePath, PreserveRequest, preserve};
    let f = GitFixture::new();
    let (store, guard) = call(&f);
    create(&store, "doc.md", b"mine");
    defer_now(&store, &guard);
    let (mut j, observed) = journal::load(&store).unwrap();
    let template = j.intents[0].clone();
    for n in 0..63 {
        let mut copy = template.clone();
        copy.id = format!("PG-{n:024x}");
        copy.entries.clear();
        j.intents.push(copy);
    }
    journal::save(&store, &j, observed.as_deref()).unwrap();
    std::fs::write(f.dir.path().join("note.md"), b"native").unwrap();
    let sha = digest(b"native");
    let paths = [PreservePath {
        relative: "note.md",
        sha256: &sha,
        len: 6,
    }];
    let request = PreserveRequest {
        paths: &paths,
        operation: None,
    };
    assert_eq!(
        preserve(&store, &guard, request).err().unwrap().code,
        "preserve_blocked"
    );
    let version = super::pending_version(&store);
    let released = recover(
        &store,
        &guard,
        &version,
        Action::Release(vec![format!("PG-{:024x}", 3)]),
    )
    .unwrap();
    assert!(released.changed);
    assert!(
        f.dir.path().join("doc.md").exists(),
        "release leaves files untouched"
    );
    assert_eq!(preserve(&store, &guard, request).unwrap().len(), 1);
    assert_eq!(
        open_intents(&store).len(),
        63,
        "only the released intent was dropped"
    );
}
