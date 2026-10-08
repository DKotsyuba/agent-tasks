//! Producer of the `compaction_work` tool: the closed operation payload, its description and the
//! handler that turns a decoded operation into the shared acknowledgement.
//!
//! The registry owner decodes the shared arguments with `input::mutation::<Compaction>`, exports the
//! schema, takes the single write lock and settles Git after the handler returns. This file owns only
//! the wire shape of the six operations, their conversion into domain inputs and the acknowledgement
//! mapping; every rule lives in `crate::compaction`, which imports nothing from `tools`.
// The registry owner registers this producer; until it does, the public payload is legitimately unused
// by the binary crate and exercised by the tests below.
#![allow(dead_code)]
use super::{
    input::{Common, RecoveryStage},
    work::{self, Ack},
};
use crate::{
    compaction::{
        self,
        env::Env,
        live::LiveEnv,
        ops::{self, Outcome, RecoveryIn, RecoveryStep},
        record::{
            ActionKind, Disposition, DropKind, PreserveKind, PreserveMode, SectionAddr, TargetRef,
        },
        review::ReviewIn,
        validate::{ActionIn, PreservationIn, ProposalIn, SectionIn, SourceIn},
    },
    model::{Finding, FindingResolution, Verdict},
    persist::EventClass,
    store::{Error, LockGuard, Result, Store},
};
use schemars::JsonSchema;
use serde::Deserialize;

/// Description text of the `compaction_work` tool, in bounded ordinary prose for the live catalog.
pub const DESCRIPTION: &str = "Purpose: Propose, review and apply a compaction of Markdown documents under independent review. An external actor prepares the proposal; the MCP validates it, stores every full revision and review, and applies it only after acceptance. Input: project, version, actor and one operation. propose and revise take sources with the observation version you read, actions create, replace, move and remove with candidate content, a ledger that accounts for every original section, and preservation items. review needs the exact revision and content hash and a verdict for every item. recover_reviewer replaces a reviewer only after observed unrecoverable loss. apply runs create, move and replace, verifies, then removes, and resumes after interruption. withdraw ends a proposal. Version is the knowledge allocation version for propose and the proposal record version for every other operation. Effects: tracked files under compactions and the document changes of an accepted proposal. Never launches a model, never deletes on its own, never rolls back; originals must already be committed in Git. Output: acknowledgement with state, applied actions and any stop reason.";

/// Address of one section of an exact document body.
#[derive(Clone, Debug, Deserialize, JsonSchema)]
#[serde(tag = "at", rename_all = "snake_case", deny_unknown_fields)]
pub enum SectionAddrIn {
    /// Bytes before the first heading.
    Preamble,
    /// One real heading of the observed bytes.
    Heading {
        /// Zero based document order ordinal.
        ordinal: usize,
        /// Heading level, 1 to 6.
        level: u8,
        /// One based occurrence among headings with identical text.
        occurrence: usize,
        /// Hex sha256 of the raw heading text.
        text_sha256: String,
    },
}

/// A document, optionally narrowed to one section.
#[derive(Clone, Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TargetRefIn {
    /// Managed document path.
    pub path: String,
    /// Section address; absent addresses the whole document.
    pub section: Option<SectionAddrIn>,
}

/// Reason class for dropping a section.
#[derive(Clone, Copy, Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum DropKindIn {
    /// Equivalent content is retained elsewhere.
    Duplicate,
    /// No longer true or relevant.
    Obsolete,
    /// Superseded by another retained section.
    Superseded,
}

/// What happens to one original section.
#[derive(Clone, Debug, Deserialize, JsonSchema)]
#[serde(tag = "fate", rename_all = "snake_case", deny_unknown_fields)]
pub enum DispositionIn {
    /// The exact section bytes appear in the named action's candidate.
    Kept {
        /// Action id.
        action: String,
    },
    /// The section moves with the named Move action.
    Moved {
        /// Move action id.
        action: String,
    },
    /// The content is rewritten into the target.
    Merged {
        /// Where the rewritten content lives.
        target: TargetRefIn,
    },
    /// The content is dropped with a reason.
    Dropped {
        /// Reason class.
        kind: DropKindIn,
        /// Reason, at most 512 bytes.
        reason: String,
    },
}

