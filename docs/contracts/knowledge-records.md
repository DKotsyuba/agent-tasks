# Typed knowledge records — planning contract

Status: canonical contract. Each pair contract in section 11 carries its own revision in the table; one artifact may serve boundaries of different revisions, so the heading of this file never names a boundary revision. It describes the implemented public boundary of the typed knowledge domain (`src/knowledge.rs`, `src/tools/knowledge_ops.rs`) as reviewed; source docstrings stay the truth for every detail below the boundary and [architecture.md](../architecture.md) for the whole product. Revision numbers belong to the provider (this domain); a consumer never defines them. The artifact identity is its committed bytes: a boundary pins a commit, path and SHA-256, never a heading.

Revision history. Revision 5 (commit 485137330f11a9ab05088801309f66c2c6e17d5b) was a planning text written before implementation and is superseded. Revision 6 changes: (a) status and consumed pins to the actual provider revisions of section 11; (b) the implemented record fields `revised_at` and `revised_by`, the public helpers listed under section 4 and the `allocation_changed` error; (c) the successor rule of section 6, where no chain walk exists because a successor must be current; (d) the runbook work classification, with embedded Atomics distinguished from Tasks, and its attribution rule; (e) the reserve refusal order of section 7, where an unreadable allocator refuses with `allocator` before any inventory check, and the create order that creates a home only after a reservation; (f) the `applicability` search label. Revision 7 of `kr-operations` (and of `kr-model-read`, whose stored summary repeated the same sentence) changes no behavior: it replaces the stored summary sentence that read as if a runbook use could name only a Task or an embedded Atomic with the complete coverage matrix of section 6 rule 2 (bare `M-`, `A-` and `E-` references, `M-nnn/T-nnn` and `M-nnn/A-nnn`). The other three boundaries stay at revision 6.

Component names: **knowledge records** (this document), **document module** (managed Markdown, `DOC-` records), **compaction module** (`CP-` records), **publication module** (store primitives, Git engine), **registry module** (tool registration, `input::Common`, schema export, routing, rendering), **qualification**.

## 1 Scope

Provided here: closed Decision, Runbook, Research and procedural Checklist records, their lifecycle, the ONE knowledge allocator and its single allocation observation across six homes, guarded storage of the four typed kinds, the typed ID grammar, and the operation payload and locked handler.

Not provided here: Markdown bodies, `DOC-` record content and reference grammar (document module); `CP-` record content and nested staging grammar (compaction module); tool registration, routing, schema export, descriptions, event construction, rendering and ranking (registry module, sole owner of `src/tools/mod.rs`, `src/tools/input.rs`, `Common`); Git, journal, locks and publication primitives (publication module); skills and end-to-end qualification. This domain edits none of those files.

## 2 Storage layout and identifiers

| Prefix | Home | Record file | Owner of content |
|---|---|---|---|
| `D-` | `decisions/` | `D-001.yaml` | knowledge records |
| `RB-` | `runbooks/` | `RB-001.yaml` | knowledge records |
| `RS-` | `research/` | `RS-001.yaml` | knowledge records |
| `CL-` | `checklists/` | `CL-001.yaml`; item `I-001` inside the record, addressed `CL-001/I-001` | knowledge records |
| `DOC-` | `documents/` | `DOC-001.yaml` | document module |
| `CP-` | `compactions/` | `CP-001.yaml` plus directory `CP-001/` holding `rN/A-NN.md` blobs | compaction module |
| allocator | `.agent-tasks/knowledge.yaml` | tracked, closed | knowledge records |

An ID is the prefix plus the decimal number formatted `{n:03}` (so `D-1000` is canonical for 1000); every other spelling is refused (`model::number` rule). Directories are created on first use by the owning writer; registration and reads create nothing. The six homes above are the only locations that carry allocator-governed numbers.

## 3 Bounds

