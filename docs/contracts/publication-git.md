# Guarded publication and automatic Git persistence: canonical boundary (revision 2)

Status: planning prose for Epic E-001, Module M-004. Nothing here is implemented. Rust below is the proposed public surface of `src/store.rs` (extended) and the new `src/persist/` module, written so consumers can code against it before the provider lands. The provider owns this file, every identifier below and every revision number. A consumer never defines them. Changing a signature, limit, code or rule that a consumer relies on, or changing the pinned artifact reference, raises the revision of every affected entry and needs fresh confirmation from every party.

Boundaries and consumers (provider M-004 for all):

| Identifier | Consumer | Sections |
|---|---|---|
| persist-store-m001 | M-001 typed knowledge | 1, 2, 3, 4, 6, 8, 10, 12 |
| persist-store-m002 | M-002 Markdown and bootstrap | 1, 2, 3, 4, 7, 9, 10, 12 |
| persist-store-m005 | M-005 compaction | 1, 2, 3, 4, 5, 6, 7, 8, 10, 12 |
| persist-git-m003 | M-003 registry, dispatch, presentation | 2, 4, 5, 6, 11, 12 |

Revision 2 against the committed revision 1: operation provenance and `effect_status` (section 7); per-path committed-original proof, stable locator and the M-001 locate mapping (section 6); optional versus required attestation so Git bookkeeping never stops ordinary writes (sections 2 to 4); work mutations inside normal settlement and one shared dispatcher lock (section 11); the producer `RecoveryOp` payload with an exact pending snapshot version (sections 6 and 11); one owner and location per type (section 12); `kind_inventory`, `own_temp_name` and `list_dir` with exact grammar (sections 1 and 10); finer compaction event classes and commit trailers (section 5); the owner's production policy `CommitAfterSuccess` with a three-state event outcome and success evidence kept per intent (sections 4 and 5).

Pinning note. Tracker entries pin immutable committed bytes, so a boundary pins the commit that holds the definition it was agreed against. The tracker currently pins `persist-store-m001` revision 2 at commit 2193a35 and `persist-store-m002` revision 3, `persist-git-m003` revision 3 and `persist-store-m005` revision 4 at commit b04c899; those pins and their history stay untouched until root signs this text. This text adds one more observable change on top of b04c899: `Tracking::NotApplicable`, a closed category for directory events and built-in administrative paths (ignored normalization backups, locks, temps) that are never tracked or untracked eligible files (sections 2, 3, 5 and 7), plus the user-ignore rule that a user-ignored or untracked business file defers the whole current intent (`IgnoredBusinessSibling`) without forced staging, together with their controls. It changes the visible event type and the receipt rules that every consumer relies on, so each of the four provided boundaries needs a higher revision and fresh agreement from every party. Revision floor, planned after root signs: `persist-store-m001` revision 2 to 3 (the event type and the first create of a new home and normalizing write that M-001 depends on), `persist-store-m002` revision 3 to 4, `persist-git-m003` revision 3 to 4 (receipt rules) and `persist-store-m005` revision 4 to 5 (current-intent gate counts). Earlier refinements keep their meaning: the complete-operation oracle (section 7, commit 70f2867) and the call-scoped receipt, `call_intents`, `intent_view` and the limited same-operation removal guard (b04c899). The artifact's own heading revision is not the tracker revision, and the artifact does not record its own commit.

Owner decision (recorded here as the production policy): the MCP commits automatically after each successful actual mutation of a document, a knowledge record, a compaction record or its effects, or a task or project record, including the existing `plan_work`, `record_work` and review mutations. Reads, true no-ops, registration's already committed bootstrap, and failed or partial business calls do not trigger a normal commit. A Git failure never undoes saved files and gives a truthful deferred or unknown receipt. Earlier successful calls that Git deferred are settled by a later successful mutation; earlier partial, failed or unknown business outcomes are never silently certified or committed and wait for explicit recovery. This file defines no threshold, cadence, daemon, configuration or test switch: the production policy is `CommitAfterSuccess` (section 5). `DeferAll` exists only for tests. Explicit `Preserve` is exceptional recovery and never a substitute for the normal feature.

## 0. Rules that hold everywhere

1. One lock scope. A mutating call takes the existing root write lock once, publishes, settles Git and releases it. No primitive or engine function takes the root lock, and no nested lock is introduced. Allocator files are published under that same lock.
2. Reads never take the write lock, never create files, never run Git that writes, and never commit.
3. Saved work is never undone because Git failed or a renderer failed. Every receipt below is built before presentation and is plain data. Git bookkeeping is optional for ordinary writes: its failure is reported, not turned into a refused write (section 3). Only an operation that explicitly requires attestation refuses, and it refuses before any effect.
4. Equal bytes are not identity. A file is an MCP-owned published change only if a private journal entry written before publication names it (section 4), or an engine commit carries the operation trailer for it (section 5). Native, unjournaled or unattested files with equal bytes are never certified as published ownership, now or later.
5. Nothing here pushes, merges, deletes branches, rewrites history, resets, edits global Git configuration or weakens signing and hooks.
6. All text is English. Errors never carry raw Git stderr, command lines, paths outside the root, or credentials.
7. Provenance limit, stated plainly: a trailer in a commit proves what the repository says, and anyone with write access to the repository can write such text. The engine therefore attests only when the trailer, the commit tree and the recorded digests all agree (section 7). That is strong against accident and drift, not against a deliberate forger with repository write access.
8. Business-data validation (caps, versions, path rules, domain checks) always refuses before any effect, in both attestation modes.

## 1. Store primitives (additive; existing signatures stay)

Unchanged and still supported: `Store::path`, `bytes`, `version`, `expect`, `lock`, `file_lock`, `prepare`, `publish(relative, bytes, observed, effects)`, `save(...)`, `encode`, `decode`. `publish` and `save` are `Attest::Optional` with no operation, so every existing work, Epic, Atomic and allocator write keeps working and joins settlement without a signature change. `encode` and `save` already verify canonical YAML through the exact read gate before any write (hotfix commit e971dac).

```rust
/// Largest cap any caller may request. Bounds memory regardless of the caller.
pub const ABSOLUTE_CAP: usize = 8 * 1024 * 1024;

/// Exact observed owned file: bytes plus the root-bound version of exactly those bytes.
pub struct Observed { pub relative: String, pub bytes: Vec<u8>, pub version: String }

/// Opaque caller-chosen identity of a multi-step operation, 1..=64 chars of A-Z a-z 0-9 . _ : / -.
/// Carried on events, in the journal and in commit trailers. It grants nothing and is never parsed.
/// Callers derive it deterministically (for example from a proposal and action), never from an
/// attempt counter, so a retry repeats the same identity. Single definition: store::OperationId.
pub struct OperationId(String);
impl OperationId { pub fn new(value: &str) -> Result<Self>; pub fn as_str(&self) -> &str; }

/// Whether the caller needs this publication to be attested by the private journal.
/// Optional: ordinary mutation. If the journal cannot admit the entry the write still happens and the
/// event says it is untracked. Required: the write happens only if the journal entry is retained
/// first (needs an operation identity); otherwise `attestation_unavailable` and no effect.
pub enum Attest { Optional, Required }

/// One guarded creation or replacement.
pub struct Publish<'a> {
    pub relative: &'a str,            // owned relative path, no links, parents exist
    pub bytes: &'a [u8],              // exact bytes, any content, never normalized
    pub observed: Option<&'a [u8]>,   // None = create without clobber; Some = replace only if current bytes equal
    pub cap: usize,                   // bytes.len() <= cap <= ABSOLUTE_CAP
    pub operation: Option<&'a OperationId>,
    pub attest: Attest,               // Required needs operation = Some, else `invalid`
}

/// One guarded removal.
pub struct Remove<'a> {
    pub relative: &'a str,
    pub observed: &'a [u8],           // removal happens only if current bytes equal this
    pub cap: usize,
    pub operation: Option<&'a OperationId>,
    pub attest: Attest,
}

/// One bounded directory listing entry. No recursion, no following of links.
pub enum EntryKind { File, Directory, Symlink, Other }
pub struct DirEntry { pub name: String, pub kind: EntryKind }
pub struct DirListing { pub entries: Vec<DirEntry>, pub complete: bool }   // sorted by name; complete=false when `cap` was hit

impl Store {
    /// Read one owned regular file with a caller cap. Refuses links, non-regular leaves and
    /// anything above `cap` (`capacity`). None = absent. No filesystem effect.
    pub fn read_exact(&self, relative: &str, cap: usize) -> Result<Option<Observed>>;

    /// Create or replace. Same algorithm as today's `publish`: exclusive temp, write, sync,
    /// recheck observed bytes, no-clobber hard link for create, atomic rename for replace, sync parent.
    pub fn publish_with(&self, request: Publish<'_>, effects: &mut Vec<String>) -> Result<Publication>;

    /// Remove only the exact observed bytes (algorithm in section 3).
    pub fn remove(&self, request: Remove<'_>, effects: &mut Vec<String>) -> Result<Publication>;

    /// Create exactly one missing directory whose parent exists. Existing directory is a no-op
    /// (`Publication` with kind `DirectoryExisting`); a file or link at the path refuses.
    pub fn create_dir(&self, relative: &str, effects: &mut Vec<String>) -> Result<Publication>;

    /// Create missing ancestors of `relative` below the root, at most 4 levels, each disclosed.
    /// Returns one `Publication` per directory actually created.
    pub fn ensure_parents(&self, relative: &str, effects: &mut Vec<String>) -> Result<Vec<Publication>>;

    /// Every typed event this Store instance (one request) has produced, in order, including
    /// events of calls that later returned an error. Replaces the effect-string prefix check.
    pub fn publications(&self) -> Vec<Publication>;

    /// Bounded, non-recursive, sorted listing of one owned directory (path rules of `Store::path`).
    /// `cap` at most 4096 entries. Absent directory is `Ok` with no entries and complete=true; a file
    /// or link at the path is `file_type`. This is the one directory-listing primitive; callers do not
    /// open directories themselves.
    pub fn list_dir(&self, relative: &str, cap: usize) -> Result<DirListing>;

    /// The existing one-directory ID inventory, made public so knowledge code reuses it (grammar in section 10).
    pub fn kind_inventory(&self, directory: &str, prefix: &str) -> Result<Inventory>;
}

/// True for this store's own publication leftovers: `.<name>.tmp-<pid>-<seq>` and
/// `.<name>.rm-<pid>-<seq>`, where `<name>` is a nonempty file name without `/` or NUL,
/// `<pid>` and `<seq>` are decimal. Foreign dotfiles stay foreign.
pub fn own_temp_name(name: &str) -> bool;
```

