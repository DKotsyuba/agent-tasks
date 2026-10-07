//! Focused qualification of typed record, document and proposal context plus the unified lexical
//! search, driven through the real registry router over disposable roots.
//!
//! Records are created through the same locked producer handlers the write tools use, so the reads
//! see real persisted files; nothing here touches an installed configuration.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "Fixture construction and explicit assertions"
)]
use super::{RefKind, classify};
use crate::markdown::{self, Wire};
use crate::{
    compaction::{
        self,
        env::Env,
        fake::FakeEnv,
        ops,
        record::{ActionKind, Disposition, DropKind},
        validate::{ActionIn, ProposalIn, SectionIn, SourceIn},
    },
    response::Templates,
    store::{Config, Store},
    tools::{input, knowledge_ops},
};
use mcp_presentation::Renderer;
use serde_json::{Value, json};
use std::{fs, path::PathBuf};

/// One disposable portable root and lazy configuration for alias `alpha`.
struct Host {
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
    fn new() -> Self {
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
            templates: Templates::new(&crate::tools::templates()).unwrap(),
            identity: Renderer::new().unwrap(),
        }
    }

    /// Run one real tool call; asserts the error flag and the 8 KiB reply budget.
    async fn call(&self, name: &str, args: Value, error: bool) -> String {
        let reply = crate::tools::call(name, args, &self.identity, &self.templates, &self.config)
            .await
            .unwrap();
        assert_eq!(reply.is_error, Some(error), "{reply:?}");
        let wire = serde_json::to_value(reply).unwrap();
        let text = wire["content"][0]["text"].as_str().unwrap().to_owned();
        assert!(text.len() <= 8192, "{}", text.len());
        text
    }

    /// One labeled value from compact text.
    fn field(text: &str, label: &str) -> String {
        text.lines()
            .find_map(|line| line.strip_prefix(label))
            .unwrap_or_else(|| panic!("{label} in {text}"))
            .to_owned()
    }

    /// Current work creation precondition from the project context.
    async fn allocation(&self) -> String {
        let text = self
            .call("get_context", json!({"project":"alpha"}), false)
            .await;
        Self::field(&text, "Allocation version: ")
    }

    /// Current knowledge creation precondition from the project context.
    async fn knowledge_token(&self) -> String {
        let text = self
            .call("get_context", json!({"project":"alpha"}), false)
            .await;
        Self::field(&text, "Knowledge allocation version: ")
    }

    /// Current whole-file version of one record, document or proposal reference.
    async fn version(&self, reference: &str) -> String {
        let text = self
            .call(
                "get_context",
                json!({"project":"alpha","ref":reference}),
                false,
            )
            .await;
        Self::field(&text, "Version: ")
    }

    /// Initialize the project and add one Module with one Task whose title mentions "cutover".
    async fn work(&self) {
        let version = self.allocation().await;
        self.call(
            "plan_work",
            json!({"project":"alpha","op":"init_project","version":version,"title":"Portable work","purpose":"Preserve strategic intent"}),
            false,
        )
        .await;
        let version = self.allocation().await;
        self.call(
            "plan_work",
            json!({"project":"alpha","op":"create_module","version":version,"title":"Cutover module","outcome":"Ship the cutover"}),
            false,
        )
        .await;
    }

    /// The resolved store of the alias.
    fn store(&self) -> Store {
        self.config.resolve("alpha").unwrap()
    }

    /// Run one knowledge operation under the real write lock; returns the new target version.
    fn knowledge(&self, version: &str, mut op: Value) -> String {
        op["project"] = json!("alpha");
        op["version"] = json!(version);
        op["actor"] = json!("tester");
        let (common, op) = input::mutation::<knowledge_ops::KnowledgeOp>(op, false).unwrap();
        let store = self.store();
        let guard = store.lock(true, &mut Vec::new()).unwrap().unwrap();
        knowledge_ops::execute_locked(&store, &guard, &common, op, &mut Vec::new())
            .unwrap()
            .version
    }

    /// Place an unmanaged Markdown file exactly as found; managed saves need the allocator
    /// owner's document id source, which the read path must not depend on.
    fn file(&self, path: &str, body: &str) {
        let full = self.root.join(path);
        fs::create_dir_all(full.parent().unwrap()).unwrap();
        fs::write(full, body.as_bytes()).unwrap();
    }
}

