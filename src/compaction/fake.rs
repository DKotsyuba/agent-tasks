//! Faithful test substitute of the provider facts behind [`Env`].
//!
//! The record and staged blobs live in a real disposable [`Store`]; documents, DOC records, the effect
//! journal, commits and the call intents are modelled in memory with the semantics the artifacts fix:
//! equal bytes never certify ownership, held intents are never committed by a later success, a
//! relocate resumes with the original identity, and directory effects are not owned files. This double
//! is module evidence only and never proves composition with the real providers.
#![allow(clippy::unwrap_used, clippy::expect_used, reason = "Test fixture")]
use super::{
    env::*,
    record::{self, SectionAddr},
};
use crate::store::{Error, Result, Store};
use std::{cell::RefCell, collections::BTreeMap};

/// Where a one-shot failure is injected inside a document operation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[allow(
    clippy::enum_variant_names,
    reason = "Each variant names the step it follows"
)]
pub enum Inject {
    /// After a save wrote the body and before it wrote the record.
    AfterBody,
    /// After a relocate created the destination body.
    AfterTo,
    /// After a relocate moved the record, before it removed the source.
    AfterRecord,
}

/// One DOC record of the in-memory document world.
#[derive(Clone, Debug)]
struct Doc {
    id: String,
    path: String,
    body_sha: String,
    revision: u64,
    retired: bool,
}
impl Doc {
    /// Exact synthetic record bytes whose digest changes with every field.
    fn bytes(&self) -> Vec<u8> {
        format!(
            "id: {}\npath: {}\nbody: {}\nrevision: {}\nretired: {}\n",
            self.id, self.path, self.body_sha, self.revision, self.retired
        )
        .into_bytes()
    }
}

/// Git state of one journal row.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Git {
    Untracked,
    Pending,
    Held,
    Committed(String),
}

/// One attested effect row of the in-memory journal.
#[derive(Clone, Debug)]
struct Row {
    op: String,
    relative: String,
    kind: EffectKindView,
    before: Option<String>,
    after: Option<String>,
    call: u32,
    git: Git,
}

/// The whole in-memory provider world behind the double.
#[derive(Default)]
struct World {
    files: BTreeMap<String, Vec<u8>>,
    docs: BTreeMap<String, Doc>,
    next_doc: u64,
    rows: Vec<Row>,
    events: Vec<EventView>,
    call: u32,
    commits: BTreeMap<String, Vec<(String, String)>>,
    commit_seq: u32,
    incoming: BTreeMap<String, IncomingFacts>,
    dangling: Vec<String>,
    inject: Vec<Inject>,
    untracked_sibling: bool,
    /// Publication events of the following effects carry no journal entry.
    untracked_events: bool,
    reserved: u64,
    doc_ops: Vec<String>,
}

/// The test double: a real store for records and blobs plus an in-memory document world.
pub struct FakeEnv {
    /// Keep the disposable root alive.
    pub dir: tempfile::TempDir,
    store: Store,
    w: RefCell<World>,
}

/// Hex sha256 of bytes.
fn sha(bytes: &[u8]) -> String {
    record::sha256_hex(bytes)
}

/// Build a refusal with a stable code.
fn err(code: &'static str, m: &str) -> Error {
    Error::new(code, m)
}

