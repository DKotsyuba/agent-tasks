//! Real-process qualification of automatic Git persistence under the production policy (M-006 criterion 4).
//!
//! Matrix rows G1 to G16 of docs/contracts/knowledge-qualification.md. The policy commits after every successful
//! actual change of a work, knowledge, document or compaction record and never for reads, true no-ops, refused or
//! partial calls. Every scenario starts the shipped binary against a disposable independent repository. Faults
//! come only from ordinary levers the test controls: repository hooks, repository configuration, natively written
//! files and permissions. A crash inside a handler is not reachable this way and stays with M-004's in-crate fault
//! points; a hook kill models only a crash during settlement, after the handler returned. No product switch exists.
//! No test is ignored: every scenario runs on the combined 14-tool candidate and an unmet contract fails honestly.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "Explicit protocol assertions"
)]
mod support;

use serde_json::json;
use std::time::Duration;
use support::{
    Project, commit_count, commit_message, commit_paths, document, git, git_status, head,
    install_hook, knowledge, pending, remove_hook, save, set_writable, staged, target, tree,
    try_call, work_cycle,
};

/// The synthetic work lifecycle runs end to end through the real tools; every step is a saved success.
#[tokio::test]
async fn work_cycle_steps_are_saved_successes() {
    let project = Project::register().await;
    let steps = work_cycle(&project).await;
    assert_eq!(steps.len(), 11);
    for step in &steps {
        assert!(
            step.reply.starts_with("SAVED"),
            "{}: {}",
            step.name,
            step.reply
        );
    }
}

/// G1 (work): every successful plan, record and review mutation yields exactly one commit of eligible files only.
#[tokio::test]
async fn g1_work_mutations_commit_once_each() {
    let project = Project::register().await;
    for step in work_cycle(&project).await {
        assert_eq!(step.commits, 1, "{}: {}", step.name, step.reply);
    }
    let paths = commit_paths(&project.root, &head(&project.root));
    assert!(
        paths.contains(&"modules/M-001.yaml".to_owned()),
        "{paths:?}"
    );
    assert!(
        paths.iter().all(|p| !p.starts_with(".agent-tasks/backups")
            && !p.ends_with(".lock")
            && !p.contains(".tmp-")),
        "administrative files are never committed: {paths:?}"
    );
    assert!(staged(&project.root).is_empty());
}

/// G1 (knowledge and documents): each family call commits only its own paths with intent trailers.
#[tokio::test]
async fn g1_knowledge_and_document_mutations_commit_exactly_their_paths() {
    let project = Project::register().await;
    let before = commit_count(&project.root);
    let created = knowledge(
        &project,
        json!({"op":"create_decision","title":"T","question":"q","decision":"d","rationale":"r"}),
        false,
    )
    .await;
    assert!(created.contains("Git: committed "), "{created}");
    assert_eq!(commit_count(&project.root), before + 1);
    assert!(
        commit_message(&project.root, &head(&project.root)).contains("Agent-Tasks-Intent: PG-")
    );
    save(&project, "docs/a.md", "# A\n", false).await;
    assert_eq!(commit_count(&project.root), before + 2);
    let paths = commit_paths(&project.root, &head(&project.root));
    assert!(
        paths.contains(&"docs/a.md".to_owned())
            && paths.iter().any(|p| p.starts_with("documents/DOC-")),
        "{paths:?}"
    );
    document(
        &project,
        json!({"op":"replace_section","ref":"docs/a.md","section":{"heading":"A"},"body":"text\n"}),
        false,
    )
    .await;
    let adopted = document(&project, json!({"op":"adopt","ref":"docs/a.md"}), true).await;
    assert!(
        adopted.contains("Only an unmanaged or drifted document can be adopted"),
        "{adopted}"
    );
    assert_eq!(
        commit_count(&project.root),
        before + 3,
        "adopt of an already managed document changes nothing and commits nothing"
    );
}

