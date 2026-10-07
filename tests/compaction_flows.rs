//! Real-process qualification of reviewed compaction through `compaction_work` (M-006 criterion 5).
//!
//! Matrix rows C1 to C14 of docs/contracts/knowledge-qualification.md against `compaction-operations` r5 and
//! `cp-inventory` r1. A trusted agent proposes, an independent reviewer accepts, and apply runs under the production
//! Git policy. Interruptions use an ordinary read-only `documents/` parent that fails a later step after an earlier one
//! published, then restore the permission and repeat the same call; a crash inside a handler stays with the provider's
//! in-crate fault points. Payload shapes follow the closed record model of compaction section 4 (external enum tags);
//! `proposal` is the one place to adjust when the exported schema differs. Every test is `#[ignore]`d until the
//! combined candidate carries `compaction_work`, and none is claimed as run.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "Explicit protocol assertions"
)]
mod support;

use serde_json::{Value, json};
use support::{
    Project, commit_count, commit_paths, git, head, pending, save, set_writable, sha256_hex,
    target, tree,
};

/// One ATX section of a simple fixture document: its address JSON and exact bytes.
struct Section {
    /// The `SectionAddr` JSON of compaction section 4 (`"Preamble"` or an external `Heading` object).
    addr: Value,
    /// Exact section bytes (a heading line through the byte before the next heading of level at most its own).
    bytes: Vec<u8>,
}

/// Split a heading-only fixture document (no fences, levels 1 and 2) into its sections in document order.
fn sections(body: &str) -> Vec<Section> {
    let mut starts: Vec<(usize, u8, String)> = Vec::new();
    let mut offset = 0;
    for line in body.split_inclusive('\n') {
        let level = line.chars().take_while(|c| *c == '#').count();
        if (1..=6).contains(&level) && line[level..].starts_with(' ') {
            starts.push((offset, level as u8, line[level + 1..].trim_end().to_owned()));
        }
        offset += line.len();
    }
    let mut out = Vec::new();
    if starts.first().is_none_or(|first| first.0 > 0) {
        let end = starts.first().map_or(body.len(), |first| first.0);
        out.push(Section {
            addr: json!("Preamble"),
            bytes: body.as_bytes()[..end].to_vec(),
        });
    }
    for (index, (start, level, text)) in starts.iter().enumerate() {
        let end = starts[index + 1..]
            .iter()
            .find(|next| next.1 <= *level)
            .map_or(body.len(), |next| next.0);
        let occurrence = starts[..=index].iter().filter(|s| &s.2 == text).count();
        out.push(Section {
            addr: json!({"Heading":{"ordinal":index,"level":level,"occurrence":occurrence,"text_sha256":sha256_hex(text.as_bytes())}}),
            bytes: body.as_bytes()[*start..end].to_vec(),
        });
    }
    out
}

/// Ledger entries for one source document, every section carrying the given disposition.
fn ledger(path: &str, body: &str, disposition: Value) -> Vec<Value> {
    sections(body)
        .into_iter()
        .map(|s| json!({"path":path,"section":s.addr,"sha256":sha256_hex(&s.bytes),"disposition":disposition}))
        .collect()
}

/// Build a propose payload (version and actor are added by the caller).
fn proposal(key: &str, sources: Vec<Value>, actions: Vec<Value>, sections: Vec<Value>) -> Value {
    json!({"op":"propose","request_key":key,"title":"Qualification compaction","sources":sources,"actions":actions,"sections":sections,"preservation":[]})
}

/// Run one `compaction_work` operation as `actor` with the right precondition; returns the reply text.
async fn compaction(project: &Project, mut args: Value, actor: &str, error: bool) -> String {
    if args.get("version").is_none() {
        let version = if args["op"] == "propose" {
            project.allocation_version().await
        } else {
            project.version(args["cp"].as_str().unwrap()).await
        };
        args["version"] = json!(version);
    }
    args["actor"] = json!(actor);
    project.call("compaction_work", args, error).await
}

