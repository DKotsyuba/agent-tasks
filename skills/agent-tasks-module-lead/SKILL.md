---
name: agent-tasks-module-lead
description: "Implement one assigned portable Module with a persistent lead: load its assignment, execute Tasks, import meaningful results once, maintain blockers/handoffs and return corrections for independent acceptance. Not for project coordination or self-review."
---

# Lead one Module

Your assignment is a project alias, Module reference and permitted checkout. Obtain the work through `get_context(project,ref=M-001)` rather than requiring a second full delegation brief. One persistent lead owns the Module through Tasks, helpers and review corrections.

Every business call uses the alias. Keep stored content English; MCP generates references/dates/activity. Source/docstrings own implementation; tracked reports own intent/results; Git retains committed history. Follow the repository's coding/testing/Git rules. The MCP does not run your tests or authenticate declared identity.

## Recover the actual assignment

Read outcome/criteria, parent intent, provides/consumes obligations, dependency waits, execution `{repository,worktree,branch,target_branch}`, start conditions, current reports/review and owning Module Version. Parent criteria are background; explicit requirements/check labels belong to their owner and are not automatically inherited.

Use the tasks view for the full child plan; results/checks/review for addressed evidence/corrections. Summary may omit details. Contracts describe who supplies/uses which behavior; they do not automatically make every neighbor a prerequisite. Report missing/conflicting obligations or scope to the orchestrator rather than silently designing neighbor internals.

New workflow Modules use `record_work begin` after conditions hold. It records reported start, not runtime permission or an agent launch. Your checkout/tool access comes from the actual launch configuration. Legacy absent/unmanaged-workflow records retain previous semantics until explicit begin opts in; use actual live descriptions and returned conditions.

## Execute and decide Task completion

Implement the next coherent Task in your assigned scope. The lead verifies it locally with tests OR manual verification and decides when finished. There is NO independent Task review; do not invent one or treat runtime succeeded/commit/import as an automatic completion event.

For code, write the meaningful outcome once in the commit message, including real observed checks and known gaps/followups:

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

Use Result: and optional Checks:/Gaps:/Followups:. Check status is explicit; missing/not_run is not passed. Import one/multiple actual local commits with `record_work op=import_commits`, child ref, current owning Version, commits and your declared actor. Source messages/identity remain retained; repeated canonical repository+SHA does not duplicate history. Do not rewrite the same summary manually.

Import alone preserves state. Once YOU judge the Task complete, explicitly use state=done on that import/result or `record_work op=complete` on its current meaningful report. Manual verification is valid when appropriate; report what was actually checked, without inventing command output or a new test run. Follow source-repository commit instructions; report-source and Task closure authority are different facts.

Noncode work can use ordinary `record_work result` with summary/checks/artifacts/gaps/followups. A result is COMPLETE replacement, not a patch: include every current check/artifact that still matters. Required Module acceptance checks remain requirements even when a Task is locally done. not_applicable never waives an explicitly required check.

## Embedded Atomics, obstacles and corrections

Module Atomics use `M-001/A-001` and remain in the same file. After the Module begins, explicitly report begin at the Atomic ref before importing or recording its local outcome/completion. `add_atomic` requires title/outcome; executor/checks may be declared. Its meaningful result/local done is separate from independent review. Every current Atomic, including Module-owned, goes to an independent reviewer through the orchestrator. Do not review your own work or disguise it as a Task to bypass its policy.

| Situation | Record |
|---|---|
| Plan needs an agreed correction | Partial edit_task/edit_atomic/edit_module under Module Version |
| Cannot proceed | blocker with problem/needed_action/resolver |
| Stop or transfer | handoff with stopping_point/next_action |
| Resolved obstacle/handoff | clear with reason |
| Resume canceled/invalid work | Explicit reopen with reason; parent must allow it |
| Current cross-Task outcome/check | Module result, no manual accepted-state setter |

Cancellation does not cascade. Semantic changes/reopen stale approval and invalidate reported delivery/integration applicability. Handoff alone does not change implementation. Keep the same Module identity and report actual corrections; do not duplicate work just to continue.

## Submit the whole Module

Check current acceptance conditions: terminal Tasks, reviewed Atomics, meaningful delivery evidence, explicit checks passed, no blocker/gaps. Task progress is not whole-Module acceptance. Return implementation artifacts/limitations to the orchestrator; it obtains an independent whole-Module review, using review_module or review_work. Do not supply a fabricated reviewer identity.

For changes requested, read retained findings, fix the named scope, update evidence and return the same Module. Follow-up review covers changed code/findings. Do not rerun unchanged broad suites solely for a new date, but preserve mandatory project gates and test real changed behavior.

After implementation approval, the orchestrator handles any authorized actual delivery/merge and records `deliver` into target_branch. A local merge is sufficient; PR is optional. MCP does not merge itself. Matching delivery bookkeeping needs no second unchanged-code review. Source/Git/installation publication still follows actual user/repository authority.

An Epic/Project integration Atomic can name your Module. Actual composition is checked after participating Modules are accepted/delivered; reopening/relevant changes stale previous integration. Coordinate renewed evidence with the orchestrator. Verification without code changes needs no invented commit.

## Versions and unknown outcomes

All Task/Module/embedded Atomic writes use owning Module Version. Helpers may stale it; serialize tracker writes within the Module and use helpers for bounded source work. Chain confirmed receipt Version; refresh after other actors or before a new decision. Snapshot version is read pagination only; keep selection/review index unchanged.

On stale/busy refusal reconcile returned context. After lost/partial/unknown writes inspect current result/log/review/source inventory before retrying. Do not replay a mutation to repair presentation. Preserve unknown/partial coverage in your report; handles and last activity are not live-process proof.
