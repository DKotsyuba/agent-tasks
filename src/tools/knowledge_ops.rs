//! `knowledge_work` producer: the closed operation payload, its description and the locked handler.
//!
//! The registry module decodes arguments with `input::mutation`, exports the schema, takes the root
//! write lock once, calls [`execute_locked`] and settles Git afterwards. This module never locks,
//! never runs Git and never renders text; it returns the existing acknowledgement.
#![allow(
    dead_code,
    reason = "Producer surface consumed by the registry module once it registers the tool"
)]
use super::{
    input::{self, Common},
    work::{self, Ack},
};
use crate::knowledge::{
    self, Alternative, Any, Checklist, ChecklistBody, Conclusion, Decision, DecisionBody, Evidence,
    ItemState, Kind, OpenQuestion, Research, ResearchBody, Retained, Revisioned, Runbook,
    RunbookBody, RunbookInput, Step, UseCheck, UseInput, UseOutcome,
};
use crate::model;
use crate::store::{self, Error, LockGuard, Result, Store};
use schemars::JsonSchema;
use serde::{Deserialize, Deserializer, Serialize, de::DeserializeOwned};

/// Catalog description of the `knowledge_work` tool.
pub const DESCRIPTION: &str = "Purpose: Create, edit, supersede and use typed project knowledge: Decisions, Runbooks, Research and procedural Checklists.\n\
Input: project alias, version and one closed op. The version is the knowledge allocation version printed by project context for create operations and the record version for every other operation. Optional actor is the declared identity and stays unknown when omitted.\n\
Operations: create_decision, create_runbook, create_research, create_checklist, edit_decision, edit_runbook, edit_research, supersede, use_runbook, add_items, resolve_item, complete_checklist, cancel_checklist, reopen_checklist. Edits replace whole fields, omitted fields are kept, null is refused. A changed definition raises the revision and keeps the previous content. Supersede names an existing current record of the same kind. A runbook use records the used revision, the real environment and evidence (a check, an artifact or an observation) and never changes the definition. A checklist item is resolved with a completion fact or a cancellation reason; a checklist completes only when every item is resolved and at least one is done.\n\
Effects: identifiers, revisions and dates are generated. Records are saved under decisions, runbooks, research and checklists. A create also advances the tracked knowledge allocator. History is never dropped unless its bytes are proven in a committed object. Output: target, new version and actual state. No Git is run here.";

/// Decode a supplied patch field: omission keeps the current value and an explicit null is refused.
fn present<'de, T: Deserialize<'de>, D: Deserializer<'de>>(
    d: D,
) -> std::result::Result<Option<T>, D::Error> {
    T::deserialize(d).map(Some)
}

