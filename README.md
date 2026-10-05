# agent-tasks

Rust stdio MCP for portable strategic work. Agents plan Modules/Tasks, report outcomes and independently review a whole Module. The owner gets one compact English status. YAML is structured internal storage; strict MiniJinja renders semantic text. Code/docstrings and Git retain their authority.

This is the minimal local core. The full future roadmap stays in docs/architecture-proposal.md. Tiny fixes may need no records. Epics/Atomics, Markdown/knowledge writers, automatic Git, agents, compaction and indexes remain deferred.

## Configure and start

The operator creates an absolute config. Root parents must already exist:

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
| get_context | Intent/evidence/conditions, versions and bounded detail views |
| project_status | One owner-ready progress/leads/blockers/review overview |
| search | Bounded lexical work search with references/excerpts |
| plan_work | Explicit init, Project/Module edits, Module creation and Task planning |
| record_work | Current results/checks/artifacts, blocker/handoff, reasoned cancel/reopen |
| review_module | Independent Module verdict and retained check provenance |

Live descriptions are mini documentation. Shapes are closed. Edit omission preserves, null clears optional fields, [] clears lists. A result replaces the complete current report. Tasks have no separate review. Accepted Module review closes its derived phase without a hidden external merge gate.

## Workflow

```text
get_context(project="product")
plan_work(project="product", op="init_project", version="<allocation_version>",
          title="Product", purpose="Strategic purpose")
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

Release stays disabled and qualification not_verified. Local SDK/filesystem checks are not certification of all hosts/platforms. Installation never restarts services, changes host configuration or migrates portable roots. Binary rollback does not undo stored work. Package only committed source:

```bash
cargo xtask package
cargo xtask package verify dist/agent-tasks-0.1.0-aarch64-apple-darwin
cargo xtask template diff --from /trusted/template/checkout
cargo xtask template upgrade --from /trusted/template/checkout --dry-run
```
