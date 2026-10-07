//! Real-binary qualification of typed knowledge through `knowledge_work` (M-006 criterion 2, `kr-operations` r5).
//!
//! Matrix rows K1 to K10 of docs/contracts/knowledge-qualification.md. Every scenario starts the shipped binary
//! with a scrubbed environment and a disposable independent documentation repository. Faults are produced only
//! with repository hooks and ordinary permissions. The tests are `#[ignore]`d with an explicit reason until the
//! combined candidate carries the producer tools; `cargo test -- --include-ignored` runs them on that candidate.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "Explicit protocol assertions"
)]
mod support;

use serde_json::json;
use support::{
    Project, commit_count, commit_paths, head, install_hook, knowledge, remove_hook, staged,
    target, tree,
};

/// Create a minimal Decision and return its id.
async fn decision(project: &Project, title: &str) -> String {
    let text = knowledge(
        project,
        json!({"op":"create_decision","title":title,"question":"Which option?","decision":"Option A","rationale":"Measured better"}),
        false,
    )
    .await;
    assert!(text.starts_with("SAVED D-"), "{text}");
    target(&text)
}

/// Create a minimal Runbook with one step and return its id.
async fn runbook(project: &Project, title: &str) -> String {
    let text = knowledge(
        project,
        json!({"op":"create_runbook","title":title,"purpose":"Restore service","steps":[{"title":"Check","description":"Inspect status","expected":"Healthy"}]}),
        false,
    )
    .await;
    assert!(text.starts_with("SAVED RB-"), "{text}");
    target(&text)
}

/// K1: each kind creates with generated metadata and exactly one commit holding the allocator and the record.
#[tokio::test]
#[ignore = "needs the combined E-001 candidate carrying knowledge_work"]
async fn k1_create_each_kind_commits_allocator_and_record_once() {
    let project = Project::register().await;
    let base = commit_count(&project.root);
    let id = decision(&project, "First decision").await;
    assert_eq!(id, "D-001");
    assert_eq!(
        commit_count(&project.root),
        base + 1,
        "one commit per successful create"
    );
    assert_eq!(
        commit_paths(&project.root, &head(&project.root)),
        vec![
            ".agent-tasks/knowledge.yaml".to_owned(),
            "decisions/D-001.yaml".to_owned()
        ],
        "the first create into a new home commits only its eligible files"
    );
    assert!(staged(&project.root).is_empty());
    let run = runbook(&project, "First runbook").await;
    assert_eq!(run, "RB-001");
    let research = knowledge(
        &project,
        json!({"op":"create_research","title":"Latency","question":"How slow?","conclusions":[{"statement":"About 5 ms","basis":"measured"}],"evidence":[{"claim":"p50 5 ms","source":"bench run","basis":"measured"}],"applicability":"This host only"}),
        false,
    )
    .await;
    assert_eq!(target(&research), "RS-001");
    let checklist = knowledge(
        &project,
        json!({"op":"create_checklist","title":"Release","purpose":"Ship it","items":["Build","Test"]}),
        false,
    )
    .await;
    assert_eq!(target(&checklist), "CL-001");
    for (id, home) in [
        ("D-001", "decisions"),
        ("RB-001", "runbooks"),
        ("RS-001", "research"),
        ("CL-001", "checklists"),
    ] {
        assert!(
            project.root.join(home).join(format!("{id}.yaml")).is_file(),
            "{id}"
        );
    }
    let unknown = knowledge(
        &project,
        json!({"op":"create_decision","title":"x","question":"q","decision":"d","rationale":"r","revision":9}),
        true,
    )
    .await;
    assert!(
        unknown.contains("revision"),
        "a caller supplied generated field is named: {unknown}"
    );
}

