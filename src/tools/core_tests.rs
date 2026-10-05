//! Focused contract, native filesystem and work-cycle regressions for the minimal core.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "Fixture construction and explicit assertions"
)]
use super::input;
use crate::{
    model::*,
    response::Templates,
    store::{self, Config, Store},
};
use mcp_presentation::Renderer;
use serde_json::{Value, json};
use std::{fs, path::PathBuf};

/// One disposable portable root and lazy operator configuration; no installed state is touched.
struct Fixture {
    /// Keep all test-owned files alive through assertions.
    directory: tempfile::TempDir,
    /// Configured final store folder, initially absent.
    root: PathBuf,
    /// Lazy location selection.
    config: Config,
    /// Trusted normal templates.
    templates: Templates,
    /// Identity renderer used by the actual registry router.
    identity: Renderer,
}
impl Fixture {
    /// Build aliases alpha/same for the same root without creating that root.
    fn new() -> Self {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path().join("docs");
        let config_path = directory.path().join("config.toml");
        fs::write(
            &config_path,
            format!(
                "schema_version = 1\n[aliases]\nalpha = {}\nsame = {}\n",
                serde_json::to_string(&root).unwrap(),
                serde_json::to_string(&root).unwrap()
            ),
        )
        .unwrap();
        Self {
            directory,
            root,
            config: Config::new(Some(config_path)),
            templates: Templates::new(&super::templates()).unwrap(),
            identity: Renderer::new().unwrap(),
        }
    }
    /// Execute the real tool route, asserting bounded single text and expected isError.
    async fn call(&self, name: &str, args: Value, error: bool) -> String {
        let reply = super::call(name, args, &self.identity, &self.templates, &self.config)
            .await
            .unwrap();
        assert_eq!(reply.is_error, Some(error), "{reply:?}");
        let wire = serde_json::to_value(reply).unwrap();
        assert_eq!(wire["content"].as_array().unwrap().len(), 1);
        let text = wire["content"][0]["text"].as_str().unwrap().to_owned();
        assert!(text.len() <= 8192, "{}", text.len());
        assert!(
            !text.contains("schema_version:"),
            "Raw storage escaped the projection"
        );
        text
    }
    /// Resolve this request's root identity just as production does.
    fn store(&self) -> Store {
        self.config.resolve("alpha").unwrap()
    }
    /// Use a real read-only context to obtain creation preconditions.
    async fn allocation(&self) -> String {
        field(
            &self
                .call("get_context", json!({"project":"alpha"}), false)
                .await,
            "Allocation version: ",
        )
    }
    /// Explicitly initialize the previously absent root.
    async fn init(&self) {
        let version = self.allocation().await;
        assert!(!self.root.exists());
        self.call("plan_work",json!({"project":"alpha","op":"init_project","version":version,"title":"Portable work","purpose":"Help agents preserve strategic intent"}),false).await;
    }
    /// Create one module via the registered API with caller-supplied semantic task content.
    async fn module(&self, tasks: Vec<Value>, lead: Option<Value>) -> String {
        let version = self.allocation().await;
        let reply=self.call("plan_work",json!({"project":"alpha","op":"create_module","version":version,"title":"Portable storage","outcome":"Preserve reported work without data loss","lead":lead,"tasks":tasks}),false).await;
        reply
            .lines()
            .next()
            .unwrap()
            .split_whitespace()
            .nth(1)
            .unwrap()
            .to_owned()
    }
    /// Read a module's exact bytes/version from its context, never an invented precondition.
    async fn version(&self, id: &str) -> String {
        field(
            &self
                .call("get_context", json!({"project":"alpha","ref":id}), false)
                .await,
            "Version: ",
        )
    }
    /// Add the current project/version/reference to one record operation.
    async fn record(&self, reference: &str, mut args: Value, error: bool) -> String {
        args["project"] = json!("alpha");
        args["ref"] = json!(reference);
        args["version"] = json!(self.version(reference.split('/').next().unwrap()).await);
        self.call("record_work", args, error).await
    }
    /// Plan a partial module/task edit with fresh owner-file version.
    async fn plan(&self, id: &str, mut args: Value, error: bool) -> String {
        args["project"] = json!("alpha");
        args["version"] = json!(self.version(id).await);
        self.call("plan_work", args, error).await
    }
    /// Record an independent verdict using current context.
    async fn review(&self, id: &str, mut args: Value, error: bool) -> String {
        args["project"] = json!("alpha");
        args["module"] = json!(id);
        args["version"] = json!(self.version(id).await);
        self.call("review_module", args, error).await
    }
}

/// Extract one labeled exact precondition from compact agent text.
fn field(text: &str, label: &str) -> String {
    text.lines()
        .find_map(|line| line.strip_prefix(label))
        .expect(text)
        .to_owned()
}

