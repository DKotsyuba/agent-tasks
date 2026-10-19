//! On-demand Git-derived progress: planning, immutable inputs and durable Agent Run admission.
pub mod bridge;
pub mod error;
pub mod git;
pub mod report;
pub mod run;
pub mod state;
use error::{Error, Result};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use state::{Archive, Bundle, Locked, Outcome, Pending, Plan, Report};
use std::{fs, io::Read, path::PathBuf};

/// Prompt semantics included in cache identities; bump after changing inference rules.
const PROMPT_REVISION: u32 = 5;
/// Maximum complete encoded socket request, leaving room below Agent Run's 1 MiB wire cap.
const REQUEST_CAP: usize = 900 * 1024;

/// Lazy settings location; discovery never accesses configuration or repository files.
#[derive(Clone)]
pub struct Config {
    path: Option<PathBuf>,
}
/// Nonsecret settings used only when an owner asks for business operations.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Settings {
    /// Owner-facing report language and bounded optional style instructions.
    #[serde(default)]
    summary: report::SummaryOptions,
    /// Optional dedicated Bridge producer; absent retains legacy explicit result reads.
    bridge: Option<bridge::Settings>,
    /// Optional explicit observer home, always separate from legacy records below it.
    home: Option<PathBuf>,
    /// Existing local Agent Run socket, never a new listener.
    socket: Option<PathBuf>,
    /// Approved configured provider; no automatic paid fallback.
    provider: Option<String>,
    /// Configured utility model, subject to semantic qualification.
    model: Option<String>,
    /// Installed read-only summarizer profile.
    profile: Option<String>,
    /// Legacy-compatible settings envelope version; no legacy aliases are loaded.
    schema_version: Option<u32>,
}
/// Resolved nonsecret execution settings.
struct Effective {
    /// Validated presentation preferences frozen with each new analysis.
    summary: report::SummaryOptions,
    /// Optional frozen delivery-only producer configuration.
    bridge: Option<bridge::Settings>,
    /// Local observer storage root.
    home: PathBuf,
    /// Existing broker address.
    socket: PathBuf,
    /// Approved provider name.
    provider: String,
    /// Approved model name.
    model: String,
    /// Read-only analysis profile name.
    profile: String,
}
impl Config {
    /// Capture an optional settings path without I/O.
    pub fn new(path: Option<PathBuf>) -> Self {
        Self { path }
    }
    /// Read bounded optional configuration; invalid existing settings never become defaults.
    fn resolve(&self) -> Result<Effective> {
        let user_home = std::env::var_os("HOME").map(PathBuf::from).ok_or_else(|| {
            Error::new(
                "configuration",
                "HOME is unavailable; provide a normal process environment.",
            )
        })?;
        let path = self
            .path
            .clone()
            .or_else(|| std::env::var_os("AGENT_TASKS_CONFIG").map(PathBuf::from))
            .unwrap_or_else(|| user_home.join(".agent-tasks/config.toml"));
        let settings = match fs::File::open(&path) {
            Ok(f) => {
                let mut bytes = Vec::new();
                f.take(65537).read_to_end(&mut bytes)?;
                if bytes.len() > 65536 {
                    return Err(Error::new(
                        "configuration",
                        "Observer settings exceed the size limit.",
                    ));
                }
                let text = std::str::from_utf8(&bytes)
                    .map_err(|_| Error::new("configuration", "Observer settings are not UTF-8."))?;
                toml::from_str::<Settings>(text)
                    .map_err(|_| Error::new("configuration", "Invalid observer settings."))?
            }
            Err(e)
                if e.kind() == std::io::ErrorKind::NotFound
                    && self.path.is_none()
                    && std::env::var_os("AGENT_TASKS_CONFIG").is_none() =>
            {
                Settings {
                    summary: report::SummaryOptions::default(),
                    bridge: None,
                    home: None,
                    socket: None,
                    provider: None,
                    model: None,
                    profile: None,
                    schema_version: None,
                }
            }
            Err(e) => return Err(e.into()),
        };
        if settings.schema_version.is_some_and(|v| v != 1) {
            return Err(Error::new(
                "configuration",
                "Unsupported observer settings version.",
            ));
        }
        settings.summary.validate()?;
        let result = Effective {
            summary: settings.summary,
            bridge: settings.bridge,
            home: settings
                .home
                .unwrap_or_else(|| user_home.join(".agent-tasks")),
            socket: settings
                .socket
                .unwrap_or_else(|| user_home.join(".agent-run/api.sock")),
            provider: settings.provider.unwrap_or_else(|| "opencode-zen".into()),
            model: settings
                .model
                .unwrap_or_else(|| "longcat-2.5-preview-free".into()),
            profile: settings
                .profile
                .unwrap_or_else(|| "progress-summary".into()),
        };
        if !result.home.is_absolute()
            || !result.socket.is_absolute()
            || [&result.provider, &result.model, &result.profile]
                .iter()
                .any(|s| s.trim().is_empty() || s.len() > 128 || s.chars().any(char::is_control))
        {
            return Err(Error::new(
                "configuration",
                "Observer settings require absolute paths and valid configured route names.",
            ));
        }
        Ok(result)
    }
}

