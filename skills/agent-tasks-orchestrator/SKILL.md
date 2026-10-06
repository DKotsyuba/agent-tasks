---
name: agent-tasks-orchestrator
description: "Coordinate portable Project/Epic/Module/Atomic work: discover aliases, plan proportional work, assign one persistent lead per Module, obtain one-call status and arrange independent Module/Epic acceptance. Use when orchestrating or resuming a registered project; use the module-lead skill when implementing one assigned Module."
---

# Orchestrate portable project work

## What the MCP provides

agent-tasks stores strategic intent, current agent reports, blockers, handoffs
and independent reviews in a portable documentation repository. Every business
call uses `project=<alias>`; `get_project_list()` discovers aliases without
requiring the agent to manage documentation paths.

The hierarchy is **Project → Epic, standalone Module or Atomic; Epic → Module or Atomic; Module → Task or Atomic**. Tasks remain leaves. Each Module owns one YAML file with embedded Tasks/Atomics and evidence. An Epic owns intent/criteria and authoritative member references; standalone Atomics have independent files. The MCP generates references,
UTC timestamps and recent activity, and returns compact English text instead
of raw YAML. Write project content in English; translate owner-facing status
when needed without changing its facts or coverage.

Code and docstrings describe current implementation. This tracker describes
intent and reported outcomes; Git retains committed history. Artifact strings
and lead handles are reported references, not verified Git contents or live
agent presence. Markdown technical documentation remains ordinary repository
documentation: current `search` searches tracked work, not Markdown or code.

Track substantial work that benefits from delegation, continuity or acceptance.
A self-contained microfix can use no records. Do not generate a Module, daily
log or duplicate documentation merely to show activity.

## Pick the tool by the decision

| Need | Call and expect |
|---|---|
| Product identity | `get_status()`; declared identity/qualification, not a workflow report |
| Discover a project | `get_project_list()`; aliases, purpose previews and availability |
| Create a documentation repository | `register_project(project, doc_dir, name, description, remote?)`; explicit local Git bootstrap and alias registration |
| Enter or resume work | `get_context(project, ref?)`; intent, remaining conditions and current write versions |
| Give the owner status | `project_status(project, module?)`; one formatted overview of tracked progress, leads, results, blockers and review |
| Find relevant work | `search(project, query, module?)`, then open the returned reference |
| Create or change the plan | `plan_work`; Project/Epic/Module/Task/Atomic creation, partial edits and Epic membership |
| Report an outcome or obstacle | `record_work`; current result, blocker/handoff, explicit cancel/reopen |
| Save whole-Module/Epic acceptance | `review_work`; independent verdict/findings; `review_module` remains Module-only compatibility |

Use live tool descriptions for exact fields and limits. Document writers, documentation compaction, agent launching and ongoing
automatic Git are not implemented. Do not substitute invented tools or create
fake work wrappers for deferred entities.

## Plan, delegate and close

1. Discover the alias if unknown; read Project context. Reading does not
   initialize storage. Register only when documentation creation is explicitly
   requested, using an absolute `doc_dir` with an existing parent. Registration
   initializes Git and commits bootstrap files, but never pushes. For an already
   configured uninitialized root, use explicit `plan_work op=init_project`
   with its returned Allocation version, then refresh Project context.
2. Create a small number of cohesive Modules with readable titles, outcomes,
   known leads and meaningful required check labels. Add Tasks only where they
   help execution; a Module need not have Tasks. Plan with `create_module` using
   the Project **Allocation version**, not its manifest or Snapshot version.
3. Give each Module one persistent lead. Delegate the project alias, generated
   Module reference, lead identity, assigned implementation checkout and source
   repository instructions. Have the lead use `agent-tasks-module-lead` and
   obtain the assignment through MCP; do not restate the stored plan.
4. On a status request, call `project_status` and present that output. Preserve
   unknown work, blockers and partial-coverage warnings. Narrow by Module or
   open a detail view only when the question actually needs it. Last reported
   activity does not certify that a lead is currently running.
5. When a Module is ready, give an independent reviewer its alias/reference
   and actual implementation artifacts. Read `tasks`, `results`, `checks` and
   `review` views as needed. The reviewer checks the whole Module; Tasks have
   no separate review. Save the actual reviewer's conclusion and attribution
   through `review_module` rather than accepting the lead's own report.
6. `accepted` immediately produces the derived accepted phase. There is no
   separate status move or hidden PR/merge gate. If the real project requires
   integration or another check before acceptance, plan that check explicitly
   and verify it outside MCP. Preserve `changes_requested` findings and return
   corrections to the same lead; changed evidence needs a new review.

Module acceptance requires terminal Tasks and Module Atomics, no active blocker or in-scope gaps,
meaningful delivery evidence and every explicitly required Module/Task check
reported as passed. Canceled Tasks are excluded; without a done Task, the
Module needs its own result. `not_applicable` does not waive required checks.
A known lead cannot self-review; unknown attribution is not authentication.

