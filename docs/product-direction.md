# Portable project tasks and knowledge — draft

This document records the proposed product direction. The repository currently
contains the standard MCP starter only. None of the storage, workflow, search,
Git automation or documentation maintenance described below is implemented.
Names and file layouts in this document are proposals, not an exported API.
The more detailed [architecture proposal](architecture-proposal.md) models
agent entry, work, status, retrieval and documentation maintenance. Its defaults
and API examples remain proposals for discussion, not accepted implementation.

## Product purpose

Give agents a small, useful project context, maintain a readable task tree and
project knowledge, and produce an owner-ready status report in one tool call.
The product stores work in portable text files. The MCP forms and maintains
structured YAML; agents use semantic tools and readable replies, not YAML dumps.
It does not depend on Linear or on a globally fixed knowledge directory.

## Design priorities

The product is designed primarily for agents: quick orientation, clear actions,
less duplicated writing, fewer unnecessary tool calls and fewer workflow errors.
The owner-facing requirement is a standardized status report produced by ONE
tool call, without manual file scans, runtime polling or reassembling the facts.
It must show progress by Task and Module, active work, reported results and
blockers, with truthful coverage and assignment attribution.

All project documentation and semantic work content are maintained in ENGLISH:
descriptions, requirements, results, reviews, decisions, research, runbooks,
work-log substance, checklists and Markdown. The orchestrator may translate the
complete status into Russian when presenting it to this owner. Preserve facts,
IDs, numbers, dates, coverage and uncertainty; no stored bilingual duplicates or
translation engine inside the MCP is needed.

The initial ideas are starting points, not a frozen specification. Architects
may simplify lifecycle gates, required fields, tools, storage and document rules
when a concrete agent scenario benefits. Choose technical defaults rather than
passing technical choices back to the owner. Preserve data and report truthfully;
do not claim implemented behavior or measured savings during design.

The current chosen granularity is ONE YAML PER MODULE. Its Tasks and Atomics stay
inside it. An Epic holds its intent and references to Modules, rather than their
full work trees. This reduces cross-lead write conflicts and makes module history
separate in Git. Cross-file references and partial plan writes still need a small,
honest design. The rest of the earlier model remains open to justified revision.

## Semantic tools and machine-maintained data

Keep a limited set of clear, flexible tools with one understandable purpose each.
They are work/context operations, not grep wrappers over YAML. Structured data
is machine-maintained storage; normal work, result, description and review reads
return compact text projected through embedded MiniJinja. Preserve meaningful
identifiers, actual outcomes, limits and recovery routes, not internal fields.

Markdown can be retrieved faithfully, usually by searching first and then reading
the useful section or document. Do not put every document body into an entry pack.
The MCP writes technical document metadata itself; the agent supplies useful text,
not timestamps, internal headers or bookkeeping.

YAML is the foundation for everything with a useful known structure. Markdown
supplies flexible explanatory text, not arbitrary versions of standard document
types. Runbooks, decisions, semantic work logs, checklists and structured research
summaries each have ONE strict versioned schema. The MCP forms those records from
semantic inputs, validates fields/types/references and fills generated data.
No agent-authored YAML or custom free-form schema is required.

The start manifest remains `project.yaml`: title, description/purpose, optional
project remote, schema and needed project settings/references. Remote is descriptive
data, not permission to fetch or inspect a source repository.

One technical file, `.agent-tasks/state.yaml`, holds MCP-owned allocator counters
and useful derived lookup/cache data. Read the last allocated number there rather
than scan all work on every creation. Maintain it during managed writes; cold
recovery or detected external drift may rebuild relevant derived data. Keep this
file inside the portable root and out of normal agent responses. Work records and
documents own their meaning; cached summaries are not another source of truth.

## Proportional tracking and documentation

Use the system when there is substantial project work worth planning, coordinating
or reporting. Do not create a work hierarchy for every quick fix. A small project
with one or two scripts may need no store or separate strategic documents at all.
Code documentation and the ordinary Git commit can be sufficient.

- A microfix does not require a standalone Epic, Module, Task or Atomic, a
  development log, a status note or a copied result report.
- A real Module may contain useful Tasks, but each tiny edit is not automatically
  another Task. Reuse the current assignment and record a meaningful outcome once.
- Save documents for useful intent, constraints or durable rationale. Large work
  does not automatically require Runbook, Log, TODO or decision templates.
- A short consequential decision can be recorded without inventing a work tree.
- Reads never initialize a store, create records or demand documentation merely
  because files or a project exist.
- Status covers tracked work. It does not claim complete coverage of unrecorded
  microfixes or live-agent activity.

