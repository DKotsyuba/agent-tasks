//! Purpose-based work tools: guarded planning, current evidence, whole-record review and bounded retrieval.
use super::input::{self, Common, Completion, Plan, ReviewArgs, ReviewWorkArgs, TaskInput, Work};
use crate::{
    model::*,
    response::Templates,
    store::{self, Config, Error, Result, Snapshot, Store},
};
use rmcp::model::{CallToolResult, ContentBlock};
use serde::Serialize;
use serde_json::Value;

/// Confirmed mutation receipt, captured before presentation. It never replays an effect.
#[derive(Serialize)]
pub struct Ack {
    /// Target or project reference.
    pub target: String,
    /// Current file or creation precondition version.
    pub version: String,
    /// Derived current phase, never externally verified delivery.
    pub phase: String,
    /// Owning record phase label; embedded children always report their Module phase.
    pub phase_label: &'static str,
    /// Whether the call changed any business record.
    pub changed: bool,
}
/// Typed compact semantic projection; raw storage structs never enter templates.
#[derive(Clone, Serialize)]
pub struct Page {
    /// Safe human-facing heading.
    pub heading: String,
    /// Bounded scope/evidence/conditions lines.
    pub lines: Vec<String>,
    /// Pageable semantic rows, never serialized YAML.
    pub rows: Vec<String>,
    /// Editable file version; empty on owner-facing status.
    pub version: String,
    /// Exact view/scope snapshot for pagination, distinct from a manifest edit.
    pub snapshot_version: String,
    /// Exact creation precondition, only on project context.
    pub allocation_version: Option<String>,
    /// Complete or PARTIAL data coverage.
    pub coverage: String,
    /// Whether all readable detail fits; complete counts are retained separately.
    pub detail_coverage: String,
    /// Next actual row offset, absent at the end.
    pub next_start: Option<usize>,
    /// Omitted rows from this snapshot.
    pub remaining: usize,
}
/// Error projection with independently recorded effects, including post-write failure.
#[derive(Serialize)]
struct Failure<'a> {
    /// Classified stable code.
    code: &'a str,
    /// Bounded quoted explanation.
    message: String,
    /// Immutable execution ledger, retained even if normal rendering fails.
    effects: &'a [String],
    /// Distinguish setup-only effects from visibly published work.
    published: bool,
    /// Tool-specific inspection route; registration may not have published an alias yet.
    recovery: &'a str,
}
/// Mutation template input; the receipt remains separate from the effects ledger.
#[derive(Serialize)]
struct Saved<'a> {
    /// Confirmed outcome.
    ack: &'a Ack,
    /// Exact observed side effects.
    effects: &'a [String],
}

/// Closed text layouts preserve execution effects, scope labels and exact continuation tokens.
pub fn templates() -> Vec<(&'static str, &'static str)> {
    vec![
        (
            "project_registered",
            "{% if changed %}REGISTERED{% else %}UNCHANGED{% endif %} {{ project }} — {{ name }}\nVersion: {{ version }}\n{% for effect in effects %}{{ effect }}\n{% endfor %}Next: get_context(project=\"{{ project }}\") for orientation, or project_status for progress.\n",
        ),
        (
            "core_page",
            "{{ heading }}\nData coverage: {{ coverage }}; detail coverage: {{ detail_coverage }}\n{% if version %}Version: {{ version }}\nSnapshot version: {{ snapshot_version }}\n{% endif %}{% if allocation_version %}Allocation version: {{ allocation_version }}\n{% endif %}{% for line in lines %}{{ line }}\n{% endfor %}{% for row in rows %}{{ row }}\n{% endfor %}{% if next_start != none %}Next: start={{ next_start }}; version={{ snapshot_version }}; remaining={{ remaining }}. Keep the same tool and selection.\n{% elif remaining %}{{ remaining }} detail rows omitted; narrow project_status with module=M-001.\n{% endif %}",
        ),
        (
            "core_ack",
            "{% if ack.changed %}SAVED{% else %}UNCHANGED{% endif %} {{ ack.target }}\n{% if ack.target == \"Project\" %}Project state:{% else %}{{ ack.phase_label }} phase:{% endif %} {{ ack.phase }}\nVersion: {{ ack.version }}\n{% for effect in effects %}{{ effect }}\n{% endfor %}Next: {% if ack.target == \"Project\" %}get_context with project only; omit ref.{% else %}get_context with ref={{ ack.target }}.{% endif %} Use project_status for the complete tracked overview. Do not replay a lost reply blindly.\n",
        ),
        (
            "core_error",
            "ERROR {{ code }}: {{ message }}\n{% if published %}Visible publication occurred; outcome/durability may be partial.\n{% else %}No business publication confirmed by this call.\n{% endif %}{% for effect in effects %}{{ effect }}\n{% endfor %}Next: {{ recovery }}\n",
        ),
    ]
}

