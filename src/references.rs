//! Reference integrity for managed documents: the typed and path reference grammar, proof of
//! existence, outgoing and incoming scans certified only by validated records, the integrity
//! preview of a proposed overlay, and byte-preserving link rewriting.
//!
//! Records are never parsed here: each owner's loader validates its records and this module only
//! searches the validated bytes for identifier and path tokens. A record that fails validation is a
//! named gap and makes coverage incomplete; unknown coverage is never reported as zero.
use crate::{
    documents::{self, DocPath, Gap, GapReason, Observation, Port, Ref, State, Unsupported},
    markdown,
    store::{Error, Result, Store},
};
use std::collections::{BTreeMap, BTreeSet};
use std::ops::Range;

/// Name of the reference dialect reported in coverage.
pub const REFS_DIALECT: &str = "md-refs-v1";
/// Parser limits always reported with coverage.
pub const LIMITS: &[&str] = &[
    "multi-line code spans",
    "HTML href",
    "wiki links",
    "setext fragments",
    "links in block quotes and lists that need container parsing",
    "reference-style links with undefined labels",
];

/// A reference target.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Target {
    /// `E-001`, `M-001`, `A-001`, `M-001/T-001`, `M-001/A-001`.
    Work(String),
    /// `D-`, `RB-`, `RS-`, `CL-`, `DOC-`, `CP-` plus canonical digits, or `CL-001/I-001`.
    Knowledge(String),
    /// A managed document path with an optional fragment.
    Doc {
        /// Managed path.
        path: DocPath,
        /// `slug-v1` fragment.
        fragment: Option<String>,
    },
}

impl Target {
    /// Canonical text form.
    pub fn canonical(&self) -> String {
        match self {
            Target::Work(s) | Target::Knowledge(s) => s.clone(),
            Target::Doc {
                path,
                fragment: None,
            } => path.as_str().into(),
            Target::Doc {
                path,
                fragment: Some(f),
            } => format!("{}#{f}", path.as_str()),
        }
    }
}

/// Canonical number: positive, formatted with at least three digits and no other leading zero.
fn canon_digits(d: &str) -> bool {
    !d.is_empty()
        && d.len() <= 18
        && d.parse::<u64>()
            .is_ok_and(|n| n > 0 && d == format!("{n:03}"))
}

/// Parse a whole identifier string strictly.
fn parse_id(s: &str) -> Option<Target> {
    let (head, child) = match s.split_once('/') {
        Some((h, c)) => (h, Some(c)),
        None => (s, None),
    };
    let (prefix, digits) = head.split_once('-')?;
    if !canon_digits(digits) {
        return None;
    }
    let child_ok = |allowed: &[&str]| match child {
        None => true,
        Some(c) => c
            .split_once('-')
            .is_some_and(|(p, d)| allowed.contains(&p) && canon_digits(d)),
    };
    match prefix {
        "M" if child_ok(&["T", "A"]) => Some(Target::Work(s.into())),
        "E" | "A" if child.is_none() => Some(Target::Work(s.into())),
        "CL" if child_ok(&["I"]) => Some(Target::Knowledge(s.into())),
        "D" | "RB" | "RS" | "DOC" | "CP" if child.is_none() => Some(Target::Knowledge(s.into())),
        _ => None,
    }
}

/// Parse one reference string. Valid: canonical IDs, a managed path, a managed path with
/// `#fragment`. Errors: `invalid_arguments`.
pub fn parse(raw: &str) -> Result<Target> {
    let bad = |why: &str| Error::new("invalid_arguments", format!("reference: {why}."));
    if let Some(t) = parse_id(raw) {
        return Ok(t);
    }
    let (path, fragment) = match raw.split_once('#') {
        Some((p, f))
            if !f.is_empty()
                && f.chars()
                    .all(|c| c.is_alphanumeric() || matches!(c, '-' | '_')) =>
        {
            (p, Some(f.to_owned()))
        }
        Some(_) => return Err(bad("invalid fragment")),
        None => (raw, None),
    };
    DocPath::parse(path)
        .map(|path| Target::Doc { path, fragment })
        .map_err(|_| bad("not a canonical identifier or managed path"))
}

/// Proof of existence for a target.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Resolution {
    /// The target exists.
    Found,
    /// A DOC identifier whose record is retired (historic metadata).
    Retired,
    /// The target does not exist.
    Missing,
    /// The document exists but has no such section.
    MissingSection,
    /// The native file is not a supported document.
    Unsupported(Unsupported),
    /// Existence could not be proven; never treated as proof.
    Unknown(String),
}

/// Resolve `target`. Reads only; the caller holds at least a shared lock.
pub fn resolve(port: &dyn Port, target: &Target) -> Result<Resolution> {
    match target {
        Target::Work(id) | Target::Knowledge(id) => port.resolve_id(id),
        Target::Doc { path, fragment } => {
            let obs = documents::observe(port, &Ref::Path(path.clone()))?;
            Ok(match obs.state {
                State::Unmanaged | State::Managed | State::Drifted => {
                    match (fragment, &obs.outline) {
                        (None, _) => Resolution::Found,
                        (Some(f), Some(o)) if o.headings.iter().any(|h| &h.slug == f) => {
                            Resolution::Found
                        }
                        (Some(_), Some(o)) if o.setext_candidates > 0 || !o.complete => {
                            Resolution::Unknown("the section may be a setext heading".into())
                        }
                        (Some(_), _) => Resolution::MissingSection,
                    }
                }
                State::Absent | State::MissingBody => Resolution::Missing,
                State::Unsupported(u) => Resolution::Unsupported(u),
                State::Conflict | State::Retired => {
                    Resolution::Unknown("conflicting claims".into())
                }
            })
        }
    }
}

