//! Module controls for the compaction domain: closed inventory, record retention, review rules, the
//! apply state machine and the removal barrier, against the faithful provider double.
//!
//! These are module evidence only. The double models the agreed provider semantics but is not the real
//! document, persistence or allocation code, so none of these results proves composition.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "Test fixtures and explicit assertions"
)]
use super::{
    apply::apply,
    env::*,
    fake::{FakeEnv, Inject, outline_of},
    inventory,
    ops::{self, Outcome, RecoveryIn, RecoveryStep},
    record::{self, *},
    review::ReviewIn,
    validate::{ActionIn, PreservationIn, ProposalIn, SectionIn, SourceIn},
};
use crate::{
    model::{Finding, Verdict},
    store::{DirEntry, DirListing as Listing, EntryKind, Error, Result},
};
use std::collections::BTreeMap;

/// A managed document with a preamble and two sections.
const A_BODY: &str = "Intro\n## S1\none\n## S2\ntwo\n";
/// An unmanaged document with a preamble and one section.
const B_BODY: &str = "Bee\n## T1\nuno\n";

/// Run one handler call as one atomic intent: success settles the intent, failure holds it.
fn call<T>(env: &FakeEnv, f: impl FnOnce(&mut Vec<String>) -> Result<T>) -> Result<T> {
    env.start_call();
    let mut fx = Vec::new();
    let r = f(&mut fx);
    env.finish_call(r.is_ok());
    r
}

/// Current version of a proposal record.
fn version(env: &FakeEnv, cp: &str) -> String {
    super::read_cp(env.store(), cp).unwrap().version
}

/// Ledger over a source: every section kept when its bytes are in `candidate`, else dropped.
fn ledger(env: &FakeEnv, path: &str, candidate: Option<(&str, &str)>) -> Vec<SectionIn> {
    let doc = env.observe(path).unwrap();
    outline_of(&doc.body.unwrap())
        .sections
        .into_iter()
        .map(|s| {
            let kept = candidate.filter(|(_, c)| {
                let c = c.as_bytes();
                c.windows(s.bytes.len()).any(|w| w == s.bytes.as_slice())
            });
            SectionIn {
                path: path.to_owned(),
                section: s.addr,
                disposition: match kept {
                    Some((action, _)) => Disposition::Kept {
                        action: action.to_owned(),
                    },
                    None => Disposition::Dropped {
                        kind: DropKind::Obsolete,
                        reason: "no longer needed".into(),
                    },
                },
            }
        })
        .collect()
}

/// A source naming a path with its current observation version.
fn source(env: &FakeEnv, path: &str) -> SourceIn {
    SourceIn {
        path: path.to_owned(),
        version: env.observe(path).unwrap().version,
    }
}

/// A bare action of the given kind with a default reason.
fn action(id: &str, kind: ActionKind, path: &str) -> ActionIn {
    ActionIn {
        id: id.into(),
        kind,
        path: path.into(),
        from: None,
        base_version: None,
        content: None,
        purpose: None,
        reason: "compaction".into(),
        absorbed_into: vec![],
    }
}

/// One Replace of a managed document keeping its preamble and first section.
fn replace_a(env: &FakeEnv) -> ProposalIn {
    let candidate = "Intro\n## S1\none\n";
    ProposalIn {
        title: "Trim A".into(),
        sources: vec![source(env, "docs/a.md")],
        actions: vec![ActionIn {
            content: Some(candidate.into()),
            ..action("A-01", ActionKind::Replace, "docs/a.md")
        }],
        sections: ledger(env, "docs/a.md", Some(("A-01", candidate))),
        preservation: vec![],
    }
}

/// A fixture with a managed and an unmanaged committed document.
fn world() -> FakeEnv {
    let env = FakeEnv::new();
    env.seed("docs/a.md", A_BODY, true);
    env.seed("docs/b.md", B_BODY, false);
    env
}

/// Propose through the real operation at the current allocation version.
fn propose(env: &FakeEnv, key: &str, input: &ProposalIn) -> Result<Outcome> {
    let v = env.allocation_version().unwrap();
    call(env, |fx| ops::propose(env, "author", &v, key, input, fx))
}

/// Accept the current revision as an independent reviewer verifying every item.
fn accept(env: &FakeEnv, cp: &str, reviewer: &str) -> Result<Outcome> {
    let snap = super::read_cp(env.store(), cp).unwrap();
    let rec = &snap.value;
    let items = super::review::review_items(rec).unwrap();
    let rev = rec.revision().unwrap();
    let input = ReviewIn {
        revision: rec.current,
        content_hash: rev.content_hash.clone(),
        verdict: Verdict::Accepted,
        summary: "verified".into(),
        verified_items: items.into_iter().collect(),
        findings: vec![],
        resolved: vec![],
    };
    call(env, |fx| {
        ops::review_cp(env, reviewer, &snap.version, cp, &input, fx)
    })
}

/// Apply as one handler call.
fn run_apply(env: &FakeEnv, cp: &str) -> Result<Outcome> {
    let v = version(env, cp);
    call(env, |fx| apply(env, "applier", &v, cp, fx))
}

/// Stable error code of a result, or `ok`.
fn code<T>(r: Result<T>) -> &'static str {
    r.err().map(|e: Error| e.code).unwrap_or("ok")
}

// ---------------------------------------------------------------- inventory

/// Build an abstract directory lister from a table.
fn fixture(entries: &[(&str, &[(&str, EntryKind)])]) -> impl Fn(&str, usize) -> Result<Listing> {
    let map: BTreeMap<String, Vec<(String, EntryKind)>> = entries
        .iter()
        .map(|(k, v)| {
            (
                (*k).to_owned(),
                v.iter().map(|(n, k)| ((*n).to_owned(), *k)).collect(),
            )
        })
        .collect();
    move |relative, cap| {
        let list = map.get(relative).cloned().unwrap_or_default();
        let complete = list.len() <= cap;
        Ok(Listing {
            entries: list
                .into_iter()
                .take(cap)
                .map(|(name, kind)| DirEntry { name, kind })
                .collect(),
            complete,
        })
    }
}

/// Positive: a record with its staged blob is complete and counts one identifier once; an orphan
/// directory counts its number with a warning; own temp leftovers are warnings only.
#[test]
fn inventory_counts_records_and_orphans_once() {
    use EntryKind::{Directory, File};
    let list = fixture(&[
        (
            "compactions",
            &[
                ("CP-001", Directory),
                ("CP-001.yaml", File),
                ("CP-002", Directory),
                (".CP-003.yaml.tmp-12-7", File),
            ],
        ),
        ("compactions/CP-001", &[("r1", Directory)]),
        (
            "compactions/CP-001/r1",
            &[("A-01.md", File), (".A-02.md.tmp-1-2", File)],
        ),
        ("compactions/CP-002", &[]),
    ]);
    let inv = inventory::inventory_with(&list).unwrap();
    assert_eq!(inv.ids, vec!["CP-001", "CP-002"]);
    assert!(inv.complete, "{:?}", inv.warnings);
    assert!(
        inv.warnings
            .iter()
            .any(|w| w.contains("CP-002") && w.contains("without a record"))
    );
}

/// One foreign inventory case: a label, top level entries and nested listings.
type Case<'a> = (
    &'a str,
    Vec<(&'a str, EntryKind)>,
    Vec<(&'a str, Vec<(&'a str, EntryKind)>)>,
);

/// Negative: every foreign nested name is named and makes the inventory incomplete.
#[test]
fn inventory_names_foreign_entries_incomplete() {
    use EntryKind::{Directory, File, Symlink};
    let cases: Vec<Case> = vec![
        ("noncanonical record", vec![("CP-0001.yaml", File)], vec![]),
        ("stray file", vec![("notes.txt", File)], vec![]),
        ("symlinked record", vec![("CP-001.yaml", Symlink)], vec![]),
        (
            "directory named like a record",
            vec![("CP-001.yaml", Directory)],
            vec![],
        ),
        (
            "foreign child",
            vec![("CP-001", Directory)],
            vec![("compactions/CP-001", vec![("x.txt", File)])],
        ),
        (
            "ninth revision",
            vec![("CP-001", Directory)],
            vec![("compactions/CP-001", vec![("r9", Directory)])],
        ),
        (
            "depth three",
            vec![("CP-001", Directory)],
            vec![
                ("compactions/CP-001", vec![("r1", Directory)]),
                ("compactions/CP-001/r1", vec![("sub", Directory)]),
            ],
        ),
        (
            "blob beyond the action range",
            vec![("CP-001", Directory)],
            vec![
                ("compactions/CP-001", vec![("r1", Directory)]),
                ("compactions/CP-001/r1", vec![("A-33.md", File)]),
            ],
        ),
    ];
    for (name, top, nested) in cases {
        let mut table: Vec<(&str, &[(&str, EntryKind)])> = vec![("compactions", &top)];
        for (k, v) in &nested {
            table.push((k, v));
        }
        let inv = inventory::inventory_with(&fixture(&table)).unwrap();
        assert!(!inv.complete, "{name} must be incomplete");
        assert!(!inv.warnings.is_empty(), "{name} must be named");
    }
}

