//! Archive collection regressions at the native HTTP boundary; no real credentials are used.
#[allow(dead_code)]
mod support;
use agent_tasks::{archive, records::child_id};
use serde_json::{Value, json};
use support::{Fixture, id};

/// Collection follows every native/recorded descendant, all connection pages and threaded replies without writes.
#[tokio::test]
async fn archive_collects_full_pages_and_explicit_unmanaged_blockers() {
    let f = Fixture::new().await;
    let project = f.project().await;
    let epic = f.work("epic", &project, None).await;
    let module = f.work("module", &project, Some(&epic)).await;
    let task = f.work("task", &project, Some(&module)).await;
    let root = id();
    let doc = id();
    let unmanaged = id();
    {
        let mut db = f.db.lock().await;
        let relation = id();
        db.relations.insert(relation.clone(), json!({"id":relation,"issue":{"id":task},"relatedIssue":{"id":module},"type":"related","archivedAt":"2026-01-01T00:00:00Z"}));
        for index in 0..151 {
            let cid = if index == 0 { root.clone() } else { id() };
            db.comments.insert(cid.clone(),json!({"id":cid,"body":format!("full comment {index}"),"issue":{"id":task},"parent":null,"reactions":[{"id":format!("reaction-{index}"),"emoji":"👍"}],"user":{"id":"author","name":"Author"}}));
        }
        for index in 0..51 {
            let cid = id();
            db.comments.insert(cid.clone(),json!({"id":cid,"body":format!("full reply {index}"),"issue":{"id":task},"parent":{"id":root}}));
        }
        db.documents.insert(doc.clone(),json!({"id":doc,"issue":{"id":task},"title":"Full child document","content":"Original 🧾 text\n## heading\n```\n## literal\n```","updatedAt":"doc-date"}));
        for index in 0..101 {
            let cid = id();
            db.comments.insert(cid.clone(),json!({"id":cid,"body":format!("document comment {index}"),"document":{"id":doc},"parent":null}));
        }
        db.issues.get_mut(&task).unwrap()["history"]=json!((0..101).map(|i|json!({"id":format!("history-{i:03}"),"actor":{"id":"actor"},"changes":{"title":i}})).collect::<Vec<_>>());
        let mut n = db.issues[&task].clone();
        n["id"] = json!(unmanaged);
        n["identifier"] = json!("FIX-UNMANAGED");
        n["parent"] = json!({"id":epic});
        n["history"] = json!([]);
        db.issues.insert(unmanaged.clone(), n);
    }
    let store = f.store();
    let graph = store.graph(&project).await.unwrap();
    let original = store.work(&epic).await.unwrap();
    let before = f.db.lock().await.operation_counts.clone();
    let set = archive::collect(&store, &original, &graph).await.unwrap();
    assert!(
        set.blockers
            .iter()
            .any(|b| b.code == "UNMANAGED_ITEM" && b.item == unmanaged)
    );
    assert_eq!(set.items.len(), 2);
    let item = set.items.iter().find(|n| n.native["id"] == task).unwrap();
    assert_eq!(item.comments.len(), 202);
    assert_eq!(item.history.len(), 101);
    assert_eq!(item.relations.len(), 1);
    assert!(item.relations[0]["archivedAt"].is_string());
    assert_eq!(item.documents.len(), 1);
    assert_eq!(item.documents[0]["comments"].as_array().unwrap().len(), 101);
    assert!(
        item.documents[0]["content"]
            .as_str()
            .unwrap()
            .contains("Original 🧾")
    );
    let db = f.db.lock().await;
    for (operation, count) in &db.operation_counts {
        if operation.starts_with('M') {
            assert_eq!(*count, before.get(operation).copied().unwrap_or(0));
        }
    }
    assert!(db.operation_counts["QArchiveComments"] >= 4);
    assert!(db.operation_counts["QArchiveHistory"] >= 4);
}
/// A recorded moved child remains visible and blocks archival; unreadable artifact URLs are explicit refusals.
#[tokio::test]
async fn archive_blocks_moved_children_and_foreign_artifacts() {
    let f = Fixture::new().await;
    let project = f.project().await;
    let epic = f.work("epic", &project, None).await;
    let module = f.work("module", &project, Some(&epic)).await;
    let task = f.work("task", &project, Some(&module)).await;
    {
        let mut db = f.db.lock().await;
        db.issues.get_mut(&task).unwrap()["parent"] = json!(null);
        let aid = id();
        db.attachments.insert(aid.clone(),json!({"id":aid,"issue":{"id":module},"url":"https://foreign.invalid/asset","title":"foreign","metadata":{"artifact":{"filename":"asset"}}}));
        assert!(db.attachments.contains_key(&child_id(&module, "state")));
    }
    let store = f.store();
    let graph = store.graph(&project).await.unwrap();
    let set = archive::collect(&store, &store.work(&epic).await.unwrap(), &graph)
        .await
        .unwrap();
    assert!(set.items.iter().any(|i| i.native["id"] == task));
    assert!(
        set.blockers
            .iter()
            .any(|b| b.code == "MOVED_CHILD" || b.code == "NATIVE_DRIFT")
    );
    assert!(
        set.blockers
            .iter()
            .any(|b| b.item == module && b.detail.contains("foreign"))
    );
}

