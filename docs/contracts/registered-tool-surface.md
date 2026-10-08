# Registered tool surface, dispatch seam and read presentation — planning contract

Status: semantic revision 5 of `registered-tool-surface` (provider M-003, consumer M-006) for root signing; the tracker entry for the signed bytes of this file is revision 5. It keeps every rule of revision 4 and adds section 15, the retained compaction reads (full retained proposal history and exact earlier candidate bodies through the existing `get_context`), with the matching rows in sections 7 and 12 and refreshed pins in section 1. The mutation seam is unchanged, so `producer-host` stays at revision 3. M-005 has accepted and root has signed the provider seam of section 15.5 as `compaction-operations` revision 7; M-006 as consumer and M-003 as provider confirm this revision before any code is written. Nothing in section 15 is implemented. [architecture.md](../architecture.md) stays the truth for implemented behavior. The immutable tracker reference revision is distinct from this heading: each tracker entry pins the actual committed bytes and gets a higher revision when this file is signed. M-003 is the only owner of `src/main.rs` module declarations, `src/tools/mod.rs`, `src/tools/input.rs`, `src/tools/read.rs`, the dispatcher and acknowledgement parts of `src/tools/work.rs`, the response templates and the exported `schemas/tools.json`. Producers never edit those files. Revision numbers belong to this provider.

## 1 Pinned consumptions

Each is a provider-owned, signed and committed artifact; the consumer pins the current published revision, commit, path and content digest, read from the live tracker context and never guessed. An affecting change by a provider raises its revision and needs fresh agreement from every party.

| Boundary | Provider | Current pinned revision | Reference (commit path sha256) |
|---|---|---|---|
| `kr-model-read` | M-001 | 7 | `commit 241b19f647ed701eed124cab4cd1c9560b081d4d path docs/contracts/knowledge-records.md sha256 156f4c9d5a0beb307e2d9e1120ff556ce1a3c2d328ce03f915baea2a601ec0e8` |
| `md-documents` | M-002 | 3 | `commit 64d9bd863abe5302527cd9603b36032d4578dfdc path docs/contracts/markdown-references.md sha256 3ae837cb839150f6d722ec2a54bf60de09fe4fded531f41a9e142c54891935bb` |
| `persist-git-m003` | M-004 | 4 | `commit eaa02c833f590c8b34410069918fbc97bd369786 path docs/contracts/publication-git.md sha256 0816759e9c2df1500f45829b0ecd509d045f387049e7455fc7f9df5257727b89` |
| `compaction-operations` | M-005 | 7 | `commit 686dab8bfc98b043587142884378ff3b3250dff3 path docs/contracts/compaction.md sha256 84a41e40c0c3ef1a70584a3e11a81899ffe16543ffcc2f1935ff12bc7d73d3a8` |

The pins above are the published, root signed artifacts. `compaction-operations` revision 7 adds the retained revision and staged candidate read helper that section 15 consumes; revision 6 is superseded for this consumer. No source interface is guessed from a staged or future text.

Provided here: `registered-tool-surface` to M-006 and `producer-host` to M-001, M-002, M-004 and M-005, all pinning this file by commit, path and sha256 once root signs it. Placeholder or staged references are never published as agreed.

## 2 Registered surface

Existing tools are unchanged: `get_status`, `register_project`, `get_project_list`, `get_context`, `project_status`, `search`, `plan_work`, `record_work`, `review_work`, `review_module`. Exactly four purpose tools are added, one per producer, no others and no alias tool:

| Tool | Producer | Payload enum (producer owned) | Purpose |
|---|---|---|---|
| `knowledge_work` | M-001 | `KnowledgeOp` | create, edit, supersede Decisions, Runbooks, Research; use a Runbook; procedural Checklists |
| `document_work` | M-002 | `DocumentOp` | save or edit Markdown (whole body or one section), adopt, remove, relocate |
| `compaction_work` | M-005 | `Compaction` | propose, revise, review, recover reviewer, apply, withdraw |
| `git_recovery` | M-004 | `RecoveryOp` | exceptional: reconcile, retry, adopt, release or preserve pending Git work |

Reads add no tool. `get_context`, `project_status` and `search` learn the new record kinds (sections 7 to 10). No tool takes a raw root, a filesystem path outside the managed grammar, or a legacy tool name. Mutation arguments are flat: `project`, `version`, optional `actor` plus the closed `op` variant, exactly like `plan_work`. `register_project` keeps its signature and its single bootstrap commit and is excluded from normal settlement.

## 3 Common arguments and validation

The single `input::Common {project, version, actor, reference}` is reused. The four new tools decode with `input::mutation::<Op>(args, false)` and export `input::mutation_schema::<Op>(false)`; `ref` stays a variant field owned by the producer. A producer defines no second Common, decoder or schema function.

