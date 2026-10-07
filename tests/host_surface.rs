//! Real stdio/SDK cases for the registered producer tools and the knowledge, document and search
//! reads: catalog, one lock with settlement, typed currentness, exact framed pages, stale
//! continuation, sections, search kinds and sanitized argument errors. Every case runs the built
//! binary against a disposable registered documentation repository.
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
use std::{path::Path, process::Stdio, time::Duration};

/// Read one exact generated precondition from the server's compact response.
fn field(text: &str, label: &str) -> String {
    text.lines()
        .find_map(|s| s.strip_prefix(label))
        .expect(text)
        .to_owned()
}

/// Observation version printed inside the document page header line.
fn page_version(text: &str) -> String {
    text.split("; Version: ")
        .nth(1)
        .and_then(|rest| rest.split_whitespace().next())
        .expect(text)
        .to_owned()
}

/// Start a fresh binary with a disposable explicit config and no inherited credentials.
async fn connect(config: &Path, home: &Path) -> RunningService<RoleClient, ()> {
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
        .env("PATH", std::env::var_os("PATH").unwrap_or_default())
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
    assert!(text.len() <= 8192, "{} bytes", text.len());
    text
}

/// Run Git in `root` and return trimmed stdout.
fn git(root: &Path, args: &[&str]) -> String {
    let output = std::process::Command::new("git")
        .arg("-C")
        .arg(root)
        .args(args)
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .output()
        .unwrap();
    assert!(output.status.success(), "git {args:?}");
    String::from_utf8(output.stdout).unwrap().trim().to_owned()
}

/// Registered project `product` in a fresh real repository.
async fn registered() -> (
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
        json!({"project":"product","doc_dir":root,"name":"Surface product","description":"Exercise the registered surface"}),
        false,
    )
    .await;
    for (key, value) in [
        ("user.name", "Fixture"),
        ("user.email", "fixture@example.invalid"),
        ("commit.gpgsign", "false"),
    ] {
        git(&root, &["config", key, value]);
    }
    (temp, root, client)
}

/// The three producer tools join the existing ten, closed and routed.
#[tokio::test]
async fn catalog_lists_the_producer_tools() {
    tokio::time::timeout(Duration::from_secs(60), async {
        let (_temp, _root, client) = registered().await;
        let tools = client.list_tools(Default::default()).await.unwrap().tools;
        let mut names: Vec<String> = tools.iter().map(|t| t.name.to_string()).collect();
        names.sort();
        for expected in [
            "compaction_work",
            "document_work",
            "get_context",
            "get_project_list",
            "get_status",
            "knowledge_work",
            "plan_work",
            "project_status",
            "record_work",
            "register_project",
            "review_module",
            "review_work",
            "search",
        ] {
            assert!(names.contains(&expected.to_owned()), "{expected} in {names:?}");
        }
        for tool in &tools {
            let schema = serde_json::to_value(&tool.input_schema).unwrap();
            assert_eq!(schema["type"], "object", "{}", tool.name);
        }
        let unknown = call(
            &client,
            "knowledge_work",
            json!({"project":"product","version":"x".repeat(64),"op":"create_decision","bogus":"hunter2"}),
            true,
        )
        .await;
        assert!(unknown.contains("Unknown field") && !unknown.contains("hunter2"), "{unknown}");
        client.cancel().await.unwrap();
    })
    .await
    .expect("Bounded SDK/stdio case");
}

