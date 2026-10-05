# Portable agent work and project intent — architecture draft

Status: proposed high/medium-level design. The repository contains the standard
Rust MCP starter only. Work storage, lifecycle, retrieval, Git automation and
document maintenance below are not implemented or qualified. This draft replaces
the one-Epic-file candidate; its historical form remains in Git.

## 1. Product contract and proportional use

Design for the agent: quick orientation, clear actions, less repeated writing,
fewer unnecessary calls and fewer mistakes. The owner's essential interaction is
ONE status call whose standardized Russian report can be printed unchanged.

Tracking is optional, not the default consequence of a coding request:

- A self-contained microfix can use code/docstrings, appropriate checks and an
  ordinary Git commit. No tracker call, new work item, document or development log.
- A tiny project may have no store. Reads never initialize one.
- Substantial work can be a standalone Module. An Epic is useful only for a
  meaningful shared outcome across Modules.
- Tasks are useful subdivisions, not compulsory wrappers around edits.
- A fix inside current work can reuse its Task or Module result. Do not invent
  another item or reopen completed work merely because the same source file changed.
- A durable decision can be one paragraph, without an Epic or a tracker.
- Do not create Runbook, Log, TODO or empty decision documents automatically.

Applicability guidance belongs in the shared orchestrator/lead skills and relevant
responses. No mandatory scope-assessment call, numerical size threshold, source
scanner, automatic log or separate routing system is needed.

## 2. Authority and supported scope

| Source | Responsibility |
|---|---|
| Code and detailed docstrings | Current implemented technical truth |
| Markdown and YAML work store | Intent, strategy, requirements, work and reported results |
| Git | Committed history of code and work, in their respective repositories |

This MCP does not inspect, parse, index, synchronize or verify source. Commit,
PR, checks and delivery references are reported facts, not independent proof.
Agents use coding tools separately.

Start with trusted agents and local filesystems. Several portable stores are
selected through configured aliases or explicit roots. A team uses one chosen live coordination root;
clones and worktree copies remain independent snapshots. There is no global
folder, inferred source-checkout binding or automatic synchronization.

MCP-only clients can manage work and Markdown. Implementing code requires the
agent's separate coding capabilities.

### Store aliases in TOML

The proposed user configuration is `~/.agent-tasks/config.toml`, selectable through
the standard explicit config option. It stores machine-local routing only, not
Project records or a second copy of business state.

```toml
schema_version = 1

[aliases]
alpha = "/work/project-alpha/docs"
private = "/private/project-beta/notes"
```

Every work/document call accepts `store="alpha"` instead of a repeated directory
path. For example, `project_status(store="private")` and
`get_context(store="alpha", ref="M-001")`. An explicit absolute root remains useful
for initial setup and an unregistered store.

Resolve the selector once at request entry and bind that request to the actual
root. There is no mutable server-wide current space. Unknown aliases return a
focused error with configured choices; missing directories never cause fallback
to another store or automatic initialization.

Observations and write locks belong to the resolved root, not the alias string.
Two aliases for the same directory share coordination; repointing an alias cannot
reuse an old observation against another directory. Alias/config changes apply
to subsequent requests, not the target of an in-flight operation.

Aliases are installation-local conveniences. The portable store retains relative
links and no machine-specific paths; moving it only requires updating its alias.
No extra register/open tool is required merely to use a configured alias.

## 3. Coherent responsibilities

| Area | What it provides |
|---|---|
| Portable store | Root selection, parsing, integrity, scoped writes, numbering, dates and Git checkpoints |
| Work cycle | Assignments, results, meaningful completion, Module review and useful handoffs |
| Context and retrieval | Project/assignment packs, lexical search and exact document sections |
| Status | Ready owner report with progress, accomplishments, blockers and coverage |
| Document maintenance | Proportional writing, link hygiene and reviewed compaction |

Ordinary Rust modules are sufficient. Do not add a daemon, database, scheduler,
search index or runtime platform without a demonstrated need.

## 4. File and record model

| Path | Owns |
|---|---|
| `project.yaml` | Identity, short purpose, store schema and optional project-level checks |
| `epics/E-001.yaml` | Shared intent, requirements, acceptance and Module references |
| `modules/M-001.yaml` | ONE Module, its assignment, optional Tasks/Atomics, results and review |
| `docs/*.md` | Optional strategy, plans and durable rationale |
| `.agent-tasks/` | Disposable coordination and candidate-edit scratch only |

An Epic's Module references are the parent authority. Do not also store a
competing parent field in each Module. Unreferenced Modules are visible as
standalone work; unattached files from a partial plan never disappear from status.

Module IDs are project-scoped; Task IDs are local to a Module, such as M-001/T-02.
A nested Atomic uses the same small work record as a Task. Project/Epic Atomics
are checkable work outside a Module, including useful integration scenarios.
Identity survives ordinary edits; allocation is coordinated and never based on
title uniqueness.

Minimum useful data:

- Project: title, purpose, schema. A long brief becomes Markdown instead of a copy.
- Epic: outcome, applicable requirements/acceptance, Module references, optional
  checks and explicit closure/cancellation.
- Module: outcome, acceptance, lead, optional Tasks, current results/checks, review,
  optional delivery facts, references, note and blocker.
- Task/check: title, optional specific criterion/check, open/done/canceled state
  and one attributed result. Inherit relevant Module criteria instead of copying.
- Report: meaningful result, reported checks or explicitly absent checks, author,
  observation date and optional artifact references.
- Document: ordinary relative path and optional section reference. No mandatory
  registry, generated document identity or taxonomy.

Counts, summaries, report drafts and activity are derived. Do not persist another
TODO list, copied owner report or independent development event stream.

## 5. Minimum work cycle

1. Plan or reuse a Module when tracking helps. Group Modules into an Epic only
   when the grouping communicates a shared goal.
2. Assign one persistent lead. Assignment is not evidence the process is running.
3. Give the lead only the store root and Module reference.
4. The lead gets its assignment, criteria, useful referenced excerpts, existing
   results and handoff in one call.
5. Record a Task or Module result once. Task start is not a mandatory extra call.
6. Review the complete Module independently. The reviewer can inspect locally
   complete work directly; a missing handover ceremony does not block review.
7. Accepted review closes the Module in the same call. Changes requested returns
   it to work with the findings.
8. Close an Epic when its intended outcome and relevant checks are satisfied.
   Integration checking is used where actual interactions need it.

Task states can be open/done/canceled. Module readiness follows its actual records:
planned, work recorded, ready/in review, reviewed complete or canceled. Epic
activity is derived; membership remains editable with scope changes visible.

Separate three facts: work reported complete, review accepted, delivery reported.
A PR reference does not imply any of them. Delivery or integration belongs in the
plan and acceptance when necessary, not as a universal hidden gate.

Missing checks say "not reported." Explicit unmet required checks cannot satisfy
their criterion. Partial results and blockers can still be saved without dummy
evidence. Reads never reopen work. Changes to actual reviewed scope/results make
an old approval non-current; unrelated later microfixes do not.

## 6. Proposed MCP interaction surface

Eight work/document tools, plus the starter's separate diagnostic `get_status`.
Names and exact schemas remain proposals; tool count is not a design quota.

| Tool | One useful intent |
|---|---|
| `project_status` | Owner report and cold project orientation |
| `get_context` | Project, assignment, reviewer or exact document context |
| `search` | Scoped lexical retrieval with useful excerpts |
| `plan_work` | Create or revise intent, assignments and membership |
| `record_work` | Results, checks, optional focus/handoff, blockers, cancellation and reopening |
| `review_module` | Independent verdict and Module closure/return |
| `save_document` | Guarded edit of one known Markdown path or unique section |
| `checkpoint` | Commit explicitly selected store paths |

Every work/document call names its store alias or explicit root. Short aliases
save repetition without an implicit current space; correctness does not depend
on one server process corresponding to one agent.

Context and write replies return ONE opaque observation reference (`obs`).
The caller copies it where needed; it never assembles hashes or revision maps.
The reference covers relevant observed inputs, not unrelated sibling Modules.
Review checks Module intent/results and required context against that observation.
A recorded approval identifies the accepted content internally and does not
invalidate itself by writing its own metadata.

Do not add per-Task proof fields, repeated reaffirmation records or read-page
certificates. A changed input returns a compact current view and assessed recovery,
not automatic permission to repeat stale reporting. Retained observations can work
after a restart when the relevant content still matches.

## 7. Agent routines and useful output

### Microfix

Inspect and fix code, update attached documentation where needed, check the result
and commit when authorized. Tracker calls: zero. New records/documents: zero.

A missing store reports "No tracked project at this root; nothing created."
It does not automatically prescribe initialization.

### Substantial project entry and delegation

`project_status(root)` returns purpose, active work, assignments, reported results,
blockers and coverage. Planning reuses existing work when possible.

`plan_work(..., obs=...)` returns confirmed work references. Delegate only
`{root, Module reference}`; do not repeat the specification in the launch prompt.

### Persistent lead, completion and review

`get_context(M-001)` returns acceptance, relevant sections, Tasks/results, note,
blocker and observation.

`record_work(M-001/T-02, result=..., checks=..., obs=...)` records the outcome once.
Its text supplies the Module report, reviewer pack and status. A small improvement
can be included in existing work rather than becoming another administrative item.

`get_context(M-001, view=reviewer)`, then
`review_module(M-001, accepted, summary=..., obs=...)` records reviewed completion.
Delivery facts remain separate. On interruption, write a useful handoff once;
the next session restores context through one read.

### Owner status

Illustrative content, not an actual project result:

> E-001 Переносимый запуск — модули 1/2 приняты; задачи 3/3 выполнены по отчётам.
>
> M-001 Хранилище — принят после ревью; лид назначен; задачи 2/2.
> Сделано: относительные ссылки сохраняются после переноса.
> Проверки: сценарий переноса пройден по отчёту.
> Доставка: PR указан; слияние не сообщено.
>
> M-002 Отчёт — готов к ревью; задачи 1/1.
> Сделано: сформирован единый отчёт о ходе работ.
> Проверки: не сообщены.
>
> Требует внимания: ревью M-002 и сведения о проверках.
> Покрытие: учтённые работы выбранного эпика. Микрофиксы вне трекера сюда не входят.

Use Russian owner labels, exact work references, known dates and assigned leads.
Do not claim liveness, checked code, full project activity or zero work from missing
records. Missing files or omitted required rows make coverage PARTIAL.

### Retrieval and document editing

Search returns relevant work fields/Markdown sections, short evidence excerpts
and exact routes. Scope and currentness filters reduce noise. Start with lexical
retrieval; measure misses before adding semantic search or an index.

After a document read, a native guarded edit plus checkpoint uses two calls and
may carry a small patch. A single `save_document` call edits/checkpoints a section
but carries its replacement text. Support both; neither always saves more tokens.
Continuations refuse changed source content rather than silently skipping results.

## 8. Writes, partial planning and Git

Use a short cooperative store write lock: reload affected files, validate relevant
references, compare the supplied observation, apply scoped changes and atomically
replace each file. Reads remain independent. No automatic merge of conflicting
prose and no protection claim against editors that ignore the lock.

A multi-file plan is NOT a transaction:

1. Create/update Epic intent when needed.
2. Exclusively create Module files and allocate identities.
3. Attach successfully created Module references.
4. Checkpoint confirmed changes.

Creation observations cover the relevant on-disk inventory, including children
that exist before a parent link is saved. Once a child exists, an old observation
cannot authorize another blind allocation. Similar content is not an identity key.

Partial outcomes name every saved ID/path, unattached Module and unfinished step.
After an uncertain reply, inspect context and continue using known IDs. Replaying
the original batch with a refreshed observation is not promised safe. Do not delete
saved work to simulate rollback or introduce a generic journal framework.

Managed writes may automatically checkpoint clean owned paths. Pre-existing dirty
content is preserved and reported as awaiting an explicit checkpoint. Preserve
unrelated staged/dirty paths and refuse conflicting staging of a selected path.
A failed commit does not undo a saved result; never replay the result to fix Git.

Commit only named store paths. No push, merge, history rewrite or source inspection.
Only committed versions have Git recovery. No automatic adoption of dirty text to
justify destructive YAML normalization.

Prefer stable readable YAML with unrelated fields/comments preserved. If the writer
cannot preserve a construct, refuse that file's mutation rather than silently drop
it. Writer/platform/Git edge behavior still needs qualification.

## 9. Documentation and compaction

Write only useful intent or rationale. No periodic log, mandatory decision bundle
or automatic aging. An unreferenced document is not automatically obsolete.

Compaction applies only to existing documentation:

1. Select a bounded scope, current text, observations and affected incoming links.
2. Preserve recoverable originals before removing/replacing material. Without
   committed originals, consolidation retains old material rather than deleting it.
3. An existing agent runtime may propose candidate patches outside canonical files.
   It names redundancy, proposed removals and where useful requirements/rationale stay.
4. Review actual candidate bytes and link changes. Changed originals or candidates
   require reassessment.
5. Apply guarded additions first, reference updates second, retirements last through
   native edits or individual saves.
6. Checkpoint selected changed paths. Interrupted apply names applied/pending paths;
   inspect and resume. Never silently roll back over newer edits.

Automatic prepare/apply tools can follow demonstrated need. The first version can
support this complete workflow using existing agent-run and file capabilities,
without embedding another agent runner or transaction platform.

## 10. First version and qualification

Smallest useful product: the Module-owned file model, proportional applicability
guidance, the eight proposed tools, independent Module review, lexical retrieval,
scoped Git and directly relayable Russian owner status. Two short shared workflow
skills orient orchestrators and leads; tool mini-docs describe purpose, effects,
required inputs, output and recovery with real examples.

Before implementation is called useful, exercise:

- zero tracking writes/documents for a tiny fix or an untracked-directory read;
- a meaningful Module without a boilerplate documentation bundle;
- one-call owner status and lead entry by root/reference;
- report-once completion and direct accepted-review closure;
- rejection of review against changed relevant inputs and visibility of missing checks;
- concurrent writes to separate Module files without lost updates;
- partial plans and edited-child lost-reply recovery without duplicate allocation;
- native and MCP document edits, Git failure and explicit checkpoint recovery;
- compaction interruption without discarded originals or broken active links;
- correct complete/PARTIAL coverage, with no invented runtime or microfix coverage.

Measure actual calls, bytes, latency and agent errors. Design examples are not
performance evidence. YAML preservation, filesystem locking, Git/index behavior,
portable store schema/delivery mapping and client compatibility remain unqualified.
No implementation, release, installation or service change is part of this draft.
