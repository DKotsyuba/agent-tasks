//! Real stdio/SDK retests of the observed host defects: field-named validation errors (AT-003),
//! Epic and Atomic review guidance (AT-001) and Epic member roll-up (AT-004).
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

/// Call one tool through the SDK, asserting the error flag, text-only content and the budget.
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
    let text = wire["content"][0]["text"].as_str().unwrap().to_owned();
    assert!(text.len() <= 8192);
    text
}

/// Current whole-file version of one record from its context.
async fn version(client: &RunningService<RoleClient, ()>, reference: &str) -> String {
    field(
        &call(
            client,
            "get_context",
            json!({"project":"product","ref":reference}),
            false,
        )
        .await,
        "Version: ",
    )
}

/// Registered project with one Epic, two member Modules and one standalone Atomic.
async fn populated() -> (
    tempfile::TempDir,
    std::path::PathBuf,
    RunningService<RoleClient, ()>,
) {
    let temp = tempfile::tempdir_in("/private/tmp").unwrap();
    let root = temp.path().join("portable-docs");
    let config = temp.path().join("config.toml");
    std::fs::write(&config, "schema_version = 1\n").unwrap();
    let client = connect(&config, temp.path()).await;
    call(
        &client,
        "register_project",
        json!({"project":"product","doc_dir":root,"name":"Host product","description":"Exercise the shared host"}),
        false,
    )
    .await;
    for title in ["First module", "Second module"] {
        let context = call(&client, "get_context", json!({"project":"product"}), false).await;
        call(
            &client,
            "plan_work",
            json!({"project":"product","op":"create_module","version":field(&context,"Allocation version: "),"title":title,"outcome":"Do the work"}),
            false,
        )
        .await;
    }
    let context = call(&client, "get_context", json!({"project":"product"}), false).await;
    call(
        &client,
        "plan_work",
        json!({"project":"product","op":"create_epic","version":field(&context,"Allocation version: "),"title":"Whole delivery","outcome":"Everything ships","criteria":["It ships"]}),
        false,
    )
    .await;
    call(
        &client,
        "plan_work",
        json!({"project":"product","op":"edit_epic","epic":"E-001","version":version(&client,"E-001").await,"modules":["M-001","M-002"]}),
        false,
    )
    .await;
    let context = call(&client, "get_context", json!({"project":"product"}), false).await;
    call(
        &client,
        "plan_work",
        json!({"project":"product","op":"create_atomic","version":field(&context,"Allocation version: "),"title":"Integration","outcome":"Pieces fit"}),
        false,
    )
    .await;
    (temp, root, client)
}

/// A rejected list names its field and limit and a wrong field name is named; no value leaks.
#[tokio::test]
async fn validation_errors_name_the_field_not_the_value() {
    tokio::time::timeout(Duration::from_secs(60), async {
        let (_temp, _root, client) = populated().await;
        call(
            &client,
            "record_work",
            json!({"project":"product","ref":"M-001","op":"bind_agent","version":version(&client,"M-001").await,"role":"lead","harness":"test","agent_id":"lead-1","communication_ref":"steer:lead-1","launch_ref":"launch-1"}),
            false,
        )
        .await;
        let refs: Vec<String> = (0..10).map(|i| format!("secret-reference-{i}")).collect();
        let long = call(
            &client,
            "record_work",
            json!({"project":"product","ref":"M-001","op":"planning","version":version(&client,"M-001").await,"responsibility":"r","scope":"s","actor":"lead-1","read_refs":refs}),
            true,
        )
        .await;
        assert!(long.contains("read_refs: ") && long.contains("eight"), "{long}");
        assert!(!long.contains("secret-reference"), "{long}");
        let nine: Vec<String> = (0..9).map(|i| format!("criterion {i}")).collect();
        let criteria = call(
            &client,
            "plan_work",
            json!({"project":"product","op":"edit_module","module":"M-001","version":version(&client,"M-001").await,"criteria":nine}),
            true,
        )
        .await;
        assert!(criteria.contains("criteria: ") && criteria.contains("eight"), "{criteria}");
        assert!(!criteria.contains("criterion 8"), "{criteria}");
        let wrong = call(
            &client,
            "plan_work",
            json!({"project":"product","op":"edit_task","reference":"M-001/T-001","version":"x","title":"hunter2"}),
            true,
        )
        .await;
        assert!(wrong.contains("Unknown field") && wrong.contains("reference"), "{wrong}");
        assert!(!wrong.contains("hunter2"), "{wrong}");
        client.cancel().await.unwrap();
    })
    .await
    .expect("Bounded SDK/stdio retest");
}

/// Epic and Atomic context name their own routes and an Epic scope never claims a complete zero.
#[tokio::test]
async fn epic_guidance_and_roll_up_over_real_stdio() {
    tokio::time::timeout(Duration::from_secs(60), async {
        let (_temp, root, client) = populated().await;
        let epic = call(
            &client,
            "get_context",
            json!({"project":"product","ref":"E-001"}),
            false,
        )
        .await;
        assert!(
            epic.contains("review_work for a whole-Epic verdict")
                && !epic.contains("review_module"),
            "{epic}"
        );
        let atomic = call(
            &client,
            "get_context",
            json!({"project":"product","ref":"A-001"}),
            false,
        )
        .await;
        assert!(
            atomic.contains("review_work for an independent Atomic verdict"),
            "{atomic}"
        );
        let clean = call(
            &client,
            "project_status",
            json!({"project":"product","module":"E-001"}),
            false,
        )
        .await;
        assert!(
            clean.contains("Data coverage: complete") && clean.contains("M-002"),
            "{clean}"
        );
        std::fs::write(root.join("modules/M-002.yaml"), b"not: [valid").unwrap();
        let partial = call(
            &client,
            "project_status",
            json!({"project":"product","module":"E-001"}),
            false,
        )
        .await;
        assert!(
            partial.contains("Data coverage: PARTIAL")
                && partial.contains("1 member(s) unreadable"),
            "{partial}"
        );
        let context = call(
            &client,
            "get_context",
            json!({"project":"product","ref":"E-001"}),
            false,
        )
        .await;
        assert!(
            context.contains("Members: 1 readable of 2 declared")
                && context.contains("Unreadable: M-002"),
            "{context}"
        );
        client.cancel().await.unwrap();
    })
    .await
    .expect("Bounded SDK/stdio retest");
}
