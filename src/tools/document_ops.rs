//! Producer seam of the document tool: the closed operation payload, the tool description and the
//! locked handler. Registration, routing, rendering and Git settlement belong to the tool surface.
//!
//! The handler runs under the caller's single write lock, maps each domain receipt to the shared
//! acknowledgement and never renders text, calls Git or locks again.
use super::{
    input::{self, Common},
    work::{Ack, ack},
};
use crate::{
    documents::{self, DocPath, Edit, Port, Ref, Save, Scope, StorePort},
    markdown::{self, Selector, Wire},
    store::{Error, LockGuard, Result, Store},
};
use schemars::JsonSchema;
use serde::Deserialize;

/// Catalog description of the document tool: purpose, operations, versions and recovery, in plain
/// bounded prose.
pub const DESCRIPTION: &str = "Purpose: Save, edit, adopt, remove and move managed Markdown documents under docs/ and the root README.md with guarded versions. Read them with get_context using a path or DOC id.\nInput: project, version and a closed op. version is the Version that get_context prints for the document; an absent path has one too, so creating needs the absent version. save: ref (docs path or DOC id), purpose (one line, required when a record is created), body (the exact new document). replace_section: ref, section (exactly one of preamble true, ordinal, or heading with optional level and occurrence), body (the new section body; the heading line is kept). adopt: ref, purpose; records the current bytes without rewriting them. remove: ref; removes the file and retires its record. relocate: ref, to (absent docs path), to_version (the absent version of to). body is a JSON string; set wire to escaped to send a backslash, carriage return or control character as the escapes backslash backslash, backslash r and backslash u with braces.\nEffects: only managed files and DOC records change; untouched bytes, BOM and line endings are preserved; a stale version saves nothing. A partial failure reports what was saved and the one recovery step; it never rolls back or retries. Names are ASCII, at most three folders deep.";

/// How the `body` string encodes document bytes.
#[derive(Deserialize, JsonSchema, Default, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum BodyWire {
    /// The string is the document text as is.
    #[default]
    Raw,
    /// The string uses the `md-text-v1` escapes for backslash, carriage return and control characters.
    Escaped,
}

/// Which section of a document an edit addresses: exactly one of the three ways.
#[derive(Deserialize, JsonSchema, Clone, Debug, Default)]
#[serde(deny_unknown_fields)]
pub struct SectionArg {
    /// True to address the bytes before the first heading.
    pub preamble: Option<bool>,
    /// Zero-based heading ordinal in document order.
    pub ordinal: Option<usize>,
    /// Exact heading text.
    pub heading: Option<String>,
    /// Heading level to match, 1 to 6, with `heading`.
    pub level: Option<u8>,
    /// One-based occurrence among identical heading text, with `heading`.
    pub occurrence: Option<usize>,
}

/// Closed operations of the document tool.
#[derive(Deserialize, JsonSchema, Debug)]
#[serde(tag = "op", rename_all = "snake_case", deny_unknown_fields)]
pub enum DocumentOp {
    /// Create or replace the whole body.
    Save {
        /// Docs path or DOC identifier.
        #[serde(rename = "ref")]
        target: String,
        /// One-line purpose; required when a record is created.
        purpose: Option<String>,
        /// The exact new document text.
        body: String,
        /// Encoding of `body`.
        #[serde(default)]
        wire: BodyWire,
    },
    /// Replace the body of one section or the preamble; the heading line is kept.
    ReplaceSection {
        /// Docs path or DOC identifier.
        #[serde(rename = "ref")]
        target: String,
        /// The section to replace.
        section: SectionArg,
        /// The exact new section body.
        body: String,
        /// Encoding of `body`.
        #[serde(default)]
        wire: BodyWire,
    },
    /// Record the current bytes without rewriting them.
    Adopt {
        /// Docs path or DOC identifier.
        #[serde(rename = "ref")]
        target: String,
        /// One-line purpose; required when a record is created.
        purpose: Option<String>,
    },
    /// Remove the file and retire its record.
    Remove {
        /// Docs path or DOC identifier.
        #[serde(rename = "ref")]
        target: String,
    },
    /// Move the document to an absent path, keeping its identity.
    Relocate {
        /// Docs path or DOC identifier of the source.
        #[serde(rename = "ref")]
        target: String,
        /// The absent destination path.
        to: String,
        /// The absent version of `to`.
        to_version: String,
    },
}

