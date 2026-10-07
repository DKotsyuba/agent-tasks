---
name: agent-tasks
description: "Use before any root agent uses the Agent Tasks MCP for project context, tracked documentation, lookup, status or project records, regardless of its coder, orchestrator or other role. Verify registration through get_project_list first; if missing, stop and obtain the owner's documentation location before registration and project work. This is a shared entry gate, not an orchestration role."
---

# Use Agent Tasks as a root agent

Keep your assigned root role. Using this MCP does not make a coder an orchestrator, authorize delegation or require an Epic/Module hierarchy. Apply this gate whenever the task needs Agent Tasks project information or documentation-related work, including read-only lookup. Work that does not need this MCP does not acquire a registration requirement from this skill.

## Check registration before project work

1. Discover the actual Agent Tasks tools and read their live descriptions. Do not invent operations or use a remembered catalog as the current contract.
2. Make `get_project_list` the first Agent Tasks MCP call for this project work, even when an alias was supplied or remembered. Do not start with `get_status`, `get_context`, a search or a write. The registration-list query is the entry check, not work on an unregistered project.
3. Identify the intended project by its returned alias and project intent. Do not guess an alias from the working directory or choose a similarly named project without establishing the match. If the match is ambiguous, ask the owner which registered project to use before opening its records.
4. If the intended alias is confirmed, continue with that exact alias. Otherwise follow the returned pagination until registration or absence is established; preserve the same snapshot and paging scope. An omitted page, failed/malformed list, stale continuation or incomplete registry observation is not proof that the project is absent. Reconcile the list or report the unknown state and stop dependent work.

An existing but unavailable, unreadable or uninitialized project is already registered. Report its actual condition; do not create a duplicate or retarget its alias. A confirmed healthy selected project may proceed even when unrelated rows are unavailable.

## Missing project: owner chooses the location

Only after a complete, consistent registration-list observation establishes absence, tell the owner, in the owner's language, that the intended project is not registered. If absence is not established, report registration as unknown instead. Ask where its documentation repository should be registered, requesting the exact absolute directory. Pause all project-scoped MCP reads, searches, status calls and writes until registration is confirmed. Do not probe guessed aliases with `get_context`, automatically register under the current repository, choose a Space folder or silently create a fallback store.

If the owner has already explicitly supplied and authorized the registration location for this same project, use that instruction after reporting the missing registration; do not ask for the same information again. A remembered path, another project's location or an inferred default is not that authorization.

After the owner supplies the location:

1. Use `register_project` with the approved `doc_dir` and the project's English name/description. Use the owner's alias when supplied; otherwise choose an unused clear alias for the already identified project from the complete registry and report that alias with the approved location. Do not select or replace an existing similarly named alias. Explain registration's actual effects: local documentation files, Git bootstrap/initial commit and alias publication. Source `remote` and documentation `docs_remote` are distinct optional facts; do not invent them. Registration does not push or grant filesystem/runtime permissions.
2. Inspect the registration receipt, then call `get_project_list` again to confirm the intended entry. Only then obtain `get_context(project=<confirmed alias>)` and resume the requested work.
3. If registration fails or its outcome is unknown, preserve disclosed files/staging and first query `get_project_list` to determine whether the intended alias was published. A confirmed healthy matching entry goes through the confirmation/context step above without another registration. If publication remains unconfirmed, inspect only the disclosed effects in the owner-approved directory and its Git state using already authorized read-only tools. This is registration recovery, not permission to read/use project records around the gate. Retry the exact approved registration arguments only after confirming the incomplete attempt and restoring its failed precondition. If inspection is inconclusive or needs new authority, report the uncertainty and stop for the owner's direction. No new alias/directory, destructive cleanup, manual registry edit or direct YAML access may bypass the gate. Conflicting existing documentation is not permission to overwrite it.

Registration must remain valid throughout the task. On an unknown-alias response or a changed project selection, return to discovery rather than falling back to another project.

## Use the registered project within the current role

- Context: `get_context` supplies project/work intent, current reports and owning write versions. Use returned references, not invented IDs or `ref=Project`.
- Lookup: `search` finds tracked semantic fields; open returned work references with `get_context`. It does not search arbitrary source or Markdown files.
- Overview: `project_status` supplies one scoped summary. Preserve omissions, unavailable data and lower-bound counts rather than presenting them as complete.
- Structured documentation/work records: use only supported `plan_work` and `record_work` operations, with fresh owning versions and the authority the operation requires. Registration does not authorize impersonating a bound lead/reviewer or bypassing its lifecycle.
- Prose documents: the current MCP does not provide a general Markdown reader/writer. After the registration gate, use only tools and target paths already authorized by the current role/task for the requested document; registration is not blanket permission to write into the documentation repository. If its destination or authority is unclear, ask the owner before writing. Do not invent a document-writing MCP tool or treat a tracked report as a saved prose document. Do not use file access to continue missing-project MCP work around this gate.

For actual Epic/Module orchestration, additionally use `agent-tasks-orchestrator`; it contains the orchestration rules. An assigned Module lead uses `agent-tasks-module-lead`. Neither specialist replaces this root entry gate or changes the root role chosen by its bootstrap.
