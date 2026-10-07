//! Typed knowledge records: closed Decision, Runbook, Research and procedural Checklist schemas,
//! their lifecycle, the ONE knowledge allocator with its single six-home allocation observation,
//! the typed identifier grammar and guarded record storage.
//!
//! The domain never imports `tools`, never locks, never runs Git and never touches the work
//! allocator. Callers hold the root write lock and pass its guard to mutating functions.
#![allow(
    dead_code,
    reason = "Public domain surface consumed by the registry module once it registers the producer"
)]
use crate::model::{self, CheckStatus};
use crate::store::{self, Error, LockGuard, Result, Snapshot, Store};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;

/// Owned YAML schema revision of every knowledge record and of the allocator file.
pub const SCHEMA: u32 = 1;
/// Records enumerated per home; the same bound as the work inventory.
pub const HOME_CAP: usize = store::MODULE_CAP;
/// Retained prior revisions per Decision, Runbook or Research record.
pub const HISTORY_CAP: usize = 16;
/// Retained runbook use records.
pub const USE_CAP: usize = 64;
/// Retained checklist events.
pub const EVENT_CAP: usize = 128;
/// Evicted-entry locators one record may keep.
pub const LOCATOR_CAP: usize = 64;
/// Items a checklist may hold.
pub const ITEM_CAP: usize = 32;
/// Steps a runbook may hold.
pub const STEP_CAP: usize = 32;
/// Depth of the successor walk performed before a supersession is written.
pub const CHAIN_CAP: usize = 32;
/// Relative path of the tracked knowledge allocator file.
pub const ALLOCATOR_PATH: &str = ".agent-tasks/knowledge.yaml";

/// Refuse caller-supplied input with the stable `invalid_arguments` code.
pub fn arguments(message: impl Into<String>) -> Error {
    Error::new("invalid_arguments", message)
}

/// Prefix a field name to a validation message so refusals name the offending field.
fn named<T>(name: &str, checked: std::result::Result<T, String>) -> std::result::Result<T, String> {
    checked.map_err(|rule| format!("{name}: {rule}"))
}

/// Check a list length against inclusive bounds.
fn count(name: &str, len: usize, min: usize, max: usize) -> std::result::Result<(), String> {
    if len < min || len > max {
        return Err(format!("{name}: expected between {min} and {max} entries."));
    }
    Ok(())
}

/// Validate a bounded list of prose strings with at most `max` entries of at most `cap` bytes.
fn prose_list(
    name: &str,
    values: &[String],
    max: usize,
    cap: usize,
) -> std::result::Result<(), String> {
    count(name, values.len(), 0, max)?;
    for value in values {
        named(name, model::text(value, cap))?;
    }
    Ok(())
}

/// Validate a generated UTC timestamp field without constraining its exact rendering.
fn stamp(name: &str, value: &str) -> std::result::Result<(), String> {
    named(name, model::text(value, 40))
}

/// The four typed record kinds this domain stores and edits.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    /// A decision with question, adopted statement and rationale.
    Decision,
    /// An ordered procedure with expected outcomes and recorded use.
    Runbook,
    /// Reusable findings with basis, sources and limits.
    Research,
    /// A procedural checklist with per-item facts or reasons.
    Checklist,
}

impl Kind {
    /// Allocator prefix owning this kind's identifiers and home directory.
    pub fn prefix(self) -> Prefix {
        match self {
            Kind::Decision => Prefix::Decision,
            Kind::Runbook => Prefix::Runbook,
            Kind::Research => Prefix::Research,
            Kind::Checklist => Prefix::Checklist,
        }
    }

    /// Resolve the typed kind of a canonical record identifier; item suffixes and the document or
    /// compaction prefixes are refused because they are not typed records of this domain.
    pub fn of(id: &str) -> std::result::Result<Kind, String> {
        let parsed = parse_id(id)?;
        if parsed.item.is_some() {
            return Err("Expected a record identifier, not an item identifier.".into());
        }
        match parsed.prefix {
            Prefix::Decision => Ok(Kind::Decision),
            Prefix::Runbook => Ok(Kind::Runbook),
            Prefix::Research => Ok(Kind::Research),
            Prefix::Checklist => Ok(Kind::Checklist),
            _ => Err("Expected a D-, RB-, RS- or CL- identifier.".into()),
        }
    }

    /// Relative record path for a canonical identifier of this kind, for example `decisions/D-001.yaml`.
    pub fn path(self, id: &str) -> std::result::Result<String, String> {
        if Kind::of(id)? != self {
            return Err("Identifier does not belong to this kind.".into());
        }
        Ok(format!("{}/{id}.yaml", self.prefix().directory()))
    }
}

/// Identifier prefixes governed by the single knowledge allocator, in fixed observation order.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum Prefix {
    /// `D-` Decisions.
    Decision,
    /// `RB-` Runbooks.
    Runbook,
    /// `RS-` Research.
    Research,
    /// `CL-` Checklists.
    Checklist,
    /// `DOC-` document metadata records owned by the document module.
    Document,
    /// `CP-` compaction records owned by the compaction module.
    Compaction,
}

impl Prefix {
    /// Every prefix in the fixed order used by the allocation observation.
    pub const ALL: [Prefix; 6] = [
        Prefix::Decision,
        Prefix::Runbook,
        Prefix::Research,
        Prefix::Checklist,
        Prefix::Document,
        Prefix::Compaction,
    ];

    /// Textual prefix including the dash, for example `RB-`.
    pub fn as_str(self) -> &'static str {
        match self {
            Prefix::Decision => "D-",
            Prefix::Runbook => "RB-",
            Prefix::Research => "RS-",
            Prefix::Checklist => "CL-",
            Prefix::Document => "DOC-",
            Prefix::Compaction => "CP-",
        }
    }

    /// Home directory of this prefix's record files, relative to the documentation root.
    pub fn directory(self) -> &'static str {
        match self {
            Prefix::Decision => "decisions",
            Prefix::Runbook => "runbooks",
            Prefix::Research => "research",
            Prefix::Checklist => "checklists",
            Prefix::Document => "documents",
            Prefix::Compaction => "compactions",
        }
    }
}

/// A parsed canonical knowledge identifier.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct KnowledgeId {
    /// Allocator prefix.
    pub prefix: Prefix,
    /// Positive record number.
    pub number: u64,
    /// Checklist item number for `CL-001/I-002` forms; always `None` for other prefixes.
    pub item: Option<u64>,
}

impl KnowledgeId {
    /// Canonical text of the identifier, the exact inverse of [`parse_id`].
    pub fn canonical(&self) -> String {
        let head = format!("{}{:03}", self.prefix.as_str(), self.number);
        match self.item {
            Some(item) => format!("{head}/I-{item:03}"),
            None => head,
        }
    }
}

/// The one typed identifier grammar: pure, no I/O, no allocator or inventory access.
///
/// Accepts `D-`, `RB-`, `RS-`, `CL-`, `DOC-` and `CP-` followed by the canonical decimal digits, and
/// for checklists an optional `/I-nnn` item suffix. Leading-zero variants, other prefixes, other
/// suffixes and path components are refused.
pub fn parse_id(raw: &str) -> std::result::Result<KnowledgeId, String> {
    let (head, tail) = match raw.split_once('/') {
        Some((head, tail)) => (head, Some(tail)),
        None => (raw, None),
    };
    let refuse =
        || "Expected a canonical knowledge identifier such as D-001 or CL-001/I-001.".to_owned();
    let (prefix, number) = Prefix::ALL
        .iter()
        .find_map(|p| model::number(head, p.as_str()).ok().map(|n| (*p, n)))
        .ok_or_else(refuse)?;
    let item = match tail {
        None => None,
        Some(tail) if prefix == Prefix::Checklist => {
            Some(model::number(tail, "I-").map_err(|_| refuse())?)
        }
        Some(_) => return Err(refuse()),
    };
    Ok(KnowledgeId {
        prefix,
        number,
        item,
    })
}

/// Explicit currentness of a Decision, Runbook or Research record.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Currentness {
    /// The record has not been replaced by a successor.
    Current,
    /// A named same-kind successor replaced it; the content stays readable.
    Superseded,
}

/// Generated identity and attribution shared by every record kind.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Meta {
    /// Owned schema revision.
    pub schema_version: u32,
    /// Canonical identifier equal to the file name.
    pub id: String,
    /// Generated UTC creation time.
    pub created_at: String,
    /// Declared actor at creation; `None` stays unknown.
    pub created_by: Option<String>,
    /// Generated UTC time of the last managed change of any kind.
    pub updated_at: String,
    /// Declared actor of the last managed change.
    pub updated_by: Option<String>,
}

/// Supersession state of a revisioned record; one pointer on the predecessor.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Lifecycle {
    /// Explicit current or superseded state.
    pub state: Currentness,
    /// Identifier of the same-kind successor when superseded.
    pub superseded_by: Option<String>,
    /// Generated UTC time the supersession was recorded.
    pub superseded_at: Option<String>,
}

impl Lifecycle {
    /// The initial current state with no successor.
    fn current() -> Self {
        Lifecycle {
            state: Currentness::Current,
            superseded_by: None,
            superseded_at: None,
        }
    }
}

/// A retained prior definition of a revisioned record.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Revision<B> {
    /// Definition revision this content held.
    pub revision: u64,
    /// Generated UTC time that content was last written.
    pub at: String,
    /// Declared actor who last wrote that content.
    pub by: Option<String>,
    /// The exact prior content.
    pub content: B,
}

/// Locator proving evicted entries exist in verified committed objects.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Evicted {
    /// Revision number for definition history, or the use or event ordinal for ledgers.
    pub revision: u64,
    /// Opaque committed-object locator of at most 256 bytes supplied by the commit proof.
    pub locator: String,
}

/// One rejected alternative of a decision.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Alternative {
    /// The option considered, at most 256 bytes.
    pub option: String,
    /// Why it was not adopted, at most 1024 bytes.
    pub rejected_because: String,
}

/// A question the decision leaves open.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct OpenQuestion {
    /// The open question, at most 512 bytes.
    pub text: String,
    /// Whether the project owner must answer it.
    pub needs_owner: bool,
}

/// Content of a Decision.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DecisionBody {
    /// Short title, at most 256 bytes.
    pub title: String,
    /// The question decided, at most 1024 bytes.
    pub question: String,
    /// The adopted statement, at most 2048 bytes.
    pub decision: String,
    /// Rationale, at most 4096 bytes.
    pub rationale: String,
    /// Up to eight rejected alternatives.
    pub alternatives: Vec<Alternative>,
    /// Up to eight open questions.
    pub open_questions: Vec<OpenQuestion>,
    /// Optional managed-document reference validated by the document module.
    pub detail: Option<String>,
}

impl DecisionBody {
    /// Validate every field bound; performs no I/O.
    pub fn validate(&self) -> std::result::Result<(), String> {
        named("title", model::text(&self.title, 256))?;
        named("question", model::text(&self.question, 1024))?;
        named("decision", model::text(&self.decision, 2048))?;
        named("rationale", model::text(&self.rationale, 4096))?;
        count("alternatives", self.alternatives.len(), 0, 8)?;
        for a in &self.alternatives {
            named("alternatives.option", model::text(&a.option, 256))?;
            named(
                "alternatives.rejected_because",
                model::text(&a.rejected_because, 1024),
            )?;
        }
        count("open_questions", self.open_questions.len(), 0, 8)?;
        for q in &self.open_questions {
            named("open_questions.text", model::text(&q.text, 512))?;
        }
        named("detail", model::optional(&self.detail, 256))
    }
}

/// Basis of a research conclusion or evidence entry.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Basis {
    /// Directly measured by the author.
    Measured,
    /// Reported by a cited source.
    Cited,
    /// Inferred without direct measurement or citation.
    Inferred,
}

/// One stated conclusion.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Conclusion {
    /// The conclusion, at most 1024 bytes.
    pub statement: String,
    /// How it is known.
    pub basis: Basis,
}

/// One supporting claim with its source.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Evidence {
    /// The claim, at most 512 bytes.
    pub claim: String,
    /// Where it comes from, at most 256 bytes.
    pub source: String,
    /// How it is known.
    pub basis: Basis,
}

/// Content of a Research record.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ResearchBody {
    /// Short title, at most 256 bytes.
    pub title: String,
    /// The researched question, at most 1024 bytes.
    pub question: String,
    /// One to eight conclusions.
    pub conclusions: Vec<Conclusion>,
    /// Up to eight evidence entries.
    pub evidence: Vec<Evidence>,
    /// Up to eight limitations, at most 512 bytes each.
    pub limitations: Vec<String>,
    /// Where the findings apply, at most 512 bytes.
    pub applicability: String,
    /// Optional managed-document reference validated by the document module.
    pub detail: Option<String>,
}