/// Decode a `body` string to the exact bytes it stands for.
fn body_bytes(body: &str, wire: BodyWire) -> Result<Vec<u8>> {
    let bytes = match wire {
        BodyWire::Raw => body.as_bytes().to_vec(),
        BodyWire::Escaped => markdown::decode(Wire::Escaped, body)
            .map_err(|e| Error::new("invalid_arguments", format!("body: {}", e.message)))?,
    };
    if bytes.len() > documents::BODY_CAP {
        return Err(Error::new(
            "invalid_arguments",
            "body: exceeds 524288 bytes.",
        ));
    }
    Ok(bytes)
}

/// Parse `ref`, naming the field on failure.
fn target_ref(raw: &str) -> Result<Ref> {
    input::field("ref", Ref::parse(raw).map_err(|e| e.message))
}

/// Selector from the loose section arguments, naming the offending part.
fn selector(section: &SectionArg) -> Result<Selector> {
    Selector::from_parts(
        section.preamble.unwrap_or(false),
        section.ordinal,
        section.heading.clone(),
        section.level,
        section.occurrence,
    )
}

/// Map a domain receipt to the shared acknowledgement. `extra` paths (a move source) join the refs.
fn acknowledge(receipt: &documents::Receipt, extra: &[&DocPath]) -> Ack {
    let target = receipt
        .id
        .clone()
        .unwrap_or_else(|| receipt.path.as_str().to_owned());
    let mut a = ack(
        target.clone(),
        receipt.version_after.clone(),
        receipt.state_after.label(),
        receipt.changed,
    );
    let mut refs: Vec<String> = Vec::new();
    refs.extend(receipt.id.clone());
    refs.push(receipt.path.as_str().to_owned());
    refs.extend(extra.iter().map(|p| p.as_str().to_owned()));
    refs.dedup();
    a.refs = refs;
    let mut notes: Vec<String> = Vec::new();
    for w in &receipt.warnings {
        notes.push(
            match w {
                documents::Warning::MixedLineEndings => {
                    "Warning: the result mixes LF and CRLF line endings."
                }
                documents::Warning::SetextIgnored => {
                    "Warning: setext underlines are not headings in this dialect."
                }
                documents::Warning::AlreadyApplied => {
                    "The interrupted move had already completed; nothing was published."
                }
            }
            .into(),
        );
    }
    match &receipt.references {
        crate::references::ReferenceCheck::Skipped => {}
        crate::references::ReferenceCheck::Unknown(why) => {
            notes.push(format!("References were not checked ({why})."));
        }
        crate::references::ReferenceCheck::Checked {
            incoming,
            introduced_dangling,
            complete,
        } => {
            notes.push(format!(
                "References: {incoming} aimed at the changed paths; {} left dangling{}.",
                introduced_dangling.len(),
                if *complete {
                    ""
                } else {
                    " (coverage incomplete)"
                }
            ));
            for d in introduced_dangling.iter().take(4) {
                notes.push(format!(
                    "Dangling: {} -> {}",
                    d.source.id_or_path,
                    d.target.canonical()
                ));
            }
            if introduced_dangling.len() > 4 {
                notes.push(format!(
                    "{} more dangling references omitted.",
                    introduced_dangling.len() - 4
                ));
            }
        }
    }
    if let Some(r) = receipt.revision {
        notes.push(format!("Revision {r}."));
    }
    if notes.len() > 8 {
        let omitted = notes.len() - 7;
        notes.truncate(7);
        notes.push(format!(
            "{omitted} more lines omitted; get_context ref={target} view=references."
        ));
    }
    for n in &mut notes {
        n.truncate(200);
    }
    a.notes = notes;
    a
}

