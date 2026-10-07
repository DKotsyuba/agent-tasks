//! Real-process qualification of automatic Git persistence under the production policy (M-006 criterion 4).
//!
//! Matrix rows G1 to G16 of docs/contracts/knowledge-qualification.md. The policy commits after every successful
//! actual change of a work, knowledge, document or compaction record and never for reads, true no-ops, refused or
//! partial calls. Every scenario starts the shipped binary against a disposable independent repository. Faults
//! come only from ordinary levers the test controls: repository hooks, repository configuration, natively written
//! files and permissions. A crash inside a handler is not reachable this way and stays with M-004's in-crate fault
//! points; a hook kill models only a crash during settlement, after the handler returned. No product switch exists.
//! Tests that need the combined candidate are `#[ignore]`d with the exact reason; the work-cycle sanity test runs on
//! any candidate.
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
#[ignore = "needs the combined E-001 candidate with production Git settlement for work mutations"]
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
#[ignore = "needs the combined E-001 candidate with production Git settlement"]
async fn g1_knowledge_and_document_mutations_commit_exactly_their_paths() {
    let project = Project::register().await;
    let before = commit_count(&project.root);
    let created = knowledge(
        &project,
        json!({"op":"create_decision","title":"T","question":"q","decision":"d","rationale":"r"}),
        false,
    )
    .await;
    assert!(created.contains("Committed"), "{created}");
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
    document(&project, json!({"op":"adopt","ref":"docs/a.md"}), false).await;
    assert_eq!(
        commit_count(&project.root),
        before + 3,
        "adopt of an already managed document is a no-op"
    );
}

/// G2: nothing that changed nothing commits: reads, unchanged edits, repeated adopt, a registration repeat, refusals.
#[tokio::test]
#[ignore = "needs the combined E-001 candidate with production Git settlement"]
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

/// G3 and G13: a real failure after the first publication is held, a later success never certifies it, and an
/// explicit recovery commits the held bytes marked adopted.
#[tokio::test]
#[ignore = "needs the combined E-001 candidate with production Git settlement and git_recovery"]
async fn g3_g13_partial_call_is_held_and_recovered_only_explicitly() {
    let project = Project::register().await;
    std::fs::create_dir_all(project.root.join("decisions")).unwrap();
    set_writable(&project.root.join("decisions"), false);
    let base = commit_count(&project.root);
    let failed = knowledge(&project, json!({"op":"create_decision","title":"Held","question":"q","decision":"d","rationale":"r"}), true).await;
    set_writable(&project.root.join("decisions"), true);
    assert!(
        failed.contains("Deferred") || failed.contains("partial") || failed.contains("Partial"),
        "{failed}"
    );
    assert_eq!(
        commit_count(&project.root),
        base,
        "a partial call is not a commit trigger"
    );
    let held = pending(&project).await;
    assert!(
        !held.intents.is_empty(),
        "the held intent is visible in context"
    );
    let later = knowledge(&project, json!({"op":"create_research","title":"Later","question":"q","conclusions":[{"statement":"s","basis":"inferred"}],"applicability":"a"}), false).await;
    assert!(later.contains("Committed"), "{later}");
    let paths = commit_paths(&project.root, &head(&project.root));
    assert!(
        paths
            .iter()
            .all(|p| p == ".agent-tasks/knowledge.yaml" || p.starts_with("research/")),
        "only the later call's own paths: {paths:?}"
    );
    let still = pending(&project).await;
    assert!(
        still.intents.iter().any(|i| held.intents.contains(i)),
        "the held intent is not certified by a later success"
    );
    let retried = project
        .call("git_recovery", json!({"op":"retry","intents":held.intents,"version":still.version,"actor":"qualification-agent"}), false)
        .await;
    assert!(retried.contains("Git recovery"), "{retried}");
    assert!(commit_message(&project.root, &head(&project.root)).contains("Agent-Tasks-Adopted"));
}

