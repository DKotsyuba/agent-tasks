//! Authoritative Rust registry; discovery and schema export share the same definitions.
mod compaction_ops;
#[allow(dead_code)]
mod document_ops;
mod input;
mod knowledge_ops;
mod pages;
mod projects;
mod read;
mod records;
mod recovery_ops;
#[cfg(test)]
mod settle_tests;
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
        definition("register_project", "Purpose: Create an independently versioned project documentation repository and make it addressable by alias in one call.\nInput: project is a unique alias (ASCII letters/digits/dash/underscore/dot); doc_dir is an absolute final directory whose parent exists; name and description are English project intent. Optional remote is the SOURCE repository metadata; docs_remote is a separate documentation Git origin. Empty remote strings mean absent. No read/version precondition is needed for first registration.\nEffects: explicit local directory/files, generated dates/allocator, Git init and one initial commit, then alias publication in projects.toml beside the selected config.toml. Creates README.md, project.yaml, modules/, .agent-tasks/state.yaml and .gitignore. No push, fetch or network call. Registration is one bootstrap commit and is never settled or replayed automatically; every later successful mutation is committed locally by its own tool (see that tool's Git line).\nExisting alias/root/metadata conflicts refuse without overwrite or retargeting. Identical completed registration is UNCHANGED and never adds a commit. A failed bootstrap may leave local files or Git staging; inspect the disclosed effects and repository before retrying with the same intent.\nOutput: compact registration receipt with alias, title, version and observed effects; no directory path. Next use get_context(project=<alias>) to plan work. To change existing project metadata use plan_work edit_project.", schema::<input::RegisterArgs>(), false),
        definition("get_project_list", "Purpose: Discover registered projects and choose the alias needed to work with their documentation.\nInput: {} returns the first page. Optional start=0, limit=1-20 (default 20) and version support continuation; reuse the returned snapshot with unchanged paging scope.\nOutput: aliases, manifest-derived English names/descriptions, data/detail coverage and continuation. Filesystem paths are omitted. Uninitialized, missing, unreadable or busy projects remain listed as unavailable; they are not silently dropped. Missing registry is a valid empty list, malformed registry is an error.\nEffects: read only. Reloads sibling projects.toml and project manifests; does not create, repair, query Git, start agents or write a status document. Follow a selected entry with get_context(project=<alias>); use register_project to add a new project.", schema::<input::ProjectListArgs>(), true),
        definition("get_context","Purpose: Read one bounded assignment/current continuation pack with the correct owning Version. Omit ref for Project; E/M/A/M/T/M/A refs select work. Default summary; tasks/results/checks/review/log/commits detail views.\nOther records: ref D-001, RB-001, RS-001 (Decision, Runbook, Research; views summary, history, references), CL-001 or CL-001/I-001 (procedural Checklist or one item; summary, references), DOC-001 or a managed path such as README.md or docs/x.md (Markdown document; summary with outline, view=content exact framed pages, references) and CP-001 (compaction proposal; summary, tasks, review, references, history, content). For CP-001 view=history lists every retained revision with its full body, every review (labeled historical when it is not for the current revision and hash) and the reviewer recovery history as pageable rows with the usual start, limit and version; the optional one based revision shows only that revision and its reviews. view=content with BOTH revision and action (an id from A-01 to A-32; ids may be sparse) reads the exact hash verified staged candidate of that action in that revision in the same framed md-text-v1 pages as a document, never from the live document. A view that does not apply to the record refuses naming view. For document view=content select at most one part with ordinal, or heading (case sensitive, with occurrence when the text repeats, optional level 1-6), or preamble=true; start is then a raw byte offset, the page size is fixed by the reply budget and a supplied limit is refused. The same holds for compaction view=content. revision and action are refused naming the field for any other record, view or use; document selectors are refused for proposals. Project context also prints the knowledge allocation version, per-kind record counts, document counts by state, open compaction proposals and pending Git facts, each with its coverage. integration is Project/Epic-only and returns ready connected components >=2, exact candidate/contract coverage keys and current/needed assembly facts without unrelated-Module barriers.\nCore summary exposes real observed harness/agent_id/contact/resume, retained predecessors, loss/immersion gaps, lead discovery, exact canonical agreements, candidates and correct-pass/mutant-fail/restored-pass controls. Module-ref assignment does not duplicate a launch brief. Labels are not runtime IDs; text grants no permissions. Current role contacts/history are distinct from quality applicability.\nVersion is the owning file/dependency observation token; embedded work shares Module Version. Allocation version creates top-level records. Snapshot version is read continuation only: reuse returned start/snapshot with unchanged project/ref/view/review_index; indexes are zero-based, limit1-20. Changed scope/data refuses.\nData/detail coverage are explicit; partial/unreadable facts are not zero or fully verified. commits reads retained messages without Git. Effects: read only, no migration/runtime polling/repair. After lost/partial/uncertain writes inspect this current scope before another mutation.",
            schema::<input::ContextArgs>(),true),
        definition("project_status","Purpose: One compact tracked-work overview for orchestration/owner reporting. Input project alias, optional module=E/M/A scope. Shows planning/agreement/role recovery attention, real observed IDs versus display labels, Module candidate quality readiness, Tasks local done, Atomic local outcome/review, connected integration coverage and business verification. No live process claim.\nCore positive Module review means ready for integration even without target-branch merge. Parent cancellation or contact bookkeeping does not invent implementation drift. Missing ownership/required facts still block new decisions. Canceled work and partial readable totals are separate; omitted detail has routes to get_context.\nEffects: read only; no saved status document, model launch, Git query or cascade. Counts cover healthy readable scope; unknown work is named, not zero. Narrow after budgets.",
            schema::<input::StatusArgs>(),true),
        definition("search","Purpose: Find relevant tracked work, typed knowledge records and Markdown documents before opening their context.\nInput: project alias and query containing 1-8 whitespace-separated words, at most 256 UTF-8 bytes. Optional kinds is a list of one to three distinct of work, knowledge, document (default all three); optional state is current, superseded or any (default any; every row shows its state). Optional module=M-001 restricts the work source only: with module and no kinds the search covers work only, and module together with knowledge or document kinds is refused naming module.\nMatching: Unicode lowercase substring search across semantic fields. Every term must match somewhere in that target; different terms may match different fields. It is not regex, fuzzy, semantic or source-code search. Rank by matching-field count, then numeric reference.\nOutput: references, titles, matching-field labels, bounded excerpts, match count and explicit data/detail coverage. An excerpt is a preview, not the complete report or proof that the whole repository was searched.\nEach row names its kind, state and an exact get_context route: ref=<reference> for work and typed records, and for a document hit ref=<path or DOC id> with heading, occurrence and level plus view=content (or preamble=true view=content for the preamble). Coverage is stated per source and a source that could not be read is PARTIAL, never silently dropped. Open Epic/Module/Task/Atomic hits with get_context ref=<reference>. A Project hit is opened with get_context(project=...) and OMITTED ref; current rendering shows Project as its label, not a legal ref.\nPages: start defaults 0; limit defaults 20, range 1-20. Continue using Snapshot version and returned Next start, preserving project/query/module. Changed data/selection refuses rather than skipping.\nEffects: read only; no write token, repair, global scan or external query. Requires initialized project context.",
            schema::<input::SearchArgs>(),true),
        definition("plan_work","Purpose: Create/edit durable intent and relationships, not run models. Common project/op/version/optional actor; top-level init/create uses Allocation version; owner edits and embedded children use owning Version. New records use core coordination; old managed1/unmanaged records keep absent-core behavior until explicit adopt_core, without read migration.\ncreate_module creates provisional title/outcome/criteria/execution/contracts/dependencies context, without invented initial Tasks. Launch the lead externally, bind the returned ID, then actual bound lead uses add_task/edit_task with actor=agent_id and planning report. edit_module lead cannot silently change/clear a bound runtime identity. Display metadata is not an ID.\nContracts={not_required,provides[],consumes[]}; each core entry={id,revision,peer,description,reference?,ready}. Provider owns canonical boundary ID/positive revision/artifact; reciprocal consumed facts must match. Bundle obligations per peer/direction; all affected bound leads explicitly agree. ready is legacy metadata, never a substitute. Affecting changes need increased revision and fresh confirmations/quality; removal does not reset retained canonical revision or definition history (16 snapshots). Literal local is reserved for local quality controls.\nDependencies={ref:E/M,condition:accepted|delivered,reason}. Only real blocking waits form cycles; contract/data-flow cycles are allowed. Cycles return architecture attention before coding.\ncreate_epic(title,outcome,criteria,lead?,checks?) / edit_epic(epic,...modules[],atomics[],criterion_scopes[]) plan provisional roster and exact business scopes {index,text,modules}. Index/text matches current criteria, zero-based. freeze_epic(epic) follows lead discovery/agreement/prepared distinct checkouts and cycle checks BEFORE implementation. It does not require acyclic completion waits already met. Atomics remain attachable; creation and attachment are independent explicit publications.\ncreate/edit_atomic uses participants/execution/environment/scenarios for real integration jobs; Module Atomics remain embedded. Omission preserves; null clears optional fields; [] clears lists. Canceled work needs reasoned reopen. Semantic intent/candidate/contracts stale affecting approval/coverage, contact-only bookkeeping does not.\nOutput saved/unchanged target, owning Version/phase and disclosed effects. No model launch or daemon. Git: after a successful actual change the MCP saves the files locally and makes one automatic local commit of exactly the changed managed paths; the reply ends with a Git line that says committed, saved and pending (deferred) or outcome unknown, plus any pending references. Staged files that belong to someone else are preserved, and nothing is pushed or fetched. A real no-op or a call that published nothing makes no commit; a partial failure keeps what was saved and reports it as pending. Use git_recovery only when the reply or get_context shows pending Git work. Lost/partial creation retains reservations and visible state; inspect before retrying.",
            input::mutation_schema::<input::Plan>(false),false),
        definition("record_work","Purpose: Record explicit trusted observations and progress, never launch models or arbitrarily set status. Common project/ref/version/actor; owning Module Version covers its Tasks/Atomics.\nadopt_core explicitly activates an old owner, stales earlier approval and invents no missing facts. bind_agent(role,harness,agent_id,communication_ref,resume_ref?,launch_ref) stores actual launch-before-bind receipt data; same ID retries do not duplicate history. Module lead/reviewer and integration integrator/reviewer persist. Epic actions have orchestrator attribution and no binding slots; generic Atomic retains executor/review policy.\nrecover_agent(role,stage=lost|immersed,...): lost requires lost=true AND unrecoverable=true plus reason/observation of inability to continue/resume; temporary timeout/model preference is insufficient. Only then may replacement bind; it needs a new actual-ID immersion report with understanding/sources/unfinished/gaps. Gaps block scoped continuation; prior identities/work/findings remain.\nplanning(responsibility,scope,exclusions,read_refs,uncertainties) is the bound lead discovery before coding; Task/contract/dependency outputs do not self-invalidate it. Significant goal/criteria/execution changes need renewal. agree_contract(contract_id,revision,summary) is each actual affected lead's exact reciprocal confirmation.\nbegin reports lifecycle only. Core Epic begin permits planning; freeze_epic later freezes negotiated roster. Module coding needs actual bound lead/current planning/agreement/environment/frozen parent and actual fulfilled waits. Separate active Module writable checkouts; integration separate.\nresult replaces COMPLETE current summary/checks/gaps/followups/artifacts with optional definite candidate/changed_scope. Module positive review needs exact restored candidate. Task completion is actual bound lead's tests OR manual decision; import/runtime/check success does not close it. complete uses the existing meaningful local report. No independent Task review.\nboundary_evidence(contract_id,revision,candidate,conditions,correct,mutation,failed,restored,artifacts) records actual passed/failed/passed controls. correct/failed/restored={status,detail,artifact?}; no assertion/setup mutation to fake failure. For contracts.not_required=true use local revision1, reserved for Module-local quality. Controls bind candidate AND meaningful goal/criteria/execution intent; changed tested scope needs fresh observations. MCP neither executes nor certifies them.\nIntegration Atomic begins with >=2 connected current-reviewed Modules, actual integrator, distinct checkout/environment/scenarios. First begin claims exact current inputs and refuses equivalent pending or accepted work. Definite assembly candidate is required for review; changed inputs need explicit reopen/recheck, never late rebinding. No code change needs no fabricated commit.\nverify_criterion(index,text,modules,candidate,environment,scenarios,summary,checks,artifacts,integration_ref?) on Epic matches its exact declared affected scope. Record actual current business/E2E verification; ONE integration_ref must cover that whole composition and match report inputs. AB+BC union is never ABC business proof. All indexes zero-based.\nimport_commits reads1-8 validated hex commit objects under ONE5s/80KiB preflight/read budget; verifies declared repo/worktree/branch, retains original metadata/messages and dedups canonical repo/fullSHA. Parsed Result/Checks/Gaps/Followups are reported assertions. Module core candidate is exact fullSHA; artifacts retain commit locators. Late failure/capacity preserves current report. Explicit done is still lead decision; duplicate import never refreshes integration evidence.\ndeliver records separate local delivery bookkeeping; it is not core Module integration readiness. blocker/handoff/clear/cancel/reopen preserve reasons/history and no cascade. Semantic candidate/reopen invalidates relevant coverage; role/contact/handoff dates alone do not.\nOutput saved/unchanged owning Version/phase and exact effects. No GitHub, runtime execution or credential reading. Git: after a successful actual change the MCP saves the files locally and makes one automatic local commit of exactly the changed managed paths; the reply ends with a Git line that says committed, saved and pending (deferred) or outcome unknown, plus any pending references. Staged files that belong to someone else are preserved, and nothing is pushed or fetched. A real no-op or a call that published nothing makes no commit; a partial failure keeps what was saved and reports it as pending. Use git_recovery only when the reply or get_context shows pending Git work. After stale/lost/partial/unknown outcomes inspect current context/history before retrying.",
            input::mutation_schema::<input::Work>(true),false),
        definition("review_work","Purpose: Current whole Module/Epic or standalone/embedded Atomic verdict; Task refs refuse. Inputs project/ref/version/verdict/summary, actual reviewer actor, findings/checks, changed_scope[], resolved_findings[{review_index,finding_index,summary}]. Finding indexes are zero-based immutable history addresses.\nCore Module review uses the SAME observed bound reviewer through initial whole review and changed-scope follow-up. Replacement requires observed irrecoverable loss/bind/new immersion; prior findings remain. Every actual whole-Module or integration review needs a definite submission. Positive review needs definite restored candidate, exact affecting canonical revisions/all-party confirmations and meaningful correct-pass/mutant-fail/restored-pass observations. Local no-cross-boundary quality is local revision1 tied to current goal/criteria/candidate. Unresolved required findings cannot disappear; follow-up names scope and independent resolutions in the reviewed target own history. Embedded Atomic reviews do not require unrelated whole-Module findings or submission.\nPositive core Module is ready for integration WITHOUT merge/delivery. Affecting contract changes stale readiness even if code unchanged; same lead/reviewer handle corrections. Runtime contact/recovery metadata alone does not erase valid quality.\nIntegration review binds actual >=2 connected exact candidate/contract assembly/environment/scenarios. Current coverage may be incremental AB then BC/ABC, with no unrelated/full-roster job barrier. Epic final acceptance still requires every business criterion current verified over its declared affected composition or actual Epic E2E; raw edge union plus generic pass is insufficient.\nEvery Atomic remains independently reviewed; generic Atomic executor policy differs from actual persistent integration role. Known authors/integrators cannot self-review. Declared ID observations are not authentication.\nNegative changes_requested is a saved successful call. Check updates preserve before/after provenance. Output owning Version/phase/effects; context review/history can be paged. No external merge, model execution or certificate platform. Git: after a successful actual change the MCP saves the files locally and makes one automatic local commit of exactly the changed managed paths; the reply ends with a Git line that says committed, saved and pending (deferred) or outcome unknown, plus any pending references. Staged files that belong to someone else are preserved, and nothing is pushed or fetched. A real no-op or a call that published nothing makes no commit; a partial failure keeps what was saved and reports it as pending. Use git_recovery only when the reply or get_context shows pending Git work. Inspect uncertain replies before another write.",
            schema::<input::ReviewWorkArgs>(),false),
        definition("review_module","Purpose: Compatible Module-only review surface; review_work also handles Epic/Atomic. Same candidate/contract/control/actual persistent reviewer rules in core mode. Initial review is whole; later changed scope resolves stable zero-based prior findings through same reviewer or explicitly recovered replacement.\nPositive core Module is ready for integration without delivery; managed1/unmanaged absent-core files preserve their old closure policy until adopt_core. Task has no review. Delivery/handoff/contact metadata alone is not implementation drift.\nInputs project/module/version/verdict/summary, actor, findings/checks, changed_scope/resolved_findings. Output saved owning Version/phase/effects; retained review readable by context. No runtime execution. Git: after a successful actual change the MCP saves the files locally and makes one automatic local commit of exactly the changed managed paths; the reply ends with a Git line that says committed, saved and pending (deferred) or outcome unknown, plus any pending references. Staged files that belong to someone else are preserved, and nothing is pushed or fetched. A real no-op or a call that published nothing makes no commit; a partial failure keeps what was saved and reports it as pending. Use git_recovery only when the reply or get_context shows pending Git work. Inspect stale/lost/partial/unknown outcomes before another mutation.",
            schema::<input::ReviewArgs>(),false),
    ]);
    // One purpose tool per producer; each closed payload enum shares `input::Common`.
    result.extend([
        definition(
            "knowledge_work",
            knowledge_ops::DESCRIPTION,
            input::mutation_schema::<knowledge_ops::KnowledgeOp>(false),
            false,
        ),
        definition(
            "document_work",
            document_ops::DESCRIPTION,
            input::mutation_schema::<document_ops::DocumentOp>(false),
            false,
        ),
        definition(
            "compaction_work",
            compaction_ops::DESCRIPTION,
            input::mutation_schema::<compaction_ops::Compaction>(false),
            false,
        ),
        definition(
            "git_recovery",
            recovery_ops::DESCRIPTION,
            input::mutation_schema::<recovery_ops::RecoveryOp>(false),
            false,
        ),
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
    result.extend(pages::templates());
    result
}

/// Focused disposable filesystem and work-contract qualification.
#[cfg(test)]
mod core_tests;

/// Focused regressions for the shared producer host seam.
#[cfg(test)]
mod host_tests;

/// Regressions for scoped context guidance and Epic member roll-up.
#[cfg(test)]
mod context_tests;

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
        ("knowledge_work", true),
        ("document_work", true),
        ("compaction_work", true),
        ("git_recovery", true),
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
