//! Authoritative Rust registry; discovery and schema export share the same definitions.
mod input;
mod projects;
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
        definition("register_project", "Purpose: Create an independently versioned project documentation repository and make it addressable by alias in one call.\nInput: project is a unique alias (ASCII letters/digits/dash/underscore/dot); doc_dir is an absolute final directory whose parent exists; name and description are English project intent. Optional remote is the SOURCE repository metadata; docs_remote is a separate documentation Git origin. Empty remote strings mean absent. No read/version precondition is needed for first registration.\nEffects: explicit local directory/files, generated dates/allocator, Git init and one initial commit, then alias publication in projects.toml beside the selected config.toml. Creates README.md, project.yaml, modules/, .agent-tasks/state.yaml and .gitignore. No push, fetch or network call; later writes are not auto-committed.\nExisting alias/root/metadata conflicts refuse without overwrite or retargeting. Identical completed registration is UNCHANGED and never adds a commit. A failed bootstrap may leave local files or Git staging; inspect the disclosed effects and repository before retrying with the same intent.\nOutput: compact registration receipt with alias, title, version and observed effects; no directory path. Next use get_context(project=<alias>) to plan work. To change existing project metadata use plan_work edit_project.", schema::<input::RegisterArgs>(), false),
        definition("get_project_list", "Purpose: Discover registered projects and choose the alias needed to work with their documentation.\nInput: {} returns the first page. Optional start=0, limit=1-20 (default 20) and version support continuation; reuse the returned snapshot with unchanged paging scope.\nOutput: aliases, manifest-derived English names/descriptions, data/detail coverage and continuation. Filesystem paths are omitted. Uninitialized, missing, unreadable or busy projects remain listed as unavailable; they are not silently dropped. Missing registry is a valid empty list, malformed registry is an error.\nEffects: read only. Reloads sibling projects.toml and project manifests; does not create, repair, query Git, start agents or write a status document. Follow a selected entry with get_context(project=<alias>); use register_project to add a new project.", schema::<input::ProjectListArgs>(), true),
        definition("get_context","Purpose: Obtain a bounded start/resume pack, actual conditions and correct write Version.\nInput: project alias; omit ref for Project, otherwise E-001/M-001/A-001/M-001/T-001/M-001/A-001. Default view=summary; work views tasks/results/checks/review/log/commits. Project supports summary; Tasks have no review. Module-ref delegation uses this pack plus addressed detail pages, without copying a second launch brief.\nOutput: intent, relevant parent background, own criteria, declared execution, provides/consumes peers, explicit waits/readiness, lead/executor, local progress, independent review, delivery and current/stale integration. Parent criteria are background; only owner-declared checks are requirements. Missing/unreadable requirements remain named PARTIAL data, not zero work. Actor/runtime/artifact facts are reported.\nVersion guards the owning record; embedded children share Module Version. Relevant dependency observations bind modern writes; Allocation version only creates top-level work. Snapshot version is READ continuation only. start=0 and limit=1-20; continue returned start/Snapshot version with unchanged project/ref/view/review_index. review_index selects retained verdict history.\ncommits reads retained source metadata/original messages without querying Git. Effects: read only, no migration/repair/runtime polling. After stale/partial/unknown outcomes inspect current scope, then reconcile; do not replay a write to fix presentation.",
            schema::<input::ContextArgs>(),true),
        definition("project_status","Purpose: Present one complete bounded overview of tracked project activity to the owner.\nInput: project alias; optional module=E-001/M-001/A-001 narrows work and totals.\nOutput: readable Epic/Module/Task/Atomic counts, active/standalone scope, declared leads/executors, local results/checks, peer obligations/waits, review/delivery/integration attention, blockers and imported report summaries. Tasks are counted separately; Atomic local done is distinct from current reviewed closure. Canceled work is separate. Existing intrinsic approvals/delivery do not become implementation drift merely because parent ownership is unknown; new decisions still refuse uncertain requirements.\nData coverage names unreadable work; partial totals are lower bounds. Detail coverage separately discloses omitted rows; use narrowing or get_context detail pages. Handles are reported references, not live-process proof.\nEffects: read only; no saved status document, Git query, agent launch or automatic cascade. Requires a readable Project manifest.",
            schema::<input::StatusArgs>(),true),
        definition("search","Purpose: Find relevant tracked work before opening its context.\nInput: project alias and query containing 1-8 whitespace-separated words, at most 256 UTF-8 bytes. Optional module=M-001 restricts the scope.\nMatching: Unicode lowercase substring search across semantic fields. Every term must match somewhere in that target; different terms may match different fields. It is not regex, fuzzy, semantic, source-code or Markdown-document search. Rank by matching-field count, then numeric reference.\nOutput: references, titles, matching-field labels, bounded excerpts, match count and explicit data/detail coverage. An excerpt is a preview, not the complete report or proof that the whole repository was searched.\nOpen Epic/Module/Task/Atomic hits with get_context ref=<reference>. A Project hit is opened with get_context(project=...) and OMITTED ref; current rendering shows Project as its label, not a legal ref.\nPages: start defaults 0; limit defaults 20, range 1-20. Continue using Snapshot version and returned Next start, preserving project/query/module. Changed data/selection refuses rather than skipping.\nEffects: read only; no write token, repair, global scan or external query. Requires initialized project context.",
            schema::<input::SearchArgs>(),true),
        definition("plan_work","Purpose: Create/edit intent and relationships; never start agents or set arbitrary status.\nInput: project, op, current Version and optional actor. init_project(title,purpose,remote?) and create_module/create_epic/create_atomic use Project Allocation version. edit_project uses manifest Version.\ncreate_module(title,outcome,lead?,criteria[],required_checks[],tasks[],execution?,contracts?,dependencies[]) creates standalone M-001. execution={repository,worktree,branch,target_branch}; contracts={not_required,provides[],consumes[]} entries={peer:M-001,description,reference?,ready}. Bundle obligations per peer/direction; reciprocal contract links are allowed and do not imply waits. dependencies={ref:E/M,condition:accepted|delivered,reason}; delivered is Module-only. Reject dangling/duplicate/self/cyclic blocking refs including parent closure cycles.\nedit_module(module, optional matching fields) uses Module Version. add_task(module,title,criterion?,required_checks[]) / edit_task(ref,...) preserve Task leaf ownership.\ncreate_epic(title,outcome,criteria[],lead?,required_checks[]) creates E-001; edit_epic(epic,intent/checks/lead?,modules[],atomics[]) replaces supplied member lists. First reported Epic begin freezes the Module roster forever through reopen; later Module changes/moves refuse. Atomics remain attachable. Creation and attachment are separate writes with disclosed independent outcomes.\ncreate_atomic(title,outcome,executor?,required_checks[],participants[],execution?,environment?,scenarios[]) creates A-001; add_atomic(module,title,outcome,executor?,required_checks[]) embeds M-001/A-001. edit_atomic(ref, optional matching fields) preserves omission; embedded Atomics inherit execution and cannot declare standalone participants/environment.\nOmission preserves; null clears optional fields, [] clears lists. Semantic edits stale approval and modern Module delivery. New records follow explicit workflow; legacy records remain grandfathered until explicit begin. Adding declarations alone does not opt legacy lifecycle in.\nOutput: saved/unchanged target, owning Version/phase and effects. Effects: local guarded YAML writes with allocated IDs/dates; no Git write, runner or document writer. Lost/partial/uncertain creation retains reserved gaps; inspect context/inventory before another mutation.",
            input::mutation_schema::<input::Plan>(false),false),
        definition("record_work","Purpose: Report actual progress, local decisions, start, delivery or explicit local Git report import.\nInput: project, op, ref, owning Version, optional declared actor. Embedded children share Module Version.\nbegin reports lifecycle start only; never launches agents. First Epic begin freezes its Module roster. Module begin needs known lead/criteria, declared repository/worktree/branch/target_branch, declared ready contracts and resolved blocking waits plus open/begun parent. Integration Atomic begin needs executor, real environment/scenarios and currently accepted/delivered participating Modules. Explicit legacy begin opts into modern policy.\nresult replaces the COMPLETE summary/checks/gaps/followups/artifacts report; omitted lists clear current data. Task/Atomic state=open|done is explicit local state, omission preserves. Task done is the known Module lead's decision after tests OR manual verification; no Task review. Modern Atomic local done remains pending independent review. Module/Epic results omit state. complete decides local Task/Atomic completion using its existing meaningful report, avoiding duplicate narration. Modern undo uses reasoned reopen.\ndeliver(ref=M,target_branch,summary,artifact?) reports the current independently reviewed Module's delivery to the declared target. Local merge counts; hosting PR optional. No second review is needed for delivery bookkeeping; semantic changes/reopen invalidate it.\nimport_commits(commits:[1-8 hex selectors],state?) resolves declared execution and uses ONE bounded five-second/80-KiB read-only Git inspection. Repository/worktree identity and actual branch must match declarations; selectors must name commit objects. Retain canonical repository/full SHA/author/date/subject/original message and parsed Result/Checks/Gaps/Followups. Dedup repository+SHA across owner targets. All reads/validation finish before publication; capacity or late failure leaves current report unchanged. Import alone never closes work; explicit done remains a local decision. Duplicate imports never refresh integration evidence. view=commits reads retained history after Git changes.\nblocker/handoff/clear_* target top-level work; clear requires reason. Handoff/date metadata does not stale evidence. cancel/reopen require reasons; parents resolve children first, no cascade, canceled parents block writes. Every modern Atomic has independent review.\nOutput: saved/unchanged target, owning phase/Version and exact effects; embedded receipts label Module phase. Imported facts remain reported assertions, not certificates. No Git commit/merge/push, GitHub API, credential reading or agent launch. After stale/lost/partial/unknown outcomes inspect context/history before retrying.",
            input::mutation_schema::<input::Work>(true),false),
        definition("review_work","Purpose: Independently review a whole Epic/Module or ANY Atomic, including embedded M-001/A-001. Tasks refuse.\nInput: project, ref=E/M/A/M/A, current owning Version, verdict=accepted|changes_requested, summary, optional actor/findings={text,must_fix}/checks={target,label,status,detail}. Known lead/executor/result author cannot self-review Atomics; known Module/Epic lead cannot self-review. Declared identities are not authentication.\nAtomic accepted needs reported begin, meaningful local completion, passed required checks and no gaps; integration evidence must be current. Embedded Atomic review writes only its owning Module and may update only that Atomic's checks. Whole Module review requires terminal Tasks and independently accepted/canceled Atomics, meaningful outcome, passed required checks and no blocker/gaps. Positive Module review yields reviewed; delivery pending until explicit matching reported delivery.\nEpic accepted needs begun frozen roster, current accepted/delivered noncanceled Modules, currently reviewed noncanceled Atomics, own result/checks/criteria and current independently reviewed integration whose participant set EXACTLY matches the active frozen roster with environment/scenarios. A zero-Module Epic needs no invented integration. Missing/unreadable decision requirements refuse.\nUpdates retain before/after check provenance; accepted forbids must_fix findings. changes_requested is a saved successful call. Semantic changes/reopen stale approval; delivery/handoff/date bookkeeping alone does not.\nOutput: saved owning Version/phase; get_context view=review provides retained verdicts. Effects: guarded local review/history/check writes, no external merge, runner or proof engine. Inspect after lost/partial/stale/unknown replies; never replay blindly.",
            schema::<input::ReviewWorkArgs>(),false),
        definition("review_module","Purpose: Compatible Module-only independent review entrypoint; use review_work for Epic or Atomic.\nInput: project, module=M-001, current Module Version, verdict, summary and optional actor/findings/check updates. Same whole-Module gates/provenance as review_work; no individual Task review and no Atomic bypass.\nFor modern work positive review is separate from reported delivery: reviewed; delivery pending becomes accepted after record_work deliver to declared target. Legacy records retain their original closure policy until explicit begin.\nOutput: saved owning Version/phase and effects. Known lead self-review refuses; negative verdict is successful. Semantic changes/reopen make approval historical; ordinary delivery/handoff/date bookkeeping does not require repeated review.\nEffects: one guarded Module write, no Git/GitHub action or runner. Retained review is readable via get_context view=review; inspect uncertain/lost outcomes before another mutation.",
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
    "annotations":{"readOnlyHint":read,"destructiveHint":!read && name != "register_project","idempotentHint":read || name == "register_project","openWorldHint":false}})
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
        ("register_project", true),
        ("get_project_list", true),
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
