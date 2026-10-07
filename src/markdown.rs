//! Pure Markdown core of the managed-document owner: the `md-dialect-v1` scanner, section ranges,
//! guarded section splicing, `slug-v1` fragments and the lossless `md-text-v1` wire encoding.
//!
//! Nothing here touches the filesystem, the clock or a lock. Every function works on exact byte
//! strings, never normalizes a BOM or a line ending, and reports failures with the stable codes of
//! the contract (`encoding`, `invalid_arguments`, `not_found`, `ambiguous_section`,
//! `structure_change`, `unterminated_heading`, `presentation_capacity`, `capacity`).
use crate::store::{Error, Result};
use std::collections::BTreeMap;
use std::ops::Range;

/// Name of the supported Markdown dialect; it is bound into every read snapshot.
pub const DIALECT: &str = "md-dialect-v1";
/// Name of the wire encoding; it is bound into every read snapshot.
pub const ENCODING: &str = "md-text-v1";
/// Smallest encoded budget a page read accepts; the widest encoded character is 10 bytes, so any
/// accepted budget guarantees progress.
pub const MIN_PAGE_BUDGET: usize = 16;
/// Headings addressed per document; more sets [`Outline::complete`] to false.
pub const HEADING_CAP: usize = 2048;

/// One ATX heading. Every offset is an absolute raw byte offset in the document.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Heading {
    /// Zero-based document-order index; valid only for the bytes it was computed from.
    pub ordinal: usize,
    /// Heading level, 1 to 6.
    pub level: u8,
    /// Raw heading text with surrounding blanks and the closing `#` run removed.
    pub text: String,
    /// One-based position among headings with identical text.
    pub occurrence: usize,
    /// `slug-v1` fragment, with `-1`, `-2` suffixes for later duplicates.
    pub slug: String,
    /// First byte of the heading line.
    pub start: usize,
    /// First byte after the heading line terminator (the document length when unterminated).
    pub body_start: usize,
    /// Exclusive end of the section: the next heading of level `<=` this one, or the document end.
    pub end: usize,
}

/// Structure and byte facts of one document under `md-dialect-v1`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Outline {
    /// Headings in document order.
    pub headings: Vec<Heading>,
    /// Exclusive end of a leading front-matter block, if any.
    pub front_matter_end: Option<usize>,
    /// Start of the first heading, or the document length when there is none.
    pub preamble_end: usize,
    /// Underline lines that look like setext headings; they are never headings.
    pub setext_candidates: usize,
    /// Whether the bytes start with a UTF-8 byte order mark.
    pub bom: bool,
    /// Line terminators that are a bare LF.
    pub lf: usize,
    /// Line terminators that are CRLF.
    pub crlf: usize,
    /// CR bytes that are not followed by LF.
    pub lone_cr: usize,
    /// NUL bytes.
    pub nul: usize,
    /// False when [`HEADING_CAP`] was exceeded and later headings are not addressable.
    pub complete: bool,
}

/// Classification of one line by the scanner.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Kind {
    /// Ordinary scannable text, including heading lines.
    Prose,
    /// Fence opener, fence content or fence closer.
    Fence,
    /// Inside an HTML comment block.
    Comment,
    /// Inside the leading front matter.
    Front,
}

/// One scanned line.
struct Line {
    /// First byte of the line.
    start: usize,
    /// End of the content, before the terminator (and before the CR of a CRLF).
    end: usize,
    /// First byte of the next line (after the terminator).
    next: usize,
    /// Region classification.
    kind: Kind,
    /// Heading level and text when the line is an ATX heading.
    heading: Option<(u8, String)>,
}

/// Result of one scan pass.
struct Scan {
    /// All lines in order.
    lines: Vec<Line>,
    /// Exclusive end of front matter.
    front: Option<usize>,
    /// Setext candidate count.
    setext: usize,
}

/// Leading-space count of at most three spaces; `None` when a tab or four spaces lead the line.
fn indent3(line: &[u8]) -> Option<usize> {
    let n = line.iter().take_while(|b| **b == b' ').count();
    (n <= 3).then_some(n)
}

/// Whether `rest` holds only spaces and tabs.
fn blank(rest: &[u8]) -> bool {
    rest.iter().all(|b| *b == b' ' || *b == b'\t')
}

