//! Closed English work records. Current facts are separate from generated history.
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};

/// Owned YAML schema revision; unfamiliar revisions are never rewritten.
pub const SCHEMA: u32 = 1;
/// Maximum children a single persistent lead manages in one module.
pub const MAX_TASKS: usize = 32;
/// Retained machine events; human reports and review history are not evicted.
pub const LOG_TAIL: usize = 256;

/// Reject empty, excessive or control-bearing input while allowing multiline prose.
pub fn text(value: &str, cap: usize) -> Result<(), String> {
    if value.trim().is_empty()
        || value.len() > cap
        || value
            .chars()
            .any(|c| c.is_control() && c != '\n' && c != '\t')
    {
        return Err(format!(
            "Text must be nonempty, at most {cap} UTF-8 bytes, without control characters."
        ));
    }
    Ok(())
}

/// Validate a bounded string list; required labels additionally must be distinct.
pub fn strings(values: &[String], cap: usize, unique: bool) -> Result<(), String> {
    if values.len() > 8 {
        return Err("At most eight values are allowed.".into());
    }
    let mut seen = BTreeSet::new();
    for value in values {
        text(value, cap)?;
        if unique && !seen.insert(value) {
            return Err("Labels must be unique.".into());
        }
    }
    Ok(())
}

/// Decode a positive canonical numeric ID, rejecting path components and alternate spellings.
pub fn number(value: &str, prefix: &str) -> Result<u64, String> {
    let n = value
        .strip_prefix(prefix)
        .and_then(|s| s.parse::<u64>().ok())
        .filter(|n| *n > 0 && *n < u64::MAX)
        .ok_or_else(|| {
            "Expected a canonical work reference such as M-001 or M-001/T-001.".to_owned()
        })?;
    if value != format!("{prefix}{n:03}") {
        return Err("Noncanonical work reference.".into());
    }
    Ok(n)
}

/// Root orientation. No source state or inherited acceptance is inferred from it.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Project {
    /// Owned schema revision.
    pub schema_version: u32,
    /// Human project title, at most 256 UTF-8 bytes.
    pub title: String,
    /// Strategic purpose, at most 1024 UTF-8 bytes.
    pub purpose: String,
    /// Optional reported repository location, at most 256 UTF-8 bytes.
    pub remote: Option<String>,
    /// Machine-generated UTC creation time.
    pub created_at: String,
    /// Machine-generated UTC last managed edit time.
    pub updated_at: String,
}
impl Project {
    /// Check schema, text bounds and dates; this performs no I/O or repair.
    pub fn validate(&self) -> Result<(), String> {
        revision(self.schema_version)?;
        text(&self.title, 256)?;
        text(&self.purpose, 1024)?;
        optional(&self.remote, 256)?;
        dates(&[&self.created_at, &self.updated_at])
    }
}

/// Durable allocator. Loss of this record blocks new module allocation.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Allocator {
    /// Allocator revision: legacy 1 is read-only compatible; new guarded writes publish revision 2.
    pub schema_version: u32,
    /// Next unused module number; reservations are never recycled.
    pub next_module: u64,
    /// Next Epic reservation; absent only for a virgin legacy kind, otherwise restoration is required.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next_epic: Option<u64>,
    /// Next standalone Atomic reservation; missing/invalid used counters block allocation.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next_atomic: Option<u64>,
}

impl Allocator {
    /// Validate numbering and upgrade only an explicit in-memory write candidate to allocator schema 2.
    /// Legacy schema 1 may initialize new kind counters only with empty inventories for those kinds.
    /// Schema 2 always requires every counter, even when a reserved publication left no visible record.
    pub fn prepare(&mut self, ids: &[String]) -> Result<(), String> {
        if !matches!(self.schema_version, 1 | 2) {
            return Err("Unsupported allocator schema_version.".into());
        }
        if self.schema_version == 1 {
            if self.next_epic.is_none() && !ids.iter().any(|id| id.starts_with("E-")) {
                self.next_epic = Some(1);
            }
            if self.next_atomic.is_none() && !ids.iter().any(|id| id.starts_with("A-")) {
                self.next_atomic = Some(1);
            }
        }
        for (prefix, counter) in [
            ("M-", Some(self.next_module)),
            ("E-", self.next_epic),
            ("A-", self.next_atomic),
        ] {
            let maximum = ids
                .iter()
                .filter_map(|id| number(id, prefix).ok())
                .max()
                .unwrap_or(0);
            if !counter.is_some_and(|n| n > maximum && n < u64::MAX) {
                return Err("Missing or invalid allocator counter; restore retained state, never guess reserved IDs.".into());
            }
        }
        self.schema_version = 2;
        Ok(())
    }
}