/// G4: unrelated staged and dirty files keep their state; a staged file on the call's own path defers it.
#[tokio::test]
#[ignore = "needs the combined E-001 candidate with production Git settlement"]
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
        deferred.contains("Deferred") && deferred.contains("ForeignStagingOnPath"),
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
#[ignore = "needs the combined E-001 candidate with production Git settlement"]
async fn g5_git_failures_defer_and_a_later_success_settles_them() {
    let project = Project::register().await;
    let branch = git(&project.root, &["rev-parse", "--abbrev-ref", "HEAD"])
        .trim()
        .to_owned();
    let base_head = head(&project.root);
    install_hook(&project.root, "pre-commit", "echo rejected >&2; exit 1");
    let a = save(&project, "docs/one.md", "# One\n", false).await;
    assert!(a.contains("Deferred"), "{a}");
    remove_hook(&project.root, "pre-commit");
    install_hook(&project.root, "commit-msg", "exit 1");
    let b = save(&project, "docs/two.md", "# Two\n", false).await;
    assert!(b.contains("Deferred"), "{b}");
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
    assert!(c.contains("Deferred") || c.contains("Unknown"), "{c}");
    git(&project.root, &["config", "--unset", "commit.gpgsign"]);
    std::fs::write(project.root.join(".git/index.lock"), "").unwrap();
    let d = save(&project, "docs/four.md", "# Four\n", false).await;
    assert!(d.contains("Deferred"), "{d}");
    std::fs::remove_file(project.root.join(".git/index.lock")).unwrap();
    std::fs::write(
        project.root.join(".git/MERGE_HEAD"),
        format!("{}\n", head(&project.root)),
    )
    .unwrap();
    let e = save(&project, "docs/five.md", "# Five\n", false).await;
    assert!(e.contains("Deferred"), "{e}");
    std::fs::remove_file(project.root.join(".git/MERGE_HEAD")).unwrap();
    git(&project.root, &["checkout", "--detach"]);
    let f = save(&project, "docs/six.md", "# Six\n", false).await;
    assert!(f.contains("Deferred"), "{f}");
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
    assert!(settled.contains("Committed"), "{settled}");
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
        changed.contains("HookChangedWorktree") || changed.contains("Deferred"),
        "a hook that edits the tree is reported, never reset: {changed}"
    );
}

/// G6: a slow hook gives Unknown; a hook that kills the server during settlement leaves a state that restart plus
/// reconcile resolves with no second commit and no replayed business write.
#[tokio::test]
#[ignore = "needs the combined E-001 candidate with production Git settlement and git_recovery"]
async fn g6_timeout_and_crash_during_settlement_reconcile_without_duplicates() {
    tokio::time::timeout(Duration::from_secs(180), async {
        let mut project = Project::register().await;
        install_hook(&project.root, "pre-commit", "sleep 15");
        let slow = save(&project, "docs/slow.md", "# Slow\n", false).await;
        assert!(slow.contains("Unknown") || slow.contains("Deferred"), "{slow}");
        remove_hook(&project.root, "pre-commit");
        let bytes = std::fs::read(project.root.join("docs/slow.md")).unwrap();
        install_hook(&project.root, "pre-commit", "kill -9 $(ps -o ppid= -p $PPID | tr -d ' ')");
        let version = project.version("docs/killed.md").await;
        let killed = try_call(&project.client, "document_work", json!({"project":"product","op":"save","ref":"docs/killed.md","purpose":"Killed","body":"# Killed\n","version":version,"actor":"qualification-agent"})).await;
        assert!(killed.is_err(), "the server was killed during settlement: {killed:?}");
        remove_hook(&project.root, "pre-commit");
        project.restart().await;
        let base = commit_count(&project.root);
        let held = pending(&project).await;
        let recovered = project
            .call("git_recovery", json!({"op":"reconcile","intents":held.intents,"version":held.version,"actor":"qualification-agent"}), false)
            .await;
        assert!(!recovered.is_empty());
        assert_eq!(bytes, std::fs::read(project.root.join("docs/slow.md")).unwrap(), "no replayed business write");
        assert!(commit_count(&project.root) <= base + 1, "reconcile makes at most the one explicit commit");
        // The same crash after the commit landed is a lost reply: restart and reconcile never add a second commit.
        install_hook(&project.root, "post-commit", "kill -9 $(ps -o ppid= -p $PPID | tr -d ' ')");
        let version = project.version("docs/lost.md").await;
        let before = commit_count(&project.root);
        let lost = try_call(&project.client, "document_work", json!({"project":"product","op":"save","ref":"docs/lost.md","purpose":"Lost","body":"# Lost\n","version":version,"actor":"qualification-agent"})).await;
        assert!(lost.is_err());
        remove_hook(&project.root, "post-commit");
        project.restart().await;
        assert_eq!(commit_count(&project.root), before + 1, "the commit landed before the reply was lost");
        let held = pending(&project).await;
        project
            .call("git_recovery", json!({"op":"reconcile","intents":held.intents,"version":held.version,"actor":"qualification-agent"}), false)
            .await;
        assert_eq!(commit_count(&project.root), before + 1, "reconcile never adds a second commit");
    })
    .await
    .expect("Bounded timeout and crash scenario");
}