/// Typed knowledge: commit after success, currentness, supersession, reads and search filters.
#[tokio::test]
async fn knowledge_flow_over_real_stdio() {
    tokio::time::timeout(Duration::from_secs(120), async {
        let (_temp, root, client) = registered().await;
        let bootstrap: usize = git(&root, &["rev-list", "--count", "HEAD"]).parse().unwrap();
        let mut ids = Vec::new();
        for (title, decision) in [("Quorum rule", "Use a simple majority quorum"), ("Quorum rule v2", "Use a two thirds quorum")] {
            let context = call(&client, "get_context", json!({"project":"product"}), false).await;
            let created = call(
                &client,
                "knowledge_work",
                json!({"project":"product","op":"create_decision","version":field(&context,"Knowledge allocation version: "),
                    "title":title,"question":"How is quorum decided?","decision":decision,"rationale":"Agreed after review"}),
                false,
            )
            .await;
            assert!(created.contains("Decision state: current") && created.contains("Git: committed "), "{created}");
            ids.push(created.lines().next().unwrap().split_whitespace().nth(1).unwrap().to_owned());
        }
        assert_eq!(ids, ["D-001", "D-002"]);
        assert_eq!(git(&root, &["rev-list", "--count", "HEAD"]).parse::<usize>().unwrap(), bootstrap + 2);
        let version = field(&call(&client, "get_context", json!({"project":"product","ref":"D-001"}), false).await, "Version: ");
        let superseded = call(
            &client,
            "knowledge_work",
            json!({"project":"product","op":"supersede","ref":"D-001","successor":"D-002","version":version}),
            false,
        )
        .await;
        assert!(superseded.contains("Git: committed "), "{superseded}");
        let before = git(&root, &["rev-parse", "HEAD"]);
        let old = call(&client, "get_context", json!({"project":"product","ref":"D-001"}), false).await;
        assert!(old.contains("SUPERSEDED by D-002") && old.contains("simple majority"), "{old}");
        let history = call(&client, "get_context", json!({"project":"product","ref":"D-001","view":"history"}), false).await;
        assert!(history.contains("No retained history"), "{history}");
        let refs = call(&client, "get_context", json!({"project":"product","ref":"D-002","view":"references"}), false).await;
        assert!(refs.contains("Incoming:"), "{refs}");
        let project = call(&client, "get_context", json!({"project":"product"}), false).await;
        assert!(project.contains("Decisions 1 current/1 superseded"), "{project}");
        assert!(project.contains("Git persistence:"), "{project}");
        let all = call(&client, "search", json!({"project":"product","query":"quorum","kinds":["knowledge"]}), false).await;
        assert!(all.contains("D-001 ") && all.contains("D-002 ") && all.contains("[knowledge superseded]") && all.contains("[knowledge current]"), "{all}");
        assert!(all.contains("open: get_context ref=D-002"), "{all}");
        let current = call(&client, "search", json!({"project":"product","query":"quorum","kinds":["knowledge"],"state":"current"}), false).await;
        assert!(current.contains("D-002 ") && !current.contains("D-001 "), "{current}");
        let old_only = call(&client, "search", json!({"project":"product","query":"quorum","kinds":["knowledge"],"state":"superseded"}), false).await;
        assert!(old_only.contains("D-001 ") && !old_only.contains("D-002 "), "{old_only}");
        let noop = call(
            &client,
            "knowledge_work",
            json!({"project":"product","op":"edit_decision","ref":"D-002","title":"Quorum rule v2","version":field(&call(&client,"get_context",json!({"project":"product","ref":"D-002"}),false).await,"Version: ")}),
            false,
        )
        .await;
        assert!(noop.starts_with("UNCHANGED D-002") && !noop.contains("Git:"), "{noop}");
        assert_eq!(git(&root, &["rev-parse", "HEAD"]), before, "reads and a no-op never commit");
        let bad_view = call(&client, "get_context", json!({"project":"product","ref":"D-001","view":"content"}), true).await;
        assert!(bad_view.contains("view:"), "{bad_view}");
        let bad_field = call(&client, "get_context", json!({"project":"product","ref":"D-001","heading":"x"}), true).await;
        assert!(bad_field.contains("heading:"), "{bad_field}");
        client.cancel().await.unwrap();
    })
    .await
    .expect("Bounded SDK/stdio case");
}

/// Decode the `md-text-v1` escapes of one page payload.
fn unescape(text: &str) -> Vec<u8> {
    let mut out = String::new();
    let mut chars = text.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.next().unwrap() {
            '\\' => out.push('\\'),
            'r' => out.push('\r'),
            'u' => {
                assert_eq!(chars.next(), Some('{'));
                let hex: String = chars.by_ref().take_while(|c| *c != '}').collect();
                out.push(char::from_u32(u32::from_str_radix(&hex, 16).unwrap()).unwrap());
            }
            other => panic!("unknown escape {other}"),
        }
    }
    out.into_bytes()
}