/// Configuration-independent discovery and alias/root binding remain usable after a cold start.
#[tokio::test]
async fn core_config_cold_aliases_and_read_only_absence() {
    let f = Fixture::new();
    assert_eq!(super::definitions().len(), 7);
    f.call("get_status", json!({}), false).await;
    let absent = f
        .call("get_context", json!({"project":"alpha"}), false)
        .await;
    assert!(absent.contains("not initialized"));
    assert!(!f.root.exists());
    assert_eq!(
        field(&absent, "Allocation version: "),
        field(
            &f.call("get_context", json!({"project":"same"}), false)
                .await,
            "Allocation version: "
        )
    );
    f.call("get_context", json!({"project":"unknown"}), true)
        .await;
    let missing = Config::new(Some(f.directory.path().join("missing.toml")));
    let identity = super::call("get_status", json!({}), &f.identity, &f.templates, &missing)
        .await
        .unwrap();
    assert_eq!(identity.is_error, Some(false));
    let bad = super::call(
        "get_context",
        json!({"project":"alpha"}),
        &f.identity,
        &f.templates,
        &missing,
    )
    .await
    .unwrap();
    assert_eq!(bad.is_error, Some(true));
    f.init().await;
    let m = f.module(vec![], None).await;
    let old = f.version(&m).await;
    let other = f.directory.path().join("other");
    fs::create_dir(&other).unwrap();
    fs::create_dir(other.join("modules")).unwrap();
    fs::create_dir(other.join(".agent-tasks")).unwrap();
    for relative in [
        "project.yaml",
        ".agent-tasks/state.yaml",
        "modules/M-001.yaml",
    ] {
        fs::copy(f.root.join(relative), other.join(relative)).unwrap();
    }
    fs::write(
        f.directory.path().join("config.toml"),
        format!(
            "schema_version = 1\n[aliases]\nalpha = {}\n",
            serde_json::to_string(&other).unwrap()
        ),
    )
    .unwrap();
    assert_ne!(old, f.version(&m).await);
    f.call("record_work",json!({"project":"alpha","ref":m,"version":old,"op":"result","summary":"Must not overwrite another clone"}),true).await;
}

/// Native subset/normalization guards reject destructive source shapes and preserve exact originals.
#[tokio::test]
async fn core_store_yaml_and_normalization_backup() {
    let f = Fixture::new();
    f.init().await;
    let id = f.module(vec![], None).await;
    let store = f.store();
    let original = store.module(&id).unwrap();
    let encoded = store::encode(&original.value).unwrap();
    assert_eq!(store::decode::<Module>(&encoded).unwrap(), original.value);
    for bad in [
        "a: 1\na: 2\n",
        "a: &anchor one\nb: *anchor\n",
        "a: !Tag x\n",
        "a: {<<: x}\n",
        "? [a,b]\n: x\n",
        "---\na: 1\n---\na: 2\n",
    ] {
        assert!(store::decode::<Value>(bad.as_bytes()).is_err(), "{bad}");
    }
    let prose = serde_yaml_ng::to_string(&vec![
        "yes",
        "null",
        "2026-10-05",
        "*literal",
        "line\n!text",
        "quotes ' \"",
    ])
    .unwrap();
    assert_eq!(
        store::decode::<Vec<String>>(prose.as_bytes())
            .unwrap()
            .len(),
        6
    );
    let mut decorated = b"# Native explanation\n".to_vec();
    decorated.extend_from_slice(&encoded);
    fs::write(
        f.root.join("modules").join(format!("{id}.yaml")),
        &decorated,
    )
    .unwrap();
    let observed = store.module(&id).unwrap();
    let saved = f
        .record(
            &id,
            json!({"op":"result","summary":"Exact original is preserved","actor":"lead"}),
            false,
        )
        .await;
    assert!(saved.contains("exact original retained"));
    let backup = store.root.join(format!(
        ".agent-tasks/backups/{id}.yaml-{}.yaml",
        observed.version
    ));
    assert_eq!(fs::read(backup).unwrap(), decorated);
    let now = store.module(&id).unwrap();
    let mut again = b"# Another explanation\n".to_vec();
    again.extend(now.bytes);
    fs::write(f.root.join(format!("modules/{id}.yaml")), &again).unwrap();
    fs::rename(
        f.root.join(".agent-tasks/backups"),
        f.root.join(".agent-tasks/kept-backups"),
    )
    .unwrap();
    fs::write(f.root.join(".agent-tasks/backups"), b"foreign").unwrap();
    f.record(
        &id,
        json!({"op":"result","summary":"Must not discard comments"}),
        true,
    )
    .await;
    assert_eq!(
        fs::read(f.root.join(format!("modules/{id}.yaml"))).unwrap(),
        again
    );
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            fs::metadata(f.root.join(".agent-tasks/state.yaml"))
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
    }
}

