//! Scripted storage wrapper and behavior tests of the managed-document owner.
//!
//! The wrapper runs the real [`StorePort`] (real publication primitives, journal, oracle and
//! knowledge allocator) in a disposable temporary Git repository and adds only scripted faults.
#![allow(clippy::unwrap_used, clippy::expect_used, reason = "Test assertions")]
use super::*;
use crate::persist::testing::GitFixture;
use crate::store::Tracking;

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

/// Real storage plus scripted faults.
pub(crate) struct Fake<'a> {
    /// The real filesystem port with the root write lock.
    pub inner: StorePort<'a>,
    /// Events the store already recorded before this wrapper existed.
    baseline: usize,
    /// Pending faults.
    pub faults: RefCell<Vec<Fault>>,
    /// Extra regular-file names listed below a directory, to model case-sensitive siblings on a
    /// case-insensitive disk: `(directory, name)`.
    pub listed: RefCell<Vec<(String, String)>>,
}

impl<'a> Fake<'a> {
    /// Wrap a store that has an initialized `.agent-tasks/` folder, taking its write lock for the
    /// rest of the test process (the lock file belongs to this test's own temporary root).
    pub fn new(store: &'a store::Store) -> Self {
        let guard: &'static store::LockGuard = Box::leak(Box::new(
            store
                .lock(true, &mut Vec::new())
                .unwrap()
                .expect("write lock"),
        ));
        Self {
            baseline: store.publications().len(),
            inner: StorePort::locked(store, guard),
            faults: RefCell::new(Vec::new()),
            listed: RefCell::new(Vec::new()),
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
        let mut listing = self.inner.list(dir, cap)?;
        for (d, name) in self.listed.borrow().iter().filter(|(d, _)| d == dir) {
            let _ = d;
            listing.entries.push(store::DirEntry {
                name: name.clone(),
                kind: EntryKind::File,
            });
        }
        listing.entries.sort_by(|a, b| a.name.cmp(&b.name));
        Ok(listing)
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
        real[self.baseline.min(real.len())..].to_vec()
    }
    fn status(&self, op: &OperationId, expected: &[ExpectedEffect]) -> EffectStatus {
        self.inner.status(op, expected)
    }
    fn reserve_id(&self) -> Result<String> {
        let fault = self.hit("reserve", "");
        if let Some((code, false)) = fault {
            return Err(Error::new(code, "scripted fault before any effect"));
        }
        let id = self.inner.reserve_id()?;
        match fault {
            Some((code, true)) => Err(Error::new(code, "scripted fault after the effect")),
            _ => Ok(id),
        }
    }
    fn records(&self, home: &str) -> Result<crate::references::RecordSet> {
        self.inner.records(home)
    }
    fn resolve_id(&self, id: &str) -> Result<crate::references::Resolution> {
        crate::references::prove_id(self.inner.store, self, id)
    }
}

