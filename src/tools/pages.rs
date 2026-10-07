//! Exact document page presentation. A page carries the encoded payload of one raw byte window
//! inside a length-framed reply so a reader can rebuild the original bytes from the text alone.
//! The whole reply, header and footer included, never exceeds the 8192 byte response limit.
#![allow(
    dead_code,
    reason = "Document reads call this once the document module is integrated; unit tests exercise it now"
)]
use crate::{
    response::Templates,
    store::{Error, Result},
};
use serde::Serialize;

/// Hard limit of one reply in bytes, shared with the template writer.
pub(super) const REPLY_LIMIT: usize = 8192;

/// Layout of one document page: header, one framing line, the payload, then the continuation.
///
/// The payload is a plain string variable inserted verbatim (template autoescape is off) and the
/// framing line states the wire, the raw offsets and `encoded_len`, so the reader delimits the
/// payload by length and never by a marker that content could contain.
pub(super) fn templates() -> Vec<(&'static str, &'static str)> {
    vec![(
        "document_page",
        "{{ heading }}\nData coverage: {{ coverage }}; State: {{ state }}; Version: {{ version }}\nSnapshot version: {{ snapshot }}\nContent: md-text-v1 wire={{ wire }} bytes={{ start }}-{{ end }} of {{ range_end }} encoded_len={{ encoded_len }}\n{{ text }}\n{% if next != none %}Next: start={{ next }}; version={{ snapshot }}; remaining={{ remaining }}. Keep the same tool and selection.\n{% else %}End of selection; no continuation.\n{% endif %}",
    )]
}

/// Typed view of one exact document page; raw storage structures never enter the template.
#[derive(Clone, Serialize)]
pub(super) struct DocumentPage {
    /// Safe human heading naming the document and the selected part.
    pub heading: String,
    /// `complete` or `PARTIAL` data coverage of the observation behind the page.
    pub coverage: String,
    /// Honest document state label (managed, unmanaged, drifted, absent and so on).
    pub state: String,
    /// Observation version a later write needs.
    pub version: String,
    /// Read snapshot that pins continuation; it never contains the byte offset.
    pub snapshot: String,
    /// Wire form of `text`: `raw` or `escaped`.
    pub wire: &'static str,
    /// Absolute raw byte offset of the first byte of this page.
    pub start: usize,
    /// Absolute raw byte offset after the last byte of this page.
    pub end: usize,
    /// Exclusive end of the selected range in raw bytes.
    pub range_end: usize,
    /// Length in bytes of `text`, the frame the reader trusts.
    pub encoded_len: usize,
    /// Encoded payload, inserted verbatim.
    pub text: String,
    /// Raw offset to continue from, absent on the final page.
    pub next: Option<usize>,
    /// Raw bytes of the selection that remain after this page.
    pub remaining: usize,
}

impl DocumentPage {
    /// Copy of this page with empty payload and maximum-width numbers, used to measure the fixed
    /// overhead of header, framing line and footer.
    fn measuring(&self) -> Self {
        let widest = usize::MAX;
        Self {
            start: widest,
            end: widest,
            range_end: widest,
            encoded_len: widest,
            next: Some(widest),
            remaining: widest,
            text: String::new(),
            ..self.clone()
        }
    }
}

/// Bytes available for the encoded payload of `view` so that the whole reply fits the limit.
///
/// Renders the page once with an empty payload and the widest possible numbers; the payload
/// budget is the limit minus that overhead, so no later page can overflow.
///
/// # Errors
/// `presentation_capacity` when the template cannot render or the overhead alone exceeds the limit.
pub(super) fn payload_budget(templates: &Templates, view: &DocumentPage) -> Result<usize> {
    let overhead = templates
        .render("document_page", &view.measuring())
        .map_err(|_| capacity())?
        .len();
    REPLY_LIMIT.checked_sub(overhead).ok_or_else(capacity)
}

/// Render one page and verify the reply: within the limit and carrying the payload verbatim.
///
/// # Errors
/// `presentation_capacity` when the reply would exceed the limit or the payload did not survive
/// rendering byte for byte; nothing is truncated and no partial text is returned.
pub(super) fn render(templates: &Templates, view: &DocumentPage) -> Result<String> {
    if view.text.len() != view.encoded_len {
        return Err(capacity());
    }
    let text = templates
        .render("document_page", view)
        .map_err(|_| capacity())?;
    if text.len() > REPLY_LIMIT || !text.contains(view.text.as_str()) {
        return Err(capacity());
    }
    Ok(text)
}

