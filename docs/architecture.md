# Architecture

Implemented core: Rust 2024, pinned toolchain, in-process stdio MCP, portable file state and one binary. Release qualification covers macOS arm64 and MCP Inspector CLI 2.7.0 only; see RELEASE_ACCEPTANCE.md.

## Boundaries

Source/docstrings describe implemented code; this tracker owns strategic intent, current agent reports and review; Git owns committed history. Artifact/session/remote strings are reported locations. The core does not inspect code correctness, GitHub or live agents. Explicit commit-report import reads bounded local Git metadata/messages without Git writes or remote fetch. All project content is English; the orchestrator may translate a complete status without changing facts/coverage.

Tiny changes may use no tracker records. The implemented hierarchy is Project → Epic or standalone Module or Atomic; Epic → Module or standalone Atomic; Module → embedded Task or Atomic. Markdown/knowledge writers, runbooks, automatic Git, agents, compaction, indexes and live-runtime queries remain deferred in the full roadmap.

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
| epics/E-001.yaml | Epic intent/criteria, sole authoritative member references, own evidence/reviews |
| modules/M-001.yaml | Module, embedded Tasks/Atomics, current evidence, blocker/handoff, reviews/reasons/log |
| atomics/A-001.yaml | Standalone Project/Epic Atomic, executor, reports and integration evidence |
| .agent-tasks/state.yaml | Durable independent next Module, Epic and standalone Atomic numbers |
| .agent-tasks/write.lock | Root coordination, created only by mutations |
| .agent-tasks/backups/ | Exact originals preserved before normalization |

Reads create/repair nothing. Explicit init creates only the final root and owned directories, publishes allocator first, manifest second. Valid empty partial initialization resumes only through explicit init with a fresh allocation version. Foreign/inconsistent combinations refuse.

IDs are positive monotonic E-001/M-001/A-001/T-001/L-001, minimum width three. Public Task and embedded Atomic refs include the Module: M-001/T-001 and M-001/A-001. Reserve top-level numbers before publication; failed creation can leave a gap, never a reusable identity. Task/Atomic/log counters publish in their Module. Missing/invalid counters block affected writes/allocations, not healthy reads. Restore retained state rather than guessing.

## Work and review

Tasks and Atomics store open/done/canceled. Modules and Epics store open/canceled; planned/working/ready/changes requested/accepted/stale approval is derived. A result replaces current summary/checks/gaps/followups/artifacts. Done Tasks need meaningful reports; missing checks can remain unreported until Module acceptance. There is no Task review.

Module acceptance needs terminal Tasks, currently reviewed owned Atomics, no blocker/in-scope gaps, every explicit required check passed, applicable independent review and matching reported delivery for current workflow records. Canceled children are excluded; with no completed child, the Module needs its own meaningful report. not_applicable does not waive a required check. Managed-1 workflow (core absent) separates implementation approval from final accepted delivery: reported merge/delivery into the declared target branch closes a reviewed Module; a hosting PR is optional and no Git API/automatic merge is performed. Core-active Modules instead become ready for integration on a current positive review without delivery (see Epic core workflow). Legacy records preserve their prior approval interpretation until explicit begin opt-in.

Reviews apply check updates before basis/epoch capture, preserving the lead summary author/date and before/after check provenance. Epoch advances once per verdict. Known lead self-review refuses; unknown identities remain unknown. Saved changes_requested is successful tool execution.

Basis includes current plan/lifecycle/blocker/result/check substance (including details), lead and embedded Task/Atomic facts/current cancellation reasons, current contract/dependency/workflow requirements. It excludes incidental dates/actors, handoff, counters, activity and review history. Delivery bookkeeping is outside the implementation basis, so reporting delivery after review does not require a second review; delivery itself is pinned to the reviewed basis. Managed semantic changes/reopen advance epoch and invalidate approval; native drift is displayed without rollback/repair. Formatting changes byte version, not semantic approval.

Canceled targets permit reopen only; canceled parents block child writes. Parent cancellation requires terminal children, never cascades. Clear/reopen reasons and dated cancellation facts are retained outside the evictable generated tail. Partial edit omission preserves, null clears optional remote/lead/criterion only, [] clears lists. Results are complete replacements.