impl ResearchBody {
    /// Validate every field bound; a measured or cited conclusion needs evidence of that basis.
    pub fn validate(&self) -> std::result::Result<(), String> {
        named("title", model::text(&self.title, 256))?;
        named("question", model::text(&self.question, 1024))?;
        count("conclusions", self.conclusions.len(), 1, 8)?;
        for c in &self.conclusions {
            named("conclusions.statement", model::text(&c.statement, 1024))?;
            if c.basis != Basis::Inferred && !self.evidence.iter().any(|e| e.basis == c.basis) {
                return Err(
                    "conclusions: a measured or cited conclusion needs evidence with the same basis."
                        .into(),
                );
            }
        }
        count("evidence", self.evidence.len(), 0, 8)?;
        for e in &self.evidence {
            named("evidence.claim", model::text(&e.claim, 512))?;
            named("evidence.source", model::text(&e.source, 256))?;
        }
        prose_list("limitations", &self.limitations, 8, 512)?;
        named("applicability", model::text(&self.applicability, 512))?;
        named("detail", model::optional(&self.detail, 256))
    }
}

/// A named runbook input.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RunbookInput {
    /// Lowercase `[a-z0-9_]` name of at most 64 bytes, unique within the runbook.
    pub name: String,
    /// What the input means, at most 512 bytes.
    pub description: String,
    /// Whether the runbook cannot run without it.
    pub required: bool,
}

/// One ordered runbook step; reading it never executes anything.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Step {
    /// Step title, at most 128 bytes.
    pub title: String,
    /// Optional command text, at most 1024 bytes.
    pub command: Option<String>,
    /// What the step does, at most 1024 bytes.
    pub description: String,
    /// The expected outcome, at most 512 bytes.
    pub expected: String,
    /// Optional recovery guidance, at most 512 bytes.
    pub recovery: Option<String>,
}

/// Content of a Runbook.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RunbookBody {
    /// Short title, at most 256 bytes.
    pub title: String,
    /// Purpose, at most 1024 bytes.
    pub purpose: String,
    /// Up to eight prerequisites, at most 512 bytes each.
    pub prerequisites: Vec<String>,
    /// Up to eight named inputs.
    pub inputs: Vec<RunbookInput>,
    /// One to thirty-two ordered steps.
    pub steps: Vec<Step>,
    /// Up to eight pitfalls, at most 512 bytes each.
    pub pitfalls: Vec<String>,
    /// Optional managed-document reference validated by the document module.
    pub detail: Option<String>,
}

impl RunbookBody {
    /// Validate every field bound and input-name uniqueness; performs no I/O.
    pub fn validate(&self) -> std::result::Result<(), String> {
        named("title", model::text(&self.title, 256))?;
        named("purpose", model::text(&self.purpose, 1024))?;
        prose_list("prerequisites", &self.prerequisites, 8, 512)?;
        count("inputs", self.inputs.len(), 0, 8)?;
        let mut names = BTreeSet::new();
        for input in &self.inputs {
            let ok = !input.name.is_empty()
                && input.name.len() <= 64
                && input
                    .name
                    .bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_');
            if !ok {
                return Err("inputs.name: use 1 to 64 characters of [a-z0-9_].".into());
            }
            if !names.insert(input.name.as_str()) {
                return Err("inputs.name: names must be unique.".into());
            }
            named("inputs.description", model::text(&input.description, 512))?;
        }
        count("steps", self.steps.len(), 1, STEP_CAP)?;
        for step in &self.steps {
            named("steps.title", model::text(&step.title, 128))?;
            named("steps.command", model::optional(&step.command, 1024))?;
            named("steps.description", model::text(&step.description, 1024))?;
            named("steps.expected", model::text(&step.expected, 512))?;
            named("steps.recovery", model::optional(&step.recovery, 512))?;
        }
        prose_list("pitfalls", &self.pitfalls, 8, 512)?;
        named("detail", model::optional(&self.detail, 256))
    }
}

/// Reported outcome of one runbook use.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum UseOutcome {
    /// The runbook achieved its purpose.
    Succeeded,
    /// The runbook did not achieve its purpose.
    Failed,
    /// Some steps worked and the purpose was only partly achieved.
    Partial,
}

/// One reported check of a runbook use.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct UseCheck {
    /// Check label, at most 64 bytes.
    pub label: String,
    /// Explicit reported status.
    pub status: CheckStatus,
    /// Optional detail, at most 256 bytes.
    pub detail: Option<String>,
}

/// An actual reported use of one runbook revision; an observation, never a verification of others.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RunbookUse {
    /// Generated use identifier such as `U-001`.
    pub id: String,
    /// Generated UTC time of the report.
    pub at: String,
    /// Declared actor; `None` stays unknown.
    pub by: Option<String>,
    /// The exact definition revision that was used.
    pub revision: u64,
    /// Real environment of the use, at most 256 bytes.
    pub environment: String,
    /// Reported outcome.
    pub outcome: UseOutcome,
    /// Up to eight reported checks.
    pub checks: Vec<UseCheck>,
    /// Up to eight primary artifact references, at most 256 bytes each.
    pub artifacts: Vec<String>,
    /// Optional manual observation, at most 1024 bytes.
    pub observation: Option<String>,
    /// Optional canonical work reference the use belongs to.
    pub work: Option<String>,
    /// Generated: the used revision was not the current one.
    pub stale_revision: bool,
    /// Generated: the record was already superseded at the time of use.
    pub superseded_record: bool,
}

/// State of one checklist item.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ItemState {
    /// Not yet resolved.
    Open,
    /// Completed with a recorded fact.
    Done,
    /// Skipped or abandoned with a recorded reason; never counts as done.
    Canceled,
}

/// Fact or reason recorded when an item was resolved.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Resolution {
    /// Generated UTC time.
    pub at: String,
    /// Declared actor; `None` stays unknown.
    pub by: Option<String>,
    /// The completion fact or cancellation reason, at most 512 bytes.
    pub text: String,
}

/// One checklist item.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Item {
    /// Generated item identifier such as `I-001`.
    pub id: String,
    /// What must be done, at most 512 bytes.
    pub text: String,
    /// Current state.
    pub state: ItemState,
    /// Present exactly when the item is done or canceled.
    pub resolution: Option<Resolution>,
}

/// Content of a procedural Checklist.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ChecklistBody {
    /// Short title, at most 256 bytes.
    pub title: String,
    /// Purpose, at most 1024 bytes.
    pub purpose: String,
    /// One to thirty-two items.
    pub items: Vec<Item>,
}

/// Lifecycle state of a checklist.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChecklistState {
    /// Accepting item changes.
    Open,
    /// Every item terminal with at least one done.
    Completed,
    /// Closed with a recorded reason.
    Canceled,
}

/// One retained checklist event.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Event {
    /// Generated UTC time.
    pub at: String,
    /// Declared actor; `None` stays unknown.
    pub by: Option<String>,
    /// What happened: created, add_items, resolve_item, complete, cancel or reopen.
    pub action: String,
    /// Optional note or reason, at most 512 bytes.
    pub note: Option<String>,
}

/// Reject a bare canonical work reference used as a checklist item, which would duplicate a Task.
fn work_reference_text(text: &str) -> bool {
    let (head, tail) = match text.split_once('/') {
        Some((h, t)) => (h, Some(t)),
        None => (text, None),
    };
    let head_ok = ["E-", "M-", "A-"]
        .iter()
        .any(|p| model::number(head, p).is_ok());
    let tail_ok =
        tail.is_none_or(|t| model::number(t, "T-").is_ok() || model::number(t, "A-").is_ok());
    head_ok && tail_ok
}

impl ChecklistBody {
    /// Validate bounds, identifiers and the open-versus-resolved item invariant; performs no I/O.
    pub fn validate(&self) -> std::result::Result<(), String> {
        named("title", model::text(&self.title, 256))?;
        named("purpose", model::text(&self.purpose, 1024))?;
        count("items", self.items.len(), 1, ITEM_CAP)?;
        let mut seen = BTreeSet::new();
        for item in &self.items {
            named("items.id", model::number(&item.id, "I-").map(|_| ()))?;
            if !seen.insert(item.id.as_str()) {
                return Err("items.id: identifiers must be unique.".into());
            }
            named("items.text", model::text(&item.text, 512))?;
            if work_reference_text(item.text.trim()) {
                return Err(
                    "items.text: a bare work reference duplicates a Task; use the canonical Task instead."
                        .into(),
                );
            }
            match (item.state, &item.resolution) {
                (ItemState::Open, None) => (),
                (ItemState::Done | ItemState::Canceled, Some(r)) => {
                    named("items.resolution", model::text(&r.text, 512))?;
                    stamp("items.resolution.at", &r.at)?;
                }
                _ => {
                    return Err(
                        "items: resolved items need a fact or reason and open items none.".into(),
                    );
                }
            }
        }
        Ok(())
    }
}

/// Validate the generated identity block against the expected canonical identifier.
fn validate_meta(meta: &Meta, id_kind: Kind) -> std::result::Result<(), String> {
    if meta.schema_version != SCHEMA {
        return Err("schema_version: unsupported knowledge schema.".into());
    }
    if Kind::of(&meta.id)? != id_kind {
        return Err("id: identifier does not match the record kind.".into());
    }
    stamp("created_at", &meta.created_at)?;
    stamp("updated_at", &meta.updated_at)?;
    model::optional(&meta.created_by, 256)?;
    model::optional(&meta.updated_by, 256)
}

/// Validate supersession consistency for one record.
fn validate_lifecycle(l: &Lifecycle, own_id: &str, kind: Kind) -> std::result::Result<(), String> {
    match (l.state, &l.superseded_by, &l.superseded_at) {
        (Currentness::Current, None, None) => Ok(()),
        (Currentness::Superseded, Some(by), Some(at)) => {
            if by == own_id || Kind::of(by)? != kind {
                return Err("superseded_by: must name a different record of the same kind.".into());
            }
            stamp("superseded_at", at)
        }
        _ => Err("lifecycle: superseded state and successor must be set together.".into()),
    }
}

/// Validate evicted locators.
fn validate_evicted(name: &str, list: &[Evicted]) -> std::result::Result<(), String> {
    count(name, list.len(), 0, LOCATOR_CAP)?;
    for e in list {
        named(name, model::text(&e.locator, 256))?;
    }
    Ok(())
}

/// Read access shared by the three revisioned record kinds, implemented by `revisioned!` types.
pub trait Revisioned: Serialize + DeserializeOwned + Clone {
    /// The content type whose retained definitions form the history.
    type Body: Clone + PartialEq;
    /// The record's generated identity block.
    fn meta_mut(&mut self) -> &mut Meta;
    /// Current definition revision.
    fn revision(&self) -> u64;
    /// Supersession state.
    fn lifecycle(&self) -> &Lifecycle;
    /// Current content.
    fn content(&self) -> &Self::Body;
    /// Apply a definition edit: archive the current content, bump the revision and install `body`.
    /// `evict` supplies a committed-object locator and is called only when the history is full.
    fn install(
        &mut self,
        body: Self::Body,
        at: &str,
        by: &Option<String>,
        evict: &mut dyn FnMut() -> Result<String>,
    ) -> Result<()>;
    /// Whether a definition of this revision is retained (current or in history).
    fn has_revision(&self, revision: u64) -> bool;
    /// Write the supersession pointer fields; callers validate the successor first.
    fn set_superseded(&mut self, successor: &str, at: &str);

    /// Install an edited definition, or report that it equals the current content.
    ///
    /// Superseded records refuse edits (`conflict`): correct them through a successor. Returns
    /// `Ok(false)` without any change when `body` equals the current content. `body` must already be
    /// validated by its kind. A full history evicts its oldest entry only after `evict` returns a
    /// committed-object locator; its failure is returned unchanged and nothing is modified.
    fn edit(
        &mut self,
        body: Self::Body,
        at: &str,
        by: &Option<String>,
        evict: &mut dyn FnMut() -> Result<String>,
    ) -> Result<bool> {
        if self.lifecycle().state == Currentness::Superseded {
            return Err(Error::new(
                "conflict",
                "A superseded record is not edited; create a successor and supersede again if needed.",
            ));
        }
        if *self.content() == body {
            return Ok(false);
        }
        self.install(body, at, by, evict)?;
        Ok(true)
    }

