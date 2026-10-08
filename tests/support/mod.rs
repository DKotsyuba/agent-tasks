//! Shared real-binary SDK support for the E-001 knowledge, document, Git and compaction qualification families.
//!
//! Every scenario starts the shipped binary through the Rust MCP SDK stdio client with `env_clear`, a disposable
//! HOME, configuration and registry, and a documentation repository below `/private/tmp`. Nothing here selects a
//! policy, injects a fault into the product or reads a real project: faults are produced only by ordinary hooks,
//! repository configuration, natively written files and file permissions that the test itself controls.
#![allow(
    dead_code,
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "Shared helpers are used selectively by each integration test crate and fail loudly by design"
)]
use rmcp::{
    RoleClient, ServiceExt, model::CallToolRequestParams, service::RunningService,
    transport::TokioChildProcess,
};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    process::Stdio,
};

/// Maximum size in bytes of any single reply text, the whole reply including header, framing and footer.
pub const REPLY_BUDGET: usize = 8192;

/// The SDK client type returned by [`connect`].
pub type Client = RunningService<RoleClient, ()>;

/// Resolve the binary under test: an explicit shipped payload or the Cargo-built product.
pub fn binary() -> PathBuf {
    std::env::var_os("MCP_TEST_BINARY")
        .map(PathBuf::from)
        .unwrap_or_else(|| env!("CARGO_BIN_EXE_agent-tasks").into())
}

/// Start a fresh server with a disposable explicit config and a scrubbed environment.
///
/// `extra` entries are added after the scrub, so a test can for example expose a signing agent explicitly.
/// Inherited Git directory variables are pointed at unrelated absent paths to prove the product clears them.
pub async fn connect_with(config: &Path, home: &Path, extra: &[(&str, &str)]) -> Client {
    connect_with_pid(config, home, extra).await.0
}

/// Start a fresh server and return its exact operating-system child PID with the SDK client.
///
/// `config` names the fixture's explicit config; `home` supplies its isolated environment and working directory.
/// `extra` overrides are added after environment scrubbing. Returns the connected client and its owned child PID
/// captured from RMCP before serving; reconnects obtain a fresh identity without global process discovery.
/// Panics if spawning, PID acquisition or protocol initialization fails.
async fn connect_with_pid(config: &Path, home: &Path, extra: &[(&str, &str)]) -> (Client, u32) {
    let mut command = tokio::process::Command::new(binary());
    command
        .arg("--config")
        .arg(config)
        .arg("mcp")
        .env_clear()
        .env("HOME", home)
        .env("GIT_DIR", home.join("unrelated-git-directory"))
        .env("GIT_WORK_TREE", home.join("unrelated-git-worktree"))
        .env("GIT_INDEX_FILE", home.join("unrelated-git-index"))
        .current_dir(home)
        .kill_on_drop(true)
        .stderr(Stdio::null());
    for (key, value) in extra {
        command.env(key, value);
    }
    let process = TokioChildProcess::new(command).unwrap();
    let pid = process
        .id()
        .expect("the spawned MCP child has a process ID");
    (().serve(process).await.unwrap(), pid)
}

/// Start a fresh server with the standard scrubbed environment and no extra variables.
pub async fn connect(config: &Path, home: &Path) -> Client {
    connect_with(config, home, &[]).await
}

/// Call one tool and assert the text-only contract.
///
/// The reply must carry no structured content, exactly one text block of at most [`REPLY_BUDGET`] bytes, no raw
/// schema marker outside a length-delimited document payload, and the expected `isError` state. Returns the text.
pub async fn call(client: &Client, name: &str, args: Value, error: bool) -> String {
    let reply = client
        .call_tool(
            CallToolRequestParams::new(name.to_owned())
                .with_arguments(args.as_object().unwrap().clone()),
        )
        .await
        .unwrap();
    let wire = serde_json::to_value(reply).unwrap();
    assert_eq!(wire["isError"], error, "{name} {args}: {wire}");
    assert!(wire.get("structuredContent").is_none());
    assert_eq!(wire["content"].as_array().unwrap().len(), 1);
    let text = wire["content"][0]["text"].as_str().unwrap().to_owned();
    assert!(
        text.len() <= REPLY_BUDGET,
        "{name} reply is {} bytes, over the {REPLY_BUDGET} byte total budget",
        text.len()
    );
    assert!(
        schema_markers_are_confined_to_payload(&text),
        "a raw schema marker is only legitimate inside a length-delimited document payload: {text}"
    );
    text
}