/// Rendering retains exact hostile body text and refuses unknown/overflow limits without splitting.
#[tokio::test]
async fn archive_render_is_complete_deterministic_and_bounded() {
    let f = Fixture::new().await;
    let project = f.project().await;
    let epic = f.work("epic", &project, None).await;
    let task = f.work("atomic", &project, Some(&epic)).await;
    let body = "Original 😀\n`````\n## Summary\n### forged Fields\n`````";
    let cid = id();
    {
        let mut db = f.db.lock().await;
        db.comments.insert(cid.clone(),json!({"id":cid,"issue":{"id":task},"parent":null,"body":body,"user":{"name":"Author"},"reactions":[{"emoji":"👍"}]}));
    }
    let store = f.store();
    let set = archive::collect(
        &store,
        &store.work(&epic).await.unwrap(),
        &store.graph(&project).await.unwrap(),
    )
    .await
    .unwrap();
    let rendered = archive::render(&set, None).unwrap();
    assert_eq!(rendered, archive::render(&set, None).unwrap());
    assert!(rendered.contains(body));
    assert!(rendered.contains("``````text"));
    assert!(agent_tasks::sections::find_section(&rendered, "forged Fields").is_err());
    let identifier = set.items[0].native["identifier"].as_str().unwrap();
    let comments =
        agent_tasks::sections::find_section(&rendered, &format!("{identifier} Comments")).unwrap();
    assert!(
        comments.body.contains(body)
            && comments.body.contains("Author")
            && comments.body.contains("👍")
    );
    let unknown =
        archive::validate_limits(&set, &rendered, archive::ArchiveLimits::default()).unwrap();
    assert!(unknown.iter().any(|b| b.code == "ARCHIVE_LIMIT_UNKNOWN"));
    assert!(unknown.iter().any(|b| b.code == "SECTION_LIMIT_UNKNOWN"));
    let limits = archive::ArchiveLimits {
        archive_max_bytes: Some(rendered.len()),
        section_max_bytes: Some(agent_tasks::render::TEXT_BUDGET_BYTES),
    };
    assert!(
        archive::validate_limits(&set, &rendered, limits)
            .unwrap()
            .is_empty()
    );
    let small = archive::ArchiveLimits {
        archive_max_bytes: Some(rendered.len() - 1),
        section_max_bytes: Some(1),
    };
    let blocked = archive::validate_limits(&set, &rendered, small).unwrap();
    assert!(blocked.iter().any(|b| b.code == "ARCHIVE_TOO_LARGE"));
    assert!(blocked.iter().any(|b| b.code == "SECTION_TOO_LARGE"));
}