/// Fence opener: character and run length when `line` opens a fence under rule 5.
fn fence_open(line: &[u8]) -> Option<(u8, usize)> {
    let i = indent3(line)?;
    let rest = &line[i..];
    let ch = *rest.first()?;
    if ch != b'`' && ch != b'~' {
        return None;
    }
    let run = rest.iter().take_while(|b| **b == ch).count();
    if run < 3 || (ch == b'`' && rest[run..].contains(&b'`')) {
        return None;
    }
    Some((ch, run))
}

/// Whether `line` closes a fence opened with `ch` and `len`.
fn fence_close(line: &[u8], ch: u8, len: usize) -> bool {
    let Some(i) = indent3(line) else { return false };
    let rest = &line[i..];
    let run = rest.iter().take_while(|b| **b == ch).count();
    run >= len && blank(&rest[run..])
}

/// Trim spaces and tabs from both ends.
fn trim_blanks(mut text: &[u8]) -> &[u8] {
    while let Some((last, head)) = text.split_last() {
        if *last == b' ' || *last == b'\t' {
            text = head;
        } else {
            break;
        }
    }
    while let Some((first, tail)) = text.split_first() {
        if *first == b' ' || *first == b'\t' {
            text = tail;
        } else {
            break;
        }
    }
    text
}

/// ATX heading level and text for `line`, following rule 7.
fn atx(line: &[u8]) -> Option<(u8, String)> {
    let i = indent3(line)?;
    let rest = &line[i..];
    let level = rest.iter().take_while(|b| **b == b'#').count();
    if !(1..=6).contains(&level) {
        return None;
    }
    let after = &rest[level..];
    if !after.is_empty() && after[0] != b' ' && after[0] != b'\t' {
        return None;
    }
    let mut text = trim_blanks(after);
    let hashes = text.iter().rev().take_while(|b| **b == b'#').count();
    if hashes > 0 {
        let before = &text[..text.len() - hashes];
        if before.is_empty() || before.last().is_some_and(|b| *b == b' ' || *b == b'\t') {
            text = trim_blanks(before);
        }
    }
    Some((level as u8, String::from_utf8_lossy(text).into_owned()))
}

/// Whether `line` is a setext underline: up to three spaces, one run of `=` or `-`, then blanks.
fn setext_underline(line: &[u8]) -> bool {
    let Some(i) = indent3(line) else { return false };
    let rest = &line[i..];
    let Some(ch) = rest.first().copied().filter(|c| *c == b'=' || *c == b'-') else {
        return false;
    };
    let run = rest.iter().take_while(|b| **b == ch).count();
    blank(&rest[run..])
}

/// Index of the first occurrence of `needle` in `hay`.
fn find(hay: &[u8], needle: &[u8]) -> Option<usize> {
    hay.windows(needle.len()).position(|w| w == needle)
}

/// Scan `bytes` (valid UTF-8) line by line into regions and headings.
fn scan(bytes: &[u8]) -> Scan {
    let bom = if bytes.starts_with(&[0xEF, 0xBB, 0xBF]) {
        3
    } else {
        0
    };
    let mut raw: Vec<(usize, usize, usize)> = Vec::new();
    let mut start = 0;
    while start < bytes.len() {
        let (content_end, next) = match bytes[start..].iter().position(|b| *b == b'\n') {
            Some(p) => {
                let lf = start + p;
                let end = if lf > start && bytes[lf - 1] == b'\r' {
                    lf - 1
                } else {
                    lf
                };
                (end, lf + 1)
            }
            None => (bytes.len(), bytes.len()),
        };
        raw.push((start, content_end, next));
        start = next;
    }
    // Front matter: the first line (after a BOM) is exactly `---` and a later line is `---` or `...`.
    let mut front_lines = 0usize;
    let mut front = None;
    if let Some(&(s, e, _)) = raw.first()
        && &bytes[s + bom..e] == b"---"
    {
        for (i, &(ls, le, nx)) in raw.iter().enumerate().skip(1) {
            let content = &bytes[ls..le];
            if content == b"---" || content == b"..." {
                front_lines = i + 1;
                front = Some(nx);
                break;
            }
        }
    }
    let mut lines = Vec::with_capacity(raw.len());
    let mut fence: Option<(u8, usize)> = None;
    let mut comment = false;
    let mut setext = 0usize;
    let mut prev_text = false;
    for (i, &(s, e, nx)) in raw.iter().enumerate() {
        let content = &bytes[if i == 0 { s + bom } else { s }..e];
        let mut heading = None;
        let kind = if i < front_lines {
            Kind::Front
        } else if let Some((ch, len)) = fence {
            if fence_close(content, ch, len) {
                fence = None;
            }
            Kind::Fence
        } else if comment {
            if find(content, b"-->").is_some() {
                comment = false;
            }
            Kind::Comment
        } else if let Some(open) = fence_open(content) {
            fence = Some(open);
            Kind::Fence
        } else if let Some(at) = indent3(content).filter(|n| content[*n..].starts_with(b"<!--")) {
            comment = find(&content[at + 4..], b"-->").is_none();
            Kind::Comment
        } else {
            heading = atx(content);
            Kind::Prose
        };
        if kind == Kind::Prose {
            let underline = heading.is_none() && setext_underline(content);
            if underline && prev_text {
                setext += 1;
            }
            prev_text = heading.is_none() && !underline && !blank(content);
        } else {
            prev_text = false;
        }
        lines.push(Line {
            start: s,
            end: e,
            next: nx,
            kind,
            heading,
        });
    }
    Scan {
        lines,
        front,
        setext,
    }
}

