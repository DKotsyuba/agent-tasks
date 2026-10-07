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
| get_context | Assignment, parent context, contracts/dependencies, execution, conditions, evidence, versions and the integration view |
| project_status | One complete tracked overview, with separate data/detail coverage |
| search | Bounded lexical work search and exact references |
| plan_work | Create/edit work, execution, contracts, dependencies, criterion scopes, Epic membership and roster freeze |
| record_work | Begin, adopt_core, agent bind/recovery, lead planning, contract agreement, results/candidates, boundary evidence, criterion verification, completion, delivery, Git import, blockers/handoffs, cancel/reopen |
| review_work | Independent whole-Epic/Module/Atomic verdict with changed-scope follow-up, including embedded Atomics |
| review_module | Compatible Module-only review entrypoint |

Live descriptions are mini documentation. Inputs are closed. Plan omission preserves; optional null/list clearing is explicit. A result replaces the complete current summary/checks/artifacts/gaps/followups, so include the current evidence to retain. Snapshot version is read continuation only.

## Plan responsibilities and contracts

The core workflow below is implemented for core-active records. The final full gate and release qualification are the repository owner's; see [architecture](docs/architecture.md) for the implemented rules and [plan](docs/epic-core-plan.md) for the governing target.

Epic owns the business outcome, criteria and declared `criterion_scopes=[{index,text,modules}]` (at most 8; ZERO-BASED index and text match the current criteria, Modules within the roster). Module owns a coherent responsibility, its own criteria, a bound lead and execution `{repository, worktree, branch, target_branch}`. Contracts record who provides/consumes what; they are separate from actual blocking dependencies.

```text
contracts={not_required:false,
  provides:[{id:"store-records",revision:1,peer:"M-002",
             description:"Supply guarded record operations",
             reference:"docs/storage-interface.md",ready:true}],
  consumes:[]}
dependencies=[{ref:"M-003",condition:"accepted",reason:"Requires the migrated store"}]
```

A core entry has a stable id and positive revision; the provider owns the canonical definition and reciprocal entries on all party Modules must match id/revision/artifact. Each participating lead confirms the exact revision with `record_work agree_contract {contract_id, revision, summary}`; `ready` is metadata and does not replace confirmation. An affecting change needs a higher revision and renewed confirmation, and invalidates the peers' review/readiness even when their code is unchanged. Use not_required=true with empty lists when no contracts apply. Dependencies are the only real waits (accepted Epic/Module, or delivered Module); each needs a reason, and invalid/dangling/duplicate/self/cyclic waits refuse. A wait cycle is an architecture problem to report and resolve before the affected work. This is a trusted work protocol, not a certificate or automated code-verification platform.

## Start, complete and accept

New records have the core workflow active. Existing managed-1 and unmanaged records keep prior behavior and digests until an explicit `record_work op=adopt_core`, which makes earlier approval historical and invents no IDs, planning, agreement, candidates or verification. `record_work op=begin` records a start and checks conditions without launching an agent.