/// Results/reviewer checks preserve summary authors, independent review and semantic invalidation.
#[tokio::test]
async fn core_workflow_review_and_rework() {
    let f = Fixture::new();
    f.init().await;
    let id=f.module(vec![json!({"title":"Protect replacements","criterion":"No stale overwrite","required_checks":["stale write"]})],Some(json!({"name":"lead"}))).await;
    let task = format!("{id}/T-001");
    f.review(
        &id,
        json!({"verdict":"accepted","summary":"Premature","actor":"reviewer"}),
        true,
    )
    .await;
    f.record(&task,json!({"op":"result","state":"done","summary":"Replacements are guarded","actor":"lead","checks":[{"label":"stale write","status":"failed","detail":"Regression remained"}],"artifacts":["commit: reported-commit"]}),false).await;
    f.review(
        &id,
        json!({"verdict":"accepted","summary":"Own code","actor":"lead"}),
        true,
    )
    .await;
    f.review(
        &id,
        json!({"verdict":"accepted","summary":"Missing required pass","actor":"reviewer"}),
        true,
    )
    .await;
    f.review(&id,json!({"verdict":"changes_requested","summary":"Fix the replacement path","actor":"reviewer","findings":[{"text":"Handle the stale branch","must_fix":true}]}),false).await;
    assert_eq!(
        f.store().module(&id).unwrap().value.phase(),
        "changes requested"
    );
    f.record(
        &id,
        json!({"op":"reopen","reason":"Apply reviewer correction"}),
        false,
    )
    .await;
    f.record(&task,json!({"op":"result","state":"done","summary":"Replacement correction implemented","actor":"lead","checks":[{"label":"stale write","status":"failed","detail":"Lead report retained before independent qualification"}]}),false).await;
    let lead_report = f.store().module(&id).unwrap().value.tasks[0]
        .result
        .clone()
        .unwrap();
    let accepted=f.review(&id,json!({"verdict":"accepted","summary":"Current correction independently qualified","actor":"reviewer","checks":[{"target":task,"label":"stale write","status":"passed","detail":"Independent reproduction succeeded"}]}),false).await;
    assert!(accepted.contains("accepted"));
    let m = f.store().module(&id).unwrap().value;
    assert_eq!(m.phase(), "accepted");
    assert_eq!(m.tasks[0].result.as_ref().unwrap(), &lead_report);
    assert_eq!(m.tasks[0].checks[0].actor.as_deref(), Some("reviewer"));
    assert_eq!(
        m.reviews.last().unwrap().check_updates[0]
            .before
            .as_ref()
            .unwrap()
            .status,
        CheckStatus::Failed
    );
    let epoch = m.review_epoch;
    f.record(&id,json!({"op":"handoff","stopping_point":"Review completed","next_action":"Owner may inspect"}),false).await;
    let m = f.store().module(&id).unwrap().value;
    assert_eq!(m.phase(), "accepted");
    assert_eq!(m.review_epoch, epoch);
    f.plan(
        &id,
        json!({"op":"edit_module","module":id,"outcome":"A changed acceptance outcome"}),
        false,
    )
    .await;
    assert_eq!(
        f.store().module(&id).unwrap().value.phase(),
        "stale approval"
    );
    let checks = f
        .call(
            "get_context",
            json!({"project":"alpha","ref":task,"view":"checks"}),
            false,
        )
        .await;
    assert!(checks.contains("reviewer") && checks.contains("passed"));
    let report = f
        .call(
            "get_context",
            json!({"project":"alpha","ref":id,"view":"review","review_index":0}),
            false,
        )
        .await;
    assert!(report.contains("Fix the replacement path") && report.contains("historical"));
    f.call(
        "get_context",
        json!({"project":"alpha","ref":task,"view":"review"}),
        true,
    )
    .await;
    let unknown = f.module(vec![], None).await;
    f.record(
        &unknown,
        json!({"op":"result","summary":"Non-code outcome reported"}),
        false,
    )
    .await;
    f.review(
        &unknown,
        json!({"verdict":"accepted","summary":"Unknown identities stay unknown"}),
        false,
    )
    .await;
    assert_eq!(
        f.store().module(&unknown).unwrap().value.phase(),
        "accepted"
    );
}