/// G2: nothing that changed nothing commits: reads, unchanged edits, repeated adopt, a registration repeat, refusals.
#[tokio::test]
async fn g2_no_commit_when_nothing_changed() {
    let project = Project::register().await;
    let id = target(&knowledge(&project, json!({"op":"create_decision","title":"T","question":"q","decision":"d","rationale":"r"}), false).await);
    let base = (commit_count(&project.root), head(&project.root));
    let index = tree(&project.root.join(".git").join("refs"));
    project.call("get_context", json!({}), false).await;
    project.call("project_status", json!({}), false).await;
    project
        .call("search", json!({"query":"decision"}), false)
        .await;
    knowledge(
        &project,
        json!({"op":"edit_decision","ref":id,"rationale":"r"}),
        false,
    )
    .await;
    knowledge(
        &project,
        json!({"op":"edit_decision","ref":id,"unknown_field":1}),
        true,
    )
    .await;
    knowledge(
        &project,
        json!({"op":"edit_decision","ref":id,"rationale":"x","version":"0".repeat(64)}),
        true,
    )
    .await;
    let again = support::call(
        &project.client,
        "register_project",
        json!({"project":"product","doc_dir":project.root,"name":"Qualification product","description":"Qualify knowledge, documents and Git persistence"}),
        false,
    )
    .await;
    assert!(again.contains("UNCHANGED"), "{again}");
    assert_eq!(base, (commit_count(&project.root), head(&project.root)));
    assert_eq!(index, tree(&project.root.join(".git").join("refs")));
    assert!(staged(&project.root).is_empty());
}

/// G3 and G13: superseding a held allocator effect keeps it held, and Retry skips it without a duplicate commit.
#[tokio::test]
async fn g3_g13_partial_allocator_supersession_is_not_adopted() {
    let project = Project::register().await;
    std::fs::create_dir_all(project.root.join("decisions")).unwrap();
    set_writable(&project.root.join("decisions"), false);
    let base = commit_count(&project.root);
    let failed = knowledge(&project, json!({"op":"create_decision","title":"Held","question":"q","decision":"d","rationale":"r"}), true).await;
    set_writable(&project.root.join("decisions"), true);
    assert!(
        failed.contains("Git: saved and pending") && failed.contains("IncompleteOutcome"),
        "the create's counter publication is Deferred(IncompleteOutcome) after its record write fails: {failed}"
    );
    assert_eq!(
        commit_count(&project.root),
        base,
        "a partial call is not a commit trigger"
    );
    let held = pending(&project).await;
    let held_intent = held
        .intents
        .first()
        .expect("the held intent is visible")
        .clone();
    let later = knowledge(&project, json!({"op":"create_research","title":"Later","question":"q","conclusions":[{"statement":"s","basis":"inferred"}],"applicability":"a"}), false).await;
    assert!(later.contains("Git: committed "), "{later}");
    let paths = commit_paths(&project.root, &head(&project.root));
    assert!(
        paths
            .iter()
            .all(|p| p == ".agent-tasks/knowledge.yaml" || p.starts_with("research/")),
        "only the later call's paths commit: {paths:?}"
    );
    assert!(
        !paths.iter().any(|p| p.starts_with("decisions/")),
        "the incomplete decision body was never published"
    );
    let counter = project.root.join(".agent-tasks/knowledge.yaml");
    let committed_counter = std::fs::read(&counter).unwrap();
    let still = pending(&project).await;
    assert!(
        still.intents.contains(&held_intent),
        "the superseded partial intent remains held"
    );
    let commits = commit_count(&project.root);
    let retried = project.call(
        "git_recovery",
        json!({"op":"retry","intents":[held_intent.clone()],"version":still.version,"actor":"qualification-agent"}),
        true,
    )
    .await;
    assert_eq!(
        commit_count(&project.root),
        commits,
        "Retry of an intent with no live paths adds no commit"
    );
    assert_eq!(
        std::fs::read(counter).unwrap(),
        committed_counter,
        "Retry leaves the committed counter unchanged"
    );
    assert!(
        retried.to_lowercase().contains("skip")
            && retried.to_lowercase().contains("supersed")
            && retried.contains(".agent-tasks/knowledge.yaml"),
        "Retry reports the superseded counter path: {retried}"
    );
    assert!(
        !commit_message(&project.root, &head(&project.root)).contains(&held_intent),
        "the later commit does not label the held intent complete"
    );
}

