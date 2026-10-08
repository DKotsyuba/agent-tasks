//! Boundary controls for the consumed `kr-operations` r7 entry, run through the real `knowledge_work` tool.
//!
//! These cases are the observed side of the implementation-mutation controls recorded for M-006: each case below
//! holds a positive and a negative expectation that a deliberate implementation mutant of the product must break
//! (the mutants and their observed results are reported with the Module evidence, never committed). The cases use
//! only the shared real-binary helpers and a scrubbed disposable project; nothing is injected into the product.
//!
//! Covered obligations of `kr-operations` r7: the runbook work coverage matrix (stored bare `M-`, `A-` and `E-`
//! records, a Task and an embedded Atomic of a Module that has them, refusal of everything else), attribution against
//! a bound lead, the allocator-file refusal that precedes inventory, no home directory before a reservation, and the
//! rule that a successor must itself be current.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "Explicit protocol assertions"
)]
mod support;

use serde_json::{Value, json};
use support::{Project, knowledge, target, tree};

/// The synthetic bound lead of the fixture Module.
const LEAD: &str = "bc-synthetic-lead";

/// Run one core work mutation with the current write precondition (allocation version for `None`).
async fn work(project: &Project, tool: &str, owner: Option<&str>, mut args: Value) -> String {
    let version = match owner {
        None => support::field(
            &project.call("get_context", json!({}), false).await,
            "Allocation version: ",
        ),
        Some(owner) => project.version(owner).await,
    };
    args["version"] = json!(version);
    project.call(tool, args, false).await
}

/// Create Module `M-001` with one embedded Atomic and, when `lead` is true, a bound lead and one Task.
async fn module(project: &Project, lead: bool) {
    work(
        project,
        "plan_work",
        None,
        json!({"op":"create_module","title":"Boundary fixture","outcome":"Runbook use has a real work target","criteria":["Runbook use has a real work target"],"contracts":{"not_required":true}}),
    )
    .await;
    work(
        project,
        "plan_work",
        Some("M-001"),
        json!({"op":"add_atomic","module":"M-001","title":"Embedded fixture","outcome":"An embedded Atomic exists"}),
    )
    .await;
    if lead {
        work(
            project,
            "record_work",
            Some("M-001"),
            json!({"op":"bind_agent","ref":"M-001","role":"lead","harness":"fixture-only","agent_id":LEAD,"communication_ref":"fixture-only: communicate","resume_ref":"fixture-only: resume","launch_ref":"fixture-only: receipt"}),
        )
        .await;
        work(
            project,
            "record_work",
            Some("M-001"),
            json!({"op":"planning","ref":"M-001","actor":LEAD,"responsibility":"Own the fixture","scope":"fixture","exclusions":["other"],"read_refs":[],"uncertainties":[]}),
        )
        .await;
        work(
            project,
            "plan_work",
            Some("M-001"),
            json!({"op":"add_task","module":"M-001","actor":LEAD,"title":"Fixture task"}),
        )
        .await;
    }
}

/// One `use_runbook` against `work` as `actor`; returns the reply and whether the tree changed.
async fn use_runbook(
    project: &Project,
    runbook: &str,
    work: &str,
    actor: Option<&str>,
    error: bool,
) -> (String, bool) {
    let mut args = json!({"op":"use_runbook","ref":runbook,"revision":1,"outcome":"succeeded","environment":"disposable fixture","checks":[{"label":"health","status":"passed"}],"work":work,"version":project.version(runbook).await});
    if let Some(actor) = actor {
        args["actor"] = json!(actor);
    }
    let before = tree(&project.root);
    let text = project.call("knowledge_work", args, error).await;
    (text, before != tree(&project.root))
}

/// Create a minimal Runbook and return its id.
async fn runbook(project: &Project) -> String {
    let text = knowledge(
        project,
        json!({"op":"create_runbook","title":"Operate","purpose":"Restore service","steps":[{"title":"Check","description":"Inspect","expected":"Healthy"}]}),
        false,
    )
    .await;
    target(&text)
}

/// Work coverage: stored bare records, a Task and an embedded Atomic are accepted; everything else is refused
/// as `invalid_arguments` with nothing written; a Module without a bound lead accepts any declared actor.
#[tokio::test]
async fn kr7_runbook_work_coverage_matrix() {
    let project = Project::register().await;
    module(&project, false).await;
    let rb = runbook(&project).await;
    for refused in [
        "M-009",
        "M-001/T-001",
        "M-001/A-009",
        "M-001/X-001",
        "M-001/T-001/extra",
        "A-001",
        "E-001",
        "docs/x.md",
    ] {
        let (text, wrote) = use_runbook(&project, &rb, refused, Some("anyone"), true).await;
        assert!(text.contains("invalid_arguments"), "{refused}: {text}");
        assert!(!wrote, "{refused}: a refused use writes nothing");
    }
    for accepted in ["M-001", "M-001/A-001"] {
        let (text, _) = use_runbook(&project, &rb, accepted, Some("anyone"), false).await;
        assert!(text.starts_with("SAVED RB-001"), "{accepted}: {text}");
    }
}

