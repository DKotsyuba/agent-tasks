//! Closed semantic arguments. Presence-aware edits never erase omitted fields.
use crate::model::{
    AgentRole, BoundaryObservation, CheckInput, CheckStatus, Contracts, CriterionScope, Dependency,
    Execution, Finding, FindingResolution, Lead, Verdict,
};
use schemars::JsonSchema;
use serde::{Deserialize, Deserializer};
use serde_json::Value;

/// Partial field edit: omission preserves, explicit null clears, a value replaces.
#[derive(Clone, Debug, Default)]
pub enum Patch<T> {
    /// Preserve the existing value.
    #[default]
    Absent,
    /// Clear an optional field; required fields reject this later.
    Clear,
    /// Replace the field with a supplied value.
    Set(T),
}
impl<'de, T: Deserialize<'de>> Deserialize<'de> for Patch<T> {
    /// Decode explicit null as Clear; serde default supplies Absent only for omission.
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        Option::<T>::deserialize(d).map(|v| v.map_or(Self::Clear, Self::Set))
    }
}
impl<T> Patch<T> {
    /// Apply an edit to optional data; omission leaves it intact.
    pub fn optional(self, value: &mut Option<T>) {
        match self {
            Self::Absent => (),
            Self::Clear => *value = None,
            Self::Set(v) => *value = Some(v),
        }
    }
    /// Apply a required-field edit, rejecting null rather than silently preserving it.
    pub fn required(self, value: &mut T) -> Result<(), String> {
        match self {
            Self::Absent => (),
            Self::Clear => return Err("A required field cannot be cleared.".into()),
            Self::Set(v) => *value = v,
        }
        Ok(())
    }
}

/// Explicit one-call documentation registration; metadata is human-authored, dates are generated.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RegisterArgs {
    /// Unique project alias, at most 128 UTF-8 bytes; later calls need only this name.
    pub project: String,
    /// Absolute final documentation folder; its parent must already exist.
    pub doc_dir: std::path::PathBuf,
    /// Human project name, at most 256 UTF-8 bytes.
    pub name: String,
    /// Strategic description, at most 1024 UTF-8 bytes.
    pub description: String,
    /// Optional source-code repository URL stored as a reported manifest field, not a Git remote.
    pub remote: Option<String>,
    /// Optional separate documentation origin; added locally, never pushed.
    pub docs_remote: Option<String>,
}

/// Discover projects with no required arguments; optional snapshot pagination bounds large registries.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ProjectListArgs {
    /// Zero-based project offset, default zero.
    #[serde(default)]
    pub start: usize,
    /// Maximum displayed projects, 1-20, default 20.
    #[serde(default = "page_limit")]
    pub limit: usize,
    /// Exact previous snapshot; required on continuation.
    pub version: Option<String>,
}

/// Allowlisted context projection; YAML is never an agent-facing response.
#[derive(Clone, Copy, Debug, Default, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum View {
    /// Compact orientation and current conditions.
    #[default]
    Summary,
    /// Embedded Tasks and Atomics, or an Epic member list, with their local state.
    Tasks,
    /// Current outcome details and reported artifacts.
    Results,
    /// Required and reported check evidence.
    Checks,
    /// One retained review with pageable findings and check changes.
    Review,
    /// Recent generated activity and explicit omitted history.
    Log,
    /// Retained local commit observations/messages without rescanning Git.
    Commits,
    /// Current ready connected components, candidate/contract coverage and gaps.
    Integration,
}

