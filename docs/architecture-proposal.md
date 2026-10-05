# Portable project strategy and task MCP — shared proposal v5

> Comparative draft, superseded as the current file-layout proposal. The chosen
> direction is now one YAML per Module; see [product direction](product-direction.md).
> The earlier one-Epic-file model below is background material for revision, not
> an accepted constraint. Earlier owner ideas and lifecycle rules are flexible
> starting points. Optimize agent productivity and one-call owner status before
> preserving an old gate, field or tool. No product behavior is implemented here.

Status: discussion draft. No proposed behavior is implemented, qualified or measured. The current repository is the standard Rust MCP starter exposing only `get_status`. Tool, field and file names below are proposals.

Legend for this comparative draft: [R] input assumptions of the original design exercise, now revisable · [D] proposed default · [O] possible product trade-off · [U] unknown, needs a spike or measurement.

## 1. Boundaries

[R] Requirements:
1. Rust, family MCP template, compact English agent text through strict embedded MiniJinja; human content keeps its language; no raw JSON, no repeated manuals.
2. Portable, human-editable Markdown + YAML. Markdown = documents. YAML = structured work and the glue that links documents. A Project is the tree root.
3. A store is any root opened like a workspace. No global folder. Several roots, moves, clones and standalone documentation repositories stay practical.
4. Project > Epic > Module > Task; Modules also directly under Project; Atomic under Project/Epic/Module. ONE YAML per Epic holds its requirements, acceptance, Modules/Tasks/Atomics, statuses and results. Long prose is referenced Markdown. No file per Task, no database.
5. Baseline lifecycle retained: Backlog/Todo/In Progress/In Review/Done; Tasks finish after local checks with no independent review; whole Modules are reviewed; PR and reported merge are artifacts; start top-down, finish bottom-up, no cascades; Epic Module membership freezes at first start and is never unlocked; standalone Modules may wait for an Epic; integration Atomic across completed Modules; rework invalidates stale result/review/integration; cancellation carries a reason.
6. One persistent lead per Module; the lead receives a reference and takes its assignment from the MCP.
7. The MCP automates numbering, dates, transitions, scoped Git commits, assembled results and status.
8. One call returns an owner-ready report with Russian labels that the agent relays without rewriting; it shows what Modules actually reported as done, not only what they intend.
9. One-call scoped retrieval, bounded output, truthful continuation; no embeddings assumed.
10. Strategic documents stay understandable and free of accumulating boilerplate; explicit reviewed compaction may use agent-run; no silent destructive cleanup, no unreviewed write over changed text.
11. Authority: code and its docstrings own implemented truth; this store owns intent, requirements and work; Git owns history. NO direct source integration: nothing parses, indexes, reads or verifies source; commit/PR references are optional reported strings.
12. Trusted agents: ordinary integrity, concurrency and data-loss checks; no proof machinery, certificates, watchers, schedulers or orchestration platform. Reads never mutate or reopen work.

Out of scope by requirement 11: reading a source checkout, importing commit messages, verifying code or tests. Rejected as speculative infrastructure: daemon, database, search index, vector/graph/LLM ranking, per-Module branches or hunk-level commits, journal/transaction log, request-id operation metadata, opaque session handles, global project registry, a nested agent runtime inside the MCP, liveness polling, content certificates.

## 2. Capability areas (ordinary modules of one crate, not services)

| Area | Owns | Does not own |
|---|---|---|
| Store | opening a root, parsing, validation diagnostics, canonical writing, lock, IDs, dates, Git commits of store paths | any source checkout |
| Lifecycle | kinds, transitions, gates, rounds, basis tokens, derived Module/Epic reports | review verdicts, priorities, cascades |
| Context and retrieval | lead/reviewer packs, lexical search, exact document pages | ranking by meaning, code context |
| Status and attention | owner report, attention facts, one next action | health labels, liveness |
| Knowledge maintenance | document registry, derived currentness signals, compaction prepare/review/apply (post-MVP) | deciding what is obsolete, launching agents |

All writes go through one validation/write path. Presenters only lay out typed views; they compute no workflow (MCP_RESPONSE_STANDARD.md:35-43). General workflow lives once in two shared skills (orchestrator, lead) outside the MCP; tool descriptions are 1-3 line mini-docs; a reply carries at most one `Next:` (MCP_RESPONSE_STANDARD.md:179).

## 3. Data floor and portable layout

```
<root>/
  project.yaml          identity, essence, schema, policy, document registry, Project-level Atomics
  epics/E-001.yaml      ONE file per Epic: everything structured about it
  epics/archive/        optional home of finished Epics; still scanned for IDs and include=all search
  modules/M-004.yaml    a Module directly under the Project
  decisions/0003-slug.md
  docs/*.md             brief (when worth a document) and direction documents
  .agent-tasks/         self-ignored scratch: lock file, compaction proposals
```
Relative paths only; no machine-specific path in a canonical record. The same layout works as a private folder, a folder next to code, or a separate documentation repository. No global registry.

Minimum records (optional fields are omitted, never filled with placeholders):
- Project: id, title, `essence` (2-3 lines; grows into docs/brief.md only when worth a document — never both copied), schema, policy (`report_locale`), `docs` registry, Project-level Atomics. A Project has no work lifecycle.
- Epic file: id, title, essence, outcome, `requirements` (R1..; a requirement may name the Modules it is scoped to, otherwise it applies Epic-wide), `acceptance` (AC1..), status, round, `frozen_modules` (deliberately redundant with the nested map: lets validation flag a Module added by hand after the freeze), `modules`, Epic Atomics.
- Module: title, outcome, acceptance (own items or named Epic AC ids), planned check, status, round, one `lead`, `after` (Epic it waits for, standalone only), refs, `pr`, `merge`, `review`, optional submit `summary`, Tasks, Atomics.
- Task: title; outcome/acceptance (own, or named parent acceptance items); planned check; status; round; `result {text, checks, artifacts[], by, at, basis}` plus `affirmed {by, at, basis}` when an unchanged result was re-affirmed.
- Atomic: a Task-sized reportable unit with independent review (baseline), under Project, Epic or Module; an integration Atomic (Project or Epic level) also names >=2 participating Modules, scenarios, environment and the Module rounds/completion it covered.
- `work_type` (code by default; non_code; integration) only where it changes a gate.
- Any record may carry two small lead-written fields: `handoff` (one current note; older ones live in Git) and `blocked {reason, needs: owner|orchestrator|other, since}`. They are the data source for "resume" and "blockers".
- Nested position defines the parent (no parent_id). Timestamps are written by the MCP only when the event happened: created, changed, started, done (UTC RFC 3339). Results, checks, reviews and merges are trusted attributed reports.