/// G3: a later independent success leaves a live partial document held until explicit Retry commits its saved body.
#[tokio::test]
async fn g3_later_success_does_not_commit_live_partial_document() {
    let project = Project::register().await;
    std::fs::create_dir_all(project.root.join("documents")).unwrap();
    set_writable(&project.root.join("documents"), false);
    let base = commit_count(&project.root);
    let partial = save(&project, "docs/held.md", "# Held\n", true).await;
    set_writable(&project.root.join("documents"), true);
    assert!(
        partial.contains("Git: saved and pending") && partial.contains("IncompleteOutcome"),
        "the partial document save is Deferred(IncompleteOutcome): {partial}"
    );
    assert_eq!(
        std::fs::read(project.root.join("docs/held.md")).unwrap(),
        b"# Held\n"
    );
    assert_eq!(
        commit_count(&project.root),
        base,
        "the partial save never commits automatically"
    );
    let held = pending(&project).await;
    let held_intent = held
        .intents
        .first()
        .expect("the partial document intent is pending")
        .clone();
    let later = save(&project, "docs/later.md", "# Later\n", false).await;
    assert!(later.contains("Git: committed "), "{later}");
    assert_eq!(
        commit_count(&project.root),
        base + 1,
        "only the independent later save commits"
    );
    let paths = commit_paths(&project.root, &head(&project.root));
    assert!(paths.contains(&"docs/later.md".to_owned()), "{paths:?}");
    assert!(
        !paths.contains(&"docs/held.md".to_owned()),
        "the later success does not adopt Held: {paths:?}"
    );
    let still = pending(&project).await;
    assert!(
        still.intents.contains(&held_intent),
        "the partial document intent remains Held"
    );
    let retried = project
        .call("git_recovery", json!({"op":"retry","intents":[held_intent],"version":still.version,"actor":"qualification-agent"}), false)
        .await;
    assert!(retried.contains("Git recovery"), "{retried}");
    assert_eq!(
        commit_count(&project.root),
        base + 2,
        "explicit Retry commits the still-live partial effect"
    );
    let recovered_paths = commit_paths(&project.root, &head(&project.root));
    assert!(
        recovered_paths.contains(&"docs/held.md".to_owned()),
        "{recovered_paths:?}"
    );
    assert!(commit_message(&project.root, &head(&project.root)).contains("Agent-Tasks-Adopted"));
}

/// G4: unrelated staged and dirty files keep their state; a staged file on the call's own path defers it.
#[tokio::test]
async fn g4_foreign_staging_is_preserved() {
    let project = Project::register().await;
    std::fs::write(
        project.root.join("foreign-staged.txt"),
        "staged by a human\n",
    )
    .unwrap();
    git(&project.root, &["add", "--", "foreign-staged.txt"]);
    std::fs::write(project.root.join("foreign-dirty.txt"), "dirty by a human\n").unwrap();
    knowledge(
        &project,
        json!({"op":"create_decision","title":"T","question":"q","decision":"d","rationale":"r"}),
        false,
    )
    .await;
    assert_eq!(
        staged(&project.root),
        vec!["foreign-staged.txt".to_owned()],
        "the foreign staged entry is untouched"
    );
    let paths = commit_paths(&project.root, &head(&project.root));
    assert!(
        !paths.contains(&"foreign-staged.txt".to_owned())
            && !paths.contains(&"foreign-dirty.txt".to_owned()),
        "{paths:?}"
    );
    std::fs::create_dir_all(project.root.join("docs")).unwrap();
    std::fs::write(project.root.join("docs/shared.md"), "# Human draft\n").unwrap();
    git(&project.root, &["add", "--", "docs/shared.md"]);
    let before = commit_count(&project.root);
    let deferred = document(
        &project,
        json!({"op":"save","ref":"docs/shared.md","purpose":"Shared","body":"# MCP save\n"}),
        false,
    )
    .await;
    assert!(
        deferred.contains("Git: saved and pending") && deferred.contains("ForeignStagingOnPath"),
        "{deferred}"
    );
    assert_eq!(
        commit_count(&project.root),
        before,
        "a staged file on the call's path defers the whole intent"
    );
    assert_eq!(
        std::fs::read(project.root.join("docs/shared.md")).unwrap(),
        b"# MCP save\n",
        "the saved bytes remain"
    );
}

