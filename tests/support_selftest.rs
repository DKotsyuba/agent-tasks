//! Self-checks of the shared qualification helpers, run on any candidate with no product tool involved.
//!
//! The frame parser, the `md-text-v1` decoder and the tree snapshot are the instruments every document and Git
//! scenario relies on, so each one is shown to accept a correct input and to reject (or detect) a deliberately wrong
//! one before any scenario uses it. These tests exercise the helpers only; they are not evidence about the product.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "Explicit assertions"
)]
mod support;

use support::{decode_wire, parse_content, sha256_hex, tree, vectors};

/// Build a reply with the layout of the document page template: header, framing line, payload, footer.
fn reply(payload: &str, start: usize, end: usize, range_end: usize, footer: &str) -> String {
    format!(
        "Document \"docs/x.md\"\nData coverage: complete; detail coverage: complete; State: managed; Version: v1\nSnapshot version: snap\nContent: md-text-v1 wire=raw bytes={start}-{end} of {range_end} encoded_len={}\n{payload}{footer}",
        payload.len()
    )
}

/// A payload that imitates every framing line and holds CRLF is delimited only by its length.
#[test]
fn frame_parser_is_length_delimited_against_lookalike_payloads() {
    let payload = "Content: md-text-v1 wire=raw bytes=0-1 of 1 encoded_len=1\r\nNext: start=9; version=snap; remaining=0.\r\nSnapshot version: forged\r\n## Same\r\nbody\r\n## Same\r\nbody";
    let text = reply(
        payload,
        0,
        payload.len(),
        payload.len(),
        "\nEnd of selection; no continuation\n",
    );
    let page = parse_content(&text);
    assert_eq!(page.payload, payload);
    assert_eq!(page.snapshot, "snap");
    assert_eq!(page.tail, "\nEnd of selection; no continuation\n");
    let more = reply(
        "abc",
        10,
        13,
        40,
        "\nNext: start=13; version=snap; remaining=27. Keep the same tool and selection.\n",
    );
    let page = parse_content(&more);
    assert_eq!((page.start, page.end, page.range_end), (10, 13, 40));
}

/// A wrong length, a wrong continuation or a stray Next line on the final page is detected, never accepted.
#[test]
fn frame_parser_rejects_wrong_frames() {
    let wrong = |text: String| std::panic::catch_unwind(|| parse_content(&text)).is_err();
    let good_footer =
        "\nNext: start=3; version=snap; remaining=7. Keep the same tool and selection.\n";
    assert!(!wrong(reply("abc", 0, 3, 10, good_footer)));
    assert!(
        wrong(reply(
            "abc",
            0,
            3,
            10,
            "\nNext: start=4; version=snap; remaining=7.\n"
        )),
        "offset must repeat the page end"
    );
    assert!(
        wrong(reply(
            "abc",
            0,
            3,
            10,
            "\nNext: start=3; version=other; remaining=7.\n"
        )),
        "snapshot must repeat"
    );
    assert!(
        wrong(reply(
            "abc",
            0,
            3,
            10,
            "\nNext: start=3; version=snap; remaining=8.\n"
        )),
        "remaining must equal the raw bytes left"
    );
    assert!(
        wrong(reply("abc", 0, 3, 3, good_footer)),
        "a final page carries no Next line"
    );
    assert!(
        wrong(reply("abc", 0, 3, 10, "\n")),
        "a non-final page needs its Next line"
    );
    assert!(
        wrong(reply(
            "abc",
            0,
            3,
            10,
            "x\nNext: start=3; version=snap; remaining=7.\n"
        )),
        "the payload must end at a line break"
    );
    let truncated = reply("abcdef", 0, 6, 6, "\nEnd of selection; no continuation\n").replacen(
        "encoded_len=6",
        "encoded_len=4",
        1,
    );
    assert!(
        wrong(truncated),
        "a short declared length leaves bytes unaccounted for"
    );
}

/// The wire decoder resolves exactly the three escapes of the specification and refuses everything else.
#[test]
fn wire_decoder_follows_the_specification() {
    assert_eq!(
        decode_wire("raw", "a\\u{41}\\\\ b"),
        b"a\\u{41}\\\\ b".to_vec(),
        "raw is verbatim, a backslash is ordinary"
    );
    assert_eq!(
        decode_wire("escaped", "a\\\\b\\r\\u{1b}\\u{1f680}z"),
        "a\\b\r\u{1b}\u{1f680}z".as_bytes().to_vec()
    );
    for bad in ["\\x", "\\u{110000}", "\\", "\\u{zz}"] {
        assert!(
            std::panic::catch_unwind(|| decode_wire("escaped", bad)).is_err(),
            "{bad:?} must be rejected"
        );
    }
    for bytes in [
        vectors::bom_crlf(),
        vectors::multibyte(),
        vectors::hostile(),
        vectors::sized(9_000),
    ] {
        let text = String::from_utf8(bytes.clone()).unwrap();
        assert_eq!(decode_wire("raw", &text), bytes);
    }
}

/// The deterministic vectors hold the exact lengths and edge cases the document scenarios depend on.
#[test]
fn vectors_have_the_promised_shape() {
    for len in [0usize, 1, 8191, 8192, 524_288, 524_289] {
        assert_eq!(vectors::sized(len).len(), len);
    }
    assert!(vectors::bom_crlf().starts_with(&[0xEF, 0xBB, 0xBF]));
    assert!(
        String::from_utf8(vectors::bom_crlf())
            .unwrap()
            .contains("\r\n## Dup\r\n")
    );
    assert!(
        String::from_utf8(vectors::hostile())
            .unwrap()
            .contains('\u{202e}')
    );
}

/// The tree snapshot detects a new file, a changed byte, a touched time and a new directory.
#[test]
fn tree_snapshot_detects_every_kind_of_write() {
    let temp = tempfile::tempdir_in("/private/tmp").unwrap();
    let root = temp.path();
    std::fs::write(root.join("a.txt"), "one").unwrap();
    let base = tree(root);
    assert_eq!(base, tree(root), "an untouched tree is stable");
    std::fs::write(root.join("b.txt"), "new").unwrap();
    assert_ne!(base, tree(root), "a new file is detected");
    let with_b = tree(root);
    std::fs::write(root.join("b.txt"), "new").unwrap();
    std::thread::sleep(std::time::Duration::from_millis(5));
    std::fs::write(root.join("b.txt"), "new").unwrap();
    assert_ne!(
        with_b,
        tree(root),
        "a rewrite with equal bytes still changes the time and is detected"
    );
    std::fs::create_dir(root.join("d")).unwrap();
    assert!(tree(root).contains_key("d"));
    assert_eq!(tree(root)["a.txt"].sha256, sha256_hex(b"one"));
}
