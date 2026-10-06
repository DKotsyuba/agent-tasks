//! Real stdio/SDK regressions for Epic/Atomic workflows, legacy data and cold restarts.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "Explicit protocol assertions"
)]
use rmcp::{
    RoleClient, ServiceExt, model::CallToolRequestParams, service::RunningService,
    transport::TokioChildProcess,
};
use serde_json::{Value, json};
use std::{process::Stdio, time::Duration};

/// Read one exact generated precondition from the server's compact response.
fn field(text: &str, label: &str) -> String {
    text.lines()
        .find_map(|s| s.strip_prefix(label))
        .expect(text)
        .to_owned()
}

/// Start a fresh binary with a disposable explicit config and no inherited credentials.
async fn connect(
    config: &std::path::Path,
    home: &std::path::Path,
) -> RunningService<RoleClient, ()> {
    let binary = std::env::var_os("MCP_TEST_BINARY")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| env!("CARGO_BIN_EXE_agent-tasks").into());
    let mut command = tokio::process::Command::new(binary);
    command
        .arg("--config")
        .arg(config)
        .arg("mcp")
        .env_clear()
        .env("HOME", home)
        .env("GIT_DIR", home.join("unrelated-git-directory"))
        .env("GIT_WORK_TREE", home.join("unrelated-git-worktree"))
        .env("GIT_INDEX_FILE", home.join("unrelated-git-index"))
        .current_dir(home)
        .kill_on_drop(true)
        .stderr(Stdio::null());
    ().serve(TokioChildProcess::new(command).unwrap())
        .await
        .unwrap()
}

/// Assert text-only contract through the SDK, retaining error semantics and bounded output.
async fn call(
    client: &RunningService<RoleClient, ()>,
    name: &str,
    args: Value,
    error: bool,
) -> String {
    let reply = client
        .call_tool(
            CallToolRequestParams::new(name.to_owned())
                .with_arguments(args.as_object().unwrap().clone()),
        )
        .await
        .unwrap();
    let wire = serde_json::to_value(reply).unwrap();
    assert_eq!(wire["isError"], error, "{name} {args}: {wire}");
    assert!(wire.get("structuredContent").is_none());
    assert_eq!(wire["content"].as_array().unwrap().len(), 1);
    let text = wire["content"][0]["text"].as_str().unwrap().to_owned();
    assert!(text.len() <= 8192);
    assert!(!text.contains("schema_version:"));
    text
}

/// Exercise six business tools through real protocol routing, then verify closure after restart.
#[tokio::test]
async fn core_stdio_cycle_and_cold_restart() {
    tokio::time::timeout(Duration::from_secs(30),async {
        let temp=tempfile::tempdir_in("/private/tmp").unwrap();let root=temp.path().join("portable-docs");let config=temp.path().join("config.toml");
        std::fs::write(&config,"schema_version = 1\n").unwrap();
        let client=connect(&config,temp.path()).await;
        let tools=client.list_tools(Default::default()).await.unwrap().tools;assert!(tools.iter().any(|tool|tool.name=="review_module") && tools.iter().any(|tool|tool.name=="review_work"));
        let listing=call(&client,"get_project_list",json!({}),false).await;assert!(listing.contains("No projects registered") && !root.exists());
        call(&client,"register_project",json!({"project":"product","doc_dir":root,"name":"Portable product","description":"Give agents work and the owner status"}),false).await;
        assert!(root.join(".git").is_dir());
        let listing=call(&client,"get_project_list",json!({}),false).await;assert!(listing.contains("Portable product") && !listing.contains(root.to_str().unwrap()));
        let context=call(&client,"get_context",json!({"project":"product"}),false).await;
        let created=call(&client,"plan_work",json!({"project":"product","version":field(&context,"Allocation version: "),"op":"create_module","title":"File-backed workflow","outcome":"Work survives process restarts","criteria":["Work survives process restarts"],"execution":execution(&root),"contracts":{"not_required":true},"lead":{"name":"lead"},"tasks":[{"title":"Persist the report","required_checks":["restart"]}]}),false).await;
        assert!(created.starts_with("SAVED M-001"));
        let begun=change(&client,"record_work",Some("M-001"),json!({"op":"begin","ref":"M-001","actor":"lead"}),false).await;let old=field(&begun,"Version: ");
        call(&client,"record_work",json!({"project":"product","version":old,"ref":"M-001/T-001","op":"result","state":"done","summary":"Report persisted to portable YAML","actor":"lead","checks":[{"label":"restart","status":"passed"}],"artifacts":["scenario: cold-restart"]}),false).await;
        let stale=call(&client,"record_work",json!({"project":"product","version":old,"ref":"M-001/T-001","op":"result","summary":"Stale replay must refuse"}),true).await;assert!(stale.contains("stale") && stale.contains("get_context"),"{stale}");
        let context=call(&client,"get_context",json!({"project":"product","ref":"M-001"}),false).await;
        call(&client,"review_module",json!({"project":"product","module":"M-001","version":field(&context,"Version: "),"verdict":"accepted","summary":"Reported native scenario independently checked","actor":"reviewer"}),false).await;
        let reviewed=call(&client,"get_context",json!({"project":"product","ref":"M-001"}),false).await;assert!(reviewed.contains("delivery pending"),"{reviewed}");
        change(&client,"record_work",Some("M-001"),json!({"op":"deliver","ref":"M-001","target_branch":"main","summary":"Local delivery reported","actor":"lead"}),false).await;
        let search=call(&client,"search",json!({"project":"product","query":"persisted portable"}),false).await;assert!(search.contains("M-001/T-001"));
        let status=call(&client,"project_status",json!({"project":"product"}),false).await;assert!(status.contains("1 accepted / 1 readable") && status.contains("1 done / 1 readable"),"{status}");
        client.cancel().await.unwrap();
        let cold=connect(&config,temp.path()).await;
        let status=call(&cold,"project_status",json!({"project":"product"}),false).await;assert!(status.contains("1 accepted / 1 readable") && status.contains("reviewer"));
        let context=call(&cold,"get_context",json!({"project":"product","ref":"M-001/T-001","view":"results"}),false).await;assert!(context.contains("scenario: cold-restart"));
        cold.cancel().await.unwrap();
        let yaml:serde_yaml_ng::Value=serde_yaml_ng::from_slice(&std::fs::read(root.join("modules/M-001.yaml")).unwrap()).unwrap();
        assert_eq!(yaml["tasks"][0]["state"].as_str(),Some("done"));assert_eq!(yaml["reviews"][0]["verdict"].as_str(),Some("accepted"));
        let binary=std::env::var_os("MCP_TEST_BINARY").map(std::path::PathBuf::from).unwrap_or_else(||env!("CARGO_BIN_EXE_agent-tasks").into());
        let doctor=std::process::Command::new(binary).args(["--config","/absent/not-a-config","doctor","--json"]).env_clear().output().unwrap();
        assert!(doctor.status.success());let doctor:Value=serde_json::from_slice(&doctor.stdout).unwrap();assert_eq!(doctor["local_ready"],true);let family:toml::Value=toml::from_str(include_str!("../family.toml")).unwrap(); assert_eq!(doctor["release_qualification"].as_str(),family["qualification"].as_str());
    }).await.expect("Bounded SDK/stdio cycle");
}