/// Classification is by identifier shape, so work routes are untouched and typed ones divert.
#[test]
fn references_classify_into_their_domain() {
    for (reference, kind) in [
        ("M-001", RefKind::Work),
        ("M-001/T-001", RefKind::Work),
        ("E-001", RefKind::Work),
        ("A-001", RefKind::Work),
        ("D-001", RefKind::Knowledge),
        ("RB-012", RefKind::Knowledge),
        ("RS-001", RefKind::Knowledge),
        ("CL-001/I-001", RefKind::Knowledge),
        ("DOC-001", RefKind::Document),
        ("docs/guide.md", RefKind::Document),
        ("README.md", RefKind::Document),
        ("CP-003", RefKind::Compaction),
    ] {
        assert_eq!(classify(reference), kind, "{reference}");
    }
}

/// Decisions show current text, supersession, history and references with honest currentness;
/// views and selectors that do not apply are refused naming the field.
#[tokio::test]
async fn decisions_render_current_history_references_and_supersession() {
    let host = Host::new();
    host.work().await;
    let token = host.knowledge_token().await;
    host.knowledge(
        &token,
        json!({"op":"create_decision","title":"Pick store","question":"Which store?","decision":"Use files.","rationale":"Portable."}),
    );
    let token = host.knowledge_token().await;
    host.knowledge(
        &token,
        json!({"op":"create_decision","title":"Pick store again","question":"Which store?","decision":"Use files and Git.","rationale":"Auditable."}),
    );
    let one = host.version("D-001").await;
    let edited = host.knowledge(
        &one,
        json!({"op":"edit_decision","ref":"D-001","rationale":"Portable and simple."}),
    );
    host.knowledge(
        &edited,
        json!({"op":"supersede","ref":"D-001","successor":"D-002"}),
    );
    let summary = host
        .call(
            "get_context",
            json!({"project":"alpha","ref":"D-001"}),
            false,
        )
        .await;
    assert!(summary.contains("SUPERSEDED by D-002"), "{summary}");
    assert!(summary.contains("Decision: Use files."), "{summary}");
    assert!(summary.contains("Rationale: Portable and simple."));
    let history = host
        .call(
            "get_context",
            json!({"project":"alpha","ref":"D-001","view":"history"}),
            false,
        )
        .await;
    assert!(history.contains("Revision 1"), "{history}");
    let current = host
        .call(
            "get_context",
            json!({"project":"alpha","ref":"D-002"}),
            false,
        )
        .await;
    assert!(!current.contains("SUPERSEDED"), "{current}");
    let refs = host
        .call(
            "get_context",
            json!({"project":"alpha","ref":"D-002","view":"references"}),
            false,
        )
        .await;
    assert!(refs.contains("Incoming"), "{refs}");
    for (args, needle) in [
        (json!({"ref":"D-001","view":"content"}), "view"),
        (json!({"ref":"D-001","heading":"x"}), "heading"),
        (json!({"ref":"M-001","view":"history"}), "view"),
        (json!({"ref":"DOC-001","view":"history"}), "view"),
    ] {
        let mut args = args;
        args["project"] = json!("alpha");
        let error = host.call("get_context", args.clone(), true).await;
        assert!(error.contains(needle), "{args} -> {error}");
    }
    let missing = host
        .call(
            "get_context",
            json!({"project":"alpha","ref":"D-099"}),
            true,
        )
        .await;
    assert!(missing.contains("not_found"), "{missing}");
}

/// A checklist reads item by item with its resolution and never copies canonical Tasks.
#[tokio::test]
async fn checklists_read_whole_and_per_item() {
    let host = Host::new();
    host.work().await;
    let token = host.knowledge_token().await;
    let version = host.knowledge(
        &token,
        json!({"op":"create_checklist","title":"Cutover","purpose":"Steps","items":["Freeze writes","Flip alias"]}),
    );
    host.knowledge(
        &version,
        json!({"op":"resolve_item","ref":"CL-001","item":"I-001","state":"done","text":"Frozen at noon"}),
    );
    let whole = host
        .call(
            "get_context",
            json!({"project":"alpha","ref":"CL-001"}),
            false,
        )
        .await;
    assert!(whole.contains("I-001 [done]: Freeze writes"), "{whole}");
    assert!(whole.contains("I-002 [open]: Flip alias"), "{whole}");
    assert!(whole.contains("1 open, 1 done, 0 canceled"), "{whole}");
    assert!(whole.contains("reuse canonical Tasks"), "{whole}");
    let item = host
        .call(
            "get_context",
            json!({"project":"alpha","ref":"CL-001/I-001"}),
            false,
        )
        .await;
    assert!(item.contains("Resolution: Frozen at noon"), "{item}");
    let absent = host
        .call(
            "get_context",
            json!({"project":"alpha","ref":"CL-001/I-009"}),
            true,
        )
        .await;
    assert!(absent.contains("not_found"), "{absent}");
}

