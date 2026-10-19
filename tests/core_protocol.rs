//! Black-box owner planning and asynchronous observer contracts against the shipped stdio binary.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "Explicit fixture assertions"
)]
use bridge_sdk::bridge_protocol as bridge;
use rmcp::{
    RoleClient, ServiceExt, model::CallToolRequestParams, service::RunningService,
    transport::TokioChildProcess,
};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    fs,
    io::{BufRead, BufReader, Write},
    os::unix::{
        fs::{OpenOptionsExt, PermissionsExt},
        net::{UnixListener, UnixStream},
    },
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::{Arc, Condvar, Mutex, mpsc},
    thread,
    time::Duration,
};

/// Root-provided native temporary directory for fixtures and child environments.
/// The task sets TMPDIR/TMP/TEMP; CI uses its own isolated runner directory.
fn scratch() -> std::path::PathBuf {
    std::env::temp_dir()
        .canonicalize()
        .expect("native temporary directory exists")
}

/// Actual SDK connection to an isolated stdio process.
type Client = RunningService<RoleClient, ()>;

/// Resolve an explicitly supplied candidate binary or Cargo's integration-test binary.
fn binary() -> PathBuf {
    std::env::var_os("MCP_TEST_BINARY")
        .map(PathBuf::from)
        .unwrap_or_else(|| env!("CARGO_BIN_EXE_agent-tasks").into())
}

/// Isolated synthetic Git repository and observer settings; Drop removes only fixture-owned data.
struct Fixture {
    /// Owns all fixture files below the approved scratch root.
    temp: tempfile::TempDir,
    /// Actual local repository, with a deliberately non-main integration branch.
    repository: PathBuf,
    /// Observer's local candidate settings, separate from source and installed state.
    config: PathBuf,
}

impl Fixture {
    /// Create one committed source file and a private observer home/socket configuration.
    fn new(socket: &Path) -> Self {
        let temp = tempfile::Builder::new()
            .prefix("obs-")
            .tempdir_in(scratch())
            .unwrap();
        let repository = temp.path().join("source");
        fs::create_dir(&repository).unwrap();
        git(&repository, &["init", "--initial-branch=delivery"]);
        git(&repository, &["config", "user.name", "Synthetic owner"]);
        git(
            &repository,
            &["config", "user.email", "fixture@example.invalid"],
        );
        fs::write(
            repository.join("feature.rs"),
            "pub fn answer() -> u32 { 1 }\n",
        )
        .unwrap();
        git(&repository, &["add", "feature.rs"]);
        git(
            &repository,
            &[
                "-c",
                "commit.gpgsign=false",
                "commit",
                "-m",
                "Existing baseline",
            ],
        );
        let config = temp.path().join("config.toml");
        write_config(
            &config,
            &temp.path().join("observer-home"),
            socket,
            "fixture",
            "fixture",
            "progress-summary",
        );
        Self {
            temp,
            repository,
            config,
        }
    }

    /// Connect without inherited credentials, while preserving executable lookup and approved tmp paths.
    async fn connect(&self) -> Client {
        connect(&self.config, self.temp.path()).await
    }

    /// One initial owner planning interaction; no implementing worker bookkeeping is involved.
    async fn save(&self, client: &Client) -> String {
        call(client, "plan", json!({"repository":self.repository,"op":"save","goal":"Observe one flat agreed capability","outcomes":["Source outcome","Storage outcome"]}), false).await
    }
}

/// Write only private fixture settings; never edits live Agent Run or installed MCP configuration.
fn write_config(
    config: &Path,
    home: &Path,
    socket: &Path,
    provider: &str,
    model: &str,
    profile: &str,
) {
    let text = format!(
        "schema_version = 1\nhome = {}\nsocket = {}\nprovider = {}\nmodel = {}\nprofile = {}\n",
        serde_json::to_string(home.to_str().unwrap()).unwrap(),
        serde_json::to_string(socket.to_str().unwrap()).unwrap(),
        serde_json::to_string(provider).unwrap(),
        serde_json::to_string(model).unwrap(),
        serde_json::to_string(profile).unwrap()
    );
    fs::write(config, text).unwrap();
}

/// Execute fixture Git with isolated configuration and no signing/hooks; rejects unsuccessful commands.
fn git(repository: &Path, args: &[&str]) -> String {
    let output = Command::new("git")
        .args(["-c", "core.hooksPath=/dev/null"])
        .args(args)
        .current_dir(repository)
        .env_clear()
        .env("PATH", std::env::var_os("PATH").unwrap_or_default())
        .env("HOME", repository)
        .env("TMPDIR", scratch())
        .env("TMP", scratch())
        .env("TEMP", scratch())
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).unwrap()
}

/// Start the actual server with fixture settings and a fully isolated environment.
async fn connect(config: &Path, home: &Path) -> Client {
    let mut command = tokio::process::Command::new(binary());
    command
        .arg("--config")
        .arg(config)
        .arg("mcp")
        .env_clear()
        .env("PATH", std::env::var_os("PATH").unwrap_or_default())
        .env("HOME", home)
        .env("TMPDIR", scratch())
        .env("TMP", scratch())
        .env("TEMP", scratch())
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .current_dir(home)
        .kill_on_drop(true)
        .stderr(Stdio::null());
    tokio::time::timeout(
        Duration::from_secs(15),
        ().serve(TokioChildProcess::new(command).unwrap()),
    )
    .await
    .expect("stdio handshake deadline")
    .unwrap()
}

/// Call the public MCP API, requiring one text block and the independent execution error flag.
async fn call(client: &Client, name: &str, args: Value, error: bool) -> String {
    let reply = tokio::time::timeout(
        Duration::from_secs(30),
        client.call_tool(
            CallToolRequestParams::new(name.to_owned())
                .with_arguments(args.as_object().unwrap().clone()),
        ),
    )
    .await
    .expect("bounded MCP tool call")
    .unwrap();
    let wire = serde_json::to_value(reply).unwrap();
    assert_eq!(
        wire["isError"].as_bool().unwrap_or(false),
        error,
        "{name}: {wire}"
    );
    assert_eq!(wire["content"].as_array().unwrap().len(), 1);
    assert!(wire.get("structuredContent").is_none());
    wire["content"][0]["text"].as_str().unwrap().to_owned()
}

/// Send protected MCP metadata separately from tool arguments; require its SDK round-trip preservation.
async fn call_with_meta(
    client: &Client,
    name: &str,
    args: Value,
    thread_id: &str,
    error: bool,
) -> String {
    let params: CallToolRequestParams = serde_json::from_value(
        json!({"name":name,"arguments":args,"_meta":{"threadId":thread_id}}),
    )
    .unwrap();
    assert_eq!(
        serde_json::to_value(&params).unwrap()["_meta"]["threadId"],
        thread_id
    );
    let reply = tokio::time::timeout(Duration::from_secs(30), client.call_tool(params))
        .await
        .expect("protected-metadata status deadline")
        .unwrap();
    let value = serde_json::to_value(reply).unwrap();
    assert_eq!(
        value["isError"].as_bool().unwrap_or(false),
        error,
        "{value}"
    );
    value["content"][0]["text"].as_str().unwrap().to_owned()
}

/// Extract actual stable outcome identities from the public plan response for later retained edits.
fn outcomes(plan: &str) -> Vec<(String, String)> {
    plan.lines()
        .filter_map(|line| line.split_once(" | "))
        .map(|(id, text)| (id.to_owned(), text.to_owned()))
        .collect()
}

/// Verify English-only pending service instructions and extract the accepted real analysis handle.
fn pending(text: &str) -> String {
    assert!(
        text.is_ascii(),
        "service instructions must be English: {text}"
    );
    assert!(
        text.contains("The report is being calculated from the saved snapshot."),
        "{text}"
    );
    text.lines()
        .find_map(|line| line.strip_prefix("PENDING "))
        .expect(text)
        .to_owned()
}

/// Decode the immutable input embedded after the schema in one captured actual start request.
fn analysis_input(start: &Value) -> Value {
    serde_json::from_str(
        start["task"]
            .as_str()
            .unwrap()
            .split_once("INPUT:\n")
            .unwrap()
            .1,
    )
    .unwrap()
}

/// Produce safe Rust fixture text; repetition controls bytes without exceeding collector file caps.
fn source_body(name: &str, repetitions: usize) -> String {
    format!(
        "pub fn {name}() -> u32 {{ 1 }}\n{}",
        "// safe source observation\n".repeat(repetitions)
    )
}

/// Planning survives process replacement, preserves retained IDs and refuses stale scope changes.
#[tokio::test]
async fn planning_cas_stable_ids_and_source_preservation() {
    let fixture = Fixture::new(Path::new("/nonexistent-fixture-agent-run.sock"));
    let before = git(
        &fixture.repository,
        &["status", "--porcelain=v1", "--untracked-files=all"],
    );
    let original = fs::read(fixture.repository.join("feature.rs")).unwrap();
    let client = fixture.connect().await;
    let catalog = client.list_tools(Default::default()).await.unwrap();
    let names: Vec<_> = catalog.tools.iter().map(|t| t.name.as_ref()).collect();
    assert_eq!(names, ["get_status", "plan", "status"]);
    let missing = call(
        &client,
        "plan",
        json!({"repository":fixture.repository,"op":"get"}),
        true,
    )
    .await;
    assert!(missing.contains("plan_missing"));
    let first = fixture.save(&client).await;
    assert!(first.contains("revision 1"));
    assert!(
        first.contains("Plan creator: address unavailable"),
        "{first}"
    );
    let ids = outcomes(&first);
    assert_eq!(ids.len(), 2);
    let unchanged = call(&client, "plan", json!({"repository":fixture.repository,"op":"save","expected_revision":1,"goal":"Observe one flat agreed capability","outcomes":["Source outcome","Storage outcome"]}), false).await;
    assert_eq!(unchanged, first);
    let edited = call(&client, "plan", json!({"repository":fixture.repository,"op":"save","expected_revision":1,"goal":"Observe revised agreed capability","outcomes":[{"id":ids[0].0,"text":"Source outcome with validation"},"New agreed outcome"]}), false).await;
    assert!(edited.contains("revision 2"));
    let revised_ids = outcomes(&edited);
    assert_eq!(revised_ids[0].0, ids[0].0);
    assert_ne!(revised_ids[1].0, ids[1].0);
    let stale = call(&client, "plan", json!({"repository":fixture.repository,"op":"save","expected_revision":1,"goal":"Stale writer","outcomes":["Must not overwrite"]}), true).await;
    assert!(stale.contains("stale_plan"));
    client.cancel().await.unwrap();
    let cold = fixture.connect().await;
    assert_eq!(
        call(
            &cold,
            "plan",
            json!({"repository":fixture.repository,"op":"get"}),
            false
        )
        .await,
        edited
    );
    cold.cancel().await.unwrap();
    assert_eq!(
        git(
            &fixture.repository,
            &["status", "--porcelain=v1", "--untracked-files=all"]
        ),
        before
    );
    assert_eq!(
        fs::read(fixture.repository.join("feature.rs")).unwrap(),
        original
    );
    assert!(!fixture.repository.join(".agent-tasks").exists());
    assert!(!fixture.repository.join("project.yaml").exists());
}

