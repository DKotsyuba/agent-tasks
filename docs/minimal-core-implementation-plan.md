# Minimal core — technical implementation plan

Status: design only; no core implementation or qualification yet. This narrows the first deliverable while preserving the [full structured roadmap](architecture-proposal.md). The source baseline remains the standard Rust MCP starter. All project content is English; the orchestrator may translate the complete status when presenting it to the owner.

The proposed MVP is a file-backed work tracker inside the existing Rust binary: a Project manifest, standalone Modules containing Tasks, six work tools, and unchanged diagnostic `get_status`. It remains an implementation plan; no future-core behavior or qualification has been demonstrated.

## 1. Scope and module boundaries

Keep the pinned Rust toolchain, rmcp, strict embedded MiniJinja, stdio, and one binary. Add no daemon, database, background jobs, source integration, or architectural workspace crates.

Include configuration aliases, explicit initialization, Module/Task planning, current results, blockers, handoff, independent Module review, lexical work search, and one-call status. Reads create nothing. Microfixes and tiny projects may use no tracker.

Preserve the broader roadmap, but defer Epics, Atomics, knowledge/document writers, Markdown retrieval, checkpoints, automatic Git, caches, semantic search, compaction, and live-agent queries. The documents explicitly describe proposed rather than implemented behavior: [product-direction.md](../docs/product-direction.md#L3), [architecture-proposal.md](../docs/architecture-proposal.md#L3).

Use ordinary app modules:

| File | Responsibility |
|---|---|
| Existing `src/main.rs` | Select configuration location and pass a lazy store adapter to work calls. Keep identity and registry construction configuration-independent. |
| Existing `src/tools/mod.rs` | Authoritative tool definitions, routes, template registration, readiness. |
| New `src/store.rs` | Storage/config types, bounded filesystem access, validation, versions, locking, numbering, publication. |
| New `src/tools/work.rs` | Six argument contracts, work transitions, retrieval, typed result/view projection. |
| Existing `src/response.rs` and embedded templates | Normal layouts and outcome-preserving fallback. |

No repository-wide refactor or shared delivery-library changes. `Handler::new` is also called by doctor and contract export, so it must not require configuration: [main.rs](../src/main.rs#L71), [callers](../src/main.rs#L178). Registry insertion points already exist in [tools/mod.rs](../src/tools/mod.rs#L8).

## 2. Versioned storage

Use closed typed schemas with `schema_version: 1`. Reject unknown fields/versions, duplicate keys/IDs, invalid counters, inconsistent references, and invalid transitions.

The exact layout is relative to the configured root:

| Path | Meaning |
|---|---|
| `project.yaml` | Root manifest |
| `modules/M-001.yaml` | Module under the root's `modules/` |
| `.agent-tasks/state.yaml` | Allocator |
| `.agent-tasks/write.lock` | Coordination file created only by writes |
| `.agent-tasks/backups/` | Originals preserved before externally reformatted YAML is normalized |

Explicit init may create the final configured root only when its parent already exists.
It prepares `modules/` and `.agent-tasks/`, then takes the write lock; no mkdir-p
ancestor chain. Technical directory/lock creation is not business publication.

| Record | Agent-supplied fields | Generated fields |
|---|---|---|
| `project.yaml` | Required `title`, `purpose`; optional `remote` | Schema version, creation/update dates |
| `.agent-tasks/state.yaml` | None | Schema version, `next_module` |
| `modules/M-001.yaml` | Required `title`, `outcome`; optional `lead`; `required_checks` defaults empty | ID, dates, state, `next_task`, `next_log`, embedded Tasks/current reports/review/log |
| Embedded Task | Required `title`; optional `criterion`; `required_checks` defaults empty | Local ID, dates, state, current result, cancellation reason |

IDs use monotonic positive integers, minimum-width formatting: `M-001`, `T-001`, `L-001`. Public Task references include the owner: `M-001/T-001`. Counters store the next unused number. Gaps are allowed; cancellation retains records. Ordinary tools never delete, renumber, or recycle identities.

Tasks have `open | done | canceled`. Modules retain an open/canceled lifecycle; presentation derives planned, working, ready, changes requested, accepted, or stale approval from current work/review facts.

Current result:

- Required `summary`.
- Target-owned current checks, default empty: `{label, status, detail?, reported_at, actor}`.
  The last two fields are generated. Check reports are alongside the result summary,
  so reviewer-only checks can exist on a Module whose summary is derived from Tasks.
- Optional bounded `gaps`, `followups`, and reported `artifacts`.
- Generated report date and nullable actor attribution.

Check status is `passed | failed | not_run | not_applicable`. Required check labels are unique within their target. Missing checks mean not reported. No check-ID registry.

Store the result directly on its target. Replacing it replaces the current report; no event-reference chain or replay engine. Full historical result retention is deferred.

A Module may contain current blocker `{problem, needed_action, resolver?}`, handoff `{stopping_point, next_action}`, and lead `{name, handle?}`. A handle is a reported location, never liveness.

Review records are `{verdict, summary, findings, basis, at, reviewer}`; verdict is `accepted | changes_requested`, findings are `{text, must_fix}`. Keep a bounded review-history list inside the Module so a fresh verdict does not erase earlier findings when automatic Git is absent. The latest review's applicability is derived from its basis; older/stale reviews are history and rework guidance, not current approval.

A review also has generated `epoch` and `check_updates:[{target,label,before,after}]`
snapshots. Each snapshot retains status/detail/reporting date/actor; before can be
absent. This preserves a lead's failure or omission when a reviewer reports a pass.
The Module has generated `review_epoch`. History is never silently pruned; the
serialized record capacity bounds retention, with closing headroom.

Generated log entries are `{id, at, actor, target, action, note?}`: short machine
change labels/check counts, not another copy of agent substance. Retain a recent
tail of 256 entries, with generated `omitted_log_entries` and retained range.
Evict only these generated events; never current results, handoff/blocker text,
cancellation reasons or review findings. IDs stay monotonic. Log reads disclose
the clipped window. Full historical work-log retention is outside this core.

Generate UTC RFC 3339 dates through one clock/format helper; never ask the caller for dates. Use `SystemTime` as the observation source and a small existing date library for formatting/serde, rather than hand-writing calendar arithmetic. `chrono` is already present transitively in the lockfile; add a direct minimal-feature dependency only for this need. Log IDs define ordering; clock changes do not. Unknown identity stays unknown. Declared labels remain explicitly declared.

**Canonical YAML subset:** one document, fixed keys, ordinary mappings/sequences/
scalars and bounded nesting. No custom tags, anchors, aliases, merge keys or complex
keys. Read valid noncanonical records diagnostically. Before normalizing external
formatting/comments on a mutation, preserve the exact original in a no-clobber
version-bound backup below `.agent-tasks/backups/`; if that preservation fails,
refuse the record change. Report normalization and backup route. Unknown fields/
versions still refuse rewriting. This is a scoped backup, not an operation journal.

`serde_yaml_ng 0.10.0` documents YAML 1.1 support and default enum tags. Qualification must prove the owned schema’s tag-free representation and scalar round trips; this plan makes no YAML 1.2 or comment-preservation claim. [Official crate documentation](https://docs.rs/serde_yaml_ng/0.10.0/serde_yaml_ng/)

## 3. Six tool contracts

Every work call requires `project`. All objects reject unknown fields. Operation variants have a flat `op` discriminator and variant-specific allowed fields; no patch bags. `?` below means optional.

| Tool | Semantic input and operation |
|---|---|
| `get_context` | `{project, ref?, view?, start?, limit?, version?, review_index?}`. Ref defaults to Project; otherwise Module/Task. View defaults `summary`; additional views are `tasks`, `results`, `checks`, `review`, `log`. Defaults: start 0, limit 20; maximum 20. Return context, coverage, whole-file version, and real continuation parameters. Project context supplies manifest `version` and `allocation_version`. `review_index` selects retained review history (default latest); review detail items are pageable. |
| `project_status` | `{project, module?}`. Return counts, Module/Task states, current results/checks, leads, blockers, review attention, last reported activity, and coverage. Read-only; no version needed. |
| `search` | `{project, query, module?, start?, limit?, version?}`. Query: 1–8 whitespace-separated words, maximum 256 bytes. Match all words using Unicode lowercase substrings over semantic fields. Rank by matching-field count, then numeric reference. Default/maximum limit 20. Return references, matched fields, excerpts, counts, and snapshot-bound continuation. |
| `plan_work` | Common `{project,op,version,actor?}`. `init_project`: `{title,purpose,remote?}`; `edit_project`: `{title?,purpose?,remote?}`. `create_module`: `{title,outcome,lead?,required_checks?,tasks?}`. `edit_module`: `{module,title?,outcome?,lead?,required_checks?}`. `add_task`: `{module,title,criterion?,required_checks?}`. `edit_task`: `{ref,title?,criterion?,required_checks?}`. Initial Tasks use the same semantic Task shape without generated fields. |
| `record_work` | Common `{project,op,ref,version,actor?}`. `result`: `{summary,state?,checks?,gaps?,followups?,artifacts?}`; Task state is open/done, default preserve. `blocker`: `{problem,needed_action,resolver?}`. `handoff`: `{stopping_point,next_action}`. `clear_blocker`, `clear_handoff`, `cancel`, `reopen`: `{reason}`. Blocker/handoff operations target Modules. |
| `review_module` | `{project,module,version,verdict,summary,findings?,checks?,actor?}`. Reviewer checks are `{target,label,status,detail?}` and update canonical check reports before review. Preserve the original result-summary author/date; individual check reports receive their own generated reporting date/actor so a reviewer update does not falsely reattribute the lead's summary. It records accepted or changes requested; Module phase is derived from that applicable verdict. |

`edit_*` applies a partial semantic edit: omitted fields preserve values; explicit null clears only optional fields, and an explicit empty list clears a list. Required title/outcome/purpose cannot be cleared. Distinguish absent versus null in the argument adapter using a small presence-aware decoder; no generic patch language. Generated IDs and unrelated Tasks/results/logs remain intact. `result` is a new complete report: it replaces the current summary and that target's check-report set in one write; it is not a partial plan edit.

Done Tasks require meaningful results, but may have unreported checks. Acceptance requires no open Tasks, no blocker, no in-scope gaps, and every required check passed. `not_applicable` does not waive requirements. Canceled work needs a reason. A Module without completed Tasks needs its own meaningful result: canceling all children does not demonstrate an implementation outcome.

Accepted review cannot contain must-fix findings. It records accepted; derived
status is accepted while epoch/basis remains applicable. Known reviewer matching
the lead is refused; missing identities must not compare as equal. Delivery/PR
refs remain reported, never a hidden merge gate.

| Condition | Allowed operations |
|---|---|
| Open Module: planned/working/ready/changes requested | Plan edits, results, blocker/handoff changes; review when acceptance holds |
| Applicable accepted Module | Reads/handoff; semantic edits/results/blocker changes are allowed with the version and invalidate old approval |
| Canceled Module | `reopen` only; child writes wait for the parent |
| Open/done Task in open Module | Edit/result/cancel; done requires meaningful summary |
| Canceled Task | `reopen` only; parent must be open |
| Module cancel | Reason and all children terminal; no cascade |
| Explicit reopen | Reason; clear active cancellation and advance review epoch |

Absent blocker/handoff clear is a no-op. Keep historical cancellation as a dated
fact. Reads never mutate state or create work.

Examples use actual returned versions in place of placeholders:

```text
get_context(project="alpha")

plan_work(project="alpha", op="create_module", version="<allocation version>",
          title="Portable storage", outcome="Guarded file updates",
          lead={name:"lead-a"},
          tasks=[{title:"Preserve stale edits",required_checks:["stale write"]}])

record_work(project="alpha", op="result", ref="M-001/T-001",
            version="<module version>", state="done",
            summary="Stale replacements refuse without changing the file",
            checks=[{label:"stale write",status:"passed"}])

review_module(project="alpha", module="M-001", version="<new version>",
              verdict="accepted", summary="Outcome independently verified",
              actor="reviewer-b")

project_status(project="alpha")
search(project="alpha", query="stale write")
```

Return typed acknowledgement/entity/page/status/error views. Confirmed writes use `isError=false`; validation, stale refusal, partial mutation, and unknown outcome use true. Partial reads use false with explicit coverage. Unknown tools retain the current protocol-error route: [tools/mod.rs](../src/tools/mod.rs#L36).

Read tools are read-only/idempotent. Replacement-capable mutations advertise destructive effects and no idempotency guarantee. All remain local closed-world operations.

## 4. Configuration and filesystem safety

Add a global `--config <absolute-path>` CLI option, accepted with the MCP command. Otherwise select `AGENT_TASKS_CONFIG`, then `$HOME/.agent-tasks/config.toml`. Do not implicitly read the discontinued installed product's `$HOME/.config/agent-tasks/config.toml`. Capture the location once; load configuration only when a work call needs it.

Missing configuration must not affect identity, discovery, doctor, or contract export. Work calls return a safe configuration error. Never write installed configuration.

Configuration contains only:

```toml
schema_version = 1
[aliases]
alpha = "/absolute/documentation/root"
```

The operator explicitly creates this TOML file/alias before business calls; the
MCP never bootstraps configuration from discovery.

No raw-root argument, registration tool, or shared current Project. Reject relative roots and unknown aliases. Resolve/canonicalize per call and hold the resolved root through the operation.

Access only the manifest, allocator, immediate Module records and their owned coordination/backup/temp paths. No recursive source traversal. Reject symlinked owned children and nonregular record files; recheck before publication.

Initial policy limits to qualify: config 64 KiB, record 512 KiB, scan 512 Modules
AND 16 MiB (first bound wins), 32 Tasks per Module, generated log tail 256.
Cap-plus-one reads precede parsing. Normal edits leave 32 KiB headroom for a terminal
review/cancel. Check exact serialized candidate size before publishing; no deletion
to fit. Give early capacity warnings. The core does not promise unbounded history.

| Value | UTF-8/count cap |
|---|---|
| title / lead name / handle | 256 / 128 / 256 bytes |
| purpose/outcome/criterion/result or review summary | 1024 bytes each |
| required labels / checks / reviewer updates | 8 per target / 8 per target / 8 per verdict |
| check label/detail | 64 / 256 bytes |
| gaps/followups/artifacts | 8 each, 256 bytes per value |
| blocker/handoff/reason text, resolver | 512 bytes per text, resolver 128 |
| review findings | 8, 256 bytes per finding |

Review history has no count-based silent eviction; exact file capacity applies.
Qualify closing reserve with worst permitted serialization/escaping. At capacity,
reduce current detail or start coherent continuation work; preserve previous bytes.
`changes_requested` is nonterminal and must retain the accepting-review reserve;
only accepted verdict or cancellation can consume terminal headroom.

An unreadable/oversize Module becomes a named unreadable row in aggregate reads;
other Modules still render. Unknown counts never become zero. Scan overflow is
PARTIAL on reads. Allocation inventory is separate: content exceeding 16 MiB does
not prevent filename/ID enumeration. Creation refuses if inventory exceeds 512
files or contains symlinks, nonregular entries or foreign/unrecognized names;
only the documented own-temp pattern is ignored with a warning.
Ignore publication temps in inventory and warn about orphans; do not remove them
as if they were this call's files.

Use one persistent `.agent-tasks/write.lock`, created only by mutations. A fresh `File::try_lock` handle covers reload, validation, version comparison, allocation, and publication. Contention returns `busy`. Readers use an existing `try_lock_shared` handle where available and never create it; returning busy during a write is intended. Stdlib locking is documented stable since 1.89, below pinned 1.98.1. [Rust File locking](https://doc.rust-lang.org/std/fs/struct.File.html#method.try_lock)

Publication uses a fully written/synced same-directory temp named
`.M-001.yaml.tmp-<pid>-<unique-suffix>` (target-specific name), created exclusively.
Existing records use qualified rename after a final version check; new records
use qualified no-clobber hard-link publication. Unsupported hard links fail clearly,
never fallback to clobbering rename. Sync the parent; clean only this call's temp.

Do not populate the final path in place: exclusive creation alone does not prevent readers seeing incomplete contents. [Rust exclusive creation](https://doc.rust-lang.org/std/fs/struct.File.html#method.create_new)

Initialize allocator first, manifest second, without clobbering. Reads never finish
init. Empty partial init means a valid allocator with `next_module=1`, no manifest
and no Module records. Fresh context returns that condition and allocation version.
Explicit init with that version publishes only missing records/directories.
Inconsistent/foreign combinations refuse. Old-version retry after partial
publication conflicts, so inspect first.

For an alias whose root does not yet exist, read-only context may resolve the existing parent and produce an absence-version without creating the directory. Explicit `init_project` may create the configured root after revalidation; no reads create it. Reuse validated allocator identity on a partial empty initialization and never overwrite foreign or inconsistent files.

Reserve Module numbers durably before publication. Failure leaves a gap. Task/log IDs and local counters publish together in one Module replacement. Validate counters against retained identities. Missing/corrupt counters refuse allocation; do not infer lost reservation history from surviving filenames. External rollback of reservation history cannot be reliably detected without additional history, which this core does not provide. This is an explicit limit: allocator loss keeps healthy context/status reads usable but blocks new allocations until valid state is restored from a retained copy/version control. The MVP does not add a repair/migration framework or silently guess zero.

## 5. Version and review contracts

Opt into existing workspace `sha2` for versions. A UUID crate is unnecessary in this core because references are scoped to the resolved root. A direct minimal-feature date/format dependency is allowed for readable UTC metadata; do not invent a custom date framework. Version is a fixed-length opaque hash over domain, canonical root, relative target
and exact bytes.

| Operation | Version from context |
|---|---|
| init_project / create_module | Project `allocation_version` |
| edit_project | Manifest `version` |
| Module/Task plan/work/review | Owner Module `version` |
| continuation | That exact view/scope's snapshot version |

Allocation scope hashes prospective canonical root, manifest/allocator bytes or
absence markers, and ordered Module filenames/IDs, not report contents. Missing/
empty modules directories mean the same empty inventory; own root/lock/directory
setup does not change the absence version. Allocator publication does.
Compare pre-reservation scope under lock. After reservation, recheck manifest/
inventory plus expected reserved allocator, not the old full digest.
Fresh init follows the same self-change rule: after allocator publication, recheck
that manifest remains absent and allocator bytes equal the intended state before
publishing the manifest.

Task writes compare the entire Module file. Another Task’s update or review metadata makes an old version stale; another Module does not. This deliberately fits one persistent lead per Module.

Allocation versions are creation preconditions, not read certificates. An absent root uses its existing canonical parent plus configured final name; invalid/non-directory/escaping targets have no absence version.

Same-root aliases coordinate. Alias retargeting and differently located clones invalidate old versions. Clones remain independent snapshots, not a distributed allocator.

Review applicability requires matching generated epoch and semantic basis: Module
title/outcome/lead/required checks/open-canceled lifecycle/blocker/current result
and check statuses, plus Task title/criterion/required checks/state/current result/
check statuses/cancellation reason. Exclude handoff, dates/actors, log, counters,
review history and display state. Apply reviewer reports before computing the new
basis. Store it as a fixed digest, not a copy of the Module.
`review_module` advances the epoch exactly once after applying its check updates,
and records that epoch. Approval requires both epoch and basis to match. Review
metadata changes byte version without self-invalidating approval.

A managed semantic change makes old approval historical and advances review epoch;
explicit reopen also advances it. It does not invent another stored phase machine:
open/canceled is stored and readiness/approval derived. Native semantic drift is
flagged on read without file writes. Formatting invalidates byte version, not
semantic approval. Project purpose is orientation, not inherited acceptance in MVP.
Only the enumerated basis-field changes advance semantic epoch; handoff or
machine date/log changes do not.

Stale refusal saves nothing and returns compact current context plus a fresh version. Lost replies require reading current work/log/inventory before another mutation. Old versions refuse replay; identical text does not establish request identity.

For creation AND rename replacement, post-publication directory-sync failure
reports visible effect with uncertain durability. Never claim nothing happened
or replay blindly. Name setup/backup effects separately from saved work.

Advisory locks coordinate MCP writers. Native editors ignoring them can race: final rechecks detect ordinary drift but cannot provide atomic compare-and-swap against all native edits. No transaction framework or operation journal is proposed.

## 6. Presentation and completeness

Reuse `response::Templates`: strict undefined handling, 50,000 fuel, recursion 16, and private 8 KiB bounded output already exist: [response.rs](../src/response.rs#L10). Target acknowledgements/errors below 2 KiB.

Use allowlisted views, safe quoted labels, exact IDs/versions, explicit omissions, and closed template selection. No normal YAML/JSON dumping.

Keep an immutable execution receipt before rendering. Presentation failure discards partial output and preserves confirmed, partial, or unknown effects with a usable recovery route. Never rerun effects: [response standard](../docs/MCP_RESPONSE_STANDARD.md#L298).

Continuations use real offsets/snapshot versions. Choose the largest prefix up to
limit that fits, reserve headers/warnings/continuation first, and advance by actual
displayed rows. Never post-truncate or skip. Only an unpresentable single row refuses.

Summary/results rows use bounded previews/counts and routes to checks/review detail.
A review page has its header plus pageable finding/check-update items;
`review_index` selects history. Module status shows brief work and omitted Task
detail with `get_context view=tasks`, not every report body. Keep coverage/blockers.

Qualify complete status on a declared fixture: three Modules, twelve Tasks, three leads, two blockers, mixed results/checks/reviews. Beyond limits, report `PARTIAL`, separate data/detail coverage, and provide Module narrowing. Counts cover tracked work; recent activity means last report, not liveness.

## 7. Implementation chunks and checks

| Chunk/owner | Deliverable and meaningful verification |
|---|---|
| Contract/config | Closed arguments, documented fields, lazy configuration; no-config discovery/status/export. |
| Store | Schemas, bounds, counters, locks/publication; stale writes, contention, allocator gaps, interrupted initialization. |
| Work | Planning/results/review; missing checks, cancellation, review self-stability and later invalidation. |
| Retrieval/presentation | Context/search/status; deterministic pages, complete small fixture, overflow, post-write render failure. |
| Integration review | Registry snapshot, supported MCP routes, failure recovery, architecture/profile documentation. |

Use focused Rust selectors after coherent chunks, then **`cargo xtask check`** at completion. Preserve credential-free discovery, invalid/unknown calls, modern cache hints, legacy omission, and EOF checks: [protocol.rs](../tests/protocol.rs#L23).

Suggested focused targets: `core_config`, `core_store`, `core_workflow`,
`core_presentation`, and existing `protocol`. Test every op's structural valid/
invalid/missing/unknown/null shapes against the exported schema as a REQUIRED
contract matrix, not an optional spike. Domain preconditions are separate from
JSON Schema. Do not hand-build a general validator framework for this test.

Resolve new crates, review/update Cargo.lock before frozen checks, and run the
existing cargo-deny gate after network preparation. YAML crate/closure are new;
chrono requires a minimal direct declaration despite transitive lock presence;
sha2 is a workspace opt-in. Rust registry definitions remain the schema authority.

Export/update the contract explicitly and review its diff. Generated stubs remain release-blocking until implemented/reviewed.

Document the stateful file profile using the family’s accepted manifest vocabulary. Keep in-process, stdio, single-binary delivery. Do not retain `state=none` after implementing storage or alter qualification/release flags to pass checks: [family.toml](../family.toml#L9).

## 8. Short qualification spikes

Before committing storage architecture choices:

- Prove YAML duplicate-key rejection, scalar round trips, tag-free schemas,
  bounded parsing and canonical comparison; backup preserves original comments
  before normalization.
- Prove locks, no-clobber creation, atomic replacement, permissions, crash recovery, and directory syncing on macOS/APFS.
- Prove normalization preserves its exact original, generated log-tail eviction loses no user substance, and closing reserve prevents a log-cap dead end.
- Prove actual SDK acceptance/export of flat closed variants, runtime/schema agreement, and configuration-independent discovery.

Other platforms/filesystems remain unqualified until equivalent evidence exists.

## Planning evidence and primary references

The source integration map was checked read-only against the existing CLI, registry, presenter and protocol tests. The selected library and filesystem contracts above are candidates for the listed qualification spikes, not already passed tests.

- [serde_yaml_ng 0.10.0](https://docs.rs/serde_yaml_ng/0.10.0/serde_yaml_ng/): typed serde serialization/deserialization; explicitly documents YAML 1.1 support.
- [Rust File locks](https://doc.rust-lang.org/std/fs/struct.File.html#method.try_lock): stdlib lock APIs stable since 1.89, with platform-specific behavior toward non-cooperating writers.
- [Rust rename](https://doc.rust-lang.org/std/fs/fn.rename.html): may replace an existing target; it is not a no-clobber creation primitive.
- [Rust exclusive creation](https://doc.rust-lang.org/std/fs/struct.OpenOptions.html#method.create_new): fails if the target already exists.
- [Rust hard links](https://doc.rust-lang.org/std/fs/fn.hard_link.html): candidate for publishing fully written new records without clobbering; native qualification is required.

No source, fixtures, installed configuration, service or release state has been changed by this technical planning work. `docs/architecture.md` remains the current implementation description until the core is actually written.