/// The one error a page presentation failure produces.
fn capacity() -> Error {
    Error::new(
        "presentation_capacity",
        "The document page cannot be presented within the reply budget; no text was truncated.",
    )
}

#[cfg(test)]
#[allow(clippy::unwrap_used, reason = "Test assertions")]
mod tests {
    use super::*;

    /// Trusted templates with only the page layout registered.
    fn registered() -> Templates {
        Templates::new(&templates()).unwrap()
    }

    /// A page around `text` with plausible header facts.
    fn page(text: &str, start: usize, end: usize, range_end: usize) -> DocumentPage {
        DocumentPage {
            heading: "docs/a.md".into(),
            coverage: "complete".into(),
            state: "managed".into(),
            version: "v".repeat(64),
            snapshot: "s".repeat(64),
            wire: "raw",
            start,
            end,
            range_end,
            encoded_len: text.len(),
            text: text.into(),
            next: (end < range_end).then_some(end),
            remaining: range_end - end,
        }
    }

    /// Recover the payload from a reply using only the framing line, as a reader would.
    fn payload(reply: &str) -> &str {
        let frame = reply
            .lines()
            .find(|line| line.starts_with("Content: "))
            .unwrap();
        let length: usize = frame
            .rsplit("encoded_len=")
            .next()
            .unwrap()
            .parse()
            .unwrap();
        let begin = reply.find(frame).unwrap() + frame.len() + 1;
        &reply[begin..begin + length]
    }

    /// Markup, quotes, ampersands, template syntax, backslashes and escapes survive byte for byte.
    #[test]
    fn payload_is_inserted_verbatim() {
        let tricky = "<b>&amp;</b> \"q\" 'a' {{ x }} {% y %} \\n \\u{1b} tab\there\n\nend {#c#}";
        let reply = render(&registered(), &page(tricky, 0, 1000, 1000)).unwrap();
        assert_eq!(payload(&reply), tricky);
        assert!(reply.ends_with("End of selection; no continuation.\n"));
    }

    /// The measured budget keeps the whole reply inside the limit for the widest numbers.
    #[test]
    fn budget_keeps_the_reply_within_the_limit() {
        let t = registered();
        let probe = page("", 0, 0, usize::MAX / 2);
        let budget = payload_budget(&t, &probe).unwrap();
        assert!(budget > 6000 && budget < REPLY_LIMIT);
        let text = "x".repeat(budget);
        let mut full = page(&text, 123_456_789, 123_456_789 + budget, usize::MAX - 1);
        full.next = Some(usize::MAX);
        full.remaining = usize::MAX;
        let reply = render(&t, &full).unwrap();
        assert!(reply.len() <= REPLY_LIMIT, "{}", reply.len());
        let over = "x".repeat(REPLY_LIMIT);
        assert!(render(&t, &page(&over, 0, over.len(), usize::MAX)).is_err());
    }

    /// An empty selection carries no payload and truthfully reports no continuation.
    #[test]
    fn empty_selection_has_no_continuation() {
        let reply = render(&registered(), &page("", 7, 7, 7)).unwrap();
        assert!(reply.contains("bytes=7-7 of 7 encoded_len=0"));
        assert!(!reply.contains("Next:"));
        assert!(reply.contains("End of selection; no continuation."));
    }

    /// A page with more to read names the offset and the pinned snapshot, never the offset in the
    /// snapshot itself.
    #[test]
    fn continuation_names_offset_and_snapshot() {
        let reply = render(&registered(), &page("abc", 0, 3, 10)).unwrap();
        assert!(reply.contains(&format!(
            "Next: start=3; version={}; remaining=7.",
            "s".repeat(64)
        )));
    }

    /// A payload whose declared length differs from its bytes is refused, not rendered.
    #[test]
    fn mismatched_frame_is_refused() {
        let mut view = page("abc", 0, 3, 3);
        view.encoded_len = 4;
        assert_eq!(
            render(&registered(), &view).unwrap_err().code,
            "presentation_capacity"
        );
    }
}