/// Partial edits and explicit lifecycle operations never erase siblings or cascade.
#[tokio::test]
async fn core_workflow_partial_edits_and_cancellation() {
    let f = Fixture::new();
    f.init().await;
    let id = f
        .module(
            vec![
                json!({"title":"First","criterion":"Old criterion"}),
                json!({"title":"Second"}),
            ],
            Some(json!({"name":"lead","handle":"reported-handle"})),
        )
        .await;
    let task = format!("{id}/T-001");
    f.plan(
        &id,
        json!({"op":"edit_task","ref":task,"criterion":null}),
        false,
    )
    .await;
    let m = f.store().module(&id).unwrap().value;
    assert!(m.tasks[0].criterion.is_none());
    assert_eq!(m.tasks[0].title, "First");
    assert_eq!(m.tasks[1].title, "Second");
    f.plan(
        &id,
        json!({"op":"edit_module","module":id,"lead":null,"required_checks":[]}),
        false,
    )
    .await;
    assert!(f.store().module(&id).unwrap().value.lead.is_none());
    f.plan(
        &id,
        json!({"op":"edit_module","module":id,"title":null}),
        true,
    )
    .await;
    let before = f.store().module(&id).unwrap().bytes;
    let noop = f
        .record(
            &id,
            json!({"op":"clear_blocker","reason":"No blocker remains"}),
            false,
        )
        .await;
    assert!(noop.contains("UNCHANGED"));
    assert_eq!(f.store().module(&id).unwrap().bytes, before);
    f.record(
        &task,
        json!({"op":"blocker","problem":"Wrong target","needed_action":"No implicit parent write"}),
        true,
    )
    .await;
    f.record(
        &id,
        json!({"op":"cancel","reason":"Parent cannot skip open children"}),
        true,
    )
    .await;
    f.record(
        &task,
        json!({"op":"cancel","reason":"Removed from scope"}),
        false,
    )
    .await;
    f.plan(
        &id,
        json!({"op":"edit_task","ref":task,"title":"Cannot edit canceled work"}),
        true,
    )
    .await;
    f.record(
        &task,
        json!({"op":"reopen","reason":"Scope restored"}),
        false,
    )
    .await;
    let m = f.store().module(&id).unwrap().value;
    assert_eq!(m.tasks[0].cancellation_history.len(), 1);
    assert!(m.reasons.iter().any(|r| r.reason == "Scope restored"));
    f.record(
        &task,
        json!({"op":"cancel","reason":"Scope removed again"}),
        false,
    )
    .await;
    f.record(
        &format!("{id}/T-002"),
        json!({"op":"result","state":"done","summary":"Second delivered"}),
        false,
    )
    .await;
    f.record(&id, json!({"op":"cancel","reason":"Module retired"}), false)
        .await;
    f.record(
        &format!("{id}/T-002"),
        json!({"op":"result","summary":"Parent is canceled"}),
        true,
    )
    .await;
    f.record(&id, json!({"op":"reopen","reason":"Module resumed"}), false)
        .await;
    assert_eq!(
        f.store().module(&id).unwrap().value.tasks[0].state,
        TaskState::Canceled
    );
    f.plan(
        &id,
        json!({"op":"add_task","module":id,"title":"Next identity"}),
        false,
    )
    .await;
    assert_eq!(f.store().module(&id).unwrap().value.tasks[2].id, "T-003");
    f.record(&id,json!({"op":"blocker","problem":"Awaiting fixture","needed_action":"Prepare native fixture","resolver":"owner"}),false).await;
    f.record(
        &id,
        json!({"op":"clear_blocker","reason":"Fixture prepared"}),
        false,
    )
    .await;
    f.record(
        &id,
        json!({"op":"handoff","stopping_point":"Ready","next_action":"Review"}),
        false,
    )
    .await;
    f.record(
        &id,
        json!({"op":"clear_handoff","reason":"Lead resumed"}),
        false,
    )
    .await;
    let project = f.store().project().unwrap().unwrap();
    f.call("plan_work",json!({"project":"alpha","op":"edit_project","version":project.version,"remote":"reported-repository"}),false).await;
    let project = f.store().project().unwrap().unwrap();
    f.call("plan_work",json!({"project":"alpha","op":"edit_project","version":project.version,"remote":null,"title":"New orientation"}),false).await;
    assert!(f.store().project().unwrap().unwrap().value.remote.is_none());
}

/// Stale siblings refuse, independent modules remain writable, and root locks coordinate aliases.
#[tokio::test]
async fn core_store_stale_busy_reservations_and_partial_init() {
    let f = Fixture::new();
    f.init().await;
    let first = f
        .module(vec![json!({"title":"One"}), json!({"title":"Two"})], None)
        .await;
    let second = f.module(vec![], None).await;
    let old = f.version(&first).await;
    let independent = f.version(&second).await;
    f.record(
        &format!("{first}/T-001"),
        json!({"op":"result","state":"done","summary":"One complete"}),
        false,
    )
    .await;
    let prior = f.store().module(&first).unwrap().bytes;
    f.call("record_work",json!({"project":"same","version":old,"ref":format!("{first}/T-002"),"op":"result","summary":"Stale sibling"}),true).await;
    assert_eq!(f.store().module(&first).unwrap().bytes, prior);
    f.call("record_work",json!({"project":"alpha","version":independent,"ref":second,"op":"result","summary":"Independent module remains writable"}),false).await;
    let store = f.store();
    let lock = store.lock(true, &mut Vec::new()).unwrap();
    let busy = f
        .call("get_context", json!({"project":"same","ref":first}), true)
        .await;
    assert!(busy.contains("busy"));
    drop(lock);
    let state = Allocator {
        schema_version: SCHEMA,
        next_module: 7,
    };
    fs::write(
        f.root.join(".agent-tasks/state.yaml"),
        store::encode(&state).unwrap(),
    )
    .unwrap();
    assert_eq!(
        f.module(vec![], None).await,
        "M-007",
        "Reservation gaps are not recycled"
    );
    fs::remove_file(f.root.join(".agent-tasks/state.yaml")).unwrap();
    f.call("project_status", json!({"project":"alpha"}), false)
        .await;
    let version = f.allocation().await;
    f.call("plan_work",json!({"project":"alpha","version":version,"op":"create_module","title":"No guessed counter","outcome":"Refuse"}),true).await;
    let partial = Fixture::new();
    let old = partial.allocation().await;
    let store = partial.store();
    store.prepare(&mut Vec::new()).unwrap();
    fs::write(
        partial.root.join(".agent-tasks/state.yaml"),
        store::encode(&Allocator {
            schema_version: SCHEMA,
            next_module: 1,
        })
        .unwrap(),
    )
    .unwrap();
    let context = partial
        .call("get_context", json!({"project":"alpha"}), false)
        .await;
    assert!(context.contains("partial initialization"));
    assert!(!partial.root.join("project.yaml").exists());
    partial.call("plan_work",json!({"project":"alpha","op":"init_project","version":old,"title":"Partial","purpose":"Resume"}),true).await;
    partial.call("plan_work",json!({"project":"alpha","op":"init_project","version":field(&context,"Allocation version: "),"title":"Partial","purpose":"Resume explicit init"}),false).await;
    assert!(partial.root.join("project.yaml").exists());
}