/// Kind of document action.
#[derive(Clone, Copy, Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ActionKindIn {
    /// Create at an absent path.
    Create,
    /// Replace a whole body.
    Replace,
    /// Move to an absent destination.
    Move,
    /// Remove a document.
    Remove,
}

/// Kind of preserved fact.
#[derive(Clone, Copy, Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum PreserveKindIn {
    /// A business requirement.
    Requirement,
    /// Durable rationale.
    Rationale,
    /// A link.
    Link,
    /// A decision.
    Decision,
    /// A constraint.
    Constraint,
}

/// How a preserved fact reaches its target.
#[derive(Clone, Copy, Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum PreserveModeIn {
    /// Exact bytes, checked mechanically.
    Verbatim,
    /// Rewritten, accepted by reviewer verdict.
    Rewritten,
}

/// One source document with the observation version the proposer read.
#[derive(Clone, Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SourceInPayload {
    /// Managed document path.
    pub path: String,
    /// Observation version read, which must still be current.
    pub version: String,
}

/// One proposed document action.
#[derive(Clone, Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ActionInPayload {
    /// Identifier `A-01` to `A-32`, unique in the proposal.
    pub id: String,
    /// Kind of action.
    pub kind: ActionKindIn,
    /// Target path; for a move the destination.
    pub path: String,
    /// Move source path.
    pub from: Option<String>,
    /// Optional assertion of the preimage version.
    pub base_version: Option<String>,
    /// Candidate body text for create and replace.
    pub content: Option<String>,
    /// Purpose when a new document record will be created, at most 240 bytes.
    pub purpose: Option<String>,
    /// Why the action belongs to the compaction, at most 512 bytes.
    pub reason: String,
    /// Where the content of a removed document now lives.
    #[serde(default)]
    pub absorbed_into: Vec<TargetRefIn>,
}

/// Disposition of one original section.
#[derive(Clone, Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SectionInPayload {
    /// Source path.
    pub path: String,
    /// Section address from the source outline.
    pub section: SectionAddrIn,
    /// What happens to the section.
    pub disposition: DispositionIn,
}

/// One claimed preservation of a requirement, rationale or link.
#[derive(Clone, Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PreservationInPayload {
    /// Identifier `P-01` to `P-64`.
    pub id: String,
    /// Kind of fact.
    pub kind: PreserveKindIn,
    /// Source path.
    pub path: String,
    /// Source section.
    pub section: SectionAddrIn,
    /// Statement, at most 256 bytes.
    pub statement: String,
    /// Where the fact now lives.
    pub target: TargetRefIn,
    /// Verification mode.
    pub mode: PreserveModeIn,
}