/// Only a regular owned publication leftover is ignorable: the same names as a link, a directory or
/// another nonregular entry are foreign, named and make the inventory incomplete, at the top level
/// and in a revision directory, while a regular leftover stays a warning and the staged directory
/// keeps its identifier.
#[test]
fn inventory_own_temp_names_must_be_regular_files() {
    use EntryKind::{Directory, File, Other, Symlink};
    for kind in [Symlink, Directory, Other] {
        let top = [(".CP-001.yaml.tmp-123-1", kind), ("CP-002", Directory)];
        let table: Vec<(&str, &[(&str, EntryKind)])> = vec![("compactions", &top)];
        let inv = inventory::inventory_with(&fixture(&table)).unwrap();
        assert!(!inv.complete, "top level {kind:?} must be incomplete");
        assert!(
            inv.warnings
                .iter()
                .any(|w| w.contains(".CP-001.yaml.tmp-123-1") && w.contains("unrecognized")),
            "top level {kind:?} must be named: {:?}",
            inv.warnings
        );
        assert_eq!(inv.ids, vec!["CP-002"], "the staged directory keeps its id");
        let rev = [(".A-01.md.tmp-123-1", kind)];
        let table: Vec<(&str, &[(&str, EntryKind)])> = vec![
            ("compactions", &[("CP-001", Directory)]),
            ("compactions/CP-001", &[("r1", Directory)]),
            ("compactions/CP-001/r1", &rev),
        ];
        let inv = inventory::inventory_with(&fixture(&table)).unwrap();
        assert!(!inv.complete, "blob level {kind:?} must be incomplete");
        assert!(
            inv.warnings
                .iter()
                .any(|w| w.contains(".A-01.md.tmp-123-1") && w.contains("unrecognized")),
            "blob level {kind:?} must be named: {:?}",
            inv.warnings
        );
    }
    let top = [(".CP-001.yaml.tmp-123-1", File), ("CP-002", Directory)];
    let rev = [(".A-01.md.tmp-123-1", File)];
    let table: Vec<(&str, &[(&str, EntryKind)])> = vec![
        ("compactions", &top),
        ("compactions/CP-002", &[("r1", Directory)]),
        ("compactions/CP-002/r1", &rev),
    ];
    let inv = inventory::inventory_with(&fixture(&table)).unwrap();
    assert!(
        inv.complete,
        "regular leftovers stay harmless: {:?}",
        inv.warnings
    );
    assert_eq!(
        inv.warnings
            .iter()
            .filter(|w| w.contains("publication leftover ignored"))
            .count(),
        2
    );
}

/// The same rule over the real store listing: a symlink and a directory carrying an owned temp name
/// are foreign, a regular file with that name is a warning only.
#[cfg(unix)]
#[test]
fn inventory_own_temp_names_over_the_real_store() {
    let env = FakeEnv::new();
    let home = env.store().path("compactions").unwrap();
    std::fs::create_dir_all(home.join("CP-001/r1")).unwrap();
    std::fs::write(home.join("CP-001.yaml"), b"x").unwrap();
    std::fs::write(home.join("CP-001/r1/A-01.md"), b"x").unwrap();
    std::fs::write(home.join(".CP-001.yaml.tmp-123-1"), b"x").unwrap();
    std::fs::write(home.join("CP-001/r1/.A-01.md.tmp-123-1"), b"x").unwrap();
    let inv = super::inventory(env.store()).unwrap();
    assert!(inv.complete, "{:?}", inv.warnings);
    assert_eq!(inv.warnings.len(), 2);
    std::fs::remove_file(home.join(".CP-001.yaml.tmp-123-1")).unwrap();
    std::os::unix::fs::symlink("CP-001.yaml", home.join(".CP-001.yaml.tmp-123-2")).unwrap();
    let inv = super::inventory(env.store()).unwrap();
    assert!(
        !inv.complete && inv.ids == vec!["CP-001"],
        "{:?}",
        inv.warnings
    );
    std::fs::remove_file(home.join(".CP-001.yaml.tmp-123-2")).unwrap();
    std::fs::remove_file(home.join("CP-001/r1/.A-01.md.tmp-123-1")).unwrap();
    std::fs::create_dir(home.join("CP-001/r1/.A-01.md.tmp-123-3")).unwrap();
    let inv = super::inventory(env.store()).unwrap();
    assert!(!inv.complete, "{:?}", inv.warnings);
}

/// The listing cap makes the inventory incomplete instead of silently truncating.
#[test]
fn inventory_cap_is_incomplete() {
    let many: Vec<(String, EntryKind)> = (1..=513)
        .map(|n| (format!("CP-{n:03}.yaml"), EntryKind::File))
        .collect();
    let list = move |relative: &str, cap: usize| -> Result<Listing> {
        assert_eq!(relative, "compactions");
        Ok(Listing {
            entries: many
                .iter()
                .take(cap)
                .map(|(n, k)| DirEntry {
                    name: n.clone(),
                    kind: *k,
                })
                .collect(),
            complete: many.len() <= cap,
        })
    };
    assert!(!inventory::inventory_with(&list).unwrap().complete);
}

/// The real store listing: an absent home is complete and empty; a regular file at the home refuses.
#[test]
fn inventory_over_the_real_store() {
    let env = FakeEnv::new();
    let inv = super::inventory(env.store()).unwrap();
    assert!(inv.ids.is_empty() && inv.complete);
    let home = env.store().path("compactions").unwrap();
    std::fs::create_dir_all(home.join("CP-004/r1")).unwrap();
    std::fs::write(home.join("CP-004/r1/A-01.md"), b"x").unwrap();
    std::fs::write(home.join("CP-004/r1/junk"), b"x").unwrap();
    let inv = super::inventory(env.store()).unwrap();
    assert_eq!(inv.ids, vec!["CP-004"]);
    assert!(!inv.complete);
    std::fs::remove_dir_all(&home).unwrap();
    std::fs::write(&home, b"file").unwrap();
    assert_eq!(code(super::inventory(env.store())), "file_type");
}

// ----------------------------------------------------------------- lifecycle

/// A proposal publishes its record and staged blob, and an identical retry is unchanged.
#[test]
fn propose_publishes_record_and_blob_and_is_idempotent() {
    let env = world();
    let input = replace_a(&env);
    let out = propose(&env, "key-0001-aa", &input).unwrap();
    assert_eq!(
        (out.id.as_str(), out.state, out.revision, out.changed),
        ("CP-001", CpState::Proposed, 1, true)
    );
    let rec = super::read_cp(env.store(), "CP-001").unwrap().value;
    assert_eq!(rec.revisions.len(), 1);
    let blob = env
        .store()
        .bytes("compactions/CP-001/r1/A-01.md")
        .unwrap()
        .unwrap();
    assert_eq!(blob, b"Intro\n## S1\none\n");
    let again = propose(&env, "key-0001-aa", &input).unwrap();
    assert!(!again.changed);
    assert_eq!(again.id, "CP-001");
    let mut other = input.clone();
    other.title = "Different".into();
    assert_eq!(
        code(propose(&env, "key-0001-aa", &other)),
        "request_key_conflict"
    );
}

/// Self review, wrong reviewer and incomplete verdicts are refused; the reviewer stays pinned.
#[test]
fn review_rules_pin_the_independent_reviewer() {
    let env = world();
    propose(&env, "key-0002-bb", &replace_a(&env)).unwrap();
    assert_eq!(code(accept(&env, "CP-001", "author")), "self_review");
    // Missing item.
    let snap = super::read_cp(env.store(), "CP-001").unwrap();
    let rev = snap.value.revision().unwrap().content_hash.clone();
    let mut items: Vec<String> = super::review::review_items(&snap.value)
        .unwrap()
        .into_iter()
        .collect();
    let dropped = items.pop().unwrap();
    let bad = ReviewIn {
        revision: 1,
        content_hash: rev.clone(),
        verdict: Verdict::Accepted,
        summary: "x".into(),
        verified_items: items.clone(),
        findings: vec![],
        resolved: vec![],
    };
    assert_eq!(
        code(call(&env, |fx| ops::review_cp(
            &env,
            "rev1",
            &snap.version,
            "CP-001",
            &bad,
            fx
        ))),
        "review_incomplete",
        "missing {dropped}"
    );
    let stale = ReviewIn {
        content_hash: "0".repeat(64),
        ..bad.clone()
    };
    assert_eq!(
        code(call(&env, |fx| ops::review_cp(
            &env,
            "rev1",
            &snap.version,
            "CP-001",
            &stale,
            fx
        ))),
        "stale_revision"
    );
    let out = accept(&env, "CP-001", "rev1").unwrap();
    assert_eq!(out.state, CpState::Accepted);
    let rec = super::read_cp(env.store(), "CP-001").unwrap().value;
    assert_eq!(rec.reviewer.as_ref().unwrap().agent_id, "rev1");
    assert!(rec.acceptance_current().is_some());
}

/// A reviewer cannot author, an author cannot review, and revise keeps every prior body and review
/// byte for byte while clearing only the acceptance pointer.
#[test]
fn revise_retains_prior_revisions_and_reviews() {
    let env = world();
    propose(&env, "key-0003-cc", &replace_a(&env)).unwrap();
    accept(&env, "CP-001", "rev1").unwrap();
    let before = super::read_cp(env.store(), "CP-001").unwrap().value;
    let mut next = replace_a(&env);
    next.title = "Trim A again".into();
    next.actions[0].content = Some("Intro\n## S1\none\nplus\n".into());
    next.sections = ledger(
        &env,
        "docs/a.md",
        Some(("A-01", "Intro\n## S1\none\nplus\n")),
    );
    let v = version(&env, "CP-001");
    assert_eq!(
        code(call(&env, |fx| ops::revise(
            &env, "rev1", &v, "CP-001", &next, fx
        ))),
        "reviewer_cannot_author"
    );
    let out = call(&env, |fx| {
        ops::revise(&env, "author", &v, "CP-001", &next, fx)
    })
    .unwrap();
    assert_eq!((out.revision, out.state), (2, CpState::Proposed));
    let after = super::read_cp(env.store(), "CP-001").unwrap().value;
    assert_eq!(
        after.revisions[0], before.revisions[0],
        "revision one is retained byte for byte"
    );
    assert_eq!(after.reviews, before.reviews, "reviews are append only");
    assert!(after.accepted_review.is_none() && after.acceptance_current().is_none());
    // An identical revise is unchanged and publishes nothing.
    let v2 = version(&env, "CP-001");
    let same = call(&env, |fx| {
        ops::revise(&env, "author", &v2, "CP-001", &next, fx)
    })
    .unwrap();
    assert!(!same.changed);
    // Both staged blobs exist in their own directories.
    assert!(
        env.store()
            .bytes("compactions/CP-001/r1/A-01.md")
            .unwrap()
            .is_some()
    );
    assert!(
        env.store()
            .bytes("compactions/CP-001/r2/A-01.md")
            .unwrap()
            .is_some()
    );
}