/// The owner status fixture fits in one response; unreadable work never reports complete zero.
#[tokio::test]
async fn core_presentation_status_search_and_snapshot_paging() {
    let f = Fixture::new();
    f.init().await;
    for i in 0..3 {
        let id=f.module((0..4).map(|j|json!({"title":format!("Scenario {i} {j}"),"criterion":"Native compatibility"})).collect(),Some(json!({"name":format!("lead-{i}")}))).await;
        for j in 1..=2 {
            f.record(&format!("{id}/T-{j:03}"),json!({"op":"result","state":"done","summary":format!("Compatibility scenario {i} {j} passed"),"checks":[{"label":"native check","status":"passed"}]}),false).await;
        }
        if i < 2 {
            f.record(&id,json!({"op":"blocker","problem":format!("Fixture {i} unavailable"),"needed_action":"Prepare native fixture"}),false).await;
        } else {
            f.review(&id,json!({"verdict":"changes_requested","summary":"Complete the remaining scenarios","actor":"reviewer"}),false).await;
        }
    }
    let status = f
        .call("project_status", json!({"project":"alpha"}), false)
        .await;
    assert!(
        status.contains("data coverage: complete") || status.contains("Data coverage: complete")
    );
    assert!(status.contains("detail coverage: complete"), "{status}");
    assert!(status.contains("6 done / 12 readable"));
    assert!(status.contains("3 readable"));
    for i in 1..=3 {
        assert!(status.contains(&format!("lead-{}", i - 1)));
        for j in 1..=4 {
            assert!(status.contains(&format!("M-{i:03}/T-{j:03}")));
        }
    }
    assert_eq!(status.matches("BLOCKER").count(), 2);
    assert!(status.contains("latest review: changes requested") && status.contains("reviewer"));
    assert!(!status.contains("Version:"));
    let search = f
        .call(
            "search",
            json!({"project":"alpha","query":"compatibility scenario","limit":2}),
            false,
        )
        .await;
    assert!(search.contains("M-001/T-001") && search.contains("Next: start=2"));
    let snapshot = field(&search, "Snapshot version: ");
    let wrong_query=f.call("search",json!({"project":"alpha","query":"native compatibility","limit":2,"start":2,"version":snapshot}),true).await;
    assert!(wrong_query.contains("stale"));
    let next=f.call("search",json!({"project":"alpha","query":"compatibility scenario","limit":2,"start":2,"version":snapshot}),false).await;
    assert!(!next.contains("M-001/T-001"));
    f.record(
        "M-001/T-001",
        json!({"op":"result","summary":"Changed current compatibility evidence"}),
        false,
    )
    .await;
    f.call("search",json!({"project":"alpha","query":"compatibility scenario","limit":2,"start":2,"version":snapshot}),true).await;
    fs::write(f.root.join("modules/M-002.yaml"), b"corrupt: [").unwrap();
    let partial = f
        .call("project_status", json!({"project":"alpha"}), false)
        .await;
    assert!(
        partial.contains("PARTIAL")
            && partial.contains("UNREADABLE")
            && partial.contains("M-002")
            && partial.contains("lower bounds")
    );
    f.call(
        "get_context",
        json!({"project":"alpha","ref":"M-001"}),
        false,
    )
    .await;
    fs::write(f.root.join("modules/foreign.yaml"), b"owner").unwrap();
    let allocation = f.allocation().await;
    f.call("plan_work",json!({"project":"alpha","version":allocation,"op":"create_module","title":"Foreign inventory","outcome":"Refuse"}),true).await;
}