/// Reject schema markers in reply text outside a parsed length-delimited document payload.
///
/// Replies without a marker need no frame. If a marker is present, [`parse_content`] must establish the payload
/// boundaries; malformed or unframed replies panic through that parser rather than gaining an exemption.
fn schema_markers_are_confined_to_payload(text: &str) -> bool {
    if !text.contains("schema_version:") {
        return true;
    }
    let page = parse_content(text);
    !page.header.contains("schema_version:") && !page.tail.contains("schema_version:")
}

/// Read one exact generated value from a reply line that starts with `label`.
pub fn field(text: &str, label: &str) -> String {
    text.lines()
        .find_map(|line| line.strip_prefix(label))
        .unwrap_or_else(|| panic!("label {label:?} absent in: {text}"))
        .to_owned()
}

/// Read the owning write Version of a record from its context summary, never deriving it in a test.
pub async fn version(client: &Client, project: &str, reference: &str) -> String {
    let text = call(
        client,
        "get_context",
        json!({"project":project,"ref":reference}),
        false,
    )
    .await;
    observation_version(&text)
}

/// Extract the write precondition from a context reply.
///
/// Work and typed records print `Version: <v>` on its own line; documents print it inside the data coverage line
/// after `Version: ` following a semicolon. The snapshot line is never confused with it.
pub fn observation_version(text: &str) -> String {
    if let Some(line) = text.lines().find_map(|line| line.strip_prefix("Version: ")) {
        return line.trim().to_owned();
    }
    for line in text.lines() {
        if let Some(position) = line.find("; Version: ") {
            return line[position + "; Version: ".len()..]
                .split(';')
                .next()
                .unwrap()
                .trim()
                .to_owned();
        }
    }
    panic!("no observation version in: {text}")
}

/// Read the pagination snapshot printed by a context reply.
pub fn snapshot_version(text: &str) -> String {
    field(text, "Snapshot version: ").trim().to_owned()
}

/// A disposable registered documentation project with a live client.
pub struct Project {
    /// Owns every file of the fixture; dropped last.
    pub temp: tempfile::TempDir,
    /// The documentation repository root registered under `alias`.
    pub root: PathBuf,
    /// The configuration file passed to the server.
    pub config: PathBuf,
    /// The registered alias.
    pub alias: &'static str,
    /// The live SDK client; `restart` replaces it with a fresh process.
    pub client: Client,
    /// The process ID returned by RMCP for this client's exact live server child; `restart` refreshes it.
    pub server_pid: u32,
}

impl Project {
    /// Register a fresh documentation project through the real `register_project` tool.
    pub async fn register() -> Project {
        let temp = tempfile::tempdir_in("/private/tmp").unwrap();
        let root = temp.path().join("portable-docs");
        let config = temp.path().join("config.toml");
        std::fs::write(&config, "schema_version = 1\n").unwrap();
        let (client, server_pid) = connect_with_pid(&config, temp.path(), &[]).await;
        call(
            &client,
            "register_project",
            json!({"project":"product","doc_dir":root,"name":"Qualification product","description":"Qualify knowledge, documents and Git persistence"}),
            false,
        )
        .await;
        Project {
            temp,
            root,
            config,
            alias: "product",
            client,
            server_pid,
        }
    }

    /// Replace the live process with a cold restart over the same disposable state.
    pub async fn restart(&mut self) {
        let (client, server_pid) = connect_with_pid(&self.config, self.temp.path(), &[]).await;
        let old = std::mem::replace(&mut self.client, client);
        self.server_pid = server_pid;
        let _ = old.cancel().await;
    }

    /// Call a tool that takes the project alias, adding `project` to `args`.
    pub async fn call(&self, name: &str, mut args: Value, error: bool) -> String {
        args["project"] = json!(self.alias);
        call(&self.client, name, args, error).await
    }

    /// Read the write version for `reference` through `get_context`.
    pub async fn version(&self, reference: &str) -> String {
        version(&self.client, self.alias, reference).await
    }

    /// Read the one knowledge allocation version printed by project context.
    pub async fn allocation_version(&self) -> String {
        let text = self.call("get_context", json!({}), false).await;
        knowledge_allocation(&text)
    }
}

