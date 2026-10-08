//! Joint real-SDK proof of the accepted M-001 typed knowledge candidate with the submitted M-004 Git persistence
//! candidate (A-003). Positive M-004 approval is enforced separately by its own review, never by this file.
//! Boundaries: `kr-paths` r6, `persist-store-m001` r3 and the shared `producer-host` r3.
//! The third party of `producer-host` r3 is the M-003 shared host (common decode, dispatch and Ack, caller-held lock
//! and settlement scope). It is used, not certified: its files are pinned byte for byte at M-003 candidate
//! 6c08c65c12a5adc295d64f3d03250adb5839f5c9 so the joint cases run under exactly the agreed host.
//!
//! Every scenario starts the shipped binary through the real stdio SDK against a disposable independent documentation
//! repository with the production policy. Faults come only from repository hooks and repository configuration. The
//! claim is limited to M-001 and M-004; no other Module or Epic is certified here.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "Explicit protocol assertions"
)]
mod support;

use serde_json::json;
use support::{
    Project, commit_count, commit_message, commit_paths, git, head, install_hook, knowledge,
    pending, remove_hook, sha256_hex, staged, target,
};

/// Owned M-001 files with the sha256 of their accepted bytes at candidate e393d73d955598c344dafa31f56d6e2225efab16.
const M001_PINS: &[(&str, &str)] = &[
    (
        "src/knowledge.rs",
        "9d9ed03e87b821b9ef4b5081642be51c2c6ce67db956974f60f53903838da5a8",
    ),
    (
        "src/tools/knowledge_ops.rs",
        "dba7d99c7cba5594d7f9def7ccd4221ea636962b7c5d0a5ffd92de769553f821",
    ),
    (
        "docs/contracts/knowledge-records.md",
        "156f4c9d5a0beb307e2d9e1120ff556ce1a3c2d328ce03f915baea2a601ec0e8",
    ),
];

/// Owned M-004 files with the sha256 of their submitted bytes at candidate a4c0cbaf7ef88cd69a6dfc22eeabce28dfa79f10.
const M004_PINS: &[(&str, &str)] = &[
    (
        "src/persist/engine.rs",
        "869a1167791242464bd698628e54072831d3c5f1741bd2e74a4b6a6f4d2bfa45",
    ),
    (
        "src/persist/git.rs",
        "faea075e86aa7e17f2eeee463300410ee5a958f9950aa891320d9fbbe04eae53",
    ),
    (
        "src/persist/journal.rs",
        "71746ab6ac4f713271dfa32f8b314e7b7de896ac20ff9977ccd888018c38bb4a",
    ),
    (
        "src/persist/locator.rs",
        "7b9fb924fd05b820be7e830c4bb643df8ec894282b908279981595ecd0e3a0bd",
    ),
    (
        "src/persist/mod.rs",
        "836251c2f953a72589ddb02a7c3f3403f4d1c3872e9b34ac4514922a6cdb47bf",
    ),
    (
        "src/persist/policy.rs",
        "c57e1ebc6648a4b6bfe7a761d213ac5d27adf193ca015f97fa38bfb895c7a893",
    ),
    (
        "src/persist/proof.rs",
        "1fdeab319334b8df6aec77cc890f2492c5b3c4068f5e5422d004cbb9586b3526",
    ),
    (
        "src/persist/receipt.rs",
        "8a0a2a3a6e06e12d803d397831b2d511f39aaa9b1d24d841b1d1c47e167bf26f",
    ),
    (
        "src/persist/recover.rs",
        "7869932cd764b793dc30162a53070095567ef77cf26c7d3cb6629791f298f0a8",
    ),
    (
        "src/persist/status.rs",
        "cfb0db59c73ef3da7d389bcf0ae0e453a33baad682844433874852d6a253f29c",
    ),
    (
        "src/persist/tests.rs",
        "ce3cab82d71a307dab1a235103333de25bbe3de7dffdb9f8a64df28b1e4e2620",
    ),
    (
        "src/persist/verify.rs",
        "e24c735033db24b02dbe7cad2c37e9c08d3905e3289327a1c6cbcae63dcbbf89",
    ),
    (
        "src/tools/recovery_ops.rs",
        "332b08418d6d33fcbf265eef135514a22139827bc0dc85a6d8a489425f21bdea",
    ),
];