/// Read one Project/Epic/Module/Task/Atomic scope with snapshot-bound pagination.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ContextArgs {
    /// Configured project alias; not a path or a global current project.
    pub project: String,
    /// Omit for Project; otherwise E-001, M-001, A-001, M-001/T-001 or M-001/A-001.
    #[serde(rename = "ref")]
    pub reference: Option<String>,
    /// Default summary; select one allowlisted detail view.
    #[serde(default)]
    pub view: View,
    /// Zero-based offset into the selected view; default zero.
    #[serde(default)]
    pub start: usize,
    /// Maximum displayed items, 1–20; default 20.
    #[serde(default = "page_limit")]
    pub limit: usize,
    /// Exact snapshot returned by the previous page; mandatory for nonzero offsets.
    pub version: Option<String>,
    /// Zero-based retained review index; omission selects the latest.
    pub review_index: Option<usize>,
}

/// Return an owner-ready English status of the tracked work, not runtime liveness.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct StatusArgs {
    /// Configured project alias.
    pub project: String,
    /// Optional E-001/M-001/A-001 record narrowing when aggregate detail is partial.
    pub module: Option<String>,
}

/// Bounded lexical work search over semantic fields, with exact continuation.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SearchArgs {
    /// Configured project alias.
    pub project: String,
    /// One to eight whitespace-separated terms, at most 256 UTF-8 bytes; all must match.
    pub query: String,
    /// Optional E-001/M-001/A-001 record narrowing.
    pub module: Option<String>,
    /// Zero-based match offset; default zero.
    #[serde(default)]
    pub start: usize,
    /// Maximum displayed matches, 1–20; default 20.
    #[serde(default = "page_limit")]
    pub limit: usize,
    /// Previous result's exact snapshot, required for a nonzero start.
    pub version: Option<String>,
}

/// Default bounded page request.
fn page_limit() -> usize {
    20
}

/// Planned task substance; numbering and dates belong to the MCP.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TaskInput {
    /// Human task title, at most 256 UTF-8 bytes.
    pub title: String,
    /// Optional acceptance criterion, at most 1024 UTF-8 bytes.
    pub criterion: Option<String>,
    /// At most eight distinct required check labels; defaults empty.
    #[serde(default)]
    pub required_checks: Vec<String>,
}

