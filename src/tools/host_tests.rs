//! Focused regressions for the shared producer host seam: the extended acknowledgement, the
//! field-naming validation helper and the sanitized serde failure text.
#![allow(clippy::unwrap_used, reason = "Test assertions")]
use super::{
    input::{self, Plan},
    work::{ack, render_ack},
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
        render_ack(&t, &plain, &["Published modules/M-001.yaml.".into()]),
        "SAVED M-001\nModule phase: working\nVersion: abc\nPublished modules/M-001.yaml.\nNext: get_context with ref=M-001. Use project_status for the complete tracked overview. Do not replay a lost reply blindly.\n"
    );
    let mut decision = ack("D-001", "def".into(), "current", true);
    decision.notes.push("Superseded D-000.".into());
    let text = render_ack(&t, &decision, &[]);
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