/// K2: an edit bumps the revision and keeps the previous content; an unchanged edit is a no-op without a commit.
#[tokio::test]
#[ignore = "needs the combined E-001 candidate carrying knowledge_work"]
async fn k2_edit_bumps_revision_and_unchanged_edit_is_a_no_op() {
    let project = Project::register().await;
    let id = decision(&project, "Editable").await;
    let edited = knowledge(
        &project,
        json!({"op":"edit_decision","ref":id,"rationale":"Measured better twice"}),
        false,
    )
    .await;
    assert!(edited.starts_with("SAVED D-001"), "{edited}");
    let history = project
        .call("get_context", json!({"ref":id,"view":"history"}), false)
        .await;
    assert!(
        history.contains("Measured better\n") || history.contains("Measured better"),
        "revision 1 content is retained: {history}"
    );
    let commits = commit_count(&project.root);
    let again = knowledge(
        &project,
        json!({"op":"edit_decision","ref":id,"rationale":"Measured better twice"}),
        false,
    )
    .await;
    assert!(
        again.starts_with("UNCHANGED") || again.contains("UNCHANGED"),
        "{again}"
    );
    assert_eq!(
        commit_count(&project.root),
        commits,
        "an unchanged edit must not commit"
    );
}

/// K3: supersession links one same-kind current successor and refuses every unsafe shape.
#[tokio::test]
#[ignore = "needs the combined E-001 candidate carrying knowledge_work"]
async fn k3_supersession_links_and_refuses_unsafe_shapes() {
    let project = Project::register().await;
    let old = decision(&project, "Old").await;
    let new = decision(&project, "New").await;
    let other = decision(&project, "Other").await;
    let rb = runbook(&project, "Different kind").await;
    knowledge(
        &project,
        json!({"op":"supersede","ref":old,"successor":new}),
        true,
    )
    .await;
    let linked = knowledge(
        &project,
        json!({"op":"supersede","ref":old,"successor":new}),
        false,
    )
    .await;
    assert!(
        linked.contains("SAVED") || linked.contains("UNCHANGED"),
        "{linked}"
    );
    let context = project.call("get_context", json!({"ref":old}), false).await;
    assert!(context.contains("superseded"), "{context}");
    assert!(context.contains(&new), "{context}");
    knowledge(
        &project,
        json!({"op":"supersede","ref":new,"successor":new}),
        true,
    )
    .await;
    knowledge(
        &project,
        json!({"op":"supersede","ref":other,"successor":rb}),
        true,
    )
    .await;
    knowledge(
        &project,
        json!({"op":"supersede","ref":other,"successor":old}),
        true,
    )
    .await;
    knowledge(
        &project,
        json!({"op":"supersede","ref":old,"successor":other}),
        true,
    )
    .await;
    knowledge(
        &project,
        json!({"op":"edit_decision","ref":old,"rationale":"must refuse"}),
        true,
    )
    .await;
}

/// K4: a Runbook use is bound to the revision used and never verifies a newer revision.
#[tokio::test]
#[ignore = "needs the combined E-001 candidate carrying knowledge_work"]
async fn k4_runbook_use_is_bound_to_its_revision() {
    let project = Project::register().await;
    let id = runbook(&project, "Operate").await;
    knowledge(
        &project,
        json!({"op":"edit_runbook","ref":id,"purpose":"Restore service quickly"}),
        false,
    )
    .await;
    let use_old = knowledge(
        &project,
        json!({"op":"use_runbook","ref":id,"revision":1,"outcome":"succeeded","environment":"disposable fixture","checks":[{"label":"health","status":"passed"}]}),
        false,
    )
    .await;
    assert!(use_old.starts_with("SAVED RB-001"), "{use_old}");
    let history = project
        .call("get_context", json!({"ref":id,"view":"history"}), false)
        .await;
    assert!(
        history.contains("stale_revision") || history.contains("stale revision"),
        "a use of an old revision is flagged: {history}"
    );
    for bad in [
        json!({"op":"use_runbook","ref":id,"revision":99,"outcome":"succeeded","environment":"x","checks":[{"label":"h","status":"passed"}]}),
        json!({"op":"use_runbook","ref":id,"revision":2,"outcome":"succeeded","checks":[{"label":"h","status":"passed"}]}),
        json!({"op":"use_runbook","ref":id,"revision":2,"outcome":"succeeded","environment":"x"}),
        json!({"op":"use_runbook","ref":id,"revision":2,"outcome":"succeeded","environment":"x","checks":[{"label":"h","status":"failed"}]}),
    ] {
        knowledge(&project, bad, true).await;
    }
}

