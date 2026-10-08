//! Regressions for scoped context guidance and Epic member roll-up (observed defects AT-001 and
//! AT-004), driven through the real registry router on a disposable root.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "Fixture construction and explicit assertions"
)]
use crate::{response::Templates, store::Config};
use mcp_presentation::Renderer;
use serde_json::{Value, json};
use std::{fs, path::PathBuf};

/// One disposable portable root and lazy configuration; no installed state is touched.
pub(super) struct Host {
    /// Keeps the temporary directory alive through the assertions.
    _directory: tempfile::TempDir,
    /// Configured documentation root, initially absent.
    root: PathBuf,
    /// Lazy location selection for alias `alpha`.
    config: Config,
    /// Trusted normal templates.
    templates: Templates,
    /// Identity renderer used by the router.
    identity: Renderer,
}

impl Host {
    /// Build alias `alpha` for a root that does not exist yet.
    pub(super) fn new() -> Self {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path().join("docs");
        let config_path = directory.path().join("config.toml");
        fs::write(&config_path, "schema_version = 1\n").unwrap();
        fs::write(
            directory.path().join("projects.toml"),
            format!(
                "schema_version = 1\n[aliases]\nalpha = {}\n",
                serde_json::to_string(&root).unwrap()
            ),
        )
        .unwrap();
        Self {
            _directory: directory,
            root,
            config: Config::new(Some(config_path)),
            templates: Templates::new(&super::templates()).unwrap(),
            identity: Renderer::new().unwrap(),
        }
    }

    /// Resolve a test-owned path beneath the disposable fixture directory.
    pub(super) fn path(&self, relative: &str) -> PathBuf {
        self._directory.path().join(relative)
    }

    /// Run one real tool call and return its text, asserting the error flag and the budget.
    pub(super) async fn call(&self, name: &str, args: Value, error: bool) -> String {
        let reply = super::call(name, args, &self.identity, &self.templates, &self.config)
            .await
            .unwrap();
        assert_eq!(reply.is_error, Some(error), "{reply:?}");
        let wire = serde_json::to_value(reply).unwrap();
        let text = wire["content"][0]["text"].as_str().unwrap().to_owned();
        assert!(text.len() <= 8192, "{}", text.len());
        text
    }

    /// Read one labeled exact precondition from compact text.
    pub(super) fn field(text: &str, label: &str) -> String {
        text.lines()
            .find_map(|line| line.strip_prefix(label))
            .expect(text)
            .to_owned()
    }

    /// Current creation precondition from the project context.
    async fn allocation(&self) -> String {
        let text = self
            .call("get_context", json!({"project":"alpha"}), false)
            .await;
        Self::field(&text, "Allocation version: ")
    }

    /// Current whole-file version of one record from its context.
    async fn version(&self, id: &str) -> String {
        let text = self
            .call("get_context", json!({"project":"alpha","ref":id}), false)
            .await;
        Self::field(&text, "Version: ")
    }

    /// Create the project, one Epic with two Modules and one standalone Atomic.
    pub(super) async fn populate(&self) {
        let version = self.allocation().await;
        self.call(
            "plan_work",
            json!({"project":"alpha","op":"init_project","version":version,"title":"Portable work","purpose":"Preserve strategic intent"}),
            false,
        )
        .await;
        for title in ["First module", "Second module"] {
            let version = self.allocation().await;
            self.call(
                "plan_work",
                json!({"project":"alpha","op":"create_module","version":version,"title":title,"outcome":"Do the work"}),
                false,
            )
            .await;
        }
        let version = self.allocation().await;
        self.call(
            "plan_work",
            json!({"project":"alpha","op":"create_epic","version":version,"title":"Whole delivery","outcome":"Everything ships","criteria":["It ships"]}),
            false,
        )
        .await;
        let version = self.version("E-001").await;
        self.call(
            "plan_work",
            json!({"project":"alpha","op":"edit_epic","epic":"E-001","version":version,"modules":["M-001","M-002"]}),
            false,
        )
        .await;
        let version = self.allocation().await;
        self.call(
            "plan_work",
            json!({"project":"alpha","op":"create_atomic","version":version,"title":"Integration","outcome":"Pieces fit"}),
            false,
        )
        .await;
    }
}