## Publication, limits and outcomes

Closed work-record schema version 1, extended by explicitly defaulted fields for old 0.8.0 records, and allocator schema revision 2 support one YAML document with ordinary string-key maps/sequences/scalars. Reject tags/anchors/aliases/merges/complex keys, duplicates, unknown fields/revisions and invalid owned data. Conservative preflight prevents alias expansion; quote literal special tokens. serde_yaml_ng 0.10.0 supports YAML 1.1; no YAML 1.2/comment-preserving claim is made.

Read cap+one before parsing. Canonical writes serialize typed data. Before normalizing external formatting/comments, preserve exact observed original bytes in a no-clobber version-bound backup. Failed preservation leaves work untouched.

One fresh OS advisory root lock covers reload/validation/version/allocation/publication. Readers share an existing lock without creating it; contention returns busy. Native editors ignoring locks can race: rechecks detect ordinary drift but are not universal atomic compare-and-swap.

Write/sync a same-directory exclusive temp. Publish new records by hard-link no-clobber; recheck observed bytes then rename replacements. Sync parent and clean only the call's temp. Unsupported publication fails with no overwrite fallback. Orphan temps are ignored with warnings; foreign entries block allocation.

Post-publication directory-sync failure returns isError=true with visible effects and uncertain durability. Partial reservations/init/backups/setup effects remain explicit. Confirmed writes retain target/version if rendering fails. Lost replies require context/activity/inventory inspection before another mutation. Versions are not request identities; equal text is not replay-idempotence. No automatic retry/rollback/journal/repair exists.

| Policy | Bound |
|---|---|
| Config / record / aggregate scan | 64 KiB / 512 KiB / 512 entries and 16 MiB |
| Tasks and embedded Atomics per Module / generated recent log / nonterminal reserve | 32 each / 256 / 32 KiB |
| Title / lead name / handle | 256 / 128 / 256 UTF-8 bytes |
| Purpose/outcome/criterion/result/review summary | 1024 UTF-8 bytes |
| Required labels/checks/review updates | 8 per target / 8 per target / 8 per verdict |
| Label/detail / gaps/followups/artifacts/findings | 64/256 bytes / 8 entries each, 256 bytes |
| Blocker/handoff/reason / resolver / UTC dates | 512 / 128 / 40 bytes |
| Text / requested context page | 8 KiB / 1–20 rows |

Only generated log events are evicted. Human reports/reasons/cancellations and review history are never silently pruned. File capacity bounds retained history; ordinary writes retain closing reserve, accepted review/cancel may consume it. This is not an unlimited full log; Git retains committed history.

Versions bind canonical root, relative target and exact bytes. Project context separates editable manifest Version, creation Allocation version and pagination Snapshot version. Task and Module Atomic writes use whole Module versions; siblings/review can stale them, another Module cannot. Epic/standalone Atomic Version additionally binds direct member/participant bytes and transitive integration participant observations within the aggregate byte cap; it is not only a hash of the owner file. Creation scope hashes manifest/allocator and filenames, not report contents. After reservation recheck intended allocator plus unchanged manifest/inventory.

Healthy aggregate records remain readable with named omissions. Data/detail coverage are separate; PARTIAL counts are lower bounds and unknown work is not zero. project_status reports scoped progress/leads/summaries/blockers/review attention/last report; never live agent presence. The declared small fixture is three Modules/twelve Tasks/three leads/two blockers. Larger detail may need Module narrowing.

Search is Unicode-lowercase all-term semantic-field substring matching, ranked by matching-field count then numeric ref. It is not Markdown/code/semantic search. Project hits open with omitted ref; Epic/Module/Task/Atomic hits use their returned ref. Receipts explicitly label the owning Module phase, distinct from Task completion. Empty detail rows use view-specific wording. Continuation hashes bind tool/ref/query/view/review selection and the exact data snapshot; changing a selection refuses instead of skipping new rows. Editable file Version is distinct. Choose the largest fitting prefix, advance by displayed rows; never post-truncate or skip.

## Verification and delivery

Focused tests cover operation shapes against JSON Schema and serde, native file publication/permissions/locks, stale/independent writes, partial init and allocation gaps/loss, backups, cancellation, reviewer attribution/approval invalidation, bounded status/search/pages/scans, recent-log eviction, closing reserve, renderer fallback and injected post-publication sync failure.

