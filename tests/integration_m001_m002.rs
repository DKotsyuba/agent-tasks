//! Joint real-binary proof of the accepted M-001 (typed knowledge) and M-002 (managed Markdown) providers (A-002).
//!
//! Scope is exactly the two reciprocal boundaries `doc-reference` r2 (provider M-002) and `kr-allocator` r6
//! (provider M-001) on the assembled candidate; nothing here accepts any other Module. Every scenario starts the
//! shipped binary through the Rust MCP SDK stdio client over a disposable independent documentation repository.
//! Faults are produced only by natively written files and ordinary Git state; no mock stands in for either provider.
//!
//! Scenarios: S1 current reference grammar and shared allocator (`a2`, `a3`), S2 guarded typed detail and Markdown
//! interactions (`a3`, `a4`, `a7`), S3 partial and retired-reference coverage and exact reads (`a5`, `a6`).
//! `a8` covers relocation: the move keeps the DOC identity while its reply names the stored Decision detail and the
//! Markdown path link it introduced as dangling, and the DOC-id token stays resolving.
//! `a1` pins every owned provider byte string to the exact accepted Git blob.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "Explicit protocol assertions"
)]
mod support;

use serde_json::json;
use std::path::Path;
use support::{
    Project, commit_count, commit_paths, document, git, git_status, head, knowledge, read_document,
    save, staged, target, tree,
};

/// One owned provider file: path, accepted candidate commit and the exact accepted Git blob id.
type Pin = (&'static str, &'static str, &'static str);

/// Accepted M-001 candidate commit (`kr-allocator` r6 provider).
const M001: &str = "e393d73d955598c344dafa31f56d6e2225efab16";
/// Accepted M-002 candidate commit (`doc-reference` r2 provider).
const M002: &str = "277bf82dfca620784a2317b6932c4c4e99fbe5fa";

/// Every file owned by an assembled Module with the blob of the accepted candidate that owns it.
///
/// A new accepted candidate changes a blob; update the pin only together with a fresh integration.
const OWNED: [Pin; 10] = [
    (
        M001,
        "src/knowledge.rs",
        "8df996702c2f3471683e543c53111d044bf9d7b3",
    ),
    (
        M001,
        "src/tools/knowledge_ops.rs",
        "44f13ad4d062e7ed95abe397fabe87f806421d85",
    ),
    (
        M001,
        "docs/contracts/knowledge-records.md",
        "d8361815d201eb759dae88e8f231f6f6907bdfc7",
    ),
    (
        M002,
        "src/markdown.rs",
        "34dc69a8fceb8c0a27fc8dac1557ad5eef636609",
    ),
    (
        M002,
        "src/documents.rs",
        "6ce2157a113e56ec361323b13adbfcd3cee737f8",
    ),
    (
        M002,
        "src/documents/tests.rs",
        "cd7f340d59e45fc659a1889c9771db77c57359dc",
    ),
    (
        M002,
        "src/references.rs",
        "b9682172d6d3c2444a0ad13fa39ef55ab014b9c5",
    ),
    (
        M002,
        "src/tools/document_ops.rs",
        "424639791423d2200cde272ea08388793db16961",
    ),
    (
        M002,
        "src/tools/projects.rs",
        "d015ad27dbf878666bc3a6ff837244ec8ee558a5",
    ),
    (
        M002,
        "docs/contracts/markdown-references.md",
        "3cc8924f962a76d235af4658ccfd1d48d1a60dc4",
    ),
];

/// Pinned contract artifacts: commit, path and sha256 exactly as the tracker pins them for the two boundaries.
const ARTIFACTS: [(&str, &str, &str); 2] = [
    (
        "ea3a17fd4169f4c254ecd1f082a85c5f51d801da",
        "docs/contracts/markdown-references.md",
        "e9808e8336df412a3219c8858435977e0ef966c19db6b7479609119b2f99f94c",
    ),
    (
        "0f9d3734e3b13d1ec9a35eaccddc90007c44a9ca",
        "docs/contracts/knowledge-records.md",
        "34d9d81bf7a7f5b018dcd34fc1a6db2963a2152d22f40e66f4f6ff7b5688cfde",
    ),
];

/// The repository that holds the assembled checkout under test.
fn checkout() -> &'static Path {
    Path::new(env!("CARGO_MANIFEST_DIR"))
}

