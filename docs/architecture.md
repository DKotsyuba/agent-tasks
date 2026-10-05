# Architecture

Implemented core: Rust 2024, pinned toolchain, in-process stdio MCP, portable file state and one binary. Release qualification covers macOS arm64 and MCP Inspector CLI 2.7.0 only; see RELEASE_ACCEPTANCE.md.

## Boundaries

Source/docstrings describe implemented code; this tracker owns strategic intent, current agent reports and review; Git owns committed history. Artifact/session/remote strings are reported locations. The core does not inspect code, Git, GitHub or live agents. All project content is English; the orchestrator may translate a complete status without changing facts/coverage.

Tiny changes may use no tracker records. The implemented hierarchy is Project → standalone Module → embedded Task. Epics, Atomics, Markdown/knowledge writers, runbooks, automatic Git, agents, compaction, indexes and live-runtime queries remain deferred in the full roadmap.

| Source | Responsibility |
|---|---|
| src/main.rs | CLI, configuration-independent identity/discovery, protocol routing |
| src/model.rs | Closed records, semantic validation, acceptance/review basis |
| src/store.rs | Lazy aliases, bounded files, versions, locks, allocation/publication |
| src/tools/input.rs | Closed flat operations and presence-aware edits |
| src/tools/work.rs | Planning, current reports, independent review, immutable receipts |
| src/tools/read.rs | Semantic context/search/status, coverage and actual pagination |
| src/response.rs | Strict embedded MiniJinja and 8 KiB output cap |

Rust registry definitions are authoritative. Schemars derives structural schemas from the same closed serde inputs; every tool has the explicit root type object required by MCP, including operation unions. schemas/tools.json is a reviewed export. Readiness, UTF-8 byte bounds and versions are separate domain conditions. Storage DTOs never enter templates.

## Configuration and state

Precedence: absolute --config, AGENT_TASKS_CONFIG, then $HOME/.agent-tasks/config.toml. Capture only the location at startup; reload sibling projects.toml for each business call. Missing config never blocks identity/discovery/doctor/export. Settings contain schema_version only. Inline aliases refuse with an explicit migration instruction; reads do not move them. The following registry lives in projects.toml:

```toml
schema_version = 1
[aliases]
product = "/absolute/project/documentation"
other = "/absolute/other/documentation"
```

Every business call takes project=<alias>. Resolve one canonical root per request. No raw-root argument/shared current project exists. An absent root requires an existing parent and one final directory name. Same-root aliases coordinate; differently located clones remain independent and retargeting invalidates old versions.

register_project is the explicit one-call entrypoint. It accepts an alias, absolute doc_dir, English name/description, optional reported source remote and separate docs_remote. It serializes registration with a sibling projects.lock, then locks the documentation root. Foreign files, conflicting aliases/metadata and Git worktree files refuse. Matching completed registration is unchanged even after human documents are added. Partial bootstrap effects are retained, and an identical inspected retry may finish them. The registry binding is published only after successful local Git bootstrap. No cross-directory transaction or automatic rollback is claimed.

Bootstrap reuses normal allocator/manifest publication, adds README.md and .gitignore and initializes Git with branch main. It commits only the four bootstrap files, using the declared machine identity agent-tasks / agent-tasks@localhost. Existing HEAD/history is preserved; hooks and signing are not bypassed. Git calls have a 30-second bound, receive argv directly, suppress provider output and remove inherited directory/index overrides. No remote push/fetch occurs and ordinary work writes remain uncommitted.

Hosts with a minimal stdio environment must pass SSH_AUTH_SOCK when the operator's Git signing policy uses an SSH agent. Missing signing capability leaves files/staging intact and does not publish the alias. Repair the host environment, inspect the repository, then repeat the same registration intent. Unit fixtures isolate machine Git configuration; shipping calls preserve it.

get_project_list reads bounded registry/manifests and renders alias/name/description previews without paths. Unavailable roots remain named rows and make data coverage partial. Paging snapshots bind registry and manifest observations; stale continuation refuses. The registry is limited to 64 KiB / 256 aliases. Settings and the registry are distinct files; project names/descriptions are not duplicated in the registry.

| Owned relative path | Data |
|---|---|
| project.yaml | Strategic title/purpose, optional reported remote, generated UTC dates |
| modules/M-001.yaml | Module, embedded Tasks, current evidence, blocker/handoff, reviews/reasons/log |
| .agent-tasks/state.yaml | Durable next Module number |
| .agent-tasks/write.lock | Root coordination, created only by mutations |
| .agent-tasks/backups/ | Exact originals preserved before normalization |

Reads create/repair nothing. Explicit init creates only the final root and owned directories, publishes allocator first, manifest second. Valid empty partial initialization resumes only through explicit init with a fresh allocation version. Foreign/inconsistent combinations refuse.

IDs are positive monotonic M-001/T-001/L-001, minimum width three; public Task refs include the Module. Reserve Module numbers before publication; failed creation can leave a gap, never a reusable identity. Task/log counters publish in their Module. Missing/invalid counters block affected writes/allocations, not healthy reads. Restore retained state rather than guessing.

## Work and review

Tasks store open/done/canceled. Modules store open/canceled; planned/working/ready/changes requested/accepted/stale approval is derived. A result replaces current summary/checks/gaps/followups/artifacts. Done Tasks need meaningful reports; missing checks can remain unreported until Module acceptance. There is no Task review.

Acceptance needs no open Tasks/blocker/in-scope gaps, every explicit required check passed and delivery evidence. Canceled children are excluded; with no completed Task, the Module needs its own report. not_applicable does not waive a required check. Accepted independent review directly yields accepted; no hidden PR/merge gate exists.