/// Verbatim document pages: every reply fits 8 KiB, the framed payloads rebuild the exact bytes
/// (CRLF, multibyte text and a stray CR included), and the snapshot never carries the offset.
#[tokio::test]
async fn document_pages_rebuild_exact_bytes_with_offset_free_snapshots() {
    let host = Host::new();
    host.work().await;
    let mut body = String::from("Preamble é\r\n\r\n## Big\r\n");
    for n in 0..900 {
        body.push_str(&format!("line {n} — ünïcode ✓ with a lone\rCR\r\n"));
    }
    body.push_str("## Tail\r\nend\r\n");
    host.file("docs/guide.md", &body);
    let mut rebuilt = Vec::new();
    let (mut start, mut snapshot, mut pages) = (0usize, None::<String>, 0);
    loop {
        let mut args =
            json!({"project":"alpha","ref":"docs/guide.md","view":"content","start":start});
        if let Some(snapshot) = &snapshot {
            args["version"] = json!(snapshot);
        }
        let text = host.call("get_context", args, false).await;
        let seen = Host::field(&text, "Snapshot version: ");
        assert!(
            snapshot.as_ref().is_none_or(|s| *s == seen),
            "stable snapshot"
        );
        snapshot = Some(seen);
        let frame = text
            .lines()
            .find(|l| l.starts_with("Content: md-text-v1"))
            .unwrap();
        let length: usize = frame.split("encoded_len=").nth(1).unwrap().parse().unwrap();
        let payload_start = text.find(frame).unwrap() + frame.len() + 1;
        let payload = &text[payload_start..payload_start + length];
        let wire = if frame.contains("wire=escaped") {
            Wire::Escaped
        } else {
            assert!(frame.contains("wire=raw"), "{frame}");
            Wire::Raw
        };
        rebuilt.extend(markdown::decode(wire, payload).unwrap());
        pages += 1;
        match text
            .lines()
            .find_map(|l| l.strip_prefix("Next: start="))
            .map(|n| n.split(';').next().unwrap().parse::<usize>().unwrap())
        {
            Some(next) => start = next,
            None => break,
        }
        assert!(pages < 100);
    }
    assert!(pages > 1, "a large document needs several pages");
    assert_eq!(rebuilt, body.as_bytes(), "verbatim reassembly");
    let stale = host
        .call(
            "get_context",
            json!({"project":"alpha","ref":"docs/guide.md","view":"content","start":10,"version":"0".repeat(64)}),
            true,
        )
        .await;
    assert!(stale.contains("stale"), "{stale}");
    let unpinned = host
        .call(
            "get_context",
            json!({"project":"alpha","ref":"docs/guide.md","view":"content","start":10}),
            true,
        )
        .await;
    assert!(unpinned.contains("stale"), "{unpinned}");
    let section = host
        .call(
            "get_context",
            json!({"project":"alpha","ref":"docs/guide.md","view":"content","heading":"Tail"}),
            false,
        )
        .await;
    assert!(
        section.contains("section \"Tail\"") && section.contains("end"),
        "{section}"
    );
    assert!(section.contains("no continuation"), "{section}");
    let limit = host
        .call(
            "get_context",
            json!({"project":"alpha","ref":"docs/guide.md","view":"content","limit":5}),
            true,
        )
        .await;
    assert!(limit.contains("limit"), "{limit}");
}

