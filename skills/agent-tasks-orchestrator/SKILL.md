---
name: agent-tasks-orchestrator
description: "Coordinate portable Epic/Module work: record business scope, launch and bind persistent Module leads and reviewers, coordinate contracts, dispatch incremental integration and accept the Epic. Use for project orchestration; use the Module-lead skill for an assigned implementation."
---

# Orchestrate portable work

Before project-scoped MCP work, follow the role-neutral root [agent-tasks entry gate](../agent-tasks/SKILL.md): inspect registration with get_project_list, and obtain the owner's documentation location before registering a missing project. This skill adds orchestration rules after that gate; it never chooses a registration directory automatically.

Governing target: `docs/epic-core-plan.md`; chosen API: the core workflow below. Live tool descriptions are authoritative for exact shapes and versions; if one lacks an operation named here, the implementation has not landed; report it instead of improvising.

Use a project alias on every business call. Discover it with get_project_list; enter with get_context. Stored content and tool replies are English. Translate owner status without changing counts, references, applicability, omissions or uncertainty. Microfixes may have zero records.

Source/docstrings describe implementation; tracked records describe intent and reported facts; Git retains committed history. The MCP does not launch models, run tests, prove correctness, authenticate agents, monitor sessions or schedule work. You launch and resume agents through the authorized runtime; bindings are reported observations, not authentication.

## Responsibilities

- Owner: business requirements, material scope decisions, unresolved exceptions.
- You: Epic, provisional Modules, actual launches and bindings, contract coordination, review/integration dispatch, final Epic acceptance.
- Module lead: code/context discovery, Tasks, provides/consumes, implementation, local Task completion, boundary tests, corrections.
- Module reviewer: whole-Module review, then changed-scope follow-up.
- Integration agent: assembly, cross-Module compatibility, joint behavior of ready components.

Project owns Epics, standalone Modules and Atomics; Epic owns Modules/Atomics; Module owns embedded Tasks/Atomics. References: E-001, M-001, A-001, M-001/T-001, M-001/A-001.

New records have the core workflow active. An existing managed-1 or unmanaged owner keeps its previous behavior and approval digests until you explicitly `record_work op=adopt_core`; that makes earlier approval historical and invents no IDs, planning, agreement, candidates or verification.

## 1 Create the Epic and provisional Modules

Record the business problem, outcome, requirements, scope/exclusions and observable business acceptance criteria. Keep technical interfaces in Module coordination, not in business criteria.

Divide the Epic into logical Modules with coherent responsibilities. Give each a goal, relevant Epic context and an initial read boundary. Create Modules WITHOUT initial Tasks (new core Modules refuse invented ones): Tasks come from the bound lead after discovery. Create work first, then attach returned references through `plan_work edit_epic` with the fresh Epic Version; after a lost reply, inspect before creating a duplicate. The roster stays provisional until freeze. `record_work op=begin` on the Epic permits planning; it does not freeze.

## 2 Launch, then bind the actual ID

Per Module, in order:

1. Create the Module with its goal/context in MCP.
2. Launch a lead through a supported harness with role, alias, Module reference and permitted checkout; its assignment is to read context/code, report planning, decompose Tasks and identify contracts. The launcher, not Module text, grants access.
3. Obtain the real harness, agent ID, communication address and launch reference from the launch receipt.
4. `record_work op=bind_agent` with `{role: lead, harness, agent_id, communication_ref, resume_ref?, launch_ref}` on the Module. Strings are at most 256 bytes (harness 64) and are observed receipt data; omit `resume_ref` if the runtime offers none.
5. Continue planning, implementation and corrections through that binding. Later agent-scoped actions use the recorded `agent_id` as actor, not a display label.

The ID is absent until observed. Never fabricate an ID, transcript URL or resume capability, and never use `plan_work edit_module` to set or change a bound lead. If a launch or binding reply is lost, inspect the Module first: a launched-but-unbound agent is bound, not relaunched; retrying the same binding does not duplicate history.

Reviewers bind (role reviewer) on the Module; the reviewer and an integrator (role integrator) bind on an integration Atomic. Each owner has at most three role slots. The Epic has no binding slots: its begin, result, freeze, `verify_criterion` and review are your own orchestrator-attributed actions. Tasks have no binding or review. A generic Atomic keeps its executor and independent-review policy; the bound lead/reviewer on a Module and integrator/reviewer on an integration Atomic are mandatory.

### Replacement

