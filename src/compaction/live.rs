//! The live [`crate::compaction::env::Env`]: every method forwards to the real document, reference,
//! knowledge, store and persistence providers.
//!
//! Publication, the complete operation oracle, committed-byte proofs, pending intents and the current
//! call's typed events are the persistence provider's functions; document observation, the whole-body
//! save, relocate, remove and adopt are the document owner's, with the reference owner's incoming scan
//! and integrity preview, and the knowledge owner's single allocation version and reservation. This
//! adapter only reshapes their answers into the domain's decision views. It owns no second journal, no
//! second oracle, no metadata serializer and no Git logic.
use super::env::{
    DocFacts, DocOp, DocState, EffectKindView, EffectRow, Env, EventView, Expected, GitView,
    IncomingFacts, IncomingRow, IntegrityFacts, LocatorView, OriginalItem, OutlineFacts,
    OverlayFacts, PendingIntent, ReceiptView, RecordFacts, SectionFact, SourceKind, StatusView,
    TrackingView,
};
use super::record::{BLOB_CAP, SectionAddr, sha256_hex};
use crate::{
    documents::{self, DocPath, Edit, MoveBasis, Port, Ref, Save, Scope, State, StorePort},
    knowledge::{self, Prefix},
    persist::{
        self, EffectGit, EffectReceipt, EffectStatus, ExpectedEffect, Phase,
        locator::{self, Item},
    },
    references::{self, Target},
    store::{Attest, EffectKind, Error, LockGuard, OperationId, Publish, Result, Store, Tracking},
};

/// The environment of one locked compaction call over the real providers.
pub struct LiveEnv<'a> {
    /// The request-local store whose write lock the caller holds.
    store: &'a Store,
    /// The root write lock acquired through `store` in this request.
    guard: &'a LockGuard,
    /// Scripted faults for the composed tests; absent from every shipping build.
    #[cfg(test)]
    pub(super) faults: std::cell::RefCell<Vec<Fault>>,
}

/// One scripted fault of a composed test: it fires on the matching `put` or `del` of the real port.
#[cfg(test)]
pub(super) struct Fault {
    /// `put` or `del`.
    pub(super) on: &'static str,
    /// Substring of the relative path that triggers the fault.
    pub(super) rel: String,
    /// Matching calls to let through before failing.
    pub(super) skip: usize,
    /// Error code returned.
    pub(super) code: &'static str,
    /// Perform the real effect first: a visible publication whose reply was lost.
    pub(super) after_effect: bool,
}

impl<'a> LiveEnv<'a> {
    /// Bind the environment to the root write lock acquired through `store` in this request.
    ///
    /// # Errors
    /// `not_locked` when `guard` is not that lock; nothing is read or written.
    pub fn new(store: &'a Store, guard: &'a LockGuard) -> Result<Self> {
        if guard.is_for(store) {
            Ok(Self {
                store,
                guard,
                #[cfg(test)]
                faults: Default::default(),
            })
        } else {
            Err(Error::new(
                "not_locked",
                "The root write lock was not acquired through this Store in this call.",
            ))
        }
    }

    /// The document owner's storage seam with the knowledge owner's allocator and record loaders.
    pub(super) fn port(&self) -> LivePort<'_> {
        LivePort {
            inner: StorePort::new(self.store),
            guard: self.guard,
            #[cfg(test)]
            faults: &self.faults,
        }
    }
}

/// The document owner's [`Port`] over the real store, completed with the knowledge owner's real
/// functions where the stock `StorePort` still reports its interim loaders as unavailable.
///
/// Reservation is the single knowledge allocator under the held lock; typed record homes and typed
/// identifier existence come from the knowledge loaders, so incoming coverage never degrades to an
/// invented gap. Everything else is the stock port, unchanged.
pub(super) struct LivePort<'a> {
    /// The stock port over the request store.
    pub(super) inner: StorePort<'a>,
    /// The root write lock acquired through the same store.
    pub(super) guard: &'a LockGuard,
    /// The environment's scripted faults, composed tests only.
    #[cfg(test)]
    faults: &'a std::cell::RefCell<Vec<Fault>>,
}

impl LivePort<'_> {
    /// The scripted fault matching this call, consuming it when it fires.
    #[cfg(test)]
    fn injected(&self, on: &str, rel: &str) -> Option<(&'static str, bool)> {
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

    /// Move the human effect lines collected by the stock port into the caller's list.
    pub(super) fn drain_into(&self, effects: &mut Vec<String>) {
        effects.append(&mut self.inner.fx.borrow_mut());
    }
}

/// Typed record kind of a structured home, when the home is a knowledge home.
fn knowledge_kind(home: &str) -> Option<knowledge::Kind> {
    [
        knowledge::Kind::Decision,
        knowledge::Kind::Runbook,
        knowledge::Kind::Research,
        knowledge::Kind::Checklist,
    ]
    .into_iter()
    .find(|k| k.prefix().directory() == home)
}