Small, internally valid example of one Epic file (a standalone illustration, independent of the §7 scenario data; generated stamps, rounds, by/at and basis are left out here only to keep it short):
```yaml
# agent-tasks work file: rewritten canonically on every MCP write; comments are not kept
id: E-001
title: Portable review
essence: Review a project offline after moving its documentation folder.
outcome: A reviewer can open the moved folder and see the same work and documents.
requirements:
  R1: Work stays human-editable YAML and Markdown.
acceptance:
  AC1: Moving the folder keeps work and document references usable.
status: in_progress
frozen_modules: [M-001]
modules:
  M-001:
    title: Store opening
    outcome: Open the documentation root the caller names.
    acceptance: [AC1]
    check: Move-and-reopen scenario.
    status: in_progress
    lead: agent-run:ag-20260928-alice
    refs:
      - {doc: "docs/store-layout.md#Opening", role: requirement}
    tasks:
      T-001:
        title: Resolve relative document links
        acceptance: [AC1]
        check: Run the move-and-reopen scenario.
        status: done
        result:
          text: Relative links survive a folder move.
          checks: Move-and-reopen scenario passed (reported).
          artifacts: [commit 3e1c9aa]
      T-002:
        title: Report root and Git facts on open
        acceptance: [AC1]
        check: Compare the card with the fixture.
        status: in_progress
        handoff: Card template done; worktree fact still missing.
```
Counts, order and reports are derived; there is no second TODO list, no copied Task file, no stored owner report and no mandatory source path.

Documents — three kinds, no "implementation description" kind (code owns that):
- brief: purpose, boundaries, priorities; always pinned.
- decision: `decisions/NNNN-slug.md`, numbered by the MCP; first paragraph states the decision, then why, then rejected alternatives; `current` or `superseded by D-x`; never rewritten into a different decision.
- direction document: target design, requirements or plan too long for YAML. It describes intent.
Glue in YAML: registry entry `{path, kind, id (decisions), superseded_by?, retired?, purpose?, pinned?}` plus `refs` on work items: `path#section` with `role: requirement | related` (default related). A requirement reference is part of the assignment: it is inlined in packs and belongs to the basis (6.3). A related reference is a route. Title is read from the H1 and the summary from the first paragraph — neither is stored, so nothing drifts.
Currentness: explicit disposition (current / superseded / retired) is stored; the rest is DERIVED and worded as evidence, not verdicts:
- "referenced by active work" / "pinned";
- "maintenance candidate: all referencing work reported Done" — never a claim about the code and never a permission to retire;
- "unreferenced" — only for direction documents; decisions, the brief and pinned documents are never flagged for lacking live Tasks;
- hygiene: broken ref; active work citing a superseded decision; oversize document; brief unchanged while N Epics closed.
When NOT to write a document: if it fits in outcome/acceptance/result it stays in YAML; a decision document only when a choice has a rejected alternative someone could plausibly retry; never restate what code and docstrings say.

## 4. Proposed MCP surface

Ten proposed tools; the starter's `get_status` (product identity and qualification) stays as it is and is a different thing from the proposed `project_status`. Common arguments: `store` = root path, optional when exactly one store was opened in this process (no opaque handle; after a process restart pass the root again); `actor` = caller reference for attribution, defaulted from launcher configuration when present. Flat argument objects, one purpose per tool. Outcomes follow MCP_RESPONSE_STANDARD.md:63-72. Three short local tokens appear in replies and are passed back where stated: `basis` (6.3), `expected` (a record's or document's own content, for prose replacement) and `members` (the set of direct child IDs of a parent, or of the document registry, for creation). They are concurrency/currentness tokens, not proofs.