/// Route only the known business tools. Expected failures return isError=true; a
/// saved changes_requested verdict returns false. Unknown tools remain protocol errors.
pub fn call(
    name: &str,
    args: Value,
    config: &Config,
    templates: &Templates,
) -> Option<CallToolResult> {
    if ![
        "register_project",
        "get_project_list",
        "get_context",
        "project_status",
        "search",
        "plan_work",
        "record_work",
        "review_module",
        "review_work",
    ]
    .contains(&name)
    {
        return None;
    }
    let mut effects = Vec::new();
    let result = (|| -> Result<String> {
        match name {
            "register_project" => {
                super::projects::register(config, decode_args(args)?, templates, &mut effects)
            }
            "get_project_list" => super::projects::list(config, decode_args(args)?, templates),
            "get_context" => {
                let args = decode_args(args)?;
                super::read::context(config, args, templates)
            }
            "project_status" => super::read::status(config, decode_args(args)?, templates),
            "search" => super::read::search(config, decode_args(args)?, templates),
            "plan_work" => {
                let (common, operation) =
                    input::mutation::<Plan>(args, false).map_err(arguments)?;
                validate_common(&common)?;
                let ack = plan(config, &common, operation, &mut effects)?;
                Ok(render_ack(templates, &ack, &effects))
            }
            "record_work" => {
                let (common, operation) = input::mutation::<Work>(args, true).map_err(arguments)?;
                validate_common(&common)?;
                let ack = record(config, &common, operation, &mut effects)?;
                Ok(render_ack(templates, &ack, &effects))
            }
            "review_module" => {
                let args: ReviewArgs = decode_args(args)?;
                number(&args.module, "M-").map_err(arguments)?;
                let ack = review(config, args, &mut effects)?;
                Ok(render_ack(templates, &ack, &effects))
            }
            "review_work" => {
                let a: ReviewWorkArgs = decode_args(args)?;
                let args = ReviewArgs {
                    project: a.project,
                    module: a.reference,
                    version: a.version,
                    verdict: a.verdict,
                    summary: a.summary,
                    findings: a.findings,
                    checks: a.checks,
                    actor: a.actor,
                };
                let ack = review(config, args, &mut effects)?;
                Ok(render_ack(templates, &ack, &effects))
            }
            _ => Err(arguments("Unknown business tool.")),
        }
    })();
    let (text, error) = match result {
        Ok(text) => (text, false),
        Err(e) => {
            let failure = Failure {
                code: e.code,
                message: store::safe(&e.message, 600),
                effects: &effects,
                published: effects.iter().any(|e| e.starts_with("Published ")),
                recovery: match name {
                    "register_project" => {
                        "Inspect the requested documentation root and get_project_list; the alias may be unpublished. Resolve the cause before repeating the same intent."
                    }
                    "get_project_list" => {
                        "Inspect settings and projects.toml; reads do not repair or migrate them."
                    }
                    _ => {
                        "Use get_context to inspect current work/version; repair the condition before retrying."
                    }
                },
            };
            let text=templates.render("core_error",&failure).unwrap_or_else(|_| format!("ERROR {}. Presentation degraded.\n{}\nEffects: {}\nInspect current state before retrying.\n",e.code,failure.message,if effects.is_empty(){"none confirmed".into()}else{effects.join("\n")}));
            (text, true)
        }
    };
    let mut reply = CallToolResult::success(vec![ContentBlock::text(text)]);
    reply.is_error = Some(error);
    Some(reply)
}

/// Closed serde argument decoding; parser source values never enter diagnostics.
pub fn decode_args<T: serde::de::DeserializeOwned>(args: Value) -> Result<T> {
    serde_json::from_value(args).map_err(|_| arguments("Unknown field, invalid type, null or missing required argument. Read the tool's input contract."))
}
/// Classify semantic/structural arguments without dumping raw inputs.
fn arguments(message: impl Into<String>) -> Error {
    Error::new("invalid_arguments", message)
}

/// Validate generated-version preconditions and declared identity at the request boundary.
fn validate_common(common: &Common) -> Result<()> {
    text(&common.project, 128).map_err(arguments)?;
    if common.version.len() != 64 || !common.version.bytes().all(|c| c.is_ascii_hexdigit()) {
        return Err(arguments(
            "Use the exact 64-character version returned by get_context.",
        ));
    }
    optional(&common.actor, 128).map_err(arguments)
}

/// Render once after capturing the immutable receipt; a presentation failure still names the saved target/version.
fn render_ack(templates: &Templates, ack: &Ack, effects: &[String]) -> String {
    templates.render("core_ack",&Saved {ack,effects}).unwrap_or_else(|_| format!(
        "{} {}. Version: {}. Presentation degraded.\nEffects:\n{}\nInspect get_context; do not replay.\n",
        if ack.changed {"SAVED"}else{"UNCHANGED"},ack.target,ack.version,effects.join("\n")))
}

/// Build a target receipt without deriving external success.
fn ack(target: impl Into<String>, version: String, phase: impl Into<String>, changed: bool) -> Ack {
    let target = target.into();
    let phase_label = if target.starts_with("E-") {
        "Epic"
    } else if target.starts_with("A-") {
        "Atomic"
    } else {
        "Module"
    };
    Ack {
        target,
        phase_label,
        version,
        phase: phase.into(),
        changed,
    }
}