`Store` is created once per request by `Config::resolve`. Its event list is request-local and lives inside the Store value; the list is how a failed call still reports what it published.

Compatibility: `publish` is `publish_with` with `cap = RECORD_CAP`, no operation, `Attest::Optional`. `save` keeps its behavior and adds the event. `Store::bytes(rel)` equals `read_exact(rel, RECORD_CAP)` without the version. Markdown bodies use `cap = RECORD_CAP` (512 KiB) as M-002 chose; a larger cap needs only a larger argument, not a new revision, up to `ABSOLUTE_CAP`.

## 2. Events, tracking, effect strings, durability

```rust
pub enum EffectKind { Created, Replaced, Removed, DirectoryCreated, DirectoryExisting }
pub enum Durability { Durable, SyncUnknown }

/// Whether the private journal holds an entry for this effect.
pub enum UntrackedReason { NoRepository, JournalFull, JournalUnavailable, JournalCorrupt }
pub enum Tracking {
    Tracked,                              // entry written before publication and confirmed after it
    Untracked(UntrackedReason),           // optional write proceeded without an entry; never certifiable later
    UnknownAfterPublication,              // entry written, but confirming it failed after the file became visible
    NotApplicable,                        // an effect that never has a file journal entry (see below); neither tracked nor untracked
}

/// Plain record of one filesystem effect. Serializable; contains no file content.
pub struct Publication {
    pub relative: String,
    pub kind: EffectKind,
    pub before: Option<String>,        // Store::version of the replaced or removed bytes, None for create
    pub after: Option<String>,         // Store::version of the new bytes, None for remove or directory
    pub durability: Durability,        // SyncUnknown = visible but parent sync failed
    pub operation: Option<String>,     // OperationId as given
    pub intent: Option<String>,        // journal identity (section 4); None when Untracked
    pub tracking: Tracking,
}
```

Not-applicable effects. This is a closed category for DIRECTORY events and BUILT-IN administrative paths only; a business file never becomes not-applicable because some ignore rule matches it. These effects never receive a file journal intent and are `Tracking::NotApplicable`, never a faked `Tracked` and never `Untracked`: directory creation and existing-directory notes (`DirectoryCreated`, `DirectoryExisting`), every file under `.agent-tasks/backups/` (ignored normalization copies), lock files such as `.agent-tasks/write.lock`, and this store's own publication temps (`own_temp_name`). `Store` decides from this fixed list (it matches the repository ignore rules written at bootstrap) and never asks Git. They are still truthful filesystem effects: each has its own `Publication` with `durability`, a `SyncUnknown` on a directory that a file depends on is reported as `Durability::SyncUnknown` and `Attention::SyncUncertain`, never claimed `Durable`, and it counts as sync-uncertain for the call (section 5). They are excluded from the eligible owned-file counts (`tracked`, `untracked`), from receipt rules 2 and 3, from commit-unit and path selection, from anything the engine stages or commits, and from the M-005 current-intent gate. The business files a call publishes (record bodies, metadata, counter files, documents) are the eligible owned files; the recovery proofs of section 6 cover those files, not empty directories.

Effect strings stay exactly as today for compatibility: `Published <relative>.`, `Created <relative>/.`, `Created <relative>.` for locks. New: `Removed <relative>.` and `Created directory <relative>/.`. A visible publication whose parent sync fails returns error code `durability_unknown` and the event is present with `durability = SyncUnknown`; it is never retried implicitly. An untracked or unknown tracking state is carried only in the event and the call's `GitReceipt` (section 5), never in a new effect string.

## 3. Limits, errors, ordering, removal, admission

Caps and checks run before any effect: `bytes.len() <= cap <= ABSOLUTE_CAP` else `capacity`; path rules as `Store::path` (`path`, `file_type`, `root_changed`). Create over an existing file or replace of changed bytes gives `stale` and saves nothing. Error codes added: `not_locked` (the cooperative write lock was not acquired through this Store in this call), `attestation_unavailable` (section below), `attestation_unknown` (section below), `removal_conflict`, `operation_repeat`, `not_committed`, `locator_too_long`, `preserve_blocked`, `encoding_unreadable` (already live). All previous codes keep their meaning. A full journal is not an error: it is `UntrackedReason::JournalFull`.

Journal admission applies to eligible owned files only. Not-applicable effects (directories, ignored backups, locks, temps; section 2) skip admission, are never refused or reported untracked because of it, and a full or unwritable journal does not touch them. Journal admission, run after the business validation and before the first effect of each eligible file publication:

| Admission result | `Attest::Optional` | `Attest::Required` |
|---|---|---|
| Entry retained (Prepared written) | publish; `Tracked` after the Published mark | same |
| No repository | publish; `Untracked(NoRepository)` | refuse `attestation_unavailable`, no effect |
| Journal full (64 intents, 256 paths or 256 KiB serialized) after pruning verified committed entries | publish; `Untracked(JournalFull)` | refuse `attestation_unavailable`, no effect |
| Journal write fails (I/O) | publish; `Untracked(JournalUnavailable)` | refuse `attestation_unavailable`, no effect |
| Journal present but unreadable or corrupt | publish; `Untracked(JournalCorrupt)`; the file is kept as evidence, never rewritten | refuse `attestation_unavailable`, no effect |

After the file became visible, if writing the Published mark fails: `Optional` returns `Ok` with `Tracking::UnknownAfterPublication` and the call's receipt is `Unknown`; `Required` returns error `attestation_unknown` carrying the visible event, like `durability_unknown`, and the caller stops. Neither mode reverses the publication.

Guarantees in every row: existing journal entries are never discarded, overwritten or reordered; only verified `Committed` entries are pruned, oldest first (section 4); an optional write that proceeds untracked claims no commit and no false "no effect"; a file written untracked is never certified later by equal bytes, and the engine never commits it. Later explicit recovery may preserve exact caller-authorized bytes (section 6 and 11).

Which callers use which mode: ordinary mutation (work records, knowledge records, document saves, allocator counters, plain `publish` and `save`) uses `Optional`. Operations whose correctness depends on attestation after an interruption use `Required` with a deterministic `OperationId`: M-005 `apply` steps. A caller picks the mode per publication.

`operation_repeat`: a publication that names an `OperationId` is refused, with no write, if the journal already holds a `Published` or later entry for the same operation and the same path. This is the cheap idempotency guard; the full oracle is `effect_status` (section 7). It never reads Git history.

Removal algorithm: validate path; read with `cap`; compare to `observed` (else `stale`); admit the journal entry per the table; atomically rename the file to a unique hidden sibling `.name.rm-<pid>-<seq>`; reread the sibling and compare again; unlink it; sync the parent; emit `Removed`. If the second compare differs the sibling is renamed back with no-clobber semantics and `removal_conflict` is returned, or if the original name was taken meanwhile the hidden sibling is left in place and named in the error and effects. Lock-free readers may observe the file absent during this short window; cooperating writers cannot, because the write lock is held.

Ordering: primitives apply exactly the order the caller issues. Compaction and similar callers create or replace first and remove last; each step produces its own event so a partial run is visible.

## 4. Pending journal, intent identity and reconciliation (private provenance)

Location: `.git/agent-tasks/pending.yaml` in the documentation repository, a closed schema_version 1 YAML file, bounded to 256 KiB of actual serialized bytes, at most 64 open intents and 256 paths; admission measures the serialized result and never truncates an entry. It lives inside the Git directory, is untracked by definition, is never staged or committed, and is local to the clone. No other Module reads or writes it; consumers use the functions in this file only. Without an independent repository (`.git` directory) there is no journal.

Identity: `IntentId` is `PG-` followed by 24 lowercase hex characters derived from the root, process id, a nanosecond clock and a counter. It identifies one journal intent: all tracked publications of one handler call. `OperationId` is optional caller identity carried through and may span several intents (an interrupted and resumed multi-step operation).

Write-ahead order for each publication under the held lock:

1. Admission as in section 3. A refusal or an untracked decision happens here, before any business file is touched.
2. Publish the journal with the new path entry in phase `Prepared`: relative path, effect kind, before version, planned after digest (sha256) and length, the operation identity, and on Unix the device and inode of the exclusive temp file that will become the target.
3. Perform the business publication as in section 1.
4. Publish the journal again moving the entry to `Published`, with the confirmed after version.

Each intent also carries its `outcome` stamp (`Unsettled` until the call settles, then `Success` or `Partial`, section 5; a `Failed` call published nothing, so it has no intent to stamp), the event class and the references; this is the retained success and completeness evidence, so the policy decision and later recovery never depend on memory of the call.

The journal file itself is written by an internal raw publication that is not journaled; it is replaced atomically, so a failed write leaves the previous journal intact. Ignored paths (lock files, `backups/`, publication temps) never get entries. M-005 keeps its proposal state in its own tracked record and writes no second file-level provenance journal: it passes an `OperationId` and `Attest::Required` on each step, and interruption recovery reads this journal, the typed events and the engine commit trailers.

Reconciliation of an entry against current bytes (pure table, used by settlement, pending facts, `effect_status` and recovery):

