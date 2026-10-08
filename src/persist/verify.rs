//! Proof that a landed commit holds exactly what the journal recorded.
//!
//! A commit is certified only when its message trailers equal the journal entries and its tree holds
//! the recorded bytes. Equal-looking trailers alone, a hook-rewritten worktree or a failed read never
//! certify anything. Git here is bounded and read-only except [`raw_blob_ids`], which stores the caller's
//! verified bytes as unreferenced objects and touches nothing else.
use super::{
    git,
    journal::{self, Intent},
};
use crate::store::{EffectKind, Store};
use sha2::{Digest, Sha256};
use std::time::Instant;

/// Result of checking a commit against recorded facts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verdict {
    /// Every checked fact matches.
    Exact,
    /// A checked fact is different.
    Mismatch,
    /// A fact could not be read, so nothing is proven.
    Unreadable,
}

/// Mode and object id of `path` in `commit`: `Ok(None)` when the commit has no such entry, `Err` when
/// Git could not answer (which is never the same as absent).
pub fn tree_entry(
    store: &Store,
    commit: &str,
    path: &str,
    deadline: Instant,
) -> Result<Option<(String, String)>, ()> {
    Ok(tree_entries(store, commit, &[path], deadline)?.remove(path))
}

/// Mode and object id of every listed path that `commit` holds, from one Git process.
///
/// A path the commit does not hold is simply absent from the map. `Err` means Git could not answer,
/// the output was cut, or a named path is not a blob; it is never the same as absent. One process for
/// the whole set keeps a retry over many pending intents linear instead of one spawn per path.
pub fn tree_entries<S: AsRef<str>>(
    store: &Store,
    commit: &str,
    paths: &[S],
    deadline: Instant,
) -> Result<std::collections::BTreeMap<String, (String, String)>, ()> {
    let mut found = std::collections::BTreeMap::new();
    if paths.is_empty() {
        return Ok(found);
    }
    let mut args = vec!["ls-tree", "-z", commit, "--"];
    args.extend(paths.iter().map(AsRef::as_ref));
    let out = git::run(&store.root, &args, None, deadline).map_err(|_| ())?;
    if !out.success() || out.truncated {
        return Err(());
    }
    let text = String::from_utf8_lossy(&out.stdout).into_owned();
    for record in text.split('\0').filter(|r| !r.is_empty()) {
        let (meta, name) = record.split_once('\t').ok_or(())?;
        if !paths.iter().any(|p| p.as_ref() == name) {
            continue;
        }
        let mut fields = meta.split(' ');
        let mode = fields.next().ok_or(())?;
        let kind = fields.next().ok_or(())?;
        let oid = fields.next().ok_or(())?;
        if kind != "blob" {
            return Err(());
        }
        found.insert(name.to_owned(), (mode.to_owned(), oid.to_owned()));
    }
    Ok(found)
}

/// Object ids of the exact `blobs`, stored without any filter, from one Git process.
///
/// `oid_len` is the hexadecimal length of this repository's object ids (40 or 64). The bytes are copied
/// to private files inside the Git directory first, so the stored objects are the caller's verified bytes
/// and never a later native edit of the working file. The returned ids are in input order. See
/// [`raw_blob_ids_in`] for the directory discipline; this wrapper only picks a fresh unpredictable name.
pub fn raw_blob_ids(
    store: &Store,
    blobs: &[&[u8]],
    oid_len: usize,
    deadline: Instant,
) -> Result<Vec<String>, ()> {
    let name = format!(
        "blobs.{}-{}",
        std::process::id(),
        journal::new_intent_id(store)
    );
    raw_blob_ids_in(store, &name, blobs, oid_len, deadline)
}