impl Port for LivePort<'_> {
    /// Root and path bound version of exact bytes or of absence, from the stock port.
    fn version(&self, rel: &str, bytes: Option<&[u8]>) -> String {
        self.inner.version(rel, bytes)
    }
    /// Capped exact read of one owned file, from the stock port.
    fn read(&self, rel: &str, cap: usize) -> Result<documents::Read> {
        self.inner.read(rel, cap)
    }
    /// Sorted bounded directory listing, from the stock port.
    fn list(&self, dir: &str, cap: usize) -> Result<crate::store::DirListing> {
        self.inner.list(dir, cap)
    }
    /// Bounded identifier inventory of one record directory, from the stock port.
    fn inventory(&self, dir: &str, prefix: &str) -> Result<crate::store::Inventory> {
        self.inner.inventory(dir, prefix)
    }
    /// Guarded create or replace through the stock port; a scripted fault may intervene in tests.
    fn put(&self, put: documents::Put<'_>) -> Result<()> {
        #[cfg(test)]
        let fault = self.injected("put", put.rel);
        #[cfg(test)]
        if let Some((code, false)) = fault {
            return Err(Error::new(code, "scripted fault before any effect"));
        }
        self.inner.put(put)?;
        #[cfg(test)]
        if let Some((code, true)) = fault {
            return Err(Error::new(code, "scripted fault after the effect"));
        }
        Ok(())
    }
    /// Guarded removal through the stock port; a scripted fault may intervene in tests.
    fn del(&self, del: documents::Del<'_>) -> Result<()> {
        #[cfg(test)]
        let fault = self.injected("del", del.rel);
        #[cfg(test)]
        if let Some((code, false)) = fault {
            return Err(Error::new(code, "scripted fault before any effect"));
        }
        self.inner.del(del)?;
        #[cfg(test)]
        if let Some((code, true)) = fault {
            return Err(Error::new(code, "scripted fault after the effect"));
        }
        Ok(())
    }
    /// Create missing owned parent directories through the stock port.
    fn ensure_parents(&self, rel: &str) -> Result<()> {
        self.inner.ensure_parents(rel)
    }
    /// Every typed publication event of this request, from the stock port.
    fn events(&self) -> Vec<crate::store::Publication> {
        self.inner.events()
    }
    /// The persistence oracle's answer for one operation, from the stock port.
    fn status(&self, op: &OperationId, expected: &[ExpectedEffect]) -> EffectStatus {
        self.inner.status(op, expected)
    }

    /// Reserve the next `DOC-` identifier at a freshly observed allocation version.
    fn reserve_id(&self) -> Result<String> {
        let version = knowledge::allocation_version(self.inner.store)?;
        knowledge::reserve(
            self.inner.store,
            self.guard,
            Prefix::Document,
            &version,
            &mut self.inner.fx.borrow_mut(),
        )
    }

    /// Work and project records through their loaders, typed homes through the knowledge scan.
    fn records(&self, home: &str) -> Result<references::RecordSet> {
        let Some(kind) = knowledge_kind(home) else {
            return self.inner.records(home);
        };
        let scan = knowledge::scan(self.inner.store, Some(kind))?;
        let mut set = references::RecordSet {
            complete: scan.complete,
            ..Default::default()
        };
        for snapshot in scan.records {
            let id = snapshot.value.id().to_owned();
            set.files.push(references::RecordFile {
                rel: format!("{home}/{id}.yaml"),
                source: references::Source {
                    kind: references::SourceKind::Knowledge,
                    id_or_path: id,
                },
                bytes: snapshot.bytes,
            });
        }
        for name in scan.unreadable.into_iter().chain(scan.warnings) {
            set.gaps.push(documents::Gap {
                what: documents::quote(&name),
                reason: documents::GapReason::Unreadable,
            });
        }
        Ok(set)
    }

    /// Typed identifiers (and checklist items) by the knowledge loaders; the rest by the stock port.
    fn resolve_id(&self, id: &str) -> Result<references::Resolution> {
        let Ok(parsed) = knowledge::parse_id(id) else {
            return self.inner.resolve_id(id);
        };
        if matches!(parsed.prefix, Prefix::Document | Prefix::Compaction) {
            return self.inner.resolve_id(id);
        }
        let found = if parsed.item.is_some() {
            match knowledge::resolve_child(self.inner.store, &parsed)? {
                knowledge::Child::Found { .. } => references::Resolution::Found,
                _ => references::Resolution::Missing,
            }
        } else {
            match knowledge::load(self.inner.store, id) {
                Ok(_) => references::Resolution::Found,
                Err(e) if e.code == "not_found" => references::Resolution::Missing,
                Err(e) => {
                    references::Resolution::Unknown(format!("record unreadable ({})", e.code))
                }
            }
        };
        Ok(found)
    }
}