| Tool | Purpose | Key inputs | Output and budget | Effect | Failure / continuation |
|---|---|---|---|---|---|
| open_project | Cold entry: bind a root and orient | root | Project card <=4 KiB: identity, essence, Git facts (repo, branch, worktree, HEAD, uncommitted store paths), Project and registry `members`, active Epics with one compact row per Module (status, lead, counts), attention facts, document signals, validation summary, one Next | read | `not_a_store` (no project.yaml: nothing created; Next names plan_work init) · PARTIAL when one work file is invalid (path/line diagnostics, coverage stated) · schema major newer than binary: exact file/document reads and diagnostics work, computed views are PARTIAL with stated coverage, writes refused |
| get_context | One-call work pack, or an exact document page | ref (work ID, decision ID, `path#heading`, `path:lines`), view=lead\|reviewer\|report, cursor | Work pack default <=6 KiB, hard 16 KiB (MCP_RESPONSE_STANDARD.md:183-192), with the record's `basis` and `members`; document page <=16 KiB | read | unknown ref -> Next: search · continuation bound to the document content token and range; a changed document refuses the cursor and names the restart route |
| project_status | Owner-ready report | scope (Epic/Module), view=report\|docs\|problems\|all, locale=ru\|en | Report body in locale labels (default from `report_locale`; ru for this owner), proposed hard cap 16 KiB; see below | read | any required fact omitted -> PARTIAL with named omissions and the narrowing call; never a silent drop |
| search | Find documents and work text | query, kind=decision\|doc\|work\|any, scope, include=current\|all, cursor | <=10 rows / 8 KiB; the top hit's section inlined when <=1.5 KiB | read | zero hits stated with the terms tried · cursor bound to scan revision + offset; a changed store refuses the cursor with the restart query (no silent re-rank, no skipped rows; MCP_RESPONSE_STANDARD.md:198-204) |
| plan_work | Create work in one owning YAML | parent, `members` (observed parent membership), nested items[] with caller labels; `init {title, essence}` only for a root without project.yaml (creates project.yaml only) | Acknowledgement: counts, label -> generated ID map, new `members`, same-titled siblings noted as a fact, one Next | write | `members_changed` -> nothing created, current child IDs listed, Next: get_context parent · frozen Epic -> refusal naming the standalone-Module route · busy |
| edit_work | Scoped field edit | ref, set{title, outcome, acceptance, check, refs, lead, after, handoff, blocked}, `expected` | Changed field names + new basis | write | `expected` mandatory when replacing non-empty prose; mismatch names the changed fields · requirement fields of In Review/Done records: "reopen first" |
| transition_work | One explicit transition with its dependent report fields | ref, to, and as applicable: basis, result, checks, artifacts, pr, merge, lead, reason, summary | Acknowledgement: new status, derived counts, Git outcome, one Next | write | refusal lists ALL unmet conditions at once; nothing written · a repeat is NOOP only when intent and round are identical; a different second result is refused, never a silent NOOP · `basis_changed` (6.3) · no cascades |
| record_review | Record an independent review fact | ref, verdict=accepted\|changes_requested, summary, findings, basis | Acknowledgement + outstanding closure step | write | only for In Review Module/Atomic/Epic; never changes status (baseline) |
| save_document | Create a document, or guarded replace of a document or one unique section | path or kind, body, section, expected, registry `members` when the call allocates a decision number, supersedes, link_to, role, purpose | Acknowledgement: path, decision ID, registry/ref changes | write | replace requires `expected`; existing path on create and ambiguous section are refused before writing · PARTIAL when a later effect fails after an earlier one (see below) |
| checkpoint | Commit pending store paths (failed commits, native or hand edits) | paths or scope, expected | committed / nothing_to_commit / failed(reason); subject generated from a structural diff of record IDs and fields; "origin unrecorded" for changes not made in this call | write (Git only) | invalid file -> refused with diagnostics; never `add -A`, push, merge, rebase |

Post-MVP: `prepare_compaction` (write: scratch proposal only) and `apply_compaction` (write) — §7.S9.

Uncertain outcomes. Local writes are observable; there is no request-id metadata and no operation journal. Known-ref writes: the caller repeats or inspects; current fields and round are compared before any replay is treated as NOOP. Creating writes cannot allocate twice blindly: plan_work needs the parent's observed `members` token (printed by open_project, get_context and every previous write, so it costs an argument, not a read). If a reply was lost, the retry's token is stale because new IDs now exist, so nothing is created and the reply lists the current children with the `get_context(<parent>)` route for reconciliation. The scope is the parent's child IDs only, so different Modules create Tasks concurrently without conflicts. Titles need not be unique. Initial creation is guarded by the absence of project.yaml. No reply hands out a recovery handle that no tool accepts.

save_document touches more than one file (the Markdown file, the registry in project.yaml, optionally a ref in a work file), so it is not one atomic replace. Its effects are ordered and each is guarded. A create that allocates a decision number carries the observed registry `members` token and registers the new ID with its intended relative path FIRST, as one guarded YAML write; then the Markdown file is created at exactly that path; then work refs are added. Membership has therefore already changed before any document bytes exist, so a retry with the old token cannot allocate a second number. A failure in between leaves a registry entry whose document is missing — an honest, repairable diagnostic: `get_context` on the Project registry gives the chosen ID and path, and the create is resumed at that known path under the current guards. A create at a caller-chosen path is guarded by the existence of that path (identical body = NOOP, different body = refused, use replace). A replacement preserves the original bytes under the 5.4 rules, writes the document, then metadata and refs.

If a later step fails after an earlier one took effect, the reply is PARTIAL and names exactly what was written and what was not — never a blanket "nothing written". With no operation journal, blind replay is not promised to finish the job: inspect the named path and registry; an identical already-applied step is NOOP, remaining steps are resumed against the current guards, and differing current content is refused.

Context pack rules (one short response must recover everything needed after context loss): fixed order = header with basis and members / Project essence / Epic essence and applicable requirements / outcome and acceptance / sections referenced with role=requirement, inlined while each is <=1.5 KiB and the pack is inside its cap / Tasks (current one expanded with check, handoff and its own basis; next few and last few as one line; the rest counted with a route) / current review findings — after a reopen the last changes-requested findings stay visible as rework guidance, marked as no longer a current verdict / blocked / related refs as routes with purpose and size / "Not shown" with exact routes / one Next. Room for required strategic excerpts is reserved before Task rows, so many children cannot crowd out the assignment. Nothing requires proof that a page was read; the trusted lead chooses further retrieval.

Owner report rules: every active Epic and Module is shown with its required facts — full ID, title, status, full lead reference (or "no lead assigned"), Task counts, blocker, outstanding closure step and the reported accomplishment line (or "no results yet"); "lead" is a recorded assignment, stated once as not being liveness; "no recorded change since <date>" instead of "idle"; "uncommitted changes (origin unrecorded)" instead of guessing an author; the accomplishment line is labeled as reported and derived from the current submit summary when present, otherwise from the first lines of the most recent current Task results — the intended outcome is shown separately as the Epic essence; no health label; an attention list of facts and exactly one `Next:`. The first and last lines are English agent control text (the first carries the full store commit ID); the body between them is the relayable report. Proposed cap 16 KiB, to be tested on the fixture. Proposed qualification envelope — an estimate, not measured capacity: 6 active Epics and 30 active Modules, of which 12 are in progress or in review. Inside the envelope every required row and fact is present. Beyond the cap the reply is PARTIAL: it names exactly what was omitted and the narrowing call; a report that dropped a required fact is never labeled complete.

