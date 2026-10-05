# Portable agent-tasks — structured architecture proposal

Status: discussion draft. All names, fields, budgets and examples below are proposed. No task-store implementation, migration, release or installation is part of this document.

## A. Purpose, authority and boundaries

Keep one useful work/context system, with strict YAML for known structure and optional Markdown for extended reasoning. A microfix or tiny project may use no store and zero tracker calls. A substantial standalone Module is sufficient; an Epic groups genuinely shared outcomes. The current executable remains the Rust starter, not this contract.

All stored documentation, semantic summaries, tool instructions, and examples are English. project_status supplies all status facts in ONE call; the orchestrator may translate its prose for the Russian-speaking owner. Translation preserves IDs, numbers, dates, states, omissions and uncertainty. No bilingual fields, second report, translation service or MCP language model.

Ordinary internal boundaries: portable store (routing, strict schemas, references, allocation, guarded writes and scoped Git); work (planning, results, review); knowledge (typed records); retrieval (bounded context/search/exact Markdown); presentation (typed views and strict embedded MiniJinja). These are responsibilities, not new crates or a platform. Code/docstrings own implementation; these records own intent and reported facts; Git owns committed history. No source reading, indexing, synchronization, mandatory symbols, runtime polling, daemon/database, Linear/Space integration, or automatic documentation expansion. This explicitly designs a future stateful profile; AGENTS.md:23 and docs/architecture.md:3 still describe the starter.

## B. Fixed records and field ownership

Each standard kind has one closed, versioned schema. R=required; A=required when applicable; O=optional. MCP generates store/record/item/criterion/log IDs, technical dates, revisions, completion stamps, attribution when available, and relationships implied by operations. Agents provide meaning and intentional references, including lead assignment. Unknown actor stays unknown; an explicit declared identity may be recorded as declared, without authentication machinery. Omitted edit fields preserve values; explicit clear applies only to optional values. Empty or absent checks mean “not reported,” not success. No lead means unassigned; absent alternatives are not recorded, run history never reported, reviewer unidentified and unmanaged Markdown metadata unknown.

References are typed same-store IDs, relative Markdown paths/sections, and optional reported artifact references. An Epic alone owns Module membership; a Module belongs to at most one Epic and may stand alone. No duplicate parent field. IDs survive edits; titles are not identities.

