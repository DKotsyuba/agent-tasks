//! One real-process composition scenario across work, typed knowledge, Markdown, compaction and Git recovery.
//!
//! The synthetic work receipts exercise tool composition only; they are not evidence of live agent execution or Epic
//! acceptance. The project, MCP server, SDK calls, automatic commits, restart and clone use the real shipped binary.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "Explicit protocol and persisted-state assertions"
)]
mod support;

use serde_json::{Value, json};
use support::{
    Project, commit_paths, git, head, knowledge, read_document, save, sha256_hex, staged, target,
    tree, work_cycle,
};

/// Return the one exact heading address used by the fixture's source and merged documents.
fn heading(ordinal: usize, text: &str, occurrence: usize) -> Value {
    json!({
        "at":"heading",
        "ordinal":ordinal,
        "level":2,
        "occurrence":occurrence,
        "text_sha256":sha256_hex(text.as_bytes())
    })
}

/// Build the existing compaction fixture shape: all original Markdown sections merge into one created document.
fn merged_sections(path: &str, target: &str, body: &str) -> Vec<Value> {
    let mut starts = Vec::new();
    for line in body.lines() {
        let level = line.chars().take_while(|c| *c == '#').count();
        if (1..=6).contains(&level) && line[level..].starts_with(' ') {
            starts.push((level, line[level + 1..].trim_end().to_owned()));
        }
    }
    starts
        .iter()
        .enumerate()
        .map(|(ordinal, (level, text))| json!({
            "path":path,
            "section":{
                "at":"heading",
                "ordinal":ordinal,
                "level":level,
                "occurrence":starts[..=ordinal].iter().filter(|(_, title)| title == text).count(),
                "text_sha256":sha256_hex(text.as_bytes())
            },
            "disposition":{"fate":"merged","target":{"path":target,"section":null}}
        }))
        .collect()
}

/// Run one compaction mutation using its current MCP-observed precondition and a declared synthetic actor.
async fn compact(project: &Project, mut args: Value, actor: &str) -> String {
    let version = if args["op"] == "propose" {
        project.allocation_version().await
    } else {
        project.version(args["cp"].as_str().unwrap()).await
    };
    args["version"] = json!(version);
    args["actor"] = json!(actor);
    project.call("compaction_work", args, false).await
}

/// Read the current proposal summary and its current tasks to construct the exact structural acceptance payload.
async fn accept_current(project: &Project, cp: &str) -> String {
    let summary = project.call("get_context", json!({"ref":cp}), false).await;
    let hash = summary
        .split_once("content hash ")
        .map(|(_, tail)| {
            tail.chars()
                .take_while(char::is_ascii_hexdigit)
                .collect::<String>()
        })
        .unwrap_or_else(|| panic!("current proposal summary has no content hash: {summary}"));
    assert_eq!(hash.len(), 64, "the current content hash is complete");
    let tasks = project
        .call("get_context", json!({"ref":cp,"view":"tasks"}), false)
        .await;
    let mut verified_items = Vec::new();
    for word in tasks.split(|c: char| !(c.is_ascii_alphanumeric() || c == '-')) {
        if (word.len() == 4 && (word.starts_with("A-") || word.starts_with("P-")))
            || (word.len() == 5 && word.starts_with("S-"))
        {
            let item = word.to_owned();
            if !verified_items.contains(&item) {
                verified_items.push(item);
            }
        }
    }
    // The fixture's three P-ids are synthetic reviewer metadata, not live acceptance evidence.
    for item in ["P-01", "P-02", "P-03"] {
        if !verified_items.iter().any(|seen| seen == item) {
            verified_items.push(item.to_owned());
        }
    }
    for item in ["A-01", "A-02", "A-03", "A-04", "S-001", "S-008"] {
        assert!(
            verified_items.iter().any(|seen| seen == item),
            "current task view includes {item}: {tasks}"
        );
    }
    let review = compact(
        project,
        json!({
            "op":"review",
            "cp":cp,
            "revision":1,
            "content_hash":hash,
            "verdict":"accepted",
            "summary":"Synthetic structural review; semantic preservation is asserted against exact content.",
            "verified_items":verified_items,
            "findings":[],
            "resolved_findings":[]
        }),
        "synthetic-independent-reviewer",
    )
    .await;
    assert!(
        review.contains("accepted"),
        "the independent review is recorded: {review}"
    );
    review
}