/// Read the knowledge allocation observation from a project context reply.
pub fn knowledge_allocation(text: &str) -> String {
    for line in text.lines() {
        if let Some(rest) = line.strip_prefix("Knowledge allocation version: ") {
            return rest.trim().to_owned();
        }
    }
    panic!("no knowledge allocation version in: {text}")
}

/// One entry of a [`Tree`] snapshot.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    /// `file`, `dir` or `link`.
    pub kind: &'static str,
    /// Byte length for files, zero otherwise.
    pub len: u64,
    /// Hex sha256 for files, empty otherwise.
    pub sha256: String,
    /// Modification time in nanoseconds since the epoch.
    pub mtime_ns: u128,
}

/// Exact observation of every path below a directory, used to prove that reads write nothing.
pub type Tree = BTreeMap<String, Entry>;

/// Hex sha256 of bytes.
pub fn sha256_hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

/// Snapshot every path below `root` (links are recorded, never followed).
///
/// Records kind, length, sha256 and modification time in nanoseconds, so a new file, directory, lock, rewritten
/// byte or touched time all change the result. The `.git` directory is included on purpose.
pub fn tree(root: &Path) -> Tree {
    fn walk(base: &Path, dir: &Path, out: &mut Tree) {
        let mut names: Vec<_> = std::fs::read_dir(dir)
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .collect();
        names.sort();
        for path in names {
            let meta = std::fs::symlink_metadata(&path).unwrap();
            let relative = path
                .strip_prefix(base)
                .unwrap()
                .to_string_lossy()
                .into_owned();
            let mtime_ns = meta
                .modified()
                .unwrap()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            if meta.file_type().is_symlink() {
                out.insert(
                    relative,
                    Entry {
                        kind: "link",
                        len: 0,
                        sha256: String::new(),
                        mtime_ns,
                    },
                );
            } else if meta.is_dir() {
                out.insert(
                    relative.clone(),
                    Entry {
                        kind: "dir",
                        len: 0,
                        sha256: String::new(),
                        mtime_ns,
                    },
                );
                walk(base, &path, out);
            } else {
                let bytes = std::fs::read(&path).unwrap();
                out.insert(
                    relative,
                    Entry {
                        kind: "file",
                        len: meta.len(),
                        sha256: sha256_hex(&bytes),
                        mtime_ns,
                    },
                );
            }
        }
    }
    let mut out = Tree::new();
    walk(root, root, &mut out);
    out
}

/// Assert that nothing below `root` changed while `body` ran (the read-writeless proof).
pub async fn assert_tree_unchanged<F, Fut>(root: &Path, label: &str, body: F)
where
    F: FnOnce() -> Fut,
    Fut: std::future::Future<Output = ()>,
{
    let before = tree(root);
    body().await;
    let after = tree(root);
    assert_eq!(
        before, after,
        "{label}: a read changed the documentation root or its Git directory"
    );
}

/// Run real Git in `root` with a clean environment and a fixed test identity, returning stdout.
///
/// The identity and hook-free defaults apply only to the test's own inspection commands, never to the product.
pub fn git(root: &Path, args: &[&str]) -> String {
    let (ok, out, err) = git_status(root, args);
    assert!(ok, "git {args:?} failed: {err}");
    out
}

/// Run real Git in `root` and report `(success, stdout, stderr)` without asserting.
pub fn git_status(root: &Path, args: &[&str]) -> (bool, String, String) {
    let output = std::process::Command::new("git")
        .args(args)
        .current_dir(root)
        .env_clear()
        .env("HOME", root)
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_AUTHOR_NAME", "Fixture")
        .env("GIT_AUTHOR_EMAIL", "fixture@example.invalid")
        .env("GIT_COMMITTER_NAME", "Fixture")
        .env("GIT_COMMITTER_EMAIL", "fixture@example.invalid")
        .output()
        .unwrap();
    (
        output.status.success(),
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
    )
}

/// The full object id of HEAD.
pub fn head(root: &Path) -> String {
    git(root, &["rev-parse", "HEAD"]).trim().to_owned()
}

/// Number of commits reachable from HEAD.
pub fn commit_count(root: &Path) -> usize {
    git(root, &["rev-list", "--count", "HEAD"])
        .trim()
        .parse()
        .unwrap()
}