/// Explicit planning action; status computation never invokes save.
#[derive(Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum PlanOperation {
    /// Read the agreed plan.
    Get,
    /// Save an owner-agreed scope revision.
    Save,
}
/// New text or an explicit retained outcome identity during a scope revision.
#[derive(Deserialize, JsonSchema)]
#[serde(untagged)]
pub enum OutcomeInput {
    /// Allocate an internal identity for new agreed text.
    Text(String),
    /// Retain the identity of a current outcome while editing its text.
    Existing {
        /// Current internal outcome identity.
        id: String,
        /// Revised owner-agreed result.
        text: String,
    },
}
/// Closed plan input; all maintenance occurs during owner-authorized planning.
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PlanArgs {
    /// Actual local repository or linked worktree root.
    pub repository: PathBuf,
    /// Read or save.
    pub op: PlanOperation,
    /// Agreed goal for save; required and nonblank there.
    pub goal: Option<String>,
    /// Flat agreed results for save; texts or retained IDs with new text.
    pub outcomes: Option<Vec<OutcomeInput>>,
    /// Current revision required when replacing an existing agreed plan.
    pub expected_revision: Option<u64>,
    /// Concrete integration branch/ref when automatic Git discovery is ambiguous.
    pub integration_ref: Option<String>,
    /// Archive the old agreed plan and capture a new baseline.
    #[serde(default)]
    pub new_plan: bool,
}
/// Actual originating host context, supplied by that caller rather than server environment.
#[derive(Clone, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Caller {
    /// Agent Run host transport (initial qualified integration: codex_queue).
    pub transport: String,
    /// Observed native chat identity.
    pub external_session_id: String,
    /// Optional native turn identity, never fabricated.
    pub external_turn_id: Option<String>,
}
/// Output detail selection, pinned to the same immutable bundle.
#[derive(Default, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum View {
    /// Short Russian owner summary.
    #[default]
    Compact,
    /// Original bounded source evidence and validated rows.
    Details,
}
/// One owner-triggered status request or an explicit pinned result read.
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct StatusArgs {
    /// Actual local repository/worktree root.
    pub repository: PathBuf,
    /// Stable returned analysis handle; reads never launch another analysis.
    pub analysis_id: Option<String>,
    /// Compact by default; details require an explicit request.
    #[serde(default)]
    pub view: View,
    /// Actual caller metadata, optional when notifications are unavailable.
    pub caller_context: Option<Caller>,
    /// Exact retained source identifier; details only and requires analysis_id.
    pub evidence_id: Option<String>,
    /// Zero-based details-row continuation; later pages require the same analysis_id.
    pub start: Option<usize>,
}