/// Allocate initial task facts; no commit, dates or counters come from the caller.
fn make_task(input: TaskInput, n: u64, at: &str) -> Task {
    Task {
        id: format!("T-{n:03}"),
        title: input.title,
        criterion: input.criterion,
        executor: None,
        required_checks: input.required_checks,
        state: TaskState::Open,
        result: None,
        checks: Vec::new(),
        cancellation: None,
        cancellation_history: Vec::new(),
        created_at: at.into(),
        updated_at: at.into(),
    }
}

/// Resolve the module portion and reject arbitrary paths or alternate numeric identities.
pub fn module_id(reference: &str) -> Result<&str> {
    let id = reference
        .split('/')
        .next()
        .ok_or_else(|| arguments("Invalid reference."))?;
    work_number(id).map_err(arguments)?;
    Ok(id)
}

/// Read a complete current module, enforce byte version and intact counters before any change.
fn current(store: &Store, id: &str, version: &str) -> Result<Snapshot<Module>> {
    store.open_parent(id)?;
    let snapshot = store.module(id)?;
    if snapshot.version != version {
        return Err(Error::new(
            "stale",
            format!(
                "No work saved. {} is {} with current version {}. Read get_context before retrying.",
                id,
                store.phase(&snapshot.value),
                snapshot.version
            ),
        ));
    }
    snapshot.value.counters().map_err(store::invalid)?;
    Ok(snapshot)
}

/// Save one module, advancing review epoch for semantic changes or explicit reopen.
/// Only the fixed canceled action may consume terminal closing headroom here;
/// accepted review uses its dedicated path. A no-op never appends a log/date.
fn save_module(
    store: &Store,
    before: &Snapshot<Module>,
    mut value: Module,
    target: &str,
    action: &str,
    actor: &Option<String>,
    effects: &mut Vec<String>,
) -> Result<Ack> {
    if value == before.value {
        return Ok(ack(
            target,
            before.version.clone(),
            store.phase(&value),
            false,
        ));
    }
    let semantic_change =
        value.basis().map_err(store::invalid)? != before.value.basis().map_err(store::invalid)?;
    if value.id.starts_with("A-") && semantic_change && action != "result reported" {
        value.completed = false;
    }
    if action == "reopened" || semantic_change {
        value.review_epoch = value
            .review_epoch
            .checked_add(1)
            .ok_or_else(|| store::invalid("Review epoch exhausted."))?;
    }
    value
        .event(target, action, &store::now(), actor)
        .map_err(store::invalid)?;
    value.validate().map_err(store::invalid)?;
    store.save(
        &work_path(&value.id).map_err(arguments)?,
        &value,
        Some(&before.bytes),
        action == "canceled",
        effects,
    )?;
    let version = store.work_version(&value, &store::encode(&value)?)?;
    Ok(ack(target, version, store.phase(&value), true))
}

/// Initialize missing manifest/allocator records under the caller's root lock.
/// Reject existing manifests, foreign modules and inconsistent partial allocators; retain every effect.
pub(super) fn initialize(
    store: &Store,
    project: Project,
    effects: &mut Vec<String>,
) -> Result<Ack> {
    if store.project()?.is_some() {
        return Err(Error::new(
            "already_initialized",
            "Project already exists; use edit_project with its manifest version.",
        ));
    }
    let inventory = store.inventory()?;
    require_inventory(&inventory)?;
    if !inventory.ids.is_empty() {
        return Err(Error::new(
            "partial_init",
            "Modules without a manifest are not an empty initialization.",
        ));
    }
    project.validate().map_err(arguments)?;
    let old = store.bytes(".agent-tasks/state.yaml")?;
    let allocator = Allocator {
        schema_version: 2,
        next_module: 1,
        next_epic: Some(1),
        next_atomic: Some(1),
    };
    if let Some(bytes) = &old {
        let mut existing: Allocator = store::decode(bytes)?;
        existing.prepare(&inventory.ids).map_err(store::invalid)?;
        if existing != allocator {
            return Err(Error::new(
                "partial_init",
                "Existing allocator is inconsistent with empty initialization; restore a retained copy.",
            ));
        }
    }
    let expected_state = old.clone().unwrap_or(store::encode(&allocator)?);
    if old.is_none() {
        store.save(".agent-tasks/state.yaml", &allocator, None, false, effects)?;
    }
    let after_inventory = store.inventory()?;
    require_inventory(&after_inventory)?;
    if store.bytes("project.yaml")?.is_some()
        || store.bytes(".agent-tasks/state.yaml")?.as_deref() != Some(expected_state.as_slice())
        || !after_inventory.ids.is_empty()
    {
        return Err(Error::new(
            "partial_init",
            "Initialization changed after allocator publication; inspect context before resuming.",
        ));
    }
    let version = store.save("project.yaml", &project, None, false, effects)?;
    Ok(ack("Project", version, "initialized", true))
}

