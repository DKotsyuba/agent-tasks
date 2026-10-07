//! Scripted storage wrapper and behavior tests of the managed-document owner.
//!
//! The wrapper runs the real [`StorePort`] (real publication primitives, real journal and oracle) in
//! a disposable temporary Git repository and adds only two things the real sources do not provide
//! yet: scripted faults, and a stand-in DOC allocator that publishes nothing.
#![allow(clippy::unwrap_used, clippy::expect_used, reason = "Test assertions")]
use super::*;
use crate::persist::testing::GitFixture;
use crate::store::{Durability, Tracking};
use std::cell::Cell;

/// One scripted fault.
pub(crate) struct Fault {
    /// `put` or `del`.
    pub on: &'static str,
    /// Substring of the relative path that triggers the fault.
    pub rel: String,
    /// Matching calls to let through before failing.
    pub skip: usize,
    /// Error code returned.
    pub code: &'static str,
    /// Perform the effect first (a visible publication whose reply failed).
    pub after_effect: bool,
}

/// Real storage plus scripted faults and a stand-in allocator.
pub(crate) struct Fake<'a> {
    /// The real filesystem port.
    pub inner: StorePort<'a>,
    /// Holds the root write lock for the test.
    _guard: store::LockGuard,
    /// Events the store already recorded before this wrapper existed.
    baseline: usize,
    /// Next DOC number to hand out.
    pub next_id: Cell<u64>,
    /// Pending faults.
    pub faults: RefCell<Vec<Fault>>,
    /// Real-event counts at which a reservation was published.
    reservations: RefCell<Vec<usize>>,
    /// Number of identifiers reserved.
    pub reserved: Cell<usize>,
}

impl<'a> Fake<'a> {
    /// Wrap a store that has an initialized `.agent-tasks/` folder, taking its write lock.
    pub fn new(store: &'a store::Store) -> Self {
        let guard = store
            .lock(true, &mut Vec::new())
            .unwrap()
            .expect("write lock");
        Self {
            baseline: store.publications().len(),
            inner: StorePort::new(store),
            _guard: guard,
            next_id: Cell::new(1),
            faults: RefCell::new(Vec::new()),
            reservations: RefCell::new(Vec::new()),
            reserved: Cell::new(0),
        }
    }

    /// Script a fault.
    pub fn fail(
        &self,
        on: &'static str,
        rel: &str,
        skip: usize,
        code: &'static str,
        after_effect: bool,
    ) {
        self.faults.borrow_mut().push(Fault {
            on,
            rel: rel.into(),
            skip,
            code,
            after_effect,
        });
    }

    /// The scripted fault matching this call, consuming it when it fires.
    fn hit(&self, on: &str, rel: &str) -> Option<(&'static str, bool)> {
        let mut faults = self.faults.borrow_mut();
        let i = faults
            .iter()
            .position(|f| f.on == on && rel.contains(&f.rel))?;
        if faults[i].skip > 0 {
            faults[i].skip -= 1;
            return None;
        }
        let f = faults.remove(i);
        Some((f.code, f.after_effect))
    }
}

impl Port for Fake<'_> {
    fn version(&self, rel: &str, bytes: Option<&[u8]>) -> String {
        self.inner.version(rel, bytes)
    }
    fn read(&self, rel: &str, cap: usize) -> Result<Read> {
        self.inner.read(rel, cap)
    }
    fn list(&self, dir: &str, cap: usize) -> Result<DirListing> {
        self.inner.list(dir, cap)
    }
    fn inventory(&self, dir: &str, prefix: &str) -> Result<store::Inventory> {
        self.inner.inventory(dir, prefix)
    }
    fn put(&self, p: Put<'_>) -> Result<()> {
        let fault = self.hit("put", p.rel);
        if let Some((code, false)) = fault {
            return Err(Error::new(code, "scripted fault before any effect"));
        }
        self.inner.put(p)?;
        match fault {
            Some((code, true)) => Err(Error::new(code, "scripted fault after the effect")),
            _ => Ok(()),
        }
    }
    fn del(&self, d: Del<'_>) -> Result<()> {
        let fault = self.hit("del", d.rel);
        if let Some((code, false)) = fault {
            return Err(Error::new(code, "scripted fault before any effect"));
        }
        self.inner.del(d)?;
        match fault {
            Some((code, true)) => Err(Error::new(code, "scripted fault after the effect")),
            _ => Ok(()),
        }
    }
    fn ensure_parents(&self, rel: &str) -> Result<()> {
        self.inner.ensure_parents(rel)
    }
    fn events(&self) -> Vec<Publication> {
        let real = self.inner.events();
        let real = &real[self.baseline.min(real.len())..];
        let marks = self.reservations.borrow();
        let mut out = Vec::new();
        for (i, event) in real.iter().enumerate() {
            out.extend(
                marks
                    .iter()
                    .filter(|m| **m == i)
                    .map(|_| reservation_event()),
            );
            out.push(event.clone());
        }
        out.extend(
            marks
                .iter()
                .filter(|m| **m >= real.len())
                .map(|_| reservation_event()),
        );
        out
    }
    fn status(&self, op: &OperationId, expected: &[ExpectedEffect]) -> EffectStatus {
        self.inner.status(op, expected)
    }
    fn reserve_id(&self) -> Result<String> {
        let n = self.next_id.get();
        self.next_id.set(n + 1);
        self.reserved.set(self.reserved.get() + 1);
        let real = self.inner.events().len().saturating_sub(self.baseline);
        self.reservations.borrow_mut().push(real);
        Ok(format!("DOC-{n:03}"))
    }
    fn records(&self, home: &str) -> Result<crate::references::RecordSet> {
        self.inner.records(home)
    }
    fn resolve_id(&self, id: &str) -> Result<crate::references::Resolution> {
        crate::references::interim_resolve(self.inner.store, self, id)
    }
}