/// Run one document operation through the document owner's mutation under `operation`.
///
/// The operation identity makes every publication of the call required-attested; the caller holds
/// the root write lock and `port` reaches the real store. Errors are the document owner's own.
pub(super) fn run_doc_op(port: &dyn Port, operation: &str, actor: &str, op: &DocOp) -> Result<()> {
    let operation = OperationId::new(operation)?;
    let mut scope = Scope {
        port,
        operation: Some(&operation),
    };
    let result = match op {
        DocOp::Save {
            path,
            body,
            purpose,
            expected,
        } => documents::save(
            &mut scope,
            Save {
                target: &Ref::Path(DocPath::parse(path)?),
                purpose: purpose.as_deref(),
                edit: Edit::Body(body),
                expected,
                actor: Some(actor),
            },
        ),
        DocOp::Relocate {
            from,
            to,
            expected: _,
            expected_to,
            basis,
        } => documents::relocate(
            &mut scope,
            &Ref::Path(DocPath::parse(from)?),
            &DocPath::parse(to)?,
            &MoveBasis {
                version: basis.version.clone(),
                body_sha256: basis.body_sha256.clone(),
                record_sha256: basis.record_sha256.clone(),
                id: basis.id.clone(),
            },
            expected_to,
            Some(actor),
        ),
        DocOp::Remove { path, expected } => documents::remove(
            &mut scope,
            &Ref::Path(DocPath::parse(path)?),
            expected,
            Some(actor),
        ),
        DocOp::Adopt {
            path,
            purpose,
            expected,
        } => documents::adopt(
            &mut scope,
            &Ref::Path(DocPath::parse(path)?),
            purpose.as_deref(),
            expected,
            Some(actor),
        ),
    };
    result.map(|_| ())
}

/// One document observation reduced to what compaction decides on.
fn doc_facts(obs: documents::Observation) -> DocFacts {
    let state = match obs.state {
        State::Managed => DocState::Managed,
        State::Unmanaged => DocState::Unmanaged,
        State::Absent => DocState::Absent,
        State::Retired => DocState::Retired,
        State::Unsupported(why) => DocState::Other(format!("unsupported ({why:?})")),
        other => DocState::Other(other.label().to_owned()),
    };
    DocFacts {
        path: obs.path.as_str().to_owned(),
        retired: obs.state == State::Retired,
        state,
        version: obs.version,
        body: obs.body,
        record: obs.record.map(|r| RecordFacts {
            sha256: sha256_hex(&r.bytes),
            len: r.bytes.len() as u64,
            path: r.rel,
            id: r.record.id,
            bound_path: r.record.path,
            body_sha256: r.record.body_sha256,
            revision: r.record.revision,
        }),
    }
}

/// Kind of a referring source in the domain's vocabulary.
fn source_kind(kind: references::SourceKind) -> SourceKind {
    match kind {
        references::SourceKind::Markdown => SourceKind::Markdown,
        references::SourceKind::Readme => SourceKind::Readme,
        references::SourceKind::Work => SourceKind::Work,
        references::SourceKind::Knowledge => SourceKind::Knowledge,
        references::SourceKind::DocMetadata => SourceKind::DocMetadata,
        references::SourceKind::Project => SourceKind::Project,
    }
}

/// Named coverage gaps as bounded text lines.
fn gap_lines(coverage: &references::Coverage) -> Vec<String> {
    coverage
        .gaps
        .iter()
        .map(|g| format!("{}: {:?}", g.what, g.reason))
        .collect()
}

/// Effect kind of a file effect; directory effects are not operation rows.
fn kind_view(kind: EffectKind) -> Option<EffectKindView> {
    match kind {
        EffectKind::Created => Some(EffectKindView::Created),
        EffectKind::Replaced => Some(EffectKindView::Replaced),
        EffectKind::Removed => Some(EffectKindView::Removed),
        EffectKind::DirectoryCreated | EffectKind::DirectoryExisting => None,
    }
}

/// Lowercase label of a reconciled intent phase, as the apply path decides on it.
fn phase_label(phase: Phase) -> &'static str {
    match phase {
        Phase::Prepared => "prepared",
        Phase::Published => "published",
        Phase::Held => "held",
        Phase::Committing => "committing",
        Phase::Unknown => "unknown",
        Phase::Drifted => "drifted",
    }
}

/// Git state of one attested effect. Anything unresolved or edited is `Unknown`, never certifiable.
fn git_view(git: EffectGit) -> GitView {
    match git {
        EffectGit::Untracked => GitView::Untracked,
        EffectGit::Committed { commit } => GitView::Committed(commit),
        EffectGit::Pending(r) => match r.phase {
            Phase::Published => GitView::Pending(r.intent),
            Phase::Held => GitView::Held(r.intent),
            Phase::Prepared | Phase::Committing | Phase::Unknown | Phase::Drifted => {
                GitView::Unknown(r.intent)
            }
        },
        EffectGit::Unknown(r) => GitView::Unknown(r.intent),
    }
}