Reviews apply check updates before basis/epoch capture, preserving the lead summary author/date and before/after check provenance. Epoch advances once per verdict. Known lead self-review refuses; unknown identities remain unknown. Saved changes_requested is successful tool execution.

Basis includes current plan/lifecycle/blocker/result/check substance (including details), lead and Task facts/current cancellation reasons. It excludes dates/actors, handoff, counters, activity and review history. Managed semantic changes/reopen advance epoch and invalidate approval; native drift is displayed without rollback/repair. Formatting changes byte version, not semantic approval.

Canceled targets permit reopen only; canceled parents block child writes. Parent cancellation requires terminal children, never cascades. Clear/reopen reasons and dated cancellation facts are retained outside the evictable generated tail. Partial edit omission preserves, null clears optional remote/lead/criterion only, [] clears lists. Results are complete replacements.

## Publication, limits and outcomes

Closed schema version 1 supports one YAML document with ordinary string-key maps/sequences/scalars. Reject tags/anchors/aliases/merges/complex keys, duplicates, unknown fields/revisions and invalid owned data. Conservative preflight prevents alias expansion; quote literal special tokens. serde_yaml_ng 0.10.0 supports YAML 1.1; no YAML 1.2/comment-preserving claim is made.

Read cap+one before parsing. Canonical writes serialize typed data. Before normalizing external formatting/comments, preserve exact observed original bytes in a no-clobber version-bound backup. Failed preservation leaves work untouched.

One fresh OS advisory root lock covers reload/validation/version/allocation/publication. Readers share an existing lock without creating it; contention returns busy. Native editors ignoring locks can race: rechecks detect ordinary drift but are not universal atomic compare-and-swap.

Write/sync a same-directory exclusive temp. Publish new records by hard-link no-clobber; recheck observed bytes then rename replacements. Sync parent and clean only the call's temp. Unsupported publication fails with no overwrite fallback. Orphan temps are ignored with warnings; foreign entries block allocation.

Post-publication directory-sync failure returns isError=true with visible effects and uncertain durability. Partial reservations/init/backups/setup effects remain explicit. Confirmed writes retain target/version if rendering fails. Lost replies require context/activity/inventory inspection before another mutation. Versions are not request identities; equal text is not replay-idempotence. No automatic retry/rollback/journal/repair exists.

| Policy | Bound |
|---|---|
| Config / record / aggregate scan | 64 KiB / 512 KiB / 512 entries and 16 MiB |
| Tasks / generated recent log / nonterminal reserve | 32 / 256 / 32 KiB |
| Title / lead name / handle | 256 / 128 / 256 UTF-8 bytes |
| Purpose/outcome/criterion/result/review summary | 1024 UTF-8 bytes |
| Required labels/checks/review updates | 8 per target / 8 per target / 8 per verdict |
| Label/detail / gaps/followups/artifacts/findings | 64/256 bytes / 8 entries each, 256 bytes |
| Blocker/handoff/reason / resolver / UTC dates | 512 / 128 / 40 bytes |
| Text / requested context page | 8 KiB / 1–20 rows |

Only generated log events are evicted. Human reports/reasons/cancellations and review history are never silently pruned. File capacity bounds retained history; ordinary writes retain closing reserve, accepted review/cancel may consume it. This is not an unlimited full log; Git retains committed history.

Versions bind canonical root, relative target and exact bytes. Project context separates editable manifest Version, creation Allocation version and pagination Snapshot version. Task writes use whole Module versions; siblings/review can stale them, another Module cannot. Creation scope hashes manifest/allocator and filenames, not report contents. After reservation recheck intended allocator plus unchanged manifest/inventory.

Healthy aggregate records remain readable with named omissions. Data/detail coverage are separate; PARTIAL counts are lower bounds and unknown work is not zero. project_status reports scoped progress/leads/summaries/blockers/review attention/last report; never live agent presence. The declared small fixture is three Modules/twelve Tasks/three leads/two blockers. Larger detail may need Module narrowing.

Search is Unicode-lowercase all-term semantic-field substring matching, ranked by matching-field count then numeric ref. It is not Markdown/code/semantic search. Project hits open with omitted ref; Module/Task hits use their returned ref. Receipts explicitly label the owning Module phase, distinct from Task completion. Empty detail rows use view-specific wording. Continuation hashes bind tool/ref/query/view/review selection and the exact data snapshot; changing a selection refuses instead of skipping new rows. Editable file Version is distinct. Choose the largest fitting prefix, advance by displayed rows; never post-truncate or skip.

## Verification and delivery

Focused tests cover operation shapes against JSON Schema and serde, native file publication/permissions/locks, stale/independent writes, partial init and allocation gaps/loss, backups, cancellation, reviewer attribution/approval invalidation, bounded status/search/pages/scans, recent-log eviction, closing reserve, renderer fallback and injected post-publication sync failure.

The real SDK/stdio test plans work, completes a Task, independently accepts a Module, reads status/search, restarts the binary and checks persisted closure. Existing protocol tests preserve discovery equality, modern private cache hints, legacy omission, invalid/unknown calls and EOF.

Full gate: cargo xtask check. Supply-chain gate: cargo deny check after explicit fetch. Native acceptance additionally uses the independent MCP Inspector CLI, including fresh-process work persistence. This is not power-loss certification or qualification of other platforms/filesystems/hosts.

Standard installation manages immutable releases under declared product home/bin only. Portable roots are external user-selected data. Binary rollback does not undo their data/schema/effects. Migration/repair or service restart is never implicit.