| Bound | Value | At the limit |
|---|---|---|
| Record bytes | `store::RECORD_CAP` (512 KiB), nonterminal writes keep `CLOSING_RESERVE` (32 KiB) | `capacity`, nothing written |
| Entries per home | `store::MODULE_CAP` (512) | allocation refused `inventory`; `complete=false` |
| Nested walk below a `CP-NNN/` child | depth 2 (`rN/A-NN.md`), 512 entries per child | `complete=false`, named warning |
| Retained prior revisions | 16 (`HISTORY_CAP`) | edit refused `history_unrecoverable` unless section 8 proof exists |
| Runbook use records | 64 (`USE_CAP`) | same rule |
| Checklist events | 128 (`EVENT_CAP`) | same rule |
| Evicted locators | 64 (`LOCATOR_CAP`) | `capacity` |
| Text | `model::text`: nonempty after trim, no control characters except `\n` `\t`, byte cap per field | `invalid_arguments` |
| Lists | at most 8 entries unless stated (steps 32, checklist items 32) | `invalid_arguments` |

## 4 Public Rust surface (implemented signatures)

```rust
// src/knowledge.rs
pub const SCHEMA: u32 = 1;
pub enum Kind { Decision, Runbook, Research, Checklist }           // the four typed kinds, serde snake_case
impl Kind {
    pub fn prefix(self) -> Prefix;                                 // Prefix::Decision ...
    pub fn of(id: &str) -> Result<Kind, String>;                   // canonical typed ID only
    pub fn path(self, id: &str) -> Result<String, String>;         // "decisions/D-001.yaml"
}
pub enum Prefix { Decision, Runbook, Research, Checklist, Document, Compaction }
impl Prefix {
    pub const ALL: [Prefix; 6];                                    // fixed order: D RB RS CL DOC CP
    pub fn as_str(self) -> &'static str;                           // "D-" "RB-" "RS-" "CL-" "DOC-" "CP-"
    pub fn directory(self) -> &'static str;                        // "decisions" ... "compactions"
}
/// The ONE typed identifier grammar. Document and compaction modules call it; none keeps a copy.
pub struct KnowledgeId { pub prefix: Prefix, pub number: u64, pub item: Option<u64> }   // item only for CL-n/I-m
impl KnowledgeId { pub fn canonical(&self) -> String; }
pub fn parse_id(raw: &str) -> Result<KnowledgeId, String>;         // "D-001", "CL-001/I-002"; refuses leading-zero variants

pub enum Currentness { Current, Superseded }
pub struct Meta { pub schema_version: u32, pub id: String, pub created_at: String, pub created_by: Option<String>,
                  pub updated_at: String, pub updated_by: Option<String> }
pub struct Lifecycle { pub state: Currentness, pub superseded_by: Option<String>, pub superseded_at: Option<String> }
pub struct Revision<B> { pub revision: u64, pub at: String, pub by: Option<String>, pub content: B }
pub struct Evicted { pub revision: u64, pub locator: String }              // locator <= 256 bytes
pub struct Decision  { pub meta: Meta, pub revision: u64, pub revised_at: String, pub revised_by: Option<String>,
                       pub lifecycle: Lifecycle, pub content: DecisionBody,
                       pub history: Vec<Revision<DecisionBody>>, pub evicted: Vec<Evicted> }
pub struct Research  { /* same shape with ResearchBody, including revised_at and revised_by */ }
pub struct Runbook   { /* same shape with RunbookBody, including revised_at and revised_by, plus */ pub uses: Vec<RunbookUse>, pub next_use: u64,
                       pub evicted_uses: Vec<Evicted> }
pub struct Checklist { pub meta: Meta, pub state: ChecklistState, pub content: ChecklistBody,
                       pub events: Vec<Event>, pub next_item: u64, pub evicted_events: Vec<Evicted> }
// Every field of every public record struct (bodies, RunbookUse, Event, item and alternative structs) is `pub`;
// the registry module renders content rows by matching `Any` variants, so no accessor per row is promised.
pub enum Any { Decision(Decision), Runbook(Runbook), Research(Research), Checklist(Checklist) }
impl Any {
    pub fn id(&self) -> &str;            pub fn kind(&self) -> Kind;     pub fn title(&self) -> &str;
    pub fn state_label(&self) -> &'static str;  // current|superseded|open|completed|canceled
    pub fn current(&self) -> bool;       // not superseded; checklist: open
    pub fn revision(&self) -> u64;       // definition revision; checklists report 1
    pub fn superseded_by(&self) -> Option<&str>;
    pub fn search_text(&self) -> Vec<(&'static str, String)>;   // ordered (label, text); no YAML
    pub fn references(&self) -> Vec<String>;                    // typed IDs, work refs, detail string
}
pub struct Scan { pub records: Vec<store::Snapshot<Any>>, pub unreadable: Vec<String>,
                  pub warnings: Vec<String>, pub complete: bool, pub version: String }
pub fn load(store: &Store, id: &str) -> store::Result<store::Snapshot<Any>>;
pub fn scan(store: &Store, kind: Option<Kind>) -> store::Result<Scan>;

// Additive public helpers, documented in source: trait Revisioned (edit, supersede, revision lookup) and trait Retained
// (eviction of the oldest retained entry); struct UseInput and UseInput::validate; builders new_decision, new_research,
// new_runbook and new_checklist; ensure_home, create_file, replace_file, evictor, require_project and arguments;
// struct AllocatorFile (the closed knowledge.yaml schema); constants HISTORY_CAP, USE_CAP, EVENT_CAP, LOCATOR_CAP,
// ITEM_CAP, STEP_CAP, HOME_CAP and ALLOCATOR_PATH.

/// One allocation observation over all six homes (section 7). No caller-specific input exists.
pub struct HomeState { pub prefix: Prefix, pub ids: Vec<String>, pub max: u64,
                       pub warnings: Vec<String>, pub complete: bool }
pub struct Allocation { pub homes: Vec<HomeState> /* in Prefix::ALL order */, pub counters: Option<Counters>,
                        pub warnings: Vec<String>, pub complete: bool, pub version: String }
pub struct Counters { pub decision: u64, pub runbook: u64, pub research: u64,
                      pub checklist: u64, pub document: u64, pub compaction: u64 }
pub fn observe_allocation(store: &Store) -> store::Result<Allocation>;
pub fn allocation_version(store: &Store) -> store::Result<String>;   // == observe_allocation(store)?.version
pub fn reserve(store: &Store, lock: &store::LockGuard, prefix: Prefix, expected: &str,
               effects: &mut Vec<String>) -> store::Result<String>;  // returns the canonical ID

/// Child-ID existence for the reference resolver (the document module resolves `CL-001/I-001`).
pub enum Child { Found { state: &'static str }, MissingItem, MissingParent }
pub fn resolve_child(store: &Store, id: &KnowledgeId) -> store::Result<Child>;   // read only, item IDs only
// Any::items() -> Vec<(String /* I-001 */, &'static str /* open|done|canceled */)> gives the same facts for a loaded checklist.

// src/tools/knowledge_ops.rs
#[derive(Deserialize, JsonSchema)]
#[serde(tag = "op", rename_all = "snake_case", deny_unknown_fields)]
pub enum KnowledgeOp { /* section 6 */ }
pub const DESCRIPTION: &str;              // description text for the registered tool `knowledge_work`
pub fn execute_locked(store: &Store, guard: &store::LockGuard, common: &input::Common, op: KnowledgeOp,
                      effects: &mut Vec<String>) -> store::Result<work::Ack>;
```