/// Sorted paths changed by one commit (deletions included).
pub fn commit_paths(root: &Path, commit: &str) -> Vec<String> {
    let mut paths: Vec<String> = git(root, &["show", "--name-only", "--format=", commit])
        .lines()
        .filter(|line| !line.is_empty())
        .map(str::to_owned)
        .collect();
    paths.sort();
    paths
}

/// Sorted paths currently staged in the index.
pub fn staged(root: &Path) -> Vec<String> {
    let mut paths: Vec<String> = git(root, &["diff", "--cached", "--name-only"])
        .lines()
        .map(str::to_owned)
        .collect();
    paths.sort();
    paths
}

/// Full commit message of one commit.
pub fn commit_message(root: &Path, commit: &str) -> String {
    git(root, &["show", "--no-patch", "--format=%B", commit])
}

/// Install an executable repository hook that runs `script` (a POSIX shell body).
pub fn install_hook(root: &Path, name: &str, script: &str) {
    use std::os::unix::fs::PermissionsExt;
    let hooks = root.join(".git").join("hooks");
    std::fs::create_dir_all(&hooks).unwrap();
    let path = hooks.join(name);
    std::fs::write(&path, format!("#!/bin/sh\n{script}\n")).unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
}

/// Remove an installed hook so the next commit attempt runs without it.
pub fn remove_hook(root: &Path, name: &str) {
    let _ = std::fs::remove_file(root.join(".git").join("hooks").join(name));
}

/// Make a directory read-only (or writable again) to produce a real post-publication failure.
pub fn set_writable(path: &Path, writable: bool) {
    use std::os::unix::fs::PermissionsExt;
    let mode = if writable { 0o755 } else { 0o555 };
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode)).unwrap();
}

/// The exact framing of one document content page, parsed from a reply with only its own fields.
#[derive(Debug, Clone)]
pub struct ContentPage {
    /// `raw` or `escaped`.
    pub wire: String,
    /// Raw byte offset of the first byte of this page.
    pub start: usize,
    /// Raw byte offset after the last byte of this page.
    pub end: usize,
    /// Raw end of the selected range.
    pub range_end: usize,
    /// Exact byte length of the encoded payload.
    pub encoded_len: usize,
    /// The encoded payload, delimited only by `encoded_len`.
    pub payload: String,
    /// The pagination snapshot of the read.
    pub snapshot: String,
    /// Everything before the framing line: the fixed header, never any payload.
    pub header: String,
    /// Everything after the payload: the footer, beginning with the line break that ends the payload.
    pub tail: String,
}

/// Parse the `Content:` framing line and its length-delimited payload, then check frame fidelity.
///
/// The framing line must be the first line that starts with `Content: md-text-v1 wire=`; the fixed header before it
/// may not contain another one. The payload begins on the line after the framing line and is exactly `encoded_len`
/// bytes, whatever it holds (a payload may contain its own `Content:`, `Next:` or `Snapshot version:` look-alike
/// lines, CRLF, or a repeated heading). The footer after the payload must begin with the line break, carry a `Next`
/// line that repeats this page's end, the snapshot and the exact remaining raw bytes when more remains, and carry no
/// `Next` line on the final page. A `.contains` check alone never proves frame fidelity; this does.
pub fn parse_content(text: &str) -> ContentPage {
    let marker = "Content: md-text-v1 wire=";
    let at = text
        .match_indices(marker)
        .map(|(at, _)| at)
        .find(|at| *at == 0 || text.as_bytes()[*at - 1] == b'\n')
        .unwrap_or_else(|| panic!("no framing line in: {text}"));
    let header = text[..at].to_owned();
    assert!(
        !header.contains(marker),
        "the header holds no second framing line: {header}"
    );
    let line_end = at + text[at..].find('\n').unwrap();
    let line = &text[at..line_end];
    let wire = line[marker.len()..].split(' ').next().unwrap().to_owned();
    let bytes_part = line.split("bytes=").nth(1).unwrap();
    let mut numbers = bytes_part.split(' ');
    let range = numbers.next().unwrap();
    let (start, end) = range.split_once('-').unwrap();
    let _of = numbers.next();
    let range_end: usize = numbers.next().unwrap().parse().unwrap();
    let encoded_len: usize = line
        .split("encoded_len=")
        .nth(1)
        .unwrap()
        .trim()
        .parse()
        .unwrap();
    let payload_start = line_end + 1;
    let payload_end = payload_start + encoded_len;
    let payload = text[payload_start..payload_end].to_owned();
    let tail = text[payload_end..].to_owned();
    let page = ContentPage {
        wire,
        start: start.parse().unwrap(),
        end: end.parse().unwrap(),
        range_end,
        encoded_len,
        payload,
        snapshot: snapshot_version(&header),
        header,
        tail,
    };
    assert!(
        page.start <= page.end && page.end <= page.range_end,
        "{page:?}"
    );
    assert_eq!(
        text.len(),
        page.header.len() + line.len() + 1 + page.encoded_len + page.tail.len(),
        "the frame accounts for every byte of the reply"
    );
    assert!(
        page.tail.starts_with('\n'),
        "the payload ends exactly at a line break: {:?}",
        page.tail
    );
    let next: Vec<&str> = page
        .tail
        .lines()
        .filter(|l| l.starts_with("Next:"))
        .collect();
    if page.end < page.range_end {
        let expected = format!(
            "Next: start={}; version={}; remaining={}.",
            page.end,
            page.snapshot,
            page.range_end - page.end
        );
        assert_eq!(
            next.len(),
            1,
            "exactly one Next line in the footer: {:?}",
            page.tail
        );
        assert!(
            next[0].starts_with(&expected),
            "{} vs {}",
            next[0],
            expected
        );
    } else {
        assert!(
            next.is_empty(),
            "the final page has no Next line: {:?}",
            page.tail
        );
    }
    page
}