1. The orchestrator creates the Epic and provisional Modules (no invented Tasks) and begins the Epic for planning.
2. It launches each lead, then `record_work op=bind_agent {role, harness, agent_id, communication_ref, resume_ref?, launch_ref}` with the actual runtime receipt. Reviewers bind on the Module; integrators and reviewers on an integration Atomic. Scoped actions use the recorded `agent_id` as actor. The Epic has no binding slots: its begin/result/freeze/`verify_criterion`/review are orchestrator-attributed.
3. The bound lead reports `record_work op=planning {responsibility, scope, exclusions, read_refs, uncertainties}`, then plans Tasks with plan_work add_task/edit_task and declares contracts and dependencies.
4. Leads confirm contracts; `plan_work freeze_epic` checks lead discovery/agreement/criterion scopes, distinct prepared checkouts and mandatory-wait cycles, then freezes the roster before coding. It does not require acyclic waits to be satisfied: a provider can begin so its consumer waits for acceptance.
5. Module begin enforces its own dependencies, a frozen parent, a ready lead, current planning/agreement and a separate worktree; active core Modules cannot share a writable checkout.
6. Task completion is the lead's explicit decision after tests or manual verification. No Task review exists, and a commit/test result never closes it.
7. The lead submits a definite candidate (`result` with `candidate` and `changed_scope`, or a commit import, whose candidate is the raw full Git SHA without a `commit:` prefix) and `record_work op=boundary_evidence {contract_id, revision, candidate, conditions, correct, mutation, failed, restored, artifacts}` where correct/failed/restored are `{status,detail,artifact?}` observing passed/failed/passed. Recording runs nothing. A Module with `contracts.not_required=true` still needs the same mutation controls under the reserved `contract_id="local"`, `revision=1`; this is a required local quality scope, not a peer contract id, and applicability pins the current Module intent and candidate.
8. The bound reviewer's positive `review_work` makes the Module ready for integration; no merge or delivery is required. Follow-ups use `changed_scope` and `resolved_findings=[{review_index,finding_index,summary}]` (ZERO-BASED indexes) with the same lead and reviewer. Delivery stays separate bookkeeping (`record_work op=deliver`).
9. `get_context view=integration` derives ready connected components of at least two Modules and their candidate/contract coverage. An integration Atomic needs a bound integrator and a distinct checkout; equivalent current coverage refuses a duplicate job, and changed affecting inputs stale it.
10. `record_work op=verify_criterion` on the Epic `{index (ZERO-BASED), text, modules, candidate, environment, scenarios, summary, checks, artifacts, integration_ref?}` pins actual business/E2E evidence for each declared criterion. A supplied `integration_ref` must be one current accepted composition covering the full Module set; A+B plus B+C is not proof of A+B+C.
11. Epic acceptance needs current-reviewed Modules, covered boundaries and current verification of every declared criterion; it needs no all-roster-per-job barrier.

```text
# Module with contracts={not_required:true}: local quality scope
record_work(op="boundary_evidence", ref="M-004", contract_id="local", revision=1,
  candidate="<raw full Git SHA>", conditions="fixed clock; empty store",
  correct={status:"passed", detail:"expected output observed"},
  mutation="Return the unsorted list",
  failed={status:"failed", detail:"ordering case failed as intended"},
  restored={status:"passed", detail:"control rerun passed after restore"})
```

Replacement of a bound agent is allowed only when it is lost, unrecoverable AND cannot continue: `recover_agent stage=lost` records it, the new agent is bound, and `recover_agent stage=immersed` (by the new `agent_id`, with understanding, sources, unfinished, gaps) must precede continuation. Atomic result/state=done is local completion; every new Atomic needs independent review, including `M-001/A-001`.

Required checks remain owner-declared; parent intent/criteria are useful background, not implicitly inherited labels. not_applicable never waives an explicit requirement. Gaps block acceptance; followups are outside the current scope. Semantic edits/reopen stale approvals and applicable coverage; handoff and bookkeeping do not establish implementation changes. Cancel/reopen require reasons, canceled work is excluded from remaining scope, and cancellation never cascades through unfinished children. Bindings are reported observations, not authentication; known self-review refuses.

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

The role-neutral root [agent-tasks](skills/agent-tasks/SKILL.md) skill checks registration through get_project_list before project-scoped reads or writes. For a missing project, the root reports it and waits for the owner's documentation directory before registration and continuation. It preserves the root's coder, orchestrator or other assigned role.

Canonical [orchestrator](skills/agent-tasks-orchestrator/SKILL.md) and [Module lead](skills/agent-tasks-module-lead/SKILL.md) skills describe the Epic core workflow; the orchestrator uses the shared root entry gate before its coordination rules. Update these files through the repository; installed symlinks share their content.

```bash
cargo xtask check
cargo test --frozen -p agent-tasks core_
cargo xtask contract update
cargo xtask contract check
```

The [Epic core workflow plan](docs/epic-core-plan.md) is the governing product target; [architecture](docs/architecture.md) describes implemented behavior. The [unified implementation plan](docs/unified-workflow-implementation-plan.md) records schema and compatibility choices. Rust registry definitions own the contract; schemas/tools.json is its reviewed export. Tests use disposable documentation/Git roots, including real stdio and cold restart.

## Delivery

Package committed source only. Release preparation is preview by default and never pushes. Existing recorded macOS/Inspector qualification does not certify a new release, another host or power-loss durability. Installation never restarts services, changes host configuration or migrates data implicitly. Knowledge writers, ongoing automatic Git, scheduling, databases, daemons, semantic search, runtime polling and compaction remain deferred.