The agent decides whether tracking is useful from the requested outcome and the
need for delegation, coordination, requirements, acceptance or durable context.
This applicability guidance belongs in the shared workflow skills and relevant
existing tool responses. Do not add a mandatory scope-assessment call, line-count
thresholds, source scanning or another framework to make that decision.

## Useful knowledge, without a mandatory document bundle

Use a small optional menu, chosen by its value to the next agent:

| Information | Home and reason to keep it |
|---|---|
| Project context | The start manifest: purpose, boundaries, priorities and important constraints, so cold entry is useful. Expand into Markdown only when necessary. |
| Decisions | A structured YAML record: question, chosen direction, rationale, alternatives and current/superseded state. Longer explanation may reference Markdown. |
| Runbooks | Strict YAML: purpose, prerequisites, inputs, ordered steps with commands and descriptions, expected outcomes and recovery. Reading it returns a useful text procedure. |
| Research | Structured YAML conclusions/evidence/sources when reusable; optional Markdown holds extended analysis. Short findings can stay in current work. |
| Handoff and open questions | A current Module note/blocker: meaningful stopping point, remaining action and who can resolve it. Write at an interruption or real blocker, not after every command. |
| Work log | Structured YAML entries maintained/appended by MCP for meaningful tracked work. Agent supplies useful substance; IDs, dates, author/context and links are generated. |
| TODO/checklist | Typed YAML items and completion data. Work TODO views reuse existing Tasks; independent procedural checklists use the same consistent item shape. |

These are optional kinds, not an empty bundle created at initialization. Their
shape is strict WHEN used. Meaningful tracked events can be logged automatically
from the same work call without another agent-authored report; microfixes do not
require a log. TODO views must not duplicate the canonical Task list.

MCP builds navigation from existing metadata and work references: return relevant
document titles/purposes, current/superseded signals and exact reading routes.
Do not require the agent to maintain a second hand-written documentation index.
The initial context should include important constraints and small relevant
sections, not the whole knowledge collection.

Technical fields are always MCP-owned. The agent provides the decision, command
intent, meaningful result or checklist item; it never types generated IDs,
timestamps, bookkeeping or a storage-format template. Markdown is an optional
detail body referenced by structured records. All standard YAML kinds read back
as compact semantic text, never raw YAML.

Technical dates/attribution are machine-maintained. A recent edit is not proof
that a decision is still applicable or a runbook was executed successfully;
verification and supersession remain explicitly reported semantic facts.

## Authority boundaries

| Source | Owns |
|---|---|
| Code and its detailed native documentation | Current implemented technical behavior, interfaces, invariants and implemented technical decisions. |
| Project strategy and work records | Purpose, desired outcomes, future direction, planned changes, requirements, acceptance criteria and remaining work. |
| Git history | The historical progression of code, strategy, Epics, Modules and Tasks through their committed changes. |

The task/document system must not become a competing description of the current
implementation. Agents inspect code and its documentation through their code
tools for technical truth; this MCP does not read, index or synchronize source.
A planned contract is a target, not evidence that the current code implements it.
Once implemented, the current technical contract belongs with the code; the work
record retains the agent's reported result and acceptance evidence.

Respecting code authority does not require a mechanical link to code. Source
symbols, file paths and docstrings are not required fields or integrity targets.
Optional commit or PR references remain reported artifacts, not a requirement
for source parsing or automatic verification of implemented behavior.

Git preserves both code and work history. Workflow status remains a structured
record: a commit alone does not imply review, acceptance or completion.

## Proposed large capability areas

These are product responsibilities organized around agent needs, not yet a
decomposition into implementation Modules or a commitment to separate crates.

| Area | Agent need | Responsibility |
|---|---|---|
| Project direction | Understand why the project exists and where it should go. | Goals, boundaries, priorities, requirements and intended outcomes. |
| Work cycle | Know what to do, who owns it and what completes it. | Epic/Module/Task/Atomic tree, assignments, criteria, dependencies, transitions, review and integration. |
| Context and retrieval | Enter a project or assignment quickly and find relevant information. | Bounded context packs, scoped Markdown/YAML search and exact document/work retrieval. Code inspection belongs to the agent's IDE. |
| Status and attention | Give the owner a ready report and identify the next useful action. | Computed progress, active work, leads, blockers, review/merge conditions and honest coverage. |
| Knowledge maintenance | Keep strategic documentation useful without duplicating code or history. | Durable rationale, current versus superseded plans, document links, scoped compaction proposals and preservation of recoverable history. |

All five areas share a portable file/Git foundation: opening an arbitrary store,
YAML validation, Markdown references, numbering, timestamps, coordinated writes
and scoped commits. That foundation automates bookkeeping; it does not determine
strategy or reinterpret the implemented behavior of code.