/// Execute the closed plan operation under one root lock; creations reserve numbers before publication.
fn plan(
    config: &Config,
    common: &Common,
    operation: Plan,
    effects: &mut Vec<String>,
) -> Result<Ack> {
    let store = config.resolve(&common.project)?;
    let init = matches!(operation, Plan::InitProject { .. });
    if init {
        store.prepare(effects)?;
    }
    let _lock = store.lock(true, effects)?;
    match operation {
        Plan::InitProject {
            title,
            purpose,
            remote,
        } => {
            expect_allocation(&store, &common.version)?;
            let at = store::now();
            initialize(
                &store,
                Project {
                    schema_version: SCHEMA,
                    title,
                    purpose,
                    remote,
                    created_at: at.clone(),
                    updated_at: at,
                },
                effects,
            )
        }
        Plan::EditProject {
            title,
            purpose,
            remote,
        } => {
            let mut snapshot = store
                .project()?
                .ok_or_else(|| Error::new("not_initialized", "Initialize the project first."))?;
            store.expect("project.yaml", &common.version)?;
            let before = snapshot.value.clone();
            title
                .required(&mut snapshot.value.title)
                .map_err(arguments)?;
            purpose
                .required(&mut snapshot.value.purpose)
                .map_err(arguments)?;
            remote.optional(&mut snapshot.value.remote);
            snapshot.value.validate().map_err(arguments)?;
            if before == snapshot.value {
                return Ok(ack("Project", snapshot.version, "unchanged", false));
            }
            snapshot.value.updated_at = store::now();
            let version = store.save(
                "project.yaml",
                &snapshot.value,
                Some(&snapshot.bytes),
                false,
                effects,
            )?;
            Ok(ack("Project", version, "updated", true))
        }
        Plan::CreateModule {
            title,
            outcome,
            lead,
            required_checks,
            tasks,
        } => {
            if tasks.len() > MAX_TASKS {
                return Err(arguments("At most 32 initial tasks."));
            }
            let at = store::now();
            let mut value = new_record("M-001", title, outcome, lead, required_checks, &at);
            value.next_task = Some(tasks.len() as u64 + 1);
            value.tasks = tasks
                .into_iter()
                .enumerate()
                .map(|(i, t)| make_task(t, i as u64 + 1, &at))
                .collect();
            create_record(&store, common, value, effects)
        }
        Plan::CreateEpic {
            title,
            outcome,
            criteria,
            lead,
            required_checks,
        } => {
            let mut value = new_record(
                "E-001",
                title,
                outcome,
                lead,
                required_checks,
                &store::now(),
            );
            value.criteria = criteria;
            create_record(&store, common, value, effects)
        }
        Plan::CreateAtomic {
            title,
            outcome,
            executor,
            required_checks,
            participants,
        } => {
            let mut value = new_record(
                "A-001",
                title,
                outcome,
                executor,
                required_checks,
                &store::now(),
            );
            value.participants = participants;
            value.validate().map_err(arguments)?;
            store.participant_basis(&value.participants)?;
            create_record(&store, common, value, effects)
        }
        Plan::EditEpic {
            epic,
            title,
            outcome,
            criteria,
            lead,
            required_checks,
            modules,
            atomics,
        } => {
            number(&epic, "E-").map_err(arguments)?;
            let before = current(&store, &epic, &common.version)?;
            let mut value = before.value.clone();
            open_module(&value)?;
            title.required(&mut value.title).map_err(arguments)?;
            outcome.required(&mut value.outcome).map_err(arguments)?;
            criteria.required(&mut value.criteria).map_err(arguments)?;
            lead.optional(&mut value.lead);
            required_checks
                .required(&mut value.required_checks)
                .map_err(arguments)?;
            modules.required(&mut value.modules).map_err(arguments)?;
            atomics
                .required(&mut value.atomic_members)
                .map_err(arguments)?;
            value.validate().map_err(arguments)?;
            store.membership(&value)?;
            for id in value.modules.iter().chain(&value.atomic_members) {
                if !before.value.modules.contains(id) && !before.value.atomic_members.contains(id) {
                    store.open_parent(id)?;
                }
            }
            save_module(
                &store,
                &before,
                value,
                &epic,
                "Epic plan edited",
                &common.actor,
                effects,
            )
        }
        Plan::AddAtomic {
            module,
            title,
            outcome,
            executor,
            required_checks,
        } => {
            number(&module, "M-").map_err(arguments)?;
            let before = current(&store, &module, &common.version)?;
            let mut value = before.value.clone();
            open_module(&value)?;
            if value.atomics.len() >= MAX_TASKS {
                return Err(arguments("At most 32 Module Atomics."));
            }
            let n = value
                .next_atomic
                .ok_or_else(|| store::invalid("Missing Atomic counter."))?;
            let mut atomic = make_task(
                TaskInput {
                    title,
                    criterion: Some(outcome),
                    required_checks,
                },
                n,
                &store::now(),
            );
            atomic.id = format!("A-{n:03}");
            atomic.executor = executor;
            let target = format!("{module}/{}", atomic.id);
            value.next_atomic = Some(
                n.checked_add(1)
                    .ok_or_else(|| store::invalid("Atomic counter exhausted."))?,
            );
            value.atomics.push(atomic);
            save_module(
                &store,
                &before,
                value,
                &target,
                "Atomic added",
                &common.actor,
                effects,
            )
        }
        Plan::EditAtomic {
            reference,
            title,
            outcome,
            executor,
            required_checks,
            participants,
        } => {
            let before = current(&store, module_id(&reference)?, &common.version)?;
            let mut value = before.value.clone();
            open_module(&value)?;
            let index = value.target(&reference).map_err(arguments)?;
            if let Some(i) = index {
                if !value.child(i).id.starts_with("A-") {
                    return Err(arguments("edit_atomic requires an Atomic."));
                }
                if !matches!(participants, input::Patch::Absent) {
                    return Err(arguments(
                        "Integration participants belong to standalone Atomics.",
                    ));
                }
                let atomic = value.child_mut(i);
                if atomic.state == TaskState::Canceled {
                    return Err(arguments("Reopen canceled Atomic first."));
                }
                title.required(&mut atomic.title).map_err(arguments)?;
                let mut expected = atomic
                    .criterion
                    .clone()
                    .ok_or_else(|| arguments("Atomic outcome missing."))?;
                outcome.required(&mut expected).map_err(arguments)?;
                atomic.criterion = Some(expected);
                executor.optional(&mut atomic.executor);
                required_checks
                    .required(&mut atomic.required_checks)
                    .map_err(arguments)?;
                if atomic != before.value.child(i) {
                    atomic.updated_at = store::now();
                }
            } else {
                number(&value.id, "A-").map_err(arguments)?;
                title.required(&mut value.title).map_err(arguments)?;
                outcome.required(&mut value.outcome).map_err(arguments)?;
                executor.optional(&mut value.lead);
                required_checks
                    .required(&mut value.required_checks)
                    .map_err(arguments)?;
                participants
                    .required(&mut value.participants)
                    .map_err(arguments)?;
                value.validate().map_err(arguments)?;
                store.participant_basis(&value.participants)?;
                if value.basis().map_err(arguments)? != before.value.basis().map_err(arguments)? {
                    value.completed = false;
                    value.participant_basis.clear();
                }
            }
            save_module(
                &store,
                &before,
                value,
                &reference,
                "Atomic plan edited",
                &common.actor,
                effects,
            )
        }
        Plan::EditModule {
            module,
            title,
            outcome,
            lead,
            required_checks,
        } => {
            number(&module, "M-").map_err(arguments)?;
            let before = current(&store, &module, &common.version)?;
            let mut value = before.value.clone();
            open_module(&value)?;
            title.required(&mut value.title).map_err(arguments)?;
            outcome.required(&mut value.outcome).map_err(arguments)?;
            lead.optional(&mut value.lead);
            required_checks
                .required(&mut value.required_checks)
                .map_err(arguments)?;
            save_module(
                &store,
                &before,
                value,
                &module,
                "plan edited",
                &common.actor,
                effects,
            )
        }
        Plan::AddTask {
            module,
            title,
            criterion,
            required_checks,
        } => {
            let before = current(&store, &module, &common.version)?;
            let mut value = before.value.clone();
            open_module(&value)?;
            if value.tasks.len() >= MAX_TASKS {
                return Err(Error::new(
                    "capacity",
                    "Module already has 32 tasks; use a coherent new module.",
                ));
            }
            let n = value
                .next_task
                .ok_or_else(|| store::invalid("Missing task counter."))?;
            let task = make_task(
                TaskInput {
                    title,
                    criterion,
                    required_checks,
                },
                n,
                &store::now(),
            );
            let target = format!("{module}/{}", task.id);
            value.next_task = Some(
                n.checked_add(1)
                    .ok_or_else(|| store::invalid("Task counter exhausted."))?,
            );
            value.tasks.push(task);
            save_module(
                &store,
                &before,
                value,
                &target,
                "task added",
                &common.actor,
                effects,
            )
        }
        Plan::EditTask {
            reference,
            title,
            criterion,
            required_checks,
        } => {
            let before = current(&store, module_id(&reference)?, &common.version)?;
            let mut value = before.value.clone();
            open_module(&value)?;
            let i = value
                .target(&reference)
                .map_err(arguments)?
                .ok_or_else(|| arguments("edit_task requires an owned task reference."))?;
            if !reference
                .split_once('/')
                .is_some_and(|(_, id)| id.starts_with("T-"))
            {
                return Err(arguments("edit_task requires a Task, not an Atomic."));
            }
            let task = value.child_mut(i);
            if task.state == TaskState::Canceled {
                return Err(Error::new(
                    "canceled",
                    "Reopen this task explicitly before editing it.",
                ));
            }
            title.required(&mut task.title).map_err(arguments)?;
            criterion.optional(&mut task.criterion);
            required_checks
                .required(&mut task.required_checks)
                .map_err(arguments)?;
            if task != before.value.child(i) {
                task.updated_at = store::now();
            }
            save_module(
                &store,
                &before,
                value,
                &reference,
                "task plan edited",
                &common.actor,
                effects,
            )
        }
    }
}

