//! Two persistent real-SDK clients over one isolated documentation home: a client that has read and
//! then idles must never keep a store lock that blocks another client's write.
//!
//! Observed once with an installed server: a reader process kept a read descriptor on `write.lock`
//! after its calls returned and every writer answered busy. Each scenario starts the shipped binary (or
//! the binary named by `MCP_TEST_BINARY`, so the installed build can be compared with the current one)
//! twice against disposable state and probes the lock with the operating system itself.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "Explicit protocol assertions"
)]
mod support;

use serde_json::json;
use std::fs::File;
use support::{Project, call, connect};

/// Run every read tool once, so any per-call read lock has been taken and must have been released.
async fn read_everything(client: &support::Client, alias: &str) {
    for (tool, args) in [
        ("get_context", json!({"project":alias})),
        ("project_status", json!({"project":alias})),
        ("search", json!({"project":alias,"query":"qualification"})),
        ("get_context", json!({"project":alias})),
    ] {
        call(client, tool, args, false).await;
    }
}

/// Create one Module through `client`, the smallest real work mutation.
async fn write_module(client: &support::Client, alias: &str, title: &str) -> String {
    let context = call(client, "get_context", json!({"project":alias}), false).await;
    let version = context
        .lines()
        .find_map(|line| line.strip_prefix("Allocation version: "))
        .expect("project context prints the allocation version")
        .trim()
        .to_owned();
    let source = std::env::temp_dir();
    call(
        client,
        "plan_work",
        json!({"project":alias,"op":"create_module","version":version,"title":title,"outcome":"Two clients write in turn","criteria":["Two clients write in turn"],"execution":{"repository":source,"worktree":source,"branch":"main","target_branch":"main"},"contracts":{"not_required":true}}),
        false,
    )
    .await
}

/// Whether the operating system grants an exclusive lock on the store's writer lock file right now.
fn exclusive_lock_is_free(project: &Project) -> bool {
    let Ok(file) = File::options()
        .read(true)
        .write(true)
        .open(project.root.join(".agent-tasks/write.lock"))
    else {
        return true;
    };
    let free = file.try_lock().is_ok();
    drop(file);
    free
}

/// One client reads everything and idles; a second client writes, then the roles swap. No write is busy
/// and the operating system never finds the writer lock held by an idle client.
#[tokio::test]
async fn an_idle_reader_never_blocks_another_persistent_client() {
    let project = Project::register().await;
    let other = connect(&project.config, project.temp.path()).await;
    read_everything(&project.client, project.alias).await;
    assert!(
        exclusive_lock_is_free(&project),
        "the idle reader keeps the writer lock"
    );
    let first = write_module(&other, project.alias, "Second client").await;
    assert!(first.starts_with("SAVED"), "{first}");
    read_everything(&other, project.alias).await;
    assert!(
        exclusive_lock_is_free(&project),
        "the second client keeps the writer lock after reading"
    );
    let second = write_module(&project.client, project.alias, "First client").await;
    assert!(second.starts_with("SAVED"), "{second}");
    assert!(exclusive_lock_is_free(&project));
}
