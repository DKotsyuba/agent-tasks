//! Structural validation of the three canonical source skills, in Rust and with no new dependency.
//!
//! The official validator needs a YAML package this repository does not use, so this test checks what that validator
//! checks for these skills: a delimited frontmatter with a `name` equal to the directory and a quoted `description`
//! within its limit, a non-empty body, no unfinished scaffold marker, working relative links, and the invariants the
//! skills must keep (the registration gate, the tracked-report boundary and honest capability wording). It checks
//! structure and named invariants only; it does not judge whether a skill makes good decisions.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "Explicit assertions"
)]
use std::path::{Path, PathBuf};

/// The canonical skills owned by this component.
const SKILLS: [&str; 3] = [
    "agent-tasks",
    "agent-tasks-orchestrator",
    "agent-tasks-module-lead",
];

/// Directory holding one skill.
fn skill_dir(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("skills")
        .join(name)
}

/// Read a skill file and split it into `(name, description, body)`, asserting the frontmatter shape.
fn parse(name: &str) -> (String, String, String) {
    let text = std::fs::read_to_string(skill_dir(name).join("SKILL.md")).unwrap();
    let rest = text
        .strip_prefix("---\n")
        .unwrap_or_else(|| panic!("{name}: frontmatter must open the file"));
    let (front, body) = rest
        .split_once("\n---\n")
        .unwrap_or_else(|| panic!("{name}: frontmatter must close"));
    let mut found_name = String::new();
    let mut found_description = String::new();
    for line in front.lines() {
        if let Some(value) = line.strip_prefix("name: ") {
            found_name = value.trim().to_owned();
        } else if let Some(value) = line.strip_prefix("description: ") {
            found_description = value.trim().to_owned();
        } else {
            panic!("{name}: unexpected frontmatter line {line:?}");
        }
    }
    (found_name, found_description, body.to_owned())
}

/// Every skill has a matching name, a bounded quoted description, a non-empty titled body and working links.
#[test]
fn every_skill_has_valid_frontmatter_and_links() {
    for name in SKILLS {
        let (found, description, body) = parse(name);
        assert_eq!(found, name, "the name matches its directory");
        assert!(
            found.len() < 64
                && found
                    .chars()
                    .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
        );
        assert!(
            description.starts_with('"') && description.ends_with('"'),
            "{name}: the description is one quoted string"
        );
        assert!(
            description.len() <= 1026,
            "{name}: description is {} bytes",
            description.len()
        );
        assert!(
            body.trim_start().starts_with("# "),
            "{name}: the body starts with a title"
        );
        for marker in ["TODO:", "[TODO", "FIXME", "<placeholder>"] {
            assert!(
                !body.contains(marker),
                "{name}: unfinished scaffold marker {marker}"
            );
        }
        for (position, _) in body.match_indices("](") {
            let target = body[position + 2..].split(')').next().unwrap();
            if target.starts_with("http") || target.starts_with('#') || target.is_empty() {
                continue;
            }
            let path = target.split('#').next().unwrap();
            assert!(
                skill_dir(name).join(path).exists(),
                "{name}: broken link {target}"
            );
        }
    }
}

/// The invariants of the three skills that later edits must not erase.
#[test]
fn skills_keep_their_named_invariants() {
    let root = parse("agent-tasks").2;
    assert!(
        root.contains("get_project_list"),
        "the registration gate starts with the project list"
    );
    assert!(
        root.contains("owner's documentation location")
            || root.contains("owner chooses the location")
    );
    assert!(
        root.contains("tracked work report is not a saved Markdown document"),
        "reports are not Markdown"
    );
    assert!(
        root.contains("live catalog"),
        "capabilities are conditional on the live catalog"
    );
    assert!(
        !root.contains("does not provide a general Markdown reader"),
        "the obsolete no-document-tools claim is gone"
    );
    let orchestrator = parse("agent-tasks-orchestrator").2;
    for needle in [
        "bind_agent",
        "freeze_epic",
        "agree_contract",
        "boundary_evidence",
        "verify_criterion",
        "git_recovery",
        "compaction_work",
        "knowledge_work",
        "document_work",
    ] {
        assert!(
            orchestrator.contains(needle),
            "orchestrator keeps or gains {needle}"
        );
    }
    assert!(
        orchestrator.contains("only when the live catalog"),
        "no operational claim without the live catalog"
    );
    let lead = parse("agent-tasks-module-lead").2;
    for needle in [
        "agree_contract",
        "boundary_evidence",
        "import_commits",
        "git_recovery",
        "op=planning",
    ] {
        assert!(lead.contains(needle), "lead keeps or gains {needle}");
    }
    assert!(lead.contains("Do not review your own work"));
}