/// The closed `compaction_work` operation set.
#[derive(Clone, Debug, Deserialize, JsonSchema)]
#[serde(tag = "op", rename_all = "snake_case", deny_unknown_fields)]
pub enum Compaction {
    /// Create a proposal; `version` is the knowledge allocation version.
    Propose {
        /// Idempotency key, 8 to 64 characters of letters, digits, dot, underscore or dash.
        request_key: String,
        /// Title, at most 128 bytes.
        title: String,
        /// Source documents.
        sources: Vec<SourceInPayload>,
        /// Actions, at most 32.
        actions: Vec<ActionInPayload>,
        /// Ledger accounting for every original section.
        sections: Vec<SectionInPayload>,
        /// Preservation items, at most 64.
        #[serde(default)]
        preservation: Vec<PreservationInPayload>,
    },
    /// Append a new full revision; every earlier revision and review is retained.
    Revise {
        /// Proposal identifier.
        cp: String,
        /// Title, at most 128 bytes.
        title: String,
        /// Source documents.
        sources: Vec<SourceInPayload>,
        /// Actions, at most 32.
        actions: Vec<ActionInPayload>,
        /// Ledger accounting for every original section.
        sections: Vec<SectionInPayload>,
        /// Preservation items, at most 64.
        #[serde(default)]
        preservation: Vec<PreservationInPayload>,
    },
    /// Record the independent review of the current revision.
    Review {
        /// Proposal identifier.
        cp: String,
        /// Revision reviewed.
        revision: u32,
        /// Content hash of that revision.
        content_hash: String,
        /// accepted or changes_requested.
        verdict: Verdict,
        /// Summary, at most 512 bytes.
        summary: String,
        /// Every verified item for an acceptance.
        #[serde(default)]
        verified_items: Vec<String>,
        /// Findings of this review.
        #[serde(default)]
        findings: Vec<Finding>,
        /// Resolutions of earlier findings by zero based review and finding index.
        #[serde(default)]
        resolved_findings: Vec<FindingResolution>,
    },
    /// Replace the pinned reviewer only after observed unrecoverable loss.
    RecoverReviewer {
        /// Proposal identifier.
        cp: String,
        /// lost reports the loss, immersed restores a replacement's context.
        stage: RecoveryStage,
        /// True for the lost stage.
        lost: Option<bool>,
        /// True for the lost stage.
        unrecoverable: Option<bool>,
        /// Observation of the inability to continue or resume.
        observation: Option<String>,
        /// Replacement's understanding, at most 512 bytes.
        understanding: Option<String>,
        /// Sources the replacement read.
        #[serde(default)]
        sources: Vec<String>,
        /// Unfinished items inherited.
        #[serde(default)]
        unfinished: Vec<String>,
        /// Unresolved gaps; any gap blocks review.
        #[serde(default)]
        gaps: Vec<String>,
    },
    /// Apply an accepted proposal, or resume an interrupted one.
    Apply {
        /// Proposal identifier.
        cp: String,
    },
    /// Withdraw a proposal; a started application needs abandon_partial.
    Withdraw {
        /// Proposal identifier.
        cp: String,
        /// Reason, at most 512 bytes.
        reason: String,
        /// Required to abandon a proposal that already started applying.
        #[serde(default)]
        abandon_partial: bool,
    },
}

impl From<SectionAddrIn> for SectionAddr {
    /// Convert the wire section address into the domain address.
    fn from(v: SectionAddrIn) -> Self {
        match v {
            SectionAddrIn::Preamble => Self::Preamble,
            SectionAddrIn::Heading {
                ordinal,
                level,
                occurrence,
                text_sha256,
            } => Self::Heading {
                ordinal,
                level,
                occurrence,
                text_sha256,
            },
        }
    }
}
impl From<TargetRefIn> for TargetRef {
    /// Convert the wire target reference into the domain reference.
    fn from(v: TargetRefIn) -> Self {
        Self {
            path: v.path,
            section: v.section.map(Into::into),
        }
    }
}
impl From<DispositionIn> for Disposition {
    /// Convert the wire disposition into the domain disposition.
    fn from(v: DispositionIn) -> Self {
        match v {
            DispositionIn::Kept { action } => Self::Kept { action },
            DispositionIn::Moved { action } => Self::Moved { action },
            DispositionIn::Merged { target } => Self::Merged {
                target: target.into(),
            },
            DispositionIn::Dropped { kind, reason } => Self::Dropped {
                kind: match kind {
                    DropKindIn::Duplicate => DropKind::Duplicate,
                    DropKindIn::Obsolete => DropKind::Obsolete,
                    DropKindIn::Superseded => DropKind::Superseded,
                },
                reason,
            },
        }
    }
}

