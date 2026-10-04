//! Managed description fields retain literal headings inside CommonMark code.
use agent_tasks::records::{patch_description, read_fields};
use serde_json::json;

/// Fenced and indented examples remain literal while genuine sections are patched.
#[test]
fn fields_preserve_code_and_unknown_prose() {
    let original = "Intro\n\n## Описание\nText\n\n```md\n## Результат\ncode\n```\n\n    ## Результат\n    indented\n\n## Unknown\nKeep exact\n\n## Результат\nold\n";
    let fields = read_fields(original).unwrap();
    assert!(
        fields["description"]
            .as_str()
            .unwrap()
            .contains("## Результат\ncode")
    );
    assert_eq!(fields["result"], "old");
    let patched = patch_description(original, &json!({"result":"new"})).unwrap();
    assert!(patched.starts_with(original.split("## Результат\nold").next().unwrap()));
    assert_eq!(read_fields(&patched).unwrap()["result"], "new");
    let removed = patch_description(&patched, &json!({"result":null})).unwrap();
    assert!(removed.contains("## Результат\ncode"));
    assert!(removed.contains("    ## Результат\n    indented"));
    assert!(read_fields(&removed).unwrap().get("result").is_none());
}

/// Duplicate real headings, including alternate ATX/setext syntax, fail before mutation.
#[test]
fn duplicate_real_fields_are_rejected() {
    let duplicate = "## Результат\nfirst\n\nРезультат\n----------\nsecond\n";
    assert!(read_fields(duplicate).is_err());
    assert!(patch_description(duplicate, &json!({"result":"replace"})).is_err());
}

/// Producers reject H1 escapes and fences that swallow a generated sibling before metadata can diverge.
#[test]
fn generated_fields_must_keep_their_boundaries() {
    for body in [
        "# Heading\noutput",
        "intro\n# Heading\noutput",
        "Heading\n=======\noutput",
    ] {
        assert!(patch_description("", &json!({"result":body})).is_err());
    }
    assert!(
        patch_description(
            "",
            &json!({"description":"```text\nexample","work_type":"code"})
        )
        .is_err()
    );
    let original = "## Описание\nSafe\n\n## Вид работы\ncode\n";
    assert!(patch_description(original, &json!({"description":"```text\nexample"})).is_err());
    let valid = patch_description(
        "",
        &json!({"description":"```text\n# Heading\nexample\n```","work_type":"code"}),
    )
    .unwrap();
    assert_eq!(read_fields(&valid).unwrap()["work_type"], "code");
}

/// Real indented ATX/setext headings nest while code and inline destinations remain exact.
#[test]
fn report_heading_normalization_uses_commonmark() {
    let source = "# One\n\n  ## [Link](https://example.test/report)\n\nРезультат\n----------\nbody\n\n```md\n## Результат\nCode\n----------\n```\n\n    ## literal\n";
    let nested = agent_tasks::sections::nest_field_headings(source);
    assert!(!agent_tasks::sections::has_field_boundary(&nested));
    assert!(nested.contains("### One"));
    assert!(nested.contains("### [Link](https://example.test/report)"));
    assert!(nested.contains("### Результат\nbody"));
    assert!(nested.contains("```md\n## Результат\nCode\n----------\n```"));
    assert!(nested.contains("    ## literal"));
}