/// Validator for the optional detail reference stored verbatim by typed records: a root-relative
/// managed path with an optional fragment, an existing supported document, a matching fragment.
/// Errors: `invalid_arguments` (typed IDs, dot segments, noncanonical spellings, missing targets).
pub fn valid_reference(port: &dyn Port, raw: &str) -> Result<()> {
    let target = parse(raw)?;
    if !matches!(target, Target::Doc { .. }) || target.canonical() != raw {
        return Err(Error::new(
            "invalid_arguments",
            "reference: use a managed document path such as docs/x.md.",
        ));
    }
    match resolve(port, &target)? {
        Resolution::Found => Ok(()),
        _ => Err(Error::new(
            "invalid_arguments",
            "reference: the document or section does not exist.",
        )),
    }
}

// ---------------------------------------------------------------------------------------------
// Tokens in prose and records
// ---------------------------------------------------------------------------------------------

/// How a reference was written.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Via {
    /// A Markdown link or reference definition.
    MarkdownLink,
    /// A bare typed identifier in prose.
    BareId,
    /// A token inside a validated record.
    RecordToken,
}

/// Kind of a reference source.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum SourceKind {
    /// A Markdown document under `docs/`.
    Markdown,
    /// The root `README.md`.
    Readme,
    /// An Epic, Module or Atomic record.
    Work,
    /// A typed knowledge record.
    Knowledge,
    /// A DOC metadata record.
    DocMetadata,
    /// `project.yaml`.
    Project,
}

/// One source of references.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Source {
    /// Source kind.
    pub kind: SourceKind,
    /// Record identifier or document path.
    pub id_or_path: String,
}

/// One reference found in a source.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Link {
    /// What it points to.
    pub target: Target,
    /// How it was written.
    pub via: Via,
    /// One-based line (0 for record tokens).
    pub line: usize,
    /// Byte range of the destination or token in the source bytes.
    pub raw: Range<usize>,
}

/// Where a Markdown destination points.
enum Dest {
    /// A URL with a scheme.
    External,
    /// Outside the managed namespace.
    Outside,
    /// A managed document and optional fragment.
    Doc(DocPath, Option<String>),
}

/// Percent-decode `s`; `None` when the result is not UTF-8 or an escape is malformed.
fn percent_decode(s: &str) -> Option<String> {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' {
            let h = std::str::from_utf8(b.get(i + 1..i + 3)?).ok()?;
            out.push(u8::from_str_radix(h, 16).ok()?);
            i += 3;
        } else {
            out.push(b[i]);
            i += 1;
        }
    }
    String::from_utf8(out).ok()
}

/// Classify one link destination written in `from`.
fn classify_dest(from: &DocPath, dest: &str) -> Dest {
    let scheme = dest.split_once(':').is_some_and(|(s, _)| {
        s.chars().next().is_some_and(|c| c.is_ascii_alphabetic())
            && s.chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '.' | '-'))
    });
    if scheme {
        return Dest::External;
    }
    let (path_part, frag) = match dest.split_once('#') {
        Some((p, f)) => (p, Some(f)),
        None => (dest, None),
    };
    let Some(path_part) = percent_decode(path_part) else {
        return Dest::Outside;
    };
    let frag = match frag {
        None => None,
        Some(f) => match percent_decode(f) {
            Some(f) if !f.is_empty() => Some(f),
            _ => return Dest::Outside,
        },
    };
    if path_part.is_empty() {
        return Dest::Doc(from.clone(), frag);
    }
    if path_part.starts_with('/') {
        return Dest::Outside;
    }
    let mut parts: Vec<&str> = if from.dir().is_empty() {
        Vec::new()
    } else {
        from.dir().split('/').collect()
    };
    for seg in path_part.split('/') {
        match seg {
            "" | "." => {}
            ".." => {
                if parts.pop().is_none() {
                    return Dest::Outside;
                }
            }
            s => parts.push(s),
        }
    }
    match DocPath::parse(&parts.join("/")) {
        Ok(p) => Dest::Doc(p, frag),
        Err(_) => Dest::Outside,
    }
}

/// Identifier tokens in `text` as `(range, target)` pairs, longest match at word starts.
fn id_tokens(text: &str) -> Vec<(Range<usize>, Target)> {
    let mut out = Vec::new();
    let b = text.as_bytes();
    let mut i = 0;
    while i < b.len() {
        if !text.is_char_boundary(i) {
            i += 1;
            continue;
        }
        let start_ok = i == 0 || {
            let prev = text[..i].chars().next_back().unwrap_or(' ');
            !(prev.is_alphanumeric() || matches!(prev, '_' | '-' | '/'))
        };
        if start_ok
            && b[i].is_ascii_uppercase()
            && let Some(end) = id_end(text, i)
            && let Some(t) = parse_id(&text[i..end])
        {
            out.push((i..end, t));
            i = end;
            continue;
        }
        i += 1;
    }
    out
}