/// The event a published allocator counter would leave.
fn reservation_event() -> Publication {
    Publication {
        relative: ".agent-tasks/knowledge.yaml".into(),
        kind: EffectKind::Replaced,
        before: None,
        after: None,
        durability: Durability::Durable,
        operation: None,
        intent: None,
        tracking: Tracking::NotApplicable,
    }
}

/// An operation identity for a test.
pub(crate) fn opid(text: &str) -> OperationId {
    OperationId::new(text).unwrap()
}

/// A disposable independent Git repository root with one commit and its store.
pub(crate) fn root() -> (tempfile::TempDir, store::Store) {
    let GitFixture { dir, store } = GitFixture::new();
    (dir, store)
}

/// Write a native file below the root, creating folders.
pub(crate) fn native(dir: &tempfile::TempDir, rel: &str, bytes: &[u8]) {
    let path = dir.path().join(rel);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, bytes).unwrap();
}

/// Observe a path.
fn obs(p: &dyn Port, path: &str) -> Observation {
    observe(p, &Ref::Path(DocPath::parse(path).unwrap())).unwrap()
}

/// Save a whole body at `path` with the current version.
fn save_at(
    p: &dyn Port,
    op: Option<&OperationId>,
    path: &str,
    body: &[u8],
    purpose: Option<&str>,
) -> Result<Receipt> {
    let r = Ref::Path(DocPath::parse(path).unwrap());
    let version = observe(p, &r)?.version;
    save(
        &mut Scope {
            port: p,
            operation: op,
        },
        Save {
            target: &r,
            purpose,
            edit: Edit::Body(body),
            expected: &version,
            actor: Some("tester"),
        },
    )
}

/// Every file below `dir` with its bytes, sorted, to prove reads change nothing.
fn tree(dir: &std::path::Path) -> Vec<(String, Vec<u8>)> {
    let mut out = Vec::new();
    fn walk(base: &std::path::Path, dir: &std::path::Path, out: &mut Vec<(String, Vec<u8>)>) {
        for e in std::fs::read_dir(dir).unwrap() {
            let e = e.unwrap();
            let p = e.path();
            let rel = p.strip_prefix(base).unwrap().to_string_lossy().into_owned();
            if rel == ".git" || rel == ".agent-tasks" {
                continue;
            }
            if p.is_dir() {
                out.push((format!("{rel}/"), Vec::new()));
                walk(base, &p, out);
            } else {
                out.push((rel, std::fs::read(&p).unwrap()));
            }
        }
    }
    walk(dir, dir, &mut out);
    out.sort();
    out
}

/// Path grammar accepts the managed namespace and refuses everything else.
#[test]
fn path_grammar_and_ref_parse() {
    for ok in [
        "README.md",
        "docs/a.md",
        "docs/a/b/c/d.md",
        "docs/FAMILY_CONTRACT.md",
        "docs/x.y.md",
    ] {
        assert_eq!(DocPath::parse(ok).unwrap().as_str(), ok);
    }
    for bad in [
        "readme.md",
        "docs/../x.md",
        "/docs/x.md",
        "docs/a b.md",
        "docs/x.MD",
        "docs/Ünï.md",
        "docs/.hidden.md",
        "docs/a/b/c/d/e.md",
        "docs/con.md",
        "docs/nul.txt.md",
        "other/x.md",
        "docs/x.md/",
        "docs//x.md",
        "docs/x.md.",
    ] {
        assert!(DocPath::parse(bad).is_err(), "{bad}");
    }
    assert_eq!(Ref::parse("DOC-001").unwrap(), Ref::Id("DOC-001".into()));
    assert!(Ref::parse("DOC-0001").is_err());
    assert_eq!(DocPath::parse("docs/a/b.md").unwrap().dir(), "docs/a");
    assert_eq!(DocPath::parse("README.md").unwrap().dir(), "");
}

