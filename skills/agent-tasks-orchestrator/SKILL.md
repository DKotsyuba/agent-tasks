---
name: agent-tasks-orchestrator
description: "Coordinate substantial tracked work through the portable agent-tasks MCP: discover projects, plan Modules and Tasks, assign one lead per Module, obtain owner-ready status and arrange independent Module acceptance. Use when orchestrating or resuming a registered project; use the module-lead skill when implementing one assigned Module."
---

# Orchestrate portable project work

## What the MCP provides

agent-tasks stores strategic intent, current agent reports, blockers, handoffs
and independent reviews in a portable documentation repository. Every business
call uses `project=<alias>`; `get_project_list()` discovers aliases without
requiring the agent to manage documentation paths.

The current hierarchy is **Project → Module → Task**. Each Module owns one
YAML file containing its Tasks and evidence. The MCP generates references,
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
| Create or change the plan | `plan_work`; Project/Module/Task creation and partial edits |
| Report an outcome or obstacle | `record_work`; current result, Module blocker/handoff, explicit cancel/reopen |
| Save whole-Module acceptance | `review_module`; an independent accepted/changes_requested verdict and retained findings |

Use live tool descriptions for exact fields and limits. Epics, Atomics,
document writers, documentation compaction, agent launching and ongoing
automatic Git are not implemented. Do not substitute invented tools or create
fake Module/Task wrappers for those deferred entities.

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

Acceptance requires terminal Tasks, no active blocker or in-scope gaps,
meaningful delivery evidence and every explicitly required Module/Task check
reported as passed. Canceled Tasks are excluded; without a done Task, the
Module needs its own result. `not_applicable` does not waive required checks.
A known lead cannot self-review; unknown attribution is not authentication.

For cancellation, first resolve unfinished Tasks, then cancel the Module with
a reason. There is no cascade. Reopen canceled work explicitly; do not create
a duplicate just to continue. Semantic edits and reopen make prior approval
historical. A handoff alone does not invalidate it.

## Versions and recovery

- Project **Allocation version**: `init_project` / `create_module`.
- Project **Version**: `edit_project`.
- Owning Module **Version**: all Module/Task edits, reports and reviews. Sibling
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