`version` means, per tool: `knowledge_work` create ops the one knowledge allocation observation, every other op the record version; `document_work` the `Version` printed by `get_context` for that path or DOC id (an absent path has a version too); `compaction_work propose` the knowledge allocation observation, other ops the CP record version; `git_recovery` the exact pending journal observation version, validated by the recovery producer's own `execute_locked` under the lock the dispatcher holds. The dispatcher never calls a private recovery engine entry point. A stale version refuses before any effect.

Field specific errors (observed defect AT-003), no new dependency. M-003 provides in `input.rs`:

```rust
/// Name the offending field in a bounded validation failure; the supplied value is never echoed.
pub fn field<T>(name: &str, checked: Result<T, String>) -> store::Result<T>; // invalid_arguments: "<name>: <rule>"
```

Producers wrap every list, text, count and enum check with it (`field("read_refs", model::strings(&refs, 256, false))`); a limit message states the field and its limit. `input::mutation` maps a serde unknown-field or missing-field failure to a sanitized message naming only the field (`unknown field "x"`, `missing field "x"`), never a value. Any other shape failure becomes one bounded generic message that names the operation and the tool and states that field type details are unavailable; user values are never echoed.

## 4 Seam: producer to dispatcher

One uniform shape per producer, all under `src/tools/`:

```rust
pub const DESCRIPTION: &str; // catalog purpose text; M-003 only registers it
#[derive(serde::Deserialize, schemars::JsonSchema)]
#[serde(tag = "op", rename_all = "snake_case", deny_unknown_fields)]
pub enum KnowledgeOp /* or DocumentOp, Compaction, RecoveryOp */ { /* closed variants */ }
/// Handler. The caller already holds the write lock for `store`; never lock here.
pub fn execute_locked(store: &Store, guard: &store::LockGuard, common: &input::Common,
                      op: Op, effects: &mut Vec<String>) -> store::Result<work::Ack>;
```

Files: `src/tools/knowledge_ops.rs`, `document_ops.rs`, `compaction_ops.rs`, `recovery_ops.rs` (producer owned content, `mod` lines owned by M-003). Domain modules sit at the top level and never import `tools`. `src/main.rs` declares `knowledge`, `markdown`, `documents`, `references`, `persist`, `compaction`. A producer branch may add only its own placeholder `mod` line; M-003's final lines win at integration.

Result type. The existing plain acknowledgement is compatible, so no new `Outcome` type is introduced. `work::Ack` gains two additive fields and its builder is shared:

```rust
pub struct Ack { pub target: String, pub version: String, pub phase: String,
                 pub phase_label: &'static str, pub changed: bool,
                 pub notes: Vec<String>,   // new: at most 8 plain lines of 200 bytes (applied actions, blocked kind, warnings)
                 pub refs: Vec<String> }   // new: at most 16 canonical refs of affected records, each at most 256 bytes
pub(super) fn ack(target: impl Into<String>, version: String, phase: impl Into<String>, changed: bool) -> Ack;
```

`ack` derives `phase_label` from the target: `E-` Epic phase, `A-` Atomic phase, `M-` Module phase (unchanged), `D-` Decision state, `RB-` Runbook state, `RS-` Research state, `CL-` Checklist state, `DOC-` or a managed path Document state, `CP-` Compaction state. The `core_ack` template prints `{{ phase_label }}: {{ phase }}` with the label carrying its own noun, so existing output is byte identical. `target` is a canonical ref usable with `get_context`; `version` is the new record or observation version usable for the next write; `phase` is a short lowercase state. Notes beyond eight or lines beyond 200 bytes are cut by the dispatcher with an explicit `N notes omitted; get_context ref=<target>` line, never silently. Producers never add a Git line, never render text and never settle.

Canonical target rule and its one closed exception. The application `Ack` is unchanged. Every work, knowledge, document and compaction target is a real canonical ref (an `E-`, `M-`, `A-`, `D-`, `RB-`, `RS-`, `CL-`, `DOC-` or `CP-` ref, or a managed path) and its `Next` route is `get_context ref=<target>`. The single exception is `git_recovery`: the presenter selects the `recovery_ack` layout from the known tool kind of the call, never from the target string and never from a template name supplied by a producer or a user. Its `target` is the display label `Git recovery`, its phase label is specific to recovery, and its `Next` route is `get_context` with the project and the `ref` omitted, which shows the actual pending version and facts. The label is never rendered as `ref=Git recovery` or `ref=Project`. For any other tool a target that is not a canonical ref cannot invent a route: the reply names the target as unrouted and points only at `get_context` with the project and `ref` omitted. No new application outcome type, no second common argument type and no backend locking are introduced.

Read models stay public in the domain modules and M-003 renders them: `knowledge::{load, scan, Any, Kind, allocation_version}`, `documents::{observe, read, inventory, corpus}`, `markdown::{outline, resolve, page, heading_at}`, `references::{incoming, outgoing}`, `persist::pending`, and the actual compaction read surface `compaction::{read_cp(store, id), summaries, inventory, scan}`. Every struct field M-003 renders is `pub`.

