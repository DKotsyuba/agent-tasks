//! Real-binary catalog, closed-schema, field-named error and read-writeless qualification (M-006 criteria 1 and 2).
//!
//! Rows covered: the registered 14-tool surface of `registered-tool-surface` r4 (catalog, closed schemas, exact
//! operation lists), the field-named validation observation AT-003, and the proof that reads change nothing.
//! Tests that need the four producer tools are `#[ignore]`d with the exact reason until the combined candidate
//! exists; the read-writeless and legacy-compatibility tests run against any candidate.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "Explicit protocol assertions"
)]
mod support;

use serde_json::{Value, json};
use std::collections::BTreeSet;
use support::{Project, call, connect, tree};

/// The ten tools that exist before E-001.
const EXISTING: [&str; 10] = [
    "get_status",
    "register_project",
    "get_project_list",
    "get_context",
    "project_status",
    "search",
    "plan_work",
    "record_work",
    "review_work",
    "review_module",
];

/// The four producer tools added by E-001, each with the closed operation list of its provider artifact.
const PRODUCERS: [(&str, &[&str]); 4] = [
    (
        "knowledge_work",
        &[
            "create_decision",
            "create_runbook",
            "create_research",
            "create_checklist",
            "edit_decision",
            "edit_runbook",
            "edit_research",
            "supersede",
            "use_runbook",
            "add_items",
            "resolve_item",
            "complete_checklist",
            "cancel_checklist",
            "reopen_checklist",
        ],
    ),
    (
        "document_work",
        &["save", "replace_section", "adopt", "remove", "relocate"],
    ),
    (
        "compaction_work",
        &[
            "propose",
            "revise",
            "review",
            "recover_reviewer",
            "apply",
            "withdraw",
        ],
    ),
    (
        "git_recovery",
        &["reconcile", "retry", "adopt", "release", "preserve"],
    ),
];

/// Collect every `op` constant a tool schema admits, whether it nests variants under `oneOf` or `anyOf`.
fn ops(schema: &Value) -> BTreeSet<String> {
    fn walk(value: &Value, out: &mut BTreeSet<String>) {
        match value {
            Value::Object(map) => {
                if let Some(op) = map.get("properties").and_then(|p| p.get("op")) {
                    if let Some(constant) = op.get("const").and_then(Value::as_str) {
                        out.insert(constant.to_owned());
                    }
                    if let Some(values) = op.get("enum").and_then(Value::as_array) {
                        out.extend(values.iter().filter_map(Value::as_str).map(str::to_owned));
                    }
                }
                map.values().for_each(|inner| walk(inner, out));
            }
            Value::Array(items) => items.iter().for_each(|inner| walk(inner, out)),
            _ => {}
        }
    }
    let mut out = BTreeSet::new();
    walk(schema, &mut out);
    out
}

/// The real discovery list: exactly ten existing plus four producer tools, each with a root object schema,
/// the provider's closed operation list, and discovery equal to the exported snapshot.
#[tokio::test]
#[ignore = "needs the combined E-001 candidate: knowledge_work, document_work, compaction_work and git_recovery"]
async fn catalog_is_exactly_fourteen_closed_tools() {
    let temp = tempfile::tempdir_in("/private/tmp").unwrap();
    let config = temp.path().join("config.toml");
    std::fs::write(&config, "schema_version = 1\n").unwrap();
    let client = connect(&config, temp.path()).await;
    let listed = client.list_tools(Default::default()).await.unwrap().tools;
    let names: BTreeSet<String> = listed.iter().map(|tool| tool.name.to_string()).collect();
    let expected: BTreeSet<String> = EXISTING
        .iter()
        .copied()
        .chain(PRODUCERS.iter().map(|(name, _)| *name))
        .map(str::to_owned)
        .collect();
    assert_eq!(
        names, expected,
        "the registered surface is exactly fourteen tools, no alias"
    );
    let wire = serde_json::to_value(&listed).unwrap();
    let exported: Value = serde_json::from_str(include_str!("../schemas/tools.json")).unwrap();
    assert_eq!(
        wire, exported,
        "discovery must equal the reviewed exported snapshot"
    );
    for tool in wire.as_array().unwrap() {
        let schema = &tool["inputSchema"];
        assert_eq!(
            schema["type"], "object",
            "{}: root type must be object for external clients",
            tool["name"]
        );
    }
    for (name, expected_ops) in PRODUCERS {
        let tool = wire
            .as_array()
            .unwrap()
            .iter()
            .find(|tool| tool["name"] == name)
            .unwrap();
        let actual = ops(&tool["inputSchema"]);
        let wanted: BTreeSet<String> = expected_ops.iter().map(|op| (*op).to_owned()).collect();
        assert_eq!(
            actual, wanted,
            "{name}: the closed operation list differs from the provider artifact"
        );
        let validator = jsonschema::validator_for(&tool["inputSchema"]).unwrap();
        let unknown = json!({"project":"p","version":"v","op":expected_ops[0],"unexpected_field_for_closed_schema":true});
        assert!(
            !validator.is_valid(&unknown),
            "{name}: an unknown field must be refused by the closed schema"
        );
    }
    client.cancel().await.unwrap();
}