/// G7: native drift of a pending published file is excluded and reported; equal bytes are never ownership.
#[tokio::test]
#[ignore = "needs the combined E-001 candidate with production Git settlement"]
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

/// G8: with every commit rejected, 64 deferred successes fill the journal; the next write is saved intact, reported
/// untracked and never auto-committed, while `git_recovery` preserve commits exactly the authorized bytes.
#[tokio::test]
#[ignore = "needs the combined E-001 candidate with production Git settlement and git_recovery"]
async fn g8_journal_capacity_never_blocks_an_ordinary_save() {
    tokio::time::timeout(Duration::from_secs(300), async {
        let project = Project::register().await;
        install_hook(&project.root, "pre-commit", "exit 1");
        for n in 0..64 {
            knowledge(&project, json!({"op":"create_decision","title":format!("D {n}"),"question":"q","decision":"d","rationale":"r"}), false).await;
        }
        let overflow = knowledge(&project, json!({"op":"create_decision","title":"Overflow","question":"q","decision":"d","rationale":"r"}), false).await;
        assert!(overflow.starts_with("SAVED"), "a full journal never refuses an ordinary save: {overflow}");
        assert!(overflow.to_lowercase().contains("untracked"), "{overflow}");
        remove_hook(&project.root, "pre-commit");
        let after = knowledge(&project, json!({"op":"create_decision","title":"After","question":"q","decision":"d","rationale":"r"}), false).await;
        let paths = commit_paths(&project.root, &head(&project.root));
        assert!(!paths.iter().any(|p| p == "decisions/D-065.yaml"), "the untracked file is never auto-committed: {paths:?} {after}");
        let bytes = std::fs::read(project.root.join("decisions/D-065.yaml")).unwrap();
        let held = pending(&project).await;
        project
            .call("git_recovery", json!({"op":"preserve","paths":[{"relative":"decisions/D-065.yaml","sha256":support::sha256_hex(&bytes),"len":bytes.len()}],"version":held.version,"actor":"qualification-agent"}), false)
            .await;
        assert!(commit_paths(&project.root, &head(&project.root)).contains(&"decisions/D-065.yaml".to_owned()));
    })
    .await
    .expect("Bounded journal capacity scenario");
}

/// G9 and G14: recovery needs the exact pending version, never replays business work, and shows its label as display
/// text only, routing its pending view through the Project route with `ref` omitted.
#[tokio::test]
#[ignore = "needs the combined E-001 candidate with production Git settlement and git_recovery"]
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
#[ignore = "needs the combined E-001 candidate with production Git settlement"]
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

/// G15: the current call defers while older eligible intents commit in the same settlement; the receipt keeps both facts apart.
#[tokio::test]
#[ignore = "needs the combined E-001 candidate with production Git settlement"]
async fn g15_current_deferred_while_older_intents_commit() {
    let project = Project::register().await;
    install_hook(&project.root, "pre-commit", "exit 1");
    save(&project, "docs/older.md", "# Older\n", false).await;
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
        reply.contains("Deferred") && reply.contains("ForeignStagingOnPath"),
        "{reply}"
    );
    assert_eq!(
        commit_count(&project.root),
        before + 1,
        "the older eligible intent committed in this settlement"
    );
    let paths = commit_paths(&project.root, &head(&project.root));
    assert!(
        paths.contains(&"docs/older.md".to_owned())
            && !paths.contains(&"docs/current.md".to_owned()),
        "{paths:?}"
    );
    assert!(
        !reply.contains("Committed:") || reply.contains("earlier"),
        "the current call is not reported committed: {reply}"
    );
}

/// G16: first homes and ignored backups are ordinary eligible commits; a user-ignored business file defers the whole call.
#[tokio::test]
#[ignore = "needs the combined E-001 candidate with production Git settlement"]
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
        reply.contains("Deferred") && reply.contains("IgnoredBusinessSibling"),
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