/// Explicit prepare/publish/reparent actions survive lost canonical replies and cold readback without duplicates.
#[tokio::test]
async fn archive_preservation_reconciles_files_and_documents() {
    let f = Fixture::new().await;
    let project = f.project().await;
    let epic = f.work("epic", &project, None).await;
    let task = f.work("atomic", &project, Some(&epic)).await;
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("preservation.bin");
    std::fs::write(&path, b"retained\0binary").unwrap();
    f.ok(
        "upload_file",
        json!({"work_id":task,"path":path.to_str().unwrap(),"note":"Original note"}),
    )
    .await;
    let doc=f.ok("save_document",json!({"issue_id":task,"title":"Native document","content":"Full native document 😀\n```\n## literal\n```"})).await;
    let store = f.store();
    let mut set = archive::collect(
        &store,
        &store.work(&epic).await.unwrap(),
        &store.graph(&project).await.unwrap(),
    )
    .await
    .unwrap();
    assert_eq!(set.assets.len(), 1);
    assert_eq!(set.assets[0].content_type, "application/octet-stream");
    assert_ne!(set.assets[0].content_type, set.assets[0].digest);
    assert!(
        archive::preservation_plan(&set).is_err(),
        "unfinished fixture sources must block a production plan"
    );
    // The test exercises preservation primitives only; no deletion or empirical gate qualification occurs.
    set.blockers.clear();
    let plan = archive::preservation_plan(&set).unwrap();
    assert_eq!(plan.len(), 2);
    let prepared = archive::execute_preservation(&store, &plan[0])
        .await
        .unwrap();
    assert_eq!(
        f.db.lock().await.upload_content_types.last().unwrap(),
        "application/octet-stream"
    );
    let next = prepared.next.unwrap();
    let serialized = serde_json::to_string(&next).unwrap();
    assert!(!serialized.contains("uploadUrl") && !serialized.contains("headers"));
    f.db.lock().await.lose = Some("MCreateArtifact".into());
    let attached = archive::execute_preservation(&store, &next).await.unwrap();
    assert!(attached.next.is_none());
    let count = f.db.lock().await.operation_counts["MCreateArtifact"];
    archive::execute_preservation(&store, &plan[0])
        .await
        .unwrap();
    assert_eq!(
        f.db.lock().await.operation_counts["MCreateArtifact"],
        count,
        "reconcile must not publish twice"
    );
    let attachment = attached.confirmed["attachment"].clone();
    assert_eq!(attachment["issue"]["id"], epic);
    assert_eq!(attachment["metadata"]["artifact"], set.assets[0].artifact);
    assert_eq!(attachment["metadata"]["compacted_from"]["issue_id"], task);
    f.db.lock().await.lose = Some("MUpdateDocument".into());
    archive::execute_preservation(&store, &plan[1])
        .await
        .unwrap();
    archive::preservation_readback(&f.store(), &plan[1])
        .await
        .unwrap();
    let native = &f.db.lock().await.documents[doc["id"].as_str().unwrap()];
    assert_eq!(native["issue"]["id"], epic);
    assert!(native["project"].is_null());
    assert!(
        native["content"]
            .as_str()
            .unwrap()
            .contains("Full native document 😀")
    );
}
/// Concurrent native Document content and copied byte drift both refuse rather than overwriting another writer.
#[tokio::test]
async fn archive_preservation_rejects_source_and_provenance_conflicts() {
    let f = Fixture::new().await;
    let project = f.project().await;
    let epic = f.work("epic", &project, None).await;
    let task = f.work("atomic", &project, Some(&epic)).await;
    let doc = f
        .ok(
            "save_document",
            json!({"issue_id":task,"title":"Owned document","content":"Original"}),
        )
        .await;
    let store = f.store();
    let mut set = archive::collect(
        &store,
        &store.work(&epic).await.unwrap(),
        &store.graph(&project).await.unwrap(),
    )
    .await
    .unwrap();
    set.blockers.clear();
    let action = archive::preservation_plan(&set).unwrap().remove(0);
    f.db.lock()
        .await
        .documents
        .get_mut(doc["id"].as_str().unwrap())
        .unwrap()["content"] = json!("Concurrent native edit");
    let fault = archive::execute_preservation(&store, &action)
        .await
        .unwrap_err();
    assert_eq!(fault.code, "SOURCE_CHANGED");
    assert_eq!(
        f.db.lock().await.documents[doc["id"].as_str().unwrap()]["issue"]["id"],
        task
    );
}