/// Save/read one flat plan with CAS; old runtime aliases and hierarchy are never consulted.
pub fn plan(config: &Config, args: PlanArgs, origin: Option<&Caller>) -> Result<String> {
    let settings = config.resolve()?;
    let mut identity = git::identify(&args.repository, Some("HEAD"))?;
    let mut locked = Locked::open(&settings.home, &identity.key)?;
    if matches!(args.op, PlanOperation::Get) {
        let current = locked.state.plan.as_ref().ok_or_else(|| {
            Error::new(
                "plan_missing",
                "No agreed plan is saved for this repository.",
            )
        })?;
        return plan_text(current);
    }
    let old = locked.state.plan.clone();
    identity.integration_ref = match args.integration_ref.as_deref() {
        Some(reference) => git::identify(&args.repository, Some(reference))?.integration_ref,
        None if !args.new_plan && old.is_some() => old
            .as_ref()
            .ok_or_else(|| Error::new("plan_missing", "No saved plan."))?
            .baseline
            .repository
            .integration_ref
            .clone(),
        None => git::identify(&args.repository, None)?.integration_ref,
    };
    if old.as_ref().map(|p| p.revision) != args.expected_revision {
        return Err(Error::new(
            "stale_plan",
            "The agreed plan changed; read it before saving another revision.",
        ));
    }
    let goal = args
        .goal
        .ok_or_else(|| Error::new("invalid_arguments", "Saving a plan requires a goal."))?;
    text(&goal, 4096)?;
    let inputs = args.outcomes.ok_or_else(|| {
        Error::new(
            "invalid_arguments",
            "Saving a plan requires flat outcome texts.",
        )
    })?;
    if inputs.is_empty() || inputs.len() > 64 {
        return Err(Error::new(
            "invalid_arguments",
            "Supply 1 to 64 high-level outcomes.",
        ));
    }
    if !args.new_plan
        && old
            .as_ref()
            .is_some_and(|p| p.baseline.repository.integration_ref != identity.integration_ref)
    {
        return Err(Error::new(
            "plan_target",
            "Changing the integration target requires a genuinely new agreed plan.",
        ));
    }
    let mut outcomes = Vec::new();
    for input in inputs {
        let (id, body) = match input {
            OutcomeInput::Text(body) => {
                let retained = if args.new_plan {
                    None
                } else {
                    old.as_ref()
                        .and_then(|p| p.outcomes.iter().find(|o| o.text == body))
                };
                let id = if let Some(o) = retained {
                    o.id.clone()
                } else {
                    locked.state.next_outcome =
                        locked.state.next_outcome.checked_add(1).ok_or_else(|| {
                            Error::new("state_limit", "Outcome identities are exhausted.")
                        })?;
                    format!("o{}", locked.state.next_outcome)
                };
                (id, body)
            }
            OutcomeInput::Existing { id, text: body } => {
                if args.new_plan
                    || !old
                        .as_ref()
                        .is_some_and(|p| p.outcomes.iter().any(|o| o.id == id))
                {
                    return Err(Error::new(
                        "invalid_arguments",
                        "Retained outcome identity is not in the current agreed plan.",
                    ));
                }
                (id, body)
            }
        };
        text(&body, 512)?;
        if outcomes
            .iter()
            .any(|o: &Outcome| o.id == id || o.text == body)
        {
            return Err(Error::new(
                "invalid_arguments",
                "Duplicate agreed outcomes are not allowed.",
            ));
        }
        outcomes.push(Outcome { id, text: body });
    }
    if !args.new_plan
        && old.as_ref().is_some_and(|p| {
            p.goal == goal
                && p.outcomes
                    .iter()
                    .map(|o| (&o.id, &o.text))
                    .eq(outcomes.iter().map(|o| (&o.id, &o.text)))
        })
    {
        return plan_text(
            old.as_ref()
                .ok_or_else(|| Error::new("plan_missing", "No plan."))?,
        );
    }
    let baseline = if args.new_plan || old.is_none() {
        git::collect(&args.repository, None, Some(&identity.integration_ref))?
    } else {
        old.as_ref()
            .ok_or_else(|| Error::new("plan_missing", "No plan."))?
            .baseline
            .clone()
    };
    if args.new_plan {
        if let Some(previous) = &old {
            if locked.state.archives.len() >= 64 {
                return Err(Error::new(
                    "archive_limit",
                    "Archive bound reached; existing plans were preserved.",
                ));
            }
            let last_summary = locked
                .state
                .reports
                .last()
                .map(|r| report::compact(r, None))
                .transpose()?;
            locked.state.archives.push(Archive {
                creator_address: previous.creator_address.clone(),
                revision: previous.revision,
                goal: previous.goal.clone(),
                outcomes: previous.outcomes.clone(),
                last_summary,
            });
        }
        locked.state.reports.clear();
    }
    let old_ids = old
        .as_ref()
        .map(|p| p.outcomes.iter().map(|o| o.id.clone()).collect::<Vec<_>>())
        .unwrap_or_default();
    let revision = old
        .as_ref()
        .map_or(Some(1), |p| p.revision.checked_add(1))
        .ok_or_else(|| Error::new("state_limit", "Plan revision exhausted."))?;
    let new = Plan {
        creator_address: if !args.new_plan && old.is_some() {
            old.as_ref().and_then(|p| p.creator_address.clone())
        } else {
            origin
                .map(|caller| bridge::return_address(settings.bridge.as_ref(), caller))
                .transpose()?
        },
        revision,
        goal,
        outcomes: outcomes.clone(),
        saved_at: state::now(),
        baseline,
        added: outcomes
            .iter()
            .filter(|o| !old_ids.contains(&o.id))
            .map(|o| o.id.clone())
            .collect(),
        removed: old_ids
            .into_iter()
            .filter(|id| !outcomes.iter().any(|o| o.id == *id))
            .collect(),
    };
    let reply = plan_text(&new)?;
    locked.state.plan = Some(new);
    locked.save()?;
    Ok(reply)
}