/// A revision that would overflow the record is refused before any publication and leaves the
/// published record and the staged blobs untouched; no prior body is pruned to make room.
#[test]
fn revise_overflow_refuses_and_keeps_the_record() {
    let env = world();
    propose(&env, "key-0004-dd", &replace_a(&env)).unwrap();
    // Bloat the retained revision to just under the nonterminal cap.
    let snap = super::read_cp(env.store(), "CP-001").unwrap();
    let mut rec = snap.value;
    let item = |i: usize| Preservation {
        id: format!("P-{i}"),
        kind: PreserveKind::Requirement,
        source_section: "S-001".into(),
        statement: "x".repeat(250),
        target: whole("docs/a.md"),
        mode: PreserveMode::Rewritten,
    };
    let base = crate::store::encode(&rec).unwrap().len();
    rec.revisions[0]
        .body
        .preservation
        .extend((0..100).map(item));
    let per = (crate::store::encode(&rec).unwrap().len() - base) / 100;
    let more = (487_000 - base) / per - 100;
    rec.revisions[0]
        .body
        .preservation
        .extend((100..100 + more).map(item));
    let bytes = crate::store::encode(&rec).unwrap();
    assert!(
        bytes.len() > 486_000 && bytes.len() < 490_500,
        "{}",
        bytes.len()
    );
    let retained = rec.revisions[0].body.preservation.len();
    env.store()
        .publish(
            "compactions/CP-001.yaml",
            &bytes,
            Some(&snap.bytes),
            &mut vec![],
        )
        .unwrap();
    let published = env
        .store()
        .bytes("compactions/CP-001.yaml")
        .unwrap()
        .unwrap();
    let mut next = replace_a(&env);
    next.actions[0].content = Some("Intro\n## S1\none\nplus\n".into());
    next.sections = ledger(
        &env,
        "docs/a.md",
        Some(("A-01", "Intro\n## S1\none\nplus\n")),
    );
    let s1 = outline_of(A_BODY.as_bytes()).sections[1].addr.clone();
    next.preservation = (1..=64)
        .map(|n| PreservationIn {
            id: format!("P-{n:02}"),
            kind: PreserveKind::Requirement,
            path: "docs/a.md".into(),
            section: s1.clone(),
            statement: "y".repeat(250),
            target: whole("docs/a.md"),
            mode: PreserveMode::Rewritten,
        })
        .collect();
    let v = version(&env, "CP-001");
    let refused = call(&env, |fx| {
        ops::revise(&env, "author", &v, "CP-001", &next, fx)
    });
    assert_eq!(code(refused), "capacity");
    assert_eq!(
        env.store()
            .bytes("compactions/CP-001.yaml")
            .unwrap()
            .unwrap(),
        published
    );
    assert!(
        env.store()
            .bytes("compactions/CP-001/r2/A-01.md")
            .unwrap()
            .is_none(),
        "no blob is published before capacity is proven"
    );
    let kept = super::read_cp(env.store(), "CP-001").unwrap();
    assert_eq!(
        kept.value.revisions[0].body.preservation.len(),
        retained,
        "no prior body is pruned"
    );
}

/// Reviewer replacement only after observed unrecoverable loss, never as a convenience switch.
#[test]
fn reviewer_replacement_is_loss_only() {
    let env = world();
    propose(&env, "key-0005-ee", &replace_a(&env)).unwrap();
    let snap = super::read_cp(env.store(), "CP-001").unwrap();
    let hash = snap.value.revision().unwrap().content_hash.clone();
    let changes = ReviewIn {
        revision: 1,
        content_hash: hash,
        verdict: Verdict::ChangesRequested,
        summary: "needs work".into(),
        verified_items: vec![],
        findings: vec![Finding {
            text: "fix S1".into(),
            must_fix: true,
        }],
        resolved: vec![],
    };
    call(&env, |fx| {
        ops::review_cp(&env, "rev1", &snap.version, "CP-001", &changes, fx)
    })
    .unwrap();
    // A different reviewer without a loss report is refused.
    let v = version(&env, "CP-001");
    let attempt = ReviewIn {
        revision: 1,
        ..changes.clone()
    };
    assert_eq!(
        code(call(&env, |fx| ops::review_cp(
            &env, "other", &v, "CP-001", &attempt, fx
        ))),
        "reviewer_pinned"
    );
    // Loss needs both flags.
    let soft = RecoveryIn {
        lost: Some(true),
        unrecoverable: Some(false),
        observation: Some("timed out".into()),
        ..RecoveryIn::default()
    };
    assert_eq!(
        code(call(&env, |fx| ops::recover_reviewer(
            &env,
            "orch",
            &v,
            "CP-001",
            RecoveryStep::Lost,
            &soft,
            fx
        ))),
        "invalid_arguments"
    );
    let lost = RecoveryIn {
        lost: Some(true),
        unrecoverable: Some(true),
        observation: Some("session ended, cannot resume".into()),
        ..RecoveryIn::default()
    };
    call(&env, |fx| {
        ops::recover_reviewer(&env, "orch", &v, "CP-001", RecoveryStep::Lost, &lost, fx)
    })
    .unwrap();
    // While lost, nobody may review.
    let v = version(&env, "CP-001");
    assert_eq!(
        code(call(&env, |fx| ops::review_cp(
            &env, "rev2", &v, "CP-001", &attempt, fx
        ))),
        "reviewer_gap"
    );
    // The author cannot become the replacement; a gap blocks review until cleared.
    let imm = RecoveryIn {
        understanding: Some("read the proposal and findings".into()),
        sources: vec!["compaction.md".into()],
        gaps: vec!["did not read S2".into()],
        ..RecoveryIn::default()
    };
    assert_eq!(
        code(call(&env, |fx| ops::recover_reviewer(
            &env,
            "author",
            &v,
            "CP-001",
            RecoveryStep::Immersed,
            &imm,
            fx
        ))),
        "self_review"
    );
    call(&env, |fx| {
        ops::recover_reviewer(&env, "rev2", &v, "CP-001", RecoveryStep::Immersed, &imm, fx)
    })
    .unwrap();
    let v = version(&env, "CP-001");
    assert_eq!(
        code(call(&env, |fx| ops::review_cp(
            &env, "rev2", &v, "CP-001", &attempt, fx
        ))),
        "reviewer_gap"
    );
    let clear = RecoveryIn {
        gaps: vec![],
        ..imm
    };
    call(&env, |fx| {
        ops::recover_reviewer(
            &env,
            "rev2",
            &v,
            "CP-001",
            RecoveryStep::Immersed,
            &clear,
            fx,
        )
    })
    .unwrap();
    let rec = super::read_cp(env.store(), "CP-001").unwrap().value;
    let b = rec.reviewer.unwrap();
    assert_eq!(
        (
            b.agent_id.as_str(),
            b.predecessors.len(),
            b.predecessors[0].agent_id.as_str()
        ),
        ("rev2", 1, "rev1")
    );
    assert_eq!(rec.reviews.len(), 1, "the predecessor's review is retained");
}

/// An unresolved must fix finding blocks acceptance until a later review resolves it by index.
#[test]
fn must_fix_findings_block_acceptance_until_resolved() {
    let env = world();
    propose(&env, "key-0006-ff", &replace_a(&env)).unwrap();
    let snap = super::read_cp(env.store(), "CP-001").unwrap();
    let hash = snap.value.revision().unwrap().content_hash.clone();
    let changes = ReviewIn {
        revision: 1,
        content_hash: hash.clone(),
        verdict: Verdict::ChangesRequested,
        summary: "needs work".into(),
        verified_items: vec![],
        findings: vec![Finding {
            text: "fix S1".into(),
            must_fix: true,
        }],
        resolved: vec![],
    };
    call(&env, |fx| {
        ops::review_cp(&env, "rev1", &snap.version, "CP-001", &changes, fx)
    })
    .unwrap();
    let snap = super::read_cp(env.store(), "CP-001").unwrap();
    let items: Vec<String> = super::review::review_items(&snap.value)
        .unwrap()
        .into_iter()
        .collect();
    let accept_unresolved = ReviewIn {
        verdict: Verdict::Accepted,
        findings: vec![],
        verified_items: items.clone(),
        ..changes.clone()
    };
    assert_eq!(
        code(call(&env, |fx| ops::review_cp(
            &env,
            "rev1",
            &snap.version,
            "CP-001",
            &accept_unresolved,
            fx
        ))),
        "review_incomplete"
    );
    let resolved = ReviewIn {
        resolved: vec![crate::model::FindingResolution {
            review_index: 0,
            finding_index: 0,
            summary: "fixed".into(),
        }],
        ..accept_unresolved
    };
    let out = call(&env, |fx| {
        ops::review_cp(&env, "rev1", &snap.version, "CP-001", &resolved, fx)
    })
    .unwrap();
    assert_eq!(out.state, CpState::Accepted);
}

/// Two live proposals cannot claim one path.
#[test]
fn live_proposals_cannot_claim_the_same_path() {
    let env = world();
    propose(&env, "key-0007-gg", &replace_a(&env)).unwrap();
    assert_eq!(
        code(propose(&env, "key-0008-hh", &replace_a(&env))),
        "path_claimed"
    );
}

/// A source that is typed, absent or stale is refused with a stable code before anything is reserved.
#[test]
fn propose_refuses_bad_sources_without_reserving() {
    let env = world();
    let mut stale = replace_a(&env);
    stale.sources[0].version = "0".repeat(64);
    assert_eq!(code(propose(&env, "key-0009-ii", &stale)), "stale_source");
    let mut missing = replace_a(&env);
    missing.sources[0] = SourceIn {
        path: "docs/none.md".into(),
        version: "x".into(),
    };
    assert_eq!(
        code(propose(&env, "key-0010-jj", &missing)),
        "source_refused"
    );
    let mut unaccounted = replace_a(&env);
    unaccounted.sections.pop();
    assert_eq!(
        code(propose(&env, "key-0011-kk", &unaccounted)),
        "section_unaccounted"
    );
    assert_eq!(
        env.allocation_version().unwrap(),
        "alloc-1",
        "no number was reserved by a refusal"
    );
}

/// Verbatim preservation is checked mechanically against the exact target bytes.
#[test]
fn verbatim_preservation_is_checked_against_the_target() {
    let env = world();
    let mut input = replace_a(&env);
    let s2 = outline_of(A_BODY.as_bytes()).sections[2].addr.clone();
    input.preservation = vec![PreservationIn {
        id: "P-01".into(),
        kind: PreserveKind::Requirement,
        path: "docs/a.md".into(),
        section: s2,
        statement: "two must survive".into(),
        target: whole("docs/a.md"),
        mode: PreserveMode::Verbatim,
    }];
    // S2 is dropped in the ledger, so preserving it is contradictory.
    assert_eq!(
        code(propose(&env, "key-0012-ll", &input)),
        "preservation_unmapped"
    );
    // Preserve S1 (kept) verbatim in the candidate: accepted.
    input.preservation[0].section = outline_of(A_BODY.as_bytes()).sections[1].addr.clone();
    assert!(propose(&env, "key-0013-mm", &input).is_ok());
}