/// A bad call to each producer tool is refused with a field-named, value-free error and saves nothing.
#[tokio::test]
#[ignore = "needs the combined E-001 candidate: knowledge_work, document_work, compaction_work and git_recovery"]
async fn producer_tools_refuse_unknown_operations_and_fields_by_name() {
    let project = Project::register().await;
    let before = tree(&project.root);
    for (name, expected_ops) in PRODUCERS {
        let text = project
            .call(
                name,
                json!({"version":"x","op":"definitely_not_an_operation"}),
                true,
            )
            .await;
        assert!(text.starts_with("ERROR"), "{name}: {text}");
        assert!(
            expected_ops
                .iter()
                .all(|op| !text.contains(&format!("\"{op}\"")) || text.len() < 4096),
            "{name}: refusal must stay bounded"
        );
        let text = project
            .call(
                name,
                json!({"version":"x","op":expected_ops[0],"zzz_unknown_field":"SECRET-VALUE-MUST-NOT-ECHO"}),
                true,
            )
            .await;
        assert!(
            text.contains("zzz_unknown_field"),
            "{name}: the field must be named: {text}"
        );
        assert!(
            !text.contains("SECRET-VALUE-MUST-NOT-ECHO"),
            "{name}: a value was echoed: {text}"
        );
    }
    assert_eq!(
        before,
        tree(&project.root),
        "refused calls must publish nothing"
    );
}

/// AT-003 retest on the real stdio path: an over-long list names its field and limit and echoes no value.
#[tokio::test]
#[ignore = "AT-003: reproduced on baseline 0.9.2 (generic error); passes only on the combined candidate with M-003 field-named errors"]
async fn over_long_list_refusal_names_the_field_and_limit() {
    let project = Project::register().await;
    let context = project.call("get_context", json!({}), false).await;
    let criteria: Vec<String> = (0..12)
        .map(|n| format!("QUALIFICATION-VALUE-{n}"))
        .collect();
    let text = project
        .call(
            "plan_work",
            json!({"version":support::field(&context,"Allocation version: "),"op":"create_module","title":"Too many criteria","outcome":"Refuse by field name","criteria":criteria,"contracts":{"not_required":true}}),
            true,
        )
        .await;
    assert!(
        text.contains("criteria"),
        "the offending field must be named: {text}"
    );
    assert!(
        !text.contains("QUALIFICATION-VALUE"),
        "a supplied value must never be echoed: {text}"
    );
}

/// AT-003 retest: a wrong field name on a planning edit is named, not reported as a generic shape failure.
#[tokio::test]
#[ignore = "AT-003: reproduced on baseline 0.9.2 (generic error); passes only on the combined candidate with M-003 field-named errors"]
async fn wrong_field_name_is_named_on_a_real_call() {
    let project = Project::register().await;
    let text = project
        .call(
            "plan_work",
            json!({"version":"0".repeat(64),"op":"edit_task","module":"M-001","task":"T-001","titel":"QUALIFICATION-VALUE"}),
            true,
        )
        .await;
    assert!(
        text.contains("titel"),
        "the unknown field must be named: {text}"
    );
    assert!(
        !text.contains("QUALIFICATION-VALUE"),
        "a supplied value must never be echoed: {text}"
    );
}