/// Show English service labels, the original agreed text and stable outcome IDs.
/// Missing creator metadata is explicit; owner-authored plan text is never translated.
fn plan_text(plan: &Plan) -> Result<String> {
    let lines = plan
        .outcomes
        .iter()
        .map(|o| format!("{} | {}", o.id, o.text))
        .collect::<Vec<_>>()
        .join("\n");
    Ok(format!(
        "PLAN revision {}\nPlan creator: {}\n{}\n{}\n",
        plan.revision,
        plan.creator_address
            .as_deref()
            .unwrap_or("address unavailable"),
        plan.goal,
        lines
    ))
}

/// Return a cached or pending owner summary; model calls happen only on explicit fresh requests.
pub fn status(config: &Config, args: StatusArgs) -> Result<String> {
    let settings = config.resolve()?;
    let identity = git::identify(&args.repository, Some("HEAD"))?;
    let mut locked = Locked::open(&settings.home, &identity.key)?;
    let key = identity.key.clone();
    let result = (|| -> Result<String> {
        let client = run::Client::new(settings.socket.clone());
        if args.evidence_id.is_some() || args.start.is_some() {
            if !matches!(args.view, View::Details) || args.analysis_id.is_none() {
                return Err(Error::new(
                    "invalid_arguments",
                    "Detail selection requires view=details and a retained analysis_id.",
                ));
            }
            let selected = locked
                .state
                .reports
                .iter()
                .find(|r| Some(&r.analysis_id) == args.analysis_id.as_ref())
                .ok_or_else(|| {
                    Error::new(
                        "snapshot_expired",
                        "Selected report is not retained; finish its compact read first.",
                    )
                })?;
            return details(
                selected,
                args.evidence_id.as_deref(),
                args.start.unwrap_or(0),
            );
        }
        if args.analysis_id.is_none() && matches!(args.view, View::Details) {
            let index = locked.state.reports.len().checked_sub(1).ok_or_else(|| {
                Error::new(
                    "report_missing",
                    "No completed report is retained; request a compact status first.",
                )
            })?;
            return show(
                &locked.state.reports[index],
                index.checked_sub(1).map(|i| &locked.state.reports[i]),
                &args.view,
            );
        }
        if let Some(id) = &args.analysis_id {
            if let Some(index) = locked
                .state
                .reports
                .iter()
                .position(|r| r.analysis_id == *id)
            {
                return show(
                    &locked.state.reports[index],
                    index.checked_sub(1).map(|i| &locked.state.reports[i]),
                    &args.view,
                );
            }
            if locked
                .state
                .pending
                .as_ref()
                .is_some_and(|p| p.receipt.is_none() && p.terminal.is_none())
            {
                return Err(Error::new(
                    "admission_unknown",
                    "Original admission receipt is unconfirmed; request status without a handle to reconcile the same saved intent.",
                ));
            }
            if !locked
                .state
                .pending
                .as_ref()
                .is_some_and(|p| p.receipt.as_ref().is_some_and(|r| r.agent_id == *id))
            {
                return Err(Error::new(
                    "snapshot_expired",
                    "That immutable report snapshot is no longer retained.",
                ));
            }
            return settle(&mut locked, &args.view);
        }
        if locked
            .state
            .pending
            .as_ref()
            .is_some_and(|p| p.terminal.is_none())
        {
            return settle(&mut locked, &args.view);
        }
        let plan = locked.state.plan.clone().ok_or_else(|| {
            Error::new(
                "plan_missing",
                "Save an agreed plan before requesting status.",
            )
        })?;
        let excluded = locked
            .state
            .reports
            .iter()
            .map(|r| r.analysis_id.clone())
            .chain(
                locked
                    .state
                    .pending
                    .iter()
                    .filter_map(|p| p.receipt.as_ref().map(|r| r.agent_id.clone())),
            )
            .collect::<Vec<_>>();
        let developers=client.developers(&identity.common_dir,&excluded).unwrap_or_else(|e|json!({"items":[],"complete":false,"omissions":[format!("Agent Run activity unavailable ({})",e.code)]}));
        let focus: Vec<PathBuf> = developers
            .get("items")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter(|row| {
                matches!(
                    row["status"].as_str(),
                    Some("created" | "starting" | "running" | "cancelling")
                )
            })
            .filter_map(|row| row["workdir"].as_str().map(PathBuf::from))
            .collect();
        let mut snapshot = git::collect_focused(
            &args.repository,
            Some(&plan.baseline),
            Some(&plan.baseline.repository.integration_ref),
            &focus,
        )?;
        if let Some(items) = developers.get("items").and_then(Value::as_array) {
            for item in items {
                if let Some(id) = item.get("agent_id").and_then(Value::as_str) {
                    snapshot.evidence.push(git::Evidence {
                        id: format!("run:{id}"),
                        kind: "runtime".into(),
                        worktree: item
                            .get("workdir")
                            .and_then(Value::as_str)
                            .unwrap_or("")
                            .into(),
                        reference: id.into(),
                        text: serde_json::to_string(item)?,
                    });
                }
            }
        }
        let input_id = state::digest(
            &json!({"repository":identity.key,"plan_revision":plan.revision,"goal":plan.goal,"outcomes":plan.outcomes,"git":snapshot.semantic_digest,"developers":developers,"socket":settings.socket,"provider":settings.provider,"model":settings.model,"profile":settings.profile,"summary":settings.summary,"prompt":PROMPT_REVISION}),
        )?;
        if let Some(index) = locked
            .state
            .reports
            .iter()
            .position(|r| r.bundle.input_id == input_id)
        {
            return show(
                &locked.state.reports[index],
                index.checked_sub(1).map(|i| &locked.state.reports[i]),
                &args.view,
            );
        }
        if locked
            .state
            .delivery
            .as_ref()
            .is_some_and(|d| !d.accepted())
        {
            return Err(Error::new(
                "bridge_delivery_pending",
                "The original status message is retained for delivery; no new model was started.",
            ));
        }
        let bundle = Bundle {
            prompt_revision: PROMPT_REVISION,
            summary: settings.summary,
            plan,
            snapshot,
            developers,
            input_id,
        };
        let task = report::prompt(&bundle, locked.state.reports.last())?;
        locked.state.generation = locked
            .state
            .generation
            .checked_add(1)
            .ok_or_else(|| Error::new("state_limit", "Analysis generation exhausted."))?;
        let generation = locked.state.generation;
        if let Some(caller) = &args.caller_context
            && (caller.transport != "codex_queue"
                || caller.external_session_id.trim().is_empty()
                || caller.external_session_id.len() > 512)
        {
            return Err(Error::new(
                "caller_context",
                "Use actual originating Codex session metadata; do not infer it from the server.",
            ));
        }
        locked.state.delivery = settings
            .bridge
            .clone()
            .map(|options| {
                bridge::prepare(
                    options,
                    args.caller_context.as_ref(),
                    &bundle.input_id,
                    generation,
                )
            })
            .transpose()?;
        let binding = if locked.state.delivery.is_some() {
            None
        } else {
            args.caller_context.as_ref()
        };
        let request = json!({"provider":settings.provider,"model":settings.model,"profile":settings.profile,"workdir":bundle.snapshot.repository.root,"task":task,"request_id":format!("observer-{}-{}",bundle.input_id,generation),"orchestrator":binding,"explicit_finish":true,"idle_ping":"observe"});
        if serde_json::to_vec(&json!({"jsonrpc":"2.0","id":1,"method":"start","params":request}))?
            .len()
            > REQUEST_CAP
        {
            return Err(Error::new(
                "evidence_limit",
                "Collected evidence exceeds one model request; no partial completion was inferred.",
            ));
        }
        locked.state.pending = Some(Pending {
            socket: settings.socket,
            bundle,
            request,
            receipt: None,
            terminal: None,
            generation,
        });
        locked.save()?;
        settle(&mut locked, &args.view)
    })();
    bridge::after_status(&mut locked, &settings.home, &key, &result)?;
    result
}

