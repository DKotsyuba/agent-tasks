//! Composed controls: the compaction domain on the real document, reference, knowledge, store and
//! persistence providers in disposable isolated Git repositories.
//!
//! Each step runs like one dispatcher request: a fresh request store, the root write lock, the live
//! environment, then the production commit policy settles the call. Scripted faults wrap only the real
//! store port's `put` and `del` (test builds only). Nothing here touches a configured documentation
//! root, and nothing is an MCP end to end proof because the tool is not registered.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "Explicit isolated fixture assertions"
)]
use super::{
    apply::apply,
    env::{DocOp, DocState, Env, whole},
    live::{Fault, LiveEnv},
    ops::{self, Outcome},
    record::{ActionKind, CpState, Disposition, DropKind},
    review::ReviewIn,
    validate::{ActionIn, ProposalIn, SectionIn, SourceIn},
};
use crate::{
    model::Verdict,
    persist::{
        EventClass, GitOutcome, GitReceipt,
        policy::{Event, EventOutcome},
        production_policy, settle,
        testing::GitFixture,
    },
    store::{Result, Store},
};

/// A disposable repository with a project manifest, committed.
struct Repo {
    /// The isolated Git fixture.
    fx: GitFixture,
}

/// Body of a managed document with a preamble and two sections.
const A_BODY: &str = "Intro\n## S1\none\n## S2\ntwo\n";
/// Body of a second document.
const M_BODY: &str = "Mover\n## M1\nmoved\n";
/// Body of an unmanaged document.
const B_BODY: &str = "Bee\n## T1\nuno\n";

impl Repo {
    /// Create the repository, the manifest and the allocator inputs, then commit them.
    fn new() -> Self {
        let fx = GitFixture::new();
        let now = crate::store::now();
        std::fs::write(
            fx.dir.path().join("project.yaml"),
            format!(
                "schema_version: 1\ntitle: Fixture\npurpose: Composed controls\ncreated_at: {now}\nupdated_at: {now}\n"
            ),
        )
        .unwrap();
        std::fs::write(
            fx.dir.path().join(".agent-tasks/state.yaml"),
            "schema_version: 2\nnext_module: 1\nnext_epic: 1\nnext_atomic: 1\n",
        )
        .unwrap();
        fx.git(&["add", "--", "project.yaml", ".agent-tasks/state.yaml"]);
        fx.git(&["commit", "--quiet", "-m", "fixture: project"]);
        Self { fx }
    }