/// Construct the existing lightweight evidence envelope for one typed top-level work reference.
fn new_record(
    id: &str,
    title: String,
    outcome: String,
    lead: Option<Lead>,
    required_checks: Vec<String>,
    at: &str,
) -> Module {
    Module {
        schema_version: SCHEMA,
        id: id.into(),
        title,
        outcome,
        lead,
        required_checks,
        state: ModuleState::Open,
        next_task: Some(1),
        next_atomic: Some(1),
        next_log: Some(1),
        omitted_log_entries: 0,
        review_epoch: 0,
        tasks: Vec::new(),
        atomics: Vec::new(),
        criteria: Vec::new(),
        modules: Vec::new(),
        atomic_members: Vec::new(),
        participants: Vec::new(),
        participant_basis: std::collections::BTreeMap::new(),
        completed: false,
        result: None,
        checks: Vec::new(),
        blocker: None,
        handoff: None,
        cancellation: None,
        cancellation_history: Vec::new(),
        reviews: Vec::new(),
        reasons: Vec::new(),
        log: Vec::new(),
        created_at: at.into(),
        updated_at: at.into(),
    }
}

/// Reserve the next kind-specific number durably, then publish one independent no-clobber file.
/// A failed publication retains its reservation/effects; membership is always a separate Epic write.
fn create_record(
    store: &Store,
    common: &Common,
    mut value: Module,
    effects: &mut Vec<String>,
) -> Result<Ack> {
    expect_allocation(store, &common.version)?;
    let project = store
        .project()?
        .ok_or_else(|| Error::new("not_initialized", "Initialize Project first."))?;
    let inventory = store.inventory()?;
    require_inventory(&inventory)?;
    if inventory.ids.len() >= store::MODULE_CAP {
        return Err(Error::new("capacity", "Work inventory at capacity."));
    }
    let bytes = store
        .bytes(".agent-tasks/state.yaml")?
        .ok_or_else(|| Error::new("allocator", "Missing allocator; restore retained state."))?;
    let mut state: Allocator = store::decode(&bytes)?;
    state.prepare(&inventory.ids).map_err(store::invalid)?;
    let prefix = work_number(&value.id).map_err(arguments)?.0;
    let counter = match prefix {
        "E-" => state.next_epic.as_mut(),
        "A-" => state.next_atomic.as_mut(),
        _ => Some(&mut state.next_module),
    }
    .ok_or_else(|| store::invalid("Missing counter."))?;
    value.id = format!("{prefix}{counter:03}");
    *counter += 1;
    let id = value.id.clone();
    value
        .event(&id, "created", &store::now(), &common.actor)
        .map_err(arguments)?;
    value.validate().map_err(arguments)?;
    if store::encode(&value)?.len() > store::RECORD_CAP - store::CLOSING_RESERVE {
        return Err(Error::new("capacity", "Initial record exceeds capacity."));
    }
    effects.push(format!(
        "Creation target {id}; attachment is a separate Epic write."
    ));
    store.prepare(effects)?;
    store.save(
        ".agent-tasks/state.yaml",
        &state,
        Some(&bytes),
        false,
        effects,
    )?;
    let after = store.inventory()?;
    if store.bytes("project.yaml")?.as_deref() != Some(&project.bytes)
        || after.ids != inventory.ids
        || after.warnings != inventory.warnings
        || !after.complete
        || store.bytes(".agent-tasks/state.yaml")?.as_deref()
            != Some(store::encode(&state)?.as_slice())
    {
        return Err(Error::new(
            "allocation_changed",
            format!(
                "Scope changed after reservation of {id}; reserved number remains a gap. Inspect context."
            ),
        ));
    }
    store.save(
        &work_path(&id).map_err(arguments)?,
        &value,
        None,
        false,
        effects,
    )?;
    let version = store.work_version(&value, &store::encode(&value)?)?;
    Ok(ack(id, version, store.phase(&value), true))
}