/// K5: checklist items require a completion fact or a cancellation reason, and work Tasks are never copied.
#[tokio::test]
#[ignore = "needs the combined E-001 candidate carrying knowledge_work"]
async fn k5_checklist_facts_reasons_and_no_task_copies() {
    let project = Project::register().await;
    let created = knowledge(&project, json!({"op":"create_checklist","title":"Release","purpose":"Ship","items":["Build","Test"]}), false).await;
    let id = target(&created);
    knowledge(
        &project,
        json!({"op":"create_checklist","title":"Copy","purpose":"x","items":["M-001/T-001"]}),
        true,
    )
    .await;
    knowledge(
        &project,
        json!({"op":"resolve_item","ref":id,"item":"I-001","state":"done"}),
        true,
    )
    .await;
    knowledge(&project, json!({"op":"resolve_item","ref":id,"item":"I-001","state":"done","text":"built in CI run 12"}), false).await;
    knowledge(&project, json!({"op":"complete_checklist","ref":id}), true).await;
    knowledge(&project, json!({"op":"resolve_item","ref":id,"item":"I-002","state":"canceled","text":"not needed this release"}), false).await;
    let done = knowledge(&project, json!({"op":"complete_checklist","ref":id}), false).await;
    assert!(done.contains("completed"), "{done}");
    knowledge(
        &project,
        json!({"op":"add_items","ref":id,"items":["Late"]}),
        true,
    )
    .await;
    let reopened = knowledge(
        &project,
        json!({"op":"reopen_checklist","ref":id,"reason":"found a regression"}),
        false,
    )
    .await;
    assert!(reopened.contains("open"), "{reopened}");
    let context = project
        .call("get_context", json!({"ref":format!("{id}/I-002")}), false)
        .await;
    assert!(context.contains("not needed this release"), "{context}");
    let status = project.call("project_status", json!({}), false).await;
    assert!(
        !status.contains("I-001"),
        "work status never copies checklist items: {status}"
    );
}

/// K6: every creator sees one allocation token, and a foreign name in a home refuses creation by name.
#[tokio::test]
#[ignore = "needs the combined E-001 candidate carrying knowledge_work"]
async fn k6_one_allocation_observation_and_named_inventory_guard() {
    let project = Project::register().await;
    let token = project.allocation_version().await;
    assert_eq!(
        token,
        project.allocation_version().await,
        "a token is stable for one disk state"
    );
    let id = decision(&project, "Allocation").await;
    assert_eq!(id, "D-001");
    let counters_before = std::fs::read(project.root.join(".agent-tasks/knowledge.yaml")).unwrap();
    assert_ne!(
        token,
        project.allocation_version().await,
        "a create changes the observation"
    );
    std::fs::create_dir_all(project.root.join("decisions/stray-directory")).unwrap();
    let before = tree(&project.root);
    let refused = knowledge(&project, json!({"op":"create_decision","title":"Blocked","question":"q","decision":"d","rationale":"r"}), true).await;
    assert!(
        refused.contains("stray-directory") || refused.contains("inventory"),
        "the gap is named: {refused}"
    );
    assert_eq!(
        before,
        tree(&project.root),
        "a refused creation writes nothing"
    );
    assert_eq!(
        counters_before,
        std::fs::read(project.root.join(".agent-tasks/knowledge.yaml")).unwrap()
    );
}