impl FakeEnv {
    /// A fresh double with an empty documentation root.
    pub fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("docs-root");
        std::fs::create_dir(&root).unwrap();
        let store = Store::from_root(&root).unwrap();
        let w = World {
            next_doc: 1,
            reserved: 1,
            ..World::default()
        };
        Self {
            dir,
            store,
            w: RefCell::new(w),
        }
    }

    /// Seed a document (and its DOC record when `managed`) and commit it, as normal saves would be.
    pub fn seed(&self, path: &str, body: &str, managed: bool) {
        let mut w = self.w.borrow_mut();
        w.files.insert(path.to_owned(), body.as_bytes().to_vec());
        w.commit_seq += 1;
        let commit = format!("c{}", w.commit_seq);
        w.commits
            .entry(path.to_owned())
            .or_default()
            .push((commit.clone(), sha(body.as_bytes())));
        if managed {
            let id = format!("DOC-{:03}", w.next_doc);
            w.next_doc += 1;
            let doc = Doc {
                id: id.clone(),
                path: path.to_owned(),
                body_sha: sha(body.as_bytes()),
                revision: 1,
                retired: false,
            };
            let rel = format!("documents/{id}.yaml");
            w.commits
                .entry(rel)
                .or_default()
                .push((commit, sha(&doc.bytes())));
            w.docs.insert(id, doc);
        }
    }
    /// Make an uncommitted original: the file changed after its last commit.
    pub fn dirty(&self, path: &str, body: &str) {
        self.w
            .borrow_mut()
            .files
            .insert(path.to_owned(), body.as_bytes().to_vec());
    }
    /// Set the incoming reference coverage of a path.
    pub fn set_incoming(&self, path: &str, facts: IncomingFacts) {
        self.w.borrow_mut().incoming.insert(path.to_owned(), facts);
    }
    /// Make the integrity preview report these dangling references.
    pub fn set_dangling(&self, v: Vec<String>) {
        self.w.borrow_mut().dangling = v;
    }
    /// Queue a one-shot failure.
    pub fn inject(&self, i: Inject) {
        self.w.borrow_mut().inject.push(i);
    }
    /// Make every following publication event untracked, as a full or unwritable journal would.
    pub fn untracked_events(&self, on: bool) {
        self.w.borrow_mut().untracked_events = on;
    }
    /// Add a real, non-ignored, untracked sibling file that blocks the whole current commit.
    pub fn untracked_sibling(&self, on: bool) {
        self.w.borrow_mut().untracked_sibling = on;
    }
    /// Forget the journal rows of operations whose identity starts with a prefix, as a pruned or lost
    /// journal would: the bytes stay, the attestation is gone.
    pub fn forget_rows(&self, prefix: &str) {
        self.w
            .borrow_mut()
            .rows
            .retain(|r| !r.op.starts_with(prefix));
    }
    /// Change only the metadata of a managed document, as an external DOC record edit would.
    pub fn bump_record(&self, path: &str) {
        let mut w = self.w.borrow_mut();
        if let Some(d) = w.docs.values_mut().find(|d| d.path == path && !d.retired) {
            d.revision += 1;
        }
    }
    /// Re-key the DOC record of a managed path to another identifier, as a foreign record swap would.
    pub fn swap_record_identity(&self, path: &str, new_id: &str) {
        let mut w = self.w.borrow_mut();
        let key = w
            .docs
            .iter()
            .find(|(_, d)| d.path == path && !d.retired)
            .map(|(k, _)| k.clone());
        if let Some(mut d) = key.and_then(|k| w.docs.remove(&k)) {
            d.id = new_id.to_owned();
            w.docs.insert(new_id.to_owned(), d);
        }
    }
    /// Begin one handler call (one atomic intent).
    pub fn start_call(&self) {
        let mut w = self.w.borrow_mut();
        w.call += 1;
        w.events.clear();
    }
    /// Settle the call: success commits the whole current intent when eligible, failure holds it.
    pub fn finish_call(&self, success: bool) {
        let mut w = self.w.borrow_mut();
        let call = w.call;
        let eligible = success
            && !w.untracked_sibling
            && w.events
                .iter()
                .all(|e| e.durable && e.tracking != TrackingView::Untracked);
        let any = w.rows.iter().any(|r| r.call == call);
        if !any {
            return;
        }
        if eligible {
            w.commit_seq += 1;
            let commit = format!("c{}", w.commit_seq);
            let mut adds = Vec::new();
            for r in w
                .rows
                .iter_mut()
                .filter(|r| r.call == call && r.git == Git::Pending)
            {
                r.git = Git::Committed(commit.clone());
                if let Some(a) = &r.after {
                    adds.push((r.relative.clone(), commit.clone(), a.clone()));
                }
            }
            for (p, c, a) in adds {
                w.commits.entry(p).or_default().push((c, a));
            }
        } else if !success {
            for r in w
                .rows
                .iter_mut()
                .filter(|r| r.call == call && r.git == Git::Pending)
            {
                r.git = Git::Held;
            }
        }
    }
    /// Explicit recovery Retry of every held or pending intent of earlier calls.
    pub fn retry_all(&self) {
        let mut w = self.w.borrow_mut();
        w.commit_seq += 1;
        let commit = format!("c{}", w.commit_seq);
        let mut adds = Vec::new();
        for r in w
            .rows
            .iter_mut()
            .filter(|r| matches!(r.git, Git::Held | Git::Pending))
        {
            r.git = Git::Committed(commit.clone());
            if let Some(a) = &r.after {
                adds.push((r.relative.clone(), commit.clone(), a.clone()));
            }
        }
        for (p, c, a) in adds {
            w.commits.entry(p).or_default().push((c, a));
        }
    }
    /// Current document body, for assertions.
    pub fn body(&self, path: &str) -> Option<Vec<u8>> {
        self.w.borrow().files.get(path).cloned()
    }
    /// The DOC record currently claiming a path, active or retired, for assertions.
    pub fn doc_for(&self, path: &str) -> Option<(String, u64, bool)> {
        self.w
            .borrow()
            .docs
            .values()
            .find(|d| d.path == path)
            .map(|d| (d.id.clone(), d.revision, d.retired))
    }
    /// Names of document operations executed so far, for assertions.
    pub fn doc_ops(&self) -> Vec<String> {
        self.w.borrow().doc_ops.clone()
    }
    /// Whether every journal row of the call is committed.
    pub fn committed_rows(&self) -> usize {
        self.w
            .borrow()
            .rows
            .iter()
            .filter(|r| matches!(r.git, Git::Committed(_)))
            .count()
    }

    /// Append an effect row and its tracked event to the current call.
    fn row(
        &self,
        op: &str,
        relative: &str,
        kind: EffectKindView,
        before: Option<String>,
        after: Option<String>,
    ) {
        let mut w = self.w.borrow_mut();
        let call = w.call;
        let untracked = w.untracked_events;
        w.events.push(EventView {
            relative: relative.to_owned(),
            operation: Some(op.to_owned()),
            intent: Some(format!("PG-{call}")),
            tracking: if untracked {
                TrackingView::Untracked
            } else {
                TrackingView::Tracked
            },
            durable: true,
        });
        w.rows.push(Row {
            op: op.to_owned(),
            relative: relative.to_owned(),
            kind,
            before,
            after,
            call,
            git: Git::Pending,
        });
    }

    /// Observe a path in the given world.
    fn facts(&self, w: &World, path: &str) -> DocFacts {
        let body = w.files.get(path).cloned();
        let claim = w.docs.values().find(|d| d.path == path && !d.retired);
        let record = claim.map(|d| RecordFacts {
            path: format!("documents/{}.yaml", d.id),
            id: d.id.clone(),
            bound_path: d.path.clone(),
            body_sha256: d.body_sha.clone(),
            revision: d.revision,
            sha256: sha(&d.bytes()),
            len: d.bytes().len() as u64,
        });
        let state = match (&body, claim) {
            (Some(b), Some(d)) => {
                if sha(b) == d.body_sha {
                    DocState::Managed
                } else {
                    DocState::Other("drifted".into())
                }
            }
            (Some(_), None) => DocState::Unmanaged,
            (None, Some(_)) => DocState::Other("missing body".into()),
            (None, None) => DocState::Absent,
        };
        let version = sha(format!(
            "{path}|{}|{}",
            body.as_deref().map(sha).unwrap_or_default(),
            record
                .as_ref()
                .map(|r| r.sha256.clone())
                .unwrap_or_default()
        )
        .as_bytes());
        DocFacts {
            path: path.to_owned(),
            state,
            version,
            body,
            record,
            retired: false,
        }
    }

    /// Consume one queued failure point when it matches.
    fn take_inject(&self, i: Inject) -> bool {
        let mut w = self.w.borrow_mut();
        if let Some(pos) = w.inject.iter().position(|x| *x == i) {
            w.inject.remove(pos);
            return true;
        }
        false
    }

    /// Publish a DOC record effect.
    fn write_record(&self, op: &str, doc: Doc, created: bool) {
        let rel = format!("documents/{}.yaml", doc.id);
        let before = if created {
            None
        } else {
            self.w.borrow().docs.get(&doc.id).map(|d| sha(&d.bytes()))
        };
        let after = sha(&doc.bytes());
        self.w.borrow_mut().docs.insert(doc.id.clone(), doc);
        self.row(
            op,
            &rel,
            if created {
                EffectKindView::Created
            } else {
                EffectKindView::Replaced
            },
            before,
            Some(after),
        );
    }
}