/// Initial counter for a previously unused kind; inventory validation prevents lost-ID reuse.
pub fn initial_counter() -> Option<u64> {
    Some(1)
}

/// Validate a top-level Epic, Module or Atomic reference and return its prefix and number.
pub fn work_number(value: &str) -> Result<(&'static str, u64), String> {
    for prefix in ["E-", "M-", "A-"] {
        if value.starts_with(prefix) {
            return number(value, prefix).map(|n| (prefix, n));
        }
    }
    Err("Expected E-001, M-001 or A-001.".into())
}

/// Canonical owned filename; callers validate references before opening it.
pub fn work_path(id: &str) -> Result<String, String> {
    let (prefix, _) = work_number(id)?;
    Ok(format!(
        "{}/{id}.yaml",
        match prefix {
            "E-" => "epics",
            "A-" => "atomics",
            _ => "modules",
        }
    ))
}

/// Reported lead identity, never proof of session liveness or authentication.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Lead {
    /// Nonempty display identity, at most 128 UTF-8 bytes.
    pub name: String,
    /// Optional reported session or transcript location, at most 256 UTF-8 bytes.
    pub handle: Option<String>,
}

/// Embedded Task/Atomic lifecycle. Review belongs to the containing Module.
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum TaskState {
    /// Work remains to be completed.
    Open,
    /// A meaningful locally reported outcome exists.
    Done,
    /// Work was removed from remaining scope with a reason.
    Canceled,
}

/// Stored Epic/Module/standalone Atomic cancellation lifecycle; current phases are derived.
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ModuleState {
    /// Work can be edited or independently reviewed.
    Open,
    /// All children are terminal and cancellation has a reason.
    Canceled,
}

/// Explicit check report; absence is not a success.
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum CheckStatus {
    /// Reported check succeeded.
    Passed,
    /// Reported check failed.
    Failed,
    /// Check has not been executed.
    NotRun,
    /// Check is reported inapplicable; this does not waive a required check.
    NotApplicable,
}

/// Agent-supplied check content. Dates and attribution are added by the MCP.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CheckInput {
    /// Unique check label, at most 64 UTF-8 bytes.
    pub label: String,
    /// Explicit check outcome.
    pub status: CheckStatus,
    /// Optional explanation, at most 256 UTF-8 bytes.
    pub detail: Option<String>,
}

/// Current check evidence, with independent per-check attribution.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Check {
    /// Unique target-local label.
    pub label: String,
    /// Explicit reported outcome.
    pub status: CheckStatus,
    /// Optional explanation.
    pub detail: Option<String>,
    /// Machine-generated UTC observation time.
    pub reported_at: String,
    /// Declared reporter; null means unknown.
    pub actor: Option<String>,
}
impl Check {
    /// Attach a generated date and declared actor to validated semantic check content.
    pub fn from_input(input: CheckInput, at: &str, actor: &Option<String>) -> Self {
        Self {
            label: input.label,
            status: input.status,
            detail: input.detail,
            reported_at: at.into(),
            actor: actor.clone(),
        }
    }
}

/// Latest target-owned report; replacing it does not imply external artifact verification.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Report {
    /// Meaningful outcome, at most 1024 UTF-8 bytes.
    pub summary: String,
    /// In-scope missing work; nonempty gaps block module acceptance.
    pub gaps: Vec<String>,
    /// Optional future work outside the current acceptance scope.
    pub followups: Vec<String>,
    /// Reported commit, PR or other artifact references; at most eight.
    pub artifacts: Vec<String>,
    /// Machine-generated UTC report time.
    pub reported_at: String,
    /// Declared author, preserved when a reviewer updates only checks.
    pub actor: Option<String>,
}