Rules for the seam:

1. `Common` is the registry module's existing `tools::input::Common` (`project`, `version`, `actor`, `reference`); this domain defines no lookalike, no decoder and no schema. The dispatcher decodes with `input::mutation::<KnowledgeOp>(args, false)` (so `ref` stays inside the operation variants and `Common.reference` is `None`) and exports the schema with `input::mutation_schema::<KnowledgeOp>(false)`.
2. The result is the registry module's existing `work::Ack`, extended by that module with `notes` (at most 8 lines of 200 bytes) and `refs` (at most 16 canonical references, each at most 256 bytes); this domain adds no outcome type and builds it with the shared `ack(target, version, phase, changed)` helper, then fills `refs`. Values supplied: `target` = canonical ID of the record written (`D-001`; a checklist item operation reports the checklist ID), `version` = `Store::version` of the record after the write (the record's current version on an unchanged call), `phase` = the record's actual state label (`current`, `superseded`, `open`, `completed`, `canceled`), `changed` = whether any file was published, `refs` = affected canonical references (the record, a superseded successor, `CL-001/I-001` for an item). `phase_label` is derived by `ack` from the `D-`, `RB-`, `RS-`, `CL-` target prefix; this domain never sets it. `refs` are advisory for the typed event; identity is the journal.
2a. Argument validation names the field: every list, text, count and enum check is wrapped with the registry module's `input::field(name, checked)`, so a refusal reads `<field>: <rule>` and never echoes a supplied value. `ref`, item, revision and every other variant field is a validated, named input.
3. There is no own-lock entry point. The dispatcher resolves the alias once, takes the root write lock once, calls `execute_locked`, then settles Git before releasing. `execute_locked` never locks and never runs Git, and a dispatcher must never route a call that locks itself. The dispatcher reads published paths from `store.publications()`, not from the `Ack`.
4. Callers must not mutate typed record files or `knowledge.yaml` outside these functions (documents and compactions mutate their own files, reserving numbers only through `reserve`).