/// Submit/reconcile the exact original intent, then make one bounded completion observation.
/// Pending instructions are always English; a completed report uses its frozen summary preferences.
fn settle(locked: &mut Locked, view: &View) -> Result<String> {
    let mut pending =
        locked.state.pending.clone().ok_or_else(|| {
            Error::new("snapshot_expired", "Pending report is no longer retained.")
        })?;
    let client = run::Client::new(pending.socket.clone());
    if let Some(code) = &pending.terminal {
        return Err(Error::new(
            code,
            "The analysis did not yield a valid report; a fresh owner status request may retry.",
        ));
    }
    if pending.receipt.is_none() {
        let receipt = match client.submit(&pending.request) {
            Ok(receipt) => receipt,
            Err(error) => {
                if matches!(
                    error.code.as_str(),
                    "ValidationError"
                        | "Unsupported"
                        | "run_request_invalid"
                        | "run_request_too_large"
                ) {
                    pending.terminal = Some(error.code.clone());
                    locked.state.pending = Some(pending);
                    locked.save()?;
                }
                return Err(error);
            }
        };
        pending.receipt = Some(receipt);
        locked.state.pending = Some(pending.clone());
        locked.save()?;
    }
    let receipt = pending.receipt.as_ref().ok_or_else(|| {
        Error::new(
            "admission_unknown",
            "Original analysis admission remains unknown.",
        )
    })?;
    match client.result(
        receipt,
        &pending.bundle.input_id,
        pending.bundle.plan.revision,
    ) {
        Ok(None) => Ok(format!(
            "PENDING {}\nThe report is being calculated from the saved snapshot.\n{}",
            receipt.agent_id,
            if locked.state.delivery.is_some() {
                "The completed report will arrive through Bridge; no follow-up call is required."
            } else if pending.request["orchestrator"].is_null() {
                "Notifications are not bound; the next explicit request will retrieve the same result.\n"
            } else {
                "After the completion notice, call status with this analysis_id; do not restart the analysis.\n"
            }
        )),
        Ok(Some(value)) => {
            let analysis = match report::validate(&pending.bundle, value) {
                Ok(v) => v,
                Err(e) => {
                    pending.terminal = Some(e.code.clone());
                    locked.state.pending = Some(pending);
                    locked.save()?;
                    return Err(e);
                }
            };
            let completed = Report {
                analysis_id: receipt.agent_id.clone(),
                bundle: pending.bundle.clone(),
                analysis,
                model_task: pending.request["task"].as_str().map(str::to_owned),
                completed_at: state::now(),
            };
            let output = show(&completed, locked.state.reports.last(), view)?;
            let compact = show(&completed, locked.state.reports.last(), &View::Compact)?;
            bridge::completed(
                &mut *locked,
                completed.bundle.plan.revision,
                &completed.bundle.snapshot.observed_at,
                &compact,
            )?;
            locked.state.pending = None;
            locked.state.reports.push(completed);
            if locked.state.reports.len() > 2 {
                locked.state.reports.remove(0);
            }
            locked.save()?;
            Ok(output)
        }
        Err(e) => {
            if matches!(
                e.code.as_str(),
                "analysis_failed"
                    | "answer_unavailable"
                    | "answer_mismatch"
                    | "analysis_invalid"
                    | "invalid_analysis"
            ) {
                pending.terminal = Some(e.code.clone());
                locked.state.pending = Some(pending);
                locked.save()?;
            }
            Err(e)
        }
    }
}