/// Convert the wire proposal fields into the validated domain input.
fn proposal(
    title: String,
    sources: Vec<SourceInPayload>,
    actions: Vec<ActionInPayload>,
    sections: Vec<SectionInPayload>,
    preservation: Vec<PreservationInPayload>,
) -> ProposalIn {
    ProposalIn {
        title,
        sources: sources
            .into_iter()
            .map(|s| SourceIn {
                path: s.path,
                version: s.version,
            })
            .collect(),
        actions: actions
            .into_iter()
            .map(|a| ActionIn {
                id: a.id,
                kind: match a.kind {
                    ActionKindIn::Create => ActionKind::Create,
                    ActionKindIn::Replace => ActionKind::Replace,
                    ActionKindIn::Move => ActionKind::Move,
                    ActionKindIn::Remove => ActionKind::Remove,
                },
                path: a.path,
                from: a.from,
                base_version: a.base_version,
                content: a.content,
                purpose: a.purpose,
                reason: a.reason,
                absorbed_into: a.absorbed_into.into_iter().map(Into::into).collect(),
            })
            .collect(),
        sections: sections
            .into_iter()
            .map(|s| SectionIn {
                path: s.path,
                section: s.section.into(),
                disposition: s.disposition.into(),
            })
            .collect(),
        preservation: preservation
            .into_iter()
            .map(|p| PreservationIn {
                id: p.id,
                kind: match p.kind {
                    PreserveKindIn::Requirement => PreserveKind::Requirement,
                    PreserveKindIn::Rationale => PreserveKind::Rationale,
                    PreserveKindIn::Link => PreserveKind::Link,
                    PreserveKindIn::Decision => PreserveKind::Decision,
                    PreserveKindIn::Constraint => PreserveKind::Constraint,
                },
                path: p.path,
                section: p.section.into(),
                statement: p.statement,
                target: p.target.into(),
                mode: match p.mode {
                    PreserveModeIn::Verbatim => PreserveMode::Verbatim,
                    PreserveModeIn::Rewritten => PreserveMode::Rewritten,
                },
            })
            .collect(),
    }
}

/// Bound a note line to 200 bytes on a character boundary.
fn bound(line: &str, cap: usize) -> String {
    let mut end = line.len().min(cap);
    while !line.is_char_boundary(end) {
        end -= 1;
    }
    line[..end].to_owned()
}

/// Map a domain outcome to the shared acknowledgement.
///
/// The phase is the actual lowercase state; notes are at most eight lines of 200 bytes and refs at most
/// sixteen of 256 bytes. When refs overflow the omitted count becomes the last note, never a silent drop.
fn acknowledgement(out: Outcome) -> Ack {
    let mut ack = work::ack(
        out.id.clone(),
        out.version.clone(),
        out.state.label(),
        out.changed,
    );
    let total = out.refs.len();
    ack.refs = out
        .refs
        .into_iter()
        .filter(|r| r.len() <= 256)
        .take(16)
        .collect();
    let omitted = total - ack.refs.len();
    let mut notes: Vec<String> = out.notes.iter().map(|n| bound(n, 200)).collect();
    notes.truncate(if omitted > 0 { 7 } else { 8 });
    if omitted > 0 {
        notes.push(format!(
            "{omitted} references omitted; get_context ref={}",
            out.id
        ));
    }
    ack.notes = notes;
    ack
}

/// Run one decoded operation against a provider adapter and return the shared acknowledgement.
///
/// `common.actor` is required by every operation. `common.version` is the knowledge allocation version
/// for `propose` and the proposal record version for every other operation. The handler never locks,
/// never resolves an alias, never runs Git and never renders; a stop after a publication is an error so
/// the dispatcher settles the call as partial.
pub fn execute_with(
    env: &dyn Env,
    common: &Common,
    op: Compaction,
    effects: &mut Vec<String>,
) -> Result<Ack> {
    let actor = common
        .actor
        .as_deref()
        .filter(|a| !a.trim().is_empty())
        .ok_or_else(|| {
            Error::new(
                "actor_required",
                "actor: a declared actor identity is required",
            )
        })?;
    let version = common.version.as_str();
    let outcome = match op {
        Compaction::Propose {
            request_key,
            title,
            sources,
            actions,
            sections,
            preservation,
        } => {
            let input = proposal(title, sources, actions, sections, preservation);
            ops::propose(env, actor, version, &request_key, &input, effects)?
        }
        Compaction::Revise {
            cp,
            title,
            sources,
            actions,
            sections,
            preservation,
        } => {
            let input = proposal(title, sources, actions, sections, preservation);
            ops::revise(env, actor, version, &cp, &input, effects)?
        }
        Compaction::Review {
            cp,
            revision,
            content_hash,
            verdict,
            summary,
            verified_items,
            findings,
            resolved_findings,
        } => {
            let input = ReviewIn {
                revision,
                content_hash,
                verdict,
                summary,
                verified_items,
                findings,
                resolved: resolved_findings,
            };
            ops::review_cp(env, actor, version, &cp, &input, effects)?
        }
        Compaction::RecoverReviewer {
            cp,
            stage,
            lost,
            unrecoverable,
            observation,
            understanding,
            sources,
            unfinished,
            gaps,
        } => {
            let step = match stage {
                RecoveryStage::Lost => RecoveryStep::Lost,
                RecoveryStage::Immersed => RecoveryStep::Immersed,
            };
            let input = RecoveryIn {
                lost,
                unrecoverable,
                observation,
                understanding,
                sources,
                unfinished,
                gaps,
            };
            ops::recover_reviewer(env, actor, version, &cp, step, &input, effects)?
        }
        Compaction::Apply { cp } => compaction::apply::apply(env, actor, version, &cp, effects)?,
        Compaction::Withdraw {
            cp,
            reason,
            abandon_partial,
        } => ops::withdraw(env, actor, version, &cp, &reason, abandon_partial, effects)?,
    };
    Ok(acknowledgement(outcome))
}