/// A1: the assembled checkout holds the exact accepted provider blobs and the exact pinned contract artifacts.
///
/// The pins always apply to the working files. When the accepted candidate or artifact objects exist in the local
/// object database they must agree byte for byte as well; an absent object is not a failure, a differing one is.
#[test]
fn a1_owned_provider_bytes_are_the_accepted_git_blobs() {
    for (candidate, path, blob) in OWNED {
        let worktree = git(checkout(), &["hash-object", "--no-filters", "--", path]);
        assert_eq!(
            worktree.trim(),
            blob,
            "{path} differs from the accepted blob"
        );
        let (present, _, _) = git_status(checkout(), &["cat-file", "-e", candidate]);
        if present {
            let accepted = git(checkout(), &["rev-parse", &format!("{candidate}:{path}")]);
            assert_eq!(
                accepted.trim(),
                blob,
                "{path} pin differs from candidate {candidate}"
            );
        }
    }
    for (commit, path, sha256) in ARTIFACTS {
        let (present, _, _) = git_status(checkout(), &["cat-file", "-e", commit]);
        if present {
            let bytes = git(checkout(), &["show", &format!("{commit}:{path}")]);
            assert_eq!(
                support::sha256_hex(bytes.as_bytes()),
                sha256,
                "artifact {path} at {commit}"
            );
        }
    }
}

/// Number of lines in a reply that start with `prefix`.
fn lines_with(text: &str, prefix: &str) -> usize {
    text.lines().filter(|l| l.starts_with(prefix)).count()
}

/// The `next_*` counters of the one allocator file, in file order, as `(name, value)` pairs.
fn counters(project: &Project) -> Vec<(String, u64)> {
    std::fs::read_to_string(project.root.join(".agent-tasks/knowledge.yaml"))
        .unwrap()
        .lines()
        .filter_map(|l| l.split_once(": "))
        .filter(|(k, _)| k.starts_with("next_"))
        .map(|(k, v)| (k.to_owned(), v.parse().unwrap()))
        .collect()
}

/// Create a Decision citing `detail` and return its generated id.
async fn decision(project: &Project, detail: Option<&str>) -> String {
    let mut args = json!({"op":"create_decision","title":"Joint","question":"Which option?","decision":"Option A","rationale":"Measured"});
    if let Some(detail) = detail {
        args["detail"] = json!(detail);
    }
    let text = knowledge(project, args, false).await;
    assert!(text.starts_with("SAVED D-"), "{text}");
    target(&text)
}

/// A2: one allocator file serves typed ids and DOC ids; each creation is one commit of exactly the owned paths.
#[tokio::test]
async fn a2_one_shared_allocator_orders_doc_and_typed_ids_without_recycling() {
    let project = Project::register().await;
    let root = &project.root;
    let base = commit_count(root);

    let saved = save(
        &project,
        "docs/design.md",
        "# Design\n\n## Section\ntext\n",
        false,
    )
    .await;
    assert_eq!(target(&saved), "DOC-001");
    assert_eq!(
        commit_paths(root, &head(root)),
        [
            ".agent-tasks/knowledge.yaml",
            "docs/design.md",
            "documents/DOC-001.yaml"
        ],
        "document creation commits the shared allocator, the body and the record once"
    );

    let d = decision(&project, Some("docs/design.md#section")).await;
    assert_eq!(d, "D-001");
    assert_eq!(
        commit_paths(root, &head(root)),
        [".agent-tasks/knowledge.yaml", "decisions/D-001.yaml"]
    );
    let rb = knowledge(
        &project,
        json!({"op":"create_runbook","title":"Restore","purpose":"Restore service","steps":[{"title":"Check","description":"Inspect","expected":"Healthy"}],"detail":"docs/design.md"}),
        false,
    )
    .await;
    assert_eq!(target(&rb), "RB-001");
    let rs = knowledge(
        &project,
        json!({"op":"create_research","title":"Findings","question":"Why?","conclusions":[{"statement":"Because","basis":"inferred"}],"applicability":"here","detail":"README.md"}),
        false,
    )
    .await;
    assert_eq!(target(&rs), "RS-001", "{rs}");
    let cl = knowledge(
        &project,
        json!({"op":"create_checklist","title":"Steps","purpose":"Do it","items":["one","two"]}),
        false,
    )
    .await;
    assert_eq!(target(&cl), "CL-001");
    assert_eq!(commit_count(root), base + 5, "five creations, five commits");

    // The counters live in one file, advance independently and a DOC id after typed ids is the next DOC number.
    let second = save(&project, "docs/second.md", "# Second\n", false).await;
    assert_eq!(target(&second), "DOC-002");
    assert_eq!(
        counters(&project),
        [
            ("next_decision".to_owned(), 2),
            ("next_runbook".to_owned(), 2),
            ("next_research".to_owned(), 2),
            ("next_checklist".to_owned(), 2),
            ("next_document".to_owned(), 3),
            ("next_compaction".to_owned(), 1),
        ]
    );

    // A retired DOC id is never recycled; a later save at the freed path takes the next number and leaves a visible gap.
    document(
        &project,
        json!({"op":"remove","ref":"docs/second.md"}),
        false,
    )
    .await;
    let reused = save(&project, "docs/second.md", "# Second again\n", false).await;
    assert_eq!(target(&reused), "DOC-003");
    let retired = project
        .call("get_context", json!({"ref":"DOC-002"}), false)
        .await;
    assert!(retired.contains("State: retired"), "{retired}");
    assert!(retired.contains("Identity: DOC-002"), "{retired}");
    let live = project
        .call("get_context", json!({"ref":"DOC-003"}), false)
        .await;
    assert!(live.contains("State: managed"), "{live}");
}