/// Compare only the creation snapshot under lock; not an idempotency certificate.
fn expect_allocation(store: &Store, expected: &str) -> Result<()> {
    let current = store.allocation_version()?;
    if current != expected {
        return Err(Error::new(
            "stale",
            format!(
                "No new work allocated. Current allocation version: {current}. Read project context before retrying."
            ),
        ));
    }
    Ok(())
}

/// Unknown/nonregular/over-limit inventories refuse allocation without blocking healthy reads.
fn require_inventory(inventory: &store::Inventory) -> Result<()> {
    if !inventory.complete {
        return Err(Error::new(
            "inventory",
            "Incomplete or foreign module inventory; inspect project context before allocating.",
        ));
    }
    Ok(())
}
/// Canceled parents accept only explicit reopen and never cascade child changes.
fn open_module(value: &Module) -> Result<()> {
    if value.state == ModuleState::Canceled {
        return Err(Error::new(
            "canceled",
            "Reopen the module explicitly before changing it or its tasks.",
        ));
    }
    Ok(())
}

/// Apply reported current substance/lifecycle under the whole-module version.
fn record(
    config: &Config,
    common: &Common,
    operation: Work,
    effects: &mut Vec<String>,
) -> Result<Ack> {
    let store = config.resolve(&common.project)?;
    let _lock = store.lock(true, effects)?;
    let reference = common
        .reference
        .as_deref()
        .ok_or_else(|| arguments("Missing target."))?;
    let before = current(&store, module_id(reference)?, &common.version)?;
    let mut value = before.value.clone();
    let index = value.target(reference).map_err(arguments)?;
    let reopening = matches!(operation, Work::Reopen { .. });
    if !reopening || index.is_some() {
        open_module(&value)?;
    }
    if index.is_some_and(|i| value.child_mut(i).state == TaskState::Canceled) && !reopening {
        return Err(Error::new(
            "canceled",
            "Reopen this canceled task before changing it.",
        ));
    }
    let at = store::now();
    let action = match operation {
        Work::Result {
            summary,
            state,
            checks,
            gaps,
            followups,
            artifacts,
        } => {
            let report = Report {
                summary,
                gaps,
                followups,
                artifacts,
                reported_at: at.clone(),
                actor: common.actor.clone(),
            };
            let current_checks = checks
                .into_iter()
                .map(|c| Check::from_input(c, &at, &common.actor))
                .collect();
            if let Some(i) = index {
                let task = value.child_mut(i);
                task.result = Some(report);
                task.checks = current_checks;
                if let Some(state) = state {
                    task.state = match state {
                        Completion::Open => TaskState::Open,
                        Completion::Done => TaskState::Done,
                    };
                }
            } else {
                if state.is_some() && !value.id.starts_with("A-") {
                    return Err(arguments(
                        "state belongs only to Task/Atomic results; Modules/Epics use independent review.",
                    ));
                }
                value.result = Some(report);
                value.checks = current_checks;
                if value.id.starts_with("A-") {
                    if let Some(state) = state {
                        value.completed = matches!(state, Completion::Done);
                    }
                    value.participant_basis = store.participant_basis(&value.participants)?;
                    if value.completed {
                        let missing = store.acceptance(&value);
                        if !missing.is_empty() {
                            return Err(Error::new(
                                "acceptance",
                                missing.into_iter().take(3).collect::<Vec<_>>().join(" "),
                            ));
                        }
                    }
                }
            }
            "result reported"
        }
        Work::Blocker {
            problem,
            needed_action,
            resolver,
        } => {
            module_only(index)?;
            value.blocker = Some(Blocker {
                problem,
                needed_action,
                resolver,
            });
            "blocker set"
        }
        Work::Handoff {
            stopping_point,
            next_action,
        } => {
            module_only(index)?;
            value.handoff = Some(Handoff {
                stopping_point,
                next_action,
            });
            "handoff set"
        }
        Work::ClearBlocker { reason } => {
            module_only(index)?;
            text(&reason, 512).map_err(arguments)?;
            if value.blocker.take().is_none() {
                return Ok(ack(reference, before.version, store.phase(&value), false));
            }
            value.reasons.push(Reason {
                target: reference.into(),
                action: "blocker cleared".into(),
                reason,
                at: at.clone(),
                actor: common.actor.clone(),
            });
            "blocker cleared"
        }
        Work::ClearHandoff { reason } => {
            module_only(index)?;
            text(&reason, 512).map_err(arguments)?;
            if value.handoff.take().is_none() {
                return Ok(ack(reference, before.version, store.phase(&value), false));
            }
            value.reasons.push(Reason {
                target: reference.into(),
                action: "handoff cleared".into(),
                reason,
                at: at.clone(),
                actor: common.actor.clone(),
            });
            "handoff cleared"
        }
        Work::Cancel { reason } => {
            text(&reason, 512).map_err(arguments)?;
            let cancellation = Cancellation {
                reason,
                at: at.clone(),
                actor: common.actor.clone(),
            };
            if let Some(i) = index {
                value.child_mut(i).state = TaskState::Canceled;
                value.child_mut(i).cancellation = Some(cancellation);
            } else {
                if value.children().any(|t| t.state == TaskState::Open) {
                    return Err(Error::new(
                        "open_children",
                        "Complete or cancel children explicitly before canceling their module.",
                    ));
                }
                if value.id.starts_with("E-") {
                    store.membership(&value)?;
                    for id in value.modules.iter().chain(&value.atomic_members) {
                        let child = store.module(id)?.value;
                        if !matches!(store.phase(&child), "accepted" | "done" | "canceled") {
                            return Err(Error::new(
                                "open_children",
                                format!("Resolve {id} before canceling Epic; no cascade."),
                            ));
                        }
                    }
                }
                value.state = ModuleState::Canceled;
                value.cancellation = Some(cancellation);
            }
            "canceled"
        }
        Work::Reopen { reason } => {
            text(&reason, 512).map_err(arguments)?;
            if let Some(i) = index {
                let task = value.child_mut(i);
                task.state = TaskState::Open;
                if let Some(c) = task.cancellation.take() {
                    task.cancellation_history.push(c);
                }
            } else {
                value.state = ModuleState::Open;
                value.completed = false;
                if let Some(c) = value.cancellation.take() {
                    value.cancellation_history.push(c);
                }
            }
            value.reasons.push(Reason {
                target: reference.into(),
                action: "reopened".into(),
                reason,
                at: at.clone(),
                actor: common.actor.clone(),
            });
            "reopened"
        }
    };
    if let Some(i) = index {
        value.child_mut(i).updated_at = at;
    }
    save_module(
        &store,
        &before,
        value,
        reference,
        action,
        &common.actor,
        effects,
    )
}