/// Creating, editing, drift, adoption and retirement keep one identity and honest states.
#[test]
fn lifecycle_create_edit_drift_adopt_remove_and_reuse() {
    let (dir, st) = root();
    let f = Fake::new(&st);
    let a = obs(&f, "docs/a.md");
    assert_eq!(a.state, State::Absent);
    assert_eq!(a.version, absent_version(&f, &a.path));
    let r = save_at(
        &f,
        None,
        "docs/a.md",
        b"# A\r\nbody\r\n",
        Some("First document"),
    )
    .unwrap();
    assert_eq!(
        (r.id.as_deref(), r.state_after, r.revision),
        (Some("DOC-001"), State::Managed, Some(1))
    );
    assert!(r.publications.iter().any(|e| e.relative == "docs/a.md"));
    let v1 = obs(&f, "docs/a.md");
    assert_eq!(v1.state, State::Managed);
    // Stale version saves nothing.
    let before = tree(dir.path());
    let stale = save(
        &mut Scope {
            port: &f,
            operation: None,
        },
        Save {
            target: &Ref::Id("DOC-001".into()),
            purpose: None,
            edit: Edit::Body(b"x"),
            expected: &a.version,
            actor: None,
        },
    );
    assert_eq!(stale.unwrap_err().code, "stale");
    assert_eq!(tree(dir.path()), before);
    // Identical save is a no-op.
    let same = save_at(&f, None, "docs/a.md", b"# A\r\nbody\r\n", None).unwrap();
    assert!(!same.changed && same.publications.is_empty());
    // Native edit makes it drifted; reads never repair it.
    std::fs::write(dir.path().join("docs/a.md"), b"# A\r\nnative\r\n").unwrap();
    assert_eq!(obs(&f, "docs/a.md").state, State::Drifted);
    let drifted = obs(&f, "docs/a.md");
    let adopted = adopt(
        &mut Scope {
            port: &f,
            operation: None,
        },
        &Ref::Id("DOC-001".into()),
        None,
        &drifted.version,
        Some("tester"),
    )
    .unwrap();
    assert_eq!(
        (adopted.id.as_deref(), adopted.state_after, adopted.revision),
        (Some("DOC-001"), State::Managed, Some(2))
    );
    assert_eq!(
        std::fs::read(dir.path().join("docs/a.md")).unwrap(),
        b"# A\r\nnative\r\n"
    );
    // Retire keeps the identity resolvable and frees the path.
    let cur = obs(&f, "docs/a.md");
    let removed = remove(
        &mut Scope {
            port: &f,
            operation: None,
        },
        &Ref::Path(cur.path.clone()),
        &cur.version,
        None,
    )
    .unwrap();
    assert_eq!(
        (removed.state_after, removed.id.as_deref()),
        (State::Retired, Some("DOC-001"))
    );
    assert!(!dir.path().join("docs/a.md").exists());
    assert_eq!(
        observe(&f, &Ref::Id("DOC-001".into())).unwrap().state,
        State::Retired
    );
    let again = save_at(&f, None, "docs/a.md", b"# new\n", Some("Reused path")).unwrap();
    assert_eq!(again.id.as_deref(), Some("DOC-002"));
    assert_eq!(
        observe(&f, &Ref::Id("DOC-001".into())).unwrap().state,
        State::Retired
    );
}

/// A section replace preserves every byte outside the section, including BOM and CRLF.
#[test]
fn section_replace_through_save_preserves_untouched_bytes() {
    let (dir, st) = root();
    let f = Fake::new(&st);
    let original = "\u{feff}pre\r\n# A\r\nold\r\n# B\r\nend\r\n";
    save_at(&f, None, "docs/s.md", original.as_bytes(), Some("Sections")).unwrap();
    let o = obs(&f, "docs/s.md");
    let sel = Selector::Heading {
        text: "A".into(),
        level: None,
        occurrence: None,
    };
    let r = save(
        &mut Scope {
            port: &f,
            operation: None,
        },
        Save {
            target: &Ref::Path(o.path.clone()),
            purpose: None,
            edit: Edit::Section {
                selector: &sel,
                body: b"new\r\n",
            },
            expected: &o.version,
            actor: None,
        },
    )
    .unwrap();
    assert!(r.changed);
    assert_eq!(
        std::fs::read(dir.path().join("docs/s.md")).unwrap(),
        "\u{feff}pre\r\n# A\r\nnew\r\n# B\r\nend\r\n".as_bytes()
    );
    let o = obs(&f, "docs/s.md");
    let bad = save(
        &mut Scope {
            port: &f,
            operation: None,
        },
        Save {
            target: &Ref::Path(o.path.clone()),
            purpose: None,
            edit: Edit::Section {
                selector: &sel,
                body: b"# B\r\n",
            },
            expected: &o.version,
            actor: None,
        },
    );
    assert_eq!(bad.unwrap_err().code, "structure_change");
}

/// The body cap is exercised at its exact boundary.
#[test]
fn body_cap_boundary() {
    let (_dir, st) = root();
    let f = Fake::new(&st);
    let ok = vec![b'a'; BODY_CAP];
    save_at(&f, None, "docs/big.md", &ok, Some("At the cap")).unwrap();
    let o = obs(&f, "docs/big.md");
    assert_eq!(o.facts.as_ref().unwrap().bytes, BODY_CAP as u64);
    let over = vec![b'a'; BODY_CAP + 1];
    let events = f.events().len();
    assert_eq!(
        save_at(&f, None, "docs/over.md", &over, Some("Over"))
            .unwrap_err()
            .code,
        "capacity"
    );
    assert_eq!(f.events().len(), events);
    assert!(save_at(&f, None, "docs/zero.md", b"a\0b", Some("NUL")).is_err());
}

