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
    /// Existing core regressions explicitly exercise pre-workflow records; modern suites opt in.
    modern: bool,
}
impl Fixture {
    /// Build aliases alpha/same for the same root without creating that root.
    fn new() -> Self {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path().join("docs");
        let config_path = directory.path().join("config.toml");
        fs::write(&config_path, "schema_version = 1\n").unwrap();
        fs::write(
            directory.path().join("projects.toml"),
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
            modern: false,
        }
    }
    /// Create a modern workflow fixture without grandfathering generated records.
    fn modern() -> Self {
        let mut f = Self::new();
        f.modern = true;
        f
    }

    /// Execute the real route; legacy fixtures strip only new optional policy from disposable generated records.
    /// This preserves c936 regression semantics while modern tests exercise the full current production lifecycle.
    async fn call(&self, name: &str, args: Value, error: bool) -> String {
        let request = args.clone();
        let reply = super::call(name, args, &self.identity, &self.templates, &self.config)
            .await
            .unwrap();
        assert_eq!(reply.is_error, Some(error), "{reply:?}");
        let wire = serde_json::to_value(reply).unwrap();
        assert_eq!(wire["content"].as_array().unwrap().len(), 1);
        let mut text = wire["content"][0]["text"].as_str().unwrap().to_owned();
        if !self.modern
            && !error
            && name == "plan_work"
            && request["op"].as_str().is_some_and(|op| {
                matches!(
                    op,
                    "create_module" | "create_epic" | "create_atomic" | "add_atomic"
                )
            })
        {
            let target = text
                .lines()
                .next()
                .unwrap()
                .split_whitespace()
                .nth(1)
                .unwrap()
                .split('/')
                .next()
                .unwrap();
            let before = self.store().module(target).unwrap();
            let mut old = before.value;
            old.workflow = None;
            for a in &mut old.atomics {
                a.atomic_workflow = None;
            }
            let path = work_path(&old.id).unwrap();
            let bytes = store::encode(&old).unwrap();
            fs::write(self.root.join(&path), &bytes).unwrap();
            let current = self.store().version(&path, Some(&bytes));
            text = text.replace(&before.version, &current);
        }
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

/// Registration persists a separate registry and a real Git commit; replay and conflicts preserve data.
#[tokio::test]
async fn core_project_registration_and_listing() {
    let f = Fixture::new();
    let root = f.directory.path().join("registered");
    let args = json!({"project":"registered","doc_dir":root,"name":"Registered product",
        "description":"A portable onboarding example","remote":"https://example.invalid/source",
        "docs_remote":"https://example.invalid/documentation"});
    let reply = f.call("register_project", args.clone(), false).await;
    assert!(reply.starts_with("REGISTERED registered") && !reply.contains(root.to_str().unwrap()));
    let manifest = f
        .config
        .resolve("registered")
        .unwrap()
        .project()
        .unwrap()
        .unwrap();
    assert_eq!(
        manifest.value.remote.as_deref(),
        Some("https://example.invalid/source")
    );
    assert_eq!(
        fs::read_to_string(f.directory.path().join("config.toml")).unwrap(),
        "schema_version = 1\n"
    );
    assert!(root.join(".git").is_dir());
    let git = |args: &[&str]| {
        std::process::Command::new("git")
            .arg("-C")
            .arg(&root)
            .args(args)
            .output()
            .unwrap()
    };
    assert!(git(&["status", "--porcelain"]).stdout.is_empty());
    assert_eq!(
        String::from_utf8(git(&["rev-list", "--count", "HEAD"]).stdout)
            .unwrap()
            .trim(),
        "1"
    );
    assert_eq!(
        String::from_utf8(git(&["remote", "get-url", "origin"]).stdout)
            .unwrap()
            .trim(),
        "https://example.invalid/documentation"
    );
    let version = manifest.version.clone();
    fs::write(root.join("notes.md"), "# Human notes\n").unwrap();
    let repeat = f.call("register_project", args.clone(), false).await;
    assert!(repeat.starts_with("UNCHANGED registered"));
    assert_eq!(
        version,
        f.config
            .resolve("registered")
            .unwrap()
            .project()
            .unwrap()
            .unwrap()
            .version
    );
    assert_eq!(
        String::from_utf8(git(&["rev-list", "--count", "HEAD"]).stdout)
            .unwrap()
            .trim(),
        "1"
    );
    let listing = f.call("get_project_list", json!({}), false).await;
    assert!(listing.contains("Registered product") && listing.contains("onboarding"));
    assert!(listing.contains("alpha") && listing.contains("unavailable"));
    assert!(!listing.contains(root.to_str().unwrap()));
    let mut conflict = args.clone();
    conflict["doc_dir"] = json!(f.directory.path().join("other-root"));
    f.call("register_project", conflict, true).await;
    assert!(!f.directory.path().join("other-root").exists());
    let mut conflict = args.clone();
    conflict["docs_remote"] = json!("https://example.invalid/different-origin");
    f.call("register_project", conflict, true).await;
    let mut conflict = args;
    conflict["description"] = json!("Do not overwrite the old purpose");
    f.call("register_project", conflict, true).await;
    assert_eq!(
        version,
        f.config
            .resolve("registered")
            .unwrap()
            .project()
            .unwrap()
            .unwrap()
            .version
    );
    f.call("get_project_list", json!({"unexpected":true}), true)
        .await;
    let page = f.call("get_project_list", json!({"limit":1}), false).await;
    let token = field(&page, "Snapshot version: ");
    let next = f
        .call(
            "get_project_list",
            json!({"limit":1,"start":1,"version":token}),
            false,
        )
        .await;
    assert!(next.contains("Registered product"));
    let store = f.config.resolve("registered").unwrap();
    let snapshot = store.project().unwrap().unwrap();
    let mut project = snapshot.value.clone();
    project.purpose = "Changed description".into();
    store
        .save(
            "project.yaml",
            &project,
            Some(&snapshot.bytes),
            false,
            &mut Vec::new(),
        )
        .unwrap();
    f.call(
        "get_project_list",
        json!({"limit":1,"start":1,"version":field(&page,"Snapshot version: ")}),
        true,
    )
    .await;
}

/// Foreign content and a failed publication remain intact; explicit retry can finish a partial bootstrap.
#[tokio::test]
async fn core_project_registration_refusals_and_partial_retry() {
    let f = Fixture::new();
    let root = f.directory.path().join("foreign");
    fs::create_dir(&root).unwrap();
    fs::write(root.join("owner.txt"), "Preserve me").unwrap();
    let args = json!({"project":"foreign","doc_dir":root,"name":"Example","description":"Must not overwrite"});
    f.call("register_project", args, true).await;
    assert_eq!(
        fs::read_to_string(root.join("owner.txt")).unwrap(),
        "Preserve me"
    );
    assert!(!root.join("project.yaml").exists());
    f.call("register_project",json!({"project":"relative","doc_dir":"relative","name":"Example","description":"Invalid root"}),true).await;
    let root = f.directory.path().join("partial");
    let args = json!({"project":"partial","doc_dir":root,"name":"Partial example","description":"Resume an explicit bootstrap"});
    store::fail_next_directory_sync();
    let error = f.call("register_project", args.clone(), true).await;
    assert!(error.contains("durability_unknown"));
    assert!(f.config.resolve("partial").is_err());
    f.call("register_project", args, false).await;
    assert!(
        f.config
            .resolve("partial")
            .unwrap()
            .project()
            .unwrap()
            .is_some()
    );
    let lock = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(f.directory.path().join("projects.lock"))
        .unwrap();
    lock.try_lock().unwrap();
    let busy = f.call("register_project",json!({"project":"busy","doc_dir":f.directory.path().join("busy"),"name":"Busy","description":"Do not race the registry"}),true).await;
    assert!(busy.contains("busy") && !f.directory.path().join("busy").exists());
}

/// A missing registry is empty without writes; inline aliases refuse with a migration instruction.
#[tokio::test]
async fn core_project_registry_cold_and_legacy_config() {
    let f = Fixture::new();
    fs::remove_file(f.directory.path().join("projects.toml")).unwrap();
    let listing = f.call("get_project_list", json!({}), false).await;
    assert!(listing.contains("No projects registered"));
    assert!(!f.directory.path().join("projects.toml").exists());
    f.call("register_project", json!({"project":"empty-remotes","doc_dir":f.directory.path().join("empty-remotes"),"name":"Empty remotes","description":"Local documentation needs no remote","remote":"","docs_remote":""}), false).await;
    assert_eq!(
        f.config
            .resolve("empty-remotes")
            .unwrap()
            .project()
            .unwrap()
            .unwrap()
            .value
            .remote,
        None
    );
    fs::write(
        f.directory.path().join("config.toml"),
        "schema_version = 1\n[aliases]\nlegacy = '/absolute/docs'\n",
    )
    .unwrap();
    let refused = f.call("get_project_list", json!({}), true).await;
    assert!(refused.contains("Move [aliases]"));
}

/// Configuration-independent discovery and alias/root binding remain usable after a cold start.
#[tokio::test]
async fn core_config_cold_aliases_and_read_only_absence() {
    let f = Fixture::new();
    assert_eq!(super::definitions().len(), 10);
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
    assert_eq!(bad.is_error, Some(false)); // Missing settings still use the sibling registry.
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
        f.directory.path().join("projects.toml"),
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
        next_epic: Some(1),
        next_atomic: Some(1),
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
            next_epic: Some(1),
            next_atomic: Some(1),
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
        json!({"op":"edit_module","module":"M-001","criteria":["Observe"],"execution":{"repository":"/source","worktree":"/checkout","branch":"feature","target_branch":"main"},"contracts":{"not_required":true},"dependencies":[{"ref":"E-001","condition":"accepted","reason":"Named Epic wait"}]}),
    ];
    let valid_work = vec![
        json!({"op":"result","summary":"Delivered","state":"done","checks":[{"label":"native","status":"passed"}]}),
        json!({"op":"blocker","problem":"Missing input","needed_action":"Supply it"}),
        json!({"op":"handoff","stopping_point":"Here","next_action":"There"}),
        json!({"op":"clear_blocker","reason":"Resolved"}),
        json!({"op":"clear_handoff","reason":"Resumed"}),
        json!({"op":"cancel","reason":"Out of scope"}),
        json!({"op":"reopen","reason":"Scope restored"}),
        json!({"op":"begin"}),
        json!({"op":"complete"}),
        json!({"op":"deliver","target_branch":"main","summary":"Local target delivery"}),
        json!({"op":"import_commits","commits":["abcdef0"]}),
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
        next_epic: Some(1),
        next_atomic: Some(1),
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

/// Allocate a new top-level kind using a real Project Allocation version; return its canonical ref.
async fn core_create_kind(f: &Fixture, op: &str, mut args: Value) -> String {
    args["project"] = json!("alpha");
    args["op"] = json!(op);
    args["version"] = json!(f.allocation().await);
    f.call("plan_work", args, false)
        .await
        .lines()
        .next()
        .unwrap()
        .split_whitespace()
        .nth(1)
        .unwrap()
        .to_owned()
}

/// Save an independent purpose-based verdict with a freshly observed dependency-bound Version.
async fn core_review_entity(f: &Fixture, reference: &str, error: bool) -> String {
    f.call("review_work",json!({"project":"alpha","ref":reference,"version":f.version(reference).await,
        "verdict":"accepted","summary":"Explicit criteria and current delivery independently checked","actor":"reviewer"}),error).await
}

/// Embedded Atomics share Module review, evidence views and lifecycle without indexing Task storage.
#[tokio::test]
async fn core_epic_atomic_embedded_lifecycle_views_and_review() {
    let f = Fixture::new();
    f.init().await;
    let module = f.module(vec![], Some(json!({"name":"Module lead"}))).await;
    for i in 1..=5 {
        f.plan(
            &module,
            json!({"op":"add_atomic","module":module,"title":format!("Verification {i}"),
            "outcome":"Observe integrated behavior","required_checks":["scenario"],
            "executor":{"name":"Atomic executor","handle":"session-atomic"}}),
            false,
        )
        .await;
    }
    f.record(
        "M-001/A-001",
        json!({"op":"result","summary":"Partial verification result"}),
        false,
    )
    .await;
    assert_eq!(f.store().module(&module).unwrap().value.phase(), "working");
    let status = f
        .call("project_status", json!({"project":"alpha"}), false)
        .await;
    assert!(status.contains("task details omitted"), "{status}");
    assert!(
        status.contains("Atomic executor") && status.contains("session-atomic"),
        "{status}"
    );
    f.record(
        "M-001/A-001",
        json!({"op":"cancel","reason":"Replace the scenario setup"}),
        false,
    )
    .await;
    f.record(
        "M-001/A-001",
        json!({"op":"reopen","reason":"Scenario setup available"}),
        false,
    )
    .await;
    f.record(
        "M-001/A-001",
        json!({"op":"result","state":"done","summary":"Verification outcome",
        "checks":[{"label":"scenario","status":"failed"}]}),
        false,
    )
    .await;
    for i in 2..=5 {
        f.record(
            &format!("M-001/A-{i:03}"),
            json!({"op":"cancel","reason":"Outside final scope"}),
            false,
        )
        .await;
    }
    core_review_entity(&f, &module, true).await;
    let reply=f.review(&module,json!({"verdict":"accepted","summary":"Whole Module checked","actor":"reviewer",
        "checks":[{"target":"M-001/A-001","label":"scenario","status":"passed","detail":"Independent scenario rerun"}]}),false).await;
    assert!(reply.contains("Module phase: accepted"), "{reply}");
    for view in ["results", "checks"] {
        let text = f
            .call(
                "get_context",
                json!({"project":"alpha","ref":module,"view":view}),
                false,
            )
            .await;
        assert!(text.contains("M-001/A-001"), "{text}");
    }
    let search = f
        .call(
            "search",
            json!({"project":"alpha","query":"Verification"}),
            false,
        )
        .await;
    assert!(
        search.contains("M-001/A-001") && search.contains("5 matches"),
        "{search}"
    );
    let atom = f
        .call(
            "get_context",
            json!({"project":"alpha","ref":"M-001/A-001"}),
            false,
        )
        .await;
    assert!(
        atom.contains("Atomic executor") && atom.contains("session-atomic"),
        "{atom}"
    );
    f.record(
        "M-001/A-001",
        json!({"op":"reopen","reason":"New scenario"}),
        false,
    )
    .await;
    assert_eq!(
        f.store().module(&module).unwrap().value.phase(),
        "stale approval"
    );
}

/// Standalone Atomic done is lightweight acceptance, and semantic edits cannot reuse its old completion.
#[tokio::test]
async fn core_epic_atomic_completion_conditions_and_edits() {
    let f = Fixture::new();
    f.init().await;
    let atomic=core_create_kind(&f,"create_atomic",json!({"title":"Root verification","outcome":"Observe outcome","required_checks":["scenario"]})).await;
    for status in ["failed", "not_run", "not_applicable"] {
        f.record(&atomic,json!({"op":"result","state":"done","summary":"Verification outcome","checks":[{"label":"scenario","status":status}]}),true).await;
    }
    let done=f.record(&atomic,json!({"op":"result","state":"done","summary":"Verification outcome","checks":[{"label":"scenario","status":"passed"}]}),false).await;
    assert!(done.contains("Atomic phase: done"), "{done}");
    let before = f.store().module(&atomic).unwrap().bytes;
    f.record(&atomic,json!({"op":"result","summary":"New gap","gaps":["Unfinished"],"checks":[{"label":"scenario","status":"passed"}]}),true).await;
    assert_eq!(f.store().module(&atomic).unwrap().bytes, before);
    f.plan(
        &atomic,
        json!({"op":"edit_atomic","ref":atomic,"outcome":"Different expected outcome"}),
        false,
    )
    .await;
    assert!(!f.store().module(&atomic).unwrap().value.completed);
    f.record(&atomic,json!({"op":"result","state":"done","summary":"New outcome verified","checks":[{"label":"scenario","status":"passed"}]}),false).await;
    f.record(&atomic,json!({"op":"blocker","problem":"Environment missing","needed_action":"Restore environment"}),false).await;
    assert!(!f.store().module(&atomic).unwrap().value.completed);
    let clear = f
        .record(
            &atomic,
            json!({"op":"clear_blocker","reason":"Environment restored"}),
            false,
        )
        .await;
    assert!(!clear.contains("Atomic phase: done"), "{clear}");
    f.call("review_work",json!({"project":"alpha","ref":atomic,"version":f.version(&atomic).await,"verdict":"accepted","summary":"Not a review target"}),true).await;
}

/// Membership is unique, cancellation never cascades, and unknown ownership preserves healthy reads.
#[tokio::test]
async fn core_epic_atomic_membership_unknown_and_canceled_parent() {
    let f = Fixture::new();
    f.init().await;
    let module = f.module(vec![], None).await;
    let epic = core_create_kind(
        &f,
        "create_epic",
        json!({"title":"Delivery","outcome":"Complete delivery","criteria":["Required outcome"]}),
    )
    .await;
    let other=core_create_kind(&f,"create_epic",json!({"title":"Other delivery","outcome":"Different outcome","criteria":["Other criterion"]})).await;
    f.plan(
        &epic,
        json!({"op":"edit_epic","epic":epic,"modules":[module]}),
        false,
    )
    .await;
    f.plan(
        &other,
        json!({"op":"edit_epic","epic":other,"modules":[module]}),
        true,
    )
    .await;
    f.plan(
        &epic,
        json!({"op":"edit_epic","epic":epic,"modules":["M-999"]}),
        true,
    )
    .await;
    f.record(&module,json!({"op":"result","summary":"Useful preserved report","checks":[{"label":"readable","status":"passed"}]}),false).await;
    let original = fs::read(f.root.join("epics/E-002.yaml")).unwrap();
    fs::write(f.root.join("epics/E-002.yaml"), "unknown: broken").unwrap();
    let own = f
        .call(
            "get_context",
            json!({"project":"alpha","ref":module,"view":"results"}),
            false,
        )
        .await;
    assert!(
        own.contains("PARTIAL") && own.contains("Useful preserved report"),
        "{own}"
    );
    f.record(
        &module,
        json!({"op":"handoff","stopping_point":"Preserve","next_action":"Resume"}),
        true,
    )
    .await;
    fs::write(f.root.join("epics/E-002.yaml"), original).unwrap();
    f.record(
        &epic,
        json!({"op":"cancel","reason":"Resolve remaining scope"}),
        true,
    )
    .await;
    f.record(
        &module,
        json!({"op":"cancel","reason":"Canceled child scope"}),
        false,
    )
    .await;
    f.record(
        &epic,
        json!({"op":"cancel","reason":"All child scope resolved"}),
        false,
    )
    .await;
    f.record(
        &module,
        json!({"op":"reopen","reason":"New child work"}),
        true,
    )
    .await;
    f.record(
        &epic,
        json!({"op":"reopen","reason":"Resume parent"}),
        false,
    )
    .await;
    f.record(
        &module,
        json!({"op":"reopen","reason":"Resume child"}),
        false,
    )
    .await;
    assert_eq!(f.store().module(&epic).unwrap().value.modules, vec![module]);
}

/// Integration dependencies outside Epic membership invalidate Epic write/read snapshots and old acceptance.
#[tokio::test]
async fn core_epic_atomic_transitive_versions_and_stale_receipts() {
    let f = Fixture::new();
    f.init().await;
    let module = f.module(vec![], None).await;
    f.record(
        &module,
        json!({"op":"result","summary":"Original participant outcome"}),
        false,
    )
    .await;
    let atomic = core_create_kind(
        &f,
        "create_atomic",
        json!({"title":"Integration","outcome":"Combined scenario","participants":[module]}),
    )
    .await;
    f.record(
        &atomic,
        json!({"op":"result","state":"done","summary":"Integration observed"}),
        false,
    )
    .await;
    let epic=core_create_kind(&f,"create_epic",json!({"title":"Integration Epic","outcome":"Combined outcome","criteria":["Scenario works"]})).await;
    f.plan(
        &epic,
        json!({"op":"edit_epic","epic":epic,"atomics":[atomic]}),
        false,
    )
    .await;
    f.record(&epic,json!({"op":"result","summary":"Combined acceptance outcome","artifacts":["report-one","report-two"]}),false).await;
    core_review_entity(&f, &epic, false).await;
    let version = f.version(&epic).await;
    let page = f
        .call(
            "get_context",
            json!({"project":"alpha","ref":epic,"view":"results","limit":1}),
            false,
        )
        .await;
    let snapshot = field(&page, "Snapshot version: ");
    f.record(
        &module,
        json!({"op":"result","summary":"Participant outcome changed"}),
        false,
    )
    .await;
    f.call("review_work",json!({"project":"alpha","ref":epic,"version":version,"verdict":"accepted","summary":"Stale observation","actor":"reviewer"}),true).await;
    f.call("get_context",json!({"project":"alpha","ref":epic,"view":"results","limit":1,"start":1,"version":snapshot}),true).await;
    assert_eq!(
        f.store().phase(&f.store().module(&atomic).unwrap().value),
        "stale completion"
    );
    assert_eq!(
        f.store().phase(&f.store().module(&epic).unwrap().value),
        "stale approval"
    );
    let noop = f
        .plan(&epic, json!({"op":"edit_epic","epic":epic}), false)
        .await;
    assert!(noop.contains("Epic phase: stale approval"), "{noop}");
    let noop = f
        .record(
            &atomic,
            json!({"op":"clear_handoff","reason":"No pending handoff"}),
            false,
        )
        .await;
    assert!(noop.contains("Atomic phase: stale completion"), "{noop}");
    let status = f
        .call("project_status", json!({"project":"alpha"}), false)
        .await;
    assert!(
        status.contains("stale completion") && status.contains("stale approval"),
        "{status}"
    );
    f.record(
        &atomic,
        json!({"op":"result","state":"done","summary":"Integration observed again"}),
        false,
    )
    .await;
    core_review_entity(&f, &epic, false).await;
}

/// New allocator metadata preserves unpublished gaps and refuses lost counters, while old roots stay untouched on read.
#[tokio::test]
async fn core_epic_atomic_allocators_gaps_roots_and_legacy() {
    let f = Fixture::new();
    f.init().await;
    let mut allocator: Allocator =
        store::decode(&fs::read(f.root.join(".agent-tasks/state.yaml")).unwrap()).unwrap();
    assert_eq!(allocator.schema_version, 2);
    allocator.next_epic = Some(7);
    allocator.next_atomic = Some(4);
    fs::write(
        f.root.join(".agent-tasks/state.yaml"),
        store::encode(&allocator).unwrap(),
    )
    .unwrap();
    let epic = core_create_kind(
        &f,
        "create_epic",
        json!({"title":"Reserved gap","outcome":"Keep IDs monotonic","criteria":["No reuse"]}),
    )
    .await;
    assert_eq!(epic, "E-007");
    let atomic = core_create_kind(
        &f,
        "create_atomic",
        json!({"title":"Reserved Atomic gap","outcome":"Keep Atomic IDs monotonic"}),
    )
    .await;
    assert_eq!(atomic, "A-004");
    let other = Fixture::new();
    other.init().await;
    assert_eq!(core_create_kind(&other,"create_epic",json!({"title":"Other root","outcome":"Independent numbering","criteria":["Independent"]})).await,"E-001");
    let modern = other.root.join(".agent-tasks/state.yaml");
    fs::write(&modern, "schema_version: 2\nnext_module: 1\nnext_epic: 2\n").unwrap();
    let version = other.allocation().await;
    other.call("plan_work",json!({"project":"alpha","version":version,"op":"create_atomic","title":"No guessing","outcome":"Refuse lost counter"}),true).await;
    let legacy = Fixture::new();
    legacy.init().await;
    let old = b"schema_version: 1\nnext_module: 1\n";
    fs::write(legacy.root.join(".agent-tasks/state.yaml"), old).unwrap();
    legacy
        .call("get_context", json!({"project":"alpha"}), false)
        .await;
    assert_eq!(
        fs::read(legacy.root.join(".agent-tasks/state.yaml")).unwrap(),
        old
    );
    assert_eq!(
        core_create_kind(
            &legacy,
            "create_atomic",
            json!({"title":"New kind in old root","outcome":"Explicit write upgrades allocator"})
        )
        .await,
        "A-001"
    );
    let new: Allocator =
        store::decode(&fs::read(legacy.root.join(".agent-tasks/state.yaml")).unwrap()).unwrap();
    assert_eq!(new.schema_version, 2);
}

/// Healthy owners retain context and can detach unknown dependencies without weakening file safety.
#[tokio::test]
async fn core_epic_atomic_unknown_dependency_recovery() {
    let f = Fixture::new();
    f.init().await;
    let module = f.module(vec![], None).await;
    let epic = core_create_kind(
        &f,
        "create_epic",
        json!({"title":"Recoverable Epic","outcome":"Keep intent","criteria":["Preserve"]}),
    )
    .await;
    f.plan(
        &epic,
        json!({"op":"edit_epic","epic":epic,"modules":[module]}),
        false,
    )
    .await;
    let atomic=core_create_kind(&f,"create_atomic",json!({"title":"Recoverable integration","outcome":"Keep own result","participants":[module]})).await;
    let child = f.root.join("modules/M-001.yaml");
    let original = fs::read(&child).unwrap();
    for oversized in [true, false] {
        if oversized {
            fs::write(&child, vec![b'x'; store::RECORD_CAP + 1]).unwrap();
        } else {
            fs::remove_file(&child).unwrap();
            fs::create_dir(&child).unwrap();
        }
        for owner in [&epic, &atomic] {
            let context = f
                .call("get_context", json!({"project":"alpha","ref":owner}), false)
                .await;
            assert!(
                context.contains("PARTIAL") && context.contains("M-001"),
                "{context}"
            );
        }
        core_review_entity(&f, &epic, true).await;
        f.plan(
            &epic,
            json!({"op":"edit_epic","epic":epic,"modules":[]}),
            false,
        )
        .await;
        f.plan(
            &atomic,
            json!({"op":"edit_atomic","ref":atomic,"participants":[]}),
            false,
        )
        .await;
        if !oversized {
            fs::remove_dir(&child).unwrap();
        }
        fs::write(&child, &original).unwrap();
        f.plan(
            &epic,
            json!({"op":"edit_epic","epic":epic,"modules":[module]}),
            false,
        )
        .await;
        f.plan(
            &atomic,
            json!({"op":"edit_atomic","ref":atomic,"participants":[module]}),
            false,
        )
        .await;
    }
}

/// New purpose-based variants remain closed and required patches never accept null.
#[test]
fn core_epic_atomic_closed_shapes() {
    for mut args in [
        json!({"op":"create_epic","title":"Epic","outcome":"Outcome","criteria":["Criterion"]}),
        json!({"op":"edit_epic","epic":"E-001","modules":[],"atomics":[]}),
        json!({"op":"create_atomic","title":"Atomic","outcome":"Outcome","participants":[]}),
        json!({"op":"add_atomic","module":"M-001","title":"Atomic","outcome":"Outcome"}),
        json!({"op":"edit_atomic","ref":"A-001","outcome":"Changed"}),
    ] {
        args["project"] = json!("alpha");
        args["version"] = json!("a".repeat(64));
        assert!(input::mutation::<input::Plan>(args.clone(), false).is_ok());
        args["unknown"] = json!("No untyped patches");
        assert!(input::mutation::<input::Plan>(args, false).is_err());
    }
    for args in [
        json!({"op":"edit_epic","epic":"E-001","modules":null}),
        json!({"op":"edit_atomic","ref":"A-001","outcome":null}),
        json!({"op":"create_epic","title":"Epic","outcome":"Outcome"}),
    ] {
        let mut args = args;
        args["project"] = json!("alpha");
        args["version"] = json!("a".repeat(64));
        assert!(input::mutation::<input::Plan>(args, false).is_err());
    }
    assert!(!input_shape::<input::ReviewWorkArgs>(
        json!({"project":"alpha","ref":"E-001","version":"a".repeat(64),"verdict":"accepted","summary":"Review","unknown":true})
    ));
}

/// Degraded mutation rendering preserves exact Epic/Atomic targets, versions and real saved effects.
#[tokio::test]
async fn core_epic_atomic_render_fallback_preserves_creation() {
    let f = Fixture::new();
    f.init().await;
    let broken = Templates::new(&[]).unwrap();
    for (op, reference, extra) in [
        (
            "create_epic",
            "E-001",
            json!({"criteria":["Required outcome"]}),
        ),
        ("create_atomic", "A-001", json!({})),
    ] {
        let mut args = extra;
        args["op"] = json!(op);
        args["project"] = json!("alpha");
        args["version"] = json!(f.allocation().await);
        args["title"] = json!("Recoverable publication");
        args["outcome"] = json!("Inspect exact target without replay");
        let reply = super::call("plan_work", args, &f.identity, &broken, &f.config)
            .await
            .unwrap();
        assert_eq!(reply.is_error, Some(false));
        let value = serde_json::to_value(reply).unwrap();
        let text = value["content"][0]["text"].as_str().unwrap();
        assert!(
            text.contains(reference)
                && text.contains("Presentation degraded")
                && text.contains("Creation target"),
            "{text}"
        );
        assert!(f.store().module(reference).is_ok());
    }
}

/// Faults after allocator or owner publication disclose exact E/A targets and preserve inspect-before-retry recovery.
#[tokio::test]
async fn core_epic_atomic_partial_publication_recovery() {
    let f = Fixture::new();
    f.init().await;
    for (op, first, second, extra) in [
        (
            "create_epic",
            "E-001",
            "E-002",
            json!({"criteria":["Keep scope"]}),
        ),
        ("create_atomic", "A-001", "A-002", json!({})),
    ] {
        let mut args = extra.clone();
        args["op"] = json!(op);
        args["title"] = json!("Partial publication");
        args["outcome"] = json!("Retain reserved gaps");
        args["project"] = json!("alpha");
        args["version"] = json!(f.allocation().await);
        store::fail_next_directory_sync();
        let failed = f.call("plan_work", args, true).await;
        assert!(
            failed.contains("durability_unknown")
                && failed.contains(first)
                && failed.contains(".agent-tasks/state.yaml"),
            "{failed}"
        );
        assert!(
            f.store().module(first).is_err(),
            "Allocator publication must not imply owner publication"
        );
        let mut next = extra;
        next["title"] = json!("Recovered creation");
        next["outcome"] = json!("Use next reservation");
        assert_eq!(core_create_kind(&f, op, next).await, second);
        let version = f.version(second).await;
        store::fail_next_directory_sync();
        let failed=f.call("record_work",json!({"project":"alpha","ref":second,"version":version,"op":"result","summary":"Visible owner report"}),true).await;
        assert!(
            failed.contains(second) && failed.contains("durability_unknown"),
            "{failed}"
        );
        let current = f.store().module(second).unwrap();
        assert_eq!(
            current.value.result.as_ref().unwrap().summary,
            "Visible owner report"
        );
        f.call("record_work",json!({"project":"alpha","ref":second,"version":version,"op":"result","summary":"Never blindly replay"}),true).await;
        assert_eq!(f.store().module(second).unwrap().bytes, current.bytes);
    }
}

/// Prepare one modern Module using reported fixture paths; this helper never claims those paths are Git proof.
async fn unified_module(f: &Fixture, tasks: Vec<Value>, checks: Vec<&str>) -> String {
    core_create_kind(f,"create_module",json!({"title":"Modern Module","outcome":"Deliver a coherent result","lead":{"name":"lead","handle":"session-lead"},
        "criteria":["Outcome is locally verified"],"required_checks":checks,"tasks":tasks,"contracts":{"not_required":true},
        "execution":{"repository":f.directory.path(),"worktree":f.directory.path(),"branch":"feature","target_branch":"main"}})).await
}

/// Record a modern reported begin with a fresh owning Version.
async fn unified_begin(f: &Fixture, reference: &str) {
    f.record(reference, json!({"op":"begin","actor":"lead"}), false)
        .await;
}

/// Independently review complete modern work using current context.
async fn unified_review(f: &Fixture, reference: &str, error: bool) -> String {
    f.call("review_work",json!({"project":"alpha","ref":reference,"version":f.version(reference.split('/').next().unwrap()).await,
        "verdict":"accepted","summary":"Current whole result independently checked","actor":"reviewer"}),error).await
}

/// Record local Module delivery after review; no hosting PR or Git mutation is invented.
async fn unified_deliver(f: &Fixture, reference: &str) {
    f.record(reference,json!({"op":"deliver","target_branch":"main","summary":"Locally merged into the declared target","actor":"lead"}),false).await;
}

/// Modern Tasks remain the lead's local decision; positive Module review and delivery are separate closure gates.
#[tokio::test]
async fn core_unified_task_authority_review_and_delivery() {
    let f = Fixture::modern();
    f.init().await;
    let m = unified_module(
        &f,
        vec![json!({"title":"Manual observation","criterion":"Observe the expected result"})],
        vec!["whole"],
    )
    .await;
    f.record(
        "M-001/T-001",
        json!({"op":"result","summary":"Before begin","state":"done","actor":"lead"}),
        true,
    )
    .await;
    unified_begin(&f, &m).await;
    f.record("M-001/T-001",json!({"op":"result","summary":"Manually observed behavior","checks":[{"label":"manual","status":"passed","detail":"Lead checked the visible scenario"}]}),false).await;
    assert_eq!(
        f.store().module(&m).unwrap().value.tasks[0].state,
        TaskState::Open
    );
    f.record(
        "M-001/T-001",
        json!({"op":"complete","actor":"helper"}),
        true,
    )
    .await;
    f.record(
        "M-001/T-001",
        json!({"op":"complete","actor":"lead"}),
        false,
    )
    .await;
    f.call("review_work",json!({"project":"alpha","ref":"M-001/T-001","version":f.version(&m).await,"verdict":"accepted","summary":"No Task review","actor":"reviewer"}),true).await;
    unified_review(&f, &m, true).await;
    f.record(&m,json!({"op":"result","summary":"Whole outcome verified","checks":[{"label":"whole","status":"passed"}],"actor":"lead"}),false).await;
    unified_review(&f, &m, false).await;
    let before = f.store().module(&m).unwrap();
    assert_eq!(f.store().phase(&before.value), "reviewed; delivery pending");
    f.record(
        &m,
        json!({"op":"deliver","target_branch":"other","summary":"Wrong target"}),
        true,
    )
    .await;
    unified_deliver(&f, &m).await;
    let delivered = f.store().module(&m).unwrap();
    assert_eq!(f.store().phase(&delivered.value), "accepted");
    assert_eq!(before.value.review_epoch, delivered.value.review_epoch);
    assert_eq!(before.value.reviews.len(), delivered.value.reviews.len());
    f.record(
        &m,
        json!({"op":"handoff","stopping_point":"Preserved","next_action":"Observe"}),
        false,
    )
    .await;
    assert_eq!(
        f.store().phase(&f.store().module(&m).unwrap().value),
        "accepted"
    );
    f.plan(
        &m,
        json!({"op":"edit_module","module":m,"outcome":"Changed implementation obligation"}),
        false,
    )
    .await;
    let changed = f.store().module(&m).unwrap();
    assert!(changed.value.workflow.as_ref().unwrap().delivery.is_none());
    assert_eq!(f.store().phase(&changed.value), "stale approval");
}

/// Reciprocal obligations are valid; only explicit waits and parent closure edges form blocking cycles.
#[tokio::test]
async fn core_unified_contracts_waits_roster_and_unknowns() {
    let f = Fixture::modern();
    f.init().await;
    let m1 = unified_module(&f, vec![], vec![]).await;
    let m2 = unified_module(&f, vec![], vec![]).await;
    for (m, peer) in [(&m1, &m2), (&m2, &m1)] {
        f.plan(m,json!({"op":"edit_module","module":m,"contracts":{"not_required":false,
            "provides":[{"peer":peer,"description":"Supplies event and error behavior","reference":"docs/contract.md","ready":true}],
            "consumes":[{"peer":peer,"description":"Consumes the peer response","ready":true}]}}),false).await;
    }
    unified_begin(&f, &m1).await;
    unified_begin(&f, &m2).await;
    let context = f
        .call("get_context", json!({"project":"alpha","ref":m1}), false)
        .await;
    assert!(
        context.contains("provides M-002") && context.contains("consumes M-002"),
        "{context}"
    );
    f.plan(&m1,json!({"op":"edit_module","module":m1,"dependencies":[{"ref":m2,"condition":"accepted","reason":"Need result"}]}),false).await;
    f.plan(&m2,json!({"op":"edit_module","module":m2,"dependencies":[{"ref":m1,"condition":"accepted","reason":"Would deadlock"}]}),true).await;
    f.plan(
        &m1,
        json!({"op":"edit_module","module":m1,"dependencies":[]}),
        false,
    )
    .await;
    let e=core_create_kind(&f,"create_epic",json!({"title":"Frozen delivery","outcome":"Compose the Module","criteria":["Combined outcome"]})).await;
    f.plan(&e, json!({"op":"edit_epic","epic":e,"modules":[m1]}), false)
        .await;
    f.plan(&m1,json!({"op":"edit_module","module":m1,"dependencies":[{"ref":e,"condition":"accepted","reason":"Impossible own-parent wait"}]}),true).await;
    unified_begin(&f, &e).await;
    f.plan(&e, json!({"op":"edit_epic","epic":e,"modules":[]}), true)
        .await;
    f.record(&e, json!({"op":"reopen","reason":"New round"}), false)
        .await;
    f.plan(
        &e,
        json!({"op":"edit_epic","epic":e,"modules":[m1,m2]}),
        true,
    )
    .await;
    let m3 = unified_module(&f, vec![], vec![]).await;
    f.plan(&m3,json!({"op":"edit_module","module":m3,"dependencies":[{"ref":e,"condition":"accepted","reason":"Explicit later Epic dependency"}]}),false).await;
    f.record(&m3, json!({"op":"begin"}), true).await;
    assert!(f.store().parent(&m3).unwrap().is_none());
    let original = fs::read(f.root.join("modules/M-002.yaml")).unwrap();
    fs::write(f.root.join("modules/M-002.yaml"), "broken: fields").unwrap();
    let healthy = f
        .call("get_context", json!({"project":"alpha","ref":m1}), false)
        .await;
    assert!(
        healthy.contains("PARTIAL") && healthy.contains("M-002") && healthy.contains("Version:"),
        "{healthy}"
    );
    f.record(&m1, json!({"op":"begin"}), true).await;
    fs::write(f.root.join("modules/M-002.yaml"), original).unwrap();
}

/// Every embedded Atomic has current independent approval; both the Module lead and distinct executor cannot self-review.
#[tokio::test]
async fn core_unified_atomic_independence_counts_and_basis() {
    let f = Fixture::modern();
    f.init().await;
    let m = unified_module(&f, vec![], vec![]).await;
    unified_begin(&f, &m).await;
    f.plan(&m,json!({"op":"add_atomic","module":m,"title":"Atomic outcome","outcome":"Observed check","executor":{"name":"executor"},"required_checks":["check"]}),false).await;
    unified_begin(&f, "M-001/A-001").await;
    f.record("M-001/A-001",json!({"op":"result","summary":"Actual outcome","state":"done","actor":"executor","checks":[{"label":"check","status":"failed"}]}),false).await;
    let pending = f
        .call("project_status", json!({"project":"alpha"}), false)
        .await;
    assert!(
        pending.contains("Atomics: 0 done / 1") && pending.contains("1 unfinished/stale"),
        "{pending}"
    );
    for actor in ["lead", "executor"] {
        f.call("review_work",json!({"project":"alpha","ref":"M-001/A-001","version":f.version(&m).await,"verdict":"accepted","summary":"Self review","actor":actor}),true).await;
    }
    unified_review(&f, "M-001/A-001", true).await;
    f.record("M-001/A-001",json!({"op":"result","summary":"Corrected outcome","state":"done","actor":"executor","checks":[{"label":"check","status":"passed"}]}),false).await;
    unified_review(&f, "M-001/A-001", false).await;
    assert_eq!(
        f.store().module(&m).unwrap().value.atomics[0].atomic_phase(),
        "accepted"
    );
    let page = f
        .call(
            "get_context",
            json!({"project":"alpha","ref":"M-001/A-001","view":"review"}),
            false,
        )
        .await;
    assert!(
        page.contains("Current whole result independently checked")
            && page.contains("Applicability: current"),
        "{page}"
    );
    unified_review(&f, &m, false).await;
    unified_deliver(&f, &m).await;
    f.record(
        "M-001/A-001",
        json!({"op":"reopen","reason":"New obligation"}),
        false,
    )
    .await;
    assert_eq!(
        f.store().module(&m).unwrap().value.atomics[0].atomic_phase(),
        "stale approval"
    );
    assert_eq!(
        f.store().phase(&f.store().module(&m).unwrap().value),
        "stale approval"
    );
}

/// Modern Epic acceptance needs an independently reviewed integration whose participants exactly match its active frozen roster.
#[tokio::test]
async fn core_unified_exact_epic_composition() {
    let f = Fixture::modern();
    f.init().await;
    let m1 = unified_module(&f, vec![], vec![]).await;
    let m2 = unified_module(&f, vec![], vec![]).await;
    let m3 = unified_module(&f, vec![], vec![]).await;
    let e=core_create_kind(&f,"create_epic",json!({"title":"Composition","outcome":"Real joined outcome","criteria":["Both Modules interact"]})).await;
    f.plan(
        &e,
        json!({"op":"edit_epic","epic":e,"modules":[m1,m2]}),
        false,
    )
    .await;
    unified_begin(&f, &e).await;
    for m in [&m1, &m2, &m3] {
        unified_begin(&f, m).await;
        f.record(
            m,
            json!({"op":"result","summary":"Whole Module outcome","actor":"lead"}),
            false,
        )
        .await;
        unified_review(&f, m, false).await;
        unified_deliver(&f, m).await;
    }
    f.record(
        &e,
        json!({"op":"result","summary":"Business criteria observed"}),
        false,
    )
    .await;
    unified_review(&f, &e, true).await;
    for participants in [
        vec![],
        vec![m1.clone()],
        vec![m1.clone(), m2.clone(), m3.clone()],
    ] {
        let a=core_create_kind(&f,"create_atomic",json!({"title":"Insufficient composition","outcome":"Reported scenario","executor":{"name":"integrator"},
            "participants":participants,"environment":"actual fixture","scenarios":["joint scenario"]})).await;
        unified_begin(&f, &a).await;
        f.record(&a,json!({"op":"result","summary":"Scenario observed","state":"done","actor":"integrator"}),false).await;
        unified_review(&f, &a, false).await;
        f.plan(&e, json!({"op":"edit_epic","epic":e,"atomics":[a]}), false)
            .await;
        unified_review(&f, &e, true).await;
    }
    let a=core_create_kind(&f,"create_atomic",json!({"title":"Exact composition","outcome":"Joined scenario","executor":{"name":"integrator"},
        "participants":[m1,m2],"environment":"actual fixture","scenarios":["real interaction"]})).await;
    f.plan(&e, json!({"op":"edit_epic","epic":e,"atomics":[a]}), false)
        .await;
    unified_begin(&f, &a).await;
    f.record(&a,json!({"op":"result","summary":"Real combined outcome","state":"done","actor":"integrator"}),false).await;
    unified_review(&f, &a, false).await;
    unified_review(&f, &e, false).await;
    assert_eq!(
        f.store().phase(&f.store().module(&e).unwrap().value),
        "accepted"
    );
    let snapshot = f
        .store()
        .participant_basis(&[m1.clone(), m2.clone()])
        .unwrap();
    f.record(
        &m1,
        json!({"op":"handoff","stopping_point":"Same implementation","next_action":"Observe"}),
        false,
    )
    .await;
    assert_eq!(
        f.store()
            .participant_basis(&[m1.clone(), m2.clone()])
            .unwrap(),
        snapshot
    );
    f.record(
        &m1,
        json!({"op":"reopen","reason":"Implementation changed"}),
        false,
    )
    .await;
    assert_eq!(
        f.store().phase(&f.store().module(&a).unwrap().value),
        "stale approval"
    );
    assert_eq!(
        f.store().phase(&f.store().module(&e).unwrap().value),
        "stale approval"
    );
}

/// Legacy fields remain grandfathered until explicit begin; a modern Epic cannot count an unreviewed legacy Atomic as accepted.
#[tokio::test]
async fn core_unified_legacy_declarations_and_opt_in() {
    let old = Fixture::new();
    old.init().await;
    let a = core_create_kind(
        &old,
        "create_atomic",
        json!({"title":"Legacy Atomic","outcome":"Legacy result"}),
    )
    .await;
    old.record(
        &a,
        json!({"op":"result","summary":"Legacy done","state":"done"}),
        false,
    )
    .await;
    let e = core_create_kind(
        &old,
        "create_epic",
        json!({"title":"Legacy parent","outcome":"Own outcome","criteria":["Own criterion"]}),
    )
    .await;
    old.plan(&e, json!({"op":"edit_epic","epic":e,"atomics":[a]}), false)
        .await;
    old.plan(&a,json!({"op":"edit_atomic","ref":a,"executor":{"name":"executor"},"execution":{"repository":old.directory.path(),"worktree":old.directory.path(),"branch":"main","target_branch":"main"}}),false).await;
    assert!(!old.store().module(&a).unwrap().value.modern());
    assert_eq!(
        old.store().phase(&old.store().module(&a).unwrap().value),
        "working"
    ); // meaningful plan change resets old completion
    old.record(
        &a,
        json!({"op":"result","summary":"Legacy done after declarations","state":"done"}),
        false,
    )
    .await;
    assert_eq!(
        old.store().phase(&old.store().module(&a).unwrap().value),
        "done"
    );
    old.record(
        &e,
        json!({"op":"result","summary":"Own parent outcome"}),
        false,
    )
    .await;
    unified_review(&old, &e, false).await;
    old.record(&e, json!({"op":"begin"}), false).await;
    unified_review(&old, &e, true).await;
    old.record(&a, json!({"op":"begin"}), false).await;
    old.record(&a, json!({"op":"complete"}), false).await;
    unified_review(&old, &a, false).await;
    unified_review(&old, &e, false).await;
    assert!(old.store().module(&a).unwrap().value.modern());
}

/// Actual local Git import captures integration basis only on a new observation and cannot erase reports after a late failure.
#[tokio::test]
async fn core_unified_git_integration_import_and_author() {
    let f = Fixture::modern();
    f.init().await;
    let m = unified_module(&f, vec![], vec![]).await;
    unified_begin(&f, &m).await;
    f.record(
        &m,
        json!({"op":"result","summary":"Delivered participant"}),
        false,
    )
    .await;
    unified_review(&f, &m, false).await;
    unified_deliver(&f, &m).await;
    let repo = f.directory.path().join("source");
    fs::create_dir(&repo).unwrap();
    let run = |args: &[&str]| {
        std::process::Command::new("git")
            .arg("-C")
            .arg(&repo)
            .args(args)
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_AUTHOR_NAME", "Author")
            .env("GIT_AUTHOR_EMAIL", "author@example.invalid")
            .env("GIT_COMMITTER_NAME", "Author")
            .env("GIT_COMMITTER_EMAIL", "author@example.invalid")
            .output()
            .unwrap()
    };
    assert!(run(&["init", "-b", "feature"]).status.success());
    fs::write(repo.join("file"), "source").unwrap();
    assert!(run(&["add", "file"]).status.success());
    assert!(
        run(&[
            "-c",
            "commit.gpgsign=false",
            "commit",
            "-m",
            "test: integration",
            "-m",
            "Result:\nActual combined behavior\nChecks:\npassed | scenario | Locally observed"
        ])
        .status
        .success()
    );
    let sha = String::from_utf8(run(&["rev-parse", "HEAD"]).stdout)
        .unwrap()
        .trim()
        .to_owned();
    let a=core_create_kind(&f,"create_atomic",json!({"title":"Imported integration","outcome":"Actual composition","executor":{"name":"executor"},"participants":[m],
        "environment":"local temporary Git","scenarios":["scenario"],"required_checks":["scenario"],
        "execution":{"repository":repo,"worktree":repo,"branch":"feature","target_branch":"main"}})).await;
    unified_begin(&f, &a).await;
    f.record(
        &a,
        json!({"op":"import_commits","commits":[sha],"state":"done","actor":"reporter"}),
        false,
    )
    .await;
    assert!(
        !f.store()
            .module(&a)
            .unwrap()
            .value
            .participant_basis
            .is_empty()
    );
    f.call("review_work",json!({"project":"alpha","ref":a,"version":f.version(&a).await,"verdict":"accepted","summary":"Self review","actor":"reporter"}),true).await;
    unified_review(&f, &a, false).await;
    let before = f.store().module(&a).unwrap().bytes;
    f.record(
        &a,
        json!({"op":"import_commits","commits":[sha,"deadbee"],"actor":"reporter"}),
        true,
    )
    .await;
    assert_eq!(f.store().module(&a).unwrap().bytes, before);
    let old = f.version(&a).await;
    let noop = f
        .record(
            &a,
            json!({"op":"import_commits","commits":[sha],"actor":"reporter"}),
            false,
        )
        .await;
    assert!(noop.starts_with("UNCHANGED"));
    assert_eq!(f.version(&a).await, old);
    let view = f
        .call(
            "get_context",
            json!({"project":"alpha","ref":a,"view":"commits"}),
            false,
        )
        .await;
    assert!(
        view.contains("Original message") && view.contains("Actual combined behavior"),
        "{view}"
    );
    f.record(
        &m,
        json!({"op":"reopen","reason":"New participant scope"}),
        false,
    )
    .await;
    assert_eq!(
        f.store().phase(&f.store().module(&a).unwrap().value),
        "stale approval"
    );
    f.record(
        &a,
        json!({"op":"import_commits","commits":[sha],"actor":"reporter"}),
        false,
    )
    .await;
    assert_eq!(
        f.store().phase(&f.store().module(&a).unwrap().value),
        "stale approval",
        "Duplicate import cannot fake new integration evidence"
    );
}

/// Parent cancellation and unrelated unknown ownership block decisions without fabricating intrinsic implementation drift.
#[tokio::test]
async fn core_unified_intrinsic_approval_survives_parent_and_unknown_scope() {
    let f = Fixture::modern();
    f.init().await;
    let m = unified_module(&f, vec![], vec![]).await;
    let e = core_create_kind(
        &f,
        "create_epic",
        json!({"title":"Parent","outcome":"Organize scope","criteria":["Organized"]}),
    )
    .await;
    f.plan(&e, json!({"op":"edit_epic","epic":e,"modules":[m]}), false)
        .await;
    unified_begin(&f, &e).await;
    unified_begin(&f, &m).await;
    f.record(
        &m,
        json!({"op":"result","summary":"Reviewed implementation"}),
        false,
    )
    .await;
    unified_review(&f, &m, false).await;
    unified_deliver(&f, &m).await;
    let before = f.store().module(&m).unwrap();
    f.record(
        &e,
        json!({"op":"cancel","reason":"Business scope withdrawn"}),
        false,
    )
    .await;
    assert_eq!(
        f.store().phase(&f.store().module(&m).unwrap().value),
        "accepted"
    );
    assert!(f.store().module(&m).unwrap().value.delivered());
    assert_eq!(
        before.bytes,
        f.store().module(&m).unwrap().bytes,
        "Parent transition is not a child cascade"
    );
    f.record(
        &m,
        json!({"op":"handoff","stopping_point":"Same","next_action":"Resume"}),
        true,
    )
    .await;
    f.record(&e, json!({"op":"reopen","reason":"Parent resumed"}), false)
        .await;
    f.record(
        &m,
        json!({"op":"result","summary":"No implicit parent begin"}),
        true,
    )
    .await;
    unified_begin(&f, &e).await;
    fs::write(
        f.root.join("epics/E-009.yaml"),
        "invalid: unrelated ownership",
    )
    .unwrap();
    let status = f
        .call("project_status", json!({"project":"alpha"}), false)
        .await;
    assert!(
        status.contains("PARTIAL")
            && status.contains("Modules: 1 accepted / 1 readable")
            && status.contains("ownership unknown"),
        "{status}"
    );
    assert_eq!(
        f.store().phase(&f.store().module(&m).unwrap().value),
        "accepted"
    );
    f.record(&m, json!({"op":"begin"}), true).await;
    assert_eq!(before.bytes, f.store().module(&m).unwrap().bytes);
}