/// Section queries omit full bodies, return at most20 code-aware matches and tell callers to narrow.
#[tokio::test]
async fn archive_document_queries_are_bounded_and_truthful() {
    let f = Fixture::new().await;
    let project = f.project().await;
    let body = format!(
        "## Items\n{}\n```\n### fake needle\n```",
        (0..25)
            .map(|i| format!("### Item {i}\nneedle body {i}\n"))
            .collect::<String>()
    );
    let doc = f
        .ok(
            "save_document",
            json!({"project_id":project,"title":"Archive","content":body}),
        )
        .await;
    let result = f
        .ok(
            "get_context",
            json!({"type":"document","id":doc["id"],"query":"NEEDLE"}),
        )
        .await;
    assert!(result.get("content").is_none());
    let rows = result["section_query"]["matches"].as_array().unwrap();
    assert_eq!(rows.len(), 20);
    assert_eq!(result["section_query"]["has_more"], true);
    assert!(
        rows.iter().all(|r| r["heading"] != "fake needle"
            && r["snippet"].as_str().unwrap().chars().count() <= 202)
    );
    let output = agent_tasks::render::render_outcome(
        "get_context",
        &json!({"type":"document","id":doc["id"],"query":"NEEDLE"}),
        &agent_tasks::model::Outcome::ok(result),
    );
    assert!(output.contains("narrow the query") && output.contains("Whole document"));
    let empty = f
        .ok(
            "get_context",
            json!({"type":"document","id":doc["id"],"query":"absent"}),
        )
        .await;
    assert_eq!(empty["section_query"]["matches"], json!([]));
    assert_eq!(empty["section_query"]["has_more"], false);
    let bad = f
        .call(
            "get_context",
            json!({"type":"document","id":doc["id"],"query":"needle","section":"Items"}),
        )
        .await;
    assert_eq!(bad.status, "blocked");
    let exactly = (0..20)
        .map(|i| format!("## {i}\nmatch\n"))
        .collect::<String>();
    assert!(
        !agent_tasks::sections::matching_sections(&exactly, "match")
            .unwrap()
            .1
    );
}
/// An over-budget native Document search retries one item with the original cursor and preserves pagination.
#[tokio::test]
async fn archive_document_search_reduces_large_native_pages() {
    let f = Fixture::new().await;
    let project = f.project().await;
    {
        let mut db = f.db.lock().await;
        for index in 0..3 {
            let id = format!("00000000-0000-4000-8000-00000000000{}", index + 1);
            db.documents.insert(id.clone(),json!({"id":id,"title":"large archive target","content":"x".repeat(4_300_000),"project":{"id":project},"updatedAt":"fixture","archivedAt":null,"hiddenAt":null}));
        }
    }
    let first = f
        .ok(
            "search",
            json!({"type":"document","query":"target","project_id":project,"first":3}),
        )
        .await;
    assert_eq!(first["native_page_size"], 1);
    assert_eq!(first["effective_first"], 1);
    assert_eq!(first["nodes"].as_array().unwrap().len(), 1);
    assert_eq!(first["pageInfo"]["hasNextPage"], true);
    let next=f.ok("search",json!({"type":"document","query":"target","project_id":project,"first":3,"after":first["pageInfo"]["endCursor"]})).await;
    assert_eq!(next["native_page_size"], 1);
    assert_ne!(next["nodes"][0]["id"], first["nodes"][0]["id"]);
    assert_eq!(next["pageInfo"]["hasNextPage"], true);
    assert_eq!(f.db.lock().await.operation_counts["QSearchDocuments"], 4);
}