/// Flat operation-specific plan arguments. Common project/version/actor are decoded separately.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(tag = "op", rename_all = "snake_case", deny_unknown_fields)]
pub enum Plan {
    /// Freeze the negotiated roster after lead discovery/agreement and cycle/isolation checks, not completion waits.
    FreezeEpic {
        /// Existing Epic reference and current owner Version.
        epic: String,
    },
    /// Explicitly initialize only missing owned records in an empty configured root.
    InitProject {
        /// Required human project title.
        title: String,
        /// Required strategic purpose.
        purpose: String,
        /// Optional reported repository location.
        remote: Option<String>,
    },
    /// Partially edit root orientation using the manifest version.
    EditProject {
        /// Omission preserves; null is forbidden.
        #[serde(default, deserialize_with = "required_patch")]
        #[schemars(with = "String")]
        title: Patch<String>,
        /// Omission preserves; null is forbidden.
        #[serde(default, deserialize_with = "required_patch")]
        #[schemars(with = "String")]
        purpose: Patch<String>,
        /// Omission preserves; null clears.
        #[serde(default)]
        #[schemars(with = "Option<String>")]
        remote: Patch<String>,
    },
    /// Allocate a standalone module and optionally its initial tasks in one request.
    CreateModule {
        /// Required human title.
        title: String,
        /// Required expected outcome.
        outcome: String,
        /// Optional declared persistent lead.
        lead: Option<Lead>,
        /// Explicit required module checks; defaults empty.
        #[serde(default)]
        required_checks: Vec<String>,
        /// Initial semantic tasks, at most 32; defaults empty.
        #[serde(default)]
        tasks: Vec<TaskInput>,
        /// Explicit Module acceptance criteria; empty planning records cannot begin.
        #[serde(default)]
        criteria: Vec<String>,
        /// Reported source/checkout/implementation/delivery branches; required before Module begin.
        execution: Option<Execution>,
        /// Declared provides/consumes or explicit not_required; no implicit dependency.
        contracts: Option<Contracts>,
        /// Actual blocking start conditions, distinct from contract direction.
        #[serde(default)]
        dependencies: Vec<Dependency>,
    },
    /// Allocate one Epic using the Project Allocation version; attach existing members separately.
    CreateEpic {
        /// Nonempty Epic title.
        title: String,
        /// Expected Epic outcome.
        outcome: String,
        /// One to eight meaningful acceptance criteria; children receive these as background context.
        criteria: Vec<String>,
        /// Optional persistent Epic lead.
        lead: Option<Lead>,
        /// Explicit Epic-owned required checks; never automatically inherited.
        #[serde(default)]
        required_checks: Vec<String>,
    },
    /// Replace Epic membership or partially edit intent using the dependency-bound Epic Version.
    EditEpic {
        /// Target E-001; module and Atomic ownership lives only in this record.
        epic: String,
        /// Omission preserves; null refuses.
        #[serde(default, deserialize_with = "required_patch")]
        #[schemars(with = "String")]
        title: Patch<String>,
        /// Omission preserves; null refuses.
        #[serde(default, deserialize_with = "required_patch")]
        #[schemars(with = "String")]
        outcome: Patch<String>,
        /// Omission preserves; explicit nonempty list replaces criteria.
        #[serde(default, deserialize_with = "required_patch")]
        #[schemars(with = "Vec<String>")]
        criteria: Patch<Vec<String>>,
        /// Omission preserves; null clears declared lead.
        #[serde(default)]
        #[schemars(with = "Option<Lead>")]
        lead: Patch<Lead>,
        /// Omission preserves; [] clears explicit checks.
        #[serde(default, deserialize_with = "required_patch")]
        #[schemars(with = "Vec<String>")]
        required_checks: Patch<Vec<String>>,
        /// Omission preserves; [] removes memberships without deleting or canceling children.
        #[serde(default, deserialize_with = "required_patch")]
        #[schemars(with = "Vec<String>")]
        modules: Patch<Vec<String>>,
        /// Omission preserves; [] removes memberships; only standalone A-001 refs are accepted.
        #[serde(default, deserialize_with = "required_patch")]
        #[schemars(with = "Vec<String>")]
        atomics: Patch<Vec<String>>,
        /// Exact zero-based current business criterion scopes; omission preserves, [] clears.
        #[serde(default, deserialize_with = "required_patch")]
        #[schemars(with = "Vec<CriterionScope>")]
        criterion_scopes: Patch<Vec<CriterionScope>>,
    },
    /// Allocate a standalone Atomic; attach it to an Epic separately using edit_epic.
    CreateAtomic {
        /// Nonempty Atomic title.
        title: String,
        /// Meaningful expected outcome; no invented Task wrapper is required.
        outcome: String,
        /// Optional declared executor/handle.
        executor: Option<Lead>,
        /// Explicit required check labels, which also name integration scenarios when useful.
        #[serde(default)]
        required_checks: Vec<String>,
        /// Participating Modules for integration evidence; at most 32, default empty.
        #[serde(default)]
        participants: Vec<String>,
        /// Optional execution context for explicit local Git report import.
        execution: Option<Execution>,
        /// Real integration environment, required before participant-based Atomic begin.
        environment: Option<String>,
        /// Integration scenarios, required before begin when participants are declared.
        #[serde(default)]
        scenarios: Vec<String>,
    },
    /// Allocate an Atomic embedded in the single owning Module file.
    AddAtomic {
        /// Owning M-001; use its whole-file Version.
        module: String,
        /// Nonempty Atomic title.
        title: String,
        /// Expected independently reviewed Atomic outcome.
        outcome: String,
        /// Optional declared executor.
        executor: Option<Lead>,
        /// Explicit Atomic checks, enforced at its independent and owning-Module reviews.
        #[serde(default)]
        required_checks: Vec<String>,
    },
    /// Partially edit an Atomic's intent; standalone participant changes invalidate current completion.
    EditAtomic {
        /// A-001 or M-001/A-001; use owning record Version.
        #[serde(rename = "ref")]
        reference: String,
        /// Omission preserves; null refuses.
        #[serde(default, deserialize_with = "required_patch")]
        #[schemars(with = "String")]
        title: Patch<String>,
        /// Omission preserves; null refuses.
        #[serde(default, deserialize_with = "required_patch")]
        #[schemars(with = "String")]
        outcome: Patch<String>,
        /// Omission preserves; null clears.
        #[serde(default)]
        #[schemars(with = "Option<Lead>")]
        executor: Patch<Lead>,
        /// Omission preserves; [] clears.
        #[serde(default, deserialize_with = "required_patch")]
        #[schemars(with = "Vec<String>")]
        required_checks: Patch<Vec<String>>,
        /// Standalone Atomics only; Module integration refs are not ownership.
        #[serde(default, deserialize_with = "required_patch")]
        #[schemars(with = "Vec<String>")]
        participants: Patch<Vec<String>>,
        /// Standalone only; omit preserves, null clears.
        #[serde(default)]
        #[schemars(with = "Option<Execution>")]
        execution: Patch<Execution>,
        /// Standalone integration environment; omit preserves, null clears.
        #[serde(default)]
        #[schemars(with = "Option<String>")]
        environment: Patch<String>,
        /// Standalone integration scenarios; [] clears, null refuses.
        #[serde(default, deserialize_with = "required_patch")]
        #[schemars(with = "Vec<String>")]
        scenarios: Patch<Vec<String>>,
    },
    /// Partially edit a module plan; semantic changes make old approval historical.
    EditModule {
        /// Target M-001.
        module: String,
        /// Omission preserves; null is forbidden.
        #[serde(default, deserialize_with = "required_patch")]
        #[schemars(with = "String")]
        title: Patch<String>,
        /// Omission preserves; null is forbidden.
        #[serde(default, deserialize_with = "required_patch")]
        #[schemars(with = "String")]
        outcome: Patch<String>,
        /// Omission preserves; null clears.
        #[serde(default)]
        #[schemars(with = "Option<Lead>")]
        lead: Patch<Lead>,
        /// Omission preserves; [] clears; null is forbidden.
        #[serde(default, deserialize_with = "required_patch")]
        #[schemars(with = "Vec<String>")]
        required_checks: Patch<Vec<String>>,
        /// Omission preserves; explicit list replaces Module criteria.
        #[serde(default, deserialize_with = "required_patch")]
        #[schemars(with = "Vec<String>")]
        criteria: Patch<Vec<String>>,
        /// Omission preserves; null clears execution and blocks begin/import.
        #[serde(default)]
        #[schemars(with = "Option<Execution>")]
        execution: Patch<Execution>,
        /// Omission preserves; null makes contract readiness unknown.
        #[serde(default)]
        #[schemars(with = "Option<Contracts>")]
        contracts: Patch<Contracts>,
        /// Omission preserves; [] clears explicit waits; blocking cycles refuse.
        #[serde(default, deserialize_with = "required_patch")]
        #[schemars(with = "Vec<Dependency>")]
        dependencies: Patch<Vec<Dependency>>,
    },
    /// Allocate one task inside its module using the module file version.
    AddTask {
        /// Target M-001.
        module: String,
        /// Human task title.
        title: String,
        /// Optional acceptance criterion.
        criterion: Option<String>,
        /// Explicit required task checks; defaults empty.
        #[serde(default)]
        required_checks: Vec<String>,
    },
    /// Partially edit one embedded task without touching its result or siblings.
    EditTask {
        /// Target M-001/T-001.
        #[serde(rename = "ref")]
        reference: String,
        /// Omission preserves; null is forbidden.
        #[serde(default, deserialize_with = "required_patch")]
        #[schemars(with = "String")]
        title: Patch<String>,
        /// Omission preserves; null clears.
        #[serde(default)]
        #[schemars(with = "Option<String>")]
        criterion: Patch<String>,
        /// Omission preserves; [] clears; null is forbidden.
        #[serde(default, deserialize_with = "required_patch")]
        #[schemars(with = "Vec<String>")]
        required_checks: Patch<Vec<String>>,
    },
}