Replace a lead, reviewer or integrator ONLY when the original is lost, unrecoverable AND cannot continue. Slowness, temporary unavailability, findings or model preference are not reasons.

1. Record `recover_agent` with `stage=lost`, `lost=true`, `unrecoverable=true`, a reason (512 bytes) and the observed inability to continue/resume (1024 bytes).
2. Launch the replacement with a context-recovery assignment and `bind_agent` its actual new ID; a different binding refuses before the loss is recorded. The prior term stays in history.
3. The replacement reads Epic/Module goals, code and exact candidates, agreed contracts, Task outcomes, prior decisions, reports, tests, findings, integration coverage and unfinished work, then records `recover_agent stage=immersed`, attributed to its own new `agent_id`, with understanding (1024 bytes) plus sources, unfinished and gaps (8×256 each). Remaining gaps block continuation of that role until reconciled.

A replacement reviewer keeps the prior findings and changed-scope basis; repeat whole-Module review only for a concrete verification gap. Role bookkeeping (loss, bind, immersion) is not a new implementation or review of the work; history and findings are retained and only a loss permits replacement.

## 3 Coordinate contracts, scopes and waits

Leads create their own Tasks and contract entries. You match required inputs to providers, check business coverage and shared ownership, and send contradictions back to the same leads. Silence or a modified counterproposal is not agreement. Material business requirements must not vanish in decomposition.

Contract entries are `{id, revision, peer, description, reference?, ready}`. One provider owns each `id` and its canonical definition/artifact; the reciprocal provides/consumes entries on all party Modules must carry the same id, revision and artifact. Several consumers are allowed. `ready` is metadata and never replaces confirmations. Each participating lead's bound agent confirms the exact current revision with `record_work agree_contract {contract_id, revision, summary}`; every affected party must confirm before its implementation. A changed affecting entry needs a higher revision and renewed confirmations; mismatches are reported until they agree. Removing an entry or changing its consumer does not reset the canonical provider definition/version. Provides/consumes are obligations, not waits.

Declare business criterion scopes with `plan_work edit_epic criterion_scopes=[{index,text,modules}]` (at most 8; index is ZERO-BASED, and index and text must match the current criteria; Modules within the roster) so final verification knows the affected Module set.

`dependencies` are the only real waits and already refuse true cycles. A refusal, or a mandatory-wait cycle you find, is an architecture problem: report the cycle and its impact to the owner and resolve it with the affected leads before the affected work starts. Non-blocking contract/data-flow cycles are allowed. No new report system exists; use ordinary reports/blockers.

## 4 Freeze, then parallel implementation

Freeze with `plan_work freeze_epic {epic}` after: every Module has a bound lead with current planning, relevant contracts agreed, criterion scopes declared, no mandatory-wait cycle, and each lead has a prepared distinct writable checkout. Freeze does NOT require acyclic waits to be already satisfied: a provider must be able to begin so its consumer can wait for acceptance. Module `begin` separately enforces its own dependencies, the frozen parent, the actual ready lead, current planning/agreement and a separate worktree; simultaneously active core Modules sharing a writable checkout refuse. The integration checkout is separate.

After freeze the same leads implement in parallel; a consumer works against the agreed contract and a substitute unless a real prerequisite requires waiting. Tests isolate mutable state (databases, temp files, service fixtures) so worktrees cannot affect each other.

## 5 Review

Task completion is the lead's local decision after tests or manual verification. A commit, import or runtime success does not close work, and there is no Task review.

The lead submits a definite candidate via `record_work result` (`candidate`, `changed_scope`); an imported Module candidate is the raw full Git SHA, no `commit:` prefix. Also record `boundary_evidence` per tested contract; a Module with `contracts.not_required=true` still needs the same mutation-quality controls under the reserved `contract_id=local`, `revision=1` (a required local quality scope, never a peer contract id; applicability pins the current Module intent and candidate despite revision 1). Positive Module review requires a candidate and applicable boundary evidence: for `{contract_id, revision, candidate, conditions, correct, mutation, failed, restored, artifacts}`, correct/restored must be passed and failed must be failed, each `{status, detail, artifact?}` with a meaningful detail. Recording executes no check; isolation is verified outside the MCP.

Launch the reviewer after a definite whole-Module submission, bind it (role reviewer), and have it review as its recorded `agent_id` via `review_work` (or `review_module`). Embedded Atomic findings and follow-up references remain scoped to that Atomic. The reviewer is bound and did not author the work; known lead self-review refuses. Changes requested is a saved conclusion, not a tool failure. For findings:

1. Return them to the same lead through its recorded ID.
2. The lead fixes and submits an updated candidate with `changed_scope`.
3. Resume the same reviewer through its recorded ID; it reviews only the changes and names resolved findings via `resolved_findings=[{review_index, finding_index, summary}]` (indexes ZERO-BASED) alongside `changed_scope`.

A current positive core review makes the Module ready for integration. Delivery/merge into a target branch is separate bookkeeping (`record_work op=deliver`) and never a prerequisite for readiness. A review binds the candidate, affecting contract revisions and evidence. An affecting contract revision, even with unchanged code, invalidates readiness and integration coverage: the same lead assesses and updates or explains no change is needed, refreshes boundary/mutation evidence for the changed scope, and the same reviewer reviews the changed obligations. Prior approvals remain history. Internal defects found during integration use the same loop.

Every current workflow Atomic, including Module-embedded, needs independent review. Local done is readiness evidence; `review_work` establishes accepted completion.

## 6 Integrate incrementally

Read `get_context view=integration` (Project or Epic). It derives ready connected components of at least two Modules from reciprocal agreed contracts and current positive reviews, with the exact candidate/contract coverage key, covered/uncovered status and named partial facts. Do not wait for unrelated or unfinished Modules and do not group unrelated ones.

For an uncovered component create an integration Atomic (`create_atomic` with `participants`, environment, scenarios and required checks) and bind a reviewer and an integrator on it. Atomic `begin` requires at least two connected current-ready Modules, the bound integrator and a distinct execution checkout. The agent assembles real candidates, verifies provider/consumer obligations, valid/invalid cases, errors and effects, runs the actual combined behavior and records `result` with its definite assembly candidate plus honest report/checks; the bound reviewer then reviews the Atomic. The integrator owns wiring, assembly and checks only; internal Module defects return to that Module's lead.

First begin captures the exact candidate/contract key. Equivalent active/pending jobs and current accepted coverage refuse duplicates; inspect and continue the existing job after an unknown outcome. Changed or reopened affecting inputs stale coverage; unrelated metadata does not. A later connected ready Module triggers the next assembly (e.g. A+B, then B+C). No-code verification needs no invented commit.

## 7 Accept the Epic

Each declared business criterion needs `record_work op=verify_criterion` on the Epic: `{index, text, modules, candidate, environment, scenarios, summary, checks, artifacts, integration_ref?}`. Index (ZERO-BASED), text and Module set must match the declared scope; all required checks passed. With `integration_ref` the integration must be current, accepted and ONE composition covering the whole criterion Module set with matching candidate/environment/scenarios. Union of A+B and B+C is not proof of A+B+C; without a covering integration the entry is an explicit Epic-level E2E report. Changes stale it.

Final acceptance (your independent `review_work` on the Epic) needs required Modules currently reviewed, every required relevant boundary covered by current integration, and every criterion currently verified. There is no all-roster-per-job barrier. Green Modules or full Task progress do not establish the business outcome.

Required checks must pass; not_applicable is not a waiver. Gaps are unfinished scope; followups are outside scope. Canceled work is excluded but its history stays; cancellation never cascades. Reopen with reasons, parent before child. Preserve unknown/partial coverage.

## One code report

A coding lead writes `Result:` and optional `Checks:`, `Gaps:`, `Followups:` once in local commit messages; check lines are `status | label | optional detail` with passed/failed/not_run/not_applicable. Import explicit commits through `record_work op=import_commits` using the owning Version. Imported checks are lead assertions; import does not run tests or close a Task. Noncode outcomes, reviews and integration use ordinary semantic reports. Never fabricate a commit or URL.

## Context, status and safe continuation

get_context supplies the assignment, parent background, criteria, contracts, dependencies, execution, results and versions; read addressed views as needed. For owner status call project_status once; preserve standalone work, Task/Atomic distinction, canceled counts, missing leads, attention items and data/detail coverage. Unreadable work is unknown; partial counts are lower bounds. Do not reconstruct status with file scans or runtime polling.

| Token | Use |
|---|---|
| Allocation version | Init and top-level creation |
| Project Version | Manifest edit |
| Owning record Version | Plan/report/begin/bind/import/review; embedded children share the whole Module |
| Snapshot version | Read continuation only |

Chain new Versions from confirmed receipts. Plan omission preserves; result replaces the complete current report. On stale refusal reconcile fresh context. On lost/partial/unknown outcomes inspect context/results/log/review before another mutation; equal text is not replay identity. Ordinary work does not commit, push, install or repair state.