/// The attested set of one operation reshaped into rows.
fn receipt_view(receipt: EffectReceipt) -> ReceiptView {
    let rows = receipt
        .paths
        .into_iter()
        .filter_map(|p| {
            Some(EffectRow {
                kind: kind_view(p.kind)?,
                relative: p.relative,
                before: p.before_sha256,
                after: p.after_sha256,
                asserted: p.asserted,
                superseded_into: p.superseded_into,
                git: git_view(p.git),
            })
        })
        .collect();
    ReceiptView {
        rows,
        complete: receipt.complete,
    }
}

impl Env for LiveEnv<'_> {
    /// The locked request-local store.
    fn store(&self) -> &Store {
        self.store
    }

    /// The single knowledge allocation version over the six homes and the allocator file.
    fn allocation_version(&self) -> Result<String> {
        knowledge::allocation_version(self.store)
    }

    /// Reserve the next `CP-` identifier at the expected allocation version under the held lock.
    ///
    /// # Errors
    /// `stale`, `inventory` and `allocator` as the allocation owner defines them.
    fn reserve(&self, expected: &str, effects: &mut Vec<String>) -> Result<String> {
        knowledge::reserve(
            self.store,
            self.guard,
            Prefix::Compaction,
            expected,
            effects,
        )
    }

    /// Whether `doc_id` is a canonical `DOC-` identifier already below the allocator's next number
    /// in a fully understood allocation.
    fn allocation_valid(&self, doc_id: &str) -> Result<bool> {
        let Ok(id) = knowledge::parse_id(doc_id) else {
            return Ok(false);
        };
        if id.prefix != Prefix::Document || id.item.is_some() {
            return Ok(false);
        }
        let allocation = knowledge::observe_allocation(self.store)?;
        Ok(allocation.complete
            && allocation
                .counters
                .is_some_and(|c| id.number > 0 && id.number < c.document))
    }

    /// Create the missing owned parents through the store; each created directory is disclosed.
    fn ensure_parents(&self, relative: &str, effects: &mut Vec<String>) -> Result<()> {
        self.store.ensure_parents(relative, effects).map(|_| ())
    }

    /// Observe one managed path through the document owner.
    fn observe(&self, path: &str) -> Result<DocFacts> {
        let port = self.port();
        documents::observe(&port, &Ref::Path(DocPath::parse(path)?)).map(doc_facts)
    }

    /// Observe a document by `DOC-` identifier through the document owner, retired records included.
    fn observe_id(&self, id: &str) -> Result<DocFacts> {
        let port = self.port();
        documents::observe(&port, &Ref::Id(id.to_owned())).map(doc_facts)
    }

    /// Outline exact bytes with the Markdown owner's dialect: the preamble when nonempty, then every
    /// heading section including its nested sections.
    fn outline(&self, bytes: &[u8]) -> Result<OutlineFacts> {
        let outline = crate::markdown::outline(bytes)?;
        let mut sections = Vec::new();
        if outline.preamble_end > 0 {
            let part = bytes[..outline.preamble_end].to_vec();
            sections.push(SectionFact {
                addr: SectionAddr::Preamble,
                sha256: sha256_hex(&part),
                bytes: part,
            });
        }
        for h in &outline.headings {
            let part = bytes[h.start..h.end].to_vec();
            sections.push(SectionFact {
                addr: SectionAddr::Heading {
                    ordinal: h.ordinal,
                    level: h.level,
                    occurrence: h.occurrence,
                    text_sha256: sha256_hex(h.text.as_bytes()),
                },
                sha256: sha256_hex(&part),
                bytes: part,
            });
        }
        Ok(OutlineFacts {
            complete: outline.complete,
            setext_candidates: outline.setext_candidates,
            slugs: outline.headings.iter().map(|h| h.slug.clone()).collect(),
            sections,
        })
    }

    /// Complete bounded incoming scan of one target document; any gap makes the answer unknown.
    fn incoming(&self, path: &str) -> Result<IncomingFacts> {
        let port = self.port();
        let target = Target::Doc {
            path: DocPath::parse(path)?,
            fragment: None,
        };
        let result = references::incoming(&port, &target)?;
        Ok(IncomingFacts {
            complete: result.coverage.complete,
            gaps: gap_lines(&result.coverage),
            rows: result
                .rows
                .into_iter()
                .map(|r| IncomingRow {
                    target: path.to_owned(),
                    kind: source_kind(r.source.kind),
                    source: r.source.id_or_path,
                    via: format!("{:?}", r.via),
                    count: r.count,
                    fragments: r.fragments,
                })
                .collect(),
        })
    }

    /// Whole-root reference integrity with the overlay applied in memory.
    fn integrity(&self, overlay: &OverlayFacts) -> Result<IntegrityFacts> {
        let port = self.port();
        let put = overlay
            .put
            .iter()
            .map(|(p, b)| Ok((DocPath::parse(p)?, b.clone())))
            .collect::<Result<Vec<_>>>()?;
        let remove = overlay
            .remove
            .iter()
            .map(|p| DocPath::parse(p))
            .collect::<Result<Vec<_>>>()?;
        let moves = overlay
            .moves
            .iter()
            .map(|(a, b)| Ok((DocPath::parse(a)?, DocPath::parse(b)?)))
            .collect::<Result<Vec<_>>>()?;
        let result = references::integrity(
            &port,
            &references::Overlay {
                put: &put,
                remove: &remove,
                moves: &moves,
            },
        )?;
        Ok(IntegrityFacts {
            complete: result.coverage.complete,
            gaps: gap_lines(&result.coverage),
            introduced: result
                .introduced
                .iter()
                .map(|d| format!("{} -> {}", d.source.id_or_path, d.target.canonical()))
                .collect(),
        })
    }

    /// One document operation in its own scope under the action's deterministic operation identity.
    ///
    /// Every publication of the call is required-attested by that identity, so an interrupted
    /// operation is resumable only through the persistence oracle.
    fn doc_op(
        &self,
        operation: &str,
        actor: &str,
        op: &DocOp,
        effects: &mut Vec<String>,
    ) -> Result<()> {
        let port = self.port();
        let result = run_doc_op(&port, operation, actor, op);
        port.drain_into(effects);
        result
    }

    /// Publish with required journal attestation under the deterministic operation identity.
    ///
    /// # Errors
    /// `invalid` for an unusable identity, plus the store errors of `publish_with`
    /// (`stale`, `operation_repeat`, `attestation_unavailable`, `attestation_unknown`, ...).
    fn publish(
        &self,
        relative: &str,
        bytes: &[u8],
        observed: Option<&[u8]>,
        operation: &str,
        effects: &mut Vec<String>,
    ) -> Result<()> {
        let operation = OperationId::new(operation)?;
        self.store
            .publish_with(
                Publish {
                    relative,
                    bytes,
                    observed,
                    cap: BLOB_CAP.max(bytes.len()),
                    operation: Some(&operation),
                    attest: Attest::Required,
                },
                effects,
            )
            .map(|_| ())
    }

    /// The provider's complete operation oracle; an unusable identity is `Unknown`, never published.
    fn effect_status(&self, operation: &str, expected: &[Expected]) -> StatusView {
        let Ok(operation) = OperationId::new(operation) else {
            return StatusView::Unknown {
                attested: None,
                reason: "The operation identity is not valid.".to_owned(),
            };
        };
        let expected: Vec<ExpectedEffect> = expected
            .iter()
            .map(|e| ExpectedEffect {
                relative: e.relative.clone(),
                kind: match e.kind {
                    EffectKindView::Created => EffectKind::Created,
                    EffectKindView::Replaced => EffectKind::Replaced,
                    EffectKindView::Removed => EffectKind::Removed,
                },
                after_sha256: e.after.clone(),
            })
            .collect();
        match persist::effect_status(self.store, &operation, &expected) {
            EffectStatus::Attested(receipt) => StatusView::Attested(receipt_view(receipt)),
            EffectStatus::Partial { attested, missing } => StatusView::Partial {
                attested: receipt_view(attested),
                missing,
            },
            EffectStatus::NotPublished => StatusView::NotPublished,
            EffectStatus::Unknown {
                attested, reason, ..
            } => StatusView::Unknown {
                attested: Some(receipt_view(attested)),
                reason: format!("{reason:?}"),
            },
            EffectStatus::Foreign { reason } => StatusView::Foreign {
                reason: format!("{reason:?}"),
            },
        }
    }

    /// Prove every item exists byte identical in a commit reachable from the attached HEAD.
    ///
    /// # Errors
    /// `not_committed` naming every unproven path, `locator_too_long` for an unencodable proof.
    fn verify_committed(&self, items: &[OriginalItem]) -> Result<Vec<LocatorView>> {
        let items: Vec<Item<'_>> = items
            .iter()
            .map(|i| Item {
                relative: &i.relative,
                sha256: &i.sha256,
                len: i.len,
                at: i.at.as_deref(),
            })
            .collect();
        locator::verify_committed(self.store, &items)?
            .into_iter()
            .map(|l| {
                Ok(LocatorView {
                    locator: l.encode()?,
                    commit: l.commit,
                    relative: l.relative,
                })
            })
            .collect()
    }

    /// Up to sixteen pending intents with their paths; unreadable evidence yields no rows.
    fn pending(&self) -> Vec<PendingIntent> {
        persist::pending(self.store)
            .refs
            .into_iter()
            .map(|r| PendingIntent {
                paths: persist::intent_view(self.store, &r.intent)
                    .map(|v| v.effects.into_iter().map(|e| e.relative).collect())
                    .unwrap_or_default(),
                phase: phase_label(r.phase).to_owned(),
                intent: r.intent,
            })
            .collect()
    }

    /// The typed publication events of this request, an unconfirmed entry counting as untracked.
    fn call_events(&self) -> Vec<EventView> {
        self.store
            .publications()
            .into_iter()
            .map(|e| EventView {
                durable: e.durability == crate::store::Durability::Durable,
                tracking: match e.tracking {
                    Tracking::Tracked => TrackingView::Tracked,
                    Tracking::NotApplicable => TrackingView::NotApplicable,
                    Tracking::Untracked(_) | Tracking::UnknownAfterPublication => {
                        TrackingView::Untracked
                    }
                },
                relative: e.relative,
                operation: e.operation,
                intent: e.intent,
            })
            .collect()
    }
}

