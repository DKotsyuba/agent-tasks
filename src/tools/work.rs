//! Six business tools: planning, current reports, independent review and bounded retrieval.
use super::input::{self, Common, Completion, Plan, ReviewArgs, TaskInput, Work};
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
}
/// Mutation template input; the receipt remains separate from the effects ledger.
#[derive(Serialize)]
struct Saved<'a> {
    /// Confirmed outcome.
    ack: &'a Ack,
    /// Exact observed side effects.
    effects: &'a [String],
}

/// Compile-time closed layouts for compact normal text, not DTO dumps.
pub fn templates() -> Vec<(&'static str, &'static str)> {
    vec![
        (
            "core_page",
            "{{ heading }}\nData coverage: {{ coverage }}; detail coverage: {{ detail_coverage }}\n{% if version %}Version: {{ version }}\nSnapshot version: {{ snapshot_version }}\n{% endif %}{% if allocation_version %}Allocation version: {{ allocation_version }}\n{% endif %}{% for line in lines %}{{ line }}\n{% endfor %}{% for row in rows %}{{ row }}\n{% endfor %}{% if next_start != none %}Next: start={{ next_start }}; version={{ snapshot_version }}; remaining={{ remaining }}. Keep the same project, ref/module, view/query and review_index.\n{% elif remaining %}{{ remaining }} detail rows omitted; narrow project_status with module=M-001.\n{% endif %}",
        ),
        (
            "core_ack",
            "{% if ack.changed %}SAVED{% else %}UNCHANGED{% endif %} {{ ack.target }} — {{ ack.phase }}\nVersion: {{ ack.version }}\n{% for effect in effects %}{{ effect }}\n{% endfor %}Next: get_context for this target; project_status for the complete tracked overview. Do not replay a lost reply blindly.\n",
        ),
        (
            "core_error",
            "ERROR {{ code }}: {{ message }}\n{% if published %}Visible publication occurred; inspect current context before another mutation. Outcome/durability may be partial.\n{% else %}No business publication confirmed by this call.\n{% endif %}{% for effect in effects %}{{ effect }}\n{% endfor %}Next: get_context to inspect current work/version; repair the reported condition before retrying.\n",
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
        "get_context",
        "project_status",
        "search",
        "plan_work",
        "record_work",
        "review_module",
    ]
    .contains(&name)
    {
        return None;
    }
    let mut effects = Vec::new();
    let result = (|| -> Result<String> {
        match name {
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
            };
            let text=templates.render("core_error",&failure).unwrap_or_else(|_| format!("ERROR {}. Presentation degraded.\n{}\nEffects: {}\nInspect get_context before retrying.\n",e.code,failure.message,if effects.is_empty(){"none confirmed".into()}else{effects.join("\n")}));
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
    Ack {
        target: target.into(),
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
    number(id, "M-").map_err(arguments)?;
    Ok(id)
}

/// Read a complete current module, enforce byte version and intact counters before any change.
fn current(store: &Store, id: &str, version: &str) -> Result<Snapshot<Module>> {
    let snapshot = store.module(id)?;
    if snapshot.version != version {
        return Err(Error::new(
            "stale",
            format!(
                "No work saved. {} is {} with current version {}. Read get_context before retrying.",
                id,
                snapshot.value.phase(),
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
        return Ok(ack(target, before.version.clone(), value.phase(), false));
    }
    if action == "reopened"
        || value.basis().map_err(store::invalid)? != before.value.basis().map_err(store::invalid)?
    {
        value.review_epoch = value
            .review_epoch
            .checked_add(1)
            .ok_or_else(|| store::invalid("Review epoch exhausted."))?;
    }
    value
        .event(target, action, &store::now(), actor)
        .map_err(store::invalid)?;
    value.validate().map_err(store::invalid)?;
    let version = store.save(
        &format!("modules/{}.yaml", value.id),
        &value,
        Some(&before.bytes),
        action == "canceled",
        effects,
    )?;
    Ok(ack(target, version, value.phase(), true))
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
            let old = store.bytes(".agent-tasks/state.yaml")?;
            let allocator = Allocator {
                schema_version: SCHEMA,
                next_module: 1,
            };
            if let Some(bytes) = &old
                && store::decode::<Allocator>(bytes)? != allocator
            {
                return Err(Error::new(
                    "partial_init",
                    "Existing allocator is inconsistent with empty initialization; restore a retained copy.",
                ));
            }
            let at = store::now();
            let project = Project {
                schema_version: SCHEMA,
                title,
                purpose,
                remote,
                created_at: at.clone(),
                updated_at: at,
            };
            project.validate().map_err(arguments)?;
            let expected_state = old.clone().unwrap_or(store::encode(&allocator)?);
            if old.is_none() {
                store.save(".agent-tasks/state.yaml", &allocator, None, false, effects)?;
            }
            let after_inventory = store.inventory()?;
            require_inventory(&after_inventory)?;
            if store.bytes("project.yaml")?.is_some()
                || store.bytes(".agent-tasks/state.yaml")?.as_deref()
                    != Some(expected_state.as_slice())
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
            expect_allocation(&store, &common.version)?;
            let project = store
                .project()?
                .ok_or_else(|| Error::new("not_initialized", "Initialize the project first."))?;
            let inventory = store.inventory()?;
            require_inventory(&inventory)?;
            if inventory.ids.len() >= store::MODULE_CAP {
                return Err(Error::new("capacity", "Module inventory is at capacity."));
            }
            let bytes = store.bytes(".agent-tasks/state.yaml")?.ok_or_else(|| {
                Error::new(
                    "allocator",
                    "Missing allocator; restore retained state before allocating.",
                )
            })?;
            let mut state: Allocator = store::decode(&bytes)?;
            let max = inventory
                .ids
                .iter()
                .filter_map(|id| number(id, "M-").ok())
                .max()
                .unwrap_or(0);
            if state.schema_version != SCHEMA
                || state.next_module <= max
                || state.next_module == u64::MAX
            {
                return Err(Error::new(
                    "allocator",
                    "Invalid allocator or exhausted IDs; no guessing or reuse.",
                ));
            }
            if tasks.len() > MAX_TASKS {
                return Err(arguments("At most 32 initial tasks are allowed."));
            }
            let id = format!("M-{:03}", state.next_module);
            let at = store::now();
            let mut module = Module {
                schema_version: SCHEMA,
                id: id.clone(),
                title,
                outcome,
                lead,
                required_checks,
                state: ModuleState::Open,
                next_task: Some(tasks.len() as u64 + 1),
                next_log: Some(1),
                omitted_log_entries: 0,
                review_epoch: 0,
                tasks: tasks
                    .into_iter()
                    .enumerate()
                    .map(|(i, t)| make_task(t, i as u64 + 1, &at))
                    .collect(),
                result: None,
                checks: Vec::new(),
                blocker: None,
                handoff: None,
                cancellation: None,
                cancellation_history: Vec::new(),
                reviews: Vec::new(),
                reasons: Vec::new(),
                log: Vec::new(),
                created_at: at.clone(),
                updated_at: at.clone(),
            };
            module
                .event(&id, "created", &at, &common.actor)
                .map_err(arguments)?;
            module.validate().map_err(arguments)?;
            if store::encode(&module)?.len() > store::RECORD_CAP - store::CLOSING_RESERVE {
                return Err(Error::new(
                    "capacity",
                    "Initial module exceeds its capacity.",
                ));
            }
            state.next_module += 1;
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
                    "Scope changed after reservation; reserved number remains a gap. Inspect context.",
                ));
            }
            let version =
                store.save(&format!("modules/{id}.yaml"), &module, None, false, effects)?;
            Ok(ack(id, version, module.phase(), true))
        }
        Plan::EditModule {
            module,
            title,
            outcome,
            lead,
            required_checks,
        } => {
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
            let task = &mut value.tasks[i];
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
            if *task != before.value.tasks[i] {
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
    if index.is_some_and(|i| value.tasks[i].state == TaskState::Canceled) && !reopening {
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
                let task = &mut value.tasks[i];
                task.result = Some(report);
                task.checks = current_checks;
                if let Some(state) = state {
                    task.state = match state {
                        Completion::Open => TaskState::Open,
                        Completion::Done => TaskState::Done,
                    };
                }
            } else {
                if state.is_some() {
                    return Err(arguments(
                        "state is only supported for task results; modules use independent review.",
                    ));
                }
                value.result = Some(report);
                value.checks = current_checks;
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
                return Ok(ack(reference, before.version, value.phase(), false));
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
                return Ok(ack(reference, before.version, value.phase(), false));
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
                value.tasks[i].state = TaskState::Canceled;
                value.tasks[i].cancellation = Some(cancellation);
            } else {
                if value.tasks.iter().any(|t| t.state == TaskState::Open) {
                    return Err(Error::new(
                        "open_children",
                        "Complete or cancel children explicitly before canceling their module.",
                    ));
                }
                value.state = ModuleState::Canceled;
                value.cancellation = Some(cancellation);
            }
            "canceled"
        }
        Work::Reopen { reason } => {
            text(&reason, 512).map_err(arguments)?;
            if let Some(i) = index {
                let task = &mut value.tasks[i];
                task.state = TaskState::Open;
                if let Some(c) = task.cancellation.take() {
                    task.cancellation_history.push(c);
                }
            } else {
                value.state = ModuleState::Open;
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
        value.tasks[i].updated_at = at;
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
        if index.is_some_and(|i| value.tasks[i].state == TaskState::Canceled) {
            return Err(Error::new(
                "canceled",
                "Review cannot change checks on canceled tasks.",
            ));
        }
        let checks = if let Some(i) = index {
            &mut value.tasks[i].checks
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
        let missing = value.acceptance();
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
    let basis = value.basis().map_err(store::invalid)?;
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
    let version = store.save(
        &format!("modules/{}.yaml", args.module),
        &value,
        Some(&before.bytes),
        args.verdict == Verdict::Accepted,
        effects,
    )?;
    Ok(ack(args.module, version, value.phase(), true))
}