/// Minimal ATX outline used by the double: preamble (when nonempty) then every heading section.
pub fn outline_of(bytes: &[u8]) -> OutlineFacts {
    let text = String::from_utf8_lossy(bytes).into_owned();
    let mut heads: Vec<(usize, u8, String)> = Vec::new();
    let mut offset = 0usize;
    let mut fenced = false;
    for line in text.split_inclusive('\n') {
        let t = line.trim_end_matches('\n');
        if t.trim_start().starts_with("```") {
            fenced = !fenced;
        } else if !fenced && t.starts_with('#') {
            let level = t.bytes().take_while(|b| *b == b'#').count();
            if (1..=6).contains(&level) && t[level..].starts_with(' ') {
                heads.push((offset, level as u8, t[level..].trim().to_owned()));
            }
        }
        offset += line.len();
    }
    let mut sections = Vec::new();
    let first = heads.first().map_or(bytes.len(), |h| h.0);
    if first > 0 {
        let b = bytes[..first].to_vec();
        sections.push(SectionFact {
            addr: SectionAddr::Preamble,
            sha256: sha(&b),
            bytes: b,
        });
    }
    let mut seen: BTreeMap<String, usize> = BTreeMap::new();
    let mut slugs = Vec::new();
    for (i, (start, level, text)) in heads.iter().enumerate() {
        let end = heads[i + 1..]
            .iter()
            .find(|h| h.1 <= *level)
            .map_or(bytes.len(), |h| h.0);
        let occ = seen.entry(text.clone()).or_insert(0);
        *occ += 1;
        let b = bytes[*start..end].to_vec();
        sections.push(SectionFact {
            addr: SectionAddr::Heading {
                ordinal: i,
                level: *level,
                occurrence: *occ,
                text_sha256: sha(text.as_bytes()),
            },
            sha256: sha(&b),
            bytes: b,
        });
        slugs.push(
            text.to_lowercase()
                .chars()
                .filter(|c| c.is_alphanumeric() || *c == ' ' || *c == '-')
                .collect::<String>()
                .replace(' ', "-"),
        );
    }
    OutlineFacts {
        complete: true,
        setext_candidates: 0,
        sections,
        slugs,
    }
}