| Kind/home | Agent-supplied meaning | Generated/derived and lifecycle |
|---|---|---|
| Project / project.yaml | R title, purpose. O boundaries, priorities, expected checks, bounded glossary, useful references, remote, brief Markdown; checkpoint auto/manual (auto default). | Store identity/schema/dates; navigation and counts derived. Remote is descriptive, never a fetch or checkpoint target. |
| Epic / epics/E-001.yaml | R title and outcome. A requirements/acceptance. O ordered Module refs, shared context, checks, handoff. | Activity derived; explicit close/cancel needs result/reason. No freeze ceremony. |
| Module / modules/M-001.yaml | R title and outcome; O specific acceptance, lead{name, handle?}, child work, dependencies with needed outcome, context refs, delivery facts. Outcome is the minimum completion criterion. | ONE file contains all Tasks/Atomics, current references, log and reviews. Planned/working/ready/accepted/canceled; blocking is orthogonal. First meaningful work can establish working; no start call. |
| Task / Atomic, embedded in its owner | R title. O criterion, required checks, dependencies. Project/Epic checks can live in their existing files. | Open/done/canceled. Done records a meaningful result; cancel needs reason. No Task required for every edit. Work TODO is a projection of these canonical items. |
| Semantic log/result, embedded in owner | Closed entry variants: result{summary, checks?, gaps?, followups?, artifacts?}; handoff{stopping_point,next_action}; blocker{problem,needed_action,resolver?,waiting_on?}; unblock/cancel/reopen{reason}; delivery fact; meaningful note. | MCP stamps entry, target, actor/date. One canonical result entry; target/current views reference it. Materialized states are updated directly, not rebuilt by replay. Corrections append a superseding entry. Generated plan/review links reuse existing substance. No command diary or hidden operation journal. |
| Review, in Module | R verdict accepted/changes_requested and summary; O declared reviewer label when caller attribution is unavailable. A findings{text,must_fix}; optional result references addressing earlier findings. | MCP stamps reviewer, finding IDs and accepted semantic basis. Expected project/Epic checks never gate a verdict: unreported ones render 'not reported'. Only a check explicitly planned as required for a criterion or item gates acceptance, and a failed one cannot be accepted. review_module may carry reviewer-reported check statuses, so a forgotten report costs no extra round trip. An accepted verdict resolves earlier must_fix findings (recorded as resolved by review when no result addressed them); a finding the reviewer keeps open makes the verdict changes_requested. Accepted review closes directly. 'Expected' alone adds no gate; a check explicitly planned as required gates even when it is inherited from the Epic or project. The verdict checks current intent/results/required context, excluding its own metadata. Accepted is a dated fact: external supersession later signals applicability; own semantic drift is flagged, and managed scope change requires reopen. |
| Decision / knowledge | R question or adopted statement. A statement and rationale when adopted. O rejected alternatives with reasons, decider/resolver, scope, supersession, detail. | Open/current/superseded/retired. An open question can resolve in the same record. Supersession preserves predecessor and incoming-reference warnings. Recorded owner decision is context, never execution permission. |
| Runbook / knowledge | R purpose and ordered steps with description, typed command or instruction, expected outcome. A preconditions, inputs{name,meaning,type,required,default?}, working context and recovery. O pitfalls, scope and detail. Execution results can reference the runbook once. | IDs/step IDs/dates; current/superseded/retired. Last use is derived from those reports, not copied prose. It is computed from canonical work when the runbook is read; citing a runbook in a result never adds a shared-state write. Command is a typed {text, working_context/cwd?} value, not an argv/executor model; reading never executes it. Input names/types/required/defaults are explicit, without expansion. No secrets as input values. Last reported success is tied to a revision; edit date is not verification. |
| Research / knowledge | R question and findings{claim,basis,evidence/source when applicable}. O conclusion, sources, limitations/open questions, applicability, detail. | IDs/dates; current/superseded/retired. Basis distinguishes measurement, primary/secondary report and inference. Internal reasoning can lack external citation when labelled. No confidence score or automatic age expiry. |
| Procedural checklist / knowledge | R purpose and ordered item descriptions; O associated work/runbook and detail. Each instance uses open/done/canceled like work; procedural skip is canceled with reason, done needs a meaningful completion fact. | MCP item IDs and completion attribution/dates. Runbook reference records the version used. Checklist progress does not create Tasks or copy a work TODO. Counts remain separate; skipped never means done. |
| Markdown detail / docs | English flexible body plus intentional relative links/section targets; a managed standalone document has one purpose line for navigation. | MCP keeps identity/date/actor metadata in its referencing YAML detail entry (Project for standalone prose), not a fragile Markdown header or disposable cache. Native bodies remain exact; unmanaged metadata is unknown until managed save. Markdown cannot substitute for a standard typed kind. |

Checks carry criterion/label, status=passed/failed/not_run/not_applicable and optional detail/artifact. Missing required checks prevent accepted completion; marking not_applicable cannot silently waive planned acceptance. Gaps concern unfinished in-scope work; followups are suggestions outside scope, not automatically new Tasks. Findings remain canonical in review: a result can address their IDs; action context shows unresolved findings beside work. No Task per finding by default. Reviewers must act independently; a known reviewer matching the lead is refused. Unknown identity is reported honestly and does not gate the verdict or create an actor.

## C. Additional knowledge that earns its cost

| Candidate and real scenario | Existing home; payload/call burden | Recommendation |
|---|---|---|
| Owner question: cold orchestrator must not guess or re-ask | Open Decision + resolver; one question, no new kind/call beyond save_document | Merge |
| Standing rule/assumption: later agent repeats a rejected approach | Decision rationale/applicability; short premise/consequence, not a risk register | Merge |
| Procedural lesson: lead repeats known recovery failure | Runbook pitfall/recovery, or Decision for general policy; one short field | Merge |
| Dependencies: choose startable Module without rebriefing | Module needs ref + needed outcome; compact upstream result in context | Keep field |
| Expected checks: reviewer cannot interpret “missing” without expectations | Project/Module acceptance/check requirements; no copied check manual | Keep field |
| Gaps/followups: reveal limits without scope creep | Result optional lists, captured in same call; no backlog file | Keep fields; defer triage workflow |
| Persistent lead location: resume same lead and locate work | Optional opaque handle + artifact refs; no liveness integration | Merge |
| Orchestrator resume point: project coordination is interrupted | Same handoff schema at Project/Epic scope; one meaningful update | Reuse |
| Procedure usefulness: distinguish edited from actually tried | Runbook reference to reported execution, not duplicate narrative | Keep reference |
| Recent activity: answer “what changed?” | Derived semantic log dates, optional since filter; zero stored duplicate | Keep derived |
| Environment/resources/glossary: cold agent lacks orientation | Manifest references/constraints and optional term/meaning pairs; a few values, no new kind/call | Merge |
| Success measures: activity is mistaken for outcome | Existing criterion sentence and reported check detail; no additional measurement machinery | Merge |
| Incident register: a recurring outage needs its recovery steps | Result + reusable runbook; a register needs upkeep | Defer |
| Release ledger: “what shipped when” | Delivery facts and artifact refs on results | Defer |
| Estimate/deadline/priority fields: ordering work under time pressure | Epic order, dependencies, manifest priorities; unmaintained otherwise | Defer until a real scheduling decision needs them |
| Inter-Module contract records: a consumer needs the producer's target contract | Epic requirement or scoped Decision + Markdown; the implemented contract belongs to code | Reject kind |
| Source map, live-agent roster, cost/usage: “where is the code, who is running” | Code tools and the runtime own these; stored copies fabricate truth | Reject |
| Risk register, follow-up backlog platform: speculative lists | Decision rationale, blocker, result followups | Reject register; defer backlog triage |