// -------------------------------------------------------------------- apply

/// A merge of two documents: Create a new document, Remove a managed and an unmanaged source.
fn merge_input(env: &FakeEnv) -> ProposalIn {
    let merged = "Merged\n## S1\none\n## T1\nuno\n";
    let abs = |p: &str| ledger(env, p, None);
    let mut sections = abs("docs/a.md");
    sections.extend(abs("docs/b.md"));
    ProposalIn {
        title: "Merge A and B".into(),
        sources: vec![source(env, "docs/a.md"), source(env, "docs/b.md")],
        actions: vec![
            ActionIn {
                content: Some(merged.into()),
                purpose: Some("merged documentation".into()),
                ..action("A-01", ActionKind::Create, "docs/ab.md")
            },
            ActionIn {
                absorbed_into: vec![whole("docs/ab.md")],
                ..action("A-02", ActionKind::Remove, "docs/a.md")
            },
            ActionIn {
                absorbed_into: vec![whole("docs/ab.md")],
                ..action("A-03", ActionKind::Remove, "docs/b.md")
            },
        ],
        sections,
        preservation: vec![],
    }
}

/// Fresh Create plus managed and unmanaged Remove in one call: one intent, one commit, managed remove
/// ends Retired with the same identity, unmanaged remove gains no record.
#[test]
fn fresh_create_and_remove_apply_in_one_whole_intent() {
    let env = world();
    propose(&env, "key-0020-aa", &merge_input(&env)).unwrap();
    accept(&env, "CP-001", "rev1").unwrap();
    let out = run_apply(&env, "CP-001").unwrap();
    assert_eq!(
        (out.state, out.applied.len(), out.total),
        (CpState::Applied, 3, 3)
    );
    assert!(env.body("docs/a.md").is_none() && env.body("docs/b.md").is_none());
    assert_eq!(
        env.body("docs/ab.md").unwrap(),
        b"Merged\n## S1\none\n## T1\nuno\n"
    );
    let (id, rev, retired) = env.doc_for("docs/a.md").unwrap();
    assert!(
        retired && rev == 2,
        "managed remove retires the original identity: {id}"
    );
    assert!(
        env.doc_for("docs/b.md").is_none(),
        "unmanaged remove has no synthetic record"
    );
    assert!(env.doc_for("docs/ab.md").is_some());
    let rec = super::read_cp(env.store(), "CP-001").unwrap().value;
    assert_eq!(rec.apply.unwrap().phase, ApplyPhase::Done);
    // The atomic whole intent of the fresh call is committed, including the removals.
    assert!(env.pending().is_empty(), "{:?}", env.pending());
    // Repeating apply on an applied proposal is unchanged and never claims verification.
    let again = run_apply(&env, "CP-001").unwrap();
    assert!(!again.changed && again.notes.iter().any(|n| n.contains("no drift observed")));
}

/// Replace of a managed and of an unmanaged document, and Move of both kinds.
#[test]
fn replace_and_move_complete_with_their_post_images() {
    let env = world();
    env.seed("docs/m1.md", "Bee\n## M1\nm\n", true);
    env.seed("docs/m2.md", "Bee\n## M2\nm\n", false);
    let mut sections = ledger(&env, "docs/a.md", Some(("A-01", "Intro\n## S1\none\n")));
    sections.extend(ledger(
        &env,
        "docs/b.md",
        Some(("A-02", "Bee\n## T1\nuno\nplus\n")),
    ));
    for (p, a) in [("docs/m1.md", "A-03"), ("docs/m2.md", "A-04")] {
        let doc = env.observe(p).unwrap();
        for s in outline_of(&doc.body.unwrap()).sections {
            sections.push(SectionIn {
                path: p.into(),
                section: s.addr,
                disposition: Disposition::Moved { action: a.into() },
            });
        }
    }
    let input = ProposalIn {
        title: "Replace and move".into(),
        sources: vec![
            source(&env, "docs/a.md"),
            source(&env, "docs/b.md"),
            source(&env, "docs/m1.md"),
            source(&env, "docs/m2.md"),
        ],
        actions: vec![
            ActionIn {
                content: Some("Intro\n## S1\none\n".into()),
                ..action("A-01", ActionKind::Replace, "docs/a.md")
            },
            ActionIn {
                content: Some("Bee\n## T1\nuno\nplus\n".into()),
                purpose: Some("b gains a record".into()),
                ..action("A-02", ActionKind::Replace, "docs/b.md")
            },
            ActionIn {
                from: Some("docs/m1.md".into()),
                ..action("A-03", ActionKind::Move, "docs/moved1.md")
            },
            ActionIn {
                from: Some("docs/m2.md".into()),
                ..action("A-04", ActionKind::Move, "docs/moved2.md")
            },
        ],
        sections,
        preservation: vec![],
    };
    let m1_id = env.doc_for("docs/m1.md").unwrap().0;
    propose(&env, "key-0021-bb", &input).unwrap();
    accept(&env, "CP-001", "rev1").unwrap();
    let out = run_apply(&env, "CP-001").unwrap();
    assert_eq!(out.state, CpState::Applied);
    assert_eq!(env.body("docs/a.md").unwrap(), b"Intro\n## S1\none\n");
    assert_eq!(
        env.doc_for("docs/a.md").unwrap().1,
        2,
        "managed replace bumps the record revision"
    );
    assert_eq!(
        env.doc_for("docs/b.md").unwrap().1,
        1,
        "an unmanaged replace gains a new record at revision one"
    );
    let (moved_id, _, retired) = env.doc_for("docs/moved1.md").unwrap();
    assert!(
        !retired && moved_id == m1_id,
        "the move keeps the source DOC identity"
    );
    assert!(env.body("docs/m1.md").is_none() && env.body("docs/m2.md").is_none());
    assert!(
        env.doc_for("docs/moved2.md").is_none(),
        "an unmanaged move gains no record"
    );
}

/// Originals that are not committed refuse before any effect and leave the record untouched.
#[test]
fn uncommitted_originals_refuse_before_any_effect() {
    let env = world();
    // An unmanaged document was edited natively after its last commit and before it was proposed, so
    // the observed bytes exist in no commit.
    env.dirty(
        "docs/b.md",
        "Bee
## T1
uno edited
",
    );
    let candidate = "Bee
## T1
uno edited
plus
";
    let input = ProposalIn {
        title: "Extend B".into(),
        sources: vec![source(&env, "docs/b.md")],
        actions: vec![ActionIn {
            content: Some(candidate.into()),
            purpose: Some("b gains a record".into()),
            ..action("A-01", ActionKind::Replace, "docs/b.md")
        }],
        sections: ledger(&env, "docs/b.md", Some(("A-01", candidate))),
        preservation: vec![],
    };
    propose(&env, "key-0022-cc", &input).unwrap();
    accept(&env, "CP-001", "rev1").unwrap();
    let before = env
        .store()
        .bytes("compactions/CP-001.yaml")
        .unwrap()
        .unwrap();
    assert_eq!(code(run_apply(&env, "CP-001")), "originals_not_committed");
    let after = env
        .store()
        .bytes("compactions/CP-001.yaml")
        .unwrap()
        .unwrap();
    assert_eq!(before, after, "a refusal before any effect changes nothing");
    assert_eq!(
        env.body("docs/b.md").unwrap(),
        b"Bee
## T1
uno edited
"
    );
}

/// Body first partial: the replacement body is published, the call fails, and the resume adopts the
/// attested body through the oracle without failing the original base version and without requiring
/// the new uncommitted body to be committed as an original.
#[test]
fn body_first_partial_resumes_through_the_oracle() {
    let env = world();
    propose(&env, "key-0023-dd", &replace_a(&env)).unwrap();
    accept(&env, "CP-001", "rev1").unwrap();
    env.inject(Inject::AfterBody);
    let first = run_apply(&env, "CP-001");
    assert_eq!(code(first), "partial_publication");
    let rec = super::read_cp(env.store(), "CP-001").unwrap().value;
    assert_eq!(rec.state, CpState::Blocked);
    assert_eq!(
        env.body("docs/a.md").unwrap(),
        b"Intro\n## S1\none\n",
        "the body is published, its record is not"
    );
    // The held effects must be committed by the explicit recovery before the resume writes again.
    assert_eq!(
        code(run_apply(&env, "CP-001")),
        "replacements_not_committed"
    );
    env.retry_all();
    let out = run_apply(&env, "CP-001").unwrap();
    assert_eq!(out.state, CpState::Applied);
    assert_eq!(env.doc_for("docs/a.md").unwrap().1, 2);
    assert!(
        env.doc_ops().iter().any(|o| o == "adopt docs/a.md"),
        "{:?}",
        env.doc_ops()
    );
    let rec = super::read_cp(env.store(), "CP-001").unwrap().value;
    assert_eq!(rec.apply.as_ref().unwrap().attempts, 2);
}

/// Equal bytes without attestation are unknown, never ownership: a byte identical native copy blocks.
#[test]
fn equal_bytes_without_attestation_block() {
    let env = world();
    propose(&env, "key-0024-ee", &replace_a(&env)).unwrap();
    accept(&env, "CP-001", "rev1").unwrap();
    // The first call publishes the body and fails before its record.
    env.inject(Inject::AfterBody);
    assert_eq!(code(run_apply(&env, "CP-001")), "partial_publication");
    // The attestation is lost; only bytes equal to the candidate remain. They certify nothing.
    env.forget_rows("cp:CP-001");
    assert_eq!(
        env.body("docs/a.md").unwrap(),
        b"Intro
## S1
one
"
    );
    assert_eq!(code(run_apply(&env, "CP-001")), "effect_unknown");
    assert!(
        env.doc_ops().iter().all(|o| o != "adopt docs/a.md"),
        "{:?}",
        env.doc_ops()
    );
}