/// Epic and Atomic context stop offering Module-only guidance while Module text is unchanged.
#[tokio::test]
async fn scoped_guidance_names_the_right_review_route() {
    let host = Host::new();
    host.populate().await;
    let epic = host
        .call(
            "get_context",
            json!({"project":"alpha","ref":"E-001"}),
            false,
        )
        .await;
    assert!(
        epic.contains("review_work for a whole-Epic verdict"),
        "{epic}"
    );
    assert!(epic.contains("verify_criterion"), "{epic}");
    assert!(!epic.contains("review_module"), "{epic}");
    let atomic = host
        .call(
            "get_context",
            json!({"project":"alpha","ref":"A-001"}),
            false,
        )
        .await;
    assert!(
        atomic.contains("review_work for an independent Atomic verdict"),
        "{atomic}"
    );
    assert!(!atomic.contains("review_module"), "{atomic}");
    let module = host
        .call(
            "get_context",
            json!({"project":"alpha","ref":"M-001"}),
            false,
        )
        .await;
    assert!(
        module.contains("review_module for a whole-module verdict"),
        "{module}"
    );
}

/// An Epic scope rolls up its members, names an unreadable one and never reports complete zero.
#[tokio::test]
async fn epic_scope_rolls_up_members_and_names_the_unreadable() {
    let host = Host::new();
    host.populate().await;
    let clean = host
        .call(
            "project_status",
            json!({"project":"alpha","module":"E-001"}),
            false,
        )
        .await;
    assert!(clean.contains("Data coverage: complete"), "{clean}");
    assert!(
        clean.contains("Scope: Epic E-001 and its declared members"),
        "{clean}"
    );
    assert!(
        clean.contains("M-001") && clean.contains("M-002"),
        "{clean}"
    );
    fs::write(host.root.join("modules/M-002.yaml"), b"not: [valid").unwrap();
    let status = host
        .call(
            "project_status",
            json!({"project":"alpha","module":"E-001"}),
            false,
        )
        .await;
    assert!(status.contains("Data coverage: PARTIAL"), "{status}");
    assert!(status.contains("UNREADABLE \"M-002"), "{status}");
    assert!(status.contains("1 member(s) unreadable"), "{status}");
    assert!(status.contains("M-001"), "{status}");
    let context = host
        .call(
            "get_context",
            json!({"project":"alpha","ref":"E-001"}),
            false,
        )
        .await;
    assert!(
        context.contains("Direct Epic Tasks (members excluded)"),
        "{context}"
    );
    assert!(
        context.contains("Members: 1 readable of 2 declared"),
        "{context}"
    );
    assert!(
        context.contains("Counts are lower bounds. Unreadable: M-002"),
        "{context}"
    );
    assert!(context.contains("Data coverage: PARTIAL"), "{context}");
}