/// G5: hook, signing, index, merge and detached-HEAD failures keep saved bytes with a truthful receipt, and the next
/// success after the cause is removed settles earlier deferred successes in one commit with one trailer block each.
#[tokio::test]
async fn g5_git_failures_defer_and_a_later_success_settles_them() {
    let project = Project::register().await;
    let branch = git(&project.root, &["rev-parse", "--abbrev-ref", "HEAD"])
        .trim()
        .to_owned();
    let base_head = head(&project.root);
    install_hook(&project.root, "pre-commit", "echo rejected >&2; exit 1");
    let a = save(&project, "docs/one.md", "# One\n", false).await;
    assert!(a.contains("Git: saved and pending"), "{a}");
    remove_hook(&project.root, "pre-commit");
    install_hook(&project.root, "commit-msg", "exit 1");
    let b = save(&project, "docs/two.md", "# Two\n", false).await;
    assert!(b.contains("Git: saved and pending"), "{b}");
    remove_hook(&project.root, "commit-msg");
    git(&project.root, &["config", "commit.gpgsign", "true"]);
    git(&project.root, &["config", "gpg.format", "ssh"]);
    git(
        &project.root,
        &[
            "config",
            "user.signingkey",
            "/nonexistent/qualification-key.pub",
        ],
    );
    let c = save(&project, "docs/three.md", "# Three\n", false).await;
    assert!(
        c.contains("Git: saved and pending") || c.contains("Git: outcome unknown"),
        "{c}"
    );
    git(&project.root, &["config", "--unset", "commit.gpgsign"]);
    std::fs::write(project.root.join(".git/index.lock"), "").unwrap();
    let d = save(&project, "docs/four.md", "# Four\n", false).await;
    assert!(d.contains("Git: saved and pending"), "{d}");
    std::fs::remove_file(project.root.join(".git/index.lock")).unwrap();
    std::fs::write(
        project.root.join(".git/MERGE_HEAD"),
        format!("{}\n", head(&project.root)),
    )
    .unwrap();
    let e = save(&project, "docs/five.md", "# Five\n", false).await;
    assert!(e.contains("Git: saved and pending"), "{e}");
    std::fs::remove_file(project.root.join(".git/MERGE_HEAD")).unwrap();
    git(&project.root, &["checkout", "--detach"]);
    let f = save(&project, "docs/six.md", "# Six\n", false).await;
    assert!(f.contains("Git: saved and pending"), "{f}");
    git(&project.root, &["checkout", &branch]);
    assert_eq!(
        head(&project.root),
        base_head,
        "nothing committed while Git was blocked"
    );
    for name in ["one", "two", "three", "four", "five", "six"] {
        assert!(
            project.root.join(format!("docs/{name}.md")).is_file(),
            "saved bytes survive: {name}"
        );
    }
    git(&project.root, &["config", "--unset", "gpg.format"]);
    git(&project.root, &["config", "--unset", "user.signingkey"]);
    let settled = save(&project, "docs/seven.md", "# Seven\n", false).await;
    assert!(settled.contains("Git: committed "), "{settled}");
    let message = commit_message(&project.root, &head(&project.root));
    assert_eq!(
        message.matches("Agent-Tasks-Intent: PG-").count(),
        7,
        "one trailer block per settled intent: {message}"
    );
    let hook_edit = "echo changed >> docs/seven.md";
    install_hook(&project.root, "pre-commit", hook_edit);
    let changed = save(&project, "docs/eight.md", "# Eight\n", false).await;
    assert!(
        changed.contains("HookChangedWorktree") || changed.contains("Git: saved and pending"),
        "a hook that edits the tree is reported, never reset: {changed}"
    );
}