/// Real-repository controls of the live adapter against the shipped store and persistence provider.
///
/// Every control uses a disposable isolated Git repository; none touches a configured documentation
/// root. They prove the adapter's reshaping and the production policy for the compaction families, not
/// document, allocation or MCP composition.
#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "Explicit isolated fixture assertions"
)]
mod tests {
    use super::*;
    use crate::{
        compaction::{
            fake::FakeEnv,
            ops,
            record::{self, CpState},
            validate::{ActionIn, ProposalIn, SectionIn, SourceIn},
        },
        persist::{
            EventClass, GitOutcome,
            policy::{Event, EventOutcome},
            production_policy, settle,
            testing::GitFixture,
        },
    };

    /// Publish `bytes` at `path` as the given operation through the live adapter.
    fn put(env: &LiveEnv<'_>, path: &str, bytes: &[u8], op: &str) -> Result<()> {
        env.publish(path, bytes, None, op, &mut Vec::new())
    }

    /// Expected creation of exactly `bytes` at `path`.
    fn created(path: &str, bytes: &[u8]) -> Expected {
        Expected {
            relative: path.to_owned(),
            kind: EffectKindView::Created,
            after: Some(record::sha256_hex(bytes)),
        }
    }

    /// A required-attested publication is visible in the oracle and in the call's events; the same
    /// operation cannot publish the path twice, a different operation sees nothing, and a wrong
    /// assertion for the recorded operation is foreign.
    #[test]
    fn publish_is_attested_and_the_oracle_maps_rows() {
        let f = GitFixture::new();
        let guard = f.lock();
        let env = LiveEnv::new(&f.store, &guard).unwrap();
        env.ensure_parents("compactions/CP-001/r1/A-01.md", &mut Vec::new())
            .unwrap();
        let op = "cp:CP-001:r1:stage:A-01";
        put(&env, "compactions/CP-001/r1/A-01.md", b"alpha\n", op).unwrap();
        let events = env.call_events();
        let file = events.last().unwrap();
        assert_eq!(
            (file.tracking, file.durable, file.operation.as_deref()),
            (TrackingView::Tracked, true, Some(op))
        );
        assert!(
            events[..events.len() - 1]
                .iter()
                .all(|e| e.tracking == TrackingView::NotApplicable)
        );
        let want = created("compactions/CP-001/r1/A-01.md", b"alpha\n");
        let StatusView::Attested(view) = env.effect_status(op, std::slice::from_ref(&want)) else {
            panic!("an attested publication must be attested");
        };
        assert!(view.complete);
        let row = &view.rows[0];
        assert_eq!(
            (row.kind, row.asserted, row.after.as_deref()),
            (EffectKindView::Created, true, want.after.as_deref())
        );
        assert!(matches!(row.git, GitView::Pending(_) | GitView::Held(_)));
        let again = env
            .publish(
                "compactions/CP-001/r1/A-01.md",
                b"beta\n",
                Some(b"alpha\n"),
                op,
                &mut Vec::new(),
            )
            .unwrap_err();
        assert_eq!(again.code, "operation_repeat");
        assert!(matches!(
            env.effect_status("cp:CP-001:r1:stage:A-02", &[]),
            StatusView::NotPublished
        ));
        let wrong = created("compactions/CP-001/r1/A-01.md", b"other\n");
        assert!(matches!(
            env.effect_status(op, &[wrong]),
            StatusView::Foreign { .. }
        ));
    }