| Journal phase | Current file | Meaning |
|---|---|---|
| Prepared | equals recorded before | Unpublished intent. Discarded automatically at the next write scope, reported as `Abandoned`. |
| Prepared | equals planned after, and same inode on Unix | Published but unattested. Phase `Unknown`; evidence shown; only explicit recovery adopts it. |
| Prepared | equals planned after, inode differs or unavailable | Phase `Unknown`; equal bytes alone certify nothing. |
| Prepared or Published | equals neither | Native drift. Excluded from every commit and reported. |
| Published | equals planned after | Eligible for settlement. |
| Committing | HEAD advanced and carries the intent trailer | Committed; verify (section 5). |
| Committing | HEAD unchanged since recorded parent | Retry allowed after reconciliation. |
| Committing | anything else | Unknown. Never retried blindly. |

Native edits made to a managed file after publication are drift. A hook that rewrites the worktree during a commit leaves entries in `Published` with a changed file: reported as mismatch, never reset. A file with no entry (untracked write, native file) is not in this table at all and is never committed by settlement.

Retention. Entries in `Committed` stay in the journal until the bounds need the space; the oldest are then pruned, and only after the commit that holds their trailers is verified reachable. Operation evidence therefore survives journal cleanup through the commit trailers (section 5), never through a second journal. Entries that are not committed (`Prepared`, `Published`, `Committing`, `Unknown`, `Drifted`) are never pruned automatically, which is why a long run of deferred commits can fill the journal; section 3 defines what that means for new writes.

## 5. Policy, event schema, settlement and receipts

Only a foreground mutating call can settle. There is no daemon and no timer. Work mutations (tasks, records, reviews), knowledge, documents and compaction all settle through the same call.

```rust
/// Closed set of mutation families. Producers pick one; the set grows only by a new revision.
pub enum EventClass {
    Work, Knowledge, Document,
    CompactionPropose, CompactionRevise, CompactionReview, CompactionApply, CompactionWithdraw,
    Recovery,
}

/// How the business call ended, as judged by the dispatcher. Success means the handler returned Ok.
/// Partial means it returned an error after publishing at least one effect (including
/// `partial_publication`, `durability_unknown` and `attestation_unknown`). Failed means an error
/// with no publication at all: nothing was written, no intent exists, and settlement is not called.
/// Settlement refines Success further: a Success call counts as Partial when any of its events has
/// `Durability::SyncUnknown`, `Tracking::UnknownAfterPublication` or `Tracking::Untracked(_)`,
/// because the call's whole business save cannot then be committed as one unit.
pub enum EventOutcome { Success, Partial, Failed }

/// What a settlement is asked about. Built by the dispatcher (M-003), not by producers.
pub struct Event {
    pub class: EventClass,
    pub refs: Vec<String>,            // up to 16 canonical references of affected records or documents, each up to 256 bytes (a managed document path is at most 160 bytes and is never truncated); an operation identity stays at most 64 characters
    pub operation: Option<OperationId>,
    pub outcome: EventOutcome,
}

/// Read-only facts the policy may use. No file content and no Git output.
pub struct PendingFacts {
    pub intents: usize,
    pub paths: usize,
    pub oldest_unix_secs: Option<u64>,
    pub unknown: usize,
    pub drifted: usize,
}

pub struct PolicyInput<'a> {
    pub event: &'a Event,
    pub pending: &'a PendingFacts,
    pub tracked: usize,               // tracked (journaled) eligible-file publications this call produced; 0 for a true no-op or when only not-applicable effects occurred
    pub sync_uncertain: bool,         // some event of this call, directories included, is SyncUnknown or UnknownAfterPublication
    pub untracked: usize,             // eligible-file publications of this call that have no journal entry (not-applicable effects are never counted)
}

pub enum Decision { Commit, Defer(DeferReason), Skip }
pub enum DeferReason { PolicyDeferred, IncompleteOutcome }     // other reasons are engine states, never policy output

/// Pure: no I/O, no clock reads, no global state, no configuration.
pub trait Policy { fn decide(&self, input: &PolicyInput<'_>) -> Decision; }

/// The production policy (owner decision): commit after each successful actual mutation.
///   Skip   when the class is Recovery (recovery commits only what it was explicitly asked to);
///   Skip   when `tracked == 0` (true no-op, or everything untracked);
///   Defer(IncompleteOutcome) when the outcome is Partial or Failed, or `sync_uncertain`, or `untracked > 0`
///     (a call with any untracked publication is never committed as a tracked subset);
///   Commit otherwise (outcome Success with at least one tracked publication).
/// No thresholds, counters, timers or settings.
pub struct CommitAfterSuccess;

/// The policy a production dispatcher passes: always `&CommitAfterSuccess`.
pub fn production_policy() -> &'static dyn Policy;

/// Test-only policy that never commits, for deferral and pending scenarios.
#[cfg(test)]
pub struct DeferAll;

#[cfg(test)]
pub mod testing {
    /// Run `body` with an injected policy visible to `production_policy()` on this thread only.
    pub fn with_policy<R>(policy: &'static dyn Policy, body: impl FnOnce() -> R) -> R;
    /// One-shot fault points: AfterIntent, AfterRename, BeforePublishedMark, JournalFull, JournalWriteFails,
    /// JournalCorrupt, CommitTimeout, CommitKilled, HookModifiesTree.
    pub fn fail_at(point: FailPoint);
    /// Disposable real repository fixture builder used by the fixture matrix and by other Modules' tests.
    pub struct GitFixture { /* temp root with independent repository, hooks and config knobs */ }
}
```

There is no environment variable, command-line option, feature flag or configuration file that selects a policy in a shipping build: the production dispatcher always passes `production_policy()`, which is `CommitAfterSuccess`. Real-process qualification exercises that real policy through the registered tools; `testing::with_policy` exists only for in-process tests.

Settlement runs while the caller holds the single lock and never fails the call:

```rust
/// Evaluate the policy and, if it says Commit, run the Git engine over the eligible published
/// intents. `guard` proves the write lock for this Store's root is held; settlement never takes it.
/// Always returns a receipt, even when the repository is missing, Git is unavailable, or the business call failed.
pub fn settle(store: &Store, guard: &LockGuard, event: &Event, policy: &dyn Policy) -> GitReceipt;

/// Convenience for dispatchers: settle after `result`, keep `result` unchanged and attach the
/// receipt. If `result` is an error with typed publications, the event is marked `failed`.
pub fn settled<T>(store: &Store, guard: &LockGuard, event: Event, policy: &dyn Policy, result: Result<T>) -> (Result<T>, GitReceipt);
```

Settlement happens after the handler returns and before the dispatcher drops the lock, so a handler cannot embed the final Git outcome in its own acknowledgement. Handlers report per-effect state with `EffectGit` (section 7); the single `GitReceipt` of the call is attached by the dispatcher to the reply.

Receipt (plain serializable data). This is the only call-level Git outcome type; no other Module defines one:

```rust
pub enum GitOutcome { Saved, Committed, Deferred, Unknown }

pub struct GitReceipt {
    pub outcome: GitOutcome,           // about THIS call's own publications only (derivation below)
    pub commit: Option<String>,        // full object id of the commit that holds THIS call's intent; None otherwise
    pub paths: Vec<String>,            // THIS call's exact relative paths: in `commit` when Committed, else the pending ones
    pub untracked: Vec<String>,        // up to 16 paths this call published without a journal entry
    pub reason: Option<Reason>,        // why THIS call is Deferred, or why it is Saved without an attempt
    pub attention: Vec<Attention>,     // things to look at even when Committed
    pub pending: Vec<PendingRef>,      // recoverable references, up to 16 (this call's first, then older)
    pub earlier: Vec<EarlierCommit>,   // up to 4 commits that this settlement also made for OLDER calls; extra facts, never this call's outcome
}
pub struct PendingRef { pub intent: String, pub phase: Phase, pub paths: usize }
pub struct EarlierCommit { pub commit: String, pub intents: usize, pub paths: usize }
pub enum Phase { Prepared, Published, Held, Committing, Unknown, Drifted }   // Held = Partial or Unsettled outcome stamp

pub enum Reason {
    NoRepository, NotIndependentRepository, GitUnavailable, PolicyDeferred, UnbornHead, DetachedHead,
    OperationInProgress, UnmergedPaths, IndexLocked, ForeignStagingOnPath, CommitNotCompleted,
    JournalFull, JournalUnavailable, JournalCorrupt, MessageBudget,
    NoMutation,                       // true no-op or nothing tracked: no commit is due
    IncompleteOutcome,                // partial or sync-uncertain call: held for explicit recovery
    UntrackedSibling,                 // some ordinary publication of this call is untracked, so the tracked rest is not committed alone
    RemovalBarrier,                   // same-operation guard only: an older held replacement of the same OperationId precedes this removal (section 5)
    IgnoredBusinessSibling,           // a business file or counter of this call is untracked or ignored by the user's own ignore rules, so the whole intent is deferred (section 5)
}
pub enum Attention { IndexRefreshFailed, TreeMismatch, HookChangedWorktree, NativeDrift, UnknownPending, UntrackedMutation, SyncUncertain }
impl GitReceipt { pub fn saved_only() -> Self; pub fn lines(&self) -> Vec<String>; }
```

`lines()` returns at most 6 plain English lines, each at most 200 bytes, with no path content beyond a count and the first path. Meaning of outcomes:

Derivation of the receipt. `outcome`, `commit`, `paths` and `reason` come only from THIS call's publications (`store.publications()` of this request) and THIS call's intent. They never come from other work the same settlement happened to commit. The rule, in order:

1. No eligible owned-file publication in this call (nothing published, or only `Tracking::NotApplicable` effects such as a directory or an ignored backup): `Saved`, `Reason::NoMutation`, empty `paths`, with `Attention::SyncUncertain` if a not-applicable effect has an uncertain sync.
2. This call published eligible files but has no tracked one (no repository, journal unavailable): `Saved`, `reason` names why, `untracked` names the paths, `Attention::UntrackedMutation`.
3. This call has both tracked and untracked eligible-file publications (not-applicable effects never count as untracked; a first create of a new home, with its directory and file, and a normalization of a legacy record with its ignored backup, are therefore ordinary fully tracked calls): its tracked intent is stamped `Partial`, never committed as a subset of the business save. `outcome` is `Deferred`, `reason` is `UntrackedSibling`, `untracked` names the untracked paths, `paths` the tracked ones, and `Attention::UntrackedMutation` is set. The save is intact; only explicit recovery commits it.
4. This call is `Partial` (error after publishing, or sync-uncertain): `Deferred` with `Reason::IncompleteOutcome`, `commit: None`, `paths` the published paths. This is the failure receipt: it is built for an `EventOutcome::Partial` call, carries the held intent in `pending`, and a `Failed` call (nothing published) has no receipt beyond `saved_only()`.
5. This call is a successful, fully tracked call: `Committed` only if this call's intent is in a commit that landed. Then `commit` is that commit and `paths` this call's paths. Otherwise `Deferred` (or `Unknown`) with the engine's reason for THIS intent, `commit: None`, `paths` this call's pending paths.

The call's intent is all or none. A commit holds every effect of this call's intent, including a counter file and the record it reserved, or none of them. There is no `Committed` receipt whose `paths` are a part of the call's business save, and no `Committed` for this call because an older call's intent landed.

Older work. When the same settlement also commits intents of OLDER successful calls that Git had deferred, the receipt lists them in `earlier` (commit, intent count, path count). Those are extra facts about earlier calls and never change this call's `outcome`, `commit`, `paths` or `reason`. A settlement may therefore return `Deferred` for this call (for example its path has foreign staging, or it does not fit the message budget) together with a non-empty `earlier`. Selection stays closed over connected path chains: an older intent that shares a path with a deferred current intent is deferred with it.

- Saved: files are on disk; no Git attempt, no repository, or every path of this call was untracked. `reason` says which, and `untracked` names the paths when the journal could not admit them, with `Attention::UntrackedMutation`: the write is intact, automatic persistence is unavailable for it, and only explicit recovery (`Preserve`) can later commit exactly those bytes.
- Committed: a commit exists that holds this call's whole intent. Attention may still list `IndexRefreshFailed`, `TreeMismatch`, `HookChangedWorktree` or `UntrackedMutation` (never together with a partial own selection, because that case is `Deferred`). These are never reset or rewritten.
- Deferred: files are saved and recorded as pending; Git state, policy or call completeness kept this call from committing. Nothing was changed in Git for this call.
- Unknown: an attempt may or may not have landed for this call (timeout, killed process, lost reply, journal failure after publication). Pending references are returned. Never retried blindly.

`lines()` shows this call's line first, then at most one line for `earlier`, so a reader sees the call's own outcome before the extra facts.

Engine obligations (observable behavior; the exact Git recipe is chosen by the fixture matrix and is not part of the contract):

- Commit unit is a whole intent: all paths of one handler call, including a counter file and the record it reserved, or none. If any ordinary publication of the call is untracked, the call is not committed as a tracked subset (receipt rule 3).
- Success evidence. When a call settles, every intent it wrote is stamped in the journal with the call's effective outcome (`Success` or `Partial`) before any Git step. An intent whose call never reached settlement (crash) stays `Unsettled`. Only `Success` intents are ever auto-committed. `Partial` and `Unsettled` intents are listed as pending with `Attention::UnknownPending` and wait for explicit recovery; a later success never certifies them and nothing replays their business write.
- Commit selection after a `Commit` decision: this call's `Success` intents plus every earlier `Success` intent still `Published` that Git had deferred (hook, signing, foreign staging, index lock, unborn or in-progress state). They go into one commit with one trailer block per intent. If two selected intents touch the same path the per-path chain must be contiguous (each `before` equals the previous `after`) and the commit holds the final bytes; a broken chain is drift and excludes the path's intents, which become `Drifted`.
- For a selected contiguous chain of successful publications on one path, compare current bytes to its final after digest; preceding after digests must match the next before digest. Do not call a superseded intermediate MCP publication native drift. Select whole intents and close selection over connected path chains, so message-budget paging cannot commit an old path image whose newer publication was excluded. If the whole connected chain cannot fit, defer it honestly. Drifted, unknown and untracked effects are excluded without dropping their evidence.
- Held predecessors on a shared path. A path that an earlier `Held` intent (a `Partial` or `Unsettled` call) touched does not block later successful calls. Example: a failed create published only an allocator counter step `c0 -> c1`; the next successful call observes the file at `c1` (its `before`), reserves `c1 -> c2`, and may be committed with its own exact final file image `c2`. The chain is contiguous because the later `before` equals the held `after`. The later commit contains the later call's own intent only; the held intent stays `Held` for its remaining paths, its entry on the shared path is marked superseded by the later intent, and it is never labeled completed, never committed with its untouched partial body or metadata, and never replayed. Recovery `Retry` of the held intent skips the superseded path and says so. If the later `before` equals neither the held `after` nor the held `before`, the chain is broken and the path is `Drifted`.
- Successful deferred chains. Earlier `Success` intents that Git deferred and a later `Success` intent on the same path form one contiguous chain (for example a document body written twice). The commit holds the latest image; every earlier intent in the chain gets its own `Agent-Tasks-Effect` line with its true `after` digest and an `into=` field naming the final digest, so each operation keeps true prior proof that it published without claiming Git holds the intermediate bytes. Selection is closed over connected chains (previous bullet), so a chain is committed whole or deferred whole.
- Separate fates in one settlement. Selection is per intent, so one settlement may commit older eligible intents while this call's own intent is deferred, for example by a user-staged change on one of its paths, by the message budget, or by a held connected chain. The older commits are reported only in `earlier`; this call's receipt stays `Deferred` with its own reason. If this call's own intent alone exceeds the message budget it is `Deferred` with `MessageBudget` and is not split.
- Same-operation removal guard (limited scope). The engine keeps one narrow guard for callers that reuse a single `OperationId` across steps: an intent containing a `Removed` effect for operation `O` is committed only if every non-removal effect of that same `O` that lives in an OLDER intent is already `Committed` or is selected into the same commit; otherwise that intent is `Deferred` with `RemovalBarrier`, and the removal stays saved on disk. This guard is not cross-action proof for compaction. M-005 uses a different `OperationId` per action (`cp:<id>:r<n>:A01`, `A02`, `A03`), so a replacement under `A01` and a removal under `A03` never share an operation, and nothing in the engine, `effect_status` or `call_intents` is claimed to connect them. M-005 owns the proposal-aware pre-removal gate, and it passes when either (a) every older replacement it depends on has complete `Committed` proof (`effect_status` `EffectGit::Committed`, or `committed_original`/`Locator` for the bytes), or (b) all of the new replacement effects it depends on are tracked and sit in the SAME whole atomic intent as the removal in the current call (`call_intents` with `all_tracked`, and the same `Publication.intent`). The engine itself keeps the atomic whole intent, the call-scoped truthful receipt and the explicit `Retry` composition (naming older and current intents together); it adds no implicit cross-action dependency list or coordinator, and it does not require older pending replacements of a fresh apply to be committed before the handler ends.
- Complete tree proof. After a commit the verification covers every path the commit changed, replacement and removal together, in an ordinary clone; it never checks a subset. Git runs with lazy fetching disabled and no network as an ordinary protection, so an object that cannot be read makes the affected proof `Unknown` and never `Committed`. No special handling of other clone shapes is added.
- User ignore rules. The user's own `.gitignore`, `.git/info/exclude` and global excludes are preserved and never overridden: no forced staging (`add -f`), no change to ignore files, no changed Git configuration. `Tracking::NotApplicable` is not a way around them. If a business file or counter of this call's intent (a record body, metadata, counter file or document) is ignored by a user pattern or is otherwise a real untracked-and-unstageable sibling, the engine classifies it as an ignored or untracked business sibling and defers the WHOLE current intent with `Reason::IgnoredBusinessSibling`; it never commits the call's other files (the new record) alone. The data stays saved, `Attention::UntrackedMutation` names the path, and the older eligible intents may still land under `earlier`. A directory or a built-in administrative path that a user pattern happens to ignore stays `NotApplicable` and is not a sibling.
- Unrelated staged and dirty files, including ones on neighbor paths, keep their content and staged state. A user-staged change on a path of the intent defers that intent with `ForeignStagingOnPath`.
- The user's Git configuration, hooks (pre-commit, commit-msg, post-commit) and signing apply unchanged. No `--no-verify`, no signing override. Identity comes from user configuration; if none exists the engine supplies `agent-tasks <agent-tasks@localhost>` only for that commit.
- Never commits on an unborn or detached HEAD, during a merge, rebase, cherry-pick, revert, bisect or am, with unmerged entries, inside a nested or linked repository, or while the index is locked; each gives `Deferred` with the matching reason. A stale index lock is reported, never deleted.
- Argv is exact: paths follow `--`, literal pathspec mode is on, inherited `GIT_DIR`, `GIT_WORK_TREE`, `GIT_INDEX_FILE`, `GIT_COMMON_DIR` and prompt/editor variables are cleared. Output is captured with a 1 MiB stdout and 64 KiB stderr cap; one command has 10 seconds and one settlement 30 seconds; the child process group is killed on expiry. Stderr is classified, never echoed.
- Commit message: fixed subject by event class, the references, and trailers (below). No user text. At most 64 KiB of actual serialized message bytes: the engine adds whole eligible intents in journal order while the serialized message stays within the cap and defers the rest with `Reason::MessageBudget`; it never truncates a trailer, path or reference, and a deferral never stops an ordinary business save.
- After a commit, the engine verifies the parent, the exact changed path set (deletions included) and each blob against the recorded digest. Mismatches become `Attention`, not rollbacks. A landed commit with a failed index refresh yields `Committed` with `IndexRefreshFailed`.
- Unknown outcome after timeout or kill: reconcile through HEAD, trailers and the tree before any further attempt.
- Registration commits its bootstrap files itself in one bootstrap commit before any journal exists; the engine treats that HEAD as baseline and never commits them again, and registration never calls `settle`.

