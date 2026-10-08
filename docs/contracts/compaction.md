# Compaction contract (M-005), final revision 8

Status: canonical provider artifact for Epic E-001, Module M-005. Implementation and qualification evidence are recorded separately; this contract is not a release certificate. This file owns two boundaries:

- `compaction-operations` revision 8 (additive to revision 7 and changing no payload or Rust surface (the explicit pre-adopt check of constraint (b) is code that follows the signed text): it states precisely when the roll-forward `adopt` of section 9 may also resume a Replace of a `Managed` document, see the amended paragraph "Missing generated metadata" of section 9; revision 7 below stays the signed read-only seam at commit 686dab8bfc98b043587142884378ff3b3250dff3 and `cp-inventory` revision 1 is unchanged). Revision 7 text: `compaction-operations` revision 7 (the tracker entry is raised only after root signs this text and the immutable pin is known; revision 6 at commit b5a0d8d09155f6338f7699bbe9ac962702d90c8d sha256 644acee4df6057f2df030dd983d5e738dc661733b9c17c419b77510f437df3a0 stays immutable history; **revision 7 is read-only and additive: it adds the retained revision selection and the hash verified staged candidate read of section 10a and changes no write operation, no payload and no code of revision 6**; revision 5 at commit a20df7af143bec0f7514f3666e133e7c1348f858 sha256 914bed36fa7bc43fef0571d91e90a4edc78e962b5c955c9080c88c5d6af397c7 and revision 4 at commit e2832346fa885cb3390d0db6c187a1de1b772edd stay immutable history), toward M-003 (sections 1 to 11 and 13 to 15): the `compaction_work` payload, handler, reads and event class. Revision 5 changes only observable apply behavior: the removal gate now also guards the destructive source removal inside a Move (section 9), and the current-intent eligibility rule ignores directory, ignored backup, lock and temp effects (section 9). It builds on the revision 4 removal gate, identity preserving Move resume and honest counter-gap statement. **Revision 6 adds two refusals that publish nothing and leave the payload, the Rust surface and every code unchanged:** the held barrier of section 9 (an earlier held intent on a path the proposal needs must be committed by explicit `git_recovery Retry` before any apply attempt writes, because a later successful rewrite would strand it) and the no-effect Replace refusal of section 6 item 6 (a Replace whose candidate equals the current document). Revision 5 allowed a resumed apply to adopt held effects and then rewrite the record, which the real persistence engine makes unrecoverable; that behavior is superseded and must not be reintroduced.
- `cp-inventory` revision 1, toward M-001 (section 7): the nested `compactions/` inventory that M-001's single knowledge allocation observation composes. Its grammar is unchanged by revision 4 and its immutable pin stays at commit 256f77c8b9013d66e77d40213d1d5357810e261b (sha256 e24fd79d2e9fd0198d791683309d36e2a98ee2d1b5ee06989a8c4444e4bd89a6); no new `cp-inventory` revision is needed.

It supersedes revision 3 (commit 256f77c8b9013d66e77d40213d1d5357810e261b), which stays immutable history, and the older revision 1 (commit e1a7559bb1a801a3eaceb601b3db4d9e37765b54). Section 12 binds M-005 to the actual provider artifacts below and is not canonical for them. A provider change that touches a signature used here raises the provider revision and needs fresh agreement from M-005. A change to this file that a consumer relies on raises these revisions.

Pinned provider artifacts (committed, planning proposals, not implementations). A consumer reference is exactly the commit, path and sha256 shown:

| Boundary | Provider | Entry revision | Reference |
|---|---|---|---|
| `kr-history` | M-001 | 5 | `commit 485137330f11a9ab05088801309f66c2c6e17d5b path docs/contracts/knowledge-records.md sha256 49833aaa21fe571683670fc78ba9150af49aacc5adb7384438780e0a07268841` |
| `documents-and-references` | M-002 | 2 | `commit ea3a17fd4169f4c254ecd1f082a85c5f51d801da path docs/contracts/markdown-references.md sha256 e9808e8336df412a3219c8858435977e0ef966c19db6b7479609119b2f99f94c` |
| `persist-store-m005` | M-004 | 3 | `commit 70f2867f73b08631035d2b7da48bea58d8521dcc path docs/contracts/publication-git.md sha256 3fe6dace61c118cab2a2b844ad4a409f7b13807faacde2a1bdb8a955c25e0bf0` (the complete-operation oracle refinement; revision 2 at commit 2193a35 is superseded for this boundary) |
| `producer-host` | M-003 | 1 | `commit 38a555371bd9b2286e25203d0d28ea1ab6ea3427 path docs/contracts/registered-tool-surface.md sha256 d6c7a12ad8482830a1b99e589dbf5762de1391b3b853ae3ca88af7cbbae8142f` |

Consumption entries are confirmed only against the provider's actual current tracker entry. The artifact heading revision of a provider file is not its tracker revision, and no artifact records its own commit.

## 1. Purpose and limits

An external trusted actor proposes a bounded compaction of Markdown documents. The MCP validates and stores the proposal, requires an independent review, then guards and applies it. The MCP never launches a model, never deletes anything on its own initiative, never infers requirements from source code and cannot judge meaning. It guarantees structure: every original section is accounted for, every claimed preservation has a target, every incoming reference is known and resolved, every original is recoverable from committed Git bytes, every prior proposal revision and approval is retained, and every effect is attributable to this proposal. A reviewer owns the semantic judgement.

Sources are `README.md` and `docs/**/*.md` that M-002 observes as `Managed` or `Unmanaged`. `Retired`, `Drifted`, `MissingBody`, `Conflict`, `Unsupported` and absent documents, typed record files, work records, DOC metadata and anything outside the managed namespace are refused as sources. Typed bodies are never compacted; typed knowledge changes only through supersession. `compactions/` is never part of any corpus or reference scan.

Persistence policy (owner decision, M-004 section 0 and 5): the production policy commits after every successful actual mutation. Every successful `propose`, `revise`, `review`, `recover_reviewer`, `apply` and `withdraw` that publishes is committed by the dispatcher's settlement, not by M-005. Failed or partial calls, true no-ops and reads never trigger a commit; a Git failure keeps the saved bytes and reports a deferred or unknown receipt. M-005 neither runs Git nor claims a commit. `DeferAll` exists only inside M-004 tests.

## 2. Tool, storage layout and ownership

One purpose tool `compaction_work`, registered only by M-003. Operations: `propose`, `revise`, `review`, `recover_reviewer`, `apply`, `withdraw`. Reads use `get_context ref=CP-001` (summary, `view=tasks` for actions and sections, `view=review`, `view=references`) and bounded summaries in project context. There is no preserve operation: originals normally become committed through the production policy; the explicit `git_recovery` Preserve action (M-004 sections 6 and 11) is the exception for originals that Git rejected or that were never tracked.

Tracked portable owned paths, never under `.agent-tasks/` and never ignored:

- `compactions/CP-001.yaml`: the record.
- `compactions/CP-001/r1/A-01.md`: staged candidate bytes of action A-01 in revision 1. Only Create and Replace actions have a blob. Revision directories are never reused or overwritten.

The closed names are `CP-NNN.yaml`, `CP-NNN` directories, `rN` revision directories (N from 1 to 8 with no leading zero) and `A-NN.md` blobs (NN from 01 to 32). Anything else is foreign and makes the inventory incomplete (section 7).

File ownership (new files only, no shared edit): `src/compaction.rs` and its private submodules are the domain and import nothing from `tools`; `src/tools/compaction_ops.rs` is the producer. M-003 owns `src/main.rs` declarations, registration, `input.rs`, `Common`, `work::Ack`, the dispatcher lock, settlement and templates. M-005 produces no catalog entry, no template, no `schema()` or decode function and no `Config` resolution.

## 3. Capacities (all exercised by tests)

| Item | Limit |
|---|---|
| sources per proposal | 16 |
| actions | 32 |
| section ledger entries | 256 |
| preservation items | 64 |
| revisions per CP | 8 |
| reviews | 16; findings 64 in total |
| finding text | 256 bytes (`model::Finding`); reason and summary texts 512 bytes |
| path | M-002 `PATH_CAP` 160 bytes, managed grammar only |
| title 128 bytes, statement 256 bytes, purpose 240 bytes (M-002 `PURPOSE_CAP`) | request_key 8 to 64 characters of `A-Za-z0-9._-` |
| one staged blob | M-002 `BODY_CAP` 524288 bytes; all blobs of one revision 2 MiB |
| non-terminal CPs per project | 32; total CPs 256 |
| `Ack.refs` / `Ack.notes` | 16 refs of at most 256 bytes; 8 notes of at most 200 bytes |