## 5 Dispatch: one mutation scope for every mutating tool

Owner decision, adopted: the MCP commits automatically after every successful actual mutation of a document, task, project or knowledge record. Reads and true no-ops never commit. A failed or partial business call keeps what it saved and is reported pending or unknown; it is not a commit trigger. A successful change whose Git step is rejected still returns the actual business result with a truthful Deferred or Unknown receipt. Registration keeps its single bootstrap commit and is not settled again. The policy implementation, its conditions and `production_policy()` belong to M-004; M-003 passes `persist::production_policy()` and never a flag, variable or injected policy.

Work writes are in scope. The three existing write handlers lock inside themselves (`plan`, `record`, `review` in work.rs, each as `config.resolve` then `store.lock(true, effects)`). They are minimally hoisted into one shared scope, with their business code, errors and public behavior unchanged and no duplicated handler:

```rust
/// work.rs, M-003 owned. The only place a mutating call resolves, locks, settles and releases.
fn mutation_scope(config: &Config, project: &str, class: persist::EventClass, prepare_first: bool,
                  effects: &mut Vec<String>,
                  body: impl FnOnce(&Store, &store::LockGuard, &mut Vec<String>) -> Result<Ack>)
                  -> (Result<Ack>, Option<persist::GitReceipt>);
```

`plan`, `record` and `review` become `*_locked(store, guard, ..)` bodies minus their first two lines; `prepare_first` keeps today's `store.prepare` before the lock for `init_project` only. The scope: (1) resolve the alias; (2) optional prepare; (3) take the write lock once; (4) run the body or `execute_locked`; (5) decide settlement; (6) call `persist::settled(&store, &guard, event, persist::production_policy(), result)` while the guard is held; (7) release; the caller then renders. Settlement is decided as follows:

A real no-op is the absence of any actual file publication, not merely `Ack.changed = false`. The scope decides from the actual typed events in `store.publications()` (a `DirectoryExisting` event is not an effect) and from the real success or error, never from the flag alone, and it never drops a receipt or commits something it should not:

- `Ok` with `changed = true`: settle with `EventOutcome::Success`.
- `Ok` with `changed = false` and no actual file publication: a real no-op; no settlement, no Git line, never a commit.
- `Ok` with `changed = false` but at least one actual file publication: not a no-op. Settle with `EventOutcome::Partial` so the effects are tracked and reported, never committed as a success and never dropped; the reply keeps the receipt and states that effects were published while the result was reported unchanged.
- `Err` with actual file publications (partial or failed after publishing): settle with `EventOutcome::Partial`; the business error is returned unchanged with the ledger and the Git lines.
- `Err` with no publication: no settlement.

Event classes: `plan_work`, `record_work`, `review_work` and `review_module` use Work; `knowledge_work` Knowledge; `document_work` Document; `compaction_work` the M-005 provided `Compaction::event_class(&self) -> persist::EventClass` (M-004 revision 2 has five Compaction variants); `refs` come from `Ack.refs`, empty on failure and advisory only, because identity is the journal. Event uses the provider-owned EventOutcome enum, not a second boolean or outcome type; settlement downgrades sync-uncertain or unknown-tracking success to Partial. `operation` is None at this level (M-005 passes its own per-action operation ids inside its handler).

Recovery. `git_recovery` runs in the same scope with the same single lock. Its producer `execute_locked` validates the exact pending journal observation version under the held lock, calls its private recovery engine and returns the explicit recovery receipt lines in `Ack.notes`. The dispatcher invokes only the uniform public producer handler, never a second engine entry point, so the version check lives in that handler and not in a dispatcher call to a private function. For this tool and only here the scope skips ordinary settlement, on `Ok` because the recovery receipt is its settlement, and on `Err` because re-entering the engine that just failed would hide the cause; the error carries M-004's pending references. The reply uses the `recovery_ack` layout selected from the known tool kind (see the canonical target rule). No handler locks itself.

Receipt projection. The receipt a call returns (outcome, commit and paths) describes that call only. Earlier pending intents and any commits made for older work are separate facts: the reply renders them as their own lines, labeled as earlier pending, and never merges them into the current call's outcome or paths. M-003 projects only the fields the provider actually publishes and invents no field names until the provider's final type is published. Under the whole intent rule the current call's atomic replacement plus removal commits as one intent; an older held replacement must be committed before a delete commit, and the dispatcher never requires all unrelated pending work to commit inside a fresh handler, so there is no dispatch deadlock. These are the providers' business guarantees; the dispatcher calls `settled` once, renders the result and adds no loop or retry.