    /// Equal bytes written by a foreign editor never become attestation; a bad identity is Unknown.
    #[test]
    fn equal_foreign_bytes_are_unknown_not_attested() {
        let f = GitFixture::new();
        let guard = f.lock();
        let env = LiveEnv::new(&f.store, &guard).unwrap();
        env.ensure_parents("compactions/CP-001/r1/A-01.md", &mut Vec::new())
            .unwrap();
        std::fs::write(
            f.dir.path().join("compactions/CP-001/r1/A-01.md"),
            b"alpha\n",
        )
        .unwrap();
        let want = created("compactions/CP-001/r1/A-01.md", b"alpha\n");
        let status = env.effect_status("cp:CP-001:r1:stage:A-01", &[want]);
        assert!(matches!(status, StatusView::Unknown { .. }), "{status:?}");
        assert!(matches!(
            env.effect_status("bad identity!", &[]),
            StatusView::Unknown { attested: None, .. }
        ));
    }

    /// Bytes are provable only after a real commit holds them; the proof carries a bounded locator.
    #[test]
    fn verify_committed_needs_a_real_commit() {
        let f = GitFixture::new();
        let guard = f.lock();
        let env = LiveEnv::new(&f.store, &guard).unwrap();
        env.ensure_parents("docs/a.md", &mut Vec::new()).unwrap();
        put(&env, "docs/a.md", b"one\n", "seed:a").unwrap();
        let item = OriginalItem {
            relative: "docs/a.md".into(),
            sha256: record::sha256_hex(b"one\n"),
            len: 4,
            at: None,
        };
        let err = env
            .verify_committed(std::slice::from_ref(&item))
            .unwrap_err();
        assert_eq!(err.code, "not_committed");
        assert!(err.message.contains("docs/a.md"));
        f.git(&["add", "--", "docs/a.md"]);
        f.git(&["commit", "--quiet", "-m", "docs: a"]);
        let proofs = env.verify_committed(std::slice::from_ref(&item)).unwrap();
        assert_eq!(proofs.len(), 1);
        assert!(proofs[0].locator.starts_with("git1:") && proofs[0].locator.len() <= 256);
        assert_eq!(proofs[0].commit, f.git(&["rev-parse", "HEAD"]));
        let at = OriginalItem {
            at: Some(proofs[0].commit.clone()),
            ..item.clone()
        };
        assert!(env.verify_committed(&[at]).is_ok());
        let bogus = OriginalItem {
            at: Some("0".repeat(40)),
            ..item
        };
        assert_eq!(
            env.verify_committed(&[bogus]).unwrap_err().code,
            "not_committed"
        );
    }

