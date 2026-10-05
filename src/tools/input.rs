//! Closed semantic arguments. Presence-aware edits never erase omitted fields.
use crate::model::{CheckInput, CheckStatus, Finding, Lead, Verdict};
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
    /// Embedded tasks and their local state.
    Tasks,
    /// Current outcome details and reported artifacts.
    Results,
    /// Required and reported check evidence.
    Checks,
    /// One retained review with pageable findings and check changes.
    Review,
    /// Recent generated activity and explicit omitted history.
    Log,
}

/// Read one project/module/task with optional snapshot-bound pagination.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ContextArgs {
    /// Configured project alias; not a path or a global current project.
    pub project: String,
    /// Omit for project; otherwise M-001 or M-001/T-001.
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
    /// Optional M-001 narrowing when aggregate detail is partial.
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
    /// Optional M-001 narrowing.
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
    /// Keep this task open.
    Open,
    /// Complete this task with its meaningful result.
    Done,
}

/// Record current substance or lifecycle, never supplied timestamps or raw patches.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(tag = "op", rename_all = "snake_case", deny_unknown_fields)]
pub enum Work {
    /// Replace the target's current complete report and check set.
    Result {
        /// Meaningful outcome, at most 1024 UTF-8 bytes.
        summary: String,
        /// Tasks only; omission preserves current open/done state.
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
    /// Set an actionable module-only blocker.
    Blocker {
        /// What prevents progress.
        problem: String,
        /// Concrete action needed to unblock.
        needed_action: String,
        /// Optional responsible person/agent.
        resolver: Option<String>,
    },
    /// Set module-only resume guidance; does not invalidate approval.
    Handoff {
        /// Where the lead stopped.
        stopping_point: String,
        /// Concrete next action.
        next_action: String,
    },
    /// Clear a module blocker; absent blocker is a no-op.
    ClearBlocker {
        /// Required explanation for clearing.
        reason: String,
    },
    /// Clear module handoff; absent handoff is a no-op.
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
    /// The reviewed module or one of its owned tasks.
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
    /// Meaningful review conclusion, at most 1024 UTF-8 bytes.
    pub summary: String,
    /// At most eight findings; accepted forbids must_fix findings.
    #[serde(default)]
    pub findings: Vec<Finding>,
    /// At most eight updates to current canonical check reports.
    #[serde(default)]
    pub checks: Vec<ReviewCheck>,
    /// Declared reviewer identity; unknown remains unknown, never authenticated.
    pub actor: Option<String>,
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
    let operation = serde_json::from_value(Value::Object(args))
        .map_err(|_| "Unknown operation, field, missing value or invalid argument shape.")?;
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