/// Closed knowledge operations; unknown operations and fields are refused.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(tag = "op", rename_all = "snake_case", deny_unknown_fields)]
pub enum KnowledgeOp {
    /// Create a Decision at revision 1.
    CreateDecision {
        /// Short title, at most 256 bytes.
        title: String,
        /// The question decided, at most 1024 bytes.
        question: String,
        /// The adopted statement, at most 2048 bytes.
        decision: String,
        /// Rationale, at most 4096 bytes.
        rationale: String,
        /// Up to eight rejected alternatives.
        #[serde(default)]
        alternatives: Vec<Alternative>,
        /// Up to eight open questions.
        #[serde(default)]
        open_questions: Vec<OpenQuestion>,
        /// Optional managed-document reference.
        #[serde(default)]
        detail: Option<String>,
    },
    /// Create a Runbook at revision 1.
    CreateRunbook {
        /// Short title, at most 256 bytes.
        title: String,
        /// Purpose, at most 1024 bytes.
        purpose: String,
        /// Up to eight prerequisites.
        #[serde(default)]
        prerequisites: Vec<String>,
        /// Up to eight named inputs.
        #[serde(default)]
        inputs: Vec<RunbookInput>,
        /// One to thirty-two ordered steps.
        steps: Vec<Step>,
        /// Up to eight pitfalls.
        #[serde(default)]
        pitfalls: Vec<String>,
        /// Optional managed-document reference.
        #[serde(default)]
        detail: Option<String>,
    },
    /// Create a Research record at revision 1.
    CreateResearch {
        /// Short title, at most 256 bytes.
        title: String,
        /// The researched question, at most 1024 bytes.
        question: String,
        /// One to eight conclusions.
        conclusions: Vec<Conclusion>,
        /// Up to eight evidence entries.
        #[serde(default)]
        evidence: Vec<Evidence>,
        /// Up to eight limitations.
        #[serde(default)]
        limitations: Vec<String>,
        /// Where the findings apply, at most 512 bytes.
        applicability: String,
        /// Optional managed-document reference.
        #[serde(default)]
        detail: Option<String>,
    },
    /// Create an open procedural Checklist.
    CreateChecklist {
        /// Short title, at most 256 bytes.
        title: String,
        /// Purpose, at most 1024 bytes.
        purpose: String,
        /// One to thirty-two item texts; identifiers are generated.
        items: Vec<String>,
    },
    /// Edit a Decision; omitted fields are kept and a changed definition raises the revision.
    EditDecision {
        /// Target `D-001`.
        #[serde(rename = "ref")]
        reference: String,
        /// New title.
        #[serde(default, deserialize_with = "present")]
        #[schemars(with = "String")]
        title: Option<String>,
        /// New question.
        #[serde(default, deserialize_with = "present")]
        #[schemars(with = "String")]
        question: Option<String>,
        /// New adopted statement.
        #[serde(default, deserialize_with = "present")]
        #[schemars(with = "String")]
        decision: Option<String>,
        /// New rationale.
        #[serde(default, deserialize_with = "present")]
        #[schemars(with = "String")]
        rationale: Option<String>,
        /// Replacement alternatives.
        #[serde(default, deserialize_with = "present")]
        #[schemars(with = "Vec<Alternative>")]
        alternatives: Option<Vec<Alternative>>,
        /// Replacement open questions.
        #[serde(default, deserialize_with = "present")]
        #[schemars(with = "Vec<OpenQuestion>")]
        open_questions: Option<Vec<OpenQuestion>>,
        /// New managed-document reference.
        #[serde(default, deserialize_with = "present")]
        #[schemars(with = "String")]
        detail: Option<String>,
    },
    /// Edit a Runbook; omitted fields are kept and a changed definition raises the revision.
    EditRunbook {
        /// Target `RB-001`.
        #[serde(rename = "ref")]
        reference: String,
        /// New title.
        #[serde(default, deserialize_with = "present")]
        #[schemars(with = "String")]
        title: Option<String>,
        /// New purpose.
        #[serde(default, deserialize_with = "present")]
        #[schemars(with = "String")]
        purpose: Option<String>,
        /// Replacement prerequisites.
        #[serde(default, deserialize_with = "present")]
        #[schemars(with = "Vec<String>")]
        prerequisites: Option<Vec<String>>,
        /// Replacement inputs.
        #[serde(default, deserialize_with = "present")]
        #[schemars(with = "Vec<RunbookInput>")]
        inputs: Option<Vec<RunbookInput>>,
        /// Replacement steps.
        #[serde(default, deserialize_with = "present")]
        #[schemars(with = "Vec<Step>")]
        steps: Option<Vec<Step>>,
        /// Replacement pitfalls.
        #[serde(default, deserialize_with = "present")]
        #[schemars(with = "Vec<String>")]
        pitfalls: Option<Vec<String>>,
        /// New managed-document reference.
        #[serde(default, deserialize_with = "present")]
        #[schemars(with = "String")]
        detail: Option<String>,
    },
    /// Edit a Research record; omitted fields are kept and a changed definition raises the revision.
    EditResearch {
        /// Target `RS-001`.
        #[serde(rename = "ref")]
        reference: String,
        /// New title.
        #[serde(default, deserialize_with = "present")]
        #[schemars(with = "String")]
        title: Option<String>,
        /// New question.
        #[serde(default, deserialize_with = "present")]
        #[schemars(with = "String")]
        question: Option<String>,
        /// Replacement conclusions.
        #[serde(default, deserialize_with = "present")]
        #[schemars(with = "Vec<Conclusion>")]
        conclusions: Option<Vec<Conclusion>>,
        /// Replacement evidence.
        #[serde(default, deserialize_with = "present")]
        #[schemars(with = "Vec<Evidence>")]
        evidence: Option<Vec<Evidence>>,
        /// Replacement limitations.
        #[serde(default, deserialize_with = "present")]
        #[schemars(with = "Vec<String>")]
        limitations: Option<Vec<String>>,
        /// New applicability.
        #[serde(default, deserialize_with = "present")]
        #[schemars(with = "String")]
        applicability: Option<String>,
        /// New managed-document reference.
        #[serde(default, deserialize_with = "present")]
        #[schemars(with = "String")]
        detail: Option<String>,
    },
    /// Mark a record superseded by an existing current record of the same kind.
    Supersede {
        /// Predecessor, for example `D-001`.
        #[serde(rename = "ref")]
        reference: String,
        /// Successor of the same kind, for example `D-002`.
        successor: String,
    },
    /// Record an actual use of one runbook revision; never changes the definition.
    UseRunbook {
        /// Target `RB-001`.
        #[serde(rename = "ref")]
        reference: String,
        /// The definition revision that was used.
        revision: u64,
        /// Reported outcome.
        outcome: UseOutcome,
        /// Real environment of the use, at most 256 bytes.
        environment: String,
        /// Up to eight reported checks.
        #[serde(default)]
        checks: Vec<UseCheck>,
        /// Up to eight artifact references.
        #[serde(default)]
        artifacts: Vec<String>,
        /// Optional manual observation, at most 1024 bytes.
        #[serde(default)]
        observation: Option<String>,
        /// Optional canonical work reference the use belongs to.
        #[serde(default)]
        work: Option<String>,
    },
    /// Append open items to an open checklist.
    AddItems {
        /// Target `CL-001`.
        #[serde(rename = "ref")]
        reference: String,
        /// Item texts.
        items: Vec<String>,
    },
    /// Resolve one open item with a completion fact or a cancellation reason.
    ResolveItem {
        /// Target `CL-001`.
        #[serde(rename = "ref")]
        reference: String,
        /// Item identifier such as `I-001`.
        item: String,
        /// `done` or `canceled`.
        state: ItemState,
        /// The completion fact or cancellation reason, at most 512 bytes.
        text: String,
    },
    /// Complete a checklist whose items are all resolved with at least one done.
    CompleteChecklist {
        /// Target `CL-001`.
        #[serde(rename = "ref")]
        reference: String,
    },
    /// Cancel a checklist with a reason.
    CancelChecklist {
        /// Target `CL-001`.
        #[serde(rename = "ref")]
        reference: String,
        /// The reason, at most 512 bytes.
        reason: String,
    },
    /// Reopen a completed or canceled checklist with a reason.
    ReopenChecklist {
        /// Target `CL-001`.
        #[serde(rename = "ref")]
        reference: String,
        /// The reason, at most 512 bytes.
        reason: String,
    },
}

/// Refuse caller-supplied input with the stable argument code.
fn bad(message: impl Into<String>) -> Error {
    knowledge::arguments(message)
}

/// Route a field-named validation message (`<field>: <rule>`) through the shared `input::field`.
fn field(checked: std::result::Result<(), String>) -> Result<()> {
    match checked {
        Ok(()) => Ok(()),
        Err(message) => match message.split_once(": ") {
            Some((name, rule)) => input::field(name, Err::<(), _>(rule.to_owned())),
            None => input::field("input", Err::<(), _>(message)),
        },
    }
}

/// Build the shared acknowledgement for a typed record; its label derives from the identifier.
fn acknowledge(id: &str, version: String, phase: &str, changed: bool) -> Ack {
    let mut ack = work::ack(id, version, phase, changed);
    ack.refs.push(id.to_owned());
    ack
}