/// End of the maximal identifier-looking token starting at `i`, honoring the right boundary.
fn id_end(text: &str, i: usize) -> Option<usize> {
    let b = text.as_bytes();
    let mut j = i;
    while j < b.len() && b[j].is_ascii_uppercase() {
        j += 1;
    }
    if j == i || j >= b.len() || b[j] != b'-' {
        return None;
    }
    j += 1;
    let d0 = j;
    while j < b.len() && b[j].is_ascii_digit() {
        j += 1;
    }
    if j == d0 {
        return None;
    }
    let mut end = j;
    if b.get(j) == Some(&b'/') {
        let mut k = j + 1;
        let p0 = k;
        while k < b.len() && b[k].is_ascii_uppercase() {
            k += 1;
        }
        if k > p0 && b.get(k) == Some(&b'-') {
            k += 1;
            let c0 = k;
            while k < b.len() && b[k].is_ascii_digit() {
                k += 1;
            }
            if k > c0 {
                end = k;
            }
        }
    }
    let next = text[end..].chars().next();
    if next.is_some_and(|c| c.is_alphanumeric() || c == '_') {
        return None;
    }
    Some(end)
}

/// Managed path tokens (`README.md`, `docs/...md`, optional `#fragment`) inside raw record text.
fn path_tokens(text: &str) -> Vec<(Range<usize>, Target)> {
    let mut out = Vec::new();
    let b = text.as_bytes();
    let mut i = 0;
    while i < b.len() {
        if !text.is_char_boundary(i) {
            i += 1;
            continue;
        }
        let at_start = text[i..].starts_with("README.md") || text[i..].starts_with("docs/");
        let boundary = i == 0 || {
            let prev = text[..i].chars().next_back().unwrap_or(' ');
            !(prev.is_alphanumeric() || matches!(prev, '/' | '.' | '_' | '-'))
        };
        if at_start && boundary {
            let mut j = i;
            while j < b.len()
                && (b[j].is_ascii_alphanumeric() || matches!(b[j], b'.' | b'_' | b'-' | b'/'))
            {
                j += 1;
            }
            let mut end = j;
            while end > i && text[i..end].ends_with('.') && !text[i..end].ends_with(".md") {
                end -= 1;
            }
            if let Ok(path) = DocPath::parse(&text[i..end]) {
                let mut stop = end;
                let mut fragment = None;
                if b.get(end) == Some(&b'#') {
                    let mut k = end + 1;
                    while k < b.len()
                        && (b[k].is_ascii_alphanumeric() || matches!(b[k], b'-' | b'_'))
                    {
                        k += 1;
                    }
                    if k > end + 1 {
                        fragment = Some(text[end + 1..k].to_owned());
                        stop = k;
                    }
                }
                out.push((i..stop, Target::Doc { path, fragment }));
                i = stop;
                continue;
            }
        }
        i += 1;
    }
    out
}

/// Parse a link destination at `k` in `s`; returns the destination range and the end of the
/// consumed text. `inline` requires the closing `)`.
fn dest_at(s: &str, mut k: usize, inline: bool) -> Option<(Range<usize>, usize)> {
    let b = s.as_bytes();
    while k < b.len() && (b[k] == b' ' || b[k] == b'\t') {
        k += 1;
    }
    let (dest, mut after) = if b.get(k) == Some(&b'<') {
        let close = s[k + 1..].find('>')? + k + 1;
        (k + 1..close, close + 1)
    } else {
        let mut depth = 0i32;
        let mut e = k;
        while e < b.len() {
            match b[e] {
                b' ' | b'\t' => break,
                b'(' => depth += 1,
                b')' => {
                    if depth == 0 {
                        break;
                    }
                    depth -= 1;
                }
                _ => {}
            }
            e += 1;
        }
        (k..e, e)
    };
    if dest.is_empty() {
        return None;
    }
    if inline {
        while after < b.len() && (b[after] == b' ' || b[after] == b'\t') {
            after += 1;
        }
        if let Some(q) = b
            .get(after)
            .copied()
            .filter(|c| matches!(c, b'"' | b'\'' | b'('))
        {
            let close = if q == b'(' { b')' } else { q };
            after = s[after + 1..].find(close as char)? + after + 2;
            while after < b.len() && (b[after] == b' ' || b[after] == b'\t') {
                after += 1;
            }
        }
        if b.get(after) != Some(&b')') {
            return None;
        }
        after += 1;
    }
    Some((dest, after))
}

/// Mask of positions inside inline code spans of one line.
fn code_mask(s: &str) -> Vec<bool> {
    let b = s.as_bytes();
    let mut mask = vec![false; b.len()];
    let mut i = 0;
    while i < b.len() {
        if b[i] != b'`' {
            i += 1;
            continue;
        }
        let n = b[i..].iter().take_while(|c| **c == b'`').count();
        let mut j = i + n;
        let mut close = None;
        while j < b.len() {
            if b[j] == b'`' {
                let m = b[j..].iter().take_while(|c| **c == b'`').count();
                if m == n {
                    close = Some(j + m);
                    break;
                }
                j += m;
            } else {
                j += 1;
            }
        }
        match close {
            Some(end) => {
                mask[i..end].iter_mut().for_each(|m| *m = true);
                i = end;
            }
            None => i += n,
        }
    }
    mask
}

/// References found in one Markdown document and the count of constructs left unparsed.
pub struct Scanned {
    /// Links and bare identifiers.
    pub links: Vec<Link>,
    /// `href=`, `<a ` and `[[` constructs seen outside code.
    pub unparsed: usize,
}