/// Exercise one real SDK business flow, including fresh bootstrap, recovery and current content in a Git clone.
#[tokio::test]
async fn full_composition_survives_compaction_restart_and_clone() {
    let mut project = Project::register().await;
    assert!(
        project.root.join("docs").is_dir(),
        "registration bootstraps a usable docs home"
    );
    assert!(
        project.root.join(".git").is_dir(),
        "registration initializes the local repository"
    );
    let initial_status = project.call("project_status", json!({}), false).await;
    assert!(
        initial_status.contains("Qualification product"),
        "registered project is usable: {initial_status}"
    );

    // work_cycle deliberately supplies synthetic fixture bindings and receipts for the existing core workflow.
    let work_base = head(&project.root);
    let work = work_cycle(&project).await;
    assert_eq!(work.len(), 11, "the entire existing core lifecycle ran");
    for step in &work {
        assert!(
            step.reply.starts_with("SAVED"),
            "{}: {}",
            step.name,
            step.reply
        );
        assert_eq!(
            step.commits, 1,
            "{} commits its successful owned mutation",
            step.name
        );
    }
    let work_status = project.call("project_status", json!({}), false).await;
    assert!(
        work_status.contains("G1 work cycle"),
        "the core Module is in project status: {work_status}"
    );
    let work_commits = git(
        &project.root,
        &["rev-list", "--reverse", &format!("{work_base}..HEAD")],
    );
    let work_paths: Vec<String> = work_commits
        .lines()
        .flat_map(|commit| commit_paths(&project.root, commit))
        .collect();
    assert!(
        !work_paths.is_empty()
            && work_paths
                .iter()
                .all(|path| path.starts_with("modules/") || path == ".agent-tasks/state.yaml"),
        "the core flow commits only its owned work records: {work_paths:?}"
    );
    assert!(
        staged(&project.root).is_empty(),
        "successful mutations leave no staged files"
    );

    let guide = format!(
        "# Field guide\n\n## Decisions\nD-001 records the reservation requirement and links this guide.\n\n## Operations\nRB-001 and CL-001 describe the reservation recovery checks.\n"
    );
    save(&project, "docs/guide.md", &guide, false).await;
    let decision = knowledge(
        &project,
        json!({
            "op":"create_decision",
            "title":"Preserve reservations",
            "question":"What must survive a recovery?",
            "decision":"Keep every active reservation exactly.",
            "rationale":"A 2% measured failure rate followed retention of reservation history.",
            "detail":"docs/guide.md#decisions"
        }),
        false,
    )
    .await;
    let decision_id = target(&decision);
    assert_eq!(decision_id, "D-001", "the fresh allocation is used");
    let runbook = knowledge(
        &project,
        json!({
            "op":"create_runbook",
            "title":"Recover reservations",
            "purpose":"Restore reservation service",
            "steps":[{"title":"Check reservation ledger","description":"Read the committed ledger","expected":"Every active reservation is present"}],
            "detail":"docs/guide.md#operations"
        }),
        false,
    )
    .await;
    let runbook_id = target(&runbook);
    assert_eq!(runbook_id, "RB-001", "the fresh Runbook allocation is used");
    let used = knowledge(
        &project,
        json!({
            "op":"use_runbook",
            "ref":runbook_id,
            "revision":1,
            "outcome":"succeeded",
            "environment":"disposable full-composition fixture",
            "checks":[{"label":"reservation ledger","status":"passed"}]
        }),
        false,
    )
    .await;
    assert!(
        used.starts_with("SAVED RB-001"),
        "the use is bound to revision 1: {used}"
    );
    let checklist = knowledge(
        &project,
        json!({
            "op":"create_checklist",
            "title":"Reservation release",
            "purpose":"Record exact release facts",
            "items":["Verify every reservation"]
        }),
        false,
    )
    .await;
    let checklist_id = target(&checklist);
    assert_eq!(
        checklist_id, "CL-001",
        "the fresh Checklist allocation is used"
    );
    let done = knowledge(
        &project,
        json!({"op":"resolve_item","ref":checklist_id,"item":"I-001","state":"done","text":"verified every reservation in fixture run"}),
        false,
    )
    .await;
    assert!(
        done.starts_with("SAVED CL-001"),
        "the checklist fact is saved: {done}"
    );
    let item = project
        .call("get_context", json!({"ref":"CL-001/I-001"}), false)
        .await;
    assert!(
        item.contains("verified every reservation in fixture run"),
        "the exact checklist completion fact is readable: {item}"
    );

    let before_reads = tree(&project.root);
    let linked = project
        .call(
            "get_context",
            json!({"ref":"docs/guide.md","view":"references"}),
            false,
        )
        .await;
    for reference in [&decision_id, &runbook_id, &checklist_id] {
        assert!(
            linked.contains(reference),
            "Markdown reference view links {reference}: {linked}"
        );
    }
    let search = project
        .call(
            "search",
            json!({"query":"reservation","kinds":["work","knowledge","document"]}),
            false,
        )
        .await;
    for reference in [&decision_id, &runbook_id, &checklist_id, "DOC-001"] {
        assert!(
            search.contains(reference),
            "cross-kind search returns {reference}: {search}"
        );
    }
    let work_search = project
        .call(
            "search",
            json!({"query":"fixture","kinds":["work","knowledge","document"]}),
            false,
        )
        .await;
    assert!(
        work_search.contains("M-001"),
        "cross-kind search finds the core work record: {work_search}"
    );
    assert_eq!(
        before_reads,
        tree(&project.root),
        "reference and search reads do not change project files"
    );

    let source_a = "# Requirements\n\n## Requirement\nKeep every active reservation exactly.\n\n## Rationale\nThe measured failure rate fell from 8% to 2% after preserving reservation history.\n";
    let source_b = format!(
        "# Operations\n\n## Recovery\nRestore every reservation from the committed ledger.\n\n## Rationale\nRollback evidence prevents repeating reservation loss.\n\n## Evidence\n{}\n",
        "reservation-evidence ".repeat(520)
    );
    let index = "# Index\n\nRead [requirements](a.md) and [operations](b.md).\n";
    for (path, body) in [
        ("docs/a.md", source_a.to_owned()),
        ("docs/b.md", source_b.clone()),
        ("docs/index.md", index.to_owned()),
    ] {
        save(&project, path, &body, false).await;
    }
    // Source versions come from the exact owning MCP observations, never from test-side hashing.
    let mut source_rows = Vec::new();
    for path in ["docs/a.md", "docs/b.md", "docs/index.md"] {
        source_rows.push(json!({"path":path,"version":project.version(path).await}));
    }

    let added_checks = (0..32)
        .map(|n| format!("Retained operational checkpoint {n}: reconcile the reservation ledger before sign-off.\n"))
        .collect::<String>();
    let combined = format!(
        "# Combined\n\n## Requirement\nKeep every active reservation exactly.\n\n## Rationale\nThe measured failure rate fell from 8% to 2% after preserving reservation history.\n\n## Recovery\nRestore every reservation from the committed ledger.\n\n## Rationale\nRollback evidence prevents repeating reservation loss.\n\n## Evidence\n{}\n\n## Supplemental checks\n{}",
        "reservation-evidence ".repeat(520),
        added_checks
    );
    let index_after =
        "# Index\n\nRead [requirements](combined.md) and [operations](combined.md).\n";
    let all_sections = [
        merged_sections("docs/a.md", "docs/combined.md", source_a),
        merged_sections("docs/b.md", "docs/combined.md", &source_b),
        merged_sections("docs/index.md", "docs/index.md", index),
    ]
    .concat();
    let payload = json!({
        "op":"propose",
        "request_key":"full-composition-merge",
        "title":"Preserve reservation requirements and rationale",
        "sources":source_rows,
        "actions":[
            {"id":"A-01","kind":"create","path":"docs/combined.md","content":combined,"purpose":"Combined recovery guidance","reason":"Consolidate reservation requirements and evidence","absorbed_into":[]},
            {"id":"A-02","kind":"replace","path":"docs/index.md","base_version":project.version("docs/index.md").await,"content":index_after,"reason":"Rewrite both incoming links to the combined guide","absorbed_into":[]},
            {"id":"A-03","kind":"remove","path":"docs/a.md","base_version":project.version("docs/a.md").await,"reason":"Merged into combined guidance","absorbed_into":[{"path":"docs/combined.md","section":null}]},
            {"id":"A-04","kind":"remove","path":"docs/b.md","base_version":project.version("docs/b.md").await,"reason":"Merged into combined guidance","absorbed_into":[{"path":"docs/combined.md","section":null}]}
        ],
        "sections":all_sections,
        "preservation":[
            {"id":"P-01","kind":"requirement","path":"docs/a.md","section":heading(1,"Requirement",1),"statement":"Keep every active reservation exactly.","target":{"path":"docs/combined.md","section":heading(1,"Requirement",1)},"mode":"verbatim"},
            {"id":"P-02","kind":"rationale","path":"docs/a.md","section":heading(2,"Rationale",1),"statement":"Preserve the measured 8% to 2% failure-rate rationale.","target":{"path":"docs/combined.md","section":heading(2,"Rationale",1)},"mode":"verbatim"},
            {"id":"P-03","kind":"rationale","path":"docs/b.md","section":heading(2,"Rationale",1),"statement":"Keep rollback evidence for reservation loss.","target":{"path":"docs/combined.md","section":heading(4,"Rationale",2)},"mode":"verbatim"}
        ]
    });
    let proposed = compact(&project, payload, "synthetic-compaction-author").await;
    let cp = target(&proposed);
    assert_eq!(
        cp, "CP-001",
        "the fresh proposal uses its reserved identifier"
    );
    let accepted = accept_current(&project, &cp).await;
    assert!(
        accepted.contains("accepted"),
        "the independent reviewer accepted current metadata"
    );

    let source_head = head(&project.root);
    let source_a_bytes = source_a.as_bytes();
    let source_b_bytes = source_b.as_bytes();
    let source_index_bytes = index.as_bytes();
    let applied = compact(
        &project,
        json!({"op":"apply","cp":cp}),
        "synthetic-compaction-author",
    )
    .await;
    assert!(
        applied.contains("applied"),
        "the reviewed Markdown proposal is applied: {applied}"
    );
    let apply_commits = git(
        &project.root,
        &["rev-list", "--reverse", &format!("{source_head}..HEAD")],
    );
    let apply_paths: Vec<String> = apply_commits
        .lines()
        .flat_map(|commit| {
            git(
                &project.root,
                &[
                    "diff-tree",
                    "--no-commit-id",
                    "--name-only",
                    "--no-renames",
                    "-r",
                    commit,
                ],
            )
            .lines()
            .map(str::to_owned)
            .collect::<Vec<_>>()
        })
        .collect();
    for path in [
        "docs/combined.md",
        "docs/index.md",
        "docs/a.md",
        "docs/b.md",
    ] {
        assert!(
            apply_paths.contains(&path.to_owned()),
            "apply owns {path}: reply={applied}; paths={apply_paths:?}"
        );
    }
    assert!(
        apply_paths.iter().all(|path| {
            path.starts_with("docs/")
                || path.starts_with("documents/")
                || path.starts_with("compactions/")
                || path == ".agent-tasks/knowledge.yaml"
        }),
        "compaction commits only its Markdown and owned records: {apply_paths:?}"
    );
    assert_eq!(
        git(
            &project.root,
            &["show", &format!("{source_head}:docs/a.md")]
        )
        .as_bytes(),
        source_a_bytes
    );
    assert_eq!(
        git(
            &project.root,
            &["show", &format!("{source_head}:docs/b.md")]
        )
        .as_bytes(),
        source_b_bytes
    );
    assert_eq!(
        git(
            &project.root,
            &["show", &format!("{source_head}:docs/index.md")]
        )
        .as_bytes(),
        source_index_bytes
    );

    let old_pid = project.server_pid;
    project.restart().await;
    assert_ne!(
        project.server_pid, old_pid,
        "restart starts a new server process"
    );
    let before_restart_reads = tree(&project.root);
    let restarted_merged = read_document(&project, "docs/combined.md", json!({})).await;
    assert!(
        restarted_merged.1 > 1,
        "the exact merged body is returned through multiple content pages"
    );
    assert_eq!(
        restarted_merged.0,
        combined.as_bytes(),
        "paged reads reproduce exact current bytes"
    );
    let restarted_index = read_document(&project, "docs/index.md", json!({})).await;
    assert_eq!(
        restarted_index.0,
        index_after.as_bytes(),
        "incoming links point to the combined document"
    );
    let current_context = project
        .call("get_context", json!({"ref":"docs/combined.md"}), false)
        .await;
    let combined_id = current_context
        .split_whitespace()
        .find(|word| word.starts_with("DOC-"))
        .map(|word| {
            word.trim_matches(|character: char| {
                !character.is_ascii_alphanumeric() && character != '-'
            })
        })
        .unwrap_or_else(|| panic!("combined document has a managed identity: {current_context}"));
    let combined_record = format!("documents/{combined_id}.yaml");
    assert!(
        project.root.join(&combined_record).is_file(),
        "managed identity record survives restart"
    );
    let references = project
        .call(
            "get_context",
            json!({"ref":"docs/guide.md","view":"references"}),
            false,
        )
        .await;
    for reference in [&decision_id, &runbook_id, &checklist_id] {
        assert!(
            references.contains(reference),
            "local links survive restart for {reference}: {references}"
        );
    }
    let task = project
        .call("get_context", json!({"ref":"M-001/T-001"}), false)
        .await;
    assert!(
        task.contains("Task counts: 1 done, 0 open, 0 canceled.")
            && task.contains("M-001/T-001 \"Implement fixture\" — done"),
        "the committed core Task remains done after restart: {task}"
    );
    for item in [
        format!("decisions/{decision_id}.yaml"),
        format!("runbooks/{runbook_id}.yaml"),
        format!("checklists/{checklist_id}.yaml"),
        "modules/M-001.yaml".to_owned(),
    ] {
        assert!(
            project.root.join(&item).is_file(),
            "current local state retains {item}"
        );
    }
    assert!(
        staged(&project.root).is_empty(),
        "reads and restart leave a clean index"
    );
    assert_eq!(
        before_restart_reads,
        tree(&project.root),
        "paged reads and context are writeless"
    );

    let clone = project.temp.path().join("full-composition-clone");
    git(
        project.temp.path(),
        &[
            "clone",
            "-q",
            project.root.to_str().unwrap(),
            clone.to_str().unwrap(),
        ],
    );
    assert_eq!(
        std::fs::read(clone.join("docs/combined.md")).unwrap(),
        combined.as_bytes()
    );
    assert_eq!(
        std::fs::read(clone.join("docs/index.md")).unwrap(),
        index_after.as_bytes()
    );
    for original in [
        ("docs/a.md", source_a_bytes),
        ("docs/b.md", source_b_bytes),
        ("docs/index.md", source_index_bytes),
    ] {
        assert_eq!(
            git(&clone, &["show", &format!("{source_head}:{}", original.0)]).as_bytes(),
            original.1,
            "{} original remains recoverable in the clone",
            original.0
        );
    }
    assert!(
        clone.join(&combined_record).is_file(),
        "clone preserves the combined document's local reference record"
    );
    for path in [
        "decisions/D-001.yaml",
        "runbooks/RB-001.yaml",
        "checklists/CL-001.yaml",
    ] {
        assert!(
            clone.join(path).is_file(),
            "clone contains local typed record {path}"
        );
    }
    assert!(
        !head(&project.root).is_empty(),
        "the applied repository has a committed current state"
    );
}