Trailers written by the engine into every commit it creates (one block per intent):

```
Agent-Tasks-Intent: PG-<24 hex>
Agent-Tasks-Effect: <operation or -> <created|replaced|removed> <path> <before-sha256 or -> <after-sha256 or -> [into=<final-sha256 or ->]
Agent-Tasks-Adopted: PG-<24 hex>        (only when recovery adopted that intent, section 7)
```

`<path>` is the relative path with every byte outside `A-Za-z0-9._/-` written as `%XX` (uppercase hex), so spaces, colons and non-ASCII never split fields. One Effect line per file path; directories are not tracked by Git and have no line. The optional `into=` field appears only on an effect that a later successful intent replaced inside the same commit; it carries the digest of the committed bytes (`-` when the chain ended in a removal) and the line's own `after` digest stays the effect's true bytes. These lines are the durable operation evidence that survives journal cleanup and works on any clone that has the history.

## 6. Pending facts and snapshot, committed-bytes proof, locators, preservation

Read-only, lock-free, bounded, never commits:

```rust
pub struct PendingSummary {
    pub complete: bool,                // false if the journal or repository could not be fully read
    pub version: String,               // exact snapshot, see below
    pub facts: PendingFacts,
    pub refs: Vec<PendingRef>,         // up to 16, oldest first
    pub warnings: Vec<String>,
}
pub fn pending(store: &Store) -> PendingSummary;   // infallible; unreadable evidence becomes complete=false

/// The snapshot digest `PendingSummary.version` carries, recomputable at any time.
pub fn pending_version(store: &Store) -> String;
```

`version` is a sha256 over the immutable observations that matter to recovery and nothing time- or offset-based: the version of the journal file bytes (or its absence), the attached branch name or the detached state, HEAD's object id (or unborn), and for every path named in the journal its current bytes version or absence. A change in the journal, a new commit, a branch change or drift of any pending file changes it. The `git_recovery` operation takes this value as its `Common.version` and checks it again under the write lock (`stale` otherwise, nothing changed). Context and status show it so an agent can pass the exact value they saw.

Committed-bytes proof. A pending, deferred, ignored or working-tree file is never proof:

```rust
/// A committed object that holds exact bytes, reachable from the current branch.
pub struct Locator { pub commit: String, pub blob: String, pub relative: String, pub sha256: String, pub len: u64 }
impl Locator {
    /// Stable wire form, at most 256 bytes: "git1:" + commit + ":" + blob + ":" + path with every byte
    /// outside A-Za-z0-9._/- written %XX. commit and blob are 40 or 64 lowercase hex characters, so the
    /// first two colons are unambiguous and everything after the third colon is the path. Longer than
    /// 256 bytes is refused with `locator_too_long`, never truncated.
    pub fn encode(&self) -> Result<String>;
    pub fn parse(text: &str) -> Result<LocatorRef>;   // commit, blob, relative; sha256 and len are re-derived by reading
}

pub struct Item<'a> { pub relative: &'a str, pub sha256: &'a str, pub len: u64, pub at: Option<&'a str> }

pub enum NotCommittedReason { Untracked, Ignored, Dirty, UnbornHead, DetachedHead, NotIndependentRepository, NoRepository }
pub enum Proof {
    Committed(Locator),                           // blob bytes hash to Item.sha256 in a commit reachable from the attached HEAD
    NotCommitted(NotCommittedReason),
    Mismatch { committed_sha256: String },        // tracked, but the committed bytes differ and the working file differs too
    Unknown(String),                              // timeout, oversize, unreadable object: never treated as proof
}

/// Prove one original by digest and length. Read-only Git, no lock needed, infallible: every
/// failure is a variant. With `at`, prove it in that commit instead of the last commit touching the path.
pub fn committed_original(store: &Store, item: &Item<'_>) -> Proof;

/// Prove several originals. Err `not_committed` names every relative path that is not `Committed`.
pub fn verify_committed(store: &Store, items: &[Item<'_>]) -> Result<Vec<Locator>>;

/// The shape M-001's `Recoverable::locate(store, relative, version, bytes)` needs. `version` must equal
/// `store.version(relative, Some(bytes))` (else `invalid`). Some(locator.encode()) only for `Proof::Committed`;
/// None for every other proof; Err `locator_too_long` when the wire form would exceed 256 bytes.
pub fn locate_committed(store: &Store, relative: &str, version: &str, bytes: &[u8]) -> Result<Option<String>>;

/// Read the exact bytes back from a locator, after checking the commit is reachable from the attached HEAD.
/// Bounded by `cap`. Err `not_committed` when it is no longer reachable.
pub fn read_committed(store: &Store, locator: &str, cap: usize) -> Result<Vec<u8>>;
```

Proof rules: the repository must be an independent repository whose top level is the store root; HEAD must be attached to a branch with at least one commit. The commit is the last commit on the path from HEAD (`rev-list -n 1 HEAD -- path`) unless `at` names one; its blob is read through `cat-file` with a cap and hashed; the hash and length must equal the item's. `Dirty` means HEAD holds different bytes while the working file equals the item (uncommitted change); `Mismatch` means HEAD differs and the working file differs from the item too. Ignored status is decided by `check-ignore`. A locator is as durable as the commit stays reachable: rewriting history or pruning unreachable objects can destroy it, and this module never does either.

Explicit preservation (exceptional recovery, never automatic, never a substitute for normal persistence): commit the exact bytes a caller authorizes for named files, tracked pending or untracked, so originals become recoverable or an untracked write becomes attested history.

```rust
pub struct PreservePath<'a> { pub relative: &'a str, pub sha256: &'a str, pub len: u64 }   // the exact bytes the caller authorizes
pub struct PreserveRequest<'a> { pub paths: &'a [PreservePath<'a>], pub operation: Option<&'a OperationId> }  // up to 32 paths
/// Commits exactly those paths with the lock held, only if each file's current bytes equal the authorized digest.
/// Allowed for unattested and untracked files because the caller is the explicit authorization; the
/// engine's trailers mark them adopted. Never uses the policy.
pub fn preserve(store: &Store, guard: &LockGuard, request: PreserveRequest<'_>) -> Result<Vec<Locator>>;   // errors: preserve_blocked (with Reason), stale (bytes changed), not_locked
```

Preservation fails (`preserve_blocked`) rather than deferring, because its caller needs the proof. M-005 has no preserve operation of its own; the explicit `git_recovery` tool (section 11) is the only exception.

## 7. Operation provenance and `effect_status` (M-005)

Purpose: after an interruption, a caller that applies a multi-step operation under a deterministic `OperationId` asks M-004 what actually happened, from M-004's evidence only, without a second journal and without trusting equal bytes. The same evidence answers idempotency and provenance of committed operations. Callers that need this evidence publish with `Attest::Required`; an untracked optional write can never become attested.

`effect_status` is a complete-operation oracle. It returns every effect that M-004's own evidence attests for the operation, whether or not the caller predicted it. The caller's expected list is a set of assertions about the effects it can compute itself (a known path, kind and body digest); it is not the list of effects that are allowed. A caller whose operation also produces generated effects (new identifiers, timestamps, counters, metadata records that another Module writes internally) does not need to know their paths or digests: they come back in the returned set for the caller to validate against its own closed rules.

```rust
/// An assertion about one effect the caller can compute exactly: path, kind and the digest of the
/// bytes it expects. Generated effects are never listed here.
pub struct ExpectedEffect { pub relative: String, pub kind: EffectKind, pub after_sha256: Option<String> }  // None for Removed

/// One attested effect of the operation, as M-004 recorded it when it happened.
pub struct PathEffect {
    pub relative: String, pub kind: EffectKind,
    pub before_sha256: Option<String>, pub after_sha256: Option<String>,
    pub asserted: bool,                    // matched one of the caller's ExpectedEffect values; false = same-operation effect the caller did not predict
    pub superseded_into: Option<String>,   // None = not superseded; Some(64-hex digest) = replaced; Some("-") = superseded by removal. Never collapse removal into None.
    pub git: EffectGit,                    // state of this effect's own intent
    pub adopted: bool,                     // this effect's intent was adopted by explicit recovery
}

/// Per-effect Git state at the time of the answer. The call-level outcome is GitReceipt (section 5).
pub enum EffectGit { Untracked, Pending(PendingRef), Committed { commit: String }, Unknown(PendingRef) }

/// The complete attested set for one operation. `complete` is true only when the journal was readable
/// and the bounded history scan finished, so a caller can tell "no more effects" from "could not look".
pub struct EffectReceipt { pub operation: OperationId, pub paths: Vec<PathEffect>, pub complete: bool }

pub enum UnknownReason { UnattestedEqualBytes, CrashAfterRename, JournalUnavailable, HistoryScanIncomplete, NoRepository, ConflictingRecord }
pub enum ForeignReason { OperationMismatch }       // an assertion's path is recorded for this operation with a different kind or digest

pub enum EffectStatus {
    Attested(EffectReceipt),                                          // every assertion attested, no unproven same-operation row; the full set is returned
    Partial { attested: EffectReceipt, missing: Vec<String> },        // some assertions attested, the listed ones never published
    NotPublished,                                                     // no attested effect, no unproven row, no look-alike bytes
    Unknown { attested: EffectReceipt, observed: Vec<(String, Option<String>)>, reason: UnknownReason },  // unproven rows: path and current sha256
    Foreign { reason: ForeignReason },
}

/// Read-only, lock-free (callers normally hold the write lock), bounded, infallible.
pub fn effect_status(store: &Store, operation: &OperationId, expected: &[ExpectedEffect]) -> EffectStatus;

/// The receipt for effects this request just published under `operation` (from `store.publications()`
/// and the journal). Err `not_found` when the operation has no event in this request.
pub fn receipt(store: &Store, operation: &OperationId) -> Result<EffectReceipt>;

/// Membership of the current call's publications, derived from `store.publications()` (already typed
/// data, nothing new is recorded). Pure and lock-free. `all_tracked` is true only when there is at least
/// one eligible owned-file publication, every eligible file publication is `Tracked`, and no event of the
/// call (directories included) has an uncertain sync. `not_applicable` counts directories, ignored
/// backups, locks and temps; they never make `all_tracked` false and are never counted in `tracked`/`untracked`.
pub struct CallIntents { pub intents: Vec<String>, pub tracked: usize, pub untracked: usize, pub uncertain: usize, pub not_applicable: usize, pub all_tracked: bool }
pub fn call_intents(store: &Store) -> CallIntents;

/// Read-only evidence for one journal intent, for a caller deciding a barrier. Plain data from the
/// private journal; `committed` is the commit id once Committed. Bounded to 256 effects. None when the
/// intent is unknown or already pruned (then `effect_status` and the history trailers are the evidence).
pub struct IntentEffect { pub relative: String, pub kind: EffectKind, pub operation: Option<String>, pub after_sha256: Option<String>, pub committed: Option<String> }
pub struct IntentView { pub intent: String, pub class: EventClass, pub outcome: IntentOutcome, pub phase: Phase, pub effects: Vec<IntentEffect> }
pub enum IntentOutcome { Unsettled, Success, Partial }
pub fn intent_view(store: &Store, intent: &str) -> Option<IntentView>;
```