/// Run one operation against a port; the testable core of [`execute_locked`].
pub fn run(port: &dyn Port, common: &Common, op: DocumentOp) -> Result<Ack> {
    let mut scope = Scope {
        port,
        operation: None,
    };
    let actor = common.actor.as_deref();
    match op {
        DocumentOp::Save {
            target,
            purpose,
            body,
            wire,
        } => {
            let r = target_ref(&target)?;
            let bytes = body_bytes(&body, wire)?;
            let receipt = documents::save(
                &mut scope,
                Save {
                    target: &r,
                    purpose: purpose.as_deref(),
                    edit: Edit::Body(&bytes),
                    expected: &common.version,
                    actor,
                },
            )?;
            Ok(acknowledge(&receipt, &[]))
        }
        DocumentOp::ReplaceSection {
            target,
            section,
            body,
            wire,
        } => {
            let r = target_ref(&target)?;
            let sel = selector(&section)?;
            let bytes = body_bytes(&body, wire)?;
            let receipt = documents::save(
                &mut scope,
                Save {
                    target: &r,
                    purpose: None,
                    edit: Edit::Section {
                        selector: &sel,
                        body: &bytes,
                    },
                    expected: &common.version,
                    actor,
                },
            )?;
            Ok(acknowledge(&receipt, &[]))
        }
        DocumentOp::Adopt { target, purpose } => {
            let r = target_ref(&target)?;
            let receipt =
                documents::adopt(&mut scope, &r, purpose.as_deref(), &common.version, actor)?;
            Ok(acknowledge(&receipt, &[]))
        }
        DocumentOp::Remove { target } => {
            let r = target_ref(&target)?;
            let receipt = documents::remove(&mut scope, &r, &common.version, actor)?;
            Ok(acknowledge(&receipt, &[]))
        }
        DocumentOp::Relocate {
            target,
            to,
            to_version,
        } => {
            let from = target_ref(&target)?;
            let dest = input::field("to", DocPath::parse(&to).map_err(|e| e.message))?;
            let obs = documents::observe(port, &from)?;
            if obs.version != common.version {
                return Err(Error::new(
                    "stale",
                    format!(
                        "No work saved. Current version: {}. Read the document before retrying.",
                        obs.version
                    ),
                ));
            }
            let basis = obs.move_basis()?;
            let receipt =
                documents::relocate(&mut scope, &from, &dest, &basis, &to_version, actor)?;
            Ok(acknowledge(&receipt, &[&obs.path]))
        }
    }
}

/// Handler. The caller already holds the write lock for `store`; never lock here. Human effect
/// strings are appended to `effects` even when the operation fails after publishing.
pub fn execute_locked(
    store: &Store,
    _guard: &LockGuard,
    common: &Common,
    op: DocumentOp,
    effects: &mut Vec<String>,
) -> Result<Ack> {
    let port = StorePort::new(store);
    let result = run(&port, common, op);
    effects.extend(port.fx.borrow_mut().drain(..));
    result
}