/// Six reciprocal Modules retain stale agreement histories while current facts decode once.
/// Exact rollups, bounded status, immutable same-call observations and fresh drift/unknown checks
/// run on generated portable records; no private project content enters this regression.
#[tokio::test]
async fn reciprocal_contract_reads_share_one_validated_scope() {
    use crate::{
        model::{Agreement, Contract, Contracts, Module},
        store::{self, Store},
    };
    let host = Host::new();
    host.populate().await;
    let store = Store::from_root(&host.root).unwrap();
    let seed = store.module("M-001").unwrap().value;
    let at = seed.updated_at.clone();
    let mut modules = Vec::new();
    for i in 0..6 {
        let mut m = seed.clone();
        m.id = format!("M-{:03}", i + 1);
        m.log.clear();
        m.title = "Generated Module".repeat(16);
        m.outcome = "Generated outcome. ".repeat(50);
        m.tasks = vec![serde_json::from_value(json!({
            "id":"T-001","title":"Generated task. ".repeat(16),"criterion":null,"required_checks":[],
            "state":"open","result":null,"checks":[],"cancellation":null,"cancellation_history":[],
            "created_at":m.created_at,"updated_at":m.updated_at
        })).unwrap()];
        m.next_task = Some(2);
        let provides = Contract {
            peer: format!("M-{:03}", (i + 1) % 6 + 1),
            description: "Generated boundary".into(),
            reference: Some(format!("artifact:boundary-{i}")),
            ready: true,
            id: Some(format!("boundary-{i}")),
            revision: Some(2),
        };
        let previous = (i + 5) % 6;
        let consumes = Contract {
            peer: format!("M-{:03}", previous + 1),
            description: "Generated consumer".into(),
            reference: Some(format!("artifact:boundary-{previous}")),
            ready: true,
            id: Some(format!("boundary-{previous}")),
            revision: Some(2),
        };
        let contracts = Contracts {
            not_required: false,
            provides: vec![provides],
            consumes: vec![consumes],
        };
        m.workflow_mut().contracts = Some(contracts.clone());
        let mut historical = contracts;
        for entry in historical
            .provides
            .iter_mut()
            .chain(&mut historical.consumes)
        {
            entry.revision = Some(1);
        }
        m.core_mut().unwrap().contract_history.push(historical);
        for id in store.contract_ids(&m) {
            for _ in 0..16 {
                m.core_mut().unwrap().agreements.push(Agreement {
                    contract_id: id.clone(),
                    revision: 1,
                    summary: "Retained generated confirmation. ".repeat(24),
                    snapshot: "0".repeat(64),
                    actor: "generated-lead".into(),
                    at: at.clone(),
                });
            }
        }
        m.validate().unwrap();
        fs::write(
            host.root.join(format!("modules/{}.yaml", m.id)),
            store::encode(&m).unwrap(),
        )
        .unwrap();
        modules.push(m);
    }
    let mut epic = store.module("E-001").unwrap().value;
    epic.modules = modules.iter().map(|m| m.id.clone()).collect();
    fs::write(
        host.root.join("epics/E-001.yaml"),
        store::encode(&epic).unwrap(),
    )
    .unwrap();
    let scope = store.clone().for_work_read();
    for m in &mut modules {
        for id in store.contract_ids(m) {
            let facts = scope.contract_facts(&id).unwrap();
            m.core_mut().unwrap().agreements.push(Agreement {
                contract_id: id,
                revision: facts.revision,
                summary: "Current generated confirmation".into(),
                snapshot: facts.snapshot,
                actor: "generated-lead".into(),
                at: at.clone(),
            });
        }
        m.validate().unwrap();
        fs::write(
            host.root.join(format!("modules/{}.yaml", m.id)),
            store::encode(m).unwrap(),
        )
        .unwrap();
    }
    let _lock = store.lock(false, &mut Vec::new()).unwrap();
    let scope = store.clone().for_work_read();
    store::DECODE_COUNT.with(|c| c.set(0));
    let facts = scope.contract_facts("boundary-0").unwrap();
    for m in &modules {
        assert!(scope.agreement_gaps(m).is_empty());
    }
    let mut reviewed = modules[0].clone();
    reviewed.reviews.push(
        serde_json::from_value(json!({
            "verdict":"accepted","summary":"Generated retained verdict","findings":[],
            "basis":scope.work_basis(&reviewed).unwrap(),"epoch":reviewed.review_epoch,
            "at":at,"reviewer":"generated-reviewer","check_updates":[]
        }))
        .unwrap(),
    );
    assert!(
        !scope.module_ready(&reviewed),
        "missing current planning/boundary evidence blocks readiness"
    );
    assert_eq!(scope.phase(&reviewed), "stale approval");
    assert!(!scope.core_review_gaps(&reviewed).is_empty());
    assert_eq!(
        store::DECODE_COUNT.with(|c| c.get()),
        8,
        "one decode per inventoried work record"
    );
    assert_eq!(facts.parties, vec!["M-001", "M-002"]);
    assert!(facts.gaps.is_empty());
    store::DECODE_COUNT.with(|c| c.set(0));
    let status_started = std::time::Instant::now();
    let current = host
        .call(
            "project_status",
            json!({"project":"alpha","module":"E-001"}),
            false,
        )
        .await;
    assert!(current.contains("Modules: 0 current-reviewed / 6 readable. Tasks: 0 done / 6 readable; 6 open; 0 canceled."),"{current}");
    assert!(current.contains("contract attention=0"), "{current}");
    let decodes = store::DECODE_COUNT.with(|c| c.get());
    eprintln!(
        "Generated full status: elapsed={:?} decodes={decodes} bytes={}",
        status_started.elapsed(),
        current.len()
    );
    assert_eq!(
        decodes, 9,
        "status decodes the manifest and eight work records once"
    );
    for id in [
        "E-001", "M-001", "M-002", "M-003", "M-004", "M-005", "M-006", "A-001", "M-999",
    ] {
        assert_eq!(
            scope.scan(Some(id)).unwrap().version,
            store.scan(Some(id)).unwrap().version,
            "selected byte snapshot algorithm for {id}"
        );
        if id != "M-999" {
            assert_eq!(
                scope.module(id).unwrap().version,
                store.module(id).unwrap().version,
                "transitive write-token algorithm for {id}"
            );
        }
    }
    assert_eq!(
        scope.scan(None).unwrap().version,
        store.scan(None).unwrap().version
    );
    assert!(
        current.contains("omitted") && current.contains("detail coverage: PARTIAL"),
        "{current}"
    );
    let before = scope.module("M-001").unwrap();
    let mut changed: Module = modules[1].clone();
    changed.workflow_mut().contracts.as_mut().unwrap().consumes[0].description =
        "Native affecting drift".into();
    fs::write(
        host.root.join("modules/M-002.yaml"),
        store::encode(&changed).unwrap(),
    )
    .unwrap();
    assert_eq!(
        scope.contract_facts("boundary-0").unwrap().snapshot,
        facts.snapshot
    );
    assert!(
        scope.agreement_gaps(&modules[0]).is_empty(),
        "same immutable locked read scope"
    );
    assert_eq!(
        scope.work_basis(&reviewed).unwrap(),
        reviewed.reviews[0].basis
    );
    let fresh = store.clone().for_work_read();
    assert_ne!(
        fresh.work_basis(&reviewed).unwrap(),
        reviewed.reviews[0].basis
    );
    assert_ne!(
        fresh.contract_facts("boundary-0").unwrap().snapshot,
        facts.snapshot
    );
    assert!(
        !fresh.agreement_gaps(&modules[0]).is_empty(),
        "fresh call must stale earlier confirmations"
    );
    assert_ne!(
        store.module("M-001").unwrap().version,
        before.version,
        "write dependency versions remain fresh"
    );
    assert_eq!(
        scope.module("M-001").unwrap().version,
        before.version,
        "read token stays pinned"
    );
    let mut effects = Vec::new();
    assert_eq!(
        scope.clone().lock(true, &mut effects).err().unwrap().code,
        "read_only"
    );
    assert_eq!(
        scope
            .save("modules/M-001.yaml", &modules[0], None, false, &mut effects)
            .unwrap_err()
            .code,
        "read_only"
    );
    assert!(effects.is_empty());
    drop(_lock);
    let write_guard = store.lock(true, &mut effects).unwrap().unwrap();
    assert!(
        !write_guard.is_for(&scope),
        "writable sibling cannot authorize the read view"
    );
    drop(write_guard);
    let refused = host
        .call(
            "plan_work",
            json!({"project":"alpha","op":"edit_module","module":"M-001",
        "version":before.version,"title":"Reject stale snapshot token"}),
            true,
        )
        .await;
    assert!(refused.contains("stale"), "{refused}");
    let stale = host
        .call(
            "project_status",
            json!({"project":"alpha","module":"M-001"}),
            false,
        )
        .await;
    assert!(stale.contains("contract attention=2"), "{stale}");
    fs::write(host.root.join("modules/M-006.yaml"), b"not: [valid").unwrap();
    assert!(
        store
            .clone()
            .for_work_read()
            .contract_facts("boundary-0")
            .is_err(),
        "incomplete scope cannot certify a healthy subset"
    );
    let partial = host
        .call(
            "project_status",
            json!({"project":"alpha","module":"E-001"}),
            false,
        )
        .await;
    assert!(
        partial.contains("Data coverage: PARTIAL") && partial.contains("UNREADABLE \"M-006"),
        "{partial}"
    );
    assert!(
        partial.contains("Tasks: 0 done / 5 readable; 5 open; 0 canceled.")
            && partial.contains("1 member(s) unreadable"),
        "{partial}"
    );
    assert!(
        partial.contains("unreadable/incomplete Module facts"),
        "{partial}"
    );
    fs::write(
        host.root.join("modules/M-006.yaml"),
        vec![b' '; store::RECORD_CAP + 1],
    )
    .unwrap();
    let capped = store.clone().for_work_read();
    let capped_scan = capped.scan(None).unwrap();
    assert!(!capped_scan.complete);
    assert!(
        capped_scan
            .unreadable
            .iter()
            .any(|s| s.contains("M-006: Record exceeds its read cap."))
    );
    assert_eq!(capped_scan.version, store.scan(None).unwrap().version);
    assert_eq!(
        capped.module("M-001").unwrap().version,
        store.module("M-001").unwrap().version,
        "unknown/capped dependencies keep the original fresh token algorithm"
    );
    assert!(capped.contract_facts("boundary-0").is_err());
}