/// Read precondition tokens through the actual MCP instead of deriving storage versions in tests.
async fn version(client: &RunningService<RoleClient, ()>, reference: &str) -> String {
    let context = call(
        client,
        "get_context",
        json!({"project":"product","ref":reference}),
        false,
    )
    .await;
    field(&context, "Version: ")
}

/// Create an isolated registered documentation root; the returned directory owns all fixture data.
async fn fixture() -> (
    tempfile::TempDir,
    std::path::PathBuf,
    std::path::PathBuf,
    RunningService<RoleClient, ()>,
) {
    let temp = tempfile::tempdir_in("/private/tmp").unwrap();
    let root = temp.path().join("portable-docs");
    let config = temp.path().join("config.toml");
    std::fs::write(&config, "schema_version = 1\n").unwrap();
    let client = connect(&config, temp.path()).await;
    call(&client, "register_project", json!({"project":"product","doc_dir":root,"name":"Integration product","description":"Verify portable work hierarchy"}), false).await;
    (temp, root, config, client)
}

/// Preserve an accepted literal 0.8.0 review across reads, a nonsemantic handoff and restart.
#[tokio::test]
async fn legacy_schema_stdio_is_read_only_until_explicit_write() {
    tokio::time::timeout(Duration::from_secs(30), async {
        let temp = tempfile::tempdir_in("/private/tmp").unwrap();
        let root = temp.path().join("legacy-docs");
        std::fs::create_dir_all(root.join("modules")).unwrap();
        std::fs::create_dir_all(root.join(".agent-tasks")).unwrap();
        let config = temp.path().join("config.toml");
        std::fs::write(&config, "schema_version = 1\n").unwrap();
        std::fs::write(temp.path().join("projects.toml"), format!("schema_version = 1\n[aliases]\nproduct = {:?}\n", root.to_str().unwrap())).unwrap();
        let project = b"schema_version: 1\ntitle: Legacy product\npurpose: Preserve old reports\nremote: null\ncreated_at: 2026-09-01T00:00:00.000Z\nupdated_at: 2026-09-01T00:00:00.000Z\n";
        let allocator = b"schema_version: 1\nnext_module: 8\n";
        let module = br#"schema_version: 1
id: M-007
title: Legacy standalone module
outcome: Keep preserved portable evidence
lead:
  name: legacy-lead
  handle: null
required_checks: []
state: open
next_task: 2
next_log: 2
omitted_log_entries: 0
review_epoch: 1
tasks:
- id: T-001
  title: Legacy task
  criterion: null
  required_checks: []
  state: done
  result:
    summary: Legacy task evidence retained
    gaps: []
    followups: []
    artifacts:
    - legacy-artifact
    reported_at: 2026-09-01T00:00:00.000Z
    actor: legacy-lead
  checks: []
  cancellation: null
  cancellation_history: []
  created_at: 2026-09-01T00:00:00.000Z
  updated_at: 2026-09-01T00:00:00.000Z
result: null
checks: []
blocker: null
handoff: null
cancellation: null
cancellation_history: []
reviews:
- verdict: accepted
  summary: Independent legacy evidence inspection passed
  findings: []
  basis: c3cddfc405c31481de8e1da6c2f66a8c50e4db0c3c43d68e3e523f13f10f321e
  epoch: 1
  at: 2026-10-06T08:06:34.931Z
  reviewer: legacy-reviewer
  check_updates: []
reasons: []
log:
- id: L-001
  at: 2026-10-06T08:06:34.931Z
  actor: legacy-reviewer
  target: M-007
  action: review recorded
  note: null
created_at: 2026-09-01T00:00:00.000Z
updated_at: 2026-10-06T08:06:34.931Z
"#;
        std::fs::write(root.join("project.yaml"), project).unwrap();
        std::fs::write(root.join(".agent-tasks/state.yaml"), allocator).unwrap();
        std::fs::write(root.join("modules/M-007.yaml"), module).unwrap();
        let client = connect(&config, temp.path()).await;
        let context = call(&client, "get_context", json!({"project":"product","ref":"M-007/T-001","view":"results"}), false).await;
        assert!(context.contains("Legacy task evidence retained") && context.contains("legacy-artifact"), "{context}");
        let status = call(&client, "project_status", json!({"project":"product"}), false).await;
        assert!(status.contains("M-007") && status.contains("1 done / 1 readable") && status.contains("legacy-lead") && status.contains("1 accepted / 1 readable") && status.contains("legacy-reviewer"), "{status}");
        let search = call(&client, "search", json!({"project":"product","query":"legacy evidence"}), false).await;
        assert!(search.contains("M-007/T-001"), "{search}");
        assert_eq!(std::fs::read(root.join("project.yaml")).unwrap(), project);
        assert_eq!(std::fs::read(root.join(".agent-tasks/state.yaml")).unwrap(), allocator);
        assert_eq!(std::fs::read(root.join("modules/M-007.yaml")).unwrap(), module);
        assert!(!root.join("epics").exists() && !root.join("atomics").exists());
        let imported = call(&client, "get_context", json!({"project":"product","ref":"M-007","view":"review"}), false).await;
        assert!(imported.contains("accepted") && imported.contains("legacy-reviewer") && imported.contains("Independent legacy evidence inspection passed"), "{imported}");
        assert_eq!(std::fs::read(root.join("modules/M-007.yaml")).unwrap(), module);
        call(&client, "record_work", json!({"project":"product","ref":"M-007","version":version(&client,"M-007").await,"op":"handoff","stopping_point":"Accepted legacy delivery is preserved","next_action":"Resume without repeating acceptance"}), false).await;
        let context = call(&client, "get_context", json!({"project":"product","ref":"M-007"}), false).await;
        assert!(context.contains("accepted"), "{context}");
        client.cancel().await.unwrap();
        let cold = connect(&config, temp.path()).await;
        let context = call(&cold, "get_context", json!({"project":"product","ref":"M-007/T-001","view":"results"}), false).await;
        assert!(context.contains("legacy-artifact") && context.contains("Legacy task evidence retained"), "{context}");
        let saved: serde_yaml_ng::Value = serde_yaml_ng::from_slice(&std::fs::read(root.join("modules/M-007.yaml")).unwrap()).unwrap();
        assert_eq!(saved["created_at"].as_str(), Some("2026-09-01T00:00:00.000Z"));
        assert_eq!(saved["tasks"][0]["id"].as_str(), Some("T-001"));
        assert_eq!(saved["tasks"][0]["result"]["reported_at"].as_str(), Some("2026-09-01T00:00:00.000Z"));
        assert_eq!(saved["reviews"][0]["basis"].as_str(), Some("c3cddfc405c31481de8e1da6c2f66a8c50e4db0c3c43d68e3e523f13f10f321e"));
        assert_eq!(saved["reviews"][0]["reviewer"].as_str(), Some("legacy-reviewer"));
        assert_eq!(saved["reviews"][0]["at"].as_str(), Some("2026-10-06T08:06:34.931Z"));
        assert_eq!(saved["reviews"].as_sequence().unwrap().len(), 1);
        let accepted = call(&cold, "get_context", json!({"project":"product","ref":"M-007"}), false).await;
        assert!(accepted.contains("Module phase: accepted") && accepted.contains("legacy-reviewer"), "{accepted}");
        cold.cancel().await.unwrap();
    }).await.expect("Bounded legacy stdio cycle");
}