    /// Record the single supersession pointer to an already validated same-kind current successor.
    ///
    /// Returns `Ok(false)` when this successor is already recorded and `conflict` when a different
    /// one is. The definition revision is not bumped.
    fn supersede(&mut self, successor: &str, at: &str, by: &Option<String>) -> Result<bool> {
        let lifecycle = self.lifecycle();
        if lifecycle.state == Currentness::Superseded {
            if lifecycle.superseded_by.as_deref() == Some(successor) {
                return Ok(false);
            }
            return Err(Error::new(
                "conflict",
                "The record is already superseded by a different successor.",
            ));
        }
        self.set_superseded(successor, at);
        let meta = self.meta_mut();
        meta.updated_at = at.to_owned();
        meta.updated_by = by.clone();
        Ok(true)
    }
}

/// Define one revisioned record struct (Decision, Research or Runbook) with its shared lifecycle
/// behavior; `$extra` adds kind-specific fields such as the runbook use ledger.
macro_rules! revisioned {
    ($(#[$doc:meta])* $name:ident, $body:ty, $kind:expr, { $($(#[$fdoc:meta])* pub $field:ident : $fty:ty),* $(,)? }) => {
        $(#[$doc])*
        #[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
        #[serde(deny_unknown_fields)]
        pub struct $name {
            /// Generated identity and attribution.
            pub meta: Meta,
            /// Current definition revision, starting at 1.
            pub revision: u64,
            /// Generated UTC time the current content was written.
            pub revised_at: String,
            /// Declared actor who wrote the current content.
            pub revised_by: Option<String>,
            /// Explicit currentness and successor pointer.
            pub lifecycle: Lifecycle,
            /// Current content.
            pub content: $body,
            /// Retained prior definitions, oldest first.
            pub history: Vec<Revision<$body>>,
            /// Locators for history entries evicted with committed proof.
            pub evicted: Vec<Evicted>,
            $($(#[$fdoc])* pub $field: $fty,)*
        }

        impl Revisioned for $name {
            /// The kind-specific content type of this record.
            type Body = $body;
            /// Mutable access to the generated identity block.
            fn meta_mut(&mut self) -> &mut Meta { &mut self.meta }
            /// Current definition revision of this record.
            fn revision(&self) -> u64 { self.revision }
            /// Supersession state of this record.
            fn lifecycle(&self) -> &Lifecycle { &self.lifecycle }
            /// Current content of this record.
            fn content(&self) -> &$body { &self.content }
            /// Archive the current content into the history and install the edited one.
            fn install(
                &mut self,
                body: $body,
                at: &str,
                by: &Option<String>,
                evict: &mut dyn FnMut() -> Result<String>,
            ) -> Result<()> {
                if self.history.len() >= HISTORY_CAP {
                    let locator = evict()?;
                    if self.evicted.len() >= LOCATOR_CAP {
                        return Err(Error::new("capacity", "Evicted locator limit reached."));
                    }
                    let oldest = self.history.remove(0);
                    self.evicted.push(Evicted { revision: oldest.revision, locator });
                }
                let prior = std::mem::replace(&mut self.content, body);
                self.history.push(Revision {
                    revision: self.revision,
                    at: std::mem::replace(&mut self.revised_at, at.to_owned()),
                    by: std::mem::replace(&mut self.revised_by, by.clone()),
                    content: prior,
                });
                self.revision += 1;
                self.meta.updated_at = at.to_owned();
                self.meta.updated_by = by.clone();
                Ok(())
            }
            /// Whether the given definition revision is current or retained in the history.
            fn has_revision(&self, revision: u64) -> bool {
                revision == self.revision || self.history.iter().any(|h| h.revision == revision)
            }
            /// Write the supersession pointer fields of this record.
            fn set_superseded(&mut self, successor: &str, at: &str) {
                self.lifecycle = Lifecycle {
                    state: Currentness::Superseded,
                    superseded_by: Some(successor.to_owned()),
                    superseded_at: Some(at.to_owned()),
                };
            }
        }

        impl $name {
            /// Validate the whole record: identity, lifecycle, content, retained history and ledgers.
            pub fn validate(&self) -> std::result::Result<(), String> {
                validate_meta(&self.meta, $kind)?;
                if self.revision == 0 {
                    return Err("revision: must start at 1.".into());
                }
                stamp("revised_at", &self.revised_at)?;
                model::optional(&self.revised_by, 256)?;
                validate_lifecycle(&self.lifecycle, &self.meta.id, $kind)?;
                self.content.validate()?;
                count("history", self.history.len(), 0, HISTORY_CAP)?;
                let mut previous = 0;
                for h in &self.history {
                    if h.revision <= previous || h.revision >= self.revision {
                        return Err("history: revisions must increase and stay below the current one.".into());
                    }
                    previous = h.revision;
                    stamp("history.at", &h.at)?;
                    h.content.validate()?;
                }
                validate_evicted("evicted", &self.evicted)?;
                self.validate_extra()
            }
        }
    };
}

revisioned!(
    /// A decision record with explicit currentness, retained revisions and supersession.
    Decision, DecisionBody, Kind::Decision, {}
);
revisioned!(
    /// A research record with explicit currentness, retained revisions and supersession.
    Research, ResearchBody, Kind::Research, {}
);
revisioned!(
    /// A runbook with retained revisions and an append-only ledger of actual uses.
    Runbook, RunbookBody, Kind::Runbook, {
        /// Reported uses, oldest first, bounded by [`USE_CAP`].
        pub uses: Vec<RunbookUse>,
        /// Next generated use number.
        pub next_use: u64,
        /// Locators for uses evicted with committed proof.
        pub evicted_uses: Vec<Evicted>,
    }
);

impl Decision {
    /// Decisions carry no kind-specific ledger to validate.
    fn validate_extra(&self) -> std::result::Result<(), String> {
        Ok(())
    }
}

impl Research {
    /// Research carries no kind-specific ledger to validate.
    fn validate_extra(&self) -> std::result::Result<(), String> {
        Ok(())
    }
}

impl Runbook {
    /// Validate the use ledger: bounds, identifiers, referenced revisions and evidence.
    fn validate_extra(&self) -> std::result::Result<(), String> {
        count("uses", self.uses.len(), 0, USE_CAP)?;
        let mut highest = 0;
        for u in &self.uses {
            let n = named("uses.id", model::number(&u.id, "U-"))?;
            if n <= highest || n >= self.next_use {
                return Err("uses.id: identifiers must increase and stay below next_use.".into());
            }
            highest = n;
            stamp("uses.at", &u.at)?;
            if u.revision == 0 || u.revision > self.revision {
                return Err("uses.revision: must name an existing definition revision.".into());
            }
            named("uses.environment", model::text(&u.environment, 256))?;
            count("uses.checks", u.checks.len(), 0, 8)?;
            for c in &u.checks {
                named("uses.checks.label", model::text(&c.label, 64))?;
                named("uses.checks.detail", model::optional(&c.detail, 256))?;
            }
            prose_list("uses.artifacts", &u.artifacts, 8, 256)?;
            named("uses.observation", model::optional(&u.observation, 1024))?;
            named("uses.work", model::optional(&u.work, 256))?;
        }
        validate_evicted("evicted_uses", &self.evicted_uses)
    }
}

/// A procedural checklist with per-item facts or reasons and an event ledger.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Checklist {
    /// Generated identity and attribution.
    pub meta: Meta,
    /// Lifecycle state.
    pub state: ChecklistState,
    /// Current content.
    pub content: ChecklistBody,
    /// Retained events, oldest first, bounded by [`EVENT_CAP`].
    pub events: Vec<Event>,
    /// Next generated item number.
    pub next_item: u64,
    /// Locators for events evicted with committed proof.
    pub evicted_events: Vec<Evicted>,
}

impl Checklist {
    /// Validate the whole record: identity, content, counters and event ledger.
    pub fn validate(&self) -> std::result::Result<(), String> {
        validate_meta(&self.meta, Kind::Checklist)?;
        self.content.validate()?;
        for item in &self.content.items {
            let n = model::number(&item.id, "I-")?;
            if n >= self.next_item {
                return Err("next_item: must exceed every item number.".into());
            }
        }
        count("events", self.events.len(), 0, EVENT_CAP)?;
        for e in &self.events {
            stamp("events.at", &e.at)?;
            named("events.action", model::text(&e.action, 32))?;
            named("events.note", model::optional(&e.note, 512))?;
        }
        validate_evicted("evicted_events", &self.evicted_events)
    }

    /// Whether every item is terminal.
    fn all_terminal(&self) -> bool {
        self.content
            .items
            .iter()
            .all(|i| i.state != ItemState::Open)
    }

    /// Append an event, evicting the oldest only with a committed locator.
    fn log(
        &mut self,
        action: &str,
        note: Option<String>,
        at: &str,
        by: &Option<String>,
        evict: &mut dyn FnMut() -> Result<String>,
    ) -> Result<()> {
        if self.events.len() >= EVENT_CAP {
            let locator = evict()?;
            if self.evicted_events.len() >= LOCATOR_CAP {
                return Err(Error::new("capacity", "Evicted locator limit reached."));
            }
            self.events.remove(0);
            let ordinal = self.evicted_events.len() as u64 + 1;
            self.evicted_events.push(Evicted {
                revision: ordinal,
                locator,
            });
        }
        self.events.push(Event {
            at: at.to_owned(),
            by: by.clone(),
            action: action.to_owned(),
            note,
        });
        self.meta.updated_at = at.to_owned();
        self.meta.updated_by = by.clone();
        Ok(())
    }
}

/// Validate a free-form reason or note shared by checklist cancellation, reopening and resolution.
fn reason_text(name: &str, value: &str) -> std::result::Result<(), String> {
    named(name, model::text(value, 512))
}

/// Draft of one runbook use as supplied by a caller.
#[derive(Clone, Debug)]
pub struct UseInput {
    /// Definition revision that was used.
    pub revision: u64,
    /// Real environment, at most 256 bytes.
    pub environment: String,
    /// Reported outcome.
    pub outcome: UseOutcome,
    /// Up to eight reported checks.
    pub checks: Vec<UseCheck>,
    /// Up to eight artifact references.
    pub artifacts: Vec<String>,
    /// Optional manual observation.
    pub observation: Option<String>,
    /// Optional canonical work reference, already verified by the caller.
    pub work: Option<String>,
}

impl UseInput {
    /// Check the evidence rules without touching a record.
    ///
    /// An environment and at least one piece of nonempty evidence (a check, an artifact or an
    /// observation) are required. `succeeded` forbids any failed check and needs a passed check, an
    /// artifact or an observation; `failed` needs a failed check or an observation.
    pub fn validate(&self) -> std::result::Result<(), String> {
        named("environment", model::text(&self.environment, 256))?;
        count("checks", self.checks.len(), 0, 8)?;
        for c in &self.checks {
            named("checks.label", model::text(&c.label, 64))?;
            named("checks.detail", model::optional(&c.detail, 256))?;
        }
        prose_list("artifacts", &self.artifacts, 8, 256)?;
        named("observation", model::optional(&self.observation, 1024))?;
        if self.checks.is_empty() && self.artifacts.is_empty() && self.observation.is_none() {
            return Err(
                "evidence: give at least one check, artifact or manual observation.".into(),
            );
        }
        let failed_check = self.checks.iter().any(|c| c.status == CheckStatus::Failed);
        let passed_check = self.checks.iter().any(|c| c.status == CheckStatus::Passed);
        match self.outcome {
            UseOutcome::Succeeded => {
                if failed_check {
                    return Err("outcome: a succeeded use cannot carry a failed check.".into());
                }
                if !(passed_check || !self.artifacts.is_empty() || self.observation.is_some()) {
                    return Err(
                        "outcome: a succeeded use needs a passed check, an artifact or an observation."
                            .into(),
                    );
                }
            }
            UseOutcome::Failed => {
                if !(failed_check || self.observation.is_some()) {
                    return Err(
                        "outcome: a failed use needs a failed check or an observation.".into(),
                    );
                }
            }
            UseOutcome::Partial => (),
        }
        Ok(())
    }
}

impl Runbook {
    /// Append an actual use of an existing definition revision; never changes the definition.
    ///
    /// The revision must be current or retained (`not_found` otherwise, including evicted ones).
    /// A use of a non-current revision is flagged `stale_revision` and a use of a superseded record
    /// `superseded_record`; neither changes currentness. A full ledger evicts its oldest use only
    /// after `evict` returns a committed locator. Returns the generated use identifier.
    pub fn add_use(
        &mut self,
        input: UseInput,
        at: &str,
        by: &Option<String>,
        evict: &mut dyn FnMut() -> Result<String>,
    ) -> Result<String> {
        input.validate().map_err(arguments)?;
        if !self.has_revision(input.revision) {
            return Err(Error::new(
                "not_found",
                "That definition revision is not retained; read the record history.",
            ));
        }
        if self.uses.len() >= USE_CAP {
            let locator = evict()?;
            if self.evicted_uses.len() >= LOCATOR_CAP {
                return Err(Error::new("capacity", "Evicted locator limit reached."));
            }
            let oldest = self.uses.remove(0);
            let number = model::number(&oldest.id, "U-").unwrap_or(0);
            self.evicted_uses.push(Evicted {
                revision: number,
                locator,
            });
        }
        let id = format!("U-{:03}", self.next_use);
        self.next_use += 1;
        self.uses.push(RunbookUse {
            id: id.clone(),
            at: at.to_owned(),
            by: by.clone(),
            revision: input.revision,
            environment: input.environment,
            outcome: input.outcome,
            checks: input.checks,
            artifacts: input.artifacts,
            observation: input.observation,
            work: input.work,
            stale_revision: input.revision != self.revision,
            superseded_record: self.lifecycle.state == Currentness::Superseded,
        });
        self.meta.updated_at = at.to_owned();
        self.meta.updated_by = by.clone();
        Ok(id)
    }
}

/// Generated identity block of a new record.
fn new_meta(id: &str, at: &str, by: &Option<String>) -> Meta {
    Meta {
        schema_version: SCHEMA,
        id: id.to_owned(),
        created_at: at.to_owned(),
        created_by: by.clone(),
        updated_at: at.to_owned(),
        updated_by: by.clone(),
    }
}

/// Build a fresh Decision at revision 1.
pub fn new_decision(id: &str, body: DecisionBody, at: &str, by: &Option<String>) -> Decision {
    Decision {
        meta: new_meta(id, at, by),
        revision: 1,
        revised_at: at.to_owned(),
        revised_by: by.clone(),
        lifecycle: Lifecycle::current(),
        content: body,
        history: Vec::new(),
        evicted: Vec::new(),
    }
}

/// Build a fresh Research record at revision 1.
pub fn new_research(id: &str, body: ResearchBody, at: &str, by: &Option<String>) -> Research {
    Research {
        meta: new_meta(id, at, by),
        revision: 1,
        revised_at: at.to_owned(),
        revised_by: by.clone(),
        lifecycle: Lifecycle::current(),
        content: body,
        history: Vec::new(),
        evicted: Vec::new(),
    }
}

/// Build a fresh Runbook at revision 1 with an empty use ledger.
pub fn new_runbook(id: &str, body: RunbookBody, at: &str, by: &Option<String>) -> Runbook {
    Runbook {
        meta: new_meta(id, at, by),
        revision: 1,
        revised_at: at.to_owned(),
        revised_by: by.clone(),
        lifecycle: Lifecycle::current(),
        content: body,
        history: Vec::new(),
        evicted: Vec::new(),
        uses: Vec::new(),
        next_use: 1,
        evicted_uses: Vec::new(),
    }
}

/// Build a fresh open Checklist from item texts; items receive generated `I-nnn` identifiers.
pub fn new_checklist(
    id: &str,
    title: String,
    purpose: String,
    texts: Vec<String>,
    at: &str,
    by: &Option<String>,
) -> Checklist {
    let items: Vec<Item> = texts
        .into_iter()
        .enumerate()
        .map(|(n, text)| Item {
            id: format!("I-{:03}", n + 1),
            text,
            state: ItemState::Open,
            resolution: None,
        })
        .collect();
    Checklist {
        meta: new_meta(id, at, by),
        state: ChecklistState::Open,
        next_item: items.len() as u64 + 1,
        content: ChecklistBody {
            title,
            purpose,
            items,
        },
        events: vec![Event {
            at: at.to_owned(),
            by: by.clone(),
            action: "created".into(),
            note: None,
        }],
        evicted_events: Vec::new(),
    }
}

impl Checklist {
    /// Require the open state before an item change.
    fn require_open(&self) -> Result<()> {
        if self.state != ChecklistState::Open {
            return Err(Error::new(
                "conflict",
                "The checklist is closed; reopen it explicitly before changing items.",
            ));
        }
        Ok(())
    }

    /// Append open items with generated identifiers. Texts are validated with the whole body.
    pub fn add_items(
        &mut self,
        texts: Vec<String>,
        at: &str,
        by: &Option<String>,
        evict: &mut dyn FnMut() -> Result<String>,
    ) -> Result<()> {
        self.require_open()?;
        if texts.is_empty() {
            return Err(arguments("items: give at least one item."));
        }
        let mut candidate = self.content.clone();
        let mut next = self.next_item;
        for text in texts {
            candidate.items.push(Item {
                id: format!("I-{next:03}"),
                text,
                state: ItemState::Open,
                resolution: None,
            });
            next += 1;
        }
        candidate.validate().map_err(arguments)?;
        let added = next - self.next_item;
        self.log("add_items", Some(format!("{added} added")), at, by, evict)?;
        self.content = candidate;
        self.next_item = next;
        Ok(())
    }

    /// Resolve one open item as done with a completion fact or canceled with a reason.
    ///
    /// The text is required for both outcomes; a skipped item is canceled and never counts as done.
    pub fn resolve_item(
        &mut self,
        item: &str,
        state: ItemState,
        text: &str,
        at: &str,
        by: &Option<String>,
        evict: &mut dyn FnMut() -> Result<String>,
    ) -> Result<()> {
        self.require_open()?;
        if state == ItemState::Open {
            return Err(arguments("state: choose done or canceled."));
        }
        reason_text("text", text).map_err(arguments)?;
        let index = self
            .content
            .items
            .iter()
            .position(|i| i.id == item)
            .ok_or_else(|| Error::new("not_found", "That checklist item does not exist."))?;
        if self.content.items[index].state != ItemState::Open {
            return Err(Error::new("conflict", "That item is already resolved."));
        }
        self.log("resolve_item", Some(item.to_owned()), at, by, evict)?;
        let entry = &mut self.content.items[index];
        entry.state = state;
        entry.resolution = Some(Resolution {
            at: at.to_owned(),
            by: by.clone(),
            text: text.to_owned(),
        });
        Ok(())
    }

    /// Complete an open checklist: every item terminal and at least one done.
    pub fn complete(
        &mut self,
        at: &str,
        by: &Option<String>,
        evict: &mut dyn FnMut() -> Result<String>,
    ) -> Result<()> {
        self.require_open()?;
        if !self.all_terminal() {
            return Err(Error::new(
                "conflict",
                "Resolve every open item before completing.",
            ));
        }
        if !self
            .content
            .items
            .iter()
            .any(|i| i.state == ItemState::Done)
        {
            return Err(Error::new(
                "conflict",
                "At least one item must be done; cancel the checklist instead.",
            ));
        }
        self.log("complete", None, at, by, evict)?;
        self.state = ChecklistState::Completed;
        Ok(())
    }

    /// Cancel an open checklist with a reason; open items stay open and nothing cascades.
    pub fn cancel(
        &mut self,
        reason: &str,
        at: &str,
        by: &Option<String>,
        evict: &mut dyn FnMut() -> Result<String>,
    ) -> Result<()> {
        self.require_open()?;
        reason_text("reason", reason).map_err(arguments)?;
        self.log("cancel", Some(reason.to_owned()), at, by, evict)?;
        self.state = ChecklistState::Canceled;
        Ok(())
    }

    /// Reopen a completed or canceled checklist explicitly, retaining the reason as an event.
    pub fn reopen(
        &mut self,
        reason: &str,
        at: &str,
        by: &Option<String>,
        evict: &mut dyn FnMut() -> Result<String>,
    ) -> Result<()> {
        if self.state == ChecklistState::Open {
            return Err(Error::new("conflict", "The checklist is already open."));
        }
        reason_text("reason", reason).map_err(arguments)?;
        self.log("reopen", Some(reason.to_owned()), at, by, evict)?;
        self.state = ChecklistState::Open;
        Ok(())
    }
}

/// Any stored typed record, as read from its home.
#[derive(Clone, Debug, PartialEq)]
pub enum Any {
    /// A Decision.
    Decision(Decision),
    /// A Runbook.
    Runbook(Runbook),
    /// A Research record.
    Research(Research),
    /// A procedural Checklist.
    Checklist(Checklist),
}

/// Lowercase label of a currentness value.
fn currentness_label(state: Currentness) -> &'static str {
    match state {
        Currentness::Current => "current",
        Currentness::Superseded => "superseded",
    }
}

impl Any {
    /// Generated identity block.
    fn meta(&self) -> &Meta {
        match self {
            Any::Decision(r) => &r.meta,
            Any::Runbook(r) => &r.meta,
            Any::Research(r) => &r.meta,
            Any::Checklist(r) => &r.meta,
        }
    }

    /// Canonical identifier.
    pub fn id(&self) -> &str {
        &self.meta().id
    }

    /// Typed kind.
    pub fn kind(&self) -> Kind {
        match self {
            Any::Decision(_) => Kind::Decision,
            Any::Runbook(_) => Kind::Runbook,
            Any::Research(_) => Kind::Research,
            Any::Checklist(_) => Kind::Checklist,
        }
    }

    /// Current title.
    pub fn title(&self) -> &str {
        match self {
            Any::Decision(r) => &r.content.title,
            Any::Runbook(r) => &r.content.title,
            Any::Research(r) => &r.content.title,
            Any::Checklist(r) => &r.content.title,
        }
    }

    /// Actual state: `current`, `superseded`, `open`, `completed` or `canceled`.
    pub fn state_label(&self) -> &'static str {
        match self {
            Any::Decision(r) => currentness_label(r.lifecycle.state),
            Any::Runbook(r) => currentness_label(r.lifecycle.state),
            Any::Research(r) => currentness_label(r.lifecycle.state),
            Any::Checklist(r) => match r.state {
                ChecklistState::Open => "open",
                ChecklistState::Completed => "completed",
                ChecklistState::Canceled => "canceled",
            },
        }
    }

    /// Whether the record is the live one: not superseded, or an open checklist.
    pub fn current(&self) -> bool {
        matches!(self.state_label(), "current" | "open")
    }

    /// Definition revision; checklists have no revisions and report 1.
    pub fn revision(&self) -> u64 {
        match self {
            Any::Decision(r) => r.revision,
            Any::Runbook(r) => r.revision,
            Any::Research(r) => r.revision,
            Any::Checklist(_) => 1,
        }
    }

    /// Identifier of the same-kind successor, when superseded.
    pub fn superseded_by(&self) -> Option<&str> {
        match self {
            Any::Decision(r) => r.lifecycle.superseded_by.as_deref(),
            Any::Runbook(r) => r.lifecycle.superseded_by.as_deref(),
            Any::Research(r) => r.lifecycle.superseded_by.as_deref(),
            Any::Checklist(_) => None,
        }
    }

    /// Checklist items as `(id, state)` pairs; empty for other kinds.
    pub fn items(&self) -> Vec<(String, &'static str)> {
        match self {
            Any::Checklist(c) => c
                .content
                .items
                .iter()
                .map(|i| {
                    let state = match i.state {
                        ItemState::Open => "open",
                        ItemState::Done => "done",
                        ItemState::Canceled => "canceled",
                    };
                    (i.id.clone(), state)
                })
                .collect(),
            _ => Vec::new(),
        }
    }

    /// Semantic text fields in a fixed order with stable labels, for lexical search; never YAML.
    pub fn search_text(&self) -> Vec<(&'static str, String)> {
        let mut rows = vec![("title", self.title().to_owned())];
        let mut add = |label: &'static str, value: &str| rows.push((label, value.to_owned()));
        match self {
            Any::Decision(r) => {
                add("question", &r.content.question);
                add("decision", &r.content.decision);
                add("rationale", &r.content.rationale);
                for a in &r.content.alternatives {
                    add(
                        "alternative",
                        &format!("{}: {}", a.option, a.rejected_because),
                    );
                }
                for q in &r.content.open_questions {
                    add("open_question", &q.text);
                }
            }
            Any::Research(r) => {
                add("question", &r.content.question);
                for c in &r.content.conclusions {
                    add("conclusion", &c.statement);
                }
                for e in &r.content.evidence {
                    add("evidence", &format!("{} ({})", e.claim, e.source));
                }
                for l in &r.content.limitations {
                    add("limitation", l);
                }
                add("applicability", &r.content.applicability);
            }
            Any::Runbook(r) => {
                add("purpose", &r.content.purpose);
                for p in &r.content.prerequisites {
                    add("prerequisite", p);
                }
                for s in &r.content.steps {
                    add("step", &format!("{}: {}", s.title, s.description));
                }
                for p in &r.content.pitfalls {
                    add("pitfall", p);
                }
            }
            Any::Checklist(c) => {
                add("purpose", &c.content.purpose);
                for i in &c.content.items {
                    add("item", &i.text);
                    if let Some(r) = &i.resolution {
                        add("reason", &r.text);
                    }
                }
            }
        }
        rows
    }

    /// Canonical identifiers, work references and the optional detail reference this record names.
    pub fn references(&self) -> Vec<String> {
        let mut refs = Vec::new();
        if let Some(by) = self.superseded_by() {
            refs.push(by.to_owned());
        }
        match self {
            Any::Decision(r) => refs.extend(r.content.detail.clone()),
            Any::Research(r) => refs.extend(r.content.detail.clone()),
            Any::Runbook(r) => {
                refs.extend(r.content.detail.clone());
                refs.extend(r.uses.iter().filter_map(|u| u.work.clone()));
            }
            Any::Checklist(_) => (),
        }
        refs
    }
}

/// The closed allocator file: six next-number counters.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AllocatorFile {
    /// Owned schema revision.
    pub schema_version: u32,
    /// Next `D-` number.
    pub next_decision: u64,
    /// Next `RB-` number.
    pub next_runbook: u64,
    /// Next `RS-` number.
    pub next_research: u64,
    /// Next `CL-` number.
    pub next_checklist: u64,
    /// Next `DOC-` number, reserved for the document module.
    pub next_document: u64,
    /// Next `CP-` number, reserved for the compaction module.
    pub next_compaction: u64,
}