/// A dated cancellation fact retained across explicit reopen.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Cancellation {
    /// Required explanation, at most 512 UTF-8 bytes.
    pub reason: String,
    /// Machine-generated UTC time.
    pub at: String,
    /// Declared author.
    pub actor: Option<String>,
}

/// Embedded Task (T-) or Atomic (A-) with target-owned evidence; whole-Module review covers both.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Task {
    /// Positive Module-local T-/A- ID; public references include the owning M- reference.
    pub id: String,
    /// Human title, at most 256 UTF-8 bytes.
    pub title: String,
    /// Optional acceptance criterion, at most 1024 UTF-8 bytes.
    pub criterion: Option<String>,
    /// Optional Atomic executor; Tasks preserve the original lead-through-Module policy.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub executor: Option<Lead>,
    /// Explicit required check labels; missing reports block module approval.
    pub required_checks: Vec<String>,
    /// Local completion lifecycle; no task review state.
    pub state: TaskState,
    /// Current outcome, if reported.
    pub result: Option<Report>,
    /// Current checks; reviewer updates do not replace the result summary.
    pub checks: Vec<Check>,
    /// Active cancellation; null after reopen.
    pub cancellation: Option<Cancellation>,
    /// Prior cancellation facts, preserved on reopen.
    pub cancellation_history: Vec<Cancellation>,
    /// Machine-generated UTC creation time.
    pub created_at: String,
    /// Machine-generated UTC last managed change time.
    pub updated_at: String,
}

/// Actionable module blocker.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Blocker {
    /// What prevents progress, at most 512 UTF-8 bytes.
    pub problem: String,
    /// What must happen to unblock, at most 512 UTF-8 bytes.
    pub needed_action: String,
    /// Optional responsible identity, at most 128 UTF-8 bytes.
    pub resolver: Option<String>,
}

/// Resume guidance independent of review applicability.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Handoff {
    /// Where the lead stopped, at most 512 UTF-8 bytes.
    pub stopping_point: String,
    /// Concrete next action, at most 512 UTF-8 bytes.
    pub next_action: String,
}

/// Independent whole-Epic/Module review verdict; leaves have no separate verdict.
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Verdict {
    /// Current evidence satisfies the module's explicit acceptance conditions.
    Accepted,
    /// The same module needs more work.
    ChangesRequested,
}

/// Reviewer finding; required corrections cannot accompany acceptance.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Finding {
    /// Actionable finding, at most 256 UTF-8 bytes.
    pub text: String,
    /// True for an in-scope correction required before acceptance.
    pub must_fix: bool,
}

/// Before/after snapshots retain attribution when review updates check evidence.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct CheckUpdate {
    /// Top-level work or Module-owned Task/Atomic reference.
    pub target: String,
    /// Target-local check label.
    pub label: String,
    /// Prior report; null when none existed.
    pub before: Option<Check>,
    /// Reviewer's new canonical report.
    pub after: Check,
}

/// Retained verdict bound to a semantic snapshot and a generated epoch.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Review {
    /// Accepted or changes_requested.
    pub verdict: Verdict,
    /// Reviewer conclusion, at most 1024 UTF-8 bytes.
    pub summary: String,
    /// At most eight findings.
    pub findings: Vec<Finding>,
    /// Semantic digest of the reviewed current work.
    pub basis: String,
    /// Epoch after this review; later managed changes invalidate it.
    pub epoch: u64,
    /// Machine-generated UTC time.
    pub at: String,
    /// Declared independent reviewer; null explicitly means unknown.
    pub reviewer: Option<String>,
    /// At most eight changes to canonical checks with original snapshots.
    pub check_updates: Vec<CheckUpdate>,
}

/// Generated concise activity; not a second human-authored development diary.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct LogEntry {
    /// Positive monotonic event ID.
    pub id: String,
    /// Machine-generated UTC time.
    pub at: String,
    /// Declared actor.
    pub actor: Option<String>,
    /// Top-level work or Module-owned Task/Atomic reference.
    pub target: String,
    /// Machine action name.
    pub action: String,
    /// Optional short machine note; never duplicated human report bodies.
    pub note: Option<String>,
}