/// Shared M-003 host files used under `producer-host` r3 with the sha256 of their bytes at M-003 candidate
/// 6c08c65c12a5adc295d64f3d03250adb5839f5c9: the process entry, tool registry and dispatcher (`src/main.rs`,
/// `src/tools/mod.rs`), the common closed decode (`src/tools/input.rs`) and the `Ack` plus settlement path
/// (`src/tools/work.rs`). Each is unchanged in the current common source.
const M003_HOST_PINS: &[(&str, &str)] = &[
    (
        "src/main.rs",
        "fa0b0351d8073afbf79402961c7b21fff0408078aac4be1ccadcd6dcd0ec7bf9",
    ),
    (
        "src/tools/mod.rs",
        "f3cc82d42e22aee807d8a9dd560eac2c1c6126b71407a32456556cda9ffc0bee",
    ),
    (
        "src/tools/input.rs",
        "f83b63c8da690ec0152b85e0f83d494cf55331160ad50ed2ce2bc2e698882f27",
    ),
    (
        "src/tools/work.rs",
        "96cecc76035a49d04fd72940b3dd3005ef5a4c52f98ac90b80fdf98f8347ce19",
    ),
];

/// Exact provider assembly: the checkout under test holds the accepted M-001 bytes, the submitted M-004 bytes and the
/// M-003 shared host bytes used under `producer-host` r3, byte for byte. This asserts byte identity only, not that
/// M-004 is accepted or that M-003 is certified.
///
/// Fails on the first owned file whose sha256 differs from its pin, so a drifted or edited provider is never joined.
#[test]
fn provider_assembly_holds_the_submitted_provider_bytes() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    for (relative, expected) in M001_PINS.iter().chain(M004_PINS).chain(M003_HOST_PINS) {
        let bytes = std::fs::read(root.join(relative)).unwrap();
        assert_eq!(
            &sha256_hex(&bytes),
            expected,
            "{relative} is not the pinned provider byte"
        );
    }
}

/// The disposable repository's staged blob id of `path`, used to prove a foreign index entry is untouched.
fn staged_blob(root: &std::path::Path, path: &str) -> String {
    git(root, &["rev-parse", &format!(":{path}")])
        .trim()
        .to_owned()
}