/// Optional result completion state; canceled requires the dedicated reasoned operation.
#[derive(Clone, Copy, Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Completion {
    /// Keep this Task/Atomic open.
    Open,
    /// Complete a meaningful local Task/Atomic result; modern Atomic final acceptance still needs independent review.
    Done,
}

/// Explicit role recovery stage; no runtime polling or automatic replacement.
#[derive(Clone, Copy, Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum RecoveryStage {
    /// Original actor is lost and cannot continue or resume.
    Lost,
    /// Replacement reconstructs context; gaps still block use.
    Immersed,
}

/// Record current substance or lifecycle, never supplied timestamps or raw patches.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(tag = "op", rename_all = "snake_case", deny_unknown_fields)]
pub enum Work {
    /// Explicitly activate lead-first coordination; reads never migrate or invent historical facts.
    AdoptCore {},
    /// Bind actual launch receipt identity; never launches a model or guesses an ID.
    BindAgent {
        /// Persistent role owned by this record.
        role: AgentRole,
        /// Supported configured harness.
        harness: String,
        /// Actual observed runtime ID.
        agent_id: String,
        /// Supported messaging address.
        communication_ref: String,
        /// Optional actual resume address.
        resume_ref: Option<String>,
        /// Observed launch/request receipt.
        launch_ref: String,
    },
    /// Report both irrecoverable loss or replacement's reconstructed understanding.
    RecoverAgent {
        /// Current persistent role.
        role: AgentRole,
        /// lost or immersed; no arbitrary state setter.
        stage: RecoveryStage,
        /// Lost-stage reason, at most 512 bytes.
        reason: Option<String>,
        /// Observed loss AND inability to continue/resume.
        observation: Option<String>,
        /// Lost stage requires true.
        lost: Option<bool>,
        /// Lost stage requires true; temporary timeouts do not qualify.
        unrecoverable: Option<bool>,
        /// Immersed understanding, at most 1024 bytes.
        understanding: Option<String>,
        /// Eight context/source references.
        #[serde(default)]
        sources: Vec<String>,
        /// Eight concrete unfinished items.
        #[serde(default)]
        unfinished: Vec<String>,
        /// Eight unresolved gaps; any gap blocks continuation.
        #[serde(default)]
        gaps: Vec<String>,
    },
    /// Actual bound lead discovers responsibility, Tasks and boundary obligations before implementation.
    Planning {
        /// Code-supported responsibility.
        responsibility: String,
        /// Inclusion/read boundary.
        scope: String,
        /// Eight exclusions.
        #[serde(default)]
        exclusions: Vec<String>,
        /// Eight code/material refs.
        #[serde(default)]
        read_refs: Vec<String>,
        /// Advisory unknowns; blockers preserve mandatory unresolved work.
        #[serde(default)]
        uncertainties: Vec<String>,
    },
    /// Actual bound affected lead confirms exact reciprocal canonical revision.
    AgreeContract {
        /// Canonical boundary ID.
        contract_id: String,
        /// Exact positive current revision.
        revision: u64,
        /// Meaningful agreement.
        summary: String,
    },
    /// Report correct pass, intended mutant failure, and restored pass for exact candidate/contract.
    BoundaryEvidence {
        /// Canonical ID, or local when no cross-boundary contract is declared.
        contract_id: String,
        /// Exact positive revision.
        revision: u64,
        /// Restored candidate.
        candidate: String,
        /// Controlled initial conditions.
        conditions: String,
        /// Correct implementation control.
        correct: BoundaryObservation,
        /// Deliberate promised-behavior violation.
        mutation: String,
        /// Intended failure under the same contract test.
        failed: BoundaryObservation,
        /// Successful restored control.
        restored: BoundaryObservation,
        /// Eight primary artifacts.
        #[serde(default)]
        artifacts: Vec<String>,
    },
    /// Actual current business/E2E verification over the declared criterion's affected composition.
    VerifyCriterion {
        /// Zero-based current Epic criterion index.
        index: usize,
        /// Exact current criterion text.
        text: String,
        /// Exact declared affected Module set.
        modules: Vec<String>,
        /// Actual composition candidate, not union of pairwise jobs.
        candidate: String,
        /// Actual environment.
        environment: String,
        /// Actual scenarios.
        scenarios: Vec<String>,
        /// Meaningful business outcome.
        summary: String,
        /// Required passed check observations.
        checks: Vec<CheckInput>,
        /// Eight primary artifacts.
        #[serde(default)]
        artifacts: Vec<String>,
        /// Optional ONE accepted composition covering the whole affected set.
        integration_ref: Option<String>,
    },
    /// Report execution start; never launches agents. Core Epic roster freezes explicitly after planning.
    Begin {},
    /// Explicitly complete a Task/Atomic using its already-recorded meaningful result; never reviews a Task.
    Complete {},
    /// Report delivery of a currently reviewed Module to its declared target; local merge is sufficient.
    Deliver {
        /// Must match execution.target_branch.
        target_branch: String,
        /// Meaningful reported delivery/merge outcome.
        summary: String,
        /// Optional reported commit/PR/artifact reference.
        artifact: Option<String>,
    },
    /// Read explicit local commits and import reports once; no Git writes and no automatic completion.
    ImportCommits {
        /// One to eight hex commit selectors; helper returns full SHA and canonical repository identity.
        commits: Vec<String>,
        /// Omission preserves current lifecycle; explicit done is the lead's local completion decision.
        state: Option<Completion>,
    },
    /// Replace the target's current complete report and check set.
    Result {
        /// Meaningful outcome, at most 1024 UTF-8 bytes.
        summary: String,
        /// Definite Module/integration candidate; no separate submission action.
        candidate: Option<String>,
        /// Eight explicit changed-scope references for later review.
        #[serde(default)]
        changed_scope: Vec<String>,
        /// Tasks/Atomics only; omission preserves local completion. Modern Atomic done remains pending independent review.
        state: Option<Completion>,
        /// Current check reports; omission means an empty set, not a partial edit.
        #[serde(default)]
        checks: Vec<CheckInput>,
        /// In-scope gaps; defaults empty and blocks acceptance when nonempty.
        #[serde(default)]
        gaps: Vec<String>,
        /// Future followups outside current acceptance; defaults empty.
        #[serde(default)]
        followups: Vec<String>,
        /// Reported commit/PR/other artifact references; defaults empty.
        #[serde(default)]
        artifacts: Vec<String>,
    },
    /// Set a top-level Epic/Module/standalone Atomic blocker; embedded work uses its Module blocker.
    Blocker {
        /// What prevents progress.
        problem: String,
        /// Concrete action needed to unblock.
        needed_action: String,
        /// Optional responsible person/agent.
        resolver: Option<String>,
    },
    /// Set top-level resume guidance; handoff does not invalidate acceptance.
    Handoff {
        /// Where the lead stopped.
        stopping_point: String,
        /// Concrete next action.
        next_action: String,
    },
    /// Clear a top-level blocker; absent blocker is a no-op.
    ClearBlocker {
        /// Required explanation for clearing.
        reason: String,
    },
    /// Clear top-level handoff; absence is a no-op and does not change acceptance.
    ClearHandoff {
        /// Required explanation for clearing.
        reason: String,
    },
    /// Cancel a target; module children must already be terminal.
    Cancel {
        /// Required cancellation reason, retained across reopen.
        reason: String,
    },
    /// Explicitly reopen the same target and invalidate earlier module approval.
    Reopen {
        /// Required explanation, recorded with the reopen.
        reason: String,
    },
}