/// The N1 trace: a Move whose destination was published by an interrupted call is held; the resume
/// blocks before relocate's source removal and names Retry; after Retry the same source identity
/// completes and the repository holds the destination.
#[test]
fn move_resume_blocks_until_the_held_destination_is_committed() {
    let env = world();
    env.seed("docs/m1.md", "Bee\n## M1\nm\n", true);
    let m1_id = env.doc_for("docs/m1.md").unwrap().0;
    let doc = env.observe("docs/m1.md").unwrap();
    let sections = outline_of(&doc.body.unwrap())
        .sections
        .into_iter()
        .map(|s| SectionIn {
            path: "docs/m1.md".into(),
            section: s.addr,
            disposition: Disposition::Moved {
                action: "A-01".into(),
            },
        })
        .collect();
    let input = ProposalIn {
        title: "Move".into(),
        sources: vec![source(&env, "docs/m1.md")],
        actions: vec![ActionIn {
            from: Some("docs/m1.md".into()),
            ..action("A-01", ActionKind::Move, "docs/new/m1.md")
        }],
        sections,
        preservation: vec![],
    };
    propose(&env, "key-0025-ff", &input).unwrap();
    accept(&env, "CP-001", "rev1").unwrap();
    env.inject(Inject::AfterTo);
    assert_eq!(code(run_apply(&env, "CP-001")), "partial_publication");
    assert!(env.body("docs/new/m1.md").is_some() && env.body("docs/m1.md").is_some());
    // Resume: the older destination is held, so relocate never runs and the source stays.
    let before = env.doc_ops().len();
    let blocked = run_apply(&env, "CP-001");
    assert_eq!(code(blocked), "replacements_not_committed");
    assert_eq!(
        env.doc_ops().len(),
        before,
        "relocate was not called while the destination is held"
    );
    assert!(env.body("docs/m1.md").is_some(), "the source is untouched");
    assert_eq!(env.doc_for("docs/m1.md").unwrap().0, m1_id);
    // Explicit Retry commits the old intent; the resume then finishes with the same identity.
    env.retry_all();
    let out = run_apply(&env, "CP-001").unwrap();
    assert_eq!(out.state, CpState::Applied);
    let (id, _, retired) = env.doc_for("docs/new/m1.md").unwrap();
    assert!(!retired && id == m1_id);
    assert!(env.body("docs/m1.md").is_none());
    assert!(env.pending().is_empty(), "{:?}", env.pending());
}

/// A later change on an applied path, even by another attested operation, blocks the unfinished
/// proposal instead of being accepted as success.
#[test]
fn later_attested_change_blocks_an_unfinished_proposal() {
    let env = world();
    let mut input = merge_input(&env);
    // Keep A as a source for a Replace instead: Replace A, Remove B.
    input.sources = vec![source(&env, "docs/a.md"), source(&env, "docs/b.md")];
    let merged = "Intro\n## S1\none\n";
    let mut sections = ledger(&env, "docs/a.md", Some(("A-01", merged)));
    sections.extend(ledger(&env, "docs/b.md", None));
    input.actions = vec![
        ActionIn {
            content: Some(merged.into()),
            ..action("A-01", ActionKind::Replace, "docs/a.md")
        },
        ActionIn {
            absorbed_into: vec![whole("docs/a.md")],
            ..action("A-02", ActionKind::Remove, "docs/b.md")
        },
    ];
    input.sections = sections;
    propose(&env, "key-0026-gg", &input).unwrap();
    accept(&env, "CP-001", "rev1").unwrap();
    env.inject(Inject::AfterBody);
    assert_eq!(code(run_apply(&env, "CP-001")), "partial_publication");
    env.retry_all();
    // Another MCP operation rewrites the applied path after the effect was attested.
    env.start_call();
    let stale = env.observe("docs/a.md").unwrap().version;
    env.doc_op(
        "other:op",
        "someone",
        &DocOp::Save {
            path: "docs/a.md".into(),
            body: b"Intro\n## S1\nchanged\n".to_vec(),
            purpose: None,
            expected: stale,
        },
        &mut vec![],
    )
    .unwrap();
    env.finish_call(true);
    assert_eq!(code(run_apply(&env, "CP-001")), "post_apply_drift");
    assert!(
        env.body("docs/b.md").is_some(),
        "no removal runs while an applied action drifted"
    );
}

/// The removal barrier: older held replacements must be committed before any Remove; a plain fresh
/// call of the same shape needs no Retry.
#[test]
fn remove_waits_for_older_held_replacements() {
    let env = world();
    let merged = "Merged\n## S1\none\n## T1\nuno\n";
    let mut sections = ledger(&env, "docs/a.md", None);
    sections.extend(ledger(&env, "docs/b.md", None));
    let input = ProposalIn {
        title: "Merge".into(),
        sources: vec![source(&env, "docs/a.md"), source(&env, "docs/b.md")],
        actions: vec![
            ActionIn {
                content: Some(merged.into()),
                purpose: Some("merged".into()),
                ..action("A-01", ActionKind::Create, "docs/ab.md")
            },
            ActionIn {
                absorbed_into: vec![whole("docs/ab.md")],
                ..action("A-02", ActionKind::Remove, "docs/a.md")
            },
            ActionIn {
                absorbed_into: vec![whole("docs/ab.md")],
                ..action("A-03", ActionKind::Remove, "docs/b.md")
            },
        ],
        sections,
        preservation: vec![],
    };
    propose(&env, "key-0027-hh", &input).unwrap();
    accept(&env, "CP-001", "rev1").unwrap();
    // The first call creates the replacement and then fails while removing: everything is held.
    env.inject(Inject::AfterBody);
    assert_eq!(code(run_apply(&env, "CP-001")), "partial_publication");
    assert!(env.body("docs/ab.md").is_some());
    // The resume cannot commit a deletion only intent while the replacement stays held.
    assert_eq!(
        code(run_apply(&env, "CP-001")),
        "replacements_not_committed"
    );
    env.retry_all();
    let out = run_apply(&env, "CP-001").unwrap();
    assert_eq!(out.state, CpState::Applied);
    assert!(env.body("docs/a.md").is_none() && env.body("docs/b.md").is_none());
    assert!(env.pending().is_empty());
}

/// Untracked siblings and sync uncertainty make the current whole intent ineligible, but directory
/// effects never count as untracked files.
#[test]
fn current_intent_eligibility_ignores_directories() {
    let tracked = |t: TrackingView, durable: bool| EventView {
        relative: "x".into(),
        operation: None,
        intent: None,
        tracking: t,
        durable,
    };
    assert!(super::gate::current_eligible(&[
        tracked(TrackingView::Tracked, true),
        tracked(TrackingView::NotApplicable, true)
    ]));
    assert!(!super::gate::current_eligible(&[tracked(
        TrackingView::Untracked,
        true
    )]));
    assert!(!super::gate::current_eligible(&[tracked(
        TrackingView::NotApplicable,
        false
    )]));
    use super::gate::{Replacement, removal_gate};
    let ok = [("A-01".to_owned(), Replacement::Current)];
    assert!(removal_gate(&ok, true).is_ok());
    assert!(removal_gate(&ok, false).is_err());
    let held = [(
        "A-01".to_owned(),
        Replacement::Older {
            committed: false,
            intents: vec!["PG-1".into()],
        },
    )];
    let block = removal_gate(&held, true).unwrap_err();
    assert_eq!(block.intents, vec!["PG-1"]);
}

/// A new external reference to a source after acceptance is never ignored.
#[test]
fn new_incoming_reference_after_acceptance_blocks_apply() {
    let env = world();
    propose(&env, "key-0028-ii", &replace_a(&env)).unwrap();
    accept(&env, "CP-001", "rev1").unwrap();
    env.set_incoming(
        "docs/a.md",
        IncomingFacts {
            complete: true,
            gaps: vec![],
            rows: vec![IncomingRow {
                target: "docs/a.md".into(),
                kind: SourceKind::Markdown,
                source: "docs/other.md".into(),
                via: "link".into(),
                count: 1,
                fragments: vec![],
            }],
        },
    );
    assert_eq!(code(run_apply(&env, "CP-001")), "incoming_changed");
}

/// Unknown coverage refuses every destructive proposal at propose time.
#[test]
fn unknown_incoming_coverage_refuses_destructive_proposals() {
    let env = world();
    env.set_incoming(
        "docs/a.md",
        IncomingFacts {
            complete: false,
            gaps: vec!["modules/M-009.yaml: unreadable".into()],
            rows: vec![],
        },
    );
    assert_eq!(
        code(propose(&env, "key-0029-jj", &replace_a(&env))),
        "coverage_incomplete"
    );
    env.set_incoming(
        "docs/a.md",
        IncomingFacts {
            complete: true,
            gaps: vec![],
            rows: vec![],
        },
    );
    env.set_dangling(vec!["docs/z.md -> docs/a.md#s2".into()]);
    assert_eq!(
        code(propose(&env, "key-0030-kk", &replace_a(&env))),
        "incoming_unresolved"
    );
}

/// A record or project referrer to a path the proposal removes cannot be rewritten and refuses.
#[test]
fn record_referrer_to_a_removed_path_refuses() {
    let env = world();
    env.set_incoming(
        "docs/b.md",
        IncomingFacts {
            complete: true,
            gaps: vec![],
            rows: vec![IncomingRow {
                target: "docs/b.md".into(),
                kind: SourceKind::Knowledge,
                source: "D-001".into(),
                via: "record token".into(),
                count: 1,
                fragments: vec![],
            }],
        },
    );
    assert_eq!(
        code(propose(&env, "key-0031-ll", &merge_input(&env))),
        "incoming_unresolved"
    );
}

/// Withdraw before apply releases the claim; an applied proposal cannot be withdrawn.
#[test]
fn withdraw_rules() {
    let env = world();
    propose(&env, "key-0032-mm", &replace_a(&env)).unwrap();
    let v = version(&env, "CP-001");
    let out = call(&env, |fx| {
        ops::withdraw(&env, "author", &v, "CP-001", "no longer needed", false, fx)
    })
    .unwrap();
    assert_eq!(out.state, CpState::Withdrawn);
    assert!(
        propose(&env, "key-0033-nn", &replace_a(&env)).is_ok(),
        "the claim is released"
    );
    let v = version(&env, "CP-001");
    assert_eq!(
        code(call(&env, |fx| ops::withdraw(
            &env, "author", &v, "CP-001", "again", false, fx
        ))),
        "already_withdrawn"
    );
}