Search rules: no index and no embeddings in MVP; files are scanned per call and latency is measured on the fixture [U]. Unit = Markdown section (heading path, line range) or one YAML record field. Rank: exact phrase > all terms > any term; heading > body; current before superseded/retired (hidden unless include=all, with a count of hidden hits); linked to active work first. Case-insensitive prefix matching softens Russian inflection; misses are measured before anything semantic is considered.

## 5. Writes, concurrency, numbering, dates, Git

5.1 Write path [D]. Short cross-process advisory lock at the store root (bounded wait; timeout = `ERROR busy; nothing written`) -> reload the owning file from disk -> check fresh-state preconditions and any required `expected`/`basis`/`members` -> apply the typed change -> validate the whole candidate (schema, unique IDs, references, lifecycle invariants) -> atomic replace (temporary file + rename) -> Git commit of that path -> unlock. The caller never resends an Epic snapshot; unrelated sibling Modules edited by other leads coexist because every write is recomputed on the latest file. Local filesystems only; network filesystem lock semantics are not assumed.

5.2 Stated limit. The lock is advisory: only cooperating MCP processes honor it. The reload immediately before the replace detects drift that already happened, but there is no atomic compare-and-swap against an editor that ignores the lock: an editor holding an old buffer can still overwrite a newer MCP write. Mitigation is ordinary: reload before hand-editing, checkpoint afterwards. An overwritten MCP change is recoverable from Git only if that version was actually committed; after a failed commit it is not. This is stated, not promoted into adversarial concurrency machinery.

5.3 YAML format [D]: canonical readable YAML for a documented subset — fixed key order, block scalars for multi-line text, non-ASCII written unescaped. Unknown keys are never discarded: they are preserved and re-emitted (older and newer binaries may share a store). A newer schema major is refused for writes; exact file and document reads and diagnostics still work, and any computed view over it is PARTIAL with stated coverage, never presented as a complete status under semantics the binary does not know. Comments and hand formatting are not carried: each file starts with a one-line header saying so, and diagnostics list comment lines that will not survive. Valid pre-normalization bytes are preserved in Git first (5.4). A store without Git refuses a write that would discard comments or unsupported representation and names the preservation route (convert them into fields, or put the store under Git); ordinary canonical files are written normally. Byte-preserving round-trip editing is deferred until real hand-editing usage warrants it. [U] the emitter/library must be qualified for block scalars, Cyrillic and stable order before the schema is frozen.
An invalid Epic file blocks writes to that file only and makes Project reads PARTIAL; other Epics keep working. Dependency errors still block the transitions they affect.

5.4 Git [D]. Git belongs to the store's enclosing repository only; no source checkout is ever opened.
- Clean target path: every MCP write is committed inside the same lock with `git add -- <path>` (new files) and `git commit --only -- <path>` (new-file/index edge cases still need qualification [U]). Other staged and dirty paths are untouched. Never `add -A`, push, merge, rebase, branch or history operations; hooks are respected.
- One Epic = one file, so history is honestly file-scoped. On a clean path each commit contains exactly one mutation because write and commit share the lock; no Module-scoped commit is promised in general and no hunk machinery exists.
- Target path already dirty at lock time (origin unrecorded: an earlier failed commit, or a hand/native edit): the file is validated, then those exact bytes are committed first as "adopt pending <path> changes (origin unrecorded)", then the mutation is applied and committed. The adoption commit is file-scoped and may contain several agents' or a human's changes to that Epic; it is local history, not acceptance, review or publication. If the selected path has a different partially staged version, automatic adoption is refused and the conflict named — staging intent is not erased; unrelated paths stay untouched.
- Adoption commit fails: when the dirty content is already canonical the write proceeds and the reply says "state saved; intermediate uncommitted versions are not preserved in Git; checkpoint pending" — a deliberate ceiling on history granularity so that a session unable to commit is not locked out after its first write. When the dirty content is not canonical the write is refused and the bytes stay untouched, because normalization would destroy the only copy.
- Commit fails after the write (hook, signing unavailable in a sandboxed session, index.lock): reply `COMMITTED T-005 done; saved, not committed: <reason>`; open/status list pending paths; `checkpoint` is the recovery. The domain write is never replayed to repair Git.
- Store without Git: reads and canonical writes work; open says "history unavailable"; compaction apply refuses.
- Separate documentation repository: simplest. Code repositories are never touched; "Task report saved; store commit X; implementation commit Y (reported)" are separate facts with no distributed transaction.
- Store next to code: supported. All agents of one team use the SAME live coordination root, independent of their code worktree; each clone or worktree copy is a separate snapshot and nothing merges them. open reports the actual root, repository, branch and worktree facts without treating a linked worktree as a problem (a docs-only worktree is a good setup); no checkout binding is inferred.
- Deployment guidance [D]: for multi-agent projects prefer a separate documentation repository (no code hooks on store commits, no store commits in code history, one obvious root); a folder inside the code repository and a docs-only branch with its own worktree are supported alternatives.
- Recovery: committed states are recovered with ordinary Git by a human or an agent; the MCP offers no history browsing in MVP and promises no history for edits that were never committed.

5.5 Numbering and dates [D]. Flat per-kind IDs within a Project: E-001, M-001, T-001, A-001, D-001. Allocated under the lock as max over ALL work files including the archive + 1; never recycled; canceled work keeps its ID; no global counter file. IDs survive an allowed move (a Module between an unstarted Epic and the Project). Two clones can allocate the same number; a Git text merge may succeed, validation then reports both locations and one record is renumbered explicitly with its references (a repair tool is deferred). Dates are UTC clock observations recorded by the MCP, not freshness proofs; freshness is content tokens, never mtime or a single updated_at. A hand edit is an unrecorded actor: no author or date is invented for it.

## 6. Lifecycle and gating