Reply. After the existing text the reply shows the effect ledger (`Published ...` strings from `effects`, which stay human text) and at most the six lines of `GitReceipt::lines()`; the added lines are additive and the pre-existing lines of every existing tool are unchanged. A rendering failure keeps the saved result: the degraded text still names target, version, effects and Git line. Errors render through `core_error`, extended with the Git lines; an unknown outcome keeps the "inspect before retrying, do not replay" recovery text. Under the owner approved policy the SDK exercises the shipped production policy with no injection flag; an automatic commit is claimed delivered only from such a run, never from a mock.

Sequencing without coding waits. Immediately after the Epic is frozen M-003 commits the seam handoff: `input::field`, the extended `Ack` and `ack`, `mutation_scope` with the hoisted handlers, and the `document_page` template skeleton. Producers code against these signatures and substitutes; final evidence uses the same physical modules, and mocks serve module level tests only.

## 6 Registry rules

`definitions()` appends four `definition(name, producer::DESCRIPTION, input::mutation_schema::<Op>(false), false)` entries; `incomplete()` lists each tool as unimplemented until its producer's tests and M-003 routing pass; `work::call`'s name list and its recovery text include them; `schemas/tools.json` changes only through `cargo xtask contract update`, reviewed as a diff, and `contract check` must pass. Descriptions are bounded ordinary prose for the live catalog; exact signatures live here, not in descriptions. Tracker text avoids YAML-sensitive punctuation.

## 7 Reference grammar, read routes and exact read schemas

`get_context` classifies `ref` once, before any file read:

| Ref form | Route |
|---|---|
| E-, M-, A- and their children | unchanged |
| D-001, RB-001, RS-001 | `knowledge::load`; summary, `view=history` (retained revisions, uses), `view=references` |
| CL-001, CL-001/I-001 | checklist summary with item facts and events; an item ref shows that item; `view=references` |
| DOC-001, `README.md`, `docs/x.md` | `documents::observe` through `Ref::parse`; summary with state and outline, `view=content` exact pages, `view=references` |
| CP-001 | `compaction::read_cp`; summary, `view=tasks` lists actions and sections of the current revision, `view=review`, `view=references`; revision 5 adds `view=history` (every retained revision, review and reviewer recovery fact) and `view=content` (one exact staged candidate), see section 15 |

Exact `ContextArgs` schema after this contract. Existing fields keep their meaning: `project`, `ref`, `view`, `start`, `limit`, `version`, `review_index`.

| Field | Type | Rule |
|---|---|---|
| `view` | enum | existing values plus `content` (documents and, from revision 5, compaction proposals), `history` (D, RB, RS and, from revision 5, CP), `references` (D, RB, RS, CL, DOC, CP); any other pairing refuses naming `view` |
| `revision` | integer, 1 or more | revision 5: compaction `view=history` (optional, selects one retained revision) and `view=content` (required); any other use refuses naming `revision` |
| `action` | string, 1 to 8 bytes | revision 5: compaction `view=content` only and required there; names one action id, provider grammar `A-01` to `A-32`, possibly sparse; any other use refuses naming `action` |
| `ordinal` | integer, 0 or more | document `view=content` only; exactly one of `ordinal`, `heading`, `preamble` may select a part; none selects the whole document |
| `heading` | string, 1 to 256 bytes | same; the heading text, case sensitive |
| `occurrence` | integer, 1 or more | only with `heading`; required when the text is ambiguous (`ambiguous_section` names the count) |
| `level` | integer 1 to 6 | only with `heading` |
| `preamble` | boolean | document `view=content` only; true selects the preamble |
| `start` | integer, 0 or more | unchanged row offset for every view except document `view=content`, where it is a raw byte offset on a character boundary (revision 5: also compaction `view=content`, a raw byte offset into the staged candidate) (otherwise `invalid_arguments` naming `start`) |
| `limit` | optional integer 1 to 20 | unchanged for rows (default 20); for document `view=content` and, from revision 5, compaction `view=content` any supplied `limit` refuses naming `limit`, because the page size is fixed by the 8192-byte budget. The field becomes optional in the closed struct so absence and supply are distinguishable; the exported default stays 20 |

A selector field used with another ref kind or view refuses naming that field. The existing paging parameters (`start`, `limit`, `version`) keep their meaning for every existing and new row based view; `version` carries the snapshot and is required when `start > 0`. An absent managed path is a successful page showing state Absent and its creation Version. Reads never create files, locks, directories or commits and never repair.

Project context (ref omitted) keeps the work `allocation_version` and adds the knowledge allocation observation, per kind record counts with current and superseded totals, document counts by state (managed, unmanaged, drifted, missing body, unsupported), non-terminal compaction count and pending Git facts. Every figure names its coverage; an incomplete inventory prints PARTIAL with the named gap, never zero. Typed record pages show currentness, `superseded_by`, outgoing refs and, from `references::incoming`, incoming attention with its coverage. Every new view stays within 8192 bytes by rows: M-003 splits any field longer than 1500 bytes into rows at character boundaries so a single row cannot exceed the budget.