/// Lowercase, keep letters, digits, spaces, hyphens and underscores, spaces become hyphens.
fn slug_of(text: &str) -> String {
    text.to_lowercase()
        .chars()
        .filter(|c| c.is_alphanumeric() || matches!(c, ' ' | '-' | '_'))
        .map(|c| if c == ' ' { '-' } else { c })
        .collect()
}

/// Compute the outline of `bytes`. Errors: `encoding` when the bytes are not valid UTF-8.
pub fn outline(bytes: &[u8]) -> Result<Outline> {
    std::str::from_utf8(bytes)
        .map_err(|_| Error::new("encoding", "The document is not valid UTF-8."))?;
    let scan = scan(bytes);
    let mut headings: Vec<Heading> = Vec::new();
    let mut complete = true;
    let mut seen: BTreeMap<String, usize> = BTreeMap::new();
    let mut slugs: BTreeMap<String, usize> = BTreeMap::new();
    for line in &scan.lines {
        let Some((level, text)) = &line.heading else {
            continue;
        };
        if headings.len() >= HEADING_CAP {
            complete = false;
            break;
        }
        let occurrence = {
            let n = seen.entry(text.clone()).or_insert(0);
            *n += 1;
            *n
        };
        let base = slug_of(text);
        let dup = {
            let n = slugs.entry(base.clone()).or_insert(0);
            let d = *n;
            *n += 1;
            d
        };
        let slug = if dup == 0 {
            base
        } else {
            format!("{base}-{dup}")
        };
        headings.push(Heading {
            ordinal: headings.len(),
            level: *level,
            text: text.clone(),
            occurrence,
            slug,
            start: line.start,
            body_start: line.next,
            end: bytes.len(),
        });
    }
    for i in 0..headings.len() {
        let level = headings[i].level;
        if let Some(next) = headings[i + 1..].iter().find(|h| h.level <= level) {
            headings[i].end = next.start;
        }
    }
    let (mut lf, mut crlf, mut lone_cr, mut nul) = (0, 0, 0, 0);
    for (i, b) in bytes.iter().enumerate() {
        match b {
            b'\n' if i > 0 && bytes[i - 1] == b'\r' => crlf += 1,
            b'\n' => lf += 1,
            b'\r' if bytes.get(i + 1) != Some(&b'\n') => lone_cr += 1,
            0 => nul += 1,
            _ => {}
        }
    }
    Ok(Outline {
        preamble_end: headings.first().map_or(bytes.len(), |h| h.start),
        front_matter_end: scan.front,
        headings,
        setext_candidates: scan.setext,
        bom: bytes.starts_with(&[0xEF, 0xBB, 0xBF]),
        lf,
        crlf,
        lone_cr,
        nul,
        complete,
    })
}

/// Byte ranges of the lines whose text may carry links and identifiers: every line outside front
/// matter, fences and comments. Each range excludes the line terminator. Errors: `encoding`.
pub fn prose_lines(bytes: &[u8]) -> Result<Vec<Range<usize>>> {
    std::str::from_utf8(bytes)
        .map_err(|_| Error::new("encoding", "The document is not valid UTF-8."))?;
    let bom = if bytes.starts_with(&[0xEF, 0xBB, 0xBF]) {
        3
    } else {
        0
    };
    Ok(scan(bytes)
        .lines
        .iter()
        .enumerate()
        .filter(|(_, l)| l.kind == Kind::Prose)
        .map(|(i, l)| (if i == 0 { l.start + bom } else { l.start })..l.end)
        .collect())
}