/// Human reason retained separately from the evictable generated activity tail.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Reason {
    /// Top-level work or Module-owned Task/Atomic reference.
    pub target: String,
    /// Clear or reopen operation.
    pub action: String,
    /// Meaningful reason, at most 512 UTF-8 bytes.
    pub reason: String,
    /// Machine-generated UTC time.
    pub at: String,
    /// Declared author.
    pub actor: Option<String>,
}

/// Shared guarded evidence record: M owns embedded Tasks/Atomics, E owns references, A owns its result.
/// Kind is determined by the canonical ID; validation rejects fields owned by another kind.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Module {
    /// Owned schema revision.
    pub schema_version: u32,
    /// Canonical M-/E-/A- identity matching its kind-specific filename.
    pub id: String,
    /// Human title, at most 256 UTF-8 bytes.
    pub title: String,
    /// Expected outcome, at most 1024 UTF-8 bytes.
    pub outcome: String,
    /// Declared persistent lead.
    pub lead: Option<Lead>,
    /// Explicit module-level required checks.
    pub required_checks: Vec<String>,
    /// Open/canceled lifecycle; phase is derived.
    pub state: ModuleState,
    /// Next task number; missing allocator metadata is readable but not writable.
    pub next_task: Option<u64>,
    /// Next log number; missing allocator metadata blocks writes.
    pub next_log: Option<u64>,
    /// Number of generated events evicted from the recent tail.
    pub omitted_log_entries: u64,
    /// Generated semantic review generation.
    pub review_epoch: u64,
    /// At most 32 embedded tasks.
    pub tasks: Vec<Task>,
    /// Module-owned Atomics, each with independent A numbering and whole-Module review.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub atomics: Vec<Task>,
    /// Next embedded Atomic reservation; checked against existing Atomics before writes.
    #[serde(default = "initial_counter", skip_serializing_if = "Option::is_none")]
    pub next_atomic: Option<u64>,
    /// Epic acceptance criteria; background for children, never implicitly inherited checks.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub criteria: Vec<String>,
    /// Epic-owned Module references; this list is the only membership authority.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub modules: Vec<String>,
    /// Epic-owned standalone Atomic references; canonical Atomic files remain independent.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub atomic_members: Vec<String>,
    /// Standalone integration Atomic's participating Modules, not an ownership relation.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub participants: Vec<String>,
    /// Participating Module semantic basis captured when an Atomic result is reported.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub participant_basis: BTreeMap<String, String>,
    /// Standalone Atomic completion; Module/Epic acceptance continues to use independent review.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub completed: bool,
    /// Current module report, optional when task reports establish the outcome.
    pub result: Option<Report>,
    /// Current module check evidence.
    pub checks: Vec<Check>,
    /// Current blocker, if any.
    pub blocker: Option<Blocker>,
    /// Current resume guidance, if any.
    pub handoff: Option<Handoff>,
    /// Active cancellation reason.
    pub cancellation: Option<Cancellation>,
    /// Dated prior cancellations retained on reopen.
    pub cancellation_history: Vec<Cancellation>,
    /// Review history, bounded by the file's capacity rather than silently evicted.
    pub reviews: Vec<Review>,
    /// Reasons for clear/reopen actions, never evicted with generated events.
    pub reasons: Vec<Reason>,
    /// Recent machine activity tail.
    pub log: Vec<LogEntry>,
    /// Machine-generated UTC creation time.
    pub created_at: String,
    /// Machine-generated UTC last managed change time, never session liveness.
    pub updated_at: String,
}

/// Require the understood schema revision.
fn revision(version: u32) -> Result<(), String> {
    if version != SCHEMA {
        return Err("Unsupported schema_version; no automatic migration.".into());
    }
    Ok(())
}

/// Validate optional bounded prose without treating null as a fabricated value.
pub fn optional(value: &Option<String>, cap: usize) -> Result<(), String> {
    if let Some(value) = value {
        text(value, cap)?;
    }
    Ok(())
}

