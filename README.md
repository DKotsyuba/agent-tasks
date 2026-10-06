# agent-tasks

Rust stdio MCP for portable strategic work. Projects own Epics, standalone Modules and Atomics; Modules own embedded Tasks and Atomics. Agents obtain an assignment by Module reference, record meaningful results once and independently review Modules, Atomics and Epics. One `project_status` call gives the tracked overview. Strict embedded MiniJinja renders compact semantic English text; YAML is structured internal storage.

Code/docstrings describe implementation, the tracker stores intent and reported work, and Git retains committed history. Track only work that benefits from planning, continuity or acceptance. A microfix may need zero records. Execution, tests, Git delivery and installation remain explicit external work.

## Configure and start

Settings and project aliases are separate. Use an absolute configuration path; default HOME/.agent-tasks/config.toml. Root parents must already exist.

```toml
# config.toml
schema_version = 1
```

```toml
# sibling projects.toml
schema_version = 1
[aliases]
product = "/absolute/project/documentation"
other = "/absolute/other/documentation"
```

```bash
cargo fetch --locked
cargo run --locked -- --config /absolute/config.toml mcp
```

Precedence: --config, AGENT_TASKS_CONFIG, then $HOME/.agent-tasks/config.toml. Every business call uses project=<alias>; no global current project exists. Identity/discovery/doctor/export do not require valid configuration. Reads never initialize or migrate roots.

## Purpose-based tools

| Tool | Purpose |
|---|---|
| get_status | Identity and declared qualification |
| register_project | Explicit documentation creation, local Git bootstrap and alias registration |
| get_project_list | Discover aliases and intent without paths |
| get_context | Assignment, parent context, contracts/dependencies, execution, conditions, evidence and versions |
| project_status | One complete tracked overview, with separate data/detail coverage |
| search | Bounded lexical work search and exact references |
| plan_work | Create/edit work, execution, contracts, dependencies and Epic membership |
| record_work | Begin, current results, local completion, delivery, Git import, blockers/handoffs, cancel/reopen |
| review_work | Independent whole-Epic/Module/Atomic verdict, including embedded Atomics |
| review_module | Compatible Module-only review entrypoint |

Live descriptions are mini documentation. Inputs are closed. Plan omission preserves; optional null/list clearing is explicit. A result replaces the complete current summary/checks/artifacts/gaps/followups, so include the current evidence to retain. Snapshot version is read continuation only.

## Plan responsibilities and contracts

Epic owns the business outcome and criteria. Module owns a coherent responsibility, its own criteria, declared lead and execution `{repository, worktree, branch, target_branch}`. Contracts record who provides/consumes what; they are separate from actual blocking dependencies.

```text
contracts={not_required:false,
  provides:[{peer:"M-002",description:"Supply guarded record operations",
             reference:"docs/storage-interface.md",ready:true}],
  consumes:[]}
dependencies=[{ref:"M-003",condition:"accepted",reason:"Requires the migrated store"}]
```

A contract entry names a Module peer, behavior and an optional canonical reference; its readiness is a declared fact. Use not_required=true with empty lists when no contracts apply. One entry per peer/direction can describe a bundle of boundary obligations. Reciprocal provides/consumes links are allowed and do not automatically serialize implementation. Dependencies explicitly wait for accepted Epic/Module work, or delivered Module work; each needs a reason. Invalid/dangling/duplicate/self/impossible cyclic waits refuse. This is a trusted work protocol, not a certificate or automated code-verification platform.

## Start, complete and accept

New records opt into the full workflow. Plan first, then report `record_work op=begin`; this records a start and checks conditions without launching an agent.

- First Epic begin freezes its Module roster permanently, including through reopen. Atomics remain addable. Create later Modules standalone under Project; an explicit dependency can wait for a named Epic.
- Module begin requires an assigned lead, declared execution and criteria, necessary ready contracts, satisfied dependencies and an active parent Epic when owned.
- Task completion is the Module lead's explicit decision after tests or manual verification. No separate Task review exists, and a commit/test result never closes it automatically.
- Atomic result/state=done is local completion. Every new Atomic requires independent review, including `M-001/A-001`; Module acceptance waits for its owned Atomics' current approval.
- Module acceptance requires current whole-Module review and reported delivery/merge into its declared target branch. Local merge counts; a hosting PR is optional. Delivery after review does not demand a second unchanged-code review.
- Integration Atomic under Project/Epic names participating Modules, environment and scenarios. Begin waits for accepted/delivered participants. Record actual joint checks, then obtain independent Atomic review. No-code verification needs no invented commit.
- Epic final acceptance requires the frozen scope's accepted/delivered Modules, current reviewed Atomics, own meaningful result and explicit required checks.

Required checks remain owner-declared; parent intent/criteria are useful background, not implicitly inherited labels. not_applicable never waives an explicit requirement. Gaps block acceptance; followups are outside the current scope. Semantic edits/reopen stale approvals and invalidate delivery/integration applicability; handoff and bookkeeping do not establish implementation changes. Cancel/reopen require reasons, canceled work is excluded from remaining scope, and cancellation never cascades through unfinished children. Identities are declared, not authenticated; known self-review refuses.