/// A3: the detail grammar is the provider's: a table of valid and invalid forms, each invalid one writing nothing.
#[tokio::test]
async fn a3_detail_grammar_accepts_exact_forms_and_refuses_the_rest_without_effect() {
    let project = Project::register().await;
    let root = &project.root;
    save(
        &project,
        "docs/design.md",
        "# Design\n\n## Section\ntext\n",
        false,
    )
    .await;
    save(
        &project,
        "docs/setext.md",
        "Title\n=====\n\n## Real\ntext\n",
        false,
    )
    .await;

    for (detail, message) in [
        (
            "docs/design.md#Section",
            "the document or section does not exist",
        ),
        (
            "docs/design.md#section-1",
            "the document or section does not exist",
        ),
        (
            "docs/setext.md#title",
            "the document or section does not exist",
        ),
        ("README.md#readme", "the document or section does not exist"),
        ("docs/missing.md", "the document or section does not exist"),
        ("docs/design.md#", "invalid fragment"),
        ("docs/design.md#a#b", "invalid fragment"),
        ("DOC-001", "use a managed document path"),
        (
            "./docs/design.md",
            "not a canonical identifier or managed path",
        ),
        (
            "docs/../README.md",
            "not a canonical identifier or managed path",
        ),
        (
            "/docs/design.md",
            "not a canonical identifier or managed path",
        ),
        ("docs/a b.md", "not a canonical identifier or managed path"),
        (
            "docs/design.MD",
            "not a canonical identifier or managed path",
        ),
        (
            "docs/a/b/c/d/e.md",
            "not a canonical identifier or managed path",
        ),
        (
            "docs/%64esign.md",
            "not a canonical identifier or managed path",
        ),
        (
            "https://example.invalid/x",
            "not a canonical identifier or managed path",
        ),
        ("M-0001", "not a canonical identifier or managed path"),
        ("D-001", "use a managed document path"),
    ] {
        let before = tree(root);
        let commits = commit_count(root);
        let text = knowledge(
            &project,
            json!({"op":"create_decision","title":"Bad","question":"q","decision":"d","rationale":"r","detail":detail}),
            true,
        )
        .await;
        assert!(
            text.starts_with("ERROR invalid_arguments: \"detail: "),
            "{detail}: {text}"
        );
        assert!(text.contains(message), "{detail}: {text}");
        assert!(
            text.contains("No business publication confirmed by this call."),
            "{text}"
        );
        assert_eq!(
            before,
            tree(root),
            "{detail}: a refused detail changed the root"
        );
        assert_eq!(
            commits,
            commit_count(root),
            "{detail}: a refused detail committed"
        );
    }

    // Accepted forms store the exact text and nothing else is reserved before the first success.
    for (n, detail) in [
        "README.md",
        "docs/design.md",
        "docs/design.md#section",
        "docs/setext.md#real",
    ]
    .into_iter()
    .enumerate()
    {
        let id = decision(&project, Some(detail)).await;
        assert_eq!(id, format!("D-{:03}", n + 1), "refusals reserved nothing");
        let ctx = project.call("get_context", json!({"ref":id}), false).await;
        assert!(
            ctx.contains(&format!("Detail: {}\n", json!(detail)))
                || ctx.ends_with(&format!("Detail: {}", json!(detail))),
            "{ctx}"
        );
    }

    // An edit validates the same way: a bad replacement is refused unchanged, a good one is stored.
    let before = tree(root);
    let edited = knowledge(
        &project,
        json!({"op":"edit_decision","ref":"D-001","detail":"docs/gone.md"}),
        true,
    )
    .await;
    assert!(edited.contains("detail:"), "{edited}");
    assert_eq!(before, tree(root));
    let ok = knowledge(
        &project,
        json!({"op":"edit_decision","ref":"D-001","detail":"docs/design.md#section"}),
        false,
    )
    .await;
    assert!(ok.starts_with("SAVED D-001"), "{ok}");
}

