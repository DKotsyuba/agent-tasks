//! Measured lexical search quality over work, typed knowledge and Markdown (M-006 criterion 6).
//!
//! The corpus is built deterministically through the real tools in a disposable project. The declared query table
//! below is run through the real `search` and each row records hits, rank of the first expected reference, reply
//! bytes and whether the expected term is actually present. Rows of class `vocab-gap` (paraphrase, misspelling) and
//! `negative` are expected misses: they prove the miss is detected and are the only rows that could ever motivate a
//! semantic proposal; no semantic service is added. A miss on any other class is a defect and fails. The measured
//! table is written to `CARGO_TARGET_TMPDIR/search-quality.md`. The work-only subset runs on any candidate; the rest
//! is `#[ignore]`d until the combined candidate carries the typed and document tools.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "Explicit protocol assertions"
)]
mod support;

use serde_json::json;
use std::fmt::Write as _;
use support::{Project, document, knowledge, save, target};

/// Distinctive invented words, one per record family index, so every expectation is unambiguous.
const WORDS: [&str; 8] = [
    "zephyr", "kestrel", "bramble", "cinder", "vesper", "onyx", "juniper", "tundra",
];

/// Second distinctive word per index, used in body fields to test terms spread over fields.
const BODY: [&str; 8] = [
    "quokka", "marlin", "saffron", "obsidian", "lantern", "cobalt", "meadow", "glacier",
];

/// One declared query: class, text, optional `kinds`, and the key of the expected record (empty for none).
struct Query {
    /// The measured class (`exact-title`, `body-field`, `spread`, `heading`, `typed-field`, `case`, `vocab-gap`, ...).
    class: &'static str,
    /// The query text sent to `search`.
    text: String,
    /// Optional source filter sent as `kinds`.
    kinds: Option<Vec<&'static str>>,
    /// Corpus key of the expected hit, resolved to a reference after creation; empty means no hit is expected.
    expect: String,
}

/// True for a canonical work or knowledge reference (`M-001`, `D-012`, `CL-001/I-002`, `DOC-003`) or a managed path.
fn is_reference(token: &str) -> bool {
    if token == "README.md" || (token.starts_with("docs/") && token.ends_with(".md")) {
        return true;
    }
    let Some((prefix, number)) = token.split_once('-') else {
        return false;
    };
    let digits: String = number.chars().take_while(char::is_ascii_digit).collect();
    ["E", "M", "A", "D", "RB", "RS", "CL", "DOC", "CP"].contains(&prefix)
        && digits.len() >= 3
        && (number.len() == digits.len() || number[digits.len()..].starts_with('/'))
}

/// The references a search reply offers as hits, in reply order, deduplicated.
///
/// A hit is the first token of a result line or the token after `ref=` on it, and must be a real reference: the
/// generic `<reference>` placeholder of the continuation hint is never a hit.
fn refs(reply: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for line in reply.lines() {
        let mut tokens: Vec<String> = line
            .split("ref=")
            .skip(1)
            .map(|part| {
                part.chars()
                    .take_while(|c| !c.is_whitespace() && *c != '"' && *c != ',' && *c != ')')
                    .collect()
            })
            .collect();
        if let Some(first) = line.split_whitespace().next() {
            tokens.insert(0, first.to_owned());
        }
        for token in tokens {
            let token = token.trim_end_matches(['.', ';', ':']).to_owned();
            if is_reference(&token) && !out.contains(&token) {
                out.push(token);
            }
        }
    }
    out
}

/// Create the three work Modules; returns `(key, reference)` pairs.
async fn work_corpus(project: &Project) -> Vec<(String, String)> {
    let mut keys = Vec::new();
    for (index, (title, outcome)) in [
        (
            "Orbit scheduler",
            "Satellite passes are planned without conflicts",
        ),
        ("Harbor ledger", "Dock fees reconcile every night"),
        ("Quartz importer", "Mineral samples load from field tablets"),
    ]
    .into_iter()
    .enumerate()
    {
        let context = project.call("get_context", json!({}), false).await;
        let created = project
            .call(
                "plan_work",
                json!({"version":support::field(&context,"Allocation version: "),"op":"create_module","title":title,"outcome":outcome,"criteria":[outcome],"contracts":{"not_required":true},"lead":{"name":"corpus"}}),
                false,
            )
            .await;
        keys.push((format!("work:{index}"), target(&created)));
    }
    keys
}