The real SDK/stdio test plans work, completes a Task, independently accepts a Module, reads status/search, restarts the binary and checks persisted closure. Existing protocol tests preserve discovery equality, modern private cache hints, legacy omission, invalid/unknown calls and EOF.

Full gate: cargo xtask check. Supply-chain gate: cargo deny check after explicit fetch. Native acceptance additionally uses the independent MCP Inspector CLI, including fresh-process work persistence. This is not power-loss certification or qualification of other platforms/filesystems/hosts.

Standard installation manages immutable releases under declared product home/bin only. Portable roots are external user-selected data. Binary rollback does not undo their data/schema/effects. Migration/repair or service restart is never implicit.

## Epic membership and Atomic integration

Epic membership is authoritative only in its own `modules` and `atomics` lists. Parent context is computed from those references; Modules/standalone Atomics have no competing parent field. Project-owned work is standalone until explicitly attached. Every child belongs to at most one Epic. Module Atomics remain embedded in their Module; Tasks remain leaves. Closed types, canonical reference validation and complete membership inventory reject duplicate ownership, dangling/invalid references and counters inconsistent with retained IDs.

Creation and attachment are separate explicit operations. `create_epic` and `create_atomic` use Project Allocation version; `edit_epic` replaces supplied member lists under Epic Version. Omitted lists preserve membership; [] detaches all members of that kind. Move by explicit detach, inspect and attach, using fresh versions. If attachment fails, the already-created child remains visible and standalone. No cross-file transaction is implied. For managed-1 Epics (core absent) the first reported Epic begin freezes the Module roster and reopen cannot change it, while Atomic membership remains editable; core-active Epics freeze explicitly (see Epic core workflow). Begin is a guarded lifecycle fact, not an arbitrary status setter or agent launcher. Legacy absence preserves prior behavior until explicit begin. Canceled parents prevent child writes; membership edits require an open Epic and validate child conditions.

Epic requires title, outcome and meaningful acceptance criteria. Its context provides bounded own intent/criteria and member facts without copying full Module reports. Required checks belong to the owner declaring them; Project purpose/Epic intent are background, not implicitly inherited check labels. Module acceptance evaluates its own and embedded children's explicit requirements. Epic acceptance evaluates its own requirements plus member completion. A check named only as contextual background never becomes a hidden gate.

`review_work` accepts an Epic, Module or Atomic reference, including an embedded Module Atomic. Task review refuses. `review_module` remains a compatible Module-only entrypoint. Epic independent acceptance requires all noncanceled member Modules currently accepted, member standalone Atomics independently accepted with fresh evidence, meaningful own result, all required checks passed and no blocker/in-scope gaps. Acceptance basis includes member semantic facts and acceptance/review generations; relevant child change, reopen, or membership edit makes old Epic acceptance historical. Review does not duplicate child results. Known lead self-review remains refused; declared identities are not authentication.

`create_atomic` stores Project/Epic Atomic work in `atomics/A-001.yaml`. `add_atomic` embeds Module Atomic work in `modules/M-001.yaml` and returns `M-001/A-001`. Both require meaningful title/outcome and may declare an executor/handle and required checks. Result replacement, artifacts/gaps/check statuses, reasoned cancellation/reopening and bounded evidence follow the existing model. Every current workflow Atomic has independent review, including Module-owned Atomics. `state=done` or complete records local completion from a meaningful result; it does not itself establish final acceptance. Explicit checks/gaps and current review participate in parent acceptance. Legacy behavior remains grandfathered until opt-in.

A standalone integration Atomic declares `participants` as bounded Module references. Required check labels and report details describe scenarios; artifacts remain optional reported evidence. A result captures current participant semantic bases and current review generations. Participant changes make done evidence stale even though historical reported state remains retained. Stale integration evidence cannot satisfy Epic completion; refresh the actual verification report explicitly. No source-code inspection, GitHub API, runtime polling or invented verification commit is required.

