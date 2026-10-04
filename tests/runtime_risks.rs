//! Measured writer-lock latency and real stdio behavior across a disposable gateway restart.
#[allow(dead_code)]
mod support;
use agent_tasks::{config::Config, gateway::Gateway, linear::Linear, server};
use rmcp::{ServiceExt, model::CallToolRequestParams, transport::TokioChildProcess};
use serde_json::{Value, json};
use std::{
    path::Path,
    time::{Duration, Instant},
};
use support::{Fixture, id};

/// Wait only for a bounded fixture condition; timeout identifies the fixture that failed.
async fn wait_delay(f: &Fixture, active: bool) {
    tokio::time::timeout(Duration::from_secs(3), async {
        while f.db.lock().await.delay_active != active {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .unwrap();
}

/// Delay a collected read while two calls queue; one caller deadline expires and the other completes.
#[tokio::test]
async fn queued_reads_wait_for_writer_and_caller_timeout_is_explicit() {
    let f = Fixture::new().await;
    let project = f.project().await;
    let template = f
        .ok(
            "add_comment",
            json!({"target_type":"project","target_id":project,"body":"discussion"}),
        )
        .await["comment"]
        .clone();
    {
        let mut db = f.db.lock().await;
        for _ in 0..500 {
            let mut row = template.clone();
            let id = id();
            row["id"] = json!(id);
            db.comments.insert(id, row);
        }
        db.delay_next = Some(("QComments".into(), Duration::from_millis(400)));
    }
    let gateway = f.gateway.clone();
    let p = project.clone();
    let held =
        tokio::spawn(async move { gateway.call("get_overview", json!({"project_id":p})).await });
    wait_delay(&f, true).await;
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let config = Config {
        listen: listener.local_addr().unwrap(),
        token: "queue-fixture-token-01234567890123456789".into(),
    };
    let cancel = tokio_util::sync::CancellationToken::new();
    let app = server::router(f.gateway.clone(), &config, cancel.clone());
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let client = reqwest::Client::new();
    let http_deadline=client.post(format!("http://{}/mcp",config.listen)).bearer_auth(&config.token)
        .header("accept","application/json, text/event-stream").header("MCP-Protocol-Version","2026-07-28")
        .header("Mcp-Method","tools/call").header("Mcp-Name","get_context").timeout(Duration::from_millis(75))
        .json(&json!({"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"get_context","arguments":{"type":"project","id":project},
            "_meta":{"io.modelcontextprotocol/protocolVersion":"2026-07-28","io.modelcontextprotocol/clientCapabilities":{}}}})).send();
    let started = Instant::now();
    let plain = f
        .gateway
        .call("get_context", json!({"type":"project","id":project}));
    let deadline = tokio::time::timeout(
        Duration::from_millis(75),
        f.gateway
            .call("get_context", json!({"type":"project","id":project})),
    );
    let (plain, deadline, http_deadline) = tokio::join!(plain, deadline, http_deadline);
    let queued = started.elapsed();
    assert_eq!(plain.status, "ok");
    assert!(deadline.is_err());
    assert!(http_deadline.unwrap_err().is_timeout());
    cancel.cancel();
    server.abort();
    assert!(queued >= Duration::from_millis(300));
    assert_eq!(held.await.unwrap().status, "ok");
    eprintln!(
        "R1 queued_read_ms={} caller_deadline_ms=75 caller_deadline=expired HTTP_client_timeout=expired collection_comments=501; no gateway command timeout observed",
        queued.as_millis()
    );
}

/// Run the production writer in a separate test binary with an explicitly supplied loopback provider.
/// This ignored entry is launched only by the restart test with disposable fixture configuration.
#[tokio::test]
#[ignore]
async fn fixture_gateway_process() {
    let path = std::env::var("RISK_GATEWAY_CONFIG").expect("fixture config required");
    let endpoint =
        std::env::var("RISK_LINEAR_ENDPOINT").expect("loopback fixture endpoint required");
    assert!(endpoint.starts_with("http://127.0.0.1:"));
    let config = Config::load(Path::new(&path)).unwrap();
    server::serve(
        Gateway::new(Linear::mock(&endpoint).unwrap()).unwrap(),
        config,
    )
    .await
    .unwrap();
}

/// Start only the ignored production-writer harness; kill-on-drop prevents leaked fixture processes.
async fn writer(config: &Path, endpoint: &str) -> tokio::process::Child {
    tokio::process::Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "fixture_gateway_process",
            "--ignored",
            "--nocapture",
        ])
        .env("RISK_GATEWAY_CONFIG", config)
        .env("RISK_LINEAR_ENDPOINT", endpoint)
        .kill_on_drop(true)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .unwrap()
}