## Record a code result once

Declare the Module execution context, then use `record_work op=import_commits` with actual local commit selectors. The helper reads local Git only; it does not push, fetch, checkout, merge or inspect code correctness.

```text
feat(storage): guard replacements

Result:
Stale replacements refuse without overwriting the current record.

Checks:
passed | stale write | cargo test core_store_stale
passed | manual check | Verified recovery text in the local client

Gaps:

Followups:
```

Result: is required; Checks:/Gaps:/Followups: are optional. Check lines use `status | label | optional detail`; detail may contain further separators. Status is passed/failed/not_run/not_applicable. These are lead reports, not independent proof.

```text
record_work(project="product", op="import_commits", ref="M-001/T-001",
            version="<owning Module Version>", commits=["<actual commit hash>"],
            actor="Storage lead")
record_work(project="product", op="complete", ref="M-001/T-001",
            version="<new Module Version>", actor="Storage lead")
```

Import alone preserves completion state; explicit state=done or complete records the lead's decision. Multiple commit Results combine in input order, later checks replace equal labels, and canonical Git repository identity/full SHA prevent duplicate history. The observed message, full identity, author/date and actual worktree remain stored even after Git history or the worktree changes. Failed/oversized/malformed reads refuse rather than publish a partial report. Noncode work, reviews and integration use ordinary semantic reports.

## One assignment and one overview

Delegate a role, project alias, Module reference and permitted checkout. The lead calls get_context and reads addressed detail views instead of receiving another manually maintained copy of the assignment. Context includes relevant parent intent, actual requirements, contracts, dependency waits, execution and review/correction state. Runtime permission is configured by the launcher; a document does not grant access.

project_status includes Epic/standalone Module progress, assigned leads, Task/Atomic counts, current result summaries, blockers and review/delivery attention. Imported commit reports supply result substance. Canceled work, unreadable work and omitted detail are distinct. Partial totals are lower bounds, not zero. A lead handle and last report do not prove a live process. Narrow by Module or read an entity view when detail exceeds the output budget.

## Storage, compatibility and recovery

| Owned path | Authority |
|---|---|
| project.yaml | Project intent and generated dates |
| epics/E-001.yaml | Epic intent, authoritative member refs, frozen roster and own evidence/review |
| modules/M-001.yaml | One Module, embedded Tasks/Atomics, workflow/contracts/dependencies, reports/import sources and reviews |
| atomics/A-001.yaml | Standalone Atomic, execution/integration and independent review |
| .agent-tasks/state.yaml | Independent monotonic top-level counters |
| .agent-tasks/write.lock | Root coordination |
| .agent-tasks/backups/ | Exact originals before normalization |

Existing 0.8.0 and pre-workflow records remain readable and preserve legacy semantic approval digests. Missing or unmanaged workflow means legacy behavior; explicit begin opts the owning record into current rules. Reads never rewrite or invent old agreements/start/delivery. Work-record schema remains closed; allocator revision 2 requires complete counters. Restore lost allocator state, never guess reserved IDs. New fields require this binary; binary rollback is not a data downgrade.

Creation uses Project Allocation version; project edits use manifest Version; embedded Task/Atomic writes use whole Module Version. Epic/Atomic/dependency observations bind relevant decision versions. Use returned versions after confirmed writes. Continue reads with exact Snapshot version and unchanged selection/actual next offset.

Root locking, bounded files, no-clobber publication, exact normalization backups and retained closing headroom protect writes. Creation/attachment are separate operations. Reservations, setup, backups and partial/unknown publication effects are explicit. Inspect current context/results/review/inventory after lost or uncertain replies before another mutation; versions are not request identities. No automatic rollback or replay idempotency is claimed.

register_project explicitly creates and commits bootstrap documentation before alias publication; ordinary work writes do not commit. Optional documentation origin is not contacted. Conflicts never overwrite/retarget. Existing0.7 inline aliases require an explicit operator configuration migration.

## Roles and verification

Canonical [orchestrator](skills/agent-tasks-orchestrator/SKILL.md) and [Module lead](skills/agent-tasks-module-lead/SKILL.md) skills describe the implemented workflow. Update those files through the repository; installed symlinks share their content.

```bash
cargo xtask check
cargo test --frozen -p agent-tasks core_
cargo xtask contract update
cargo xtask contract check
```

The [architecture](docs/architecture.md) is implemented truth. The [unified implementation plan](docs/unified-workflow-implementation-plan.md) records schema and compatibility choices. Rust registry definitions own the contract; schemas/tools.json is its reviewed export. Tests use disposable documentation/Git roots, including real stdio and cold restart.

## Delivery

Package committed source only. Release preparation is preview by default and never pushes. Existing recorded macOS/Inspector qualification does not certify a new release, another host or power-loss durability. Installation never restarts services, changes host configuration or migrates data implicitly. Knowledge writers, ongoing automatic Git, scheduling, databases, daemons, semantic search, runtime polling and compaction remain deferred.
