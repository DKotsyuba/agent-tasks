//! Explicit adoption/reopening and retirement guards against native manual description drift.
#[allow(dead_code)]
mod support;
use agent_tasks::records::{child_id, patch_description};
use serde_json::json;
use support::{Fixture, id};

/// Done Task reopening adopts native fields/prose, invalidates once, and preserves code examples.
#[tokio::test]
async fn done_task_reopens_current_description_once() {
    let f = Fixture::new().await;
    let project = f.project().await;
    let module = f.work("module", &project, None).await;
    f.mv(&module, "In Progress").await;
    let task = f.work("task", &project, Some(&module)).await;
    f.mv(&task, "In Progress").await;
    f.result("task", &task).await;
    f.mv(&task, "Done").await;
    let aid = child_id(&task, "state");
    let before = f.db.lock().await.attachments[&aid]["metadata"]["workflow"].clone();
    let body = patch_description(
        before["description"].as_str().unwrap(),
        &json!({"expected_result":"Manually updated"}),
    )
    .unwrap()
        + "\n## Human notes\nKeep me\n\n```md\n## Результат\nliteral\n```\n";
    f.db.lock().await.issues.get_mut(&task).unwrap()["description"] = json!(body);
    let preview = f
        .ok(
            "move_status",
            json!({"id":task,"status":"In Progress","actor_role":"worker","check_only":true}),
        )
        .await;
    assert_eq!(preview["allowed"], true);
    let a = json!({"request_id":id(),"id":task,"status":"In Progress","actor_role":"worker"});
    f.ok("move_status", a.clone()).await;
    f.ok("move_status", a).await;
    let after = f.ok("get_context", json!({"type":"issue","id":task})).await;
    assert_eq!(after["fields"]["expected_result"], "Manually updated");
    assert!(after["fields"]["result"].is_null());
    assert!(
        after["issue"]["description"]
            .as_str()
            .unwrap()
            .contains("Keep me")
    );
    assert!(
        after["issue"]["description"]
            .as_str()
            .unwrap()
            .contains("## Результат\nliteral")
    );
    assert_eq!(
        after["workflow"]["round"].as_u64().unwrap(),
        before["round"].as_u64().unwrap() + 1
    );
    assert_eq!(
        after["workflow"]["revision"].as_u64().unwrap(),
        before["revision"].as_u64().unwrap() + 1
    );
    assert!(after["workflow"]["review"].is_null());
    assert_eq!(after["workflow"]["integration"], json!({}));
}

/// Reviewed Module description adoption refuses invalid fields/structure and directs orchestrator reopening.
#[tokio::test]
async fn reviewed_module_adoption_validates_before_writing() {
    let f = Fixture::new().await;
    let project = f.project().await;
    let module = f.work("module", &project, None).await;
    f.mv(&module, "In Progress").await;
    f.result("module", &module).await;
    f.mv(&module, "In Review").await;
    f.review(&module, "changes_requested").await;
    let original = f.db.lock().await.issues[&module]["description"]
        .as_str()
        .unwrap()
        .to_owned();
    let modified = patch_description(
        &original,
        &json!({"description":"Changed\n\n```md\n## Scope\ncode\n```"}),
    )
    .unwrap()
        + "\n## Human notes\nretain\n";
    f.db.lock().await.issues.get_mut(&module).unwrap()["description"] = json!(modified.clone());
    let context = f
        .ok("get_context", json!({"type":"issue","id":module}))
        .await;
    assert_eq!(
        context["guidance"]["next_action"]["actor_role"],
        "orchestrator"
    );
    assert_eq!(context["guidance"]["next_action"]["tool"], "move_status");
    f.db.lock().await.issues.get_mut(&module).unwrap()["description"] =
        json!(patch_description(&modified, &json!({"work_type":"integration"})).unwrap());
    assert_eq!(
        f.call(
            "move_status",
            json!({"id":module,"status":"In Progress","actor_role":"orchestrator"})
        )
        .await
        .status,
        "blocked"
    );
    f.db.lock().await.issues.get_mut(&module).unwrap()["description"] = json!(modified.clone());
    f.db.lock().await.issues.get_mut(&module).unwrap()["parent"] = json!({"id":id()});
    assert_eq!(
        f.call(
            "move_status",
            json!({"id":module,"status":"In Progress","actor_role":"orchestrator"})
        )
        .await
        .status,
        "blocked"
    );
    f.db.lock().await.issues.get_mut(&module).unwrap()["parent"] = serde_json::Value::Null;
    f.mv(&module, "In Progress").await;
    let after = f
        .ok("get_context", json!({"type":"issue","id":module}))
        .await;
    assert!(
        after["issue"]["description"]
            .as_str()
            .unwrap()
            .contains("retain")
    );
    assert!(after["workflow"]["review"].is_null());
}

/// Active explicit adoption invalidates content identity while title/priority-only edits preserve it.
#[tokio::test]
async fn active_adoption_counts_as_content_change() {
    let f = Fixture::new().await;
    let project = f.project().await;
    let module = f.work("module", &project, None).await;
    f.mv(&module, "In Progress").await;
    let aid = child_id(&module, "state");
    let before = f.db.lock().await.attachments[&aid]["metadata"]["workflow"].clone();
    f.db.lock().await.issues.get_mut(&module).unwrap()["description"] =
        json!(before["description"].as_str().unwrap().to_owned() + "\n## Notes\nHuman prose\n");
    f.ok(
        "edit_module",
        json!({"id":module,"title":"Presentation","priority":2}),
    )
    .await;
    assert_eq!(
        f.db.lock().await.attachments[&aid]["metadata"]["workflow"]["revision"],
        before["revision"]
    );
    f.ok("edit_module", json!({"id":module,"fields":{}})).await;
    assert_eq!(
        f.db.lock().await.attachments[&aid]["metadata"]["workflow"]["revision"]
            .as_u64()
            .unwrap(),
        before["revision"].as_u64().unwrap() + 1
    );
    assert!(
        f.db.lock().await.attachments[&aid]["metadata"]["workflow"]["description"]
            .as_str()
            .unwrap()
            .contains("Human prose")
    );
}

/// Retiring completed children under a Done parent requires reopening that parent first.
#[tokio::test]
async fn retirement_requires_active_clean_parent() {
    let f = Fixture::new().await;
    let project = f.project().await;
    let module = f.work("module", &project, None).await;
    f.mv(&module, "In Progress").await;
    let task = f.work("task", &project, Some(&module)).await;
    f.mv(&task, "In Progress").await;
    f.result("task", &task).await;
    f.mv(&task, "Done").await;
    f.ok("edit_task", json!({"id":task,"fields":{}})).await;
    // Reason is part of the completed fixture before parent closure.
    f.mv(&task, "In Progress").await;
    f.ok(
        "edit_task",
        json!({"id":task,"fields":{"reason":"No longer needed"}}),
    )
    .await;
    f.result("task", &task).await;
    f.mv(&task, "Done").await;
    f.result("module", &module).await;
    f.mv(&module, "In Review").await;
    f.review(&module, "accepted").await;
    f.ok(
        "edit_module",
        json!({"id":module,"fields":{"merge_report":"Merged"}}),
    )
    .await;
    f.mv(&module, "Done").await;
    assert_eq!(
        f.call(
            "move_status",
            json!({"id":task,"status":"Canceled","actor_role":"worker"})
        )
        .await
        .status,
        "blocked"
    );
    f.mv(&module, "In Progress").await;
    f.mv(&task, "Canceled").await;
}