impl Compaction {
    /// The persistence mutation family of this operation, used by the dispatcher to settle Git.
    ///
    /// Reviewer recovery is a review-side mutation and settles as a review; every other operation
    /// maps to its own family. The mapping is total and pure.
    pub fn event_class(&self) -> EventClass {
        match self {
            Self::Propose { .. } => EventClass::CompactionPropose,
            Self::Revise { .. } => EventClass::CompactionRevise,
            Self::Review { .. } | Self::RecoverReviewer { .. } => EventClass::CompactionReview,
            Self::Apply { .. } => EventClass::CompactionApply,
            Self::Withdraw { .. } => EventClass::CompactionWithdraw,
        }
    }
}

/// Run one decoded operation under the held root write lock against the real providers.
///
/// `guard` must be the lock acquired through `store` in this request. Documents, references, the single
/// knowledge allocator, publication and the persistence oracle are the owners' own functions. The
/// dispatcher settles Git after this returns with [`Compaction::event_class`]; the handler itself never
/// commits, and a failure after a publication is an error so the call settles as partial.
///
/// # Errors
/// `not_locked` for a foreign guard, otherwise the errors of [`execute_with`].
pub fn execute_locked(
    store: &Store,
    guard: &LockGuard,
    common: &Common,
    op: Compaction,
    effects: &mut Vec<String>,
) -> Result<Ack> {
    let env = LiveEnv::new(store, guard)?;
    execute_with(&env, common, op, effects)
}

