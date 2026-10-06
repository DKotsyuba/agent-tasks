# agent-tasks

Rust stdio MCP for portable strategic work: Projects, Epics, Modules, embedded Tasks and independently trackable Atomics. Agents plan work, report meaningful outcomes and independently accept whole Modules/Epics. The owner gets one compact English status. YAML is internal structured storage; strict embedded MiniJinja renders semantic text. Code/docstrings and Git retain their authority.

Track only work that benefits from planning, delegation, continuity or acceptance. A microfix can use zero records. Knowledge/document writers, automatic Git maintenance, scheduling, databases, daemons, semantic search and compaction remain deferred.

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

Precedence: --config, AGENT_TASKS_CONFIG, then $HOME/.agent-tasks/config.toml. The retired Linear config is not read. Identity/discovery/doctor/export need no valid config. Every business call takes project=<alias>; no global current project exists.

## Purpose-based tools

| Tool | Purpose |
|---|---|
| get_status | Product identity and declared qualification |
| register_project | Explicit documentation creation, local Git bootstrap and alias publication |
| get_project_list | Discover aliases and manifest-derived intent without paths |
| get_context | Bounded assignment/parent intent, conditions, evidence and write versions |
| project_status | One tracked overview: Epic acceptance, Module leads, Task/Atomic progress, results and attention |
| search | Bounded lexical work search with references/excerpts |
| plan_work | Create/edit Project, Epic, Module, Task and Atomic plans; edit Epic membership |
| record_work | Current reports/checks/artifacts, blocker/handoff and reasoned cancel/reopen |
| review_work | Independent whole-Module or whole-Epic acceptance |
| review_module | Compatible Module-only review entrypoint |

Live descriptions are mini documentation; argument shapes are closed. Plan omission preserves, null clears optional fields, [] clears lists. A result replaces the complete current report: omitted checks/artifacts/gaps/followups become empty.

## Hierarchy and acceptance

Project owns Epics, standalone Modules and standalone Atomics. Epic membership is stored only in its own record and references Modules and standalone Atomics; each belongs to at most one Epic. A Module stays one YAML file with its embedded Tasks and Module Atomics. Tasks are leaves. References are E-001, M-001, A-001, M-001/T-001 and M-001/A-001.

Create work first, then attach the returned exact reference through edit_epic. These are separate publications. A saved child whose attachment failed remains visible as standalone work. Membership is editable while the Epic is open; it invalidates previous acceptance. There is no frozen membership or manual start ceremony imported from the retired workflow.

| Work | Completion policy |
|---|---|
| Task | Meaningful result/state=done; no individual review |
| Module Atomic | Meaningful result/state=done; required evidence is covered by whole-Module review |
| Standalone Project/Epic Atomic | result/state=done requires meaningful result, required checks passed and no gaps/blocker; no mandatory reviewer |
| Module | Independent review after all children terminal, delivery evidence, required checks passed, no blocker/gaps |
| Epic | Independent review after noncanceled Modules currently accepted, Atomics currently done and fresh, own result/criteria/checks satisfied, no blocker/gaps |

Canceled children are excluded from remaining work. Parent cancellation does not cascade and requires terminal children. Reopen explicitly with a reason. Semantic changes/reopen make prior acceptance historical; handoff alone does not. Required checks are local to their owner: parent intent/criteria are context, not implicit inherited check labels. not_applicable never waives a required check.

An integration Atomic declares participating Module references. Its recorded scenarios/check results capture their semantic bases and current review generations. Changes or a new review of a participant mark evidence stale; refresh the actual verification report before relying on completion. A check that changes no code needs no invented commit. The MCP does not inspect source, GitHub or live agents.

## Example

Use actual returned versions:

```text
get_context(project="product")
plan_work(project="product", op="create_epic", version="<Allocation version>",
          title="Portable release", outcome="Components work together",
          criteria=["Storage and reporting work after restart"])
get_context(project="product")
plan_work(project="product", op="create_module", version="<Allocation version>",
          title="Portable storage", outcome="Guarded file updates",
          lead={name:"Storage lead"}, tasks=[{title:"Protect stale writes"}])
get_context(project="product", ref="E-001")
plan_work(project="product", op="edit_epic", epic="E-001",
          version="<Epic Version>", modules=["M-001"])
get_context(project="product", ref="M-001")
record_work(project="product", op="result", ref="M-001/T-001",
            version="<Module Version>", state="done",
            summary="Stale replacement refuses without overwriting")
review_module(project="product", module="M-001", version="<new Module Version>",
              verdict="accepted", summary="Outcome independently checked", actor="Reviewer")
project_status(project="product")
```