## 5 Record content

All structs use `deny_unknown_fields`; text is English.

- **DecisionBody**: `title` (256), `question` (1024), `decision` (2048, the adopted statement), `rationale` (4096), `alternatives` ≤8 of `{option (256), rejected_because (1024)}`, `open_questions` ≤8 of `{text (512), needs_owner: bool}`, `detail: Option<String>` (256).
- **ResearchBody**: `title`, `question` (1024), `conclusions` 1–8 of `{statement (1024), basis: measured|cited|inferred}`, `evidence` ≤8 of `{claim (512), source (256), basis}`, `limitations` ≤8 (512), `applicability` (512), `detail`. A measured or cited conclusion requires at least one evidence entry with that basis.
- **RunbookBody**: `title`, `purpose` (1024), `prerequisites` ≤8 (512), `inputs` ≤8 of `{name ([a-z0-9_], 64), description (512), required}`, `steps` 1–32 of `{title (128), command: Option (1024), description (1024), expected (512), recovery: Option (512)}`, `pitfalls` ≤8 (512), `detail`. Step ordinals are positions; reading never executes a command.
- **ChecklistBody**: `title`, `purpose` (1024), `items` ≤32 of `{id: "I-001", text (512), state: open|done|canceled, resolution: Option<{at, by, text (512)}>}`.
- **RunbookUse**: `{id: "U-001", at, by: Option, revision, environment (256), outcome: succeeded|failed|partial, checks ≤8 of {label (64), status: passed|failed|not_run|not_applicable, detail?}, artifacts ≤8 (256), observation: Option (1024), work: Option<String>, stale_revision: bool, superseded_record: bool}`.
- **Event** (checklist): `{at, by, action: created|add_items|resolve_item|complete|cancel|reopen, note: Option (512)}`.
- **ChecklistState**: `open | completed | canceled`.

`detail` is an optional reference to a managed document, stored verbatim and accepted only if the document module's `valid_reference(store, raw)` returns `Ok` (section 11, consumed `doc-reference`): root-relative `README.md` or `docs/<path>.md`, optional `#fragment`, target must exist and the fragment must resolve. A refusal is passed through unchanged as `invalid_arguments`. Reads never revalidate it (a document may later move; the reference scan reports that).

Generated, never accepted from callers: `id`, `schema_version`, all dates, `revision`, item and use IDs, `superseded_at`, `stale_revision`, `superseded_record`. `by` is the declared `actor`; absence stays `None`, never invented.

## 6 Operations (registered by the registry module as the purpose tool `knowledge_work`)

Common fields come from `input::Common`: `project`, `version`, optional `actor`. `version` is the knowledge allocation version (`allocation_version(store)`, the value any reader shows) for `create_*`, and the record version (`Store::version` of the exact file bytes) for every other op.

| op | Fields | Effect |
|---|---|---|
| `create_decision` | title, question, decision, rationale, alternatives?, open_questions?, detail? | reserve ID, publish allocator then record, revision 1 |
| `create_runbook` | title, purpose, prerequisites?, inputs?, steps, pitfalls?, detail? | same |
| `create_research` | title, question, conclusions, evidence?, limitations?, applicability, detail? | same |
| `create_checklist` | title, purpose, items (1–32 texts) | same; items get `I-001…`, state open |
| `edit_decision` / `edit_runbook` / `edit_research` | ref, any subset of content fields | omitted keeps, null refused, value replaces the whole field; changed content → revision+1, previous content pushed to `history`; identical content → unchanged, no write |
| `supersede` | ref (predecessor), successor | predecessor `lifecycle` → superseded with `superseded_by`; one file written; revision not bumped |
| `use_runbook` | ref, revision, outcome, environment, checks?, artifacts?, observation?, work? | append a `RunbookUse`; definition revision unchanged; one file written |
| `add_items` | ref, items (texts) | open checklist only |
| `resolve_item` | ref, item, state: done\|canceled, text | done needs a completion fact, canceled a reason (`text` required for both) |
| `complete_checklist` | ref | every item terminal and at least one done |
| `cancel_checklist` | ref, reason | remaining open items stay open, no cascade |
| `reopen_checklist` | ref, reason | completed/canceled → open, event retained |