/// A4: typed details and Markdown references see each other: incoming rows, dangling attention, retired identity.
#[tokio::test]
async fn a4_typed_details_and_markdown_interact_through_incoming_dangling_and_retired_ids() {
    let project = Project::register().await;
    save(
        &project,
        "docs/design.md",
        "# Design\n\n## Section\ntext\n",
        false,
    )
    .await;
    let d1 = decision(&project, Some("docs/design.md#section")).await;
    let d2 = decision(&project, Some("docs/design.md")).await;
    save(
        &project,
        "docs/notes.md",
        &format!("# Notes\n\nSee {d1} and [design](design.md#section).\n"),
        false,
    )
    .await;

    // Incoming rows name each typed record and the Markdown source; the DOC id and the path answer identically.
    let by_path = project
        .call(
            "get_context",
            json!({"ref":"docs/design.md","view":"references"}),
            false,
        )
        .await;
    let by_id = project
        .call(
            "get_context",
            json!({"ref":"DOC-001","view":"references"}),
            false,
        )
        .await;
    for text in [&by_path, &by_id] {
        assert!(
            text.contains(&format!(
                "Knowledge \"{d1}\" via RecordToken x1 fragments \"section\""
            )),
            "{text}"
        );
        assert!(
            text.contains(&format!("Knowledge \"{d2}\" via RecordToken x1")),
            "{text}"
        );
        assert!(text.contains("docs/notes.md"), "{text}");
        assert!(text.contains("Data coverage: complete"), "{text}");
    }
    let typed = project
        .call("get_context", json!({"ref":d1,"view":"references"}), false)
        .await;
    assert!(
        typed.contains("Outgoing: \"docs/design.md#section\""),
        "{typed}"
    );
    assert!(
        typed.contains("Incoming:") && typed.contains("docs/notes.md"),
        "{typed}"
    );

    // A section replace that removes the heading names exactly the introduced dangling references.
    let replaced = document(
        &project,
        json!({"op":"save","ref":"docs/design.md","purpose":"Design","body":"# Design only\n"}),
        false,
    )
    .await;
    assert!(replaced.contains("Dangling: "), "{replaced}");
    assert!(
        replaced.contains(&format!("{d1} -> docs/design.md#section")),
        "{replaced}"
    );
    assert!(!replaced.contains("coverage incomplete"), "{replaced}");

    // Removal retires the DOC id: it stays resolvable forever while path details to it are refused.
    document(
        &project,
        json!({"op":"remove","ref":"docs/design.md"}),
        false,
    )
    .await;
    let retired = project
        .call("get_context", json!({"ref":"DOC-001"}), false)
        .await;
    assert!(retired.contains("State: retired"), "{retired}");
    let refused = knowledge(
        &project,
        json!({"op":"create_decision","title":"Late","question":"q","decision":"d","rationale":"r","detail":"docs/design.md"}),
        true,
    )
    .await;
    assert!(
        refused.contains("the document or section does not exist"),
        "{refused}"
    );
    let again = save(
        &project,
        "docs/design.md",
        "# Design\n\n## Section\nreborn\n",
        false,
    )
    .await;
    assert_eq!(
        target(&again),
        "DOC-003",
        "a reused path is a new identity after DOC-002 was taken"
    );
    let still = project
        .call("get_context", json!({"ref":"DOC-001"}), false)
        .await;
    assert!(still.contains("State: retired"), "{still}");
    let accepted = decision(&project, Some("docs/design.md#section")).await;
    assert_eq!(accepted, "D-003");
}