/// Joint knowledge and Git flow: typed creates and an edit commit only their own paths once through the shared
/// allocator, while foreign staged, tracked-dirty and untracked files keep their exact state.
#[tokio::test]
async fn typed_knowledge_commits_only_owned_paths_and_preserves_foreign_state() {
    let project = Project::register().await;
    let root = &project.root;
    std::fs::write(root.join("foreign-tracked.txt"), "tracked v1\n").unwrap();
    git(root, &["add", "--", "foreign-tracked.txt"]);
    git(root, &["commit", "-q", "-m", "foreign tracked"]);
    std::fs::write(root.join("foreign-tracked.txt"), "tracked v2 dirty\n").unwrap();
    std::fs::write(root.join("foreign-staged.txt"), "staged by a human\n").unwrap();
    git(root, &["add", "--", "foreign-staged.txt"]);
    std::fs::write(root.join("foreign-untracked.txt"), "untracked\n").unwrap();
    let blob = staged_blob(root, "foreign-staged.txt");
    let status = git(root, &["status", "--porcelain"]);
    let base = commit_count(root);

    let decision = knowledge(
        &project,
        json!({"op":"create_decision","title":"Joint","question":"Which?","decision":"A","rationale":"Measured"}),
        false,
    )
    .await;
    assert!(decision.contains("Git: committed "), "{decision}");
    let id = target(&decision);
    assert_eq!(id, "D-001");
    assert_eq!(commit_count(root), base + 1, "one commit per create");
    assert_eq!(
        commit_paths(root, &head(root)),
        vec![
            ".agent-tasks/knowledge.yaml".to_owned(),
            "decisions/D-001.yaml".to_owned()
        ]
    );
    assert!(commit_message(root, &head(root)).contains("Agent-Tasks-Intent: PG-"));

    let checklist = knowledge(
        &project,
        json!({"op":"create_checklist","title":"Release","purpose":"Ship","items":["Build","Test"]}),
        false,
    )
    .await;
    assert_eq!(
        target(&checklist),
        "CL-001",
        "the shared allocator continues"
    );
    assert_eq!(commit_count(root), base + 2);
    assert_eq!(
        commit_paths(root, &head(root)),
        vec![
            ".agent-tasks/knowledge.yaml".to_owned(),
            "checklists/CL-001.yaml".to_owned()
        ]
    );

    let edited = knowledge(
        &project,
        json!({"op":"edit_decision","ref":id,"decision":"B"}),
        false,
    )
    .await;
    assert!(edited.contains("Git: committed "), "{edited}");
    assert_eq!(commit_count(root), base + 3);
    assert_eq!(
        commit_paths(root, &head(root)),
        vec!["decisions/D-001.yaml".to_owned()]
    );
    let after_edit = head(root);
    let unchanged = knowledge(
        &project,
        json!({"op":"edit_decision","ref":id,"decision":"B"}),
        false,
    )
    .await;
    assert!(unchanged.contains("UNCHANGED"), "{unchanged}");
    assert_eq!(
        (head(root), commit_count(root)),
        (after_edit, base + 3),
        "a no-op commits nothing"
    );

    assert_eq!(staged(root), vec!["foreign-staged.txt".to_owned()]);
    assert_eq!(staged_blob(root, "foreign-staged.txt"), blob);
    assert_eq!(
        std::fs::read(root.join("foreign-tracked.txt")).unwrap(),
        b"tracked v2 dirty\n"
    );
    assert_eq!(
        std::fs::read(root.join("foreign-untracked.txt")).unwrap(),
        b"untracked\n"
    );
    assert_eq!(
        git(root, &["status", "--porcelain"]),
        status,
        "foreign state is byte-for-byte as left"
    );
}

