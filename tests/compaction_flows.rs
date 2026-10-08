//! Real-process qualification of reviewed compaction through `compaction_work` (M-006 criterion 5).
//!
//! Matrix rows C1 to C14 of docs/contracts/knowledge-qualification.md against `compaction-operations` r6 and
//! `cp-inventory` r1. A trusted agent proposes, an independent reviewer accepts, and apply runs under the production
//! Git policy. Interruptions use an ordinary read-only `documents/` parent that fails a later step after an earlier one
//! published, then restore the permission and repeat the same call; a crash inside a handler stays with the provider's
//! in-crate fault points. Payload shapes follow the exported closed wire of `compaction_work`: section addresses are
//! internally tagged by `at` (`preamble`, `heading`), dispositions by `fate` (`kept`, `moved`, `merged`, `dropped`),
//! and the ledger carries no digest because the MCP computes section hashes itself. `proposal` is the one place to
//! adjust when the exported schema differs.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "Explicit protocol assertions"
)]
mod support;

use serde_json::{Value, json};
use support::{
    Project, commit_count, commit_paths, field, git, head, pending, read_document, save,
    set_writable, sha256_hex, snapshot_version, target, tree,
};

/// One ATX section of a simple fixture document: its wire address JSON.
struct Section {
    /// The `SectionAddr` wire JSON: `{"at":"preamble"}` or `{"at":"heading",ordinal,level,occurrence,text_sha256}`.
    addr: Value,
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
        out.push(Section {
            addr: json!({"at":"preamble"}),
        });
    }
    for (index, (_, level, text)) in starts.iter().enumerate() {
        let occurrence = starts[..=index].iter().filter(|s| &s.2 == text).count();
        out.push(Section {
            addr: json!({"at":"heading","ordinal":index,"level":level,"occurrence":occurrence,"text_sha256":sha256_hex(text.as_bytes())}),
        });
    }
    out
}

/// Ledger entries for one source document, every section carrying the given disposition.
fn ledger(path: &str, body: &str, disposition: Value) -> Vec<Value> {
    sections(body)
        .into_iter()
        .map(|s| json!({"path":path,"section":s.addr,"disposition":disposition}))
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
        json!({"id":"A-01","kind":"create","path":"docs/ab.md","content":"# AB\nalpha\nbeta\n","purpose":"Merged","reason":"Merge a and b","absorbed_into":[]}),
        json!({"id":"A-02","kind":"replace","path":"docs/index.md","base_version":version("docs/index.md"),"content":"# Index\nSee [a](ab.md) and [b](ab.md).\n","reason":"Rewrite links","absorbed_into":[]}),
        json!({"id":"A-03","kind":"remove","path":"docs/a.md","base_version":version("docs/a.md"),"reason":"Merged into ab","absorbed_into":[{"path":"docs/ab.md","section":null}]}),
        json!({"id":"A-04","kind":"remove","path":"docs/b.md","base_version":version("docs/b.md"),"reason":"Merged into ab","absorbed_into":[{"path":"docs/ab.md","section":null}]}),
    ];
    let merged = json!({"fate":"merged","target":{"path":"docs/ab.md","section":null}});
    let mut ledger_rows = ledger("docs/a.md", "# A\nalpha\n", merged.clone());
    ledger_rows.extend(ledger("docs/b.md", "# B\nbeta\n", merged.clone()));
    ledger_rows.extend(ledger(
        "docs/index.md",
        "# Index\nSee [a](a.md) and [b](b.md).\n",
        json!({"fate":"merged","target":{"path":"docs/index.md","section":null}}),
    ));
    proposal(key, sources, actions, ledger_rows)
}