/// The six counters as observed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Counters {
    /// Next `D-` number.
    pub decision: u64,
    /// Next `RB-` number.
    pub runbook: u64,
    /// Next `RS-` number.
    pub research: u64,
    /// Next `CL-` number.
    pub checklist: u64,
    /// Next `DOC-` number.
    pub document: u64,
    /// Next `CP-` number.
    pub compaction: u64,
}

impl AllocatorFile {
    /// A virgin allocator with every counter at 1.
    fn virgin() -> Self {
        AllocatorFile {
            schema_version: SCHEMA,
            next_decision: 1,
            next_runbook: 1,
            next_research: 1,
            next_checklist: 1,
            next_document: 1,
            next_compaction: 1,
        }
    }

    /// Mutable counter of one prefix.
    fn counter(&mut self, prefix: Prefix) -> &mut u64 {
        match prefix {
            Prefix::Decision => &mut self.next_decision,
            Prefix::Runbook => &mut self.next_runbook,
            Prefix::Research => &mut self.next_research,
            Prefix::Checklist => &mut self.next_checklist,
            Prefix::Document => &mut self.next_document,
            Prefix::Compaction => &mut self.next_compaction,
        }
    }

    /// Counter of one prefix.
    fn get(&self, prefix: Prefix) -> u64 {
        match prefix {
            Prefix::Decision => self.next_decision,
            Prefix::Runbook => self.next_runbook,
            Prefix::Research => self.next_research,
            Prefix::Checklist => self.next_checklist,
            Prefix::Document => self.next_document,
            Prefix::Compaction => self.next_compaction,
        }
    }