/// Refuse module-only blocker/handoff writes to tasks rather than silently rerouting them.
fn module_only(index: Option<usize>) -> Result<()> {
    if index.is_some() {
        return Err(arguments("This operation requires a module reference."));
    }
    Ok(())
}

/// Independently review the whole module, applying new check reports before basis/epoch capture.
fn review(config: &Config, args: ReviewArgs, effects: &mut Vec<String>) -> Result<Ack> {
    validate_common(&Common {
        project: args.project.clone(),
        version: args.version.clone(),
        actor: args.actor.clone(),
        reference: None,
    })?;
    let store = config.resolve(&args.project)?;
    let _lock = store.lock(true, effects)?;
    let (kind, _) = work_number(&args.module).map_err(arguments)?;
    if kind == "A-" {
        return Err(arguments(
            "Atomics use lightweight result completion; review belongs to Modules/Epics.",
        ));
    }
    let before = current(&store, &args.module, &args.version)?;
    let mut value = before.value.clone();
    open_module(&value)?;
    if args.actor.as_ref().is_some_and(|a| {
        value
            .lead
            .as_ref()
            .is_some_and(|l| &l.name == a || l.handle.as_ref() == Some(a))
    }) {
        return Err(Error::new(
            "self_review",
            "A known lead cannot independently review the same module.",
        ));
    }
    text(&args.summary, 1024).map_err(arguments)?;
    if args.findings.len() > 8 || args.checks.len() > 8 {
        return Err(arguments(
            "At most eight findings/check updates per verdict.",
        ));
    }
    if args.verdict == Verdict::Accepted && args.findings.iter().any(|f| f.must_fix) {
        return Err(Error::new(
            "acceptance",
            "Accepted verdict cannot contain must-fix findings.",
        ));
    }
    let at = store::now();
    let mut updates = Vec::new();
    let mut seen = std::collections::BTreeSet::new();
    for input in args.checks {
        if !seen.insert((input.target.clone(), input.label.clone())) {
            return Err(arguments("Duplicate review check update."));
        }
        let index = value.target(&input.target).map_err(arguments)?;
        if index.is_some_and(|i| value.child(i).state == TaskState::Canceled) {
            return Err(Error::new(
                "canceled",
                "Review cannot change checks on canceled tasks.",
            ));
        }
        let checks = if let Some(i) = index {
            &mut value.child_mut(i).checks
        } else {
            &mut value.checks
        };
        let after = Check::from_input(
            CheckInput {
                label: input.label.clone(),
                status: input.status,
                detail: input.detail,
            },
            &at,
            &args.actor,
        );
        let previous = checks.iter().position(|c| c.label == input.label);
        let original = previous.map(|i| checks[i].clone());
        if let Some(i) = previous {
            checks[i] = after.clone();
        } else {
            checks.push(after.clone());
        }
        updates.push(CheckUpdate {
            target: input.target,
            label: input.label,
            before: original,
            after,
        });
    }
    value.validate().map_err(arguments)?;
    if args.verdict == Verdict::Accepted {
        let missing = store.acceptance(&value);
        if !missing.is_empty() {
            return Err(Error::new(
                "acceptance",
                format!(
                    "{} Read get_context for remaining conditions.",
                    missing.into_iter().take(3).collect::<Vec<_>>().join(" ")
                ),
            ));
        }
    }
    value.review_epoch = value
        .review_epoch
        .checked_add(1)
        .ok_or_else(|| store::invalid("Review epoch exhausted."))?;
    let basis = store.work_basis(&value)?;
    value.reviews.push(Review {
        verdict: args.verdict,
        summary: args.summary,
        findings: args.findings,
        basis,
        epoch: value.review_epoch,
        at: at.clone(),
        reviewer: args.actor.clone(),
        check_updates: updates,
    });
    value
        .event(&args.module, "review recorded", &at, &args.actor)
        .map_err(store::invalid)?;
    value.validate().map_err(arguments)?;
    store.save(
        &work_path(&args.module).map_err(arguments)?,
        &value,
        Some(&before.bytes),
        args.verdict == Verdict::Accepted,
        effects,
    )?;
    let version = store.work_version(&value, &store::encode(&value)?)?;
    Ok(ack(args.module, version, store.phase(&value), true))
}