/// Render compact owner text or exact retained bounded details; never recollect current files.
fn show(report: &Report, previous: Option<&Report>, view: &View) -> Result<String> {
    match view {
        View::Compact => report::compact(report, previous),
        View::Details => details(report, None, 0),
    }
}

/// Return a bounded page or one exact preserved source, never a current filesystem reread.
fn details(report: &Report, evidence_id: Option<&str>, start: usize) -> Result<String> {
    const CAP: usize = 64 * 1024;
    if let Some(id) = evidence_id {
        let source = report
            .bundle
            .snapshot
            .evidence
            .iter()
            .find(|e| e.id == id)
            .ok_or_else(|| {
                Error::new(
                    "evidence_missing",
                    "That source is not part of the selected immutable report.",
                )
            })?;
        let text = serde_json::to_string_pretty(
            &json!({"analysis_id":report.analysis_id,"as_of":report.bundle.snapshot.observed_at,"source":source}),
        )?;
        return if text.len() <= CAP {
            Ok(text)
        } else {
            Err(Error::new(
                "detail_limit",
                "The preserved source exceeds one detail response.",
            ))
        };
    }
    let mut rows = Vec::new();
    for row in &report.analysis.outcomes {
        let mut value = serde_json::to_value(row)?;
        value["goal_text"] = json!(
            report
                .bundle
                .plan
                .outcomes
                .iter()
                .find(|o| o.id == row.id)
                .map(|o| &o.text)
        );
        rows.push(("outcomes", value));
    }
    for finding in &report.analysis.findings {
        rows.push(("findings", serde_json::to_value(finding)?));
    }
    for source in &report.bundle.snapshot.evidence {
        let preview: String = source.text.chars().take(160).collect();
        rows.push(("evidence",json!({"id":source.id,"kind":source.kind,"reference":source.reference,"worktree":source.worktree,"bytes":source.text.len(),"preview":preview,"preview_complete":preview.len()==source.text.len()})));
    }
    if start > rows.len() {
        return Err(Error::new(
            "invalid_arguments",
            "Detail continuation is beyond this retained snapshot.",
        ));
    }
    let mut view = json!({"analysis_id":report.analysis_id,"as_of":report.bundle.snapshot.observed_at,"plan":{"revision":report.bundle.plan.revision,"goal":report.bundle.plan.goal},"analysis":{"analysis_input_id":report.analysis.analysis_input_id,"plan_revision":report.analysis.plan_revision,"outcomes":[],"findings":[]},"evidence":[],"model_task_bytes":report.model_task.as_ref().map(String::len),"omissions":report.bundle.snapshot.omissions,"runtime_complete":report.bundle.developers.get("complete"),"page":{"start":start,"returned":0,"total":rows.len(),"next_start":null},"source_read":"Use evidence_id with this analysis_id for exact preserved text. Checks remain historical/unknown unless matching checked content is observed."});
    let mut returned = 0;
    for (group, row) in rows.iter().skip(start).take(32) {
        detail_rows(&mut view, group)?.push(row.clone());
        view["page"]["returned"] = json!(returned + 1);
        let next = start + returned + 1;
        view["page"]["next_start"] = if next < rows.len() {
            json!(next)
        } else {
            Value::Null
        };
        if serde_json::to_vec_pretty(&view)?.len() > CAP {
            detail_rows(&mut view, group)?.pop();
            view["page"]["returned"] = json!(returned);
            view["page"]["next_start"] = json!(start + returned);
            break;
        }
        returned += 1;
    }
    let text = serde_json::to_string_pretty(&view)?;
    if text.len() > CAP || (returned == 0 && start < rows.len()) {
        return Err(Error::new(
            "detail_limit",
            "A single detail row exceeds the response limit; compact status remains available.",
        ));
    }
    Ok(text)
}

/// Select one allowlisted detail group for page construction; arbitrary provider fields never render.
fn detail_rows<'a>(view: &'a mut Value, group: &str) -> Result<&'a mut Vec<Value>> {
    let value = if group == "evidence" {
        &mut view["evidence"]
    } else {
        &mut view["analysis"][group]
    };
    value
        .as_array_mut()
        .ok_or_else(|| Error::new("presentation_failed", "Invalid internal detail view."))
}

/// Validate bounded owner-authored text without exposing source excerpts in errors.
fn text(value: &str, limit: usize) -> Result<()> {
    if value.trim().is_empty()
        || value.len() > limit
        || value
            .chars()
            .any(|c| c == '\0' || matches!(c,'\u{202a}'..='\u{202e}'|'\u{2066}'..='\u{2069}'))
    {
        return Err(Error::new(
            "invalid_arguments",
            "Plan text is empty, oversized or contains unsafe formatting.",
        ));
    }
    Ok(())
}