/// Save the three fixture documents and return their `(path, observation version)` source rows.
async fn fixture_docs(project: &Project) -> Vec<(String, String)> {
    let docs = [
        ("docs/a.md", "# A\nalpha\n"),
        ("docs/b.md", "# B\nbeta\n"),
        ("docs/index.md", "# Index\nSee [a](a.md) and [b](b.md).\n"),
    ];
    let mut rows = Vec::new();
    for (path, body) in docs {
        save(project, path, body, false).await;
        rows.push((path.to_owned(), project.version(path).await));
    }
    rows
}

/// A merge proposal: create `docs/ab.md`, rewrite the index links, remove both sources.
fn merge_proposal(rows: &[(String, String)], key: &str) -> Value {
    let version = |path: &str| rows.iter().find(|r| r.0 == path).unwrap().1.clone();
    let sources: Vec<Value> = rows
        .iter()
        .map(|(path, version)| json!({"path":path,"version":version}))
        .collect();
    let actions = vec![
        json!({"id":"A-01","kind":"Create","path":"docs/ab.md","content":"# AB\nalpha\nbeta\n","purpose":"Merged","reason":"Merge a and b","absorbed_into":[]}),
        json!({"id":"A-02","kind":"Replace","path":"docs/index.md","base_version":version("docs/index.md"),"content":"# Index\nSee [a](ab.md) and [b](ab.md).\n","reason":"Rewrite links","absorbed_into":[]}),
        json!({"id":"A-03","kind":"Remove","path":"docs/a.md","base_version":version("docs/a.md"),"reason":"Merged into ab","absorbed_into":[{"path":"docs/ab.md","section":null}]}),
        json!({"id":"A-04","kind":"Remove","path":"docs/b.md","base_version":version("docs/b.md"),"reason":"Merged into ab","absorbed_into":[{"path":"docs/ab.md","section":null}]}),
    ];
    let merged = json!({"Merged":{"target":{"path":"docs/ab.md","section":null}}});
    let mut ledger_rows = ledger("docs/a.md", "# A\nalpha\n", merged.clone());
    ledger_rows.extend(ledger("docs/b.md", "# B\nbeta\n", merged.clone()));
    ledger_rows.extend(ledger(
        "docs/index.md",
        "# Index\nSee [a](a.md) and [b](b.md).\n",
        json!({"Merged":{"target":{"path":"docs/index.md","section":null}}}),
    ));
    proposal(key, sources, actions, ledger_rows)
}

/// Every action, section and preservation id a reviewer must name, read from the proposal's own task view.
async fn items(project: &Project, cp: &str) -> Vec<String> {
    let text = project
        .call("get_context", json!({"ref":cp,"view":"tasks"}), false)
        .await;
    let mut out: Vec<String> = Vec::new();
    for word in text.split(|c: char| !(c.is_ascii_alphanumeric() || c == '-')) {
        let ok = (word.len() == 4 && word.starts_with("A-"))
            || (word.len() == 5 && word.starts_with("S-"))
            || (word.len() == 4 && word.starts_with("P-"));
        if ok && !out.contains(&word.to_owned()) {
            out.push(word.to_owned());
        }
    }
    out
}

/// The accepted review payload for the current revision of a proposal.
async fn accept(project: &Project, cp: &str) -> Value {
    let summary = project.call("get_context", json!({"ref":cp}), false).await;
    json!({"op":"review","cp":cp,"revision":1,"content_hash":support::field(&summary,"Content hash: ").trim(),"verdict":"accepted","summary":"Independent structural review","verified_items":items(project,cp).await,"findings":[],"resolved_findings":[]})
}

