//! Request settlement through the real registry router on disposable real Git repositories: a
//! changed success commits only its own files, a real no-op and a read never commit, unrelated
//! staged and dirty files survive, and a root that is not a repository still reports a truthful
//! saved receipt.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "Fixture construction and explicit assertions"
)]
use super::context_tests::Host;
use crate::persist::testing::isolate_git;
use serde_json::json;
use std::{fs, path::Path, process::Command};

/// Run Git in `root` with isolated configuration and return trimmed stdout.
fn git(root: &Path, args: &[&str]) -> String {
    let output = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(args)
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).unwrap().trim().to_owned()
}

/// Number of commits reachable from HEAD.
fn commits(root: &Path) -> usize {
    git(root, &["rev-list", "--count", "HEAD"]).parse().unwrap()
}

/// Register a real documentation repository `reg` under the host and configure a test identity
/// without signing inside it. Returns the repository root.
async fn registered(host: &Host) -> std::path::PathBuf {
    let root = host.path("reg-docs");
    host.call(
        "register_project",
        json!({"project":"reg","doc_dir":root,"name":"Registered","description":"Settlement fixture"}),
        false,
    )
    .await;
    for (key, value) in [
        ("user.name", "Fixture"),
        ("user.email", "fixture@example.invalid"),
        ("commit.gpgsign", "false"),
    ] {
        git(&root, &["config", key, value]);
    }
    root
}

/// A changed work mutation commits exactly its own files and leaves foreign staging and dirt.
#[tokio::test]
async fn changed_success_commits_only_its_own_files() {
    isolate_git();
    let host = Host::new();
    let root = registered(&host).await;
    let bootstrap = commits(&root);
    fs::write(root.join("foreign-dirty.txt"), b"dirty\n").unwrap();
    fs::write(root.join("foreign-staged.txt"), b"staged\n").unwrap();
    git(&root, &["add", "--", "foreign-staged.txt"]);
    let context = host
        .call("get_context", json!({"project":"reg"}), false)
        .await;
    let reply = host
        .call(
            "plan_work",
            json!({"project":"reg","op":"create_module","version":Host::field(&context,"Allocation version: "),"title":"Settled","outcome":"Commit after success"}),
            false,
        )
        .await;
    assert!(reply.contains("\nGit: committed "), "{reply}");
    assert_eq!(commits(&root), bootstrap + 1);
    let files = git(&root, &["show", "--name-only", "--format=", "HEAD"]);
    assert!(files.contains("modules/M-001.yaml"), "{files}");
    assert!(!files.contains("foreign"), "{files}");
    assert_eq!(
        git(&root, &["diff", "--cached", "--name-only"]),
        "foreign-staged.txt"
    );
    assert!(git(&root, &["status", "--porcelain"]).contains("?? foreign-dirty.txt"));
}

/// A real no-op and every read leave Git untouched and print no Git line.
#[tokio::test]
async fn noop_and_reads_never_commit() {
    isolate_git();
    let host = Host::new();
    let root = registered(&host).await;
    let context = host
        .call("get_context", json!({"project":"reg"}), false)
        .await;
    host.call(
        "plan_work",
        json!({"project":"reg","op":"create_module","version":Host::field(&context,"Allocation version: "),"title":"Same","outcome":"Stay the same"}),
        false,
    )
    .await;
    let settled = commits(&root);
    let module = host
        .call("get_context", json!({"project":"reg","ref":"M-001"}), false)
        .await;
    let noop = host
        .call(
            "plan_work",
            json!({"project":"reg","op":"edit_module","module":"M-001","version":Host::field(&module,"Version: "),"title":"Same"}),
            false,
        )
        .await;
    assert!(noop.starts_with("UNCHANGED M-001"), "{noop}");
    assert!(!noop.contains("Git:"), "{noop}");
    for read in [
        ("get_context", json!({"project":"reg"})),
        ("project_status", json!({"project":"reg"})),
        ("search", json!({"project":"reg","query":"same"})),
    ] {
        let text = host.call(read.0, read.1, false).await;
        assert!(!text.contains("Git: committed"), "{text}");
    }
    assert_eq!(commits(&root), settled);
}

/// A root that is not an independent repository reports a truthful saved receipt.
#[tokio::test]
async fn non_repository_root_reports_saved() {
    isolate_git();
    let host = Host::new();
    host.populate().await;
    let context = host
        .call("get_context", json!({"project":"alpha"}), false)
        .await;
    let reply = host
        .call(
            "plan_work",
            json!({"project":"alpha","op":"create_module","version":Host::field(&context,"Allocation version: "),"title":"Plain","outcome":"No repository"}),
            false,
        )
        .await;
    assert!(
        reply.contains("Git: saved on disk; nothing was committed for this call."),
        "{reply}"
    );
    assert!(!reply.contains("Git: committed"), "{reply}");
}

/// Create a module in project `reg` through the real router and return the reply.
async fn module(host: &Host, title: &str, error: bool) -> String {
    let context = host
        .call("get_context", json!({"project":"reg"}), false)
        .await;
    host.call(
        "plan_work",
        json!({"project":"reg","op":"create_module","version":Host::field(&context,"Allocation version: "),"title":title,"outcome":"Settle after success"}),
        error,
    )
    .await
}