/// [`raw_blob_ids`] with an explicit private directory `name` below `.git/agent-tasks`.
///
/// The parent must already be a real directory (a link is refused, never followed). The private
/// directory is created exclusively: anything already at `name`, including a link or foreign data, is
/// refused untouched. Every byte file is created exclusively inside it, so no path collision can
/// overwrite or follow anything. Cleanup removes only the files and the directory this call created, and
/// only when it created them. The ids must be exactly one lowercase hexadecimal line of `oid_len`
/// characters per blob in valid UTF-8, from a successful and untruncated Git run; anything else is `Err`.
pub fn raw_blob_ids_in(
    store: &Store,
    name: &str,
    blobs: &[&[u8]],
    oid_len: usize,
    deadline: Instant,
) -> Result<Vec<String>, ()> {
    if blobs.is_empty() {
        return Ok(Vec::new());
    }
    let parent = store.root.join(".git/agent-tasks");
    if !std::fs::symlink_metadata(&parent).is_ok_and(|m| m.is_dir()) {
        return Err(());
    }
    let dir = parent.join(name);
    let mut builder = std::fs::DirBuilder::new();
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        builder.mode(0o700);
    }
    builder.create(&dir).map_err(|_| ())?;
    // From here the directory is ours alone; `created` names every file this call made inside it.
    let mut created = Vec::new();
    let result = (|| {
        let mut list = String::new();
        for (n, bytes) in blobs.iter().enumerate() {
            let file = dir.join(n.to_string());
            let mut options = std::fs::OpenOptions::new();
            options.write(true).create_new(true);
            #[cfg(unix)]
            {
                use std::os::unix::fs::OpenOptionsExt;
                options.mode(0o600);
            }
            let mut handle = options.open(&file).map_err(|_| ())?;
            created.push(file);
            std::io::Write::write_all(&mut handle, bytes).map_err(|_| ())?;
            // Git runs inside the root, so the list is relative to it and no root name can break it.
            list.push_str(&format!(".git/agent-tasks/{name}/{n}\n"));
        }
        let out = git::run(
            &store.root,
            &["hash-object", "-w", "--no-filters", "--stdin-paths"],
            Some(list.as_bytes()),
            deadline,
        )
        .map_err(|_| ())?;
        if !out.success() || out.truncated {
            return Err(());
        }
        let text = String::from_utf8(out.stdout).map_err(|_| ())?;
        let ids: Vec<&str> = text.strip_suffix('\n').ok_or(())?.split('\n').collect();
        let valid = |id: &&str| {
            id.len() == oid_len && id.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
        };
        (ids.len() == blobs.len() && ids.iter().all(valid))
            .then(|| ids.into_iter().map(str::to_owned).collect())
            .ok_or(())
    })();
    for file in &created {
        let _ = std::fs::remove_file(file);
    }
    let _ = std::fs::remove_dir(&dir);
    result
}

/// Whether the committed bytes of `path` have SHA-256 `digest` (`None` means the path must be absent).
pub fn path_matches(
    store: &Store,
    commit: &str,
    path: &str,
    digest: Option<&str>,
    deadline: Instant,
) -> Verdict {
    let entry = match tree_entry(store, commit, path, deadline) {
        Ok(entry) => entry,
        Err(()) => return Verdict::Unreadable,
    };
    let (Some(want), Some((_, oid))) = (digest, entry.as_ref()) else {
        return match (digest, entry) {
            (None, None) => Verdict::Exact,
            _ => Verdict::Mismatch,
        };
    };
    let size = git::read_text(&store.root, &["cat-file", "-s", oid], deadline)
        .and_then(|s| s.parse::<usize>().ok());
    match size {
        Some(n) if n <= git::STDOUT_CAP => {
            match git::run(&store.root, &["cat-file", "blob", oid], None, deadline) {
                Ok(b) if b.success() && !b.truncated => {
                    if format!("{:x}", Sha256::digest(&b.stdout)) == want {
                        Verdict::Exact
                    } else {
                        Verdict::Mismatch
                    }
                }
                _ => Verdict::Unreadable,
            }
        }
        Some(_) => {
            // Too large to read back through the bounded runner: compare the blob id with the id of the
            // file's raw bytes, which is only meaningful while the file still has the recorded digest.
            let current = store
                .read_exact(path, crate::store::ABSOLUTE_CAP)
                .ok()
                .flatten()
                .filter(|f| journal::sha256_hex(&f.bytes) == want);
            match current {
                Some(file) => match raw_blob_id(store, &file.bytes, false, deadline) {
                    Some(id) if id == *oid => Verdict::Exact,
                    Some(_) => Verdict::Mismatch,
                    None => Verdict::Unreadable,
                },
                None => Verdict::Unreadable,
            }
        }
        None => Verdict::Unreadable,
    }
}