/// Decode `md-text-v1` text: raw is verbatim, escaped resolves `\\`, `\r` and `\u{h}`.
pub fn decode_wire(wire: &str, text: &str) -> Vec<u8> {
    if wire == "raw" {
        return text.as_bytes().to_vec();
    }
    let mut out = String::new();
    let mut chars = text.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.next().expect("unterminated escape") {
            '\\' => out.push('\\'),
            'r' => out.push('\r'),
            'u' => {
                assert_eq!(chars.next(), Some('{'));
                let mut hex = String::new();
                for h in chars.by_ref() {
                    if h == '}' {
                        break;
                    }
                    hex.push(h);
                }
                out.push(char::from_u32(u32::from_str_radix(&hex, 16).unwrap()).unwrap());
            }
            other => panic!("unknown escape \\{other}"),
        }
    }
    out.into_bytes()
}

/// Read a whole document or selection page by page through `get_context view=content`, checking every frame.
///
/// `selector` holds the optional selection fields (`heading`, `occurrence`, `level`, `ordinal`, `preamble`). Every
/// page is parsed with [`parse_content`] (frame fidelity), must continue exactly where the previous page ended and
/// repeat the first page's snapshot and range end; every reply is also checked against the budget by [`call`].
pub async fn read_pages(project: &Project, reference: &str, selector: Value) -> Vec<ContentPage> {
    let mut pages: Vec<ContentPage> = Vec::new();
    loop {
        let mut args = json!({"ref":reference,"view":"content"});
        if let Value::Object(map) = &selector {
            for (key, value) in map {
                args[key] = value.clone();
            }
        }
        if let Some(previous) = pages.last() {
            args["start"] = json!(previous.end);
            args["version"] = json!(previous.snapshot);
        }
        let text = project.call("get_context", args, false).await;
        let page = parse_content(&text);
        if let Some(previous) = pages.last() {
            assert_eq!(page.start, previous.end, "pages are contiguous");
            assert_eq!(
                page.snapshot, previous.snapshot,
                "one snapshot for the whole read"
            );
            assert_eq!(page.range_end, previous.range_end, "one selected range");
            assert!(
                page.end > page.start || page.encoded_len == 0,
                "progress on every page"
            );
        }
        let done = page.end >= page.range_end;
        pages.push(page);
        if done {
            return pages;
        }
    }
}

/// Read a whole document or selection and reassemble its raw bytes; returns the bytes and the page count.
pub async fn read_document(
    project: &Project,
    reference: &str,
    selector: Value,
) -> (Vec<u8>, usize) {
    let pages = read_pages(project, reference, selector).await;
    let bytes: Vec<u8> = pages
        .iter()
        .flat_map(|page| decode_wire(&page.wire, &page.payload))
        .collect();
    (bytes, pages.len())
}