    /// Public snapshot of the six counters.
    fn counters(&self) -> Counters {
        Counters {
            decision: self.next_decision,
            runbook: self.next_runbook,
            research: self.next_research,
            checklist: self.next_checklist,
            document: self.next_document,
            compaction: self.next_compaction,
        }
    }
}

/// Observed identifiers of one home.
#[derive(Clone, Debug, PartialEq)]
pub struct HomeState {
    /// The home's prefix.
    pub prefix: Prefix,
    /// Canonical identifiers in numeric order.
    pub ids: Vec<String>,
    /// Largest numeric identifier, 0 when empty.
    pub max: u64,
    /// Named coverage warnings.
    pub warnings: Vec<String>,
    /// Whether every entry was recognized and the bound was not reached.
    pub complete: bool,
}

/// The single allocation observation over all six homes and the allocator file.
#[derive(Clone, Debug, PartialEq)]
pub struct Allocation {
    /// One entry per prefix in [`Prefix::ALL`] order.
    pub homes: Vec<HomeState>,
    /// The six counters when the allocator file exists and decodes.
    pub counters: Option<Counters>,
    /// All named warnings.
    pub warnings: Vec<String>,
    /// Whether every home and the allocator file were fully understood.
    pub complete: bool,
    /// Caller-independent token over exactly these inputs.
    pub version: String,
}

/// Observe one home through the shared bounded one-directory inventory of the store.
///
/// The five flat homes use [`Store::kind_inventory`] directly. The compactions home currently uses
/// the same flat listing, so nested `CP-NNN/` staging directories report the home incomplete; the
/// compaction module's own inventory replaces that single call once its source handoff lands.
fn home_state(store: &Store, prefix: Prefix) -> Result<HomeState> {
    let inventory = store.kind_inventory(prefix.directory(), prefix.as_str())?;
    let max = inventory
        .ids
        .iter()
        .filter_map(|id| model::number(id, prefix.as_str()).ok())
        .max()
        .unwrap_or(0);
    Ok(HomeState {
        prefix,
        ids: inventory.ids,
        max,
        warnings: inventory.warnings,
        complete: inventory.complete,
    })
}

/// Exact allocator bytes (when present) with their decode result.
type AllocatorRead = (Option<Vec<u8>>, Result<Option<AllocatorFile>>);

/// Read and decode the allocator file, returning its exact bytes when present.
fn read_allocator(store: &Store) -> Result<AllocatorRead> {
    let bytes = store.bytes(ALLOCATOR_PATH)?;
    let decoded = match &bytes {
        None => Ok(None),
        Some(b) => store::decode::<AllocatorFile>(b).and_then(|f| {
            if f.schema_version == SCHEMA {
                Ok(Some(f))
            } else {
                Err(Error::new(
                    "allocator",
                    "Unsupported knowledge allocator schema.",
                ))
            }
        }),
    };
    Ok((bytes, decoded))
}

/// Observe all six homes and the allocator once, with no caller-specific input.
///
/// The token covers the version of `project.yaml`, the version of the allocator file (or its
/// absence) and, in [`Prefix::ALL`] order, each home's identifiers, warnings and completeness.
/// Work allocator state and work inventory are not inputs.
pub fn observe_allocation(store: &Store) -> Result<Allocation> {
    let project = store.bytes("project.yaml")?;
    let (allocator_bytes, decoded) = read_allocator(store)?;
    let mut homes = Vec::new();
    for prefix in Prefix::ALL {
        homes.push(home_state(store, prefix)?);
    }
    let mut warnings: Vec<String> = homes.iter().flat_map(|h| h.warnings.clone()).collect();
    let mut complete = homes.iter().all(|h| h.complete);
    let counters = match decoded {
        Ok(file) => file.map(|f| f.counters()),
        Err(e) => {
            complete = false;
            warnings.push(format!("Allocator file unreadable: {}", e.message));
            None
        }
    };
    let mut digest = Sha256::new();
    let mut part = |bytes: &[u8]| {
        digest.update((bytes.len() as u64).to_le_bytes());
        digest.update(bytes);
    };
    part(b"agent-tasks/knowledge-allocation/v1");
    part(store.version("project.yaml", project.as_deref()).as_bytes());
    part(
        store
            .version(ALLOCATOR_PATH, allocator_bytes.as_deref())
            .as_bytes(),
    );
    for home in &homes {
        part(home.prefix.as_str().as_bytes());
        part(home.ids.join(",").as_bytes());
        part(home.warnings.join("\n").as_bytes());
        part(&[u8::from(home.complete)]);
    }
    let version = format!("{:x}", digest.finalize());
    Ok(Allocation {
        homes,
        counters,
        warnings,
        complete,
        version,
    })
}

/// The caller-independent allocation token every reader and creator uses.
pub fn allocation_version(store: &Store) -> Result<String> {
    Ok(observe_allocation(store)?.version)
}

/// Reserve the next number of one prefix under the held root write lock.
///
/// `expected` must equal a fresh [`allocation_version`]. The incremented allocator file is published
/// before the caller publishes its record; a later failure leaves the number as a visible gap that
/// is never recycled. An absent allocator file is valid only while all six homes are empty. Every
/// counter must exceed the largest existing number of its own home. Returns the canonical identifier.
pub fn reserve(
    store: &Store,
    guard: &LockGuard,
    prefix: Prefix,
    expected: &str,
    effects: &mut Vec<String>,
) -> Result<String> {
    if !guard.is_for(store) {
        return Err(Error::new(
            "not_locked",
            "Reserve under the root write lock acquired through this store.",
        ));
    }
    let observed = observe_allocation(store)?;
    if observed.version != expected {
        return Err(Error::new(
            "stale",
            format!(
                "No identifier reserved. Current allocation version: {}. Read project context before retrying.",
                observed.version
            ),
        ));
    }
    if !observed.complete {
        return Err(Error::new(
            "inventory",
            "A knowledge home is incomplete or foreign; inspect project context before allocating.",
        ));
    }
    let (bytes, decoded) = read_allocator(store)?;
    let mut file = match decoded? {
        Some(file) => {
            for home in &observed.homes {
                let counter = file.get(home.prefix);
                if counter == 0 || counter == u64::MAX || counter <= home.max {
                    return Err(Error::new(
                        "allocator",
                        "Missing or invalid allocator counter; restore retained state, never guess reserved IDs.",
                    ));
                }
            }
            file
        }
        None => {
            if observed.homes.iter().any(|h| !h.ids.is_empty()) {
                return Err(Error::new(
                    "allocator",
                    "Knowledge records exist without an allocator; restore retained state, never guess reserved IDs.",
                ));
            }
            AllocatorFile::virgin()
        }
    };
    let number = *file.counter(prefix);
    *file.counter(prefix) = number + 1;
    store.save(ALLOCATOR_PATH, &file, bytes.as_deref(), false, effects)?;
    if store.bytes(ALLOCATOR_PATH)?.as_deref() != Some(store::encode(&file)?.as_slice()) {
        return Err(Error::new(
            "allocation_changed",
            format!(
                "Allocator changed after reserving {}{number:03}; inspect context.",
                prefix.as_str()
            ),
        ));
    }
    Ok(format!("{}{number:03}", prefix.as_str()))
}

/// Create the home directory of a prefix when it is absent, through the store's directory primitive.
///
/// An existing directory is left untouched and adds no effect line. The caller holds the root write
/// lock acquired through this store.
pub fn ensure_home(store: &Store, prefix: Prefix, effects: &mut Vec<String>) -> Result<()> {
    let directory = prefix.directory();
    if store.path(directory)?.exists() {
        return Ok(());
    }
    store.create_dir(directory, effects).map(|_| ())
}

/// Decode and validate one record file as `Any`, binding it to its exact bytes and version.
fn read_record(store: &Store, kind: Kind, id: &str) -> Result<Snapshot<Any>> {
    let relative = kind.path(id).map_err(store::invalid)?;
    let bytes = store
        .bytes(&relative)?
        .ok_or_else(|| Error::new("not_found", format!("{id} does not exist.")))?;
    let value = match kind {
        Kind::Decision => {
            let r: Decision = store::decode(&bytes)?;
            r.validate().map_err(store::invalid)?;
            Any::Decision(r)
        }
        Kind::Runbook => {
            let r: Runbook = store::decode(&bytes)?;
            r.validate().map_err(store::invalid)?;
            Any::Runbook(r)
        }
        Kind::Research => {
            let r: Research = store::decode(&bytes)?;
            r.validate().map_err(store::invalid)?;
            Any::Research(r)
        }
        Kind::Checklist => {
            let r: Checklist = store::decode(&bytes)?;
            r.validate().map_err(store::invalid)?;
            Any::Checklist(r)
        }
    };
    if value.id() != id {
        return Err(store::invalid(
            "Record identifier does not match its file name.",
        ));
    }
    let version = store.version(&relative, Some(&bytes));
    Ok(Snapshot {
        value,
        bytes,
        version,
    })
}