## D. Semantic tool surface and mini-docs

Keep eight purpose tools plus independent diagnostic get_status. Every work call takes project=<local TOML alias>, or mutually exclusive root for bootstrap. Resolve once; bind locks/observations to the resolved root plus store identity. Same-root aliases coordinate; alias retargeting cannot reuse an old observation. No global current project and no read-created store.

| Tool | Use/input -> result/effects and recovery |
|---|---|
| project_status | Owner asks for progress: project, optional scope/since -> complete scoped facts in English, separate data/detail coverage, no obs. Read-only; invalid files mean partial data. No agent-side scans/polling. |
| get_context | Enter/resume/review or read known content: ref/view/section -> project or assignment pack, semantic record, TODO/log view, or exact Markdown slice; one obs and omitted routes. Changed continuation refuses, then reread. |
| search | Unknown reference: words, scope/kind/currentness -> ranked semantic-field/Markdown excerpts with directly readable references. Lexical first; no YAML grep/dump. |
| plan_work | Intend/create/change work: semantic Project/Epic/Module/child/checklist plan, assignments/membership, obs -> confirmed IDs and new obs. Explicit initialization only; partial result names saved/unattached/pending parts. |
| record_work | Report what happened once: target, relevant result/checks/state/handoff/blocker/delivery, obs -> one semantic save, current work/log/status/review reuse. A stale replacement returns changed facts, nothing saved and fresh obs; inspect uncertain effects before retry. An explicitly historical note may append without changing current completion. |
| review_module | Judge independently: Module, verdict/summary/findings, obs -> accepted closure or changes requested. Relevant changed context refuses stale approval. |
| save_document | Preserve reusable knowledge: one typed Decision/Runbook/Research or Markdown body/detail, optional existing ref and obs -> validated save, generated metadata, exact refs. No raw YAML or arbitrary field bag. |
| checkpoint | Recover Git or deliberately adopt named store edits: refs/paths and current observation -> commit outcome, including relevant valid machine-owned state. Saved work survives commit failure; no result replay. |

Mini-doc pattern: “Use when / Skip when / Give / Effect / Returns / Recovery,” with semantic field help and one real call. Policy budget, to be measured: each mini-doc about 700 bytes, all tool discovery about 8 KiB; field help lives in input-schema property descriptions, and a validation error names the field and lists the accepted fields of that variant so one retry suffices. Example: record_work is for a meaningful tracked outcome; skip untracked microfixes and routine narration. Give only applicable fields and the supplied obs. It saves one canonical result, updates work/log views, and checkpoints eligible paths. Reply names confirmed effects/new obs. On lost reply, get_context the target; never retry a creation or result merely to repair presentation/Git.

project_status mini-doc: Use when owner asks status; Skip when assignment context is needed; Give project/scope; Effect none; Returns English report with data/detail coverage; Recovery report named PARTIAL limits without filling gaps. Translate the whole answer for the owner, preserving facts.

Routing: owner asks -> project_status; enter -> get_context; unknown ref -> search; intend -> plan_work; happened -> record_work; judge -> review_module; reusable knowledge -> save_document; self-contained fix -> none.