/// Scan one document's prose for links, reference definitions and bare typed identifiers.
pub fn scan_markdown(from: &DocPath, bytes: &[u8]) -> Result<Scanned> {
    let text = std::str::from_utf8(bytes)
        .map_err(|_| Error::new("encoding", "The document is not valid UTF-8."))?;
    let mut links = Vec::new();
    let mut unparsed = 0;
    let line_starts: Vec<usize> = std::iter::once(0)
        .chain(text.match_indices('\n').map(|(i, _)| i + 1))
        .collect();
    for range in markdown::prose_lines(bytes)? {
        let s = &text[range.clone()];
        let line_no = line_starts.partition_point(|st| *st <= range.start);
        let mask = code_mask(s);
        let mut dests: Vec<Range<usize>> = Vec::new();
        let mut j = 0;
        while j + 1 < s.len() {
            if !mask[j]
                && s.is_char_boundary(j)
                && s[j..].starts_with("](")
                && let Some((d, after)) = dest_at(s, j + 2, true)
            {
                dests.push(d);
                j = after;
                continue;
            }
            j += 1;
        }
        let indent = s.len() - s.trim_start_matches(' ').len();
        if indent <= 3
            && s[indent..].starts_with('[')
            && let Some(p) = s.find("]:")
            && let Some((d, _)) = dest_at(s, p + 2, false)
        {
            dests.push(d);
        }
        for d in &dests {
            if let Dest::Doc(path, fragment) = classify_dest(from, &s[d.clone()]) {
                links.push(Link {
                    target: Target::Doc { path, fragment },
                    via: Via::MarkdownLink,
                    line: line_no,
                    raw: range.start + d.start..range.start + d.end,
                });
            }
        }
        for (r, t) in id_tokens(s) {
            if mask[r.start] || dests.iter().any(|d| d.start <= r.start && r.end <= d.end) {
                continue;
            }
            links.push(Link {
                target: t,
                via: Via::BareId,
                line: line_no,
                raw: range.start + r.start..range.start + r.end,
            });
        }
        for marker in ["href=", "<a ", "[["] {
            unparsed += s.match_indices(marker).filter(|(i, _)| !mask[*i]).count();
        }
    }
    Ok(Scanned { links, unparsed })
}

/// Outgoing references of one observed document with their proof of existence.
pub struct Outgoing {
    /// The reference.
    pub link: Link,
    /// Existence proof.
    pub resolution: Resolution,
}

/// Outgoing references and coverage of one document.
pub struct OutgoingResult {
    /// References in document order.
    pub rows: Vec<Outgoing>,
    /// Coverage of this single document scan.
    pub coverage: Coverage,
}

/// Outgoing references of an observed document, each resolved.
pub fn outgoing(port: &dyn Port, obs: &Observation) -> Result<OutgoingResult> {
    let Some(body) = &obs.body else {
        return Ok(OutgoingResult {
            rows: Vec::new(),
            coverage: Coverage::empty(true),
        });
    };
    let scanned = scan_markdown(&obs.path, body)?;
    let mut cache: BTreeMap<Target, Resolution> = BTreeMap::new();
    let mut rows = Vec::new();
    for link in scanned.links {
        let resolution = match cache.get(&link.target) {
            Some(r) => r.clone(),
            None => {
                let r = resolve(port, &link.target)?;
                cache.insert(link.target.clone(), r.clone());
                r
            }
        };
        rows.push(Outgoing { link, resolution });
    }
    let mut coverage = Coverage::empty(scanned.unparsed == 0);
    coverage.unparsed = scanned.unparsed;
    coverage.files_read = 1;
    coverage.bytes_read = body.len() as u64;
    Ok(OutgoingResult { rows, coverage })
}

// ---------------------------------------------------------------------------------------------
// Coverage, records and incoming scan
// ---------------------------------------------------------------------------------------------

/// How completely a scan covered the namespace.
#[derive(Clone, Debug)]
pub struct Coverage {
    /// Every home listed, every file loaded and read within caps, no gap, nothing unparsed.
    pub complete: bool,
    /// Files read.
    pub files_read: usize,
    /// Bytes read.
    pub bytes_read: u64,
    /// Named omissions.
    pub gaps: Vec<Gap>,
    /// Constructs seen but not interpreted.
    pub unparsed: usize,
    /// Declared parser limits.
    pub limits: &'static [&'static str],
    /// Digest of every scanned file version and home.
    pub version: String,
}

impl Coverage {
    /// An empty coverage with the given completeness.
    fn empty(complete: bool) -> Self {
        Coverage {
            complete,
            files_read: 0,
            bytes_read: 0,
            gaps: Vec::new(),
            unparsed: 0,
            limits: LIMITS,
            version: String::new(),
        }
    }
}

/// One validated record file handed over by its owner's loader.
#[derive(Clone, Debug)]
pub struct RecordFile {
    /// Relative path.
    pub rel: String,
    /// Source classification.
    pub source: Source,
    /// Exact validated bytes.
    pub bytes: Vec<u8>,
}

/// The validated records of one home.
#[derive(Clone, Debug, Default)]
pub struct RecordSet {
    /// Validated files.
    pub files: Vec<RecordFile>,
    /// Named omissions (unreadable, unknown schema, unrecognized entries, missing loader).
    pub gaps: Vec<Gap>,
    /// False when the home could not be fully validated.
    pub complete: bool,
}

/// The structured homes whose records are searched.
pub const HOMES: [&str; 6] = [
    "work",
    "project",
    "decisions",
    "runbooks",
    "research",
    "checklists",
];