/// Every action, section and preservation id a reviewer must name, read from all task-view pages.
async fn items(project: &Project, cp: &str) -> Vec<String> {
    let (text, _) = paged_cp_view(project, cp, "tasks", None).await;
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

/// Read every page of a CP tasks view or one retained revision's history view.
///
/// `view` is `tasks` or `history`; `revision` is absent for tasks and selects the retained revision for history.
/// The read is side-effect free and every page remains subject to [`Project::call`]'s reply budget. Returns all reply
/// text and the number of pages. Snapshot changes, malformed continuation tokens or non-progress panic.
/// A generous 128-page safety bound permits lossless field rows constrained by the reply byte budget.
async fn paged_cp_view(
    project: &Project,
    cp: &str,
    view: &str,
    revision: Option<u32>,
) -> (String, usize) {
    let mut text = String::new();
    let mut start = 0;
    let mut version = None;
    let mut previous_remaining = None;
    let mut pages = 0;
    loop {
        let mut args = json!({"ref":cp,"view":view,"start":start});
        if let Some(revision) = revision {
            args["revision"] = json!(revision);
        }
        if let Some(snapshot) = &version {
            args["version"] = json!(snapshot);
        }
        let page = project.call("get_context", args, false).await;
        let snapshot = snapshot_version(&page);
        if let Some(expected) = &version {
            assert_eq!(&snapshot, expected, "CP pages share one snapshot");
        }
        text.push_str(&page);
        text.push('\n');
        pages += 1;
        assert!(
            pages <= 128,
            "the bounded CP fixture exceeds its pagination safety limit"
        );
        if !page.lines().any(|line| line.starts_with("Next: start=")) {
            return (text, pages);
        }
        let next = field(&page, "Next: start=");
        let (next_start, next) = next.split_once("; version=").unwrap();
        let (next_version, remaining) = next.split_once("; remaining=").unwrap();
        let next_start: usize = next_start.parse().unwrap();
        let remaining: usize = remaining
            .trim_start()
            .split(|c: char| !c.is_ascii_digit())
            .next()
            .unwrap()
            .parse()
            .unwrap();
        assert!(next_start > start, "CP pagination makes strict progress");
        assert_eq!(
            next_version, snapshot,
            "Next carries the page snapshot version"
        );
        assert!(remaining > 0, "a continuation has remaining rows");
        if let Some(previous) = previous_remaining {
            assert!(remaining < previous, "remaining row count decreases");
        }
        start = next_start;
        version = Some(next_version.to_owned());
        previous_remaining = Some(remaining);
    }
}

/// The current revision content hash printed inline as `content hash <64 hex>` by a proposal summary.
fn content_hash(summary: &str) -> String {
    let at = summary
        .find("content hash ")
        .unwrap_or_else(|| panic!("no content hash in: {summary}"));
    summary[at + "content hash ".len()..]
        .chars()
        .take_while(char::is_ascii_hexdigit)
        .collect()
}

/// The accepted review payload for the current revision of a proposal.
async fn accept(project: &Project, cp: &str) -> Value {
    let summary = project.call("get_context", json!({"ref":cp}), false).await;
    json!({"op":"review","cp":cp,"revision":1,"content_hash":content_hash(&summary),"verdict":"accepted","summary":"Independent structural review","verified_items":items(project,cp).await,"findings":[],"resolved_findings":[]})
}

/// C1, C2 and C7: review independence, acceptance invalidation, and paginated retrieval of the full prior revision.
#[tokio::test]
async fn c1_c2_review_independence_and_revision_history() {
    let project = Project::register().await;
    let rows = fixture_docs(&project).await;
    let revision_one_candidate = format!("# AB\n{}\n", "alpha\n".repeat(1500));
    let mut first = merge_proposal(&rows, "qual-merge-1");
    first["actions"][0]["content"] = json!(revision_one_candidate);
    first["actions"].as_array_mut().unwrap().extend((5..=21).map(|n| json!({"id":format!("A-{n:02}"),"kind":"create","path":format!("docs/history-{n:02}.md"),"content":format!("# History {n}\n"),"purpose":"History row","reason":format!("History row {n}"),"absorbed_into":[]})));
    let proposed = compaction(&project, first, "author-agent", false).await;
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
    let revision_two_candidate = format!("# AB revision two\n{}\n", "beta\n".repeat(1500));
    changed["actions"][0]["content"] = json!(revision_two_candidate);
    changed.as_object_mut().unwrap().remove("request_key");
    compaction(&project, changed, "author-agent", false).await;
    let after = project
        .call("get_context", json!({"ref":cp,"view":"review"}), false)
        .await;
    assert!(
        after.contains("accepted") && after.contains("revision"),
        "the earlier review stays as history: {after}"
    );
    let before_reads = tree(&project.root);
    let (history, history_pages) = paged_cp_view(&project, &cp, "history", Some(1)).await;
    assert!(
        history_pages > 1,
        "the fixture crosses the 20-row history page size"
    );
    for retained in [
        "Qualification compaction",
        "Merge a and b",
        "docs/a.md",
        "docs/b.md",
        "docs/index.md",
        "Independent structural review",
        "reviewer-agent",
        "accepted",
    ] {
        assert!(
            history.contains(retained),
            "revision 1 body and review remain retrievable ({retained}): {history}"
        );
    }
    for action in 1..=21 {
        assert!(
            history.contains(&format!("A-{action:02}")),
            "history retains A-{action:02}: {history}"
        );
    }
    let (candidate, content_pages) =
        read_document(&project, &cp, json!({"revision":1,"action":"A-01"})).await;
    assert!(
        content_pages > 1,
        "the retained candidate crosses the 8192-byte page budget"
    );
    assert_eq!(candidate, revision_one_candidate.as_bytes());
    let revision_one =
        std::fs::read(project.root.join(format!("compactions/{cp}/r1/A-01.md"))).unwrap();
    let revision_two =
        std::fs::read(project.root.join(format!("compactions/{cp}/r2/A-01.md"))).unwrap();
    assert_eq!(revision_one, revision_one_candidate.as_bytes());
    assert_eq!(revision_two, revision_two_candidate.as_bytes());
    assert_ne!(
        candidate, revision_two,
        "revision 1 reads cannot substitute the current revision's candidate"
    );
    let record =
        std::fs::read_to_string(project.root.join(format!("compactions/{cp}.yaml"))).unwrap();
    assert!(record.contains("Qualification compaction") && record.contains("Changed title"));
    assert_eq!(
        tree(&project.root),
        before_reads,
        "revision reads leave the proposal tree unchanged"
    );
}

/// C6 (r6): a Replace whose candidate equals the current bytes is refused at propose with no record and no effect.
#[tokio::test]
async fn c6_identical_replace_is_refused_at_propose() {
    let project = Project::register().await;
    let rows = fixture_docs(&project).await;
    let mut payload = merge_proposal(&rows, "qual-identical");
    payload["actions"][1]["content"] = json!("# Index\nSee [a](a.md) and [b](b.md).\n");
    let before = tree(&project.root);
    let commits = commit_count(&project.root);
    let refused = compaction(&project, payload, "author-agent", true).await;
    assert!(
        refused.contains("actions.content") && refused.contains("equals the current document"),
        "{refused}"
    );
    assert_eq!(
        before,
        tree(&project.root),
        "a refused propose publishes nothing"
    );
    assert_eq!(commits, commit_count(&project.root));
}

/// C3: only Managed or Unmanaged Markdown sources are accepted; typed records and unsupported states refuse.
#[tokio::test]
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
            text.contains("source_refused")
                || text.contains("stale_source")
                || text.contains("Not a managed document path"),
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
        let after = tree(&project.root);
        let changed: Vec<_> = before
            .keys()
            .chain(after.keys())
            .filter(|key| before.get(*key) != after.get(*key))
            .collect();
        assert!(
            changed.is_empty(),
            "{foreign}: a refused creation changed {changed:?}"
        );
        std::fs::remove_file(&path).unwrap();
        let _ = std::fs::remove_dir_all(project.root.join("compactions/CP-001/r1/sub"));
    }
}

/// C5, C6 and C10: an accepted proposal applies in order, originals stay recoverable from Git, and a plain clone is complete.
#[tokio::test]
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
    let move_action = json!({"id":"A-01","kind":"move","path":"docs/to.md","from":"docs/from.md","reason":"Rename","absorbed_into":[]});
    let moved = json!({"fate":"moved","action":"A-01"});
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