Examples (illustrative, not live APIs):
- get_context(project="alpha",ref="M-001") -> outcome, inherited constraints, expected acceptance/checks, Tasks/latest results, lead, dependencies, current decisions, handoff/blockers, next action, obs.
- record_work(project="alpha",ref="M-001/T-02",result="Relative links survive a folder move",checks=[{label:"move scenario",status:"passed"}],transition="done",obs="...") -> “COMMITTED M-001/T-02; result M-001/L-03 saved; Task done; Git committed.”
- save_document(project="alpha",decision={question:"Storage boundary?",statement:"One YAML per Module",rationale:"Independent leads edit separate files",rejected:[{option:"one Epic file",reason:"unrelated write conflicts"}]},obs="...") -> exact Decision ref, no copied document body.
- search(project="alpha",query="restore",kind="runbook"), then get_context(project="alpha",ref="RB-002") -> ordered prerequisites/inputs/steps/expected outcomes/recovery; Markdown detail read only if needed.
- plan_work(project="alpha",checklist={purpose:"Restore rehearsal",items:["Verify restored backup"]},obs="..."); record_work(project="alpha",ref="C-002/I-01",result="Backup restored successfully",transition="done",obs="...").

Typed domain outcomes are projected before strict embedded MiniJinja rendering. Exact IDs, outcomes, omissions, pagination and recovery survive. Rendering failure returns a small outcome-preserving receipt; it never reruns effects or dumps YAML/JSON. Exact MD reads have labelled source/range/revision and real continuation.

## E. Agent walk-throughs and status

Microfix: zero calls, records or documents. Cold orchestrator: get_context(project="alpha") gives purpose, constraints, priorities, expected checks, work/attention, open questions, handoff, knowledge routes, uncommitted refs and obs. Delegation passes only alias+Module ref; one lead context restores assignment. Persistent lead records a meaningful result once; reviewer reads that same substance and accepts directly. Checklist progress uses record_work, never a copied Task list. A decision/research record is created only when future work benefits; long reasoning optionally links MD.

Reviewer context gives criterion/checks, results, gaps/followups and addressed findings; TODO shows open items/findings; log shows ordered entries. Inline explicitly linked small sections first, then scoped decision statements; Omitted counts provide exact routes. Oversized Markdown offers a heading outline/section routes or faithful continuation, never a substitute summary.

Runbook read example: “RB-002 Run the gate; last run not reported. Requires: pinned toolchain; repository root. Inputs: none. 1. Check the project. Command: cargo xtask check. Expected: exit 0. Recovery: inspect the first failure, repair, rerun.” Reading executes nothing.

One-call status example for a representative small project (all rows shown, no omissions):

```text
Alpha — tracked data complete; detail complete
Modules: 1/2 accepted. Tasks: 2/3 done by report; canceled: 0.
M-001 Storage — accepted; lead codex-a; Tasks 2/2.
  T-01 Schema: done. T-02 Move: done.
  Result: relative links survive a folder move.
  Checks: move scenario passed by report. Delivery: PR reported; merge not reported.
M-002 Release guide — working; lead claude-b; Tasks 0/1.
  T-01 Audience scope: open. Checks: not reported.
  Blocker: public release scope D-007; resolver owner.
Attention: resolve D-007 before M-002 can complete.
Coverage: tracked records only; untracked microfixes and agent liveness excluded.
```

The header carries alias and as-of date, e.g. `OK status alpha "Alpha" as of 2026-10-05; data coverage: complete; detail: full`. Each non-closed Module row shows `last record <date>`, the only honest substitute for liveness. Decisions awaiting the owner form their own group above Attention. When present, one line `Uncommitted: <refs>`. No reply ever suggests creating a record or document.

The orchestrator translates prose into Russian without collecting more facts. Never trim open Module rows/leads, blockers, owner questions, ready reviews, failed checks, open closure conditions or coverage. Trim older accepted rows, done Task detail, then recent activity; name omitted counts and exact narrowing routes. If essential rows overflow, return PARTIAL with per-Epic routes, never a complete claim. Proposed 16 KiB status budget must be demonstrated on a declared fixture (12 open Modules, 60 Task rows, 20 attention/owner rows; older closed work aggregated); the cap itself proves neither usefulness nor speed.

Lost result reply: record_work lands T-02 done with L-18 but loses its reply. One get_context shows current result/event; continue if present, otherwise reassess and submit with fresh obs. Replaying old obs refuses with current event details, without claiming ownership of that event or content-based NOOP. An explicitly historical append-only note can retain a stale basis label; it certifies no current completion.

Lost creation reply: creation saves M-005, but its reply/attachment is lost; a lead later edits M-005. The old creation obs now refuses, and get_context shows M-005 as standalone. The caller attaches that exact ID rather than recreating similar text. Unknown append likewise requires inspecting the canonical event list; repeated text is not identity. Interrupted plan: saved child with missing Epic attachment remains visible as standalone work. Inspect confirmed IDs/inventory and attach or continue them; same-content creation is not identity. Compaction: existing agent-run prepares bounded candidates; reviewer examines bytes and incoming links; additions, link updates, retirements follow guarded saves. Originals remain recoverable; interrupted apply reports applied/pending paths. No source inspection or automatic deletion.