/// Behavior of the document tool payload and its acknowledgement mapping, over the scripted
/// storage double.
#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, reason = "Test assertions")]
    use super::*;
    use crate::documents::{
        Ref, observe,
        tests::{Fake, native, root},
    };
    use serde_json::json;

    /// Common arguments with the given observation version.
    fn common(version: &str) -> Common {
        Common {
            project: "p".into(),
            version: version.into(),
            actor: Some("tester".into()),
            reference: None,
        }
    }

    /// The version `get_context` would print for a path.
    fn version_of(port: &dyn Port, path: &str) -> String {
        observe(port, &Ref::Path(DocPath::parse(path).unwrap()))
            .unwrap()
            .version
    }

    /// Decode one operation from JSON as the registry would.
    fn op(value: serde_json::Value) -> DocumentOp {
        serde_json::from_value(value).unwrap()
    }

    /// The payload is closed: unknown fields and variants are refused.
    #[test]
    fn payload_is_closed() {
        let bad = |v: serde_json::Value| serde_json::from_value::<DocumentOp>(v).is_err();
        assert!(bad(
            json!({"op":"save","ref":"docs/a.md","body":"x","extra":1})
        ));
        assert!(bad(json!({"op":"save","ref":"docs/a.md"})));
        assert!(bad(json!({"op":"merge","ref":"docs/a.md"})));
        assert!(bad(
            json!({"op":"save","ref":"docs/a.md","body":"x","wire":"hex"})
        ));
        assert!(!bad(
            json!({"op":"relocate","ref":"docs/a.md","to":"docs/b.md","to_version":"v"})
        ));
    }

    /// A save maps its receipt to a document acknowledgement with identity, state and refs.
    #[test]
    fn save_acknowledges_identity_state_and_refs() {
        let (dir, st) = root();
        let f = Fake::new(&st);
        let v = version_of(&f, "docs/a.md");
        let a = run(
            &f,
            &common(&v),
            op(json!({"op":"save","ref":"docs/a.md","purpose":"First","body":"# A\r\n"})),
        )
        .unwrap();
        assert_eq!(
            (
                a.target.as_str(),
                a.phase.as_str(),
                a.phase_label,
                a.changed
            ),
            ("DOC-001", "managed", "Document state", true)
        );
        assert!(
            a.refs.contains(&"DOC-001".to_string()) && a.refs.contains(&"docs/a.md".to_string())
        );
        assert_eq!(a.version, version_of(&f, "docs/a.md"));
        assert_eq!(
            std::fs::read(dir.path().join("docs/a.md")).unwrap(),
            b"# A\r\n"
        );
        // The same version is stale after the save.
        let e = run(
            &f,
            &common(&v),
            op(json!({"op":"save","ref":"docs/a.md","body":"y"})),
        )
        .err()
        .unwrap();
        assert_eq!(e.code, "stale");
    }

    /// An escaped body reaches the file as the exact bytes it stands for; a bad escape names `body`.
    #[test]
    fn escaped_wire_decodes_exactly() {
        let (dir, st) = root();
        let f = Fake::new(&st);
        let v = version_of(&f, "docs/e.md");
        run(&f, &common(&v), op(json!({"op":"save","ref":"docs/e.md","purpose":"Escapes","wire":"escaped","body":"a\\\\b\\r\nc\\u{7f}"}))).unwrap();
        assert_eq!(
            std::fs::read(dir.path().join("docs/e.md")).unwrap(),
            b"a\\b\r\nc\x7f"
        );
        let v = version_of(&f, "docs/e2.md");
        let e = run(
            &f,
            &common(&v),
            op(json!({"op":"save","ref":"docs/e2.md","purpose":"x","wire":"escaped","body":"\\q"})),
        )
        .err()
        .unwrap();
        assert_eq!(
            (e.code, e.message.starts_with("body:")),
            ("invalid_arguments", true)
        );
    }

    /// Section replace needs exactly one selector part and keeps the other bytes.
    #[test]
    fn replace_section_selector_rules() {
        let (dir, st) = root();
        let f = Fake::new(&st);
        native(&dir, "docs/s.md", b"# A\nold\n# B\nend\n");
        let v = version_of(&f, "docs/s.md");
        let both = op(
            json!({"op":"replace_section","ref":"docs/s.md","section":{"preamble":true,"ordinal":0},"body":"x"}),
        );
        let e = run(&f, &common(&v), both).err().unwrap();
        assert!(e.code == "invalid_arguments" && e.message.starts_with("preamble:"));
        let adopt = op(json!({"op":"adopt","ref":"docs/s.md","purpose":"Sections"}));
        run(&f, &common(&v), adopt).unwrap();
        let v = version_of(&f, "docs/s.md");
        let edit = op(
            json!({"op":"replace_section","ref":"docs/s.md","section":{"heading":"A"},"body":"new\n"}),
        );
        let a = run(&f, &common(&v), edit).unwrap();
        assert!(a.changed);
        assert_eq!(
            std::fs::read(dir.path().join("docs/s.md")).unwrap(),
            b"# A\nnew\n# B\nend\n"
        );
    }

    /// Relocate checks the source version, names `to` on a bad path and keeps the identity.
    #[test]
    fn relocate_through_the_tool() {
        let (dir, st) = root();
        let f = Fake::new(&st);
        let v = version_of(&f, "docs/old.md");
        run(
            &f,
            &common(&v),
            op(json!({"op":"save","ref":"docs/old.md","purpose":"Move me","body":"# M\n"})),
        )
        .unwrap();
        let v = version_of(&f, "docs/old.md");
        let bad =
            op(json!({"op":"relocate","ref":"docs/old.md","to":"docs/Ü.md","to_version":"x"}));
        assert!(
            run(&f, &common(&v), bad)
                .err()
                .unwrap()
                .message
                .starts_with("to:")
        );
        let to_version = version_of(&f, "docs/new.md");
        let moved = op(
            json!({"op":"relocate","ref":"docs/old.md","to":"docs/new.md","to_version":to_version}),
        );
        let stale = run(
            &f,
            &common("0"),
            op(
                json!({"op":"relocate","ref":"docs/old.md","to":"docs/new.md","to_version":to_version}),
            ),
        );
        assert_eq!(stale.err().unwrap().code, "stale");
        let a = run(&f, &common(&v), moved).unwrap();
        assert_eq!(
            (a.target.as_str(), a.phase.as_str()),
            ("DOC-001", "managed")
        );
        assert!(
            a.refs.contains(&"docs/old.md".to_string())
                && a.refs.contains(&"docs/new.md".to_string())
        );
        assert!(
            dir.path().join("docs/new.md").exists() && !dir.path().join("docs/old.md").exists()
        );
    }

    /// A removal retires the record and reports the retired state without a path reference loss.
    #[test]
    fn remove_reports_retired_identity() {
        let (_dir, st) = root();
        let f = Fake::new(&st);
        let v = version_of(&f, "docs/r.md");
        run(
            &f,
            &common(&v),
            op(json!({"op":"save","ref":"docs/r.md","purpose":"Remove me","body":"x"})),
        )
        .unwrap();
        let v = version_of(&f, "docs/r.md");
        let a = run(
            &f,
            &common(&v),
            op(json!({"op":"remove","ref":"docs/r.md"})),
        )
        .unwrap();
        assert_eq!(
            (a.target.as_str(), a.phase.as_str(), a.changed),
            ("DOC-001", "retired", true)
        );
    }

    /// A reference to a heading removed by an edit is reported as dangling attention.
    #[test]
    fn dangling_attention_is_reported_in_notes() {
        let (dir, st) = root();
        let f = Fake::new(&st);
        native(&dir, "docs/t.md", b"# T\n## Part\n");
        native(&dir, "docs/l.md", b"[x](t.md#part)\n");
        let v = version_of(&f, "docs/t.md");
        let a = run(
            &f,
            &common(&v),
            op(json!({"op":"save","ref":"docs/t.md","purpose":"Target","body":"# T\n## Other\n"})),
        )
        .unwrap();
        assert!(
            a.notes.iter().any(|n| n.contains("1 left dangling")),
            "{:?}",
            a.notes
        );
        assert!(
            a.notes.iter().any(|n| n.starts_with("Dangling: docs/l.md")),
            "{:?}",
            a.notes
        );
    }
}
