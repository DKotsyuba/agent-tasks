//! Archive collection regressions at the native HTTP boundary; no real credentials are used.
#[allow(dead_code)]
mod support;
use agent_tasks::{archive, records::child_id};
use serde_json::json;
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
