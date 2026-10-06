# agent-tasks

Rust stdio MCP for portable strategic work. Agents plan Modules/Tasks, report outcomes and independently review a whole Module. The owner gets one compact English status. YAML is structured internal storage; strict MiniJinja renders semantic text. Code/docstrings and Git retain their authority.

This is the minimal local core. The full future roadmap stays in docs/architecture-proposal.md. Tiny fixes may need no records. Epics/Atomics, Markdown/knowledge writers, ongoing automatic Git, agents, compaction and indexes remain deferred.

## Configure and start

Settings and the project registry are separate. Select an absolute config path; the default is HOME/.agent-tasks/config.toml. Registration creates missing settings. Root parents must already exist.

config.toml contains settings only:

```toml
schema_version = 1
```

Sibling projects.toml is the machine-managed alias registry:

```toml
schema_version = 1
[aliases]
product = "/absolute/project/documentation"
other = "/absolute/other/documentation"
```

```bash
cargo fetch --locked
cargo run --locked -- --config /absolute/config.toml mcp
```

Precedence: --config, AGENT_TASKS_CONFIG, then $HOME/.agent-tasks/config.toml. The old Linear config is not read. Identity/discovery/doctor/export need no valid config. Every business call takes project=<alias>; no global current project exists.

## Tools

| Tool | Purpose |
|---|---|
| get_status | Product identity and declared qualification |
| register_project | Create documentation, initialize Git and register its alias |
| get_project_list | Discover aliases and manifest-derived names/descriptions without paths |
| get_context | Intent/evidence/conditions, versions and bounded detail views |
| project_status | One owner-ready progress/leads/blockers/review overview |
| search | Bounded lexical work search with references/excerpts |
| plan_work | Explicit init, Project/Module edits, Module creation and Task planning |
| record_work | Current results/checks/artifacts, blocker/handoff, reasoned cancel/reopen |
| review_module | Independent Module verdict and retained check provenance |

Live descriptions are mini documentation. Shapes are closed. Edit omission preserves, null clears optional fields, [] clears lists. A result replaces the complete current report. Tasks have no separate review. Accepted Module review closes its derived phase without a hidden external merge gate.

## Workflow

Role skills describe the current portable workflow and its limits:

- [Orchestrator](skills/agent-tasks-orchestrator/SKILL.md): project discovery, planning, one lead per Module, status and independent acceptance.
- [Module lead](skills/agent-tasks-module-lead/SKILL.md): assignment context, Task execution, evidence, blockers/handoffs and review corrections.

Install each skill directory through the host's skill mechanism. These instructions do not add tools, initialize project records or implement deferred roadmap entities.

```text
get_project_list()
register_project(project="product", doc_dir="/absolute/project/documentation",
                 name="Product", description="Strategic purpose")
get_context(project="product")
plan_work(project="product", op="create_module", version="<allocation_version>",
          title="Portable storage", outcome="Guarded file updates",
          lead={name:"lead"}, tasks=[{title:"Protect stale writes",
                                     required_checks:["stale write"]}])
get_context(project="product", ref="M-001")
record_work(project="product", op="result", ref="M-001/T-001",
            version="<module Version>", state="done",
            summary="Stale replacement refuses",
            checks=[{label:"stale write",status:"passed"}])
get_context(project="product", ref="M-001")
review_module(project="product", module="M-001", version="<module Version>",
              verdict="accepted", summary="Outcome independently checked",
              actor="reviewer")
project_status(project="product")
```

Use actual returned versions: init/create_module use Allocation version; edit_project manifest Version; Module/Task writes whole Module Version. IDs/UTC dates/activity are generated. Reads never create roots. The orchestrator may translate status without changing coverage/unknown facts. Reported references/checks do not prove live agents or queried Git contents.

register_project needs no prior version. It creates project.yaml, modules/, .agent-tasks/state.yaml, README.md and .gitignore, initializes a local Git repository and commits the four bootstrap files before publishing the alias. Identical completed registration is a no-op; conflicts never overwrite or retarget. The source repository remote is manifest metadata; optional docs_remote sets a separate documentation origin without contacting it. No push or ongoing auto-commit occurs. Partial failures retain files/staging and disclosed effects for inspection.

get_project_list accepts {} and optional start/limit/version paging. Missing or unreadable roots remain unavailable entries. Descriptions are previews; get_context supplies full intent. For existing 0.7.0 settings, explicitly move [aliases] into sibling projects.toml while retaining schema_version = 1 in both files. Reads never perform that migration.

## Storage and recovery

project.yaml is root orientation; modules/M-001.yaml owns a Module, Tasks, current evidence and reviews; .agent-tasks/state.yaml owns durable Module numbering. Roots are portable and selected independently by aliases.

Writes lock and compare root-bound versions. Synced temps publish new files without clobbering; normalization preserves exact originals first. Lost/partial/uncertain replies require context inspection before another mutation. Restore missing counters from retained state, never guess. No status cascade, implicit rollback or replay-idempotency exists.

Context/search pages use actual returned start and Snapshot version with unchanged scope/view/query/review selection. Data/detail coverage is explicit. Long status can be narrowed by module; unreadable work is unknown, not zero. Records keep closing headroom and a disclosed generated-log tail. Human reports/reviews/reasons are not silently pruned. See docs/architecture.md for limits and filesystem boundaries.

## Development

```bash
cargo xtask check
cargo test --frozen -p agent-tasks core_
cargo xtask contract update
cargo xtask contract check
cargo deny fetch
cargo deny check
cargo run --locked -- doctor --json
```

Rust registry definitions are authoritative; schemas/tools.json is a reviewed export. Product layouts are embedded in src/tools/work.rs. Source documentation changes with behavior; Cargo.lock is committed. No Python/Node tooling is needed. Finish coherent work with cargo xtask check.

## Delivery and template

The template baseline is pinned in .family/origin.json. Upgrades are read-only plans applied through reviewed Git changes. Runtime/build needs no private template access.

Release is enabled for macOS arm64 with MCP Inspector CLI 2.7.0 host acceptance recorded in docs/RELEASE_ACCEPTANCE.md. This qualification does not certify other hosts/platforms or power-loss durability. Installation never restarts services, changes host configuration or migrates portable roots. Binary rollback does not undo stored work. Package only committed source:

```bash
cargo xtask package
cargo xtask package verify dist/agent-tasks-0.1.0-aarch64-apple-darwin
cargo xtask template diff --from /trusted/template/checkout
cargo xtask template upgrade --from /trusted/template/checkout --dry-run
```
