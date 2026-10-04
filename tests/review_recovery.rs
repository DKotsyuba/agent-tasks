//! Review intent, legacy crash recovery and Project identity regressions.
#[allow(dead_code)]
mod support;
use agent_tasks::records::child_id;
use serde_json::{Value, json};
use support::{Fixture, id};

/// Prepare a reviewable Atomic with complete local output in a disposable Project.
async fn reviewable() -> (Fixture, String) {
    let f = Fixture::new().await;
    let project = f.project().await;
    let atom = f.work("atomic", &project, None).await;
    f.mv(&atom, "In Progress").await;
    f.result("atomic", &atom).await;
    f.mv(&atom, "In Review").await;
    (f, atom)
}

/// Build immutable review arguments; fixture attribution is added by its public caller.
fn review(atom: &str, verdict: &str) -> Value {
    json!({"request_id":id(),"id":atom,"reviewer":"independent","verdict":verdict,"summary":"Checked result","findings":"None","artifacts":["https://example.test/report"]})
}

/// An older accepted review never displaces a newer rejection, including after restart.
#[tokio::test]
async fn reviews_are_monotonic() {
    let (mut f, atom) = reviewable().await;
    let a = review(&atom, "accepted");
    let b = review(&atom, "changes_requested");
    f.ok("record_review", a.clone()).await;
    f.ok("record_review", b.clone()).await;
    f.restart();
    let replay = f.ok("record_review", a.clone()).await;
    assert_eq!(replay["historical"], true);
    assert_eq!(replay["review"]["id"], b["request_id"]);
    assert_eq!(replay["review"]["accepted"], false);
    let mut conflict = a;
    conflict["summary"] = json!("different");
    assert_eq!(f.call("record_review", conflict).await.status, "blocked");
}

/// Lost intent, comment and finalization responses survive cold restart without duplicates.
#[tokio::test]
async fn review_crash_boundaries_resume_only_identical_intent() {
    for (op, nth) in [
        ("MUpdateAttachment", 1),
        ("MCreateComment", 1),
        ("MUpdateAttachment", 2),
    ] {
        let (mut f, atom) = reviewable().await;
        let a = review(&atom, "accepted");
        f.ok("record_review", a).await;
        let b = review(&atom, "changes_requested");
        f.db.lock().await.lose_nth = Some((op.into(), nth));
        assert_eq!(
            f.call("record_review", b.clone()).await.status,
            "outcome_unknown"
        );
        f.restart();
        let aid = child_id(&atom, "state");
        let pending = f.db.lock().await.attachments[&aid]["metadata"]["workflow"]["pending_review"]
            .is_object();
        if pending {
            assert_eq!(
                f.call("edit_atomic", json!({"id":atom,"title":"competing"}))
                    .await
                    .status,
                "blocked"
            );
            let context = f.ok("get_context", json!({"type":"issue","id":atom})).await;
            assert_eq!(context["guidance"]["next_action"]["tool"], "record_review");
        }
        let outcome = f.ok("record_review", b.clone()).await;
        assert_eq!(outcome["review"]["id"], b["request_id"]);
        let db = f.db.lock().await;
        let meta = &db.attachments[&aid]["metadata"]["workflow"];
        assert_eq!(meta["schema"], 3);
        assert!(meta.get("pending_review").is_none());
        assert_eq!(db.comments.len(), 2);
    }
}

/// A schema-two comment-save crash recovers first/newer decisions but rejects ambiguous order.
#[tokio::test]
async fn legacy_review_recovery_requires_ordered_current_stamp() {
    for predecessor in [false, true] {
        let (f, atom) = reviewable().await;
        let aid = child_id(&atom, "state");
        if predecessor {
            f.ok("record_review", review(&atom, "accepted")).await;
        }
        let mut before = f.db.lock().await.attachments[&aid]["metadata"]["workflow"].clone();
        before["schema"] = json!(2);
        let b = review(&atom, "changes_requested");
        f.ok("record_review", b.clone()).await;
        f.db.lock().await.attachments.get_mut(&aid).unwrap()["metadata"]["workflow"] =
            before.clone();
        if predecessor {
            let previous = before["review"]["id"].as_str().unwrap();
            let original_time =
                f.db.lock().await.comments[b["request_id"].as_str().unwrap()]["createdAt"].clone();
            let previous_time = f.db.lock().await.comments[previous]["createdAt"].clone();
            f.db.lock()
                .await
                .comments
                .get_mut(b["request_id"].as_str().unwrap())
                .unwrap()["createdAt"] = previous_time;
            assert_eq!(f.call("record_review", b.clone()).await.status, "blocked");
            f.db.lock()
                .await
                .comments
                .get_mut(b["request_id"].as_str().unwrap())
                .unwrap()["createdAt"] = json!("unknown");
            assert_eq!(f.call("record_review", b.clone()).await.status, "blocked");
            f.db.lock()
                .await
                .comments
                .get_mut(b["request_id"].as_str().unwrap())
                .unwrap()["createdAt"] = original_time;
        }
        f.ok("record_review", b.clone()).await;
        assert_eq!(
            f.db.lock().await.attachments[&aid]["metadata"]["workflow"]["schema"],
            2
        );
        assert_eq!(
            f.db.lock().await.attachments[&aid]["metadata"]["workflow"]["review"]["id"],
            b["request_id"]
        );
    }
}

/// New features upgrade state once; subsequent ordinary writes retain schema three.
#[tokio::test]
async fn schemas_remain_compatible_without_downgrade() {
    let (f, atom) = reviewable().await;
    let aid = child_id(&atom, "state");
    assert_eq!(
        f.db.lock().await.attachments[&aid]["metadata"]["workflow"]["schema"],
        2
    );
    f.ok("record_review", review(&atom, "accepted")).await;
    f.ok("edit_atomic", json!({"id":atom,"title":"presentation"}))
        .await;
    assert_eq!(
        f.db.lock().await.attachments[&aid]["metadata"]["workflow"]["schema"],
        3
    );
    let work = f.gateway.store.work(&atom).await.unwrap();
    let mut legacy = work.managed().unwrap().clone();
    legacy.schema = 2;
    assert!(f.gateway.store.save(&work.native, &legacy).await.is_err());
    f.db.lock().await.attachments.get_mut(&aid).unwrap()["metadata"]["workflow"]["schema"] =
        json!(4);
    assert_eq!(
        f.call("get_context", json!({"type":"issue","id":atom}))
            .await
            .status,
        "blocked"
    );
}

/// Project create retries compare the complete exact team set, including later pages.
#[tokio::test]
async fn project_replay_checks_exact_team_membership() {
    let f = Fixture::new().await;
    let a = json!({"request_id":id(),"team_id":f.team,"title":"Replay","description":"Exact"});
    let p = f.ok("create_project", a.clone()).await;
    let pid = p["project"]["id"].as_str().unwrap();
    f.ok("create_project", a.clone()).await;
    f.db.lock().await.projects.get_mut(pid).unwrap()["teams"]["nodes"] = json!([{"id":id()}]);
    assert_eq!(f.call("create_project", a.clone()).await.status, "blocked");
    let teams: Vec<_> = std::iter::once(json!({"id":f.team}))
        .chain((0..100).map(|_| json!({"id":id()})))
        .collect();
    f.db.lock().await.projects.get_mut(pid).unwrap()["teams"]["nodes"] = json!(teams);
    assert_eq!(f.call("create_project", a).await.status, "blocked");
    assert!(f.db.lock().await.operation_counts["QProjectTeams"] >= 4);
}