/// One fake accepted execution, whose result is released only by an explicit test event.
struct FakeRun {
    /// Stable admission identity associated with the persisted request key.
    id: String,
    /// Exact original start parameters, including caller namespace and schema.
    request: Value,
    /// Verified JSON answer, absent until the fixture's completion event.
    answer: Option<String>,
}

/// Shared broker fixture facts, protected only during short synchronous request handling.
#[derive(Default)]
struct FakeState {
    /// Every submission, including recovery replays, in arrival order.
    starts: Vec<Value>,
    /// Distinct model launches keyed by their idempotent request ID.
    runs: BTreeMap<String, FakeRun>,
    /// Close one start connection after admission to model a lost acceptance response.
    lose_next_receipt: bool,
    /// Finish the accepted execution before deliberately losing its first start acknowledgement.
    complete_lost_start: bool,
    /// Break only the first accepted execution read, before its eventual valid completion.
    invalidate_next_execution_inventory: bool,
    /// Answer reads prove cached/pinned reads do not poll after completion.
    answer_reads: usize,
    /// A matching native worker whose transcript coverage is incomplete.
    runtime_gap_workdir: Option<PathBuf>,
    /// Public wait calls, recorded separately from bounded result-fencing reads.
    waits: Vec<Value>,
    /// Shutdown signal releases every still-waiting fixture connection.
    stopped: bool,
}

/// Deterministic fake Agent Run JSON-RPC socket; no native/model provider is contacted.
struct Broker {
    /// Owns the short Unix socket path under the approved scratch root.
    temp: tempfile::TempDir,
    /// Shared fixture admissions and completion events.
    state: Arc<Mutex<FakeState>>,
    /// Completion condition shared by all independently owned request handlers.
    completed: Arc<Condvar>,
    /// Event receipt proving the continuation reached its single public wait.
    waiting: mpsc::Receiver<String>,
    /// Owned listener thread, woken and joined by Drop.
    worker: Option<thread::JoinHandle<()>>,
}

impl Broker {
    /// Bind before returning, eliminating readiness sleeps and connection races.
    fn new() -> Self {
        let temp = tempfile::Builder::new()
            .prefix("sock-")
            .tempdir_in(scratch())
            .unwrap();
        let listener = UnixListener::bind(temp.path().join("api.sock")).unwrap();
        let state = Arc::new(Mutex::new(FakeState::default()));
        let shared = state.clone();
        let completed = Arc::new(Condvar::new());
        let condition = completed.clone();
        let (wait_tx, waiting) = mpsc::sync_channel(16);
        let worker = thread::spawn(move || {
            let mut handlers = Vec::new();
            for socket in listener.incoming() {
                let mut socket = socket.unwrap();
                socket
                    .set_read_timeout(Some(Duration::from_secs(5)))
                    .unwrap();
                socket
                    .set_write_timeout(Some(Duration::from_secs(5)))
                    .unwrap();
                let mut line = String::new();
                BufReader::new(&mut socket).read_line(&mut line).unwrap();
                let request: Value = serde_json::from_str(&line).unwrap();
                if request["method"] == "fixture_stop" {
                    shared.lock().unwrap().stopped = true;
                    condition.notify_all();
                    break;
                }
                let state = shared.clone();
                let completed = condition.clone();
                let ready = wait_tx.clone();
                handlers.push(thread::spawn(move || {
                    let result = if request["method"] == "wait" {
                        let id = request["params"]["agent_id"].as_str().unwrap();
                        assert!(request["params"].get("timeout").is_none(),"one public wait must not use completion polling timeouts");
                        let mut state = state.lock().unwrap();
                        state.waits.push(request["params"].clone());
                        ready.send(id.to_owned()).unwrap();
                        loop {
                            if state.stopped { return; }
                            let run = state.runs.values().find(|r|r.id==id).expect("wait binds accepted execution");
                            if run.answer.is_some() { break Some(json!({"agent_id":id,"status":"succeeded","agent":agent(run),"completed":true})); }
                            state = completed.wait(state).unwrap();
                        }
                    } else {
                        dispatch(&mut state.lock().unwrap(), &request)
                    };
                    if let Some(result) = result {
                        writeln!(socket,"{}",json!({"jsonrpc":"2.0","id":request["id"],"result":result})).unwrap();
                    }
                }));
            }
            for handler in handlers {
                handler.join().expect("owned broker handler must stop");
            }
        });
        Self {
            temp,
            state,
            completed,
            waiting,
            worker: Some(worker),
        }
    }

    /// Existing bound fixture socket, ready for private observer settings.
    fn socket(&self) -> PathBuf {
        self.temp.path().join("api.sock")
    }

    /// Release one valid schema-bound answer, optionally mutating it to exercise rejection.
    fn complete(&self, id: &str, alter: impl FnOnce(&mut Value)) {
        let mut state = self.state.lock().unwrap();
        let run = state.runs.values_mut().find(|run| run.id == id).unwrap();
        let task = run.request["task"].as_str().unwrap();
        let input = analysis_input(&run.request);
        assert!(
            run.request.get("output_schema").is_none(),
            "Do not depend on harness-specific schema admission"
        );
        assert!(task.contains("SCHEMA:\n"));
        let mut value = json!({"analysis_input_id":input["analysis_input_id"],"plan_revision":input["plan_revision"],"outcomes":input["outcomes"].as_array().unwrap().iter().map(|o| json!({"id":o["id"],"state":"no_evidence","summary":"Insufficient observed evidence","evidence":[],"integrated":null})).collect::<Vec<_>>(),"findings":[]});
        alter(&mut value);
        run.answer = Some(serde_json::to_string(&value).unwrap());
        self.completed.notify_all();
    }

    /// Snapshot original submissions to assert deterministic admission/recovery behavior.
    fn starts(&self) -> Vec<Value> {
        self.state.lock().unwrap().starts.clone()
    }

    /// Number of distinct model launches after applying idempotent request keys.
    fn launches(&self) -> usize {
        self.state.lock().unwrap().runs.len()
    }

    /// Number of original verified-answer requests observed by the fake broker.
    fn answer_reads(&self) -> usize {
        self.state.lock().unwrap().answer_reads
    }

    /// Wait for the exact continuation readiness event, never a guessed sleep or inventory poll.
    fn wait_until_waiting(&self, id: &str) {
        assert_eq!(
            self.waiting
                .recv_timeout(Duration::from_secs(15))
                .expect("continuation public wait deadline"),
            id
        );
    }

    /// Number of public wait admissions; a continuation must make one wait rather than poll.
    fn wait_calls(&self) -> usize {
        self.state.lock().unwrap().waits.len()
    }
}

impl Drop for Broker {
    /// Wake the listener explicitly and join it; no background fixture thread survives a test.
    fn drop(&mut self) {
        let stop = UnixStream::connect(self.socket()).ok().map(|mut socket| {
            let _ = writeln!(socket, "{}", json!({"method":"fixture_stop"}));
            socket
        });
        if let Some(worker) = self.worker.take() {
            let joined = worker.join();
            if !thread::panicking() {
                joined.expect("fixture broker must stop cleanly");
            }
        }
        drop(stop);
    }
}

/// Build required public execution facts; current fixture answers are hash verified.
fn agent(run: &FakeRun) -> Value {
    json!({"agent_id":run.id,"sequence":1,"workdir":run.request["workdir"],"task_summary":"You are a read-only progress summarizer.","profile":"progress-summary","status":if run.answer.is_some() {"succeeded"} else {"running"},"phase":"running","health":null,"answer_sha256":run.answer.as_ref().map(|s| format!("{:x}",Sha256::digest(s.as_bytes())))})
}

/// Handle actual API methods and panic on unexpected RPCs, including resume or developer mutations.
fn dispatch(state: &mut FakeState, request: &Value) -> Option<Value> {
    match request["method"].as_str().unwrap() {
        "list_agents" => {
            if !state.runs.is_empty()
                && std::mem::take(&mut state.invalidate_next_execution_inventory)
            {
                return Some(
                    json!({"total":state.runs.len(),"items":state.runs.values().map(agent).collect::<Vec<_>>(),"next_offset":null,"complete":true,"revision":null}),
                );
            }
            let mut items = state.runs.values().map(agent).collect::<Vec<_>>();
            if let Some(workdir) = &state.runtime_gap_workdir {
                items.push(json!({"agent_id":"ag-native-unknown","sequence":1,"workdir":workdir,"task_summary":"Implement source outcome","profile":"worker","status":"running","phase":"running","health":null,"answer_sha256":null}));
            }
            Some(
                json!({"total":items.len(),"items":items,"next_offset":null,"complete":true,"revision":1}),
            )
        }
        "transcript" if request["params"]["agent_id"] == "ag-native-unknown" => Some(
            json!({"agent_id":"ag-native-unknown","messages":[],"cursor":0,"limit":4,"next_cursor":1,"complete":false}),
        ),
        "start" => {
            let params = request["params"].clone();
            state.starts.push(params.clone());
            let next_id = format!("ag-fixture-{}", state.runs.len() + 1);
            let key = params["request_id"].as_str().unwrap().to_owned();
            let run = state.runs.entry(key).or_insert_with(|| FakeRun {
                id: next_id,
                request: params.clone(),
                answer: None,
            });
            assert_eq!(
                run.request, params,
                "recovery must replay the complete original caller namespace and start intent"
            );
            let result = json!({"agent_id":run.id,"sequence":1,"created":true,"agent":agent(run)});
            if std::mem::take(&mut state.lose_next_receipt) {
                if state.complete_lost_start {
                    let input = analysis_input(&run.request);
                    run.answer = Some(serde_json::to_string(&json!({"analysis_input_id":input["analysis_input_id"],"plan_revision":input["plan_revision"],"outcomes":input["outcomes"].as_array().unwrap().iter().map(|o| json!({"id":o["id"],"state":"no_evidence","summary":"Insufficient observed evidence","evidence":[],"integrated":null})).collect::<Vec<_>>(),"findings":[]})).unwrap());
                }
                None
            } else {
                Some(result)
            }
        }
        "answer" => {
            state.answer_reads += 1;
            let id = request["params"]["agent_id"].as_str().unwrap();
            let run = state.runs.values().find(|run| run.id == id).unwrap();
            let content = run.answer.as_ref().unwrap();
            Some(
                json!({"agent_id":id,"available":true,"inline_complete":true,"status":"succeeded","content":content,"sha256":format!("{:x}",Sha256::digest(content.as_bytes()))}),
            )
        }
        other => panic!("unexpected Agent Run RPC {other}"),
    }
}