Current-call identity for M-005. The identity of "this call's whole atomic intent" is `Publication.intent` on every event of the request, which `call_intents` summarizes. An apply that needs to know whether its replacements and its removals will commit together checks that `all_tracked` holds and that the removal events share the replacements' `intent`; that is condition (b) of M-005's pre-removal gate in section 5. For older replacements, `effect_status` returns each effect's `EffectGit` (`Pending`, `Unknown` or `Committed`) and `intent_view` shows the owning older intent and whether it is `Held`; that is condition (a). M-005 owns the gate and its proposal-aware dependency knowledge across its different per-action operation identities. M-004 adds no coordinator, daemon, dependency list or second journal.

Algorithm. M-004 first collects every row it holds for the operation identity, then judges the caller's assertions against that collection:

1. Journal rows. All entries carrying this operation identity (at most 64 intents, so cheap). A `Published`, `Committing`, `Committed` or adopted entry is an attested effect. A `Held` intent's entries that were published are attested effects too, with `git` showing their pending state; being held never turns published bytes into "not published" and never marks the business call complete.
2. History rows. Whenever a repository exists, read bounded Git history from the attached HEAD for every engine trailer `Agent-Tasks-Effect:` carrying this operation, because the journal may have been pruned after commit and only the history can say that no further effect exists. A history row is attested only if its commit is reachable and the commit tree holds the bytes the line names (the `after` digest, or the `into` digest when the line says the effect was superseded in that commit, or no entry for Removed). A row whose tree disagrees is a `ConflictingRecord`. A scan that hits its time or size bound makes `complete = false` and the answer `Unknown(HistoryScanIncomplete)`, never `NotPublished`.
3. Unproven rows. A `Prepared` entry whose current bytes equal the planned after digest and carry no confirmation is `Unknown(CrashAfterRename)`; the recorded device and inode (Unix) go into the evidence recovery shows, but never promote the entry. A `Prepared` entry whose file still equals the before bytes is not published and contributes nothing. A journal that cannot be read gives `Unknown(JournalUnavailable)`.
4. Look-alikes. For each assertion with no row anywhere, current bytes equal to the asserted after bytes (or an absent file for an asserted removal) are `Unknown(UnattestedEqualBytes)`. This is the case equal bytes never certify; it also covers a path written untracked. No repository means no rows at all: equal bytes give `Unknown(NoRepository)`, otherwise nothing is published.
5. Assertions. An assertion is satisfied when an attested row has the same path, kind and after digest; that row is returned with `asserted = true`. An attested row for an asserted path with a different kind or digest is a conflict and the answer is `Foreign(OperationMismatch)`.
6. Extra rows. Attested same-operation rows that no assertion names are returned with `asserted = false`. They are not `Foreign` merely because the caller did not predict them, and they are not certified by equal bytes either: they are attested only because M-004's journal or trailer recorded them as published by this operation. The caller validates the complete returned set against its own closed rules (for example M-002's metadata and current post state) before adopting the action.

Combining: any `Foreign` wins; else any unproven row or look-alike gives `Unknown` (still returning the attested part and the observed paths with their current digests); else all assertions satisfied gives `Attested` with the complete set; some satisfied gives `Partial` with the missing assertions' paths; no attested row gives `NotPublished`. With an empty assertion list the answer reports what the operation did: `Attested` with its set, or `NotPublished`. `Attested` means "every asserted effect happened and nothing about the operation is unproven"; it does not mean the caller's whole business step is finished. A caller that needs generated effects to exist keeps its own state `Partial` until the returned set proves them. A path changed by someone else after a genuine attested publication is still attested (it was published); post-state checks belong to the caller. No new serializer, wildcard ownership, planning transaction or second journal is involved.

Superseded effects. When two successful intents on one path are committed together (section 5) the earlier intent's effect is still an attested effect of its own operation with its true `after_sha256`, but the commit holds only the final bytes. Its row therefore carries `superseded_into`, and the history line records `into=<digest>`. M-004 never claims the earlier bytes are recoverable from Git, and it never rewrites the earlier operation's digest to the later blob.

Idempotency: `Attested` is the prior receipt, and the caller adopts it instead of publishing again. `publish_with` adds the `operation_repeat` guard for journal-visible repeats; after journal cleanup the caller asks `effect_status` first, which M-005's recovery table already does. A different operation with identical bytes has a different identity and is never adopted.

Adoption by recovery: the `Adopt` recovery operation (section 11) may promote an `Unknown` journal entry to attested with `adopted = true` after explicit authorization. Only then does `effect_status` answer `Attested` for it, and the engine commit it ends up in carries an `Agent-Tasks-Adopted` trailer for that intent, so the receipt read back from Git also shows `adopted = true`. Paths that were untracked have no entry to adopt; they are handled by `Preserve`.

Mapping from M-005's proposed `EffectPort` to the actual functions (M-005 conforms to these types and defines none of its own for the same facts):

| M-005 need | Actual M-004 function |
|---|---|
| `publish_new` | `Store::publish_with(Publish { observed: None, operation: Some(op), attest: Attest::Required, .. })`, then `receipt(store, op)` |
| `publish_replace` | `Store::publish_with(Publish { observed: Some(bytes), operation: Some(op), attest: Attest::Required, .. })` |
| remove | `Store::remove(Remove { operation: Some(op), attest: Attest::Required, .. })` |
| `effect_status` | `persist::effect_status(store, &op, &expected)` |
| `committed_original` | `persist::committed_original(store, &Item)` returning `persist::Proof` |
| `OperationId`, `EffectKind` | `store::OperationId`, `store::EffectKind` |
| `EffectReceipt`, `PathEffect`, `EffectStatus`, `GitOutcome` | `persist::EffectReceipt`, `persist::PathEffect`, `persist::EffectStatus`, `persist::GitOutcome` / `persist::GitReceipt` |
| event class | `EventClass::CompactionPropose \| Revise \| Review \| Apply \| Withdraw` |

M-005 calls the provider functions with the same Store and LockGuard. It defines no duplicate operation, receipt, proof or port-trait family; private data translations may convert a validated observation to its proposal fields.

## 8. History eviction and destructive steps (M-001, M-005)

A caller may drop uncommitted revision or original content only after `committed_original` returned `Proof::Committed` (or `locate_committed` returned `Some`) for each exact original, and it must retain the locators. `Phase::Published` or `Deferred` pending work is never preservation proof. M-001's `Recoverable::locate` is implemented as a one-line forward to `locate_committed`; M-001 owns the trait, M-004 owns the proof. A destructive M-005 step additionally requires `Attest::Required` for its own publications, so it refuses before effects if its proof cannot be retained.

## 9. Markdown, registration and bootstrap notes (M-002)

Documents are raw bytes: `publish_with` never parses, normalizes line endings or strips a BOM. `read_exact(relative, cap)` returns the same bytes. Paths are ASCII managed paths under `docs/` and the root README; unsupported native names are the caller's coverage problem. `list_dir` and `own_temp_name` give the `docs/` inventory its listing and its temp warnings.

Identity-bearing relocate. A document relocate has several steps (create the body at the target, replace the record, remove the body at the source). When M-002 runs it under a deterministic `OperationId` with `Attest::Required`, it resumes after an interruption through `effect_status` and `receipt` (section 7), exactly as M-005 does, and consumes the same types: `ExpectedEffect` assertions for the body effects whose path and digest it can compute, the complete attested set back (including the generated record effect, flagged unasserted, which M-002 validates against its own closed metadata and current post state), and the call-scoped `GitReceipt` of section 5. An ordinary one-step save stays `Attest::Optional`. The record and the body of one document write are published in one call and commit as one whole intent.

Registration is outside this engine. Fresh and partial registration commit whatever bootstrap files the registration owner lists in its single bootstrap commit; M-002 proposes adding `docs/.gitkeep`, so this document asserts no fixed file count. Registration uses `create_dir` and its own bounded Git call. The engine starts afterwards, finds a HEAD that already contains those paths and nothing pending from bootstrap, and never commits them.

## 10. Allocators, counters, `kr-paths` and inventory grammar (M-001, M-005)