/// C1 and C2: independent review, self-review and reviewer-authoring refusals, and a changed proposal clears acceptance.
#[tokio::test]
#[ignore = "needs the combined E-001 candidate carrying compaction_work"]
async fn c1_c2_review_independence_and_revision_history() {
    let project = Project::register().await;
    let rows = fixture_docs(&project).await;
    let proposed = compaction(
        &project,
        merge_proposal(&rows, "qual-merge-1"),
        "author-agent",
        false,
    )
    .await;
    let cp = target(&proposed);
    assert_eq!(cp, "CP-001");
    let review = accept(&project, &cp).await;
    let own = compaction(&project, review.clone(), "author-agent", true).await;
    assert!(own.contains("self_review"), "{own}");
    let mut missing = review.clone();
    missing["verified_items"] = json!([]);
    assert!(
        compaction(&project, missing, "reviewer-agent", true)
            .await
            .contains("review_incomplete")
    );
    compaction(&project, review, "reviewer-agent", false).await;
    let revise_by_reviewer = compaction(&project, json!({"op":"revise","cp":cp,"title":"Changed","sources":[],"actions":[],"sections":[],"preservation":[]}), "reviewer-agent", true).await;
    assert!(
        revise_by_reviewer.contains("reviewer_cannot_author"),
        "{revise_by_reviewer}"
    );
    let mut changed = merge_proposal(&rows, "unused");
    changed["op"] = json!("revise");
    changed["cp"] = json!(cp);
    changed["title"] = json!("Changed title changes the hash");
    changed.as_object_mut().unwrap().remove("request_key");
    compaction(&project, changed, "author-agent", false).await;
    let after = project
        .call("get_context", json!({"ref":cp,"view":"review"}), false)
        .await;
    assert!(
        after.contains("accepted") && after.contains("revision"),
        "the earlier review stays as history: {after}"
    );
    let history = project
        .call("get_context", json!({"ref":cp,"view":"tasks"}), false)
        .await;
    assert!(
        history.contains("Qualification compaction"),
        "revision 1 body stays retrievable: {history}"
    );
}

/// C3: only Managed or Unmanaged Markdown sources are accepted; typed records and unsupported states refuse.
#[tokio::test]
#[ignore = "needs the combined E-001 candidate carrying compaction_work"]
async fn c3_sources_and_typed_bodies_are_refused() {
    let project = Project::register().await;
    let rows = fixture_docs(&project).await;
    let id = target(&support::knowledge(&project, json!({"op":"create_decision","title":"T","question":"q","decision":"d","rationale":"r"}), false).await);
    let typed = json!({"path":format!("decisions/{id}.yaml"),"version":project.version(&id).await});
    for bad in [
        typed,
        json!({"path":"docs/missing.md","version":"0".repeat(64)}),
        json!({"path":"docs/a.md","version":"0".repeat(64)}),
    ] {
        let mut payload = merge_proposal(&rows, "qual-bad");
        payload["sources"] = json!([bad]);
        let text = compaction(&project, payload, "author-agent", true).await;
        assert!(
            text.contains("source_refused") || text.contains("stale_source"),
            "{text}"
        );
    }
    std::fs::write(project.root.join("docs/a.md"), "# A\nnative drift\n").unwrap();
    let mut payload = merge_proposal(&rows, "qual-drift");
    payload["sources"] = json!([{"path":"docs/a.md","version":project.version("docs/a.md").await}]);
    assert!(
        compaction(&project, payload, "author-agent", true)
            .await
            .contains("source_refused")
    );
}

/// C9: foreign names under `compactions/` make the inventory incomplete and refuse every record creation by name.
#[tokio::test]
#[ignore = "needs the combined E-001 candidate carrying compaction_work"]
async fn c9_foreign_nested_names_block_creation_by_name() {
    let project = Project::register().await;
    let rows = fixture_docs(&project).await;
    compaction(
        &project,
        merge_proposal(&rows, "qual-inventory"),
        "author-agent",
        false,
    )
    .await;
    for foreign in [
        "compactions/CP-001/x.txt",
        "compactions/CP-001/r1/sub/deep.md",
        "compactions/CP-0001.yaml",
    ] {
        let path = project.root.join(foreign);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, "foreign").unwrap();
        let before = tree(&project.root);
        let refused = support::knowledge(&project, json!({"op":"create_decision","title":"Blocked","question":"q","decision":"d","rationale":"r"}), true).await;
        assert!(
            refused.contains("inventory") || refused.contains(foreign.rsplit('/').next().unwrap()),
            "{foreign}: {refused}"
        );
        assert_eq!(before, tree(&project.root));
        std::fs::remove_file(&path).unwrap();
        let _ = std::fs::remove_dir_all(project.root.join("compactions/CP-001/r1/sub"));
    }
}