Rules:

1. **Supersession.** One authoritative pointer on the predecessor. Required: different ID, same kind, successor state current and readable. A current successor carries no successor pointer, so no chain or cycle can exist and no walk is performed; a superseded successor is refused. A predecessor already pointing to the same successor returns unchanged; a different one, `conflict`; a superseded successor, `conflict`. Superseded records refuse `edit_*` (`conflict`); correct them through a successor. Creating a successor and linking it are separate writes; an interruption between them leaves two current records, which context shows, and the explicit `supersede` is the recovery. No transaction is invented.
2. **Runbook use** records an observation, never a verification of other revisions. `revision` must exist (current, or retained in `history`; else `not_found`). `environment` is required, plus at least one piece of evidence: a check, an artifact or an `observation`, each nonempty under `model::text` (no byte threshold beyond nonempty). `succeeded` forbids any failed check and needs at least one passed check, an artifact or an observation; `failed` needs a failed check or an observation. `work`, when given, must name stored work. Coverage matrix: a bare `M-`, `A-` or `E-` reference is accepted when that stored record exists; `M-nnn/T-nnn` is accepted when the Module has that Task; `M-nnn/A-nnn` is accepted when the Module has that embedded Atomic (the two child kinds are looked up in their own lists). Everything else is `invalid_arguments`: a missing record, Task or Atomic, or any other child kind. If the Module has a bound lead without a recorded loss and `actor` is not that lead's agent id (or is absent), `attribution` refusal with nothing written. Without `work`, attribution stays the declared actor or unknown. A use of a non-current revision sets `stale_revision`; a use of a superseded record sets `superseded_record`. A use never bumps `revision` and never changes currentness.
3. **Checklists.** Item text equal to a bare canonical work reference (for example a Module or Task reference) is refused: work TODO views reuse canonical Tasks. Skipped is `canceled` with a reason and never counts as done. Closed checklists refuse everything except `reopen_checklist`.
4. Edits and uses never alter `evicted` entries.

## 7 Allocator and the single allocation observation (`kr-allocator`)

`.agent-tasks/knowledge.yaml`, tracked, closed:

```yaml
schema_version: 1
next_decision: 1
next_runbook: 1
next_research: 1
next_checklist: 1
next_document: 1
next_compaction: 1
```

**The observation.** `observe_allocation(store)` reads, in this fixed order and with no caller input:

1. the version of `project.yaml`;
2. the version of `knowledge.yaml` (or its absence marker);
3. for each home in Prefix::ALL order, sorted identifiers, named warnings and completeness from its owning inventory. The five flat homes use the existing Store::kind_inventory(directory, prefix) without a new generic callback or changed work-directory grammar. The compaction home uses compaction::inventory(store) -> store::Result<store::Inventory>, owned by M-005 under the reciprocal cp-inventory contract toward M-001. It reuses Store::list_dir and store::own_temp_name, and validates its own closed CP-NNN.yaml plus CP-NNN/rN/A-NN.md namespace. A CP-NNN directory counts as that identifier's existence even if its record publication was interrupted. Unknown nested names, links, nonregular entries, excessive depth or caps produce named incomplete coverage. Its use of knowledge::parse_id is pure identifier parsing and never calls the allocation observer, so this source coupling creates no runtime recursion or completion wait.

`version` is SHA-256 over a length-prefixed encoding (domain `agent-tasks/knowledge-allocation/v1`) of those inputs. The work allocator state, the work inventory and any Markdown are not inputs; knowledge writes therefore never invalidate a pending work creation and work writes never invalidate a pending knowledge creation. Readers, creators of any prefix, and the compaction module's `observe` all call this same function and obtain the same token for the same disk state.