/// Supply the latest root allocation or owned file version, preserving explicit wire arguments.
async fn change(
    client: &RunningService<RoleClient, ()>,
    name: &str,
    owner: Option<&str>,
    mut args: Value,
    error: bool,
) -> String {
    let token = if let Some(reference) = owner {
        version(client, reference).await
    } else {
        field(
            &call(client, "get_context", json!({"project":"product"}), false).await,
            "Allocation version: ",
        )
    };
    let args = args.as_object_mut().unwrap();
    args.insert("project".into(), json!("product"));
    args.insert("version".into(), json!(token));
    call(client, name, json!(args), error).await
}

/// Verify hierarchy, independent acceptance and all Atomic owners through text-only SDK calls.
#[tokio::test]
async fn epic_atomic_hierarchy_stdio_cycle() {
    tokio::time::timeout(Duration::from_secs(60), async {
        let (temp, root, config, client) = fixture().await;
        let epic = change(&client, "plan_work", None, json!({"op":"create_epic","title":"Portable release","outcome":"Modules deliver together","criteria":["Both execution scopes deliver"],"required_checks":["epic-gate"],"lead":{"name":"orchestrator"}}), false).await;
        assert!(epic.starts_with("SAVED E-001"), "{epic}");
        let module = change(&client, "plan_work", None, json!({"op":"create_module","title":"Epic implementation","outcome":"Durable hierarchy works","criteria":["Durable hierarchy works"],"execution":execution(&root),"contracts":{"not_required":true},"lead":{"name":"lead"},"tasks":[{"title":"Persist hierarchy","required_checks":["task-gate"]}]}), false).await;
        assert!(module.starts_with("SAVED M-001"), "{module}");
        change(&client, "plan_work", Some("M-001"), json!({"op":"add_atomic","module":"M-001","title":"Module verification","outcome":"Embedded verification succeeds","required_checks":["module-atomic-gate"]}), false).await;
        change(&client, "plan_work", None, json!({"op":"create_atomic","title":"Epic verification","outcome":"Epic scenario succeeds","executor":{"name":"verifier"},"participants":["M-001"],"environment":"disposable hierarchy scenario","scenarios":["Epic Module interaction"]}), false).await;
        change(&client, "plan_work", None, json!({"op":"create_atomic","title":"Project microfix","outcome":"Tiny isolated correction is recorded","executor":{"name":"executor"}}), false).await;
        change(&client, "plan_work", None, json!({"op":"create_module","title":"Standalone compatibility","outcome":"Standalone Modules remain valid","criteria":["Standalone delivery"],"execution":execution(&root),"contracts":{"not_required":true},"lead":{"name":"standalone-lead"}}), false).await;
        change(&client, "plan_work", Some("E-001"), json!({"op":"edit_epic","epic":"E-001","modules":["M-001"],"atomics":["A-001"]}), false).await;
        change(&client, "plan_work", None, json!({"op":"create_epic","title":"Separate Epic","outcome":"Membership remains exclusive","criteria":["No duplicate ownership"]}), false).await;
        let duplicate = change(&client, "plan_work", Some("E-002"), json!({"op":"edit_epic","epic":"E-002","modules":["M-001"]}), true).await;
        assert!(duplicate.contains("already belongs") && duplicate.contains("Epic"), "{duplicate}");
        let dangling = change(&client, "plan_work", Some("E-001"), json!({"op":"edit_epic","epic":"E-001","modules":["M-999"]}), true).await;
        assert!(dangling.contains("not_found") && dangling.contains("Module does not exist"), "{dangling}");
        change(&client,"record_work",Some("E-001"),json!({"op":"begin","ref":"E-001","actor":"orchestrator"}),false).await;
        change(&client,"record_work",Some("M-001"),json!({"op":"begin","ref":"M-001","actor":"lead"}),false).await;
        change(&client,"record_work",Some("M-002"),json!({"op":"begin","ref":"M-002","actor":"standalone-lead"}),false).await;
        for (reference, owner, actor) in [("M-001/A-001","M-001","lead"),("A-002","A-002","executor")] {
            change(&client,"record_work",Some(owner),json!({"op":"begin","ref":reference,"actor":actor}),false).await;
        }
        let module_context = call(&client, "get_context", json!({"project":"product","ref":"M-001"}), false).await;
        assert!(module_context.contains("E-001") && module_context.contains("Modules deliver together") && module_context.contains("Both execution scopes deliver"), "{module_context}");
        let refusal = change(&client, "record_work", Some("E-001"), json!({"op":"cancel","ref":"E-001","reason":"Cannot implicitly cancel unfinished children"}), true).await;
        assert!(refusal.contains("M-001") || refusal.contains("children") || refusal.contains("member"), "{refusal}");
        change(&client, "record_work", Some("M-001"), json!({"op":"result","ref":"M-001/T-001","state":"done","summary":"Hierarchy persisted through portable files","actor":"lead","checks":[{"label":"task-gate","status":"passed"}]}), false).await;
        change(&client, "record_work", Some("M-001"), json!({"op":"result","ref":"M-001/A-001","state":"done","summary":"Embedded verification reported","checks":[{"label":"module-atomic-gate","status":"failed"}]}), false).await;
        let blocked = change(&client, "review_module", Some("M-001"), json!({"module":"M-001","verdict":"accepted","summary":"Premature approval must refuse","actor":"reviewer"}), true).await;
        assert!(blocked.contains("module-atomic-gate") && blocked.contains("passed"), "{blocked}");
        change(&client, "record_work", Some("M-001"), json!({"op":"result","ref":"M-001/A-001","state":"done","summary":"Embedded verification passed","checks":[{"label":"module-atomic-gate","status":"passed"}]}), false).await;
        change(&client,"review_work",Some("M-001"),json!({"ref":"M-001/A-001","verdict":"accepted","summary":"Embedded Atomic independently checked","actor":"reviewer"}),false).await;
        let self_review = change(&client, "review_work", Some("M-001"), json!({"ref":"M-001","verdict":"accepted","summary":"Lead cannot independently accept own Module","actor":"lead"}), true).await;
        assert!(self_review.contains("independent") || self_review.contains("lead") || self_review.contains("self"), "{self_review}");
        change(&client, "review_module", Some("M-001"), json!({"module":"M-001","verdict":"accepted","summary":"Task and embedded Atomic independently verified","actor":"reviewer"}), false).await;
        change(&client,"record_work",Some("M-001"),json!({"op":"deliver","ref":"M-001","target_branch":"main","summary":"Epic Module locally delivered","actor":"lead"}),false).await;
        let leaf_review = change(&client, "review_work", Some("M-001"), json!({"ref":"M-001/T-001","verdict":"accepted","summary":"Tasks do not have independent review","actor":"reviewer"}), true).await;
        assert!(leaf_review.contains("invalid") || leaf_review.contains("review"), "{leaf_review}");
        change(&client,"record_work",Some("A-001"),json!({"op":"begin","ref":"A-001","actor":"verifier"}),false).await;
        change(&client, "record_work", Some("A-001"), json!({"op":"result","ref":"A-001","state":"done","summary":"Epic scoped scenario passed","artifacts":["scenario: portable hierarchy"]}), false).await;
        change(&client, "record_work", Some("A-002"), json!({"op":"result","ref":"A-002","state":"done","summary":"Project microfix locally completed","actor":"executor"}), false).await;
        for reference in ["A-001","A-002"] {
            change(&client,"review_work",Some(reference),json!({"ref":reference,"verdict":"accepted","summary":"Independent Atomic acceptance","actor":"reviewer"}),false).await;
        }
        change(&client, "record_work", Some("M-002"), json!({"op":"result","ref":"M-002","summary":"Standalone Module delivered without Tasks","actor":"standalone-lead"}), false).await;
        change(&client, "review_work", Some("M-002"), json!({"ref":"M-002","verdict":"accepted","summary":"Standalone delivery independently checked","actor":"reviewer"}), false).await;
        change(&client,"record_work",Some("M-002"),json!({"op":"deliver","ref":"M-002","target_branch":"main","summary":"Standalone Module locally delivered","actor":"standalone-lead"}),false).await;
        change(&client, "record_work", Some("E-001"), json!({"op":"result","ref":"E-001","summary":"Epic delivery evidence assembled","checks":[{"label":"epic-gate","status":"not_applicable"}]}), false).await;
        let blocked = change(&client, "review_work", Some("E-001"), json!({"ref":"E-001","verdict":"accepted","summary":"Required check cannot be waived","actor":"epic-reviewer"}), true).await;
        assert!(blocked.contains("epic-gate") && blocked.contains("passed"), "{blocked}");
        change(&client, "record_work", Some("E-001"), json!({"op":"result","ref":"E-001","summary":"Epic delivery evidence assembled","checks":[{"label":"epic-gate","status":"passed"}]}), false).await;
        change(&client, "review_work", Some("E-001"), json!({"ref":"E-001","verdict":"accepted","summary":"Whole Epic independently accepted","actor":"epic-reviewer"}), false).await;
        let embedded = call(&client, "get_context", json!({"project":"product","ref":"M-001/A-001","view":"results"}), false).await;
        assert!(embedded.contains("M-001/A-001") && embedded.contains("Embedded verification passed"), "{embedded}");
        let atomic_context = call(&client, "get_context", json!({"project":"product","ref":"A-001"}), false).await;
        assert!(atomic_context.contains("E-001") && atomic_context.contains("Modules deliver together") && atomic_context.contains("Both execution scopes deliver"), "{atomic_context}");
        let epic_search = call(&client, "search", json!({"project":"product","query":"both execution scopes"}), false).await;
        assert!(epic_search.contains("E-001") && epic_search.contains("criteria"), "{epic_search}");
        let status = call(&client, "project_status", json!({"project":"product"}), false).await;
        assert!(status.contains("E-001") && status.contains("M-001") && status.contains("M-002") && status.contains("A-001") && status.contains("A-002"), "{status}");
        assert!(status.contains("2 accepted / 2 readable") && status.contains("1 done / 1 readable"), "{status}");
        assert!(status.contains("Epics: 1 accepted / 2 readable") && status.contains("Atomics: 3 done / 3 readable"), "{status}");
        assert!(status.contains("epic-reviewer") && status.contains("standalone-lead"), "{status}");
        let search = call(&client, "search", json!({"project":"product","query":"verification","limit":1}), false).await;
        assert!(search.contains("A-001") || search.contains("M-001/A-001"), "{search}");
        let page = call(&client, "search", json!({"project":"product","query":"verification","limit":1,"start":1,"version":field(&search,"Snapshot version: ")}), false).await;
        assert!(page.contains("A-001") || page.contains("M-001/A-001"), "{page}");
        assert_ne!(search.contains("M-001/A-001"), page.contains("M-001/A-001"));
        assert!(search.contains("2 matches") && page.contains("2 matches"), "{search}\n{page}");
        client.cancel().await.unwrap();
        let cold = connect(&config, temp.path()).await;
        let status = call(&cold, "project_status", json!({"project":"product"}), false).await;
        assert!(status.contains("2 accepted / 2 readable") && status.contains("epic-reviewer"), "{status}");
        let module: serde_yaml_ng::Value = serde_yaml_ng::from_slice(&std::fs::read(root.join("modules/M-001.yaml")).unwrap()).unwrap();
        assert_eq!(module["atomics"][0]["id"].as_str(), Some("A-001"));
        assert_eq!(module["tasks"][0]["state"].as_str(), Some("done"));
        assert_eq!(module["reviews"].as_sequence().unwrap().len(), 1);
        std::fs::write(root.join("epics/E-009.yaml"), "schema_version: broken\n").unwrap();
        let partial = call(&cold, "project_status", json!({"project":"product"}), false).await;
        assert!(partial.contains("PARTIAL") && partial.contains("E-009") && partial.contains("unknown"), "{partial}");
        assert!(partial.contains("2 accepted / 2 readable"), "{partial}");
        cold.cancel().await.unwrap();
    }).await.expect("Bounded hierarchy stdio cycle");
}