/// G6: a timeout is Unknown; pre- and post-commit hooks kill only the exact RMCP-owned server and recovery proves both outcomes.
#[tokio::test]
async fn g6_timeout_and_crash_during_settlement_reconcile_without_duplicates() {
    tokio::time::timeout(Duration::from_secs(180), async {
        let mut project = Project::register().await;
        install_hook(&project.root, "pre-commit", "sleep 15");
        let slow = save(&project, "docs/slow.md", "# Slow\n", false).await;
        assert!(slow.contains("Git: outcome unknown"), "a timed-out commit is Unknown: {slow}");
        remove_hook(&project.root, "pre-commit");
        let bytes = std::fs::read(project.root.join("docs/slow.md")).unwrap();
        let before_pre_kill = commit_count(&project.root);
        let killed_pid = project.server_pid;
        let pre_hook_done = project.temp.path().join("pre-commit-hook-finished");
        install_hook(
            &project.root,
            "pre-commit",
            &format!("kill -9 {killed_pid}; echo done > {}; exit 1", pre_hook_done.display()),
        );
        let version = project.version("docs/killed.md").await;
        let killed = try_call(&project.client, "document_work", json!({"project":"product","op":"save","ref":"docs/killed.md","purpose":"Killed","body":"# Killed\n","version":version,"actor":"qualification-agent"})).await;
        assert!(killed.is_err(), "the exact live server child was killed: {killed:?}");
        assert!(try_call(&project.client, "project_status", json!({})).await.is_err(), "the killed server transport stays closed");
        assert!(pre_hook_done.is_file(), "the pre-commit hook ran through its post-kill marker");
        remove_hook(&project.root, "pre-commit");
        assert_eq!(commit_count(&project.root), before_pre_kill, "the pre-commit hook prevented landing");
        project.restart().await;
        let pre_held = pending(&project).await;
        let pre_intent = pre_held.intents.last().expect("the killed request remains pending").clone();
        let pre_recovered = project
            .call("git_recovery", json!({"op":"reconcile","intents":[pre_intent.clone()],"version":pre_held.version,"actor":"qualification-agent"}), false)
            .await;
        assert!(pre_recovered.contains("Git recovery"), "{pre_recovered}");
        assert_eq!(commit_count(&project.root), before_pre_kill, "pre-commit death reconciles without inventing a commit");
        assert!(pending(&project).await.intents.contains(&pre_intent), "uncommitted bytes remain pending after reconciliation");
        assert_eq!(std::fs::read(project.root.join("docs/killed.md")).unwrap(), b"# Killed\n");
        assert_eq!(bytes, std::fs::read(project.root.join("docs/slow.md")).unwrap(), "no business write was replayed");
        let post_pid = project.server_pid;
        let post_hook_done = project.temp.path().join("post-commit-hook-finished");
        let before_post = commit_count(&project.root);
        install_hook(
            &project.root,
            "post-commit",
            &format!("kill -9 {post_pid}; echo done > {}; exit 1", post_hook_done.display()),
        );
        let version = project.version("docs/lost.md").await;
        let lost = try_call(&project.client, "document_work", json!({"project":"product","op":"save","ref":"docs/lost.md","purpose":"Lost","body":"# Lost\n","version":version,"actor":"qualification-agent"})).await;
        assert!(lost.is_err(), "post-commit kill loses the reply: {lost:?}");
        assert!(try_call(&project.client, "project_status", json!({})).await.is_err(), "the post-commit kill closes the transport");
        assert!(post_hook_done.is_file(), "the post-commit hook ran through its post-kill marker");
        remove_hook(&project.root, "post-commit");
        project.restart().await;
        assert_eq!(commit_count(&project.root), before_post + 1, "the commit landed before the reply was lost");
        let landed_paths = commit_paths(&project.root, &head(&project.root));
        assert!(landed_paths.contains(&"docs/lost.md".to_owned()), "the lost reply's commit is on HEAD: {landed_paths:?}");
        let post_held = pending(&project).await;
        let post_intent = post_held.intents.last().expect("the committing intent awaits reconciliation").clone();
        let post_recovered = project
            .call("git_recovery", json!({"op":"reconcile","intents":[post_intent],"version":post_held.version,"actor":"qualification-agent"}), false)
            .await;
        assert!(post_recovered.contains("Git recovery"), "{post_recovered}");
        assert_eq!(commit_count(&project.root), before_post + 1, "reconcile does not duplicate the landed commit");
    })
    .await
    .expect("Bounded timeout and crash scenario");
}

/// G7: native drift of a pending published file is excluded and reported; equal bytes are never ownership.
#[tokio::test]
async fn g7_native_drift_is_excluded_and_equal_bytes_are_not_ownership() {
    let project = Project::register().await;
    install_hook(&project.root, "pre-commit", "exit 1");
    save(&project, "docs/drift.md", "# Drift\n", false).await;
    remove_hook(&project.root, "pre-commit");
    std::fs::write(project.root.join("docs/drift.md"), "# Drift\nnative edit\n").unwrap();
    std::fs::write(
        project.root.join("docs/native-lookalike.md"),
        "# Look-alike\n",
    )
    .unwrap();
    let settled = save(&project, "docs/after.md", "# After\n", false).await;
    let paths = commit_paths(&project.root, &head(&project.root));
    assert!(
        !paths.contains(&"docs/drift.md".to_owned())
            && !paths.contains(&"docs/native-lookalike.md".to_owned()),
        "{paths:?} / {settled}"
    );
    let context = project.call("get_context", json!({}), false).await;
    assert!(
        context.contains("drift") || context.contains("Drift"),
        "{context}"
    );
}