/// Interim loader over the current store: work records through the work loader, the manifest through
/// the project loader, and a named gap for a typed home that exists, because the knowledge owner's
/// loader is not available yet.
pub fn interim_records(store: &Store, home: &str) -> Result<RecordSet> {
    let mut set = RecordSet {
        complete: true,
        ..Default::default()
    };
    match home {
        "work" => {
            let scan = store.scan(None)?;
            set.complete = scan.complete;
            for m in scan.modules {
                let rel = crate::model::work_path(&m.value.id).map_err(crate::store::invalid)?;
                set.files.push(RecordFile {
                    rel,
                    source: Source {
                        kind: SourceKind::Work,
                        id_or_path: m.value.id.clone(),
                    },
                    bytes: m.bytes,
                });
            }
            for name in scan.unreadable {
                set.gaps.push(Gap {
                    what: documents::quote(&name),
                    reason: GapReason::Unreadable,
                });
            }
        }
        "project" => match store.project() {
            Ok(Some(p)) => set.files.push(RecordFile {
                rel: "project.yaml".into(),
                source: Source {
                    kind: SourceKind::Project,
                    id_or_path: "project.yaml".into(),
                },
                bytes: p.bytes,
            }),
            Ok(None) => {}
            Err(_) => {
                set.complete = false;
                set.gaps.push(Gap {
                    what: "project.yaml".into(),
                    reason: GapReason::Unreadable,
                });
            }
        },
        typed => {
            let path = store.path(typed)?;
            if std::fs::symlink_metadata(&path).is_ok() {
                let nonempty = std::fs::read_dir(&path)
                    .map(|mut d| d.next().is_some())
                    .unwrap_or(true);
                if nonempty {
                    set.complete = false;
                    set.gaps.push(Gap {
                        what: format!("{typed}/"),
                        reason: GapReason::LoaderUnavailable,
                    });
                }
            }
        }
    }
    Ok(set)
}

/// Interim existence proof over the current store (work records, DOC records, CP files); typed
/// knowledge identifiers are unknown until the knowledge owner's loader lands.
pub fn interim_resolve(store: &Store, port: &dyn Port, id: &str) -> Result<Resolution> {
    let Some(target) = parse_id(id) else {
        return Ok(Resolution::Missing);
    };
    let (head, child) = match id.split_once('/') {
        Some((h, c)) => (h, Some(c)),
        None => (id, None),
    };
    match target {
        Target::Work(_) => match store.module(head) {
            Err(e) if e.code == "not_found" => Ok(Resolution::Missing),
            Err(e) => Ok(Resolution::Unknown(format!(
                "work record unreadable ({})",
                e.code
            ))),
            Ok(snap) => Ok(match child {
                None => Resolution::Found,
                Some(c)
                    if snap
                        .value
                        .tasks
                        .iter()
                        .chain(snap.value.atomics.iter())
                        .any(|t| t.id == c) =>
                {
                    Resolution::Found
                }
                Some(_) => Resolution::Missing,
            }),
        },
        Target::Knowledge(_) if head.starts_with("DOC-") => {
            let records = documents::load_records(port)?;
            Ok(match records.by_id(head) {
                Some(e) if e.record.state == documents::RecordState::Retired => Resolution::Retired,
                Some(_) => Resolution::Found,
                None if records.complete => Resolution::Missing,
                None => Resolution::Unknown("document records are incomplete".into()),
            })
        }
        Target::Knowledge(_) if head.starts_with("CP-") => Ok(
            match port.read(&format!("compactions/{head}.yaml"), documents::BODY_CAP)? {
                documents::Read::Bytes(_) => Resolution::Found,
                documents::Read::Absent => Resolution::Missing,
                _ => Resolution::Unknown("not a regular file".into()),
            },
        ),
        _ => Ok(Resolution::Unknown(
            "the knowledge loader is not available".into(),
        )),
    }
}

/// One aggregated incoming reference source.
#[derive(Clone, Debug)]
pub struct Incoming {
    /// The referring source.
    pub source: Source,
    /// First way it refers.
    pub via: Via,
    /// Number of references.
    pub count: usize,
    /// Fragments named by the references.
    pub fragments: Vec<String>,
}

/// Incoming references to one target with coverage.
pub struct IncomingResult {
    /// The queried target.
    pub target: Target,
    /// Sources in byte order of identifier or path, then kind, then way of referring.
    pub rows: Vec<Incoming>,
    /// Coverage; callers that remove, move or compact must refuse unless complete.
    pub coverage: Coverage,
}

/// Every reference found in every source, with coverage; the common core of `incoming` and
/// `integrity`.
struct Graph {
    /// `(source, reference)` pairs.
    refs: Vec<(Source, Link)>,
    /// Post-state managed files and their outlines (`None` for unsupported files).
    files: BTreeMap<DocPath, Option<markdown::Outline>>,
    /// Known DOC identifiers.
    docs: BTreeSet<String>,
    /// Coverage of the scan.
    coverage: Coverage,
}