/// Verify lightweight Atomic gates, integration freshness and noncascading cancellation after restart.
#[tokio::test]
async fn integration_atomic_freshness_and_lifecycle_stdio() {
    tokio::time::timeout(Duration::from_secs(60), async {
        let (temp, root, config, client) = fixture().await;
        change(&client, "plan_work", None, json!({"op":"create_module","title":"Integration participant","outcome":"Participant works","criteria":["Participant delivers"],"execution":execution(&root),"contracts":{"not_required":true},"lead":{"name":"lead"},"tasks":[{"title":"Participant task"}]}), false).await;
        change(&client,"record_work",Some("M-001"),json!({"op":"begin","ref":"M-001","actor":"lead"}),false).await;
        change(&client, "record_work", Some("M-001"), json!({"op":"result","ref":"M-001/T-001","state":"done","summary":"Participant delivers initial behavior","actor":"lead"}), false).await;
        change(&client, "review_module", Some("M-001"), json!({"module":"M-001","verdict":"accepted","summary":"Participant independently accepted","actor":"reviewer"}), false).await;
        change(&client,"record_work",Some("M-001"),json!({"op":"deliver","ref":"M-001","target_branch":"main","summary":"Participant locally delivered","actor":"lead"}),false).await;
        change(&client, "plan_work", None, json!({"op":"create_epic","title":"Integration Epic","outcome":"Participating Module works together","criteria":["Current integration verification"]}), false).await;
        change(&client, "plan_work", None, json!({"op":"create_atomic","title":"Integration scenario","outcome":"Native cross-module scenario passes","executor":{"name":"integrator"},"environment":"disposable native fixture","scenarios":["Participant interaction"],"participants":["M-001"],"required_checks":["integration"]}), false).await;
        change(&client, "plan_work", Some("E-001"), json!({"op":"edit_epic","epic":"E-001","modules":["M-001"],"atomics":["A-001"]}), false).await;
        change(&client,"record_work",Some("E-001"),json!({"op":"begin","ref":"E-001"}),false).await;
        change(&client,"record_work",Some("A-001"),json!({"op":"begin","ref":"A-001","actor":"integrator"}),false).await;
        let failed = change(&client,"record_work",Some("A-001"),json!({"op":"result","ref":"A-001","state":"done","summary":"Scenario failed","checks":[{"label":"integration","status":"failed"}]}),false).await;
        assert!(failed.starts_with("SAVED A-001"),"{failed}");
        let failed_review=change(&client,"review_work",Some("A-001"),json!({"ref":"A-001","verdict":"accepted","summary":"Failed required check cannot be accepted","actor":"reviewer"}),true).await;
        assert!(failed_review.contains("integration") && failed_review.contains("passed"),"{failed_review}");
        change(&client, "record_work", Some("A-001"), json!({"op":"result","ref":"A-001","state":"done","summary":"Scenario initial execution passed","checks":[{"label":"integration","status":"passed","detail":"Scenario: member interaction"}],"artifacts":["scenario: initial native execution"]}), false).await;
        change(&client,"review_work",Some("A-001"),json!({"ref":"A-001","verdict":"accepted","summary":"Integration independently accepted","actor":"reviewer"}),false).await;
        change(&client, "record_work", Some("E-001"), json!({"op":"result","ref":"E-001","summary":"Current participant and integration accepted"}), false).await;
        change(&client, "review_work", Some("E-001"), json!({"ref":"E-001","verdict":"accepted","summary":"Independent Epic integration acceptance","actor":"epic-reviewer"}), false).await;
        let before = call(&client, "get_context", json!({"project":"product","ref":"A-001"}), false).await;
        assert!(before.contains("accepted") && before.contains("M-001"), "{before}");
        let search_before = call(&client, "search", json!({"project":"product","query":"participant","limit":1}), false).await;
        change(&client, "record_work", Some("M-001"), json!({"op":"reopen","ref":"M-001/T-001","reason":"Participant behavior needs correction"}), false).await;
        let stale_write = call(&client, "record_work", json!({"project":"product","version":field(&before,"Version: "),"op":"result","ref":"A-001","state":"done","summary":"Old integration version cannot reaffirm changed members","checks":[{"label":"integration","status":"passed"}]}), true).await;
        assert!(stale_write.contains("stale") && stale_write.contains("get_context"), "{stale_write}");
        let stale_page = call(&client, "search", json!({"project":"product","query":"participant","limit":1,"start":1,"version":field(&search_before,"Snapshot version: ")}), true).await;
        assert!(stale_page.contains("stale") && stale_page.contains("snapshot"), "{stale_page}");
        let stale_atomic = call(&client, "get_context", json!({"project":"product","ref":"A-001"}), false).await;
        assert!(stale_atomic.contains("stale") && stale_atomic.contains("M-001"), "{stale_atomic}");
        let stale_epic = call(&client, "get_context", json!({"project":"product","ref":"E-001"}), false).await;
        assert!(stale_epic.contains("stale"), "{stale_epic}");
        let blocked = change(&client, "review_work", Some("E-001"), json!({"ref":"E-001","verdict":"accepted","summary":"Cannot accept stale integration","actor":"epic-reviewer"}), true).await;
        assert!(blocked.contains("M-001") || blocked.contains("A-001"), "{blocked}");
        change(&client, "record_work", Some("M-001"), json!({"op":"result","ref":"M-001/T-001","state":"done","summary":"Participant corrected behavior delivered","actor":"lead"}), false).await;
        change(&client, "review_module", Some("M-001"), json!({"module":"M-001","verdict":"accepted","summary":"Corrected participant independently accepted","actor":"reviewer"}), false).await;
        change(&client,"record_work",Some("M-001"),json!({"op":"deliver","ref":"M-001","target_branch":"main","summary":"Participant locally delivered","actor":"lead"}),false).await;
        change(&client, "record_work", Some("A-001"), json!({"op":"result","ref":"A-001","state":"done","summary":"Scenario rerun against current participant passed","checks":[{"label":"integration","status":"passed"}]}), false).await;
        change(&client,"review_work",Some("A-001"),json!({"ref":"A-001","verdict":"accepted","summary":"Integration independently accepted","actor":"reviewer"}),false).await;
        change(&client, "review_work", Some("E-001"), json!({"ref":"E-001","verdict":"accepted","summary":"Current integration independently accepted","actor":"epic-reviewer"}), false).await;
        change(&client, "record_work", Some("E-001"), json!({"op":"cancel","ref":"E-001","reason":"Completed scope intentionally retired"}), false).await;
        let member = call(&client, "get_context", json!({"project":"product","ref":"M-001"}), false).await;
        assert!(member.contains("Module phase: accepted"), "{member}");
        change(&client, "record_work", Some("E-001"), json!({"op":"reopen","ref":"E-001","reason":"Same Epic receives followup acceptance"}), false).await;
        let epic = call(&client, "get_context", json!({"project":"product","ref":"E-001"}), false).await;
        assert!(epic.contains("stale"), "{epic}");
        let still_frozen = change(&client,"plan_work",Some("E-001"),json!({"op":"edit_epic","epic":"E-001","modules":[]}),true).await;
        assert!(still_frozen.contains("frozen") || still_frozen.contains("roster"),"{still_frozen}");
        change(&client, "record_work", Some("A-001"), json!({"op":"cancel","ref":"A-001","reason":"Integration is explicitly deferred"}), false).await;
        change(&client, "record_work", Some("A-001"), json!({"op":"reopen","ref":"A-001","reason":"Integration will be rerun"}), false).await;
        let atomic = call(&client, "get_context", json!({"project":"product","ref":"A-001"}), false).await;
        assert!(atomic.contains("open"), "{atomic}");
        client.cancel().await.unwrap();
        let cold = connect(&config, temp.path()).await;
        let atomic = call(&cold, "get_context", json!({"project":"product","ref":"A-001"}), false).await;
        assert!(atomic.contains("open") && atomic.contains("M-001"), "{atomic}");
        let status = call(&cold, "project_status", json!({"project":"product"}), false).await;
        assert!(status.contains("E-001") && status.contains("A-001") && status.contains("M-001"), "{status}");
        cold.cancel().await.unwrap();
    }).await.expect("Bounded integration lifecycle stdio cycle");
}