/// Task versus embedded Atomic are looked up in their own lists, and a bound lead gates attribution.
#[tokio::test]
async fn kr7_task_versus_atomic_and_attribution() {
    let project = Project::register().await;
    module(&project, true).await;
    let rb = runbook(&project).await;
    // The Task T-001 exists and the embedded Atomic A-001 exists: each is looked up in its own list.
    for accepted in ["M-001/T-001", "M-001/A-001", "M-001"] {
        let (text, _) = use_runbook(&project, &rb, accepted, Some(LEAD), false).await;
        assert!(text.starts_with("SAVED RB-001"), "{accepted}: {text}");
    }
    // A Task number that only exists as an Atomic (and the reverse) must not resolve.
    for refused in ["M-001/T-002", "M-001/A-002"] {
        let (text, wrote) = use_runbook(&project, &rb, refused, Some(LEAD), true).await;
        assert!(
            text.contains("invalid_arguments") && !wrote,
            "{refused}: {text}"
        );
    }
    // Attribution: another agent and an absent actor are refused against the bound lead, with nothing written.
    for actor in [Some("someone-else"), None] {
        let (text, wrote) = use_runbook(&project, &rb, "M-001/T-001", actor, true).await;
        assert!(text.contains("attribution"), "{actor:?}: {text}");
        assert!(!wrote, "{actor:?}: an attribution refusal writes nothing");
    }
}

/// The allocator file is read before any inventory check; a refusal before the reservation creates no home.
#[tokio::test]
async fn kr7_allocator_refusal_order_and_no_home_before_reservation() {
    let project = Project::register().await;
    // A refusal before the reservation (a stale write precondition and a bad field) creates no home or counter file.
    let stale = knowledge(
        &project,
        json!({"op":"create_decision","title":"t","question":"q","decision":"d","rationale":"r","version":"0".repeat(64)}),
        true,
    )
    .await;
    assert!(stale.contains("stale"), "{stale}");
    let bad = knowledge(
        &project,
        json!({"op":"create_decision","title":"","question":"q","decision":"d","rationale":"r"}),
        true,
    )
    .await;
    assert!(
        bad.contains("invalid_arguments") || bad.contains("title"),
        "{bad}"
    );
    assert!(
        !project.root.join("decisions").exists(),
        "no home before a reservation"
    );
    assert!(!project.root.join(".agent-tasks/knowledge.yaml").exists());
    // A first create establishes the allocator; then a corrupt allocator file refuses before inventory, untouched.
    knowledge(
        &project,
        json!({"op":"create_decision","title":"First","question":"q","decision":"d","rationale":"r"}),
        false,
    )
    .await;
    let allocator = project.root.join(".agent-tasks/knowledge.yaml");
    std::fs::write(&allocator, "not: [valid").unwrap();
    std::fs::create_dir_all(project.root.join("research/stray-directory")).unwrap();
    let before = tree(&project.root);
    let refused = project
        .call(
            "knowledge_work",
            json!({"op":"create_decision","title":"Second","question":"q","decision":"d","rationale":"r","version":"0".repeat(64),"actor":"qualification-agent"}),
            true,
        )
        .await;
    assert!(
        refused.contains("allocator") || refused.contains("stale"),
        "{refused}"
    );
    assert_eq!(
        before,
        tree(&project.root),
        "a refused create writes nothing and leaves the corrupt file untouched"
    );
    assert_eq!(std::fs::read(&allocator).unwrap(), b"not: [valid");
}

/// A successor must itself be current: superseding into a superseded record is refused; a current successor links.
#[tokio::test]
async fn kr7_successor_must_be_current() {
    let project = Project::register().await;
    let mut ids = Vec::new();
    for title in ["One", "Two", "Three"] {
        let text = knowledge(
            &project,
            json!({"op":"create_decision","title":title,"question":"q","decision":"d","rationale":"r"}),
            false,
        )
        .await;
        ids.push(target(&text));
    }
    let linked = knowledge(
        &project,
        json!({"op":"supersede","ref":ids[0],"successor":ids[1]}),
        false,
    )
    .await;
    assert!(linked.starts_with("SAVED"), "{linked}");
    let refused = knowledge(
        &project,
        json!({"op":"supersede","ref":ids[2],"successor":ids[0]}),
        true,
    )
    .await;
    assert!(
        refused.contains("conflict") || refused.contains("current"),
        "{refused}"
    );
    let before = tree(&project.root);
    knowledge(
        &project,
        json!({"op":"supersede","ref":ids[1],"successor":ids[1]}),
        true,
    )
    .await;
    assert_eq!(
        before,
        tree(&project.root),
        "a refused supersession writes nothing"
    );
}