/// Exact bytes of the knowledge allocator file, `None` before the first reservation.
pub(crate) fn allocator_bytes(dir: &tempfile::TempDir) -> Option<Vec<u8>> {
    std::fs::read(dir.path().join(".agent-tasks/knowledge.yaml")).ok()
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
    assert!(
        allocator_bytes(&dir).is_none(),
        "no identifier is reserved before the refusal"
    );
    assert!(f.events().is_empty(), "nothing was published");
    assert_eq!(
        std::fs::read(dir.path().join("docs/a.md")).unwrap(),
        b"# a\n"
    );
    // A case-insensitive disk answers the differently cased name with the same file, which is never
    // editable as that name; a case-sensitive disk simply has no such file.
    let aliased = dir.path().join("docs/A.md").exists();
    assert_eq!(
        obs(&f, "docs/A.md").state,
        if aliased {
            State::Unsupported(Unsupported::Collision)
        } else {
            State::Absent
        }
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
    let allocator_before = allocator_bytes(&dir);
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
    assert_eq!(
        allocator_bytes(&dir),
        allocator_before,
        "a move reserves nothing"
    );
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
        let reserved = allocator_bytes(&dir);
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
        assert_eq!(allocator_bytes(&dir), reserved, "resume never reserves");
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
            done.changed
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
    assert_eq!(
        resolve(&f, &crate::references::Target::Knowledge("D-001".into())).unwrap(),
        Resolution::Missing
    );
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

/// A new document reserves its identifier through the real knowledge allocator first, then publishes
/// the body and the record, all tracked; the next document gets the next number.
#[test]
fn live_store_allocates_before_publishing_and_never_recycles() {
    let (dir, st) = root();
    let guard = st.lock(true, &mut Vec::new()).unwrap().unwrap();
    let port = StorePort::locked(&st, &guard);
    let first = save_at(&port, None, "docs/one.md", b"# One\n", Some("First")).unwrap();
    assert_eq!(first.id.as_deref(), Some("DOC-001"));
    let files: Vec<_> = first
        .publications
        .iter()
        .filter(|p| p.kind != EffectKind::DirectoryCreated)
        .map(|p| p.relative.as_str())
        .collect();
    assert_eq!(
        files,
        vec![
            ".agent-tasks/knowledge.yaml",
            "docs/one.md",
            "documents/DOC-001.yaml"
        ]
    );
    assert!(
        first
            .publications
            .iter()
            .filter(|p| p.kind != EffectKind::DirectoryCreated)
            .all(|p| p.tracking == Tracking::Tracked)
    );
    let second = save_at(&port, None, "docs/two.md", b"# Two\n", Some("Second")).unwrap();
    assert_eq!(second.id.as_deref(), Some("DOC-002"));
    let counters = String::from_utf8(allocator_bytes(&dir).unwrap()).unwrap();
    assert!(counters.contains("next_document: 3"), "{counters}");
    // A port without the lock reads but cannot reserve.
    let reader = StorePort::new(&st);
    assert_eq!(reader.reserve_id().unwrap_err().code, "not_locked");
}

/// While any DOC record is unreadable the claims on a path are unknown: save, adopt, remove and a
/// move (whose source and destination are both undecidable) refuse with `partial_coverage` before
/// reserving an identifier or publishing, whether the path is unmanaged or absent.
#[test]
fn unreadable_record_refuses_every_mutation_before_effects() {
    let (dir, st) = root();
    let f = Fake::new(&st);
    native(&dir, "docs/a.md", b"# a\n");
    native(&dir, "documents/DOC-009.yaml", b"not: [valid");
    let o = obs(&f, "docs/a.md");
    assert!(!o.records_complete);
    let before = tree(dir.path());
    let r = Ref::Path(o.path.clone());
    let mut scope = Scope {
        port: &f,
        operation: None,
    };
    let to = DocPath::parse("docs/new/b.md").unwrap();
    let results = [
        save(
            &mut scope,
            Save {
                target: &r,
                purpose: Some("Purpose"),
                edit: Edit::Body(b"# b\n"),
                expected: &o.version,
                actor: None,
            },
        ),
        adopt(&mut scope, &r, Some("Purpose"), &o.version, None),
        remove(&mut scope, &r, &o.version, None),
        relocate(
            &mut scope,
            &r,
            &to,
            &o.move_basis().unwrap(),
            &absent_version(&f, &to),
            None,
        ),
        save_at(&f, None, "docs/n.md", b"x", Some("Purpose")),
    ];
    for result in results {
        assert_eq!(result.unwrap_err().code, "partial_coverage");
    }
    assert!(f.events().is_empty() && allocator_bytes(&dir).is_none());
    assert_eq!(tree(dir.path()), before);
}

/// A `documents/` home that is a file is a named gap with unknown claims, not a hard error.
#[test]
fn documents_home_that_is_a_file_is_a_named_gap() {
    let (dir, st) = root();
    let f = Fake::new(&st);
    native(&dir, "docs/a.md", b"# a\n");
    native(&dir, "documents", b"not a directory");
    let o = obs(&f, "docs/a.md");
    assert!(!o.records_complete);
    assert_eq!(o.state, State::Unmanaged);
    let inv = inventory(&f).unwrap();
    assert!(
        !inv.complete
            && inv
                .gaps
                .iter()
                .any(|g| g.reason == GapReason::NotADirectory)
    );
    assert_eq!(
        save_at(&f, None, "docs/a.md", b"# b\n", Some("P"))
            .unwrap_err()
            .code,
        "partial_coverage"
    );
}

/// Adoption refuses a NUL body exactly like a save, before any identifier is reserved.
#[test]
fn adopt_refuses_a_nul_body_before_reserving() {
    let (dir, st) = root();
    let f = Fake::new(&st);
    native(&dir, "docs/zero.md", b"a\0b");
    let o = obs(&f, "docs/zero.md");
    let err = adopt(
        &mut Scope {
            port: &f,
            operation: None,
        },
        &Ref::Path(o.path.clone()),
        Some("Purpose"),
        &o.version,
        None,
    )
    .unwrap_err();
    assert_eq!(err.code, "invalid_arguments");
    assert!(f.events().is_empty() && allocator_bytes(&dir).is_none());
}

/// Every member of a case collision is listed as `Unsupported(Collision)` and named as a gap.
#[test]
fn case_collision_members_are_listed_unsupported_rows() {
    let (dir, st) = root();
    let f = Fake::new(&st);
    native(&dir, "docs/a.md", b"# a\n");
    native(&dir, "docs/ok.md", b"# ok\n");
    f.listed.borrow_mut().push(("docs".into(), "A.md".into()));
    let inv = inventory(&f).unwrap();
    let rows: Vec<_> = inv
        .rows
        .iter()
        .map(|r| (r.path.as_str(), r.state))
        .collect();
    assert_eq!(
        rows,
        vec![
            ("docs/A.md", State::Unsupported(Unsupported::Collision)),
            ("docs/a.md", State::Unsupported(Unsupported::Collision)),
            ("docs/ok.md", State::Unmanaged),
        ]
    );
    assert!(!inv.complete);
    assert_eq!(
        inv.gaps
            .iter()
            .filter(|g| g.reason == GapReason::Collision)
            .count(),
        2
    );
}

/// A fragment link into a document with setext headings that no ATX heading answers is unprovable:
/// coverage is incomplete and the link is never reported dangling; an answered fragment stays proof.
#[test]
fn fragment_into_setext_document_is_unparsed_not_proof() {
    let (dir, st) = root();
    let f = Fake::new(&st);
    native(&dir, "docs/s.md", b"Title\n=====\n\n# Atx\n");
    native(&dir, "docs/ok.md", b"[atx](s.md#atx)\n");
    let t = crate::references::Target::Doc {
        path: DocPath::parse("docs/s.md").unwrap(),
        fragment: None,
    };
    let r = crate::references::incoming(&f, &t).unwrap();
    assert!(r.coverage.complete && r.coverage.unparsed == 0);
    native(&dir, "docs/maybe.md", b"[t](s.md#title)\n");
    let r = crate::references::incoming(&f, &t).unwrap();
    assert!(!r.coverage.complete && r.coverage.unparsed == 1);
    let i = crate::references::integrity(&f, &crate::references::Overlay::default()).unwrap();
    assert!(i.dangling_after.is_empty() && !i.coverage.complete);
}

/// Unrecognized entries of the work home are named gaps, as they are for the knowledge home.
#[test]
fn work_home_warnings_become_named_gaps() {
    let (dir, st) = root();
    let f = Fake::new(&st);
    native(&dir, "modules/stray.txt", b"x");
    let set = crate::references::load_home(&st, "work").unwrap();
    assert!(
        set.gaps
            .iter()
            .any(|g| g.reason == GapReason::UnrecognizedEntry),
        "{:?}",
        set.gaps
    );
    drop(f);
}

/// A reservation refused before its first effect passes its code through; one that fails after the
/// allocator was published (provider `allocation_changed`) is a partial publication that names the
/// step and leaves no body or record.
#[test]
fn reservation_failure_is_classed_by_its_effects() {
    let (dir, st) = root();
    let f = Fake::new(&st);
    f.fail("reserve", "", 0, "allocator", false);
    let err = save_at(&f, None, "docs/a.md", b"# a\n", Some("Purpose")).unwrap_err();
    assert_eq!(err.code, "allocator");
    assert!(f.events().is_empty() && allocator_bytes(&dir).is_none());
    f.fail("reserve", "", 0, "allocation_changed", true);
    let err = save_at(&f, None, "docs/a.md", b"# a\n", Some("Purpose")).unwrap_err();
    assert_eq!(err.code, "partial_publication");
    assert!(
        err.message.contains("reserve identifier") && err.message.contains("allocation_changed")
    );
    assert!(allocator_bytes(&dir).is_some());
    assert!(!dir.path().join("docs/a.md").exists());
    assert_eq!(obs(&f, "docs/a.md").state, State::Absent);
    // Adoption classes the same failure identically.
    native(&dir, "docs/b.md", b"# b\n");
    let o = obs(&f, "docs/b.md");
    f.fail("reserve", "", 0, "allocation_changed", true);
    let err = adopt(
        &mut Scope {
            port: &f,
            operation: None,
        },
        &Ref::Path(o.path.clone()),
        Some("Purpose"),
        &o.version,
        None,
    )
    .unwrap_err();
    assert_eq!(err.code, "partial_publication");
}

/// A document is one target however it is spelled: incoming by its DOC identifier counts the path
/// links (with or without a fragment) and incoming by its path counts the identifier mentions, while
/// a fragment query narrows to that fragment only. A compaction asking by either spelling sees all.
#[test]
fn incoming_treats_identity_and_path_as_one_document() {
    use crate::references::{Target, Via, incoming};
    let (_dir, st) = root();
    let f = Fake::new(&st);
    save_at(
        &f,
        None,
        "docs/target.md",
        b"# Target\n\n## Keep\ntext\n",
        Some("Target"),
    )
    .unwrap();
    save_at(
        &f,
        None,
        "docs/linker.md",
        b"# Linker\n\nSee [keep](target.md#keep) and DOC-001.\n",
        Some("Linker"),
    )
    .unwrap();
    let path = DocPath::parse("docs/target.md").unwrap();
    let by = |t: &Target| -> Vec<(String, Via)> {
        incoming(&f, t)
            .unwrap()
            .rows
            .iter()
            .map(|r| (r.source.id_or_path.clone(), r.via))
            .collect()
    };
    let both = vec![
        ("docs/linker.md".to_owned(), Via::MarkdownLink),
        ("docs/linker.md".to_owned(), Via::BareId),
    ];
    let whole = Target::Doc {
        path: path.clone(),
        fragment: None,
    };
    assert_eq!(by(&whole), both);
    assert_eq!(by(&Target::Knowledge("DOC-001".into())), both);
    let narrowed = Target::Doc {
        path: path.clone(),
        fragment: Some("keep".into()),
    };
    assert_eq!(
        by(&narrowed),
        vec![("docs/linker.md".to_owned(), Via::MarkdownLink)]
    );
    // A retired identity no longer stands for the path: path links address whatever lives there.
    let o = obs(&f, "docs/target.md");
    remove(
        &mut Scope {
            port: &f,
            operation: None,
        },
        &Ref::Path(path),
        &o.version,
        None,
    )
    .unwrap();
    assert_eq!(
        by(&Target::Knowledge("DOC-001".into())),
        vec![("docs/linker.md".to_owned(), Via::BareId)]
    );
}