Consequence stated honestly: any reservation or any creation of a record in any of the six homes changes the token, so every other pending knowledge, document or compaction creation becomes `stale` and must re-read. Edits, uses and supersessions do not change it.

**`reserve(store, lock, prefix, expected, effects)`**, with the root write lock held by the caller:

1. `guard` must be the lock acquired through this store (else `not_locked`). `expected` must equal a fresh `observe_allocation(store).version` (else `stale`).
2. The allocator file is read before any inventory check. A corrupt, unknown-schema or unknown-field file refuses `allocator` with restore guidance and is never rewritten. Then an incomplete or foreign home refuses `inventory`, nothing reserved. Absent file is valid only when all six homes are empty; it is then created with all six counters 1 before incrementing. Absent file with any existing ID, a zero or `u64::MAX` counter, or any counter not strictly above the largest numeric ID of its own home (all six checked, whoever the caller is) → `allocator` ("restore retained state"); counters are never guessed.
3. The incremented file is published with the exact observed bytes (guarded replace) and read back; effect `Published .agent-tasks/knowledge.yaml.` follows. Only then does the caller publish its record (typed kinds: this domain; `DOC-`: document module; `CP-`: compaction module). A failure in between leaves the reservation published and the number a visible gap; numbers are never recycled and no later reservation fills a gap.
4. The return value is the canonical ID.

Every creation uses this one allocation observation. The observer composes each owning inventory once; document body and metadata observations remain M-002's responsibility, while CP nested inventory remains M-005's responsibility. No caller supplies a private list or maintains another allocation token.

## 8 Publication, effects and history safety (`kr-paths`)

Each mutation writes only: `<home>/<ID>.yaml` for the four typed kinds (new: no-clobber; replace: observed-bytes guarded), `.agent-tasks/knowledge.yaml` on create, and gitignored `.agent-tasks/backups/` normalization copies made by `Store::save`. Writes use the publication module's primitives (today `Store::save`); the typed events they emit and the existing `Published <relative>.` effect lines are the only effect record, also after a partial failure. This domain runs no Git, writes no journal, never stages and never commits. `create_*` writes two files in one handler call (one intent): the allocator first, then the record. The home directory is created (a not-applicable directory event) only after the reservation succeeded, so a refusal before it creates no home and no effect; every other op writes exactly one file. The dispatcher alone builds the event and settles.

Owner decision on persistence: every successful mutation of a typed record is followed, by the dispatcher inside the same lock scope, by an automatic local commit of exactly that call's paths. Reads and unchanged results (`changed=false`, no publication) trigger no settlement; a failed handler that already published is settled by the dispatcher as failed and its policy never commits it (pending or unknown receipt); a refused or failed Git step leaves the saved files and a truthful pending or unknown receipt, never an undone write. Open interaction with the publication module: a `create_*` that fails after the allocator reservation was published (record unpublished) must not commit, so `.agent-tasks/knowledge.yaml` stays a pending published path with a visible number gap; the next successful mutation that rewrites that file commits it inside its own intent. The publication module must confirm that a failed handler's published paths stay pending and are not committed alone.

Reads use `Store::bytes`/`Store::version`; no read writes. All writes pass through one private `persist` call site.

History: edit, use and event appends never discard prior content. At `HISTORY_CAP`, `USE_CAP` or `EVENT_CAP`, or when the encoded record would exceed the byte cap, the oldest entry may be evicted only after the handler calls the publication module's committed-bytes proof directly (`persist::locate_committed(store, relative, version, bytes)` for the exact current record bytes, a private one-line forward inside the knowledge handler, publication revision 2 shape) and receives a locator proving them present in a verified committed object; the locator is stored as `Evicted`. This domain implements the call site only: no trait, adapter, Git command or provenance logic lives here. A `not_committed` answer, a missing repository or any failure refuses the write `history_unrecoverable` and nothing is written. Pending, deferred or unverified persistence is not proof. The proof is for the bytes before the new write, since those hold the entries being dropped.

## 9 Read contract (`kr-model-read`)