/// Deterministic byte vectors covering every Markdown dialect edge the document tests assert on.
pub mod vectors {
    /// A BOM, CRLF line ends, a fenced heading, duplicate headings and a final line without terminator.
    pub fn bom_crlf() -> Vec<u8> {
        let mut bytes = vec![0xEF, 0xBB, 0xBF];
        bytes.extend_from_slice(
            "# Title\r\npreamble line\r\n\r\n## Dup\r\nfirst\r\n```\r\n## not a heading\r\n```\r\n## Dup\r\nsecond\r\n## Tail\r\nno terminator"
                .as_bytes(),
        );
        bytes
    }

    /// Characters that force the escaped wire form: ESC, bidi override, C1 control, U+2028, DEL and a lone CR.
    pub fn hostile() -> Vec<u8> {
        "# Hostile\n\u{1b}[31mred\u{1b}[0m \u{202e}rtl\u{2028}sep \u{85}c1 \u{7f}del lone\rcr\n"
            .as_bytes()
            .to_vec()
    }

    /// Multibyte and non-BMP text with a backslash that must stay literal on the raw wire.
    pub fn multibyte() -> Vec<u8> {
        "# Привет 日本語 🚀\n\nC:\\path and a \\u{41} look-alike stay literal.\n"
            .as_bytes()
            .to_vec()
    }

    /// A document of exactly `len` bytes of ASCII paragraphs under a few headings.
    pub fn sized(len: usize) -> Vec<u8> {
        let mut out = String::new();
        let mut n = 0usize;
        while out.len() < len {
            if n.is_multiple_of(40) {
                out.push_str(&format!("## Section {}\n", n / 40));
            }
            out.push_str(&format!("line {n} of the sized qualification document.\n"));
            n += 1;
        }
        let mut bytes = out.into_bytes();
        bytes.truncate(len);
        bytes
    }
}

/// Run one `knowledge_work` operation with the right write precondition and a declared test actor.
///
/// `create_*` operations use the single knowledge allocation version; every other operation uses the record's own
/// version read through `get_context`. A caller may pre-set `version` to test a stale or foreign token.
pub async fn knowledge(project: &Project, mut args: Value, error: bool) -> String {
    let op = args["op"].as_str().unwrap().to_owned();
    if args.get("version").is_none() {
        let version = if op.starts_with("create_") {
            project.allocation_version().await
        } else {
            project.version(args["ref"].as_str().unwrap()).await
        };
        args["version"] = json!(version);
    }
    args["actor"] = json!("qualification-agent");
    project.call("knowledge_work", args, error).await
}

/// Run one `document_work` operation with the observation version of `ref` and a declared test actor.
pub async fn document(project: &Project, mut args: Value, error: bool) -> String {
    if args.get("version").is_none() {
        args["version"] = json!(project.version(args["ref"].as_str().unwrap()).await);
    }
    args["actor"] = json!("qualification-agent");
    project.call("document_work", args, error).await
}

/// Save a UTF-8 body at a managed path, creating or replacing it.
pub async fn save(project: &Project, path: &str, body: &str, error: bool) -> String {
    document(
        project,
        json!({"op":"save","ref":path,"purpose":"Qualification document","body":body}),
        error,
    )
    .await
}

/// The canonical target in the `SAVED <target>` first line of a mutation reply.
pub fn target(text: &str) -> String {
    text.lines()
        .next()
        .unwrap()
        .split_whitespace()
        .nth(1)
        .unwrap_or_else(|| panic!("no target in: {text}"))
        .to_owned()
}

/// Pending Git facts read from project context: the exact recovery version and every pending intent id.
#[derive(Debug, Clone)]
pub struct Pending {
    /// The exact pending observation version that `git_recovery` requires, empty when none is printed.
    pub version: String,
    /// Intent ids (`PG-` plus 24 hex characters) named by the context, oldest first.
    pub intents: Vec<String>,
}