/// Require bounded UTC RFC 3339 metadata; generated dates are 24 bytes.
fn dates(values: &[&str]) -> Result<(), String> {
    for value in values {
        text(value, 40)?;
        let date =
            chrono::DateTime::parse_from_rfc3339(value).map_err(|_| "Invalid RFC 3339 date.")?;
        if date.offset().local_minus_utc() != 0 {
            return Err("Dates must use UTC.".into());
        }
    }
    Ok(())
}

/// Validate current report bounds, metadata and evidence lists.
fn report(value: &Option<Report>) -> Result<(), String> {
    if let Some(r) = value {
        text(&r.summary, 1024)?;
        strings(&r.gaps, 256, false)?;
        strings(&r.followups, 256, false)?;
        strings(&r.artifacts, 256, false)?;
        optional(&r.actor, 128)?;
        dates(&[&r.reported_at])?;
    }
    Ok(())
}

/// Validate bounded target check reports, unique labels and generated metadata.
fn checks(values: &[Check]) -> Result<(), String> {
    strings(
        &values.iter().map(|c| c.label.clone()).collect::<Vec<_>>(),
        64,
        true,
    )?;
    for c in values {
        optional(&c.detail, 256)?;
        optional(&c.actor, 128)?;
        dates(&[&c.reported_at])?;
    }
    Ok(())
}