/// A failure after publication keeps what was saved, is pending, and never commits.
#[cfg(unix)]
#[tokio::test]
async fn failure_after_publication_stays_saved_and_pending() {
    use std::os::unix::fs::PermissionsExt;
    isolate_git();
    let host = Host::new();
    let root = registered(&host).await;
    module(&host, "First", false).await;
    let settled = commits(&root);
    let modules = root.join("modules");
    fs::set_permissions(&modules, fs::Permissions::from_mode(0o555)).unwrap();
    let failed = module(&host, "Second", true).await;
    fs::set_permissions(&modules, fs::Permissions::from_mode(0o755)).unwrap();
    assert!(failed.contains("Visible publication occurred"), "{failed}");
    assert!(failed.contains("Git: "), "{failed}");
    assert!(!failed.contains("Git: committed"), "{failed}");
    assert_eq!(commits(&root), settled, "a partial call is never committed");
    assert!(
        git(&root, &["status", "--porcelain"]).contains(".agent-tasks/state.yaml"),
        "the reservation stays saved on disk"
    );
}

/// Recovery runs in the same scope without ordinary settlement, routes by tool kind and
/// refuses a stale pending version.
#[cfg(unix)]
#[tokio::test]
async fn recovery_uses_the_project_route_and_refuses_a_stale_version() {
    use std::os::unix::fs::PermissionsExt;
    isolate_git();
    let host = Host::new();
    let root = registered(&host).await;
    module(&host, "First", false).await;
    let modules = root.join("modules");
    fs::set_permissions(&modules, fs::Permissions::from_mode(0o555)).unwrap();
    let failed = module(&host, "Second", true).await;
    fs::set_permissions(&modules, fs::Permissions::from_mode(0o755)).unwrap();
    let intent = failed
        .split("First: ")
        .chain(failed.split("Pending: 1 intent(s). First: "))
        .find_map(|part| {
            part.strip_prefix("PG-")
                .map(|rest| format!("PG-{}", &rest[..24]))
        })
        .expect(&failed);
    let before = commits(&root);
    let stale = host
        .call(
            "git_recovery",
            json!({"project":"reg","op":"release","intents":[intent],"version":"0".repeat(64)}),
            true,
        )
        .await;
    assert!(stale.contains("stale"), "{stale}");
    assert!(!stale.contains("Git: committed"), "{stale}");
    let project = host
        .call("get_context", json!({"project":"reg"}), false)
        .await;
    let version = project
        .split("pending version ")
        .nth(1)
        .unwrap()
        .split(|c: char| !c.is_ascii_hexdigit())
        .next()
        .unwrap()
        .to_owned();
    let ack = host
        .call(
            "git_recovery",
            json!({"project":"reg","op":"release","intents":[intent],"version":version}),
            false,
        )
        .await;
    assert!(
        ack.starts_with("SAVED Git recovery\nGit recovery state: "),
        "{ack}"
    );
    assert!(
        ack.contains("Next: get_context with project only; omit ref"),
        "{ack}"
    );
    assert!(
        !ack.contains("ref=Git recovery") && !ack.contains("ref=Project"),
        "{ack}"
    );
    assert_eq!(commits(&root), before, "release never commits");
}

/// The whole-Project status and the project context expose the pending Git and compaction facts,
/// and every pending `PG-` identity is a pageable row under the pending-aware snapshot.
#[cfg(unix)]
#[tokio::test]
async fn project_reads_list_pending_intents_and_status_has_git_and_compaction() {
    use std::os::unix::fs::PermissionsExt;
    isolate_git();
    let host = Host::new();
    let root = registered(&host).await;
    module(&host, "First", false).await;
    let clean = host
        .call("project_status", json!({"project":"reg"}), false)
        .await;
    assert!(
        clean.contains("Git persistence: 0 pending intent(s)"),
        "{clean}"
    );
    assert!(clean.contains("Compaction proposals:"), "{clean}");
    let modules = root.join("modules");
    fs::set_permissions(&modules, fs::Permissions::from_mode(0o555)).unwrap();
    let failed = module(&host, "Second", true).await;
    fs::set_permissions(&modules, fs::Permissions::from_mode(0o755)).unwrap();
    let status = host
        .call("project_status", json!({"project":"reg"}), false)
        .await;
    assert!(
        status.contains("Git persistence: 1 pending intent(s)"),
        "{status}"
    );
    assert!(status.contains("pending version "), "{status}");
    let scoped = host
        .call(
            "project_status",
            json!({"project":"reg","module":"M-001"}),
            false,
        )
        .await;
    assert!(!scoped.contains("Git persistence"), "{scoped}");
    let page = host
        .call("get_context", json!({"project":"reg","limit":1}), false)
        .await;
    assert!(page.contains("pending Git intent, phase "), "{page}");
    assert!(failed.contains("PG-"), "{failed}");
    let snapshot = Host::field(&page, "Snapshot version: ");
    let next = host
        .call(
            "get_context",
            json!({"project":"reg","limit":1,"start":1,"version":snapshot}),
            false,
        )
        .await;
    assert!(next.contains("M-001"), "{next}");
}

/// A reply that cannot use its template keeps every Git line, attention flags included, and stays
/// within the reply limit however many effect lines there are.
#[test]
fn degraded_reply_keeps_git_attention_within_the_limit() {
    use crate::{response::Templates, tools::work::ToolKind};
    let templates = Templates::new(&super::templates()).unwrap();
    let effects: Vec<String> = (0..80)
        .map(|n| format!("Published modules/M-{n:03}.yaml {}", "x".repeat(300)))
        .collect();
    let git = vec![
        "Git: saved and pending; 2 path(s) not committed.".to_owned(),
        "Attention: [HookChangedWorktree].".to_owned(),
    ];
    let ack = super::work::ack("M-001", "v".repeat(64), "open", true);
    let text = super::work::render_ack(&templates, ToolKind::Work, &ack, &effects, &git);
    assert!(text.len() <= 8192, "{}", text.len());
    assert!(text.contains("Presentation degraded."), "{text}");
    assert!(text.contains("Attention: [HookChangedWorktree]."), "{text}");
    assert!(text.contains("effect line(s) omitted."), "{text}");
}