6.1 Gates (baseline retained). A refusal lists everything missing at once. "Current" is defined in 6.3.

| Kind | -> In Progress | -> In Review | -> Done |
|---|---|---|---|
| Epic | requirements, outcome and acceptance; freezes Module membership | children terminal with current results/reviews; result composed; current integration covering delivered Modules when several were delivered | current accepted review |
| Module | parent Epic In Progress, or awaited Epic Done; outcome + acceptance; a lead | Tasks/Atomics terminal with current results/reviews; reported PR (code) or artifact (non-code); report composed from child results | current accepted review + reported merge |
| Task | Module In Progress; outcome/acceptance; planned check | — | result + reported checks, optional artifacts |
| Atomic | parent In Progress if any; outcome + acceptance; planned check; integration: participating Modules Done with reported merge | result + reported checks | current accepted review |

Backlog and Todo are preparation states without gates. Reopen = explicit return to In Progress with a reason and a new round: the record's CURRENT result, checks and review stop being current, and an integration that covered it stops being current; earlier reports stay recoverable in Git. Cancel requires a reason; canceled children leave the denominator and are counted separately; a parent retires only when its children are terminal. No transition cascades; a completion that needs an ancestor reopened says so and the caller does it explicitly. A reported PR/merge is validated as data only; the MCP does not remotely verify that a PR exists or was merged.

6.2 Labeled departures from the old implementation [D]:
- Proposed removal from mandatory start gates: repository, branch, worktree, both contract fields, executor and session URL (previous workflow baseline). Avoiding source inspection does not itself require removing these fields. Planned contracts and assignment details remain useful optional work data; their requiredness is a separate workflow decision.
- Simplification: a Task or Module may satisfy "acceptance" by naming parent acceptance items instead of restating them.
- Dropped: native attachment provenance (previous native attachment layer), auto-created Runbook/Decisions documents (previous default document creation), check_only previews (a complete side-effect-free refusal replaces them), the Linear-specific Duplicate status (cancel with reason, optionally naming the original). The commit-message reader (previous code-commit import) is out of scope by requirement 11.
- Merged calls: Task Done carries result + checks; Module In Review composes its report; Module Done carries the reported merge. Review acceptance stays a separately recordable fact so a cold orchestrator can see "accepted, merge not reported".

6.3 Rounds and basis [D] — a stored accepted review must not silently approve changed requirements, changed results or another round; no proof that an agent read anything is demanded.
- `basis` is one short two-part token over REQUIRED inputs: own part (the record's planning fields and round; for a container also its children's status, round and result content) . inherited part (all applicable ancestor requirements — a requirement scoped to named Modules is inherited only there; the named parent acceptance items, or all of them when none are named; and the current bytes of sections referenced with role=requirement). It is a local currentness token, not a read-proof or certificate. Related references are not part of it and are not change-tracked in the first version.
- It is printed in the pack and in every write reply for that record. Required from the caller only when the call records a result, a submission for review, or a review — not for start, assignment, cancel, reopen, handoff, blocked or closure.
- Mismatch at submission: nothing is written; the refusal says which part changed and lists that part's current inputs (inlined when short, otherwise by exact route), and prints the current basis. No old snapshots are kept, so the exact changed field and its change date are given only when available (the MCP recorded the change); a guaranteed old/new diff is not promised. Recovery is one assessed retry; after context loss, one get_context. This is also how a persistent lead learns that direction changed.
- Stored with each result, review and integration record; integration also records participating Module rounds and completion. A result, review or integration is CURRENT only while the round and the basis it recorded still match.
- A container cannot be submitted for review while a child result, review or integration is not current; the refusal lists them. An unchanged result is re-affirmed without retyping: `transition_work(ref, to=done, basis=<current>)` on an already-Done Task records "result still stands" with its own actor, date and basis; it does not claim the checks were re-run and does not change their reported date. Otherwise the Task is reopened.
- A review whose required inputs changed is not reused: closing is refused with the changed part named, and the existing paths resolve it — a fresh record_review against the current basis when the work still stands, or reopen for rework.
- After Done, a later change is reported as a fact and never reopens anything. No historical manifests, no revalidate tool. Reads never change state.

## 7. Scenarios (illustrative data; explicit calls; `<full-store-commit>` stands for a full commit ID)

S1 Cold entry. Input: owner says "continue Atlas", root known from the repository's AGENTS line or the owner.
```
open_project(root="/private/atlas-notes")
OK project Atlas | schema 1 | /private/atlas-notes | members a17c
Git: repo atlas-notes, branch main, HEAD <full-store-commit>; uncommitted store paths: 0
Essence: keep project review usable offline; no hosted task backend.
E-001 "Portable review" in_progress | Modules 1/3 done | Tasks 7/9
  M-001 "Store opening" done | lead agent-run:ag-20260928-alice | Tasks 2/2
  M-002 "Owner report" in_progress | lead agent-run:ag-20261001-bob | Tasks 2/4 | blocked
  M-003 "Search" in_review | lead agent-run:ag-20261002-carol | Tasks 3/3
  A-001 "Integration check" todo | waits M-002, M-003
Standalone: M-004 "Docs compaction" todo | no lead | Tasks 0/0 | waits E-001
Attention:
- M-003 in_review: review accepted 2026-10-05; merge not reported
- M-002 blocked since 2026-10-04 (needs owner)
- T-006 in_progress: no recorded change since 2026-09-29
Docs: brief docs/brief.md; 6 decisions current; signals: 1 maintenance candidate, 1 oversize | members 7d21
Validation: ok (4 files)
Next: transition_work M-003 done merge=<reported merge>
```
Avoided: walking files, reading history, a workflow manual. A root without project.yaml answers `not_a_store`, creates nothing and names `plan_work init`.

S2 Find a decision (1 call, exact text when a second is needed).
```
search(query="hosted backend", kind="decision")
OK search: 1 hit; more=false; 1 superseded hit hidden (include=all)
D-003 current 2026-09-27 decisions/0003-plain-files.md:1-14 "Plain files, no service account"
  <whole section inline: decision, why, rejected alternatives>
```