/// Parse pending Git facts from a project context reply without assuming their layout beyond the ids.
pub fn pending_of(text: &str) -> Pending {
    let mut intents: Vec<String> = Vec::new();
    for word in text.split(|c: char| !(c.is_ascii_alphanumeric() || c == '-')) {
        if word.len() == 27 && word.starts_with("PG-") && !intents.contains(&word.to_owned()) {
            intents.push(word.to_owned());
        }
    }
    let version = text
        .lines()
        .find_map(|line| {
            let lower = line.to_lowercase();
            lower.find("pending version").map(|at| {
                line[at..]
                    .split([':', ' ', '.', ';'])
                    .find(|w| w.len() == 64)
                    .unwrap_or("")
                    .to_owned()
            })
        })
        .unwrap_or_default();
    Pending { version, intents }
}

/// Read pending Git facts through the real project context.
pub async fn pending(project: &Project) -> Pending {
    pending_of(&project.call("get_context", json!({}), false).await)
}

/// Call a tool and report whether the transport survived, for scenarios that kill the server on purpose.
pub async fn try_call(client: &Client, name: &str, args: Value) -> Result<String, String> {
    match client
        .call_tool(
            CallToolRequestParams::new(name.to_owned())
                .with_arguments(args.as_object().unwrap().clone()),
        )
        .await
    {
        Ok(reply) => Ok(serde_json::to_value(reply).unwrap()["content"][0]["text"]
            .as_str()
            .unwrap_or("")
            .to_owned()),
        Err(error) => Err(error.to_string()),
    }
}

/// Create or extend a disposable code repository with one commit whose message carries a Result report.
///
/// This is the *source* repository a Module reports against, not the documentation repository. Returns the full
/// commit id. Global and system Git configuration, hooks and signing are disabled for this fixture only.
pub fn source_commit(dir: &Path, file: &str, contents: &str, message: &str) -> String {
    std::fs::create_dir_all(dir).unwrap();
    let run = |args: &[&str]| {
        let output = std::process::Command::new("git")
            .current_dir(dir)
            .args(args)
            .env_clear()
            .env("HOME", dir)
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_AUTHOR_NAME", "Code report author")
            .env("GIT_AUTHOR_EMAIL", "report@example.invalid")
            .env("GIT_COMMITTER_NAME", "Code report committer")
            .env("GIT_COMMITTER_EMAIL", "committer@example.invalid")
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "source fixture git {args:?} failed"
        );
        String::from_utf8(output.stdout).unwrap().trim().to_owned()
    };
    if !dir.join(".git").exists() {
        run(&["-c", "init.defaultBranch=main", "init", "--quiet"]);
    }
    std::fs::write(dir.join(file), contents).unwrap();
    run(&["add", "--", file]);
    run(&[
        "-c",
        "commit.gpgsign=false",
        "-c",
        "core.hooksPath=/dev/null",
        "commit",
        "--quiet",
        "--cleanup=verbatim",
        "-m",
        message,
    ]);
    run(&["rev-parse", "HEAD"])
}

/// One executed step of [`work_cycle`] and the number of documentation commits it produced.
#[derive(Debug, Clone)]
pub struct Step {
    /// A short English name of the tool call.
    pub name: &'static str,
    /// First line of the reply.
    pub reply: String,
    /// Commits added to the documentation repository by this call.
    pub commits: usize,
}

/// Run one core work mutation, supplying the current write precondition and measuring commits it produced.
async fn work_step(
    project: &Project,
    name: &'static str,
    tool: &str,
    owner: Option<&str>,
    mut args: Value,
    error: bool,
) -> Step {
    let version = match owner {
        None => field(
            &project.call("get_context", json!({}), false).await,
            "Allocation version: ",
        ),
        Some(owner) => project.version(owner).await,
    };
    args["version"] = json!(version);
    let before = commit_count(&project.root);
    let reply = project.call(tool, args, error).await;
    Step {
        name,
        reply: reply.lines().next().unwrap_or("").to_owned(),
        commits: commit_count(&project.root) - before,
    }
}