/// A5: unknown coverage is named, never zero: corrupt records, foreign names and unreadable documents are gaps.
#[tokio::test]
async fn a5_partial_and_unknown_coverage_is_named_and_never_counted_as_proof() {
    let project = Project::register().await;
    let root = &project.root;
    save(
        &project,
        "docs/design.md",
        "# Design\n\n## Section\ntext\n",
        false,
    )
    .await;
    let d1 = decision(&project, Some("docs/design.md#section")).await;

    // Each defect is a separate readable gap of the incoming scan; none is reported as complete.
    std::fs::write(root.join("decisions/D-099.yaml"), "not: [valid").unwrap();
    std::fs::create_dir_all(root.join("modules")).unwrap();
    std::fs::write(root.join("modules/M-099.yaml"), "not: [valid").unwrap();
    std::fs::create_dir_all(root.join("research")).unwrap();
    std::fs::write(root.join("research/stray.txt"), "x").unwrap();
    std::fs::write(root.join("docs/binary.md"), [0xff, 0xfe, 0x00]).unwrap();
    for view in ["docs/design.md", "DOC-001"] {
        let text = project
            .call(
                "get_context",
                json!({"ref":view,"view":"references"}),
                false,
            )
            .await;
        assert!(text.contains("Data coverage: PARTIAL"), "{text}");
        let gaps: usize = text
            .split("Incoming coverage PARTIAL: ")
            .nth(1)
            .and_then(|s| s.split(" gap(s)").next())
            .and_then(|n| n.parse().ok())
            .unwrap_or_else(|| panic!("no gap count: {text}"));
        assert!(gaps >= 3, "{gaps} gaps in: {text}");
        assert!(text.contains("lower bounds"), "{text}");
    }
    let ctx = project.call("get_context", json!({}), false).await;
    assert!(ctx.contains("Data coverage: PARTIAL"), "{ctx}");
    assert!(
        ctx.contains("Knowledge UNREADABLE") && ctx.contains("D-099"),
        "{ctx}"
    );

    // A destructive document change under unknown coverage still performs, but never claims zero dangling.
    let removed = document(
        &project,
        json!({"op":"remove","ref":"docs/design.md"}),
        false,
    )
    .await;
    assert!(removed.contains("(coverage incomplete)"), "{removed}");
    assert!(
        !removed.contains("0 left dangling."),
        "unknown coverage reported as zero: {removed}"
    );

    // The valid sibling record keeps resolving through the typed loader while the corrupt one stays named.
    let ctx = project.call("get_context", json!({"ref":d1}), false).await;
    assert!(
        ctx.contains(&format!("Detail: {}", json!("docs/design.md#section"))),
        "{ctx}"
    );
}

/// A6: allocator corruption and foreign names refuse both consumers by code with no effect, then recover without reuse.
#[tokio::test]
async fn a6_corrupt_allocator_and_foreign_homes_refuse_document_and_typed_creation_alike() {
    let project = Project::register().await;
    let root = &project.root;
    save(&project, "docs/design.md", "# Design\n", false).await;
    let allocator = root.join(".agent-tasks/knowledge.yaml");
    let good = std::fs::read(&allocator).unwrap();
    let new_doc = project.version("docs/new.md").await;

    for bad in [
        &b"garbage: ["[..],
        b"schema_version: 9\nnext_document: 2\n",
        b"schema_version: 1\nnext_decision: 1\nnext_runbook: 1\nnext_research: 1\nnext_checklist: 1\nnext_document: 2\nnext_compaction: 1\nextra: 1\n",
        b"schema_version: 1\nnext_decision: 1\nnext_runbook: 1\nnext_research: 1\nnext_checklist: 1\nnext_document: 1\nnext_compaction: 1\n",
    ] {
        std::fs::write(&allocator, bad).unwrap();
        let before = tree(root);
        let commits = commit_count(root);
        let doc = document(
            &project,
            json!({"op":"save","ref":"docs/new.md","purpose":"x","body":"# New\n","version":new_doc}),
            true,
        )
        .await;
        assert!(doc.starts_with("ERROR allocator: "), "{doc}");
        let observed = project.allocation_version().await;
        let typed = knowledge(
            &project,
            json!({"op":"create_decision","title":"T","question":"q","decision":"d","rationale":"r","version":observed}),
            true,
        )
        .await;
        assert!(typed.starts_with("ERROR allocator: "), "{typed}");
        assert!(doc.contains("never guess reserved IDs") && typed.contains("never guess reserved IDs"));
        assert_eq!(before, tree(root), "a refused reservation wrote something");
        assert_eq!(commits, commit_count(root));
    }
    std::fs::write(&allocator, &good).unwrap();

    // A foreign name in any counted home refuses both consumers: documents/ by the document reader, decisions/ by the allocator.
    for (home, expect_doc, expect_typed) in [
        ("documents", "ERROR partial_coverage: ", "ERROR inventory: "),
        ("decisions", "ERROR inventory: ", "ERROR inventory: "),
    ] {
        std::fs::create_dir_all(root.join(home)).unwrap();
        let stray = root.join(home).join("stray.txt");
        std::fs::write(&stray, "x").unwrap();
        let before = tree(root);
        let doc = document(
            &project,
            json!({"op":"save","ref":"docs/new.md","purpose":"x","body":"# New\n","version":new_doc}),
            true,
        )
        .await;
        assert!(doc.starts_with(expect_doc), "{home}: {doc}");
        let observed = project.allocation_version().await;
        let typed = knowledge(
            &project,
            json!({"op":"create_decision","title":"T","question":"q","decision":"d","rationale":"r","version":observed}),
            true,
        )
        .await;
        assert!(typed.starts_with(expect_typed), "{home}: {typed}");
        assert_eq!(before, tree(root), "{home}: refusal wrote something");
        std::fs::remove_file(&stray).unwrap();
    }

    // Restored state allocates the next numbers: nothing was reserved or recycled by any refusal.
    let created = save(&project, "docs/new.md", "# New\n", false).await;
    assert_eq!(target(&created), "DOC-002");
    assert_eq!(decision(&project, Some("docs/new.md")).await, "D-001");
}

