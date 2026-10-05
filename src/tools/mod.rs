//! Authoritative Rust registry; discovery and schema export share the same definitions.
mod input;
mod read;
mod work;
use crate::{response::Templates, store::Config};
use mcp_presentation::Renderer;
use rmcp::model::{CallToolResult, ContentBlock};
use serde_json::{Value, json};
// xtask:modules

/// Export the configuration-independent catalog; local work arguments are closed serde schemas.
pub fn definitions() -> Vec<Value> {
    let mut result = vec![
        json!({"name":"get_status","description":"Report product identity and declared release qualification. Read-only and configuration-independent; it does not inspect project work. Use project_status for the actual tracked progress.",
        "inputSchema":{"type":"object","properties":{},"additionalProperties":false},
        "annotations":{"readOnlyHint":true,"destructiveHint":false,"idempotentHint":true,"openWorldHint":false}}),
        // xtask:definitions
    ];
    result.extend([
        definition("get_context","Read compact context for a configured project alias, Module (M-001), or embedded Task (M-001/T-001). Start here before planning or resuming work. Returns human-readable intent, current facts, conditions, exact file version and bounded details; project context also returns allocation_version. Select summary/tasks/results/checks/review/log; no task-specific review exists. Reads create/repair nothing. For continuation keep the same scope/view/review_index and use the returned start and snapshot version. Use manifest Version for edit_project, allocation_version for init_project/create_module, and Module Version for all child writes. Missing configuration is a safe refusal; YAML is internal storage.",
            schema::<input::ContextArgs>(),true),
        definition("project_status","Produce one English status suitable for the owner: project purpose, readable Module/Task progress, declared leads, current summaries, blockers, review attention and last reported activity. One call replaces manual status collection. Read-only; never polls live agents or Git. Unknown/unreadable work is explicit PARTIAL with lower-bound counts, not zero. Long detail is omitted explicitly; module=M-001 narrows the response. Only tracked work is counted; small fixes may have no tracker records.",
            schema::<input::StatusArgs>(),true),
        definition("search","Find tracked work by 1–8 words (maximum 256 UTF-8 bytes). All terms must match semantic fields using Unicode lowercase substring matching; rank by matching-field count then numeric reference. Returns references, field labels and relevant excerpts rather than YAML. Read matches through get_context and select detail views. Optional module narrows the scan; later pages require the exact prior snapshot with unchanged project/query/module. Coverage limits/unreadable records remain explicit. This is lexical work search; Markdown/code/semantic search is outside the core.",
            schema::<input::SearchArgs>(),true),
        definition("plan_work","Create or partially edit strategic work. init_project explicitly initializes the final configured root; edit_project changes orientation; create_module allocates one standalone Module and optional initial Tasks; edit_module/add_task/edit_task change one whole Module file. Obtain get_context first. init_project/create_module require allocation_version, edit_project manifest Version, all Module/Task operations Module Version. IDs, UTC dates, counters and concise activity are generated. Edit omission preserves; null clears only optional remote/lead/criterion; [] clears a check list. Required fields cannot be cleared. No status cascades, agents, Git or external writes. Stale/busy/validation refuses; unknown or partial publication requires inspecting context before retry, never blind replay.",
            input::mutation_schema::<input::Plan>(false),false),
        definition("record_work","Save current result, blocker, handoff or explicit cancel/reopen on one Module or owned Task using its current whole-Module Version. result replaces summary/checks/gaps/followups/artifacts; it is not a partial patch. Task state optionally open/done; completion needs a meaningful result, no separate task review. Module-only blocker/handoff records actionable context. Clear/reopen/cancel require reasons; absent clears are no-ops. Canceled work permits reopen only; a canceled parent blocks child writes. Module cancellation requires terminal children, never cascades. Semantic changes invalidate old module approval; handoff does not. Evidence/artifact strings are reported, not Git-verified. Reads after lost replies determine effects; mutation is not replay-idempotent.",
            input::mutation_schema::<input::Work>(true),false),
        definition("review_module","Record an independent whole-Module review against its current Version. accepted requires no open Tasks/blocker/in-scope gaps, every explicit required check passed, and meaningful delivery evidence; canceled children are excluded. changes_requested is a successfully saved negative verdict, not a tool error. Optional check updates replace canonical target checks while preserving before/after attribution; lead summary author/date stays unchanged. Known lead cannot self-review; unknown identity stays unknown. Reviews are retained and bound to current semantic basis/epoch; later semantic edits or reopen make approval historical. Accepted directly yields accepted phase: no hidden PR/merge gate or external verification. Read view=review for report/history, and project_status for closure attention.",
            schema::<input::ReviewArgs>(),false),
    ]);
    result
}

/// One source of schema export; generated JSON is a reviewed snapshot.
fn schema<T: schemars::JsonSchema>() -> Value {
    serde_json::to_value(schemars::schema_for!(T)).unwrap_or(Value::Null)
}

/// Build one local closed-world catalog entry with its actual mutation/read hints.
fn definition(name: &str, description: &str, input: Value, read: bool) -> Value {
    json!({"name":name,"description":description,"inputSchema":input,
    "annotations":{"readOnlyHint":read,"destructiveHint":!read,"idempotentHint":read,"openWorldHint":false}})
}

/// Register trusted product layouts without configuration access.
pub fn templates() -> Vec<(&'static str, &'static str)> {
    let mut result = vec![
        (
            "invalid_arguments",
            "ERROR invalid_arguments: get_status accepts an empty argument object.\n",
        ),
        // xtask:templates
    ];
    result.extend(work::templates());
    result
}

/// Focused disposable filesystem and work-contract qualification.
#[cfg(test)]
mod core_tests;

/// Report only actual unimplemented skeletons; release qualification stays independent.
pub fn incomplete() -> Vec<&'static str> {
    let statuses: &[(&str, bool)] = &[
        ("get_status", true),
        ("get_context", true),
        ("project_status", true),
        ("search", true),
        ("plan_work", true),
        ("record_work", true),
        ("review_module", true),
        // xtask:readiness
    ];
    statuses
        .iter()
        .filter(|(_, implemented)| !implemented)
        .map(|(name, _)| *name)
        .collect()
}

/// Route identity without config I/O; business tools resolve an alias only when called.
pub async fn call(
    name: &str,
    args: Value,
    identity: &Renderer,
    _templates: &Templates,
    config: &Config,
) -> Option<CallToolResult> {
    match name {
        "get_status" => {
            if !args.as_object().is_some_and(|a| a.is_empty()) {
                let text = _templates
                    .render("invalid_arguments", &())
                    .unwrap_or_else(|_| {
                        "ERROR invalid_arguments: no effect performed. Presentation degraded.\n"
                            .to_owned()
                    });
                let mut result = CallToolResult::success(vec![ContentBlock::text(text)]);
                result.is_error = Some(true);
                return Some(result);
            }
            let reply = identity.identity(
                env!("CARGO_PKG_NAME"),
                env!("CARGO_PKG_VERSION"),
                crate::qualification(),
            );
            let mut result =
                CallToolResult::success(vec![ContentBlock::text(reply.text().to_owned())]);
            result.is_error = Some(reply.is_error());
            Some(result)
        }
        // xtask:routes
        _ => work::call(name, args, config, _templates),
    }
}