/// The declared work queries, including the paraphrase and misspelling rows that must be detected as misses.
fn work_queries() -> Vec<Query> {
    let q = |class, text: &str, expect: &str| Query {
        class,
        text: text.to_owned(),
        kinds: None,
        expect: expect.to_owned(),
    };
    vec![
        q("exact-title", "orbit scheduler", "work:0"),
        q("exact-title", "Harbor Ledger", "work:1"),
        q("body-field", "mineral samples", "work:2"),
        q("body-field", "dock fees", "work:1"),
        q("spread", "satellite scheduler", "work:0"),
        q("case", "QUARTZ IMPORTER", "work:2"),
        q("vocab-gap", "boat accounting", ""),
        q("vocab-gap", "orbit schedulr", ""),
        q("negative", "nonexistentwordxyz", ""),
    ]
}

/// Run every query, write the measured table, and return the defect lines (expected hit missing).
async fn measure(
    project: &Project,
    keys: &[(String, String)],
    queries: &[Query],
    label: &str,
) -> Vec<String> {
    let mut table = String::new();
    writeln!(table, "# Lexical search quality: {label}\n\n| class | query | expected | rank | refs | reply bytes | verdict |\n|---|---|---|---|---|---|---|").unwrap();
    let mut defects = Vec::new();
    for query in queries {
        let mut args = json!({"query":query.text});
        if let Some(kinds) = &query.kinds {
            args["kinds"] = json!(kinds);
        }
        let reply = project.call("search", args, false).await;
        let found = refs(&reply);
        let expected = keys
            .iter()
            .find(|(key, _)| *key == query.expect)
            .map(|(_, reference)| reference.clone());
        let rank = expected.as_ref().and_then(|reference| {
            found
                .iter()
                .position(|f| f == reference || f.starts_with(reference.as_str()))
        });
        let expected_miss = query.class == "vocab-gap" || query.class == "negative";
        let verdict = match (&expected, rank, expected_miss) {
            (None, _, true) if found.is_empty() || query.class == "vocab-gap" => "miss detected",
            (None, _, _) => "unexpected hit",
            (Some(_), Some(_), false) => "hit",
            _ => "DEFECT",
        };
        if verdict == "DEFECT" || (verdict == "unexpected hit" && query.class == "negative") {
            defects.push(format!(
                "{}: {:?} expected {:?}, got {:?}",
                query.class, query.text, expected, found
            ));
        }
        writeln!(
            table,
            "| {} | {} | {} | {} | {} | {} | {} |",
            query.class,
            query.text,
            expected.unwrap_or_default(),
            rank.map_or("-".into(), |r| (r + 1).to_string()),
            found.len(),
            reply.len(),
            verdict
        )
        .unwrap();
    }
    let out = std::path::Path::new(env!("CARGO_TARGET_TMPDIR"))
        .join(format!("search-quality-{label}.md"));
    std::fs::write(out, table).unwrap();
    defects
}

/// The work subset: exact, field, spread and case queries hit; paraphrase, misspelling and nonsense are detected misses.
#[tokio::test]
async fn work_queries_hit_and_vocabulary_gaps_are_detected() {
    let project = Project::register().await;
    let keys = work_corpus(&project).await;
    let defects = measure(&project, &keys, &work_queries(), "work").await;
    assert!(defects.is_empty(), "defect misses: {defects:#?}");
    let too_many = project
        .call("search", json!({"query":"a b c d e f g h i"}), true)
        .await;
    assert!(
        too_many.contains("query") || too_many.contains("eight") || too_many.contains("words"),
        "nine words are refused: {too_many}"
    );
    let eight = project
        .call(
            "search",
            json!({"query":"orbit scheduler satellite passes are planned without conflicts"}),
            false,
        )
        .await;
    assert!(
        !refs(&eight).is_empty(),
        "eight words are accepted: {eight}"
    );
}