/// Load one typed record by canonical identifier: `not_found` when absent, `invalid_data` when
/// the file is corrupt, foreign-schema or misnamed.
pub fn load(store: &Store, id: &str) -> Result<Snapshot<Any>> {
    let kind = Kind::of(id).map_err(store::invalid)?;
    read_record(store, kind, id)
}

/// Outcome of resolving a checklist child reference.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Child {
    /// The item exists with this state.
    Found {
        /// `open`, `done` or `canceled`.
        state: &'static str,
    },
    /// The checklist exists but has no such item.
    MissingItem,
    /// The checklist does not exist.
    MissingParent,
}

/// Resolve `CL-001/I-001` existence without writing; other shapes are refused.
pub fn resolve_child(store: &Store, id: &KnowledgeId) -> Result<Child> {
    let (Prefix::Checklist, Some(item)) = (id.prefix, id.item) else {
        return Err(store::invalid(
            "Only checklist item identifiers have children.",
        ));
    };
    let parent = KnowledgeId {
        prefix: Prefix::Checklist,
        number: id.number,
        item: None,
    }
    .canonical();
    let snapshot = match load(store, &parent) {
        Ok(s) => s,
        Err(e) if e.code == "not_found" => return Ok(Child::MissingParent),
        Err(e) => return Err(e),
    };
    let wanted = format!("I-{item:03}");
    Ok(snapshot
        .value
        .items()
        .into_iter()
        .find(|(i, _)| *i == wanted)
        .map_or(Child::MissingItem, |(_, state)| Child::Found { state }))
}

/// Bounded scan result: readable records plus named omissions.
pub struct Scan {
    /// Healthy records ordered by kind then number.
    pub records: Vec<Snapshot<Any>>,
    /// Named unreadable or unscanned records.
    pub unreadable: Vec<String>,
    /// Inventory warnings.
    pub warnings: Vec<String>,
    /// Whether every record of the scanned kinds was read.
    pub complete: bool,
    /// Digest of exactly the scanned bytes and omissions.
    pub version: String,
}

/// Read every readable record of one kind or of all four; one bad sibling never hides the rest.
pub fn scan(store: &Store, kind: Option<Kind>) -> Result<Scan> {
    let kinds = match kind {
        Some(k) => vec![k],
        None => vec![
            Kind::Decision,
            Kind::Runbook,
            Kind::Research,
            Kind::Checklist,
        ],
    };
    let mut scan = Scan {
        records: Vec::new(),
        unreadable: Vec::new(),
        warnings: Vec::new(),
        complete: true,
        version: String::new(),
    };
    let mut total = 0usize;
    for kind in kinds {
        let home = home_state(store, kind.prefix())?;
        scan.warnings.extend(home.warnings);
        scan.complete &= home.complete;
        for id in home.ids {
            if total >= store::SCAN_CAP {
                scan.complete = false;
                scan.unreadable
                    .push(format!("{id}: aggregate read budget reached."));
                continue;
            }
            match read_record(store, kind, &id) {
                Ok(snapshot) => {
                    total += snapshot.bytes.len();
                    scan.records.push(snapshot);
                }
                Err(e) => {
                    scan.complete = false;
                    scan.unreadable
                        .push(format!("{id}: {}", store::safe(&e.message, 160)));
                }
            }
        }
    }
    let mut digest = Sha256::new();
    for record in &scan.records {
        digest.update(record.value.id().as_bytes());
        digest.update(record.version.as_bytes());
    }
    for text in scan.unreadable.iter().chain(&scan.warnings) {
        digest.update(text.as_bytes());
    }
    digest.update([u8::from(scan.complete)]);
    scan.version = format!("{:x}", digest.finalize());
    Ok(scan)
}

/// Locator proving the exact current record bytes exist in a reachable committed object.
///
/// Forwards to the publication module's proof. Pending, deferred, ignored or unproven bytes refuse
/// with `history_unrecoverable` so no retained history is dropped without a recoverable original.
fn committed_locator(store: &Store, relative: &str, bytes: &[u8]) -> Result<String> {
    let version = store.version(relative, Some(bytes));
    match crate::persist::locator::locate_committed(store, relative, &version, bytes) {
        Ok(Some(locator)) => Ok(locator),
        Ok(None) => Err(Error::new(
            "history_unrecoverable",
            "Retained history is full and its bytes are not proven in a committed object; nothing was written.",
        )),
        Err(e) => Err(e),
    }
}

/// Eviction closure for one record file: asks for a locator of its current bytes at most once.
pub fn evictor<'a>(
    store: &'a Store,
    relative: &'a str,
    bytes: &'a [u8],
) -> impl FnMut() -> Result<String> + 'a {
    let mut cached: Option<String> = None;
    move || {
        if let Some(locator) = &cached {
            return Ok(locator.clone());
        }
        let locator = committed_locator(store, relative, bytes)?;
        cached = Some(locator.clone());
        Ok(locator)
    }
}

/// Records whose retained ledgers can shed their oldest entry once it is provably committed.
pub trait Retained {
    /// Drop the oldest retained entry, recording its locator; `false` when nothing can be dropped.
    fn evict_oldest(&mut self, locator: &str) -> bool;
}

/// Shared eviction for the revisioned kinds; history first, then (runbooks) uses.
macro_rules! retain_revisioned {
    ($name:ident $(, $uses:ident)?) => {
        impl Retained for $name {
            /// Drop the oldest history entry (then the oldest runbook use) and record its locator.
            fn evict_oldest(&mut self, locator: &str) -> bool {
                if self.evicted.len() < LOCATOR_CAP && !self.history.is_empty() {
                    let oldest = self.history.remove(0);
                    self.evicted.push(Evicted { revision: oldest.revision, locator: locator.to_owned() });
                    return true;
                }
                $(
                    if self.evicted_uses.len() < LOCATOR_CAP && !self.$uses.is_empty() {
                        let oldest = self.$uses.remove(0);
                        let number = model::number(&oldest.id, "U-").unwrap_or(0);
                        self.evicted_uses.push(Evicted { revision: number, locator: locator.to_owned() });
                        return true;
                    }
                )?
                false
            }
        }
    };
}
retain_revisioned!(Decision);
retain_revisioned!(Research);
retain_revisioned!(Runbook, uses);

impl Retained for Checklist {
    /// Drop the oldest event and record its locator.
    fn evict_oldest(&mut self, locator: &str) -> bool {
        if self.evicted_events.len() < LOCATOR_CAP && !self.events.is_empty() {
            self.events.remove(0);
            let ordinal = self.evicted_events.len() as u64 + 1;
            self.evicted_events.push(Evicted {
                revision: ordinal,
                locator: locator.to_owned(),
            });
            return true;
        }
        false
    }
}

/// Replace a record file with `observed` exact bytes as the guard.
///
/// When the encoded record would exceed the nonterminal byte cap, the oldest retained entry is
/// evicted only after `evict` returns a committed locator, repeatedly until it fits; any failure
/// leaves the file untouched. Returns the new file version.
pub fn replace_file<T: Serialize + DeserializeOwned + Retained>(
    store: &Store,
    relative: &str,
    observed: &[u8],
    record: &mut T,
    evict: &mut dyn FnMut() -> Result<String>,
    effects: &mut Vec<String>,
) -> Result<String> {
    let cap = store::RECORD_CAP - store::CLOSING_RESERVE;
    while store::encode(&*record)?.len() > cap {
        let locator = evict()?;
        if !record.evict_oldest(&locator) {
            return Err(Error::new(
                "capacity",
                "Record capacity reached and nothing more can be evicted.",
            ));
        }
    }
    store.save(relative, &*record, Some(observed), false, effects)
}

/// Confirm that the project exists before any knowledge write.
pub fn require_project(store: &Store) -> Result<()> {
    store
        .project()?
        .map(|_| ())
        .ok_or_else(|| Error::new("not_initialized", "Initialize Project first."))
}