/// Validate cancellation provenance and its bounded reason.
fn cancellation(value: &Cancellation) -> Result<(), String> {
    text(&value.reason, 512)?;
    optional(&value.actor, 128)?;
    dates(&[&value.at])
}
impl Module {
    /// Validate all owned semantic data. Missing counters remain diagnostically readable.
    pub fn validate(&self) -> Result<(), String> {
        revision(self.schema_version)?;
        let (kind, _) = work_number(&self.id)?;
        strings(&self.criteria, 1024, true)?;
        if kind == "E-" && self.criteria.is_empty() {
            return Err("Epic needs acceptance criteria.".into());
        }
        for (values, prefix) in [
            (&self.modules, "M-"),
            (&self.atomic_members, "A-"),
            (&self.participants, "M-"),
        ] {
            if values.len() > MAX_TASKS {
                return Err("At most 32 referenced children or participants.".into());
            }
            let mut seen = BTreeSet::new();
            for id in values {
                number(id, prefix)?;
                if !seen.insert(id) {
                    return Err("Duplicate referenced work.".into());
                }
            }
        }
        if kind != "E-"
            && (!self.modules.is_empty()
                || !self.atomic_members.is_empty()
                || !self.criteria.is_empty())
        {
            return Err("Only an Epic owns member references and Epic criteria.".into());
        }
        if kind != "A-"
            && (!self.participants.is_empty()
                || !self.participant_basis.is_empty()
                || self.completed)
        {
            return Err("Only standalone Atomics own integration bases or completion.".into());
        }
        if kind != "M-" && (!self.tasks.is_empty() || !self.atomics.is_empty()) {
            return Err("Only Modules embed execution work.".into());
        }
        if self.completed && self.result.is_none() {
            return Err("Completed Atomic has no result.".into());
        }
        for (id, basis) in &self.participant_basis {
            if !self.participants.contains(id)
                || basis.len() != 64
                || !basis.bytes().all(|c| c.is_ascii_hexdigit())
            {
                return Err("Invalid integration basis.".into());
            }
        }
        text(&self.title, 256)?;
        text(&self.outcome, 1024)?;
        strings(&self.required_checks, 64, true)?;
        if let Some(l) = &self.lead {
            text(&l.name, 128)?;
            optional(&l.handle, 256)?;
        }
        if self.tasks.len() > MAX_TASKS
            || self.atomics.len() > MAX_TASKS
            || self.log.len() > LOG_TAIL
        {
            return Err("Record count limit exceeded.".into());
        }
        let mut ids = BTreeSet::new();
        for (t, prefix) in self
            .tasks
            .iter()
            .map(|t| (t, "T-"))
            .chain(self.atomics.iter().map(|t| (t, "A-")))
        {
            number(&t.id, prefix)?;
            if prefix == "A-" && t.criterion.is_none() {
                return Err("Atomic needs an expected outcome.".into());
            }
            if let Some(l) = &t.executor {
                text(&l.name, 128)?;
                optional(&l.handle, 256)?;
            }
            if !ids.insert(&t.id) {
                return Err("Duplicate task ID.".into());
            }
            text(&t.title, 256)?;
            optional(&t.criterion, 1024)?;
            strings(&t.required_checks, 64, true)?;
            report(&t.result)?;
            checks(&t.checks)?;
            dates(&[&t.created_at, &t.updated_at])?;
            if (t.state == TaskState::Canceled) != t.cancellation.is_some() {
                return Err("Task cancellation state and reason disagree.".into());
            }
            if t.state == TaskState::Done && t.result.is_none() {
                return Err("Done task has no result.".into());
            }
            for c in t.cancellation.iter().chain(&t.cancellation_history) {
                cancellation(c)?;
            }
        }
        report(&self.result)?;
        checks(&self.checks)?;
        if let Some(b) = &self.blocker {
            text(&b.problem, 512)?;
            text(&b.needed_action, 512)?;
            optional(&b.resolver, 128)?;
        }
        if let Some(h) = &self.handoff {
            text(&h.stopping_point, 512)?;
            text(&h.next_action, 512)?;
        }
        if (self.state == ModuleState::Canceled) != self.cancellation.is_some() {
            return Err("Module cancellation state and reason disagree.".into());
        }
        if self.state == ModuleState::Canceled
            && self.children().any(|t| t.state == TaskState::Open)
        {
            return Err("Canceled module has open tasks.".into());
        }
        for c in self.cancellation.iter().chain(&self.cancellation_history) {
            cancellation(c)?;
        }
        for reason in &self.reasons {
            self.target(&reason.target)?;
            text(&reason.action, 64)?;
            text(&reason.reason, 512)?;
            optional(&reason.actor, 128)?;
            dates(&[&reason.at])?;
        }
        let mut last = 0;
        for l in &self.log {
            let n = number(&l.id, "L-")?;
            if n <= last {
                return Err("Log IDs are not strictly ordered.".into());
            }
            last = n;
            self.target(&l.target)?;
            text(&l.action, 64)?;
            optional(&l.note, 256)?;
            optional(&l.actor, 128)?;
            dates(&[&l.at])?;
        }
        for r in &self.reviews {
            text(&r.summary, 1024)?;
            optional(&r.reviewer, 128)?;
            dates(&[&r.at])?;
            if r.basis.len() != 64
                || !r.basis.bytes().all(|c| c.is_ascii_hexdigit())
                || r.epoch > self.review_epoch
            {
                return Err("Invalid review basis or epoch.".into());
            }
            if r.findings.len() > 8 || r.check_updates.len() > 8 {
                return Err("Review item limit exceeded.".into());
            }
            for f in &r.findings {
                text(&f.text, 256)?;
            }
            if r.verdict == Verdict::Accepted && r.findings.iter().any(|f| f.must_fix) {
                return Err("Accepted review contains required corrections.".into());
            }
            for u in &r.check_updates {
                self.target(&u.target)?;
                if u.label != u.after.label || u.before.as_ref().is_some_and(|c| c.label != u.label)
                {
                    return Err("Review check labels disagree.".into());
                }
                checks(std::slice::from_ref(&u.after))?;
                if let Some(c) = &u.before {
                    checks(std::slice::from_ref(c))?;
                }
            }
        }
        dates(&[&self.created_at, &self.updated_at])
    }

    /// Resolve a module or owned task reference, refusing unrelated/path-like references.
    pub fn target(&self, reference: &str) -> Result<Option<usize>, String> {
        if reference == self.id {
            return Ok(None);
        }
        let (module, task) = reference.split_once('/').ok_or("Unknown work reference.")?;
        if module != self.id {
            return Err("Reference belongs to another module.".into());
        }
        if task.starts_with("T-") {
            number(task, "T-")?;
            self.tasks
                .iter()
                .position(|t| t.id == task)
                .map(Some)
                .ok_or("Unknown task.".into())
        } else {
            number(task, "A-")?;
            self.atomics
                .iter()
                .position(|t| t.id == task)
                .map(|i| Some(self.tasks.len() + i))
                .ok_or("Unknown Atomic.".into())
        }
    }