/// Rebuild the exact bytes of a read from its framed pages using only the reply text.
async fn read_all(client: &RunningService<RoleClient, ()>, args: Value) -> Vec<u8> {
    let mut bytes = Vec::new();
    let mut start = 0usize;
    let mut version: Option<String> = None;
    loop {
        let mut request = args.clone();
        request["start"] = json!(start);
        if let Some(v) = &version {
            request["version"] = json!(v);
        }
        let reply = call(client, "get_context", request, false).await;
        let frame = reply
            .lines()
            .find(|l| l.starts_with("Content: "))
            .unwrap()
            .to_owned();
        let wire = frame
            .split("wire=")
            .nth(1)
            .unwrap()
            .split(' ')
            .next()
            .unwrap()
            .to_owned();
        let length: usize = frame
            .rsplit("encoded_len=")
            .next()
            .unwrap()
            .parse()
            .unwrap();
        let at = reply.find(&frame).unwrap() + frame.len() + 1;
        let payload = &reply[at..at + length];
        bytes.extend(if wire == "escaped" {
            unescape(payload)
        } else {
            payload.as_bytes().to_vec()
        });
        version = Some(field(&reply, "Snapshot version: "));
        match reply.lines().find_map(|l| l.strip_prefix("Next: start=")) {
            Some(next) => start = next.split(';').next().unwrap().parse().unwrap(),
            None => return bytes,
        }
    }
}

/// Managed Markdown: exact large pages with CRLF and BOM, sections, stale continuation, search.
#[tokio::test]
async fn document_flow_over_real_stdio() {
    tokio::time::timeout(Duration::from_secs(180), async {
        let (_temp, root, client) = registered().await;
        let mut body = String::from("\u{feff}# Guide\r\nintro\r\n");
        for n in 0..160 {
            body.push_str(&format!("## Setup\r\nunique-token-{n} line one\r\n```\r\n## not a heading\r\n```\r\nmore text\r\n"));
        }
        body.push_str("trailing without terminator");
        let absent = call(&client, "get_context", json!({"project":"product","ref":"docs/guide.md"}), false).await;
        assert!(absent.contains("State: absent"), "{absent}");
        let saved = call(
            &client,
            "document_work",
            json!({"project":"product","op":"save","ref":"docs/guide.md","purpose":"Operator guide","body":body,"version":field(&absent,"Version: ")}),
            false,
        )
        .await;
        assert!(saved.contains("Git: committed "), "{saved}");
        let rebuilt = read_all(&client, json!({"project":"product","ref":"docs/guide.md","view":"content"})).await;
        assert_eq!(rebuilt, body.as_bytes(), "pages reconstruct the exact bytes");
        let summary = call(&client, "get_context", json!({"project":"product","ref":"docs/guide.md"}), false).await;
        assert!(summary.contains("State: managed") && summary.contains("occurrence 2"), "{summary}");
        let second = call(&client, "get_context", json!({"project":"product","ref":"docs/guide.md","view":"content","heading":"Setup","occurrence":2}), false).await;
        assert!(second.contains("unique-token-1 "), "{second}");
        let ambiguous = call(&client, "get_context", json!({"project":"product","ref":"docs/guide.md","view":"content","heading":"Setup"}), true).await;
        assert!(ambiguous.contains("ambiguous"), "{ambiguous}");
        let limit = call(&client, "get_context", json!({"project":"product","ref":"docs/guide.md","view":"content","limit":5}), true).await;
        assert!(limit.contains("limit:"), "{limit}");
        let selector = call(&client, "get_context", json!({"project":"product","ref":"docs/guide.md","ordinal":1}), true).await;
        assert!(selector.contains("ordinal:"), "{selector}");
        let first = call(&client, "get_context", json!({"project":"product","ref":"docs/guide.md","view":"content"}), false).await;
        let next: usize = first.lines().find_map(|l| l.strip_prefix("Next: start=")).unwrap().split(';').next().unwrap().parse().unwrap();
        let snapshot = field(&first, "Snapshot version: ");
        call(
            &client,
            "document_work",
            json!({"project":"product","op":"save","ref":"docs/guide.md","body":format!("{body} changed"),"version":page_version(&first)}),
            false,
        )
        .await;
        let stale = call(&client, "get_context", json!({"project":"product","ref":"docs/guide.md","view":"content","start":next,"version":snapshot}), true).await;
        assert!(stale.contains("stale"), "{stale}");
        let hit = call(&client, "search", json!({"project":"product","query":"unique-token-3","kinds":["document"]}), false).await;
        assert!(hit.contains("docs/guide.md") && hit.contains("heading=\"Setup\" occurrence=4") && hit.contains("Coverage documents:"), "{hit}");
        let readme = call(&client, "get_context", json!({"project":"product","ref":"README.md"}), false).await;
        assert!(readme.contains("State: unmanaged"), "{readme}");
        client.cancel().await.unwrap();
        assert!(root.join("docs/guide.md").is_file());
    })
    .await
    .expect("Bounded SDK/stdio case");
}