/// Exported closed schemas and serde agree on every operation's structural shapes.
#[test]
fn core_contract_all_operation_shapes() {
    let catalog = super::definitions();
    let valid_plan = vec![
        json!({"op":"init_project","title":"Project","purpose":"Intent"}),
        json!({"op":"edit_project","remote":null}),
        json!({"op":"create_module","title":"Module","outcome":"Result","tasks":[{"title":"Task"}]}),
        json!({"op":"edit_module","module":"M-001","lead":null,"required_checks":[]}),
        json!({"op":"add_task","module":"M-001","title":"Task"}),
        json!({"op":"edit_task","ref":"M-001/T-001","criterion":null}),
    ];
    let valid_work = vec![
        json!({"op":"result","summary":"Delivered","state":"done","checks":[{"label":"native","status":"passed"}]}),
        json!({"op":"blocker","problem":"Missing input","needed_action":"Supply it"}),
        json!({"op":"handoff","stopping_point":"Here","next_action":"There"}),
        json!({"op":"clear_blocker","reason":"Resolved"}),
        json!({"op":"clear_handoff","reason":"Resumed"}),
        json!({"op":"cancel","reason":"Out of scope"}),
        json!({"op":"reopen","reason":"Scope restored"}),
    ];
    for (name, operations) in [("plan_work", valid_plan), ("record_work", valid_work)] {
        let schema = &catalog.iter().find(|t| t["name"] == name).unwrap()["inputSchema"];
        let validator = jsonschema::validator_for(schema).unwrap();
        for mut args in operations {
            args["project"] = json!("alpha");
            args["version"] = json!("v");
            if name == "record_work" {
                args["ref"] = json!("M-001/T-001");
            }
            assert!(
                validator.is_valid(&args),
                "{name}: {args}: {:?}",
                validator.validate(&args)
            );
            let decode = |v: Value| {
                if name == "plan_work" {
                    input::mutation::<input::Plan>(v, false).is_ok()
                } else {
                    input::mutation::<input::Work>(v, true).is_ok()
                }
            };
            assert!(decode(args.clone()));
            let mut unknown = args.clone();
            unknown["extra"] = json!(true);
            assert!(!validator.is_valid(&unknown) && !decode(unknown));
            let mut missing = args.clone();
            missing.as_object_mut().unwrap().remove("project");
            assert!(!validator.is_valid(&missing) && !decode(missing));
            let mut null = args.clone();
            null["project"] = Value::Null;
            assert!(!validator.is_valid(&null) && !decode(null));
            let mut wrong = args.clone();
            wrong["op"] = json!("unsupported");
            assert!(!validator.is_valid(&wrong) && !decode(wrong));
            let variant = schema["oneOf"]
                .as_array()
                .unwrap()
                .iter()
                .find(|v| v["properties"]["op"]["const"] == args["op"])
                .unwrap();
            for required in variant["required"].as_array().unwrap() {
                let mut missing = args.clone();
                missing
                    .as_object_mut()
                    .unwrap()
                    .remove(required.as_str().unwrap());
                assert!(
                    !validator.is_valid(&missing) && !decode(missing),
                    "{name} missing {required}"
                );
            }
        }
    }
    for (name, args, decode) in [
        (
            "get_context",
            json!({"project":"alpha"}),
            input_shape::<input::ContextArgs> as fn(Value) -> bool,
        ),
        (
            "project_status",
            json!({"project":"alpha"}),
            input_shape::<input::StatusArgs>,
        ),
        (
            "search",
            json!({"project":"alpha","query":"work"}),
            input_shape::<input::SearchArgs>,
        ),
        (
            "review_module",
            json!({"project":"alpha","module":"M-001","version":"v","verdict":"accepted","summary":"Qualified"}),
            input_shape::<input::ReviewArgs>,
        ),
    ] {
        let validator = jsonschema::validator_for(
            &catalog.iter().find(|t| t["name"] == name).unwrap()["inputSchema"],
        )
        .unwrap();
        assert!(validator.is_valid(&args) && decode(args.clone()));
        let mut unknown = args.clone();
        unknown["extra"] = json!(true);
        assert!(!validator.is_valid(&unknown) && !decode(unknown));
        let mut null = args.clone();
        null["project"] = Value::Null;
        assert!(!validator.is_valid(&null) && !decode(null));
        let mut missing = args.clone();
        missing.as_object_mut().unwrap().remove("project");
        assert!(!validator.is_valid(&missing) && !decode(missing));
    }
    for op in ["edit_project", "edit_module", "edit_task"] {
        let mut args = json!({"op":op,"project":"alpha","version":"v","module":"M-001","ref":"M-001/T-001","title":null});
        if op == "edit_project" {
            args.as_object_mut().unwrap().remove("module");
        }
        if op != "edit_task" {
            args.as_object_mut().unwrap().remove("ref");
        } else {
            args.as_object_mut().unwrap().remove("module");
        }
        assert!(input::mutation::<input::Plan>(args, false).is_err());
    }
}

/// Project search routes and receipt scope labels remain usable and populated views never look empty.
#[tokio::test]
async fn core_presentation_action_routes_and_scope_labels() {
    let f = Fixture::new();
    f.init().await;
    let id = f
        .module(vec![json!({"title":"Deliver result"})], None)
        .await;
    let saved = f
        .record(
            &format!("{id}/T-001"),
            json!({"op":"result","state":"done","summary":"Delivered"}),
            false,
        )
        .await;
    assert!(saved.contains("Module phase: ready"));
    assert!(!saved.contains("T-001 — ready"));
    let context = f
        .call("get_context", json!({"project":"alpha","ref":id}), false)
        .await;
    assert!(!context.contains("No entries in this view"));
    f.review(
        &id,
        json!({"verdict":"accepted","summary":"Independent result accepted","actor":"reviewer"}),
        false,
    )
    .await;
    let review = f
        .call(
            "get_context",
            json!({"project":"alpha","ref":id,"view":"review"}),
            false,
        )
        .await;
    assert!(review.contains("This review has no findings or check updates"));
    let found = f
        .call(
            "search",
            json!({"project":"alpha","query":"strategic intent"}),
            false,
        )
        .await;
    assert!(found.contains("Project ") && found.contains("omit ref"));
    let root = f
        .call("get_context", json!({"project":"alpha"}), false)
        .await;
    assert!(root.contains("Portable work"));
}