/// A maximal record stays within the nonterminal capacity.
#[test]
fn maximal_record_fits_the_capacity() {
    let mut body = ProposalBody::default();
    for i in 0..16 {
        body.sources.push(SourceObs {
            path: format!("docs/{}.md", "a".repeat(120)) + &i.to_string(),
            doc_id: Some("DOC-001".into()),
            version: "v".repeat(64),
            sha256: "s".repeat(64),
            len: 1,
            managed: true,
            record_path: Some("documents/DOC-001.yaml".into()),
            record_sha256: Some("r".repeat(64)),
            record_revision: Some(1),
            record_len: Some(1),
            move_basis: None,
        });
    }
    for i in 0..32 {
        body.actions.push(Action {
            id: format!("A-{:02}", i + 1),
            kind: ActionKind::Replace,
            path: format!("docs/{}.md", "b".repeat(120)),
            from: None,
            base_version: Some("v".repeat(64)),
            purpose: Some("p".repeat(240)),
            staged_sha256: Some("h".repeat(64)),
            staged_len: Some(1),
            reason: "r".repeat(512),
            absorbed_into: vec![],
            state: ActionState::Pending,
            publications: vec![],
        });
    }
    for i in 0..256 {
        body.sections.push(SectionEntry {
            id: format!("S-{:03}", i + 1),
            path: format!("docs/{}.md", "c".repeat(120)),
            section: SectionAddr::Heading {
                ordinal: i,
                level: 2,
                occurrence: 1,
                text_sha256: "t".repeat(64),
            },
            sha256: "s".repeat(64),
            disposition: Disposition::Dropped {
                kind: DropKind::Obsolete,
                reason: "r".repeat(512),
            },
        });
    }
    for i in 0..64 {
        body.preservation.push(Preservation {
            id: format!("P-{:02}", i + 1),
            kind: PreserveKind::Requirement,
            source_section: "S-001".into(),
            statement: "s".repeat(256),
            target: whole("docs/x.md"),
            mode: PreserveMode::Rewritten,
        });
    }
    let rec = CpRecord {
        schema_version: SCHEMA,
        id: "CP-001".into(),
        request_key: "k".repeat(64),
        state: CpState::Proposed,
        current: 1,
        revisions: vec![RevisionRecord {
            revision: 1,
            content_hash: "h".repeat(64),
            author: "a".into(),
            at: "t".into(),
            title: "t".into(),
            body,
            staged_dir: "compactions/CP-001/r1".into(),
        }],
        reviewer: None,
        reviews: vec![],
        accepted_review: None,
        apply: None,
        writes: 1,
        created_at: "t".into(),
        updated_at: "t".into(),
    };
    let bytes = encode_record(&rec, false).unwrap();
    assert!(
        bytes.len() < 480 * 1024,
        "one maximal revision uses {} bytes",
        bytes.len()
    );
    assert_eq!(decode_record(&bytes, "CP-001").unwrap(), rec);
}

// ------------------------------------------------------- post images (pure)

/// A frozen source observation fixture.
fn source_obs(managed: bool, id: Option<&str>, sha: &str, revision: Option<u64>) -> SourceObs {
    SourceObs {
        path: "docs/x.md".into(),
        doc_id: id.map(str::to_owned),
        version: "v".into(),
        sha256: sha.into(),
        len: 3,
        managed,
        record_path: id.map(|i| format!("documents/{i}.yaml")),
        record_sha256: managed.then(|| "r0".to_owned()),
        record_revision: revision,
        record_len: managed.then_some(9),
        move_basis: None,
    }
}

/// A pending action fixture of a kind.
fn action_of(kind: ActionKind, staged: Option<&str>) -> Action {
    Action {
        id: "A-01".into(),
        kind,
        path: "docs/x.md".into(),
        from: (kind == ActionKind::Move).then(|| "docs/old.md".to_owned()),
        base_version: Some("b".into()),
        purpose: None,
        staged_sha256: staged.map(str::to_owned),
        staged_len: staged.map(|_| 3),
        reason: "r".into(),
        absorbed_into: vec![],
        state: ActionState::Pending,
        publications: vec![],
    }
}

/// An asserted pending effect row fixture.
fn row(relative: &str, kind: EffectKindView, after: Option<&str>) -> EffectRow {
    EffectRow {
        relative: relative.into(),
        kind,
        before: None,
        after: after.map(str::to_owned),
        asserted: true,
        superseded_into: None,
        git: GitView::Pending("PG-1".into()),
    }
}

/// A document observation fixture.
fn doc(
    path: &str,
    state: DocState,
    body: Option<&str>,
    rec: Option<(&str, u64, &str)>,
) -> DocFacts {
    DocFacts {
        path: path.into(),
        state,
        version: "v".into(),
        body: body.map(|b| b.as_bytes().to_vec()),
        record: rec.map(|(id, revision, rec_sha)| RecordFacts {
            path: format!("documents/{id}.yaml"),
            id: id.into(),
            bound_path: path.into(),
            body_sha256: body
                .map(|b| record::sha256_hex(b.as_bytes()))
                .unwrap_or_default(),
            revision,
            sha256: rec_sha.into(),
            len: 9,
        }),
        retired: false,
    }
}

/// Kind specific post images: a Replace of a managed document needs the next record revision with its
/// exact record effect; the same body without its record is never complete.
#[test]
fn post_image_requires_body_and_exact_metadata() {
    use super::gate::{PostFacts, PostFail, verify_post};
    let sha = record::sha256_hex(b"new");
    let a = action_of(ActionKind::Replace, Some(&sha));
    let src = source_obs(true, Some("DOC-001"), "old", Some(1));
    let ok_rows = ReceiptView {
        rows: vec![
            row("docs/x.md", EffectKindView::Replaced, Some(&sha)),
            row(
                "documents/DOC-001.yaml",
                EffectKindView::Replaced,
                Some("rec2"),
            ),
        ],
        complete: true,
    };
    let managed = doc(
        "docs/x.md",
        DocState::Managed,
        Some("new"),
        Some(("DOC-001", 2, "rec2")),
    );
    let facts = PostFacts {
        dest: managed.clone(),
        from: None,
        by_id: None,
        allocation_ok: true,
    };
    assert_eq!(verify_post(&a, Some(&src), &facts, &ok_rows), Ok(()));
    // Same body, record never published: incomplete, never applied.
    let body_only = ReceiptView {
        rows: vec![ok_rows.rows[0].clone()],
        complete: true,
    };
    assert!(matches!(
        verify_post(&a, Some(&src), &facts, &body_only),
        Err(PostFail::Incomplete(_))
    ));
    // Wrong record revision, other identity and a record changed after the effect are metadata faults.
    let wrong_rev = PostFacts {
        dest: doc(
            "docs/x.md",
            DocState::Managed,
            Some("new"),
            Some(("DOC-001", 3, "rec2")),
        ),
        ..facts.clone()
    };
    assert!(matches!(
        verify_post(&a, Some(&src), &wrong_rev, &ok_rows),
        Err(PostFail::Metadata(_))
    ));
    let other_id = PostFacts {
        dest: doc(
            "docs/x.md",
            DocState::Managed,
            Some("new"),
            Some(("DOC-009", 2, "rec2")),
        ),
        ..facts.clone()
    };
    assert!(matches!(
        verify_post(&a, Some(&src), &other_id, &ok_rows),
        Err(PostFail::Metadata(_))
    ));
    let changed = PostFacts {
        dest: doc(
            "docs/x.md",
            DocState::Managed,
            Some("new"),
            Some(("DOC-001", 2, "other")),
        ),
        ..facts.clone()
    };
    assert!(matches!(
        verify_post(&a, Some(&src), &changed, &ok_rows),
        Err(PostFail::Metadata(_))
    ));
    // A body that differs now, or an effect superseded by a later publication, is drift.
    let drifted = PostFacts {
        dest: doc(
            "docs/x.md",
            DocState::Managed,
            Some("later"),
            Some(("DOC-001", 2, "rec2")),
        ),
        ..facts.clone()
    };
    assert!(matches!(
        verify_post(&a, Some(&src), &drifted, &ok_rows),
        Err(PostFail::Drift(_))
    ));
    let mut superseded = ok_rows.clone();
    superseded.rows[0].superseded_into = Some(record::sha256_hex(b"later"));
    assert!(matches!(
        verify_post(&a, Some(&src), &facts, &superseded),
        Err(PostFail::Drift(_))
    ));
    // An incomplete receipt never completes.
    let truncated = ReceiptView {
        complete: false,
        ..ok_rows
    };
    assert!(matches!(
        verify_post(&a, Some(&src), &facts, &truncated),
        Err(PostFail::Incomplete(_))
    ));
}

/// Remove ends Absent; a managed source ends Retired with the same identity and its own record
/// effect, an unmanaged source gains no record, and Remove is never required to be Managed.
#[test]
fn remove_post_images_differ_for_managed_and_unmanaged() {
    use super::gate::{PostFacts, PostFail, verify_post};
    let a = action_of(ActionKind::Remove, None);
    let managed = source_obs(true, Some("DOC-001"), "oldsha", Some(1));
    let mut retired = doc(
        "docs/x.md",
        DocState::Retired,
        None,
        Some(("DOC-001", 2, "rec2")),
    );
    retired.retired = true;
    if let Some(r) = retired.record.as_mut() {
        r.body_sha256 = "oldsha".into();
    }
    let absent = doc("docs/x.md", DocState::Absent, None, None);
    let rows = ReceiptView {
        rows: vec![
            row("docs/x.md", EffectKindView::Removed, None),
            row(
                "documents/DOC-001.yaml",
                EffectKindView::Replaced,
                Some("rec2"),
            ),
        ],
        complete: true,
    };
    let facts = PostFacts {
        dest: absent.clone(),
        from: None,
        by_id: Some(retired.clone()),
        allocation_ok: true,
    };
    assert_eq!(verify_post(&a, Some(&managed), &facts, &rows), Ok(()));
    // Not retired yet: the step stays partial.
    let live = PostFacts {
        by_id: Some(doc(
            "docs/x.md",
            DocState::Absent,
            None,
            Some(("DOC-001", 1, "r0")),
        )),
        ..facts.clone()
    };
    assert!(matches!(
        verify_post(&a, Some(&managed), &live, &rows),
        Err(PostFail::Incomplete(_))
    ));
    // Unmanaged: no record row and no record are expected.
    let unmanaged = source_obs(false, None, "oldsha", None);
    let body_row = ReceiptView {
        rows: vec![rows.rows[0].clone()],
        complete: true,
    };
    let bare = PostFacts {
        dest: absent.clone(),
        from: None,
        by_id: None,
        allocation_ok: true,
    };
    assert_eq!(verify_post(&a, Some(&unmanaged), &bare, &body_row), Ok(()));
    // A synthetic record for an unmanaged remove is a metadata fault.
    let synthetic = PostFacts {
        by_id: Some(retired),
        ..bare
    };
    assert!(matches!(
        verify_post(&a, Some(&unmanaged), &synthetic, &body_row),
        Err(PostFail::Metadata(_))
    ));
}