/// C5, C6 and C10: an accepted proposal applies in order, originals stay recoverable from Git, and a plain clone is complete.
#[tokio::test]
#[ignore = "needs the combined E-001 candidate carrying compaction_work"]
async fn c5_c6_apply_order_originals_and_clone_completeness() {
    let project = Project::register().await;
    let rows = fixture_docs(&project).await;
    let originals = head(&project.root);
    let cp = target(
        &compaction(
            &project,
            merge_proposal(&rows, "qual-apply"),
            "author-agent",
            false,
        )
        .await,
    );
    compaction(
        &project,
        accept(&project, &cp).await,
        "reviewer-agent",
        false,
    )
    .await;
    let version = project.version(&cp).await;
    let applied = compaction(
        &project,
        json!({"op":"apply","cp":cp,"version":version}),
        "author-agent",
        false,
    )
    .await;
    assert!(applied.contains("applied"), "{applied}");
    assert!(
        project.root.join("docs/ab.md").is_file()
            && !project.root.join("docs/a.md").exists()
            && !project.root.join("docs/b.md").exists()
    );
    assert_eq!(
        git(&project.root, &["show", &format!("{originals}:docs/a.md")]),
        "# A\nalpha\n",
        "the original is recoverable from its commit"
    );
    let clone = project.temp.path().join("clone");
    git(
        project.temp.path(),
        &[
            "clone",
            "-q",
            project.root.to_str().unwrap(),
            clone.to_str().unwrap(),
        ],
    );
    assert!(clone.join("docs/ab.md").is_file() && !clone.join("docs/a.md").exists());
    assert_eq!(
        std::fs::read_to_string(clone.join("docs/index.md")).unwrap(),
        "# Index\nSee [a](ab.md) and [b](ab.md).\n"
    );
    let again = compaction(
        &project,
        json!({"op":"apply","cp":cp,"version":project.version(&cp).await}),
        "author-agent",
        false,
    )
    .await;
    let commits = commit_count(&project.root);
    assert!(again.contains("UNCHANGED"), "{again}");
    assert_eq!(
        commits,
        commit_count(&project.root),
        "a repeat apply publishes nothing"
    );
}

/// C14: an interrupted first apply holds the replacement; the resume blocks before any removal until the exact Retry.
#[tokio::test]
#[ignore = "needs the combined E-001 candidate carrying compaction_work and git_recovery"]
async fn c14_held_replacement_blocks_removal_until_retry() {
    let project = Project::register().await;
    let rows = fixture_docs(&project).await;
    let cp = target(
        &compaction(
            &project,
            merge_proposal(&rows, "qual-gate"),
            "author-agent",
            false,
        )
        .await,
    );
    compaction(
        &project,
        accept(&project, &cp).await,
        "reviewer-agent",
        false,
    )
    .await;
    std::fs::create_dir_all(project.root.join("documents")).ok();
    set_writable(&project.root.join("documents"), false);
    let failed = compaction(
        &project,
        json!({"op":"apply","cp":cp,"version":project.version(&cp).await}),
        "author-agent",
        true,
    )
    .await;
    set_writable(&project.root.join("documents"), true);
    assert!(!failed.is_empty());
    let commits = commit_count(&project.root);
    let blocked = compaction(
        &project,
        json!({"op":"apply","cp":cp,"version":project.version(&cp).await}),
        "author-agent",
        true,
    )
    .await;
    assert!(blocked.contains("replacements_not_committed"), "{blocked}");
    assert!(
        project.root.join("docs/a.md").exists(),
        "no removal while a replacement is held"
    );
    assert_eq!(
        commits,
        commit_count(&project.root),
        "a deletion-only commit never happens"
    );
    let held = pending(&project).await;
    project.call("git_recovery", json!({"op":"retry","intents":held.intents,"version":held.version,"actor":"qualification-agent"}), false).await;
    let done = compaction(
        &project,
        json!({"op":"apply","cp":cp,"version":project.version(&cp).await}),
        "author-agent",
        false,
    )
    .await;
    assert!(done.contains("applied"), "{done}");
    let clone = project.temp.path().join("clone");
    git(
        project.temp.path(),
        &[
            "clone",
            "-q",
            project.root.to_str().unwrap(),
            clone.to_str().unwrap(),
        ],
    );
    assert!(
        clone.join("docs/ab.md").is_file() && !clone.join("docs/a.md").exists(),
        "a clone never sees a removal without its replacement"
    );
}

