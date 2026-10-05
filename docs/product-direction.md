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
| Source of truth | YAML and Markdown only. A search index, if needed, is disposable and rebuildable; it never owns task or document state. |
| Structured integrity | Validate schema, unique identifiers, existing references, parent relationships and workflow conditions. Invalid manually edited data produces actionable diagnostics. |
| Context | Return the project brief, assigned work, relevant current decisions, contracts and next action within a declared budget. Retrieve long documents explicitly. |
| Status | Deterministically compute progress, leads, blockers and required actions from structured data. Expose freshness and incomplete coverage rather than inventing counts. |
| Writes | Scoped edits preserve unrelated fields and documents. Concurrent edits to the same Epic file must not overwrite one another; use an observed revision and a scoped write lock. |
| Git | Commit only explicitly owned paths; preserve unrelated staged and dirty work. No automatic push, merge, branch deletion or history rewrite. Separate code and documentation repositories remain possible. |
| Documentation | Keep a short project brief and current decisions. Write new documents only for durable information; do not duplicate task history or code descriptions. |
| Search | Start with text, structured filters and links. Return ranked excerpts and exact document routes. Add semantic retrieval only if measured misses justify it. |
| Compaction | Produce a reviewed diff against a known source revision. Preserve current requirements, contracts and the reasons behind rejected choices. Superseded decisions remain recoverable in history. |

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
