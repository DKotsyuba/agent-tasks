# Managed Markdown, document references and documentation bootstrap

Status: planning contract, revision 3. Revisions 1 and 2 are preserved unchanged in history (commits d4bd4ac1df9db01c1a2e3dbdd4e5f90430cf16bf and ea3a17fd4169f4c254ecd1f082a85c5f51d801da). Revision 3 changes only the relocation recovery: the three relocation windows are defined, `relocate` takes a recorded move observation instead of a bare version, identity-bearing scopes resume an interrupted move from storage's operation oracle without minting an identity, and the ordinary recovery text is corrected (section 7.2, the recovery list in section 7.1 and the two affected boundaries in section 11). Boundary impact: `doc-reference` is unchanged and stays revision 2; `md-documents` and `documents-and-references` are raised to revision 3; the storage boundary this owner consumes must cover the operation oracle (section 13). Revision 2 changes: one contradiction fixed in `valid_reference` and in the scanned homes; incoming coverage is certified only after structural validation by the existing loaders; `resolve` verifies child identifiers instead of answering unverified; retired DOC identifiers stay resolvable; `integrity` models live DOC records in the post state; error passthrough and `partial_publication` are distinguished; the producer seam for the document tool (section 12); the allocator and inventory dependencies conform to the providers instead of defining them. Normative for the Markdown owner and its three consumers (typed knowledge records, retrieval and presentation, compaction). Source behavior and docstrings become the truth once implemented. A change to any numbered rule below that affects a consumer raises the contract revision.

Component names used here: **Markdown owner** (this document), **knowledge records** (typed Decision, Runbook, Research, Checklist and the knowledge allocator), **storage** (guarded publication, removal, directory creation, effect receipts, Git engine), **tool surface** (registry, inputs, reads, presentation), **compaction**.