/// Native nullable trash flags stay distinct from an unreadable/missing flag and positive deletion.
#[test]
fn archive_native_trash_flag_is_nullable_but_present() {
    assert!(!archive::is_trashed(&json!({"trashed":null})).unwrap());
    assert!(!archive::is_trashed(&json!({"trashed":false})).unwrap());
    assert!(archive::is_trashed(&json!({"trashed":true})).unwrap());
    assert_eq!(
        archive::is_trashed(&json!({})).unwrap_err().code,
        "INCOMPLETE_DATA"
    );
    assert!(archive::is_trashed(&json!({"trashed":"false"})).is_err());
}
/// Fresh native Issues with present null flags collect successfully without fabricating trash confirmation.
#[tokio::test]
async fn archive_collects_native_null_trash_flags() {
    let f = Fixture::new().await;
    let project = f.project().await;
    let epic = f.work("epic", &project, None).await;
    let task = f.work("atomic", &project, Some(&epic)).await;
    {
        let mut db = f.db.lock().await;
        db.issues.get_mut(&epic).unwrap()["trashed"] = Value::Null;
        db.issues.get_mut(&task).unwrap()["trashed"] = Value::Null;
    }
    let store = f.store();
    let set = archive::collect(
        &store,
        &store.work(&epic).await.unwrap(),
        &store.graph(&project).await.unwrap(),
    )
    .await
    .unwrap();
    assert!(set.epic.native.get("trashed").is_some());
    assert!(set.epic.native["trashed"].is_null());
    assert_eq!(set.items.len(), 1);
}

/// Per-item snapshots include new comments/Documents even when the parent Issue timestamp stays unchanged.
#[tokio::test]
async fn archive_collect_item_rechecks_details_without_timestamp_assumptions() {
    let f = Fixture::new().await;
    let project = f.project().await;
    let task = f.work("atomic", &project, None).await;
    let store = f.store();
    let work = store.work(&task).await.unwrap();
    let before = archive::collect_item(&store, &work).await.unwrap();
    {
        let mut db = f.db.lock().await;
        let comment = id();
        db.comments.insert(comment.clone(),json!({"id":comment,"issue":{"id":task},"parent":null,"body":"new comment without Issue timestamp change"}));
        let doc = id();
        db.documents.insert(doc.clone(),json!({"id":doc,"issue":{"id":task},"title":"new Document","content":"new Document body","updatedAt":"doc-only-time"}));
    }
    let after = archive::collect_item(&store, &work).await.unwrap();
    assert_eq!(before.native["updatedAt"], after.native["updatedAt"]);
    assert_eq!(after.comments.len(), before.comments.len() + 1);
    assert_eq!(after.documents.len(), before.documents.len() + 1);
    assert_ne!(
        serde_json::to_value(&before).unwrap(),
        serde_json::to_value(&after).unwrap()
    );
    f.db.lock().await.issues.get_mut(&task).unwrap()["trashed"] = json!(true);
    assert_eq!(
        archive::collect_item(&store, &work).await.unwrap_err().code,
        "SOURCE_CHANGED"
    );
}

/// Native outer fence normalization is allowed only while literal source text and complete structure stay exact.
#[test]
fn archive_readback_proof_is_literal_and_structural() {
    let expected = "# Archive\n\n## Summary\ntext\n\n## FIX · TASK\n\n### FIX Comments\n\n````text\n| a | b |\n|---|---|\n``` nested\n😀  \n````\n";
    let normalized = expected
        .replace("````text", "~~~~")
        .replace("\n````\n", "\n~~~~\n");
    archive::verify_readback(expected, expected).unwrap();
    archive::verify_readback(expected, &normalized).unwrap();
    assert!(archive::verify_readback(expected, &normalized.replace("😀  ", "😀 ")).is_err());
    assert!(
        archive::verify_readback(expected, &normalized.replace("FIX Comments", "FIX Files"))
            .is_err()
    );
    assert!(
        archive::verify_readback(expected, &format!("{normalized}\n### unexpected\n")).is_err()
    );
    let crlf = "# Archive\n\n```text\noriginal\r\nline\n```\n";
    assert!(
        archive::verify_readback(crlf, &crlf.replace("\r\n", "\n")).is_err(),
        "source CRLF may not be silently normalized"
    );
}
/// Metadata allowance applies to actual serialized JSON including the existing state and escaping.
#[test]
fn archive_metadata_budget_counts_the_complete_json_envelope() {
    let value = json!({"workflow":{"existing":"x".repeat(100),"compaction":{"manifest":["😀","quoted\\text"]}}});
    let bytes = archive::metadata_budget(&value).unwrap();
    assert_eq!(bytes, serde_json::to_vec(&value).unwrap().len());
    assert!(
        archive::metadata_budget(
            &json!({"workflow":{"existing":"x".repeat(archive::METADATA_BUDGET_BYTES)}})
        )
        .is_err()
    );
    let accepted = json!({"probe":"x".repeat(16384)});
    assert!(serde_json::to_vec(&accepted).unwrap().len() > archive::METADATA_BUDGET_BYTES);
}