/// Review check update; it preserves before/after report provenance in review history.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ReviewCheck {
    /// Reviewed Epic/Module itself or a Module-owned Task/Atomic; updates never cross owner files.
    pub target: String,
    /// Target-local check label.
    pub label: String,
    /// Reviewer's explicit outcome.
    pub status: CheckStatus,
    /// Optional explanation.
    pub detail: Option<String>,
}

/// Independent whole-module verdict. It does not review tasks separately or merge code.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ReviewArgs {
    /// Configured project alias.
    pub project: String,
    /// Target M-001.
    pub module: String,
    /// Current whole-module version from get_context.
    pub version: String,
    /// accepted or changes_requested; a saved negative verdict is not a tool error.
    pub verdict: Verdict,
    /// Meaningful review conclusion, at most 1024 UTF-8 bytes; core whole-Module reviews require a submission.
    pub summary: String,
    /// At most eight findings; accepted forbids must_fix findings.
    #[serde(default)]
    pub findings: Vec<Finding>,
    /// At most eight updates to current canonical check reports.
    #[serde(default)]
    pub checks: Vec<ReviewCheck>,
    /// Declared reviewer identity; unknown remains unknown, never authenticated.
    pub actor: Option<String>,
    /// Explicit follow-up scope; empty for initial whole review.
    #[serde(default)]
    pub changed_scope: Vec<String>,
    /// Independently resolved retained finding refs, zero-based.
    #[serde(default)]
    pub resolved_findings: Vec<FindingResolution>,
}