    /// Seed a proposed record through the domain double and copy its exact bytes into the live store.
    fn seed_record(env: &LiveEnv<'_>) -> String {
        let fake = FakeEnv::new();
        fake.seed("docs/a.md", "Intro\n## S1\none\n## S2\ntwo\n", true);
        let doc = fake.observe("docs/a.md").unwrap();
        let candidate = "Intro\n## S1\none\n";
        let sections: Vec<SectionIn> = crate::compaction::fake::outline_of(&doc.body.unwrap())
            .sections
            .into_iter()
            .map(|s| SectionIn {
                path: "docs/a.md".into(),
                section: s.addr,
                disposition: if candidate.contains(std::str::from_utf8(&s.bytes).unwrap()) {
                    record::Disposition::Kept {
                        action: "A-01".into(),
                    }
                } else {
                    record::Disposition::Dropped {
                        kind: record::DropKind::Obsolete,
                        reason: "no longer needed".into(),
                    }
                },
            })
            .collect();
        let input = ProposalIn {
            title: "Trim".into(),
            sources: vec![SourceIn {
                path: "docs/a.md".into(),
                version: fake.observe("docs/a.md").unwrap().version,
            }],
            actions: vec![ActionIn {
                id: "A-01".into(),
                kind: record::ActionKind::Replace,
                path: "docs/a.md".into(),
                from: None,
                base_version: None,
                content: Some(candidate.into()),
                purpose: None,
                reason: "compaction".into(),
                absorbed_into: vec![],
            }],
            sections,
            preservation: vec![],
        };
        let v = fake.allocation_version().unwrap();
        fake.start_call();
        ops::propose(
            &fake,
            "author",
            &v,
            "key-live-0001",
            &input,
            &mut Vec::new(),
        )
        .unwrap();
        fake.finish_call(true);
        let bytes = fake
            .store()
            .bytes(&record::record_path("CP-001"))
            .unwrap()
            .unwrap();
        env.ensure_parents(&record::record_path("CP-001"), &mut Vec::new())
            .unwrap();
        put(env, &record::record_path("CP-001"), &bytes, "seed:cp-001").unwrap();
        "CP-001".into()
    }