/// Deferred persistence and recovery: a rejecting hook and a failing signing setup keep the saved typed records, a
/// later explicit `retry` commits exactly their recorded paths once, and nothing is replayed or committed twice.
#[tokio::test]
async fn deferred_hook_and_signing_failures_recover_exactly_once_without_replay() {
    let project = Project::register().await;
    let root = &project.root;
    std::fs::write(root.join("foreign-staged.txt"), "staged by a human\n").unwrap();
    git(root, &["add", "--", "foreign-staged.txt"]);
    std::fs::write(root.join("foreign-untracked.txt"), "untracked\n").unwrap();
    let blob = staged_blob(root, "foreign-staged.txt");
    let base = commit_count(root);

    install_hook(root, "pre-commit", "echo rejected >&2; exit 1");
    let hooked = knowledge(
        &project,
        json!({"op":"create_decision","title":"Hooked","question":"q","decision":"d","rationale":"r"}),
        false,
    )
    .await;
    assert!(hooked.contains("Git: saved and pending"), "{hooked}");
    remove_hook(root, "pre-commit");
    git(root, &["config", "commit.gpgsign", "true"]);
    git(root, &["config", "gpg.format", "ssh"]);
    git(
        root,
        &[
            "config",
            "user.signingkey",
            "/nonexistent/integration-key.pub",
        ],
    );
    let signed = knowledge(
        &project,
        json!({"op":"create_checklist","title":"Signed","purpose":"p","items":["one"]}),
        false,
    )
    .await;
    assert!(signed.contains("Git: saved and pending"), "{signed}");
    git(root, &["config", "--unset", "commit.gpgsign"]);
    git(root, &["config", "--unset", "gpg.format"]);
    git(root, &["config", "--unset", "user.signingkey"]);

    assert_eq!(
        commit_count(root),
        base,
        "nothing committed while Git was blocked"
    );
    let records = [
        "decisions/D-001.yaml",
        "checklists/CL-001.yaml",
        ".agent-tasks/knowledge.yaml",
    ];
    let saved: Vec<Vec<u8>> = records
        .iter()
        .map(|p| std::fs::read(root.join(p)).unwrap())
        .collect();
    let held = pending(&project).await;
    assert_eq!(
        held.intents.len(),
        2,
        "both deferred successes are pending: {held:?}"
    );

    let recovered = project
        .call(
            "git_recovery",
            json!({"op":"retry","intents":held.intents,"version":held.version,"actor":"integration-agent"}),
            false,
        )
        .await;
    assert!(recovered.contains("Git recovery"), "{recovered}");
    assert_eq!(
        commit_count(root),
        base + 1,
        "recovery settles both intents in exactly one commit"
    );
    assert_eq!(
        commit_message(root, &head(root))
            .matches("Agent-Tasks-Intent: PG-")
            .count(),
        2,
        "one trailer block per recovered intent"
    );
    assert_eq!(
        commit_paths(root, &head(root)),
        vec![
            ".agent-tasks/knowledge.yaml".to_owned(),
            "checklists/CL-001.yaml".to_owned(),
            "decisions/D-001.yaml".to_owned()
        ],
        "recovery commits only the recorded owned paths"
    );
    for (path, bytes) in records.iter().zip(&saved) {
        assert_eq!(
            &std::fs::read(root.join(path)).unwrap(),
            bytes,
            "{path} disk bytes are not replayed"
        );
        assert_eq!(
            git(root, &["show", &format!("HEAD:{path}")]).as_bytes(),
            bytes.as_slice(),
            "{path} committed bytes"
        );
    }
    assert_eq!(staged(root), vec!["foreign-staged.txt".to_owned()]);
    assert_eq!(staged_blob(root, "foreign-staged.txt"), blob);
    assert_eq!(
        std::fs::read(root.join("foreign-untracked.txt")).unwrap(),
        b"untracked\n"
    );
    let settled = pending(&project).await;
    assert!(
        settled.intents.is_empty(),
        "nothing remains pending: {settled:?}"
    );

    let (count, tip) = (commit_count(root), head(root));
    let again = project
        .call(
            "git_recovery",
            json!({"op":"retry","intents":held.intents,"version":settled.version,"actor":"integration-agent"}),
            true,
        )
        .await;
    assert!(!again.is_empty());
    assert_eq!(
        (commit_count(root), head(root)),
        (count, tip.clone()),
        "a repeated recovery adds no commit"
    );
    let noop = knowledge(
        &project,
        json!({"op":"edit_decision","ref":"D-001","decision":"d"}),
        false,
    )
    .await;
    assert!(noop.contains("UNCHANGED"), "{noop}");
    assert_eq!(
        (commit_count(root), head(root)),
        (count, tip),
        "a no-op edit adds no duplicate commit"
    );

    let next = knowledge(
        &project,
        json!({"op":"create_decision","title":"After","question":"q","decision":"d","rationale":"r"}),
        false,
    )
    .await;
    assert_eq!(
        target(&next),
        "D-002",
        "the recovered allocator was neither rewound nor replayed"
    );
    assert_eq!(commit_count(root), count + 1);
    assert_eq!(
        commit_paths(root, &head(root)),
        vec![
            ".agent-tasks/knowledge.yaml".to_owned(),
            "decisions/D-002.yaml".to_owned()
        ]
    );
}