    /// Iterate embedded Tasks then Atomics without loading any other record.
    pub fn children(&self) -> impl Iterator<Item = &Task> {
        self.tasks.iter().chain(&self.atomics)
    }

    /// Read a child index returned by target; only validated indices are accepted.
    pub fn child(&self, index: usize) -> &Task {
        if index < self.tasks.len() {
            &self.tasks[index]
        } else {
            &self.atomics[index - self.tasks.len()]
        }
    }

    /// Mutate an already-resolved child while preserving siblings in the same owning file.
    pub fn child_mut(&mut self, index: usize) -> &mut Task {
        let tasks = self.tasks.len();
        if index < tasks {
            &mut self.tasks[index]
        } else {
            &mut self.atomics[index - tasks]
        }
    }

    /// Require intact next-number metadata before any mutation, without guessing lost reservations.
    pub fn counters(&self) -> Result<(), String> {
        let task_max = self
            .tasks
            .iter()
            .filter_map(|t| number(&t.id, "T-").ok())
            .max()
            .unwrap_or(0);
        let log_max = self
            .log
            .iter()
            .filter_map(|t| number(&t.id, "L-").ok())
            .max()
            .unwrap_or(0);
        let atomic_max = self
            .atomics
            .iter()
            .filter_map(|t| number(&t.id, "A-").ok())
            .max()
            .unwrap_or(0);
        if !self
            .next_atomic
            .is_some_and(|n| n > atomic_max && n < u64::MAX)
        {
            return Err("Missing or invalid Atomic counter; restore retained state.".into());
        }
        if !self.next_task.is_some_and(|n| n > task_max && n < u64::MAX)
            || !self.next_log.is_some_and(|n| n > log_max && n < u64::MAX)
        {
            return Err(
                "Missing or invalid module counters; restore a retained copy before writing."
                    .into(),
            );
        }
        Ok(())
    }

    /// Hash only acceptance-relevant current work, excluding metadata, logs and handoff.
    pub fn basis(&self) -> Result<String, String> {
        let tasks: Vec<_> = self.tasks.iter().map(|t| serde_json::json!({
            "id":t.id,"title":t.title,"criterion":t.criterion,"required_checks":t.required_checks,
            "state":t.state,"result":report_basis(&t.result),"checks":check_basis(&t.checks),
            "cancellation":t.cancellation.as_ref().map(|c| &c.reason)
        })).collect();
        let mut value = serde_json::json!({"title":self.title,"outcome":self.outcome,"lead":self.lead,
            "required_checks":self.required_checks,"state":self.state,"blocker":self.blocker,
            "result":report_basis(&self.result),"checks":check_basis(&self.checks),"tasks":tasks,
            "cancellation":self.cancellation.as_ref().map(|c| &c.reason)});
        // Preserve legacy Module review digests when every new semantic field is absent.
        if !self.atomics.is_empty() {
            value["atomics"] = serde_json::Value::Array(self.atomics.iter().map(|t| serde_json::json!({
                "id":t.id,"title":t.title,"criterion":t.criterion,"executor":t.executor,
                "required_checks":t.required_checks,"state":t.state,"result":report_basis(&t.result),
                "checks":check_basis(&t.checks),"cancellation":t.cancellation.as_ref().map(|c| &c.reason)
            })).collect());
        }
        if self.id.starts_with("E-") {
            value["epic"] = serde_json::json!({"criteria":self.criteria,"modules":self.modules,"atomics":self.atomic_members});
        }
        if self.id.starts_with("A-") {
            value["atomic"] = serde_json::json!({"completed":self.completed,"participants":self.participants,"basis":self.participant_basis});
        }
        let bytes = serde_json::to_vec(&value).map_err(|_| "Cannot encode review basis.")?;
        Ok(format!("{:x}", Sha256::digest(bytes)))
    }