/// A7: reads are writeless and exact; foreign Git state and native files survive owned mutations from both Modules.
#[tokio::test]
async fn a7_reads_write_nothing_and_foreign_state_is_preserved() {
    let mut project = Project::register().await;
    let root = project.root.clone();
    let body = "# Design\r\n\r\n## Section\r\ntext \u{1f980}\r\n";
    save(&project, "docs/design.md", body, false).await;
    let d1 = decision(&project, Some("docs/design.md#section")).await;
    knowledge(
        &project,
        json!({"op":"create_checklist","title":"Steps","purpose":"Do","items":["one"]}),
        false,
    )
    .await;

    // Every read route of both providers, plus search and status, leaves bytes, mtimes and Git untouched.
    let commits = commit_count(&root);
    support::assert_tree_unchanged(&root, "joint reads", || async {
        for args in [
            json!({}),
            json!({"ref":d1}),
            json!({"ref":d1,"view":"history"}),
            json!({"ref":d1,"view":"references"}),
            json!({"ref":"docs/design.md"}),
            json!({"ref":"DOC-001"}),
            json!({"ref":"DOC-001","view":"references"}),
            json!({"ref":"CL-001"}),
        ] {
            project.call("get_context", args, false).await;
        }
        project
            .call(
                "search",
                json!({"query":"design","kinds":["document","knowledge"]}),
                false,
            )
            .await;
        project.call("project_status", json!({}), false).await;
        let (bytes, _) = read_document(&project, "docs/design.md", json!({})).await;
        assert_eq!(bytes, body.as_bytes(), "exact CRLF and non-BMP bytes");
        let (section, _) =
            read_document(&project, "docs/design.md", json!({"heading":"Section"})).await;
        assert_eq!(section, "## Section\r\ntext \u{1f980}\r\n".as_bytes());
    })
    .await;
    assert_eq!(commits, commit_count(&root));

    // Foreign staged, dirty and untracked state of unrelated paths survives mutations from both Modules.
    std::fs::write(root.join("foreign-staged.txt"), "staged by a human\n").unwrap();
    git(&root, &["add", "--", "foreign-staged.txt"]);
    std::fs::write(root.join("foreign-untracked.txt"), "untracked\n").unwrap();
    std::fs::write(root.join("README.md"), "# Native README\n").unwrap();
    let mut owned_commits = Vec::new();
    save(&project, "docs/second.md", "# Second\n", false).await;
    owned_commits.push(head(&root));
    decision(&project, Some("docs/second.md")).await;
    owned_commits.push(head(&root));
    document(
        &project,
        json!({"op":"remove","ref":"docs/second.md"}),
        false,
    )
    .await;
    owned_commits.push(head(&root));
    for commit in &owned_commits {
        let paths = commit_paths(&root, commit);
        for foreign in ["foreign-staged.txt", "foreign-untracked.txt", "README.md"] {
            assert!(
                !paths.contains(&foreign.to_owned()),
                "{commit} swept {foreign}: {paths:?}"
            );
        }
    }
    assert_eq!(
        staged(&root),
        ["foreign-staged.txt"],
        "the foreign staged entry is untouched"
    );
    assert_eq!(
        std::fs::read(root.join("foreign-untracked.txt")).unwrap(),
        b"untracked\n"
    );
    assert_eq!(
        std::fs::read(root.join("README.md")).unwrap(),
        b"# Native README\n"
    );

    // A cold restart serves the same exact bytes and the same references.
    let before = project
        .call(
            "get_context",
            json!({"ref":"DOC-001","view":"references"}),
            false,
        )
        .await;
    project.restart().await;
    let after = project
        .call(
            "get_context",
            json!({"ref":"DOC-001","view":"references"}),
            false,
        )
        .await;
    assert_eq!(
        lines_with(&before, "Incoming: Knowledge"),
        lines_with(&after, "Incoming: Knowledge")
    );
    let (bytes, _) = read_document(&project, "docs/design.md", json!({})).await;
    assert_eq!(bytes, body.as_bytes());
}