## 8 Document pages and wire

M-003 owns template `document_page`. Layout is a header, one framing line, the payload, a footer:

```
<heading>
Data coverage: ...; State: <state>; Version: <observation.version>
Snapshot version: <read snapshot>
Content: md-text-v1 wire=<raw|escaped> bytes=<start>-<end> of <range end> encoded_len=<N>
<exactly N bytes of encoded text>
Next: start=<end>; version=<snapshot>; remaining=<raw bytes>. Keep the same tool and selection.
```

The reader delimits the payload by `encoded_len` (the framing line also carries the wire and the raw byte offsets), so a parser reconstructs the exact original bytes from the reply alone, whatever the content. Budget: M-003 renders the page once with empty text and maximum width numbers, measures the overhead, and passes `budget = 8192 - overhead` to `documents::read` (which uses `markdown::page`). Template autoescape is off and the payload is a plain string variable, so no HTML, JSON or template escaping applies; a test compares the rendered reply bytes with `markdown::encode` output and requires the whole reply at most 8192 bytes including header and footer. A budget under `MIN_PAGE_BUDGET` is `presentation_capacity`, never truncation.

An empty whole or preamble selection finishes truthfully: the page carries `encoded_len=0`, `bytes=<a>-<a> of <a>` and no `Next` line, and the reply says `End of selection; no continuation`. A final page also omits `Next`. A continuation whose `start` equals the range end returns that same empty final page.

Snapshot: the value returned by `documents::read` is shown unchanged. It binds the observation version, resolved selector, encoding and dialect, never the byte offset, limit or page, so page two of an unchanged document continues and any change gives `stale` with the current snapshot. `start > 0` without a version is `stale`, as for every other page. Other kinds use `scope_version(selection, record version, scan basis)`, which already excludes `start` and `limit`.

## 9 Search

`search` keeps one query language: one to eight words, all must match, Unicode lowercase substring. Existing `start`, `limit`, `version` and `module` keep their meaning. Exact additions:

| Field | Type | Rule |
|---|---|---|
| `kinds` | optional list of `work`, `knowledge`, `document`, 1 to 3 distinct | default all three; with `module` supplied and `kinds` absent the default is `work` only, so existing scoped calls behave as before |
| `state` | optional `current`, `superseded`, `any` | default `any`; state is shown on every row |
| `module` | unchanged | applies to the work source only; supplying it with `kinds` that include knowledge or document refuses naming `module` |

Sources: work fields as today, `Any::search_text()` for knowledge with its stable labels, `documents::corpus()` text and headings for Markdown. M-003 owns matching, ranking (matching fields, then currentness, then numeric reference) and excerpts; a document excerpt is cut at character boundaries and located with `heading_at`. Rows are bounded to at most 600 bytes each. Reuse the existing measured page renderer: return up to the requested limit of 20 rows while the whole reply, including coverage, fits 8192 bytes. Continue from the actual returned row count; never claim 20 maximum-size rows always fit, silently drop rows, or return a nonprogressing page.

Every row names kind, state and a real read route: `get_context ref=D-001`, or `get_context ref=docs/x.md heading="..." occurrence=2 view=content` (the preamble uses `preamble=true`). The reply states coverage per source: work, knowledge and documents each complete or PARTIAL with counts and named gaps. The snapshot digests the query, kinds, state, module scope and every source version, never the offset.

Measurement: a declared query corpus lives in the repository test data. A test runs it against a disposable project and records per query hits, expected hits found, misses, per source coverage and reply bytes. Queries marked as expected misses (paraphrases) prove the miss is detected. Semantic retrieval is not built; it is reconsidered only from recorded lexical misses.

## 10 Epic scope projection (observed defects AT-001 and AT-004)

AT-001: Epic and Atomic summaries stop saying `review_module` and `Report the module outcome`. Epic guidance names `review_work` and `record_work verify_criterion`; Atomics name their own review route; Module text is unchanged. The text lives in `read.rs` and the narrow acceptance message in `model.rs`.

AT-004 decision: an Epic scope rolls up its declared members. `project_status module=E-001` and the Epic summary read each member Module and Atomic, print member counts (readable of declared) and Task counts across readable members, label the Epic's own Tasks as direct only, name every unreadable member, set coverage to PARTIAL when any member is unread or missing, and route omitted descendants with `get_context ref=M-00x view=tasks`. The Epic row counts an unreadable member as unreadable, not as not reviewed. The merge happens in `read.rs` over per-member scans, so `Store::scan` is untouched. A complete claim or a zero total is never printed while a member is unread.

## 11 Allocation versions