/// Fill the pending journal with 64 confirmed deferred knowledge creates, then make one untracked overflow save.
///
/// Returns the overflow reply, its saved bytes, and the baseline commit count. Asserts the 64 successful replies, the
/// public pending count, and no commits both before overflow and after it; callers can verify the same baseline before
/// Release or Preserve.
async fn full_journal(project: &Project) -> (String, Vec<u8>, usize) {
    let baseline = commit_count(&project.root);
    install_hook(&project.root, "pre-commit", "exit 1");
    for n in 0..64 {
        let reply = knowledge(project, json!({"op":"create_decision","title":format!("D {n}"),"question":"q","decision":"d","rationale":"r"}), false).await;
        assert!(
            reply.starts_with("SAVED") && reply.contains("Git: saved and pending"),
            "intent {n} was not a deferred success: {reply}"
        );
    }
    assert_eq!(
        commit_count(&project.root),
        baseline,
        "the 64 deferred creates commit nothing"
    );
    let context = project.call("get_context", json!({}), false).await;
    assert!(
        context
            .lines()
            .any(|line| line.starts_with("Git persistence: 64 pending intent(s),")),
        "the journal contains 64 open intents: {context}"
    );
    let overflow = knowledge(project, json!({"op":"create_decision","title":"Overflow","question":"q","decision":"d","rationale":"r"}), false).await;
    assert!(
        overflow.starts_with("SAVED") && overflow.to_lowercase().contains("untracked"),
        "overflow remains saved and untracked: {overflow}"
    );
    remove_hook(&project.root, "pre-commit");
    let bytes = std::fs::read(project.root.join("decisions/D-065.yaml")).unwrap();
    assert_eq!(
        commit_count(&project.root),
        baseline,
        "neither the overflow nor the pending intents commit"
    );
    (overflow, bytes, baseline)
}

/// G8: with every commit rejected, 64 deferred successes fill the journal; the next write is saved intact, reported
/// untracked and never auto-committed. While the journal stays full, `git_recovery` preserve is refused as
/// `preserve_blocked` (proof unavailable), commits nothing, and discards no held row.
#[tokio::test]
async fn g8_journal_capacity_never_blocks_an_ordinary_save() {
    tokio::time::timeout(Duration::from_secs(300), async {
        let project = Project::register().await;
        let (overflow, bytes, baseline) = full_journal(&project).await;
        assert!(overflow.starts_with("SAVED"), "a full journal never refuses an ordinary save: {overflow}");
        let after = knowledge(&project, json!({"op":"create_decision","title":"After","question":"q","decision":"d","rationale":"r"}), false).await;
        assert_eq!(commit_count(&project.root), baseline, "no commits occur before explicit recovery");
        let paths = commit_paths(&project.root, &head(&project.root));
        assert!(!paths.iter().any(|p| p == "decisions/D-065.yaml"), "the untracked file is never auto-committed: {paths:?} {after}");
        assert_eq!(bytes, std::fs::read(project.root.join("decisions/D-065.yaml")).unwrap());
        let held = pending(&project).await;
        let commits = commit_count(&project.root);
        let blocked = project
            .call("git_recovery", json!({"op":"preserve","paths":[{"relative":"decisions/D-065.yaml","sha256":support::sha256_hex(&bytes),"len":bytes.len()}],"version":held.version,"actor":"qualification-agent"}), true)
            .await;
        assert!(blocked.contains("preserve_blocked"), "a full journal blocks preserve explicitly: {blocked}");
        assert_eq!(commits, commit_count(&project.root), "a blocked preserve commits nothing");
        assert_eq!(
            pending(&project).await.version,
            held.version,
            "a blocked preserve discards no held row"
        );
    })
    .await
    .expect("Bounded journal capacity scenario");
}

/// G8 (recovery tail): explicit Release of held rows makes room, after which preserve commits exactly the authorized
/// bytes of the untracked file. Needs the public pending identities from project context.
#[tokio::test]
async fn g8b_release_makes_room_then_preserve_commits_the_untracked_file() {
    tokio::time::timeout(Duration::from_secs(300), async {
        let project = Project::register().await;
        let (_, bytes, baseline) = full_journal(&project).await;
        assert_eq!(commit_count(&project.root), baseline, "no commits occur before Release");
        let held = pending(&project).await;
        assert!(
            held.intents.len() >= 16,
            "context must list at least sixteen pending identities, found {}",
            held.intents.len()
        );
        let released = project
            .call("git_recovery", json!({"op":"release","intents":held.intents[..16],"version":held.version,"actor":"qualification-agent"}), false)
            .await;
        assert!(released.contains("Git recovery"), "{released}");
        let now = pending(&project).await;
        project
            .call("git_recovery", json!({"op":"preserve","paths":[{"relative":"decisions/D-065.yaml","sha256":support::sha256_hex(&bytes),"len":bytes.len()}],"version":now.version,"actor":"qualification-agent"}), false)
            .await;
        assert!(commit_paths(&project.root, &head(&project.root)).contains(&"decisions/D-065.yaml".to_owned()));
        assert_eq!(bytes, std::fs::read(project.root.join("decisions/D-065.yaml")).unwrap());
    })
    .await
    .expect("Bounded release scenario");
}