/// Case collisions, links and non-UTF-8 files are refused or named, never edited.
#[test]
fn collisions_links_and_unsupported_files() {
    let (dir, st) = root();
    let f = Fake::new(&st);
    native(&dir, "docs/a.md", b"# a\n");
    let err = save_at(&f, None, "docs/A.md", b"x", Some("Collides")).unwrap_err();
    assert_eq!(err.code, "collision");
    assert_eq!(
        f.reserved.get(),
        0,
        "no identifier is reserved before the refusal"
    );
    assert!(f.events().is_empty(), "nothing was published");
    assert_eq!(
        std::fs::read(dir.path().join("docs/a.md")).unwrap(),
        b"# a\n"
    );
    assert_eq!(
        obs(&f, "docs/A.md").state,
        State::Unsupported(Unsupported::Collision)
    );
    assert_eq!(obs(&f, "docs/a.md").state, State::Unmanaged);
    native(&dir, "docs/bad.md", &[0xff, 0xfe]);
    assert_eq!(
        obs(&f, "docs/bad.md").state,
        State::Unsupported(Unsupported::NotUtf8)
    );
    assert_eq!(
        save_at(&f, None, "docs/bad.md", b"x", Some("No"))
            .unwrap_err()
            .code,
        "unsupported"
    );
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(
            dir.path().join("docs/a.md"),
            dir.path().join("docs/link.md"),
        )
        .unwrap();
        assert_eq!(
            obs(&f, "docs/link.md").state,
            State::Unsupported(Unsupported::Symlink)
        );
        let inv = inventory(&f).unwrap();
        assert!(!inv.complete && inv.gaps.iter().any(|g| g.reason == GapReason::Link));
    }
    native(&dir, "docs/ünï.md", b"x");
    let inv = inventory(&f).unwrap();
    assert!(inv.gaps.iter().any(|g| g.reason == GapReason::Name));
}

/// A pre-existing documentation set stays readable and searchable without any file being created.
#[test]
fn legacy_set_reads_without_writes() {
    let (dir, st) = root();
    let f = Fake::new(&st);
    for i in 0..11 {
        native(
            &dir,
            &format!("docs/legacy{i}.md"),
            format!("# Legacy {i}\n\nplain text\n").as_bytes(),
        );
    }
    native(&dir, "README.md", b"# Home\n");
    let before = tree(dir.path());
    let inv = inventory(&f).unwrap();
    assert!(inv.complete);
    assert_eq!(inv.rows.len(), 12);
    assert!(inv.rows.iter().all(|r| r.state == State::Unmanaged));
    let c = corpus(&f).unwrap();
    assert_eq!(c.docs.len(), 12);
    assert!(c.docs[1].text.contains("plain text") && c.bytes_read > 0);
    let o = obs(&f, "docs/legacy3.md");
    let page = read(&o, &Selector::Whole, None, None, 4000).unwrap();
    assert_eq!(page.page.text, "# Legacy 3\n\nplain text\n");
    assert_eq!(tree(dir.path()), before);
}

/// Continuation is bound to the document and selection but never to the offset.
#[test]
fn read_snapshot_binds_content_not_offset() {
    let (dir, st) = root();
    let f = Fake::new(&st);
    let text = "# T\n".to_string() + &"line of text\n".repeat(40);
    native(&dir, "docs/r.md", text.as_bytes());
    let o = obs(&f, "docs/r.md");
    let p1 = read(&o, &Selector::Whole, None, None, 100).unwrap();
    assert!(p1.page.more);
    let p2 = read(
        &o,
        &Selector::Whole,
        Some(p1.page.end),
        Some(&p1.snapshot),
        100,
    )
    .unwrap();
    assert_eq!(p2.snapshot, p1.snapshot);
    assert_eq!(
        read(&o, &Selector::Whole, Some(p1.page.end), None, 100)
            .unwrap_err()
            .code,
        "stale"
    );
    std::fs::write(dir.path().join("docs/r.md"), format!("{text}more")).unwrap();
    let changed = obs(&f, "docs/r.md");
    let e = read(
        &changed,
        &Selector::Whole,
        Some(p1.page.end),
        Some(&p1.snapshot),
        100,
    )
    .unwrap_err();
    assert_eq!(e.code, "stale");
    assert_ne!(
        read(&o, &Selector::Ordinal(0), None, None, 100)
            .unwrap()
            .snapshot,
        p1.snapshot
    );
}

/// A record write that fails after the body is partial, named, and recoverable by adoption.
#[test]
fn partial_publication_is_disclosed_and_adopt_recovers() {
    let (dir, st) = root();
    let f = Fake::new(&st);
    f.fail("put", "documents/DOC-001.yaml", 0, "io", false);
    let err = save_at(&f, None, "docs/p.md", b"# P\n", Some("Partial")).unwrap_err();
    assert_eq!(err.code, "partial_publication");
    assert!(
        err.message.starts_with("Partially saved.") && err.message.contains("created docs/p.md")
    );
    assert!(!err.message.contains("No work saved"));
    assert!(dir.path().join("docs/p.md").exists());
    let o = obs(&f, "docs/p.md");
    assert_eq!(o.state, State::Unmanaged);
    let a = adopt(
        &mut Scope {
            port: &f,
            operation: None,
        },
        &Ref::Path(o.path.clone()),
        Some("Recovered"),
        &o.version,
        None,
    )
    .unwrap();
    assert_eq!(a.state_after, State::Managed);
    // A failure before any effect passes through unchanged.
    f.fail("put", "docs/q.md", 0, "io", false);
    let gap = save_at(&f, None, "docs/q.md", b"x", Some("Q")).unwrap_err();
    assert_eq!(
        gap.code, "partial_publication",
        "a published reservation makes the failure partial"
    );
    assert!(gap.message.contains("knowledge.yaml") && gap.message.contains("(io)"));
    assert!(!dir.path().join("docs/q.md").exists());
    // A final-step sync failure is not partial.
    f.fail("put", "documents/", 0, "durability_unknown", true);
    let unk = save_at(&f, None, "docs/r.md", b"# R\n", Some("R")).unwrap_err();
    assert_eq!(unk.code, "durability_unknown");
}