/// Real configuration parsing rejects invalid preferences without starting any model work.
#[tokio::test]
async fn summary_configuration_rejects_invalid_language_prompt_and_unknown_fields() {
    let fixture = Fixture::new(Path::new("/nonexistent-fixture-agent-run.sock"));
    let original = fs::read_to_string(&fixture.config).unwrap();
    let client = fixture.connect().await;
    for fields in [
        r#"language = "de""#.to_owned(),
        r#"promt = "typo""#.to_owned(),
        format!(
            "prompt = {}",
            serde_json::to_string(&"x".repeat(4097)).unwrap()
        ),
        r#"prompt = "\u0000""#.to_owned(),
    ] {
        fs::write(
            &fixture.config,
            format!("{original}\n[summary]\n{fields}\n"),
        )
        .unwrap();
        let error = call(
            &client,
            "plan",
            json!({"repository":fixture.repository,"op":"get"}),
            true,
        )
        .await;
        assert!(error.contains("configuration"), "{error}");
    }
    fs::write(
        &fixture.config,
        format!(
            "{original}\n[summary]\nlanguage = \"en\"\nprompt = {}\n",
            serde_json::to_string(&"x".repeat(4096)).unwrap()
        ),
    )
    .unwrap();
    assert!(
        fixture
            .save(&client)
            .await
            .contains("Plan creator: address unavailable")
    );
    client.cancel().await.unwrap();
}