/// G9 and G14: recovery needs the exact pending version, never replays business work, and shows its label as display
/// text only, routing its pending view through the Project route with `ref` omitted.
#[tokio::test]
async fn g9_g14_recovery_version_and_display_label() {
    let project = Project::register().await;
    install_hook(&project.root, "pre-commit", "exit 1");
    save(&project, "docs/held.md", "# Held\n", false).await;
    remove_hook(&project.root, "pre-commit");
    let held = pending(&project).await;
    let stale = project
        .call("git_recovery", json!({"op":"reconcile","intents":held.intents,"version":"0".repeat(64),"actor":"qualification-agent"}), true)
        .await;
    assert!(stale.contains("stale"), "{stale}");
    let before = std::fs::read(project.root.join("docs/held.md")).unwrap();
    let ok = project
        .call("git_recovery", json!({"op":"reconcile","intents":held.intents,"version":held.version,"actor":"qualification-agent"}), false)
        .await;
    assert!(ok.contains("Git recovery"), "{ok}");
    assert!(
        !ok.contains("ref=Git recovery") && !ok.contains("ref=Project"),
        "the label is never a reference: {ok}"
    );
    assert!(
        ok.contains("get_context"),
        "the next route is the Project route: {ok}"
    );
    assert_eq!(
        before,
        std::fs::read(project.root.join("docs/held.md")).unwrap()
    );
}

/// G12: two successful writes of one document deferred by a failing hook commit as one chain with true digests.
#[tokio::test]
async fn g12_deferred_chain_commits_the_latest_image_with_true_digests() {
    let project = Project::register().await;
    install_hook(&project.root, "pre-commit", "exit 1");
    save(&project, "docs/chain.md", "# Chain\nfirst\n", false).await;
    save(&project, "docs/chain.md", "# Chain\nsecond\n", false).await;
    remove_hook(&project.root, "pre-commit");
    knowledge(&project, json!({"op":"create_decision","title":"Settle","question":"q","decision":"d","rationale":"r"}), false).await;
    let message = commit_message(&project.root, &head(&project.root));
    let effects: Vec<&str> = message
        .lines()
        .filter(|l| l.starts_with("Agent-Tasks-Effect:") && l.contains("docs/chain.md"))
        .collect();
    assert!(
        effects.len() >= 2,
        "each earlier intent keeps its own effect line: {message}"
    );
    assert!(
        effects.iter().any(|l| l.contains("into=")),
        "the superseded effect names the committed digest: {message}"
    );
    assert_eq!(
        git(
            &project.root,
            &["show", &format!("{}:docs/chain.md", head(&project.root))]
        ),
        "# Chain\nsecond\n"
    );
}

/// G15 (independent): a current call deferred by foreign staging on its own path does not hold back an older eligible
/// intent on disjoint paths (a managed-document edit, no allocator path); the receipt keeps both facts apart.
#[tokio::test]
async fn g15_independent_older_edit_lands_while_current_defers() {
    let project = Project::register().await;
    save(&project, "docs/older.md", "# Older\ntext\n", false).await;
    install_hook(&project.root, "pre-commit", "exit 1");
    let held = document(
        &project,
        json!({"op":"replace_section","ref":"docs/older.md","section":{"heading":"Older"},"body":"edited\n"}),
        false,
    )
    .await;
    assert!(held.contains("Git: saved and pending"), "{held}");
    remove_hook(&project.root, "pre-commit");
    std::fs::write(project.root.join("docs/current.md"), "# Human\n").unwrap();
    git(&project.root, &["add", "--", "docs/current.md"]);
    let before = commit_count(&project.root);
    let reply = document(
        &project,
        json!({"op":"save","ref":"docs/current.md","purpose":"Current","body":"# Current\n"}),
        false,
    )
    .await;
    assert!(
        reply.contains("Git: saved and pending") && reply.contains("ForeignStagingOnPath"),
        "{reply}"
    );
    assert!(
        reply.contains("Also committed"),
        "the earlier commit is its own line: {reply}"
    );
    assert!(
        !reply.contains("Git: committed "),
        "the current call is not reported committed: {reply}"
    );
    assert_eq!(
        commit_count(&project.root),
        before + 1,
        "the older independent intent committed in this settlement"
    );
    let paths = commit_paths(&project.root, &head(&project.root));
    assert!(
        paths.contains(&"docs/older.md".to_owned())
            && paths
                .iter()
                .all(|p| p == "docs/older.md" || p.starts_with("documents/DOC-")),
        "only the older call's own paths: {paths:?}"
    );
    assert_eq!(
        git(&project.root, &["show", "HEAD:docs/older.md"]),
        "# Older\nedited\n"
    );
    assert_eq!(
        std::fs::read_to_string(project.root.join("docs/current.md")).unwrap(),
        "# Current\n",
        "the current call's saved bytes stay on disk, uncommitted"
    );
}