Use create_atomic for standalone work, add_atomic for Module-owned work. Create an integration Atomic with participants=["M-001", "M-002"] and explicit required_checks; report real scenarios in checks/detail. review_work(ref="E-001") records final Epic acceptance; it does not manufacture member results.

## Storage, versions and recovery

| Owned path | Authority |
|---|---|
| project.yaml | Project title/purpose, optional reported remote, generated dates |
| epics/E-001.yaml | Epic intent/criteria, authoritative member references, own evidence and review |
| modules/M-001.yaml | One Module, embedded Tasks/Atomics, reports, blocker/handoff, reviews/history |
| atomics/A-001.yaml | Standalone Atomic intent, executor, reports and integration participant evidence |
| .agent-tasks/state.yaml | Durable monotonic top-level counters |
| .agent-tasks/write.lock | Root coordination |
| .agent-tasks/backups/ | Exact originals preserved before normalization |

Creation uses Project Allocation version. Project edits use manifest Version. Epic/standalone Atomic writes use returned owner Version bound to current member/participant observations, including transitive integration participants; Task/Module Atomic writes use the whole Module Version. Snapshot version is read continuation only. Keep unchanged scope/view/query/review selection and actual returned next offsets. Ordinary work writes do not commit or push.

Existing 0.8.0 schema-1 roots remain readable, with defaulted record fields and no rewrite on reads. New/rewritten allocators use revision 2 with required independent counters; missing/null new counters refuse even when publication left an empty inventory. Only legacy revision-1 allocators may start an unused new kind at one after proving its inventory empty. Lost counters are never reconstructed by guessing. New writes may emit expanded schema-1 fields even on existing records. Older binaries may reject these rewritten records and cannot operate on new Epic/Atomic records; binary rollback does not roll back stored data.

Writes lock/reload/compare root-bound versions. Reserve IDs before no-clobber publication; a failure can leave a retained gap. Lost/partial/uncertain replies require current context/log/inventory inspection before another mutation. No automatic rollback, cross-file transaction or replay idempotency is claimed. Counter, setup and backup effects are disclosed.

Context/search pages and project_status distinguish data coverage from omitted detail. Unreadable work is unknown, not zero; partial counts are lower bounds. Narrow by Module or open entity details when needed. Status is tracked work, not live agent presence or all untracked microfixes.

register_project explicitly creates storage, README/.gitignore and local Git bootstrap before publishing the alias. Identical completed registration is a no-op; conflicts never overwrite/retarget. Optional docs_remote adds a separate origin without contacting it. Partial failures retain files/staging and disclose effects. For 0.7.0 inline settings aliases, explicitly move [aliases] into sibling projects.toml; reads do not migrate configuration.

## Roles and development

Update/install the canonical [orchestrator](skills/agent-tasks-orchestrator/SKILL.md) and [Module lead](skills/agent-tasks-module-lead/SKILL.md) skills through the host mechanism. These describe real workflow and proportional tracking; they do not schedule agents or initialize projects implicitly.

```bash
cargo xtask check
cargo test --frozen -p agent-tasks core_
cargo xtask contract update
cargo xtask contract check
cargo deny fetch
cargo deny check
cargo run --locked -- doctor --json
```

Rust registry definitions are authoritative; schemas/tools.json is a reviewed export. Source documentation changes with behavior. The [implemented architecture](docs/architecture.md) describes limits and filesystem boundaries; the [Epic/Atomic implementation plan](docs/epic-atomic-implementation-plan.md) records policy choices. The wider [proposal](docs/architecture-proposal.md) remains a discussion roadmap.

## Delivery

Template baseline is pinned in .family/origin.json. Updates are dry-run three-way plans followed by Git review. Runtime/build needs no private template access. Qualification in docs/RELEASE_ACCEPTANCE.md covers the recorded macOS arm64/MCP Inspector acceptance, not untested hosts or power-loss durability.

Package only committed source. Release preparation defaults to preview and never pushes. Installation never restarts services, modifies host configuration or migrates portable roots. A binary rollback does not undo stored work.