/// Require the fresh listener, with a bounded wait and no real provider or credential access.
async fn healthy(config: &Config) {
    let client = reqwest::Client::new();
    tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            if client
                .get(format!("http://{}/health", config.listen))
                .send()
                .await
                .is_ok()
            {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
}

/// Send actual SIGTERM to the exact owned writer PID and verify process completion.
async fn terminate(child: &mut tokio::process::Child) {
    let pid = child.id().unwrap().to_string();
    assert!(
        tokio::process::Command::new("/bin/kill")
            .args(["-TERM", &pid])
            .status()
            .await
            .unwrap()
            .success()
    );
    let status = tokio::time::timeout(Duration::from_secs(3), child.wait())
        .await
        .unwrap()
        .unwrap();
    assert!(!status.success());
}

/// Build one public MCP tool request with exact supplied arguments.
fn request(name: &str, args: Value) -> CallToolRequestParams {
    CallToolRequestParams::new(name.to_owned()).with_arguments(args.as_object().unwrap().clone())
}

/// Actual product stdio bridges expose post-SIGTERM connection behavior; pending native writes reconcile after restart.
#[cfg(unix)]
#[tokio::test]
async fn stdio_restart_and_inflight_write_are_measured() {
    let f = Fixture::new().await;
    let project = f.project().await;
    let module = f.work("module", &project, None).await;
    f.mv(&module, "In Progress").await;
    let task = f.work("task", &project, Some(&module)).await;
    f.mv(&task, "In Progress").await;
    let root = tempfile::tempdir().unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    drop(listener);
    let config = Config {
        listen: address,
        token: "fixture-gateway-token-01234567890123456789".into(),
    };
    let config_path = root.path().join("config.toml");
    config.write_new(&config_path).unwrap();
    let mut child = writer(&config_path, f.endpoint()).await;
    healthy(&config).await;
    let mut command = tokio::process::Command::new(support::product_binary());
    command.arg("--config").arg(&config_path).arg("stdio");
    let bridge = ().serve(TokioChildProcess::new(command).unwrap()).await.unwrap();
    assert_eq!(
        bridge.peer().list_tools(None).await.unwrap().tools.len(),
        25
    );
    terminate(&mut child).await;
    child = writer(&config_path, f.endpoint()).await;
    healthy(&config).await;
    let old = tokio::time::timeout(Duration::from_secs(2), bridge.peer().list_tools(None)).await;
    eprintln!(
        "R2 existing_stdio_after_SIGTERM_restart={:?}",
        old.as_ref()
            .map(|r| r.as_ref().map(|l| l.tools.len()).map_err(|e| e.to_string()))
    );
    let mut command = tokio::process::Command::new(support::product_binary());
    command.arg("--config").arg(&config_path).arg("stdio");
    let fresh = ().serve(TokioChildProcess::new(command).unwrap()).await.unwrap();
    assert_eq!(fresh.peer().list_tools(None).await.unwrap().tools.len(), 25);
    let args = json!({"request_id":id(),"actor":"codex:runtime-fixture","id":task,"fields":{"result":"Survives interruption"}});
    f.db.lock().await.delay_next = Some(("MUpdateIssue".into(), Duration::from_millis(400)));
    let peer = fresh.peer().clone();
    let call = request("edit_task", args.clone());
    let inflight = tokio::spawn(async move { peer.call_tool(call).await });
    wait_delay(&f, true).await;
    terminate(&mut child).await;
    let result = tokio::time::timeout(Duration::from_secs(2), inflight).await;
    eprintln!(
        "R2 inflight_write_after_SIGTERM={:?}",
        result.as_ref().map(|r| r
            .as_ref()
            .map(|o| o.as_ref().map(|v| v.is_error).map_err(|e| e.to_string()))
            .map_err(|e| e.to_string()))
    );
    tokio::time::sleep(Duration::from_millis(450)).await;
    let applied_before_retry = f.db.lock().await.issues[&task]["description"]
        .as_str()
        .unwrap()
        .contains("Survives interruption");
    f.db.lock().await.delay_active = false;
    let writes_before = f.db.lock().await.operation_counts["MUpdateIssue"];
    assert!(
        f.gateway
            .store
            .work(&task)
            .await
            .unwrap()
            .managed()
            .unwrap()
            .pending
            .is_some()
    );
    child = writer(&config_path, f.endpoint()).await;
    healthy(&config).await;
    let mut command = tokio::process::Command::new(support::product_binary());
    command.arg("--config").arg(&config_path).arg("stdio");
    let recovery = ().serve(TokioChildProcess::new(command).unwrap()).await.unwrap();
    let retried = recovery
        .peer()
        .call_tool(request("edit_task", args))
        .await
        .unwrap();
    assert_eq!(retried.is_error, Some(false));
    let current = f.gateway.store.work(&task).await.unwrap();
    assert!(current.managed().unwrap().pending.is_none());
    assert_eq!(current.fields["result"], "Survives interruption");
    assert_eq!(
        f.db.lock().await.operation_counts["MUpdateIssue"],
        writes_before + u32::from(!applied_before_retry)
    );
    eprintln!(
        "R2 interrupted native write reconciled through exact retry; applied_before_retry={} additional_MUpdateIssue={} fresh_stdio=25tools",
        applied_before_retry,
        u32::from(!applied_before_retry)
    );
    let _ = recovery.cancel().await;
    let _ = fresh.cancel().await;
    let _ = bridge.cancel().await;
    terminate(&mut child).await;
}