Record capacity is checked by serialization before every publication. A nonterminal write must fit `RECORD_CAP - CLOSING_RESERVE` (480 KiB); terminal writes (apply completion, withdraw, abandon) may use the reserve up to `RECORD_CAP`. Every revision is retained in full inside the record, so eight maximal revisions need not fit: a revision that would overflow is refused `capacity` before any effect and the published record bytes are untouched. Nothing is evicted in this revision.

## 4. Record model (Rust, closed serde, `deny_unknown_fields`)

Reused types: `model::Verdict`, `model::Finding` (text 256 bytes, `must_fix`), `model::FindingResolution`. `input::RecoveryStage` is a payload type used only in `tools::compaction_ops`; the domain stores plain data.

```rust
pub struct CpRecord {
    pub schema_version: u32,              // 1
    pub id: String,                       // CP-001, from knowledge::reserve, never recycled
    pub request_key: String,              // caller idempotency key for propose
    pub state: CpState,
    pub current: u32,                     // current revision number, 1..=8
    pub revisions: Vec<RevisionRecord>,   // append only, full bodies of every revision, oldest first
    pub reviewer: Option<ReviewerBinding>,
    pub reviews: Vec<ReviewRecord>,       // append only, immutable
    pub accepted_review: Option<usize>,   // pointer into reviews, force only while that review's hash equals the current hash
    pub apply: Option<ApplyProgress>,
    pub writes: u32,                      // record publication counter, part of record operation ids
    pub created_at: String, pub updated_at: String, // generated, never supplied
}
pub enum CpState { Proposed, ChangesRequested, Accepted, Applying, Blocked, Applied, Withdrawn, AbandonedPartial }
pub struct RevisionRecord {
    pub revision: u32, pub content_hash: String, pub author: String, pub at: String,
    pub title: String,
    pub body: ProposalBody,               // full ledgers, preservation statements, staged hashes of that revision
    pub staged_dir: String,               // compactions/CP-001/r1
}
pub struct ProposalBody { pub sources: Vec<SourceObs>, pub actions: Vec<Action>, pub sections: Vec<SectionEntry>, pub preservation: Vec<Preservation> }
pub struct SourceObs { pub path: String, pub doc_id: Option<String>, pub version: String, pub sha256: String, pub len: u64, pub managed: bool, pub record_path: Option<String>,
                       pub record_sha256: Option<String>,     // Observation.record_sha256 at proposal; None for an Unmanaged document
                       pub move_basis: Option<documents::MoveBasis> } // {version, body_sha256, record_sha256, id}, captured once at propose for every Move source
pub enum ActionKind { Create, Replace, Move, Remove }
pub struct Action {
    pub id: String,                       // A-01..
    pub kind: ActionKind,
    pub path: String,                     // target; for Move the destination
    pub from: Option<String>,             // Move source
    pub base_version: Option<String>,     // documents::Observation.version; for Create and Move the absent destination's version
    pub purpose: Option<String>,          // required when M-002 will create a record (new path, or Replace of an Unmanaged document)
    pub staged_sha256: Option<String>, pub staged_len: Option<u64>,
    pub reason: String,
    pub absorbed_into: Vec<TargetRef>,
    pub state: ActionState,               // Pending | Applied | Blocked
    pub publications: Vec<store::Publication>, // M-004 events verbatim; the journal and trailers stay authoritative
}
pub enum SectionAddr { Preamble, Heading { ordinal: usize, level: u8, occurrence: usize, text_sha256: String } }
pub struct TargetRef { pub path: String, pub section: Option<SectionAddr> }
pub enum Disposition { Kept { action: String }, Moved { action: String }, Merged { target: TargetRef }, Dropped { kind: DropKind, reason: String } }
pub struct SectionEntry { pub id: String, pub path: String, pub section: SectionAddr, pub sha256: String, pub disposition: Disposition }
pub struct Preservation { pub id: String, pub kind: PreserveKind, pub source_section: String, pub statement: String, pub target: TargetRef, pub mode: PreserveMode }
pub struct ReviewRecord { pub revision: u32, pub content_hash: String, pub reviewer: String, pub verdict: model::Verdict, pub summary: String, pub items_hash: Option<String>, pub findings: Vec<model::Finding>, pub resolved: Vec<model::FindingResolution>, pub acceptance: Option<Acceptance>, pub at: String }
pub struct Acceptance { pub sources_digest: String, pub incoming_digest: String, pub overlay_digest: String }
pub struct ReviewerBinding { pub agent_id: String, pub pinned_at: String, pub predecessors: Vec<LostReport>, pub immersion: Option<Immersion> }
pub struct ApplyProgress { pub attempts: u32, pub phase: ApplyPhase, pub originals: Vec<String> /* encoded persist::Locator, at most 256 bytes each */, pub blocked: Option<BlockedInfo> }
```

The preamble is `SectionAddr::Preamble` and is never encoded as a heading with ordinal 0; heading ordinals are zero-based over real headings of the exact bytes (M-002 outline). Conversion to M-002: `Preamble` is `Selector::Preamble`, `Heading` is `Selector::Ordinal(ordinal)`.

Prior approvals are immutable history: an `Acceptance` lives inside its `ReviewRecord`, which is never rewritten or removed. `accepted_review` is only a pointer; revise clears it, and clearing it never edits the review. Dropping a prior revision, review or finding is a control mutation that must fail (C7).

## 5. Revision, content hash and retention

`content_hash` is sha256 over a versioned length-prefixed encoding (`agent-tasks/cp/v3`) of the title, sources (path, version, sha256), actions in order (kind, path, from, base_version, purpose, staged sha256, reason, absorbed_into), section entries and preservation items. State, reviews, authors and times are excluded. `revise` with an identical hash is UNCHANGED (no publication, no commit). Otherwise it appends a `RevisionRecord` (the previous full body stays byte for byte as stored), increments `current`, sets `Proposed` and clears `accepted_review`. A revise that would overflow the nonterminal cap is refused `capacity` before any effect. Staged blobs of earlier revisions stay in their own directories. No destructive revision exists: nothing in a prior revision, review, finding or approval is lost at any point, and retained history is not conditional on Git.

## 6. Mechanical validation (propose and revise)