/// Which part of a document a read or an edit addresses.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Selector {
    /// The whole document.
    Whole,
    /// The bytes before the first heading (BOM and front matter included).
    Preamble,
    /// The section of the heading with this document-order ordinal.
    Ordinal(usize),
    /// The section of the heading with this raw text.
    Heading {
        /// Raw heading text, compared byte for byte.
        text: String,
        /// Restrict to this level when present.
        level: Option<u8>,
        /// One-based occurrence among identical text; required when the text is not unique.
        occurrence: Option<usize>,
    },
}

impl Selector {
    /// Build a selector from loose optional parts. Exactly one of `preamble = true`, `ordinal` or
    /// `heading` is allowed; `level` and `occurrence` apply only with `heading`; none is `Whole`.
    /// Errors: `invalid_arguments` naming the offending part.
    pub fn from_parts(
        preamble: bool,
        ordinal: Option<usize>,
        heading: Option<String>,
        level: Option<u8>,
        occurrence: Option<usize>,
    ) -> Result<Selector> {
        let invalid =
            |field: &str, rule: &str| Error::new("invalid_arguments", format!("{field}: {rule}"));
        let chosen =
            usize::from(preamble) + usize::from(ordinal.is_some()) + usize::from(heading.is_some());
        if chosen > 1 {
            let field = if preamble {
                "preamble"
            } else if ordinal.is_some() {
                "ordinal"
            } else {
                "heading"
            };
            return Err(invalid(
                field,
                "use exactly one of preamble, ordinal or heading",
            ));
        }
        if heading.is_none() {
            if level.is_some() {
                return Err(invalid("level", "applies only with heading"));
            }
            if occurrence.is_some() {
                return Err(invalid("occurrence", "applies only with heading"));
            }
        }
        if level.is_some_and(|l| !(1..=6).contains(&l)) {
            return Err(invalid("level", "must be between 1 and 6"));
        }
        if occurrence == Some(0) {
            return Err(invalid("occurrence", "is one-based"));
        }
        Ok(if preamble {
            Selector::Preamble
        } else if let Some(n) = ordinal {
            Selector::Ordinal(n)
        } else if let Some(text) = heading {
            Selector::Heading {
                text,
                level,
                occurrence,
            }
        } else {
            Selector::Whole
        })
    }
}

/// A selector resolved against one outline: its byte range and a stable key for snapshots.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Resolved {
    /// Absolute raw byte range of the selection.
    pub range: Range<usize>,
    /// `whole`, `preamble` or `ordinal:N`; equal selections resolve to equal keys.
    pub key: String,
    /// Ordinal of the addressed heading, `None` for the whole document and the preamble.
    pub ordinal: Option<usize>,
}

/// Resolve `selector` to a range. Errors: `not_found`, `ambiguous_section` (the text matches more
/// than one heading and no occurrence was given), `capacity` (the outline is incomplete and the
/// ordinal lies beyond the addressable headings).
pub fn resolve_selection(outline: &Outline, total: usize, selector: &Selector) -> Result<Resolved> {
    let by_ordinal = |n: usize| -> Result<Resolved> {
        let h = outline.headings.get(n).ok_or_else(|| {
            if outline.complete {
                Error::new("not_found", "No heading has that ordinal.")
            } else {
                Error::new(
                    "capacity",
                    "The document has more headings than are addressable.",
                )
            }
        })?;
        Ok(Resolved {
            range: h.start..h.end,
            key: format!("ordinal:{n}"),
            ordinal: Some(n),
        })
    };
    match selector {
        Selector::Whole => Ok(Resolved {
            range: 0..total,
            key: "whole".into(),
            ordinal: None,
        }),
        Selector::Preamble => Ok(Resolved {
            range: 0..outline.preamble_end,
            key: "preamble".into(),
            ordinal: None,
        }),
        Selector::Ordinal(n) => by_ordinal(*n),
        Selector::Heading {
            text,
            level,
            occurrence,
        } => {
            let matches: Vec<&Heading> = outline
                .headings
                .iter()
                .filter(|h| &h.text == text && level.is_none_or(|l| l == h.level))
                .collect();
            if matches.is_empty() {
                return Err(Error::new("not_found", "No heading has that text."));
            }
            let chosen = match (occurrence, matches.as_slice()) {
                (Some(k), list) => list.iter().find(|h| h.occurrence == *k).copied(),
                (None, [one]) => Some(*one),
                (None, _) => {
                    return Err(Error::new(
                        "ambiguous_section",
                        format!(
                            "{} headings have that text; give occurrence.",
                            matches.len()
                        ),
                    ));
                }
            };
            let chosen =
                chosen.ok_or_else(|| Error::new("not_found", "No heading has that occurrence."))?;
            by_ordinal(chosen.ordinal)
        }
    }
}