/// The full corpus across work, typed records and Markdown with at least sixty declared queries.
#[tokio::test]
#[ignore = "needs the combined E-001 candidate carrying knowledge_work and document_work and kinds filters in search"]
async fn full_corpus_measurement_over_work_knowledge_and_documents() {
    let project = Project::register().await;
    let mut keys = work_corpus(&project).await;
    let mut queries = work_queries();
    for (index, word) in WORDS.iter().enumerate() {
        let body = BODY[index];
        let decision = knowledge(&project, json!({"op":"create_decision","title":format!("Adopt {word} cache"),"question":format!("Which layer holds {word} data?"),"decision":format!("Use the {word} tier"),"rationale":format!("{body} latency stays low under burst")}), false).await;
        keys.push((format!("decision:{index}"), target(&decision)));
        let runbook = knowledge(&project, json!({"op":"create_runbook","title":format!("Restore {word} service"),"purpose":"Bring it back","steps":[{"title":format!("Drain {word} queue"),"description":format!("Empty the {body} backlog"),"expected":format!("Queue {word} empty")}]}), false).await;
        keys.push((format!("runbook:{index}"), target(&runbook)));
        let research = knowledge(&project, json!({"op":"create_research","title":format!("Measure {word} throughput"),"question":"How fast?","conclusions":[{"statement":format!("{word} sustains ten thousand requests"),"basis":"inferred"}],"applicability":format!("{body} hardware only")}), false).await;
        keys.push((format!("research:{index}"), target(&research)));
        let checklist = knowledge(&project, json!({"op":"create_checklist","title":format!("Release {word}"),"purpose":"Ship","items":[format!("Verify {word} manifest")]}), false).await;
        keys.push((format!("checklist:{index}"), target(&checklist)));
        let path = format!("docs/{word}-guide.md");
        save(&project, &path, &format!("# {word} guide\n\n## Overview\nThe {body} sector is described here.\n\n## Details\nMore about {word}.\n"), false).await;
        keys.push((format!("doc:{index}"), path));
        queries.extend([
            Query {
                class: "exact-title",
                text: format!("{word} cache"),
                kinds: Some(vec!["knowledge"]),
                expect: format!("decision:{index}"),
            },
            Query {
                class: "body-field",
                text: format!("{body} latency"),
                kinds: Some(vec!["knowledge"]),
                expect: format!("decision:{index}"),
            },
            Query {
                class: "spread",
                text: format!("{word} {body}"),
                kinds: Some(vec!["knowledge"]),
                expect: format!("decision:{index}"),
            },
            Query {
                class: "typed-field",
                text: format!("drain {word} queue"),
                kinds: Some(vec!["knowledge"]),
                expect: format!("runbook:{index}"),
            },
            Query {
                class: "typed-field",
                text: format!("{word} sustains"),
                kinds: Some(vec!["knowledge"]),
                expect: format!("research:{index}"),
            },
            Query {
                class: "typed-field",
                text: format!("verify {word} manifest"),
                kinds: Some(vec!["knowledge"]),
                expect: format!("checklist:{index}"),
            },
            Query {
                class: "heading",
                text: format!("{word} guide"),
                kinds: Some(vec!["document"]),
                expect: format!("doc:{index}"),
            },
            Query {
                class: "case",
                text: word.to_uppercase(),
                kinds: Some(vec!["knowledge", "document"]),
                expect: String::new(),
            },
        ]);
    }
    queries.retain(|q| !(q.class == "case" && q.expect.is_empty()));
    queries.extend([
        Query {
            class: "vocab-gap",
            text: "fast memory layer".into(),
            kinds: None,
            expect: String::new(),
        },
        Query {
            class: "vocab-gap",
            text: "zephyr cahce".into(),
            kinds: None,
            expect: String::new(),
        },
        Query {
            class: "negative",
            text: "nonexistentwordxyz".into(),
            kinds: None,
            expect: String::new(),
        },
    ]);
    assert!(
        queries.len() >= 60,
        "at least sixty declared queries, have {}",
        queries.len()
    );
    let defects = measure(&project, &keys, &queries, "full").await;
    assert!(defects.is_empty(), "defect misses: {defects:#?}");
    std::fs::write(project.root.join("docs/Ünï.md"), "# non-ascii name\n").unwrap();
    let partial = project
        .call(
            "search",
            json!({"query":"guide","kinds":["document"]}),
            false,
        )
        .await;
    assert!(
        partial.contains("PARTIAL"),
        "an unsupported native name keeps coverage honest: {partial}"
    );
    let state = document(
        &project,
        json!({"op":"adopt","ref":"docs/zephyr-guide.md"}),
        false,
    )
    .await;
    assert!(!state.is_empty());
}
