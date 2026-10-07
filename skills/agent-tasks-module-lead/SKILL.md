---
name: agent-tasks-module-lead
description: "Lead one assigned portable Module with a persistent identity: report discovery, plan its Tasks and boundary contracts, implement in an isolated worktree, prove boundary tests, import results once and fix review findings. Not for project coordination or self-review."
---

# Lead one Module

Governing target: `docs/epic-core-plan.md`. Live tool descriptions are authoritative for exact shapes and versions; if an operation named here is missing, the implementation has not landed; tell the orchestrator instead of improvising.

Your assignment is a project alias, Module reference and permitted checkout. Load it with `get_context(project,ref=M-001)`; do not require a second delegation brief. You are one persistent lead for planning, implementation and corrections; keep your Module context warm. The orchestrator binds your actual runtime identity after launch (`bind_agent`). Use that recorded `agent_id` as your actor on scoped actions, never a display label, and never invent or change an ID.

Every business call uses the alias. Stored content is English; MCP generates references/dates/activity. Source/docstrings own implementation; tracked reports own intent/results; Git retains history. Follow the repository's coding/testing/Git rules. The MCP does not run tests or authenticate identity.

## 1 Discover and plan (before coding)

Read the Module goal, Epic context, criteria, contracts/dependencies, execution `{repository,worktree,branch,target_branch}`, reports and Version, then inspect your permitted code. Your first familiarization is part of this assignment. Report it with `record_work op=planning {responsibility, scope, exclusions, read_refs, uncertainties}` (responsibility/scope ≤1024 bytes; lists 8×256): the responsibility and boundaries the code supports, what you read, and open uncertainties.

After planning you create and refine Tasks with `plan_work add_task/edit_task` as the bound lead; the orchestrator does not supply them. A Task is a bounded implementation unit with criteria and local verification, not a reviewed deliverable per edit. Declare `provides`/`consumes` contract entries and real `dependencies` through `plan_work edit_module` (it cannot change the bound lead). Report missing/conflicting obligations or scope to the orchestrator; do not design a neighbor's internals or let a business requirement vanish in decomposition.

## 2 Agree contracts

A contract covers interface/protocol, input/output meaning, valid/invalid inputs, expected results/errors and effects, with one canonical definition or artifact reference per boundary. Entries are `{id, revision, peer, description, reference?, ready}`: the provider owns the canonical definition and each party's reciprocal entry must carry the same id, revision and artifact. `ready` is metadata only. Provides/consumes are obligations, not waits; use `dependencies` only for real mandatory waits.

Negotiate with neighboring leads through the orchestrator until the interaction, including the checks that demonstrate compatibility, is agreed. Silence or a counterproposal is not agreement. Confirm with `record_work op=agree_contract {contract_id, revision, summary}` as your bound agent; every affected party must confirm before its implementation. Changing an affecting entry requires a higher revision and renewed confirmations, updated Tasks/artifacts and explicit impact handling; deleting/re-adding it or changing its consumer cannot reset the provider definition/version; independent work continues. A true mandatory-wait cycle is refused by `dependencies`; if a refusal or your own analysis shows one, record a blocker and report it as an architecture issue instead of waiting.

## 3 Implement in isolation

Start only after the Epic is frozen, your contracts are agreed, your own dependencies allow `begin` and you have a distinct prepared worktree. Never share a writable checkout; the Module begin refuses a conflicting one. Isolate mutable test state (databases, temp files, service fixtures). A consumer uses the agreed interface and a substitute that reflects the contract and must not reimplement the neighbor's business algorithm or persistence; a passing substitute does not prove composition.

Implement the next coherent Task. You verify locally by tests or manually and decide when it is done. There is no independent Task review; a commit, import or runtime success is not an automatic completion.

For code, write the outcome once in the commit message:

```text
fix(storage): preserve the current file on stale writes

Result:
Stale replacement refuses without overwriting the current record.

Checks:
passed | stale write | cargo test core_store_stale
passed | manual check | Inspected the refusal and recovery text

Gaps:

Followups:
```

`Result:` is required; `Checks:`/`Gaps:`/`Followups:` optional. Check status is explicit (passed/failed/not_run/not_applicable); missing is not passed. Import actual local commits with `record_work op=import_commits` (child ref, current owning Version, commits, your actor); repeated repository+SHA does not duplicate. Do not rewrite the summary by hand. Import preserves state; when you judge a Task complete, set state=done or use `record_work op=complete`. Manual verification is valid; report only what was actually checked. Noncode work uses `record_work result`, a COMPLETE replacement: keep every current check/artifact. Explicit required checks stay requirements; not_applicable never waives one.

## 4 Prove the boundary test

Your central test exercises the public input/output contract: for a contract mapping A to B, identical relevant conditions produce B from A. Control initial state, time, randomness and external responses when they influence behavior. Determinism does not imply statelessness or idempotency.

A green test is not enough. Prove it detects a real violation:

1. Run the agreed boundary cases on the correct implementation; observe the expected results.
2. Introduce a controlled meaningful mutation or fault that violates an input/output obligation (wrong output mapping, accepted invalid input, wrong error behavior, missing promised effect).
3. Run the same test; observe its intended failure.
4. Restore the implementation; rerun the control and observe success.

Do not mutate the assertion or unrelated setup to manufacture a failure. Mutate only inside your isolated worktree/state with no concurrent writes to those files; restore only your deliberate change, preserve unrelated edits, and submit only the restored candidate after the successful control. A mutation never enters another Module's candidate or shared integration state.

Record the result with `record_work op=boundary_evidence {contract_id, revision, candidate, conditions, correct, mutation, failed, restored, artifacts}`: conditions/mutation ≤1024 bytes; `correct`, `failed`, `restored` are `{status, detail, artifact?}` (detail required, ≤512) with observed statuses passed, failed, passed; artifacts 8×256. It binds your candidate, the current contract revision and your agent; recording runs nothing. A Module with `contracts.not_required=true` still requires these mutation-quality controls: use the reserved `contract_id=local`, `revision=1`. It is a required local quality scope, never a peer contract id; applicability pins the current Module intent and candidate despite revision 1. Keep useful internal tests too.

## Obstacles and plan changes

| Situation | Record |
|---|---|
| Agreed plan correction | Partial edit_task/edit_atomic/edit_module under Module Version |
| Cannot proceed | blocker with problem/needed_action/resolver |
| Stop or transfer | handoff with stopping_point/next_action |
| Resolved obstacle/handoff | clear with reason |
| Resume canceled/invalid work | Explicit reopen with reason |
| Cross-Task outcome/check | Module result; no manual accepted-state setter |

Cancellation does not cascade. Semantic changes/reopen stale approval and integration coverage. Handoff alone changes nothing. Keep the same Module identity; do not duplicate work to continue. Module Atomics (`M-001/A-001`) report begin after the Module begin; their local result/done is separate from the independent review the orchestrator arranges. Do not review your own work or disguise an Atomic as a Task.

## 5 Submit and correct

Check readiness: terminal Tasks, reviewed Atomics, explicit checks passed, no blocker/gaps, boundary evidence for each tested contract (or `local`). Submit a definite candidate with `record_work result` (`candidate` ≤256 bytes plus `changed_scope` 8×256, along with the complete current summary/checks/artifacts/gaps/followups) or a commit import, whose candidate is the raw full Git SHA with no `commit:` prefix. Any whole core Module review requires the definite submitted candidate. Embedded Atomic review uses its own retained findings, not unrelated whole-Module findings. The orchestrator obtains the independent review by the bound reviewer; do not supply a fabricated reviewer identity.

For changes requested, you (the same lead, via your recorded ID) read the retained findings, fix the named scope, update evidence and resubmit with the new candidate and `changed_scope`. The same reviewer follows up and names the findings it resolved by ZERO-BASED review and finding index; do not rerun unchanged broad suites solely for a new date, but keep mandatory project gates and test real changed behavior.

If a contract revision affects you, readiness is invalid even with unchanged code: assess the Module, change it or explain why no change is needed, refresh boundary/mutation evidence for the changed scope, then resubmit. Prior approval is history.

A positive review makes the Module ready for integration; merge/delivery is separate bookkeeping and not a prerequisite. The MCP never merges. You own Module internals: defects the integrator finds come back to you through the orchestrator, while the integrator owns wiring, assembly and checks only.

## Documentation and knowledge tools

When the live catalog lists `document_work` or `knowledge_work`, use them, not tracker YAML or file edits, for durable prose and reusable knowledge your task produces. The MCP commits each successful change to the documentation repository itself and reports one truthful Git outcome; that is separate from the Result commit report you write in your own source repository and import with `record_work op=import_commits`. Reads write nothing. A deferred or unknown commit keeps the saved bytes: inspect pending facts in `get_context`, never replay the business call, and use `git_recovery` only for explicit recovery with the exact pending version.

## Versions and unknown outcomes

All Task/Module/embedded Atomic writes use the owning Module Version. Serialize tracker writes within the Module; helpers handle bounded source work. Chain the confirmed receipt Version; refresh after other actors. Snapshot version is read pagination only.

On stale/busy refusal reconcile returned context. After lost/partial/unknown writes inspect result/log/review/source inventory before retrying; do not replay to repair presentation. Preserve unknown/partial coverage; handles and last activity are not proof of a live process.

## If you are a replacement

An agent is replaced only when the original was lost, unrecoverable and unable to continue; the orchestrator records that and binds you. First reconstruct context: read Epic/Module goals, code and exact candidates, agreed contracts, Task outcomes, prior decisions, reports, tests, findings and unfinished work. Then record `recover_agent stage=immersed` as your own new `agent_id` with your understanding of what the predecessor did, what remains valid and what comes next, plus sources, unfinished and gaps. Gaps block continuation until reconciled. Role bookkeeping is not a new implementation; prior history and findings are retained.