/// C12: a managed Move interrupted after the destination body resumes only after Retry, keeping the source DOC id.
#[tokio::test]
#[ignore = "needs the combined E-001 candidate carrying compaction_work and git_recovery"]
async fn c12_move_source_removal_gate_keeps_identity() {
    let project = Project::register().await;
    save(&project, "docs/from.md", "# Move me\nbody\n", false).await;
    let managed = project
        .call("get_context", json!({"ref":"docs/from.md"}), false)
        .await;
    let id = managed
        .split_whitespace()
        .find(|w| w.starts_with("DOC-"))
        .map(|w| {
            w.trim_matches(|c: char| !c.is_ascii_alphanumeric() && c != '-')
                .to_owned()
        })
        .unwrap();
    let version = project.version("docs/from.md").await;
    let to_version = project.version("docs/to.md").await;
    let move_action = json!({"id":"A-01","kind":"Move","path":"docs/to.md","from":"docs/from.md","base_version":to_version,"reason":"Rename","absorbed_into":[]});
    let moved = json!({"Moved":{"action":"A-01"}});
    let payload = proposal(
        "qual-move",
        vec![json!({"path":"docs/from.md","version":version})],
        vec![move_action],
        ledger("docs/from.md", "# Move me\nbody\n", moved),
    );
    let cp = target(&compaction(&project, payload, "author-agent", false).await);
    compaction(
        &project,
        accept(&project, &cp).await,
        "reviewer-agent",
        false,
    )
    .await;
    set_writable(&project.root.join("documents"), false);
    compaction(
        &project,
        json!({"op":"apply","cp":cp,"version":project.version(&cp).await}),
        "author-agent",
        true,
    )
    .await;
    set_writable(&project.root.join("documents"), true);
    let commits = commit_count(&project.root);
    let blocked = compaction(
        &project,
        json!({"op":"apply","cp":cp,"version":project.version(&cp).await}),
        "author-agent",
        true,
    )
    .await;
    assert!(blocked.contains("replacements_not_committed"), "{blocked}");
    assert!(
        project.root.join("docs/from.md").exists(),
        "the source is untouched while the old destination is held"
    );
    assert_eq!(commits, commit_count(&project.root));
    let held = pending(&project).await;
    project.call("git_recovery", json!({"op":"retry","intents":held.intents,"version":held.version,"actor":"qualification-agent"}), false).await;
    let done = compaction(
        &project,
        json!({"op":"apply","cp":cp,"version":project.version(&cp).await}),
        "author-agent",
        false,
    )
    .await;
    assert!(done.contains("applied"), "{done}");
    let destination = project
        .call("get_context", json!({"ref":"docs/to.md"}), false)
        .await;
    assert!(
        destination.contains(&id),
        "the source DOC id follows the destination: {destination}"
    );
    assert!(!project.root.join("docs/from.md").exists());
    let clone = project.temp.path().join("clone");
    git(
        project.temp.path(),
        &[
            "clone",
            "-q",
            project.root.to_str().unwrap(),
            clone.to_str().unwrap(),
        ],
    );
    assert!(clone.join("docs/to.md").is_file() && !clone.join("docs/from.md").exists());
    let paths = commit_paths(&project.root, &head(&project.root));
    assert!(!paths.is_empty());
}
