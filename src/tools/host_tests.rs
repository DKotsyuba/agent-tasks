//! Focused regressions for the shared producer host seam: the extended acknowledgement, the
//! field-naming validation helper and the sanitized serde failure text.
#![allow(clippy::unwrap_used, reason = "Test assertions")]
use super::{
    input::{self, Plan},
    work::{ToolKind, ack, is_canonical_ref, render_ack},
};
use crate::response::Templates;
use serde_json::json;

/// Registered templates exactly as the server composes them.
fn templates() -> Templates {
    Templates::new(&super::templates()).unwrap()
}

/// Existing work targets keep their historical phase wording and the producer kinds get a state
/// noun, so a reply label never depends on the producer.
#[test]
fn ack_labels_name_the_record_kind() {
    for (target, label) in [
        ("E-001", "Epic phase"),
        ("A-001", "Atomic phase"),
        ("M-001", "Module phase"),
        ("M-001/T-001", "Module phase"),
        ("D-001", "Decision state"),
        ("RB-001", "Runbook state"),
        ("RS-001", "Research state"),
        ("CL-001", "Checklist state"),
        ("CL-001/I-001", "Checklist state"),
        ("DOC-001", "Document state"),
        ("README.md", "Document state"),
        ("docs/guide/a.md", "Document state"),
        ("CP-001", "Compaction state"),
    ] {
        assert_eq!(ack(target, "v".into(), "x", true).phase_label, label);
    }
}

/// A rendered work acknowledgement is unchanged for existing targets and prints producer notes
/// after the effect ledger.
#[test]
fn ack_renders_existing_text_and_notes() {
    let t = templates();
    let plain = ack("M-001", "abc".into(), "working", true);
    assert_eq!(
        render_ack(
            &t,
            ToolKind::Work,
            &plain,
            &["Published modules/M-001.yaml.".into()],
            &[],
        ),
        "SAVED M-001\nModule phase: working\nVersion: abc\nPublished modules/M-001.yaml.\nNext: get_context with ref=M-001. Use project_status for the complete tracked overview. Do not replay a lost reply blindly.\n"
    );
    let mut decision = ack("D-001", "def".into(), "current", true);
    decision.notes.push("Superseded D-000.".into());
    let text = render_ack(&t, ToolKind::Knowledge, &decision, &[], &[]);
    assert!(
        text.starts_with("SAVED D-001\nDecision state: current\nVersion: def\nSuperseded D-000.\n")
    );
    assert!(text.contains("Next: get_context with ref=D-001."));
}

/// A rejected bounded field names the field and its rule, never the supplied value.
#[test]
fn field_names_the_field_without_the_value() {
    let ok: crate::store::Result<()> = input::field("read_refs", Ok(()));
    assert!(ok.is_ok());
    let values: Vec<String> = (0..10).map(|i| format!("secret-value-{i}")).collect();
    let error = input::field("read_refs", crate::model::strings(&values, 256, false)).unwrap_err();
    assert_eq!(error.code, "invalid_arguments");
    assert!(error.message.starts_with("read_refs: "));
    assert!(error.message.contains("eight"));
    assert!(!error.message.contains("secret-value"));
}

/// Serde decoding failures name the unknown or missing field and never echo values.
#[test]
fn shape_errors_name_fields_not_values() {
    let unknown = input::mutation::<Plan>(
        json!({"project":"p","version":"v","op":"edit_project","bogus":"hunter2"}),
        false,
    )
    .err()
    .unwrap();
    assert!(unknown.contains("\"bogus\""), "{unknown}");
    assert!(!unknown.contains("hunter2"));
    let missing = input::mutation::<Plan>(
        json!({"project":"p","version":"v","op":"create_module"}),
        false,
    )
    .err()
    .unwrap();
    assert!(missing.contains("Missing required field"), "{missing}");
    let variant = input::mutation::<Plan>(json!({"project":"p","version":"v","op":"nope"}), false)
        .err()
        .unwrap();
    assert!(variant.contains("Unknown operation"), "{variant}");
    let shape = input::mutation::<Plan>(
        json!({"project":"p","version":"v","op":"create_epic","title":["hunter2"],"outcome":"o","criteria":["c"]}),
        false,
    )
    .err()
    .unwrap();
    assert!(shape.contains("Invalid argument shape"), "{shape}");
    assert!(!shape.contains("hunter2"));
}

/// Producer notes and refs are bounded and sanitized at presentation, with an explicit omitted
/// count and no forged lines.
#[test]
fn notes_and_refs_are_bounded_and_sanitized() {
    let t = templates();
    let mut receipt = ack("D-001", "v".into(), "current", true);
    receipt.notes = (0..11).map(|i| format!("note {i}")).collect();
    receipt.notes[0] = format!("first\nSAVED forged\r\n{}", "x".repeat(400));
    receipt.refs = vec!["D-001".into(); 20];
    receipt.refs.push("x".repeat(257));
    let text = render_ack(&t, ToolKind::Knowledge, &receipt, &[], &[]);
    assert!(
        text.contains("3 notes omitted; get_context with ref=D-001."),
        "{text}"
    );
    assert!(!text.contains("note 8"), "{text}");
    assert!(!text.contains("\nSAVED forged"), "{text}");
    assert!(
        text.lines()
            .all(|line| line.len() <= 256 || line.starts_with("Next:")),
        "{text}"
    );
    assert!(
        text.lines()
            .any(|line| line.starts_with("first SAVED forged "))
    );
}

/// Only the known recovery tool kind uses the recovery layout, whose route omits the ref; any
/// other tool with a non-canonical target gets no invented ref route.
#[test]
fn recovery_layout_comes_from_the_tool_kind() {
    let t = templates();
    let mut recovery = ack("Git recovery", "p".repeat(64), "released", true);
    recovery.notes.push("Released 1 intent.".into());
    let text = render_ack(
        &t,
        ToolKind::GitRecovery,
        &recovery,
        &[],
        &["Git: saved.".into()],
    );
    assert!(
        text.starts_with("SAVED Git recovery\nGit recovery state: released\n"),
        "{text}"
    );
    assert!(text.contains("Released 1 intent.\nGit: saved.\n"), "{text}");
    assert!(text.contains(
        "Next: get_context with project only; omit ref, to read the pending version and facts."
    ));
    assert!(!text.contains("ref=Git recovery") && !text.contains("ref=Project"));
    let other = render_ack(&t, ToolKind::Work, &recovery, &[], &[]);
    assert!(
        other.contains("the target is not a canonical reference"),
        "{other}"
    );
    assert!(!other.contains("ref=Git recovery"), "{other}");
}

/// Canonical reference recognition accepts real references and refuses labels and paths that
/// escape the managed grammar.
#[test]
fn canonical_references_are_recognized_exactly() {
    for good in [
        "E-001",
        "M-001/T-001",
        "A-001",
        "D-001",
        "RB-001",
        "RS-1000",
        "CL-001/I-001",
        "DOC-001",
        "CP-001",
        "README.md",
        "docs/a/b.md",
    ] {
        assert!(is_canonical_ref(good), "{good}");
    }
    for bad in [
        "Git recovery",
        "Project",
        "D-1",
        "M-001/X-001",
        "docs/../a.md",
        "docs/a.txt",
        "/docs/a.md",
        "docs/é.md",
        "d-001",
        "",
    ] {
        assert!(!is_canonical_ref(bad), "{bad}");
    }
}