/// Independent whole-Epic/Module and standalone/embedded Atomic review; Tasks have no review.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ReviewWorkArgs {
    /// Configured portable root alias.
    pub project: String,
    /// E-001, M-001, A-001 or M-001/A-001; individual Task references refuse.
    #[serde(rename = "ref")]
    pub reference: String,
    /// Exact current owning record Version, including Epic child observations.
    pub version: String,
    /// Accepted or changes_requested.
    pub verdict: Verdict,
    /// Meaningful reviewer conclusion.
    pub summary: String,
    /// At most eight retained findings.
    #[serde(default)]
    pub findings: Vec<Finding>,
    /// Owner or embedded child canonical check updates; never writes another file.
    #[serde(default)]
    pub checks: Vec<ReviewCheck>,
    /// Declared independent reviewer; unknown remains unknown.
    pub actor: Option<String>,
    /// Explicit follow-up scope, eight 256-byte refs.
    #[serde(default)]
    pub changed_scope: Vec<String>,
    /// Stable zero-based prior finding refs in this target's own history, with verified resolutions.
    #[serde(default)]
    pub resolved_findings: Vec<FindingResolution>,
}

/// Common mutation fields removed before decoding the closed operation variant.
pub struct Common {
    /// Chosen project alias.
    pub project: String,
    /// Exact expected file/allocation version.
    pub version: String,
    /// Declared actor; omission/null means unknown.
    pub actor: Option<String>,
    /// record_work target; absent on plan_work.
    pub reference: Option<String>,
}