/// Keep Atomic-only Modules alive on compact status, child lifecycle and review check-update paths.
#[tokio::test]
async fn atomic_only_module_compact_stdio_regression() {
    tokio::time::timeout(Duration::from_secs(60), async {
        let (_temp, root, _config, client) = fixture().await;
        change(&client, "plan_work", None, json!({"op":"create_module","title":"Atomic-only delivery","outcome":"Five atomic scenarios deliver","criteria":["Five scenarios deliver"],"execution":execution(&root),"contracts":{"not_required":true},"lead":{"name":"module-lead","handle":"session: module-lead"}}), false).await;
        change(&client,"record_work",Some("M-001"),json!({"op":"begin","ref":"M-001","actor":"module-lead"}),false).await;
        for index in 1..=5 {
            let checks = if index == 1 { vec!["atomic-native"] } else { Vec::new() };
            change(&client, "plan_work", Some("M-001"), json!({"op":"add_atomic","module":"M-001","title":format!("Atomic scenario {index}"),"outcome":format!("Atomic scenario {index} passes"),"required_checks":checks,"executor":{"name":"atomic-executor","handle":"session: atomic-executor"}}), false).await;
            let reference = format!("M-001/A-{index:03}");
            change(&client,"record_work",Some("M-001"),json!({"op":"begin","ref":reference,"actor":"atomic-executor"}),false).await;
            let checks = if index == 1 { json!([{"label":"atomic-native","status":"failed","detail":"Scenario: atomic-first execution"}]) } else { json!([]) };
            change(&client, "record_work", Some("M-001"), json!({"op":"result","ref":reference,"state":"done","summary":format!("Unique atomic evidence {index}"),"actor":"atomic-executor","checks":checks,"artifacts":[format!("atomic-artifact-{index}")]}), false).await;
        }
        let compact = call(&client, "project_status", json!({"project":"product"}), false).await;
        assert!(compact.contains("Tasks: 0 done / 0 readable") && compact.contains("Atomic local outcomes: 5 done; final current closure: 0") && compact.contains("Atomics: 0 done / 5 readable"), "{compact}");
        assert!(compact.contains("PARTIAL") && compact.lines().any(|line| line.starts_with("M-001: 1 ") && line.contains("omitted")), "{compact}");
        assert!(compact.contains("atomic-executor") && compact.contains("session: atomic-executor"), "{compact}");
        let child = call(&client, "get_context", json!({"project":"product","ref":"M-001/A-001"}), false).await;
        assert!(child.contains("atomic-executor") && child.contains("session: atomic-executor"), "{child}");
        change(&client, "record_work", Some("M-001"), json!({"op":"cancel","ref":"M-001/A-001","reason":"First verification explicitly deferred"}), false).await;
        let canceled = call(&client, "get_context", json!({"project":"product","ref":"M-001/A-001"}), false).await;
        assert!(canceled.contains("canceled") && canceled.contains("First verification explicitly deferred"), "{canceled}");
        change(&client, "record_work", Some("M-001"), json!({"op":"reopen","ref":"M-001/A-001","reason":"First verification scheduled again"}), false).await;
        let reopened = call(&client, "get_context", json!({"project":"product","ref":"M-001/A-001"}), false).await;
        assert!(reopened.contains("open"), "{reopened}");
        change(&client,"record_work",Some("M-001"),json!({"op":"begin","ref":"M-001/A-001","actor":"atomic-executor"}),false).await;
        change(&client, "record_work", Some("M-001"), json!({"op":"result","ref":"M-001/A-001","state":"done","summary":"Unique atomic evidence 1 refreshed","actor":"atomic-executor","checks":[{"label":"atomic-native","status":"failed","detail":"Scenario: atomic-first execution"}],"artifacts":["atomic-artifact-1"]}), false).await;
        change(&client, "record_work", Some("M-001"), json!({"op":"result","ref":"M-001","summary":"Atomic-only Module delivered all scenarios","actor":"module-lead"}), false).await;
        for index in 1..=5 {
            let reference=format!("M-001/A-{index:03}");
            let checks=if index==1 {json!([{ "target":"M-001/A-001","label":"atomic-native","status":"passed","detail":"Scenario independently rerun"}])} else {json!([])};
            change(&client,"review_work",Some("M-001"),json!({"ref":reference,"verdict":"accepted","summary":"Atomic independently accepted","actor":"reviewer","checks":checks}),false).await;
        }
        change(&client, "review_module", Some("M-001"), json!({"module":"M-001","verdict":"accepted","summary":"Independent check verifies Atomic-only Module","actor":"reviewer","checks":[{"target":"M-001/A-001","label":"atomic-native","status":"passed","detail":"Scenario independently rerun"}]}), false).await;
        change(&client,"record_work",Some("M-001"),json!({"op":"deliver","ref":"M-001","target_branch":"main","summary":"Atomic-only delivery reported","actor":"module-lead"}),false).await;
        let checks = call(&client, "get_context", json!({"project":"product","ref":"M-001","view":"checks"}), false).await;
        assert!(checks.contains("M-001/A-001") && checks.contains("atomic-native") && checks.contains("passed") && checks.contains("Scenario independently rerun"), "{checks}");
        let results = call(&client, "get_context", json!({"project":"product","ref":"M-001","view":"results"}), false).await;
        assert!(results.contains("Unique atomic evidence 1 refreshed") && results.contains("Unique atomic evidence 5"), "{results}");
        let atomic_results = call(&client, "get_context", json!({"project":"product","ref":"M-001/A-001","view":"results"}), false).await;
        assert!(atomic_results.contains("Unique atomic evidence 1 refreshed") && atomic_results.contains("atomic-artifact-1"), "{atomic_results}");
        let search = call(&client, "search", json!({"project":"product","query":"unique atomic evidence"}), false).await;
        assert!(search.contains("5 matches") && search.contains("M-001/A-001") && search.contains("M-001/A-005"), "{search}");
        let compact = call(&client, "project_status", json!({"project":"product"}), false).await;
        assert!(compact.contains("Modules: 1 accepted / 1 readable") && compact.contains("Tasks: 0 done / 0 readable") && compact.contains("Atomics: 5 done / 5 readable"), "{compact}");
        assert!(compact.contains("PARTIAL") && compact.contains("omitted"), "{compact}");
        client.cancel().await.unwrap();
    }).await.expect("Bounded Atomic-only Module stdio cycle");
}