/// Drive the complete work lifecycle through the real tools with synthetic, clearly labeled fixture receipts.
///
/// Covers `plan_work` (create, Task), `record_work` (bindings, discovery, begin, commit imports, completion,
/// boundary evidence) and `review_work` (independent acceptance). The binding receipts and boundary observations are
/// synthetic fixture data used only to exercise commit behavior; they prove no model session and no boundary.
/// Returns every step with its commit delta so a test can assert one commit per successful mutation.
pub async fn work_cycle(project: &Project) -> Vec<Step> {
    let source = project.temp.path().join("g1-source");
    let candidate = source_commit(
        &source,
        "lib.rs",
        "pub fn f(i: i32) -> i32 { i + 1 }\n",
        "fixture: boundary\n\nResult: fixture candidate\nChecks:\npassed | fixture | synthetic\n",
    );
    let execution =
        json!({"repository":source,"worktree":source,"branch":"main","target_branch":"main"});
    let lead = "g1-synthetic-lead";
    let reviewer = "g1-synthetic-reviewer";
    let bind = |role: &str, id: &str| json!({"op":"bind_agent","ref":"M-001","role":role,"harness":"fixture-only","agent_id":id,"communication_ref":format!("fixture-only: communicate {id}"),"resume_ref":format!("fixture-only: resume {id}"),"launch_ref":format!("fixture-only: receipt {id}")});
    let mut steps = Vec::new();
    steps.push(work_step(project, "create_module", "plan_work", None, json!({"op":"create_module","title":"G1 work cycle","outcome":"Each success commits once","criteria":["Each success commits once"],"execution":execution,"contracts":{"not_required":true}}), false).await);
    steps.push(
        work_step(
            project,
            "bind lead",
            "record_work",
            Some("M-001"),
            bind("lead", lead),
            false,
        )
        .await,
    );
    steps.push(work_step(project, "planning", "record_work", Some("M-001"), json!({"op":"planning","ref":"M-001","actor":lead,"responsibility":"Own the fixture","scope":"lib.rs","exclusions":["other"],"read_refs":["lib.rs"],"uncertainties":[]}), false).await);
    steps.push(
        work_step(
            project,
            "add_task",
            "plan_work",
            Some("M-001"),
            json!({"op":"add_task","module":"M-001","actor":lead,"title":"Implement fixture"}),
            false,
        )
        .await,
    );
    steps.push(
        work_step(
            project,
            "begin",
            "record_work",
            Some("M-001"),
            json!({"op":"begin","ref":"M-001","actor":lead}),
            false,
        )
        .await,
    );
    steps.push(
        work_step(
            project,
            "import task commit",
            "record_work",
            Some("M-001"),
            json!({"op":"import_commits","ref":"M-001/T-001","commits":[candidate],"actor":lead}),
            false,
        )
        .await,
    );
    steps.push(
        work_step(
            project,
            "complete task",
            "record_work",
            Some("M-001"),
            json!({"op":"complete","ref":"M-001/T-001","actor":lead}),
            false,
        )
        .await,
    );
    steps.push(
        work_step(
            project,
            "import module candidate",
            "record_work",
            Some("M-001"),
            json!({"op":"import_commits","ref":"M-001","commits":[candidate],"actor":lead}),
            false,
        )
        .await,
    );
    steps.push(work_step(project, "boundary evidence", "record_work", Some("M-001"), json!({"op":"boundary_evidence","ref":"M-001","actor":lead,"contract_id":"local","revision":1,"candidate":candidate,"conditions":"Synthetic fixture conditions","correct":{"status":"passed","detail":"synthetic"},"mutation":"Synthetic mutation","failed":{"status":"failed","detail":"synthetic"},"restored":{"status":"passed","detail":"synthetic"},"artifacts":[]}), false).await);
    steps.push(
        work_step(
            project,
            "bind reviewer",
            "record_work",
            Some("M-001"),
            bind("reviewer", reviewer),
            false,
        )
        .await,
    );
    steps.push(work_step(project, "review accepted", "review_work", Some("M-001"), json!({"ref":"M-001","verdict":"accepted","summary":"Synthetic fixture review","actor":reviewer}), false).await);
    steps
}

/// Focused checks for shared test-support invariants.
#[cfg(test)]
mod tests {
    /// The raw schema marker is allowed inside the parsed payload, but nowhere else in the reply.
    #[test]
    fn schema_marker_is_exempt_only_inside_the_framed_payload() {
        let reply = "Snapshot version: test\nContent: md-text-v1 wire=raw bytes=0-15 of 15 encoded_len=15\nschema_version:\n";
        assert!(super::schema_markers_are_confined_to_payload(reply));
        assert!(!super::schema_markers_are_confined_to_payload(&format!(
            "schema_version:\n{reply}"
        )));
        assert!(!super::schema_markers_are_confined_to_payload(&format!(
            "{reply}schema_version:"
        )));
    }
}