There is exactly one knowledge allocation observation, covering all six kinds (D, RB, RS, CL, DOC, CP) and the allocator state. M-003 prints it unchanged in project context and consumers pass that same token to `knowledge_work create_*` and `compaction_work propose`; no caller computes or supplies a per-consumer `foreign` list, and M-003 never fabricates one. M-001 owns the observation and derives the DOC and CP identifiers itself from the names in `documents/` and `compactions/`; M-002 and M-005 reserve through the same function with no foreign argument. Incomplete inventory refuses with the named gap. Work allocation is untouched. If M-001 and M-002 instead keep a composed list, one M-001 function must produce it and the printed token must equal the token every writer checks.

## 12 Controls

Each is positive, negative and a named implementation mutant that must fail the same test, then restored, tied to the restored candidate and the published tracker revision of each boundary. Substitutes are not E2E proof; the integration Atomic and real stdio protocol runs prove composition. Git fault tests live in M-004's in-crate fixtures because the SDK cannot reach `cfg(test)` code; the SDK qualifies shipped public behavior.

- Catalog: exactly the existing ten plus four tools; schemas closed; `contract check` clean. Mutant: a tool unrouted.
- Dispatch scope: the lock is held during the body and `settled`; a probing policy cannot take the lock; a stale version has no effect; an unknown op or field is refused naming the field. Mutants: settle after the guard drops; a value echoed in an error.
- Settlement rules: a changed success settles, a real no-op (no actual file publication) does not, an `Ok` with `changed = false` but actual publications settles as Partial and keeps its receipt, an error after publications settles as Partial with the error text unchanged, `git_recovery` skips ordinary settlement, registration is not settled. Mutants: settle a real no-op; settle a read; drop the receipt of an unchanged result with publications; commit that case as a success.
- Acknowledgement target: every work, knowledge, document and compaction target routes `get_context ref=<target>`; `git_recovery` selects `recovery_ack` from the tool kind and routes `get_context` with `ref` omitted; a non-canonical target on another tool invents no route. Mutants: a recovery reply rendered as `ref=Git recovery`; the layout chosen from the target string.
- Receipt projection: the current call's outcome, commit and paths are shown apart from earlier pending facts. Mutant: earlier pending paths merged into the current receipt.
- Hoisted work handlers: the full existing core and protocol suites pass unchanged except for the additive Git line. Mutant: a hoisted handler loses its version check.
- Wire: pages for every budget concatenate to the original for the CRLF, BOM, lone CR, NUL, bidi, non-BMP and fenced-heading fixture, the whole reply is at most 8192 bytes, and an empty span ends with no continuation. Mutants: HTML-escape the payload; subtract no overhead; keep a `Next` on the final page.
- Continuation and schemas: pages share one snapshot, an edit between pages gives `stale`; a `limit` on document content and a selector on a non-document refuse naming the field; row paging of existing views is unchanged. Mutants: offset in the snapshot; `limit` ignored.
- Read: unmanaged, drifted, absent, superseded and partial inventory are labeled; a read changes no byte of the tree. Mutants: read creates `documents/`; partial printed as zero.
- Search: kinds and state filters, per source coverage, routes that open the exact hit; scoped `module` calls unchanged; corpus run recorded. Mutant: a source dropped without a coverage line.
- Epic: a fixture Epic with one unreadable member prints PARTIAL, names it, never zero. Mutants: complete printed; direct counts unlabeled.
- Errors: AT-003 retest names `read_refs` and the limit without the value, and a wrong field name on `plan_work edit_task` is named, on a real stdio call.
- Retained compaction reads (revision 5): the controls of section 15.7 for the history projection, the exact earlier candidate pages, misuse refusals by field and the read only tree check, each with a named mutant that fails the same test and restore.

## 13 Provider alignment against the final published references

The providers' final published references in section 1 govern. This section lists only what M-003 relies on or must project; the earlier proposal level mismatches are closed by those publications, and any divergence found later is a mismatch to report to the provider, never patched here or guessed from staged text.

- M-001, M-002, M-004 and M-005 each supply, in `src/tools/<producer>_ops.rs`, a closed operation enum, `DESCRIPTION` and `execute_locked(store, guard, common, op, effects) -> work::Ack` under `input::Common`. None defines a second common type, decoder, schema helper or outcome type, takes the lock, renders text or adds a Git line. Domain code never imports `tools`.
- M-004 owns the commit policy, `EventOutcome`, `settled`, the pending facts with their exact observation version, the receipt types and the proofs; the store owns `OperationId`, `Publication` and attestation. M-003 consumes them and defines none. The recovery producer validates the version itself.
- M-002 owns document reads, pages, inventory and corpus; M-001 the one knowledge allocation observation and typed read models; M-005 the actual compaction read surface `read_cp(store, id)`, `summaries`, `inventory` and `scan` (the early name `inventory_ids` is obsolete) and the operation handler with its removal barrier. M-003 renders them and defines none.
- References in `Ack.refs` and tracker entries are at most 256 bytes.
- The receipt projection obligation in section 5 holds until the provider publishes its final receipt type; M-003 then renders exactly the published fields.