/// Object id of `bytes` stored without any filter; `write` also stores the object.
pub fn raw_blob_id(store: &Store, bytes: &[u8], write: bool, deadline: Instant) -> Option<String> {
    let args: &[&str] = if write {
        &["hash-object", "-w", "--no-filters", "--stdin"]
    } else {
        &["hash-object", "--no-filters", "--stdin"]
    };
    let out = git::run(&store.root, args, Some(bytes), deadline).ok()?;
    out.success()
        .then(|| out.text())
        .filter(|id| !id.is_empty())
}

/// One `Agent-Tasks-Effect` trailer line, fields as written.
struct Effect {
    /// Intent block the line belongs to.
    intent: String,
    /// Operation identity or `-`.
    operation: String,
    /// Effect kind word.
    kind: String,
    /// Decoded path.
    path: String,
    /// Before digest or `-`.
    before: String,
    /// After digest or `-`.
    after: String,
}

/// Effect trailers of one commit message, in order.
fn effects(message: &str) -> Vec<Effect> {
    let mut intent = String::new();
    let mut out = Vec::new();
    for line in message.lines() {
        if let Some(id) = line.strip_prefix("Agent-Tasks-Intent: ") {
            intent = id.trim().to_owned();
        } else if let Some(rest) = line.strip_prefix("Agent-Tasks-Effect: ") {
            let parts: Vec<&str> = rest.split(' ').collect();
            if parts.len() >= 5 {
                out.push(Effect {
                    intent: intent.clone(),
                    operation: parts[0].to_owned(),
                    kind: parts[1].to_owned(),
                    path: super::status::decode_path(parts[2]),
                    before: parts[3].to_owned(),
                    after: parts[4].to_owned(),
                });
            }
        }
    }
    out
}

/// Trailer kind word of an effect kind.
pub fn kind_word(kind: EffectKind) -> &'static str {
    match kind {
        EffectKind::Created => "created",
        EffectKind::Replaced => "replaced",
        _ => "removed",
    }
}

/// Whether `commit` is the commit the journal says it is for `intent`: the message carries the intent
/// block with trailer lines equal to the journal entries, and for every entry not superseded by a later
/// commit the tree holds the last recorded bytes of that path.
pub fn intent_in_commit(
    store: &Store,
    commit: &str,
    intent: &Intent,
    deadline: Instant,
) -> Verdict {
    let Some(message) =
        git::read_text(&store.root, &["log", "-1", "--format=%B", commit], deadline)
    else {
        return Verdict::Unreadable;
    };
    let all = effects(&message);
    let mine: Vec<&Effect> = all.iter().filter(|e| e.intent == intent.id).collect();
    if !message
        .lines()
        .any(|l| l.strip_prefix("Agent-Tasks-Intent: ").map(str::trim) == Some(&intent.id))
        || mine.len() != intent.entries.len()
    {
        return Verdict::Mismatch;
    }
    let digest = |d: &Option<String>| d.clone().unwrap_or_else(|| "-".to_owned());
    for (entry, line) in intent.entries.iter().zip(&mine) {
        if line.operation != entry.operation.as_deref().unwrap_or("-")
            || line.kind != kind_word(entry.kind)
            || line.path != entry.path
            || line.before != digest(&entry.before_sha256)
            || line.after != digest(&entry.after_sha256)
        {
            return Verdict::Mismatch;
        }
    }
    let mut result = Verdict::Exact;
    for entry in intent.entries.iter().filter(|e| e.superseded_by.is_none()) {
        let last = all
            .iter()
            .rfind(|e| e.path == entry.path)
            .map(|e| e.after.clone());
        let want = last.filter(|a| a != "-");
        match path_matches(store, commit, &entry.path, want.as_deref(), deadline) {
            Verdict::Exact => (),
            Verdict::Mismatch => return Verdict::Mismatch,
            Verdict::Unreadable => result = Verdict::Unreadable,
        }
    }
    result
}