/// A large complete Document remains in the archive while its reparent journal carries only an exact compact witness.
#[tokio::test]
async fn archive_reparent_journal_is_compact_and_content_exact() {
    let f = Fixture::new().await;
    let project = f.project().await;
    let epic = f.work("epic", &project, None).await;
    let task = f.work("atomic", &project, Some(&epic)).await;
    let doc=f.ok("save_document",json!({"issue_id":task,"title":"Large exact source","content":"large original\n".repeat(5000)})).await;
    let store = f.store();
    let mut set = archive::collect(
        &store,
        &store.work(&epic).await.unwrap(),
        &store.graph(&project).await.unwrap(),
    )
    .await
    .unwrap();
    set.blockers.clear();
    let action = archive::preservation_plan(&set).unwrap().remove(0);
    assert!(serde_json::to_vec(&action).unwrap().len() < 2048);
    assert!(
        archive::render(&set, None)
            .unwrap()
            .contains("large original\nlarge original")
    );
    f.db.lock()
        .await
        .documents
        .get_mut(doc["id"].as_str().unwrap())
        .unwrap()["content"] = json!("large original\n".repeat(4999));
    assert_eq!(
        archive::execute_preservation(&store, &action)
            .await
            .unwrap_err()
            .code,
        "SOURCE_CHANGED"
    );
}

/// Plain Documents and preamble text are searched honestly, without claiming absent matches.
#[tokio::test]
async fn archive_section_queries_search_unheaded_content() {
    let f = Fixture::new().await;
    let project = f.project().await;
    for content in [
        "needle in plain text",
        "needle before headings\n\n## Other\nunrelated",
    ] {
        let doc = f
            .ok(
                "save_document",
                json!({"project_id":project,"title":"Unheaded","content":content}),
            )
            .await;
        let result = f
            .ok(
                "get_context",
                json!({"type":"document","id":doc["id"],"query":"needle"}),
            )
            .await;
        assert_eq!(
            result["section_query"]["matches"].as_array().unwrap().len(),
            1
        );
        assert_eq!(result["section_query"]["matches"][0]["index"], 0);
        let text = agent_tasks::render::render_outcome(
            "get_context",
            &json!({"type":"document","id":doc["id"],"query":"needle"}),
            &agent_tasks::model::Outcome::ok(result),
        );
        assert!(
            text.contains("Document preamble")
                && text.contains("needle")
                && !text.contains("presentation")
        );
    }
}