State files are plain owned files published with `save` under the single lock, `Attest::Optional`. A counter file such as `.agent-tasks/knowledge.yaml` is committed like any other eligible tracked path. Reservation then record publication are two events in one intent; a failure between leaves a visible gap and a pending reference, never a recycled number. `backups/` normalization copies and lock files are ignored by the repository's `.gitignore` and are excluded from intents. M-004 consumes M-001's `kr-paths` only for the list of paths M-001 may publish and for its statement that it never runs Git; ownership of a published file still comes from the journal and trailers, never from that list.

M-001 owns the canonical `allocation_version(store)` across all six homes (and its counter file); M-004 computes nothing of it and the existing work `Store::allocation_version` is unchanged.

Exact supported grammar of the shared inventory helpers:

- `kind_inventory(directory, prefix)` lists one owned directory without recursion. `prefix` is an ID prefix of the form `[A-Z]{1,3}-`. An entry is an ID when it is a regular file named `<prefix><digits>.yaml` with the canonical digit form of `model::number` (at least three digits, no leading zeros beyond that). A symlink or non-regular entry with an ID name is listed and sets `complete=false`. Entries beyond `MODULE_CAP` set `complete=false`. Any other name, including every subdirectory, sets `complete=false` with a warning; nothing is deleted or ignored silently.
- Own publication leftovers are warnings and do not set `complete=false`: for the three existing work directories `modules`, `epics` and `atomics` the recognition is bit for bit today's rule (only `M-` canonical stems), so work coverage is not weakened or widened, and a regression test keeps an orphan `.E-001.yaml.tmp-<pid>-<seq>` in `epics/` incomplete exactly as now. For every other directory a leftover is own when `own_temp_name(name)` holds and its stem is `<prefix><digits>.yaml` for the requested prefix.
- Nested homes are not covered by `kind_inventory`. M-005 owns validation of `compactions/CP-NNN.yaml` and `compactions/CP-NNN/rN/A-NN.md`; M-002 owns `docs/`. They use `list_dir` for listing and `own_temp_name` for leftovers, never `fs::read_dir` of their own.

## 11. Recovery operation and dispatcher duties (M-003)

M-003 owns the registry entry, input schema aggregation, routing, shared dispatcher and rendering. The tool purpose is `git_recovery` and is exceptional: pending, unknown, drifted or explicitly authorized work only. The normal commit path is automatic settlement, never a checkpoint call. M-004 provides the producer payload in the same shape as the other producers; it takes no lock of its own.

```rust
// src/tools/recovery_ops.rs
/// Closed wire payload; `input::Common` (project, version, actor) is M-003's and carries
/// `version = PendingSummary.version`, checked again under the write lock.
#[derive(Deserialize, JsonSchema)]
#[serde(tag = "op", rename_all = "snake_case", deny_unknown_fields)]
pub enum RecoveryOp {
    Reconcile { intents: Vec<String> },          // up to 16 IntentId values
    Retry     { intents: Vec<String> },
    Adopt     { intents: Vec<String> },          // only Unknown entries with matching bytes; never drifted
    Release   { intents: Vec<String> },          // stop tracking, files untouched
    Preserve  { paths: Vec<PreservePathIn> },    // up to 32 exact caller-authorized byte identities
}
pub struct PreservePathIn { pub relative: String, pub sha256: String, pub len: u64 }

pub const DESCRIPTION: &str;                     // tool description text for the registry owner

/// Runs under the dispatcher's single lock; never locks. Refuses `stale` before any effect when
/// `common.version` differs from `pending_version(store)` taken under that same guard. Never replays
/// a business mutation. Returns M-003's existing `work::Ack` (no second outcome type): `target` is
/// only the presentation label "Git recovery" and never a canonical reference, so it must not be
/// passed to `get_context ref=`; M-003 selects an explicit recovery presentation discriminator
/// (not an invented ref) and prints the pending route as `get_context(project)` with `ref`
/// omitted, which is the Project route. `version` is the pending snapshot after the operation,
/// `changed` says whether anything was committed, released or adopted, and the Ack notes carry
/// `GitReceipt::lines()` plus one plain line per change. M-003 owns the Ack shape and any notes field.
pub fn execute_locked(store: &Store, guard: &LockGuard, common: &input::Common, op: RecoveryOp,
                      effects: &mut Vec<String>) -> Result<work::Ack>;
```

Failures are `store::Error` with stable codes `stale`, `invalid`, `not_locked`, `preserve_blocked` and `recovery_blocked`. The message names up to four pending references and their phases, and the call's effects ledger carries one line per remaining pending reference, so unknown and held facts are never lost on an error.

- Reconcile: read-only Git inspection that resolves `Unknown` and `Committing` entries using HEAD, trailers and the tree.
- Retry: commit the named `Published` or `Held` intents through the engine rules without a policy decision. For a `Held` intent (earlier partial, failed or unsettled call) this is the explicit authorization to commit exactly the recorded bytes; the commit carries the `Agent-Tasks-Adopted` trailer and nothing claims the business call completed.
- Adopt: explicit authorization to treat an `Unknown` intent with matching bytes as published, reported as adopted with its evidence.
- Release: stop tracking named intents without committing; files are left untouched.
- Preserve: section 6; for any named path, tracked pending or untracked, whose current bytes equal the authorized digest.

Alignment with M-003's registered tool surface (`registered-tool-surface` revision 2, commit 79f1476, `docs/contracts/registered-tool-surface.md`):

- The seam is `execute_locked(store, guard, common, op, effects) -> work::Ack` for every producer. M-004 has exactly one public entry point for the recovery tool, `execute_locked`; the engine's `recover` is an internal function of the same module and is not exported, so there is no second route and no second lock.
- Notes: the recovery Ack carries at most 8 notes. Receipt lines (`GitReceipt::lines()`, at most 6) come first, then at most 2 plain change lines. `Ack.refs` holds up to 16 references of 256 bytes each. Producers never add a Git line to their own Ack; the dispatcher adds the one receipt of the call. For `git_recovery` the dispatcher skips ordinary settlement because the recovery receipt is its settlement; if it settled anyway, `EventClass::Recovery` makes the policy `Skip`.
- Event mapping the dispatcher uses: `Ok` with `changed = true` gives `EventOutcome::Success`; `Err` with typed publications gives `EventOutcome::Partial`; `Err` without publication does not settle; `Ok` with `changed = false` and no typed publication does not settle. `settle` also tolerates being called for a true no-op: it returns a `Saved` receipt with `Reason::NoMutation`, writes nothing to the journal and runs no Git. M-003's "failed flag" is this three-state outcome in the provider's type; no boolean exists.
- `Event.refs` are advisory. They may be empty on failure; identity is the journal intent, never the reference list.
- `PolicyInput.tracked` is derived inside `settle` from this request's `store.publications()` and the journal, so the dispatcher passes no changed flag.
- Owner confirmation recorded: knowledge records, managed documents and compaction records and effects count as "document, task or project record" for the commit policy, together with the existing work mutations.

Dispatcher duties (M-003 owns the hoist): the existing work handlers acquire the root lock themselves today (`plan`, `record`, review and creation paths in `src/tools/work.rs`). M-003 moves lock acquisition into one shared dispatcher that holds a single `LockGuard`, calls the handler's locked form, then calls `settled` (or `settle`) with `EventClass::Work`, before the guard drops. Knowledge, document, compaction and recovery handlers are called the same way through their `execute_locked` forms. Settlement runs even when the handler returned an error with typed publications; the dispatcher sets `EventOutcome::Success` only for `Ok`, `Partial` for an error after at least one publication, and `Failed` for an error with none. Reads, true no-ops and registration never settle for a commit. The dispatcher renders `GitReceipt::lines()` in the mutation reply, shows `pending(store)` (including its `version`) in context and status, derives the published flag from `store.publications()`, and never calls `settle` from a read. M-004 never acquires the root lock anywhere. Handlers, including M-005 `apply`, return their own acknowledgement without a Git outcome. Registration keeps its own lock order and its one bootstrap commit and is not routed through `settle`.

## 12. Type ownership: one definition, one location

| Type or function | Owner location |
|---|---|
| `OperationId`, `Attest`, `Publish`, `Remove`, `Observed`, `EffectKind`, `Durability`, `Tracking`, `UntrackedReason`, `Publication`, `EntryKind`, `DirEntry`, `DirListing`, `ABSOLUTE_CAP`, `own_temp_name`, `Store::{read_exact, publish_with, remove, create_dir, ensure_parents, publications, list_dir, kind_inventory}` | `store` (src/store.rs) |
| `EventClass`, `Event`, `PendingFacts`, `PolicyInput`, `Decision`, `DeferReason`, `Policy`, `CommitAfterSuccess`, `production_policy`, test-only `DeferAll`, `settle`, `settled` | `persist` |
| `GitOutcome`, `GitReceipt`, `EarlierCommit`, `PendingRef`, `Phase`, `Reason`, `Attention`, `PendingSummary`, `pending`, `pending_version` | `persist` |
| `ExpectedEffect`, `PathEffect`, `EffectGit`, `EffectReceipt`, `UnknownReason`, `ForeignReason`, `EffectStatus`, `effect_status`, `receipt`, `CallIntents`, `call_intents`, `IntentEffect`, `IntentView`, `IntentOutcome`, `intent_view` | `persist` |
| `Locator`, `Item`, `NotCommittedReason`, `Proof`, `committed_original`, `verify_committed`, `locate_committed`, `read_committed`, `PreservePath`, `PreserveRequest`, `preserve` | `persist` |
| `RecoveryOp`, `PreservePathIn`, `DESCRIPTION`, `execute_locked` | `tools::recovery_ops` |
| `input::Common`, `work::Ack`, the shared mutation scope and dispatcher, tool registration, rendering | M-003 |
| `Recoverable`, `allocation_version`, knowledge allocator | M-001 |
| CP records and private proposal data translations | M-005 |