/// A prepared move: source `docs/old.md` managed under DOC-001, destination absent.
fn prepared(f: &Fake<'_>) -> (Observation, MoveBasis, String) {
    save_at(
        f,
        None,
        "docs/old.md",
        b"# Moving\r\nbody\r\n",
        Some("To move"),
    )
    .unwrap();
    let src = obs(f, "docs/old.md");
    let basis = src.move_basis().unwrap();
    let to = DocPath::parse("docs/new/moved.md").unwrap();
    let expected_to = absent_version(f, &to);
    (src, basis, expected_to)
}

/// A normal move keeps the identity and the exact bytes in the stated order.
#[test]
fn relocate_normal_keeps_identity_and_order() {
    let (dir, st) = root();
    let f = Fake::new(&st);
    let (src, basis, expected_to) = prepared(&f);
    let to = DocPath::parse("docs/new/moved.md").unwrap();
    let r = relocate(
        &mut Scope {
            port: &f,
            operation: None,
        },
        &Ref::Path(src.path.clone()),
        &to,
        &basis,
        &expected_to,
        None,
    )
    .unwrap();
    assert_eq!(
        (r.id.as_deref(), r.state_after, r.revision),
        (Some("DOC-001"), State::Managed, Some(2))
    );
    let order: Vec<_> = r
        .publications
        .iter()
        .filter(|e| e.kind != EffectKind::DirectoryCreated)
        .map(|e| (e.kind, e.relative.as_str()))
        .collect();
    assert_eq!(
        order,
        vec![
            (EffectKind::Created, "docs/new/moved.md"),
            (EffectKind::Replaced, "documents/DOC-001.yaml"),
            (EffectKind::Removed, "docs/old.md")
        ]
    );
    assert_eq!(
        std::fs::read(dir.path().join("docs/new/moved.md")).unwrap(),
        b"# Moving\r\nbody\r\n"
    );
    assert!(!dir.path().join("docs/old.md").exists());
    assert_eq!(f.reserved.get(), 1);
}

/// Without an operation identity an occupied destination always refuses and nothing is overwritten.
#[test]
fn relocate_ordinary_recovery_windows_and_occupied_destination() {
    let (dir, st) = root();
    let f = Fake::new(&st);
    let (src, basis, expected_to) = prepared(&f);
    let to = DocPath::parse("docs/new/moved.md").unwrap();
    // W1: crash after the destination body.
    f.fail("put", "documents/DOC-001.yaml", 0, "io", false);
    let e = relocate(
        &mut Scope {
            port: &f,
            operation: None,
        },
        &Ref::Path(src.path.clone()),
        &to,
        &basis,
        &expected_to,
        None,
    )
    .unwrap_err();
    assert_eq!(e.code, "partial_publication");
    let copy = obs(&f, "docs/new/moved.md");
    assert_eq!(copy.state, State::Unmanaged);
    // A repeat onto the occupied destination refuses and keeps both files.
    let again = relocate(
        &mut Scope {
            port: &f,
            operation: None,
        },
        &Ref::Path(src.path.clone()),
        &to,
        &basis,
        &expected_to,
        None,
    )
    .unwrap_err();
    assert_eq!(again.code, "stale");
    // Ordinary recovery: remove the unclaimed copy, then repeat.
    remove(
        &mut Scope {
            port: &f,
            operation: None,
        },
        &Ref::Path(to.clone()),
        &copy.version,
        None,
    )
    .unwrap();
    let expected_to = absent_version(&f, &to);
    f.fail("del", "docs/old.md", 0, "io", false);
    let w2 = relocate(
        &mut Scope {
            port: &f,
            operation: None,
        },
        &Ref::Path(src.path.clone()),
        &to,
        &basis,
        &expected_to,
        None,
    )
    .unwrap_err();
    assert_eq!(w2.code, "partial_publication");
    // W2: the record moved; the leftover at the source is unmanaged and removable without retiring.
    let moved = obs(&f, "docs/new/moved.md");
    assert_eq!(
        (moved.state, moved.id.as_deref()),
        (State::Managed, Some("DOC-001"))
    );
    let left = obs(&f, "docs/old.md");
    assert_eq!(left.state, State::Unmanaged);
    remove(
        &mut Scope {
            port: &f,
            operation: None,
        },
        &Ref::Path(left.path.clone()),
        &left.version,
        None,
    )
    .unwrap();
    assert_eq!(obs(&f, "docs/new/moved.md").state, State::Managed);
    assert!(
        dir.path().join("docs/new/moved.md").exists() && !dir.path().join("docs/old.md").exists()
    );
}