/// A managed Move keeps the source DOC identity at the destination and leaves the source absent; an
/// unmanaged Move gains no record.
#[test]
fn move_post_image_keeps_the_source_identity() {
    use super::gate::{PostFacts, PostFail, verify_post};
    let sha = record::sha256_hex(b"body");
    let a = action_of(ActionKind::Move, None);
    let src = source_obs(true, Some("DOC-001"), &sha, Some(1));
    let rows = ReceiptView {
        rows: vec![
            row("docs/x.md", EffectKindView::Created, Some(&sha)),
            row("docs/old.md", EffectKindView::Removed, None),
            row(
                "documents/DOC-001.yaml",
                EffectKindView::Replaced,
                Some("rec2"),
            ),
        ],
        complete: true,
    };
    let dest = doc(
        "docs/x.md",
        DocState::Managed,
        Some("body"),
        Some(("DOC-001", 2, "rec2")),
    );
    let from = doc("docs/old.md", DocState::Absent, None, None);
    let facts = PostFacts {
        dest,
        from: Some(from.clone()),
        by_id: None,
        allocation_ok: true,
    };
    assert_eq!(verify_post(&a, Some(&src), &facts, &rows), Ok(()));
    // A new identity at the destination (recreate or adopt) is refused.
    let recreated = PostFacts {
        dest: doc(
            "docs/x.md",
            DocState::Managed,
            Some("body"),
            Some(("DOC-002", 1, "rec2")),
        ),
        ..facts.clone()
    };
    assert!(matches!(
        verify_post(&a, Some(&src), &recreated, &rows),
        Err(PostFail::Metadata(_))
    ));
    // The source still present is drift, not success.
    let present = PostFacts {
        from: Some(doc("docs/old.md", DocState::Unmanaged, Some("body"), None)),
        ..facts.clone()
    };
    assert!(matches!(
        verify_post(&a, Some(&src), &present, &rows),
        Err(PostFail::Drift(_))
    ));
    // Unmanaged move: destination Unmanaged, no record.
    let usrc = source_obs(false, None, &sha, None);
    let urows = ReceiptView {
        rows: rows.rows[..2].to_vec(),
        complete: true,
    };
    let udest = PostFacts {
        dest: doc("docs/x.md", DocState::Unmanaged, Some("body"), None),
        from: Some(from),
        by_id: None,
        allocation_ok: true,
    };
    assert_eq!(verify_post(&a, Some(&usrc), &udest, &urows), Ok(()));
}

// -------------------------------------------------------- more apply controls

/// Staged blobs are never adopted by equal bytes: an existing file at a stage path refuses unless the
/// oracle attests this exact operation.
#[test]
fn equal_staged_bytes_prove_content_not_ownership() {
    let env = world();
    propose(&env, "key-0040-aa", &replace_a(&env)).unwrap();
    let mut next = replace_a(&env);
    next.title = "Trim A v2".into();
    next.actions[0].content = Some("Intro\n## S1\none\nplus\n".into());
    next.sections = ledger(
        &env,
        "docs/a.md",
        Some(("A-01", "Intro\n## S1\none\nplus\n")),
    );
    // A foreign file with exactly the bytes of the next staged blob already sits at the stage path.
    let dir = env.store().path("compactions/CP-001/r2").unwrap();
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("A-01.md"), b"Intro\n## S1\none\nplus\n").unwrap();
    let v = version(&env, "CP-001");
    let before = env
        .store()
        .bytes("compactions/CP-001.yaml")
        .unwrap()
        .unwrap();
    assert_eq!(
        code(call(&env, |fx| ops::revise(
            &env, "author", &v, "CP-001", &next, fx
        ))),
        "stage_unowned"
    );
    assert_eq!(
        env.store()
            .bytes("compactions/CP-001.yaml")
            .unwrap()
            .unwrap(),
        before
    );
}

/// The first propose that creates the whole home commits its owned files; directory effects are not
/// owned files, so only a real untracked sibling defers the whole call while the bytes stay saved.
#[test]
fn first_propose_commits_owned_files_and_only_untracked_siblings_defer() {
    let env = world();
    propose(&env, "key-0041-aa", &replace_a(&env)).unwrap();
    assert_eq!(
        env.committed_rows(),
        2,
        "record and blob rows are committed after the call"
    );
    assert!(env.pending().is_empty());
    env.untracked_sibling(true);
    let mut other = replace_a(&env);
    other.sources = vec![source(&env, "docs/b.md")];
    other.actions = vec![ActionIn {
        content: Some("Bee\n## T1\nuno\nplus\n".into()),
        purpose: Some("b".into()),
        ..action("A-01", ActionKind::Replace, "docs/b.md")
    }];
    other.sections = ledger(&env, "docs/b.md", Some(("A-01", "Bee\n## T1\nuno\nplus\n")));
    propose(&env, "key-0042-bb", &other).unwrap();
    assert!(
        !env.pending().is_empty(),
        "an untracked sibling defers the whole call"
    );
    assert!(
        env.store()
            .bytes("compactions/CP-002.yaml")
            .unwrap()
            .is_some(),
        "the saved bytes survive"
    );
}

/// A resumed Move whose record already moved finishes only the source removal, with the same identity.
#[test]
fn move_resumes_after_the_record_moved() {
    let env = world();
    env.seed("docs/m1.md", "Bee\n## M1\nm\n", true);
    let m1_id = env.doc_for("docs/m1.md").unwrap().0;
    let sections = outline_of(&env.observe("docs/m1.md").unwrap().body.unwrap())
        .sections
        .into_iter()
        .map(|s| SectionIn {
            path: "docs/m1.md".into(),
            section: s.addr,
            disposition: Disposition::Moved {
                action: "A-01".into(),
            },
        })
        .collect();
    let input = ProposalIn {
        title: "Move".into(),
        sources: vec![source(&env, "docs/m1.md")],
        actions: vec![ActionIn {
            from: Some("docs/m1.md".into()),
            ..action("A-01", ActionKind::Move, "docs/m1b.md")
        }],
        sections,
        preservation: vec![],
    };
    propose(&env, "key-0043-cc", &input).unwrap();
    accept(&env, "CP-001", "rev1").unwrap();
    env.inject(Inject::AfterRecord);
    assert_eq!(code(run_apply(&env, "CP-001")), "partial_publication");
    assert_eq!(
        code(run_apply(&env, "CP-001")),
        "replacements_not_committed"
    );
    env.retry_all();
    assert_eq!(run_apply(&env, "CP-001").unwrap().state, CpState::Applied);
    assert_eq!(env.doc_for("docs/m1b.md").unwrap().0, m1_id);
    assert!(env.body("docs/m1.md").is_none());
}

/// A removal interrupted after the body was removed resumes by retiring the record.
#[test]
fn remove_resumes_by_retiring_the_record() {
    let env = world();
    let sections = ledger(&env, "docs/a.md", None);
    let input = ProposalIn {
        title: "Drop A".into(),
        sources: vec![source(&env, "docs/a.md")],
        actions: vec![action("A-01", ActionKind::Remove, "docs/a.md")],
        sections,
        preservation: vec![],
    };
    propose(&env, "key-0044-dd", &input).unwrap();
    accept(&env, "CP-001", "rev1").unwrap();
    env.inject(Inject::AfterBody);
    assert_eq!(code(run_apply(&env, "CP-001")), "partial_publication");
    assert!(env.body("docs/a.md").is_none() && !env.doc_for("docs/a.md").unwrap().2);
    assert_eq!(
        code(run_apply(&env, "CP-001")),
        "replacements_not_committed"
    );
    env.retry_all();
    assert_eq!(run_apply(&env, "CP-001").unwrap().state, CpState::Applied);
    assert!(env.doc_for("docs/a.md").unwrap().2, "the record is retired");
}

/// An applied proposal reports later drift honestly and never claims the current state verified.
#[test]
fn applied_repeat_reports_later_drift() {
    let env = world();
    propose(&env, "key-0045-ee", &replace_a(&env)).unwrap();
    accept(&env, "CP-001", "rev1").unwrap();
    assert_eq!(run_apply(&env, "CP-001").unwrap().state, CpState::Applied);
    env.dirty("docs/a.md", "Intro\n## S1\nedited natively\n");
    let again = run_apply(&env, "CP-001").unwrap();
    assert!(!again.changed);
    assert!(
        again.notes.iter().any(|n| n.contains("post_apply_drift")),
        "{:?}",
        again.notes
    );
    assert!(!again.notes.iter().any(|n| n.contains("verified")));
}

/// A real external metadata edit changes the accepted source digest: nothing of the observation is
/// ignored, while the proposal's own progress is.
#[test]
fn external_metadata_edit_after_acceptance_blocks_the_first_apply() {
    let env = world();
    propose(&env, "key-0046-ff", &replace_a(&env)).unwrap();
    accept(&env, "CP-001", "rev1").unwrap();
    env.bump_record("docs/a.md");
    assert_eq!(code(run_apply(&env, "CP-001")), "stale_source");
    assert_eq!(
        env.body("docs/a.md").unwrap(),
        A_BODY.as_bytes(),
        "no effect ran"
    );
}