No other Module defines an operation identity, a Git outcome, a publication receipt, an effect status or a proof type, and none re-exports one under a second path. Module tests use the actual temporary Store and provider code; any unit-only substitute must use these actual provider types and never counts as composition proof.

## 13. Checks each consumer can run

Every boundary has correct, mutant and restored observations tied to the restored candidate and this revision:

- store boundaries: replace with changed bytes gives `stale` and no write; create over existing gives `stale`; an oversized body gives `capacity`; a symlink gives `file_type`; a forced parent-sync failure gives `durability_unknown` with `SyncUnknown`; removal of changed bytes gives `stale` and keeps the file; a repeated operation and path gives `operation_repeat`. Mutants: skip the observed-bytes recheck, drop the cap check, follow links, allow the repeat.
- attestation boundaries, both modes, real repositories: with the journal full of non-committed entries an `Optional` write succeeds with `Untracked(JournalFull)` and `Attention::UntrackedMutation` while every old entry is byte-identical afterwards, and a `Required` write refuses `attestation_unavailable` with no effect and no changed journal; the same pair for an unwritable journal, a corrupt journal (kept, not rewritten) and no repository; a failure after publication gives `UnknownAfterPublication` for `Optional` and `attestation_unknown` for `Required`; business validation still refuses before effects in both. A later settlement never commits the untracked file and `effect_status` on it is `Unknown`. Mutants: block optional writes when full, let required proceed untracked, prune an uncommitted entry, certify by equal bytes.
- journal and `effect_status`: kill after intent, after rename, before the Published mark; each reconciles to the table in section 4. Same operation after journal pruning is answered from the commit trailer. Equal bytes without a record give `Unknown`. A conflicting digest on an asserted path gives `Foreign`. Mutants: treat equal bytes as ownership; answer from the journal only; ignore the digest in the trailer.
- complete-operation oracle: an operation that publishes its asserted body plus a generated metadata record and a counter whose paths and digests the caller never listed returns `Attested` with all three effects, the body `asserted = true` and the others `asserted = false`, none `Foreign`; with only the body published it is `Attested` with a one-element set and the consumer's own rules keep its step partial; with the body published and the metadata row `Prepared` with matching bytes it is `Unknown(CrashAfterRename)` returning the attested body; a metadata row whose commit tree disagrees is `Unknown(ConflictingRecord)`; a conflicting digest on the asserted body path is `Foreign`; a pruned journal with a history scan cut short is `Unknown(HistoryScanIncomplete)` with `complete = false`; an empty assertion list reports the operation's own set. Mutants: return only asserted rows; mark extra rows `Foreign`; certify a look-alike by equal bytes; report `complete = true` after a truncated scan.
- chain semantics, both testable with real repositories: (a) two successful document writes to one path are deferred by a failing hook, a later successful mutation commits the latest image in one commit with both intents' trailers, the earlier effect shows its own true digest with `into=` and `superseded_into`, and no earlier-bytes recovery claim is made; a chain ending in removal returns superseded_into = Some("-") distinctly from None and verifies the committed path is absent; (b) a failed create leaves only the allocator step `c0 -> c1` held, the next successful reservation observes `c1`, is committed with its own `c2` image without blocking, the held intent stays `Held` with that path superseded, nothing labels it completed, its untouched partial body or metadata is not committed, and a broken `before` makes the path `Drifted`. Mutants: block every later call on a held path; label the held intent completed; commit the held intent's partial body; rewrite an earlier digest to the final blob.
- committed proof: committed, untracked, ignored, dirty, mismatched, unborn and detached fixtures each give their proof; a locator with a colon, a space and non-ASCII in the path round-trips; a path that makes the wire form exceed 256 bytes gives `locator_too_long`. Mutants: accept the working tree, accept a pending commit, truncate the locator.
- recovery: `RecoveryOp` round-trips through its schema and decoder, rejects unknown fields, and refuses a stale `Common.version` under the lock with no effect; the snapshot changes when the journal, HEAD or a pending file changes and not otherwise. Mutants: skip the version check under the lock; derive the version from time.
- inventory: the three work directories keep today's exact results including an orphan epic temp; a new home accepts own temps as warnings and refuses foreign names as incomplete; `list_dir` reports `complete=false` at the cap and refuses links. Mutants: widen work temp recognition, drop the cap.
- engine fixtures: foreign staged and dirty files survive; same-path staging defers; hooks and signing failures leave pending and a truthful receipt; a lost commit reply reconciles without a second commit.
- production policy, real router and real repositories, one test per mutation family (existing work plan, record and review mutations, a knowledge create and edit, a document save, a compaction step): each successful actual mutation yields exactly one commit holding exactly its intended paths with its trailers and nothing else; unrelated staged and dirty files keep their state. A read, a true no-op, a registration repeat, a refused call and a failed call yield no commit. A partial call (injected failure after the first publication) and a sync-uncertain call yield `Deferred(IncompleteOutcome)` and no commit, and a later successful mutation does not commit their intents; explicit recovery `Retry` does and marks them adopted. An earlier successful call deferred by a hook, signing failure or foreign staging is included in the commit of a later successful mutation (one trailer block each) and a broken per-path chain is reported as drift. A Git failure leaves every saved file intact with a `Deferred` or `Unknown` receipt. Mutants: commit on a read or no-op, commit a partial call, skip earlier deferred successes, certify a held intent by a later success, reset after a hook change.
- receipt derivation, real repositories with older deferred successful intents already pending (a failing hook made them wait), then one current successful call: (1) the current call's path has user-staged changes: the older intents commit, `earlier` lists that commit, and the current receipt is `Deferred(ForeignStagingOnPath)` with `commit: None` and the current paths, never `Committed`; (2) the current call alone exceeds the message budget: older intents commit, current is `Deferred(MessageBudget)`; (3) the current call has one tracked and one untracked ordinary publication (a full journal forces the untracked one): nothing of the current call is committed, the receipt is `Deferred(UntrackedSibling)` with `untracked` and the tracked `paths`, and older eligible intents may still land under `earlier`; (4) the current call reserves a counter and writes its record: both are in one commit or neither, never the record alone; (5) the current call touches a path of an older deferred intent that is itself deferred: both are deferred together. Mutants: report `Committed` because an older intent landed; commit the tracked subset; fill `paths` from the whole commit; fold older commits into the current outcome.
- user-ignored business file, one existing fixture, no new framework: the repository's own `.gitignore` ignores the knowledge counter file (a user pattern, not a built-in path) and a first create reserves that counter and publishes a new record. The call stays saved on disk, is `Deferred(IgnoredBusinessSibling)` with `commit: None` and `Attention::UntrackedMutation`, nothing of it is committed (the record is not committed alone), the `.gitignore` bytes and the index are unchanged, and no forced stage was used; older eligible intents may land under `earlier`. A user pattern over a directory leaves its event `NotApplicable`. Mutants: commit the record alone; force-stage the ignored counter (`add -f`); edit `.gitignore`; treat the ignored counter as `NotApplicable`.
- not-applicable effects, real repositories and real router: (1) the first Decision create in a repository with no `decisions/` home creates the directory, the counter file and the record in one call: the directory event is `NotApplicable`, the two files are `Tracked`, the call is `Committed`, and the commit holds exactly the counter file and the record, with no directory entry, no backup and no journal intent for the directory; (2) a normalizing write of a legacy record (native comment before the YAML) publishes the retained backup under `.agent-tasks/backups/` and the record: the backup is `NotApplicable` and ignored, only the record is journaled and committed, and `git status` shows no staged or tracked backup; (3) the same first create with the journal forced full for the record file only, or with a real non-ignored file that stays untracked, still blocks the whole current call with `Deferred(UntrackedSibling)` while the directory and backup stay truthful filesystem effects; (4) a `SyncUnknown` on the new directory is reported as `Durability::SyncUnknown` with `Attention::SyncUncertain`, is never shown `Durable`, and holds that call as `Partial`; (5) `Preserve` of the counter and record files works and an empty directory is not a preservable item. Mutants: count the directory or the backup as an untracked sibling so no commit lands; stage the directory or the backup; mark them `Tracked` with a fake intent; drop the sync-uncertain attention.
- failure receipt: a call that errors after its first publication gives `Deferred(IncompleteOutcome)`, `commit: None`, the held intent in `pending`, and a later success does not commit it; a call that errors before publishing gives `saved_only()` with no settlement. Mutant: treat the failed call as committed or settle an empty call.
- same-operation removal guard and the M-005 gate, real repositories: with a reused `OperationId` `O`, call 1 publishes replacements under `O` and fails (Held), call 2 removes under `O`: the removal intent is `Deferred(RemovalBarrier)`, the removal stays saved on disk, and `Retry` naming call 1 and call 2 together commits them as one composition. With per-action operation identities (`cp:c:r1:A01` replacement held from call 1, `cp:c:r1:A03` removal in call 2) the engine commits the removal intent atomically without claiming a link between them; the test shows M-005's gate refuses that removal unless the replacement has `Committed` proof or sits in the same whole intent. A fresh apply whose replacements and removals are in one call commits together with no older pending replacement required. Mutants: commit a same-operation removal alone; claim the engine connects different operation identities; require all pending replacements before any handler returns.
- tree proof: a commit that holds a replacement and a removal shows both in the verified changed set; an unreadable object gives `Unknown`, runs no fetch, and never `Committed`. Mutant: verify a subset of the changed paths.
- receipts: `settle` returns a receipt on every path including work mutations; the dispatcher keeps the business result when rendering fails.

Test substitutes: the real Store on a temporary root is the faithful substitute for all store primitives (it is a portable file store); `GitReceipt::saved_only()` and `testing::with_policy` serve presentation tests before the engine lands. A pure fake of `effect_status` is acceptable for M-005 unit tests but never counts as composition proof.