/// Identity-bearing resume completes W1 and W2 from the oracle without a new identity.
#[test]
fn relocate_identity_resume_windows() {
    for window in [1, 2] {
        let (dir, st) = root();
        let f = Fake::new(&st);
        let (src, basis, expected_to) = prepared(&f);
        let to = DocPath::parse("docs/new/moved.md").unwrap();
        let op_id = opid("move:1");
        let op = &op_id;
        // The attested publications need the journal double, so the first run is required-attested.
        if window == 1 {
            f.fail("put", "documents/DOC-001.yaml", 0, "io", false);
        } else {
            f.fail("del", "docs/old.md", 0, "io", false);
        }
        let e = relocate(
            &mut Scope {
                port: &f,
                operation: Some(op),
            },
            &Ref::Path(src.path.clone()),
            &to,
            &basis,
            &expected_to,
            None,
        )
        .unwrap_err();
        assert_eq!(e.code, "partial_publication", "window {window}");
        let reserved = f.reserved.get();
        let r = relocate(
            &mut Scope {
                port: &f,
                operation: Some(op),
            },
            &Ref::Path(src.path.clone()),
            &to,
            &basis,
            &expected_to,
            None,
        )
        .unwrap();
        assert_eq!(
            (r.id.as_deref(), r.state_after),
            (Some("DOC-001"), State::Managed),
            "window {window}"
        );
        assert_eq!(f.reserved.get(), reserved, "resume never reserves");
        assert!(!dir.path().join("docs/old.md").exists());
        assert_eq!(
            std::fs::read(dir.path().join("docs/new/moved.md")).unwrap(),
            b"# Moving\r\nbody\r\n"
        );
        let kinds: Vec<_> = r.publications.iter().map(|e| e.kind).collect();
        assert!(
            !kinds.contains(&EffectKind::Created),
            "resume never recreates the copy"
        );
        // W3: a repeat after completion publishes nothing.
        let done = relocate(
            &mut Scope {
                port: &f,
                operation: Some(op),
            },
            &Ref::Path(src.path.clone()),
            &to,
            &basis,
            &expected_to,
            None,
        )
        .unwrap();
        assert!(
            !done.changed
                && done.warnings.contains(&Warning::AlreadyApplied)
                && done.publications.is_empty()
        );
    }
}

/// An equal copy that the oracle cannot attest is never adopted, overwritten or completed.
#[test]
fn relocate_refuses_unattested_equal_copy() {
    let (dir, st) = root();
    let f = Fake::new(&st);
    let (src, basis, expected_to) = prepared(&f);
    let to = DocPath::parse("docs/new/moved.md").unwrap();
    native(&dir, "docs/new/moved.md", b"# Moving\r\nbody\r\n");
    let tree_before = tree(dir.path());
    let err = relocate(
        &mut Scope {
            port: &f,
            operation: Some(&opid("move:2")),
        },
        &Ref::Path(src.path.clone()),
        &to,
        &basis,
        &expected_to,
        None,
    )
    .unwrap_err();
    assert_eq!(err.code, "resume_unproven");
    assert_eq!(tree(dir.path()), tree_before);
    // Another operation identity's copy is equally unproven.
    std::fs::remove_file(dir.path().join("docs/new/moved.md")).unwrap();
    let other = opid("other:1");
    st.publish_with(
        store::Publish {
            relative: "docs/new/moved.md",
            bytes: b"# Moving\r\nbody\r\n",
            observed: None,
            cap: BODY_CAP,
            operation: Some(&other),
            attest: Attest::Required,
        },
        &mut Vec::new(),
    )
    .unwrap();
    let err = relocate(
        &mut Scope {
            port: &f,
            operation: Some(&opid("move:2")),
        },
        &Ref::Path(src.path.clone()),
        &to,
        &basis,
        &expected_to,
        None,
    )
    .unwrap_err();
    assert_eq!(err.code, "resume_unproven");
}

/// Incoming scans are complete only when every validated source was read.
#[test]
fn incoming_links_and_validated_coverage() {
    let (dir, st) = root();
    let f = Fake::new(&st);
    native(&dir, "docs/target.md", b"# Target\n## Part\n");
    native(&dir, "docs/a.md", b"see [t](target.md#part) and DOC-009\n");
    native(&dir, "README.md", b"[home](docs/target.md)\n");
    let t = crate::references::Target::Doc {
        path: DocPath::parse("docs/target.md").unwrap(),
        fragment: None,
    };
    let r = crate::references::incoming(&f, &t).unwrap();
    let got: Vec<_> = r
        .rows
        .iter()
        .map(|i| (i.source.id_or_path.as_str(), i.count))
        .collect();
    assert_eq!(got, vec![("README.md", 1), ("docs/a.md", 1)]);
    assert!(r.coverage.complete);
    // An unparsed construct or an unreadable record makes coverage incomplete, never zero.
    native(&dir, "docs/h.md", b"<a href=\"x\">x</a>\n");
    assert!(
        !crate::references::incoming(&f, &t)
            .unwrap()
            .coverage
            .complete
    );
    std::fs::remove_file(dir.path().join("docs/h.md")).unwrap();
    native(&dir, "modules/M-009.yaml", b"not: [valid");
    let r = crate::references::incoming(&f, &t).unwrap();
    assert!(
        !r.coverage.complete
            && r.coverage
                .gaps
                .iter()
                .any(|g| g.reason == GapReason::Unreadable)
    );
    // Compaction proposals are outside every scan.
    std::fs::remove_file(dir.path().join("modules/M-009.yaml")).unwrap();
    native(
        &dir,
        "compactions/CP-001.yaml",
        b"mentions: docs/target.md\n",
    );
    let r = crate::references::incoming(&f, &t).unwrap();
    assert!(r.coverage.complete && r.rows.len() == 2);
}