`scan(store, kind)` reads every readable record of the four kinds in numeric order and always returns a `Scan`: a corrupt, oversized, symlinked, foreign-schema or misnamed file becomes an `unreadable` row naming the ID or file plus a warning; `complete=false` when anything was skipped, the cap was hit or a home is not a directory. One bad sibling is never an error and never a silent drop. `version` digests exactly the scanned bytes. `load` is the exact-read route (`not_found` for an absent file, `invalid_data` for an unreadable one). `search_text` returns, in fixed order per kind with stable labels (`title`, `question`, `decision`, `rationale`, `alternative`, `open_question`, `conclusion`, `evidence`, `limitation`, `applicability`, `purpose`, `prerequisite`, `step`, `pitfall`, `item`, `reason`), the semantic fields only. `references` returns canonical typed IDs, work references and the `detail` string; the document module's reference scan resolves them.

## 10 Errors

Codes (`store::Error::code`): `invalid_arguments`, `stale`, `busy`, `not_initialized`, `not_found`, `conflict`, `attribution`, `capacity`, `allocator`, `allocation_changed` (the allocator bytes differ after the guarded publish; inspect context, the number stays a gap), `inventory`, `invalid_data`, `history_unrecoverable`, `file_type`, `io`, and passed through unchanged from the publication module: `not_locked`, `pending_full`, `durability_unknown`, `not_committed`, `encoding_unreadable`. Messages are bounded, English and carry the recovery step; no raw YAML, Git output or paths outside the root. A refusal publishes nothing except the create case between allocator and record, which the events and effect lines name.

## 11 Pair contracts

Provided (provider: knowledge records), revision per boundary:

| ID | Revision | Consumer | Obligation |
|---|---|---|---|
| `kr-model-read` | 7 | registry module | sections 4, 5, 6, 9, 10: read API, `resolve_child`, `KnowledgeOp`, `DESCRIPTION`, `execute_locked` returning `work::Ack`; the registry module owns registration, `Common`, `Ack`, locking, event construction, rendering, ranking |
| `kr-allocator` | 6 | document module | sections 2, 4 (`parse_id`, `Prefix`, `observe_allocation`, `allocation_version`, `reserve`), 7, 10: single token, `DOC-` counter and home, allocator refusal codes |
| `kr-history` | 6 | compaction module | sections 7, 8, 9: `CP-` counter and home, nested grammar hook, lossless history, `references()`; typed record files are never rewritten by compaction |
| `kr-paths` | 6 | publication module | section 8: owned paths, one lock scope, direct committed-bytes proof call, no Git or provenance here |
| `kr-operations` | 7 | qualification | sections 6, 10, 13: operation semantics and controls, exercised only through the registered tool |

Consumed (provider-owned id, revision and artifact; this domain never defines them):

- `doc-reference` revision 2 from the document module: `references::valid_reference(port, raw)` over the document store port, as used in section 5. Pinned at commit ea3a17fd4169f4c254ecd1f082a85c5f51d801da, `docs/contracts/markdown-references.md`, SHA-256 `e9808e8336df412a3219c8858435977e0ef966c19db6b7479609119b2f99f94c`.
- `persist-store-m001` revision 3 from the publication module: guarded `Store::save`, the public `Store::kind_inventory(directory, prefix)`, `Store::create_dir`, `LockGuard::is_for` and the committed-bytes proof `locate_committed`. Pinned at commit eaa02c833f590c8b34410069918fbc97bd369786, `docs/contracts/publication-git.md`, SHA-256 `0816759e9c2df1500f45829b0ecd509d045f387049e7455fc7f9df5257727b89`.
- `cp-inventory` revision 1 from the compaction module: `compaction::inventory(store)` for the compactions home. Pinned at commit 256f77c8b9013d66e77d40213d1d5357810e261b, `docs/contracts/compaction.md`, SHA-256 `e24fd79d2e9fd0198d791683309d36e2a98ee2d1b5ee06989a8c4444e4bd89a6`.
- `producer-host` revision 3 from the registry module: the shared `Common`, `input::mutation`, `input::field` and `work::ack`. Pinned at commit e7621290918d568dd4175d8b3dd8b90663e104bd, `docs/contracts/registered-tool-surface.md`, SHA-256 `56c9924077ffcb13a1ef44be8f601267f14d8c57c1db3ba2219bed84e937940c`.

