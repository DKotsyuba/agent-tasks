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
        json!({"name":"get_status","description":"Purpose: Check the MCP product identity and declared release qualification.\nUse this for connection/diagnostic orientation. For progress of tracked work, use project_status.\nInput: an empty object {}. Configuration is not required.\nOutput: one English text block naming the product, version and qualification. not_verified is a release/host qualification statement, not a claim that a work item failed.\nEffects: none. Does not inspect work, create storage, access Git or query agents.",
        "inputSchema":{"type":"object","properties":{},"additionalProperties":false},
        "annotations":{"readOnlyHint":true,"destructiveHint":false,"idempotentHint":true,"openWorldHint":false}}),
        // xtask:definitions
    ];
    result.extend([
        definition("get_context","Purpose: Read enough current context to choose the next action and obtain the correct write precondition.\nInput: project is a configured alias. Omit ref for Project; otherwise use M-001 or M-001/T-001. Do not pass ref=\"Project\".\nViews: summary is the default. Project supports summary only. Module supports summary, tasks, results, checks, review and log. Task supports summary, results, checks and log; review belongs to its Module.\nsummary returns orientation, phase/counts, lead, blockers/handoff and remaining acceptance conditions; it is not the full task list or result body. Use tasks for the Module's children, results for current reports/artifacts, checks for reported/required evidence, review for a retained verdict, and log for recent generated activity. A Module results view previews Task results; read the Task ref for its full result.\nOutput is compact English text with separate Data coverage and Detail coverage. Version is the editable manifest/whole-Module version. Allocation version is the Project creation precondition for init_project/create_module. Snapshot version is for READ continuation, never a write token.\nPages: start defaults 0; limit defaults 20 and must be 1-20. When Next supplies start/version, continue with Snapshot version and unchanged project/ref/view/review_index. review_index is zero-based; omission selects latest. A changed selection or snapshot refuses. Narrow scope/view after capacity limits.\nEffects: read only; never creates, repairs or finishes initialization. An absent configured root returns orientation and a creation precondition. Unknown aliases/missing config refuse safely.",
            schema::<input::ContextArgs>(),true),
        definition("project_status","Purpose: Produce the tracked-work overview that the orchestrator can present to the owner in one call.\nInput: project is a configured alias; optional module=M-001 narrows both displayed work and totals to that Module. Omit module for project-wide totals.\nOutput: compact English text with project purpose, readable Module/Task counts and phases, declared leads/handles, current result summaries, blockers, latest review information and last reported activity. This is not live agent monitoring or independent artifact verification.\nData coverage reports unreadable/unscanned work; PARTIAL counts are lower bounds, not zero. Detail coverage separately reports omitted text. Project-wide status shows at most four Task details per Module before response-budget limits; counts still include readable Tasks. Use module narrowing or get_context view=tasks/results/checks/review for detail.\nEffects: read only. No version is needed. Does not save a status document, change lifecycle, start agents or inspect Git. The project must have a valid manifest.",
            schema::<input::StatusArgs>(),true),
        definition("search","Purpose: Find relevant tracked work before opening its context.\nInput: project alias and query containing 1-8 whitespace-separated words, at most 256 UTF-8 bytes. Optional module=M-001 restricts the scope.\nMatching: Unicode lowercase substring search across semantic fields. Every term must match somewhere in that target; different terms may match different fields. It is not regex, fuzzy, semantic, source-code or Markdown-document search. Rank by matching-field count, then numeric reference.\nOutput: references, titles, matching-field labels, bounded excerpts, match count and explicit data/detail coverage. An excerpt is a preview, not the complete report or proof that the whole repository was searched.\nOpen Module/Task hits with get_context ref=<reference>. A Project hit is opened with get_context(project=...) and OMITTED ref; current rendering shows Project as its label, not a legal ref.\nPages: start defaults 0; limit defaults 20, range 1-20. Continue using Snapshot version and returned Next start, preserving project/query/module. Changed data/selection refuses rather than skipping.\nEffects: read only; no write token, repair, global scan or external query. Requires initialized project context.",
            schema::<input::SearchArgs>(),true),
        definition("plan_work","Purpose: Create or partially edit the work plan, not report completion.\nInput: project alias, op and the correct current version; actor is optional declared attribution, not authentication. Do not supply generated IDs, dates or raw YAML.\nOperations:\n- init_project: explicitly create missing owned project storage; requires title/purpose, optional reported remote. Obtain Project Allocation version first. Root parent must already exist; reads do not create it.\n- edit_project: optional title/purpose/remote patch; use Project Version.\n- create_module: required title/outcome, optional lead={name,handle}, required_checks and initial semantic tasks; use Project Allocation version. Returns the generated M-001 reference. Maximum 32 Tasks per Module.\n- edit_module: module plus optional title/outcome/lead/required_checks; use that Module Version.\n- add_task: module/title, optional criterion/required_checks; use Module Version. MCP allocates the next Task ID.\n- edit_task: ref=M-001/T-001 and optional title/criterion/required_checks; use the owning Module Version.\nEdit omission preserves existing data; null clears only optional remote/lead/criterion; [] clears a list. Required fields cannot be cleared. Plan edits preserve unrelated results/history but semantic changes make old Module approval historical. Canceled work must be reopened explicitly.\nOutput: SAVED or UNCHANGED, generated target/current file Version, owning Module phase where applicable, and disclosed filesystem effects. After init_project, read Project context again for fresh Allocation version before create_module. Do not treat the returned manifest Version as an allocation token.\nEffects: explicit local storage writes with generated IDs/UTC dates/activity. No status setter/cascade, Git action, agent launch or document writer. On stale/busy/validation refusal follow the diagnostic. A partial/uncertain publication or lost reply requires inspecting current context/log/inventory before another mutation; never blindly replay.",
            input::mutation_schema::<input::Plan>(false),false),
        definition("record_work","Purpose: Record current outcome, an actionable obstacle/resume point, or an explicit lifecycle decision.\nInput: project alias, op, ref and current owning Module Version; actor is optional declared attribution.\nOperations:\n- result: required summary; optional checks/gaps/followups/artifacts constitute a COMPLETE current report replacement. Omitted lists become empty and erase the prior current lists; include every current check/artifact you intend to retain. Task state may be open or done; omission PRESERVES its current state, so reporting a result alone does not complete an open Task. Module results must omit state; Module acceptance uses review_module.\n- blocker: Module only; problem/needed_action, optional resolver.\n- handoff: Module only; stopping_point/next_action. Handoff does not invalidate current approval.\n- clear_blocker/clear_handoff: Module only, required reason; absent data is a no-op.\n- cancel: required reason; Module children must already be terminal. Never cascades.\n- reopen: required reason; resumes the same target and makes old Module approval historical.\nCanceled targets accept reopen only; canceled parents block child writes. Done Tasks need a meaningful result. Failed/missing check reports can coexist with Task done; explicit required checks must pass before Module acceptance.\nOutput: SAVED or UNCHANGED, target, current owning Module phase and whole-Module Version, plus effects. For a Task receipt, ready/accepted/etc. is the MODULE phase, not a Task state; inspect Task context/status for its state. Current report and reviewer updates retain declared attribution.\nEffects: local writes, generated dates and activity. Artifact/commit/PR references are reported strings; no Git/GitHub verification, commit, PR operation or agent launch occurs. Semantic changes invalidate old approval. Inspect context after stale/partial/uncertain outcomes; do not replay blindly.",
            input::mutation_schema::<input::Work>(true),false),
        definition("review_module","Purpose: Save an independent verdict on the WHOLE Module, not review or close a Task separately.\nInput: project alias, module=M-001, current Module Version, verdict=accepted|changes_requested and summary; optional findings={text,must_fix}, checks={target,label,status,detail} and actor. Supply reviewer attribution when known. A known lead matching actor by name/handle cannot self-review; unknown identities remain unknown rather than authenticated.\naccepted requires an open Module with no open Tasks, blocker or in-scope gaps; all explicitly required Module/Task check labels must have passed. Canceled children are excluded. If no Task is done, the Module needs its own meaningful result. not_applicable does not waive required checks. accepted cannot contain must_fix findings.\nchanges_requested saves a negative conclusion without requiring acceptance readiness. It is a successful call with isError=false, not a tool failure.\nOptional check updates replace canonical Module/owned Task check reports BEFORE validation/verdict capture. They retain before/after provenance without changing the lead result-summary author/date. Canceled Task checks cannot be changed by review.\nOutput: a saved receipt with current Module Version and derived phase. accepted immediately yields accepted while current; no separate status move/merge gate is required. Read get_context ref=M-001 view=review for conclusion/findings/check updates/history, using zero-based review_index if needed.\nEffects: local review/history/check/activity writes, no Git merge or artifact verification. Later semantic changes/reopen make approval historical; handoff alone does not. Retained reviews are not silently pruned. On a lost/partial/uncertain reply inspect current review/context before another write.",
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