/// Resolve `selector` to its absolute byte range only; see [`resolve_selection`].
pub fn resolve(outline: &Outline, total: usize, selector: &Selector) -> Result<Range<usize>> {
    resolve_selection(outline, total, selector).map(|r| r.range)
}

/// How a span crosses the text channel.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Wire {
    /// The text is the span verbatim; a backslash is an ordinary character.
    Raw,
    /// Backslash, CR and the listed characters are escaped (`\\`, `\r`, `\u{h}`).
    Escaped,
}

/// Characters that force the escaped wire form.
fn special(c: char) -> bool {
    matches!(c,
        '\r' | '\u{feff}' | '\u{7f}' | '\u{2028}' | '\u{2029}'
        | '\u{80}'..='\u{9f}' | '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}')
        || (c < ' ' && c != '\n' && c != '\t')
}

/// Encoded length of `c` in the escaped form.
fn escaped_len(c: char) -> usize {
    if c == '\\' || c == '\r' {
        2
    } else if special(c) {
        4 + format!("{:x}", c as u32).len()
    } else {
        c.len_utf8()
    }
}

/// Encode one complete span. Errors: `encoding` when the span is not valid UTF-8.
pub fn encode(span: &[u8]) -> Result<(Wire, String)> {
    let text = std::str::from_utf8(span)
        .map_err(|_| Error::new("encoding", "The span is not valid UTF-8."))?;
    if !text.chars().any(special) {
        return Ok((Wire::Raw, text.to_owned()));
    }
    let mut out = String::with_capacity(text.len() + 8);
    for c in text.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '\r' => out.push_str("\\r"),
            c if special(c) => out.push_str(&format!("\\u{{{:x}}}", c as u32)),
            c => out.push(c),
        }
    }
    Ok((Wire::Escaped, out))
}

/// Length in bytes of the encoded form of `span` without allocating it; invalid UTF-8 gives 0.
pub fn encoded_len(span: &[u8]) -> usize {
    let Ok(text) = std::str::from_utf8(span) else {
        return 0;
    };
    if text.chars().any(special) {
        text.chars().map(escaped_len).sum()
    } else {
        text.len()
    }
}

/// Inverse of [`encode`]. Errors: `encoding` for an unknown or unterminated escape or a value that
/// is not a Unicode scalar.
pub fn decode(wire: Wire, text: &str) -> Result<Vec<u8>> {
    if wire == Wire::Raw {
        return Ok(text.as_bytes().to_vec());
    }
    let bad = || Error::new("encoding", "The escaped text has an invalid escape.");
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.next().ok_or_else(bad)? {
            '\\' => out.push('\\'),
            'r' => out.push('\r'),
            'u' => {
                if chars.next() != Some('{') {
                    return Err(bad());
                }
                let mut hex = String::new();
                loop {
                    match chars.next().ok_or_else(bad)? {
                        '}' => break,
                        h if h.is_ascii_hexdigit() && hex.len() < 6 => hex.push(h),
                        _ => return Err(bad()),
                    }
                }
                let v = u32::from_str_radix(&hex, 16).map_err(|_| bad())?;
                out.push(char::from_u32(v).ok_or_else(bad)?);
            }
            _ => return Err(bad()),
        }
    }
    Ok(out.into_bytes())
}

/// One page of an encoded span.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Page {
    /// Wire form chosen from this page's own raw bytes.
    pub wire: Wire,
    /// Encoded text, at most the requested budget in bytes.
    pub text: String,
    /// Absolute raw offset of the first byte of the page.
    pub start: usize,
    /// Absolute raw offset after the last byte of the page; greater than `start` while bytes remain.
    pub end: usize,
    /// Exclusive absolute end of the whole span.
    pub span_end: usize,
    /// Whether `end < span_end`.
    pub more: bool,
}