    /// List actionable acceptance conditions; canceled children are excluded without erasing history.
    pub fn acceptance(&self) -> Vec<String> {
        let mut missing = Vec::new();
        if self.state == ModuleState::Canceled {
            missing.push("Module is canceled.".into());
        }
        if self.blocker.is_some() {
            missing.push("Clear the active blocker.".into());
        }
        if self.children().any(|t| t.state == TaskState::Open) {
            missing.push("Complete or cancel open tasks.".into());
        }
        if !self.children().any(|t| t.state == TaskState::Done) && self.result.is_none() {
            missing.push(
                "Report the module outcome; canceled tasks are not delivery evidence.".into(),
            );
        }
        for (target, r, required, current) in std::iter::once((
            self.id.clone(),
            &self.result,
            &self.required_checks,
            &self.checks,
        ))
        .chain(
            self.children()
                .filter(|t| t.state != TaskState::Canceled)
                .map(|t| {
                    (
                        format!("{}/{}", self.id, t.id),
                        &t.result,
                        &t.required_checks,
                        &t.checks,
                    )
                }),
        ) {
            if r.as_ref().is_some_and(|r| !r.gaps.is_empty()) {
                missing.push(format!("{target}: resolve in-scope gaps."));
            }
            for label in required {
                if !current
                    .iter()
                    .any(|c| &c.label == label && c.status == CheckStatus::Passed)
                {
                    missing.push(format!(
                        "{target}: required check {label:?} has not passed."
                    ));
                }
            }
        }
        missing
    }

    /// Derive the display phase without writing files or inventing agent liveness.
    pub fn phase(&self) -> &'static str {
        if self.state == ModuleState::Canceled {
            return "canceled";
        }
        if self.id.starts_with("A-") {
            return if self.completed && self.acceptance().is_empty() {
                "done"
            } else if self.completed {
                "stale completion"
            } else if self.result.is_some() {
                "working"
            } else {
                "planned"
            };
        }
        if let Some(r) = self.reviews.last() {
            let applicable =
                r.epoch == self.review_epoch && self.basis().is_ok_and(|b| b == r.basis);
            if applicable {
                return if r.verdict == Verdict::Accepted {
                    "accepted"
                } else {
                    "changes requested"
                };
            }
            if r.verdict == Verdict::Accepted {
                return "stale approval";
            }
        }
        if self.acceptance().is_empty() {
            "ready"
        } else if self.result.is_some()
            || self
                .children()
                .any(|t| t.state != TaskState::Open || t.result.is_some())
        {
            "working"
        } else {
            "planned"
        }
    }

    /// Append one generated event and evict only redundant machine tail entries.
    pub fn event(
        &mut self,
        target: &str,
        action: &str,
        at: &str,
        actor: &Option<String>,
    ) -> Result<(), String> {
        self.counters()?;
        let n = self.next_log.ok_or("Missing log counter.")?;
        self.next_log = Some(n.checked_add(1).ok_or("Log counter exhausted.")?);
        self.log.push(LogEntry {
            id: format!("L-{n:03}"),
            at: at.into(),
            actor: actor.clone(),
            target: target.into(),
            action: action.into(),
            note: None,
        });
        if self.log.len() > LOG_TAIL {
            self.log.remove(0);
            self.omitted_log_entries = self
                .omitted_log_entries
                .checked_add(1)
                .ok_or("Log omission counter exhausted.")?;
        }
        self.updated_at = at.into();
        Ok(())
    }
}

/// Acceptance-related report projection; no dates or actor labels participate.
fn report_basis(report: &Option<Report>) -> serde_json::Value {
    report.as_ref().map(|r| serde_json::json!({"summary":r.summary,"gaps":r.gaps,"followups":r.followups,"artifacts":r.artifacts})).unwrap_or(serde_json::Value::Null)
}

/// Acceptance-related check projection, deterministically ordered by label.
fn check_basis(checks: &[Check]) -> Vec<serde_json::Value> {
    let mut checks: Vec<_> = checks.iter().collect();
    checks.sort_by(|a, b| a.label.cmp(&b.label));
    checks
        .iter()
        .map(|c| serde_json::json!({"label":c.label,"status":c.status,"detail":c.detail}))
        .collect()
}