    /// One dispatcher-like request: lock, run, judge the outcome, settle with the production policy.
    fn step<T>(
        &self,
        class: EventClass,
        faults: Vec<Fault>,
        body: impl FnOnce(&LiveEnv<'_>, &mut Vec<String>) -> Result<T>,
    ) -> (Result<T>, GitReceipt) {
        let store = Store::from_root(self.fx.dir.path()).unwrap();
        let guard = store.lock(true, &mut Vec::new()).unwrap().unwrap();
        let env = LiveEnv::new(&store, &guard).unwrap();
        *env.faults.borrow_mut() = faults;
        let mut effects = Vec::new();
        let result = body(&env, &mut effects);
        let outcome = match (&result, store.publications().is_empty()) {
            (Ok(_), _) => EventOutcome::Success,
            (Err(_), false) => EventOutcome::Partial,
            (Err(_), true) => EventOutcome::Failed,
        };
        let receipt = if outcome == EventOutcome::Failed {
            GitReceipt::saved_only()
        } else {
            settle(
                &store,
                &guard,
                &Event {
                    class,
                    refs: vec![],
                    operation: None,
                    outcome,
                },
                production_policy(),
            )
        };
        (result, receipt)
    }

    /// Create a managed document through the document owner and commit it.
    fn seed(&self, path: &str, body: &str) {
        let (result, receipt) = self.step(EventClass::Document, vec![], |env, fx| {
            let expected = env.observe(path)?.version;
            env.doc_op(
                &format!("seed:{}", path.replace('/', "-")),
                "seeder",
                &DocOp::Save {
                    path: path.into(),
                    body: body.as_bytes().to_vec(),
                    purpose: Some("fixture".into()),
                    expected,
                },
                fx,
            )
        });
        result.unwrap();
        assert_eq!(receipt.outcome, GitOutcome::Committed, "{receipt:?}");
    }

    /// Create an unmanaged document natively and commit it.
    fn seed_unmanaged(&self, path: &str, body: &str, commit: bool) {
        let full = self.fx.dir.path().join(path);
        std::fs::create_dir_all(full.parent().unwrap()).unwrap();
        std::fs::write(&full, body).unwrap();
        if commit {
            self.fx.git(&["add", "--", path]);
            self.fx
                .git(&["commit", "--quiet", "-m", "fixture: native document"]);
        }
    }

    /// Current exact bytes of a native path, `None` when absent.
    fn bytes(&self, path: &str) -> Option<Vec<u8>> {
        std::fs::read(self.fx.dir.path().join(path)).ok()
    }

    /// Current record version of one proposal.
    fn cp_version(&self, cp: &str) -> String {
        let store = Store::from_root(self.fx.dir.path()).unwrap();
        super::read_cp(&store, cp).unwrap().version
    }

    /// The decoded proposal record.
    fn cp(&self, cp: &str) -> super::record::CpRecord {
        let store = Store::from_root(self.fx.dir.path()).unwrap();
        super::read_cp(&store, cp).unwrap().value
    }

    /// Propose through the real operation at the current allocation version.
    fn propose(
        &self,
        key: &str,
        build: impl FnOnce(&LiveEnv<'_>) -> ProposalIn,
    ) -> Result<Outcome> {
        self.step(EventClass::CompactionPropose, vec![], |env, fx| {
            let input = build(env);
            let v = env.allocation_version()?;
            ops::propose(env, "author", &v, key, &input, fx)
        })
        .0
    }

    /// Accept the current revision as an independent reviewer who verifies every item.
    fn accept(&self, cp: &str) -> Result<Outcome> {
        let snap = {
            let store = Store::from_root(self.fx.dir.path()).unwrap();
            super::read_cp(&store, cp).unwrap()
        };
        let rec = &snap.value;
        let input = ReviewIn {
            revision: rec.current,
            content_hash: rec.revision().unwrap().content_hash.clone(),
            verdict: Verdict::Accepted,
            summary: "verified".into(),
            verified_items: super::review::review_items(rec)
                .unwrap()
                .into_iter()
                .collect(),
            findings: vec![],
            resolved: vec![],
        };
        self.step(EventClass::CompactionReview, vec![], |env, fx| {
            ops::review_cp(env, "reviewer", &snap.version, cp, &input, fx)
        })
        .0
    }

    /// Explicit Git recovery: commit every held or published pending intent together, as the recovery
    /// tool would under the dispatcher lock.
    fn retry_pending(&self) -> GitReceipt {
        let store = Store::from_root(self.fx.dir.path()).unwrap();
        let guard = store.lock(true, &mut Vec::new()).unwrap().unwrap();
        let pending = crate::persist::pending(&store);
        let ids: Vec<String> = pending.refs.iter().map(|r| r.intent.clone()).collect();
        assert!(!ids.is_empty(), "nothing is pending");
        crate::persist::recover::recover(
            &store,
            &guard,
            &pending.version,
            crate::persist::recover::Action::Retry(ids),
        )
        .unwrap()
        .receipt
    }

    /// Intents still pending after the calls so far.
    fn pending_intents(&self) -> usize {
        let store = Store::from_root(self.fx.dir.path()).unwrap();
        crate::persist::pending(&store).refs.len()
    }

    /// Apply as one dispatcher request, with scripted faults.
    fn apply(&self, cp: &str, faults: Vec<Fault>) -> (Result<Outcome>, GitReceipt) {
        let v = self.cp_version(cp);
        self.step(EventClass::CompactionApply, faults, |env, fx| {
            apply(env, "applier", &v, cp, fx)
        })
    }
}

/// Stable error code of a result, or `ok`.
fn code<T>(r: &Result<T>) -> &'static str {
    r.as_ref().err().map_or("ok", |e| e.code)
}

/// One scripted fault.
fn fault(on: &'static str, rel: &str, code: &'static str, after_effect: bool) -> Fault {
    Fault {
        on,
        rel: rel.into(),
        skip: 0,
        code,
        after_effect,
    }
}

/// A source observation of one path.
fn source(env: &LiveEnv<'_>, path: &str) -> SourceIn {
    SourceIn {
        path: path.into(),
        version: env.observe(path).unwrap().version,
    }
}

/// Ledger over a source: every section kept when its exact bytes occur in `candidate`, else moved
/// into `moved` when given, else dropped as obsolete.
fn ledger(
    env: &LiveEnv<'_>,
    path: &str,
    candidate: Option<(&str, &str)>,
    moved: Option<&str>,
) -> Vec<SectionIn> {
    let body = env.observe(path).unwrap().body.unwrap();
    env.outline(&body)
        .unwrap()
        .sections
        .into_iter()
        .map(|s| {
            let kept = candidate.filter(|(_, c)| {
                c.as_bytes()
                    .windows(s.bytes.len())
                    .any(|w| w == s.bytes.as_slice())
            });
            SectionIn {
                path: path.into(),
                section: s.addr,
                disposition: match (kept, moved) {
                    (Some((action, _)), _) => Disposition::Kept {
                        action: action.into(),
                    },
                    (None, Some(action)) => Disposition::Moved {
                        action: action.into(),
                    },
                    (None, None) => Disposition::Dropped {
                        kind: DropKind::Obsolete,
                        reason: "no longer needed".into(),
                    },
                },
            }
        })
        .collect()
}

/// Ledger over a source whose every section is merged (rewritten) into a whole target document.
fn merged_ledger(env: &LiveEnv<'_>, path: &str, target: &str) -> Vec<SectionIn> {
    let body = env.observe(path).unwrap().body.unwrap();
    env.outline(&body)
        .unwrap()
        .sections
        .into_iter()
        .map(|s| SectionIn {
            path: path.into(),
            section: s.addr,
            disposition: Disposition::Merged {
                target: whole(target),
            },
        })
        .collect()
}

/// A bare action.
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

/// Replace of `docs/a.md` keeping its preamble and first section.
fn replace_a(env: &LiveEnv<'_>) -> ProposalIn {
    let candidate = "Intro\n## S1\none\n";
    ProposalIn {
        title: "Trim A".into(),
        sources: vec![source(env, "docs/a.md")],
        actions: vec![ActionIn {
            content: Some(candidate.into()),
            ..action("A-01", ActionKind::Replace, "docs/a.md")
        }],
        sections: ledger(env, "docs/a.md", Some(("A-01", candidate)), None),
        preservation: vec![],
    }
}

/// The record of a git commit message and its file list, newest commit.
fn last_commit(repo: &Repo) -> (String, Vec<String>) {
    let message = repo.fx.git(&["log", "-1", "--format=%B"]);
    let files = repo
        .fx
        .git(&["show", "--name-only", "--format=", "HEAD"])
        .lines()
        .map(str::to_owned)
        .collect();
    (message, files)
}

/// A replace of a managed document runs through the real document owner: body and metadata in one
/// attested intent, the production policy commits it once, and the committed original stays
/// recoverable from Git.
#[test]
fn replace_applies_on_the_real_providers_and_commits_one_intent() {
    let repo = Repo::new();
    repo.seed("docs/a.md", A_BODY);
    let original_commit = repo.fx.git(&["rev-parse", "HEAD"]);
    repo.propose("key-real-0001", replace_a).unwrap();
    repo.accept("CP-001").unwrap();
    let (out, receipt) = repo.apply("CP-001", vec![]);
    let out = out.unwrap();
    assert_eq!(out.state, CpState::Applied, "{out:?}");
    assert_eq!(receipt.outcome, GitOutcome::Committed, "{receipt:?}");
    assert_eq!(repo.bytes("docs/a.md").unwrap(), b"Intro\n## S1\none\n");
    let (message, files) = last_commit(&repo);
    assert!(message.contains("compaction-apply"), "{message}");
    for expected in [
        "docs/a.md",
        "documents/DOC-001.yaml",
        "compactions/CP-001.yaml",
    ] {
        assert!(files.iter().any(|f| f == expected), "{files:?}");
    }
    let original = repo
        .fx
        .git(&["show", &format!("{original_commit}:docs/a.md")]);
    assert_eq!(original, A_BODY.trim_end());
    let rec = repo.cp("CP-001");
    let mut paths: Vec<&str> = rec
        .apply
        .as_ref()
        .unwrap()
        .originals
        .iter()
        .map(|o| o.relative.as_str())
        .collect();
    paths.sort_unstable();
    assert_eq!(paths, ["docs/a.md", "documents/DOC-001.yaml"]);
}

/// A merge of a managed and an unmanaged document into a new one.
fn merge_input(env: &LiveEnv<'_>) -> ProposalIn {
    let merged = "Merged\n## S1\none\n## T1\nuno\n";
    let mut sections = merged_ledger(env, "docs/a.md", "docs/ab.md");
    sections.extend(merged_ledger(env, "docs/b.md", "docs/ab.md"));
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

/// Fresh Create plus managed and unmanaged Remove through the real owners in one call: one whole
/// atomic intent holds the new body, its fresh DOC record, both removals and the retirement, and the
/// production policy commits all of it once.
#[test]
fn merge_creates_removes_and_commits_the_whole_intent() {
    let repo = Repo::new();
    repo.seed("docs/a.md", A_BODY);
    repo.seed_unmanaged("docs/b.md", B_BODY, true);
    repo.propose("key-real-0002", merge_input).unwrap();
    repo.accept("CP-001").unwrap();
    let (out, receipt) = repo.apply("CP-001", vec![]);
    let out = out.unwrap();
    assert_eq!(
        (out.state, out.applied.len(), out.total),
        (CpState::Applied, 3, 3),
        "{out:?}"
    );
    assert_eq!(receipt.outcome, GitOutcome::Committed, "{receipt:?}");
    assert!(repo.bytes("docs/a.md").is_none() && repo.bytes("docs/b.md").is_none());
    assert_eq!(
        repo.bytes("docs/ab.md").unwrap(),
        b"Merged\n## S1\none\n## T1\nuno\n"
    );
    let (_, files) = last_commit(&repo);
    for expected in [
        "docs/ab.md",
        "docs/a.md",
        "docs/b.md",
        "documents/DOC-001.yaml",
        "documents/DOC-002.yaml",
        "compactions/CP-001.yaml",
    ] {
        assert!(files.iter().any(|f| f == expected), "{expected}: {files:?}");
    }
    let retired = String::from_utf8(repo.bytes("documents/DOC-001.yaml").unwrap()).unwrap();
    assert!(retired.contains("state: retired"), "{retired}");
    assert!(
        repo.bytes("documents/DOC-003.yaml").is_none(),
        "an unmanaged removal gains no synthetic record"
    );
}

/// Replace of an unmanaged document and Move of a managed and an unmanaged one: the managed move keeps
/// its DOC identity, and the moved source is gone only after the destination and record exist.
#[test]
fn replace_and_move_keep_identity_on_the_real_providers() {
    let repo = Repo::new();
    repo.seed("docs/a.md", A_BODY);
    repo.seed("docs/m1.md", M_BODY);
    repo.seed_unmanaged("docs/m2.md", "Bee\n## M2\nm\n", true);
    let id_before = {
        let store = Store::from_root(repo.fx.dir.path()).unwrap();
        let guard = store.lock(true, &mut Vec::new()).unwrap().unwrap();
        let env = LiveEnv::new(&store, &guard).unwrap();
        env.observe("docs/m1.md").unwrap().record.unwrap().id
    };
    repo.propose("key-real-0003", |env| {
        let mut sections = ledger(
            env,
            "docs/a.md",
            Some(("A-01", "Intro\n## S1\none\n")),
            None,
        );
        for (p, a) in [("docs/m1.md", "A-02"), ("docs/m2.md", "A-03")] {
            sections.extend(ledger(env, p, None, Some(a)));
        }
        ProposalIn {
            title: "Replace and move".into(),
            sources: vec![
                source(env, "docs/a.md"),
                source(env, "docs/m1.md"),
                source(env, "docs/m2.md"),
            ],
            actions: vec![
                ActionIn {
                    content: Some("Intro\n## S1\none\n".into()),
                    ..action("A-01", ActionKind::Replace, "docs/a.md")
                },
                ActionIn {
                    from: Some("docs/m1.md".into()),
                    ..action("A-02", ActionKind::Move, "docs/moved1.md")
                },
                ActionIn {
                    from: Some("docs/m2.md".into()),
                    ..action("A-03", ActionKind::Move, "docs/moved2.md")
                },
            ],
            sections,
            preservation: vec![],
        }
    })
    .unwrap();
    repo.accept("CP-001").unwrap();
    let (out, receipt) = repo.apply("CP-001", vec![]);
    let out = out.unwrap();
    assert_eq!(out.state, CpState::Applied, "{out:?}");
    assert_eq!(receipt.outcome, GitOutcome::Committed, "{receipt:?}");
    assert!(repo.bytes("docs/m1.md").is_none() && repo.bytes("docs/m2.md").is_none());
    assert_eq!(repo.bytes("docs/moved1.md").unwrap(), M_BODY.as_bytes());
    let record =
        String::from_utf8(repo.bytes(&format!("documents/{id_before}.yaml")).unwrap()).unwrap();
    assert!(
        record.contains("path: docs/moved1.md") && record.contains("state: active"),
        "{record}"
    );
}

/// An interrupted body-first replace: the body is published under the action's operation identity but
/// its record is not. The next apply proves the body through the persistence oracle and adopts it, and
/// never treats the same bytes as ownership by themselves.
#[test]
fn body_first_partial_resumes_through_the_real_oracle() {
    let repo = Repo::new();
    repo.seed("docs/a.md", A_BODY);
    repo.propose("key-real-0004", replace_a).unwrap();
    repo.accept("CP-001").unwrap();
    let (first, receipt) = repo.apply(
        "CP-001",
        vec![fault("put", "documents/DOC-001.yaml", "io", false)],
    );
    assert_eq!(code(&first), "partial_publication", "{first:?}");
    assert_ne!(receipt.outcome, GitOutcome::Committed, "{receipt:?}");
    assert_eq!(repo.cp("CP-001").state, CpState::Blocked);
    assert_eq!(repo.bytes("docs/a.md").unwrap(), b"Intro\n## S1\none\n");
    // The held body must be committed by the explicit recovery before the resume writes again.
    let (early, _) = repo.apply("CP-001", vec![]);
    assert_eq!(code(&early), "replacements_not_committed", "{early:?}");
    assert_eq!(repo.pending_intents(), 1);
    let recovered = repo.retry_pending();
    assert_eq!(recovered.outcome, GitOutcome::Committed, "{recovered:?}");
    let (second, receipt) = repo.apply("CP-001", vec![]);
    let second = second.unwrap();
    assert_eq!(second.state, CpState::Applied, "{second:?}");
    assert_eq!(receipt.outcome, GitOutcome::Committed, "{receipt:?}");
    assert!(repo.cp("CP-001").apply.unwrap().attempts >= 2);
    assert_eq!(repo.pending_intents(), 0);
    assert_eq!(repo.pending_intents(), 0, "nothing stays pending");
    let (_, files) = last_commit(&repo);
    assert!(
        files.iter().any(|f| f == "documents/DOC-001.yaml"),
        "{files:?}"
    );
}

/// Contract revision 8 on the real providers: after the interrupted Managed Replace is committed by the
/// explicit Retry, a native edit that advanced the DOC record's revision means the observed record is no
/// longer the frozen source's own at its base revision, so the resume refuses and adopts nothing.
#[test]
fn managed_replace_resume_refuses_an_advanced_record_revision() {
    let repo = Repo::new();
    repo.seed("docs/a.md", A_BODY);
    repo.propose("key-real-0031", replace_a).unwrap();
    repo.accept("CP-001").unwrap();
    let (first, _) = repo.apply(
        "CP-001",
        vec![fault("put", "documents/DOC-001.yaml", "io", false)],
    );
    assert_eq!(code(&first), "partial_publication", "{first:?}");
    assert_eq!(repo.retry_pending().outcome, GitOutcome::Committed);
    let record =
        std::fs::read_to_string(repo.fx.dir.path().join("documents/DOC-001.yaml")).unwrap();
    assert!(record.contains("revision: 1\n"), "{record}");
    std::fs::write(
        repo.fx.dir.path().join("documents/DOC-001.yaml"),
        record.replace("revision: 1\n", "revision: 2\n"),
    )
    .unwrap();
    let (second, _) = repo.apply("CP-001", vec![]);
    assert_eq!(code(&second), "metadata_mismatch", "{second:?}");
    assert_ne!(repo.cp("CP-001").state, CpState::Applied);
    let after = std::fs::read_to_string(repo.fx.dir.path().join("documents/DOC-001.yaml")).unwrap();
    assert!(
        after.contains("revision: 2\n"),
        "nothing adopted over the edited record: {after}"
    );
}

/// Interrupted move: the destination and moved record exist but the source removal failed. The held
/// intent of the first call is not committed, so the second call may finish the removal only through
/// the oracle-proven resume, and the original source stays recoverable until then.
#[test]
fn move_interrupted_before_the_source_removal_resumes() {
    let repo = Repo::new();
    repo.seed("docs/m1.md", M_BODY);
    repo.seed("docs/a.md", A_BODY);
    repo.propose("key-real-0005", |env| {
        let mut sections = ledger(env, "docs/m1.md", None, Some("A-01"));
        sections.extend(ledger(
            env,
            "docs/a.md",
            Some(("A-02", "Intro\n## S1\none\n")),
            None,
        ));
        ProposalIn {
            title: "Move".into(),
            sources: vec![source(env, "docs/m1.md"), source(env, "docs/a.md")],
            actions: vec![
                ActionIn {
                    from: Some("docs/m1.md".into()),
                    ..action("A-01", ActionKind::Move, "docs/moved.md")
                },
                ActionIn {
                    content: Some("Intro\n## S1\none\n".into()),
                    ..action("A-02", ActionKind::Replace, "docs/a.md")
                },
            ],
            sections,
            preservation: vec![],
        }
    })
    .unwrap();
    repo.accept("CP-001").unwrap();
    let (first, _) = repo.apply("CP-001", vec![fault("del", "docs/m1.md", "io", false)]);
    assert_eq!(code(&first), "partial_publication", "{first:?}");
    assert!(
        repo.bytes("docs/m1.md").is_some(),
        "the source is still there"
    );
    assert!(repo.bytes("docs/moved.md").is_some());
    // The destination body and moved record sit in a held, uncommitted intent: the source may not be
    // removed until explicit recovery commits them.
    let (blocked, _) = repo.apply("CP-001", vec![]);
    assert_eq!(code(&blocked), "replacements_not_committed", "{blocked:?}");
    assert!(repo.bytes("docs/m1.md").is_some());
    let recovered = repo.retry_pending();
    assert_eq!(recovered.outcome, GitOutcome::Committed, "{recovered:?}");
    let (second, receipt) = repo.apply("CP-001", vec![]);
    let second = second.unwrap();
    assert_eq!(second.state, CpState::Applied, "{second:?}");
    assert_eq!(receipt.outcome, GitOutcome::Committed, "{receipt:?}");
    assert!(repo.bytes("docs/m1.md").is_none());
}

/// A second proposal after the first one staged its candidate directory must still allocate: the
/// allocation observation has to read the nested compactions home through the closed inventory.
#[test]
fn a_second_proposal_allocates_after_a_staged_directory() {
    let repo = Repo::new();
    repo.seed("docs/a.md", A_BODY);
    repo.seed("docs/m1.md", M_BODY);
    repo.propose("key-real-0006", replace_a).unwrap();
    let second = repo.propose("key-real-0007", |env| ProposalIn {
        title: "Trim M".into(),
        sources: vec![source(env, "docs/m1.md")],
        actions: vec![ActionIn {
            content: Some("Mover\n".into()),
            ..action("A-01", ActionKind::Replace, "docs/m1.md")
        }],
        sections: ledger(env, "docs/m1.md", Some(("A-01", "Mover\n")), None),
        preservation: vec![],
    });
    assert_eq!(second.unwrap().id, "CP-002");
}

/// A typed record home with an unreadable record is a coverage gap: destructive proposals refuse
/// instead of treating the unknown as zero references.
#[test]
fn an_unreadable_typed_home_blocks_destructive_proposals() {
    let repo = Repo::new();
    repo.seed("docs/a.md", A_BODY);
    repo.seed_unmanaged("decisions/D-001.yaml", "not: a record\n", false);
    let refused = repo.propose("key-real-0008", |env| ProposalIn {
        title: "Remove A".into(),
        sources: vec![source(env, "docs/a.md")],
        actions: vec![action("A-01", ActionKind::Remove, "docs/a.md")],
        sections: ledger(env, "docs/a.md", None, None),
        preservation: vec![],
    });
    assert_eq!(code(&refused), "coverage_incomplete", "{refused:?}");
    // The refusal names the unreadable record, never a blank unknown.
    assert!(
        refused.as_ref().unwrap_err().message.contains("D-001"),
        "{refused:?}"
    );
}

/// An original that exists only in the working tree is not provably committed: apply publishes nothing.
#[test]
fn an_uncommitted_original_is_refused_before_any_effect() {
    let repo = Repo::new();
    repo.seed_unmanaged("docs/b.md", B_BODY, false);
    repo.propose("key-real-0009", |env| ProposalIn {
        title: "Trim B".into(),
        sources: vec![source(env, "docs/b.md")],
        actions: vec![ActionIn {
            content: Some("Bee\n".into()),
            purpose: Some("trimmed".into()),
            ..action("A-01", ActionKind::Replace, "docs/b.md")
        }],
        sections: ledger(env, "docs/b.md", Some(("A-01", "Bee\n")), None),
        preservation: vec![],
    })
    .unwrap();
    repo.accept("CP-001").unwrap();
    let (out, _) = repo.apply("CP-001", vec![]);
    assert_eq!(code(&out), "originals_not_committed", "{out:?}");
    assert_eq!(repo.bytes("docs/b.md").unwrap(), B_BODY.as_bytes());
}

/// A document that links to the removed target would dangle: the real reference owner's integrity
/// preview refuses the proposal before anything is staged.
#[test]
fn removing_a_linked_document_is_refused_by_the_reference_owner() {
    let repo = Repo::new();
    repo.seed("docs/a.md", A_BODY);
    repo.seed("docs/ref.md", "Ref\nSee [a](a.md).\n");
    let refused = repo.propose("key-real-0010", |env| ProposalIn {
        title: "Remove A".into(),
        sources: vec![source(env, "docs/a.md")],
        actions: vec![action("A-01", ActionKind::Remove, "docs/a.md")],
        sections: ledger(env, "docs/a.md", None, None),
        preservation: vec![],
    });
    assert_eq!(code(&refused), "incoming_unresolved", "{refused:?}");
    assert!(repo.bytes("compactions/CP-001.yaml").is_none());
}

/// Dropping a section that another document links by fragment is refused the same way.
#[test]
fn dropping_a_linked_section_is_refused_by_the_reference_owner() {
    let repo = Repo::new();
    repo.seed("docs/a.md", A_BODY);
    repo.seed("docs/ref.md", "Ref\nSee [two](a.md#s2).\n");
    let refused = repo.propose("key-real-0011", replace_a);
    assert_eq!(code(&refused), "incoming_unresolved", "{refused:?}");
}

/// Explicitly releasing the held intent of an interrupted call leaves only unattested equal bytes:
/// the oracle reports them unknown, so the resume refuses and never adopts them as its own work.
#[test]
fn released_attestation_leaves_equal_bytes_unknown() {
    let repo = Repo::new();
    repo.seed("docs/a.md", A_BODY);
    repo.propose("key-real-0012", replace_a).unwrap();
    repo.accept("CP-001").unwrap();
    let (first, _) = repo.apply(
        "CP-001",
        vec![fault("put", "documents/DOC-001.yaml", "io", false)],
    );
    assert_eq!(code(&first), "partial_publication", "{first:?}");
    {
        let store = Store::from_root(repo.fx.dir.path()).unwrap();
        let guard = store.lock(true, &mut Vec::new()).unwrap().unwrap();
        let pending = crate::persist::pending(&store);
        let ids: Vec<String> = pending.refs.iter().map(|r| r.intent.clone()).collect();
        crate::persist::recover::recover(
            &store,
            &guard,
            &pending.version,
            crate::persist::recover::Action::Release(ids),
        )
        .unwrap();
    }
    let (second, _) = repo.apply("CP-001", vec![]);
    assert_eq!(code(&second), "effect_unknown", "{second:?}");
    let record = String::from_utf8(repo.bytes("documents/DOC-001.yaml").unwrap()).unwrap();
    assert!(record.contains("revision: 1"), "never adopted: {record}");
}

impl Repo {
    /// Create a typed decision record through the knowledge owner, optionally naming a document, and
    /// commit it with the production policy.
    fn seed_decision(&self, detail: Option<&str>) {
        use crate::knowledge::{self, Alternative, DecisionBody, Prefix};
        let store = Store::from_root(self.fx.dir.path()).unwrap();
        let guard = store.lock(true, &mut Vec::new()).unwrap().unwrap();
        let mut fx = Vec::new();
        let version = knowledge::allocation_version(&store).unwrap();
        let id = knowledge::reserve(&store, &guard, Prefix::Decision, &version, &mut fx).unwrap();
        knowledge::ensure_home(&store, Prefix::Decision, &mut fx).unwrap();
        let body = DecisionBody {
            title: "Keep files".into(),
            question: "Which store?".into(),
            decision: "Use files.".into(),
            rationale: "Portable.".into(),
            alternatives: vec![Alternative {
                option: "database".into(),
                rejected_because: "extra daemon".into(),
            }],
            open_questions: vec![],
            detail: detail.map(str::to_owned),
        };
        let record = knowledge::new_decision(&id, body, &crate::store::now(), &None);
        knowledge::create_file(&store, &format!("decisions/{id}.yaml"), &record, &mut fx).unwrap();
        let receipt = settle(
            &store,
            &guard,
            &Event {
                class: EventClass::Knowledge,
                refs: vec![id],
                operation: None,
                outcome: EventOutcome::Success,
            },
            production_policy(),
        );
        assert_eq!(receipt.outcome, GitOutcome::Committed, "{receipt:?}");
    }
}

/// A healthy typed record is covered, not a gap: a decision that names the document by path keeps a
/// replace (with its fragments intact) possible, but makes a removal unresolved because a record
/// referrer cannot be rewritten.
#[test]
fn a_healthy_typed_record_is_covered_and_blocks_only_the_removal() {
    let repo = Repo::new();
    repo.seed("docs/a.md", A_BODY);
    repo.seed_decision(Some("docs/a.md"));
    let removal = repo.propose("key-real-0013", |env| ProposalIn {
        title: "Remove A".into(),
        sources: vec![source(env, "docs/a.md")],
        actions: vec![action("A-01", ActionKind::Remove, "docs/a.md")],
        sections: ledger(env, "docs/a.md", None, None),
        preservation: vec![],
    });
    assert_eq!(code(&removal), "incoming_unresolved", "{removal:?}");
    let replace = repo.propose("key-real-0014", replace_a);
    assert_eq!(replace.unwrap().state, CpState::Proposed);
}

/// The allocation validity of a DOC identifier follows the real allocator: only identifiers below the
/// next number of a fully understood allocation are valid.
#[test]
fn allocation_validity_follows_the_real_allocator() {
    let repo = Repo::new();
    repo.seed("docs/a.md", A_BODY);
    let store = Store::from_root(repo.fx.dir.path()).unwrap();
    let guard = store.lock(true, &mut Vec::new()).unwrap().unwrap();
    let env = LiveEnv::new(&store, &guard).unwrap();
    assert!(env.allocation_valid("DOC-001").unwrap());
    for refused in ["DOC-002", "DOC-1", "D-001", "CP-001", "nonsense"] {
        assert!(!env.allocation_valid(refused).unwrap(), "{refused}");
    }
}

/// The outline the domain decides on is the Markdown owner's: the preamble first, then every heading
/// section with its nested sections, exact bytes, heading text digests and unique slugs.
#[test]
fn the_outline_is_the_markdown_owners_sections() {
    let repo = Repo::new();
    let store = Store::from_root(repo.fx.dir.path()).unwrap();
    let guard = store.lock(true, &mut Vec::new()).unwrap().unwrap();
    let env = LiveEnv::new(&store, &guard).unwrap();
    let body = "Intro\n## S1\none\n### Sub\nx\n## S2\ntwo\n";
    let outline = env.outline(body.as_bytes()).unwrap();
    assert!(outline.complete && outline.setext_candidates == 0);
    let texts: Vec<&str> = outline
        .sections
        .iter()
        .map(|s| std::str::from_utf8(&s.bytes).unwrap())
        .collect();
    assert_eq!(
        texts,
        [
            "Intro\n",
            "## S1\none\n### Sub\nx\n",
            "### Sub\nx\n",
            "## S2\ntwo\n"
        ]
    );
    assert_eq!(
        outline.sections[0].addr,
        super::record::SectionAddr::Preamble
    );
    assert!(matches!(
        &outline.sections[2].addr,
        super::record::SectionAddr::Heading { ordinal: 1, level: 3, occurrence: 1, text_sha256 }
            if *text_sha256 == super::record::sha256_hex(b"Sub")
    ));
    assert_eq!(outline.slugs, ["s1", "sub", "s2"]);
    let bare = env.outline(b"## Only\nx\n").unwrap();
    assert_eq!(
        bare.sections.len(),
        1,
        "no preamble section when it is empty"
    );
}

/// What a later committed record write does to an older held intent. A successful later call that
/// rewrites the proposal record commits without the older held intent, so the intent's last image of the
/// record is superseded. The persistence engine of this checkout recovers that case by explicit `Retry`:
/// it skips the superseded paths, keeps their committed successors and commits the rest, where the
/// engine this test was first written against refused `recovery_blocked`. The revision 6 held barrier
/// still refuses an apply attempt before it can write over such an intent, which stays the conservative
/// contract; this control pins the engine's present recovery so a change in either direction is seen.
/// Here the later call is a withdrawal with `abandon_partial`.
#[test]
fn a_later_committed_record_write_supersedes_an_older_held_intent() {
    let repo = Repo::new();
    repo.seed("docs/a.md", A_BODY);
    let original_commit = repo.fx.git(&["rev-parse", "HEAD"]);
    repo.propose("key-real-0015", replace_a).unwrap();
    repo.accept("CP-001").unwrap();
    let (first, _) = repo.apply(
        "CP-001",
        vec![fault("put", "documents/DOC-001.yaml", "io", false)],
    );
    assert_eq!(code(&first), "partial_publication", "{first:?}");
    assert_eq!(repo.pending_intents(), 1);
    let version = repo.cp_version("CP-001");
    let (withdrawn, receipt) = repo.step(EventClass::CompactionWithdraw, vec![], |env, fx| {
        ops::withdraw(env, "author", &version, "CP-001", "stop", true, fx)
    });
    assert_eq!(withdrawn.unwrap().state, CpState::AbandonedPartial);
    assert_eq!(receipt.outcome, GitOutcome::Committed, "{receipt:?}");
    let store = Store::from_root(repo.fx.dir.path()).unwrap();
    let guard = store.lock(true, &mut Vec::new()).unwrap().unwrap();
    let pending = crate::persist::pending(&store);
    let ids: Vec<String> = pending.refs.iter().map(|r| r.intent.clone()).collect();
    assert_eq!(ids.len(), 1, "the held intent is still pending");
    let retry = crate::persist::recover::recover(
        &store,
        &guard,
        &pending.version,
        crate::persist::recover::Action::Retry(ids),
    );
    let report = retry.unwrap_or_else(|e| panic!("retry refused: {e:?}"));
    // Only the held intent's own unsuperseded body is committed, by one new commit.
    assert_eq!(report.receipt.outcome, GitOutcome::Committed);
    assert_eq!(report.receipt.paths, ["docs/a.md"]);
    assert!(
        report
            .lines
            .iter()
            .any(|l| l.starts_with("Skipped 2 superseded path(s)")),
        "{:?}",
        report.lines
    );
    assert!(crate::persist::pending(&store).refs.is_empty());
    assert!(repo.fx.git(&["status", "--porcelain"]).is_empty());
    assert_eq!(
        repo.fx
            .git(&["show", "--name-only", "--format=", "HEAD"])
            .trim(),
        "docs/a.md"
    );
    // The newer record bytes are retained: the retry commit left the record exactly as the withdrawal
    // committed it, and the proposal is still the abandoned partial the owner chose.
    let withdraw_image = repo.fx.git(&["show", "HEAD~1:compactions/CP-001.yaml"]);
    assert_eq!(
        repo.fx.git(&["show", "HEAD:compactions/CP-001.yaml"]),
        withdraw_image
    );
    let current = repo.bytes("compactions/CP-001.yaml").unwrap();
    assert_eq!(
        String::from_utf8(current.clone()).unwrap().trim_end(),
        withdraw_image
    );
    assert_eq!(repo.cp("CP-001").state, CpState::AbandonedPartial);
    // Both superseded record effects keep their own true digests and name the exact committed successor
    // (`into=`), which is the withdrawal image; the body effect is the staged candidate's own digest.
    let successor = super::record::sha256_hex(&current);
    let trailers = repo.fx.git(&["show", "--format=%B", "--no-patch", "HEAD"]);
    let effects: Vec<&str> = trailers
        .lines()
        .filter_map(|l| l.strip_prefix("Agent-Tasks-Effect: "))
        .collect();
    assert_eq!(effects.len(), 3, "{effects:?}");
    for rec in ["cp:CP-001:r1:rec:3 ", "cp:CP-001:r1:rec:4 "] {
        let line = effects.iter().find(|l| l.starts_with(rec)).unwrap();
        assert!(line.ends_with(&format!(" into={successor}")), "{line}");
        assert!(
            !line.contains(&format!(" {successor} ")),
            "never rewritten to the successor: {line}"
        );
    }
    let staged = repo.cp("CP-001").revisions[0].body.actions[0]
        .staged_sha256
        .clone()
        .unwrap();
    let body = effects
        .iter()
        .find(|l| l.starts_with("cp:CP-001:r1:A-01 replaced docs/a.md "))
        .unwrap();
    assert!(
        body.ends_with(&format!(" {staged}")) && !body.contains("into="),
        "{body}"
    );
    assert_eq!(repo.bytes("docs/a.md").unwrap(), b"Intro\n## S1\none\n");
    // Honest limit: the failed document record put was never published, so the committed body is new
    // while `documents/DOC-001.yaml` still records the old body digest. Abandoning the partial apply
    // accepts this state; nothing reports it as applied or complete.
    let doc = String::from_utf8(repo.bytes("documents/DOC-001.yaml").unwrap()).unwrap();
    assert!(
        doc.contains(&format!(
            "body_sha256: {}",
            super::record::sha256_hex(A_BODY.as_bytes())
        )),
        "{doc}"
    );
    assert!(!doc.contains(&staged), "{doc}");
    drop(guard);
    // The mismatch stays visible and is never counted as applied or as current coverage: the document
    // owner reports the record and the path drifted, the proposal stays an abandoned partial that cannot
    // be applied again, the original body is still retrievable from its committed seed, and a new
    // destructive proposal over the drifted document is refused at its source.
    let (obs, _) = repo.step(EventClass::Document, vec![], |env, _| {
        Ok((env.observe_id("DOC-001")?, env.observe("docs/a.md")?))
    });
    let (by_id, by_path) = obs.unwrap();
    assert_eq!(by_id.state, DocState::Other("drifted".into()));
    assert_eq!(by_path.state, DocState::Other("drifted".into()));
    assert_eq!(repo.cp("CP-001").state, CpState::AbandonedPartial);
    assert_eq!(
        repo.fx
            .git(&["show", &format!("{original_commit}:docs/a.md")]),
        A_BODY.trim_end()
    );
    let (again, _) = repo.apply("CP-001", vec![]);
    assert_eq!(code(&again), "already_withdrawn");
    let (next, _) = repo.step(EventClass::CompactionPropose, vec![], |env, fx| {
        let input = ProposalIn {
            title: "Remove A".into(),
            sources: vec![source(env, "docs/a.md")],
            actions: vec![action("A-01", ActionKind::Remove, "docs/a.md")],
            sections: ledger(env, "docs/a.md", None, None),
            preservation: vec![],
        };
        ops::propose(
            env,
            "author",
            &env.allocation_version()?,
            "key-real-0099",
            &input,
            fx,
        )
    });
    assert_eq!(code(&next), "source_refused");
}

/// A held intent that carries only the proposal's own record (the first apply published its `Applying`
/// record and failed before any document effect) also blocks the resume: rewriting the record later would
/// strand it. After the explicit Retry the resume completes.
#[test]
fn a_held_record_only_intent_blocks_until_it_is_committed() {
    let repo = Repo::new();
    repo.seed("docs/a.md", A_BODY);
    repo.propose("key-real-0016", replace_a).unwrap();
    repo.accept("CP-001").unwrap();
    let (first, _) = repo.apply("CP-001", vec![fault("put", "docs/a.md", "io", false)]);
    assert_eq!(code(&first), "io", "{first:?}");
    assert_eq!(repo.bytes("docs/a.md").unwrap(), A_BODY.as_bytes());
    assert_eq!(repo.pending_intents(), 1);
    let (early, _) = repo.apply("CP-001", vec![]);
    assert_eq!(code(&early), "replacements_not_committed", "{early:?}");
    let recovered = repo.retry_pending();
    assert_eq!(recovered.outcome, GitOutcome::Committed, "{recovered:?}");
    let (second, receipt) = repo.apply("CP-001", vec![]);
    assert_eq!(second.unwrap().state, CpState::Applied);
    assert_eq!(receipt.outcome, GitOutcome::Committed, "{receipt:?}");
    assert_eq!(repo.pending_intents(), 0);
}

/// An older held intent on unrelated paths never blocks another proposal, the fresh call still commits
/// its whole intent, and the unrelated intent stays recoverable by Retry afterwards.
#[test]
fn an_unrelated_held_intent_neither_blocks_nor_is_stranded() {
    let repo = Repo::new();
    repo.seed("docs/a.md", A_BODY);
    repo.seed("docs/m1.md", M_BODY);
    repo.propose("key-real-0017", replace_a).unwrap();
    repo.propose("key-real-0018", |env| ProposalIn {
        title: "Trim M".into(),
        sources: vec![source(env, "docs/m1.md")],
        actions: vec![ActionIn {
            content: Some("Mover\n".into()),
            ..action("A-01", ActionKind::Replace, "docs/m1.md")
        }],
        sections: ledger(env, "docs/m1.md", Some(("A-01", "Mover\n")), None),
        preservation: vec![],
    })
    .unwrap();
    repo.accept("CP-001").unwrap();
    repo.accept("CP-002").unwrap();
    let (partial, _) = repo.apply(
        "CP-002",
        vec![fault("put", "documents/DOC-002.yaml", "io", false)],
    );
    assert_eq!(code(&partial), "partial_publication", "{partial:?}");
    assert_eq!(repo.pending_intents(), 1);
    let (out, receipt) = repo.apply("CP-001", vec![]);
    assert_eq!(out.unwrap().state, CpState::Applied);
    assert_eq!(receipt.outcome, GitOutcome::Committed, "{receipt:?}");
    assert_eq!(
        repo.pending_intents(),
        1,
        "the unrelated held intent remains"
    );
    let recovered = repo.retry_pending();
    assert_eq!(recovered.outcome, GitOutcome::Committed, "{recovered:?}");
    let (second, _) = repo.apply("CP-002", vec![]);
    assert_eq!(second.unwrap().state, CpState::Applied);
}

/// The engine lists at most sixteen pending intents. With seventeen, the newest, which here holds the
/// proposal's own document, is hidden from the listing, so the held barrier alone could not see it. Apply
/// must refuse and say why rather than write over an intent it cannot rule out.
#[test]
fn seventeen_pending_intents_hide_an_overlapping_held_intent_so_apply_refuses() {
    let repo = Repo::new();
    repo.seed("docs/a.md", A_BODY);
    repo.propose("key-real-0030", replace_a).unwrap();
    repo.accept("CP-001").unwrap();
    for n in 0..16 {
        let path = format!("docs/u{n:02}.md");
        let (held, _) = repo.step(
            EventClass::Document,
            vec![fault("put", "documents/DOC-", "io", false)],
            |env, fx| {
                let expected = env.observe(&path)?.version;
                env.doc_op(
                    &format!("unrelated-{n}"),
                    "tester",
                    &DocOp::Save {
                        path: path.clone(),
                        body: b"U\n".to_vec(),
                        purpose: Some("fixture".into()),
                        expected,
                    },
                    fx,
                )
            },
        );
        assert!(
            held.is_err(),
            "{path}: the scripted fault leaves a held intent"
        );
    }
    assert_eq!(repo.pending_intents(), 16);
    // Sixteen unrelated intents are fully listed, so the first apply runs and itself leaves a held intent
    // on the proposal's document: the seventeenth, beyond the engine's listing.
    let (first, _) = repo.apply(
        "CP-001",
        vec![fault("put", "documents/DOC-001.yaml", "io", false)],
    );
    assert_eq!(code(&first), "partial_publication", "{first:?}");
    let store = Store::from_root(repo.fx.dir.path()).unwrap();
    let summary = crate::persist::pending(&store);
    assert_eq!((summary.facts.intents, summary.refs.len()), (17, 16));
    let before = repo.bytes("docs/a.md");
    let (second, _) = repo.apply("CP-001", vec![]);
    let err = second.unwrap_err();
    assert_eq!(err.code, "effect_unknown", "{err:?}");
    assert!(
        err.message.contains("17 pending intents") && err.message.contains("only 16"),
        "{}",
        err.message
    );
    assert_eq!(repo.bytes("docs/a.md"), before, "nothing was written");
}