/// Declare an actual disposable checkout location; lifecycle calls report branch facts without Git writes.
fn execution(root: &std::path::Path) -> Value {
    json!({"repository":root,"worktree":root,"branch":"main","target_branch":"main"})
}

/// Use ordinary safe fixture Git commits; returned hashes identify actual disposable code reports.
fn commit_fixture(root: &std::path::Path, message: &str, source: &str) -> String {
    std::fs::create_dir_all(root).unwrap();
    let run = |args: &[&str]| {
        let mut command = std::process::Command::new("git");
        for (name, _) in std::env::vars_os() {
            if name.to_str().is_some_and(|name| name.starts_with("GIT_")) {
                command.env_remove(name);
            }
        }
        let output = command
            .current_dir(root)
            .args(args)
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_AUTHOR_NAME", "Code report author")
            .env("GIT_AUTHOR_EMAIL", "report@example.invalid")
            .env("GIT_COMMITTER_NAME", "Code report committer")
            .env("GIT_COMMITTER_EMAIL", "committer@example.invalid")
            .output()
            .unwrap();
        assert!(output.status.success(), "Disposable fixture Git failed");
        String::from_utf8(output.stdout).unwrap().trim().to_owned()
    };
    if !root.join(".git").exists() {
        run(&["-c", "init.defaultBranch=main", "init", "--quiet"]);
    }
    std::fs::write(root.join("lib.rs"), source).unwrap();
    run(&["add", "--", "lib.rs"]);
    run(&[
        "-c",
        "commit.gpgsign=false",
        "-c",
        "core.hooksPath=/dev/null",
        "commit",
        "--quiet",
        "--cleanup=verbatim",
        "-m",
        message,
    ]);
    run(&["rev-parse", "HEAD"])
}