/// Payload, acknowledgement and handler controls against the provider substitute.
#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, reason = "Test assertions")]
    use super::*;
    use crate::compaction::fake::FakeEnv;
    use serde_json::json;

    /// Decode through the shared helper exactly as the registry owner will.
    fn decode(args: serde_json::Value) -> std::result::Result<(Common, Compaction), String> {
        crate::tools::input::mutation::<Compaction>(args, false)
    }

    /// Every operation decodes through the shared closed decoder; an unknown operation or field is
    /// refused naming at most the field, never a value.
    #[test]
    fn payload_is_closed_and_decodes_every_operation() {
        let base = json!({"project": "p", "version": "v", "actor": "a"});
        let with = |extra: serde_json::Value| {
            let mut v = base.clone();
            v.as_object_mut()
                .unwrap()
                .extend(extra.as_object().unwrap().clone());
            v
        };
        for op in [
            json!({"op":"apply","cp":"CP-001"}),
            json!({"op":"withdraw","cp":"CP-001","reason":"r"}),
            json!({"op":"recover_reviewer","cp":"CP-001","stage":"lost","lost":true,"unrecoverable":true,"observation":"o"}),
            json!({"op":"review","cp":"CP-001","revision":1,"content_hash":"h","verdict":"accepted","summary":"s"}),
            json!({"op":"revise","cp":"CP-001","title":"t","sources":[],"actions":[],"sections":[]}),
            json!({"op":"propose","request_key":"key-12345","title":"t","sources":[],"actions":[],"sections":[]}),
        ] {
            decode(with(op.clone())).unwrap_or_else(|e| panic!("{op}: {e}"));
        }
        let unknown_op = decode(with(json!({"op":"preserve_originals","cp":"CP-001"})))
            .err()
            .unwrap();
        assert!(unknown_op.contains("Unknown operation"), "{unknown_op}");
        let extra = decode(with(json!({"op":"apply","cp":"CP-001","paths":["x"]})))
            .err()
            .unwrap();
        assert!(extra.contains("paths") && !extra.contains("x\""), "{extra}");
        let schema = crate::tools::input::mutation_schema::<Compaction>(false);
        assert_eq!(schema["type"], "object");
        assert_eq!(schema["oneOf"].as_array().unwrap().len(), 6);
    }

    /// A missing actor is refused with no effect and names the field.
    #[test]
    fn actor_is_required() {
        let env = FakeEnv::new();
        let common = Common {
            project: "p".into(),
            version: "v".into(),
            actor: None,
            reference: None,
        };
        let mut fx = Vec::new();
        let err = execute_with(
            &env,
            &common,
            Compaction::Apply {
                cp: "CP-001".into(),
            },
            &mut fx,
        )
        .err()
        .unwrap();
        assert_eq!(err.code, "actor_required");
        assert!(err.message.starts_with("actor:") && fx.is_empty());
    }

    /// Every operation maps to its own persistence family, and the locked entry refuses a missing
    /// provider before any effect against a real repository.
    #[test]
    fn locked_entry_maps_families_and_refuses_stale_before_effects() {
        use crate::persist::{EventClass, testing::GitFixture};
        let withdraw = Compaction::Withdraw {
            cp: "CP-001".into(),
            reason: "r".into(),
            abandon_partial: false,
        };
        assert_eq!(withdraw.event_class(), EventClass::CompactionWithdraw);
        assert_eq!(
            Compaction::Apply {
                cp: "CP-001".into()
            }
            .event_class(),
            EventClass::CompactionApply
        );
        let f = GitFixture::new();
        let guard = f.lock();
        let common = Common {
            project: "p".into(),
            version: "alloc".into(),
            actor: Some("author".into()),
            reference: None,
        };
        let propose = Compaction::Propose {
            request_key: "key-live-0002".into(),
            title: "t".into(),
            sources: vec![],
            actions: vec![],
            sections: vec![],
            preservation: vec![],
        };
        assert_eq!(propose.event_class(), EventClass::CompactionPropose);
        let mut fx = Vec::new();
        let err = execute_locked(&f.store, &guard, &common, propose, &mut fx)
            .err()
            .unwrap();
        assert_eq!(err.code, "stale");
        assert!(fx.is_empty() && f.store.publications().is_empty());
    }

    /// The acknowledgement carries the real state, bounded notes and refs, and the record label.
    #[test]
    fn acknowledgement_maps_state_and_bounds() {
        let out = Outcome {
            id: "CP-001".into(),
            version: "v".into(),
            state: compaction::record::CpState::Blocked,
            revision: 2,
            changed: true,
            applied: vec![],
            total: 3,
            blocked: None,
            notes: (0..12).map(|i| "n".repeat(300) + &i.to_string()).collect(),
            refs: (0..20).map(|i| format!("docs/{i}.md")).collect(),
        };
        let ack = acknowledgement(out);
        assert_eq!(
            (ack.phase.as_str(), ack.phase_label),
            ("blocked", "Compaction state")
        );
        assert_eq!(ack.refs.len(), 16);
        assert!(ack.notes.len() <= 8 && ack.notes.iter().all(|n| n.len() <= 200));
        assert!(ack.notes.last().unwrap().contains("4 references omitted"));
    }

    /// A reference too long for the acknowledgement is counted as omitted, never silently dropped.
    #[test]
    fn acknowledgement_counts_every_omitted_reference() {
        let out = Outcome {
            id: "CP-001".into(),
            version: "v".into(),
            state: compaction::record::CpState::Proposed,
            revision: 1,
            changed: true,
            applied: vec![],
            total: 1,
            blocked: None,
            notes: vec![],
            refs: vec!["CP-001".into(), "d/".to_owned() + &"x".repeat(300)],
        };
        let ack = acknowledgement(out);
        assert_eq!(ack.refs, ["CP-001"]);
        assert!(ack.notes.last().unwrap().contains("1 references omitted"));
    }
}
