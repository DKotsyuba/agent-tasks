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