## 14 Open items

- Markdown scan cost and the query corpus size are measured, not assumed.
- The conditions of the commit policy are owned by M-004 under the owner decision; nothing here chooses a threshold.
- A later M-005 input revision (move and removal handling) will be re-pinned and reconfirmed when published; the current signed pin stays valid until then.
- Closed by decision: work writes settle (section 5); field errors need no dependency (section 3).
- Section 15 (retained compaction reads) is implemented only after root signs this revision and M-006 confirms it; the provider revision 7 is already signed.

## 15 Retained compaction reads (revision 5)

Status: the provider seam is accepted by M-005 and root signed as `compaction-operations` revision 7 (section 1); this section is the M-003 consumer side for root signing. Not implemented, and no code is written before M-006 confirms this revision. Observed gap: a compaction record retains every revision body, every review and the staged candidate bytes of every revision (`compaction.md` sections 2 and 5), but the surface of revision 4 can read only the current revision. On the live tree `ContextArgs` has no `revision` field, `records::validate` allows only `summary`, `tasks`, `review` and `references` for a `CP-` ref, and a public call `get_context ref=CP-001 view=history revision=1` is rejected as an unknown field before any body is read. Title level checks do not prove retention. The owner requirement is that the full E-001 functionality includes exact access to an earlier proposal, its reviews and an earlier candidate body.

### 15.1 Rule: no new tool, storage or dependency

The existing `get_context` is extended. Reads stay read only: no lock file, no directory, no repair, no commit, no Git line. No live document content ever substitutes for a missing or corrupt historical byte.

### 15.2 Exact argument rules

`ContextArgs` gains two optional closed fields: `revision` (integer, 1 or more, one based retained revision number) and `action` (string, 1 to 8 bytes; the provider owns the grammar, exactly `A-` followed by two digits from 01 to 32, so identifiers may be sparse and a consumer never infers the set from the action count). The validation owner is M-003 (`records::validate` and the exported schema). For a `CP-` ref:

| Call | Meaning |
|---|---|
| `view=history` | every retained revision with its full body, every review, and the separate reviewer recovery history; rows paged by `start`, `limit` (1 to 20) and snapshot `version` |
| `view=history revision=N` | only revision N, only the reviews of revision N; the reviewer recovery history is omitted and the page says so |
| `view=content revision=N action=A-01` | the exact hash verified staged Markdown candidate of that action in that revision, as one framed page; both selectors are required |

Refusals, all `invalid_arguments` naming the field, before any read: `revision` or `action` with a ref that is not a `CP-` ref; `action` with any view other than `content`; `revision` with a view other than `history` or `content`; `view=content` without `revision` or without `action`; `revision` equal to 0; `view=content` with `limit`; any document selector (`ordinal`, `heading`, `occurrence`, `level`, `preamble`); `review_index` outside `view=review` (unchanged, still zero based and never repurposed). Other fields keep their meaning. The document selectors stay document only.

### 15.3 History projection (lossless, typed, paged)

M-003 renders the public typed read models, never a storage dump, and every field of every retained struct reaches a row. Row groups in order: (1) record facts (state, current revision, accepted review index and whether that approval is still current, request key, times, apply progress including blocked kind and stored originals); (2) for each selected retained revision in ascending order: header (revision, `current` or `historical`, content hash, author, time, title, staged directory), sources, every action field (id, kind, path, from, base version, purpose, staged digest and length, reason, absorbed targets, state, publication copies), section ledger entries with their dispositions, preservation items; (3) every review in zero based order with revision, content hash, reviewer, verdict, summary, items hash, findings, resolutions, acceptance digests and time; (4) reviewer binding, lost report, predecessors and immersion (omitted with a `revision` selector). A fact longer than 1500 bytes is split into continuation rows at character boundaries with the existing field splitter; nothing is cut. A review of an earlier revision or of a hash that is not the current hash is labeled historical and states the current revision and hash, so historical approval is never shown as current readiness. Enumerations print their provider wire names. The snapshot binds the project, the CP record version, the selector (`all` or the revision) and the view; it excludes `start` and `limit`, so an unchanged record continues and any record change is `stale` with the current snapshot.

### 15.4 Candidate content page

The page reuses template `document_page`, `pages::payload_budget` and `pages::render` and the `md-text-v1` dialect (`markdown::page` over the verified bytes, whole range), with the same exactness: payload delimited by `encoded_len`, total reply at most 8192 bytes, `start` an absolute raw byte offset on a UTF-8 character boundary (otherwise `invalid_arguments` naming `start`), `start` equal to the end gives the empty final page, a missing `version` with `start > 0` is `stale`. Header: heading names the proposal, revision, action, kind and target path; `State:` is `retained revision N of M staged candidate` with `current` or `historical`; `Version:` is the CP record version. Snapshot: `scope_version` over the selection (`get_context:CP-001:content:rN:A-01:md-text-v1`), the CP record version and the verified sha256 of the staged bytes; it excludes `start`. Errors, bounded and naming the field or the relative path: `revision` not retained (the message states the retained range), `action` unknown for that revision, `action` of kind Move or Remove (no staged candidate), `invalid_data` for a missing staged file or a digest or length that does not match the record, and `cp_not_found`. A zero byte candidate is a valid empty final page.