/// Selected native metadata remains in the export, and truncated labels refuse instead of disappearing.
#[tokio::test]
async fn archive_exports_metadata_and_refuses_incomplete_labels() {
    let f = Fixture::new().await;
    let project = f.project().await;
    let task = f.work("atomic", &project, None).await;
    {
        let mut db = f.db.lock().await;
        let native = db.issues.get_mut(&task).unwrap();
        native["createdAt"] = json!("creation");
        native["creator"] = json!({"id":"creator","name":"Creator"});
        native["assignee"] = json!({"id":"assignee","name":"Assignee"});
        native["dueDate"] = json!("2026-10-10");
        native["estimate"] = json!(3.0);
        native["cycle"] = json!({"id":"cycle","name":"Cycle","number":2});
        let attachment = id();
        db.attachments.insert(attachment.clone(),json!({"id":attachment,"issue":{"id":task},"title":"Link","subtitle":"Native subtitle","source":{"provider":"native"},"createdAt":"attachment-created","url":"https://example.test","metadata":{}}));
    }
    let store = f.store();
    let work = store.work(&task).await.unwrap();
    let item = archive::collect_item(&store, &work).await.unwrap();
    assert_eq!(item.native["dueDate"], "2026-10-10");
    assert_eq!(item.native["creator"]["name"], "Creator");
    assert_eq!(item.native["cycle"]["number"], 2);
    assert_eq!(item.files[0]["subtitle"], "Native subtitle");
    assert_eq!(item.files[0]["source"]["provider"], "native");
    f.db.lock().await.issues.get_mut(&task).unwrap()["labels"]["pageInfo"]["hasNextPage"] =
        json!(true);
    assert_eq!(
        archive::collect_item(&store, &store.work(&task).await.unwrap())
            .await
            .unwrap_err()
            .code,
        "INCOMPLETE_DATA"
    );
}
/// A readable but zero-byte referenced file is an explicit preservation blocker before any upload.
#[tokio::test]
async fn archive_zero_byte_assets_block_before_preservation() {
    let f = Fixture::new().await;
    let project = f.project().await;
    let epic = f.work("epic", &project, None).await;
    {
        let mut db = f.db.lock().await;
        db.assets.insert("empty.bin".into(), vec![]);
        let id = id();
        let url = format!("{}/asset/empty.bin", db.base);
        db.attachments.insert(id.clone(),json!({"id":id,"issue":{"id":epic},"url":url,"metadata":{"artifact":{"filename":"empty.bin","content_type":"application/octet-stream"}}}));
    }
    let store = f.store();
    let set = archive::collect(
        &store,
        &store.work(&epic).await.unwrap(),
        &store.graph(&project).await.unwrap(),
    )
    .await
    .unwrap();
    assert!(set.blockers.iter().any(|b| b.code == "EMPTY_ASSET"));
    assert!(set.assets.is_empty());
}
/// Escaped transport size and Summary/item bodies use the same conservative section refusal as H3 data.
#[tokio::test]
async fn archive_limits_include_escaped_wire_and_summary_sections() {
    let f = Fixture::new().await;
    let project = f.project().await;
    let epic = f.work("epic", &project, None).await;
    let store = f.store();
    let set = archive::collect(
        &store,
        &store.work(&epic).await.unwrap(),
        &store.graph(&project).await.unwrap(),
    )
    .await
    .unwrap();
    let body = archive::render(&set, None).unwrap();
    let limits = archive::ArchiveLimits {
        archive_max_bytes: Some(8 * 1024 * 1024),
        section_max_bytes: Some(1),
    };
    assert!(
        archive::validate_limits(&set, &body, limits)
            .unwrap()
            .iter()
            .any(|b| b.detail.starts_with("Summary"))
    );
    let escaped = "\"".repeat(4_200_000);
    assert!(
        archive::validate_limits(
            &set,
            &escaped,
            archive::ArchiveLimits {
                archive_max_bytes: Some(8 * 1024 * 1024),
                section_max_bytes: None
            }
        )
        .unwrap()
        .iter()
        .any(|b| b.code == "ARCHIVE_WIRE_TOO_LARGE")
    );
    let native = json!({"id":"native-doc","title":"Archive","content":body,"url":"https://linear.app/example/document/native","updatedAt":"native"});
    let limits = archive::ArchiveLimits {
        archive_max_bytes: Some(body.len()),
        section_max_bytes: Some(agent_tasks::render::TEXT_BUDGET_BYTES),
    };
    assert!(
        archive::validate_readback(&set, &body, &native, limits)
            .unwrap()
            .is_empty()
    );
}