/// A replacement whose content equals the current document changes nothing and is refused at proposal
/// time, so it can never leave an unattested equal-bytes look-alike for the oracle to doubt.
#[test]
fn replace_with_identical_content_is_refused() {
    let env = world();
    let mut input = replace_a(&env);
    input.actions[0].content = Some(A_BODY.into());
    input.sections = ledger(&env, "docs/a.md", Some(("A-01", A_BODY)));
    assert_eq!(
        code(propose(&env, "key-0050-aa", &input)),
        "invalid_arguments"
    );
}

/// A held intent on paths a proposal never touches does not block it, so the barrier is scoped to the
/// proposal's own documents, records and staged blobs.
#[test]
fn an_unrelated_held_intent_does_not_block_another_proposal() {
    let env = world();
    propose(&env, "key-0051-aa", &replace_a(&env)).unwrap();
    accept(&env, "CP-001", "rev1").unwrap();
    let mut other = replace_a(&env);
    other.sources = vec![source(&env, "docs/b.md")];
    other.actions = vec![ActionIn {
        content: Some("Bee\n## T1\nuno\nplus\n".into()),
        purpose: Some("b".into()),
        ..action("A-01", ActionKind::Replace, "docs/b.md")
    }];
    other.sections = ledger(&env, "docs/b.md", Some(("A-01", "Bee\n## T1\nuno\nplus\n")));
    propose(&env, "key-0052-bb", &other).unwrap();
    accept(&env, "CP-002", "rev1").unwrap();
    env.inject(Inject::AfterBody);
    assert_eq!(code(run_apply(&env, "CP-002")), "partial_publication");
    assert_eq!(run_apply(&env, "CP-001").unwrap().state, CpState::Applied);
    assert_eq!(
        code(run_apply(&env, "CP-002")),
        "replacements_not_committed"
    );
}

/// The removal gate also guards the fresh call itself: when an eligible replacement of this very call
/// carries no journal entry, settlement would hold the whole intent, so no removal may run after it and
/// the original documents stay.
#[test]
fn removal_waits_when_the_current_intent_cannot_commit_atomically() {
    let env = world();
    propose(&env, "key-0053-aa", &merge_input(&env)).unwrap();
    accept(&env, "CP-001", "rev1").unwrap();
    env.untracked_events(true);
    assert_eq!(
        code(run_apply(&env, "CP-001")),
        "replacements_not_committed"
    );
    assert!(env.body("docs/a.md").is_some() && env.body("docs/b.md").is_some());
    assert!(
        env.body("docs/ab.md").is_some(),
        "the replacement was published"
    );
}

// ------------------------------------------------------- retained reads (revision 7)

/// A revise of [`replace_a`] whose candidate differs from revision one, so a read of the wrong
/// revision is visible in the bytes.
fn revise_a(env: &FakeEnv) {
    let candidate = "Intro\n## S1\none\nplus\n";
    let mut next = replace_a(env);
    next.title = "Trim A again".into();
    next.actions[0].content = Some(candidate.into());
    next.sections = ledger(env, "docs/a.md", Some(("A-01", candidate)));
    let v = version(env, "CP-001");
    call(env, |fx| {
        ops::revise(env, "author", &v, "CP-001", &next, fx)
    })
    .unwrap();
}

/// Revision selection returns exactly the named retained revision and refuses zero and unretained
/// numbers naming `revision` with the retained range.
#[test]
fn retained_revision_selects_exactly_the_named_revision() {
    let env = world();
    propose(&env, "key-0090-aa", &replace_a(&env)).unwrap();
    accept(&env, "CP-001", "rev1").unwrap();
    revise_a(&env);
    let rec = super::read_cp(env.store(), "CP-001").unwrap().value;
    assert_eq!(rec.current, 2);
    // The review bound to revision one is retained and still names exactly revision one's hash.
    let bound: Vec<_> = rec.reviews.iter().filter(|r| r.revision == 1).collect();
    assert_eq!(bound.len(), 1);
    assert_eq!(
        bound[0].content_hash,
        super::retained_revision(&rec, 1).unwrap().content_hash
    );
    for n in [1, 2] {
        assert_eq!(super::retained_revision(&rec, n).unwrap().revision, n);
    }
    assert_eq!(
        super::retained_revision(&rec, 1).unwrap(),
        &rec.revisions[0]
    );
    assert_ne!(rec.revisions[0].title, rec.revisions[1].title);
    for n in [0, 3, u32::MAX] {
        let e = super::retained_revision(&rec, n).unwrap_err();
        assert_eq!(e.code, "invalid_arguments");
        assert!(
            e.message.starts_with("revision:") && e.message.contains("1 to 2"),
            "{}",
            e.message
        );
    }
}

/// An earlier revision still yields its own verified bytes after a revise, equal to what was staged,
/// and the live document and the tree stay untouched by the read.
#[test]
fn read_staged_returns_earlier_revisions_exactly() {
    let env = world();
    propose(&env, "key-0091-aa", &replace_a(&env)).unwrap();
    revise_a(&env);
    let snap = super::read_cp(env.store(), "CP-001").unwrap();
    let one = super::read_staged(env.store(), &snap.value, 1, "A-01").unwrap();
    let two = super::read_staged(env.store(), &snap.value, 2, "A-01").unwrap();
    assert_eq!(one.bytes, b"Intro\n## S1\none\n");
    assert_eq!(two.bytes, b"Intro\n## S1\none\nplus\n");
    assert_eq!(
        (
            one.revision,
            one.action.as_str(),
            one.kind,
            one.path.as_str()
        ),
        (1, "A-01", ActionKind::Replace, "docs/a.md")
    );
    assert_eq!(one.sha256, record::sha256_hex(&one.bytes));
    assert_ne!(one.sha256, two.sha256);
    // A live edit changes no candidate byte and a read repairs or writes nothing.
    env.dirty("docs/a.md", "Changed live\n");
    let again = super::read_staged(env.store(), &snap.value, 1, "A-01").unwrap();
    assert_eq!(again, one);
    assert_eq!(version(&env, "CP-001"), snap.version);
}

/// Action identifiers are unique but not dense: A-32 alone is readable and the others are absent.
#[test]
fn read_staged_accepts_sparse_action_identifiers() {
    let env = world();
    let mut input = replace_a(&env);
    input.actions[0].id = "A-32".into();
    input.sections = ledger(&env, "docs/a.md", Some(("A-32", "Intro\n## S1\none\n")));
    propose(&env, "key-0092-aa", &input).unwrap();
    let rec = super::read_cp(env.store(), "CP-001").unwrap().value;
    let got = super::read_staged(env.store(), &rec, 1, "A-32").unwrap();
    assert_eq!(got.bytes, b"Intro\n## S1\none\n");
    for id in ["A-01", "A-31"] {
        let e = super::read_staged(env.store(), &rec, 1, id).unwrap_err();
        assert_eq!(e.code, "invalid_arguments");
        assert!(e.message.starts_with("action:"), "{}", e.message);
    }
}

/// Non canonical, absent and candidate-less action selections refuse naming `action`; a bad revision
/// refuses naming `revision` first.
#[test]
fn read_staged_refuses_bad_selectors() {
    let env = world();
    propose(&env, "key-0093-aa", &merge_input(&env)).unwrap();
    let rec = super::read_cp(env.store(), "CP-001").unwrap().value;
    assert!(super::read_staged(env.store(), &rec, 1, "A-01").is_ok());
    for id in [
        "", "A-1", "a-01", "A-00", "A-33", "A-001", "A-+1", " A-01", "A-01 ", "A-04", "CP-001",
        "../A-01",
    ] {
        let e = super::read_staged(env.store(), &rec, 1, id).unwrap_err();
        assert_eq!(
            (e.code, e.message.starts_with("action:")),
            ("invalid_arguments", true),
            "{id:?}: {}",
            e.message
        );
    }
    for id in ["A-02", "A-03"] {
        let e = super::read_staged(env.store(), &rec, 1, id).unwrap_err();
        assert_eq!(e.code, "invalid_arguments");
        assert!(e.message.starts_with("action:") && e.message.contains("Remove"));
    }
    let e = super::read_staged(env.store(), &rec, 2, "A-99").unwrap_err();
    assert!(e.message.starts_with("revision:"), "{}", e.message);
}

/// A missing blob, a corrupt blob of the same length, a different length and an oversize file all
/// refuse `invalid_data` with the relative path, and apply preparation refuses identically because
/// both use the one verifier.
#[test]
fn read_staged_verifies_blob_hash_and_length() {
    let env = world();
    propose(&env, "key-0094-aa", &replace_a(&env)).unwrap();
    let rec = super::read_cp(env.store(), "CP-001").unwrap().value;
    let relative = record::stage_path("CP-001", 1, "A-01");
    let path = env.store().path(&relative).unwrap();
    let good = std::fs::read(&path).unwrap();
    let mismatch = format!("{relative}: staged candidate does not match its recorded hash.");
    let check = |expected: &str| {
        let e = super::read_staged(env.store(), &rec, 1, "A-01").unwrap_err();
        assert_eq!((e.code, e.message.as_str()), ("invalid_data", expected));
        let e = ops::load_blobs(&env, &rec).unwrap_err();
        assert_eq!((e.code, e.message.as_str()), ("invalid_data", expected));
    };
    let mut same_length = good.clone();
    same_length[0] ^= 1;
    std::fs::write(&path, &same_length).unwrap();
    check(&mismatch);
    std::fs::write(&path, [good.as_slice(), b"x"].concat()).unwrap();
    check(&mismatch);
    std::fs::write(&path, vec![b'x'; BLOB_CAP + 1]).unwrap();
    check(&mismatch);
    std::fs::remove_file(&path).unwrap();
    check(&format!("{relative}: staged candidate is missing."));
    // Correct bytes whose recorded length disagrees are refused even though the hash matches.
    std::fs::write(&path, &good).unwrap();
    let mut wrong_len = rec.clone();
    wrong_len.revisions[0].body.actions[0].staged_len = Some(good.len() as u64 + 1);
    let e = super::read_staged(env.store(), &wrong_len, 1, "A-01").unwrap_err();
    assert_eq!(
        (e.code, e.message.as_str()),
        ("invalid_data", mismatch.as_str())
    );
    assert!(super::read_staged(env.store(), &rec, 1, "A-01").is_ok());
}
