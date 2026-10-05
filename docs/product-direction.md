# Portable project tasks and knowledge — draft

This document records the proposed product direction. The repository currently
contains the standard MCP starter only. None of the storage, workflow, search,
Git automation or documentation maintenance described below is implemented.
Names and file layouts in this document are proposals, not an exported API.

## Product purpose

Give agents a small, useful project context, maintain a readable task tree and
project knowledge, and produce an owner-ready status report in one tool call.
The product stores its authoritative data in portable, human-editable files.
It does not depend on Linear or on a globally fixed knowledge directory.

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

## Requested foundation

- Rust and the standard family MCP template, including compact MiniJinja replies.
- Markdown stores documents; YAML stores structured work and links to documents.
- A project is the root of the work tree. The existing Epic, Module, Task and
  Atomic model and workflow remain the starting point.
- One Epic YAML contains its structured specification, acceptance criteria,
  Modules, Tasks, Atomics and their important work data. Long supporting documents
  remain Markdown and are referenced from YAML.
- Modules may also exist directly under a Project. Atomics may belong to a
  Project, Epic or Module. Tasks belong to Modules.
- Tasks have local checks; independent review covers a whole Module.
  Integration checks verify the seams between completed Modules.
- The MCP handles identifiers and numbering, recording current dates and status
  transitions, Git operations and commits, and other repetitive bookkeeping.
- The owner can request status and receive the tool's complete report directly,
  without the agent assembling counts or rewriting the response.
- Documentation and decisions must stay discoverable and useful. An explicit
  compaction operation can delegate a scoped Markdown cleanup through agent-run.
- Search returns useful scoped matches in one call, with routes to exact content.

## Proposed minimum design

| Concern | Proposed behavior |
|---|---|
| Opening a store | An explicit start/open call takes a documentation root, similar to opening a workspace. Each later call identifies that store. |
| Portability | Relative paths and stable identifiers inside the root; moving or cloning the directory preserves links. No machine-specific absolute paths in canonical records. |
| Work-record source of truth | YAML and Markdown own strategy, work records and document links. Code owns implemented behavior. A search index is disposable and never owns authoritative records. |
| Structured integrity | Validate schema, unique identifiers, existing references, parent relationships and workflow conditions. Invalid manually edited data produces actionable diagnostics. |
| Context | Return the project brief, assigned work, relevant current decisions, contracts and next action within a declared budget. Retrieve long documents explicitly. |
| Status | Deterministically compute progress, leads, blockers and required actions from structured data. Expose freshness and incomplete coverage rather than inventing counts. |
| Writes | Scoped edits preserve unrelated fields and documents. Concurrent edits to the same Epic file must not overwrite one another; use an observed revision and a scoped write lock. |
| Git | Commit only explicitly owned paths; preserve unrelated staged and dirty work. No automatic push, merge, branch deletion or history rewrite. Separate code and documentation repositories remain possible. |
| Documentation | Keep a short project brief, strategic rationale and intended changes. Implemented technical decisions live in code documentation independently; no synchronization or required source links. Git supplies committed history. |
| Search | Start with text, structured filters and links. Return ranked excerpts and exact document routes. Add semantic retrieval only if measured misses justify it. |
| Compaction | Produce a reviewed diff against a known document-store revision. Preserve current goals, requirements, target contracts and durable reasons behind choices. Propose removing redundant implementation prose without inferring technical truth from source code. Superseded decisions remain recoverable in history. |

An Epic-sized YAML is intentionally the first model to evaluate. Parallel leads
editing different Modules still share that file: write conflict behavior needs
to be decided before implementation, rather than hidden by last-writer-wins.

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