/// A8: relocation keeps the DOC identity; details now resolve only at the new path and the old path is free again.
///
/// The relocation reply names the references it left dangling (the stored Decision detail and the Markdown link by
/// path) and never the DOC-id token, which keeps resolving through the unchanged identity.
#[tokio::test]
async fn a8_relocation_keeps_the_identity_and_moves_which_details_validate() {
    let project = Project::register().await;
    let root = &project.root;
    save(
        &project,
        "docs/design.md",
        "# Design\n\n## Section\ntext\n",
        false,
    )
    .await;
    let d1 = decision(&project, Some("docs/design.md#section")).await;
    save(
        &project,
        "docs/notes.md",
        "# Notes\n\nSee DOC-001 and [design](design.md#section).\n",
        false,
    )
    .await;
    let before = std::fs::read(root.join("docs/design.md")).unwrap();
    let version = project.version("docs/design.md").await;
    let to_version = project.version("docs/moved.md").await;
    let moved = project
        .call(
            "document_work",
            json!({"op":"relocate","ref":"docs/design.md","to":"docs/moved.md","to_version":to_version,"version":version,"actor":"qualification-agent"}),
            false,
        )
        .await;
    assert_eq!(target(&moved), "DOC-001", "{moved}");
    assert!(moved.contains("Dangling: "), "{moved}");
    assert!(
        moved.contains(&format!("{d1} -> docs/design.md#section")),
        "{moved}"
    );
    assert!(
        moved.contains("docs/notes.md -> docs/design.md#section"),
        "{moved}"
    );
    assert!(!moved.contains("-> DOC-001"), "{moved}");
    assert!(!moved.contains("coverage incomplete"), "{moved}");
    assert_eq!(std::fs::read(root.join("docs/moved.md")).unwrap(), before);
    assert!(!root.join("docs/design.md").exists());
    let mut paths: Vec<String> = git(
        root,
        &["show", "--name-only", "--no-renames", "--format=", "HEAD"],
    )
    .lines()
    .map(str::to_owned)
    .collect();
    paths.sort();
    assert_eq!(
        paths,
        ["docs/design.md", "docs/moved.md", "documents/DOC-001.yaml"],
        "one commit moves the body and rewrites the record"
    );
    let by_id = project
        .call("get_context", json!({"ref":"DOC-001"}), false)
        .await;
    assert!(
        by_id.contains("State: managed") && by_id.contains("docs/moved.md"),
        "{by_id}"
    );
    let old = knowledge(
        &project,
        json!({"op":"create_decision","title":"Old","question":"q","decision":"d","rationale":"r","detail":"docs/design.md#section"}),
        true,
    )
    .await;
    assert!(
        old.contains("the document or section does not exist"),
        "{old}"
    );
    assert_eq!(
        decision(&project, Some("docs/moved.md#section")).await,
        "D-002"
    );
    let incoming = project
        .call(
            "get_context",
            json!({"ref":"DOC-001","view":"references"}),
            false,
        )
        .await;
    assert!(
        incoming.contains("Knowledge \"D-002\" via RecordToken x1 fragments \"section\""),
        "{incoming}"
    );
}