/// Integrity reports references made dangling by a removal or a heading change and models moves.
#[test]
fn integrity_overlay_reports_introduced_dangling_only() {
    let (dir, st) = root();
    let f = Fake::new(&st);
    native(&dir, "docs/target.md", b"# Target\n## Part\n");
    native(
        &dir,
        "docs/a.md",
        b"[t](target.md#part) [gone](missing.md)\n",
    );
    let target = DocPath::parse("docs/target.md").unwrap();
    let i = crate::references::integrity(
        &f,
        &crate::references::Overlay {
            remove: std::slice::from_ref(&target),
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(i.dangling_before, 1);
    assert_eq!(i.introduced.len(), 1);
    assert!(i.coverage.complete);
    let edited = [(target.clone(), b"# Target\n## Other\n".to_vec())];
    let i = crate::references::integrity(
        &f,
        &crate::references::Overlay {
            put: &edited,
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(i.introduced.len(), 1);
    let moved = [(target.clone(), DocPath::parse("docs/elsewhere.md").unwrap())];
    let i = crate::references::integrity(
        &f,
        &crate::references::Overlay {
            moves: &moved,
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(
        i.introduced.len(),
        1,
        "links by path are not rewritten by a plain move"
    );
}

/// Resolution proves work children and retired documents instead of guessing.
#[test]
fn resolve_documents_and_retired_ids() {
    let (dir, st) = root();
    let f = Fake::new(&st);
    native(&dir, "docs/x.md", b"# Real heading\n");
    let doc = |p: &str, frag: Option<&str>| crate::references::Target::Doc {
        path: DocPath::parse(p).unwrap(),
        fragment: frag.map(str::to_owned),
    };
    use crate::references::{Resolution, resolve};
    assert_eq!(
        resolve(&f, &doc("docs/x.md", None)).unwrap(),
        Resolution::Found
    );
    assert_eq!(
        resolve(&f, &doc("docs/x.md", Some("real-heading"))).unwrap(),
        Resolution::Found
    );
    assert_eq!(
        resolve(&f, &doc("docs/x.md", Some("nope"))).unwrap(),
        Resolution::MissingSection
    );
    assert_eq!(
        resolve(&f, &doc("docs/y.md", None)).unwrap(),
        Resolution::Missing
    );
    save_at(&f, None, "docs/z.md", b"# z\n", Some("Z")).unwrap();
    let id = crate::references::Target::Knowledge("DOC-001".into());
    assert_eq!(resolve(&f, &id).unwrap(), Resolution::Found);
    let z = obs(&f, "docs/z.md");
    remove(
        &mut Scope {
            port: &f,
            operation: None,
        },
        &Ref::Path(z.path.clone()),
        &z.version,
        None,
    )
    .unwrap();
    assert_eq!(resolve(&f, &id).unwrap(), Resolution::Retired);
    assert_eq!(
        resolve(&f, &crate::references::Target::Knowledge("DOC-077".into())).unwrap(),
        Resolution::Missing
    );
    assert!(matches!(
        resolve(&f, &crate::references::Target::Knowledge("D-001".into())).unwrap(),
        Resolution::Unknown(_)
    ));
    assert!(crate::references::valid_reference(&f, "docs/x.md#real-heading").is_ok());
    assert!(crate::references::valid_reference(&f, "docs/x.md#nope").is_err());
    assert!(crate::references::valid_reference(&f, "M-001").is_err());
}

/// A leftover edited after the stop, a wrong record digest and a stale basis each refuse resume.
#[test]
fn relocate_resume_refuses_changed_state_and_mismatched_basis() {
    let (dir, st) = root();
    let f = Fake::new(&st);
    let (src, basis, expected_to) = prepared(&f);
    let to = DocPath::parse("docs/new/moved.md").unwrap();
    f.fail("del", "docs/old.md", 0, "io", false);
    let op_id = opid("move:3");
    let op = Some(&op_id);
    let e = relocate(
        &mut Scope {
            port: &f,
            operation: op,
        },
        &Ref::Path(src.path.clone()),
        &to,
        &basis,
        &expected_to,
        None,
    )
    .unwrap_err();
    assert_eq!(e.code, "partial_publication");
    // The leftover is edited natively after the stop.
    std::fs::write(dir.path().join("docs/old.md"), b"# Edited\n").unwrap();
    let before = tree(dir.path());
    let e = relocate(
        &mut Scope {
            port: &f,
            operation: op,
        },
        &Ref::Path(src.path.clone()),
        &to,
        &basis,
        &expected_to,
        None,
    )
    .unwrap_err();
    assert_eq!(e.code, "stale");
    assert_eq!(tree(dir.path()), before, "a refused resume changes nothing");
    std::fs::write(dir.path().join("docs/old.md"), b"# Moving\r\nbody\r\n").unwrap();
    // A basis built from different record bytes does not match the attested record effect.
    let wrong = MoveBasis {
        record_sha256: Some(sha256_hex(b"other")),
        ..basis.clone()
    };
    let e = relocate(
        &mut Scope {
            port: &f,
            operation: op,
        },
        &Ref::Path(src.path.clone()),
        &to,
        &wrong,
        &expected_to,
        None,
    )
    .unwrap_err();
    assert_eq!(e.code, "resume_unproven");
    assert_eq!(
        tree(dir.path()),
        before
            .iter()
            .map(|(p, b)| (
                p.clone(),
                if p == "docs/old.md" {
                    b"# Moving\r\nbody\r\n".to_vec()
                } else {
                    b.clone()
                }
            ))
            .collect::<Vec<_>>()
    );
    // The exact basis still resumes: the record is not retired and the leftover is removed.
    let ok = relocate(
        &mut Scope {
            port: &f,
            operation: op,
        },
        &Ref::Path(src.path.clone()),
        &to,
        &basis,
        &expected_to,
        None,
    )
    .unwrap();
    assert_eq!(
        (ok.id.as_deref(), ok.state_after),
        (Some("DOC-001"), State::Managed)
    );
}

/// Hostile bytes survive a save, a read at every budget and the escaped wire exactly.
#[test]
fn raw_bytes_round_trip_through_save_and_pages() {
    let (dir, st) = root();
    let f = Fake::new(&st);
    let mut seed = 0x2545_f491_4f6c_dd1du64;
    let pool: Vec<char> = "ab é€😀\\\r\n\t\u{1b}\u{7f}\u{85}\u{feff}\u{2028}\u{202e}#`~<>[]()-=!"
        .chars()
        .collect();
    for round in 0..40 {
        let mut text = String::new();
        for _ in 0..(round * 7 + 3) {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            text.push(pool[(seed % pool.len() as u64) as usize]);
        }
        let path = format!("docs/fuzz{round}.md");
        save_at(&f, None, &path, text.as_bytes(), Some("Fuzz")).unwrap();
        assert_eq!(
            std::fs::read(dir.path().join(&path)).unwrap(),
            text.as_bytes()
        );
        let o = obs(&f, &path);
        for budget in [16usize, 17, 33, 100] {
            let mut at = None;
            let mut snap: Option<String> = None;
            let mut out = Vec::new();
            loop {
                let page = read(&o, &Selector::Whole, at, snap.as_deref(), budget).unwrap();
                out.extend(markdown::decode(page.page.wire, &page.page.text).unwrap());
                if !page.page.more {
                    break;
                }
                at = Some(page.page.end);
                snap = Some(page.snapshot);
            }
            assert_eq!(out, text.as_bytes(), "round {round} budget {budget}");
        }
    }
}

/// The real store, journal and oracle move an unmanaged document with tracked, attested events.
#[test]
fn live_store_relocates_with_tracked_attested_events() {
    let (dir, st) = root();
    let _guard = st.lock(true, &mut Vec::new()).unwrap().unwrap();
    let port = StorePort::new(&st);
    native(&dir, "docs/u.md", b"# U\r\nbody\r\n");
    let src = obs(&port, "docs/u.md");
    let basis = src.move_basis().unwrap();
    let to = DocPath::parse("docs/v/w.md").unwrap();
    let id = opid("live:1");
    let r = relocate(
        &mut Scope {
            port: &port,
            operation: Some(&id),
        },
        &Ref::Path(src.path.clone()),
        &to,
        &basis,
        &absent_version(&port, &to),
        None,
    )
    .unwrap();
    assert_eq!((r.id.as_deref(), r.state_after), (None, State::Unmanaged));
    let files: Vec<_> = r
        .publications
        .iter()
        .filter(|p| p.kind != EffectKind::DirectoryCreated)
        .collect();
    assert_eq!(files.len(), 2);
    assert!(files.iter().all(|p| p.tracking == Tracking::Tracked));
    assert_eq!(
        std::fs::read(dir.path().join("docs/v/w.md")).unwrap(),
        b"# U\r\nbody\r\n"
    );
    assert!(!dir.path().join("docs/u.md").exists());
    let expect = [ExpectedEffect {
        relative: "docs/v/w.md".into(),
        kind: EffectKind::Created,
        after_sha256: Some(basis.body_sha256.clone()),
    }];
    assert!(matches!(
        port.status(&id, &expect),
        EffectStatus::Attested(_)
    ));
}

/// Until the knowledge allocator source exists, a save that needs an identifier refuses before any
/// effect and says so; an existing document is unaffected.
#[test]
fn live_store_reports_the_missing_allocator_without_effects() {
    let (dir, st) = root();
    let _guard = st.lock(true, &mut Vec::new()).unwrap().unwrap();
    let port = StorePort::new(&st);
    let before = tree(dir.path());
    let err = save_at(&port, None, "docs/new.md", b"# N\n", Some("Needs an id")).unwrap_err();
    assert_eq!(err.code, "allocator");
    assert_eq!(tree(dir.path()), before);
    assert!(port.events().is_empty());
}