New root counters are independent (`next_epic`, `next_atomic`); Module-local `next_atomic` is independent of `next_task`. Old schema-1 records without new fields read without rewrites, preserving dates/reports/history and legacy semantic review basis. Modern allocator revision 2 requires every counter; missing/null counters refuse even with an empty inventory after a failed publication. Legacy revision-1 allocators may start unused new kinds at one only after proving their corresponding inventory empty. Lost allocator state is never guessed. New writes may emit expanded fields on existing records. Older binaries do not understand those expanded records; binary rollback leaves stored data intact and is not a data downgrade.

Context, lexical search and one-call status include all entity kinds once, standalone/Epic-owned Modules, assigned leads/executors, Task/Atomic progress, meaningful current reports, blockers/review attention and current/stale acceptance. Relevant parents and participating Module evidence bind continuation snapshots. Data coverage and omitted detail remain explicit: unreadable work is unknown and partial totals are lower bounds. Module narrowing keeps bounded status useful; no agent-side scans or live-runtime polling are needed.

## Epic core workflow (core-active records)

[epic-core-plan.md](epic-core-plan.md) is the governing target. This section describes the implemented core rules for records whose workflow has `core` set. Sections elsewhere that require delivery for Module acceptance, a first-Epic-begin roster freeze or an integration covering the entire roster describe the older managed-1 behavior (core absent), which is preserved unchanged. Local validation passed `cargo xtask check`: formatter, frozen all-target Clippy, 118 Rust tests, rustdoc and frozen build; explicit contract check and both canonical skill validators passed. Independent source review accepted the corrected lifecycle. The separate observed-runtime receipt test confirms binding persistence/interoperability, not actor authentication or live correctness. Release, broader host and power-loss qualification are unchanged.

**Activation.** An optional `Workflow.core` marks a record core-active. New records have it; absent core preserves managed-1 and unmanaged records and their legacy semantic digests. `record_work adopt_core` explicitly activates an existing owner and makes earlier approval historical; it invents no IDs, planning, agreement, candidates or verification. Record schema, allocator and reference formats are unchanged.

**Persistent identity.** `bind_agent {role: lead|reviewer|integrator, harness, agent_id, communication_ref, resume_ref?, launch_ref}` stores observed runtime receipt data (strings ≤256 bytes, harness ≤64). Lead and reviewer bind on a Module; integrator and reviewer bind on an integration Atomic; at most three role slots per owner. Tasks have no binding. The Epic has no binding slots: its begin, result, freeze, `verify_criterion` and review are orchestrator-attributed. A generic Atomic keeps its executor/independent-review policy, while a Module requires bound lead and reviewer and an integration Atomic a bound integrator and reviewer. Loss, bind and immersion are role bookkeeping, not a new implementation or review. `recover_agent` records `stage=lost` (needs lost and unrecoverable true, reason ≤512, observed inability to continue ≤1024) before a different binding is accepted, then `stage=immersed`, attributed to the new `agent_id`, with understanding ≤1024 and sources/unfinished/gaps (8×256); remaining gaps block continuation. History of earlier terms is retained and same-ID retries do not duplicate it. Scoped actions compare the actor with the current recorded `agent_id`, not a display label. Bindings are reports, not authentication; `plan_work edit_module` cannot change a bound lead.

**Lead planning and contracts.** `record_work planning {responsibility, scope, exclusions, read_refs, uncertainties}` is reported by the bound lead before coding (≤1024 bytes; lists 8×256) and stores the current plan-intent basis. Core Modules are created without initial Tasks; `add_task`/`edit_task` follow that actor. Contract entries become `{id, revision, peer, description, reference?, ready}`: one provider per id, reciprocal entries must match id/revision/artifact and party Modules, `ready` is legacy metadata. Affecting edits retain prior facts, require a higher revision and report mismatches. Provider definitions are canonical per ID across consumer and provider changes; removing an entry does not reset its retained revision or permit a different definition at the same revision. `agree_contract {contract_id, revision, summary}` by each participant's bound lead stores a canonical snapshot, date and actor; all affected confirmations precede implementation, and an affecting peer or canonical revision change stales review/readiness even with unchanged code. Provides/consumes never create waits.

