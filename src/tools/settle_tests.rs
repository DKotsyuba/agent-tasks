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
    assert_eq!(git(&root, &["diff", "--cached", "--name-only"]), "foreign-staged.txt");
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
    assert!(reply.contains("Git: saved on disk; nothing was committed for this call."), "{reply}");
    assert!(!reply.contains("Git: committed"), "{reply}");
}