/// Import multiple actual commit reports once while leaving Task completion to the known lead.
#[tokio::test]
async fn git_reports_import_once_stdio_and_retained_source_restart() {
    tokio::time::timeout(Duration::from_secs(60), async {
        let (temp, root, config, client) = fixture().await;
        let code = temp.path().join("code-source");
        let message = "feat: first report\n\nResult: Imported first behavior\nChecks:\npassed | inspection | Author reports manual inspection\nFollowups:\n- Optional future scenario\n";
        let first = commit_fixture(&code, message, "/// Disposable fixture function.\npub fn value() -> u8 { 1 }\n");
        let second = commit_fixture(&code, "fix: second report\n\nResult: Imported second behavior\nChecks:\nnot_run | inspection | Author did not run an automated test\n", "/// Disposable fixture function.\npub fn value() -> u8 { 2 }\n");
        change(&client,"plan_work",None,json!({"op":"create_module","title":"Code source import","outcome":"Import reports without copying","criteria":["Imported source retained"],"execution":execution(&code),"contracts":{"not_required":true},"lead":{"name":"lead"},"tasks":[{"title":"Import code report"}]}),false).await;
        change(&client,"record_work",Some("M-001"),json!({"op":"begin","ref":"M-001","actor":"lead"}),false).await;
        let stale = version(&client,"M-001").await;
        change(&client,"record_work",Some("M-001"),json!({"op":"import_commits","ref":"M-001/T-001","commits":[first,second],"actor":"lead"}),false).await;
        let task = call(&client,"get_context",json!({"project":"product","ref":"M-001/T-001"}),false).await;
        assert!(task.contains("open"),"{task}");
        let results = call(&client,"get_context",json!({"project":"product","ref":"M-001/T-001","view":"results"}),false).await;
        assert!(results.contains("Imported first behavior") && results.contains("Imported second behavior"),"{results}");
        let current = version(&client,"M-001").await;
        change(&client,"record_work",Some("M-001"),json!({"op":"import_commits","ref":"M-001/T-001","commits":[first,second],"actor":"lead"}),false).await;
        assert_eq!(version(&client,"M-001").await,current);
        let refused = call(&client,"record_work",json!({"project":"product","version":stale,"op":"import_commits","ref":"M-001/T-001","commits":[first],"actor":"lead"}),true).await;
        assert!(refused.contains("stale"),"{refused}");
        let foreign = change(&client,"record_work",Some("M-001"),json!({"op":"complete","ref":"M-001/T-001","actor":"other-agent"}),true).await;
        assert!(foreign.contains("lead"),"{foreign}");
        change(&client,"record_work",Some("M-001"),json!({"op":"complete","ref":"M-001/T-001","actor":"lead"}),false).await;
        let task = call(&client,"get_context",json!({"project":"product","ref":"M-001/T-001"}),false).await;
        assert!(task.contains("done"),"{task}");
        let self_review = change(&client,"review_work",Some("M-001"),json!({"ref":"M-001","verdict":"accepted","summary":"Imported report author cannot independently review","actor":"lead"}),true).await;
        assert!(self_review.contains("independent") || self_review.contains("lead") || self_review.contains("self"),"{self_review}");
        change(&client,"plan_work",Some("M-001"),json!({"op":"add_task","module":"M-001","title":"Second source consumer"}),false).await;
        change(&client,"record_work",Some("M-001"),json!({"op":"import_commits","ref":"M-001/T-002","commits":[first],"actor":"lead"}),false).await;

        let yaml:serde_yaml_ng::Value=serde_yaml_ng::from_slice(&std::fs::read(root.join("modules/M-001.yaml")).unwrap()).unwrap();
        assert_eq!(yaml["imports"].as_sequence().unwrap().len(),2);
        assert_eq!(yaml["imports"][0]["commit"]["message"].as_str(),Some(message));
        assert_eq!(yaml["imports"][0]["commit"]["sha"].as_str(),Some(first.as_str()));
        assert_eq!(yaml["imports"][0]["targets"][0].as_str(),Some("M-001/T-001"));
        assert_eq!(yaml["imports"][0]["targets"].as_sequence().unwrap().len(),2);
        assert_eq!(yaml["tasks"][1]["state"].as_str(),Some("open"));
        let search = call(&client,"search",json!({"project":"product","query":"imported second"}),false).await;
        assert!(search.contains("M-001/T-001"),"{search}");
        client.cancel().await.unwrap();
        let hidden = temp.path().join("temporarily-unavailable-source");
        std::fs::rename(&code,&hidden).unwrap();
        let cold = connect(&config,temp.path()).await;
        let retained = call(&cold,"get_context",json!({"project":"product","ref":"M-001/T-001","view":"results"}),false).await;
        assert!(retained.contains("Imported first behavior") && retained.contains("Imported second behavior"),"{retained}");
        let old_version = version(&cold,"M-001").await;
        let missing = change(&cold,"record_work",Some("M-001"),json!({"op":"import_commits","ref":"M-001/T-001","commits":[first],"actor":"lead"}),true).await;
        assert!(missing.contains("git_source") || missing.contains("unavailable"),"{missing}");
        assert_eq!(version(&cold,"M-001").await,old_version);
        cold.cancel().await.unwrap();
    }).await.expect("Bounded commit report import stdio cycle");
}