**Scopes, freeze and begin.** `edit_epic criterion_scopes=[{index,text,modules}]` (≤8; ZERO-BASED index) must match current criteria and the roster. `plan_work freeze_epic {epic}` validates bound-lead discovery, current planning, relevant agreement, criterion scopes, distinct prepared checkouts and mandatory-wait cycles, and freezes the roster before coding. It does not require acyclic completion waits to be satisfied, so a provider can begin and its consumer wait for acceptance. Epic begin permits planning without freezing. Module begin enforces its own dependencies, a frozen parent, a ready bound lead, current planning/agreement and a separate declared worktree; simultaneously active core Modules sharing a writable checkout refuse, and the integration checkout is distinct. Wait cycles return explicit architecture attention or refusal; there is no new report subsystem.

**Candidate, boundary evidence and review.** `result` accepts `candidate` (≤256 bytes, a definite commit/artifact; a commit import supplies the raw full Git SHA, with no `commit:` prefix) and `changed_scope` (8×256). `boundary_evidence {contract_id, revision, candidate, conditions, correct, mutation, failed, restored, artifacts}` stores observations `{status,detail,artifact?}` (detail required ≤512; conditions/mutation ≤1024), bound to the exact candidate, contract revision and actual lead. Any whole core Module or integration review requires a definite submitted candidate; positive Module review also needs applicable boundary evidence with statuses passed/failed/passed. Review provenance distinguishes new core findings from legacy history, and embedded Atomic follow-up references belong to that Atomic rather than its owning Module. A Module declaring `contracts.not_required=true` still requires the same mutation controls under the reserved `contract_id=local`, `revision=1`; this is a required local quality scope that cannot be a peer contract id, and its applicability pins the current Module intent and candidate despite revision 1. Recording does not execute a check, and isolation or non-mutation of assertions is not certified by the MCP. `review_work` adds `changed_scope` and `resolved_findings=[{review_index, finding_index, summary}]` (review and finding indexes ZERO-BASED) addressing retained immutable review history; the current bound reviewer is required for Module and integration review, and a replacement reviewer needs loss, bind and immersion. A positive core review makes the Module ready for integration with no merge or delivery prerequisite; delivery remains separate bookkeeping.

**Incremental integration and business verification.** `get_context view=integration` derives ready connected components of at least two Modules from reciprocal agreed contracts and current positive reviews, with candidate/contract coverage keys, covered/uncovered status and named partial facts; unrelated or unfinished Modules neither join nor block a component. Integration Atomics keep participants/environment/scenarios (≤32 participants); begin needs at least two connected current-ready Modules, a bound integrator and a distinct checkout. First integration begin captures the exact participant candidate/contract key. Equivalent active or pending jobs and accepted current coverage block duplicate work for the same environment/scenarios; pending claims remain distinct from verified coverage. The result requires a definite assembly candidate. Changed or reopened inputs stale applicability while unrelated metadata does not. `verify_criterion {index (ZERO-BASED), text, modules, candidate, environment, scenarios, summary, checks, artifacts, integration_ref?}` on the Epic must match a declared criterion scope and pass all required checks; an `integration_ref` must be a current accepted integration whose single composition covers the whole Module set with matching candidate/environment/scenarios. A union of A+B and B+C is not proof for A+B+C; without a reference the record is an explicit Epic-level E2E report. Final Epic acceptance needs required Modules current-reviewed, relevant boundaries covered and every declared criterion currently verified, with no all-roster-per-job barrier.

Shared limits remain 512 KiB records, 16 MiB scans and eight list items per report field. Core retains at most 16 prior contract-definition snapshots; reaching a bound refuses publication rather than silently pruning facts. Review and agent histories remain bounded by record capacity. No runtime launch, daemon, new dependency or remote API exists in the MCP.

## Unified reported workflow

The following four sections describe managed-1 behavior (core absent) and the shared lifecycle engine. A `workflow` record separates declared start, execution, contracts/dependencies and delivery from the existing evidence/review engine. Newly created owners opt into workflow revision 1. Absent or unmanaged workflow preserves existing schema-1 records and exact legacy semantic approval digests; reads do not migrate. Planning fields may be declared for a legacy owner without activating new gates; explicit begin opts it into new conditions. This is one extended lifecycle, not a second unrelated status engine.

