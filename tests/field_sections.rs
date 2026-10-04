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