1. `request_key`: the same key and same content hash returns the existing CP unchanged; the same key with different content refuses `request_key_conflict`. Lookup scans the CP inventory and records.
2. Each source: `documents::observe` must return `Managed` or `Unmanaged`. The caller-supplied `version` must equal `Observation.version` (body and record together), else `stale_source`. Any other state, unsupported reason, typed record path or unsafe path refuses `source_refused`. A Managed source stores its `documents/DOC-n.yaml` path and `Observation.record_sha256`. For every Move source M-005 captures `Observation::move_basis()` (M-002's `MoveBasis { version, body_sha256, record_sha256, id }`) from this original observation and stores it in the frozen `SourceObs` (and mirrors it in the Move action). The basis is never recomputed from a partial or new state: a resume passes the stored value, never one rebuilt from a half-moved tree.
3. The section ledger must equal `markdown::outline` of every source exactly (preamble plus every heading): a missing section is `section_unaccounted`, an unknown one `section_unknown`. Duplicate headings use ordinal plus occurrence. A source whose outline is incomplete or has `setext_candidates > 0` is refused `source_refused` because its addresses cannot be proven.
4. `Kept` and `Moved` are checked mechanically: the exact source section bytes appear in the candidate (or moved document). `Merged` targets must resolve to a section of a candidate or of a retained document. `Dropped` needs kind and reason; a section that is the source of a preservation item cannot be dropped.
5. Preservation: the source section exists; the target resolves; `Verbatim` requires the exact source section bytes contiguous in the target section; `Rewritten` is accepted only by reviewer verdict. A target is never reported resolved when `references::resolve` answers `Unknown`, `Unsupported` or `MissingSection`; unknown is unproven.
6. Action shape: Create path absent; Replace and Remove require a source entry and `base_version` equal to the current observation; Move needs a source and an absent destination; a Replace whose candidate bytes equal the current document bytes refuses `invalid_arguments` naming `actions.content` (it changes nothing, so it has no effect to attest and an equal-bytes look-alike could only ever read `UnattestedEqualBytes`); every non-dropped Remove names `absorbed_into`; no two actions touch one path; candidates are valid UTF-8, without NUL, within `BODY_CAP`; `purpose` is present when M-002 will create a record.
7. Path claims: any touched path held by another non-terminal CP refuses `path_claimed`.
8. Incoming coverage (section 8) is computed. A proposal containing any Replace, Move or Remove must have complete coverage at propose, revise, review and apply, always evaluated for the actions of the current revision. A create-only proposal needs none, but that never carries over: any revise changes the hash, clears acceptance and re-evaluates the new action set, and apply recomputes from the current actions, so a create-only acceptance can never authorize a destructive step.
9. Capacities and total blob size.

Publication order for propose, all under the dispatcher's single lock: `knowledge::reserve(store, guard, Prefix::Compaction, expected, effects)` (a failure afterwards leaves a visible gap, never a recycled number), `ensure_parents`, staged blobs, the record last. Every M-005 publication uses `Attest::Required` and a deterministic `store::OperationId`: `cp:CP-001:r1:stage:A-01` for blobs and `cp:CP-001:r1:rec:<writes+1>` for record writes (`writes` is stored in the record, so ids never depend on an attempt count and never repeat). If the journal cannot retain the entry, that `Attest::Required` write refuses `attestation_unavailable` before its own effect; a repository is required for compaction. The claim is not made for the whole propose: the allocator reservation is an ordinary `Attest::Optional` publication of M-001 that never stops for a full or unwritable journal, and it is published first, so a propose refused at a later Required step can already have published a counter reservation. That is a visible, retained gap (never recycled) listed in the ledger and `store.publications()`, and since the call returns an error with a publication it settles as `Partial` and is held, not committed. An existing blob at a staged path is never overwritten and never silently adopted. Equal bytes prove only content integrity: `effect_status` for that exact operation id must answer `Attested`; `Unknown` or any other answer refuses `stage_unowned`, and the recovery is a fresh `request_key`, which reserves a new CP number while the orphan stays visible and counted. Orphan CP directories are never reused and never deleted.

## 7. `cp-inventory` revision 1: the nested compactions inventory (provider M-005, consumer M-001)

M-001's `observe_allocation` composes six homes; five use `Store::kind_inventory`, the sixth calls exactly this function, which returns the store's existing inventory type and never recurses back into the allocation observer:

```rust
// src/compaction.rs (domain, no tools imports)
pub fn inventory(store: &Store) -> store::Result<store::Inventory>;   // Inventory { ids, warnings, complete }
```

Grammar, using only `Store::list_dir` (never `fs::read_dir`), `store::own_temp_name` and the pure `knowledge::parse_id`:

1. `compactions/` absent is complete and empty. A file or link at that path is `file_type`.
2. List `compactions` with cap `MODULE_CAP` (512). An entry named `CP-<canonical digits>.yaml` that is a regular file counts its identifier. A directory named `CP-<canonical digits>` counts the same identifier, even without a record (an interrupted propose), and adds the warning `CP-007: staged directory without a record`. A record and a directory of one number count once. Canonical digits are `{n:03}` or more with no other leading zero, parsed by `knowledge::parse_id` with `Prefix::Compaction` and re-rendered equal to the name.
3. Own publication leftovers (`own_temp_name(name)` true and its stem `CP-NNN.yaml`) are warnings and do not make the inventory incomplete. Every other entry (other names, noncanonical numbers, symlinks, nonregular files, a regular file named like a directory home) is named in a warning and sets `complete = false`. Reaching the cap sets `complete = false`.
4. For every `CP-NNN` directory, list it with cap 512: allowed children are directories `rN` with N in 1..=8, canonical decimal; anything else is named and incomplete (a ninth revision directory, `x.txt`, a file, a link).
5. For every `rN`, list it with cap 512: allowed children are regular files `A-NN.md` with NN in 01..=32 and own temp leftovers of such names (warnings). Anything else, including any subdirectory (depth 3), is named and incomplete.
6. The returned ids are sorted numerically and unique. The function reads names only: it never reads or decodes records, never writes and never creates the home.
7. Cost is bounded by 256 CPs, so at most 1 + 256 + 256 x 8 directory listings; no recursion beyond depth 2.

`compaction::scan(store) -> store::Result<CpScan>` additionally decodes records for reads and summaries: `CpScan { records: Vec<store::Snapshot<CpRecord>>, orphans: Vec<String>, unreadable: Vec<String>, warnings: Vec<String>, complete: bool, version: String }`. A corrupt, oversized, symlinked or misnamed record becomes a named `unreadable` row, never a silent drop. M-001's observation uses only `inventory`.

Controls (M-001 section 13 "Nested recognition", plus orphans): P `CP-001.yaml` with `CP-001/r1/A-01.md` is complete and counts number 1 once; an orphan `CP-002/` without a record counts 2 with its warning; own temp leftovers are warnings. N `CP-001/x.txt`, depth 3 (`r1/sub/`), `r9`, a symlink, a non-canonical `CP-0001.yaml` and 513 entries each set `complete = false` and name the entry. M: unknown nested names ignored; recognized names reported foreign; orphan directory ignored; inventory calls `allocation_version`.

## 8. Review, incoming coverage and acceptance digests

**Review.** The reviewer binding is created by the first `review` from an actor that is not an author of any revision; thereafter that agent is the only accepted reviewer through every correction of every revision. Authors and reviewer stay disjoint both ways: a reviewer calling `revise` refuses `reviewer_cannot_author`, an author calling `review` refuses `self_review`. Declared ids are observations, not authentication. `review {cp, revision, content_hash, verdict, summary, verified_items, findings, resolved_findings}` must match the current revision and hash, else `stale_revision`. Items are all `P-` ids, all action ids and every non-`Kept` section id. `Accepted` requires `verified_items` to equal that set exactly (else `review_incomplete`), no `must_fix` finding in the same review, no unresolved `must_fix` finding from any earlier review of any revision, and a summary. `ChangesRequested` requires a finding. Findings are immutable and zero-based per review; follow-up names resolutions by `review_index` and `finding_index`.

Replacement is not a convenience switch. It happens only after observed unrecoverable loss: `recover_reviewer stage=lost` (requires `lost=true`, `unrecoverable=true` and an observation of inability to continue or resume; a timeout or preference never qualifies) then `stage=immersed` by the new actor (not an author, not a predecessor) with understanding, sources, unfinished and gaps; any gap blocks review. Predecessors, reviews and findings remain; an acceptance already bound to the current hash stays valid. `RecoverReviewer` mutates CP review metadata only and is event class `CompactionReview`; it is never `EventClass::Recovery`, which is reserved for explicit Git recovery.

**Incoming coverage.** For every source path and every Move destination M-005 calls `references::incoming(store, &Target::Doc { path, fragment: None })` once per target; row fragments list what each referrer names. Coverage must satisfy `coverage.complete`. Any gap (`Coverage.gaps`: a corrupt or unknown-schema work or typed record, non-UTF-8 file, capped inventory or byte budget, unrecognized entry, unparsed construct) refuses every Replace, Move or Remove with `coverage_incomplete`, and the refusal names the gaps; unknown is never zero. The coverage version is not an input to any decision.

Every row must end in one of: still valid in the post state, rewritten by a Replace action on its referrer (Markdown destinations through `references::rewrite`), or removed with reason on that referrer. Rows whose source kind is `Work`, `Knowledge`, `DocMetadata` (of another document) or `Project` cannot be rewritten by compaction: they are acceptable only when the target path stays and every named fragment still exists in the candidate outline; if the target is moved or removed, or loses a referenced fragment, apply refuses `incoming_unresolved`. `README.md` is a managed path: a README referrer is rewritten with a Replace action through `documents::save` like any other and is not blanket refused (an unmanaged README gains a DOC record in that save and the action needs `purpose`).

**Post state.** `references::integrity(store, &Overlay { put, remove, moves })` is built from the proposal: Create and Replace as `put`, Remove as `remove`, Move as `moves` (the file and its active record move; the destination must be absent or in `put`). The result must have `coverage.complete` and `introduced` empty. M-002's integrity uses the live DOC records, so a moved or retired DOC identifier is not dangling and a record never conflicts with its own move.

**Acceptance digests.** Three digests are stored inside the `Acceptance` of the accepting review and describe the FULL immutable accepted proposal. They are stable by construction, so own progress cannot cause a stale loop, while real external changes are never ignored:

- `sources_digest`: sha256 over `(path, Observation.version, sha256)` for every source, every base document and every destination observation. `Observation.version` covers body and DOC record together, so an external edit of body or metadata changes it.
- `incoming_digest`: sha256 over the sorted incoming rows (target, source kind, source id or path, via, count, fragments) excluding only rows whose source is a document that has an action in this CP and the DocMetadata rows of those documents. It excludes `Coverage.version`, bytes and file counts, DOC record revisions and dates, and `compactions/` state (which is not scanned). Own progress touches only documents that are excluded, so it cannot change this digest.
- `overlay_digest`: sha256 over the full overlay (paths and candidate sha256, moves).

Where they are compared: `sources_digest` and `overlay_digest` are compared in full at review time and at the first apply attempt before any effect (state `Accepted`). After the first effect they are history and are never recompared with current state, because own progress necessarily changes sources and the pending set shrinks; the full accepted digests are never overwritten. `incoming_digest` is recomputed and compared in full on every apply attempt, resume included, so a new external reference or a changed untouched referrer is never ignored (`incoming_changed`). Resume uses a separate execution overlay and per-action checks (section 9).

## 9. Apply and recovery

`apply` runs inside the dispatcher's single lock scope (`execute_locked`, no relock, no Git) and resumes after any interruption by the same call.

**First apply (state `Accepted`, no effect yet).** Preflight, all required before any effect: the accepted review hash equals the current hash; the full `sources_digest`, `incoming_digest` and `overlay_digest` are fresh; coverage complete and integrity clean; no other non-terminal CP claims a touched path; `persist::pending` shows no `Unknown` or `Drifted` intent on a touched path; and for every document a Replace, Move or Remove changes, its body and, when it exists, its `documents/DOC-n.yaml` record are proven by `persist::verify_committed` over `Item { relative, sha256, len, at: None }` of the observed bytes. The locators are stored in `apply.originals` together with the frozen original identity they prove (path, sha256, len, matched to the frozen `SourceObs` and record). `NotCommitted` (untracked, ignored, dirty, unborn or detached HEAD, no repository), `Mismatch`, `Unknown` or changed bytes refuse `originals_not_committed`. An ignored backup, a deferred or pending intent, or a Git failure is never proof. Normal saves are committed by the production policy; originals that Git rejected or that were never tracked need the explicit `git_recovery` Preserve (at most 32 paths per call; repeat for larger sets), and the reply names that route. Preflight changes nothing.

**Held barrier (revision 6; every attempt, first or resumed, before any effect).** M-005 reads `persist::pending` and `persist::intent_view`. If an intent in phase `held` (left by an earlier call that returned an error) holds any path this proposal needs, apply refuses `replacements_not_committed` naming the intents and `git_recovery Retry`, and publishes nothing, so nothing settles and the record is untouched. The needed paths are the touched paths of the current revision (documents, move sources), the DOC record paths of the sources, the proposal's own record `compactions/CP-nnn.yaml` and its staged blobs `compactions/CP-nnn/**`. Why: Retry (M-004 section 5) commits an intent only while every path of it has an unbroken chain of images ending at the current bytes. A later successful call that rewrites the proposal record or a document commits without the held intent and leaves it drifted, so Retry then fails `recovery_blocked` and only Release or Preserve remain. The explicit Retry commits the interrupted call's partial images as they are; the next apply then resumes through the oracle as below. Intents on other paths never block, and neither do intents that are only `published` (a successful call whose commit was deferred: the next success commits them in the same chain). `Unknown` and `Drifted` intents on touched paths still refuse `effect_unknown`. The same-call whole-intent eligibility of the removal gate is unchanged. Evidence: composed controls `a_later_committed_record_write_strands_an_older_held_intent`, `body_first_partial_resumes_through_the_real_oracle`, `a_held_record_only_intent_blocks_until_it_is_committed`, `an_unrelated_held_intent_neither_blocks_nor_is_stranded`.

**Resume (state `Applying` or `Blocked`).** The same call resumes after any interruption, and it is a different check set so a partial run can never fail its own preflight:

1. Originals are not re-proven from current bytes. The stored locators are matched to the frozen original identities and each is re-verified with `persist::verify_committed` using `at = locator.commit` (still reachable, bytes equal). A partly published, uncommitted new body or record is never required to be committed as an original. A stored locator that no longer verifies refuses `originals_not_committed`.
2. The oracle comes first. Every action not recorded `Applied` is classified by `persist::effect_status` before any original-version check. Attested, partial or unknown actions have their known after-images and metadata verified through the oracle, never against `base_version`, because a body-first partial would otherwise always fail that check.
3. Only actions the oracle reports `NotPublished` are checked against their original preimages: current `Observation.version` equal to `base_version` and the source bytes equal to the frozen source sha256, else `Blocked` `stale_source`.
4. Every action already recorded `Applied` has its current post-image re-verified (the table below) on each unfinished resume. Drift there is not attention: it blocks the CP with `post_apply_drift`, and no further effect, above all no Remove, runs while any applied action has drifted.
5. The execution overlay holds only actions not yet adopted. `incoming` and `integrity` are recomputed against the actual current state plus that overlay; `incoming_digest` is compared in full. A new external reference or changed source can never be ignored.

Order: Create, Move, Replace (rewrites), a fresh `incoming` and `integrity` verification of the real state (complete, nothing introduced dangling, no row still pointing at a removal target), then Remove. Roll forward only; Git holds the originals.

**Removal gate: replacements must be committed or in the same whole intent.** Under the production policy a successful call commits its whole current intent, all paths or none, and an earlier call that returned an error left its intents `Held`, which a later success never commits (M-004 section 5). A resumed apply whose earlier Create, Replace or Move effects were published by such an interrupted call would otherwise commit only the removals, and a clone would then lose the originals without the replacements. Therefore, before the first removal effect of any kind, every Create, Replace and Move effect of this CP must satisfy exactly one of. A removal effect is a Remove action and also the destructive source removal that happens inside a Move (M-002's relocate order is destination body, record, then source removal), which runs in the Move phase before the Remove phase; the gate therefore runs before M-005 calls `relocate` whenever that call would perform the source removal:

- (a) it is already `Committed` in an older intent: the oracle row for the body and for its DOC record carries `EffectGit::Committed { commit }`, and `persist::committed_original` (or `verify_committed`) with `Item { relative, sha256, len, at: Some(commit) }` proves the final bytes present in that reachable commit; a row that was superseded into later bytes is judged by its final committed image, never by the older digest; or
- (b) it was published by this same handler call, so it is a member of the current call's whole intent and the dispatcher's settlement commits it atomically with the removals (all paths or none). M-005 decides this itself: per-action operation ids differ (`cp:<CP-id>:r<revision>:<action-id>`), so it does not delegate the cross-action barrier to M-004's same-operation rule and needs no coordinator. The gate checks every Create, Replace and Move action of the CP, including ones executed earlier in the same call, against `Store::publications()` of the current Store: each effect of such an action must appear as an event of this call with an `intent` and `Tracking::Tracked`, and the whole current intent must be eligible, meaning every eligible owned-file event of the call, replacement and removal alike, is tracked, durable (no `SyncUnknown`, no `UnknownAfterPublication`) and none is untracked. If any eligible event of the current call is untracked or sync-uncertain, settlement would hold the call as `Partial`, so (b) is not satisfied and the gate falls back to (a) for those actions or blocks.

**Eligible owned files only.** Directory, ignored and housekeeping effects never receive a file journal intent and are not owned files: `DirectoryCreated`, `DirectoryExisting`, ignored normalization backups under `.agent-tasks/backups/`, lock files and publication temps. They carry M-004's explicit `Tracking::NotApplicable` (or the one equivalent typed state M-004 publishes; a fake `Tracked` is never used) and are excluded from the tracked and untracked counts, from M-004's call-intent receipt rules, from atomic commit-unit and path selection, and from this gate's eligibility. They remain truthful filesystem effects: a parent sync that failed on a necessary directory is still never reported `Durable` and still makes the call sync-uncertain. The recovery proof (`committed_original`, locators) covers the business body, DOC metadata and counter files, never empty directories. Consequently the first propose that creates the `compactions/` home and its revision directories, and a legacy record normalization that writes an ignored backup, still commit their owned files, and only a real non-ignored untracked sibling file blocks the whole current call. A fresh call is never deadlocked by this rule: a plain same-call Create, Replace, Move and Remove stays one intent and is allowed.

If any earlier replacement effect is `Held`, `Pending`, `Unknown`, deferred, untracked or otherwise not committed, apply blocks before any removal with `replacements_not_committed`. The error names the pending intents from `persist::pending` and the explicit exact `git_recovery Retry` for them, and no removal is attempted. After an exact Retry proves those effects committed, a resumed apply continues and satisfies (a). M-005 does not require the current call's own effects to be `Committed` inside the handler, because they settle only after it returns; the whole current intent is atomic under M-004's rule. A fresh apply that performs Create, Replace, Move and Remove in one call needs no Retry and ends in one commit. This uses only the typed M-004 data (`EffectGit`, `PendingRef`, `Proof`, `Locator`) and no new transaction engine, rollback or journal.

**Move source removal.** For a Move, before any call that can remove the source body (a fresh `relocate` or a resumed W1, W2 or unmanaged resume), M-005 requires the created destination body and its replaced DOC metadata (the record moved to the destination, same source DOC id; none for an unmanaged document) to be either (a) verified `Committed` from an older intent, or (b) among the new semantic files of the current atomic whole intent. A destination body published by an interrupted earlier call is `Held`: resume does not call `relocate`, does not move the record and does not remove the source. It blocks with `replacements_not_committed`, names the pending intent and the exact `git_recovery Retry` for it, and leaves the source untouched. Restoring permissions or clearing the fault alone cannot finish the move: the old destination must first be committed, because otherwise the resumed call would commit the record move and source removal while the destination body stays held and a clone would lose the document. After Retry proves the destination and its metadata committed, a resumed call finishes through M-002's identity bearing recovery with the stored `MoveBasis` and the same source DOC id. This needs no cross-operation M-004 engine and no coordinator; M-005 checks each action's own operation rows.

Per action the operation id is `cp:<CP-id>:r<revision>:<action-id>`, deterministic and never carrying an attempt nonce. Before the first document effect the record is published in state `Applying` (`Attest::Required`; if the journal cannot retain it, apply refuses before any document effect, and apply itself allocates no counter). Document effects use `documents::{save, relocate, remove}` with one `Scope` per action whose `operation` is that id; M-002 then publishes body and record with `Attest::Required`. M-005 writes no second effect journal: `store::Publication` copies in the action are convenience, never authority.

Recovery uses M-004's complete-operation oracle, `persist::effect_status(store, &op, &expected)`. The `expected` list asserts only the semantic body effects M-005 can compute exactly: Create asserts `Created(path, sha256)`; Replace asserts `Replaced(path, sha256)`; Move asserts `Created(destination, sha256)` and `Removed(source)`; Remove asserts `Removed(path)`. M-005 never lists, predicts or hashes generated effects (the DOC record with its revision and dates, created directories, and the allocator counter) and invents no serializer, plan transaction or wildcard. The oracle returns every attested same-operation effect with `asserted`, `complete`, `superseded_into`, `git` and `adopted` per effect. An unlisted attested effect is not foreign and is not certified by equal bytes either; it is attested only because M-004's journal or trailer recorded it for this operation.

`Attested` is not whole-step success, and completion is specific to the action kind. The receipt must have `complete = true`, and then the current post-image must be exactly this (`observe` is M-002's, rows are the oracle's):

| Action | Current post-image required for `Applied` |
|---|---|
| Create | `observe(path)` is `Managed`, DOC id bound to exactly this path, `body_sha256` equals the candidate, record revision 1; the record row for `documents/DOC-n.yaml` is `Created` with `after_sha256` equal to the current record bytes; the DOC id is valid in the six-home allocator |
| Replace of a `Managed` document | `Managed`, same DOC id, body equals the candidate, revision equals the base revision plus one; record row `Replaced` with `after_sha256` equal to the current record bytes |
| Replace of an `Unmanaged` document | `Managed` with a new DOC id, body equals the candidate, revision 1; record row `Created`, allocator valid |
| Move of a `Managed` document | destination `Managed` with the SOURCE's DOC id (the old identity follows the destination, it is not retired), record path equals the destination, body bytes equal the frozen source bytes, revision plus one, record row `Replaced`; the source path observes `Absent` |
| Move of an `Unmanaged` document | destination `Unmanaged` with the frozen source bytes; the source path `Absent`; no DOC record exists or was created |
| Remove of a `Managed` document | body absent and `observe(path)` is `Absent`; `observe(Ref::Id(id))` is `Retired` with the same id, the same last path and the original body hash; the record row is `Replaced` with `after_sha256` equal to the current record bytes |
| Remove of an `Unmanaged` document | body absent and `observe(path)` is `Absent`; no DOC record claims the path and no synthetic record exists |

Every row of the table requires the current record bytes to hash to the attested record `after_sha256` and that row not to be superseded: metadata changed by anyone after the actual effect blocks with `metadata_mismatch`. The same body without its required metadata is never `Applied`, and a Remove is never required to find a `Managed` document.

Later changes are not accepted because another MCP operation attested them or because a row carries `superseded_into = Some`. Each action touches its path once, so any later body or metadata change on a path of this proposal lies outside the reviewed proposal. The historical effect stays attested and its `store::Publication` copies stay in the record, but the current business post-state is `Blocked` with `post_apply_drift` (or `stale` before the action started); the desired bytes are not claimed, and nothing is overwritten or deleted to restore them.

Missing generated metadata keeps the step `Partial`. M-005 then rolls forward only through real M-002 operations whose precondition is that the oracle attested the body for this operation: `documents::adopt` only for an attested body without a record of a Create or of a Replace of an `Unmanaged` document, where the new record is created by design (never for an unattested body and never for a Move destination), **and, since revision 8, for the interrupted Replace of a `Managed` document whose body was published but whose DOC record was not updated, under all of these constraints: (a) the oracle attested the body row for this action's own deterministic operation identity, so equal bytes, a native edit or a body of another operation is never ownership; (b) the document is observed with the frozen SOURCE's DOC identifier and a record revision still equal to the frozen base revision, so no foreign record edit is adopted; (c) the verified post image of the Replace of a Managed document (same DOC id, revision equal to the base plus one, recorded body hash equal to the candidate's hash) holds after the adopt, otherwise the action blocks with `metadata_mismatch`; the adopt never reserves a new identifier for it, never loosens the native foreign state check and never applies to a Move, a Remove or an unattested body**, and repeat `remove` for a missing body. A Move resumes only through M-002's identity bearing `relocate`, called in a fresh scope with the same operation id and the stored `MoveBasis` captured at propose (never a basis rebuilt from the partial new state). M-002's corrected relocate requires that basis and recovers three states: W1, the destination body is attested and an earlier record move did not happen, so it skips the attested create of the destination body, moves the exact source DOC record and removes the source body; W2, the record already moved, so it removes the unclaimed source body without retiring anything; W3, nothing left to do, a no-op. M-005 asks the oracle to validate the allowed rows and digests for these states (the destination body create asserted, the record row and source removal as returned unasserted or asserted rows) and consumes the actual old DOC identifier: the destination record must carry the SOURCE's DOC id and `documents::observe` of the destination must return that id. M-005 never deletes and recreates the destination, never calls `adopt` on a destination body (that would allocate a new DOC id), and a destination with a different or new DOC id blocks with `metadata_mismatch`. Until M-002's recovery step is published with that behavior, a partial Move stays `Blocked` rather than guessed. Unknown, conflicting, truncated or `complete = false` history always blocks.

| `EffectStatus` | Action |
|---|---|
| `Attested` and the post-image row above passes | adopt, record the `store::Publication` copies, mark Applied |
| `Attested` and the post-image fails or `complete = false` | stay `Partial`; roll forward only as described above, else `Blocked` `effect_incomplete`, or `post_apply_drift` / `metadata_mismatch` when current state changed |
| `Partial { missing }` | the asserted effects named missing were never published: roll forward through the M-002 recovery step only when the attested part matches the plan and current bytes agree; otherwise `Blocked` with the M-002 `partial_publication` error |
| `NotPublished` and the original preimage checks of resume item 3 pass | execute normally |
| `NotPublished` and the observation changed | `Blocked` `stale_source` |
| `Unknown` (any reason, including `UnattestedEqualBytes`, `CrashAfterRename`, `ConflictingRecord`, `HistoryScanIncomplete`, `JournalUnavailable`, `NoRepository`) | `Blocked` `effect_unknown`, returning the attested part for display; only M-004 explicit recovery (`Reconcile` or `Adopt` through `git_recovery`) can change it; equal bytes alone are never ownership |
| `Foreign` | `Blocked` `effect_foreign` |

Held predecessors (M-004 section 5): a failed or partial earlier call leaves its intents `Held`. A later successful call, including a fresh `propose` after a failed reservation, commits only its own current counter image and own paths; the older failed intent stays held and superseded on the shared counter path and is never labeled complete, never committed with its untouched partial body or metadata, and never replayed. M-005 therefore never treats a later success as certification of a held action, and a held partial body is never claimed complete.

A different CP with identical bytes has a different operation id and is never adopted. Once every action is `Applied` and the state is `Applied`, that is a historical fact. Repeating `apply` then returns UNCHANGED with the stored copies, publishes nothing, and may add an honest note of current drift observed at that read (`post_apply_drift` paths); it never states or implies that the current post-state is verified. `Applying` and `Blocked` freeze the proposal: `revise` refuses `apply_started`. `withdraw` is refused for `Applied`; with attested effects it needs `abandon_partial=true` and a reason, yields `AbandonedPartial` (an Ok result that publishes only the record) and releases claims.

**Outcome rule for the dispatcher.** `execute_locked` returns `Ok` only when the operation completed. For `apply` that means state `Applied`, or an UNCHANGED repeat. An apply that published anything and then blocks (stale, unknown, foreign, partial) returns `Err` with the specific stable code after publishing the `Blocked` record, so the dispatcher settles it as `Partial` and the production policy holds, not commits, the intent. Held intents are never certified by a later success. A resumed apply is refused by the held barrier until explicit `git_recovery Retry` has committed the held earlier intents that touch the proposal's paths, and only then adopts their effects through `effect_status`. A preflight refusal has no publication, so nothing settles. Reaching the removal gate with a held replacement returns `Err` `replacements_not_committed`; the call settles as `Partial` and commits no removal.

## 10. Rust surface provided to M-003 (canonical `compaction-operations` revision 4)

```rust
// src/tools/compaction_ops.rs  (producer; the only file in tools)
#[derive(Deserialize, JsonSchema)]
#[serde(tag = "op", rename_all = "snake_case", deny_unknown_fields)]
pub enum Compaction {
    Propose { request_key: String, title: String, sources: Vec<SourceIn>, actions: Vec<ActionIn>,
              sections: Vec<SectionIn>, preservation: Vec<PreservationIn> },
    Revise  { cp: String, title: String, sources: Vec<SourceIn>, actions: Vec<ActionIn>,
              sections: Vec<SectionIn>, preservation: Vec<PreservationIn> },
    Review  { cp: String, revision: u32, content_hash: String, verdict: model::Verdict, summary: String,
              verified_items: Vec<String>, findings: Vec<model::Finding>, resolved_findings: Vec<model::FindingResolution> },
    RecoverReviewer { cp: String, stage: input::RecoveryStage, lost: Option<bool>, unrecoverable: Option<bool>,
              observation: Option<String>, understanding: Option<String>, sources: Vec<String>,
              unfinished: Vec<String>, gaps: Vec<String> },
    Apply   { cp: String },
    Withdraw { cp: String, reason: String, abandon_partial: bool },
}
impl Compaction { pub fn event_class(&self) -> persist::EventClass; }
pub const DESCRIPTION: &str;
pub fn execute_locked(store: &Store, guard: &store::LockGuard, common: &input::Common, op: Compaction,
                      effects: &mut Vec<String>) -> store::Result<work::Ack>;

// src/compaction.rs  (domain: no tools imports; public fields for rendering)
pub fn read_cp(store: &Store, id: &str) -> store::Result<store::Snapshot<CpRecord>>;   // read only, no lock
pub fn summaries(store: &Store, limit: usize) -> store::Result<CpSummaries>;           // at most 20 rows plus total and omitted counts
pub fn inventory(store: &Store) -> store::Result<store::Inventory>;                    // section 7
pub fn scan(store: &Store) -> store::Result<CpScan>;
```

`SourceIn { path, version }`, `ActionIn { id, kind, path, from, base_version, content, purpose, reason, absorbed_into }`, `SectionIn`, `PreservationIn` carry the ledger fields of section 4; `content` is the candidate as UTF-8 text and the handler stages its exact bytes.

Rules:

- Decoding and schema belong to M-003: `input::mutation::<Compaction>(args, false)` and `input::mutation_schema::<Compaction>(false)`; `ref` is absent and `Common.reference` is `None`. M-005 defines no decoder, schema function, `Common`, template or tool definition.
- `event_class`: `Propose` is `CompactionPropose`, `Revise` is `CompactionRevise`, `Review` and `RecoverReviewer` are `CompactionReview`, `Apply` is `CompactionApply`, `Withdraw` is `CompactionWithdraw`. The dispatcher builds `Event` with `Ack.refs`, `operation = None` (per-action operation ids stay inside the handler) and settles after the handler, also on error.
- `common.actor` is required by every operation; a missing actor refuses naming the field through `input::field("actor", ...)`. All list, text, count and enum checks use `input::field`, so a refusal reads `<field>: <rule>` and never echoes a value. `common.version` is `knowledge::allocation_version(store)` for `propose` and the record `Store::version` of `compactions/CP-001.yaml` for every other operation; a stale version refuses `stale` before any effect.
- The handler never locks, never resolves an alias, never calls Git and never renders.
- `Ack`: `target` is the CP id; `version` is the record version after the call (the next write version); `phase` is the lowercase state (`proposed`, `changes_requested`, `accepted`, `applying`, `blocked`, `applied`, `withdrawn`, `abandoned_partial`); `phase_label` is derived by the shared `ack` builder from the `CP-` prefix; `changed` is whether any file was published; `notes` (at most 8 lines of 200 bytes) carry in order: applied and total action counts, the blocked kind and action, review state, coverage gaps and `post_apply_drift` paths (blocking while the apply is unfinished, an honest read-only note after `Applied`); `refs` (at most 16, each at most 256 bytes) are the CP id, affected DOC ids and exact affected paths, truncated never silently (an omitted count is a note). No Git line is ever added by M-005.
- `read_cp` and `scan` are read only, never repair or write, report partial coverage and orphans honestly; `get_context ref=CP-001` paging snapshots are M-003's `scope_version` over the record bytes. `summaries` rows give id, state, revision, reviewer set, applied over total actions and blocked kind; pending Git facts come from `persist::pending`.

## 10a. Retained revision selection and staged candidate read (revision 7, additive and read only)

Sections 2 and 5 retain every full revision body, every review and every staged candidate losslessly, but revision 6 exposes only the current revision. Revision 7 adds exactly two functions to `src/compaction/read.rs`, re-exported from `src/compaction.rs`. They import nothing from `tools`, take no lock, never write, never repair, never commit and never fall back to a live document or to the current revision. All record fields stay public; the typed `CpRecord` already carries every history row (revisions, reviews, findings, resolutions, reviewer binding, apply progress, stored originals). **M-003 owns row pagination, the snapshot token, the fixed page framing and the `get_context` views; M-005 owns only the selection and the one shared verifier.**

```rust
/// Retained revision by one based number.
pub fn retained_revision(record: &CpRecord, revision: u32) -> store::Result<&RevisionRecord>;

/// Exact verified staged candidate of one action of one retained revision.
pub struct StagedCandidate {
    pub revision: u32,
    pub action: String,   // canonical action identifier, for example A-01
    pub kind: ActionKind, // Create or Replace
    pub path: String,     // target document path of the action
    pub sha256: String,   // recorded digest, verified against the bytes
    pub bytes: Vec<u8>,   // exact staged bytes; their length equals the recorded staged_len
}
pub fn read_staged(store: &Store, record: &CpRecord, revision: u32, action: &str) -> store::Result<StagedCandidate>;
```

Semantics, exact:

1. `retained_revision` returns the entry whose one based `revision` equals the argument. Zero, or a number that is not retained, refuses `invalid_arguments` naming `revision`; the message states the retained range (`1 to <current>`). It reads nothing and never returns another revision.
2. `read_staged` selects the revision as above, then the action by exact identifier in that revision. An identifier that is not canonical, or is absent from that revision, refuses `invalid_arguments` naming `action`. The canonical grammar is exactly `A-` followed by two ASCII digits whose number is 01 to 32 (`record::parse_action_id`). Identifiers are unique within a revision but need not be dense or start at `A-01`: the cap of 32 actions is also the largest numeric identifier, so a consumer must never infer the identifier set from the action count.
3. A Move or Remove action has no staged candidate (`staged_sha256` is absent): `invalid_arguments` naming `action` and stating the kind.
4. It reads exactly `record::stage_path(id, revision, action)` (`compactions/CP-nnn/rN/A-NN.md`) through `Store::bytes`, bounded by `BLOB_CAP`. A missing file, a length that differs from `staged_len`, a sha256 that differs from `staged_sha256`, or an oversize file refuses `invalid_data` with the relative path in the message and returns no bytes. The messages are `<relative>: staged candidate is missing.` and `<relative>: staged candidate does not match its recorded hash.`
5. `ops::load_blobs` (apply and revise) calls the same private verifier for the current revision, a behavior preserving extraction: one path rule and one hash and length rule exist. The apply messages stay unchanged.
6. A missing proposal is the existing `cp_not_found` from `read_cp`, read by the caller; no function here reads the record. Revisions are append-only and staged directories are never reused (section 5), so one `(record version, revision, action)` always yields the same bytes.
7. No new error code, no `expected` or snapshot parameter and no history row type are defined; `stale` is M-003's scoped view rule. `read_cp`, `summaries`, `inventory`, `scan` and `cp-inventory` revision 1 are unchanged.

Nothing here changes `propose`, `revise`, `review`, `recover_reviewer`, `apply` or `withdraw`. The revision 6 held barrier with its explicit Retry first, the identical Replace refusal, current-intent eligibility, the removal gate over every destructive effect and the deterministic operation identities stay exactly as specified in sections 6 and 9.

Confirmation needed before implementation: M-003 (`registered-tool-surface`, the `get_context` CP history and content views) and M-006 (qualification of those views) confirm this seam against the published revision 7 pin, and root signs this text. No other module is affected.

## 11. Dispatcher alignment (M-003 owns; stated so both sides can test it)

Scope sequence for `compaction_work`: resolve the alias once, take the single write lock, call `execute_locked`, settle with `persist::settled(..., production_policy(), result)` while the guard is held, release, render. `Ok` with `changed = true` settles as `Success`; `Ok` with `changed = false` and no publication settles nothing; `Err` with typed publications settles as `Partial`; `Err` without publication settles nothing. M-005 therefore reports `changed = false` and publishes nothing for an UNCHANGED revise or repeated apply.

## 12. Bindings to actual provider functions (consumer requirements, no duplicate types)

M-005 uses these provider functions and types unchanged and defines no port, trait or struct family for them. Private adapters may translate (for example `Observation` to `SourceObs`, `Heading` to `SectionAddr`). Compile-time access comes from the providers' and M-003's host types and primitives handed off early after the Epic freeze; that handoff is not a completion wait and no `dependencies` entry is needed. Until a provider's code lands, M-005 tests use the real `Store` on a temporary root and real provider code, and a unit-only substitute uses the provider's own types and never counts as composition proof.

- **M-001 `kr-history` r5:** `knowledge::{allocation_version(store), reserve(store, guard, Prefix::Compaction, expected, effects), parse_id, Prefix}`. There is no `foreign` list: M-001 derives CP identifiers from `compaction::inventory` (section 7). M-005 never rewrites typed records; typed references reach it through M-002 coverage.
- **M-002 `documents-and-references` r2:** `documents::{observe, save, adopt, relocate, remove, Scope, Ref, Edit, Save, Receipt, MoveBasis, BODY_CAP}`, `Observation::{record_sha256, move_basis}` (M-002 provider revision 3 is forthcoming; M-005 consumes only what M-002 publishes and invents no future canonical), `markdown::{outline, Heading}`, `DocPath::parse`, `references::{incoming, integrity, rewrite, resolve, Overlay, Coverage, Target}`; whole-body saves only (`Edit::Body`).
- **M-004 `persist-store-m005` r3:** `Store::{read_exact, publish_with, remove, ensure_parents, list_dir, publications}`, `store::{Publish, Remove, OperationId, Attest, Publication, own_temp_name}`, `persist::{effect_status, ExpectedEffect, EffectStatus, EffectReceipt, PathEffect, receipt, committed_original, verify_committed, Item, Locator, Proof, pending, EventClass}`, the dispatcher's `settled`. M-005 uses `Attest::Required` for every publication and the complete-operation oracle of section 9.
- **M-003 `producer-host` r1:** the shared closed `input::mutation::<Compaction>` decode and `mutation_schema`, `input::field` bounded validation, one caller-held `Store` and `LockGuard` with no repeated lock, the existing `work::Ack` with phase label, notes and refs, and the `execute_locked` signature; the host renders one truthful receipt or error.

## 13. Errors and effects

Codes: `invalid_arguments` (field named), `actor_required`, `cp_not_found`, `stale`, `stale_source`, `stale_revision`, `request_key_conflict`, `source_refused`, `section_unaccounted`, `section_unknown`, `preservation_unmapped`, `path_claimed`, `capacity`, `coverage_incomplete`, `incoming_unresolved`, `stage_unowned`, `self_review`, `reviewer_cannot_author`, `reviewer_gap`, `review_incomplete`, `not_accepted`, `apply_started`, `originals_not_committed`, `effect_unknown`, `effect_foreign`, `apply_blocked`, `effect_incomplete`, `replacements_not_committed`, `post_apply_drift`, `metadata_mismatch`, `incoming_changed`, `already_withdrawn`, and passed through unchanged from providers: `partial_publication`, `durability_unknown`, `attestation_unavailable`, `attestation_unknown`, `not_locked`, `allocator`, `inventory`, `busy`, `not_committed`. Every refusal before the first publication has no effect. The ledger carries the store's `Published ...` lines plus `Reserved CP-nnn.`, `Adopted attested effect <operation>.` and `Blocked <kind> at <action>.`; typed events stay in `store.publications()`.

## 14. Executable controls (real provider code, real Git temporary repositories)

Each control has positive, negative and a named mutation of the implementation that must fail the same case; observations run correct, mutated and restored, tied to the restored candidate and contract revision. A pure fake of `effect_status` is acceptable for unit tests but never counts as composition proof.

| Control | Positive | Negative | Mutation |
|---|---|---|---|
| C1 operations | each op decodes by `input::mutation::<Compaction>` and returns the `Ack` with ledger; production policy commits exactly the call's paths after success | unknown op, extra field, stale version, missing actor naming the field; partial apply is held not committed | ledger dropped; ack reports Ok for a blocked apply; unchanged revise publishes |
| C2 review | independent accept with exact items; follow-up resolves a finding by index; same reviewer through corrections | author reviews; reviewer revises; stale hash; missing or extra item; unresolved must_fix; reviewer swap without lost evidence | self-review allowed; acceptance survives revise; reviewer swapped |
| C3 coverage | complete coverage, rewritten Markdown and README referrers, clean integrity, apply | gap from a corrupt typed or work record; record referrer to a moved or removed path; new incoming after acceptance; `Resolution::Unknown` target | partial treated complete; unknown scope ignored |
| C4 originals (real Git) | committed body and record pass; bytes read back from the locator equal pre-apply bytes | untracked, dirty, ignored backup, pending deferred intent, changed since commit, unborn or detached HEAD | working tree or backup accepted as proof |
| C5 apply order | create, move, replace, verify, remove; originals intact in Git; one commit holds exactly the call's paths | removal before verification; dangling after replace stops removals | removals first |
| C6 interruption | crash after the Applying record, after body before record, after effect before copy, lost reply, resume by same call | equal bytes without attestation blocked; foreign edit blocked; second CP with identical bytes not adopted; finished CP UNCHANGED; own progress never changes acceptance digests; external body or DOC metadata edit does change them | adoption by byte equality; attempt nonce in operation id; coverage version in digest |
| C7 retention and capacity | maximal record within caps; revision 2 keeps revision 1 body and reviews byte identical; accepted review retained after revise | overflowing revise refuses with prior record bytes untouched; ninth revision; oversized blob | prior body dropped; review rewritten; cap unenforced |
| C8 staging | blobs no clobber; orphan visible and counted | equal-bytes blob without attestation refuses `stage_unowned`; id never recycled | adopt by equality; overwrite |
| C9 inventory | section 7 controls through the real `Store::list_dir` and M-001 observation | each foreign nested name incomplete and named | foreign names ignored |
| C11 oracle consumption | operation publishing body, generated record and counter returns all rows; adoption only after complete receipt, current body, M-002 managed metadata and exact record effect; superseded body noted | body only, same body without record, Prepared metadata with matching bytes, conflicting record row, truncated history (`complete = false`), changed metadata after the effect, wrong revision or path: each stays Partial or Blocked, never Applied; adoption of an unattested body refused | adopt on `Attested` alone; unlisted rows treated Foreign; same body without record Applied; `complete = false` ignored; changed metadata passes; counter row required as ownership |
| C12 action kinds (real repository) | Create, Replace of Managed, Replace of Unmanaged, Move of Managed, Move of Unmanaged, Remove of Managed and Remove of Unmanaged each reach `Applied` only with their table post-image: Managed remove ends Retired with own attested record effect, unmanaged remove has no synthetic record, managed move keeps the old DOC id at the destination and the source Absent | the same body without its record, a record with wrong revision, path or body hash, a Remove checked as `Managed`, a Move that retires the old id, unmanaged operations creating a record: none is `Applied` | require `Managed` for every kind; accept body without metadata; synthesize a record for unmanaged remove |
| C13 resume (real repository) | body written, record never published (crash between): resume classifies by the oracle first, then rolls forward with `documents::adopt` and reaches `Applied`; after the first action the full `sources_digest` and `overlay_digest` are not recompared while `incoming_digest` stays equal; resume re-verifies stored original locators at their commit and accepts the partly published uncommitted body | external new incoming reference or untouched referrer change after the first action blocks; a later unrelated attested MCP save or removal of an applied path, or an external metadata edit, blocks `post_apply_drift` with no Remove run and nothing overwritten; repeating apply on `Applied` publishes nothing and reports drift without calling the state verified; a stored locator that no longer verifies refuses; a partial Move resumes only through M-002's identity bearing relocate with the `MoveBasis` stored at propose, covering W1 (attested destination body, record unmoved), W2 (record moved, source body remains) and W3 (no-op), and the destination keeps the source DOC id | recompare full digests after first effect; check `base_version` before the oracle; require the new body to be committed as an original; accept `superseded_into = Some` as success; treat drift as attention during resume; delete and recreate the destination; adopt a Move destination under a new DOC id; rebuild the `MoveBasis` from the partial new state; omit `record_sha256` from the stored source |
| C10 provenance availability | Required publication admitted with a journal | no repository or full or corrupt journal refuses an `Attest::Required` write with `attestation_unavailable` before its own effect; a propose refused at a later Required step may already have published the optional allocator reservation, which shows as a visible gap and a held partial call, never as a claim of no effect | proceed untracked; claim no effect after a published counter gap |
| C14 removal gate (real repository, production policy) | interrupted first apply writes the replacement and returns `Err` (intent `Held`); after the exact old `git_recovery Retry` proves it committed, the resumed apply verifies the replacement at its commit, removes, and one removal commit follows; a fresh apply with Create, Replace, Move and Remove in one call ends in exactly one commit holding all its paths | resumed apply with the replacement still `Held`, `Pending`, `Unknown` or untracked cannot commit a deletion-only intent: it blocks `replacements_not_committed` before any removal and names Retry; a clone of the repository never sees a removal without its replacement | commit deletion-only; skip the gate; treat `Held` as committed; demand the current call's own new effects be `Committed` inside the handler; judge a superseded row by its old digest |
| C15 move source removal gate (real repository, SDK sequence) | exact sequence: (1) first apply of a Move publishes the destination body then errors, so the intent is `Held`; (2) resume hits the gate and BLOCKS `replacements_not_committed` before calling relocate, leaving the source body and record unmoved and naming Retry; (3) `git_recovery Retry` commits the old destination intent; (4) resume finishes through the identity bearing recovery with the stored basis and the same source DOC id, reaching `Applied`; (5) a normal clone holds the destination body, the record at the destination with the source DOC id, and no source; the same sequence holds for the W2 and unmanaged variants; a plain fresh same-call Move is one intent and succeeds | after step 1 a resume that only restores permissions or removes the fault does not finish the move while the old destination is `Held`; a clone taken before Retry never lacks the destination because the source was never removed | call relocate and its source removal while the old destination is `Held`; apply the gate only in the Remove phase; require the current call's own files to be `Committed` inside the handler and deadlock a fresh call |
| C17 held barrier (r6) | an interrupted apply is retried by explicit Retry and then resumes to `Applied` with nothing left pending; a held intent holding only the proposal record also blocks until Retry | resume before Retry refuses `replacements_not_committed` and publishes nothing; a later committed record write strands an older held intent (Retry `recovery_blocked`); an older held intent on unrelated paths neither blocks nor is stranded | barrier removed; barrier ignores the record path; barrier blocks every held intent |
| C18 no-effect Replace (r6) | a Replace with changed content is accepted | a Replace whose candidate equals the current document refuses `invalid_arguments` naming `actions.content` | refusal removed |
| C19 retained reads (r7) | after a revise, `retained_revision(1)` returns revision 1 byte for byte with its bound reviews and `read_staged(1, A-01)` returns the revision 1 candidate while revision 2 differs; the verifier `read_staged` uses is the one `load_blobs` calls | revision 0 and a revision beyond current refuse `invalid_arguments` naming `revision`; a non canonical, absent, Move or Remove action refuses naming `action`; a missing, corrupt or wrong length blob refuses `invalid_data` and returns no bytes | the current revision selected instead of the named one; the digest check skipped; the live document read when the blob is missing |
| C16 eligible owned files | the first propose that creates the `compactions/` home and a legacy record normalization with an ignored backup each commit exactly their owned files; directory, backup, lock and temp effects are not staged and not journal-attested and are excluded from gate counts | a real non-ignored untracked sibling file blocks the whole current call (held, data saved); a failed parent sync on a necessary directory is not reported durable | count directory or backup events as untracked siblings; mark them `Tracked`; stage them |

Substitute passes are not joint behavior proof; the integration Atomic and real SDK runs under the production policy prove composition. No test is claimed passed by this document.

## 15. Provider gaps and mismatches observed (not agreement)

Closed against actual provider text:

- **P1 closed (M-004 `persist-store-m005` r3):** the complete-operation oracle returns every attested same-operation effect and asserts only the caller's computable body effects; generated metadata returns `asserted = false` and is never foreign merely for being unlisted. Section 9 consumes it. Counter reservations have no action `OperationId`.
- **P2 closed (M-001):** counter reservations are validated through the six-home allocator and current DOC identity, not through operation rows.
- **Compilation waits closed:** host types and primitives are handed off early after freeze; no dependency is declared.

Still open (to be settled by the owning provider, not by M-005):

- **P3 M-003:** its artifact lists `compaction::{read_cp, summaries, inventory_ids}` and its tracker entry consumes `compaction-operations` revision 2 with the pin left to the provider; `inventory_ids` does not exist, M-005 provides `read_cp(store, id)`, `summaries`, `inventory` and `scan`, and the artifact reference is the root commit of this file once it exists.
- **P4 M-004 text:** section 11 cites `registered-tool-surface` revision 2 at commit 79f1476 while M-003's entry is revision 3 at 38a5553; the artifact heading revisions of M-001 (3) and M-003 (2) differ from their tracker revisions (5, 3), which is only a labeling mismatch.
- **P5 cost:** M-001's observation lists the compactions home on every read and reserve; section 7 bounds it, but M-001 should call `inventory` once per observation.
- **P7 M-002 identity bearing relocate resume:** M-002's corrected relocate takes a `MoveBasis` built from the original observation, and `Observation` gains `record_sha256`. M-005 stores both at propose and passes the stored basis on resume. The `documents-and-references` pin stays at revision 2 (commit ea3a17f) until M-002 actually publishes its forthcoming revision 3 and root commits it; M-005 then re-pins and re-agrees, and meanwhile a partial Move stays `Blocked`. M-002's ordinary operations with no operation identity resolve a copy only after fresh observations before an explicit remove and never adopt native bytes by equality; M-005 always passes an operation identity and does not rely on that path.
- **P9 M-004 `Tracking::NotApplicable`:** N2 needs M-004 to publish the explicit non-file tracking state and to exclude directory, ignored backup, lock and temp effects from tracked and untracked counts and commit-unit selection; M-004 raises its revision. M-005 consumes only the published state and meanwhile excludes those classes by `EffectKind` and path class in its own gate.
- **P8 held chain:** the removal gate relies on M-004's held predecessor and `EffectGit::Committed` data as already published in `persist-store-m005`; no M-004 change is requested.
- **P6 trailer chain:** after a deferred chain on one path, an earlier operation's body shows `superseded_into`; the historical effect stays attested, but M-005 treats the changed current state as `post_apply_drift`, never as success of the earlier bytes and never as proof that they are recoverable from Git.

## 16. Explicit non-goals

No model or agent launch, no daemon, no automatic deletion or proposal generation, no semantic judgement, no typed body compaction, no rollback engine, no eviction of history, no push or history rewrite, no Git calls by M-005, no shipping test trigger, no hidden policy switch, no producer templates or catalog, no duplicate provider type.