## F. Ordinary-file integrity, cache and Git

One tracked .agent-tasks/state.yaml contains durable allocator high-water values and disposable lookup/cache sections. Initial cache: ID/path and useful structural reference metadata; status and review read canonical work. State changes on top-level allocation, rename/membership/knowledge-reference changes; ordinary result/handoff writes stay in their one owner file. Later validated summary caching is allowed if measurement warrants it. No cache is approval authority.

A short cooperative root write lock covers reload/validation, relevant obs checks, durable number reservation before publication, atomic replacement of each file and scoped checkpoint. Numbers may have gaps; high-water values never decrease. Supported retirement retains the ID-bearing record/tombstone. Normal allocation reads counters, not a whole-store recount; generated child/criterion/log high-water counters live in their owning YAML, so local allocation reads a counter without an extra shared-state write. Durable reservation before publication applies to top-level numbers in state.yaml and the new files they name. A local Task, criterion or log ID and its owning YAML high-water field publish together in ONE atomic replacement of the owner file; there is no separate counter-only rewrite, so no local allocation needs two writes. Existing-target exclusivity prevents overwrites. Reads do not persist caches or initialize stores. Cold/stale cache falls back to canonical files in memory. Missing/corrupt counters reconcile retained canonical IDs, canceled/archived records and available historical high-water state under the next managed write; incomplete recovery refuses new allocation. Lost issued-ID history requires explicit restoration, not silent recycling. Clones are independent snapshots, not a distributed allocator.

One opaque observation (policy target <=128 bytes for single-record operations, 1 KiB hard cap; size and agent copy reliability to be measured) covers operation-relevant contents and actual creation inventory, bound to the resolved root. A T-02 result compares T-02 and inherited acceptance, not an unrelated T-07/note; a handoff compares its current pointer. A managed allocation sequence alone cannot detect native inventory drift. Review basis also covers required context. No proof of reading, three-token ritual or hidden per-agent session. Multi-file operations are explicitly partial, not transactions; confirmed IDs and unfinished steps are returned. Old creation inventory cannot authorize blind replacement after a lost reply, even if an orphan was subsequently edited.

Valid dirty records remain writable under scoped observations; only Git adoption is deferred. Auto-checkpoint only clean owned business paths; preserve dirty user work and unrelated/conflicting staging. Valid pending MCP-owned state can be included automatically without manual adoption. A failed commit leaves saved work; continue with checkpoint pending, then recover once at a useful boundary, not after each result. One checkpoint(project="alpha",refs=["M-001"],obs="...") commits the selected saved version plus valid relevant state; no repeated result or separate state call. Each write has one Git line: committed + exact commit ID; saved/not committed + reason and pending refs; manual mode; or no repository. Status/orchestrator context list all uncommitted scoped store refs from Git (honest PARTIAL if too large), so checkpoint needs no agent file scan. No repository means saved without Git history. Only committed originals have Git recovery; no push/merge/history rewrite or source access.

Closed schemas reject unknown fields/versions for mutation, malformed references, duplicate IDs/parentage and invalid lifecycle. Reads expose partial coverage instead of inventing empty success. Migration is an explicit deterministic preview/apply operation with a scoped diff and supported-version policy, not an ordinary-write surprise or an agent-run platform. Unsupported YAML preservation refuses that file's mutation; native Markdown remains exact. Locking, durable replacement, Git/index edge cases and preservation require platform qualification.

## G. First version, verification and limits

First useful version: portable manifest/Module/Epic, minimal typed knowledge/checklist schemas, one-result log model, eight tools, one-call status, lexical retrieval, counters/lookup cache, ordinary locks and scoped Git. Defer semantic search, compaction automation, checklist templates, generalized backlog/risk/incident systems, log archival and source/runtime integration.

Qualification must exercise zero-paperwork microfixes; cold alias entry/delegation; one result feeding log/status/review; typed runbook/checklist/decision/research reads; stale same-Module review versus unrelated Module edits; missing checks; native edits; partial plan with edited orphan; cache/counter recovery; dirty Git/index and failed commit recovery; post-save rendering failure; exact Markdown continuation; compaction interruption. Measure calls, output/discovery bytes, latency, validation refusals, duplicate writing in transcripts and owner-marked missing/wrong status facts, without claiming unmeasured savings.

This remains a high/medium-level design, not an implemented or qualified product. The repository executable is the standard starter. Runtime, filesystem, Git, schema preservation, response budgets and usefulness must be demonstrated on the scenarios above before readiness is claimed.