S3 Plan and delegate.
```
plan_work(parent="project", members="a17c", items=[{kind:"epic", label:"e", title:"Owner reporting", essence:"…", outcome:"…", requirements:{R1:"…"}, acceptance:{AC1:"…"},
  children:[{kind:"module", label:"m", title:"Owner report", outcome:"…", acceptance:["AC1"],
    children:[{kind:"task", title:"Progress counts", acceptance:["AC1"], check:"compare with fixture"}, …]}]}])
COMMITTED plan in epics/E-002.yaml: 1 Epic, 1 Module, 4 Tasks | e=E-002 m=M-005 | T-010..T-013 | git <full-store-commit>
Project members now b93e
Next: transition_work E-002 in_progress (freezes Module membership: M-005)
```
If that reply is lost and the call is repeated with members="a17c":
```
ERROR members_changed project: nothing created
Children now: E-001, E-002 "Owner reporting" (created 2026-10-05T09:12Z), M-004
Next: get_context E-002
```
Then `transition_work(ref="E-002", to="in_progress")`; the orchestrator launches the lead with its own runtime tools and the whole brief is `store=/private/atlas-notes ref=M-005`; `transition_work(ref="M-005", to="in_progress", lead="agent-run:ag-20261005-bob")`. A Module wanted after the freeze is refused for E-002 and created under the Project with `after: E-002`.
Avoided: one call per row; a restated assignment essay; a blind second allocation.

S4 Lead entry and Task finish (the lead inspects code with its own IDE tools).
```
get_context(store="/private/atlas-notes", ref="M-005")
OK M-005 "Owner report" in_progress round 1 | lead agent-run:ag-20261005-bob | basis 9c1f.77aa | members 4e0d
Project: Atlas — keep project review usable offline; no hosted task backend.
Epic E-002 "Owner reporting" in_progress: R1 … | AC1 …
Outcome: show active work and remaining closure steps in one call.
Acceptance: AC1
Requirement ref (inlined): docs/owner-report.md#Layout
  <exact section text>
Tasks 0/4 done:
T-010 todo "Progress counts" | check: compare with fixture | basis 5ad2.9c1f   <- next
T-011 todo "Lead lines" · T-012 todo "Closure steps" · T-013 todo "Russian labels"
Related: D-003 current "Plain files, no service account" decisions/0003-plain-files.md (0.8 KiB)
Next: transition_work T-010 in_progress
```
```
transition_work(ref="T-010", to="done", basis="5ad2.9c1f",
  result="Counts of done/total Tasks match the fixture; canceled Tasks are counted separately.",
  checks="status fixtures: 6 passed", artifacts=["commit 3e1c9aa"])
COMMITTED T-010 done | M-005 Tasks 1/4 | git <full-store-commit>
Next: transition_work T-011 in_progress
```
The result text is typed once. It reappears unchanged in the reviewer pack, the report view (usable as a PR body), the owner status and the Epic roll-up.
If the orchestrator changed AC1 through the MCP meanwhile (so the change detail is available):
```
ERROR basis_changed T-010: inherited requirements changed; nothing written
E-002 acceptance AC1 (changed 2026-10-05): "<current text>"
Current basis: 5ad2.e410
Next: assess the change, then retry with basis=5ad2.e410
```
After a hand edit the same refusal lists the current inherited inputs without claiming which field changed.

S5 Reviewed Module closure.
```
transition_work(ref="M-005", to="in_review", basis="…", pr="https://example.invalid/atlas/pull/18")
COMMITTED M-005 in_review round 1 | report composed from 4 Task results | git <full-store-commit>
Next: reviewer: get_context M-005 view=reviewer
```
```
record_review(ref="M-005", verdict="accepted", summary="Meets AC1; fixtures reproduced.", basis="…")
COMMITTED review M-005 accepted (round 1) | status stays in_review | outstanding: reported merge
Next: transition_work M-005 done merge=<reported merge>
```
```
transition_work(ref="M-005", to="done", merge="PR #18 merged as 77c0d1e into main")
COMMITTED M-005 done | E-002 Modules 1/1 | git <full-store-commit>
```
Old flow: record result, record checks, record review, edit merge report, move — and a Module could sit In Review unnoticed. Here closure after an accepted review is one call, and until then open, status and context all print the outstanding step. `changes_requested` records findings; the orchestrator returns the Module to In Progress explicitly (round 2) and the findings stay in the lead pack as rework guidance. If AC1 was edited after the review was recorded, the closing call is refused naming the changed part; a fresh record_review against the current basis, or a reopen, resolves it. If a Task result predates such a change, submission is refused listing it; `transition_work(ref="T-010", to="done", basis="<current>")` re-affirms it without retyping.

S6 Owner asks for status — one call, body relayed as is.
```
project_status()
OK status Atlas: complete; locale=ru; store commit <full-store-commit>; relay the report body unchanged
# Atlas — статус на 2026-10-05 09:10 UTC
Хранилище: /private/atlas-notes · незакоммиченных изменений нет · структура: ок
Лид — записанное назначение, а не признак работающего процесса.

## E-001 Переносимое ревью — в работе с 2026-09-28 · модули 1/3 завершены · задачи 7/9
Суть: проверять проект офлайн после переноса папки с документацией.
- M-001 Открытие хранилища — завершён 2026-10-02 · лид agent-run:ag-20260928-alice · задачи 2/2 · ревью принято · слияние (по отчёту): PR #17
    Сделано (по отчётам): относительные ссылки переживают перенос папки.
- M-002 Отчёт владельцу — в работе · лид agent-run:ag-20261001-bob (назначен 2026-09-30) · задачи 2/4 · последнее записанное изменение 2026-10-04
    Сделано (по отчётам): T-003 — счётчики совпадают с эталоном; T-004 — лид показан как назначение.
    Сейчас: T-005 «Шаги закрытия модуля в отчёте»
    Блокер с 2026-10-04 (нужен владелец): «показывать ли отменённые задачи отдельной строкой?»
- M-003 Поиск — на ревью с 2026-10-04 · лид agent-run:ag-20261002-carol · задачи 3/3 · ревью принято 2026-10-05 · осталось: записать слияние и закрыть
    Сделано (по отчётам): поиск по разделам с точными ссылками.
- A-001 Интеграционная проверка — ожидает завершения M-002, M-003

## Вне эпиков
- M-004 Сжатие документации — к выполнению · лид не назначен · задачи 0/0 · результатов пока нет · ждёт завершения E-001

## Требует внимания
1. M-003: ревью принято, слияние не записано.
2. M-002: блокер ждёт решения владельца.
3. T-006 (M-002): в работе, записанных изменений нет с 2026-09-29.

Не показано: 2 эпика в бэклоге; подробности завершённых задач — project_status scope=E-001 или view=all
Next: transition_work M-003 done merge=<reported merge>
```
Avoided: tree walking, hand-counted progress, translation, rewritten prose.