The owner provides three bundled boundaries, one per consumer: [doc-reference](#doc-reference-r2), [md-documents](#md-documents-r2) and [documents-and-references](#documents-and-references-r2). It consumes the knowledge allocator and typed ID grammar from knowledge records and the publication primitives from storage. Every function below is plain in-process Rust. No domain function takes a tool name, JSON, a template or a raw filesystem root. The document tool payload, description and locked handler are the one producer file described in section 12; registration, routing and read framing belong to the tool surface.

## 1 Scope, namespace and caps

Managed namespace: `README.md` at the documentation root and every `*.md` below `docs/`. Nothing else is prose. Source code, `.git`, `.agent-tasks/`, work and knowledge records, and every other path are never read as prose and never listed.

| Constant | Value | Meaning |
|---|---|---|
| `BODY_CAP` | 524288 bytes | Largest managed body accepted by a save and readable as supported. A proposal that tests exercise at 524288 (saves) and 524289 (refuses). It is not a claim about the YAML record cap. |
| `PURPOSE_CAP` | 240 bytes | One-line purpose in metadata. |
| `PATH_CAP` | 160 bytes | Whole relative path. |
| `DEPTH_CAP` | 3 | Directory segments below `docs/` (storage creates at most 4 missing levels per call). |
| `HEADING_CAP` | 2048 | Headings addressed per document. More sets `Outline.complete = false`. |
| `FILE_CAP` | 1024 | Managed Markdown files inventoried. |
| `DOC_RECORD_CAP` | 1024 | Document metadata records inventoried. |
| `REF_FILE_CAP` | 4096 | Files scanned by one reference scan. |
| `SCAN_BYTES_CAP` | 16 MiB | Aggregate bytes read by one corpus or reference scan (the existing aggregate scan cap). |
| `MIN_PAGE_BUDGET` | 16 bytes | Smallest encoded budget accepted by a page read. |

Reaching any inventory or byte cap yields `complete = false` with a named gap and lower-bound counts. A cap never truncates silently.

## 2 Markdown dialect `md-dialect-v1`

The input is the exact byte string of one file. It must be valid UTF-8. Otherwise the file is `Unsupported(NotUtf8)`: listed, never decoded lossily, never edited. Rules:

1. **BOM.** A leading `EF BB BF` is part of the bytes and of the preamble. It is ignored only when classifying the first line. It is never added, removed or moved by any operation.
2. **Line ends.** A line ends at LF. A CR immediately before that LF belongs to the terminator (CRLF). A CR not followed by LF is ordinary content (lone CR); it never ends a line. The last line may lack a terminator. Facts report counts of LF-only, CRLF and lone-CR terminators.
3. **NUL.** Readable if present (escaped on the wire). Saves and adoption refuse a body that contains U+0000.
4. **Front matter.** If line 0 is exactly `---` (after the BOM) and a later line is exactly `---` or `...`, lines 0 through the closing line are front matter. Front matter is not scanned for headings, links or references. Without a closing line there is no front matter.
5. **Fences.** A fence opener is a line with at most 3 leading spaces (a tab counts as 4 columns, so it disqualifies), then at least 3 identical backticks or tildes. For backticks the rest of the line may not contain a backtick. A closer has at most 3 leading spaces, the same character, at least the opener's length, and only spaces and tabs after. An unclosed fence runs to end of file. Fences nested in block quotes or list continuation indented by 4 or more spaces are not recognized (see rule 9).
6. **Comments.** A line whose first non-space content (after at most 3 spaces) is `<!--` starts a comment region that ends at the line that contains `-->` (that line included). Inline comments elsewhere in a line are ordinary text.
7. **ATX headings.** A line outside fences, comments and front matter with at most 3 leading spaces, 1 to 6 `#`, then a space, tab or end of line. Text is the remainder with surrounding spaces and tabs trimmed and an optional closing run of `#` (preceded by a space or tab) removed. Raw inline markup stays in the text. The text is not otherwise normalized and is compared byte for byte and case sensitive. An empty heading is valid.
8. **Setext headings are not headings.** Underline lines (`===`, `---`) are ordinary content. The outline counts candidate lines in `setext_candidates`. Documents with candidates are honest partial for fragment resolution (section 8).
9. **Not headings, by design.** Lines indented 4 or more spaces, anything inside a block quote (`>`), a list item marker line, a table row, or a multi-line construct this dialect does not parse. A heading written inside those is ordinary content. This is a deliberate boundary, not a defect.
10. **Sections.** Heading `i` with level `L` owns the bytes from its line start to the start of the next heading with level `<= L`, or to end of file. Child headings are inside the section. The **preamble** is the bytes before the first heading (BOM and front matter included). The **body** of a section is its bytes after the heading line's terminator. A heading on the last line without a terminator has an empty body.
11. **Duplicate headings.** Every heading has a zero-based document-order `ordinal` and a one-based `occurrence` among headings with identical text. Ordinals are valid only for the observation they came from.
12. **Slugs for link fragments** (`slug-v1`): Unicode lowercase the heading text, delete every character that is not a letter, digit, space, hyphen or underscore, turn each space into a hyphen, then add `-1`, `-2`, ... to later duplicates in document order. Inline markup is not interpreted.

```rust
pub const DIALECT: &str = "md-dialect-v1";

/// One heading. All offsets are absolute raw byte offsets in the document.
pub struct Heading {
    pub ordinal: usize,
    pub level: u8,            // 1..=6
    pub text: String,         // raw text, rule 7
    pub occurrence: usize,    // 1-based among equal text
    pub slug: String,         // rule 12
    pub start: usize,         // first byte of the heading line
    pub body_start: usize,    // first byte after the heading terminator
    pub end: usize,           // section end, exclusive
}

pub struct Outline {
    pub headings: Vec<Heading>,
    pub front_matter_end: Option<usize>, // exclusive end of front matter
    pub preamble_end: usize,             // start of first heading, or document length
    pub setext_candidates: usize,
    pub bom: bool,
    pub lf: usize, pub crlf: usize, pub lone_cr: usize, pub nul: usize,
    pub complete: bool,                  // false when HEADING_CAP was exceeded
}

/// Pure. Errors: `encoding` when not UTF-8.
pub fn outline(bytes: &[u8]) -> store::Result<Outline>;
```

## 3 Wire encoding `md-text-v1` and pages

Document bytes cross a text channel as follows. Encoding is chosen per page from the page's own raw span:

- **Raw** when the span contains none of: CR; U+FEFF; any other C0 control except LF and TAB; DEL; C1 controls U+0080 to U+009F; U+2028; U+2029; bidi controls U+202A to U+202E and U+2066 to U+2069. The text is the span verbatim. The reader must treat it as byte exact and a backslash as an ordinary character.
- **Escaped** otherwise. Only these substitutions are made: `\` becomes `\\`; CR becomes `\r`; every other listed character becomes `\u{h}` with lowercase hexadecimal and no leading zeros. LF and TAB stay literal. Everything else, including non-BMP characters, stays literal.

Escaped text decodes uniquely, because every backslash in the output begins an escape. The tool surface must label each page with `wire` so a reader never guesses. `decode` rejects unknown escapes, an unterminated escape and a lone `\u{..}` outside the Unicode scalar range with `encoding`.

```rust
pub const ENCODING: &str = "md-text-v1";
pub const MIN_PAGE_BUDGET: usize = 16;

pub enum Wire { Raw, Escaped }

/// Encode a complete span. Chooses Raw whenever no listed character is present.
pub fn encode(span: &[u8]) -> store::Result<(Wire, String)>;
pub fn decode(wire: Wire, text: &str) -> store::Result<Vec<u8>>;
/// Length in bytes of the encoded form, without allocating it.
pub fn encoded_len(span: &[u8]) -> usize;

pub struct Page {
    pub wire: Wire,
    pub text: String,   // encoded text, len <= budget
    pub start: usize,   // absolute raw offset of the first byte of this page
    pub end: usize,     // absolute raw offset after the last byte (> start unless span empty)
    pub span_end: usize,
    pub more: bool,     // end < span_end
}

/// Largest prefix of `bytes[at..span.end]` that ends on a UTF-8 character boundary and
/// whose encoded length is <= `budget`.
/// Preconditions: `span` within `bytes`, `at` in `span.start..=span.end` on a character boundary,
/// `budget >= MIN_PAGE_BUDGET`. Errors: `presentation_capacity` (budget too small), `invalid_arguments`
/// (offset outside span or inside a character).
pub fn page(bytes: &[u8], span: Range<usize>, at: usize, budget: usize) -> store::Result<Page>;
```

Guarantees (`encoded_len` of a prefix never decreases as the prefix grows, so the maximum is well defined):

1. Concatenating the decoded text of pages `at = span.start`, then each `page.end`, equals `bytes[span]` exactly.
2. A page never splits a character and is never empty while bytes remain, because the widest encoded character is 10 bytes and `MIN_PAGE_BUDGET` is 16. Progress is therefore guaranteed for every accepted budget.
3. `budget` is the number of encoded bytes available for text only. The tool surface subtracts its own header, labels and footer from the whole reply limit of 8192 bytes, and inserts `text` verbatim with no further escaping. A template that re-escapes `text` breaks guarantee 1; the md-documents boundary test checks the rendered bytes, not only this function.
4. Offsets are raw byte offsets of the stored document, never encoded offsets.

### Version-bound continuation

A read selects `Whole`, `Preamble` or a heading. Its **read snapshot** is `sha256("agent-tasks/doc-read/v1", observation.version, selector as resolved (whole | preamble | ordinal:N), ENCODING, DIALECT)`. It binds the resolved path, the body bytes, the metadata record observation and the stable selection. It never contains the offset. A continuation carries `(snapshot, at)`. A changed body, a changed record, a different selection or another dialect/encoding revision gives `stale` with the current snapshot, never mixed bytes. `at` is checked only as an in-span character boundary (any boundary yields correct bytes). A continuation with `at > span.start` and no snapshot gives `stale`.

## 4 Paths and collisions

```rust
/// Validated managed path. Text is always `README.md` or `docs/<seg>/.../<name>.md`.
pub struct DocPath(String);
impl DocPath {
    /// Errors: `path` (grammar), `invalid_arguments` (cap).
    pub fn parse(raw: &str) -> store::Result<DocPath>;
    pub fn as_str(&self) -> &str;
    /// Directory part for relative link resolution: "" or "docs/a".
    pub fn dir(&self) -> &str;
}
```

Grammar (ASCII only so no Unicode normalization ambiguity exists): each segment matches `[A-Za-z0-9][A-Za-z0-9._-]{0,63}` and does not end in `.`; the last segment ends in the exact lowercase extension `.md` with a non-empty stem; no `.`/`..` segments, empty segments, backslashes or leading `/`; Windows reserved stems (`CON PRN AUX NUL COM1-9 LPT1-9`, case-insensitive) are refused; length and directory depth and length caps in section 1. The segment form excludes dotfiles, so publication temps (`.<name>.tmp-<pid>-<n>`) and the bootstrap keeper never collide.

Rules:

- **Symlinks.** A symlinked file, directory or ancestor is never followed. Observation reports `Unsupported(Symlink)`; writes refuse with `file_type`. Existing native files that are not regular files report `Unsupported(NotRegular)`.
- **Case collisions.** Creating a path (file or directory) refuses with `collision` when an existing sibling differs only by ASCII case, or when the path would need a directory where a file exists or the reverse. Existing native collisions are all listed as `Unsupported(Collision)` in the inventory for every member and stay readable by their exact path.
- **Unsupported native names.** Non-ASCII, space, over-length, `.MD`, too deep and similar names are never renamed. The inventory lists each as `Unsupported(Name)` with its quoted lossy-safe name and `complete = false`. They are not addressable.
- **Supported-with-limits.** Over `BODY_CAP` is `Unsupported(TooLarge)`; not UTF-8 is `Unsupported(NotUtf8)`. Both are reported, never partially read.

## 5 Document metadata record

Home `documents/DOC-<n>.yaml`, one closed YAML record, IDs allocated by knowledge records (`DOC-` plus canonical digits). The body stays in the Markdown file; the record holds only generated facts.

```rust
pub struct Record {
    pub schema_version: u32,       // 1
    pub id: String,                // DOC-001, equals the file name
    pub path: String,              // DocPath text
    pub purpose: String,           // one line, 1..=PURPOSE_CAP, no control characters
    pub origin: Origin,            // Saved | Adopted
    pub state: RecordState,        // Active | Retired
    pub revision: u64,             // 1 at creation, +1 per managed save/adopt/relocate
    pub created_at: String,        // generated, RFC 3339 UTC
    pub updated_at: String,        // generated
    pub retired_at: Option<String>,
    pub actor: Option<String>,     // declared writer of the last managed change, never inferred
    pub body_sha256: String,       // hex of the bytes last published or adopted
    pub body_bytes: u64,
}
```

No dates, actors or revisions are ever derived from file times or native edits. The record contains no history; recoverable history is the Git history of the documentation repository. The `documents/` home is enumerated with the storage-provided bounded inventory (`Store::kind_inventory("documents", "DOC-")`; names are `DOC-<canonical digits>.yaml`, the store's own publication leftovers are warnings recognized by `store::own_temp_name`, any other entry sets `complete = false`). Each record is decoded by this owner's closed schema (the owner validates its own record; no other module does). A record that fails to decode, has an unknown schema version, whose `id` differs from its file name, or whose path is invalid is a named unreadable record and sets `complete = false` wherever claims matter.

States, derived only from actual bytes and records at observation time:

| State | Condition |
|---|---|
| `Absent` | no file and no active record claims the path |
| `Unmanaged` | supported file, no active record claims it. Metadata is unknown, never invented. Pre-existing documents (for example a migrated documentation set) and any README start here. |
| `Managed` | active record and `sha256(body) == body_sha256` and length equal |
| `Drifted` | active record, file present, bytes differ. Cause (native edit or interrupted managed write) is unknown and not guessed. |
| `MissingBody` | active record, no file |
| `Conflict` | more than one active record claims the path |
| `Retired` | observed by ID: the record is retired. It stays resolvable forever as historic metadata with its last path, hash and `retired_at`. Its path is free again: a later save creates a new record with a new identifier, and no rule blocks the reuse. A reference by path addresses whatever currently lives at that path and never follows a retired identity; a reference by the retired DOC identifier keeps resolving to the retired record. The version of a retired observation covers its record and an absent body, never a file that a later document placed at the old path. |
| `Unsupported(reason)` | section 4 reasons, plus `PartialCoverage` when the record inventory is incomplete so claims cannot be decided |

Observation never writes, creates a directory or lock, repairs, adopts or schedules anything. It may be called with a shared lock or no lock; mutations require the caller to hold the root write lock (section 7).

```rust
pub enum Ref { Id(String), Path(DocPath) }       // "DOC-001" or "docs/x.md"
impl Ref { pub fn parse(raw: &str) -> store::Result<Ref>; }

pub struct Facts { pub bytes: u64, pub sha256: String, pub bom: bool, pub lf: usize, pub crlf: usize, pub lone_cr: usize, pub nul: usize }

pub struct Observation {
    pub path: DocPath,
    pub id: Option<String>,
    pub state: State,
    pub record: Option<store::Snapshot<Record>>,
    pub body: Option<Vec<u8>>,
    pub facts: Option<Facts>,
    pub body_version: String,     // store.version(path, bytes or absence)
    pub record_version: String,   // digest of every record file claiming the path (id + version) or absence
    pub record_sha256: Option<String>, // sha256 of the record file bytes, None without a record
    pub version: String,          // sha256("agent-tasks/doc/v1", root, path, body_version, record_version)
}

/// Errors: `invalid_arguments`, `path`, `not_found` (unknown DOC id), `file_type`, `io`.
/// Unsupported files are returned as an Observation with `State::Unsupported`, not an error.
pub fn observe(store: &Store, target: &Ref) -> store::Result<Observation>;
```

Absence has a version too. Creating a document requires `expected == observe(path).version` of the absent path, so a concurrently created file, record or claim makes the create stale.

## 6 Reads, inventory and search corpus

```rust
pub enum Selector {
    Whole,
    Preamble,
    Ordinal(usize),
    Heading { text: String, level: Option<u8>, occurrence: Option<usize> },
}
impl Selector {
    /// Build a selector from loose optional parts, as supplied by a tool. Exactly one of `preamble = true`,
    /// `ordinal` or `heading` is allowed; `level` and `occurrence` apply only with `heading`; none supplied is `Whole`.
    /// Errors: `invalid_arguments` naming the offending part (`preamble`, `ordinal`, `heading`, `level`, `occurrence`).
    pub fn from_parts(preamble: bool, ordinal: Option<usize>, heading: Option<String>, level: Option<u8>, occurrence: Option<usize>) -> store::Result<Selector>;
}

/// Resolve to an absolute raw byte range. `Heading` without `occurrence` and with more than one
/// match gives `ambiguous_section` (never the first match). No match gives `not_found`.
pub fn resolve(outline: &Outline, total: usize, s: &Selector) -> store::Result<Range<usize>>;

pub struct ReadPage {
    pub page: markdown::Page,
    pub range: Range<usize>,       // the resolved selection
    pub snapshot: String,          // read snapshot, section 3
    pub observation_version: String,
    pub state: State,
}

/// Exact document or section read. `snapshot` None starts at the span start and requires `at == None`.
/// Errors: `not_found` (absent, missing body, retired), `stale`, `ambiguous_section`, `encoding`, `presentation_capacity`.
/// Reads never create files and never change state. Drifted and unmanaged bodies are read as found.
pub fn read(obs: &Observation, selector: &Selector, at: Option<usize>, snapshot: Option<&str>, budget: usize) -> store::Result<ReadPage>;

pub struct Row { pub path: String, pub id: Option<String>, pub state: State, pub bytes: Option<u64> }
pub struct Gap { pub what: String /* quoted name or home */, pub reason: GapReason }
pub struct Inventory { pub rows: Vec<Row>, pub gaps: Vec<Gap>, pub complete: bool, pub files_read: usize, pub bytes_read: u64, pub version: String }
/// Bounded sorted enumeration of managed files plus records (so MissingBody and Retired appear).
/// `docs/` absent is a complete empty result. Never creates files.
pub fn inventory(store: &Store) -> store::Result<Inventory>;

pub struct CorpusDoc {
    pub reference: String,         // DOC-001 when managed or path
    pub path: DocPath,
    pub id: Option<String>,
    pub state: State,
    pub purpose: Option<String>,
    pub headings: Vec<markdown::Heading>,
    pub text: String,              // exact UTF-8 body for lexical matching
}
pub struct Corpus { pub docs: Vec<CorpusDoc>, pub gaps: Vec<Gap>, pub complete: bool, pub files_read: usize, pub bytes_read: u64, pub version: String }
/// All supported documents within SCAN_BYTES_CAP; unsupported files become gaps and set `complete = false`.
pub fn corpus(store: &Store) -> store::Result<Corpus>;
/// Innermost heading ordinal containing a raw byte offset (None in the preamble).
pub fn heading_at(headings: &[markdown::Heading], offset: usize) -> Option<usize>;
```

Matching, ranking and excerpts belong to the tool surface. Search and context must present `Corpus.complete`, `gaps` and the quoted names of unsupported files.

## 7 Mutations

All mutations require: the caller holds the root write lock for the whole call (the Markdown owner never locks, so storage can keep one lock scope without nested relocking); `expected` equal to a fresh observation taken inside the call; every effect performed through the storage primitives; no read-side migration of other files. All of them return `stale` before any effect when `expected` differs.

Ports the owner consumes. Each provider owns its identifiers, revision and artifact; this document states the calls the owner needs and the provider's pinned contract entry holds the exact signatures. Where a provider has not yet published a revision, the owner implements against a faithful substitute and does not define the provider's interface.

- **Storage (publication and Git module).** Exact raw reads with an explicit cap (`Store::read_exact`), guarded create and replace (`Store::publish_with`), guarded removal (`Store::remove`), `Store::create_dir` and `Store::ensure_parents` (at most 4 missing levels per call), request-local typed events (`Store::publications()`) carrying the caller `OperationId`, the public bounded inventory `Store::kind_inventory(directory, prefix)` and `store::own_temp_name`. Every body publication and removal uses `cap = BODY_CAP`; records use the ordinary record cap. Because storage creates at most four missing levels per call, document paths have at most three directory segments below `docs/` (the first write also creates `docs/` itself).
- **Knowledge allocator (knowledge records module).** One exact allocation observation covering every counter home and a reservation for the document prefix under the caller's lock guard, published before the record, gaps retained, numbers never recycled. The earlier revision-1 form in which this owner passed a `foreign` DOC identifier list is withdrawn by this artifact: the owner supplies no allocator input and keeps only the record schema and files (`documents/DOC-<n>.yaml`). The owner calls the allocator exactly as the provider's pinned revision defines it.
- **Structured record loaders (for reference scanning).** `Store::scan` and `Store::module` for work records, `knowledge::scan` and `knowledge::load` for typed records, `Store::project` for the manifest. They are the only validators; this owner implements no YAML parser and no business validator of another module's records.

Operations. `Scope` carries what storage requires for a single lock scope and what compaction requires for operation identity:

```rust
pub struct Scope<'a> {
    pub store: &'a Store,
    pub lock: &'a store::LockGuard,                  // write lock acquired through this Store in this call
    pub operation: Option<&'a store::OperationId>,   // carried on every typed event; never parsed; one id per scope
    pub fx: &'a mut Vec<String>,                     // existing human effect strings
}
pub enum Edit<'a> {
    /// Replace the whole body with these exact bytes.
    Body(&'a [u8]),
    /// Replace only the body of one section (or the preamble), keeping its heading line.
    Section { selector: &'a Selector, body: &'a [u8] },
}
pub struct Save<'a> { pub target: &'a Ref, pub purpose: Option<&'a str>, pub edit: Edit<'a>, pub expected: &'a str, pub actor: Option<&'a str> }
pub struct Receipt {
    pub id: Option<String>, pub path: DocPath, pub changed: bool,
    pub state_before: State, pub state_after: State,
    pub version_before: String, pub version_after: String, // after: observe by DOC id when one exists (also retired), else by path
    pub revision: Option<u64>, pub bytes: u64, pub sha256: String,
    pub publications: Vec<store::Publication>,  // exactly the typed events this call produced, in order
    pub warnings: Vec<Warning>,                 // MixedLineEndings, SetextIgnored, ...
    pub references: ReferenceCheck,             // section 9
}
pub fn save(scope: &mut Scope<'_>, req: Save<'_>) -> store::Result<Receipt>;
pub fn adopt(scope: &mut Scope<'_>, target: &Ref, purpose: Option<&str>, expected: &str, actor: Option<&str>) -> store::Result<Receipt>;
pub fn remove(scope: &mut Scope<'_>, target: &Ref, expected: &str, actor: Option<&str>) -> store::Result<Receipt>;
pub fn relocate(scope: &mut Scope<'_>, from: &Ref, to: &DocPath, basis: &MoveBasis, expected_to: &str, actor: Option<&str>) -> store::Result<Receipt>;
```

`Receipt.id` is `None` only for an `Unmanaged` document that was relocated or removed without ever having a record. Callers needing several operation identities build one `Scope` per action.

Semantics, exactly:


- **save, new path** (`Edit::Body`, state `Absent`): purpose required. Validate caps, UTF-8, no NUL. Reserve `DOC-n` (`knowledge::reserve`, published before any record; a later failure leaves a visible gap), create missing `docs/` and parent directories (`ensure_parents`, one typed event each), create the body without clobber, create the record without clobber. A path whose directory chain contains a symlink refuses before any effect.
- **save over existing** (`Managed`, `Drifted`, `Unmanaged`): guarded replace of the body, then guarded replace of the record (revision plus 1, new hash and length, `updated_at`). Unmanaged documents gain a new record in this save (`origin = Saved`, purpose required). Identical result bytes and unchanged purpose give `changed = false` and no effect.
- **section replace**: the new body bytes are spliced after the heading line's terminator through the section end. All bytes outside the section, including BOM, front matter, CRLF and everything else, are byte identical. The replacement may not (a) contain a heading of level `<= L` outside fences, (b) leave a fence or comment open at its end, or (c) be applied to a heading line without terminator unless it begins with LF or CRLF. Violations give `structure_change` or `unterminated_heading`. As a post condition the owner rescans: headings before are unchanged, the edited heading is unchanged, headings inside have level `> L`, headings after are unchanged. A mismatch gives `structure_change` with no effect. `Preamble` replacement may contain no headings. The bytes supplied are inserted verbatim: no line-ending conversion. When the result mixes LF and CRLF and the original did not, the receipt carries `MixedLineEndings`.
- **adopt**: only `Unmanaged` or `Drifted`. Reserve an ID when none exists, publish the record with the current bytes' hash. The body file is untouched (no effect on it). Purpose required unless a record exists.
- **remove** (retire): `Managed`, `Drifted`, `MissingBody` or `Unmanaged`. Order: guarded removal of the body, then record `state = Retired` (when a record exists). Retirement keeps the ID, its record and its last path forever, so the ID stays resolvable as historic metadata. The path becomes free; a later save at it creates a new record. Empty directories are left.
- **relocate**: `from` is `Managed`, `Drifted` or `Unmanaged`; `to` is `Absent` (`expected_to` is its version; `basis` is the recorded move observation of section 7.2). Order: S1 create the body at `to` without clobber (creating directories), S2 replace the record path and revision keeping the identifier, S3 remove the body at `from`. Windows W1 to W3 and the identity-bearing resume are defined in section 7.2. Incoming references are not rewritten here (section 9).
- **Partial results.** See section 7.1.

Common refusals: `stale`, `invalid_arguments` (purpose, NUL, empty selection, over cap), `capacity`, `path`, `file_type`, `collision`, `conflict` (several active claims, record path mismatch), `unsupported` (named reason; never edited), `partial_coverage` (record inventory incomplete, claims unknown), `not_found`, `ambiguous_section`, `structure_change`, `unterminated_heading`, `encoding`, `partial_publication`.

### 7.1 Errors, passthrough and partial publication

Every mutation takes `n0 = store.publications().len()` on entry. The error class is decided from the typed events produced since `n0`:

1. **No event produced since `n0`** (every validation refusal, a stale version, and a storage or allocator refusal before its first effect). The underlying code passes through unchanged (`stale`, `capacity`, `path`, `file_type`, `root_changed`, `not_locked`, `busy`, `io`, `allocator`, `inventory`, `publication`, ...). Only in this class may the message say that no work was saved.
2. **At least one event produced and a later step failed** (including a reservation published, a body published without its record, a record replaced without the old body removed). The code is `partial_publication`. The message never claims that nothing was saved. It begins with the words "Partially saved.", names the failing step and the underlying cause code, lists at most 8 completed effects as `<kind> <relative path>` in order (the complete typed list is `store.publications()`), states that nothing was rolled back or retried, and gives the single recovery step: `adopt` for a body written without a record or a drifted body, repeat the same call with a fresh version after a reservation that left a gap (the reserved number stays a visible gap), repeat `remove` for `MissingBody`. For `relocate` the recovery is never `adopt` and never a plain repeat: it is the window-specific step of section 7.2 (resume under an identity-bearing scope; for the ordinary tool, `remove` the unclaimed copy at `to` in W1 and repeat `relocate`, or `remove` the unclaimed leftover at `from` in W2).
3. **The final step of an operation published visibly and only the parent sync failed.** Storage's `durability_unknown` passes through unchanged. All steps are visible and none is missing; the message says to inspect and not to replay.

The owner never rolls back, never retries a step, and never treats equal bytes as proof that an earlier effect was its own: only the typed events of this call, as attested by storage with the caller's operation identity, certify ownership. A caller that must reconcile a partial result later uses storage's operation status, not byte equality.


### 7.2 Relocation windows, identity-bearing resume and ordinary recovery

`relocate` publishes in this order, each step a separate typed event: S1 create the body at `to` (no clobber; `ensure_parents` events first), S2 replace the record (same DOC identifier, `path = to`, revision plus 1; skipped when the source has no record), S3 remove the body at `from` (guarded on the observed bytes). A crash or failure leaves one of these states. The terms are exact:

| Window | `to` | record | `from` | Observed as |
|---|---|---|---|---|
| W0 | absent | binds `from` | present | normal start |
| W1 (after S1) | copy of the source bytes, claimed by no record | binds `from` | present | `to` Unmanaged, source `Managed` or `Unmanaged` |
| W2 (after S2) | present | binds `to` | leftover, claimed by no record | `to` Managed, `from` Unmanaged |
| W3 (after S3) | present | binds `to` | absent | complete |

**What was wrong before.** A plain repeat of `relocate` requires `to` to be absent, so it refuses in W1 and W2. `adopt` of `to` in W1 would mint a new DOC identifier and leave the old record bound to `from`. Deleting the copy and recreating it, under an identity-bearing scope, is refused by storage's `operation_repeat` guard because the journal already holds a `Published` entry for that operation and path. A repeat of `relocate` in W2 could not name the leftover (the record no longer binds `from`) and `adopt` would mint another identity. None of those is a recovery.

**Recorded move observation.** The only new input is a plain value the caller already holds from its planning observation, so the owner does not serialize any metadata for the caller:

```rust
/// What the caller observed about the source when it planned the move. Built by `Observation::move_basis`.
pub struct MoveBasis {
    pub version: String,                 // Observation.version of the source (also the old `expected`)
    pub body_sha256: String,             // sha256 of the source body bytes actually present
    pub record_sha256: Option<String>,   // sha256 of the record file bytes; None when the source has no record
    pub id: Option<String>,              // DOC identifier when a record exists
}
impl Observation {
    // Observation also gains the additive field `pub record_sha256: Option<String>` (sha256 of the record file bytes).
    /// Errors: `unsupported` (any Unsupported, Conflict, MissingBody or Retired state), `not_found` (Absent).
    pub fn move_basis(&self) -> store::Result<MoveBasis>;
}
/// Version of an absent managed path, computable from the path alone (no files read). Equals `observe(path).version` of an absent path.
pub fn absent_version(store: &Store, path: &DocPath) -> String;
```

The relocation signature therefore replaces `expected: &str` by the basis:
`pub fn relocate(scope: &mut Scope<'_>, from: &Ref, to: &DocPath, basis: &MoveBasis, expected_to: &str, actor: Option<&str>) -> store::Result<Receipt>;`
`basis.version` plays the old `expected`. Compaction stores the four basis fields next to the base version it already records. The tool surface path (`document_work relocate`) builds the basis from a fresh observation inside the handler and keeps its payload and its `version` and `to_version` arguments unchanged.

**Normal start (W0), any scope.** Fresh observations must satisfy `observe(from).version == basis.version` and `observe(to).version == expected_to == absent_version(to)`; otherwise `stale` with no effect. When `Scope.operation` is `Some`, S1 to S3 use `Attest::Required`; storage refuses with `attestation_unavailable` and no effect when it cannot attest (no repository, full or unreadable journal).

**Identity-bearing resume.** Only when the normal start fails and `Scope.operation` is `Some`. The owner asks storage's operation oracle (`persist::effect_status` with the scope's `OperationId`) for the complete attested set of that operation, with one assertion: `Created(to, basis.body_sha256)`. It never decides from equal bytes. Resume proceeds only when the answer is `Attested` or `Partial` (every row proven by a journal entry or an engine commit trailer for this exact operation) and every returned row is one of: a directory effect, `Created(to)` with `after_sha256 == basis.body_sha256`, `Replaced(documents/DOC-<n>.yaml)` whose `before_sha256` equals `basis.record_sha256` (and `n` equals `basis.id` when present), `Removed(from)` with `before_sha256 == basis.body_sha256`. Any other row, `Foreign`, `Unknown` (including an unattested equal copy, a crash-after-rename entry, an unreadable journal or an incomplete history scan), `NotPublished` with a state that is not W0, or a result whose `complete` flag is false is refused as `resume_unproven` with no effect; the message lists the observed paths and digests and says what the user can do (below). The accepted states are then validated against the current files before any effect:

- **W1 resume.** Attested rows: `Created(to)` only. Required: `observe(from).version == basis.version` (the source, record and body are exactly what was planned), `to` is `Unmanaged` with a body whose sha256 is `basis.body_sha256`, and `expected_to == absent_version(to)` (the planned precondition). The owner skips S1 and performs S2 (replace the record, using the exact observed record bytes as the guard; the identifier and every field except `path`, `revision` and `updated_at` are unchanged; no allocator call) and then S3. An unmanaged source without a record skips S2.
- **W2 resume.** Attested rows: `Created(to)` and `Replaced(record)`. Required: the current record file hashes to the row's `after_sha256`, binds `to`, has the identifier of that row and `body_sha256 == basis.body_sha256`; `to` holds exactly those bytes (`Managed`); the leftover at `from` is a regular file whose bytes hash to `basis.body_sha256` and which no record claims. The owner performs only S3 (guarded removal of the leftover). The moved identity is never retired and its record is not touched.
- **W3 (complete).** Attested rows include `Removed(from)` and the current state matches W3: the owner publishes nothing and returns the receipt with `changed = true`, `state_after` of `to`, and the warning `AlreadyApplied`. If the current state does not match, `resume_unproven`.

Resume never calls `adopt`, never reserves an identifier, never overwrites an existing equal native copy, never resets storage's journal, and never creates a transaction. If storage reports `operation_repeat` for a step that the oracle did not attest (journal and oracle disagree), the error passes through (class 1) and the caller must use a new operation identity; it must not delete and recreate.

**Ordinary recovery (`Scope.operation` is `None`).** The ordinary tool has no identity and no resume. It stops at the first failure with `partial_publication` and the exact states below; nothing needs an unsupported repeat:

- W1: inspect both paths. The copy at `to` is `Unmanaged`. If it is the intended destination, `remove` it (a guarded, explicit removal of an unclaimed file; no record is retired) and repeat `relocate`; or keep it and treat the source as not moved. `adopt` of `to` is wrong because it mints a new identifier.
- W2: the leftover at `from` is `Unmanaged` and the moved document at `to` is `Managed` with its original identifier. `remove` the leftover path (no record exists for it, so nothing is retired). Do not `adopt` or `relocate` it.
- A repeat of `relocate` onto an occupied `to` always refuses `stale`; the owner never overwrites an existing destination, even when its bytes are equal.


## 8 References

```rust
pub enum Target {
    Work(String),        // E-001, M-001, A-001, M-001/T-001, M-001/A-001
    Knowledge(String),   // D- RB- RS- CL- DOC- CP- plus canonical digits, CL-001/I-001
    Doc { path: DocPath, fragment: Option<String> },
}
pub enum Via { MarkdownLink, BareId, RecordToken }
pub enum SourceKind { Markdown, Readme, Work, Knowledge, DocMetadata, Project }
pub struct Source { pub kind: SourceKind, pub id_or_path: String }
pub struct Link { pub target: Target, pub via: Via, pub line: usize, pub raw: Range<usize> }

pub const REFS_DIALECT: &str = "md-refs-v1";
/// Parse one reference string (the form stored in records and accepted by tools).
/// Valid: canonical IDs (`M-001`, `M-1000`: the decimal number formatted with at least 3 digits and no other leading zero),
/// a managed path, a managed path with `#fragment`. Invalid: `M-0001`, `m-001`, `docs/../x.md`, `/docs/x.md`, `http://..`.
/// Errors: `invalid_arguments`.
pub fn parse(raw: &str) -> store::Result<Target>;
impl Target { pub fn canonical(&self) -> String; }

/// Validator for the optional detail reference stored verbatim by typed knowledge records.
/// Accepts only a document target in root-relative form: `README.md` or a managed path such as `docs/x.md`,
/// optionally followed by `#fragment`. The document must exist as a supported file (not `Unsupported`) and the fragment
/// must equal one `slug-v1` of its outline. Rejects typed IDs, absolute paths, any `..` or `.` segment, URLs,
/// noncanonical spellings, missing targets and missing fragments with `invalid_arguments`.
/// Read only; the caller holds at least a shared lock.
pub fn valid_reference(store: &Store, raw: &str) -> store::Result<()>;

pub enum Resolution { Found, Retired, Missing, MissingSection, Unsupported(Unsupported), Unknown(String) }
/// Proof of existence. Reads only. Every case below is verified by loading the record through its owner's loader:
/// - Work `E-`/`M-`/`A-`: `Store::module(id)` (the existing validating loader). `M-001/T-001` and `M-001/A-001` are `Found` only when the
///   loaded parent contains that task or atomic; an absent parent or child is `Missing`.
/// - Knowledge `D-`/`RB-`/`RS-`/`CL-`: `knowledge::load(id)`. A checklist item `CL-001/I-001` is `Found` only when the loaded record has that item
///   (the lookup is a read accessor owned by knowledge records; until it is pinned the answer is `Unknown`).
/// - `DOC-`: the record file decoded by this owner: `Found` when active, `Retired` when retired (historic metadata, still resolvable), else `Missing`.
/// - `CP-`: existence of the regular file `compactions/CP-<n>.yaml` only; structural validation belongs to the compaction owner.
/// - Doc path: `observe`; `Found` for a supported file, `Unsupported` with its reason, `Missing` otherwise; a fragment equal to no `slug-v1` is
///   `MissingSection`, or `Unknown` when the document has `setext_candidates > 0` because the fragment may address a setext heading.
/// A loader failure, unreadable record or unknown schema is `Unknown(reason)`. `Unknown` is never proof: a destructive caller
/// must treat everything except `Found` (and `Retired` where historic metadata suffices) as unproven.
pub fn resolve(store: &Store, t: &Target) -> store::Result<Resolution>;
```

**Recognized in Markdown prose** (outside front matter, fences, comments and inline code spans on one line):

- inline links and images `[t](dest "title")` and `![a](dest)`, reference definitions `[label]: dest "title"`. Destination forms: `<...>` or a bare run without spaces with balanced parentheses.
- bare typed IDs: longest match of the ID grammar at a word start (previous character not a letter, digit, `_`, `-` or `/`; next character not a letter, digit or `_`).

**Destination classification:** a URL scheme (`^[A-Za-z][A-Za-z0-9+.-]*:`) is external and ignored; `#frag` is same document; a leading `/` or a path leaving the root is `Outside` and ignored for incoming counts; otherwise percent-decode (invalid UTF-8 is `Outside`), resolve lexically against the containing file's directory, and accept only a managed path. Fragments match `slug-v1` after percent-decoding.

**Homes and sources of one scan.** Exactly these homes are read: `README.md`, the managed Markdown under `docs/`, `project.yaml`, and the record homes `epics/`, `modules/`, `atomics/`, `decisions/`, `runbooks/`, `research/`, `checklists/`, `documents/`. Everything else is outside every scan and is not a gap: `.git/`, `.agent-tasks/` (counters and backups), source code, and `compactions/`. The current compaction proposal state, its history and its staged content are excluded from every incoming and reference corpus: they are not a source, not a home and not a gap. The compaction owner separately detects live proposals that claim an overlapping document path. An absent home directory is complete and empty. A home that exists as a file or a link is a gap.

**Structural validation before certification.** A record contributes references only after the existing loader of its owner accepted it; its validated raw bytes are then searched for tokens, with no YAML parsing and no business validation implemented here:

| Home | Enumeration and loader | Token search runs on |
|---|---|---|
| `epics/ modules/ atomics/` | `Store::inventory` and `Store::scan` (healthy snapshots plus named unreadable rows) | the loaded snapshot bytes |
| `decisions/ runbooks/ research/ checklists/` | `knowledge::scan(store, None)` | the loaded snapshot bytes |
| `documents/` | `Store::kind_inventory("documents", "DOC-")` and this owner's closed record decoder | the decoded record bytes |
| `project.yaml` | `Store::project` | the loaded bytes |

Tokens searched are the typed ID grammar and the managed path token `(README.md|docs/<segments>.md)(#fragment)?`. A record is never a source for itself or for the children it owns (`M-001/T-001` inside `M-001.yaml`). A `documents/` record is a source only for the path it binds and only when another record also binds that path (a conflict), so a normal binding is never reported as an incoming reference.

A record the loader rejects, a record with an unknown schema, an unreadable or non-UTF-8 file, a regular file whose name the inventory does not recognize, a link or non-regular entry in a home, a capped inventory and a capped byte budget each produce a named gap and make the scan incomplete. A raw token search over a record that failed validation never counts as coverage.

**Declared parser limits** (always returned in `Coverage.limits`): multi-line code spans, HTML `href`, wiki links, setext fragments, links inside block quotes and lists that need container parsing, and reference-style links with undefined labels. When the scan sees an `href=`, `<a `, `[[` or a heading-fragment link to a document with `setext_candidates > 0`, it counts it in `Coverage.unparsed` and sets `complete = false`.

```rust
pub struct Coverage {
    pub complete: bool,               // every home listed, every file loaded and read within caps, no gap, unparsed == 0
    pub files_read: usize, pub bytes_read: u64,
    pub gaps: Vec<Gap>,              // quoted names with reason: Unreadable, UnknownSchema, NotUtf8, Capped, UnrecognizedEntry, NotADirectory
    pub unparsed: usize,
    pub limits: &'static [&'static str],
    pub version: String,              // digest of every scanned file version and the home inventories
}
pub struct Incoming { pub source: Source, pub via: Via, pub count: usize, pub fragments: Vec<String> }
pub struct IncomingResult { pub target: Target, pub rows: Vec<Incoming> /* sorted, all sources */, pub coverage: Coverage }
/// Complete bounded incoming scan. Never writes. Temp files left by storage (`own_temp_name`) are ignored with a warning.
pub fn incoming(store: &Store, target: &Target) -> store::Result<IncomingResult>;

pub struct Outgoing { pub link: Link, pub resolution: Resolution }
pub struct OutgoingResult { pub rows: Vec<Outgoing>, pub coverage: Coverage }
pub fn outgoing(obs: &documents::Observation, store: &Store) -> store::Result<OutgoingResult>;
```

**Callers that remove, move or compact must refuse unless `coverage.complete`.** Incomplete coverage is unknown, never zero.

## 9 Update attention, integrity preview and link rewrite

```rust
pub enum ReferenceCheck {
    Skipped,                                  // no heading, path or removal change
    Checked { incoming: usize, introduced_dangling: Vec<Dangling>, coverage: Coverage },
}
pub struct Dangling { pub source: Source, pub target: Target, pub line: Option<usize> }

pub struct Overlay<'a> {
    pub put: &'a [(DocPath, Vec<u8>)],       // create or replace the whole body in the post state
    pub remove: &'a [DocPath],               // remove the file in the post state (an active record becomes Retired)
    pub moves: &'a [(DocPath, DocPath)],     // from, to: the file and its active record move; `to` must be absent or in `put`
}
pub struct Integrity { pub dangling_before: usize, pub dangling_after: Vec<Dangling>, pub introduced: Vec<Dangling>, pub coverage: Coverage }
/// Whole-root reference graph with the overlay applied in memory. Only document targets (path, fragment, DOC id)
/// can become dangling through prose changes; typed work and knowledge targets are reported by `incoming` and checked by `resolve`.
/// The post state uses the live DOC records: a record bound to a moved path follows its file, a record bound to a removed path
/// is `Retired`, and a link by DOC identifier to a retired or moved document is not dangling, so a legitimate change never
/// conflicts with its own record. A link by path to a removed path, or a fragment no longer in the post outline, is dangling.
/// `introduced` is `dangling_after` minus the dangling references that already existed before.
/// Never writes. `coverage.complete == false` means unknown: callers must refuse destructive application.
pub fn integrity(store: &Store, overlay: &Overlay<'_>) -> store::Result<Integrity>;

pub struct Rewrite { pub from: DocPath, pub to: DocPath, pub fragments: Vec<(String, String)> }
pub struct Rewritten { pub bytes: Vec<u8>, pub replaced: usize, pub skipped: Vec<Link> /* destinations it could not re-express */ }
/// Pure. Changes only Markdown link and reference-definition destinations that resolve to `rewrite.from`, re-expressed
/// relative to `source`'s directory; `<...>` wrapping, titles, surrounding text, BOM and line endings are preserved byte for byte.
pub fn rewrite(source: &DocPath, bytes: &[u8], rules: &[Rewrite]) -> store::Result<Rewritten>;
```

Ordinary saves return `ReferenceCheck::Checked` when the heading list, the path or the existence changed (section replace that removes a targeted heading, whole-body save that changes headings, remove, relocate). The save still succeeds; attention lists the introduced dangling links. Compaction uses `integrity` over the complete proposed result and `rewrite` for Markdown sources. Incoming references from records are never rewritten by the owner; a record source of a moved or removed path means compaction must refuse, and the same holds for a record source that names a fragment the post outline no longer has.

## 10 Documentation bootstrap and compatibility

Changes inside registration only (`register` keeps its signature):

1. **Admission.** `docs` joins the names allowed in a nonempty root, so registering an existing home that already has `docs/` is no longer refused as foreign content.
2. **Fresh and partial registration.** After the root is prepared and before Git bootstrap, create `docs/` when absent and a zero-byte `docs/.gitkeep` only when `docs/` is empty. The keeper is added to the single bootstrap commit with the existing four files. It is not a document: dotfiles are outside the managed namespace and never listed. No other folder (`documents/`, knowledge homes, `compactions/`) is created at registration; optional kinds appear on first use.
3. **README.** An existing `README.md` is kept as found on a partial retry. The root-entry check already admits README, and a completed registration returns unchanged before any bootstrap comparison (`register`, completed-binding branch), so a human-edited README is never refused. Only the generated text for new READMEs changes (it names `docs/` and the document tools generically). `project.yaml` and `.gitignore` keep the strict comparison. Managed README support follows: `README.md` is a managed path like any `docs/` file, `Unmanaged` until adopted.
4. **Completed legacy registration.** Returns unchanged and creates nothing, including no `docs/`. The first managed save creates `docs/` (one `create_dir` effect) lazily. An absent `docs/` is a complete empty inventory.
5. **Clone and move.** The keeper keeps `docs/` through a clone. References are root-relative so moving or cloning the documentation repository keeps links and IDs valid. A clone of a legacy registration without `docs/` is the lazy case.
6. **Existing documents.** Every pre-existing file and any native Markdown are `Unmanaged`, readable, searchable and referenceable with no migration, no metadata and no file written by any read.
7. **Git.** The bootstrap commit is the only commit this owner makes; it uses the existing explicit-path recipe. The owner policy is that every successful actual mutation (a document save, section replace, adopt, remove or relocate that changed bytes or metadata) is committed automatically by the dispatcher's settlement, not by this owner: this owner never calls Git outside bootstrap. A read, a no-op (`Receipt.changed = false`) and a failed or partial business result are not commit triggers. Saved bytes survive a rejected commit, and the receipt then reports pending or unknown honestly. The persistence engine starts after registration and must find nothing pending from bootstrap.

```text
fresh root                      -> docs/ + docs/.gitkeep + README + project + state + .gitignore, one commit
partial retry, README edited    -> README kept, remaining files created, alias published after Git succeeds
completed, docs/ missing        -> unchanged, nothing created
root with docs/*.md             -> admitted, no keeper, markdown stays untracked until storage adopts it
docs is a symlink or a file     -> refusal, no effect
```

## 11 Boundaries and executable evidence

Each boundary below is exercised through the public functions listed, with a correct-pass, a deliberate implementation mutant that must fail the same test, and a restored-pass, all tied to the restored candidate and the boundary revision named in its heading (`doc-reference` r2, `md-documents` r3, `documents-and-references` r3). Substitutes for providers that have not landed are faithful temporary-root fixtures; a passing substitute is not composition proof.

### doc-reference r2
Provider: Markdown owner. Consumer: knowledge records. Use: validate the detail reference stored in typed records and resolve targets. Functions: `references::parse`, `references::valid_reference`, `Target::canonical`, `references::resolve`, `documents::Ref::parse`, `DocPath::parse`.
Scenarios: (a) a table of valid and invalid references round-trips through `canonical`; `valid_reference` accepts an existing `docs/x.md` with and without a matching fragment and refuses a typed ID, a missing target, a missing fragment, `docs/../x.md` and `./docs/x.md`; mutant accepts `M-0001`, a missing target or a `..` segment; (b) `resolve` finds an existing path, a missing path, a missing fragment, a retired DOC as `Retired`, an existing task child as `Found`, an absent task child as `Missing` and a corrupt parent as `Unknown`; mutant answers `Found` for an absent child or for a corrupt parent; (c) `DocPath::parse` refuses `docs/../x.md`, a non-ASCII name, `docs/a b.md`, `docs/x.MD` and a fourth directory level; mutant skips the segment grammar.

### md-documents r3
Raised from revision 2 because the document tool (this owner's producer handler, section 12) uses the changed `relocate` and its ordinary-recovery text is observable in `document_work` replies; the tool surface calls no domain mutation directly and its read functions, snapshot, encoding and payload are unchanged.
Provider: Markdown owner. Consumer: tool surface. Functions: `observe`, `Selector::from_parts`, `markdown::outline`, `resolve`, `read`, `markdown::page/encode/decode`, `inventory`, `corpus`, `heading_at`, `incoming`, `outgoing`, `save`, `adopt`, `remove`, `relocate` and the section 12 producer seam.
Scenarios: (a) pages for every budget from 16 up concatenate to the original for a fixture with BOM, CRLF, lone CR, NUL, ESC, bidi, multibyte, non-BMP, a fence containing headings and a trailing unterminated line; mutant drops the backslash escape or splits a character; (b) continuation after a body edit, a record edit or a different selector gives `stale`, and a different offset in the same snapshot continues; mutant puts the offset in the snapshot or ignores the record version; (c) duplicate headings need an occurrence (`ambiguous_section`); mutant picks the first; (d) a fixture reproducing a pre-existing documentation set (11 files, ASCII and UTF-8, LF only, up to 34 KB) observes as `Unmanaged`, reads exactly, enumerates in `inventory` and `corpus` and the directory tree and file bytes are identical before and after; mutant creates `documents/` or a lock; (e) drift, missing-body, retired and conflict classification from hand-made records; mutant hides drift; (f) the rendered tool reply (supplied by the tool surface test) contains the encoded text byte for byte within 8192 bytes; mutant HTML-escapes the template output; (g) the 512 KiB body cap: 524288 bytes saves and 524289 refuses with `capacity` and no effect; (h) error classes: an injected stale version before any effect passes `stale` through with no-save wording, and an injected failure after the body and before the record gives `partial_publication` with the completed events and the adopt recovery and never the no-save wording; mutant maps both to one code; (i) ordinary relocation windows (operation `None`): an injected failure after S1 gives `partial_publication` and the W1 recovery (remove the unclaimed copy, repeat) completes with the original identifier unchanged; an injected failure after S2 gives the W2 recovery (remove the unclaimed leftover) and leaves the moved document `Managed` under its original identifier; a repeat of `relocate` onto an occupied destination, even one with equal bytes, refuses `stale` and changes nothing; mutant adopts the destination (a new identifier appears), overwrites the equal destination, or retires the record in W2.

### documents-and-references r3
Raised from revision 2: `relocate` takes `MoveBasis`, `Observation` gains `record_sha256` and `move_basis`, an identity-bearing scope resumes interrupted moves (section 7.2), and every other function in this boundary is unchanged.
Provider: Markdown owner. Consumer: compaction. Functions: `observe`, `markdown::outline`, `save`, `adopt`, `remove`, `relocate`, `integrity`, `incoming`, `rewrite`, `Overlay`, `Coverage`, `resolve`, `BODY_CAP`, `Scope`.
Mapping: `observe` is `documents::observe` (a managed document is always free prose; a retired record is the tombstone and stays resolvable); sections are `markdown::outline` (ordinals are zero-based over real headings, the preamble is `Selector::Preamble` and has no ordinal); `incoming` is `references::incoming` (per target; `Coverage.gaps` are the unknown scopes, `Coverage.version` the digest); `post_state` is `references::integrity`; create and replace are `documents::save`; move is `documents::relocate`; remove is `documents::remove` (retires the record, keeps the DOC identity); `body_cap` is `documents::BODY_CAP`. The root README is rewritable through `save` like any managed path. Every call carries the compaction operation identity in `Scope.operation`, one per scope. A supplied operation identity uses storage Attest::Required for all body and record publications and removals; ordinary operations with no identity use Attest::Optional. No document operation invents a second identity or journal.
Scenarios: (a) a section replace leaves every byte outside it identical (CRLF and BOM fixture) and refuses a heading of level `<= L`, an unclosed fence and a stale version with no effect; mutant normalizes line endings; (b) injected storage failure between body and record reports `partial_publication`, the completed events and the adopt recovery; mutant swallows the failure or retries; (c) `incoming` finds links, bare IDs and record tokens in every home; a corrupt work record, a corrupt typed record, an unknown schema, a non-UTF-8 doc and an unrecognized entry in a home each make `Coverage.complete` false with a named gap; mutant counts tokens in a corrupt record as covered, or skips `documents/`; (d) `integrity` reports a link introduced dangling by removing a target or heading, treats a moved or retired DOC identifier as not dangling, shows no conflict between a record and its own moved path, and reports unknown coverage; mutant ignores overlay removals or the live record move; (e) `rewrite` changes only matching destinations and keeps all other bytes; mutant rewrites text outside links; (f) `relocate` order (new body, record, old removal): the three typed events appear in that order and the identifier, `body_sha256` and `created_at` are unchanged while `path`, `revision` and `updated_at` change; mutant removes first or mints a new identifier; (h) identity-bearing resume with a real oracle fixture (the temporary-root store with its journal, or the storage oracle substitute that returns `Attested`, `Partial`, `Unknown` and `Foreign`): after an injected stop following S1 the same call with the same operation identity and the same `MoveBasis` completes S2 and S3 with no allocator call, no create step and no `operation_repeat` error; after an injected stop following S2 it completes S3 only and the record is not retired and not rewritten; a repeat after completion publishes nothing and reports `AlreadyApplied`; a stop with an identical copy at `to` that the oracle cannot attest (hand made, written by another operation identity or journal removed) is refused `resume_unproven` with no effect, no adoption and no overwrite; a leftover at `from` edited after the stop, a record whose `before_sha256` differs from `basis.record_sha256`, a stale `basis.version` and an unexpected extra row each refuse with no effect; mutants: treat equal bytes as proof, call `adopt` in resume, retire the record in W2, skip the `basis` checks, delete and recreate (storage refuses `operation_repeat`); (g) `compactions/` content is never a source: a proposal that mentions a path is not an incoming row; mutant scans it.

Bootstrap compatibility scenarios (private to the owner, reported in the same evidence): fresh registration, completed legacy registration (nothing created), partial retry with an edited README, root with existing `docs/` files, symlinked `docs`.

## 12 Producer seam for the document tool

The owner supplies the payload, schema type, description text and locked handler of the document tool in one new file, `src/tools/document_ops.rs`. It owns no registration, routing, catalog entry, template, `mod` line other than a placeholder, `input.rs`, `read.rs` or `main.rs`. It reuses the tool surface's common argument decoding (`project`, `version`, `actor`), field-named validation helper and the existing `work::Ack` with its shared `work::ack` builder; it defines no second `Common`, decoder, schema function, result type, renderer or Git call. The dispatcher takes the root write lock once, calls the handler, settles Git and renders; the handler never locks.

```rust
pub const DESCRIPTION: &str; // bounded ordinary prose, no YAML-sensitive punctuation
#[derive(serde::Deserialize, schemars::JsonSchema)]
#[serde(tag = "op", rename_all = "snake_case", deny_unknown_fields)]
pub enum DocumentOp {
    /// Replace or create the whole body.
    Save { #[serde(rename = "ref")] target: String, purpose: Option<String>, body: String, #[serde(default)] wire: BodyWire },
    /// Replace the body of one section or the preamble; the heading line is kept.
    ReplaceSection { #[serde(rename = "ref")] target: String, section: SectionArg, body: String, #[serde(default)] wire: BodyWire },
    /// Record the current bytes without rewriting them.
    Adopt { #[serde(rename = "ref")] target: String, purpose: Option<String> },
    /// Remove the file and retire its record.
    Remove { #[serde(rename = "ref")] target: String },
    /// Move to a new absent path. `version` is the source observation and `to_version` the destination's.
    Relocate { #[serde(rename = "ref")] target: String, to: String, to_version: String },
}
#[derive(serde::Deserialize, schemars::JsonSchema, Default)]
#[serde(rename_all = "snake_case")]
pub enum BodyWire { #[default] Raw, Escaped }          // Escaped decodes through markdown::decode
#[derive(serde::Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SectionArg { pub preamble: Option<bool>, pub ordinal: Option<usize>, pub heading: Option<String>, pub level: Option<u8>, pub occurrence: Option<usize> }

/// Handler. The caller already holds the write lock for `store`; never lock here. Maps `Receipt` to the existing acknowledgement.
pub fn execute_locked(store: &Store, guard: &store::LockGuard, common: &input::Common, op: DocumentOp, effects: &mut Vec<String>) -> store::Result<work::Ack>;
```

Rules: `version` in the common arguments is `Observation.version` of `ref` (an absent path has one). `ref` is a path or a DOC id (`Ref::parse`); a section selector is exactly one of `preamble: true`, `ordinal`, or `heading` (with optional `level` and `occurrence`) and is built by `Selector::from_parts`, which names the offending field. `body` is a JSON string; `wire = escaped` decodes with `markdown::decode` and `raw` uses the string as is; a `body` that decodes to NUL or exceeds `BODY_CAP` refuses `invalid_arguments` naming `body`. `actor` is passed through as the declared writer, never inferred. The handler passes `Scope.operation = None`; compaction calls the domain functions directly with its own identities.

`Ack` mapping (built with the shared `work::ack`; `Ack` is `{target, version, phase, phase_label, changed}` plus the additive `notes` and `refs` that the tool surface adds; there is no revision field): `target` is the DOC id when a record exists after the call (including a retired one), else the path; `version` is the combined body and record observation after the call, taken from `observe(Ref::Id(id))` when a DOC id is the target and from `observe(path)` otherwise, so it is directly usable as the next write version; `phase` is the lowercase state label after the call, exactly one of `managed`, `unmanaged`, `drifted`, `missing_body`, `retired`, `absent` (a save and an adopt end `managed`; a remove ends `retired` when a record existed and `absent` when none did; a relocation ends `managed` or `unmanaged`); `phase_label` is `Document`, provided by the shared builder's DOC identifier and managed-path rules in the committed registry proposal; `changed` is `Receipt.changed`. The record revision, when useful, is a note line (`revision 3`). `refs` follow the committed tool surface limit of at most 16 entries of at most 256 bytes: the DOC identifier when a record exists and the exact affected paths, including both relocation paths. Every supported path fits. No reference is truncated or silently omitted; exact publication paths also remain in `store.publications()`. `notes` are at most 8 plain lines of 200 bytes, in this order: warnings, the reference check summary with the incoming count and up to 4 introduced dangling links, the recovery text of a partial result; when more detail exists the last note states the omitted count and the route `get_context ref=<target> view=references`. The handler never adds a Git line. Partial and passthrough errors are returned as the section 7.1 codes. A partial result is not a commit trigger: the effects ledger still shows every completed `Published` string and the typed events stay in `store.publications()`, so the dispatcher can report the exact pending effects without certifying a commit; the recovery step the message names (for example `adopt`) is itself an ordinary mutation and, when it changes bytes or metadata, a trigger.

Read side, owned by the tool surface and fed by this owner: `get_context` for a document reference builds `Selector::from_parts` from its optional arguments, calls `documents::observe` then `documents::read` with the snapshot it printed earlier and the budget it measured, and renders; `search` calls `documents::corpus` (and `incoming` for attention). The owner defines no read tool and no template.

## 13 Open items outside this contract

- The storage boundary this owner consumes must cover the operation oracle (`persist::effect_status` returning the complete attested set, with `Attest::Required` publications) in addition to the primitives it covers today; the owner cannot implement section 7.2 against a boundary that does not name it. The current pin names the primitives and the optional attestation, not the oracle, so storage raises its boundary and this owner re-pins before agreement.
- The exact storage and allocator identifiers and revisions come from their providers' contract entries; this artifact states the calls it needs and withdraws the revision-1 `foreign` list. The owner pins the provider revisions that are committed at agreement time and raises its own entries when a provider change affects a call.
- A checklist-item existence accessor on typed knowledge records is needed for full child verification in `resolve`.
- The tool surface decides how selectors appear in its read arguments; both forms resolve identically.
- The owner chose automatic local commits after every successful actual mutation; the commit engine, receipts and settlement belong to storage and the tool surface. This artifact only fixes which document outcomes are mutations (changed bytes or metadata) and which are not (reads, no-ops, partial or failed results). Metadata and body partial writes keep the exact typed events and stay honestly pending; nothing certifies them as committed.