    /// The real record chain: a withdraw replaces the record through required-attested publication and
    /// the production policy then commits exactly that call's whole intent as a compaction event.
    #[test]
    fn withdraw_publishes_and_the_production_policy_commits() {
        let f = GitFixture::new();
        let guard = f.lock();
        let env = LiveEnv::new(&f.store, &guard).unwrap();
        let cp = seed_record(&env);
        // The seeding call is its own settled request in production; settle it first.
        let seeded = settle(
            &f.store,
            &guard,
            &Event {
                class: EventClass::CompactionPropose,
                refs: vec![cp.clone()],
                operation: None,
                outcome: EventOutcome::Success,
            },
            production_policy(),
        );
        assert_eq!(seeded.outcome, GitOutcome::Committed, "{seeded:?}");
        drop(guard);
        let store = Store::from_root(f.dir.path()).unwrap();
        let guard = store.lock(true, &mut Vec::new()).unwrap().unwrap();
        let env = LiveEnv::new(&store, &guard).unwrap();
        let snap = crate::compaction::read_cp(&store, &cp).unwrap();
        let mut fx = Vec::new();
        let out = ops::withdraw(
            &env,
            "author",
            &snap.version,
            &cp,
            "not needed",
            false,
            &mut fx,
        )
        .unwrap();
        assert!(out.changed && out.state == CpState::Withdrawn);
        let events = env.call_events();
        assert!(
            events
                .iter()
                .any(|e| e.relative == record::record_path(&cp)
                    && e.tracking == TrackingView::Tracked)
        );
        let receipt = settle(
            &store,
            &guard,
            &Event {
                class: EventClass::CompactionWithdraw,
                refs: vec![cp.clone()],
                operation: None,
                outcome: EventOutcome::Success,
            },
            production_policy(),
        );
        assert_eq!(receipt.outcome, GitOutcome::Committed, "{receipt:?}");
        assert_eq!(receipt.paths, vec![record::record_path(&cp)]);
        assert!(
            f.git(&["log", "-1", "--format=%B"])
                .contains("compaction-withdraw")
        );
        assert!(env.pending().is_empty());
        let after = crate::compaction::read_cp(&store, &cp).unwrap();
        assert_eq!(after.value.state, CpState::Withdrawn);
    }

    /// Attestation is required: outside an independent repository nothing can be journaled, so the
    /// publication is refused before any effect instead of proceeding untracked.
    #[test]
    fn required_attestation_refuses_without_a_repository() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::from_root(dir.path()).unwrap();
        store.prepare(&mut Vec::new()).unwrap();
        let guard = store.lock(true, &mut Vec::new()).unwrap().unwrap();
        let env = LiveEnv::new(&store, &guard).unwrap();
        let err = put(&env, "a.md", b"x", "cp:CP-001:r1:rec:1").unwrap_err();
        assert_eq!(err.code, "attestation_unavailable");
        assert!(!dir.path().join("a.md").exists());
        store
            .publish_with(
                crate::store::Publish {
                    relative: "b.md",
                    bytes: b"y",
                    observed: None,
                    cap: 16,
                    operation: None,
                    attest: crate::store::Attest::Optional,
                },
                &mut Vec::new(),
            )
            .unwrap();
        assert_eq!(
            env.call_events().last().unwrap().tracking,
            TrackingView::Untracked
        );
    }

    /// A call that ended partial is held, never committed; a later request sees its effect as held,
    /// and a replacement is reported with its own kind and before digest.
    #[test]
    fn partial_calls_surface_as_held_and_replace_keeps_its_kind() {
        let f = GitFixture::new();
        let guard = f.lock();
        let env = LiveEnv::new(&f.store, &guard).unwrap();
        env.ensure_parents("docs/a.md", &mut Vec::new()).unwrap();
        put(&env, "docs/a.md", b"one\n", "cp:CP-001:r1:rec:1").unwrap();
        let partial = settle(
            &f.store,
            &guard,
            &Event {
                class: EventClass::CompactionApply,
                refs: vec![],
                operation: None,
                outcome: EventOutcome::Partial,
            },
            production_policy(),
        );
        assert_ne!(partial.outcome, GitOutcome::Committed, "{partial:?}");
        drop(guard);
        let store = Store::from_root(f.dir.path()).unwrap();
        let guard = store.lock(true, &mut Vec::new()).unwrap().unwrap();
        let env = LiveEnv::new(&store, &guard).unwrap();
        let want = created("docs/a.md", b"one\n");
        let StatusView::Attested(view) = env.effect_status("cp:CP-001:r1:rec:1", &[want]) else {
            panic!("the earlier partial call must stay attested");
        };
        assert!(
            matches!(view.rows[0].git, GitView::Held(_)),
            "{:?}",
            view.rows
        );
        assert!(
            env.pending()
                .iter()
                .any(|p| p.phase == "held" && p.paths == ["docs/a.md"])
        );
        env.publish(
            "docs/a.md",
            b"two\n",
            Some(b"one\n"),
            "cp:CP-001:r1:rec:2",
            &mut Vec::new(),
        )
        .unwrap();
        let replaced = Expected {
            relative: "docs/a.md".into(),
            kind: EffectKindView::Replaced,
            after: Some(record::sha256_hex(b"two\n")),
        };
        let StatusView::Attested(view) = env.effect_status("cp:CP-001:r1:rec:2", &[replaced])
        else {
            panic!("the replacement must be attested");
        };
        assert_eq!(
            (view.rows[0].kind, view.rows[0].before.as_deref()),
            (
                EffectKindView::Replaced,
                Some(record::sha256_hex(b"one\n").as_str())
            )
        );
    }

    /// A guard that is not this store's lock is refused outright.
    #[test]
    fn foreign_guards_are_refused() {
        let f = GitFixture::new();
        let other = GitFixture::new();
        let foreign = other.lock();
        assert_eq!(
            LiveEnv::new(&f.store, &foreign).err().unwrap().code,
            "not_locked"
        );
        assert!(f.store.publications().is_empty());
    }
}