None of these is a completion wait; each is real in-process source.

## 12 Compatibility

Existing Project, Epic, Module and Atomic files, `.agent-tasks/state.yaml` (allocator schema 2), the work allocation version, the work `inventory()`, `project_status` counts and `register_project` are unchanged in both directions. A store without any knowledge file is valid and reads as empty. Older builds ignore the new homes and file. `knowledge.yaml` schema 1 is read-compatible by later builds; an unknown schema is refused, never rewritten.

## 13 Controls

For each: positive (P), negative (N), mutation (M) that the same test must catch.

- **Schema/lifecycle** — P: create each kind; edit bumps revision to 2 with previous content in `history`. N: unknown field, caller-supplied `revision`/`id`/date, empty or oversize text, null patch, edit of a superseded record. M: edit leaves revision unchanged; unknown fields accepted.
- **Supersession** — P: current same-kind successor links; repeat is unchanged. N: self, different kind, superseded successor, second different successor, cycle fixture. M: self-supersession allowed; pointer written on the successor.
- **Allocation observation** — P: the document-style, compaction-style and typed-style callers read the identical token for one disk state; first reserve creates six counters; counters survive restart. N: file absent with a record in any of six homes, counter at or below an existing ID of another home, stale token, incomplete home, corrupt or unknown-schema allocator file (code `allocator`, file untouched, no effect). M: the token depends on the caller or omits a home (a `DOC` creation then leaves the typed token unchanged); a counter not advanced after a failed record publish (ID reused); work allocator bytes changed.
- **Nested recognition** — P: `compactions/CP-001.yaml` with `CP-001/r1/A-01.md` is complete and counts number 1 once. N: `CP-001/x.txt`, depth 3, a symlink and 513 entries each set `complete=false` and name the entry. M: unknown nested names ignored; recognized names reported foreign.
- **Runbook use** — P: use of the current revision leaves revision and content unchanged and stores environment plus evidence; a Task reference, an embedded Atomic reference and a bare Module reference are accepted. N: unknown revision, no environment, no evidence, succeeded with a failed check, wrong or absent actor for a Module with a bound lead, a missing Task, a missing embedded Atomic, an unknown child kind. M: Atomics looked up among Tasks; attribution skipped. M: use bumps the definition revision; a use of an old revision is unflagged.
- **Checklist** — P: resolve with fact or reason, complete with one done and the rest canceled, cancel with reason, explicit reopen. N: done without fact, cancel without reason, complete with an open item or all canceled, bare work reference as an item, change on a closed list. M: skipped counted as done; completion with an open item allowed.
- **Reads** — P: scan lists the four kinds in order with complete=true. N: a corrupt sibling and a symlink appear in `unreadable` with complete=false. M: scan drops the corrupt record.
- **Locked handler and publication** — P: create emits allocator then record; other ops one path; a stale write leaves the file byte-identical; the handler never locks and a second lock attempt from a caller reports `busy`; a refusal before the reservation creates no home and no effect. N: history at cap in a store without a committed copy refuses `history_unrecoverable` unchanged. P: the same cap after the record bytes are committed in a disposable real repository evicts the oldest entry and stores the locator. M: eviction without a locator; handler takes its own lock.
- **Ack and child resolution** — P: each op returns the target, actual state label, `changed` and refs; `resolve_child` finds `CL-001/I-001`. N: unchanged call reports `changed=false` and no publication; a missing item and a missing checklist are distinct answers. M: `phase` echoes the request instead of the stored state.
- **Detail reference** — P: an existing `docs/x.md#frag` accepted and stored verbatim. N: missing target, `..`, absolute path, typed ID refused unchanged. M: reference stored without validation.
- **Compatibility** — P: existing fixture stores and the legacy-schema scenario read and write exactly as before; knowledge homes add no work inventory warnings. M: the work inventory counts a knowledge file.

## 14 Open items, not decided here

The registered tool's final name and schema text belong to the registry module. The 16/64/128 retention counts and 512 per home are initial defaults to validate against real record sizes during implementation. The owner chose automatic local commits after every successful mutation (see section 8); the engine, policy and receipts stay with the publication module and no policy switch exists here. Semantic retrieval is out of scope; lexical measurement belongs to the registry module.
