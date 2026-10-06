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
async fn workflow1_stdio_cycle_and_cold_restart() {
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
        let created=legacy_change(&client,&root,"plan_work",None,json!({"project":"product","version":field(&context,"Allocation version: "),"op":"create_module","title":"File-backed workflow","outcome":"Work survives process restarts","criteria":["Work survives process restarts"],"execution":execution(&root),"contracts":{"not_required":true},"lead":{"name":"lead"},"tasks":[{"title":"Persist the report","required_checks":["restart"]}]}),false).await;
        assert!(created.starts_with("SAVED M-001"));
        let begun=legacy_change(&client,&root,"record_work",Some("M-001"),json!({"op":"begin","ref":"M-001","actor":"lead"}),false).await;let old=field(&begun,"Version: ");
        call(&client,"record_work",json!({"project":"product","version":old,"ref":"M-001/T-001","op":"result","state":"done","summary":"Report persisted to portable YAML","actor":"lead","checks":[{"label":"restart","status":"passed"}],"artifacts":["scenario: cold-restart"]}),false).await;
        let stale=call(&client,"record_work",json!({"project":"product","version":old,"ref":"M-001/T-001","op":"result","summary":"Stale replay must refuse"}),true).await;assert!(stale.contains("stale") && stale.contains("get_context"),"{stale}");
        let context=call(&client,"get_context",json!({"project":"product","ref":"M-001"}),false).await;
        call(&client,"review_module",json!({"project":"product","module":"M-001","version":field(&context,"Version: "),"verdict":"accepted","summary":"Reported native scenario independently checked","actor":"reviewer"}),false).await;
        let reviewed=call(&client,"get_context",json!({"project":"product","ref":"M-001"}),false).await;assert!(reviewed.contains("delivery pending"),"{reviewed}");
        legacy_change(&client,&root,"record_work",Some("M-001"),json!({"op":"deliver","ref":"M-001","target_branch":"main","summary":"Local delivery reported","actor":"lead"}),false).await;
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
async fn core_change(
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
async fn workflow1_epic_atomic_hierarchy_stdio_cycle() {
    tokio::time::timeout(Duration::from_secs(60), async {
        let (temp, root, config, client) = fixture().await;
        let epic = legacy_change(&client,&root, "plan_work", None, json!({"op":"create_epic","title":"Portable release","outcome":"Modules deliver together","criteria":["Both execution scopes deliver"],"required_checks":["epic-gate"],"lead":{"name":"orchestrator"}}), false).await;
        assert!(epic.starts_with("SAVED E-001"), "{epic}");
        let module = legacy_change(&client,&root, "plan_work", None, json!({"op":"create_module","title":"Epic implementation","outcome":"Durable hierarchy works","criteria":["Durable hierarchy works"],"execution":execution(&root),"contracts":{"not_required":true},"lead":{"name":"lead"},"tasks":[{"title":"Persist hierarchy","required_checks":["task-gate"]}]}), false).await;
        assert!(module.starts_with("SAVED M-001"), "{module}");
        legacy_change(&client,&root, "plan_work", Some("M-001"), json!({"op":"add_atomic","module":"M-001","title":"Module verification","outcome":"Embedded verification succeeds","required_checks":["module-atomic-gate"]}), false).await;
        legacy_change(&client,&root, "plan_work", None, json!({"op":"create_atomic","title":"Epic verification","outcome":"Epic scenario succeeds","executor":{"name":"verifier"},"participants":["M-001"],"environment":"disposable hierarchy scenario","scenarios":["Epic Module interaction"]}), false).await;
        legacy_change(&client,&root, "plan_work", None, json!({"op":"create_atomic","title":"Project microfix","outcome":"Tiny isolated correction is recorded","executor":{"name":"executor"}}), false).await;
        legacy_change(&client,&root, "plan_work", None, json!({"op":"create_module","title":"Standalone compatibility","outcome":"Standalone Modules remain valid","criteria":["Standalone delivery"],"execution":execution(&root),"contracts":{"not_required":true},"lead":{"name":"standalone-lead"}}), false).await;
        legacy_change(&client,&root, "plan_work", Some("E-001"), json!({"op":"edit_epic","epic":"E-001","modules":["M-001"],"atomics":["A-001"]}), false).await;
        legacy_change(&client,&root, "plan_work", None, json!({"op":"create_epic","title":"Separate Epic","outcome":"Membership remains exclusive","criteria":["No duplicate ownership"]}), false).await;
        let duplicate = legacy_change(&client,&root, "plan_work", Some("E-002"), json!({"op":"edit_epic","epic":"E-002","modules":["M-001"]}), true).await;
        assert!(duplicate.contains("already belongs") && duplicate.contains("Epic"), "{duplicate}");
        let dangling = legacy_change(&client,&root, "plan_work", Some("E-001"), json!({"op":"edit_epic","epic":"E-001","modules":["M-999"]}), true).await;
        assert!(dangling.contains("not_found") && dangling.contains("Module does not exist"), "{dangling}");
        legacy_change(&client,&root,"record_work",Some("E-001"),json!({"op":"begin","ref":"E-001","actor":"orchestrator"}),false).await;
        legacy_change(&client,&root,"record_work",Some("M-001"),json!({"op":"begin","ref":"M-001","actor":"lead"}),false).await;
        legacy_change(&client,&root,"record_work",Some("M-002"),json!({"op":"begin","ref":"M-002","actor":"standalone-lead"}),false).await;
        for (reference, owner, actor) in [("M-001/A-001","M-001","lead"),("A-002","A-002","executor")] {
            legacy_change(&client,&root,"record_work",Some(owner),json!({"op":"begin","ref":reference,"actor":actor}),false).await;
        }
        let module_context = call(&client, "get_context", json!({"project":"product","ref":"M-001"}), false).await;
        assert!(module_context.contains("E-001") && module_context.contains("Modules deliver together") && module_context.contains("Both execution scopes deliver"), "{module_context}");
        let refusal = legacy_change(&client,&root, "record_work", Some("E-001"), json!({"op":"cancel","ref":"E-001","reason":"Cannot implicitly cancel unfinished children"}), true).await;
        assert!(refusal.contains("M-001") || refusal.contains("children") || refusal.contains("member"), "{refusal}");
        legacy_change(&client,&root, "record_work", Some("M-001"), json!({"op":"result","ref":"M-001/T-001","state":"done","summary":"Hierarchy persisted through portable files","actor":"lead","checks":[{"label":"task-gate","status":"passed"}]}), false).await;
        legacy_change(&client,&root, "record_work", Some("M-001"), json!({"op":"result","ref":"M-001/A-001","state":"done","summary":"Embedded verification reported","checks":[{"label":"module-atomic-gate","status":"failed"}]}), false).await;
        let blocked = legacy_change(&client,&root, "review_module", Some("M-001"), json!({"module":"M-001","verdict":"accepted","summary":"Premature approval must refuse","actor":"reviewer"}), true).await;
        assert!(blocked.contains("module-atomic-gate") && blocked.contains("passed"), "{blocked}");
        legacy_change(&client,&root, "record_work", Some("M-001"), json!({"op":"result","ref":"M-001/A-001","state":"done","summary":"Embedded verification passed","checks":[{"label":"module-atomic-gate","status":"passed"}]}), false).await;
        legacy_change(&client,&root,"review_work",Some("M-001"),json!({"ref":"M-001/A-001","verdict":"accepted","summary":"Embedded Atomic independently checked","actor":"reviewer"}),false).await;
        let self_review = legacy_change(&client,&root, "review_work", Some("M-001"), json!({"ref":"M-001","verdict":"accepted","summary":"Lead cannot independently accept own Module","actor":"lead"}), true).await;
        assert!(self_review.contains("independent") || self_review.contains("lead") || self_review.contains("self"), "{self_review}");
        legacy_change(&client,&root, "review_module", Some("M-001"), json!({"module":"M-001","verdict":"accepted","summary":"Task and embedded Atomic independently verified","actor":"reviewer"}), false).await;
        legacy_change(&client,&root,"record_work",Some("M-001"),json!({"op":"deliver","ref":"M-001","target_branch":"main","summary":"Epic Module locally delivered","actor":"lead"}),false).await;
        let leaf_review = legacy_change(&client,&root, "review_work", Some("M-001"), json!({"ref":"M-001/T-001","verdict":"accepted","summary":"Tasks do not have independent review","actor":"reviewer"}), true).await;
        assert!(leaf_review.contains("invalid") || leaf_review.contains("review"), "{leaf_review}");
        legacy_change(&client,&root,"record_work",Some("A-001"),json!({"op":"begin","ref":"A-001","actor":"verifier"}),false).await;
        legacy_change(&client,&root, "record_work", Some("A-001"), json!({"op":"result","ref":"A-001","state":"done","summary":"Epic scoped scenario passed","artifacts":["scenario: portable hierarchy"]}), false).await;
        legacy_change(&client,&root, "record_work", Some("A-002"), json!({"op":"result","ref":"A-002","state":"done","summary":"Project microfix locally completed","actor":"executor"}), false).await;
        for reference in ["A-001","A-002"] {
            legacy_change(&client,&root,"review_work",Some(reference),json!({"ref":reference,"verdict":"accepted","summary":"Independent Atomic acceptance","actor":"reviewer"}),false).await;
        }
        legacy_change(&client,&root, "record_work", Some("M-002"), json!({"op":"result","ref":"M-002","summary":"Standalone Module delivered without Tasks","actor":"standalone-lead"}), false).await;
        legacy_change(&client,&root, "review_work", Some("M-002"), json!({"ref":"M-002","verdict":"accepted","summary":"Standalone delivery independently checked","actor":"reviewer"}), false).await;
        legacy_change(&client,&root,"record_work",Some("M-002"),json!({"op":"deliver","ref":"M-002","target_branch":"main","summary":"Standalone Module locally delivered","actor":"standalone-lead"}),false).await;
        legacy_change(&client,&root, "record_work", Some("E-001"), json!({"op":"result","ref":"E-001","summary":"Epic delivery evidence assembled","checks":[{"label":"epic-gate","status":"not_applicable"}]}), false).await;
        let blocked = legacy_change(&client,&root, "review_work", Some("E-001"), json!({"ref":"E-001","verdict":"accepted","summary":"Required check cannot be waived","actor":"epic-reviewer"}), true).await;
        assert!(blocked.contains("epic-gate") && blocked.contains("passed"), "{blocked}");
        legacy_change(&client,&root, "record_work", Some("E-001"), json!({"op":"result","ref":"E-001","summary":"Epic delivery evidence assembled","checks":[{"label":"epic-gate","status":"passed"}]}), false).await;
        legacy_change(&client,&root, "review_work", Some("E-001"), json!({"ref":"E-001","verdict":"accepted","summary":"Whole Epic independently accepted","actor":"epic-reviewer"}), false).await;
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
async fn workflow1_integration_atomic_freshness_and_lifecycle_stdio() {
    tokio::time::timeout(Duration::from_secs(60), async {
        let (temp, root, config, client) = fixture().await;
        legacy_change(&client,&root, "plan_work", None, json!({"op":"create_module","title":"Integration participant","outcome":"Participant works","criteria":["Participant delivers"],"execution":execution(&root),"contracts":{"not_required":true},"lead":{"name":"lead"},"tasks":[{"title":"Participant task"}]}), false).await;
        legacy_change(&client,&root,"record_work",Some("M-001"),json!({"op":"begin","ref":"M-001","actor":"lead"}),false).await;
        legacy_change(&client,&root, "record_work", Some("M-001"), json!({"op":"result","ref":"M-001/T-001","state":"done","summary":"Participant delivers initial behavior","actor":"lead"}), false).await;
        legacy_change(&client,&root, "review_module", Some("M-001"), json!({"module":"M-001","verdict":"accepted","summary":"Participant independently accepted","actor":"reviewer"}), false).await;
        legacy_change(&client,&root,"record_work",Some("M-001"),json!({"op":"deliver","ref":"M-001","target_branch":"main","summary":"Participant locally delivered","actor":"lead"}),false).await;
        legacy_change(&client,&root, "plan_work", None, json!({"op":"create_epic","title":"Integration Epic","outcome":"Participating Module works together","criteria":["Current integration verification"]}), false).await;
        legacy_change(&client,&root, "plan_work", None, json!({"op":"create_atomic","title":"Integration scenario","outcome":"Native cross-module scenario passes","executor":{"name":"integrator"},"environment":"disposable native fixture","scenarios":["Participant interaction"],"participants":["M-001"],"required_checks":["integration"]}), false).await;
        legacy_change(&client,&root, "plan_work", Some("E-001"), json!({"op":"edit_epic","epic":"E-001","modules":["M-001"],"atomics":["A-001"]}), false).await;
        legacy_change(&client,&root,"record_work",Some("E-001"),json!({"op":"begin","ref":"E-001"}),false).await;
        legacy_change(&client,&root,"record_work",Some("A-001"),json!({"op":"begin","ref":"A-001","actor":"integrator"}),false).await;
        let failed = legacy_change(&client,&root,"record_work",Some("A-001"),json!({"op":"result","ref":"A-001","state":"done","summary":"Scenario failed","checks":[{"label":"integration","status":"failed"}]}),false).await;
        assert!(failed.starts_with("SAVED A-001"),"{failed}");
        let failed_review=legacy_change(&client,&root,"review_work",Some("A-001"),json!({"ref":"A-001","verdict":"accepted","summary":"Failed required check cannot be accepted","actor":"reviewer"}),true).await;
        assert!(failed_review.contains("integration") && failed_review.contains("passed"),"{failed_review}");
        legacy_change(&client,&root, "record_work", Some("A-001"), json!({"op":"result","ref":"A-001","state":"done","summary":"Scenario initial execution passed","checks":[{"label":"integration","status":"passed","detail":"Scenario: member interaction"}],"artifacts":["scenario: initial native execution"]}), false).await;
        legacy_change(&client,&root,"review_work",Some("A-001"),json!({"ref":"A-001","verdict":"accepted","summary":"Integration independently accepted","actor":"reviewer"}),false).await;
        legacy_change(&client,&root, "record_work", Some("E-001"), json!({"op":"result","ref":"E-001","summary":"Current participant and integration accepted"}), false).await;
        legacy_change(&client,&root, "review_work", Some("E-001"), json!({"ref":"E-001","verdict":"accepted","summary":"Independent Epic integration acceptance","actor":"epic-reviewer"}), false).await;
        let before = call(&client, "get_context", json!({"project":"product","ref":"A-001"}), false).await;
        assert!(before.contains("accepted") && before.contains("M-001"), "{before}");
        let search_before = call(&client, "search", json!({"project":"product","query":"participant","limit":1}), false).await;
        legacy_change(&client,&root, "record_work", Some("M-001"), json!({"op":"reopen","ref":"M-001/T-001","reason":"Participant behavior needs correction"}), false).await;
        let stale_write = call(&client, "record_work", json!({"project":"product","version":field(&before,"Version: "),"op":"result","ref":"A-001","state":"done","summary":"Old integration version cannot reaffirm changed members","checks":[{"label":"integration","status":"passed"}]}), true).await;
        assert!(stale_write.contains("stale") && stale_write.contains("get_context"), "{stale_write}");
        let stale_page = call(&client, "search", json!({"project":"product","query":"participant","limit":1,"start":1,"version":field(&search_before,"Snapshot version: ")}), true).await;
        assert!(stale_page.contains("stale") && stale_page.contains("snapshot"), "{stale_page}");
        let stale_atomic = call(&client, "get_context", json!({"project":"product","ref":"A-001"}), false).await;
        assert!(stale_atomic.contains("stale") && stale_atomic.contains("M-001"), "{stale_atomic}");
        let stale_epic = call(&client, "get_context", json!({"project":"product","ref":"E-001"}), false).await;
        assert!(stale_epic.contains("stale"), "{stale_epic}");
        let blocked = legacy_change(&client,&root, "review_work", Some("E-001"), json!({"ref":"E-001","verdict":"accepted","summary":"Cannot accept stale integration","actor":"epic-reviewer"}), true).await;
        assert!(blocked.contains("M-001") || blocked.contains("A-001"), "{blocked}");
        legacy_change(&client,&root, "record_work", Some("M-001"), json!({"op":"result","ref":"M-001/T-001","state":"done","summary":"Participant corrected behavior delivered","actor":"lead"}), false).await;
        legacy_change(&client,&root, "review_module", Some("M-001"), json!({"module":"M-001","verdict":"accepted","summary":"Corrected participant independently accepted","actor":"reviewer"}), false).await;
        legacy_change(&client,&root,"record_work",Some("M-001"),json!({"op":"deliver","ref":"M-001","target_branch":"main","summary":"Participant locally delivered","actor":"lead"}),false).await;
        legacy_change(&client,&root, "record_work", Some("A-001"), json!({"op":"result","ref":"A-001","state":"done","summary":"Scenario rerun against current participant passed","checks":[{"label":"integration","status":"passed"}]}), false).await;
        legacy_change(&client,&root,"review_work",Some("A-001"),json!({"ref":"A-001","verdict":"accepted","summary":"Integration independently accepted","actor":"reviewer"}),false).await;
        legacy_change(&client,&root, "review_work", Some("E-001"), json!({"ref":"E-001","verdict":"accepted","summary":"Current integration independently accepted","actor":"epic-reviewer"}), false).await;
        legacy_change(&client,&root, "record_work", Some("E-001"), json!({"op":"cancel","ref":"E-001","reason":"Completed scope intentionally retired"}), false).await;
        let member = call(&client, "get_context", json!({"project":"product","ref":"M-001"}), false).await;
        assert!(member.contains("Module phase: accepted"), "{member}");
        legacy_change(&client,&root, "record_work", Some("E-001"), json!({"op":"reopen","ref":"E-001","reason":"Same Epic receives followup acceptance"}), false).await;
        let epic = call(&client, "get_context", json!({"project":"product","ref":"E-001"}), false).await;
        assert!(epic.contains("stale"), "{epic}");
        let still_frozen = legacy_change(&client,&root,"plan_work",Some("E-001"),json!({"op":"edit_epic","epic":"E-001","modules":[]}),true).await;
        assert!(still_frozen.contains("frozen") || still_frozen.contains("roster"),"{still_frozen}");
        legacy_change(&client,&root, "record_work", Some("A-001"), json!({"op":"cancel","ref":"A-001","reason":"Integration is explicitly deferred"}), false).await;
        legacy_change(&client,&root, "record_work", Some("A-001"), json!({"op":"reopen","ref":"A-001","reason":"Integration will be rerun"}), false).await;
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
async fn workflow1_atomic_only_module_compact_stdio_regression() {
    tokio::time::timeout(Duration::from_secs(60), async {
        let (_temp, root, _config, client) = fixture().await;
        legacy_change(&client,&root, "plan_work", None, json!({"op":"create_module","title":"Atomic-only delivery","outcome":"Five atomic scenarios deliver","criteria":["Five scenarios deliver"],"execution":execution(&root),"contracts":{"not_required":true},"lead":{"name":"module-lead","handle":"session: module-lead"}}), false).await;
        legacy_change(&client,&root,"record_work",Some("M-001"),json!({"op":"begin","ref":"M-001","actor":"module-lead"}),false).await;
        for index in 1..=5 {
            let checks = if index == 1 { vec!["atomic-native"] } else { Vec::new() };
            legacy_change(&client,&root, "plan_work", Some("M-001"), json!({"op":"add_atomic","module":"M-001","title":format!("Atomic scenario {index}"),"outcome":format!("Atomic scenario {index} passes"),"required_checks":checks,"executor":{"name":"atomic-executor","handle":"session: atomic-executor"}}), false).await;
            let reference = format!("M-001/A-{index:03}");
            legacy_change(&client,&root,"record_work",Some("M-001"),json!({"op":"begin","ref":reference,"actor":"atomic-executor"}),false).await;
            let checks = if index == 1 { json!([{"label":"atomic-native","status":"failed","detail":"Scenario: atomic-first execution"}]) } else { json!([]) };
            legacy_change(&client,&root, "record_work", Some("M-001"), json!({"op":"result","ref":reference,"state":"done","summary":format!("Unique atomic evidence {index}"),"actor":"atomic-executor","checks":checks,"artifacts":[format!("atomic-artifact-{index}")]}), false).await;
        }
        let compact = call(&client, "project_status", json!({"project":"product"}), false).await;
        assert!(compact.contains("Tasks: 0 done / 0 readable") && compact.contains("Atomic local outcomes: 5 done; final current closure: 0") && compact.contains("Atomics: 0 done / 5 readable"), "{compact}");
        assert!(compact.contains("PARTIAL") && compact.lines().any(|line| line.starts_with("M-001: 1 ") && line.contains("omitted")), "{compact}");
        assert!(compact.contains("atomic-executor") && compact.contains("session: atomic-executor"), "{compact}");
        let child = call(&client, "get_context", json!({"project":"product","ref":"M-001/A-001"}), false).await;
        assert!(child.contains("atomic-executor") && child.contains("session: atomic-executor"), "{child}");
        legacy_change(&client,&root, "record_work", Some("M-001"), json!({"op":"cancel","ref":"M-001/A-001","reason":"First verification explicitly deferred"}), false).await;
        let canceled = call(&client, "get_context", json!({"project":"product","ref":"M-001/A-001"}), false).await;
        assert!(canceled.contains("canceled") && canceled.contains("First verification explicitly deferred"), "{canceled}");
        legacy_change(&client,&root, "record_work", Some("M-001"), json!({"op":"reopen","ref":"M-001/A-001","reason":"First verification scheduled again"}), false).await;
        let reopened = call(&client, "get_context", json!({"project":"product","ref":"M-001/A-001"}), false).await;
        assert!(reopened.contains("open"), "{reopened}");
        legacy_change(&client,&root,"record_work",Some("M-001"),json!({"op":"begin","ref":"M-001/A-001","actor":"atomic-executor"}),false).await;
        legacy_change(&client,&root, "record_work", Some("M-001"), json!({"op":"result","ref":"M-001/A-001","state":"done","summary":"Unique atomic evidence 1 refreshed","actor":"atomic-executor","checks":[{"label":"atomic-native","status":"failed","detail":"Scenario: atomic-first execution"}],"artifacts":["atomic-artifact-1"]}), false).await;
        legacy_change(&client,&root, "record_work", Some("M-001"), json!({"op":"result","ref":"M-001","summary":"Atomic-only Module delivered all scenarios","actor":"module-lead"}), false).await;
        for index in 1..=5 {
            let reference=format!("M-001/A-{index:03}");
            let checks=if index==1 {json!([{ "target":"M-001/A-001","label":"atomic-native","status":"passed","detail":"Scenario independently rerun"}])} else {json!([])};
            legacy_change(&client,&root,"review_work",Some("M-001"),json!({"ref":reference,"verdict":"accepted","summary":"Atomic independently accepted","actor":"reviewer","checks":checks}),false).await;
        }
        legacy_change(&client,&root, "review_module", Some("M-001"), json!({"module":"M-001","verdict":"accepted","summary":"Independent check verifies Atomic-only Module","actor":"reviewer","checks":[{"target":"M-001/A-001","label":"atomic-native","status":"passed","detail":"Scenario independently rerun"}]}), false).await;
        legacy_change(&client,&root,"record_work",Some("M-001"),json!({"op":"deliver","ref":"M-001","target_branch":"main","summary":"Atomic-only delivery reported","actor":"module-lead"}),false).await;
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
async fn workflow1_git_reports_import_once_stdio_and_retained_source_restart() {
    tokio::time::timeout(Duration::from_secs(60), async {
        let (temp, root, config, client) = fixture().await;
        let code = temp.path().join("code-source");
        let message = "feat: first report\n\nResult: Imported first behavior\nChecks:\npassed | inspection | Author reports manual inspection\nFollowups:\n- Optional future scenario\n";
        let first = commit_fixture(&code, message, "/// Disposable fixture function.\npub fn value() -> u8 { 1 }\n");
        let second = commit_fixture(&code, "fix: second report\n\nResult: Imported second behavior\nChecks:\nnot_run | inspection | Author did not run an automated test\n", "/// Disposable fixture function.\npub fn value() -> u8 { 2 }\n");
        legacy_change(&client,&root,"plan_work",None,json!({"op":"create_module","title":"Code source import","outcome":"Import reports without copying","criteria":["Imported source retained"],"execution":execution(&code),"contracts":{"not_required":true},"lead":{"name":"lead"},"tasks":[{"title":"Import code report"}]}),false).await;
        legacy_change(&client,&root,"record_work",Some("M-001"),json!({"op":"begin","ref":"M-001","actor":"lead"}),false).await;
        let stale = version(&client,"M-001").await;
        legacy_change(&client,&root,"record_work",Some("M-001"),json!({"op":"import_commits","ref":"M-001/T-001","commits":[first,second],"actor":"lead"}),false).await;
        let task = call(&client,"get_context",json!({"project":"product","ref":"M-001/T-001"}),false).await;
        assert!(task.contains("open"),"{task}");
        let results = call(&client,"get_context",json!({"project":"product","ref":"M-001/T-001","view":"results"}),false).await;
        assert!(results.contains("Imported first behavior") && results.contains("Imported second behavior"),"{results}");
        let current = version(&client,"M-001").await;
        legacy_change(&client,&root,"record_work",Some("M-001"),json!({"op":"import_commits","ref":"M-001/T-001","commits":[first,second],"actor":"lead"}),false).await;
        assert_eq!(version(&client,"M-001").await,current);
        let refused = call(&client,"record_work",json!({"project":"product","version":stale,"op":"import_commits","ref":"M-001/T-001","commits":[first],"actor":"lead"}),true).await;
        assert!(refused.contains("stale"),"{refused}");
        let foreign = legacy_change(&client,&root,"record_work",Some("M-001"),json!({"op":"complete","ref":"M-001/T-001","actor":"other-agent"}),true).await;
        assert!(foreign.contains("lead"),"{foreign}");
        legacy_change(&client,&root,"record_work",Some("M-001"),json!({"op":"complete","ref":"M-001/T-001","actor":"lead"}),false).await;
        let task = call(&client,"get_context",json!({"project":"product","ref":"M-001/T-001"}),false).await;
        assert!(task.contains("done"),"{task}");
        let self_review = legacy_change(&client,&root,"review_work",Some("M-001"),json!({"ref":"M-001","verdict":"accepted","summary":"Imported report author cannot independently review","actor":"lead"}),true).await;
        assert!(self_review.contains("independent") || self_review.contains("lead") || self_review.contains("self"),"{self_review}");
        legacy_change(&client,&root,"plan_work",Some("M-001"),json!({"op":"add_task","module":"M-001","title":"Second source consumer"}),false).await;
        legacy_change(&client,&root,"record_work",Some("M-001"),json!({"op":"import_commits","ref":"M-001/T-002","commits":[first],"actor":"lead"}),false).await;

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
        let missing = legacy_change(&cold,&root,"record_work",Some("M-001"),json!({"op":"import_commits","ref":"M-001/T-001","commits":[first],"actor":"lead"}),true).await;
        assert!(missing.contains("git_source") || missing.contains("unavailable"),"{missing}");
        assert_eq!(version(&cold,"M-001").await,old_version);
        cold.cancel().await.unwrap();
    }).await.expect("Bounded commit report import stdio cycle");
}

/// Contract links permit parallel work while explicit waits/cycles and frozen Epic roster stay truthful.
#[tokio::test]
async fn workflow1_module_contract_readiness_dependencies_and_frozen_roster_stdio() {
    tokio::time::timeout(Duration::from_secs(60), async {
        let (_temp,root,_config,client)=fixture().await;
        legacy_change(&client,&root,"plan_work",None,json!({"op":"create_epic","title":"Contract composition","outcome":"Provider and consumer fit","criteria":["Business composition delivers"]}),false).await;
        for title in ["Provider Module","Consumer Module"] {
            legacy_change(&client,&root,"plan_work",None,json!({"op":"create_module","title":title,"outcome":"Declared responsibility delivers","criteria":["Boundary behavior fits"],"execution":execution(&root),"contracts":{"not_required":true},"lead":{"name":"lead"}}),false).await;
        }
        legacy_change(&client,&root,"plan_work",Some("E-001"),json!({"op":"edit_epic","epic":"E-001","modules":["M-001","M-002"]}),false).await;
        let blocked_parent=legacy_change(&client,&root,"record_work",Some("M-001"),json!({"op":"begin","ref":"M-001","actor":"lead"}),true).await;
        assert!(blocked_parent.contains("Epic") || blocked_parent.contains("E-001"),"{blocked_parent}");
        legacy_change(&client,&root,"plan_work",Some("M-001"),json!({"op":"edit_module","module":"M-001","contracts":{"not_required":false,"provides":[{"peer":"M-002","description":"Stable payload","reference":"contracts/payload","ready":true}],"consumes":[{"peer":"M-002","description":"Processed response","ready":true}]}}),false).await;
        legacy_change(&client,&root,"plan_work",Some("M-002"),json!({"op":"edit_module","module":"M-002","contracts":{"not_required":false,"provides":[{"peer":"M-001","description":"Processed response","ready":true}],"consumes":[{"peer":"M-001","description":"Stable payload","reference":"contracts/payload","ready":false}]}}),false).await;
        legacy_change(&client,&root,"record_work",Some("E-001"),json!({"op":"begin","ref":"E-001"}),false).await;
        let not_ready=legacy_change(&client,&root,"record_work",Some("M-002"),json!({"op":"begin","ref":"M-002","actor":"lead"}),true).await;
        assert!(not_ready.contains("contract") || not_ready.contains("ready"),"{not_ready}");
        legacy_change(&client,&root,"plan_work",Some("M-002"),json!({"op":"edit_module","module":"M-002","contracts":{"not_required":false,"provides":[{"peer":"M-001","description":"Processed response","ready":true}],"consumes":[{"peer":"M-001","description":"Stable payload","reference":"contracts/payload","ready":true}]}}),false).await;
        legacy_change(&client,&root,"record_work",Some("M-002"),json!({"op":"begin","ref":"M-002","actor":"lead"}),false).await;
        legacy_change(&client,&root,"record_work",Some("M-001"),json!({"op":"begin","ref":"M-001","actor":"lead"}),false).await;
        legacy_change(&client,&root,"plan_work",None,json!({"op":"create_module","title":"Later standalone Module","outcome":"Separate later work","criteria":["Standalone outcome"],"execution":execution(&root),"contracts":{"not_required":true},"lead":{"name":"later-lead"},"dependencies":[{"ref":"M-001","condition":"delivered","reason":"Needs delivered provider artifact"}]}),false).await;
        let frozen=legacy_change(&client,&root,"plan_work",Some("E-001"),json!({"op":"edit_epic","epic":"E-001","modules":["M-001","M-002","M-003"]}),true).await;
        assert!(frozen.contains("frozen") || frozen.contains("roster"),"{frozen}");
        let pending=legacy_change(&client,&root,"record_work",Some("M-003"),json!({"op":"begin","ref":"M-003","actor":"later-lead"}),true).await;
        assert!(pending.contains("M-001") && pending.contains("deliver"),"{pending}");
        let cycle=legacy_change(&client,&root,"plan_work",Some("M-001"),json!({"op":"edit_module","module":"M-001","dependencies":[{"ref":"M-003","condition":"accepted","reason":"This would create a blocking cycle"}]}),true).await;
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

/// Deterministic public boundary used solely in disposable fixture source directories.
const BOUNDARY_PROGRAM: &str = r#"/// Map valid fixture inputs to one greater value; invalid inputs are rejected.
fn produce(input: i32) -> Option<i32> {
    if (0..=10).contains(&input) { Some(input + 1) } else { None }
}
/// Expose the fixture mapping as a tiny command with explicit invalid-input exit status.
fn main() {
    let input: i32 = std::env::args().nth(1).expect("fixture input").parse().expect("integer input");
    match produce(input) {
        Some(output) => println!("{output}"),
        None => { eprintln!("invalid input"); std::process::exit(2); }
    }
}
"#;

/// Compile only the supplied disposable fixture source; preserve unrelated files and return its binary.
fn compile_boundary_program(root: &std::path::Path, source: &str) -> std::path::PathBuf {
    std::fs::create_dir_all(root).unwrap();
    std::fs::write(root.join("lib.rs"), source).unwrap();
    let binary = root.join("boundary-program");
    let output = std::process::Command::new("rustc")
        .current_dir(root)
        .args([
            "--edition",
            "2024",
            "--crate-name",
            "fixture_boundary",
            "lib.rs",
            "-o",
        ])
        .arg(&binary)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "Disposable fixture compilation failed"
    );
    binary
}

/// Apply the identical valid/invalid public-boundary checks to correct and deliberately faulty code.
/// Return actual pass/fail and observed output; no assertion or setup is mutated between executions.
fn boundary_check(binary: &std::path::Path) -> (bool, String) {
    let valid = std::process::Command::new(binary)
        .arg("1")
        .output()
        .unwrap();
    let invalid = std::process::Command::new(binary)
        .arg("-1")
        .output()
        .unwrap();
    let output = String::from_utf8(valid.stdout).unwrap();
    let error = String::from_utf8(invalid.stderr).unwrap();
    let passed = valid.status.success()
        && output.trim() == "2"
        && invalid.status.code() == Some(2)
        && error.trim() == "invalid input";
    (
        passed,
        format!(
            "input1=>{}; invalid-1=>exit{:?}:{}",
            output.trim(),
            invalid.status.code(),
            error.trim()
        ),
    )
}

/// Observe real correct/mutant/restored executions without inventing a verification commit.
/// The fault changes the implementation's output mapping; another actor's checkout is never touched.
fn observed_boundary_cycle(root: &std::path::Path) -> Value {
    std::fs::create_dir_all(root).unwrap();
    let sentinel = root.join("unrelated-sentinel");
    std::fs::write(&sentinel, "preserve unrelated fixture state").unwrap();
    let correct_binary = compile_boundary_program(root, BOUNDARY_PROGRAM);
    let (correct_passed, correct) = boundary_check(&correct_binary);
    assert!(correct_passed, "{correct}");
    let mutant = BOUNDARY_PROGRAM.replace("Some(input + 1)", "Some(input + 2)");
    let mutant_binary = compile_boundary_program(root, &mutant);
    let (mutant_passed, failed) = boundary_check(&mutant_binary);
    assert!(
        !mutant_passed && failed.contains("input1=>3"),
        "Wrong output mapping must fail the same boundary check: {failed}"
    );
    let restored_binary = compile_boundary_program(root, BOUNDARY_PROGRAM);
    let (restored_passed, restored) = boundary_check(&restored_binary);
    assert!(restored_passed, "{restored}");
    assert_eq!(
        std::fs::read_to_string(root.join("lib.rs")).unwrap(),
        BOUNDARY_PROGRAM
    );
    assert_eq!(
        std::fs::read_to_string(sentinel).unwrap(),
        "preserve unrelated fixture state"
    );
    json!({"conditions":"Inputs1 and-1; isolated source state; no clock, randomness or external services","correct":{"status":"passed","detail":correct},"mutation":"Changed only implementation output from input+1 to input+2","failed":{"status":"failed","detail":failed},"restored":{"status":"passed","detail":restored},"artifacts":[restored_binary]})
}

/// Commit the first restored code candidate after actual boundary/mutation/control observations.
fn observed_boundary_fixture(root: &std::path::Path) -> Value {
    let mut observations = observed_boundary_cycle(root);
    let candidate = commit_fixture(
        root,
        "test: fixture boundary\n\nResult: Correct public mapping verified; deliberate wrong output failed; restored control passed\nChecks:\npassed | boundary | Inputs1/-1 correct and restored; meaningful wrong-output mutant detected\n",
        BOUNDARY_PROGRAM,
    );
    observations["candidate"] = json!(candidate);
    observations
}

/// Preserve workflow-1 test records by removing only optional core metadata in isolated fixture setup.
/// Initial Tasks are added through old-compatible public operations after this explicit old-record setup.
async fn legacy_change(
    client: &RunningService<RoleClient, ()>,
    root: &std::path::Path,
    name: &str,
    owner: Option<&str>,
    mut args: Value,
    error: bool,
) -> String {
    let op = args["op"].as_str().unwrap_or_default().to_owned();
    let creation = name == "plan_work"
        && matches!(
            op.as_str(),
            "create_module" | "create_epic" | "create_atomic"
        );
    if !creation {
        return core_change(client, name, owner, args, error).await;
    }
    let tasks = args
        .as_object_mut()
        .unwrap()
        .remove("tasks")
        .unwrap_or_else(|| json!([]));
    let saved = core_change(client, name, owner, args, error).await;
    if error {
        return saved;
    }
    let reference = saved
        .lines()
        .next()
        .unwrap()
        .strip_prefix("SAVED ")
        .unwrap()
        .to_owned();
    let directory = match reference.as_bytes()[0] {
        b'M' => "modules",
        b'E' => "epics",
        b'A' => "atomics",
        _ => panic!("Fixture creation returned invalid ref"),
    };
    let path = root.join(directory).join(format!("{reference}.yaml"));
    let mut record: serde_yaml_ng::Value =
        serde_yaml_ng::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    if let Some(workflow) = record["workflow"].as_mapping_mut() {
        workflow.remove(serde_yaml_ng::Value::String("core".into()));
    }
    std::fs::write(path, serde_yaml_ng::to_string(&record).unwrap()).unwrap();
    for task in tasks.as_array().unwrap() {
        let mut task = task.clone();
        let fields = task.as_object_mut().unwrap();
        fields.insert("op".into(), json!("add_task"));
        fields.insert("module".into(), json!(reference));
        core_change(client, "plan_work", Some(&reference), task, false).await;
    }
    let current = version(client, &reference).await;
    let old = field(&saved, "Version: ");
    saved.replace(&format!("Version: {old}"), &format!("Version: {current}"))
}

/// Create a provisional new-core Module without inventing Tasks or a runtime session before launch.
async fn create_core_module(
    client: &RunningService<RoleClient, ()>,
    source: &std::path::Path,
    title: &str,
) -> String {
    std::fs::create_dir_all(source).unwrap();
    std::fs::write(source.join("lib.rs"), BOUNDARY_PROGRAM).unwrap();
    let saved = core_change(client,"plan_work",None,json!({"op":"create_module","title":title,"outcome":"Known fixture responsibility delivers","criteria":["Public fixture behavior fits"],"execution":execution(source),"contracts":{"not_required":true}}),false).await;
    saved
        .lines()
        .next()
        .unwrap()
        .strip_prefix("SAVED ")
        .unwrap()
        .to_owned()
}

/// Model only durable binding gates with explicitly synthetic receipt data; this proves no live launch.
async fn bind_fixture_agent(
    client: &RunningService<RoleClient, ()>,
    owner: &str,
    reference: &str,
    role: &str,
    id: &str,
) -> String {
    core_change(client,"record_work",Some(owner),json!({"op":"bind_agent","ref":reference,"role":role,"harness":"fixture-only","agent_id":id,"communication_ref":format!("fixture-only: communicate {id}"),"resume_ref":format!("fixture-only: resume {id}"),"launch_ref":format!("fixture-only: observed test receipt {id}")}),false).await
}

/// Record current code/discovery scope through the bound synthetic lead before any Task planning.
async fn plan_fixture_module(client: &RunningService<RoleClient, ()>, module: &str, lead: &str) {
    core_change(client,"record_work",Some(module),json!({"op":"planning","ref":module,"actor":lead,"responsibility":"Own the public fixture mapping","scope":"Inspect isolated lib.rs and its input/output boundary","exclusions":["Other Module source state"],"read_refs":["lib.rs"],"uncertainties":[]}),false).await;
}

/// Declare one canonical provider/consumer boundary; legacy ready metadata is not a confirmation.
fn fixture_contract(id: &str, revision: u64, peer: &str, reference: &std::path::Path) -> Value {
    json!({"id":id,"revision":revision,"peer":peer,"description":"Valid numeric mapping; invalid input rejected","reference":reference,"ready":true})
}

/// Confirm a canonical revision through the current recorded synthetic lead identity.
async fn agree_fixture_contract(
    client: &RunningService<RoleClient, ()>,
    module: &str,
    lead: &str,
    id: &str,
    revision: u64,
) {
    core_change(client,"record_work",Some(module),json!({"op":"agree_contract","ref":module,"actor":lead,"contract_id":id,"revision":revision,"summary":"Fixture party checked the exact shared definition"}),false).await;
}

/// Project observed boundary data into the purpose-based MCP action for one declared contract scope.
async fn record_fixture_boundary(
    client: &RunningService<RoleClient, ()>,
    module: &str,
    lead: &str,
    id: &str,
    revision: u64,
    observation: &Value,
) {
    let mut args = observation.clone();
    let fields = args.as_object_mut().unwrap();
    fields.insert("op".into(), json!("boundary_evidence"));
    fields.insert("ref".into(), json!(module));
    fields.insert("actor".into(), json!(lead));
    fields.insert("contract_id".into(), json!(id));
    fields.insert("revision".into(), json!(revision));
    core_change(client, "record_work", Some(module), args, false).await;
}

/// Bind, discover and optionally Task-plan a Module using explicit synthetic protocol-only receipts.
async fn prepare_core_lead(
    client: &RunningService<RoleClient, ()>,
    module: &str,
    lead: &str,
    task: bool,
) {
    bind_fixture_agent(client, module, module, "lead", lead).await;
    plan_fixture_module(client, module, lead).await;
    if task {
        core_change(client,"plan_work",Some(module),json!({"op":"add_task","module":module,"actor":lead,"title":"Implement the public fixture boundary"}),false).await;
    }
}

/// Discovery precedes exact party agreement/freeze; lead ownership and genuine waits remain separate.
#[tokio::test]
async fn epic_core_lead_discovery_agreement_freeze_stdio() {
    tokio::time::timeout(Duration::from_secs(60),async {
        let (temp,_root,config,client)=fixture().await;
        core_change(&client,"plan_work",None,json!({"op":"create_epic","title":"Lead discovery Epic","outcome":"Agreed provider and consumer","criteria":["Joint mapping"]}),false).await;
        let a=create_core_module(&client,&temp.path().join("provider-source"),"Provider discovery").await;
        let b=create_core_module(&client,&temp.path().join("consumer-source"),"Consumer discovery").await;
        core_change(&client,"plan_work",Some("E-001"),json!({"op":"edit_epic","epic":"E-001","modules":[a,b],"criterion_scopes":[{"index":0,"text":"Joint mapping","modules":[a,b]}]}),false).await;
        core_change(&client,"record_work",Some("E-001"),json!({"op":"begin","ref":"E-001"}),false).await;
        let before=call(&client,"get_context",json!({"project":"product","ref":a}),false).await;
        assert!(before.contains("Provider discovery") && before.contains("Lead discovery Epic"),"{before}");
        let unbound=core_change(&client,"plan_work",Some(&a),json!({"op":"add_task","module":a,"actor":"fixture-lead-a","title":"Unbound planning must refuse"}),true).await;
        assert!(unbound.contains("bind") || unbound.contains("lead"),"{unbound}");
        prepare_core_lead(&client,&a,"fixture-lead-a",true).await;
        prepare_core_lead(&client,&b,"fixture-lead-b",true).await;
        let version_before=version(&client,&a).await;
        bind_fixture_agent(&client,&a,&a,"lead","fixture-lead-a").await;
        assert_eq!(version(&client,&a).await,version_before,"Same observed binding must be idempotent");
        let wrong_lead=core_change(&client,"plan_work",Some(&a),json!({"op":"add_task","module":a,"actor":"fixture-lead-b","title":"Wrong lead Task"}),true).await;
        assert!(wrong_lead.contains("lead") || wrong_lead.contains("actor"),"{wrong_lead}");
        let artifact=temp.path().join("payload-contract.md");
        std::fs::write(&artifact,"Revision1: valid numeric mapping and explicit invalid-input rejection\n").unwrap();
        core_change(&client,"plan_work",Some(&a),json!({"op":"edit_module","module":a,"contracts":{"not_required":false,"provides":[fixture_contract("payload-ab",1,&b,&artifact)],"consumes":[]}}),false).await;
        core_change(&client,"plan_work",Some(&b),json!({"op":"edit_module","module":b,"contracts":{"not_required":false,"provides":[],"consumes":[fixture_contract("payload-ab",1,&a,&artifact)]},"dependencies":[{"ref":a,"condition":"accepted","reason":"A concrete reviewed provider artifact is needed"}]}),false).await;
        let unilateral=core_change(&client,"plan_work",Some("E-001"),json!({"op":"freeze_epic","epic":"E-001"}),true).await;
        assert!(unilateral.contains("agree") || unilateral.contains("confirm") || unilateral.contains("contract"),"{unilateral}");
        plan_fixture_module(&client,&a,"fixture-lead-a").await;
        plan_fixture_module(&client,&b,"fixture-lead-b").await;
        agree_fixture_contract(&client,&a,"fixture-lead-a","payload-ab",1).await;
        let unilateral=core_change(&client,"plan_work",Some("E-001"),json!({"op":"freeze_epic","epic":"E-001"}),true).await;
        assert!(unilateral.contains("agree") || unilateral.contains("confirm") || unilateral.contains("contract"),"{unilateral}");
        agree_fixture_contract(&client,&b,"fixture-lead-b","payload-ab",1).await;
        let cycle=core_change(&client,"plan_work",Some(&a),json!({"op":"edit_module","module":a,"dependencies":[{"ref":b,"condition":"accepted","reason":"This creates a mandatory wait cycle"}]}),true).await;
        assert!(cycle.contains("cycle") || cycle.contains("architecture"),"{cycle}");
        core_change(&client,"plan_work",Some("E-001"),json!({"op":"freeze_epic","epic":"E-001"}),false).await;
        core_change(&client,"record_work",Some(&a),json!({"op":"begin","ref":a,"actor":"fixture-lead-a"}),false).await;
        let wait=core_change(&client,"record_work",Some(&b),json!({"op":"begin","ref":b,"actor":"fixture-lead-b"}),true).await;
        assert!(wait.contains(&a) && (wait.contains("accepted") || wait.contains("ready")),"{wait}");
        let later=create_core_module(&client,&temp.path().join("later-source"),"Later standalone discovery").await;
        let frozen=core_change(&client,"plan_work",Some("E-001"),json!({"op":"edit_epic","epic":"E-001","modules":[a,b,later]}),true).await;
        assert!(frozen.contains("frozen") || frozen.contains("roster"),"{frozen}");
        client.cancel().await.unwrap();
        let cold=connect(&config,temp.path()).await;
        let restored=call(&cold,"get_context",json!({"project":"product","ref":a}),false).await;
        assert!(restored.contains("fixture-lead-a") && restored.contains("payload-ab"),"{restored}");
        cold.cancel().await.unwrap();
    }).await.expect("Bounded lead-discovery/freeze protocol scenario");
}

/// Check already observed runtime receipt interoperability only; no model launch/authentication is tested.
/// Root supplies a bounded JSON object with `lead`/`reviewer` bind_agent fields through the environment.
#[tokio::test]
#[ignore = "Requires root-provided real broker receipt data; no live runtime is launched by this test"]
async fn observed_runtime_binding_receipts_stdio_smoke() {
    tokio::time::timeout(Duration::from_secs(60),async {
        let path=std::env::var_os("MCP_TEST_BINDING_RECEIPTS").expect("Set MCP_TEST_BINDING_RECEIPTS to observed receipt JSON");
        use std::io::Read;
        let mut bytes=Vec::new();
        std::fs::File::open(path).unwrap().take(8193).read_to_end(&mut bytes).unwrap();
        assert!(bytes.len()<=8192,"Receipt fixture is bounded");
        let receipts:Value=serde_json::from_slice(&bytes).unwrap();
        let (temp,_root,config,client)=fixture().await;
        let module=create_core_module(&client,&temp.path().join("receipt-source"),"Observed receipt interoperability").await;
        let before=call(&client,"get_context",json!({"project":"product","ref":module}),false).await;
        let lead_id=receipts["lead"]["agent_id"].as_str().unwrap();
        assert!(!before.contains(lead_id),"Runtime ID must remain absent before binding");
        for role in ["lead","reviewer"] {
            let mut args=receipts[role].clone();
            let fields=args.as_object_mut().unwrap();
            fields.insert("op".into(),json!("bind_agent"));
            fields.insert("ref".into(),json!(module));
            fields.insert("role".into(),json!(role));
            core_change(&client,"record_work",Some(&module),args.clone(),false).await;
            let token=version(&client,&module).await;
            core_change(&client,"record_work",Some(&module),args,false).await;
            assert_eq!(version(&client,&module).await,token);
        }
        let substitute=core_change(&client,"record_work",Some(&module),json!({"op":"bind_agent","ref":module,"role":"lead","harness":"fixture-only","agent_id":"fixture-unapproved-substitute","communication_ref":"fixture-only: substitute","launch_ref":"fixture-only: no loss observation"}),true).await;
        assert!(substitute.contains("lost") || substitute.contains("replace") || substitute.contains("recover"),"{substitute}");
        client.cancel().await.unwrap();
        let cold=connect(&config,temp.path()).await;
        let restored=call(&cold,"get_context",json!({"project":"product","ref":module}),false).await;
        assert!(restored.contains(lead_id) && restored.contains(receipts["reviewer"]["agent_id"].as_str().unwrap()),"{restored}");
        cold.cancel().await.unwrap();
    }).await.expect("Bounded observed receipt-record interoperability smoke");
}

/// Import one actual code report for Task and Module while the current lead separately decides Task done.
async fn report_fixture_candidate(
    client: &RunningService<RoleClient, ()>,
    module: &str,
    lead: &str,
    candidate: &str,
    task: bool,
) {
    if task {
        let reference = format!("{module}/T-001");
        core_change(
            client,
            "record_work",
            Some(module),
            json!({"op":"import_commits","ref":reference,"commits":[candidate],"actor":lead}),
            false,
        )
        .await;
        let context = call(
            client,
            "get_context",
            json!({"project":"product","ref":reference}),
            false,
        )
        .await;
        assert!(
            context.contains("open"),
            "Commit import must not decide local Task completion: {context}"
        );
        core_change(
            client,
            "record_work",
            Some(module),
            json!({"op":"complete","ref":reference,"actor":lead}),
            false,
        )
        .await;
    }
    core_change(
        client,
        "record_work",
        Some(module),
        json!({"op":"import_commits","ref":module,"commits":[candidate],"actor":lead}),
        false,
    )
    .await;
}

/// A real mutant/control cycle and exact contract/candidate evidence make a Module ready before merge.
#[tokio::test]
async fn epic_core_candidate_mutation_review_and_contract_invalidation_stdio() {
    tokio::time::timeout(Duration::from_secs(90),async {
        let (temp,_root,_config,client)=fixture().await;
        let source_a=temp.path().join("candidate-a");
        let a=create_core_module(&client,&source_a,"Candidate provider").await;
        let b=create_core_module(&client,&temp.path().join("candidate-b"),"Future consumer").await;
        prepare_core_lead(&client,&a,"fixture-lead-a",true).await;
        prepare_core_lead(&client,&b,"fixture-lead-b",false).await;
        let artifact=source_a.join("payload-contract.md");
        std::fs::write(&artifact,"Revision1: numeric payload mapping and invalid-input rejection\n").unwrap();
        for (module,peer,lead,provider) in [(&a,&b,"fixture-lead-a",true),(&b,&a,"fixture-lead-b",false)] {
            let entry=fixture_contract("payload-ab",1,peer,&artifact);
            let contracts=if provider {json!({"not_required":false,"provides":[entry],"consumes":[]})} else {json!({"not_required":false,"provides":[],"consumes":[entry]})};
            core_change(&client,"plan_work",Some(module),json!({"op":"edit_module","module":module,"contracts":contracts}),false).await;
            plan_fixture_module(&client,module,lead).await;
        }
        agree_fixture_contract(&client,&a,"fixture-lead-a","payload-ab",1).await;
        agree_fixture_contract(&client,&b,"fixture-lead-b","payload-ab",1).await;
        core_change(&client,"record_work",Some(&a),json!({"op":"begin","ref":a,"actor":"fixture-lead-a"}),false).await;
        let observation=observed_boundary_fixture(&source_a);
        let candidate=observation["candidate"].as_str().unwrap().to_owned();
        report_fixture_candidate(&client,&a,"fixture-lead-a",&candidate,true).await;
        bind_fixture_agent(&client,&a,&a,"reviewer","fixture-reviewer-a").await;
        let no_proof=core_change(&client,"review_work",Some(&a),json!({"ref":a,"verdict":"accepted","summary":"A green source report cannot replace mutation evidence","actor":"fixture-reviewer-a"}),true).await;
        assert!(no_proof.contains("boundary") || no_proof.contains("mutation") || no_proof.contains("evidence") || no_proof.contains("observations"),"{no_proof}");
        record_fixture_boundary(&client,&a,"fixture-lead-a","payload-ab",1,&observation).await;
        let self_review=core_change(&client,"review_work",Some(&a),json!({"ref":a,"verdict":"accepted","summary":"The lead cannot be its independent reviewer","actor":"fixture-lead-a"}),true).await;
        assert!(self_review.contains("reviewer") || self_review.contains("independent") || self_review.contains("actor"),"{self_review}");
        core_change(&client,"review_work",Some(&a),json!({"ref":a,"verdict":"accepted","summary":"Whole candidate and exact boundary independently reviewed","actor":"fixture-reviewer-a"}),false).await;
        let ready=call(&client,"get_context",json!({"project":"product","ref":a}),false).await;
        assert!(ready.contains(&candidate) && (ready.contains("ready for integration") || ready.contains("ready_for_integration")),"{ready}");
        assert!(!ready.contains("Delivery [current]"),"Review readiness must not require reported merge: {ready}");
        std::fs::write(&artifact,"Revision2: same implementation mapping with clarified contracted invalid-input behavior\n").unwrap();
        for (module,peer,lead,provider) in [(&a,&b,"fixture-lead-a",true),(&b,&a,"fixture-lead-b",false)] {
            let entry=fixture_contract("payload-ab",2,peer,&artifact);
            let contracts=if provider {json!({"not_required":false,"provides":[entry],"consumes":[]})} else {json!({"not_required":false,"provides":[],"consumes":[entry]})};
            core_change(&client,"plan_work",Some(module),json!({"op":"edit_module","module":module,"contracts":contracts}),false).await;
            plan_fixture_module(&client,module,lead).await;
        }
        let stale=call(&client,"get_context",json!({"project":"product","ref":a}),false).await;
        assert!(stale.contains(&candidate) && (stale.contains("stale") || stale.contains("not ready")),"Changed contract must stale readiness with unchanged code: {stale}");
        agree_fixture_contract(&client,&a,"fixture-lead-a","payload-ab",2).await;
        agree_fixture_contract(&client,&b,"fixture-lead-b","payload-ab",2).await;
        let old_proof=core_change(&client,"review_work",Some(&a),json!({"ref":a,"verdict":"accepted","summary":"Old mutation evidence cannot authorize revised obligations","actor":"fixture-reviewer-a","changed_scope":["payload-ab revision2"]}),true).await;
        assert!(old_proof.contains("boundary") || old_proof.contains("revision") || old_proof.contains("evidence") || old_proof.contains("observations"),"{old_proof}");
        core_change(&client,"review_work",Some(&a),json!({"ref":a,"verdict":"changes_requested","summary":"Changed obligation needs a current boundary refresh","actor":"fixture-reviewer-a","changed_scope":["payload-ab revision2"],"findings":[{"text":"Refresh boundary evidence for revision2","must_fix":true}]}),false).await;
        let mut refreshed=observed_boundary_cycle(&source_a);
        refreshed["candidate"]=json!(candidate);
        record_fixture_boundary(&client,&a,"fixture-lead-a","payload-ab",2,&refreshed).await;
        let unresolved=core_change(&client,"review_work",Some(&a),json!({"ref":a,"verdict":"accepted","summary":"Required finding still needs explicit resolution","actor":"fixture-reviewer-a","changed_scope":["payload-ab revision2"]}),true).await;
        assert!(unresolved.contains("finding") || unresolved.contains("resolve"),"{unresolved}");
        core_change(&client,"review_work",Some(&a),json!({"ref":a,"verdict":"accepted","summary":"Same reviewer verified changed obligations and refreshed actual controls","actor":"fixture-reviewer-a","changed_scope":["payload-ab revision2"],"resolved_findings":[{"review_index":1,"finding_index":0,"summary":"Actual current revision boundary/mutant/restored controls refreshed"}]}),false).await;
        let current=call(&client,"get_context",json!({"project":"product","ref":a}),false).await;
        assert!(current.contains("fixture-reviewer-a") && current.contains(&candidate) && (current.contains("ready for integration") || current.contains("ready_for_integration")),"{current}");
        client.cancel().await.unwrap();
    }).await.expect("Bounded candidate/mutation/review protocol scenario");
}

/// Compile exact committed fixture sources into a separate assembly tree; never use or edit Module binaries.
/// Return a real manifest candidate and actual valid/invalid composed behavior over the selected set.
fn assemble_fixture_candidates(
    root: &std::path::Path,
    sources: &[(String, std::path::PathBuf, String)],
    input: i32,
) -> Value {
    std::fs::create_dir_all(root).unwrap();
    let mut binaries = Vec::new();
    for (module, repository, candidate) in sources {
        let output = std::process::Command::new("git")
            .current_dir(repository)
            .args([
                "--no-pager",
                "show",
                "--no-ext-diff",
                &format!("{candidate}:lib.rs"),
            ])
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_NO_LAZY_FETCH", "1")
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "Read only the exact fixture candidate source"
        );
        let code = String::from_utf8(output.stdout).unwrap();
        binaries.push(compile_boundary_program(&root.join(module), &code));
    }
    let mut current = input;
    for binary in &binaries {
        let output = std::process::Command::new(binary)
            .arg(current.to_string())
            .output()
            .unwrap();
        assert!(output.status.success(), "Real composed valid case failed");
        current = String::from_utf8(output.stdout)
            .unwrap()
            .trim()
            .parse()
            .unwrap();
    }
    assert_eq!(current, input + i32::try_from(sources.len()).unwrap());
    let invalid = std::process::Command::new(&binaries[0])
        .arg("-1")
        .output()
        .unwrap();
    assert_eq!(invalid.status.code(), Some(2));
    assert_eq!(
        String::from_utf8(invalid.stderr).unwrap().trim(),
        "invalid input"
    );
    let manifest = root.join("assembly-manifest.json");
    std::fs::write(&manifest,serde_json::to_vec(&json!({"sources":sources,"input":input,"output":current,"invalid_input":-1,"invalid_exit":2})).unwrap()).unwrap();
    json!({"candidate":manifest,"summary":format!("Exact candidate composition {input}=>{current}; invalid-1 rejected"),"checks":[{"label":"composition","status":"passed","detail":"Actual valid mapping and invalid-input propagation executed"}],"artifacts":[manifest]})
}

/// Start/report/review one real isolated integration assembly with synthetic protocol-only role bindings.
async fn record_core_integration(
    client: &RunningService<RoleClient, ()>,
    root: &std::path::Path,
    participants: &[String],
    observations: &Value,
    environment: &str,
    scenarios: &[&str],
    suffix: &str,
) -> String {
    let saved=core_change(client,"plan_work",None,json!({"op":"create_atomic","title":format!("Connected integration {suffix}"),"outcome":"Actual selected candidates compose","participants":participants,"environment":environment,"scenarios":scenarios,"execution":execution(root),"executor":{"name":format!("fixture-integrator-{suffix}")}}),false).await;
    let reference = saved
        .lines()
        .next()
        .unwrap()
        .strip_prefix("SAVED ")
        .unwrap()
        .to_owned();
    let integrator = format!("fixture-integrator-{suffix}");
    bind_fixture_agent(client, &reference, &reference, "integrator", &integrator).await;
    bind_fixture_agent(
        client,
        &reference,
        &reference,
        "reviewer",
        &format!("fixture-integration-reviewer-{suffix}"),
    )
    .await;
    core_change(
        client,
        "record_work",
        Some(&reference),
        json!({"op":"begin","ref":reference,"actor":integrator}),
        false,
    )
    .await;
    let mut report = observations.clone();
    let fields = report.as_object_mut().unwrap();
    fields.insert("op".into(), json!("result"));
    fields.insert("ref".into(), json!(reference));
    fields.insert("actor".into(), json!(integrator));
    fields.insert("state".into(), json!("done"));
    core_change(client, "record_work", Some(&reference), report, false).await;
    core_change(client,"review_work",Some(&reference),json!({"ref":reference,"verdict":"accepted","summary":"Actual candidate assembly independently reviewed","actor":format!("fixture-integration-reviewer-{suffix}")}),false).await;
    reference
}

/// Ready AB integrates while C is unfinished; pairwise edge coverage never fabricates ABC business proof.
/// Its many RPCs and native fixture builds need a bounded host-speed budget, not a product latency gate.
#[tokio::test]
async fn epic_core_connected_ab_before_c_and_exact_business_coverage_stdio() {
    tokio::time::timeout(Duration::from_secs(600),async {
        let (temp,_root,_config,client)=fixture().await;
        let criterion="ABC business composition";
        core_change(&client,"plan_work",None,json!({"op":"create_epic","title":"Incremental ABC Epic","outcome":"Exact business composition verified","criteria":[criterion]}),false).await;
        let mut modules=Vec::new();let mut paths=Vec::new();let leads=["fixture-lead-a","fixture-lead-b","fixture-lead-c"];
        for (index,lead) in leads.iter().enumerate() {
            let path=temp.path().join(format!("source-{index}"));
            let module=create_core_module(&client,&path,&format!("Module {index}")).await;
            prepare_core_lead(&client,&module,lead,true).await;
            modules.push(module);paths.push(path);
        }
        let ab=paths[0].join("contract-ab.md");let bc=paths[1].join("contract-bc.md");
        std::fs::write(&ab,"Revision1: A numeric output is B input; invalid input rejected\n").unwrap();
        std::fs::write(&bc,"Revision1: B numeric output is C input; invalid input rejected\n").unwrap();
        let contracts=[
            json!({"not_required":false,"provides":[fixture_contract("payload-ab",1,&modules[1],&ab)],"consumes":[]}),
            json!({"not_required":false,"provides":[fixture_contract("payload-bc",1,&modules[2],&bc)],"consumes":[fixture_contract("payload-ab",1,&modules[0],&ab)]}),
            json!({"not_required":false,"provides":[],"consumes":[fixture_contract("payload-bc",1,&modules[1],&bc)]}),
        ];
        for index in 0..3 {
            core_change(&client,"plan_work",Some(&modules[index]),json!({"op":"edit_module","module":modules[index],"contracts":contracts[index]}),false).await;
            plan_fixture_module(&client,&modules[index],leads[index]).await;
        }
        for (index,id) in [(0,"payload-ab"),(1,"payload-ab"),(1,"payload-bc"),(2,"payload-bc")] {
            agree_fixture_contract(&client,&modules[index],leads[index],id,1).await;
        }
        core_change(&client,"plan_work",Some("E-001"),json!({"op":"edit_epic","epic":"E-001","modules":modules,"criterion_scopes":[{"index":0,"text":criterion,"modules":modules}]}),false).await;
        core_change(&client,"record_work",Some("E-001"),json!({"op":"begin","ref":"E-001"}),false).await;
        core_change(&client,"plan_work",Some("E-001"),json!({"op":"freeze_epic","epic":"E-001"}),false).await;
        let mut candidates=Vec::new();
        for index in 0..2 {
            core_change(&client,"record_work",Some(&modules[index]),json!({"op":"begin","ref":modules[index],"actor":leads[index]}),false).await;
            let observation=observed_boundary_fixture(&paths[index]);
            let candidate=observation["candidate"].as_str().unwrap().to_owned();
            report_fixture_candidate(&client,&modules[index],leads[index],&candidate,true).await;
            record_fixture_boundary(&client,&modules[index],leads[index],"payload-ab",1,&observation).await;
            if index==1 { record_fixture_boundary(&client,&modules[index],leads[index],"payload-bc",1,&observation).await; }
            bind_fixture_agent(&client,&modules[index],&modules[index],"reviewer",&format!("fixture-reviewer-{index}")).await;
            core_change(&client,"review_work",Some(&modules[index]),json!({"ref":modules[index],"verdict":"accepted","summary":"Exact restored Module candidate independently reviewed","actor":format!("fixture-reviewer-{index}")}),false).await;
            candidates.push(candidate);
        }
        let ready=call(&client,"get_context",json!({"project":"product","ref":"E-001","view":"integration"}),false).await;
        assert!(ready.contains(&modules[0]) && ready.contains(&modules[1]),"AB must be ready while C remains unfinished: {ready}");
        let unfinished=call(&client,"get_context",json!({"project":"product","ref":format!("{}/T-001",modules[2])}),false).await;
        assert!(unfinished.contains("open"),"{unfinished}");
        let source_ab=vec![(modules[0].clone(),paths[0].clone(),candidates[0].clone()),(modules[1].clone(),paths[1].clone(),candidates[1].clone())];
        let assembly_ab=temp.path().join("assembly-ab");
        let actual_ab=assemble_fixture_candidates(&assembly_ab,&source_ab,1);
        let job_ab=record_core_integration(&client,&assembly_ab,&modules[..2],&actual_ab,"fixture native assembly",&["valid1=>3; invalid-1 rejection"],"ab").await;
        core_change(&client,"plan_work",Some("E-001"),json!({"op":"edit_epic","epic":"E-001","atomics":[job_ab]}),false).await;
        let duplicate=core_change(&client,"plan_work",None,json!({"op":"create_atomic","title":"Duplicate AB coverage","outcome":"Must inspect current accepted coverage","participants":&modules[..2],"environment":"fixture native assembly","scenarios":["valid1=>3; invalid-1 rejection"],"execution":execution(&temp.path().join("assembly-ab-duplicate")),"executor":{"name":"fixture-duplicate-integrator"}}),false).await;
        let duplicate=duplicate.lines().next().unwrap().strip_prefix("SAVED ").unwrap().to_owned();
        bind_fixture_agent(&client,&duplicate,&duplicate,"integrator","fixture-duplicate-integrator").await;
        let refused=core_change(&client,"record_work",Some(&duplicate),json!({"op":"begin","ref":duplicate,"actor":"fixture-duplicate-integrator"}),true).await;
        assert!(refused.contains("coverage") || refused.contains("duplicate") || refused.contains("current"),"{refused}");
        core_change(&client,"record_work",Some(&modules[2]),json!({"op":"handoff","ref":modules[2],"stopping_point":"Unrelated C implementation is not finished","next_action":"Continue C independently","actor":leads[2]}),false).await;
        let preserved=call(&client,"get_context",json!({"project":"product","ref":job_ab}),false).await;
        assert!(preserved.contains("accepted"),"Unrelated C metadata must preserve AB coverage: {preserved}");
        core_change(&client,"record_work",Some(&modules[2]),json!({"op":"begin","ref":modules[2],"actor":leads[2]}),false).await;
        let observation_c=observed_boundary_fixture(&paths[2]);
        let candidate_c=observation_c["candidate"].as_str().unwrap().to_owned();
        report_fixture_candidate(&client,&modules[2],leads[2],&candidate_c,true).await;
        record_fixture_boundary(&client,&modules[2],leads[2],"payload-bc",1,&observation_c).await;
        bind_fixture_agent(&client,&modules[2],&modules[2],"reviewer","fixture-reviewer-2").await;
        core_change(&client,"review_work",Some(&modules[2]),json!({"ref":modules[2],"verdict":"accepted","summary":"C candidate independently reviewed","actor":"fixture-reviewer-2"}),false).await;
        candidates.push(candidate_c);
        let source_bc=vec![(modules[1].clone(),paths[1].clone(),candidates[1].clone()),(modules[2].clone(),paths[2].clone(),candidates[2].clone())];
        let assembly_bc=temp.path().join("assembly-bc");
        let actual_bc=assemble_fixture_candidates(&assembly_bc,&source_bc,2);
        let job_bc=record_core_integration(&client,&assembly_bc,&modules[1..],&actual_bc,"fixture native assembly",&["valid2=>4; invalid-1 rejection"],"bc").await;
        core_change(&client,"plan_work",Some("E-001"),json!({"op":"edit_epic","epic":"E-001","atomics":[job_ab,job_bc]}),false).await;
        core_change(&client,"record_work",Some("E-001"),json!({"op":"result","ref":"E-001","summary":"Pairwise boundaries covered; whole business scenario still missing","actor":"fixture-orchestrator"}),false).await;
        let no_business=core_change(&client,"review_work",Some("E-001"),json!({"ref":"E-001","verdict":"accepted","summary":"AB plus BC is not proof of ABC business behavior","actor":"fixture-orchestrator"}),true).await;
        assert!(no_business.contains("criterion") || no_business.contains("business") || no_business.contains("verif"),"{no_business}");
        for (job,actual,scenario) in [(&job_ab,&actual_ab,"valid1=>3; invalid-1 rejection"),(&job_bc,&actual_bc,"valid2=>4; invalid-1 rejection")] {
            let incomplete=core_change(&client,"record_work",Some("E-001"),json!({"op":"verify_criterion","ref":"E-001","index":0,"text":criterion,"modules":modules,"candidate":actual["candidate"],"environment":"fixture native assembly","scenarios":[scenario],"summary":"One pair cannot cover the complete ABC criterion","checks":[{"label":"composition","status":"passed"}],"integration_ref":job}),true).await;
            assert!(incomplete.contains("scope") || incomplete.contains("cover") || incomplete.contains("composition") || incomplete.contains("match"),"{incomplete}");
        }
        let all_sources:Vec<_>=(0..3).map(|index|(modules[index].clone(),paths[index].clone(),candidates[index].clone())).collect();
        let actual_abc=assemble_fixture_candidates(&temp.path().join("assembly-abc-business"),&all_sources,1);
        core_change(&client,"record_work",Some("E-001"),json!({"op":"verify_criterion","ref":"E-001","index":0,"text":criterion,"modules":modules,"candidate":actual_abc["candidate"],"environment":"fixture ABC business assembly","scenarios":["valid1=>4; invalid-1 rejection"],"summary":actual_abc["summary"],"checks":actual_abc["checks"],"artifacts":actual_abc["artifacts"],"actor":"fixture-orchestrator"}),false).await;
        core_change(&client,"review_work",Some("E-001"),json!({"ref":"E-001","verdict":"accepted","summary":"Current exact ABC business composition and applicable boundaries covered","actor":"fixture-orchestrator"}),false).await;
        core_change(&client,"record_work",Some(&modules[1]),json!({"op":"reopen","ref":format!("{}/T-001",modules[1]),"reason":"B needs an internal correction","actor":leads[1]}),false).await;
        let invalidated=call(&client,"get_context",json!({"project":"product","ref":job_ab}),false).await;
        let epic=call(&client,"get_context",json!({"project":"product","ref":"E-001"}),false).await;
        assert!(invalidated.contains("stale") && epic.contains("stale"),"{invalidated}\n{epic}");
        client.cancel().await.unwrap();
    }).await.expect("Bounded incremental real-assembly/business protocol scenario");
}

/// Only irrecoverable loss permits identity replacement; immersion preserves predecessor work/findings.
#[tokio::test]
async fn epic_core_participant_loss_immersion_and_retained_findings_stdio() {
    tokio::time::timeout(Duration::from_secs(90),async {
        let (temp,root,config,client)=fixture().await;
        let source=temp.path().join("recovery-source");
        let module=create_core_module(&client,&source,"Persistent recovery Module").await;
        prepare_core_lead(&client,&module,"fixture-lead-old",true).await;
        plan_fixture_module(&client,&module,"fixture-lead-old").await;
        core_change(&client,"record_work",Some(&module),json!({"op":"begin","ref":module,"actor":"fixture-lead-old"}),false).await;
        let observation=observed_boundary_fixture(&source);
        let candidate=observation["candidate"].as_str().unwrap().to_owned();
        report_fixture_candidate(&client,&module,"fixture-lead-old",&candidate,true).await;
        record_fixture_boundary(&client,&module,"fixture-lead-old","local",1,&observation).await;
        bind_fixture_agent(&client,&module,&module,"reviewer","fixture-reviewer-old").await;
        core_change(&client,"review_work",Some(&module),json!({"ref":module,"verdict":"changes_requested","summary":"Whole candidate review found a required clarification","actor":"fixture-reviewer-old","findings":[{"text":"Retain this required predecessor finding","must_fix":true}]}),false).await;
        let arbitrary=core_change(&client,"record_work",Some(&module),json!({"op":"bind_agent","ref":module,"role":"lead","harness":"fixture-only","agent_id":"fixture-lead-new","communication_ref":"fixture-only: new lead","launch_ref":"fixture-only: replacement without loss"}),true).await;
        assert!(arbitrary.contains("lost") || arbitrary.contains("replace") || arbitrary.contains("recover"),"{arbitrary}");
        let temporary=core_change(&client,"record_work",Some(&module),json!({"op":"recover_agent","ref":module,"role":"lead","stage":"lost","lost":true,"unrecoverable":false,"reason":"Temporary unavailability is not replacement permission","observation":"Fixture original session can resume"}),true).await;
        assert!(temporary.contains("unrecover") || temporary.contains("lost") || temporary.contains("continue"),"{temporary}");
        core_change(&client,"record_work",Some(&module),json!({"op":"recover_agent","ref":module,"role":"lead","stage":"lost","lost":true,"unrecoverable":true,"reason":"Fixture-only original lead term irrecoverably lost","observation":"Fixture terminal lookup and resume both report unavailable; original cannot continue"}),false).await;
        bind_fixture_agent(&client,&module,&module,"lead","fixture-lead-new").await;
        let before_immersion=core_change(&client,"plan_work",Some(&module),json!({"op":"edit_task","ref":format!("{module}/T-001"),"actor":"fixture-lead-new","title":"Premature replacement change"}),true).await;
        assert!(before_immersion.contains("immers") || before_immersion.contains("recover") || before_immersion.contains("gap"),"{before_immersion}");
        core_change(&client,"record_work",Some(&module),json!({"op":"recover_agent","ref":module,"role":"lead","stage":"immersed","actor":"fixture-lead-new","understanding":"Read goals, exact candidate, predecessor Task result and required review finding","sources":[module,candidate,"review:0"],"unfinished":["Resolve predecessor finding"],"gaps":["Need current agreement clarification"]}),false).await;
        let gap=core_change(&client,"record_work",Some(&module),json!({"op":"planning","ref":module,"actor":"fixture-lead-new","responsibility":"Recover original responsibility","scope":"Same isolated fixture source"}),true).await;
        assert!(gap.contains("gap") || gap.contains("recover") || gap.contains("immers"),"{gap}");
        let restored_context=call(&client,"get_context",json!({"project":"product","ref":module}),false).await;
        assert!(restored_context.contains("fixture-lead-old") && restored_context.contains("fixture-lead-new") && restored_context.contains(&candidate),"{restored_context}");
        core_change(&client,"record_work",Some(&module),json!({"op":"recover_agent","ref":module,"role":"lead","stage":"immersed","actor":"fixture-lead-new","understanding":"Reconstructed unchanged candidate, known goals, completed Task and unresolved finding; no context gaps remain","sources":[module,candidate,"review:0"],"unfinished":["Resolve predecessor finding"],"gaps":[]}),false).await;
        plan_fixture_module(&client,&module,"fixture-lead-new").await;
        let old_lead=core_change(&client,"plan_work",Some(&module),json!({"op":"edit_task","ref":format!("{module}/T-001"),"actor":"fixture-lead-old","title":"Lost predecessor may not continue"}),true).await;
        assert!(old_lead.contains("lead") || old_lead.contains("actor"),"{old_lead}");
        core_change(&client,"record_work",Some(&module),json!({"op":"recover_agent","ref":module,"role":"reviewer","stage":"lost","lost":true,"unrecoverable":true,"reason":"Fixture-only reviewer term lost","observation":"Fixture resume unavailable; no recoverable reviewer session"}),false).await;
        bind_fixture_agent(&client,&module,&module,"reviewer","fixture-reviewer-new").await;
        let reviewer_early=core_change(&client,"review_work",Some(&module),json!({"ref":module,"verdict":"accepted","summary":"Replacement reviewer must recover context first","actor":"fixture-reviewer-new","changed_scope":["Required clarification"]}),true).await;
        assert!(reviewer_early.contains("immers") || reviewer_early.contains("recover") || reviewer_early.contains("gap"),"{reviewer_early}");
        core_change(&client,"record_work",Some(&module),json!({"op":"recover_agent","ref":module,"role":"reviewer","stage":"immersed","actor":"fixture-reviewer-new","understanding":"Read exact candidate and complete predecessor whole-review/findings; only changed scope remains","sources":[module,candidate,"review:0"],"unfinished":["Required predecessor clarification"],"gaps":[]}),false).await;
        let review=call(&client,"get_context",json!({"project":"product","ref":module,"view":"review","review_index":0}),false).await;
        assert!(review.contains("Retain this required predecessor finding") && review.contains("fixture-reviewer-old"),"{review}");
        let unresolved=core_change(&client,"review_work",Some(&module),json!({"ref":module,"verdict":"accepted","summary":"Replacement may not erase predecessor findings","actor":"fixture-reviewer-new","changed_scope":["Required clarification"]}),true).await;
        assert!(unresolved.contains("finding") || unresolved.contains("resolve"),"{unresolved}");
        let atomic=core_change(&client,"plan_work",None,json!({"op":"create_atomic","title":"Integration identity recovery","outcome":"Preserve integration role context","participants":[module,"M-002"],"environment":"fixture-only recovery environment","scenarios":["Future assembly"],"execution":execution(&temp.path().join("recovery-assembly"))}),true).await;
        assert!(atomic.contains("M-002") || atomic.contains("exist") || atomic.contains("ref"),"{atomic}");
        let other=create_core_module(&client,&temp.path().join("other-recovery-source"),"Other recovery participant").await;
        let atomic=core_change(&client,"plan_work",None,json!({"op":"create_atomic","title":"Integration identity recovery","outcome":"Preserve integration role context","participants":[module,other],"environment":"fixture-only recovery environment","scenarios":["Future assembly"],"execution":execution(&temp.path().join("recovery-assembly"))}),false).await;
        let atomic=atomic.lines().next().unwrap().strip_prefix("SAVED ").unwrap().to_owned();
        bind_fixture_agent(&client,&atomic,&atomic,"integrator","fixture-integrator-old").await;
        core_change(&client,"record_work",Some(&atomic),json!({"op":"recover_agent","ref":atomic,"role":"integrator","stage":"lost","lost":true,"unrecoverable":true,"reason":"Fixture-only integrator term lost","observation":"Fixture integration session cannot continue or resume"}),false).await;
        bind_fixture_agent(&client,&atomic,&atomic,"integrator","fixture-integrator-new").await;
        let integration_early=core_change(&client,"record_work",Some(&atomic),json!({"op":"begin","ref":atomic,"actor":"fixture-integrator-new"}),true).await;
        assert!(integration_early.contains("immers") || integration_early.contains("recover") || integration_early.contains("gap"),"{integration_early}");
        core_change(&client,"record_work",Some(&atomic),json!({"op":"recover_agent","ref":atomic,"role":"integrator","stage":"immersed","actor":"fixture-integrator-new","understanding":"Read exact participant context, pending candidates and no prior accepted assembly; preserve Module internals ownership","sources":[module,other,atomic],"unfinished":["Wait for ready connected candidates"],"gaps":[]}),false).await;
        client.cancel().await.unwrap();
        let cold=connect(&config,temp.path()).await;
        let context=call(&cold,"get_context",json!({"project":"product","ref":module}),false).await;
        assert!(context.contains("fixture-lead-new") && context.contains("fixture-reviewer-new"),"{context}");
        let saved=std::fs::read_to_string(root.join("modules").join(format!("{module}.yaml"))).unwrap();
        assert!(saved.contains("fixture-lead-old") && saved.contains("fixture-reviewer-old") && saved.contains("Retain this required predecessor finding"));
        cold.cancel().await.unwrap();
    }).await.expect("Bounded irrecoverable participant recovery protocol scenario");
}