/// Largest prefix of `bytes[at..span.end]` that ends on a character boundary and whose encoded
/// length is at most `budget`. Errors: `presentation_capacity` (budget below [`MIN_PAGE_BUDGET`]),
/// `invalid_arguments` (span outside the bytes, `at` outside the span or inside a character),
/// `encoding` (the span is not valid UTF-8).
pub fn page(bytes: &[u8], span: Range<usize>, at: usize, budget: usize) -> Result<Page> {
    if budget < MIN_PAGE_BUDGET {
        return Err(Error::new(
            "presentation_capacity",
            "The page budget is too small for one character; nothing was read.",
        ));
    }
    if span.start > span.end || span.end > bytes.len() || at < span.start || at > span.end {
        return Err(Error::new(
            "invalid_arguments",
            "The offset lies outside the selection.",
        ));
    }
    let text = std::str::from_utf8(&bytes[span.clone()])
        .map_err(|_| Error::new("encoding", "The selection is not valid UTF-8."))?;
    let rel = at - span.start;
    if !text.is_char_boundary(rel) {
        return Err(Error::new(
            "invalid_arguments",
            "The offset lies inside a character.",
        ));
    }
    let (mut raw, mut esc, mut has_special, mut taken) = (0usize, 0usize, false, 0usize);
    for c in text[rel..].chars() {
        let n_special = has_special || special(c);
        let n_raw = raw + c.len_utf8();
        let n_esc = esc + escaped_len(c);
        if (if n_special { n_esc } else { n_raw }) > budget {
            break;
        }
        (raw, esc, has_special, taken) = (n_raw, n_esc, n_special, taken + c.len_utf8());
    }
    let end = at + taken;
    let (wire, text) = encode(&bytes[at..end])?;
    Ok(Page {
        wire,
        text,
        start: at,
        end,
        span_end: span.end,
        more: end < span.end,
    })
}

/// Innermost heading ordinal containing the raw byte `offset`; `None` inside the preamble.
pub fn heading_at(headings: &[Heading], offset: usize) -> Option<usize> {
    headings
        .iter()
        .filter(|h| h.start <= offset && offset < h.end.max(h.start + 1))
        .map(|h| h.ordinal)
        .next_back()
}

/// Replace the body of one section (or the preamble) with `new_body`, preserving every other byte.
/// `target` must be [`Selector::Preamble`] or [`Selector::Ordinal`] and `outline` must be the
/// outline of `bytes`. The heading line is kept. The new body may not contain a heading of a level
/// at or above the section's (the preamble may contain none), may not leave a fence or comment
/// open, and the heading list outside the edited region must be unchanged. Errors:
/// `invalid_arguments` (unsupported selector, NUL), `unterminated_heading` (the heading line has no
/// terminator and the body does not start with a line break), `structure_change`, `capacity`
/// (incomplete outline), `encoding`, `not_found`.
pub fn replace_body(
    bytes: &[u8],
    outline: &Outline,
    target: &Selector,
    new_body: &[u8],
) -> Result<Vec<u8>> {
    if !outline.complete {
        return Err(Error::new(
            "capacity",
            "The document has more headings than are addressable.",
        ));
    }
    std::str::from_utf8(new_body)
        .map_err(|_| Error::new("encoding", "The replacement is not valid UTF-8."))?;
    if new_body.contains(&0) {
        return Err(Error::new(
            "invalid_arguments",
            "body: must not contain NUL.",
        ));
    }
    let (region, level, kept) = match target {
        Selector::Preamble => (0..outline.preamble_end, 0u8, 0usize),
        Selector::Ordinal(n) => {
            let h = outline
                .headings
                .get(*n)
                .ok_or_else(|| Error::new("not_found", "No heading has that ordinal."))?;
            if h.body_start == bytes.len()
                && !bytes.ends_with(b"\n")
                && !new_body.is_empty()
                && !new_body.starts_with(b"\n")
                && !new_body.starts_with(b"\r\n")
            {
                return Err(Error::new(
                    "unterminated_heading",
                    "The heading line has no terminator; the body must begin with a line break.",
                ));
            }
            (h.body_start..h.end, h.level, n + 1)
        }
        _ => {
            return Err(Error::new(
                "invalid_arguments",
                "Only a heading or the preamble can be replaced.",
            ));
        }
    };
    let mut result = Vec::with_capacity(bytes.len() - region.len() + new_body.len());
    result.extend_from_slice(&bytes[..region.start]);
    result.extend_from_slice(new_body);
    result.extend_from_slice(&bytes[region.end..]);
    let key = |h: &Heading| (h.level, h.text.clone());
    let before: Vec<_> = outline.headings[..kept].iter().map(key).collect();
    let after: Vec<_> = outline
        .headings
        .iter()
        .filter(|h| h.start >= region.end)
        .map(key)
        .collect();
    let fresh = self::outline(&result)?;
    let all: Vec<_> = fresh.headings.iter().map(key).collect();
    let structure = || {
        Error::new(
            "structure_change",
            "The replacement would change the document structure outside the section body.",
        )
    };
    if all.len() < before.len() + after.len() || !fresh.complete {
        return Err(structure());
    }
    let inner = &all[before.len()..all.len() - after.len()];
    if all[..before.len()] != before[..]
        || all[all.len() - after.len()..] != after[..]
        || inner.iter().any(|(l, _)| *l <= level)
        || (level == 0 && !inner.is_empty())
        || !ends_clean(new_body)
    {
        return Err(structure());
    }
    Ok(result)
}