### 15.5 Provider seam from M-005 (`compaction-operations` revision 7, signed)

M-005 owns retained revision and action selection and the hash verified retrieval; there is one verifier. The published text of `compaction.md` section 10a governs; this is the surface M-003 consumes, in `compaction` (re-exported from `src/compaction.rs`), read only, taking no lock:

```rust
/// Retained revision by one based number; invalid_arguments naming `revision` when it is 0 or not retained.
pub fn retained_revision(record: &CpRecord, revision: u32) -> store::Result<&RevisionRecord>;
/// Exact verified staged candidate of one action of one retained revision.
pub struct StagedCandidate { pub revision: u32, pub action: String, pub kind: ActionKind,
                             pub path: String, pub sha256: String, pub bytes: Vec<u8> }
pub fn read_staged(store: &Store, record: &CpRecord, revision: u32, action: &str)
                   -> store::Result<StagedCandidate>; // no lock, no write, no repair, no live fallback
```

`read_staged` selects the revision, finds the action (`invalid_arguments` naming `action` when absent), requires a staged digest (Move and Remove refuse naming `action`), reads `stage_path(id, revision, action)` through `Store::bytes`, and refuses `invalid_data` for a missing file, a digest mismatch or a length mismatch. `load_blobs` (apply and revise) calls the same private verifier for the current revision. This extraction preserves every valid revision 6 write, whose recorded length and digest match its staged bytes, and it now also enforces the already recorded `staged_len`, which the earlier `load_blobs` did not check; the contract makes no claim that a record with a malformed recorded length behaves as before. The apply messages stay unchanged. The messages of `read_staged` are `<relative>: staged candidate is missing.` and `<relative>: staged candidate does not match its recorded hash.`, and `retained_revision` states the retained range `1 to <current>`. No `expected` or snapshot parameter, history pager or new error code exists in the provider; row paging, the snapshot token and the page framing are M-003's. `read_cp`, `summaries`, `inventory`, `scan` and every record struct field stay as published; no record is pruned and `cp-inventory` revision 1 is unchanged. M-003 needs no other M-005 function; the history projection uses the public struct fields.

### 15.6 Ownership, parties and sequence

M-003 owns `ContextArgs`, the exported schema and catalog description, validation, the typed lossless projection, the paging and the page presentation. M-005 owns the helper of 15.5. M-006 owns the public SDK reconstruction (15.7) as consumer of `registered-tool-surface`. Producer-host stays at revision 3 because no mutation seam, `Ack` field or producer handler changes. Sequence: root signed `compaction-operations` revision 7 (done) and M-005 publishes and agrees its provides; M-003 consumes revision 7 with the exact reference of section 1 and agrees; root signs `registered-tool-surface` revision 5 and M-003 publishes the tracker entry with that signed reference; M-006 confirms; only then code. This file names no commit of its own. M-001, M-002 and M-004 are not affected: no knowledge, document or Git surface changes.

### 15.7 Controls (positive, negative, named mutant, restored)

Provider (M-005): revision 2 keeps revision 1 byte identical; `read_staged` returns revision 1 bytes after a revise, a missing blob, a corrupt blob and a length different from the recorded length refuse `invalid_data`, Move, Remove, a non canonical and an absent identifier refuse naming `action`, the verifier is the one `load_blobs` uses. Mutants: select the current revision instead of the named one; skip the digest check; fall back to the live document.

M-003 (unit and stdio): after revision 2 the history lists both bodies and the revision 1 review as historical; `revision=1` shows revision 1 only; a structural leaf walk of the serialized retained record proves every leaf value appears in the unselected history. The candidate of revision 1 differs from revision 2 (fixture with BOM, CRLF, fenced headings and non-BMP text), pages concatenate to the exact staged bytes, every reply is at most 8192 bytes, `limit` refuses, a start inside a character names `start`, a record change makes continuation `stale`, and editing or deleting the live document changes no byte of the page. Misuse refuses by field for every row of 15.2. The tree bytes before and after every read are identical. Mutants: history omits a field; `revision` ignored; live document read; digest unchecked; offset inside the snapshot; `limit` accepted.

Public SDK (M-006): the same reconstruction of revision 1 proposal, its reviews and its candidate bytes after revision 2 on the shipped binary with read only tree checks, bounded framing and stale continuation; a title only check is not proof. Strict pages and any candidate over 8192 bytes are never waived.