/// Parse a record reference and require the expected kind.
fn record_id(reference: &str, expected: Kind) -> Result<String> {
    let kind = Kind::of(reference).map_err(|e| bad(format!("ref: {e}")))?;
    if kind != expected {
        return Err(bad(format!(
            "ref: expected a {} identifier.",
            expected.prefix().as_str()
        )));
    }
    Ok(reference.to_owned())
}

/// Validate an optional detail reference through the document module's reference validator.
///
/// A supplied detail must be a root-relative managed document path with an optional fragment whose
/// document and section exist; the provider's refusal is reported under the `detail` field and any
/// other failure is passed through unchanged. Absent details need no validation.
fn check_detail(store: &Store, detail: &Option<String>) -> Result<()> {
    let Some(raw) = detail else {
        return Ok(());
    };
    match crate::references::valid_reference(&crate::documents::StorePort::new(store), raw) {
        Err(e) if e.code == "invalid_arguments" => Err(bad(format!(
            "detail: {}",
            e.message.trim_start_matches("reference: ")
        ))),
        other => other,
    }
}

/// Require the caller's expected version to equal the record's current file version.
fn require_version(common: &Common, current: &str) -> Result<()> {
    if common.version != current {
        return Err(Error::new(
            "stale",
            format!("No work saved. Current version: {current}. Read get_context before retrying."),
        ));
    }
    Ok(())
}

/// Create one record: check the token, create the home, reserve, validate and publish.
fn create<R: Serialize + DeserializeOwned>(
    store: &Store,
    guard: &LockGuard,
    common: &Common,
    kind: Kind,
    build: impl FnOnce(&str, &str, &Option<String>) -> R,
    validate: impl FnOnce(&R) -> std::result::Result<(), String>,
    effects: &mut Vec<String>,
) -> Result<Ack> {
    let phase = if kind == Kind::Checklist {
        "open"
    } else {
        "current"
    };
    knowledge::require_project(store)?;
    let current = knowledge::allocation_version(store)?;
    if current != common.version {
        return Err(Error::new(
            "stale",
            format!(
                "No identifier reserved. Current allocation version: {current}. Read project context before retrying."
            ),
        ));
    }
    knowledge::ensure_home(store, kind.prefix(), effects)?;
    let id = knowledge::reserve(store, guard, kind.prefix(), &common.version, effects)?;
    let at = store::now();
    let record = build(&id, &at, &common.actor);
    validate(&record).map_err(|e| {
        Error::new(
            "invalid_data",
            format!("{id} was reserved but its record is invalid ({e}); the number stays a gap."),
        )
    })?;
    let relative = kind.path(&id).map_err(store::invalid)?;
    knowledge::create_file(store, &relative, &record, effects)?;
    let version = store.version(&relative, Some(&store::encode(&record)?));
    Ok(acknowledge(&id, version, phase, true))
}

/// Load a record of an expected kind and check the caller's version against its exact bytes.
fn load_checked(
    store: &Store,
    common: &Common,
    reference: &str,
    kind: Kind,
) -> Result<(String, String, store::Snapshot<Any>)> {
    let id = record_id(reference, kind)?;
    let relative = kind.path(&id).map_err(store::invalid)?;
    let snapshot = knowledge::load(store, &id)?;
    require_version(common, &snapshot.version)?;
    Ok((id, relative, snapshot))
}

/// Apply an edit transition to a loaded record and publish it when it changed.
#[allow(clippy::too_many_arguments, reason = "One cohesive transition input")]
fn finish<R: Serialize + DeserializeOwned + Retained>(
    store: &Store,
    id: &str,
    relative: &str,
    observed: &[u8],
    mut record: R,
    changed: bool,
    phase: &str,
    current_version: String,
    effects: &mut Vec<String>,
    evict: &mut dyn FnMut() -> Result<String>,
) -> Result<Ack> {
    if !changed {
        return Ok(acknowledge(id, current_version, phase, false));
    }
    let version = knowledge::replace_file(store, relative, observed, &mut record, evict, effects)?;
    Ok(acknowledge(id, version, phase, true))
}

/// Lowercase state label of a revisioned record.
fn phase_of<R: Revisioned>(record: &R) -> &'static str {
    match record.lifecycle().state {
        knowledge::Currentness::Current => "current",
        knowledge::Currentness::Superseded => "superseded",
    }
}

/// Verify a runbook use's work reference exists and that the actor matches a bound lead.
fn bind_work(store: &Store, work: &str, actor: &Option<String>) -> Result<()> {
    let head = work::module_id(work)?;
    let tail = work.split_once('/').map(|(_, t)| t);
    if let Some(t) = tail
        && !(model::number(t, "T-").is_ok() || model::number(t, "A-").is_ok())
    {
        return Err(bad("work: expected a canonical work reference."));
    }
    let module = store.module(head).map_err(|e| {
        if e.code == "not_found" {
            bad("work: that work record does not exist.")
        } else {
            e
        }
    })?;
    if let Some(t) = tail
        && !module.value.tasks.iter().any(|task| task.id == t)
    {
        return Err(bad("work: that task or atomic does not exist."));
    }
    let lead = module
        .value
        .workflow
        .as_ref()
        .and_then(|w| w.core.as_ref())
        .and_then(|c| {
            c.bindings
                .iter()
                .find(|b| b.role == model::AgentRole::Lead && b.current.loss.is_none())
        })
        .map(|b| b.current.identity.agent_id.clone());
    if let Some(lead) = lead
        && actor.as_deref() != Some(lead.as_str())
    {
        return Err(Error::new(
            "attribution",
            "The work has a bound lead; declare that lead as the actor to report a use against it.",
        ));
    }
    Ok(())
}