/// Reads of work records change no byte, lock, directory, time or index entry, and create no knowledge home.
#[tokio::test]
async fn work_reads_are_writeless() {
    let project = Project::register().await;
    let context = project.call("get_context", json!({}), false).await;
    let created = project
        .call(
            "plan_work",
            json!({"version":support::field(&context,"Allocation version: "),"op":"create_module","title":"Writeless fixture","outcome":"Reads write nothing","criteria":["Reads write nothing"],"contracts":{"not_required":true},"lead":{"name":"fixture"}}),
            false,
        )
        .await;
    assert!(created.starts_with("SAVED M-001"), "{created}");
    let before = tree(&project.root);
    for (name, args) in [
        ("get_context", json!({})),
        ("get_context", json!({"ref":"M-001"})),
        ("get_context", json!({"ref":"M-001","view":"log"})),
        ("project_status", json!({})),
        ("search", json!({"query":"writeless"})),
        ("get_project_list", json!({})),
    ] {
        let mut args = args;
        if name != "get_project_list" {
            args["project"] = json!("product");
        }
        call(&project.client, name, args, false).await;
    }
    let after = tree(&project.root);
    assert_eq!(
        before, after,
        "reads must not write, lock, create or touch anything"
    );
    for home in [
        "decisions",
        "runbooks",
        "research",
        "checklists",
        "documents",
        "compactions",
    ] {
        assert!(
            !project.root.join(home).exists(),
            "{home} must not appear before first use"
        );
    }
}

/// A restart over the same disposable state reads the same registered project (cold compatibility).
#[tokio::test]
async fn cold_restart_reads_the_same_project() {
    let mut project = Project::register().await;
    let before = project.call("get_context", json!({}), false).await;
    project.restart().await;
    let after = project.call("get_context", json!({}), false).await;
    assert_eq!(
        support::field(&before, "Allocation version: "),
        support::field(&after, "Allocation version: ")
    );
}

/// AT-004 regression: an Epic scope in `project_status` keeps honest child counts and names unreadable members.
///
/// A project with an Epic and two member Modules must report the two members as readable of declared, never zero,
/// and a corrupted member must be named with partial coverage rather than counted as nothing.
#[tokio::test]
#[ignore = "AT-004: reproduced on baseline 0.9.2 (project_status module=E-001 shows no readable members); passes with M-003 Epic roll-up"]
async fn epic_status_keeps_honest_child_counts() {
    let project = Project::register().await;
    let version = |text: &str| support::field(text, "Allocation version: ");
    let context = project.call("get_context", json!({}), false).await;
    let epic = project
        .call(
            "plan_work",
            json!({"version":version(&context),"op":"create_epic","title":"Roll-up Epic","outcome":"Children are counted","criteria":["Children are counted"]}),
            false,
        )
        .await;
    assert!(epic.starts_with("SAVED E-001"), "{epic}");
    let mut modules = Vec::new();
    for title in ["First child", "Second child"] {
        let context = project.call("get_context", json!({}), false).await;
        let created = project
            .call(
                "plan_work",
                json!({"version":version(&context),"op":"create_module","title":title,"outcome":"Child outcome","criteria":["Child outcome"],"contracts":{"not_required":true}}),
                false,
            )
            .await;
        modules.push(support::target(&created));
    }
    let epic_version = project.version("E-001").await;
    project
        .call(
            "plan_work",
            json!({"version":epic_version,"op":"edit_epic","epic":"E-001","modules":modules}),
            false,
        )
        .await;
    let status = project
        .call("project_status", json!({"module":"E-001"}), false)
        .await;
    assert!(
        !status.contains("/ 0 readable") && status.contains('2'),
        "both declared members are counted readable, never zero: {status}"
    );
    std::fs::write(project.root.join("modules/M-002.yaml"), "not: [valid").unwrap();
    let partial = project
        .call("project_status", json!({"module":"E-001"}), false)
        .await;
    assert!(
        partial.contains("M-002")
            && (partial.contains("PARTIAL") || partial.contains("unreadable")),
        "an unreadable member is named, never zero: {partial}"
    );
}