For cancellation, first resolve unfinished Tasks and Module Atomics, then cancel the Module with
a reason. There is no cascade. Reopen canceled work explicitly; do not create
a duplicate just to continue. Semantic edits and reopen make prior approval
historical. A handoff alone does not invalidate it.

## Versions and recovery

- Project **Allocation version**: `init_project` / `create_module` / `create_epic` / `create_atomic`.
- Project **Version**: `edit_project`.
- Owning Module **Version**: all Module/Task/embedded Atomic edits, reports and reviews. Sibling
  Task writes share it; another lead's Module has an independent version.
- **Snapshot version**: read continuation only. Keep project/ref/view/query
  and review selection unchanged and use the returned next offset.

Use the new Version from a confirmed write receipt for the next write to that
same Module. Refresh context after concurrent changes or before a new decision.
Plan-edit omission preserves data; explicit null/list clearing has tool-defined
meaning. A `record_work result` is a complete current report replacement:
retain every current check, artifact, gap and follow-up that still matters.

On stale refusal, read current context and reconcile the intent. On a lost,
partial or uncertain reply, inspect current context, log, review or inventory
before any new mutation. A version is not a request identity. Never replay a
creation or report merely to repair presentation. Markdown documents and
ordinary work changes are not automatically committed by these tools.

## Example: a compact plan

After `get_context(project="product")`, substitute the actual Allocation
version in this call; references and dates are allocated by MCP:

```json
{
  "project": "product",
  "op": "create_module",
  "version": "COPY_THE_RETURNED_ALLOCATION_VERSION",
  "title": "Portable documentation",
  "outcome": "Documentation remains usable after moving its directory",
  "lead": {"name": "Storage lead"},
  "tasks": [
    {"title": "Verify directory relocation", "criterion": "Both aliases resolve the moved documentation", "required_checks": ["relocation"]}
  ]
}
```

Delegate the returned Module reference, then report progress with
`project_status(project="product")`. Use real identities and observed evidence;
the example is a plan, not proof of implementation.

## Epic and Atomic coordination

Use an Epic when multiple Modules/Atomics share a meaningful outcome. Use a standalone Module when no such parent is useful. A small independently tracked verification or outcome may use an Atomic; a microfix still needs no record.

1. `create_epic` requires title/outcome/criteria; optional required_checks/lead describe its own requirements and attribution. Create Modules and standalone Atomics separately with fresh Project Allocation version.
2. Read Epic context and attach exact returned references with `edit_epic(epic, modules, atomics, version)`. Membership lives only in that Epic. Omission preserves, [] clears. Do not recreate work after a lost attachment reply: inspect inventory/context first. A separately created child remains standalone until attachment succeeds.
3. Membership is editable while open and changes invalidate acceptance. Move by guarded detach then guarded attach; there is no cross-file transaction or frozen/start ceremony. A Module or standalone Atomic belongs to at most one Epic. Parent criteria are useful assignment context; check labels are not implicitly inherited.
4. `create_atomic` requires title/outcome; optional executor, required_checks and participants configure independent work. `add_atomic(module, ...)` creates Module-owned work. Module Atomics use `M-001/A-001`, standalone Atomics use `A-001`; Tasks retain `M-001/T-001`.
5. Module Atomics are covered by whole-Module review, like Tasks. Standalone Atomics complete through `record_work result/state=done` with a meaningful result, all required checks passed and no blocker/gaps; do not invent a reviewer ceremony.
6. An integration Atomic uses `participants=["M-001", "M-002"]`, actual scenarios in required check labels/report details, and real reported artifacts when available. Result recording captures participant semantic bases/current review generations. A changed participant or new Module review makes completion evidence stale; rerun the real verification and refresh its result. Verification without code changes needs no invented commit.
7. Record a meaningful Epic result/own checks. An independent reviewer then uses `review_work(ref="E-001", ...)`. Acceptance requires noncanceled Modules currently accepted, Atomics currently done with fresh evidence, own required checks passed and no blocker/gaps. Relevant member changes/reopen or membership edits make previous approval historical.

Canceled parents block child writes. Resolve terminal child state before canceling a parent; no cascade occurs. Reopen parents before children with explicit reasons. Required checks must pass; not_applicable never waives them. Report acceptance applicability separately from historical reported completion.

Epic and standalone Atomic writes use their owner **Version**. Embedded Atomic writes use whole Module Version. Continue reads only with **Snapshot version** and unchanged selection. Preserve unknown/unreadable work, canceled counts, stale evidence, omitted detail and review attention when translating `project_status`; count each entity once. Do not scan files or poll runtimes to reconstruct owner status.

Existing 0.8.0 records are read without migration or rewrite. Expanded records and rewritten schema-1 fields require the new binary; rolling back a binary does not downgrade stored work. New/rewritten allocators use revision 2; missing/null counters refuse even with no published records. Only a legacy revision-1 allocator may start an unused new kind at one with proven empty inventory. Reads never recover lost counters, and ordinary planning/reporting does not automatically commit or push.