impl Env for FakeEnv {
    /// The disposable real store.
    fn store(&self) -> &Store {
        &self.store
    }
    /// Allocation version derived from the reservation counter.
    fn allocation_version(&self) -> Result<String> {
        Ok(format!("alloc-{}", self.w.borrow().reserved))
    }
    /// Reserve the next proposal number.
    fn reserve(&self, expected: &str, effects: &mut Vec<String>) -> Result<String> {
        let current = self.allocation_version()?;
        if current != expected {
            return Err(err("stale", "allocation version changed"));
        }
        let mut w = self.w.borrow_mut();
        let id = format!("CP-{:03}", w.reserved);
        w.reserved += 1;
        effects.push("Published .agent-tasks/knowledge.yaml.".into());
        Ok(id)
    }
    /// Whether a DOC identifier exists in the world.
    fn allocation_valid(&self, doc_id: &str) -> Result<bool> {
        Ok(self.w.borrow().docs.contains_key(doc_id))
    }
    /// Create parent directories and record a not-applicable directory event.
    fn ensure_parents(&self, relative: &str, effects: &mut Vec<String>) -> Result<()> {
        let path = self.store.path(relative)?;
        if let Some(parent) = path.parent()
            && !parent.exists()
        {
            std::fs::create_dir_all(parent).map_err(|_| err("io", "cannot create parents"))?;
            let mut w = self.w.borrow_mut();
            w.events.push(EventView {
                relative: relative.to_owned(),
                operation: None,
                intent: None,
                tracking: TrackingView::NotApplicable,
                durable: true,
            });
            effects.push(format!("Created directory for {relative}."));
        }
        Ok(())
    }
    /// Observe one document path.
    fn observe(&self, path: &str) -> Result<DocFacts> {
        let w = self.w.borrow();
        Ok(self.facts(&w, path))
    }
    /// Observe a DOC identifier including a retired record.
    fn observe_id(&self, id: &str) -> Result<DocFacts> {
        let w = self.w.borrow();
        let d = w
            .docs
            .get(id)
            .ok_or_else(|| err("not_found", "unknown DOC id"))?;
        let mut f = self.facts(&w, &d.path);
        let rec = RecordFacts {
            path: format!("documents/{}.yaml", d.id),
            id: d.id.clone(),
            bound_path: d.path.clone(),
            body_sha256: d.body_sha.clone(),
            revision: d.revision,
            sha256: sha(&d.bytes()),
            len: d.bytes().len() as u64,
        };
        f.record = Some(rec);
        if d.retired {
            f.state = DocState::Retired;
            f.retired = true;
        }
        Ok(f)
    }
    /// Outline exact bytes.
    fn outline(&self, bytes: &[u8]) -> Result<OutlineFacts> {
        Ok(outline_of(bytes))
    }
    /// Configured incoming coverage, complete and empty by default.
    fn incoming(&self, path: &str) -> Result<IncomingFacts> {
        Ok(self
            .w
            .borrow()
            .incoming
            .get(path)
            .cloned()
            .unwrap_or(IncomingFacts {
                complete: true,
                gaps: vec![],
                rows: vec![],
            }))
    }
    /// Configured integrity preview.
    fn integrity(&self, _overlay: &OverlayFacts) -> Result<IntegrityFacts> {
        Ok(IntegrityFacts {
            complete: true,
            gaps: vec![],
            introduced: self.w.borrow().dangling.clone(),
        })
    }
    /// Execute one document operation with the semantics the document owner fixes.
    fn doc_op(
        &self,
        op: &str,
        _actor: &str,
        doc: &DocOp,
        _effects: &mut Vec<String>,
    ) -> Result<()> {
        match doc {
            DocOp::Save {
                path,
                body,
                purpose: _,
                expected,
            } => {
                self.w.borrow_mut().doc_ops.push(format!("save {path}"));
                let f = self.observe(path)?;
                if &f.version != expected {
                    return Err(err("stale", "save: stale version"));
                }
                let before = f.body_sha256();
                let new_sha = sha(body);
                self.w.borrow_mut().files.insert(path.clone(), body.clone());
                self.row(
                    op,
                    path,
                    if before.is_some() {
                        EffectKindView::Replaced
                    } else {
                        EffectKindView::Created
                    },
                    before,
                    Some(new_sha.clone()),
                );
                if self.take_inject(Inject::AfterBody) {
                    return Err(err(
                        "partial_publication",
                        "Partially saved. body published, record missing",
                    ));
                }
                let existing = self
                    .w
                    .borrow()
                    .docs
                    .values()
                    .find(|d| &d.path == path && !d.retired)
                    .cloned();
                match existing {
                    Some(mut d) => {
                        d.body_sha = new_sha;
                        d.revision += 1;
                        self.write_record(op, d, false);
                    }
                    None => {
                        let id = {
                            let mut w = self.w.borrow_mut();
                            let id = format!("DOC-{:03}", w.next_doc);
                            w.next_doc += 1;
                            id
                        };
                        let d = Doc {
                            id,
                            path: path.clone(),
                            body_sha: new_sha,
                            revision: 1,
                            retired: false,
                        };
                        self.write_record(op, d, true);
                    }
                }
                Ok(())
            }
            DocOp::Adopt {
                path,
                purpose: _,
                expected,
            } => {
                self.w.borrow_mut().doc_ops.push(format!("adopt {path}"));
                let f = self.observe(path)?;
                if &f.version != expected {
                    return Err(err("stale", "adopt: stale version"));
                }
                let new_sha = f.body_sha256().ok_or_else(|| err("not_found", "no body"))?;
                let existing = self
                    .w
                    .borrow()
                    .docs
                    .values()
                    .find(|d| &d.path == path && !d.retired)
                    .cloned();
                match existing {
                    Some(mut d) => {
                        d.body_sha = new_sha;
                        d.revision += 1;
                        self.write_record(op, d, false);
                    }
                    None => {
                        let id = {
                            let mut w = self.w.borrow_mut();
                            let id = format!("DOC-{:03}", w.next_doc);
                            w.next_doc += 1;
                            id
                        };
                        self.write_record(
                            op,
                            Doc {
                                id,
                                path: path.clone(),
                                body_sha: new_sha,
                                revision: 1,
                                retired: false,
                            },
                            true,
                        );
                    }
                }
                Ok(())
            }
            DocOp::Relocate {
                from,
                to,
                expected: _,
                expected_to: _,
                basis,
            } => {
                self.w
                    .borrow_mut()
                    .doc_ops
                    .push(format!("relocate {from} {to}"));
                let (body_from, body_to, rec_from, rec_to) = {
                    let w = self.w.borrow();
                    (
                        w.files.get(from).cloned(),
                        w.files.get(to).cloned(),
                        w.docs
                            .values()
                            .find(|d| d.path == *from && !d.retired)
                            .cloned(),
                        w.docs
                            .values()
                            .find(|d| d.path == *to && !d.retired)
                            .cloned(),
                    )
                };
                // Identity bearing resume: the destination body is skipped only when this operation
                // attested its creation; an equal unattested copy is never adopted.
                let created_to =
                    self.w.borrow().rows.iter().any(|r| {
                        r.op == op && r.relative == *to && r.kind == EffectKindView::Created
                    });
                if body_to.is_some() && !created_to {
                    return Err(err("collision", "relocate: destination is occupied"));
                }
                if !created_to {
                    let src = body_from
                        .clone()
                        .ok_or_else(|| err("not_found", "relocate: source missing"))?;
                    self.w.borrow_mut().files.insert(to.clone(), src.clone());
                    self.row(op, to, EffectKindView::Created, None, Some(sha(&src)));
                    if self.take_inject(Inject::AfterTo) {
                        return Err(err(
                            "partial_publication",
                            "Partially saved. destination created",
                        ));
                    }
                }
                if let Some(mut d) = rec_from.filter(|_| rec_to.is_none()) {
                    if basis.id.as_deref() != Some(d.id.as_str()) {
                        return Err(err(
                            "metadata_mismatch",
                            "relocate: identity differs from the basis",
                        ));
                    }
                    d.path = to.clone();
                    d.revision += 1;
                    self.write_record(op, d, false);
                    if self.take_inject(Inject::AfterRecord) {
                        return Err(err("partial_publication", "Partially saved. record moved"));
                    }
                }
                let still = self.w.borrow().files.get(from).cloned();
                if let Some(b) = still {
                    self.w.borrow_mut().files.remove(from);
                    self.row(op, from, EffectKindView::Removed, Some(sha(&b)), None);
                }
                Ok(())
            }
            DocOp::Remove { path, expected: _ } => {
                self.w.borrow_mut().doc_ops.push(format!("remove {path}"));
                let body = self.w.borrow().files.get(path).cloned();
                if let Some(b) = body {
                    self.w.borrow_mut().files.remove(path);
                    self.row(op, path, EffectKindView::Removed, Some(sha(&b)), None);
                    if self.take_inject(Inject::AfterBody) {
                        return Err(err(
                            "partial_publication",
                            "Partially saved. body removed, record not retired",
                        ));
                    }
                }
                let rec = self
                    .w
                    .borrow()
                    .docs
                    .values()
                    .find(|d| &d.path == path && !d.retired)
                    .cloned();
                if let Some(mut d) = rec {
                    d.retired = true;
                    d.revision += 1;
                    self.write_record(op, d, false);
                }
                Ok(())
            }
        }
    }
    /// Publish through the real store and record an attested row.
    fn publish(
        &self,
        relative: &str,
        bytes: &[u8],
        observed: Option<&[u8]>,
        operation: &str,
        effects: &mut Vec<String>,
    ) -> Result<()> {
        let before = observed.map(sha);
        self.store.publish(relative, bytes, observed, effects)?;
        self.row(
            operation,
            relative,
            if observed.is_some() {
                EffectKindView::Replaced
            } else {
                EffectKindView::Created
            },
            before,
            Some(sha(bytes)),
        );
        Ok(())
    }
    /// The complete operation oracle over the in-memory journal; equal bytes never certify.
    fn effect_status(&self, operation: &str, expected: &[Expected]) -> StatusView {
        let w = self.w.borrow();
        let rows: Vec<&Row> = w.rows.iter().filter(|r| r.op == operation).collect();
        if rows.is_empty() {
            // Equal bytes without a journal row never certify ownership.
            for e in expected {
                let equal = match e.kind {
                    EffectKindView::Removed => !w.files.contains_key(&e.relative),
                    _ => w.files.get(&e.relative).map(|b| sha(b)) == e.after,
                };
                if equal {
                    return StatusView::Unknown {
                        attested: None,
                        reason: "equal bytes without attestation".into(),
                    };
                }
            }
            return StatusView::NotPublished;
        }
        let mut missing = Vec::new();
        for e in expected {
            let hit = rows
                .iter()
                .find(|r| r.relative == e.relative && r.kind == e.kind);
            match hit {
                Some(r) if r.after == e.after => {}
                Some(_) => {
                    return StatusView::Foreign {
                        reason: "operation recorded other bytes".into(),
                    };
                }
                None => missing.push(e.relative.clone()),
            }
        }
        let mut seen = BTreeMap::<(String, u8), usize>::new();
        let mut out = Vec::new();
        for (i, r) in rows.iter().enumerate() {
            let asserted = expected
                .iter()
                .any(|e| e.relative == r.relative && e.kind == r.kind && e.after == r.after);
            seen.insert((r.relative.clone(), r.kind as u8), i);
            // Superseded: a later row on the same path by another operation.
            let later = w.rows.iter().rev().find(|x| {
                x.relative == r.relative && x.op != r.op && x.call > r.call && x.after.is_some()
            });
            let superseded_into = later.map(|x| x.after.clone().unwrap_or_default());
            let git = match &r.git {
                Git::Untracked => GitView::Untracked,
                Git::Pending => GitView::Pending(format!("PG-{}", r.call)),
                Git::Held => GitView::Held(format!("PG-{}", r.call)),
                Git::Committed(c) => GitView::Committed(c.clone()),
            };
            out.push(EffectRow {
                relative: r.relative.clone(),
                kind: r.kind,
                before: r.before.clone(),
                after: r.after.clone(),
                asserted,
                superseded_into,
                git,
            });
        }
        let receipt = ReceiptView {
            rows: out,
            complete: true,
        };
        if missing.is_empty() {
            StatusView::Attested(receipt)
        } else {
            StatusView::Partial {
                attested: receipt,
                missing,
            }
        }
    }
    /// Prove items in recorded commits.
    fn verify_committed(&self, items: &[OriginalItem]) -> Result<Vec<LocatorView>> {
        let w = self.w.borrow();
        let mut located = Vec::new();
        let mut missing = Vec::new();
        for i in items {
            let hit = w.commits.get(&i.relative).and_then(|v| {
                v.iter()
                    .rev()
                    .find(|(c, s)| *s == i.sha256 && i.at.as_ref().is_none_or(|at| at == c))
            });
            match hit {
                Some((c, _)) => located.push(LocatorView {
                    relative: i.relative.clone(),
                    commit: c.clone(),
                    locator: format!("git1:{c}:{}", i.relative),
                }),
                None => missing.push(i.relative.clone()),
            }
        }
        if missing.is_empty() {
            Ok(located)
        } else {
            Err(err("not_committed", &missing.join(", ")))
        }
    }
    /// Pending and held intents.
    fn pending(&self) -> Vec<PendingIntent> {
        let w = self.w.borrow();
        let mut by: BTreeMap<u32, (String, Vec<String>)> = BTreeMap::new();
        for r in &w.rows {
            let phase = match r.git {
                Git::Held => "held",
                Git::Pending => "published",
                _ => continue,
            };
            let e = by.entry(r.call).or_insert((phase.to_owned(), Vec::new()));
            e.1.push(r.relative.clone());
        }
        by.into_iter()
            .map(|(c, (phase, paths))| PendingIntent {
                intent: format!("PG-{c}"),
                phase,
                paths,
            })
            .collect()
    }
    /// Publication events of the current call.
    fn call_events(&self) -> Vec<EventView> {
        self.w.borrow().events.clone()
    }
}