/// Every registered tool advertises an object root so independent MCP clients retain write tools.
#[test]
fn core_contract_mcp_object_roots() {
    for tool in super::definitions() {
        assert_eq!(tool["inputSchema"]["type"], "object", "{}", tool["name"]);
    }
}

/// Decode only structural argument shapes; domain readiness is tested through real tools.
fn input_shape<T: serde::de::DeserializeOwned>(value: Value) -> bool {
    serde_json::from_value::<T>(value).is_ok()
}

/// Post-publication failure exposes saved bytes without false rollback or automatic replay.
#[tokio::test]
async fn core_store_visible_sync_failure_and_native_drift() {
    let f = Fixture::new();
    f.init().await;
    let id = f.module(vec![], None).await;
    let version = f.version(&id).await;
    store::fail_next_directory_sync();
    let reply=f.call("record_work",json!({"project":"alpha","ref":id,"version":version,"op":"result","summary":"Visibly saved; durability receipt is uncertain"}),true).await;
    assert!(reply.contains("durability_unknown") && reply.contains("Published modules/M-001.yaml"));
    let saved = f.store().module(&id).unwrap();
    assert!(
        saved
            .value
            .result
            .as_ref()
            .unwrap()
            .summary
            .contains("Visibly saved")
    );
    f.call("record_work",json!({"project":"alpha","ref":id,"version":version,"op":"result","summary":"Do not replay"}),true).await;
    assert_eq!(f.store().module(&id).unwrap().bytes, saved.bytes);
    f.review(
        &id,
        json!({"verdict":"accepted","summary":"Saved result reviewed","actor":"reviewer"}),
        false,
    )
    .await;
    let original = f.store().module(&id).unwrap();
    let mut native = original.value.clone();
    native.outcome = "Externally changed intended result".into();
    fs::write(
        f.root.join("modules/M-001.yaml"),
        store::encode(&native).unwrap(),
    )
    .unwrap();
    let context = f
        .call("get_context", json!({"project":"alpha","ref":id}), false)
        .await;
    assert!(context.contains("stale approval") && context.contains("possible native edit"));
    assert_eq!(
        f.store().module(&id).unwrap().value.review_epoch,
        original.value.review_epoch
    );
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(
            f.root.join("modules/M-001.yaml"),
            f.root.join("modules/M-002.yaml"),
        )
        .unwrap();
        let partial = f
            .call("project_status", json!({"project":"alpha"}), false)
            .await;
        assert!(partial.contains("PARTIAL") && partial.contains("M-002"));
        f.call(
            "get_context",
            json!({"project":"alpha","ref":"M-002"}),
            true,
        )
        .await;
    }
}

/// Large files/aggregate scans remain bounded, and budget pages advance by displayed rows.
#[tokio::test]
async fn core_presentation_large_scan_and_exact_budget_pages() {
    let f = Fixture::new();
    f.init().await;
    let id = f
        .module(
            (0..32)
                .map(|i| json!({"title":format!("Task {i}"),"criterion":"c".repeat(1024)}))
                .collect(),
            None,
        )
        .await;
    for i in 1..=8 {
        f.record(&format!("{id}/T-{i:03}"),json!({"op":"result","state":"done","summary":"s".repeat(1024),"gaps":(0..8).map(|n|format!("{n}{}","x".repeat(240))).collect::<Vec<_>>()}),false).await;
    }
    let first = f
        .call(
            "get_context",
            json!({"project":"alpha","ref":id,"view":"tasks","limit":20}),
            false,
        )
        .await;
    let next = first
        .lines()
        .find_map(|s| s.strip_prefix("Next: start="))
        .unwrap()
        .split(';')
        .next()
        .unwrap()
        .parse::<usize>()
        .unwrap();
    assert!(next > 0 && next <= 20);
    let wrong_view=f.call("get_context",json!({"project":"alpha","ref":id,"view":"checks","limit":20,"start":next,"version":field(&first,"Snapshot version: ")}),true).await;
    assert!(wrong_view.contains("stale"));
    let second=f.call("get_context",json!({"project":"alpha","ref":id,"view":"tasks","limit":20,"start":next,"version":field(&first,"Snapshot version: ")}),false).await;
    assert!(!second.contains("M-001/T-001 "));
    let template = f.store().module(&id).unwrap().value;
    for n in 2..=35 {
        let mut m = template.clone();
        m.id = format!("M-{n:03}");
        m.log.clear();
        m.next_log = Some(1);
        let mut bytes = store::encode(&m).unwrap();
        bytes.extend_from_slice(
            format!("# {}\n", "padding".repeat((500 * 1024 - bytes.len()) / 7)).as_bytes(),
        );
        fs::write(f.root.join(format!("modules/{}.yaml", m.id)), bytes).unwrap();
    }
    let scan = f.store().scan(None).unwrap();
    assert!(!scan.complete);
    assert!(!scan.unreadable.is_empty());
    assert!(scan.modules.iter().map(|m| m.bytes.len()).sum::<usize>() <= store::SCAN_CAP);
    let narrowed = f
        .call(
            "project_status",
            json!({"project":"alpha","module":"M-035"}),
            false,
        )
        .await;
    assert!(narrowed.contains("M-035"));
    let state = Allocator {
        schema_version: SCHEMA,
        next_module: 36,
    };
    fs::write(
        f.root.join(".agent-tasks/state.yaml"),
        store::encode(&state).unwrap(),
    )
    .unwrap();
    let created = f.module(vec![], None).await;
    assert_eq!(
        created, "M-036",
        "Content budget must not block complete filename allocation"
    );
}