/// Language/style changes affect new cache identities while pinned results keep their frozen format.
#[tokio::test]
async fn summary_preferences_are_frozen_cached_and_count_real_commits() {
    let broker = Broker::new();
    let fixture = Fixture::new(&broker.socket());
    let client = fixture.connect().await;
    fixture.save(&client).await;
    fs::write(
        fixture.repository.join("feature.rs"),
        "pub fn answer() -> u32 { 2 }\n",
    )
    .unwrap();
    git(&fixture.repository, &["add", "feature.rs"]);
    git(
        &fixture.repository,
        &["commit", "-m", "Implement the agreed source outcome"],
    );
    git(&fixture.repository, &["branch", "same-commit-worker"]);
    fs::write(
        fixture.repository.join("feature.rs"),
        "pub fn answer() -> u32 { 3 }\n",
    )
    .unwrap();
    let original = fs::read_to_string(&fixture.config).unwrap();
    let english = format!(
        "{original}\n[summary]\nlanguage = \"en\"\nprompt = \"Emphasize remaining work.\"\n"
    );
    fs::write(&fixture.config, &english).unwrap();
    let first = pending(
        &call(
            &client,
            "status",
            json!({"repository":fixture.repository}),
            false,
        )
        .await,
    );
    let first_start = broker.starts()[0].clone();
    let task = first_start["task"].as_str().unwrap();
    assert!(task.contains(r#""language":"en""#), "{task}");
    assert!(task.contains("Emphasize remaining work."), "{task}");
    assert!(task.contains("cannot override these rules"), "{task}");
    fs::write(
        &fixture.config,
        format!(
            "{original}\n[summary]\nlanguage = \"ru\"\nprompt = \"Emphasize observed changes.\"\n"
        ),
    )
    .unwrap();
    broker.complete(&first, |_| {});
    let first_report = call(
        &client,
        "status",
        json!({"repository":fixture.repository,"analysis_id":first}),
        false,
    )
    .await;
    assert!(first_report.starts_with("- Estimate:"), "{first_report}");
    assert!(first_report.contains("- Commits: 1\n"), "{first_report}");
    assert!(
        first_report.lines().all(|line| line.starts_with("- ")),
        "{first_report}"
    );
    let second = pending(
        &call(
            &client,
            "status",
            json!({"repository":fixture.repository}),
            false,
        )
        .await,
    );
    assert_ne!(first, second);
    assert_ne!(
        analysis_input(&first_start)["analysis_input_id"],
        analysis_input(&broker.starts()[1])["analysis_input_id"]
    );
    broker.complete(&second, |_| {});
    let second_report = call(
        &client,
        "status",
        json!({"repository":fixture.repository,"analysis_id":second}),
        false,
    )
    .await;
    assert!(second_report.starts_with("- Оценка:"), "{second_report}");
    assert!(second_report.contains("- Коммиты: 1\n"), "{second_report}");
    fs::write(&fixture.config, &english).unwrap();
    assert_eq!(
        call(
            &client,
            "status",
            json!({"repository":fixture.repository}),
            false
        )
        .await,
        first_report
    );
    assert_eq!(
        broker.launches(),
        2,
        "restoring identical preferences reuses the matching report"
    );
    client.cancel().await.unwrap();
}

/// Actual asynchronous admission/result flow caches unchanged semantic inputs and expires old handles.
#[tokio::test]
async fn asynchronous_status_pinned_cache_and_retention() {
    let broker = Broker::new();
    let fixture = Fixture::new(&broker.socket());
    let client = fixture.connect().await;
    fixture.save(&client).await;
    let baseline = git(
        &fixture.repository,
        &["status", "--porcelain=v1", "--untracked-files=all"],
    );
    let first = pending(
        &call(
            &client,
            "status",
            json!({"repository":fixture.repository}),
            false,
        )
        .await,
    );
    assert_eq!(broker.launches(), 1);
    broker.complete(&first, |_| {});
    let report = call(
        &client,
        "status",
        json!({"repository":fixture.repository,"analysis_id":first}),
        false,
    )
    .await;
    assert!(
        report.contains("0/2") && report.contains("0 в работе"),
        "{report}"
    );
    let reads = broker.answer_reads();
    assert_eq!(
        call(
            &client,
            "status",
            json!({"repository":fixture.repository}),
            false
        )
        .await,
        report
    );
    assert_eq!(
        broker.starts().len(),
        1,
        "clock passage and analyzer telemetry must not relaunch inference"
    );
    assert_eq!(
        broker.answer_reads(),
        reads,
        "bound completion must not be polled again"
    );
    assert_eq!(
        git(
            &fixture.repository,
            &["status", "--porcelain=v1", "--untracked-files=all"]
        ),
        baseline
    );
    let details = call(
        &client,
        "status",
        json!({"repository":fixture.repository,"analysis_id":first,"view":"details"}),
        false,
    )
    .await;
    assert!(details.contains(&first), "{details}");
    fs::write(
        fixture.repository.join("feature.rs"),
        "pub fn answer() -> u32 { 2 }\n",
    )
    .unwrap();
    assert_eq!(
        call(
            &client,
            "status",
            json!({"repository":fixture.repository,"analysis_id":first,"view":"details"}),
            false
        )
        .await,
        details,
        "pinned details must not recollect a changed draft"
    );
    let second = pending(
        &call(
            &client,
            "status",
            json!({"repository":fixture.repository}),
            false,
        )
        .await,
    );
    broker.complete(&second, |_| {});
    call(
        &client,
        "status",
        json!({"repository":fixture.repository,"analysis_id":second}),
        false,
    )
    .await;
    fs::write(
        fixture.repository.join("feature.rs"),
        "pub fn answer() -> u32 { 3 }\n",
    )
    .unwrap();
    let third = pending(
        &call(
            &client,
            "status",
            json!({"repository":fixture.repository}),
            false,
        )
        .await,
    );
    broker.complete(&third, |_| {});
    call(
        &client,
        "status",
        json!({"repository":fixture.repository,"analysis_id":third}),
        false,
    )
    .await;
    let expired = call(
        &client,
        "status",
        json!({"repository":fixture.repository,"analysis_id":first,"view":"details"}),
        true,
    )
    .await;
    assert!(expired.contains("snapshot_expired"), "{expired}");
    assert_eq!(broker.launches(), 3);
    let record = fs::read_dir(fixture.temp.path().join("observer-home/observer-v1"))
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .path()
        .join("state.json");
    let stored: Value = serde_json::from_slice(&fs::read(record).unwrap()).unwrap();
    let starts = broker.starts();
    assert_eq!(stored["reports"].as_array().unwrap().len(), 2);
    assert_eq!(stored["reports"][0]["model_task"], starts[1]["task"]);
    assert_eq!(stored["reports"][1]["model_task"], starts[2]["task"]);
    client.cancel().await.unwrap();
}

/// Large stored bundles remain readable through bounded combined-row pages and exact pinned sources.
#[tokio::test]
async fn large_details_are_paged_and_exact_sources_remain_pinned_until_eviction() {
    let broker = Broker::new();
    let fixture = Fixture::new(&broker.socket());
    let client = fixture.connect().await;
    let agreed = (0..40)
        .map(|i| format!("Agreed outcome {i}"))
        .collect::<Vec<_>>();
    call(&client,"plan",json!({"repository":fixture.repository,"op":"save","goal":"Observe bounded detail pages","outcomes":agreed}),false).await;
    let unpinned = call(
        &client,
        "status",
        json!({"repository":fixture.repository,"view":"details","start":1}),
        true,
    )
    .await;
    assert!(unpinned.contains("invalid_arguments"), "{unpinned}");
    assert_eq!(
        broker.launches(),
        0,
        "an unpinned continuation must not admit analysis"
    );
    fs::write(
        fixture.repository.join("staged.rs"),
        source_body("staged", 400),
    )
    .unwrap();
    git(&fixture.repository, &["add", "staged.rs"]);
    fs::write(
        fixture.repository.join("feature.rs"),
        source_body("unstaged", 400),
    )
    .unwrap();
    for i in 0..12 {
        let body = source_body(&format!("part_{i:02}"), 148);
        assert!(body.len() <= 4096, "fixture exceeds one draft-file cap");
        fs::write(fixture.repository.join(format!("part_{i:02}.rs")), body).unwrap();
    }
    let id = pending(
        &call(
            &client,
            "status",
            json!({"repository":fixture.repository}),
            false,
        )
        .await,
    );
    let input = analysis_input(&broker.starts()[0]);
    let original = input["git"]["evidence"].as_array().unwrap();
    let stored_bytes: usize = original
        .iter()
        .map(|row| row["text"].as_str().unwrap().len())
        .sum();
    assert!(
        stored_bytes > 64 * 1024,
        "fixture must exercise a bundle exceeding one detail response, got {stored_bytes} bytes"
    );
    let source = original
        .iter()
        .find(|row| row["kind"] == "draft_text")
        .expect("preserved new source")
        .clone();
    let source_id = source["id"].as_str().unwrap();
    broker.complete(&id, |value| {
        value["findings"] =
            json!([{"text":"Observed drafts need more implementation","evidence":[source_id]}]);
    });
    call(
        &client,
        "status",
        json!({"repository":fixture.repository,"analysis_id":id}),
        false,
    )
    .await;
    let mut start = 0;
    let mut combined_rows = 0;
    let mut evidence_indexes = BTreeMap::new();
    let mut outcome_ids = std::collections::BTreeSet::new();
    let mut finding_count = 0;
    loop {
        let args = if start == 0 {
            json!({"repository":fixture.repository,"analysis_id":id,"view":"details"})
        } else {
            json!({"repository":fixture.repository,"analysis_id":id,"view":"details","start":start})
        };
        let text = call(&client, "status", args, false).await;
        assert!(
            text.len() <= 64 * 1024,
            "one detail page exceeded its wire bound"
        );
        let page: Value = serde_json::from_str(&text).unwrap();
        let outcomes = page["analysis"]["outcomes"].as_array().unwrap();
        let findings = page["analysis"]["findings"].as_array().unwrap();
        let evidence = page["evidence"].as_array().unwrap();
        let returned = outcomes.len() + findings.len() + evidence.len();
        assert_eq!(page["page"]["start"], start);
        assert_eq!(page["page"]["returned"], returned);
        assert_eq!(page["page"]["total"], 41 + original.len());
        assert!(returned > 0);
        for row in outcomes {
            assert!(outcome_ids.insert(row["id"].as_str().unwrap().to_owned()));
        }
        finding_count += findings.len();
        for row in evidence {
            for field in [
                "id",
                "kind",
                "reference",
                "bytes",
                "preview",
                "preview_complete",
            ] {
                assert!(
                    row.get(field).is_some(),
                    "missing evidence-index field {field}: {row}"
                );
            }
            assert!(
                row.get("text").is_none(),
                "index must not duplicate full source bodies"
            );
            assert!(row["preview"].as_str().unwrap().chars().count() <= 160);
            let source = original
                .iter()
                .find(|source| source["id"] == row["id"])
                .unwrap();
            assert_eq!(row["bytes"], source["text"].as_str().unwrap().len());
            assert!(
                evidence_indexes
                    .insert(row["id"].as_str().unwrap().to_owned(), row.clone())
                    .is_none()
            );
        }
        combined_rows += returned;
        match page["page"]["next_start"].as_u64() {
            Some(next) => {
                let next = usize::try_from(next).unwrap();
                assert_eq!(next, start + returned);
                start = next;
            }
            None => break,
        }
    }
    assert_eq!(outcome_ids.len(), 40);
    assert_eq!(finding_count, 1);
    assert_eq!(evidence_indexes.len(), original.len());
    assert_eq!(combined_rows, 41 + original.len());
    assert!(
        start > 0,
        "fixture must exercise at least one actual continuation"
    );
    assert_eq!(evidence_indexes[source_id]["preview_complete"], false);
    let exact_args = json!({"repository":fixture.repository,"analysis_id":id,"view":"details","evidence_id":source_id});
    let exact = call(&client, "status", exact_args.clone(), false).await;
    assert_eq!(
        serde_json::from_str::<Value>(&exact).unwrap()["source"],
        source
    );
    fs::write(
        fixture.repository.join("part_00.rs"),
        "pub fn changed_after_observation() {}\n",
    )
    .unwrap();
    assert_eq!(
        call(&client, "status", exact_args.clone(), false).await,
        exact,
        "an exact pinned source read must not recollect present-day content"
    );
    for i in 0..2 {
        fs::write(
            fixture.repository.join("feature.rs"),
            format!("pub fn changed_snapshot() -> u32 {{ {i} }}\n"),
        )
        .unwrap();
        let next = pending(
            &call(
                &client,
                "status",
                json!({"repository":fixture.repository}),
                false,
            )
            .await,
        );
        broker.complete(&next, |_| {});
        call(
            &client,
            "status",
            json!({"repository":fixture.repository,"analysis_id":next}),
            false,
        )
        .await;
    }
    let expired = call(&client, "status", exact_args, true).await;
    assert!(expired.contains("snapshot_expired"), "{expired}");
    assert_eq!(broker.launches(), 3);
    client.cancel().await.unwrap();
}

/// Identical Git objects and agreed scope in separate clones cannot share one admission key/job.
#[tokio::test]
async fn identical_separate_clones_admit_distinct_analysis_jobs() {
    let broker = Broker::new();
    let first = Fixture::new(&broker.socket());
    let clone_temp = tempfile::Builder::new()
        .prefix("clone-")
        .tempdir_in(scratch())
        .unwrap();
    let clone_repository = clone_temp.path().join("source");
    git(
        clone_temp.path(),
        &[
            "clone",
            "--no-hardlinks",
            first.repository.to_str().unwrap(),
            clone_repository.to_str().unwrap(),
        ],
    );
    let clone_config = clone_temp.path().join("config.toml");
    write_config(
        &clone_config,
        &clone_temp.path().join("observer-home"),
        &broker.socket(),
        "fixture",
        "fixture",
        "progress-summary",
    );
    let second = Fixture {
        temp: clone_temp,
        repository: clone_repository,
        config: clone_config,
    };
    assert_eq!(
        git(&first.repository, &["rev-parse", "HEAD"]),
        git(&second.repository, &["rev-parse", "HEAD"])
    );
    let first_client = first.connect().await;
    let second_client = second.connect().await;
    assert_eq!(
        first.save(&first_client).await,
        second.save(&second_client).await
    );
    let first_id = pending(
        &call(
            &first_client,
            "status",
            json!({"repository":first.repository}),
            false,
        )
        .await,
    );
    let second_id = pending(
        &call(
            &second_client,
            "status",
            json!({"repository":second.repository}),
            false,
        )
        .await,
    );
    let starts = broker.starts();
    assert_eq!(starts.len(), 2);
    assert_ne!(
        starts[0]["request_id"], starts[1]["request_id"],
        "repository identity must namespace otherwise identical inputs"
    );
    assert_ne!(
        analysis_input(&starts[0])["analysis_input_id"],
        analysis_input(&starts[1])["analysis_input_id"]
    );
    assert_ne!(first_id, second_id);
    assert_eq!(broker.launches(), 2);
    broker.complete(&first_id, |_| {});
    broker.complete(&second_id, |_| {});
    call(
        &first_client,
        "status",
        json!({"repository":first.repository,"analysis_id":first_id}),
        false,
    )
    .await;
    call(
        &second_client,
        "status",
        json!({"repository":second.repository,"analysis_id":second_id}),
        false,
    )
    .await;
    first_client.cancel().await.unwrap();
    second_client.cancel().await.unwrap();
}

/// Unknown outcome counts and incomplete runtime coverage stay visible without model findings.
#[tokio::test]
async fn unknown_outcomes_and_runtime_gaps_remain_visible_without_findings() {
    let broker = Broker::new();
    let fixture = Fixture::new(&broker.socket());
    broker.state.lock().unwrap().runtime_gap_workdir = Some(fixture.repository.clone());
    let client = fixture.connect().await;
    fixture.save(&client).await;
    for uncertain in [false, true] {
        if uncertain {
            fs::write(
                fixture.repository.join("feature.rs"),
                "pub fn answer() -> u32 { 2 }\n",
            )
            .unwrap();
        }
        let before = fs::read(fixture.repository.join("feature.rs")).unwrap();
        let git_before = git(
            &fixture.repository,
            &["status", "--porcelain=v1", "--untracked-files=all"],
        );
        let id = pending(
            &call(
                &client,
                "status",
                json!({"repository":fixture.repository}),
                false,
            )
            .await,
        );
        let input = analysis_input(broker.starts().last().unwrap());
        assert_eq!(input["developers"]["complete"], false);
        let source = input["git"]["evidence"]
            .as_array()
            .unwrap()
            .first()
            .map(|row| row["id"].clone());
        broker.complete(&id, |value| {
            assert_eq!(value["findings"], json!([]));
            if uncertain {
                for row in value["outcomes"].as_array_mut().unwrap() {
                    row["state"] = json!("uncertain");
                    row["evidence"] = json!([source.as_ref().expect("observed changed source")]);
                }
            }
        });
        let report = call(
            &client,
            "status",
            json!({"repository":fixture.repository,"analysis_id":id}),
            false,
        )
        .await;
        assert!(report.contains("не оценено: 2"), "{report}");
        assert!(
            report.contains("Внимание:") && report.contains("Неполные данные"),
            "{report}"
        );
        assert!(
            report.contains("0/2") && report.contains("0 в работе"),
            "unknown rows must not become completed or active: {report}"
        );
        assert_eq!(
            fs::read(fixture.repository.join("feature.rs")).unwrap(),
            before
        );
        assert_eq!(
            git(
                &fixture.repository,
                &["status", "--porcelain=v1", "--untracked-files=all"]
            ),
            git_before
        );
        assert!(!fixture.repository.join(".agent-tasks").exists());
    }
    assert_eq!(broker.launches(), 2);
    client.cancel().await.unwrap();
}

/// Four inferred active outcomes count distinctly while normal output includes only three work lines.
#[tokio::test]
async fn compact_status_limits_active_lines_and_counts_each_outcome_once() {
    let broker = Broker::new();
    let fixture = Fixture::new(&broker.socket());
    let client = fixture.connect().await;
    call(&client,"plan",json!({"repository":fixture.repository,"op":"save","goal":"Observe four independent outcomes","outcomes":["API","Storage","UI","Validation"]}),false).await;
    fs::write(
        fixture.repository.join("feature.rs"),
        "pub fn answer() -> u32 { 2 }\n",
    )
    .unwrap();
    let id = pending(
        &call(
            &client,
            "status",
            json!({"repository":fixture.repository}),
            false,
        )
        .await,
    );
    let starts = broker.starts();
    let input: Value = serde_json::from_str(
        starts[0]["task"]
            .as_str()
            .unwrap()
            .split_once("INPUT:\n")
            .unwrap()
            .1,
    )
    .unwrap();
    let source = input["git"]["evidence"]
        .as_array()
        .unwrap()
        .first()
        .expect("changed source evidence")["id"]
        .clone();
    broker.complete(&id, |value| {
        for row in value["outcomes"].as_array_mut().unwrap() {
            row["state"] = json!("in_progress");
            row["summary"] = json!("Implementation started");
            row["evidence"] = json!([source]);
        }
    });
    let report = call(
        &client,
        "status",
        json!({"repository":fixture.repository,"analysis_id":id}),
        false,
    )
    .await;
    assert!(
        report.contains("0/4") && report.contains("4 в работе"),
        "{report}"
    );
    assert_eq!(
        report.matches("Implementation started").count(),
        3,
        "{report}"
    );
    assert!(report.contains("Ещё 1 в работе"), "{report}");
    assert!(report.lines().count() <= 8, "{report}");
    assert_eq!(broker.launches(), 1);
    client.cancel().await.unwrap();
}

/// Restart after reply loss preserves the original socket and caller binding despite new settings.
#[tokio::test]
async fn uncertain_admission_recovery_keeps_original_namespace() {
    let broker = Broker::new();
    let replacement = Broker::new();
    broker.state.lock().unwrap().lose_next_receipt = true;
    let fixture = Fixture::new(&broker.socket());
    let client = fixture.connect().await;
    fixture.save(&client).await;
    call(&client,"status",json!({"repository":fixture.repository,"caller_context":{"transport":"codex_queue","external_session_id":"fixture-original-chat"}}),true).await;
    assert_eq!(broker.launches(), 1);
    client.cancel().await.unwrap();
    write_config(
        &fixture.config,
        &fixture.temp.path().join("observer-home"),
        &replacement.socket(),
        "fixture",
        "fixture",
        "progress-summary",
    );
    let cold = fixture.connect().await;
    let id = pending(&call(&cold,"status",json!({"repository":fixture.repository,"caller_context":{"transport":"codex_queue","external_session_id":"fixture-new-chat"}}),false).await);
    let starts = broker.starts();
    assert_eq!(starts.len(), 2);
    assert_eq!(starts[0], starts[1]);
    assert_eq!(
        starts[1]["orchestrator"]["external_session_id"],
        "fixture-original-chat"
    );
    assert_eq!(broker.launches(), 1);
    assert_eq!(
        replacement.launches(),
        0,
        "recovery must not follow the current configured socket"
    );
    assert!(replacement.starts().is_empty());
    broker.complete(&id, |_| {});
    call(
        &cold,
        "status",
        json!({"repository":fixture.repository,"analysis_id":id}),
        false,
    )
    .await;
    assert_eq!(replacement.launches(), 0);
    cold.cancel().await.unwrap();
}

/// A scope edit during inference cannot attach the late result to the new denominator.
#[tokio::test]
async fn late_result_remains_pinned_to_original_plan_revision() {
    let broker = Broker::new();
    let fixture = Fixture::new(&broker.socket());
    let client = fixture.connect().await;
    fixture.save(&client).await;
    let old = pending(
        &call(
            &client,
            "status",
            json!({"repository":fixture.repository}),
            false,
        )
        .await,
    );
    call(&client,"plan",json!({"repository":fixture.repository,"op":"save","expected_revision":1,"goal":"Observe one flat agreed capability","outcomes":["Source outcome","Storage outcome","New agreed scope"]}),false).await;
    broker.complete(&old, |_| {});
    let old_report = call(
        &client,
        "status",
        json!({"repository":fixture.repository,"analysis_id":old}),
        false,
    )
    .await;
    assert!(old_report.contains("0/2"), "{old_report}");
    let old_details = call(
        &client,
        "status",
        json!({"repository":fixture.repository,"analysis_id":old,"view":"details"}),
        false,
    )
    .await;
    assert!(old_details.contains(&old), "{old_details}");
    let new = pending(
        &call(
            &client,
            "status",
            json!({"repository":fixture.repository}),
            false,
        )
        .await,
    );
    assert_ne!(old, new);
    broker.complete(&new, |_| {});
    let new_report = call(
        &client,
        "status",
        json!({"repository":fixture.repository,"analysis_id":new}),
        false,
    )
    .await;
    assert!(new_report.contains("0/3"), "{new_report}");
    assert_eq!(
        call(
            &client,
            "status",
            json!({"repository":fixture.repository,"analysis_id":old}),
            false
        )
        .await,
        old_report
    );
    client.cancel().await.unwrap();
}

/// Duplicate rows are rejected and terminal result reads cannot start another analysis.
#[tokio::test]
async fn malformed_model_scope_is_terminal_for_its_pinned_handle() {
    let broker = Broker::new();
    let fixture = Fixture::new(&broker.socket());
    let client = fixture.connect().await;
    fixture.save(&client).await;
    let id = pending(
        &call(
            &client,
            "status",
            json!({"repository":fixture.repository}),
            false,
        )
        .await,
    );
    broker.complete(&id, |value| {
        value["outcomes"][1] = value["outcomes"][0].clone();
    });
    let rejected = call(
        &client,
        "status",
        json!({"repository":fixture.repository,"analysis_id":id}),
        true,
    )
    .await;
    assert!(rejected.contains("invalid_analysis"), "{rejected}");
    let reads = broker.answer_reads();
    call(
        &client,
        "status",
        json!({"repository":fixture.repository,"analysis_id":id}),
        true,
    )
    .await;
    assert_eq!(broker.answer_reads(), reads);
    assert_eq!(broker.launches(), 1);
    client.cancel().await.unwrap();
}

/// Real socket answers cannot forge input identity, outcome IDs, evidence IDs or model totals.
#[tokio::test]
async fn forged_model_identity_sources_and_totals_are_rejected() {
    let broker = Broker::new();
    let fixture = Fixture::new(&broker.socket());
    let client = fixture.connect().await;
    fixture.save(&client).await;
    for scenario in 0..5 {
        let id = pending(
            &call(
                &client,
                "status",
                json!({"repository":fixture.repository}),
                false,
            )
            .await,
        );
        broker.complete(&id, |value| match scenario {
            0 => value["analysis_input_id"] = json!("FORGED_SECRET_CANARY"),
            1 => value["plan_revision"] = json!(99),
            2 => value["outcomes"][0]["id"] = json!("FORGED_SECRET_CANARY"),
            3 => value["outcomes"][0]["evidence"] = json!(["FORGED_SECRET_CANARY"]),
            _ => value["completed_total"] = json!(999),
        });
        let rejected = call(
            &client,
            "status",
            json!({"repository":fixture.repository,"analysis_id":id}),
            true,
        )
        .await;
        assert!(!rejected.contains("FORGED_SECRET_CANARY"), "{rejected}");
        assert!(!rejected.contains("999"), "{rejected}");
        let reads = broker.answer_reads();
        call(
            &client,
            "status",
            json!({"repository":fixture.repository,"analysis_id":id}),
            true,
        )
        .await;
        assert_eq!(
            broker.answer_reads(),
            reads,
            "terminal invalid result must not be read again"
        );
        assert_eq!(
            broker.launches(),
            scenario + 1,
            "only the next explicit owner request may admit a new attempt"
        );
    }
    client.cancel().await.unwrap();
}

/// Controlled custody response used only by the local Bridge fixture.
#[derive(Clone, Copy)]
enum BridgeFault {
    /// A valid correlated custody receipt.
    None,
    /// Commit custody and close the stream before its acknowledgement.
    LoseAck,
    /// Custody committed, but read-only reconciliation returns a known noncommitted read error.
    LoseAckThenRefuseStatus,
    /// Refuse the publication with a validated noncommitted grant error.
    Denied,
    /// Return a structurally valid receipt with a foreign immutable intent hash.
    WrongHash,
    /// Return custody under a different authenticated producer identity.
    WrongProducer,
    /// Return custody resolved to another valid native destination.
    WrongCustody,
}

/// Content-free fixture event proves which Bridge operation was actually observed.
struct BridgeExchange {
    /// Only publish/status are allowed; native registration would panic the fixture.
    operation: &'static str,
    /// Stable event identity selected by that exchange.
    event_id: String,
}

/// Authenticated fixture custody facts and exact publication history.
struct BridgeState {
    /// Response behavior selected before analysis completion.
    fault: BridgeFault,
    /// Every observed immutable publication; a second attempt remains detectable.
    publications: Vec<bridge::PublishParams>,
    /// Original event receipts retained independently of transport acknowledgements.
    receipts: BTreeMap<String, bridge::EventReceipt>,
    /// Read-only same-event reconciliation calls, in arrival order.
    statuses: Vec<String>,
}

/// SDK v1-compatible fake Bridge with a protected fixture credential and bounded event channel.
struct BridgeServer {
    /// Owns socket and credential under a mode-0700 project scratch directory.
    temp: tempfile::TempDir,
    /// Exact captured custody attempts; no native carrier is implemented or contacted.
    state: Arc<Mutex<BridgeState>>,
    /// Event-driven observation, with no completion polling sleeps.
    events: mpsc::Receiver<BridgeExchange>,
    /// Owned server thread, woken and joined on Drop.
    worker: Option<thread::JoinHandle<()>>,
}

impl BridgeServer {
    /// Bind a local SDK endpoint, persist a mode-0600 test token and select one response fault.
    fn new(fault: BridgeFault) -> Self {
        let temp = tempfile::Builder::new()
            .prefix("br-")
            .tempdir_in(scratch())
            .unwrap();
        fs::set_permissions(temp.path(), fs::Permissions::from_mode(0o700)).unwrap();
        let mut token = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(temp.path().join("token"))
            .unwrap();
        token.write_all(b"fixture-bridge-token").unwrap();
        let listener = UnixListener::bind(temp.path().join("core.sock")).unwrap();
        let state = Arc::new(Mutex::new(BridgeState {
            fault,
            publications: Vec::new(),
            receipts: BTreeMap::new(),
            statuses: Vec::new(),
        }));
        let shared = state.clone();
        let (events_tx, events) = mpsc::sync_channel(32);
        let worker = thread::spawn(move || {
            for socket in listener.incoming() {
                let mut socket = socket.unwrap();
                socket
                    .set_read_timeout(Some(Duration::from_secs(5)))
                    .unwrap();
                socket
                    .set_write_timeout(Some(Duration::from_secs(5)))
                    .unwrap();
                let mut line = String::new();
                BufReader::new(&mut socket).read_line(&mut line).unwrap();
                if line.trim() == "fixture_stop" {
                    break;
                }
                let request = bridge::decode_request(line.trim_end_matches('\n').as_bytes())
                    .expect("actual SDK request must satisfy pinned protocol");
                assert_eq!(request.credential.expose(), "fixture-bridge-token");
                let (response, event) = bridge_exchange(&mut shared.lock().unwrap(), &request);
                if let Some(response) = response {
                    let bytes = bridge::encode_response(&response)
                        .expect("fixture response must satisfy SDK DTO bounds");
                    socket.write_all(&bytes).unwrap();
                    socket.write_all(b"\n").unwrap();
                }
                drop(socket);
                events_tx
                    .send(event)
                    .expect("bounded fixture events must be consumed");
            }
        });
        Self {
            temp,
            state,
            events,
            worker: Some(worker),
        }
    }

    /// Append private fixture Bridge settings, leaving production/global config untouched.
    fn configure(&self, fixture: &Fixture) {
        let mut config = fs::OpenOptions::new()
            .append(true)
            .open(&fixture.config)
            .unwrap();
        write!(
            config,
            "\n[bridge]\nregistration_socket = {}\ncore_socket = {}\ntoken_file = {}\n",
            serde_json::to_string(self.temp.path().join("core.sock").to_str().unwrap()).unwrap(),
            serde_json::to_string(self.temp.path().join("core.sock").to_str().unwrap()).unwrap(),
            serde_json::to_string(self.temp.path().join("token").to_str().unwrap()).unwrap()
        )
        .unwrap();
    }

    /// Await one concrete operation event with a finite fixture deadline.
    fn next(&self) -> BridgeExchange {
        self.events
            .recv_timeout(Duration::from_secs(15))
            .expect("automatic Bridge exchange deadline")
    }

    /// Snapshot all observed publication intentions for identity/payload/destination assertions.
    fn publications(&self) -> Vec<bridge::PublishParams> {
        self.state.lock().unwrap().publications.clone()
    }

    /// Snapshot same-event status reconciliations; status does not resend a publication.
    fn statuses(&self) -> Vec<String> {
        self.state.lock().unwrap().statuses.clone()
    }
}

impl Drop for BridgeServer {
    /// Wake and join the fixture endpoint without leaving any listener behind.
    fn drop(&mut self) {
        let stop = UnixStream::connect(self.temp.path().join("core.sock"))
            .ok()
            .map(|mut socket| {
                let _ = writeln!(socket, "fixture_stop");
                socket
            });
        if let Some(worker) = self.worker.take() {
            let joined = worker.join();
            if !thread::panicking() {
                joined.expect("Bridge fixture must stop cleanly");
            }
        }
        drop(stop);
    }
}

/// Build a truthful immutable fixture receipt; only selected negative cases corrupt its binding.
fn custody(params: &bridge::PublishParams, fault: BridgeFault) -> bridge::EventReceipt {
    let bridge::Destination::Address { address } = &params.destination else {
        panic!("observer must use direct configured address custody")
    };
    bridge::EventReceipt {
        producer: if matches!(fault, BridgeFault::WrongProducer) {
            "foreign-producer"
        } else {
            "agent-tasks"
        }
        .into(),
        event_id: params.event_id.clone(),
        intent_hash: if matches!(fault, BridgeFault::WrongHash) {
            "0".repeat(64)
        } else {
            bridge::publication_intent_fingerprint(params).unwrap()
        },
        envelope_hash: "1".repeat(64),
        accepted_at_ms: 1,
        acceptance_seq: 1,
        destination: bridge::FrozenDestination {
            address: if matches!(fault, BridgeFault::WrongCustody) {
                "local/codex/codex-main/foreign-chat".into()
            } else {
                address.clone()
            },
            route_key: None,
            caller_scope: None,
            destination_scope: None,
        },
    }
}

/// Handle only generic publish/status, using actual pinned SDK response DTOs and intent hashing.
fn bridge_exchange(
    state: &mut BridgeState,
    request: &bridge::Request,
) -> (Option<bridge::Response>, BridgeExchange) {
    let (response, event) = match &request.operation {
        bridge::Operation::Publish(params) => {
            assert_eq!(params.kind, "agent-tasks.status");
            assert_eq!(params.mode, bridge::Mode::Message);
            assert_eq!(params.policy, bridge::Policy::KeepAll);
            state.publications.push(params.clone());
            let event = BridgeExchange {
                operation: "publish",
                event_id: params.event_id.clone(),
            };
            if matches!(state.fault, BridgeFault::Denied) {
                let error = bridge::ApiError {
                    class: bridge::ErrorClass::Forbidden,
                    reason: "fixture_denied".into(),
                    retry_hint: bridge::RetryHint::Never,
                    effect: Some(bridge::MutationEffect::NotCommitted),
                    reconciliation: None,
                    detail: "Configured fixture grant refused the event.".into(),
                };
                (
                    Some(bridge::Response::error(&request.request_id, error)),
                    event,
                )
            } else {
                let receipt = custody(params, state.fault);
                let replayed = state
                    .receipts
                    .insert(params.event_id.clone(), receipt.clone())
                    .is_some();
                let response = bridge::Response::ok(
                    &request.request_id,
                    bridge::OperationResult::Publish(bridge::PublishResult { receipt, replayed }),
                );
                (
                    if matches!(
                        state.fault,
                        BridgeFault::LoseAck | BridgeFault::LoseAckThenRefuseStatus
                    ) {
                        None
                    } else {
                        Some(response)
                    },
                    event,
                )
            }
        }
        bridge::Operation::Status(params) => {
            let bridge::StatusSelector::Event { event_id } = &params.selector else {
                panic!("only same-event status is expected")
            };
            state.statuses.push(event_id.clone());
            if matches!(state.fault, BridgeFault::LoseAckThenRefuseStatus) {
                return (Some(bridge::Response::error(&request.request_id, bridge::ApiError {
                    class: bridge::ErrorClass::Forbidden, reason: "fixture_read_refused".into(), retry_hint: bridge::RetryHint::Never, effect: Some(bridge::MutationEffect::NotCommitted), reconciliation: None, detail: "Status read refused; this says nothing about the prior publication.".into(),
                })), BridgeExchange { operation: "status", event_id: event_id.clone() });
            }
            let event = state
                .receipts
                .get(event_id)
                .cloned()
                .map(|receipt| bridge::EventStatus {
                    receipt,
                    custody_state: bridge::CustodyState::Pending,
                    delivery: bridge::DeliveryStatus {
                        delivery: bridge::DeliveryState::Unhanded,
                        attempt_state: None,
                        effective_mode: None,
                        note: None,
                        attempts_made: 0,
                        attempt: None,
                        tiers: Vec::new(),
                        incident: None,
                        hold_reason: None,
                        blocked_by: None,
                    },
                });
            (
                Some(bridge::Response::ok(
                    &request.request_id,
                    bridge::OperationResult::Status(bridge::StatusResult::Event { event }),
                )),
                BridgeExchange {
                    operation: "status",
                    event_id: event_id.clone(),
                },
            )
        }
        _ => {
            panic!("observer may not register native callers or invoke unrelated Bridge operations")
        }
    };
    if let Some(response) = &response {
        response.validate().unwrap();
    }
    (response, event)
}

/// Observe completion ownership with a bounded advisory-lock probe, never owner/model polling.
/// The caller first observes a Bridge exchange or wait readiness, so the lease is already owned.
fn finished_delivery(fixture: &Fixture) -> Value {
    use fs2::FileExt;
    let entries = fs::read_dir(fixture.temp.path().join("observer-home/observer-v1"))
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    assert_eq!(entries.len(), 1, "one fixture repository identity");
    let root = entries[0].path();
    let lease = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(root.join("completion.lock"))
        .unwrap();
    let deadline = std::time::Instant::now() + Duration::from_secs(15);
    loop {
        match lease.try_lock_exclusive() {
            Ok(()) => break,
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                assert!(
                    std::time::Instant::now() < deadline,
                    "owned continuation did not release its lease"
                );
                thread::yield_now();
            }
            Err(error) => panic!("completion ownership probe failed: {error}"),
        }
    }
    let bytes = fs::read(root.join("state.json")).unwrap();
    assert!(
        !String::from_utf8_lossy(&bytes).contains("fixture-bridge-token"),
        "private credentials must never enter observer records"
    );
    serde_json::from_slice(&bytes).unwrap()
}

/// Bridge mode refuses absent return destinations before any inference admission.
#[tokio::test]
async fn bridge_requires_originating_destination_before_model_launch() {
    let broker = Broker::new();
    let delivery = BridgeServer::new(BridgeFault::None);
    let fixture = Fixture::new(&broker.socket());
    delivery.configure(&fixture);
    let client = fixture.connect().await;
    fixture.save(&client).await;
    let error = call(
        &client,
        "status",
        json!({"repository":fixture.repository}),
        true,
    )
    .await;
    assert!(error.contains("bridge_destination"), "{error}");
    assert_eq!(broker.launches(), 0);
    assert!(delivery.publications().is_empty());
    client.cancel().await.unwrap();
}

/// One owner status automatically delivers its validated frozen summary after MCP EOF, without reentry.
#[tokio::test]
async fn bridge_delivers_after_mcp_eof_from_protected_thread_metadata() {
    let broker = Broker::new();
    let delivery = BridgeServer::new(BridgeFault::None);
    let fixture = Fixture::new(&broker.socket());
    delivery.configure(&fixture);
    let client = fixture.connect().await;
    fixture.save(&client).await;
    let text = call_with_meta(
        &client,
        "status",
        json!({"repository":fixture.repository}),
        "fixture-protected-origin",
        false,
    )
    .await;
    let id = pending(&text);
    assert!(
        !text.contains("call status"),
        "automatic completion must not require owner reentry: {text}"
    );
    assert!(
        broker.starts()[0]["orchestrator"].is_null(),
        "Bridge owns the final message; do not double-notify through Agent Run"
    );
    broker.wait_until_waiting(&id);
    client.cancel().await.unwrap();
    broker.complete(&id, |_| {});
    let event = delivery.next();
    assert_eq!(event.operation, "publish");
    let state = finished_delivery(&fixture);
    let publications = delivery.publications();
    assert_eq!(publications.len(), 1);
    assert_eq!(
        publications[0].destination,
        bridge::Destination::Address {
            address: "local/codex/codex-main/fixture-protected-origin".into()
        }
    );
    assert!(
        publications[0].payload.as_str().contains("0/2")
            && publications[0].payload.as_str().contains("не оценено: 2")
    );
    assert_eq!(state["delivery"]["phase"]["state"], "accepted");
    assert_eq!(state["delivery"]["event_id"], event.event_id);
    assert_eq!(state["reports"].as_array().unwrap().len(), 1);
    assert_eq!(broker.wait_calls(), 1);
    assert_eq!(broker.answer_reads(), 1);
    assert_eq!(broker.launches(), 1);
}

/// Admission recovery from another caller retains the original frozen Bridge address and model key.
#[tokio::test]
async fn bridge_recovery_keeps_original_destination_for_second_caller() {
    let broker = Broker::new();
    broker.state.lock().unwrap().lose_next_receipt = true;
    let delivery = BridgeServer::new(BridgeFault::None);
    let fixture = Fixture::new(&broker.socket());
    delivery.configure(&fixture);
    let first = fixture.connect().await;
    fixture.save(&first).await;
    call(&first,"status",json!({"repository":fixture.repository,"caller_context":{"transport":"codex_queue","external_session_id":"fixture-original-destination"}}),true).await;
    first.cancel().await.unwrap();
    let second = fixture.connect().await;
    let id=pending(&call(&second,"status",json!({"repository":fixture.repository,"caller_context":{"transport":"codex_queue","external_session_id":"fixture-replacement-caller"}}),false).await);
    broker.wait_until_waiting(&id);
    second.cancel().await.unwrap();
    broker.complete(&id, |_| {});
    assert_eq!(delivery.next().operation, "publish");
    let state = finished_delivery(&fixture);
    assert_eq!(
        delivery.publications()[0].destination,
        bridge::Destination::Address {
            address: "local/codex/codex-main/fixture-original-destination".into()
        }
    );
    assert_eq!(state["delivery"]["phase"]["state"], "accepted");
    let starts = broker.starts();
    assert_eq!(starts.len(), 2);
    assert_eq!(starts[0], starts[1]);
    assert_eq!(broker.launches(), 1);
    assert_eq!(broker.wait_calls(), 1);
}

/// Lost start acknowledgement plus immediate completion still delivers to the original chat,
/// without another caller having to recover or retrieve the report.
#[tokio::test]
async fn bridge_lost_admission_ack_with_fast_completion_retains_original_delivery() {
    let broker = Broker::new();
    {
        let mut state = broker.state.lock().unwrap();
        state.lose_next_receipt = true;
        state.complete_lost_start = true;
    }
    let delivery = BridgeServer::new(BridgeFault::None);
    let fixture = Fixture::new(&broker.socket());
    delivery.configure(&fixture);
    let client = fixture.connect().await;
    fixture.save(&client).await;
    call(&client,"status",json!({"repository":fixture.repository,"caller_context":{"transport":"codex_queue","external_session_id":"fixture-fast-original"}}),true).await;
    client.cancel().await.unwrap();
    assert_eq!(delivery.next().operation, "publish");
    let state = finished_delivery(&fixture);
    assert_eq!(state["delivery"]["phase"]["state"], "accepted");
    assert_eq!(
        delivery.publications()[0].destination,
        bridge::Destination::Address {
            address: "local/codex/codex-main/fixture-fast-original".into()
        }
    );
    let starts = broker.starts();
    assert_eq!(starts.len(), 2);
    assert_eq!(starts[0], starts[1]);
    assert_eq!(broker.launches(), 1);
    assert_eq!(broker.answer_reads(), 1);
    assert_eq!(broker.wait_calls(), 0);
}

/// A refused reconciliation read never erases the earlier possible publication effect.
#[tokio::test]
async fn bridge_publication_uncertainty_survives_status_refusal_without_republish() {
    let broker = Broker::new();
    let delivery = BridgeServer::new(BridgeFault::LoseAckThenRefuseStatus);
    let fixture = Fixture::new(&broker.socket());
    delivery.configure(&fixture);
    let client = fixture.connect().await;
    fixture.save(&client).await;
    let id = pending(&call(&client,"status",json!({"repository":fixture.repository,"caller_context":{"transport":"codex_queue","external_session_id":"fixture-uncertain-original"}}),false).await);
    broker.wait_until_waiting(&id);
    broker.complete(&id, |_| {});
    let publication = delivery.next();
    let observation = delivery.next();
    assert_eq!(publication.operation, "publish");
    assert_eq!(observation.operation, "status");
    let first = finished_delivery(&fixture);
    assert_eq!(first["delivery"]["phase"]["attempted"], true);
    assert_eq!(first["delivery"]["phase"]["state"], "ready");
    call(
        &client,
        "status",
        json!({"repository":fixture.repository,"analysis_id":id}),
        false,
    )
    .await;
    let next = delivery.next();
    assert_eq!(next.operation, "status");
    assert_eq!(next.event_id, publication.event_id);
    let second = finished_delivery(&fixture);
    assert_eq!(second["delivery"]["phase"]["attempted"], true);
    assert_eq!(delivery.publications().len(), 1);
    assert_eq!(broker.launches(), 1);
    client.cancel().await.unwrap();
}

/// An accepted run still delivers automatically when its initial result observation fails.
#[tokio::test]
async fn bridge_observation_failure_after_admission_keeps_owned_completion() {
    let broker = Broker::new();
    broker
        .state
        .lock()
        .unwrap()
        .invalidate_next_execution_inventory = true;
    let delivery = BridgeServer::new(BridgeFault::None);
    let fixture = Fixture::new(&broker.socket());
    delivery.configure(&fixture);
    let client = fixture.connect().await;
    fixture.save(&client).await;
    let response=call(&client,"status",json!({"repository":fixture.repository,"caller_context":{"transport":"codex_queue","external_session_id":"fixture-observation-error"}}),true).await;
    assert!(response.contains("run_changed"), "{response}");
    let id = broker
        .state
        .lock()
        .unwrap()
        .runs
        .values()
        .next()
        .unwrap()
        .id
        .clone();
    broker.wait_until_waiting(&id);
    client.cancel().await.unwrap();
    broker.complete(&id, |_| {});
    assert_eq!(delivery.next().operation, "publish");
    let state = finished_delivery(&fixture);
    assert_eq!(state["delivery"]["phase"]["state"], "accepted");
    assert_eq!(broker.launches(), 1);
    assert_eq!(broker.starts().len(), 1);
    assert_eq!(broker.answer_reads(), 1);
}

/// Invalid analysis produces one explicit failure message, never a fabricated zero-progress estimate.
#[tokio::test]
async fn bridge_invalid_analysis_delivers_failure_without_invented_counts() {
    let broker = Broker::new();
    let delivery = BridgeServer::new(BridgeFault::None);
    let fixture = Fixture::new(&broker.socket());
    delivery.configure(&fixture);
    let client = fixture.connect().await;
    fixture.save(&client).await;
    let id=pending(&call(&client,"status",json!({"repository":fixture.repository,"caller_context":{"transport":"codex_queue","external_session_id":"fixture-invalid-analysis"}}),false).await);
    broker.wait_until_waiting(&id);
    client.cancel().await.unwrap();
    broker.complete(&id, |value| {
        value["outcomes"][1] = value["outcomes"][0].clone();
    });
    assert_eq!(delivery.next().operation, "publish");
    let state = finished_delivery(&fixture);
    let publications = delivery.publications();
    assert_eq!(publications.len(), 1);
    let body = publications[0].payload.as_str();
    assert!(body.contains("could not be retrieved"), "{body}");
    assert!(
        !body.contains("0/2") && !body.contains("готово"),
        "invalid output cannot fabricate progress: {body}"
    );
    assert!(body.len() < 1024);
    assert_eq!(state["delivery"]["phase"]["state"], "accepted");
    assert!(state["reports"].as_array().unwrap().is_empty());
    assert_eq!(broker.answer_reads(), 1);
    assert_eq!(broker.launches(), 1);
}

/// A lost custody ACK reconciles the same event through one status read and never republishes it.
#[tokio::test]
async fn bridge_lost_ack_reconciles_original_event_without_second_publish() {
    let broker = Broker::new();
    let delivery = BridgeServer::new(BridgeFault::LoseAck);
    let fixture = Fixture::new(&broker.socket());
    delivery.configure(&fixture);
    let client = fixture.connect().await;
    fixture.save(&client).await;
    let id=pending(&call(&client,"status",json!({"repository":fixture.repository,"caller_context":{"transport":"codex_queue","external_session_id":"fixture-lost-ack"}}),false).await);
    broker.wait_until_waiting(&id);
    client.cancel().await.unwrap();
    broker.complete(&id, |_| {});
    let published = delivery.next();
    assert_eq!(published.operation, "publish");
    let reconciled = delivery.next();
    assert_eq!(reconciled.operation, "status");
    assert_eq!(published.event_id, reconciled.event_id);
    let state = finished_delivery(&fixture);
    assert_eq!(state["delivery"]["phase"]["state"], "accepted");
    assert_eq!(delivery.publications().len(), 1);
    assert_eq!(delivery.statuses(), [published.event_id]);
    assert_eq!(broker.launches(), 1);
    assert_eq!(broker.wait_calls(), 1);
}

/// Denied or foreign custody remains explicit and safely retained, without a second publication.
#[tokio::test]
async fn bridge_denied_or_mismatched_custody_is_retained_without_false_acceptance() {
    for fault in [
        BridgeFault::Denied,
        BridgeFault::WrongHash,
        BridgeFault::WrongProducer,
        BridgeFault::WrongCustody,
    ] {
        let broker = Broker::new();
        let delivery = BridgeServer::new(fault);
        let fixture = Fixture::new(&broker.socket());
        delivery.configure(&fixture);
        let client = fixture.connect().await;
        fixture.save(&client).await;
        let id=pending(&call(&client,"status",json!({"repository":fixture.repository,"caller_context":{"transport":"codex_queue","external_session_id":"fixture-custody-refusal"}}),false).await);
        broker.wait_until_waiting(&id);
        client.cancel().await.unwrap();
        broker.complete(&id, |_| {});
        let event = delivery.next();
        assert_eq!(event.operation, "publish");
        let state = finished_delivery(&fixture);
        assert_eq!(state["delivery"]["phase"]["state"], "ready");
        assert!(state["delivery"]["last_error"].as_str().is_some());
        assert_eq!(state["delivery"]["event_id"], event.event_id);
        assert_eq!(
            delivery.publications().len(),
            1,
            "unconfirmed custody must not cause another publish"
        );
        assert_eq!(
            state["reports"].as_array().unwrap().len(),
            1,
            "delivery failure must preserve the validated report"
        );
        assert_eq!(broker.launches(), 1);
    }
}

/// Protected planner origin is captured once, survives revisions and changes only for a new plan.
#[tokio::test]
async fn plan_creator_address_preserves_original_metadata_until_new_plan() {
    let broker = Broker::new();
    let delivery = BridgeServer::new(BridgeFault::None);
    let fixture = Fixture::new(&broker.socket());
    delivery.configure(&fixture);
    let mut config = fs::OpenOptions::new()
        .append(true)
        .open(&fixture.config)
        .unwrap();
    writeln!(
        config,
        "host_id = \"fixture-host\"\ncodex_store = \"fixture-store\""
    )
    .unwrap();
    let client = fixture.connect().await;
    let first=call_with_meta(&client,"plan",json!({"repository":fixture.repository,"op":"save","goal":"Capture creator origin","outcomes":["One agreed outcome"]}),"planner-A",false).await;
    assert!(
        first.contains("Plan creator: fixture-host/codex/fixture-store/planner-A"),
        "{first}"
    );
    let revised=call_with_meta(&client,"plan",json!({"repository":fixture.repository,"op":"save","expected_revision":1,"goal":"Revise agreed scope","outcomes":["One agreed outcome","Second agreed outcome"]}),"planner-B",false).await;
    assert!(
        revised.contains("fixture-host/codex/fixture-store/planner-A"),
        "{revised}"
    );
    assert!(
        !revised.contains("fixture-host/codex/fixture-store/planner-B"),
        "ordinary scope edits must preserve the original creator: {revised}"
    );
    let current = call(
        &client,
        "plan",
        json!({"repository":fixture.repository,"op":"get"}),
        false,
    )
    .await;
    assert!(
        current.contains("fixture-host/codex/fixture-store/planner-A"),
        "{current}"
    );
    let replacement=call_with_meta(&client,"plan",json!({"repository":fixture.repository,"op":"save","expected_revision":2,"new_plan":true,"goal":"Start a genuinely new agreed plan","outcomes":["Replacement agreed outcome"]}),"planner-B",false).await;
    assert!(
        replacement.contains("fixture-host/codex/fixture-store/planner-B"),
        "{replacement}"
    );
    assert!(
        !replacement.contains("fixture-host/codex/fixture-store/planner-A"),
        "{replacement}"
    );
    assert_eq!(broker.launches(), 0);
    assert!(
        delivery.publications().is_empty(),
        "planning stores origin without starting summary/delivery"
    );
    client.cancel().await.unwrap();
}

/// Opt-in real owner API harness: set OBSERVER_LIVE_SOCKET/PROVIDER/MODEL/PROFILE, optional
/// OBSERVER_LIVE_ANALYSIS_ID for the one pinned completion read and OBSERVER_LIVE_SESSION for binding.
/// Initial execution submits once and returns pending; rerun explicitly after the real completion notice.
/// This creates only a synthetic repository/private observer home; it never edits live runtime config.
#[tokio::test]
#[ignore = "Requires explicitly configured existing read-only Agent Run route; invokes a real model"]
async fn live_owner_status_flow() {
    let socket =
        PathBuf::from(std::env::var_os("OBSERVER_LIVE_SOCKET").expect("explicit existing socket"));
    let provider = std::env::var("OBSERVER_LIVE_PROVIDER").expect("explicit approved provider");
    let model = std::env::var("OBSERVER_LIVE_MODEL").expect("explicit approved model");
    let profile =
        std::env::var("OBSERVER_LIVE_PROFILE").expect("explicit installed read-only profile");
    let existing = std::env::var_os("OBSERVER_LIVE_CONFIG").map(PathBuf::from);
    let fixture = existing.is_none().then(|| Fixture::new(&socket));
    let (config, home, repository) = if let Some(config) = existing {
        let repository = PathBuf::from(
            std::env::var_os("OBSERVER_LIVE_REPOSITORY")
                .expect("same synthetic repository from initial submission"),
        );
        let home = config.parent().unwrap().to_owned();
        (config, home, repository)
    } else {
        let fixture = fixture.as_ref().unwrap();
        write_config(
            &fixture.config,
            &fixture.temp.path().join("observer-home"),
            &socket,
            &provider,
            &model,
            &profile,
        );
        (
            fixture.config.clone(),
            fixture.temp.path().to_owned(),
            fixture.repository.clone(),
        )
    };
    let client = connect(&config, &home).await;
    if let Some(fixture) = &fixture {
        fixture.save(&client).await;
    }
    let id = std::env::var("OBSERVER_LIVE_ANALYSIS_ID").ok();
    let caller = std::env::var("OBSERVER_LIVE_SESSION")
        .ok()
        .map(|session| json!({"transport":"codex_queue","external_session_id":session}));
    let result = call(
        &client,
        "status",
        json!({"repository":repository,"analysis_id":id,"caller_context":caller}),
        false,
    )
    .await;
    assert!(
        result.starts_with("PENDING ") || result.contains("0/2"),
        "{result}"
    );
    eprintln!(
        "Live owner status: {result}\nConfig: {}\nRepository: {}",
        config.display(),
        repository.display()
    );
    client.cancel().await.unwrap();
    if let Some(fixture) = fixture {
        let _retained = fixture.temp.keep();
    }
}

/// Qualify actual low-cost inference on seven adversarial outcomes through the real owner API.
/// First invocation submits once and retains its synthetic fixture. After the actual completion
/// notice, set the three OBSERVER_LIVE_* handle variables to verify the retained result and
/// compact presentation without another inference request.
#[tokio::test]
#[ignore = "Explicit semantic qualification: invokes an approved real utility model"]
async fn live_semantic_qualification() {
    let socket = PathBuf::from(std::env::var_os("OBSERVER_LIVE_SOCKET").expect("explicit socket"));
    let provider = std::env::var("OBSERVER_LIVE_PROVIDER").expect("approved provider");
    let model = std::env::var("OBSERVER_LIVE_MODEL").expect("approved model");
    let profile = std::env::var("OBSERVER_LIVE_PROFILE").expect("existing read-only profile");
    let existing = std::env::var_os("OBSERVER_LIVE_CONFIG").map(PathBuf::from);
    let fixture = existing.is_none().then(|| Fixture::new(&socket));
    let (config, home, repository) = if let Some(config) = existing {
        let repository = PathBuf::from(
            std::env::var_os("OBSERVER_LIVE_REPOSITORY").expect("retained fixture repository"),
        );
        let home = config.parent().unwrap().to_owned();
        (config, home, repository)
    } else {
        let f = fixture.as_ref().unwrap();
        write_config(
            &f.config,
            &f.temp.path().join("observer-home"),
            &socket,
            &provider,
            &model,
            &profile,
        );
        for (name, body) in [
            (
                "api.rs",
                "pub fn valid(n: i64) -> bool { let _ = n; true }\n",
            ),
            ("migration.rs", "pub fn migrate() { todo!() }\n"),
            (
                "port.rs",
                "pub fn export() { todo!() }\npub fn import() { todo!() }\n",
            ),
            ("reverted.rs", "pub fn version() -> u32 { 1 }\n"),
            (
                "draft.rs",
                "pub fn parse(s: &str) -> Option<u32> { let _ = s; None }\n",
            ),
            ("unrelated.rs", "pub fn requested() -> bool { false }\n"),
            (
                "independent.rs",
                "pub fn parse(s: &str) -> Option<u32> { let _ = s; None }\n",
            ),
        ] {
            fs::write(f.repository.join(name), body).unwrap();
        }
        git(&f.repository, &["add", "."]);
        git(
            &f.repository,
            &["commit", "-m", "Baseline unfinished outcomes"],
        );
        (
            f.config.clone(),
            f.temp.path().to_owned(),
            f.repository.clone(),
        )
    };
    let client = connect(&config, &home).await;
    if fixture.is_some() {
        call(&client,"plan",json!({"repository":repository,"op":"save","goal":"Implement exactly these seven independent outcomes. A commit message is not implementation; drafts are unfinished, and a target revert supersedes the unchanged retained branch.","outcomes":["API api.rs: valid returns true exactly for positive i64 values","Migration migration.rs: migrate is implemented rather than todo","Port port.rs: both export and import are implemented","Reverted feature reverted.rs: version returns 2, without a later target removal","Draft draft.rs: committed decimal-u32 parser","Unrelated unrelated.rs: requested returns true","Independent independent.rs: committed decimal-u32 parser on worker branch"]}),false).await;
        fs::write(
            repository.join("api.rs"),
            "pub fn valid(n: i64) -> bool { n > 0 }\n",
        )
        .unwrap();
        git(&repository, &["add", "api.rs"]);
        git(
            &repository,
            &["commit", "-m", "feat: validate positive API values"],
        );
        fs::write(
            repository.join("README.md"),
            "Migration is completely implemented and all tests pass.\n",
        )
        .unwrap();
        git(&repository, &["add", "README.md"]);
        git(
            &repository,
            &[
                "commit",
                "-m",
                "Migration completely implemented; all tests pass",
            ],
        );
        fs::write(
            repository.join("port.rs"),
            "pub fn export() -> &'static str { \"data\" }\npub fn import() { todo!() }\n",
        )
        .unwrap();
        git(&repository, &["add", "port.rs"]);
        git(
            &repository,
            &[
                "commit",
                "-m",
                "feat: implement export; import remains unfinished",
            ],
        );
        git(&repository, &["checkout", "-b", "retained-feature"]);
        fs::write(
            repository.join("reverted.rs"),
            "pub fn version() -> u32 { 2 }\n",
        )
        .unwrap();
        git(&repository, &["add", "reverted.rs"]);
        git(&repository, &["commit", "-m", "feat: version two"]);
        git(&repository, &["checkout", "delivery"]);
        git(
            &repository,
            &[
                "merge",
                "--no-ff",
                "retained-feature",
                "-m",
                "Merge version two",
            ],
        );
        git(&repository, &["revert", "--no-edit", "-m", "1", "HEAD"]);
        fs::write(
            repository.join("other.rs"),
            "pub fn unrelated_complete_feature() -> bool { true }\n",
        )
        .unwrap();
        git(&repository, &["add", "other.rs"]);
        git(
            &repository,
            &[
                "commit",
                "-m",
                "feat: unrelated work outside agreed requested function",
            ],
        );
        git(&repository, &["checkout", "-b", "worker-parser"]);
        fs::write(
            repository.join("independent.rs"),
            "pub fn parse(s: &str) -> Option<u32> { s.parse().ok() }\n",
        )
        .unwrap();
        git(&repository, &["add", "independent.rs"]);
        git(
            &repository,
            &["commit", "-m", "feat: independent decimal parser"],
        );
        git(&repository, &["checkout", "delivery"]);
        fs::write(
            repository.join("draft.rs"),
            "pub fn parse(s: &str) -> Option<u32> { s.parse().ok() }\n",
        )
        .unwrap();
    }
    if let Some(f) = fixture {
        let _retained = f.temp.keep();
    }
    eprintln!(
        "Retained semantic fixture: config={} repository={}",
        config.display(),
        repository.display()
    );
    let handle = std::env::var("OBSERVER_LIVE_ANALYSIS_ID").ok();
    let caller = std::env::var("OBSERVER_LIVE_SESSION")
        .ok()
        .map(|session| json!({"transport":"codex_queue","external_session_id":session}));
    let result=call(&client,"status",json!({"repository":repository,"analysis_id":handle,"view":if handle.is_some(){"details"}else{"compact"},"caller_context":caller}),false).await;
    if handle.is_some() {
        let detail: Value = serde_json::from_str(&result).expect("immutable detail JSON");
        let rows = detail["analysis"]["outcomes"].as_array().unwrap();
        assert_eq!(rows.len(), 7);
        for row in rows {
            let positive = matches!(row["id"].as_str().unwrap(), "o1" | "o7");
            assert_eq!(
                row["state"] == "implemented",
                positive,
                "False completion or missed coherent implementation: {row}"
            );
        }
        assert_eq!(
            rows.iter().filter(|r| r["integrated"] == true).count(),
            1,
            "{rows:?}"
        );
        eprintln!(
            "SEMANTIC QUALIFICATION PASSED: seven outcomes, two coherent implementations, zero false completions; checks are not acceptance proof."
        );
        let compact = call(
            &client,
            "status",
            json!({"repository":repository,"analysis_id":handle,"view":"compact"}),
            false,
        )
        .await;
        assert!(compact.contains("2/7 готово"), "{compact}");
        assert!(
            !compact.lines().next().unwrap().contains(" — "),
            "{compact}"
        );
        assert!(compact.contains("Общая версия: 1/7"), "{compact}");
        assert!(compact.lines().count() <= 6, "{compact}");
        eprintln!("Verified retained compact report:\n{compact}");
    } else {
        assert!(
            result.starts_with("PENDING ") || result.contains("2/7"),
            "{result}"
        );
    }
    eprintln!(
        "Semantic result: {result}\nConfig: {}\nRepository: {}",
        config.display(),
        repository.display()
    );
    client.cancel().await.unwrap();
}