/// Publish a new record file without clobbering; its home directory must already exist.
pub fn create_file<T: Serialize + DeserializeOwned>(
    store: &Store,
    relative: &str,
    record: &T,
    effects: &mut Vec<String>,
) -> Result<String> {
    store.save(relative, record, None, false, effects)
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
    use std::fs;

    /// Disposable initialized root; every test owns its own directory.
    struct Fx {
        /// Keeps the directory alive.
        _dir: tempfile::TempDir,
        /// Store over the disposable root.
        store: Store,
    }

    /// Create a project root with `project.yaml`, the work directories and a work allocator file.
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
        Fx { _dir: dir, store }
    }

    /// Acquire the write lock for one test step.
    fn lock(store: &Store) -> LockGuard {
        store.lock(true, &mut Vec::new()).unwrap().unwrap()
    }

    /// A valid decision body.
    fn decision_body(title: &str) -> DecisionBody {
        DecisionBody {
            title: title.into(),
            question: "Which store?".into(),
            decision: "Use files.".into(),
            rationale: "Portable.".into(),
            alternatives: vec![Alternative {
                option: "database".into(),
                rejected_because: "extra daemon".into(),
            }],
            open_questions: vec![],
            detail: None,
        }
    }

    /// A valid runbook body.
    fn runbook_body() -> RunbookBody {
        RunbookBody {
            title: "Deploy".into(),
            purpose: "Ship it".into(),
            prerequisites: vec![],
            inputs: vec![RunbookInput {
                name: "target".into(),
                description: "where".into(),
                required: true,
            }],
            steps: vec![Step {
                title: "Build".into(),
                command: Some("cargo build".into()),
                description: "compile".into(),
                expected: "binary".into(),
                recovery: None,
            }],
            pitfalls: vec![],
            detail: None,
        }
    }

    /// An evictor that always refuses, as in a store without a committed copy.
    fn refuse() -> impl FnMut() -> Result<String> {
        || Err(Error::new("history_unrecoverable", "not committed"))
    }

    #[test]
    fn identifier_grammar_is_canonical_and_pure() {
        for (raw, prefix, number, item) in [
            ("D-001", Prefix::Decision, 1, None),
            ("RB-012", Prefix::Runbook, 12, None),
            ("RS-1000", Prefix::Research, 1000, None),
            ("CL-001/I-002", Prefix::Checklist, 1, Some(2)),
            ("DOC-003", Prefix::Document, 3, None),
            ("CP-004", Prefix::Compaction, 4, None),
        ] {
            let parsed = parse_id(raw).unwrap();
            assert_eq!(
                (parsed.prefix, parsed.number, parsed.item),
                (prefix, number, item)
            );
            assert_eq!(parsed.canonical(), raw);
        }
        for raw in [
            "D-1",
            "D-0001",
            "d-001",
            "D-000",
            "X-001",
            "D-001/I-001",
            "CL-001/I-1",
            "CL-001/T-001",
            "D-001/",
            "../D-001",
            "",
            "D-001 ",
            "D-",
        ] {
            assert!(parse_id(raw).is_err(), "{raw} must be refused");
        }
        assert_eq!(Kind::of("D-001").unwrap(), Kind::Decision);
        assert!(Kind::of("DOC-001").is_err());
        assert!(Kind::of("CL-001/I-001").is_err());
        assert_eq!(
            Kind::Runbook.path("RB-002").unwrap(),
            "runbooks/RB-002.yaml"
        );
        assert!(Kind::Runbook.path("D-002").is_err());
    }

    #[test]
    fn closed_schema_refuses_unknown_fields_and_bad_text() {
        let record = new_decision(
            "D-001",
            decision_body("Pick"),
            "2026-01-01T00:00:00.000Z",
            &None,
        );
        record.validate().unwrap();
        let mut bytes = store::encode(&record).unwrap();
        bytes.extend_from_slice(b"surprise: 1\n");
        assert!(store::decode::<Decision>(&bytes).is_err());
        let mut body = decision_body("Pick");
        body.title = " ".into();
        assert!(body.validate().unwrap_err().starts_with("title:"));
        body = decision_body("Pick");
        body.alternatives = vec![body.alternatives[0].clone(); 9];
        assert!(body.validate().unwrap_err().starts_with("alternatives:"));
        let mut research = ResearchBody {
            title: "R".into(),
            question: "Q".into(),
            conclusions: vec![Conclusion {
                statement: "S".into(),
                basis: Basis::Measured,
            }],
            evidence: vec![],
            limitations: vec![],
            applicability: "A".into(),
            detail: None,
        };
        assert!(research.validate().unwrap_err().contains("needs evidence"));
        research.evidence.push(Evidence {
            claim: "c".into(),
            source: "s".into(),
            basis: Basis::Measured,
        });
        research.validate().unwrap();
        let mut bad = runbook_body();
        bad.inputs.push(bad.inputs[0].clone());
        assert!(bad.validate().unwrap_err().contains("unique"));
        bad = runbook_body();
        bad.inputs[0].name = "Bad Name".into();
        assert!(bad.validate().is_err());
    }

    #[test]
    fn edits_bump_revision_and_keep_history() {
        let mut record = new_decision(
            "D-001",
            decision_body("One"),
            "2026-01-01T00:00:00.000Z",
            &None,
        );
        let by = Some("agent-1".to_owned());
        assert!(
            !record
                .edit(
                    decision_body("One"),
                    "2026-01-02T00:00:00.000Z",
                    &by,
                    &mut refuse()
                )
                .unwrap()
        );
        assert_eq!(record.revision, 1);
        assert!(
            record
                .edit(
                    decision_body("Two"),
                    "2026-01-02T00:00:00.000Z",
                    &by,
                    &mut refuse()
                )
                .unwrap()
        );
        assert_eq!(record.revision, 2);
        assert_eq!(record.history.len(), 1);
        assert_eq!(record.history[0].revision, 1);
        assert_eq!(record.history[0].content.title, "One");
        assert_eq!(record.content.title, "Two");
        assert_eq!(record.meta.updated_by, by);
        assert_eq!(record.meta.created_by, None);
        record.validate().unwrap();
        record
            .supersede("D-002", "2026-01-03T00:00:00.000Z", &None)
            .unwrap();
        assert_eq!(
            record.revision, 2,
            "supersession is not a definition change"
        );
        let refused = record.edit(
            decision_body("Three"),
            "2026-01-04T00:00:00.000Z",
            &None,
            &mut refuse(),
        );
        assert_eq!(refused.unwrap_err().code, "conflict");
    }

    #[test]
    fn supersession_pointer_is_single_and_idempotent() {
        let mut record = new_research_fixture();
        assert!(record.supersede("RS-002", "t", &None).unwrap());
        assert!(!record.supersede("RS-002", "t", &None).unwrap());
        assert_eq!(
            record.supersede("RS-003", "t", &None).unwrap_err().code,
            "conflict"
        );
        assert_eq!(record.lifecycle.superseded_by.as_deref(), Some("RS-002"));
        record.validate().unwrap();
    }

    /// A valid research record whose timestamps are fixed text.
    fn new_research_fixture() -> Research {
        new_research(
            "RS-001",
            ResearchBody {
                title: "R".into(),
                question: "Q".into(),
                conclusions: vec![Conclusion {
                    statement: "S".into(),
                    basis: Basis::Inferred,
                }],
                evidence: vec![],
                limitations: vec![],
                applicability: "A".into(),
                detail: None,
            },
            "2026-01-01T00:00:00.000Z",
            &None,
        )
    }

    #[test]
    fn history_is_evicted_only_with_a_committed_locator() {
        let mut record = new_decision("D-001", decision_body("0"), "t", &None);
        for n in 1..=HISTORY_CAP {
            record
                .edit(decision_body(&n.to_string()), "t", &None, &mut refuse())
                .unwrap();
        }
        assert_eq!(record.history.len(), HISTORY_CAP);
        let before = record.clone();
        let err = record
            .edit(decision_body("overflow"), "t", &None, &mut refuse())
            .unwrap_err();
        assert_eq!(err.code, "history_unrecoverable");
        assert_eq!(record, before, "a refused eviction changes nothing");
        let mut calls = 0;
        let mut prove = || {
            calls += 1;
            Ok("commit:abc".to_owned())
        };
        record
            .edit(decision_body("overflow"), "t", &None, &mut prove)
            .unwrap();
        assert_eq!(calls, 1);
        assert_eq!(record.history.len(), HISTORY_CAP);
        assert_eq!(
            record.evicted,
            vec![Evicted {
                revision: 1,
                locator: "commit:abc".into()
            }]
        );
        assert_eq!(record.history[0].revision, 2);
    }

    #[test]
    fn runbook_use_records_evidence_without_touching_the_definition() {
        let mut runbook = new_runbook("RB-001", runbook_body(), "t", &None);
        let passed = UseCheck {
            label: "smoke".into(),
            status: CheckStatus::Passed,
            detail: None,
        };
        let base = UseInput {
            revision: 1,
            environment: "staging".into(),
            outcome: UseOutcome::Succeeded,
            checks: vec![passed.clone()],
            artifacts: vec![],
            observation: None,
            work: None,
        };
        let id = runbook
            .add_use(base.clone(), "t", &Some("a".into()), &mut refuse())
            .unwrap();
        assert_eq!(id, "U-001");
        assert_eq!(runbook.revision, 1);
        assert!(!runbook.uses[0].stale_revision);
        runbook.validate().unwrap();
        runbook
            .edit(
                {
                    let mut b = runbook_body();
                    b.purpose = "Ship safely".into();
                    b
                },
                "t",
                &None,
                &mut refuse(),
            )
            .unwrap();
        runbook
            .add_use(base.clone(), "t", &None, &mut refuse())
            .unwrap();
        assert!(
            runbook.uses[1].stale_revision,
            "revision 1 is no longer current"
        );
        assert_eq!(runbook.revision, 2);
        for (name, mutate) in [
            (
                "unknown revision",
                Box::new(|u: &mut UseInput| u.revision = 9) as Box<dyn Fn(&mut UseInput)>,
            ),
            (
                "no environment",
                Box::new(|u: &mut UseInput| u.environment = " ".into()),
            ),
            (
                "no evidence",
                Box::new(|u: &mut UseInput| {
                    u.checks.clear();
                }),
            ),
            (
                "succeeded with failed check",
                Box::new(|u: &mut UseInput| {
                    u.checks.push(UseCheck {
                        label: "x".into(),
                        status: CheckStatus::Failed,
                        detail: None,
                    })
                }),
            ),
            (
                "failed without failure",
                Box::new(|u: &mut UseInput| u.outcome = UseOutcome::Failed),
            ),
        ] {
            let mut input = base.clone();
            mutate(&mut input);
            let before = runbook.clone();
            assert!(
                runbook.add_use(input, "t", &None, &mut refuse()).is_err(),
                "{name} must be refused"
            );
            assert_eq!(runbook, before, "{name} changed the record");
        }
        let mut observed = base.clone();
        observed.checks.clear();
        observed.observation = Some("Ran the full procedure by hand; all steps behaved.".into());
        runbook
            .add_use(observed, "t", &None, &mut refuse())
            .unwrap();
        runbook.supersede("RB-002", "t", &None).unwrap();
        runbook.add_use(base, "t", &None, &mut refuse()).unwrap();
        assert!(runbook.uses.last().unwrap().superseded_record);
        runbook.validate().unwrap();
    }

    #[test]
    fn checklist_keeps_facts_reasons_and_explicit_lifecycle() {
        let by = Some("a".to_owned());
        let mut list = new_checklist(
            "CL-001",
            "Release".into(),
            "Steps".into(),
            vec!["build".into(), "tag".into()],
            "t",
            &by,
        );
        list.validate().unwrap();
        assert!(
            list.complete("t", &by, &mut refuse()).is_err(),
            "open items block completion"
        );
        assert!(
            list.resolve_item("I-001", ItemState::Done, " ", "t", &by, &mut refuse())
                .is_err(),
            "a fact is required"
        );
        assert!(
            list.resolve_item("I-009", ItemState::Done, "x", "t", &by, &mut refuse())
                .is_err()
        );
        list.resolve_item(
            "I-001",
            ItemState::Done,
            "binary built",
            "t",
            &by,
            &mut refuse(),
        )
        .unwrap();
        assert!(
            list.resolve_item(
                "I-001",
                ItemState::Canceled,
                "oops",
                "t",
                &by,
                &mut refuse()
            )
            .is_err(),
            "resolved items stay resolved"
        );
        list.resolve_item(
            "I-002",
            ItemState::Canceled,
            "not needed",
            "t",
            &by,
            &mut refuse(),
        )
        .unwrap();
        list.add_items(vec!["publish".into()], "t", &by, &mut refuse())
            .unwrap();
        assert_eq!(list.content.items[2].id, "I-003");
        assert!(list.complete("t", &by, &mut refuse()).is_err());
        list.resolve_item(
            "I-003",
            ItemState::Canceled,
            "later",
            "t",
            &by,
            &mut refuse(),
        )
        .unwrap();
        list.complete("t", &by, &mut refuse()).unwrap();
        assert_eq!(list.state, ChecklistState::Completed);
        assert!(
            list.add_items(vec!["late".into()], "t", &by, &mut refuse())
                .is_err(),
            "closed lists refuse item changes"
        );
        assert!(list.cancel("why", "t", &by, &mut refuse()).is_err());
        list.reopen("found a gap", "t", &by, &mut refuse()).unwrap();
        assert_eq!(list.state, ChecklistState::Open);
        list.validate().unwrap();
        let mut only_canceled = new_checklist(
            "CL-002",
            "T".into(),
            "P".into(),
            vec!["a".into()],
            "t",
            &None,
        );
        only_canceled
            .resolve_item(
                "I-001",
                ItemState::Canceled,
                "skip",
                "t",
                &None,
                &mut refuse(),
            )
            .unwrap();
        assert!(
            only_canceled.complete("t", &None, &mut refuse()).is_err(),
            "skipped is never done"
        );
        only_canceled
            .cancel("abandoned", "t", &None, &mut refuse())
            .unwrap();
        assert_eq!(only_canceled.state, ChecklistState::Canceled);
        assert!(only_canceled.cancel("", "t", &None, &mut refuse()).is_err());
        let mut copy = new_checklist(
            "CL-003",
            "T".into(),
            "P".into(),
            vec!["M-001/T-001".into()],
            "t",
            &None,
        );
        assert!(copy.validate().unwrap_err().contains("duplicates a Task"));
        copy.content.items[0].text = "Run M-001/T-001 locally".into();
        copy.validate().unwrap();
    }

    #[test]
    fn allocation_token_is_caller_independent_and_covers_all_six_homes() {
        let fx = fixture();
        let guard = lock(&fx.store);
        let mut effects = Vec::new();
        let token = allocation_version(&fx.store).unwrap();
        assert_eq!(token, allocation_version(&fx.store).unwrap());
        let state_before = fs::read(fx.store.path(".agent-tasks/state.yaml").unwrap()).unwrap();
        assert!(reserve(&fx.store, &guard, Prefix::Decision, "stale", &mut effects).is_err());
        assert!(effects.is_empty());
        ensure_home(&fx.store, Prefix::Decision, &mut effects).unwrap();
        assert_eq!(
            allocation_version(&fx.store).unwrap(),
            token,
            "an empty home changes nothing"
        );
        let id = reserve(&fx.store, &guard, Prefix::Decision, &token, &mut effects).unwrap();
        assert_eq!(id, "D-001");
        assert!(
            effects
                .iter()
                .any(|e| e == "Published .agent-tasks/knowledge.yaml.")
        );
        let counters = observe_allocation(&fx.store).unwrap().counters.unwrap();
        assert_eq!((counters.decision, counters.document), (2, 1));
        let after_reserve = allocation_version(&fx.store).unwrap();
        assert_ne!(
            after_reserve, token,
            "a reservation changes the token for every caller"
        );
        assert!(
            reserve(&fx.store, &guard, Prefix::Decision, &token, &mut effects).is_err(),
            "the old token is stale"
        );
        // A document record created elsewhere changes the same token the typed callers use.
        let doc = reserve(
            &fx.store,
            &guard,
            Prefix::Document,
            &after_reserve,
            &mut effects,
        )
        .unwrap();
        assert_eq!(doc, "DOC-001");
        let after_doc = allocation_version(&fx.store).unwrap();
        fs::create_dir(fx.store.path("documents").unwrap()).unwrap();
        fs::write(fx.store.path("documents/DOC-001.yaml").unwrap(), "x: 1\n").unwrap();
        assert_ne!(
            allocation_version(&fx.store).unwrap(),
            after_doc,
            "a record in a foreign home changes the token"
        );
        let next = reserve(
            &fx.store,
            &guard,
            Prefix::Compaction,
            &allocation_version(&fx.store).unwrap(),
            &mut effects,
        )
        .unwrap();
        assert_eq!(next, "CP-001");
        let state_after = fs::read(fx.store.path(".agent-tasks/state.yaml").unwrap()).unwrap();
        assert_eq!(
            state_before, state_after,
            "the work allocator is never touched"
        );
    }

    #[test]
    fn allocator_never_guesses_or_recycles() {
        let fx = fixture();
        let guard = lock(&fx.store);
        let mut effects = Vec::new();
        ensure_home(&fx.store, Prefix::Decision, &mut effects).unwrap();
        fs::write(fx.store.path("decisions/D-003.yaml").unwrap(), "x: 1\n").unwrap();
        let token = allocation_version(&fx.store).unwrap();
        let err = reserve(&fx.store, &guard, Prefix::Decision, &token, &mut effects).unwrap_err();
        assert_eq!(
            err.code, "allocator",
            "records without an allocator are never guessed"
        );
        fs::remove_file(fx.store.path("decisions/D-003.yaml").unwrap()).unwrap();
        let token = allocation_version(&fx.store).unwrap();
        reserve(&fx.store, &guard, Prefix::Decision, &token, &mut effects).unwrap();
        fs::write(fx.store.path("decisions/D-005.yaml").unwrap(), "x: 1\n").unwrap();
        let token = allocation_version(&fx.store).unwrap();
        let err = reserve(&fx.store, &guard, Prefix::Decision, &token, &mut effects).unwrap_err();
        assert_eq!(
            err.code, "allocator",
            "a counter at or below an existing id is refused"
        );
        fs::remove_file(fx.store.path("decisions/D-005.yaml").unwrap()).unwrap();
        fs::write(fx.store.path("decisions/stray.txt").unwrap(), "x").unwrap();
        let token = allocation_version(&fx.store).unwrap();
        let err = reserve(&fx.store, &guard, Prefix::Decision, &token, &mut effects).unwrap_err();
        assert_eq!(
            err.code, "inventory",
            "an unrecognized entry blocks allocation"
        );
        fs::remove_file(fx.store.path("decisions/stray.txt").unwrap()).unwrap();
        fs::write(
            fx.store.path(ALLOCATOR_PATH).unwrap(),
            "schema_version: 9\n",
        )
        .unwrap();
        assert!(!observe_allocation(&fx.store).unwrap().complete);
        let token = allocation_version(&fx.store).unwrap();
        assert!(reserve(&fx.store, &guard, Prefix::Decision, &token, &mut effects).is_err());
    }

    #[test]
    fn scan_names_unreadable_siblings_and_never_drops_them() {
        let fx = fixture();
        let guard = lock(&fx.store);
        let mut effects = Vec::new();
        ensure_home(&fx.store, Prefix::Decision, &mut effects).unwrap();
        let token = allocation_version(&fx.store).unwrap();
        let id = reserve(&fx.store, &guard, Prefix::Decision, &token, &mut effects).unwrap();
        let record = new_decision(&id, decision_body("Good"), &store::now(), &None);
        create_file(&fx.store, "decisions/D-001.yaml", &record, &mut effects).unwrap();
        fs::write(
            fx.store.path("decisions/D-002.yaml").unwrap(),
            "not: [valid",
        )
        .unwrap();
        fs::write(fx.store.path("decisions/.D-003.yaml.tmp-1-1").unwrap(), "x").unwrap();
        let scan = scan(&fx.store, Some(Kind::Decision)).unwrap();
        assert_eq!(scan.records.len(), 1);
        assert_eq!(scan.records[0].value.id(), "D-001");
        assert_eq!(scan.unreadable.len(), 1);
        assert!(scan.unreadable[0].starts_with("D-002:"));
        assert!(!scan.complete);
        assert!(
            scan.warnings
                .iter()
                .any(|w| w.contains("Orphan publication temp"))
        );
        assert_eq!(load(&fx.store, "D-009").err().unwrap().code, "not_found");
        assert!(load(&fx.store, "D-002").is_err());
        assert_eq!(
            load(&fx.store, "D-001").unwrap().value.state_label(),
            "current"
        );
    }

    #[test]
    fn child_resolution_distinguishes_missing_parent_and_item() {
        let fx = fixture();
        let guard = lock(&fx.store);
        let mut effects = Vec::new();
        ensure_home(&fx.store, Prefix::Checklist, &mut effects).unwrap();
        let token = allocation_version(&fx.store).unwrap();
        let id = reserve(&fx.store, &guard, Prefix::Checklist, &token, &mut effects).unwrap();
        let list = new_checklist(
            &id,
            "T".into(),
            "P".into(),
            vec!["a".into()],
            &store::now(),
            &None,
        );
        create_file(&fx.store, "checklists/CL-001.yaml", &list, &mut effects).unwrap();
        let child = |raw: &str| resolve_child(&fx.store, &parse_id(raw).unwrap()).unwrap();
        assert_eq!(child("CL-001/I-001"), Child::Found { state: "open" });
        assert_eq!(child("CL-001/I-002"), Child::MissingItem);
        assert_eq!(child("CL-002/I-001"), Child::MissingParent);
        assert!(resolve_child(&fx.store, &parse_id("D-001").unwrap()).is_err());
    }

    #[test]
    fn nested_compaction_staging_keeps_allocation_closed_until_its_inventory_is_consumed() {
        let fx = fixture();
        let guard = lock(&fx.store);
        let mut effects = Vec::new();
        fs::create_dir_all(fx.store.path("compactions/CP-001/r1").unwrap()).unwrap();
        fs::write(fx.store.path("compactions/CP-001.yaml").unwrap(), "x: 1\n").unwrap();
        let observed = observe_allocation(&fx.store).unwrap();
        assert!(
            !observed.complete,
            "unrecognized nested names are never ignored"
        );
        assert!(observed.warnings.iter().any(|w| w.contains("CP-001")));
        let err = reserve(
            &fx.store,
            &guard,
            Prefix::Decision,
            &observed.version,
            &mut effects,
        )
        .unwrap_err();
        assert_eq!(err.code, "inventory");
        assert!(
            effects.is_empty(),
            "nothing is reserved while a home is incomplete"
        );
    }

    #[test]
    fn nonregular_record_entries_make_a_home_incomplete_and_unreadable() {
        let fx = fixture();
        let _guard = lock(&fx.store);
        let mut effects = Vec::new();
        ensure_home(&fx.store, Prefix::Decision, &mut effects).unwrap();
        fs::write(fx.store.path("elsewhere.txt").unwrap(), "x").unwrap();
        std::os::unix::fs::symlink(
            fx.store.path("elsewhere.txt").unwrap(),
            fx.store.path("decisions/D-001.yaml").unwrap(),
        )
        .unwrap();
        let home = home_state(&fx.store, Prefix::Decision).unwrap();
        assert_eq!(home.ids, ["D-001"]);
        assert!(!home.complete);
        let scan = scan(&fx.store, Some(Kind::Decision)).unwrap();
        assert!(scan.records.is_empty());
        assert_eq!(
            scan.unreadable.len(),
            1,
            "a symlinked record is named, never followed"
        );
    }

    #[test]
    fn byte_cap_evicts_only_with_a_committed_locator() {
        let fx = fixture();
        let _guard = lock(&fx.store);
        let mut effects = Vec::new();
        ensure_home(&fx.store, Prefix::Runbook, &mut effects).unwrap();
        let big = |tag: &str| RunbookBody {
            title: tag.into(),
            purpose: "p".into(),
            prerequisites: vec![],
            inputs: vec![],
            steps: (0..STEP_CAP)
                .map(|n| Step {
                    title: format!("step {n}"),
                    command: Some("c".repeat(1024)),
                    description: "d".repeat(1024),
                    expected: "e".repeat(512),
                    recovery: Some("r".repeat(512)),
                })
                .collect(),
            pitfalls: vec![],
            detail: None,
        };
        let mut record = new_runbook("RB-001", big("v0"), "t", &None);
        for n in 1..=6 {
            record
                .edit(big(&format!("v{n}")), "t", &None, &mut refuse())
                .unwrap();
        }
        assert!(store::encode(&record).unwrap().len() > store::RECORD_CAP - store::CLOSING_RESERVE);
        let relative = "runbooks/RB-001.yaml";
        let small = new_runbook("RB-001", runbook_body(), "t", &None);
        create_file(&fx.store, relative, &small, &mut effects).unwrap();
        let observed = fx.store.bytes(relative).unwrap().unwrap();
        effects.clear();
        let mut refused = record.clone();
        let err = replace_file(
            &fx.store,
            relative,
            &observed,
            &mut refused,
            &mut refuse(),
            &mut effects,
        )
        .unwrap_err();
        assert_eq!(err.code, "history_unrecoverable");
        assert!(effects.is_empty());
        assert_eq!(
            fx.store.bytes(relative).unwrap().unwrap(),
            observed,
            "a refused eviction writes nothing"
        );
        let mut prove = || Ok("commit:1234".to_owned());
        replace_file(
            &fx.store,
            relative,
            &observed,
            &mut record,
            &mut prove,
            &mut effects,
        )
        .unwrap();
        assert_eq!(effects, ["Published runbooks/RB-001.yaml."]);
        let Any::Runbook(stored) = load(&fx.store, "RB-001").unwrap().value else {
            panic!()
        };
        assert!(
            !stored.evicted.is_empty(),
            "history was evicted with the locator"
        );
        assert!(stored.evicted.iter().all(|e| e.locator == "commit:1234"));
        assert!(
            store::encode(&stored).unwrap().len() <= store::RECORD_CAP - store::CLOSING_RESERVE
        );
        assert!(stored.history.first().is_none_or(|h| h.revision > 1));
        assert_eq!(stored.content.title, "v6");
    }

    #[test]
    fn history_proof_uses_real_committed_objects() {
        let fx = crate::persist::testing::GitFixture::new();
        let _guard = fx.lock();
        let mut effects = Vec::new();
        ensure_home(&fx.store, Prefix::Decision, &mut effects).unwrap();
        let record = new_decision("D-001", decision_body("Pick"), &store::now(), &None);
        let relative = "decisions/D-001.yaml";
        create_file(&fx.store, relative, &record, &mut effects).unwrap();
        let bytes = fx.store.bytes(relative).unwrap().unwrap();
        let err = evictor(&fx.store, relative, &bytes)().unwrap_err();
        assert_eq!(
            err.code, "history_unrecoverable",
            "unpublished bytes are never proof"
        );
        fx.git(&["add", "--", relative]);
        fx.git(&["commit", "--quiet", "-m", "fixture: decision"]);
        let locator = evictor(&fx.store, relative, &bytes)().unwrap();
        assert!(locator.len() <= 256);
        let recovered =
            crate::persist::locator::read_committed(&fx.store, &locator, store::RECORD_CAP)
                .unwrap();
        assert_eq!(
            recovered, bytes,
            "the locator recovers the exact original bytes"
        );
        let mut changed = bytes.clone();
        changed.extend_from_slice(b"# edited\n");
        assert_eq!(
            evictor(&fx.store, relative, &changed)().unwrap_err().code,
            "history_unrecoverable",
            "changed bytes are not the committed ones"
        );
    }

    #[test]
    fn a_reservation_without_a_record_stays_a_visible_gap() {
        let fx = fixture();
        let guard = lock(&fx.store);
        let mut effects = Vec::new();
        ensure_home(&fx.store, Prefix::Decision, &mut effects).unwrap();
        let token = allocation_version(&fx.store).unwrap();
        assert_eq!(
            reserve(&fx.store, &guard, Prefix::Decision, &token, &mut effects).unwrap(),
            "D-001"
        );
        let token = allocation_version(&fx.store).unwrap();
        assert_eq!(
            reserve(&fx.store, &guard, Prefix::Decision, &token, &mut effects).unwrap(),
            "D-002",
            "an unpublished record leaves its number as a gap that is never reused"
        );
        let counters = observe_allocation(&fx.store).unwrap().counters.unwrap();
        assert_eq!(counters.decision, 3);
        assert!(
            scan(&fx.store, Some(Kind::Decision))
                .unwrap()
                .records
                .is_empty()
        );
    }

    #[test]
    fn search_text_and_references_expose_semantic_fields_only() {
        let mut record = new_decision("D-001", decision_body("Pick"), "t", &None);
        record.supersede("D-002", "t", &None).unwrap();
        let any = Any::Decision(record);
        let labels: Vec<&str> = any.search_text().iter().map(|(l, _)| *l).collect();
        assert_eq!(
            labels,
            ["title", "question", "decision", "rationale", "alternative"]
        );
        assert_eq!(any.references(), vec!["D-002".to_owned()]);
        assert!(!any.current());
        assert_eq!(any.state_label(), "superseded");
        assert_eq!(any.superseded_by(), Some("D-002"));
    }
}