/// A work search with no `kinds` covers work, knowledge and documents with kind, state and an
/// exact read route on every row; `kinds` and `state` narrow it, and bad combinations are refused.
#[tokio::test]
async fn search_unifies_work_knowledge_and_documents_with_state_and_routes() {
    let host = Host::new();
    host.work().await;
    let token = host.knowledge_token().await;
    host.knowledge(
        &token,
        json!({"op":"create_decision","title":"Cutover plan","question":"How to cut over?","decision":"Flip the alias.","rationale":"Fast."}),
    );
    let token = host.knowledge_token().await;
    host.knowledge(
        &token,
        json!({"op":"create_decision","title":"Cutover plan v2","question":"How to cut over?","decision":"Flip the alias after a freeze.","rationale":"Safe."}),
    );
    let one = host.version("D-001").await;
    host.knowledge(
        &one,
        json!({"op":"supersede","ref":"D-001","successor":"D-002"}),
    );
    let token = host.knowledge_token().await;
    host.knowledge(
        &token,
        json!({"op":"create_checklist","title":"Steps","purpose":"Cutover steps","items":["Freeze writes"]}),
    );
    host.file(
        "docs/ops.md",
        "Intro text\n\n## Rollback plan\nUndo the cutover by flipping back.\n\n## Rollback plan\nsecond cutover note\n",
    );
    let all = host
        .call(
            "search",
            json!({"project":"alpha","query":"cutover"}),
            false,
        )
        .await;
    for needle in [
        "M-001 \"Cutover module\" [work ",
        "D-001 \"Cutover plan\" [knowledge superseded]",
        "D-002 \"Cutover plan v2\" [knowledge current]",
        "CL-001 \"Steps\" [knowledge ",
        "[document unmanaged]",
        "open: get_context ref=D-002",
        "Coverage knowledge:",
        "Coverage documents:",
    ] {
        assert!(all.contains(needle), "{needle} in {all}");
    }
    let doc_row = all
        .lines()
        .find(|l| l.starts_with("docs/ops.md"))
        .expect(&all);
    assert!(
        doc_row.contains(
            "open: get_context ref=docs/ops.md heading=\"Rollback plan\" occurrence=1 level=2 view=content"
        ),
        "{doc_row}"
    );
    let routed = host
        .call(
            "get_context",
            json!({"project":"alpha","ref":"docs/ops.md","heading":"Rollback plan","occurrence":1,"level":2,"view":"content"}),
            false,
        )
        .await;
    assert!(routed.contains("Undo the cutover"), "{routed}");
    let current = host
        .call(
            "search",
            json!({"project":"alpha","query":"cutover","kinds":["knowledge"],"state":"current"}),
            false,
        )
        .await;
    assert!(
        current.contains("D-002") && !current.contains("D-001 "),
        "{current}"
    );
    assert!(
        !current.contains("M-001") && !current.contains("docs/ops.md"),
        "{current}"
    );
    let superseded = host
        .call(
            "search",
            json!({"project":"alpha","query":"cutover","state":"superseded"}),
            false,
        )
        .await;
    assert!(
        superseded.contains("D-001 ") && !superseded.contains("D-002 "),
        "{superseded}"
    );
    let scoped = host
        .call(
            "search",
            json!({"project":"alpha","query":"cutover","module":"M-001"}),
            false,
        )
        .await;
    assert!(
        scoped.contains("M-001") && !scoped.contains("D-002"),
        "{scoped}"
    );
    for bad in [
        json!({"kinds":[]}),
        json!({"kinds":["work","work"]}),
        json!({"kinds":["document"],"module":"M-001"}),
        json!({"kinds":["nonsense"]}),
    ] {
        let mut args = json!({"project":"alpha","query":"cutover"});
        args.as_object_mut()
            .unwrap()
            .extend(bad.as_object().unwrap().clone());
        let error = host.call("search", args.clone(), true).await;
        assert!(error.contains("invalid_arguments"), "{args} -> {error}");
    }
}

/// Search pages are pinned: continuing after any searched source changed is refused as stale.
#[tokio::test]
async fn search_continuation_is_pinned_to_every_searched_source() {
    let host = Host::new();
    host.work().await;
    host.file("docs/a.md", "alpha beta\n");
    host.file("docs/b.md", "alpha gamma\n");
    let first = host
        .call(
            "search",
            json!({"project":"alpha","query":"alpha","kinds":["document"],"limit":1}),
            false,
        )
        .await;
    let next = Host::field(&first, "Next: start=");
    let (start, rest) = next.split_once("; version=").unwrap();
    let snapshot = rest.split(';').next().unwrap();
    let second = host
        .call(
            "search",
            json!({"project":"alpha","query":"alpha","kinds":["document"],"limit":1,"start":start.parse::<u64>().unwrap(),"version":snapshot}),
            false,
        )
        .await;
    assert!(second.contains("docs/b.md"), "{second}");
    host.file("docs/a.md", "alpha beta changed\n");
    let stale = host
        .call(
            "search",
            json!({"project":"alpha","query":"alpha","kinds":["document"],"limit":1,"start":1,"version":snapshot}),
            true,
        )
        .await;
    assert!(stale.contains("stale"), "{stale}");
}