S7 Paused work, resumed with a fresh context. Before pausing: `edit_work(ref="T-005", set={handoff:"template done; fixture for accepted-not-merged case missing"})`. A new session calls `get_context(store, ref="M-002")` once and receives the T-005 row with its check, that handoff, its basis, the blocker and — when a required input changed after a result — the fact "T-003 result is not current: required inputs changed". No transcript, no orchestrator restatement.

S8 Documents become stale. The open card shows one signal line; details in one call:
```
project_status(view="docs")
OK docs Atlas: 14 registered; 3 signals
docs/store-opening-design.md — maintenance candidate: all referencing work reported Done (M-001, 2026-10-02); not a statement about the code
decisions/0001-hosted-sync.md — superseded by D-003 but cited by M-004 (todo)
docs/brief.md — unchanged while 2 Epics closed
Next: get_context docs/store-opening-design.md
```
Whether text is obsolete stays an agent/owner judgment. In MVP the fix is a reviewed hand edit through `save_document` (`expected` guards a concurrent change) or a superseding decision.

S9 Compaction proposal meets a concurrent edit (post-MVP increment; not claimed as built). The MCP launches nothing.
1. `prepare_compaction(scope="docs/")` — write effect, scratch only: creates `.agent-tasks/proposals/c-01/` with a manifest of candidates, their base content tokens and signals; replies with a brief and the path.
2. The orchestrator delegates through agent-run. The delegate writes proposed files into the proposal directory and records per file: rewrite / merge into X / retire, the reason, and where durable "why" moved (a decision). Limiting its write access to that directory is a launch option where the host supports it.
3. `get_context(ref="proposal:c-01")` — bounded review table (action, bytes before/after, removed headings, affected refs, base check) plus a review token; exact diffs by continuation. Document bodies never pass through tool arguments, so reviewed text is written once.
4. Apply = pre-flight acceptance, not a transaction:
```
apply_compaction(proposal="c-01", reviewed="<review token>")
ERROR blocked: proposal c-01 not applied; nothing written
Changed after prepare: docs/store-opening-design.md (uncommitted changes, origin unrecorded)
Next: prepare_compaction scope=<remaining paths>
```
Pre-flight verifies every original against its base, and the manifest and proposed files against the review token; any difference writes nothing, and a smaller proposal is regenerated rather than a subset applied. After an accepted pre-flight: originals are committed first (Git required; without Git apply refuses "recoverable history unavailable"), then additions -> reference re-pointing in YAML -> retirements, then one commit. An I/O failure can still leave a partial apply: the scratch proposal and the commit of the originals are kept, the exact changed paths are reported, and nothing is rolled back over newer edits. Retiring a document still referenced by active work is refused unless its replacement route is named.

## 8. Real failure modes

| Failure | Behavior |
|---|---|
| Several copies of the store (worktrees, clones) used as if one | One live coordination root per team; open prints root/branch/worktree facts; copies are separate snapshots |
| Hand edit breaks a work file | Diagnostics with path/line; that file's writes blocked; Project reads PARTIAL with coverage |
| Editor overwrites a newer MCP write | Not preventable; recoverable only when that version was committed |
| Commit unavailable | Saved-not-committed reported; pending paths in open/status; checkpoint recovers; history granularity reduced and said so |
| Lost reply to a creating write | Retry carries a stale `members` token: nothing created, current children listed |
| Multi-file document save fails midway | PARTIAL naming what was and was not written; inspect the named path and registry; an identical applied step is NOOP, remaining steps resume against current guards, differing content is refused |
| Report or review against changed intent | basis refusal at submission; evidence whose required inputs changed is not current and cannot close |
| Lead reference without a living process | Shown as recorded assignment plus last recorded change; never "running" |
| Accepted review, forgotten merge | Outstanding closure step printed by open, status, context and the review reply |
| Duplicate IDs after merging clones | Validation names both locations; explicit renumber |
| Mixed binary versions on one store | Unknown fields preserved; newer schema major -> writes refused, computed views PARTIAL with coverage |
| Status larger than its cap | PARTIAL with named omissions and narrowing call; never labeled complete |
| Search misses (inflection, synonyms) | Prefix matching; honest zero-hit reply; measure before adding anything |
| Epic file keeps growing | Views stay bounded; results stay short, long reports become Markdown; finished Epics may move to the archive |
| Direction documents drift into describing code | No implementation document kind; "maintenance candidate" signal; compaction distills durable why into decisions |
| A green check mistaken for usefulness | Acceptance below is scenario-based on a fixture, not unit-test counts |

## 9. First version, deferred, owner choices

Minimum useful first version: the store (open any root, validation, canonical writer, lock, IDs, dates, each-write commits with adopt-first, checkpoint); the baseline lifecycle with rounds and basis; the ten tools of §4; ru and en report templates; lexical search and exact pages; decisions with supersede; derived document signals; guarded single-document save; two shared skills. Compaction in this version is a reviewed manual workflow.