/// Decode a supplied required-field edit; null is invalid, omission is handled by serde default.
fn required_patch<'de, T: Deserialize<'de>, D: Deserializer<'de>>(
    d: D,
) -> std::result::Result<Patch<T>, D::Error> {
    T::deserialize(d).map(Patch::Set)
}

/// Reduce a serde decoding failure to one bounded message that names at most one schema field.
///
/// Unknown-field and missing-field failures name the field (an unknown key is reduced to ASCII
/// letters, digits and underscores and cut at 64 bytes); an unknown operation says so; every
/// other failure is one generic shape message because serde exposes no field path without a
/// new dependency. Supplied values never enter the text.
pub fn shape_error(error: &serde_json::Error) -> String {
    let text = error.to_string();
    let name = |prefix: &str| {
        text.strip_prefix(prefix)
            .and_then(|rest| rest.split('`').next())
            .map(|raw| {
                raw.chars()
                    .filter(|c| c.is_ascii_alphanumeric() || *c == '_')
                    .take(64)
                    .collect::<String>()
            })
            .filter(|clean| !clean.is_empty())
    };
    if let Some(field) = name("unknown field `") {
        format!("Unknown field \"{field}\". Read the tool's input contract.")
    } else if let Some(field) = name("missing field `") {
        format!("Missing required field \"{field}\".")
    } else if text.starts_with("unknown variant") {
        "Unknown operation. Read the tool's input contract.".into()
    } else {
        "Invalid argument shape or type; field type details are unavailable. Read the tool's input contract."
            .into()
    }
}