/// G15 (coupled counter): an older and a current knowledge create share `.agent-tasks/knowledge.yaml`; when the
/// current one defers, the shared counter chain is never split, so the older one defers with it and nothing commits.
#[tokio::test]
async fn g15_coupled_counter_chain_defers_together() {
    let project = Project::register().await;
    install_hook(&project.root, "pre-commit", "exit 1");
    let older = knowledge(&project, json!({"op":"create_decision","title":"Older","question":"q","decision":"d","rationale":"r"}), false).await;
    assert!(older.contains("Git: saved and pending"), "{older}");
    remove_hook(&project.root, "pre-commit");
    let counter = project.root.join(".agent-tasks/knowledge.yaml");
    let after_older = std::fs::read(&counter).unwrap();
    git(&project.root, &["add", "--", ".agent-tasks/knowledge.yaml"]);
    let before = commit_count(&project.root);
    let reply = knowledge(&project, json!({"op":"create_decision","title":"Current","question":"q","decision":"d","rationale":"r"}), false).await;
    assert!(
        reply.contains("Git: saved and pending") && reply.contains("ForeignStagingOnPath"),
        "{reply}"
    );
    assert!(
        reply.contains("Pending: 2 intent(s)"),
        "both coupled intents stay pending: {reply}"
    );
    assert_eq!(
        commit_count(&project.root),
        before,
        "the shared allocator chain and both records remain deferred"
    );
    assert!(!reply.contains("Also committed"), "{reply}");
    assert_ne!(
        std::fs::read(&counter).unwrap(),
        after_older,
        "both creates updated the same allocator path"
    );
    assert_eq!(
        staged(&project.root),
        vec![".agent-tasks/knowledge.yaml".to_owned()],
        "the shared counter's human staging is untouched"
    );
    assert_eq!(
        git(&project.root, &["show", ":.agent-tasks/knowledge.yaml"]).as_bytes(),
        after_older,
        "the staged counter image remains unchanged"
    );
    assert!(project.root.join("decisions/D-001.yaml").is_file());
    assert!(project.root.join("decisions/D-002.yaml").is_file());
    assert!(counter.is_file());
}

/// G16: first homes and ignored backups are ordinary eligible commits; a user-ignored business file defers the whole call.
#[tokio::test]
async fn g16_eligible_files_only_and_ignored_business_sibling() {
    let project = Project::register().await;
    knowledge(&project, json!({"op":"create_decision","title":"First home","question":"q","decision":"d","rationale":"r"}), false).await;
    let paths = commit_paths(&project.root, &head(&project.root));
    assert!(
        paths
            .iter()
            .all(|p| p == ".agent-tasks/knowledge.yaml" || p == "decisions/D-001.yaml"),
        "no directory, lock or backup is committed: {paths:?}"
    );
    let ignore = std::fs::read_to_string(project.root.join(".gitignore")).unwrap_or_default();
    std::fs::write(
        project.root.join(".gitignore"),
        format!("{ignore}research/\n"),
    )
    .unwrap();
    git(&project.root, &["add", "--", ".gitignore"]);
    let (ok, _, err) = git_status(
        &project.root,
        &["commit", "-q", "-m", "user: ignore research"],
    );
    assert!(ok, "{err}");
    let before = commit_count(&project.root);
    let reply = knowledge(&project, json!({"op":"create_research","title":"Ignored","question":"q","conclusions":[{"statement":"s","basis":"inferred"}],"applicability":"a"}), false).await;
    assert!(
        reply.contains("Git: saved and pending") && reply.contains("IgnoredBusinessSibling"),
        "{reply}"
    );
    assert_eq!(
        commit_count(&project.root),
        before,
        "the counter is not committed alone"
    );
    assert!(
        project.root.join("research/RS-001.yaml").is_file(),
        "saved bytes remain"
    );
}