/// Execute one knowledge operation under the root write lock the caller already holds.
///
/// Create operations compare `common.version` with the knowledge allocation version, reserve an
/// identifier and publish the allocator before the record; every other operation compares it with
/// the record's current version. Nothing is published when validation, version or lifecycle checks
/// refuse. The returned acknowledgement carries the record, its new version and its actual state.
pub fn execute_locked(
    store: &Store,
    guard: &LockGuard,
    common: &Common,
    op: KnowledgeOp,
    effects: &mut Vec<String>,
) -> Result<Ack> {
    match op {
        KnowledgeOp::CreateDecision {
            title,
            question,
            decision,
            rationale,
            alternatives,
            open_questions,
            detail,
        } => {
            let body = DecisionBody {
                title,
                question,
                decision,
                rationale,
                alternatives,
                open_questions,
                detail,
            };
            field(body.validate())?;
            check_detail(store, &body.detail)?;
            create(
                store,
                guard,
                common,
                Kind::Decision,
                |id, at, by| knowledge::new_decision(id, body, at, by),
                Decision::validate,
                effects,
            )
        }
        KnowledgeOp::CreateRunbook {
            title,
            purpose,
            prerequisites,
            inputs,
            steps,
            pitfalls,
            detail,
        } => {
            let body = RunbookBody {
                title,
                purpose,
                prerequisites,
                inputs,
                steps,
                pitfalls,
                detail,
            };
            field(body.validate())?;
            check_detail(store, &body.detail)?;
            create(
                store,
                guard,
                common,
                Kind::Runbook,
                |id, at, by| knowledge::new_runbook(id, body, at, by),
                Runbook::validate,
                effects,
            )
        }
        KnowledgeOp::CreateResearch {
            title,
            question,
            conclusions,
            evidence,
            limitations,
            applicability,
            detail,
        } => {
            let body = ResearchBody {
                title,
                question,
                conclusions,
                evidence,
                limitations,
                applicability,
                detail,
            };
            field(body.validate())?;
            check_detail(store, &body.detail)?;
            create(
                store,
                guard,
                common,
                Kind::Research,
                |id, at, by| knowledge::new_research(id, body, at, by),
                Research::validate,
                effects,
            )
        }
        KnowledgeOp::CreateChecklist {
            title,
            purpose,
            items,
        } => {
            let probe = ChecklistBody {
                title: title.clone(),
                purpose: purpose.clone(),
                items: items
                    .iter()
                    .enumerate()
                    .map(|(n, text)| knowledge::Item {
                        id: format!("I-{:03}", n + 1),
                        text: text.clone(),
                        state: ItemState::Open,
                        resolution: None,
                    })
                    .collect(),
            };
            field(probe.validate())?;
            create(
                store,
                guard,
                common,
                Kind::Checklist,
                |id, at, by| knowledge::new_checklist(id, title, purpose, items, at, by),
                Checklist::validate,
                effects,
            )
        }
        KnowledgeOp::EditDecision {
            reference,
            title,
            question,
            decision,
            rationale,
            alternatives,
            open_questions,
            detail,
        } => {
            let (id, relative, snap) = load_checked(store, common, &reference, Kind::Decision)?;
            let Any::Decision(mut record) = snap.value else {
                return Err(store::invalid("Record kind mismatch."));
            };
            let mut body = record.content.clone();
            body.title = title.unwrap_or(body.title);
            body.question = question.unwrap_or(body.question);
            body.decision = decision.unwrap_or(body.decision);
            body.rationale = rationale.unwrap_or(body.rationale);
            body.alternatives = alternatives.unwrap_or(body.alternatives);
            body.open_questions = open_questions.unwrap_or(body.open_questions);
            if detail.is_some() {
                check_detail(store, &detail)?;
                body.detail = detail;
            }
            field(body.validate())?;
            let at = store::now();
            let mut evict = knowledge::evictor(store, &relative, &snap.bytes);
            let changed = record.edit(body, &at, &common.actor, &mut evict)?;
            let phase = phase_of(&record);
            finish(
                store,
                &id,
                &relative,
                &snap.bytes,
                record,
                changed,
                phase,
                snap.version,
                effects,
                &mut evict,
            )
        }
        KnowledgeOp::EditRunbook {
            reference,
            title,
            purpose,
            prerequisites,
            inputs,
            steps,
            pitfalls,
            detail,
        } => {
            let (id, relative, snap) = load_checked(store, common, &reference, Kind::Runbook)?;
            let Any::Runbook(mut record) = snap.value else {
                return Err(store::invalid("Record kind mismatch."));
            };
            let mut body = record.content.clone();
            body.title = title.unwrap_or(body.title);
            body.purpose = purpose.unwrap_or(body.purpose);
            body.prerequisites = prerequisites.unwrap_or(body.prerequisites);
            body.inputs = inputs.unwrap_or(body.inputs);
            body.steps = steps.unwrap_or(body.steps);
            body.pitfalls = pitfalls.unwrap_or(body.pitfalls);
            if detail.is_some() {
                check_detail(store, &detail)?;
                body.detail = detail;
            }
            field(body.validate())?;
            let at = store::now();
            let mut evict = knowledge::evictor(store, &relative, &snap.bytes);
            let changed = record.edit(body, &at, &common.actor, &mut evict)?;
            let phase = phase_of(&record);
            finish(
                store,
                &id,
                &relative,
                &snap.bytes,
                record,
                changed,
                phase,
                snap.version,
                effects,
                &mut evict,
            )
        }
        KnowledgeOp::EditResearch {
            reference,
            title,
            question,
            conclusions,
            evidence,
            limitations,
            applicability,
            detail,
        } => {
            let (id, relative, snap) = load_checked(store, common, &reference, Kind::Research)?;
            let Any::Research(mut record) = snap.value else {
                return Err(store::invalid("Record kind mismatch."));
            };
            let mut body = record.content.clone();
            body.title = title.unwrap_or(body.title);
            body.question = question.unwrap_or(body.question);
            body.conclusions = conclusions.unwrap_or(body.conclusions);
            body.evidence = evidence.unwrap_or(body.evidence);
            body.limitations = limitations.unwrap_or(body.limitations);
            body.applicability = applicability.unwrap_or(body.applicability);
            if detail.is_some() {
                check_detail(store, &detail)?;
                body.detail = detail;
            }
            field(body.validate())?;
            let at = store::now();
            let mut evict = knowledge::evictor(store, &relative, &snap.bytes);
            let changed = record.edit(body, &at, &common.actor, &mut evict)?;
            let phase = phase_of(&record);
            finish(
                store,
                &id,
                &relative,
                &snap.bytes,
                record,
                changed,
                phase,
                snap.version,
                effects,
                &mut evict,
            )
        }
        KnowledgeOp::Supersede {
            reference,
            successor,
        } => {
            let predecessor_kind = Kind::of(&reference).map_err(|e| bad(format!("ref: {e}")))?;
            if predecessor_kind == Kind::Checklist {
                return Err(bad(
                    "ref: checklists are not superseded; cancel them instead.",
                ));
            }
            let successor_id = record_id(&successor, predecessor_kind)
                .map_err(|_| bad("successor: expected an existing record of the same kind."))?;
            if successor_id == reference {
                return Err(bad("successor: a record cannot supersede itself."));
            }
            let (id, relative, snap) = load_checked(store, common, &reference, predecessor_kind)?;
            let next = knowledge::load(store, &successor_id)?;
            if !next.value.current() {
                return Err(Error::new(
                    "conflict",
                    "The successor is itself superseded; name its current successor instead.",
                ));
            }
            let at = store::now();
            let mut evict = knowledge::evictor(store, &relative, &snap.bytes);
            let acknowledged = match snap.value {
                Any::Decision(mut record) => {
                    let changed = record.supersede(&successor_id, &at, &common.actor)?;
                    let phase = phase_of(&record);
                    finish(
                        store,
                        &id,
                        &relative,
                        &snap.bytes,
                        record,
                        changed,
                        phase,
                        snap.version,
                        effects,
                        &mut evict,
                    )
                }
                Any::Research(mut record) => {
                    let changed = record.supersede(&successor_id, &at, &common.actor)?;
                    let phase = phase_of(&record);
                    finish(
                        store,
                        &id,
                        &relative,
                        &snap.bytes,
                        record,
                        changed,
                        phase,
                        snap.version,
                        effects,
                        &mut evict,
                    )
                }
                Any::Runbook(mut record) => {
                    let changed = record.supersede(&successor_id, &at, &common.actor)?;
                    let phase = phase_of(&record);
                    finish(
                        store,
                        &id,
                        &relative,
                        &snap.bytes,
                        record,
                        changed,
                        phase,
                        snap.version,
                        effects,
                        &mut evict,
                    )
                }
                Any::Checklist(_) => Err(store::invalid("Record kind mismatch.")),
            };
            acknowledged.map(|mut ack| {
                ack.refs.push(successor_id);
                ack
            })
        }
        KnowledgeOp::UseRunbook {
            reference,
            revision,
            outcome,
            environment,
            checks,
            artifacts,
            observation,
            work,
        } => {
            let (id, relative, snap) = load_checked(store, common, &reference, Kind::Runbook)?;
            let Any::Runbook(mut record) = snap.value else {
                return Err(store::invalid("Record kind mismatch."));
            };
            if let Some(work) = &work {
                bind_work(store, work, &common.actor)?;
            }
            let input = UseInput {
                revision,
                environment,
                outcome,
                checks,
                artifacts,
                observation,
                work,
            };
            let at = store::now();
            let mut evict = knowledge::evictor(store, &relative, &snap.bytes);
            record.add_use(input, &at, &common.actor, &mut evict)?;
            let phase = phase_of(&record);
            finish(
                store,
                &id,
                &relative,
                &snap.bytes,
                record,
                true,
                phase,
                snap.version,
                effects,
                &mut evict,
            )
        }
        KnowledgeOp::AddItems { reference, items } => {
            checklist_step(store, common, &reference, effects, |c, at, by, evict| {
                c.add_items(items, at, by, evict)
            })
        }
        KnowledgeOp::ResolveItem {
            reference,
            item,
            state,
            text,
        } => {
            let item_ref = format!("{reference}/{item}");
            checklist_step(store, common, &reference, effects, |c, at, by, evict| {
                c.resolve_item(&item, state, &text, at, by, evict)
            })
            .map(|mut ack| {
                ack.refs.push(item_ref);
                ack
            })
        }
        KnowledgeOp::CompleteChecklist { reference } => {
            checklist_step(store, common, &reference, effects, |c, at, by, evict| {
                c.complete(at, by, evict)
            })
        }
        KnowledgeOp::CancelChecklist { reference, reason } => {
            checklist_step(store, common, &reference, effects, |c, at, by, evict| {
                c.cancel(&reason, at, by, evict)
            })
        }
        KnowledgeOp::ReopenChecklist { reference, reason } => {
            checklist_step(store, common, &reference, effects, |c, at, by, evict| {
                c.reopen(&reason, at, by, evict)
            })
        }
    }
}