/// Name the offending field in a bounded validation failure; the supplied value is never echoed.
///
/// `checked` is the result of a model validator such as `text` or `strings`, whose message states
/// the rule and its limit. A failure becomes `invalid_arguments` reading `<name>: <rule>`.
pub fn field<T>(name: &str, checked: Result<T, String>) -> crate::store::Result<T> {
    checked.map_err(|rule| crate::store::Error::new("invalid_arguments", format!("{name}: {rule}")))
}

/// Decode flat common fields and the closed operation; no arbitrary patch data survives.
pub fn mutation<T: serde::de::DeserializeOwned>(
    value: Value,
    record: bool,
) -> Result<(Common, T), String> {
    let Value::Object(mut args) = value else {
        return Err("Expected an argument object.".into());
    };
    let project = serde_json::from_value(args.remove("project").ok_or("Missing project alias.")?)
        .map_err(|_| "project must be text.")?;
    let version = serde_json::from_value(
        args.remove("version")
            .ok_or("Missing version; call get_context first.")?,
    )
    .map_err(|_| "version must be text.")?;
    let actor = serde_json::from_value(args.remove("actor").unwrap_or(Value::Null))
        .map_err(|_| "actor must be text or null.")?;
    let reference = if record {
        Some(
            serde_json::from_value(args.remove("ref").ok_or("Missing work ref.")?)
                .map_err(|_| "ref must be text.")?,
        )
    } else {
        None
    };
    let operation =
        serde_json::from_value(Value::Object(args)).map_err(|error| shape_error(&error))?;
    Ok((
        Common {
            project,
            version,
            actor,
            reference,
        },
        operation,
    ))
}

/// Generate closed serde variants with flat common fields and the MCP-required root object type.
pub fn mutation_schema<T: JsonSchema>(record: bool) -> Value {
    let mut schema = serde_json::to_value(schemars::schema_for!(T)).unwrap_or(Value::Null);
    schema["type"] = Value::String("object".into());
    if let Some(variants) = schema.get_mut("oneOf").and_then(Value::as_array_mut) {
        for variant in variants {
            if let Some(props) = variant.get_mut("properties").and_then(Value::as_object_mut) {
                props.insert("project".into(), serde_json::json!({"type":"string","description":"Configured project alias, not a filesystem path."}));
                props.insert("version".into(), serde_json::json!({"type":"string","description":"Exact manifest/module or allocation version returned by get_context."}));
                props.insert("actor".into(), serde_json::json!({"type":["string","null"],"description":"Declared identity; omitted/null remains unknown."}));
                if record {
                    props.insert("ref".into(), serde_json::json!({"type":"string","description":"Module or owned task reference."}));
                }
            }
            if let Some(required) = variant.get_mut("required").and_then(Value::as_array_mut) {
                required.extend(["project", "version"].map(|s| Value::String(s.into())));
                if record {
                    required.push(Value::String("ref".into()));
                }
            }
        }
    }
    schema
}
