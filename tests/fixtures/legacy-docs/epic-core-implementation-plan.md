# Epic core technical implementation plan

Implement the governing [Epic core workflow](epic-core-plan.md) by extending the existing portable record, semantic-basis and guarded-publication engine. Reuse the current purpose-based MCP tools, file layout, root lock, expected versions, compact typed views and bounded local Git reports. No new runtime, daemon, database or dependency is needed.

## Work boundaries

| Area | Implementation |
|---|---|
| src/model.rs | Optional CoreWorkflow with bounded runtime bindings/history, recovered context, agreed contract revisions, criterion scopes and meaningful boundary/mutation observations; current approval includes applicable contract facts |
| src/tools/input.rs and src/tools/work.rs | Closed operation shapes; explicit bind/recovery facts; discovery before coding; contract agreement and roster freeze; same-reviewer correction attribution; ready Modules before delivery |
| src/store.rs | Existing file/lock/version/publication path, related-observation validation, prerequisite cycle and candidate coverage checks; preserve backup/reservation/uncertain-outcome behavior |
| src/tools/read.rs | One Module assignment and one overview expose planning, bindings/recovery, agreement, review currency, boundary evidence and connected ready integration attention |
| tests/core_protocol.rs and existing core unit tests | Actual SDK/stdio persistence and negative controls for authority, versions, recovery, contract freshness and incremental assembly |
| Canonical skills, README and architecture.md | Exact implemented operation shapes plus role workflow; target and implemented behavior remain distinct until checked |

Source/docstrings stay with named callables and types. The core implementation has one writer; tests and prose have separate ownership. Parallel Module development uses separate managed worktrees and isolated mutable test state. Source writes and mutation restores cannot race inside a candidate.

## Lifecycle and evidence

1. Create business Epic and provisional Modules. Launch a planning lead before recording its observed harness ID. Missing IDs remain absent; binding is reported runtime receipt data, not authenticated live-runtime access.
2. The same lead reads code/context, creates Tasks and establishes public boundaries. Reconcile required provides/consumes through one canonical revision and affected-party confirmation. Reject actual mandatory-wait cycles; the orchestrator reports them as architecture issues and resolves them before affected development.
3. Freeze the Epic Module roster after agreement, immediately before coding. Beginning coding must require a prepared isolated checkout, usable persistent lead binding and agreed necessary boundaries. Freeze checks wait cycles but does not require every acyclic completion dependency already satisfied: a provider must be able to begin while its consumer waits. The consumer's own begin gates that dependency. Planning does not require a preassigned fake ID or frozen Task list.
4. The lead decides Task completion after tests/manual checks. Report imports alone do not close Tasks. At Module submission, retain exact tested candidate, contract revision, conditions and meaningful mutation failure/restored control. The MCP stores these observed reports; it does not execute tests or prove their correctness.
5. Independently review the Module; retain the observed reviewer binding. Current positive approval makes it ready for assembly without a merge/delivery prerequisite. An affecting contract revision stales approval even if code is unchanged. The same lead updates/confirms the work and the same reviewer checks changed obligations.
6. Derive ready connected components from current agreed boundary relationships. Integration jobs require at least two actually connected ready Modules, exact candidates/contracts, environment and joint scenarios. Preserve duplicate-job guards and stale/unknown coverage. No all-Epic roster barrier per job. Delivery remains a separate report.
7. Final Epic acceptance requires current required business criteria and actual applicable joint/end-to-end verification. Individual green Modules and disconnected passing subsets cannot replace the required composition. The orchestrator executes models; the MCP exposes durable conditions and coverage.
8. Replace any participant ONLY when the original agent is lost and cannot continue, with no recoverable session. Preserve previous identities/history, launch the replacement, bind its observed ID, and require a new immersion/context reconstruction report before continuation. Temporary unavailability or review findings never justify switching.

## Compatibility and checks

Preserve existing references, one YAML file per Module, absence/default semantics, old unmanaged records and read-without-migration behavior. Use explicit activation where old records lack new facts; never invent historical runtime IDs, agreement, mutation observations or context recovery. Keep exact owning-file versions and expected related candidates/contracts. Reports and histories must refuse capacity rather than prune human facts.

Verification: meaningful core regressions plus four public flows: lead-first planning/agreement/freeze; candidate+contract review and actual mutant/control observations; AB integration while C unfinished followed by freshness/final criterion checks; irrecoverable participant loss/replacement/immersion. Check closed serde/schema shapes and exported contract. Root runs cargo xtask check from the candidate worktree and validates both canonical skills with the approved interpreter. Independent review follows the full candidate; corrections receive changed-scope review from the same reviewer. A disposable live runtime receipt smoke checks launch-before-ID without modifying real/demo projects.