/// An unreadable typed record makes search and project coverage PARTIAL and is named, never zero.
#[tokio::test]
async fn unreadable_records_make_coverage_partial_not_empty() {
    let host = Host::new();
    host.work().await;
    let token = host.knowledge_token().await;
    host.knowledge(
        &token,
        json!({"op":"create_decision","title":"Cutover plan","question":"Q?","decision":"Flip.","rationale":"Fast."}),
    );
    fs::write(host.root.join("decisions/D-002.yaml"), "not: [valid").unwrap();
    let found = host
        .call(
            "search",
            json!({"project":"alpha","query":"cutover","kinds":["knowledge"]}),
            false,
        )
        .await;
    assert!(found.contains("Data coverage: PARTIAL"), "{found}");
    assert!(found.contains("D-001"), "{found}");
    assert!(found.contains("knowledge unreadable"), "{found}");
    let project = host
        .call("get_context", json!({"project":"alpha"}), false)
        .await;
    assert!(project.contains("Data coverage: PARTIAL"), "{project}");
    assert!(
        project.contains("Knowledge: Decisions 1 current"),
        "{project}"
    );
}

/// A compaction proposal reads as summary, actions/sections, review and references.
#[test]
fn compaction_proposals_read_through_context() {
    let env = FakeEnv::new();
    env.seed("docs/a.md", "Intro\n## S1\none\n## S2\ntwo\n", true);
    let candidate = "Intro\n## S1\none\n";
    let sections: Vec<SectionIn> = ["", "## S1\none\n", "## S2\ntwo\n"]
        .iter()
        .enumerate()
        .map(|(n, _)| n)
        .map(|n| (n, env.observe("docs/a.md").unwrap()))
        .flat_map(|(n, doc)| {
            let outline = crate::compaction::fake::outline_of(&doc.body.unwrap());
            outline.sections.into_iter().nth(n)
        })
        .map(|s| {
            let kept = candidate
                .as_bytes()
                .windows(s.bytes.len())
                .any(|w| w == s.bytes.as_slice());
            SectionIn {
                path: "docs/a.md".into(),
                section: s.addr,
                disposition: if kept {
                    Disposition::Kept {
                        action: "A-01".into(),
                    }
                } else {
                    Disposition::Dropped {
                        kind: DropKind::Obsolete,
                        reason: "no longer needed".into(),
                    }
                },
            }
        })
        .collect();
    let input = ProposalIn {
        title: "Trim A".into(),
        sources: vec![SourceIn {
            path: "docs/a.md".into(),
            version: env.observe("docs/a.md").unwrap().version,
        }],
        actions: vec![ActionIn {
            id: "A-01".into(),
            kind: ActionKind::Replace,
            path: "docs/a.md".into(),
            from: None,
            base_version: None,
            content: Some(candidate.into()),
            purpose: None,
            reason: "compaction".into(),
            absorbed_into: vec![],
        }],
        sections,
        preservation: vec![],
    };
    let version = env.allocation_version().unwrap();
    env.start_call();
    let out = ops::propose(
        &env,
        "author",
        &version,
        "key-0001-aa",
        &input,
        &mut Vec::new(),
    );
    env.finish_call(out.is_ok());
    assert_eq!(out.unwrap().id, "CP-001");
    let templates = Templates::new(&crate::tools::templates()).unwrap();
    let read = |view: &str| {
        let args: input::ContextArgs =
            serde_json::from_value(json!({"project":"alpha","ref":"CP-001","view":view})).unwrap();
        let kind = classify("CP-001");
        super::validate(&args, kind)
            .and_then(|()| super::context(env.store(), &args, kind, &templates))
    };
    let summary = read("summary").unwrap();
    assert!(
        summary.contains("State: proposed") && summary.contains("Trim A"),
        "{summary}"
    );
    assert!(summary.contains("Actions 0/1 applied"), "{summary}");
    let tasks = read("tasks").unwrap();
    assert!(
        tasks.contains("Action A-01") && tasks.contains("Section "),
        "{tasks}"
    );
    assert!(
        read("review")
            .unwrap()
            .contains("No review has been recorded")
    );
    let references = read("references").unwrap();
    assert!(
        references.contains("Source: ") && references.contains("docs/a.md"),
        "{references}"
    );
    assert_eq!(read("content").err().unwrap().code, "invalid_arguments");
    assert_eq!(
        compaction::read_cp(env.store(), "CP-009")
            .err()
            .unwrap()
            .code,
        "cp_not_found"
    );
}