/// Contract links permit parallel work while explicit waits/cycles and frozen Epic roster stay truthful.
#[tokio::test]
async fn module_contract_readiness_dependencies_and_frozen_roster_stdio() {
    tokio::time::timeout(Duration::from_secs(60), async {
        let (_temp,root,_config,client)=fixture().await;
        change(&client,"plan_work",None,json!({"op":"create_epic","title":"Contract composition","outcome":"Provider and consumer fit","criteria":["Business composition delivers"]}),false).await;
        for title in ["Provider Module","Consumer Module"] {
            change(&client,"plan_work",None,json!({"op":"create_module","title":title,"outcome":"Declared responsibility delivers","criteria":["Boundary behavior fits"],"execution":execution(&root),"contracts":{"not_required":true},"lead":{"name":"lead"}}),false).await;
        }
        change(&client,"plan_work",Some("E-001"),json!({"op":"edit_epic","epic":"E-001","modules":["M-001","M-002"]}),false).await;
        let blocked_parent=change(&client,"record_work",Some("M-001"),json!({"op":"begin","ref":"M-001","actor":"lead"}),true).await;
        assert!(blocked_parent.contains("Epic") || blocked_parent.contains("E-001"),"{blocked_parent}");
        change(&client,"plan_work",Some("M-001"),json!({"op":"edit_module","module":"M-001","contracts":{"not_required":false,"provides":[{"peer":"M-002","description":"Stable payload","reference":"contracts/payload","ready":true}],"consumes":[{"peer":"M-002","description":"Processed response","ready":true}]}}),false).await;
        change(&client,"plan_work",Some("M-002"),json!({"op":"edit_module","module":"M-002","contracts":{"not_required":false,"provides":[{"peer":"M-001","description":"Processed response","ready":true}],"consumes":[{"peer":"M-001","description":"Stable payload","reference":"contracts/payload","ready":false}]}}),false).await;
        change(&client,"record_work",Some("E-001"),json!({"op":"begin","ref":"E-001"}),false).await;
        let not_ready=change(&client,"record_work",Some("M-002"),json!({"op":"begin","ref":"M-002","actor":"lead"}),true).await;
        assert!(not_ready.contains("contract") || not_ready.contains("ready"),"{not_ready}");
        change(&client,"plan_work",Some("M-002"),json!({"op":"edit_module","module":"M-002","contracts":{"not_required":false,"provides":[{"peer":"M-001","description":"Processed response","ready":true}],"consumes":[{"peer":"M-001","description":"Stable payload","reference":"contracts/payload","ready":true}]}}),false).await;
        change(&client,"record_work",Some("M-002"),json!({"op":"begin","ref":"M-002","actor":"lead"}),false).await;
        change(&client,"record_work",Some("M-001"),json!({"op":"begin","ref":"M-001","actor":"lead"}),false).await;
        change(&client,"plan_work",None,json!({"op":"create_module","title":"Later standalone Module","outcome":"Separate later work","criteria":["Standalone outcome"],"execution":execution(&root),"contracts":{"not_required":true},"lead":{"name":"later-lead"},"dependencies":[{"ref":"M-001","condition":"delivered","reason":"Needs delivered provider artifact"}]}),false).await;
        let frozen=change(&client,"plan_work",Some("E-001"),json!({"op":"edit_epic","epic":"E-001","modules":["M-001","M-002","M-003"]}),true).await;
        assert!(frozen.contains("frozen") || frozen.contains("roster"),"{frozen}");
        let pending=change(&client,"record_work",Some("M-003"),json!({"op":"begin","ref":"M-003","actor":"later-lead"}),true).await;
        assert!(pending.contains("M-001") && pending.contains("deliver"),"{pending}");
        let cycle=change(&client,"plan_work",Some("M-001"),json!({"op":"edit_module","module":"M-001","dependencies":[{"ref":"M-003","condition":"accepted","reason":"This would create a blocking cycle"}]}),true).await;
        assert!(cycle.contains("cycle") || cycle.contains("cyclic"),"{cycle}");
        let context=call(&client,"get_context",json!({"project":"product","ref":"M-002"}),false).await;
        assert!(context.contains("M-001") && context.contains("Stable payload") && context.contains("Processed response"),"{context}");
        let standalone=call(&client,"get_context",json!({"project":"product","ref":"M-003"}),false).await;
        assert!(standalone.contains("standalone") && standalone.contains("M-001") && standalone.contains("delivered"),"{standalone}");
        let status=call(&client,"project_status",json!({"project":"product"}),false).await;
        assert!(status.contains("3 readable") && status.contains("M-003"),"{status}");
        client.cancel().await.unwrap();
    }).await.expect("Bounded contract/start stdio cycle");
}