Local completion is a reviewed, committed candidate with commands/results attached to its exact source. Main merge, push, release, install and service activation remain separate authorized actions.

## MCP changes

Extend `Workflow` with an optional bounded `CoreWorkflow`. Newly created work activates the core; absent core preserves existing managed/unmanaged behavior and semantic digests. `record_work adopt_core` explicitly activates the new rules for old records, retaining historical approvals without fabricating missing facts.

Keep common project/ref/owning-version/actor arguments and the existing closed operation unions. Add actions within those tools rather than a new tool per lifecycle step:

| Operation | Purpose |
|---|---|
| record_work bind_agent | Record role, harness, observed agent ID, launch reference and supported communication/resume reference; refuse changing an existing available participant |
| record_work recover_agent | Record confirmed irrecoverable loss or the replacement's immersion report; history retained, unresolved context gaps block continuation |
| record_work planning | Bound lead records discovered responsibility/scope/exclusions, read references and uncertainties before coding |
| record_work agree_contract | Each affected bound lead confirms the exact canonical contract ID and revision; unilateral ready flags do not establish agreement |
| plan_work edit_epic criterion_scopes | Declare each business criterion's exact current index/text and affected Module set |
| plan_work freeze_epic | Freeze provisional Module membership only after required planning and agreement; begin no longer invents the discovery phase |
| record_work boundary_evidence | Record exact contract revision/candidate/conditions and meaningful correct-pass, mutant-fail and restored-pass observations |
| record_work result | Extend existing complete report with definite candidate and changed scope; explicit completion remains separate |
| review_work | Retain independent verdict/findings; add changed scope and finding resolution references for the same-reviewer follow-up |
| get_context integration view | Return ready connected candidate sets and current applicable/uncovered composition facts through bounded pagination |
| record_work verify_criterion | Record actual business/end-to-end checks over the declared exact current affected composition, optionally referencing one applicable accepted integration |

Contract entries retain provides/consumes peers and canonical artifact references and add explicit ID/revision facts. One provider defines a canonical boundary; multiple consumers confirm the same revision/artifact. The exact affected peer observations participate in approval currency. Integration Atomics retain their existing participants/environment/scenarios/review engine and add current composition facts.

`criterion_scopes=[{index,text,modules}]` must match the Epic criteria and roster. `verify_criterion` records that exact set, assembly candidate, environment, scenarios, summary, checks and artifacts. An optional integration reference must be one current accepted composition covering the full criterion set and its actual assembly context. Without a reference, the explicit Epic-level end-to-end report captures the current affected Module candidates/contracts. Pairwise edge coverage alone cannot satisfy a criterion spanning a larger composition.

Boundary observations use `{status,detail,artifact?}` and require passed/failed/passed controls for correct/mutated/restored behavior. A Module without cross-Module contracts uses the reserved quality scope `contract_id="local", revision=1`; `contracts.not_required=true` removes peer obligations, not the Module mutation-control requirement. Criterion, review and finding indexes are zero-based, matching existing review selection. Recovery loss records require both lost and unrecoverable declarations plus the observed inability to continue; a replacement's own immersion report retains sources, unfinished work and explicit context gaps.

The implementation and actual exported schema settle detailed field types and bounds; the canonical skills will use those implemented shapes after the public-flow tests pass.

## Observed local validation

The final local candidate passed `cargo xtask check`: structural checks, formatter, frozen all-target Clippy, 118 Rust tests (52 application, 11 real SDK/stdio, five transport, nine delivery, 26 presentation and 15 xtask), rustdoc and frozen product build. Explicit `cargo xtask contract check` and both canonical skill validators passed. The separate observed Agent Run receipt-record smoke passed with actual returned identities; it verifies binding/restart interoperability and does not authenticate actors or qualify live code correctness.

Independent review accepted all seven corrected findings, including canonical provider definitions across consumer/provider transfers. Real SDK scenarios exercise isolated mutation/control, candidate/revision-bound review, loss-only replacement/new immersion, incremental AB/BC work and actual applicable ABC business verification. Legacy managed-1 and literal 0.8 fixtures preserve read/explicit-write boundaries. Core stores at most 16 retained definition snapshots and preserves scoped review provenance; capacity refuses rather than silently pruning facts.

No dependency, embedded model runtime, daemon, installation, release, service change or real/demo project mutation was added. Local verification does not publish or install the binary or integrate a target branch.