/// K7: at the history cap with Git rejecting commits an edit is refused unchanged; after the originals commit it evicts.
#[tokio::test]
#[ignore = "needs the combined E-001 candidate carrying knowledge_work"]
async fn k7_history_cap_needs_committed_originals() {
    let project = Project::register().await;
    let id = decision(&project, "Capped").await;
    install_hook(&project.root, "pre-commit", "exit 1");
    let mut last = String::new();
    for n in 0..16 {
        last = knowledge(
            &project,
            json!({"op":"edit_decision","ref":id,"rationale":format!("Revision text {n}")}),
            false,
        )
        .await;
    }
    assert!(
        last.contains("Deferred") || last.contains("deferred"),
        "commits are deferred while the hook fails: {last}"
    );
    let path = project.root.join("decisions/D-001.yaml");
    let bytes = std::fs::read(&path).unwrap();
    let refused = knowledge(
        &project,
        json!({"op":"edit_decision","ref":id,"rationale":"One edit past the cap"}),
        true,
    )
    .await;
    assert!(refused.contains("history_unrecoverable"), "{refused}");
    assert_eq!(
        bytes,
        std::fs::read(&path).unwrap(),
        "a refused edit leaves the file byte identical"
    );
    remove_hook(&project.root, "pre-commit");
    knowledge(&project, json!({"op":"create_research","title":"Settle","question":"q","conclusions":[{"statement":"s","basis":"inferred"}],"applicability":"a"}), false).await;
    let evicted = knowledge(
        &project,
        json!({"op":"edit_decision","ref":id,"rationale":"Now it may evict"}),
        false,
    )
    .await;
    assert!(evicted.starts_with("SAVED D-001"), "{evicted}");
}

/// K8: typed reads (summary, history, references, search) change no byte, lock, time or index entry.
#[tokio::test]
#[ignore = "needs the combined E-001 candidate carrying knowledge_work"]
async fn k8_typed_reads_are_writeless() {
    let project = Project::register().await;
    let id = decision(&project, "Readable").await;
    let before = tree(&project.root);
    for args in [
        json!({"ref":id}),
        json!({"ref":id,"view":"history"}),
        json!({"ref":id,"view":"references"}),
        json!({}),
    ] {
        project.call("get_context", args, false).await;
    }
    project
        .call(
            "search",
            json!({"query":"readable","kinds":["knowledge"]}),
            false,
        )
        .await;
    project.call("project_status", json!({}), false).await;
    assert_eq!(before, tree(&project.root));
    std::fs::write(project.root.join("decisions/D-002.yaml"), "not: [valid").unwrap();
    let context = project.call("get_context", json!({}), false).await;
    assert!(
        context.contains("PARTIAL") || context.contains("unreadable"),
        "a corrupt sibling is named, never dropped: {context}"
    );
}

/// K9: the optional detail reference accepts an existing document and refuses unsafe forms.
#[tokio::test]
#[ignore = "needs the combined E-001 candidate carrying knowledge_work and document_work"]
async fn k9_detail_reference_validation() {
    let project = Project::register().await;
    let doc_version = project.version("docs/design.md").await;
    project
        .call("document_work", json!({"op":"save","ref":"docs/design.md","purpose":"Design","body":"# Design\n\n## Section\ntext\n","version":doc_version,"actor":"qualification-agent"}), false)
        .await;
    for (detail, error) in [
        ("docs/design.md#section", false),
        ("docs/missing.md", true),
        ("docs/../x.md", true),
        ("/docs/design.md", true),
        ("D-001", true),
    ] {
        knowledge(
            &project,
            json!({"op":"create_decision","title":"Detail","question":"q","decision":"d","rationale":"r","detail":detail}),
            error,
        )
        .await;
    }
}

/// K10: the existing core workflow behaves as before and knowledge homes add no work inventory warning.
#[tokio::test]
#[ignore = "needs the combined E-001 candidate carrying knowledge_work"]
async fn k10_core_workflow_unchanged_next_to_knowledge() {
    let project = Project::register().await;
    decision(&project, "Beside work").await;
    let context = project.call("get_context", json!({}), false).await;
    let created = project
        .call(
            "plan_work",
            json!({"version":support::field(&context,"Allocation version: "),"op":"create_module","title":"Work beside knowledge","outcome":"Work still allocates","criteria":["Work still allocates"],"contracts":{"not_required":true},"lead":{"name":"fixture"}}),
            false,
        )
        .await;
    assert!(created.starts_with("SAVED M-001"), "{created}");
    let status = project.call("project_status", json!({}), false).await;
    assert!(
        !status.contains("warning"),
        "knowledge homes add no inventory warning: {status}"
    );
}