/// Load a checklist, apply one transition and publish the replacement.
fn checklist_step(
    store: &Store,
    common: &Common,
    reference: &str,
    effects: &mut Vec<String>,
    step: impl FnOnce(
        &mut Checklist,
        &str,
        &Option<String>,
        &mut dyn FnMut() -> Result<String>,
    ) -> Result<()>,
) -> Result<Ack> {
    let (id, relative, snap) = load_checked(store, common, reference, Kind::Checklist)?;
    let Any::Checklist(mut record) = snap.value else {
        return Err(store::invalid("Record kind mismatch."));
    };
    let at = store::now();
    let mut evict = knowledge::evictor(store, &relative, &snap.bytes);
    step(&mut record, &at, &common.actor, &mut evict)?;
    let phase = match record.state {
        knowledge::ChecklistState::Open => "open",
        knowledge::ChecklistState::Completed => "completed",
        knowledge::ChecklistState::Canceled => "canceled",
    };
    finish(
        store,
        &id,
        &relative,
        &snap.bytes,
        record,
        true,
        phase,
        snap.version,
        effects,
        &mut evict,
    )
}

#[cfg(test)]
mod tests {
    #![allow(
        clippy::unwrap_used,
        clippy::expect_used,
        reason = "Fixture construction and explicit assertions"
    )]
    use super::*;
    use crate::model::Project;
    use serde_json::{Value, json};
    use std::fs;

    /// Disposable initialized root with its write lock held for the whole test.
    struct Fx {
        /// Keeps the directory alive.
        _dir: tempfile::TempDir,
        /// Store over the disposable root.
        store: Store,
        /// The held root write lock.
        guard: LockGuard,
    }

    /// Create a project root, a work allocator file and the held lock.
    fn fixture() -> Fx {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::from_root(&dir.path().canonicalize().unwrap()).unwrap();
        let mut fx = Vec::new();
        store.prepare(&mut fx).unwrap();
        let project = Project {
            schema_version: 1,
            title: "Fixture".into(),
            purpose: "Knowledge tests".into(),
            remote: None,
            created_at: store::now(),
            updated_at: store::now(),
        };
        store
            .save("project.yaml", &project, None, false, &mut fx)
            .unwrap();
        fs::write(
            dir.path().join(".agent-tasks/state.yaml"),
            "schema_version: 2\nnext_module: 1\nnext_epic: 1\nnext_atomic: 1\n",
        )
        .unwrap();
        let guard = store.lock(true, &mut Vec::new()).unwrap().unwrap();
        Fx {
            _dir: dir,
            store,
            guard,
        }
    }

    /// Decode a JSON argument object exactly as the registry module does.
    fn decode(args: Value) -> std::result::Result<(Common, KnowledgeOp), String> {
        super::super::input::mutation::<KnowledgeOp>(args, false)
    }

    /// Run one operation given as JSON with a version; returns the acknowledgement and the ledger.
    fn run(fx: &Fx, version: &str, op: Value) -> (Result<Ack>, Vec<String>) {
        let mut args = op;
        args["project"] = json!("p");
        args["version"] = json!(version);
        args["actor"] = json!("tester");
        let (common, op) = decode(args).unwrap();
        let mut effects = Vec::new();
        let result = execute_locked(&fx.store, &fx.guard, &common, op, &mut effects);
        (result, effects)
    }

    /// Create with the current allocation version and return the acknowledgement.
    fn create(fx: &Fx, op: Value) -> Ack {
        let token = knowledge::allocation_version(&fx.store).unwrap();
        let (result, _) = run(fx, &token, op);
        result.unwrap()
    }

    /// A decision creation payload.
    fn decision(title: &str) -> Value {
        json!({"op":"create_decision","title":title,"question":"Which store?","decision":"Use files.","rationale":"Portable."})
    }

    /// A runbook creation payload.
    fn runbook() -> Value {
        json!({"op":"create_runbook","title":"Deploy","purpose":"Ship it",
            "steps":[{"title":"Build","command":null,"description":"compile","expected":"binary","recovery":null}]})
    }

    #[test]
    fn yaml_sensitive_prose_round_trips_or_is_refused_without_effect() {
        let fx = fixture();
        let token = knowledge::allocation_version(&fx.store).unwrap();
        let mut op = decision("R&D *spike* !done");
        op["rationale"] = json!(
            "Use A & B, then *check* !done 'quote' \"dq\" # hash: colon, [x] {y} | > - ? @ ` %"
        );
        let (result, effects) = run(&fx, &token, op);
        match result {
            Ok(_) => {
                let Any::Decision(record) = knowledge::load(&fx.store, "D-001").unwrap().value
                else {
                    panic!()
                };
                assert_eq!(record.content.title, "R&D *spike* !done");
                assert!(record.content.rationale.contains("A & B"));
            }
            Err(e) => {
                assert_eq!(e.code, "encoding_unreadable", "{}", e.message);
                assert!(!effects.iter().any(|f| f.contains("decisions/D-001.yaml")));
            }
        }
    }

    #[test]
    fn detail_is_stored_only_after_the_real_document_reference_validator() {
        let fx = fixture();
        fs::create_dir_all(fx.store.path("docs").unwrap()).unwrap();
        fs::write(
            fx.store.path("docs/why.md").unwrap(),
            "# Context\n\nBecause.\n",
        )
        .unwrap();
        let token = knowledge::allocation_version(&fx.store).unwrap();
        for (raw, label) in [
            ("docs/missing.md", "missing document"),
            ("docs/why.md#nowhere", "missing fragment"),
            ("D-001", "typed id"),
            ("/docs/why.md", "absolute path"),
            ("docs/../docs/why.md", "dot segments"),
        ] {
            let mut op = decision("Pick");
            op["detail"] = json!(raw);
            let (result, effects) = run(&fx, &token, op);
            let err = result
                .err()
                .unwrap_or_else(|| panic!("{label} must be refused"));
            assert_eq!(err.code, "invalid_arguments", "{label}");
            assert!(
                err.message.starts_with("detail:"),
                "{label}: {}",
                err.message
            );
            assert!(effects.is_empty(), "{label} published something");
        }
        let mut ok = decision("Pick");
        ok["detail"] = json!("docs/why.md#context");
        let created = run(&fx, &token, ok).0.unwrap();
        let Any::Decision(record) = knowledge::load(&fx.store, &created.target).unwrap().value
        else {
            panic!()
        };
        assert_eq!(
            record.content.detail.as_deref(),
            Some("docs/why.md#context")
        );
        let (edit, _) = run(
            &fx,
            &created.version,
            json!({"op":"edit_decision","ref":"D-001","detail":"docs/missing.md"}),
        );
        assert_eq!(edit.err().unwrap().code, "invalid_arguments");
    }

    #[test]
    fn create_publishes_allocator_then_record_with_generated_metadata() {
        let fx = fixture();
        let token = knowledge::allocation_version(&fx.store).unwrap();
        let (result, effects) = run(&fx, &token, decision("Pick"));
        let ack = result.unwrap();
        assert_eq!(
            effects,
            [
                "Created directory decisions/.",
                "Published .agent-tasks/knowledge.yaml.",
                "Published decisions/D-001.yaml."
            ]
        );
        assert_eq!(
            (ack.target.as_str(), ack.phase.as_str(), ack.changed),
            ("D-001", "current", true)
        );
        assert_eq!(ack.phase_label, "Decision state");
        assert_eq!(ack.refs, ["D-001"]);
        let loaded = knowledge::load(&fx.store, "D-001").unwrap();
        assert_eq!(ack.version, loaded.version);
        let Any::Decision(record) = loaded.value else {
            panic!("kind")
        };
        assert_eq!(record.revision, 1);
        assert_eq!(record.meta.created_by.as_deref(), Some("tester"));
        assert!(record.meta.created_at.ends_with('Z'));
        let second = create(&fx, decision("Next"));
        assert_eq!(second.target, "D-002", "numbers are never reused");
    }

    #[test]
    fn invalid_or_stale_creates_reserve_and_publish_nothing() {
        let fx = fixture();
        let token = knowledge::allocation_version(&fx.store).unwrap();
        let mut bad = decision(" ");
        bad["alternatives"] = json!([]);
        let (result, effects) = run(&fx, &token, bad);
        let err = result.err().unwrap();
        assert_eq!(err.code, "invalid_arguments");
        assert!(err.message.starts_with("title:"), "{}", err.message);
        assert!(effects.is_empty());
        assert!(fx.store.bytes(knowledge::ALLOCATOR_PATH).unwrap().is_none());
        assert!(!fx.store.path("decisions").unwrap().exists());
        let (result, effects) = run(&fx, "stale", decision("Pick"));
        assert_eq!(result.err().unwrap().code, "stale");
        assert!(effects.is_empty());
        assert!(
            !fx.store.path("decisions").unwrap().exists(),
            "a stale call creates no home"
        );
        let mut detail = decision("Pick");
        detail["detail"] = json!("docs/x.md");
        let (result, effects) = run(&fx, &token, detail);
        assert_eq!(result.err().unwrap().code, "invalid_arguments");
        assert!(effects.is_empty());
    }

    #[test]
    fn edits_are_version_guarded_revisioned_and_idempotent() {
        let fx = fixture();
        let created = create(&fx, decision("Pick"));
        let (stale, _) = run(
            &fx,
            "wrong",
            json!({"op":"edit_decision","ref":"D-001","title":"X"}),
        );
        assert_eq!(stale.err().unwrap().code, "stale");
        let before = fs::read(fx.store.path("decisions/D-001.yaml").unwrap()).unwrap();
        let (same, effects) = run(
            &fx,
            &created.version,
            json!({"op":"edit_decision","ref":"D-001","title":"Pick"}),
        );
        let same = same.unwrap();
        assert!(!same.changed && same.version == created.version && effects.is_empty());
        let (edited, effects) = run(
            &fx,
            &created.version,
            json!({"op":"edit_decision","ref":"D-001","rationale":"Because"}),
        );
        let edited = edited.unwrap();
        assert!(edited.changed);
        assert_eq!(effects, ["Published decisions/D-001.yaml."]);
        let Any::Decision(record) = knowledge::load(&fx.store, "D-001").unwrap().value else {
            panic!()
        };
        assert_eq!(
            (
                record.revision,
                record.history.len(),
                record.content.rationale.as_str()
            ),
            (2, 1, "Because")
        );
        assert_eq!(record.history[0].content.rationale, "Portable.");
        let (stale, _) = run(
            &fx,
            &created.version,
            json!({"op":"edit_decision","ref":"D-001","title":"Y"}),
        );
        assert_eq!(
            stale.err().unwrap().code,
            "stale",
            "the old version no longer applies"
        );
        assert_ne!(
            before,
            fs::read(fx.store.path("decisions/D-001.yaml").unwrap()).unwrap()
        );
        let (wrong_kind, _) = run(
            &fx,
            &edited.version,
            json!({"op":"edit_runbook","ref":"D-001","title":"Y"}),
        );
        assert_eq!(wrong_kind.err().unwrap().code, "invalid_arguments");
    }

    #[test]
    fn supersession_requires_an_existing_current_same_kind_successor() {
        let fx = fixture();
        let one = create(&fx, decision("One"));
        let two = create(&fx, decision("Two"));
        let rb = create(&fx, runbook());
        for (reference, successor, code) in [
            ("D-001", "D-001", "invalid_arguments"),
            ("D-001", "RB-001", "invalid_arguments"),
            ("D-001", "D-009", "not_found"),
        ] {
            let (result, effects) = run(
                &fx,
                &one.version,
                json!({"op":"supersede","ref":reference,"successor":successor}),
            );
            assert_eq!(
                result.err().unwrap().code,
                code,
                "{reference} -> {successor}"
            );
            assert!(effects.is_empty());
        }
        let (result, effects) = run(
            &fx,
            &one.version,
            json!({"op":"supersede","ref":"D-001","successor":"D-002"}),
        );
        let done = result.unwrap();
        assert_eq!((done.phase.as_str(), done.changed), ("superseded", true));
        assert_eq!(done.refs, ["D-001", "D-002"]);
        assert_eq!(
            effects,
            ["Published decisions/D-001.yaml."],
            "exactly one file is written"
        );
        let (again, effects) = run(
            &fx,
            &done.version,
            json!({"op":"supersede","ref":"D-001","successor":"D-002"}),
        );
        assert!(!again.unwrap().changed && effects.is_empty());
        let three = create(&fx, decision("Three"));
        let (conflict, _) = run(
            &fx,
            &done.version,
            json!({"op":"supersede","ref":"D-001","successor":"D-003"}),
        );
        assert_eq!(conflict.err().unwrap().code, "conflict");
        let (cycle, _) = run(
            &fx,
            &two.version,
            json!({"op":"supersede","ref":"D-002","successor":"D-001"}),
        );
        assert_eq!(
            cycle.err().unwrap().code,
            "conflict",
            "a superseded successor is refused"
        );
        let (edit, _) = run(
            &fx,
            &done.version,
            json!({"op":"edit_decision","ref":"D-001","title":"No"}),
        );
        assert_eq!(edit.err().unwrap().code, "conflict");
        let (list, _) = run(
            &fx,
            &rb.version,
            json!({"op":"supersede","ref":"CL-001","successor":"CL-002"}),
        );
        assert_eq!(list.err().unwrap().code, "invalid_arguments");
        let _ = three;
    }

    #[test]
    fn runbook_use_is_an_observation_bound_to_a_revision() {
        let fx = fixture();
        let created = create(&fx, runbook());
        let use_op = |revision: u64| {
            json!({"op":"use_runbook","ref":"RB-001","revision":revision,"outcome":"succeeded","environment":"staging",
                "checks":[{"label":"smoke","status":"passed","detail":null}]})
        };
        let (ok, effects) = run(&fx, &created.version, use_op(1));
        let used = ok.unwrap();
        assert_eq!(effects, ["Published runbooks/RB-001.yaml."]);
        let Any::Runbook(record) = knowledge::load(&fx.store, "RB-001").unwrap().value else {
            panic!()
        };
        assert_eq!(
            (
                record.revision,
                record.uses.len(),
                record.uses[0].id.as_str()
            ),
            (1, 1, "U-001")
        );
        assert_eq!(record.uses[0].by.as_deref(), Some("tester"));
        let (missing, _) = run(&fx, &used.version, use_op(7));
        assert_eq!(missing.err().unwrap().code, "not_found");
        let mut no_evidence = use_op(1);
        no_evidence["checks"] = json!([]);
        let (none, _) = run(&fx, &used.version, no_evidence);
        assert!(none.err().unwrap().message.starts_with("evidence:"));
        let mut work = use_op(1);
        work["work"] = json!("M-009");
        let (unknown, effects) = run(&fx, &used.version, work);
        assert_eq!(unknown.err().unwrap().code, "invalid_arguments");
        assert!(effects.is_empty());
    }

    #[test]
    fn checklist_lifecycle_runs_through_operations() {
        let fx = fixture();
        let created = create(
            &fx,
            json!({"op":"create_checklist","title":"Release","purpose":"Steps","items":["build","tag"]}),
        );
        assert_eq!(created.phase, "open");
        let step = |version: &str, op: Value| run(&fx, version, op).0.unwrap();
        let a = step(
            &created.version,
            json!({"op":"resolve_item","ref":"CL-001","item":"I-001","state":"done","text":"built"}),
        );
        assert_eq!(a.refs, ["CL-001", "CL-001/I-001"]);
        let (early, _) = run(
            &fx,
            &a.version,
            json!({"op":"complete_checklist","ref":"CL-001"}),
        );
        assert_eq!(early.err().unwrap().code, "conflict");
        let b = step(
            &a.version,
            json!({"op":"resolve_item","ref":"CL-001","item":"I-002","state":"canceled","text":"not needed"}),
        );
        let c = step(
            &b.version,
            json!({"op":"complete_checklist","ref":"CL-001"}),
        );
        assert_eq!(c.phase, "completed");
        let (closed, _) = run(
            &fx,
            &c.version,
            json!({"op":"add_items","ref":"CL-001","items":["late"]}),
        );
        assert_eq!(closed.err().unwrap().code, "conflict");
        let d = step(
            &c.version,
            json!({"op":"reopen_checklist","ref":"CL-001","reason":"found a gap"}),
        );
        assert_eq!(d.phase, "open");
        let e = step(
            &d.version,
            json!({"op":"cancel_checklist","ref":"CL-001","reason":"abandoned"}),
        );
        assert_eq!(e.phase, "canceled");
        let (nothing, _) = run(
            &fx,
            &e.version,
            json!({"op":"cancel_checklist","ref":"CL-001","reason":""}),
        );
        assert!(nothing.is_err());
    }

    #[test]
    fn payload_decoding_is_closed_and_refuses_null_patches() {
        let ok = decode(
            json!({"project":"p","version":"v","op":"edit_decision","ref":"D-001","title":"T"}),
        );
        assert!(ok.is_ok());
        for bad in [
            json!({"project":"p","version":"v","op":"drop_everything"}),
            json!({"project":"p","version":"v","op":"edit_decision","ref":"D-001","surprise":1}),
            json!({"project":"p","version":"v","op":"edit_decision","ref":"D-001","title":null}),
            json!({"project":"p","version":"v","op":"create_decision","title":"T"}),
        ] {
            assert!(decode(bad.clone()).is_err(), "{bad}");
        }
        let schema = super::super::input::mutation_schema::<KnowledgeOp>(false);
        assert_eq!(schema["type"], "object");
        assert!(schema.to_string().contains("create_checklist"));
    }

    #[test]
    fn knowledge_writes_never_touch_the_work_allocator_or_its_token() {
        let fx = fixture();
        let state = fs::read(fx.store.path(".agent-tasks/state.yaml").unwrap()).unwrap();
        let work_token = fx.store.allocation_version().unwrap();
        create(&fx, decision("Pick"));
        create(&fx, runbook());
        assert_eq!(
            state,
            fs::read(fx.store.path(".agent-tasks/state.yaml").unwrap()).unwrap()
        );
        assert_eq!(
            work_token,
            fx.store.allocation_version().unwrap(),
            "work allocation is unaffected"
        );
        assert!(
            fx.store.inventory().unwrap().complete,
            "knowledge homes add no work inventory warnings"
        );
    }
}
