//! Real stdio/SDK cycle with restart: generated storage survives a fresh process.
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
    assert_eq!(wire["isError"], error, "{wire}");
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
        let temp=tempfile::tempdir().unwrap();let root=temp.path().join("portable-docs");let config=temp.path().join("config.toml");
        std::fs::write(&config,format!("schema_version = 1\n[aliases]\nproduct = {}\n",serde_json::to_string(&root).unwrap())).unwrap();
        let client=connect(&config,temp.path()).await;
        assert_eq!(client.list_tools(Default::default()).await.unwrap().tools.len(),7);
        let context=call(&client,"get_context",json!({"project":"product"}),false).await;assert!(!root.exists());
        call(&client,"plan_work",json!({"project":"product","version":field(&context,"Allocation version: "),"op":"init_project","title":"Portable product","purpose":"Give agents work and the owner status"}),false).await;
        let context=call(&client,"get_context",json!({"project":"product"}),false).await;
        let created=call(&client,"plan_work",json!({"project":"product","version":field(&context,"Allocation version: "),"op":"create_module","title":"File-backed workflow","outcome":"Work survives process restarts","lead":{"name":"lead"},"tasks":[{"title":"Persist the report","required_checks":["restart"]}]}),false).await;
        assert!(created.starts_with("SAVED M-001"));let old=field(&created,"Version: ");
        call(&client,"record_work",json!({"project":"product","version":old,"ref":"M-001/T-001","op":"result","state":"done","summary":"Report persisted to portable YAML","actor":"lead","checks":[{"label":"restart","status":"passed"}],"artifacts":["commit: local-example"]}),false).await;
        call(&client,"record_work",json!({"project":"product","version":old,"ref":"M-001/T-001","op":"result","summary":"Stale replay must refuse"}),true).await;
        let context=call(&client,"get_context",json!({"project":"product","ref":"M-001"}),false).await;
        call(&client,"review_module",json!({"project":"product","module":"M-001","version":field(&context,"Version: "),"verdict":"accepted","summary":"Reported native scenario independently checked","actor":"reviewer"}),false).await;
        let search=call(&client,"search",json!({"project":"product","query":"persisted portable"}),false).await;assert!(search.contains("M-001/T-001"));
        let status=call(&client,"project_status",json!({"project":"product"}),false).await;assert!(status.contains("1 accepted / 1 readable") && status.contains("1 done / 1 readable"),"{status}");
        client.cancel().await.unwrap();
        let cold=connect(&config,temp.path()).await;
        let status=call(&cold,"project_status",json!({"project":"product"}),false).await;assert!(status.contains("1 accepted / 1 readable") && status.contains("reviewer"));
        let context=call(&cold,"get_context",json!({"project":"product","ref":"M-001/T-001","view":"results"}),false).await;assert!(context.contains("commit: local-example"));
        cold.cancel().await.unwrap();
        let yaml:serde_yaml_ng::Value=serde_yaml_ng::from_slice(&std::fs::read(root.join("modules/M-001.yaml")).unwrap()).unwrap();
        assert_eq!(yaml["tasks"][0]["state"].as_str(),Some("done"));assert_eq!(yaml["reviews"][0]["verdict"].as_str(),Some("accepted"));
        let binary=std::env::var_os("MCP_TEST_BINARY").map(std::path::PathBuf::from).unwrap_or_else(||env!("CARGO_BIN_EXE_agent-tasks").into());
        let doctor=std::process::Command::new(binary).args(["--config","/absent/not-a-config","doctor","--json"]).env_clear().output().unwrap();
        assert!(doctor.status.success());let doctor:Value=serde_json::from_slice(&doctor.stdout).unwrap();assert_eq!(doctor["local_ready"],true);let family:toml::Value=toml::from_str(include_str!("../family.toml")).unwrap(); assert_eq!(doctor["release_qualification"].as_str(),family["qualification"].as_str());
    }).await.expect("Bounded SDK/stdio cycle");
}