## Current starting model

These ideas guide the design but do not freeze every field, tool or workflow gate.

- Rust and the standard family MCP template, including compact MiniJinja replies.
- YAML stores standard structured work/knowledge records; Markdown holds flexible
  extended text referenced from those records.
- A project is the root of the work tree. The existing Epic, Module, Task and
  Atomic model and workflow remain the starting point.
- One Module YAML contains its specification, acceptance criteria, Tasks,
  Atomics, status and results. An Epic YAML contains its intent, requirements,
  acceptance criteria and Module references. Long documents remain Markdown.
- Modules may also exist directly under a Project. Atomics may belong to a
  Project, Epic or Module. Tasks belong to Modules.
- Tasks are optional useful subdivisions with local checks where relevant;
  independent review covers a whole Module. Atomic-shaped checks need no separate
  review ceremony by default. Integration checks belong to work whose actual
  interactions warrant them.
- The MCP handles identifiers and numbering, recording current dates and status
  transitions, Git operations and commits, and other repetitive bookkeeping.
- The owner can request status and receive the tool's complete report directly,
  without the agent assembling counts; presentation-language translation is allowed.
- Documentation and decisions must stay discoverable and useful. An explicit
  compaction operation can delegate a scoped Markdown cleanup through agent-run.
- Search returns useful scoped matches in one call, with routes to exact content.

## Proposed minimum design

| Concern | Proposed behavior |
|---|---|
| Opening a store | All work/document calls carry `project`, the configured project name/alias. An explicit `root` alternative supports setup. No separate open/register call or shared mutable current space. |
| Store aliases | TOML maps project name to absolute folder. Names resolve per request; portable records keep relative links. Changing a mapping does not retarget an in-flight write or an old observation. |
| Portability | Relative paths and stable identifiers inside the root; moving or cloning the directory preserves links. No machine-specific absolute paths in canonical records. |
| Work-record source of truth | YAML and Markdown own strategy, work records and document links. Code owns implemented behavior. A search index is disposable and never owns authoritative records. |
| Structured integrity | Validate schema, unique identifiers, existing references, parent relationships and workflow conditions. Invalid manually edited data produces actionable diagnostics. |
| Context | Return the project brief, assigned work, relevant current decisions, contracts and next action within a declared budget. Retrieve long documents explicitly. |
| Status | Deterministically compute progress, leads, blockers and required actions from structured data. Expose freshness and incomplete coverage rather than inventing counts. |
| Writes | Leads edit separate Module files. Scoped edits preserve other fields and documents; coordinate shared Project/Epic writes and refuse stale overwrites of the same work. |
| Git | Commit only explicitly owned paths; preserve unrelated staged and dirty work. No automatic push, merge, branch deletion or history rewrite. Separate code and documentation repositories remain possible. |
| Documentation | Keep a short project brief, strategic rationale and intended changes. Implemented technical decisions live in code documentation independently; no synchronization or required source links. Git supplies committed history. |
| Search | Start with text, structured filters and links. Return ranked excerpts and exact document routes. Add semantic retrieval only if measured misses justify it. |
| Compaction | Produce a reviewed diff against a known document-store revision. Preserve current goals, requirements, target contracts and durable reasons behind choices. Propose removing redundant implementation prose without inferring technical truth from source code. Superseded decisions remain recoverable in history. |

File layout and tool contracts should be tested against the agent's cold entry,
lead assignment, completion, interruption and owner-status scenarios. Do not keep
a constraint solely because it appeared in the discontinued Linear workflow.

## Questions for the next design discussion

1. Define the smallest complete Project and Epic records, including standalone
   Modules and project-level Atomics, without duplicating narrative documents.
2. Specify the owner-facing status report and the lead's first context response.
3. Decide which Git actions are automatic, which are explicit, and how a store
   outside the source repository is associated with that repository.
4. Define when information deserves a document, when a decision is superseded,
   and what compaction may propose or remove.
5. Choose a minimal initial tool set, then specify workflow and integrity checks.

## References to study

These projects are sources of ideas, not dependencies or accepted designs.

- [MemoryCustodian](https://github.com/waittim/MemoryCustodian): explicit manifest
  routing and bounded task-specific context from repository Markdown.
- [Keep the Why](https://github.com/oliver-zehentleitner/keep-the-why): retain the
  rationale and rejected choices in Git-versioned project Markdown.
- [IWE](https://github.com/iwe-org/iwe): connected Markdown knowledge through
  editor, CLI and MCP interfaces.

The remaining candidate systems can be compared against concrete gaps after the
file model and the first context/status/search examples are agreed. No external
memory backend or code knowledge graph is required by the current proposal.