/// Apply an overlay to the walk and scan everything.
fn graph(port: &dyn Port, overlay: &Overlay<'_>) -> Result<Graph> {
    let walk = documents::walk(port)?;
    let records = documents::load_records(port)?;
    let mut cov = Coverage::empty(walk.complete && records.complete);
    cov.files_read = walk.files_read;
    cov.bytes_read = walk.bytes_read;
    cov.gaps.extend(walk.gaps.iter().cloned());
    cov.gaps.extend(records.gaps.iter().cloned());
    let mut digest = sha2::Sha256::default();
    let mut docs: BTreeMap<DocPath, Option<Vec<u8>>> = BTreeMap::new();
    for f in &walk.files {
        docs.insert(f.path.clone(), f.bytes().map(|b| b.to_vec()));
    }
    for (from, to) in overlay.moves {
        let bytes = docs.remove(from).flatten();
        docs.insert(to.clone(), bytes);
    }
    for r in overlay.remove {
        docs.remove(r);
    }
    for (p, b) in overlay.put {
        docs.insert(p.clone(), Some(b.clone()));
    }
    let mut out = Graph {
        refs: Vec::new(),
        files: BTreeMap::new(),
        docs: BTreeSet::new(),
        coverage: cov,
    };
    for (path, bytes) in &docs {
        let Some(bytes) = bytes else {
            out.files.insert(path.clone(), None);
            continue;
        };
        use sha2::Digest;
        digest.update(port.version(path.as_str(), Some(bytes)).as_bytes());
        let scanned = scan_markdown(path, bytes)?;
        out.coverage.unparsed += scanned.unparsed;
        let outline = markdown::outline(bytes)?;
        let kind = if path.as_str() == "README.md" {
            SourceKind::Readme
        } else {
            SourceKind::Markdown
        };
        for link in scanned.links {
            out.refs.push((
                Source {
                    kind,
                    id_or_path: path.as_str().into(),
                },
                link,
            ));
        }
        out.files.insert(path.clone(), Some(outline));
    }
    // DOC records: identifiers exist in any state; a path claimed by two active records is a source of
    // the conflicting claim.
    let mut claims: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for e in &records.entries {
        out.docs.insert(e.record.id.clone());
        if e.record.state == documents::RecordState::Active {
            let moved = overlay
                .moves
                .iter()
                .find(|(f, _)| f.as_str() == e.record.path)
                .map(|(_, t)| t.as_str());
            let removed = overlay.remove.iter().any(|r| r.as_str() == e.record.path);
            if removed && moved.is_none() {
                continue;
            }
            claims
                .entry(moved.unwrap_or(&e.record.path).to_owned())
                .or_default()
                .push(e.record.id.clone());
        }
    }
    for (path, ids) in claims.iter().filter(|(_, v)| v.len() > 1) {
        if let Ok(p) = DocPath::parse(path) {
            for id in ids {
                out.refs.push((
                    Source {
                        kind: SourceKind::DocMetadata,
                        id_or_path: id.clone(),
                    },
                    Link {
                        target: Target::Doc {
                            path: p.clone(),
                            fragment: None,
                        },
                        via: Via::RecordToken,
                        line: 0,
                        raw: 0..0,
                    },
                ));
            }
        }
    }
    for home in HOMES {
        let set = port.records(home)?;
        out.coverage.complete &= set.complete;
        out.coverage.gaps.extend(set.gaps.iter().cloned());
        for f in set.files {
            let Ok(text) = std::str::from_utf8(&f.bytes) else {
                out.coverage.complete = false;
                out.coverage.gaps.push(Gap {
                    what: documents::quote(&f.rel),
                    reason: GapReason::NotUtf8,
                });
                continue;
            };
            use sha2::Digest;
            digest.update(port.version(&f.rel, Some(&f.bytes)).as_bytes());
            out.coverage.files_read += 1;
            out.coverage.bytes_read += f.bytes.len() as u64;
            let own = f.source.id_or_path.clone();
            for (r, t) in id_tokens(text).into_iter().chain(path_tokens(text)) {
                if let Target::Work(id) | Target::Knowledge(id) = &t
                    && id.split('/').next() == own.split('/').next()
                {
                    continue;
                }
                out.refs.push((
                    f.source.clone(),
                    Link {
                        target: t,
                        via: Via::RecordToken,
                        line: 0,
                        raw: r,
                    },
                ));
            }
        }
    }
    out.coverage.complete &= out.coverage.unparsed == 0
        && !out
            .coverage
            .gaps
            .iter()
            .any(|g| g.reason == GapReason::Capped);
    {
        use sha2::Digest;
        out.coverage.version = format!("{:x}", digest.finalize());
    }
    Ok(out)
}

/// Whether reference `t` is aimed at the queried `target`.
fn matches(target: &Target, t: &Target) -> bool {
    match (target, t) {
        (
            Target::Doc {
                path: a,
                fragment: fa,
            },
            Target::Doc {
                path: b,
                fragment: fb,
            },
        ) => a == b && (fa.is_none() || fa == fb),
        (a, b) => a == b,
    }
}

/// Complete bounded incoming scan to `target`. Never writes.
pub fn incoming(port: &dyn Port, target: &Target) -> Result<IncomingResult> {
    let g = graph(port, &Overlay::default())?;
    let mut rows: BTreeMap<(String, SourceKind, Via), Incoming> = BTreeMap::new();
    for (source, link) in &g.refs {
        if !matches(target, &link.target) {
            continue;
        }
        if let Target::Doc { path, .. } = target
            && source.id_or_path == path.as_str()
            && matches!(source.kind, SourceKind::Markdown | SourceKind::Readme)
        {
            continue;
        }
        let row = rows
            .entry((source.id_or_path.clone(), source.kind, link.via))
            .or_insert_with(|| Incoming {
                source: source.clone(),
                via: link.via,
                count: 0,
                fragments: Vec::new(),
            });
        row.count += 1;
        if let Target::Doc {
            fragment: Some(f), ..
        } = &link.target
            && !row.fragments.contains(f)
        {
            row.fragments.push(f.clone());
        }
    }
    Ok(IncomingResult {
        target: target.clone(),
        rows: rows.into_values().collect(),
        coverage: g.coverage,
    })
}

