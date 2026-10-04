//! Canonical historical duplicate records remain readable only with authoritative native facts.
#[allow(dead_code)]
mod support;
use agent_tasks::records::child_id;
use serde_json::{Value, json};
use support::{Fixture, id};

/// Retire a source through the public duplicate transition, then model native null originalIssue.
async fn transferred_duplicate() -> (Fixture, String, String, String) {
    let f = Fixture::new().await;
    let project = f.project().await;
    let source = f.work("atomic", &project, None).await;
    let target = f.work("atomic", &project, None).await;
    let target_url = f.db.lock().await.issues[&target]["url"].clone();
    f.ok(
        "edit_atomic",
        json!({"id":source,"fields":{"reason":"Replaced","duplicate_of":target_url}}),
    )
    .await;
    f.mv(&source, "Duplicate").await;
    f.db.lock()
        .await
        .attachments
        .get_mut(&child_id(&source, "state"))
        .unwrap()["originalIssue"] = Value::Null;
    (f, project, source, target)
}

/// Exact directed native duplicate facts allow both readers without changing or enabling writes to the record.
#[tokio::test]
async fn null_original_duplicate_is_corroborated_on_meta_and_graph_reads() {
    let (f, project, source, target) = transferred_duplicate().await;
    let aid = child_id(&source, "state");
    {
        let mut db = f.db.lock().await;
        for _ in 0..100 {
            let id = id();
            db.relations.insert(id.clone(), json!({"id":id,"type":"related","archivedAt":null,"issue":{"id":source},"relatedIssue":{"id":target}}));
        }
    }
    let before = f.db.lock().await.attachments.clone();
    let meta = f.gateway.store.meta(&source).await.unwrap().unwrap();
    assert_eq!(meta.status, agent_tasks::model::Status::Duplicate);
    let graph = f.gateway.store.graph(&project).await.unwrap();
    let historical = graph.iter().find(|w| w.id() == source).unwrap();
    assert_eq!(historical.managed().unwrap().fields, meta.fields);
    assert!(
        graph
            .iter()
            .find(|w| w.id() == target)
            .unwrap()
            .meta
            .is_some()
    );
    assert!(f.db.lock().await.operation_counts["QIssueRelations"] >= 4);
    assert!(
        f.gateway
            .store
            .save(&historical.native, &meta)
            .await
            .is_err()
    );
    assert_eq!(f.db.lock().await.attachments, before);
    assert_eq!(before[&aid]["id"], aid);
}

/// Missing provenance, nonduplicate state/metadata, mismatched target and ambiguous relations all refuse.
#[tokio::test]
async fn uncorroborated_transfers_are_refused_without_mutation() {
    for case in [
        "missing_original",
        "foreign_original",
        "native_open",
        "metadata_open",
        "wrong_target",
        "missing_relation",
        "nonduplicate_relation",
        "archived_relation",
        "unknown_archive",
        "ambiguous_relation",
        "missing_target",
    ] {
        let (f, project, source, target) = transferred_duplicate().await;
        let aid = child_id(&source, "state");
        {
            let mut db = f.db.lock().await;
            match case {
                "missing_original" => {
                    db.attachments
                        .get_mut(&aid)
                        .unwrap()
                        .as_object_mut()
                        .unwrap()
                        .remove("originalIssue");
                }
                "foreign_original" => {
                    db.attachments.get_mut(&aid).unwrap()["originalIssue"] = json!({"id":target})
                }
                "native_open" => {
                    db.issues.get_mut(&source).unwrap()["state"] =
                        json!({"name":"In Progress","type":"started"})
                }
                "metadata_open" => {
                    db.attachments.get_mut(&aid).unwrap()["metadata"]["workflow"]["status"] =
                        json!("In Progress")
                }
                "wrong_target" => {
                    db.attachments.get_mut(&aid).unwrap()["metadata"]["workflow"]["fields"]["duplicate_of"] =
                        json!("https://linear.app/example/issue/OTHER-1")
                }
                "missing_relation" => db.relations.clear(),
                "nonduplicate_relation" => {
                    db.relations.values_mut().next().unwrap()["type"] = json!("related")
                }
                "archived_relation" => {
                    db.relations.values_mut().next().unwrap()["archivedAt"] =
                        json!("2026-01-01T00:00:00Z")
                }
                "unknown_archive" => {
                    db.relations
                        .values_mut()
                        .next()
                        .unwrap()
                        .as_object_mut()
                        .unwrap()
                        .remove("archivedAt");
                }
                "ambiguous_relation" => {
                    let mut second = db.relations.values().next().unwrap().clone();
                    let id = id();
                    second["id"] = json!(id);
                    db.relations.insert(id, second);
                }
                "missing_target" => {
                    db.issues.remove(&target);
                }
                _ => unreachable!(),
            }
        }
        let before = f.db.lock().await.attachments.clone();
        assert!(f.gateway.store.meta(&source).await.is_err(), "{case}");
        assert!(f.gateway.store.graph(&project).await.is_err(), "{case}");
        assert_eq!(f.db.lock().await.attachments, before, "{case}");
    }
}

/// An incomplete provider response never grants read ownership from hidden metadata alone.
#[tokio::test]
async fn relation_read_failure_keeps_ownership_unverified() {
    let (f, _, source, _) = transferred_duplicate().await;
    let before = f.db.lock().await.attachments.clone();
    f.db.lock().await.lose = Some("QIssueRelations".into());
    assert!(f.gateway.store.meta(&source).await.is_err());
    assert_eq!(f.db.lock().await.attachments, before);
}