Deferred: prepare/apply compaction (first increment after MVP), independent compaction groups and subset apply, a no-Git preserve-originals mode, byte-preserving YAML editing, an `on_checkpoint` commit policy (only if hook or call costs are measured), change notes for related references, renumber/repair tool, history browsing and deltas through the MCP, disposable index or semantic ranking, liveness adapters, integrated compaction launch, multi-host coordination, archive automation.

[O] Owner choices — optional lifecycle simplifications; the baseline stays the default and none needs deciding now:
1. `record_review(changes_requested)` also returns the work to In Progress with a new round (one call less per rework; departs from "a review never changes status").
2. An Atomic nested in a Module is covered by the Module review instead of its own review.
3. Code Modules in a repository without a remote: accept a branch or commit range as the review target instead of a reported PR.

Open standards question: family.toml:13 says `state = "none"`, which would misdescribe a product that persists canonical work files. Proposed direction [D]: in-process + local durable state in user-selected portable roots with a per-store schema, external programs = Git. AGENT_MCP_STANDARD.md:81 defines `local` as the product's own durable state; :305 lists a product-home `state/` directory only for the local profile and :576 packages `local` with the product's state schema — neither says where all local state must live, so the delivery and schema mapping for portable stores needs explicit design. The installer must never migrate unopened user stores. No family.toml edit is part of this exercise.

## 10. Observable usability acceptance (scripted on one fixture store with Russian content at the §4 envelope; counts of calls, typed text and bytes — no savings percentage is claimed)

1. Cold orchestrator: a correct owner report after at most two calls (open_project, project_status) and no file reads.
2. An agent given only {root, ref} states the acceptance and the correct next action after ONE get_context; the requirement section is inline; the pack is inside its cap.
3. Task finish is one call; its result text is found unchanged in the reviewer pack, the report view and the status; it was typed once.
4. An Epic with Modules and Tasks is created by one plan_work call that returns every ID; repeating the call with the old `members` token creates nothing and lists the current children.
5. After a recorded accepted review, Module closure is one call; before it, open, status and context each print the outstanding step.
6. The Russian report body contains no English label, lists every active Epic and Module with full IDs and all required facts, shows reported accomplishments, never calls a lead running, and is within the cap — or is PARTIAL with named omissions.
7. A decision question is answered in at most two calls; superseded decisions are hidden by default and counted.
8. Two MCP processes interleaving writes to different Modules of one Epic file lose no update; on clean paths each commit holds one mutation; a forced commit failure yields "saved, not committed" and checkpoint recovers it.
9. Finishing with an old basis after a requirement change is refused naming the changed part and the retry succeeds; a Module with a non-current child result cannot be submitted until it is re-affirmed (one call, no retyping) or reopened; an accepted review recorded before a change of a required input cannot close until a fresh review is recorded; an edit of a related document changes nothing.
10. A hand edit adding a comment, an unknown field and an invalid line: diagnostics name path/line; other Epics are still served; after the line is fixed the unknown field survives the next write and the comment bytes are in an adoption commit made before normalization.
11. A store change between two search pages yields a stale-continuation refusal and no skipped row.
12. (post-MVP) Editing an original after prepare makes apply write nothing and list the drift.

## 11. Design review points

This proposal is a starting point for product discussion, not an accepted
implementation specification. It preserves the source-authority boundary and
reduces repeated planning and reporting. Its larger scenarios still require a
working prototype before any productivity claim.

Before implementation, resolve these tradeoffs explicitly:

- Automatic commits of already-dirty files are a proposed default, not approved
  product behavior. They preserve pending bytes but can include edits from several
  authors within one Epic file. An explicit checkpoint is the alternative.
- Canonical YAML formatting simplifies the writer but removes comments and custom
  layout. Git recovery is not the same as retaining comments in the current file.
  Human editing expectations must be settled before choosing the writer.
- Currentness and concurrency tokens should remain internal bookkeeping returned
  by tools. The initial API must demonstrate that callers can perform ordinary
  work without additional reads or manually assembling revisions and reports.
- The ten proposed work/document tools are additional to the starter's diagnostic
  `get_status`; retaining that tool means eleven tools in the initial catalogue.
- Compaction must be fully designed even if implemented after the first version.
  Its apply path cannot promise a multi-file atomic transaction. Recoverability,
  review of candidate bytes and preservation of current references remain required.
- One live coordination root per active team is a convention within a portable
  store model. It must not turn into a globally fixed folder or an inferred binding
  to a source checkout.
- Gate reductions and removal of a distinct Duplicate status are proposed workflow
  simplifications, not consequences forced by moving off Linear. Keep them labeled
  for discussion alongside the retained lifecycle. The non-code Module merge gate
  also needs a precise applicable/not-applicable rule before the schema is fixed.
- The proposed response caps and fixture sizes are design assumptions. Measure
  actual output size, latency and agent interaction on representative records.

## 12. Primary references

These references support specific technical facts and selected design ideas,
not claims that the proposed product has been implemented or qualified.

- [Rust file locks](https://doc.rust-lang.org/std/fs/struct.File.html#method.lock):
  behavior toward processes that do not hold the lock is platform-specific.
- [Git commit](https://git-scm.com/docs/git-commit): explicit path selection records
  working-tree contents for selected tracked paths; the new-file and existing
  index cases need qualification for the actual implementation.
- [YAML 1.2.2](https://yaml.org/spec/1.2.2/): comments are presentation details and
  mapping keys must be unique.
- [MemoryCustodian](https://github.com/waittim/MemoryCustodian): selective explicit
  routing and bounded project context.
- [Keep the Why](https://github.com/oliver-zehentleitner/keep-the-why): proportional
  retention of rationale and rejected choices.
- [IWE](https://github.com/iwe-org/iwe): Markdown reachable through editor, CLI
  and MCP interfaces; an LSP graph is not required by this proposal.
