---
name: agent-tasks-module-lead
description: "Implement one assigned Module using the portable agent-tasks MCP: recover its context, work through Tasks, report outcomes/checks/artifacts, maintain blockers and handoffs, and return corrections for independent whole-Module review. Use for a Module assignment or lead resumption; not for coordinating other Modules or reviewing your own implementation."
---

# Lead one portable Module

## Your assignment and the tracker

Your entry point is a **project alias + Module reference**, for example
`project="product", ref="M-001"`, plus the assigned implementation checkout.
One persistent lead owns one Module. Read the stored assignment instead of
requiring the orchestrator to repeat it in a launch prompt.

agent-tasks preserves intent, current results, check reports, blockers,
handoffs and independent Module reviews. Project metadata and each Module's
embedded Tasks use machine-managed YAML; responses are compact English text.
Use the alias on every business call, keep tracked content English, and let
MCP generate IDs, UTC dates and activity. Do not edit generated fields manually.

Code/docstrings describe implementation; tracked reports describe outcomes;
Git holds committed history. The MCP does not run tests, agents or Git for
ordinary work, and does not verify artifact strings. Follow the assigned
repository's implementation, testing and Git instructions. Keep useful technical
Markdown alongside the project code without duplicating it into task narration.

The current model is **Project → Module → Task**. There are no Epic/Atomic
operations, Markdown/document search or document-writing tools. Tiny fixes may
need no tracked record; do not manufacture tracking work outside your assignment.

## Enter and choose the next action

1. Call `get_context(project=<alias>, ref=<Module>)` for the outcome, lead,
   derived phase, blockers/handoff, remaining acceptance conditions and Version.
2. Read `get_context` with `view="tasks"` for the actual Task list. Summary is
   not a complete Task list. Read a Task's `results` or `checks` when needed;
   Module results show previews. Use `view="review"` for correction findings.
3. Continue the next meaningful Task or Module outcome in the assigned checkout.
   Surface an unclear criterion or missing dependency to the orchestrator rather
   than silently expanding scope. Use `search` only to locate tracked work whose
   reference is unknown; it is lexical, not Markdown or source-code search.

| What happened | MCP action |
|---|---|
| Implemented a meaningful result | `record_work op=result`; summary, actual checks, useful artifacts and remaining gaps/follow-ups |
| Finished a Task | Same call with its Task ref and explicit `state="done"`; omitted state preserves its old state |
| Need a plan correction | Agreed `plan_work op=edit_task/add_task/edit_module` with the owning Module Version |
| Cannot proceed | Module `record_work op=blocker` with problem, needed_action and known resolver |
| Stopping or transferring work | Module `record_work op=handoff` with stopping_point and next_action |
| Obstacle or handoff resolved | `clear_blocker` / `clear_handoff` with a reason |
| Need to resume canceled/unfinished work | Reasoned `reopen` on the same target; canceled parents must be reopened first |
| Need to report Module progress | `project_status(project, module=<Module>)`; counts and last reports, not live runtime state |

## Report once, preserve the evidence

Write the substance: what changed, actual check outcomes, unresolved problems
and useful artifacts. Numbering, dates, phase calculation and recent activity
are MCP's work. Do not duplicate a result into a development log or restate
every routine tool call.

A result is a **complete replacement** of the current report, not an append
or partial patch. Omitted checks/artifacts/gaps/followups become empty. Supply
every current item you need to retain. A Task marked done needs a meaningful
result, but done alone does not prove the Module's required checks passed.
Report passed/failed/not_run/not_applicable honestly; do not substitute
not_applicable for an explicit required check.

Record Module-level checks or cross-Task outcomes at the Module ref, omitting
`state`. Do not copy all Task summaries into a second Module summary: status
already shows their outcomes. A Module with no done Task needs its own report
to establish delivery evidence.

Example `record_work` after using the actual Task ref and owning Module Version:

```json
{
  "project": "product",
  "op": "result",
  "ref": "M-001/T-001",
  "version": "COPY_THE_RETURNED_MODULE_VERSION",
  "actor": "Storage lead",
  "state": "done",
  "summary": "Documentation resolves correctly after directory relocation",
  "checks": [{"label": "relocation", "status": "passed", "detail": "Moved the test directory and read its context through both aliases"}],
  "artifacts": ["A real commit or report reference when available"]
}
```

Use actual evidence; omit an unavailable artifact instead of saving the example
text. A Task receipt's phase is the owning **Module phase**, not the Task state.

## Hand off the whole Module

Use remaining acceptance conditions to find open Tasks, blockers, in-scope gaps
or missing required checks. Once the tracked outcome is ready, hand the
orchestrator the project alias, Module reference, implementation artifact
locations and any relevant limitations. The tracker supplies the plan and
recorded evidence, so a duplicate assignment/report is unnecessary.

Do not review your own implementation or invent a reviewer identity. The
independent reviewer uses `review_module` for the whole Module; there is no
Task review and no manual Module status setter. An accepted current review
directly yields the accepted phase. There is no hidden PR/merge gate; follow
the actual repository's integration requirements and planned checks separately.

For changes_requested, read the retained review, correct the named findings,
update current evidence and return the same Module for another review. Reopen a
Task with a reason if its completion is no longer true. Semantic changes make
old approval historical; a handoff alone does not. Module cancellation cannot
cascade through open Tasks; ask the orchestrator to resolve the remaining scope.

## Stay within the returned versions

All Task/Module writes use the **owning Module Version**, including planning,
reports, blocker/handoff and review. Task writes share one file, so a helper's
write can stale yours. Serialize writes within your Module; use helpers for
bounded implementation work without competing tracker writes.

Chain the new Version from a confirmed receipt for the next write to the same
Module. Refresh context when another actor changed it. Snapshot version and
next offsets are for read continuation only; preserve the selected ref/view
and review_index. Keep data/detail coverage warnings when reporting progress.

On stale refusal, reconcile fresh context before writing again. On lost,
partial or uncertain outcomes, inspect context/results/log/review before retrying.
Do not replay a mutation just because its prose could not be rendered. Normal
work writes remain uncommitted; any Git action belongs to the repository's
authorized workflow.