/// Whether scanning `body` from a clean state leaves no fence or comment open at its end.
fn ends_clean(body: &[u8]) -> bool {
    let mut fence: Option<(u8, usize)> = None;
    let mut comment = false;
    let mut start = 0;
    while start < body.len() {
        let (end, next) = match body[start..].iter().position(|b| *b == b'\n') {
            Some(p) => {
                let lf = start + p;
                (
                    if lf > start && body[lf - 1] == b'\r' {
                        lf - 1
                    } else {
                        lf
                    },
                    lf + 1,
                )
            }
            None => (body.len(), body.len()),
        };
        let line = &body[start..end];
        if let Some((ch, len)) = fence {
            if fence_close(line, ch, len) {
                fence = None;
            }
        } else if comment {
            if find(line, b"-->").is_some() {
                comment = false;
            }
        } else if let Some(open) = fence_open(line) {
            fence = Some(open);
        } else if let Some(at) = indent3(line).filter(|n| line[*n..].starts_with(b"<!--")) {
            comment = find(&line[at + 4..], b"-->").is_none();
        }
        start = next;
    }
    fence.is_none() && !comment
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, reason = "Test assertions")]
    use super::*;

    /// Reconstruct a span by walking pages with `budget` and decoding each.
    fn walk(bytes: &[u8], budget: usize) -> Vec<u8> {
        let mut out = Vec::new();
        let mut at = 0;
        loop {
            let p = page(bytes, 0..bytes.len(), at, budget).unwrap();
            assert!(p.text.len() <= budget);
            out.extend(decode(p.wire, &p.text).unwrap());
            if !p.more {
                break;
            }
            assert!(p.end > at, "pages must progress");
            at = p.end;
        }
        out
    }

    /// Pages concatenate to the original bytes for hostile content at every budget.
    #[test]
    fn pages_reassemble_hostile_bytes_for_every_budget() {
        let doc = "\u{feff}# T\r\nline\rlone \\ back\u{0}nul \u{1b}[0m \u{202e}bidi\n```\n# not a heading\n```\n\u{1f600} é\u{2028}x\n tail";
        for budget in MIN_PAGE_BUDGET..40 {
            assert_eq!(
                walk(doc.as_bytes(), budget),
                doc.as_bytes(),
                "budget {budget}"
            );
        }
    }

    /// Wire form choice and decode failures follow the dialect rules.
    #[test]
    fn raw_and_escaped_choice_and_decode_errors() {
        assert_eq!(encode(b"a\\b\n\t").unwrap(), (Wire::Raw, "a\\b\n\t".into()));
        let (w, t) = encode("a\\b\r\u{7f}".as_bytes()).unwrap();
        assert_eq!((w, t.as_str()), (Wire::Escaped, "a\\\\b\\r\\u{7f}"));
        assert!(decode(Wire::Escaped, "\\q").is_err());
        assert!(decode(Wire::Escaped, "\\u{110000}").is_err());
        assert!(decode(Wire::Escaped, "tail\\").is_err());
        assert_eq!(encoded_len("a\\b\r".as_bytes()), "a\\\\b\\r".len());
        assert_eq!(
            page(b"x", 0..1, 0, 15).unwrap_err().code,
            "presentation_capacity"
        );
    }

    /// Headings ignore fences, comments, front matter, indented and quoted lines; setext is counted.
    #[test]
    fn dialect_headings_fences_front_matter_comments_and_setext() {
        let doc = "---\n# yaml comment\n---\n# One\n```\n# fenced\n```\n<!--\n# commented\n-->\n## Two ##\n    # indented\n> # quoted\nSetext\n===\n#NoSpace\n## Two\n";
        let o = outline(doc.as_bytes()).unwrap();
        let texts: Vec<_> = o
            .headings
            .iter()
            .map(|h| (h.level, h.text.as_str()))
            .collect();
        assert_eq!(texts, vec![(1, "One"), (2, "Two"), (2, "Two")]);
        assert_eq!((o.headings[1].occurrence, o.headings[2].occurrence), (1, 2));
        assert_eq!(o.headings[2].slug, "two-1");
        assert_eq!(o.setext_candidates, 1);
        assert_eq!((o.front_matter_end, o.preamble_end), (Some(23), 23));
        assert_eq!(o.headings[0].end, doc.len());
    }

    /// Byte facts count BOM, CRLF, bare LF, lone CR and NUL separately.
    #[test]
    fn counts_bom_crlf_lone_cr_and_nul() {
        let o = outline(b"\xEF\xBB\xBF# A\r\nb\rc\nd\0").unwrap();
        assert!(o.bom);
        assert_eq!((o.crlf, o.lf, o.lone_cr, o.nul), (1, 1, 1, 1));
        assert_eq!(o.headings[0].text, "A");
        assert_eq!(outline(&[0xff]).unwrap_err().code, "encoding");
    }

    /// Duplicate headings need an occurrence and keys are stable.
    #[test]
    fn selectors_resolve_and_refuse_ambiguity() {
        let doc = "# A\none\n# B\n## B\n# A\ntwo\n";
        let o = outline(doc.as_bytes()).unwrap();
        let heading = |t: &str, l, k| Selector::Heading {
            text: t.into(),
            level: l,
            occurrence: k,
        };
        assert_eq!(
            resolve(&o, doc.len(), &heading("A", None, Some(2))).unwrap(),
            17..doc.len()
        );
        assert_eq!(
            resolve(&o, doc.len(), &heading("A", None, None))
                .unwrap_err()
                .code,
            "ambiguous_section"
        );
        assert_eq!(
            resolve(&o, doc.len(), &heading("B", Some(2), None)).unwrap(),
            12..17
        );
        assert_eq!(
            resolve(&o, doc.len(), &Selector::Ordinal(9))
                .unwrap_err()
                .code,
            "not_found"
        );
        assert_eq!(
            resolve_selection(&o, doc.len(), &Selector::Ordinal(1))
                .unwrap()
                .key,
            "ordinal:1"
        );
        assert!(Selector::from_parts(true, Some(1), None, None, None).is_err());
        assert!(Selector::from_parts(false, None, None, Some(1), None).is_err());
        assert_eq!(heading_at(&o.headings, 14), Some(2));
    }

    /// Section replacement keeps outside bytes and refuses structure changes.
    #[test]
    fn section_replace_keeps_outside_bytes_and_guards_structure() {
        let doc = "\u{feff}pre\r\n# A\r\nold\r\n## C\r\nkeep\r\n# B\r\nend\r\n";
        let o = outline(doc.as_bytes()).unwrap();
        let out = replace_body(
            doc.as_bytes(),
            &o,
            &Selector::Ordinal(0),
            b"new\r\n### deeper\r\nx\r\n",
        )
        .unwrap();
        assert_eq!(
            String::from_utf8(out).unwrap(),
            "\u{feff}pre\r\n# A\r\nnew\r\n### deeper\r\nx\r\n# B\r\nend\r\n"
        );
        for bad in [&b"# up\n"[..], b"```\nopen\n", b"<!-- open\n"] {
            let e = replace_body(doc.as_bytes(), &o, &Selector::Ordinal(0), bad).unwrap_err();
            assert_eq!(e.code, "structure_change");
        }
        let e = replace_body(doc.as_bytes(), &o, &Selector::Preamble, b"# h\n").unwrap_err();
        assert_eq!(e.code, "structure_change");
        let out = replace_body(doc.as_bytes(), &o, &Selector::Preamble, b"other\r\n").unwrap();
        assert!(out.starts_with(b"other\r\n# A"));
        let tail = "# only";
        let o2 = outline(tail.as_bytes()).unwrap();
        let e = replace_body(tail.as_bytes(), &o2, &Selector::Ordinal(0), b"x").unwrap_err();
        assert_eq!(e.code, "unterminated_heading");
        assert_eq!(
            replace_body(tail.as_bytes(), &o2, &Selector::Ordinal(0), b"\nx").unwrap(),
            b"# only\nx"
        );
    }

    /// Link scanning sees only prose lines.
    #[test]
    fn prose_lines_skip_fences_comments_and_front_matter() {
        let doc = "---\nm: [x](y)\n---\ntext\n```\ncode\n```\n<!-- c -->\nmore";
        let lines: Vec<_> = prose_lines(doc.as_bytes())
            .unwrap()
            .into_iter()
            .map(|r| &doc[r])
            .collect();
        assert_eq!(lines, vec!["text", "more"]);
    }
}