Module intent includes criteria, lead and execution `{repository,worktree,branch,target_branch}`. Repository/worktree paths and branch names are declared planning facts until explicit Git import verifies them. Startup checks those facts, parent activity, necessary contracts and actual dependencies. Begin records a generated UTC date and activity, never starts a process or grants runtime permission.

`contracts={not_required,provides,consumes}` owns descriptive obligations. Each entry has a peer Module reference, description, optional canonical reference and declared ready flag. Explicit not_required requires empty entries. A reciprocal runtime boundary is allowed; it is not automatically an execution wait. `dependencies=[{ref,condition,reason}]` declares waits for accepted Epic/Module or delivered Module work. Unknown/dangling/duplicate/self/impossible cycles refuse decisions. Parent-completion waits must not hide a circular start/acceptance gate. Healthy selected data remains readable with named partial coverage when related work is unavailable.

Under managed-1 (core absent), Epic start captures a permanent Module roster. Modules may be prepared and attached before start. Afterward they cannot be appended, detached or moved out through another Epic; cancel/reopen retains the roster. New work can stay standalone, with an explicit named Epic wait when needed. Atomics remain attachable without changing the Module roster; relevant scope changes make previous acceptance historical.

The lead decides Task local completion after a test or manual verification. No individual Task review exists; neither commit import nor a reported pass automatically closes work. An explicit complete operation can use the current imported report, preventing a duplicate written summary. Meaningful report and declared attribution are retained; parent required checks are evaluated honestly.

Under managed-1 (core absent), whole-Module independent approval precedes final reported delivery/merge. Delivery names the declared target branch, summary and optional artifact, with captured implementation basis/date/actor. It is an agent report, not proof from a GitHub query. Semantic changes and reopen clear its applicability. Posting matching delivery after review is bookkeeping and does not self-invalidate that review. Local merge is sufficient; the MCP does not execute it.

Every current workflow Atomic is reviewed independently. Embedded Atomic review/check provenance/history publishes in its owning Module file; siblings share its write Version. Local done and independent accepted completion remain separate. Known lead/executor self-review refuses; declared identities are not authenticated credentials. Task references cannot use review_work.

Under managed-1 (core absent), integration Atomic begin waits for participating Modules' current accepted implementation and reported delivery. Its environment and scenarios define the actual joint check. Final modern Epic acceptance requires current independently accepted integration whose participant set exactly matches the active noncanceled frozen Module roster, in addition to all other owned required Atomics and own criteria/checks. A generic Atomic or partial participant list cannot stand in for full composition. No-Module Epics use their own criteria and Atomic evidence without inventing a Module integration.

## Local Git report boundary

`src/git_reports.rs` is a bounded read-only argv adapter. It accepts at most eight explicit hex selectors, resolves full 40/64-hex commits, observes canonical common-Git-directory identity plus actual worktree, author/UTC date, subject and original message. Source repository/worktree/branch coherence is verified for explicit imports. Credentials, arbitrary revision expressions/shell, code inspection, push/fetch/checkout/merge and lazy promisor fetching are outside this operation. Provider stderr and malformed bodies are not diagnostic text.

A shared five-second deadline and cap-before-UTF8/parse protect metadata reads. Each retained message is at most 8 KiB; parsed Result summary is bounded at 1024 bytes, checks/gaps/followups use existing limits. Required `Result:` plus optional `Checks:`, `Gaps:` and `Followups:` are a simple standardized report, not a new schema language. Check lines use `status | label | detail`; the first two separators delimit status/label while later separators belong to detail. Statuses are explicit reports, never a claim the importer ran tests.

Core owns expected-version revalidation, root locking, whole-report replacement and source-history publication. Multiple Result summaries combine in caller order, later check labels supersede earlier current labels, and repository identity/full SHA deduplicate repeated observations across worktrees. Observed source messages remain retained after repository history or a worktree is removed. Malformed/missing/oversized sources refuse before business publication; capacity refuses rather than silently pruning human source history. Import does not close a Task unless completion is explicitly selected by its lead.

Context/search/status include criteria, peer direction, dependency reasons/readiness, execution, start/freeze/delivery/review state and imported report substance. A Module reference supplies one assignment pack; runtime permissions still come from the launcher. One-call overview retains separate Task/Atomic/canceled/unknown/omitted counts and declared lead locations. It does not poll runtimes or duplicate every report.