// ---------------------------------------------------------------------------------------------
// Integrity preview and update attention
// ---------------------------------------------------------------------------------------------

/// A proposed change to the managed documents, applied in memory only.
#[derive(Default)]
pub struct Overlay<'a> {
    /// Documents created or replaced with these bytes.
    pub put: &'a [(DocPath, Vec<u8>)],
    /// Documents removed (an active record becomes retired).
    pub remove: &'a [DocPath],
    /// Documents moved (the file and its active record move): `(from, to)`.
    pub moves: &'a [(DocPath, DocPath)],
}

/// One reference that points at nothing.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Dangling {
    /// The referring source.
    pub source: Source,
    /// The missing target.
    pub target: Target,
    /// One-based line when known.
    pub line: Option<usize>,
}

/// Result of an integrity preview.
pub struct Integrity {
    /// Dangling references that exist before the change.
    pub dangling_before: usize,
    /// All dangling references after the change.
    pub dangling_after: Vec<Dangling>,
    /// References made dangling by the change.
    pub introduced: Vec<Dangling>,
    /// Coverage; incomplete means unknown.
    pub coverage: Coverage,
}

/// Dangling document references in a graph.
fn dangling_of(g: &Graph) -> Vec<Dangling> {
    let mut out = Vec::new();
    for (source, link) in &g.refs {
        let missing = match &link.target {
            Target::Doc { path, fragment } => match g.files.get(path) {
                None => true,
                Some(Some(o)) => fragment.as_ref().is_some_and(|f| {
                    !o.headings.iter().any(|h| &h.slug == f) && o.setext_candidates == 0
                }),
                Some(None) => false,
            },
            Target::Knowledge(id) if id.starts_with("DOC-") && !id.contains('/') => {
                !g.docs.contains(id)
            }
            _ => false,
        };
        if missing {
            out.push(Dangling {
                source: source.clone(),
                target: link.target.clone(),
                line: (link.line > 0).then_some(link.line),
            });
        }
    }
    out.sort();
    out.dedup();
    out
}

/// Whole-root reference graph with `overlay` applied in memory. Only document targets can become
/// dangling through prose changes. Never writes; incomplete coverage means unknown.
pub fn integrity(port: &dyn Port, overlay: &Overlay<'_>) -> Result<Integrity> {
    let before = dangling_of(&graph(port, &Overlay::default())?);
    let g = graph(port, overlay)?;
    let after = dangling_of(&g);
    let introduced = after
        .iter()
        .filter(|d| !before.contains(d))
        .cloned()
        .collect();
    Ok(Integrity {
        dangling_before: before.len(),
        dangling_after: after,
        introduced,
        coverage: g.coverage,
    })
}

/// Reference attention returned with a mutation.
#[derive(Debug)]
pub enum ReferenceCheck {
    /// No heading, path or removal change.
    Skipped,
    /// Checked with a complete or incomplete scan.
    Checked {
        /// References aimed at the changed paths.
        incoming: usize,
        /// References made dangling by the change.
        introduced_dangling: Vec<Dangling>,
        /// Whether coverage was complete.
        complete: bool,
    },
    /// The check could not run; the mutation proceeds and says so.
    Unknown(String),
}

/// Preview the references affected by an overlay when headings, a path or existence change.
pub fn preview(
    port: &dyn Port,
    overlay: &Overlay<'_>,
    before: Option<&markdown::Outline>,
    after: Option<&markdown::Outline>,
) -> ReferenceCheck {
    let slugs = |o: Option<&markdown::Outline>| {
        o.map(|o| {
            o.headings
                .iter()
                .map(|h| h.slug.clone())
                .collect::<Vec<_>>()
        })
    };
    if overlay.remove.is_empty() && overlay.moves.is_empty() && slugs(before) == slugs(after) {
        return ReferenceCheck::Skipped;
    }
    match integrity(port, overlay) {
        Err(e) => ReferenceCheck::Unknown(e.code.into()),
        Ok(i) => {
            let paths: BTreeSet<&DocPath> = overlay
                .put
                .iter()
                .map(|(p, _)| p)
                .chain(overlay.remove.iter())
                .chain(overlay.moves.iter().map(|(f, _)| f))
                .collect();
            let incoming = i
                .dangling_after
                .iter()
                .filter(|d| matches!(&d.target, Target::Doc { path, .. } if paths.contains(path)))
                .count();
            ReferenceCheck::Checked {
                incoming,
                introduced_dangling: i.introduced,
                complete: i.coverage.complete,
            }
        }
    }
}

// ---------------------------------------------------------------------------------------------
// Link rewrite
// ---------------------------------------------------------------------------------------------

/// A document move to apply to links.
pub struct Rewrite {
    /// Old path.
    pub from: DocPath,
    /// New path.
    pub to: DocPath,
    /// Fragment renames `(old, new)`.
    pub fragments: Vec<(String, String)>,
}

/// Result of a rewrite.
pub struct Rewritten {
    /// Rewritten bytes.
    pub bytes: Vec<u8>,
    /// Destinations changed.
    pub replaced: usize,
    /// Links to the old path that could not be re-expressed.
    pub skipped: Vec<Link>,
}

/// Path of `to` relative to the directory of `source`.
fn relative_path(source: &DocPath, to: &DocPath) -> String {
    let from: Vec<&str> = if source.dir().is_empty() {
        Vec::new()
    } else {
        source.dir().split('/').collect()
    };
    let dest: Vec<&str> = to.as_str().split('/').collect();
    let common = from
        .iter()
        .zip(dest.iter())
        .take_while(|(a, b)| a == b)
        .count()
        .min(dest.len() - 1);
    let mut parts: Vec<&str> = vec![".."; from.len() - common.min(from.len())];
    parts.extend(&dest[common..]);
    parts.join("/")
}