/// Worst bounded reviewer snapshots fit the reserved final-review headroom.
#[tokio::test]
async fn core_store_closing_reserve_with_escaped_review() {
    let f = Fixture::new();
    f.init().await;
    let id = f.module(vec![], None).await;
    let actor = format!("a{}", "\t".repeat(127));
    let labels: Vec<_> = (0..8).map(|i| format!("c{i}{}", "\t".repeat(62))).collect();
    f.record(&id,json!({"op":"result","summary":format!("s{}","\t".repeat(1023)),"actor":actor,
        "checks":labels.iter().map(|l|json!({"label":l,"status":"passed","detail":format!("d{}","\t".repeat(255))})).collect::<Vec<_>>()}),false).await;
    let before = f.store().module(&id).unwrap().bytes.len();
    f.review(&id,json!({"verdict":"accepted","summary":format!("r{}","\t".repeat(1023)),"actor":actor,
        "findings":(0..8).map(|_|json!({"text":format!("f{}","\t".repeat(255)),"must_fix":false})).collect::<Vec<_>>(),
        "checks":labels.iter().map(|l|json!({"target":id,"label":l,"status":"passed","detail":format!("d{}","\t".repeat(255))})).collect::<Vec<_>>()}),false).await;
    let after = f.store().module(&id).unwrap().bytes.len();
    assert!(
        after - before < store::CLOSING_RESERVE,
        "{}",
        after - before
    );
}

/// Log eviction retains substance, backups and rendering fallback never repeat saved work.
#[tokio::test]
async fn core_store_capacity_log_tail_and_post_write_presentation() {
    let f = Fixture::new();
    f.init().await;
    let id = f.module(vec![], None).await;
    f.record(
        &id,
        json!({"op":"result","summary":"Human substance is retained"}),
        false,
    )
    .await;
    f.record(
        &id,
        json!({"op":"reopen","reason":"Persistent human reason"}),
        false,
    )
    .await;
    let store = f.store();
    let before = store.module(&id).unwrap();
    let mut m = before.value.clone();
    for _ in 0..300 {
        m.event(&id, "machine activity", &store::now(), &None)
            .unwrap();
    }
    assert_eq!(m.log.len(), LOG_TAIL);
    assert!(m.omitted_log_entries > 0);
    assert_eq!(
        m.result.as_ref().unwrap().summary,
        "Human substance is retained"
    );
    assert!(
        m.reasons
            .iter()
            .any(|r| r.reason == "Persistent human reason")
    );
    store
        .save(
            &format!("modules/{id}.yaml"),
            &m,
            Some(&before.bytes),
            false,
            &mut Vec::new(),
        )
        .unwrap();
    let version = f.version(&id).await;
    let bad_templates = Templates::new(&[]).unwrap();
    let reply=super::call("record_work",json!({"project":"alpha","ref":id,"version":version,"op":"result","summary":"Saved despite unavailable presenter"}),&f.identity,&bad_templates,&f.config).await.unwrap();
    assert_eq!(reply.is_error, Some(false));
    let body = serde_json::to_value(reply).unwrap()["content"][0]["text"]
        .as_str()
        .unwrap()
        .to_owned();
    assert!(
        body.contains("SAVED")
            && body.contains("Presentation degraded")
            && body.contains("Version: ")
    );
    let after = store.module(&id).unwrap();
    assert_eq!(
        after.value.result.as_ref().unwrap().summary,
        "Saved despite unavailable presenter"
    );
    f.call("record_work",json!({"project":"alpha","ref":id,"version":version,"op":"result","summary":"Must not replay"}),true).await;
    assert_eq!(store.module(&id).unwrap().bytes, after.bytes);
    let mut large = after.value.clone();
    let review = Review {
        verdict: Verdict::ChangesRequested,
        summary: "x".repeat(1024),
        findings: (0..8)
            .map(|_| Finding {
                text: "y".repeat(256),
                must_fix: true,
            })
            .collect(),
        basis: large.basis().unwrap(),
        epoch: large.review_epoch,
        at: store::now(),
        reviewer: None,
        check_updates: vec![],
    };
    while store::encode(&large).unwrap().len() < store::RECORD_CAP - store::CLOSING_RESERVE {
        large.reviews.push(review.clone());
    }
    let retained = store.module(&id).unwrap();
    assert!(
        store
            .save(
                &format!("modules/{id}.yaml"),
                &large,
                Some(&retained.bytes),
                false,
                &mut Vec::new()
            )
            .is_err()
    );
    assert_eq!(store.module(&id).unwrap().bytes, retained.bytes);
    let bytes = store::encode(&large).unwrap();
    assert!(bytes.len() < store::RECORD_CAP);
    store
        .save(
            &format!("modules/{id}.yaml"),
            &large,
            Some(&retained.bytes),
            true,
            &mut Vec::new(),
        )
        .unwrap();
}