/// Rewrite Markdown link and reference-definition destinations that resolve to a moved document.
/// Changes only those destination bytes. Errors: `encoding`.
pub fn rewrite(source: &DocPath, bytes: &[u8], rules: &[Rewrite]) -> Result<Rewritten> {
    let scanned = scan_markdown(source, bytes)?;
    let mut edits: Vec<(Range<usize>, String)> = Vec::new();
    let mut skipped = Vec::new();
    for link in scanned
        .links
        .into_iter()
        .filter(|l| l.via == Via::MarkdownLink)
    {
        let Target::Doc { path, fragment } = &link.target else {
            continue;
        };
        let Some(rule) = rules.iter().find(|r| &r.from == path) else {
            continue;
        };
        let original = std::str::from_utf8(&bytes[link.raw.clone()]).unwrap_or_default();
        if original.starts_with('#') {
            continue;
        }
        let frag = fragment.as_ref().map(|f| {
            rule.fragments
                .iter()
                .find(|(o, _)| o == f)
                .map_or(f.clone(), |(_, n)| n.clone())
        });
        let mut text = relative_path(source, &rule.to);
        if let Some(f) = frag {
            text = format!("{text}#{f}");
        }
        if text.contains([' ', ')', '(', '<', '>']) {
            skipped.push(link);
            continue;
        }
        edits.push((link.raw, text));
    }
    edits.sort_by_key(|(r, _)| r.start);
    let mut out = Vec::with_capacity(bytes.len());
    let mut at = 0;
    for (r, text) in &edits {
        out.extend_from_slice(&bytes[at..r.start]);
        out.extend_from_slice(text.as_bytes());
        at = r.end;
    }
    out.extend_from_slice(&bytes[at..]);
    Ok(Rewritten {
        bytes: out,
        replaced: edits.len(),
        skipped,
    })
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, reason = "Test assertions")]
    use super::*;

    /// Parse a path, panicking on a test typo.
    fn p(s: &str) -> DocPath {
        DocPath::parse(s).unwrap()
    }

    /// Identifier and path grammar accepts canonical forms only.
    #[test]
    fn parse_accepts_canonical_forms_and_rejects_others() {
        for ok in [
            "M-001",
            "M-1000",
            "M-001/T-001",
            "CL-001/I-001",
            "DOC-007",
            "D-001",
            "docs/x.md",
            "docs/a/b.md#sec-1",
            "README.md",
        ] {
            assert_eq!(parse(ok).unwrap().canonical(), ok);
        }
        for bad in [
            "M-0001",
            "m-001",
            "docs/../x.md",
            "/docs/x.md",
            "http://x",
            "M-001/X-001",
            "E-001/T-001",
            "docs/x.md#",
            "docs/Ünï.md",
            "docs/x.MD",
        ] {
            assert!(parse(bad).is_err(), "{bad}");
        }
    }

    /// Links resolve relative to the source, with fragments, and skip code and external targets.
    #[test]
    fn markdown_scan_resolves_links_ids_and_masks_code() {
        let doc = "See [a](../b.md#sec) and [c](<c d.md>) `[x](docs/no.md)` [e](https://e.org/x.md) M-001 and `M-002`.\n[ref]: sub/z.md\n";
        let s = scan_markdown(&p("docs/dir/a.md"), doc.as_bytes()).unwrap();
        let got: Vec<String> = s.links.iter().map(|l| l.target.canonical()).collect();
        assert_eq!(got, vec!["docs/b.md#sec", "M-001", "docs/dir/sub/z.md"]);
        assert_eq!(&doc[s.links[0].raw.clone()], "../b.md#sec");
        let root = scan_markdown(&p("README.md"), b"[d](docs/x.md)\n[up](../x.md)").unwrap();
        assert_eq!(root.links.len(), 1);
    }

    /// Identifier tokens honor boundaries and child grammar.
    #[test]
    fn id_tokens_respect_boundaries() {
        let toks: Vec<String> =
            id_tokens("x M-001/T-002, DOC-003. docs/M-004 M-0005 AM-006 CL-001/I-002")
                .into_iter()
                .map(|(_, t)| t.canonical())
                .collect();
        assert_eq!(toks, vec!["M-001/T-002", "DOC-003", "CL-001/I-002"]);
    }

    /// Rewrite changes only matching destinations and keeps every other byte.
    #[test]
    fn rewrite_changes_only_destinations() {
        let doc = "\u{feff}Text [l](old.md#a) \"keep old.md\" [m](<old.md>)\r\n[n](other.md)\r\n";
        let rules = [Rewrite {
            from: p("docs/old.md"),
            to: p("docs/new/moved.md"),
            fragments: vec![("a".into(), "b".into())],
        }];
        let out = rewrite(&p("docs/src.md"), doc.as_bytes(), &rules).unwrap();
        assert_eq!(out.replaced, 2);
        assert_eq!(
            String::from_utf8(out.bytes).unwrap(),
            "\u{feff}Text [l](new/moved.md#b) \"keep old.md\" [m](<new/moved.md>)\r\n[n](other.md)\r\n"
        );
        assert_eq!(
            relative_path(&p("docs/a/x.md"), &p("docs/b/y.md")),
            "../b/y.md"
        );
    }
}
