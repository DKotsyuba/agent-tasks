//! Complete, read-only archive collection and explicit preservation actions.
//! Unknown native limits are supplied by measured gates; collection never deletes anything.
use crate::{
    model::{Fault, Kind, Meta, Result, Status, Work, require},
    records::{Store, child_id},
    rules,
};
use pulldown_cmark::{Event, Parser, Tag};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet, VecDeque};

/// One explicit refusal; items without known ownership/kind never enter a deletion manifest.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ArchiveBlocker {
    /// Stable machine-readable cause, never a success fallback.
    pub code: String,
    /// Exact source identity, or the Epic for archive-wide bounds.
    pub item: String,
    /// Human-readable reason preserving the observed mismatch.
    pub detail: String,
}
/// One completely collected managed native Issue; vectors are sorted deterministically by identity.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ArchiveItem {
    /// Concrete managed kind; unmanaged native children are blockers instead.
    pub kind: Kind,
    /// Full selected native Issue fields, including reactions and source timestamps.
    pub native: Value,
    /// Exact workflow record, including reports, reviews and frozen membership.
    pub workflow: Meta,
    /// Fully paginated root comments and recursively paginated replies.
    pub comments: Vec<Value>,
    /// Native Documents with complete content and recursively paginated comments.
    pub documents: Vec<Value>,
    /// All native attachment records, excluding only the workflow attachment represented above.
    pub files: Vec<Value>,
    /// Union of outgoing and incoming relations, deduplicated by native identity.
    pub relations: Vec<Value>,
    /// Available native change records and actors; no revision-history promise is made.
    pub history: Vec<Value>,
}
/// A readable byte asset captured from an attachment or an embedded source link.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ArchiveAsset {
    /// Original owning Issue UUID.
    pub issue_id: String,
    /// Original human identifier for historical provenance.
    pub identifier: String,
    /// Original attachment UUID or embedded URL, used for deterministic copy identity.
    pub origin: String,
    /// Canonical protected upload URL; credentials are never sent to another host.
    pub url: String,
    /// Captured byte length within the product file cap.
    pub size: u64,
    /// SHA-256 digest of the exact captured bytes.
    pub digest: String,
    /// Native or inferred filename used for the surviving artifact.
    pub filename: String,
    /// Content type returned by the source asset transport.
    pub content_type: String,
    /// Existing artifact intent preserved exactly, or a compatible new intent for an embedded upload.
    pub artifact: Value,
}
/// Deterministic complete source export, with blockers retained for a read-only preview.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ArchiveSet {
    /// The permanent Epic and its complete original details.
    pub epic: ArchiveItem,
    /// Managed native/recorded descendants sorted by identifier then UUID.
    pub items: Vec<ArchiveItem>,
    /// Readable assets that need byte preservation before deletion.
    pub assets: Vec<ArchiveAsset>,
    /// Explicit causes preventing writes/deletion; no caller may treat a nonempty list as eligible.
    pub blockers: Vec<ArchiveBlocker>,
}

/// Empirically measured native/rendered bounds; absent values always block application.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct ArchiveLimits {
    /// Largest verified single-document source byte count; no splitting/truncation is allowed.
    pub archive_max_bytes: Option<usize>,
    /// Largest verified complete H3 section body byte count within the rendered response budget.
    pub section_max_bytes: Option<usize>,
}
/// Optional facts captured once by the caller; rendering itself never reads a clock.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ArchiveFacts {
    /// Applied timestamp captured in durable intent, absent for a pure preview.
    pub applied_at: Option<String>,
}

/// Read the native nullable Issue trash flag after an exact-ID object read.
/// Only a present `true` confirms trash. Present `false` or `null` means untrashed;
/// an omitted field or another type is `INCOMPLETE_DATA`, never a deletion confirmation.
pub fn is_trashed(issue: &Value) -> Result<bool> {
    match issue.get("trashed") {
        Some(Value::Bool(true)) => Ok(true),
        Some(Value::Bool(false) | Value::Null) => Ok(false),
        _ => Err(Fault::new("INCOMPLETE_DATA", "Native Issue trash flag is missing or invalid")),
    }
}

/// One caller-journaled preservation action; no future compaction receipt type is required.
/// Signed upload URLs/headers exist only inside a running prepare attempt and are never serialized.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum PreservationAction {
    /// Read/recheck source bytes, reserve and PUT one temporary copy; no canonical attachment is published.
    /// A cold retry first reconciles the deterministic attachment; a new explicit prepare attempt
    /// may leave an orphan temporary upload after an uncertain reservation/PUT response.
    PrepareCopy {
        /// Permanent managed Epic receiving the canonical artifact.
        target_issue: String,
        /// Deterministic canonical attachment identity, independent of temporary reservations.
        attachment_id: String,
        /// Captured source provenance, byte digest and unchanged artifact intent.
        asset: ArchiveAsset,
    },
    /// Publish an already prepared canonical asset URL using one deterministic attachment mutation.
    AttachCopy {
        /// Permanent managed Epic receiving the canonical artifact.
        target_issue: String,
        /// Same deterministic attachment identity as the prepare action.
        attachment_id: String,
        /// Captured source provenance, byte digest and unchanged artifact intent.
        asset: ArchiveAsset,
        /// Canonical asset URL only; signed upload transport information must never enter this field.
        asset_url: String,
    },
    /// Reparent one unchanged native Document to the permanent Epic in one mutation.
    ReparentDocument {
        /// Permanent managed Epic receiving the native Document.
        target_issue: String,
        /// Original full Document snapshot including source ownership/content/visibility and updatedAt.
        document: Value,
    },
}
/// Confirmed action outcome with an optional next action to journal before executing.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PreservationEffect {
    /// Canonical identity/provenance/readback facts, excluding signed upload transport information.
    pub confirmed: Value,
    /// Follow-up action, absent when the canonical native object is confirmed.
    pub next: Option<PreservationAction>,
}
/// Derive the canonical artifact identity from its complete originating Issue/source identity.
fn copy_id(epic: &str, asset: &ArchiveAsset) -> String {
    child_id(
        epic,
        &format!("compact-file:{}:{}", asset.issue_id, asset.origin),
    )
}
/// Generate deterministic actions only for a complete eligible export; Epic-owned objects survive already.
/// Each prepare effect produces its attach follow-up. The caller must persist the current action
/// before executing it and persist its result before advancing; this function performs no writes.
pub fn preservation_plan(set: &ArchiveSet) -> Result<Vec<PreservationAction>> {
    require(
        set.blockers.is_empty(),
        "PRESERVATION_BLOCKED",
        "Archive has explicit source blockers",
    )?;
    let epic = identity(&set.epic.native)?;
    let mut actions = vec![];
    for asset in &set.assets {
        if asset.issue_id != epic {
            actions.push(PreservationAction::PrepareCopy {
                target_issue: epic.into(),
                attachment_id: copy_id(epic, asset),
                asset: asset.clone(),
            });
        }
    }
    for source in &set.items {
        for document in &source.documents {
            require(
                document["issue"]["id"] == source.native["id"],
                "SOURCE_CHANGED",
                "Document ownership differs from collected Issue",
            )?;
            actions.push(PreservationAction::ReparentDocument {
                target_issue: epic.into(),
                document: document.clone(),
            });
        }
    }
    Ok(actions)
}
/// Preserve the existing artifact intent and add self-contained source provenance without secrets.
fn copy_metadata(asset: &ArchiveAsset) -> Value {
    json!({"artifact":asset.artifact,"compacted_from":{"issue_id":asset.issue_id,"identifier":asset.identifier,"source_id":asset.origin,"original_url":asset.url}})
}
/// Check one byte download against the captured source identity; changed bytes cannot be published.
async fn verify_bytes(store: &Store, url: &str, asset: &ArchiveAsset) -> Result<Vec<u8>> {
    let (bytes, _) = store.linear.get_asset(url).await?;
    require(
        bytes.len() as u64 == asset.size && format!("{:x}", Sha256::digest(&bytes)) == asset.digest,
        "ASSET_CHANGED",
        "Asset size/digest differs from collected source",
    )?;
    Ok(bytes)
}
/// Compare original Document content/visibility/title without treating a confirmed reparent timestamp as drift.
fn same_document(source: &Value, current: &Value) -> bool {
    ["id", "title", "archivedAt", "hiddenAt"]
        .iter()
        .all(|k| source[*k] == current[*k])
        && crate::records::markdown_equivalent(
            source["content"].as_str().unwrap_or(""),
            current["content"].as_str().unwrap_or(""),
        )
}
/// Require a known managed Epic target in the same native Project as the originating Issue.
/// No authority is inferred from an arbitrary action ID, and this helper never writes.
async fn preservation_target(store: &Store, target: &str, origin: &str) -> Result<()> {
    let epic = store.work(target).await?;
    require(
        epic.managed()?.kind == Kind::Epic,
        "INVALID_INPUT",
        "Preservation target must be a managed Epic",
    )?;
    let source = store.work(origin).await?;
    require(
        source.native["project"]["id"] == epic.native["project"]["id"],
        "FOREIGN_ITEM",
        "Preservation origin belongs to another Project",
    )?;
    Ok(())
}
/// Reconcile a deterministic canonical artifact or unchanged reparent by exact-ID reads.
/// `None` means no canonical effect is confirmed yet; partial/auth/unknown responses propagate.
/// A confirmed copy always has native ownership, exact provenance/intent and verified bytes.
/// Temporary upload reservations have no invented lookup or durable signed-slot representation.
pub async fn reconcile_preservation(
    store: &Store,
    action: &PreservationAction,
) -> Result<Option<PreservationEffect>> {
    match action {
        PreservationAction::PrepareCopy {
            target_issue,
            attachment_id,
            asset,
        }
        | PreservationAction::AttachCopy {
            target_issue,
            attachment_id,
            asset,
            ..
        } => {
            require(
                attachment_id == &copy_id(target_issue, asset),
                "STATE_INVALID",
                "Canonical copy identity differs from source intent",
            )?;
            let Some(native) = store
                .optional("QArtifact", "attachment", attachment_id)
                .await?
            else {
                return Ok(None);
            };
            require(
                native["issue"]["id"] == *target_issue
                    && native["metadata"] == copy_metadata(asset),
                "PRESERVATION_CONFLICT",
                "Canonical attachment is owned by different intent",
            )?;
            let url = native["url"]
                .as_str()
                .ok_or_else(|| Fault::new("INCOMPLETE_DATA", "Canonical artifact URL missing"))?;
            verify_bytes(store, url, asset).await?;
            Ok(Some(PreservationEffect {
                confirmed: json!({"attachment":native,"size_bytes":asset.size,"sha256":asset.digest}),
                next: None,
            }))
        }
        PreservationAction::ReparentDocument {
            target_issue,
            document,
        } => {
            let current = store
                .linear
                .object("QDocument", "document", identity(document)?)
                .await?;
            require(
                same_document(document, &current),
                "SOURCE_CHANGED",
                "Document content/visibility/title changed during preservation",
            )?;
            if current["issue"]["id"] == *target_issue && current["project"]["id"].is_null() {
                return Ok(Some(PreservationEffect {
                    confirmed: json!({"document":current}),
                    next: None,
                }));
            }
            require(
                current["issue"]["id"] == document["issue"]["id"]
                    && current["project"]["id"] == document["project"]["id"]
                    && current["updatedAt"] == document["updatedAt"],
                "SOURCE_CHANGED",
                "Document owner/timestamp changed before reparent",
            )?;
            Ok(None)
        }
    }
}
/// Execute exactly one previously journaled preservation action and reconcile its canonical result.
/// Prepare performs an explicit temporary reservation+PUT attempt using signed information only
/// in memory, returning a canonical attach action for the caller to journal separately. Attach
/// and reparent each make at most one canonical mutation. Lost canonical replies are reconciled
/// by exact ID; ambiguous readbacks remain uncertain. This helper never deletes or loops actions.
pub async fn execute_preservation(
    store: &Store,
    action: &PreservationAction,
) -> Result<PreservationEffect> {
    if let Some(effect) = reconcile_preservation(store, action).await? {
        return Ok(effect);
    }
    match action {
        PreservationAction::PrepareCopy {
            target_issue,
            attachment_id,
            asset,
        } => {
            preservation_target(store, target_issue, &asset.issue_id).await?;
            let bytes = verify_bytes(store, &asset.url, asset).await?;
            let slot = store
                .linear
                .reserve_upload(&asset.content_type, &asset.filename, asset.size)
                .await?;
            store.linear.put_upload(&slot, bytes).await?;
            let asset_url = slot["assetUrl"]
                .as_str()
                .ok_or_else(|| {
                    Fault::new(
                        "INCOMPLETE_DATA",
                        "Upload reservation lacks canonical asset URL",
                    )
                    .uncertain()
                })?
                .to_owned();
            Ok(PreservationEffect {
                confirmed: json!({"temporary_copy":{"asset_url":asset_url,"size_bytes":asset.size,"sha256":asset.digest},"temporary_upload_may_remain":true}),
                next: Some(PreservationAction::AttachCopy {
                    target_issue: target_issue.clone(),
                    attachment_id: attachment_id.clone(),
                    asset: asset.clone(),
                    asset_url,
                }),
            })
        }
        PreservationAction::AttachCopy {
            target_issue,
            attachment_id,
            asset,
            asset_url,
        } => {
            preservation_target(store, target_issue, &asset.issue_id).await?;
            verify_bytes(store, asset_url, asset).await?;
            let result=store.linear.call("MCreateArtifact",json!({"input":{"id":attachment_id,"issueId":target_issue,"title":asset.artifact["title"].as_str().unwrap_or(&asset.filename),"url":asset_url,"metadata":copy_metadata(asset)}})).await;
            if let Err(f) = result {
                if !f.uncertain {
                    return Err(f);
                }
                if let Some(effect) = reconcile_preservation(store, action)
                    .await
                    .map_err(Fault::uncertain)?
                {
                    return Ok(effect);
                }
                return Err(f);
            }
            reconcile_preservation(store, action)
                .await
                .map_err(Fault::uncertain)?
                .ok_or_else(|| {
                    Fault::new(
                        "NATIVE_STATE_MISMATCH",
                        "Published copy is not confirmed by exact-ID readback",
                    )
                    .uncertain()
                })
        }
        PreservationAction::ReparentDocument {
            target_issue,
            document,
        } => {
            let origin = document["issue"]["id"]
                .as_str()
                .ok_or_else(|| Fault::new("INCOMPLETE_DATA", "Original Document Issue missing"))?;
            preservation_target(store, target_issue, origin).await?;
            let result=store.linear.call("MUpdateDocument",json!({"id":identity(document)?,"input":{"issueId":target_issue,"projectId":null}})).await;
            if let Err(f) = result {
                if !f.uncertain {
                    return Err(f);
                }
                if let Some(effect) = reconcile_preservation(store, action)
                    .await
                    .map_err(Fault::uncertain)?
                {
                    return Ok(effect);
                }
                return Err(f);
            }
            reconcile_preservation(store, action)
                .await
                .map_err(Fault::uncertain)?
                .ok_or_else(|| {
                    Fault::new(
                        "NATIVE_STATE_MISMATCH",
                        "Reparented Document is not confirmed",
                    )
                    .uncertain()
                })
        }
    }
}
/// Confirm a completed action from native ownership/content/provenance/bytes; pending state refuses.
/// The caller may use this before any deletion even after a cold process restart.
pub async fn preservation_readback(
    store: &Store,
    action: &PreservationAction,
) -> Result<PreservationEffect> {
    reconcile_preservation(store, action).await?.ok_or_else(|| {
        Fault::new(
            "PRESERVATION_INCOMPLETE",
            "Canonical preservation effect is not confirmed",
        )
    })
}

/// Preserve a body inside a backtick fence strictly longer than every native backtick run.
fn fenced(body: &str) -> String {
    let longest = body.split(|c| c != '\x60').map(str::len).max().unwrap_or(0);
    let fence = "`".repeat(longest.saturating_add(1).max(3));
    format!("{fence}text\n{body}\n{fence}\n")
}
/// Serialize machine/native records with stable key order; encoding failure is explicit.
fn record(value: &Value) -> Result<String> {
    serde_json::to_string_pretty(value)
        .map_err(|_| Fault::new("STATE_INVALID", "Archive record encoding failed"))
}
/// Render exact text bodies independently of their metadata, preserving threading/actors/reactions.
fn bodies(records: &[Value]) -> Result<String> {
    let mut out = String::new();
    for native in records {
        let mut metadata = native.clone();
        let body = metadata
            .as_object_mut()
            .and_then(|m| m.remove("body"))
            .unwrap_or(Value::Null);
        out.push_str(&fenced(&record(&metadata)?));
        out.push_str(&fenced(body.as_str().unwrap_or("")));
    }
    Ok(out)
}
/// Produce every required H3 section body for one source without dropping native body text.
fn item_sections(item: &ArchiveItem) -> Result<Vec<(&'static str, String)>> {
    let mut fields = item.native.clone();
    let description = fields
        .as_object_mut()
        .and_then(|m| m.remove("description"))
        .unwrap_or(Value::Null);
    let fields = format!(
        "{}{}",
        fenced(&record(&fields)?),
        fenced(description.as_str().unwrap_or(""))
    );
    let workflow = serde_json::to_value(&item.workflow)
        .map_err(|_| Fault::new("STATE_INVALID", "Archive workflow encoding failed"))?;
    let mut documents = String::new();
    for document in &item.documents {
        let mut metadata = document.clone();
        let map = metadata
            .as_object_mut()
            .ok_or_else(|| Fault::new("INCOMPLETE_DATA", "Document record is invalid"))?;
        let body = map.remove("content").unwrap_or(Value::Null);
        let comments = map.remove("comments").unwrap_or(json!([]));
        documents.push_str(&fenced(&record(&metadata)?));
        documents.push_str(&fenced(body.as_str().unwrap_or("")));
        documents.push_str(&bodies(comments.as_array().ok_or_else(|| {
            Fault::new("INCOMPLETE_DATA", "Document comments are invalid")
        })?)?);
    }
    Ok(vec![
        ("Fields", fields),
        ("Workflow", fenced(&record(&workflow)?)),
        ("Comments", bodies(&item.comments)?),
        ("Documents", documents),
        ("Files", fenced(&record(&json!(item.files))?)),
        ("Relations", fenced(&record(&json!(item.relations))?)),
        ("History", fenced(&record(&json!(item.history))?)),
    ])
}
/// Validate a native identifier before using it as a unique archive heading component.
fn heading_identifier(item: &ArchiveItem) -> Result<&str> {
    let identifier = item.native["identifier"]
        .as_str()
        .ok_or_else(|| Fault::new("INCOMPLETE_DATA", "Archive identifier missing"))?;
    require(
        !identifier.is_empty()
            && identifier
                .chars()
                .all(|c| c.is_alphanumeric() || matches!(c, '-' | '_')),
        "INCOMPLETE_DATA",
        "Archive identifier cannot form a safe heading",
    )?;
    Ok(identifier)
}
/// Render one deterministic readable full archive, with Summary/Contents/H2 items and unique H3 sections.
/// Original descriptions, comments and Document text remain exact inside safe fences. Facts
/// are optional caller-captured values. All original URLs are historical; no clock or I/O occurs.
pub fn render(set: &ArchiveSet, facts: Option<&ArchiveFacts>) -> Result<String> {
    let mut out = String::from(
        "# Complete Epic archive\n\n## Summary\n\nOriginal URLs below are historical and may stop resolving after soft deletion.\n\n",
    );
    out.push_str(&fenced(&record(&set.epic.workflow.fields)?));
    if let Some(timestamp) = facts.and_then(|f| f.applied_at.as_deref()) {
        out.push_str(&format!(
            "Applied at: {}\n\n",
            serde_json::to_string(timestamp)
                .map_err(|_| Fault::new("STATE_INVALID", "Timestamp encoding failed"))?
        ));
    }
    let sources: Vec<_> = std::iter::once(&set.epic).chain(&set.items).collect();
    let mut unique = BTreeSet::new();
    for source in &sources {
        let identifier = heading_identifier(source)?;
        require(
            unique.insert(identifier),
            "INCOMPLETE_DATA",
            "Duplicate native identifier would make archive headings ambiguous",
        )?;
        out.push_str(&format!(
            "- {identifier} · {} · {} · {} · completedAt={}\n",
            source.kind.label(),
            serde_json::to_string(&source.native["title"])
                .map_err(|_| Fault::new("STATE_INVALID", "Title encoding failed"))?,
            source.workflow.status.name(),
            serde_json::to_string(&source.native["completedAt"])
                .map_err(|_| Fault::new("STATE_INVALID", "Completion encoding failed"))?
        ));
    }
    out.push_str("\n## Contents\n\n");
    for source in &sources {
        let identifier = heading_identifier(source)?;
        out.push_str(&format!("- {identifier} · {}\n", source.kind.label()));
        for (name, _) in item_sections(source)? {
            out.push_str(&format!("  - {identifier} {name}\n"));
        }
    }
    for source in sources {
        let identifier = heading_identifier(source)?;
        out.push_str(&format!("\n## {identifier} · {}\n", source.kind.label()));
        for (name, body) in item_sections(source)? {
            out.push_str(&format!("\n### {identifier} {name}\n\n{body}"));
        }
    }
    Ok(out)
}
/// Return all explicit unknown/oversize document and complete-section bounds without writes.
/// `document` must be the exact output of `render(set, facts)`; caller-provided measured limits
/// never waive transport/render budgets, and equality at a limit is accepted.
pub fn validate_limits(
    set: &ArchiveSet,
    document: &str,
    limits: ArchiveLimits,
) -> Result<Vec<ArchiveBlocker>> {
    let mut blockers = vec![];
    let epic = identity(&set.epic.native)?;
    match limits.archive_max_bytes {
        None => block(
            &mut blockers,
            "ARCHIVE_LIMIT_UNKNOWN",
            epic,
            "Single native Document capacity has not been measured",
        ),
        Some(cap) if cap == 0 || cap >= 8 * 1024 * 1024 || document.len() > cap => block(
            &mut blockers,
            "ARCHIVE_TOO_LARGE",
            epic,
            format!(
                "Archive is {} bytes, measured cap {cap}; no splitting/truncation",
                document.len()
            ),
        ),
        _ => {}
    }
    let Some(cap) = limits.section_max_bytes else {
        block(
            &mut blockers,
            "SECTION_LIMIT_UNKNOWN",
            epic,
            "Complete rendered section capacity has not been measured",
        );
        return Ok(blockers);
    };
    require(
        cap > 0 && cap <= crate::render::TEXT_BUDGET_BYTES,
        "INVALID_INPUT",
        "Section cap must fit the existing response text budget",
    )?;
    for source in std::iter::once(&set.epic).chain(&set.items) {
        let identifier = heading_identifier(source)?;
        for (name, _) in item_sections(source)? {
            let heading = format!("{identifier} {name}");
            let section = crate::sections::find_section(document, &heading)?;
            if section.body.len() > cap {
                block(
                    &mut blockers,
                    "SECTION_TOO_LARGE",
                    identity(&source.native)?,
                    format!(
                        "{heading}: {} bytes > {cap}; no truncation",
                        section.body.len()
                    ),
                );
            }
        }
    }
    Ok(blockers)
}

/// Append a stable refusal to an archive preview without performing writes.
fn block(blockers: &mut Vec<ArchiveBlocker>, code: &str, item: &str, detail: impl Into<String>) {
    blockers.push(ArchiveBlocker {
        code: code.into(),
        item: item.into(),
        detail: detail.into(),
    });
}
/// Require an exact native node identity; missing IDs are incomplete data, never anonymous records.
fn identity(node: &Value) -> Result<&str> {
    node["id"]
        .as_str()
        .filter(|s| !s.is_empty())
        .ok_or_else(|| Fault::new("INCOMPLETE_DATA", "Archive node has no identity"))
}
/// Sort and deduplicate native records by exact identity; conflicting duplicate payloads refuse.
fn ordered(nodes: Vec<Value>) -> Result<Vec<Value>> {
    let mut by_id = BTreeMap::new();
    for node in nodes {
        let id = identity(&node)?.to_owned();
        if let Some(previous) = by_id.insert(id, node.clone()) {
            require(
                previous == node,
                "SOURCE_CHANGED",
                "Repeated native identity carries different data",
            )?;
        }
    }
    Ok(by_id.into_values().collect())
}
/// Read every reply recursively, including replies not present in the Issue's root connection.
/// The graph is bounded at 20,000 comments; repeated IDs must carry identical records.
/// Each identity's child connection is read once and the final records are sorted by ID.
async fn threaded(store: &Store, roots: Vec<Value>) -> Result<Vec<Value>> {
    let mut queue = VecDeque::from(ordered(roots)?);
    let mut nodes = BTreeMap::new();
    while let Some(node) = queue.pop_front() {
        let id = identity(&node)?.to_owned();
        if let Some(previous) = nodes.get(&id) {
            require(
                previous == &node,
                "SOURCE_CHANGED",
                "Repeated comment identity carries different data",
            )?;
            continue;
        }
        nodes.insert(id.clone(), node);
        require(
            nodes.len() <= 20_000,
            "INCOMPLETE_DATA",
            "Comment tree exceeds archive budget",
        )?;
        queue.extend(
            store
                .pages("QArchiveReplies", "/comment/children", json!({"id":id}))
                .await?,
        );
    }
    Ok(nodes.into_values().collect())
}
/// Collect one source's fully paginated details; no remote mutation or credential extraction occurs.
async fn item(store: &Store, work: &Work) -> Result<ArchiveItem> {
    let native = store
        .linear
        .object("QArchiveIssue", "issue", work.id())
        .await?;
    require(
        native["id"] == work.native["id"]
            && native["updatedAt"] == work.native["updatedAt"]
            && native["description"] == work.native["description"]
            && native["parent"] == work.native["parent"]
            && native["project"]["id"] == work.native["project"]["id"]
            && native["state"]["id"] == work.native["state"]["id"],
        "SOURCE_CHANGED",
        "Issue changed while archive collection started",
    )?;
    require(
        !is_trashed(&native)?,
        "SOURCE_CHANGED",
        "Archive source is already trashed or trash visibility is unknown",
    )?;
    let comments = threaded(
        store,
        store
            .pages(
                "QArchiveComments",
                "/issue/comments",
                json!({"id":work.id()}),
            )
            .await?,
    )
    .await?;
    let mut documents = ordered(
        store
            .pages(
                "QArchiveDocuments",
                "/issue/documents",
                json!({"id":work.id()}),
            )
            .await?,
    )?;
    for document in &mut documents {
        require(
            document["content"].is_string() || document["content"].is_null(),
            "INCOMPLETE_DATA",
            "Unreadable Document content",
        )?;
        let comments = store
            .pages(
                "QArchiveDocumentComments",
                "/document/comments",
                json!({"id":identity(document)?}),
            )
            .await?;
        document["comments"] = json!(threaded(store, comments).await?);
    }
    let mut relations = store
        .pages(
            "QArchiveRelations",
            "/issue/relations",
            json!({"id":work.id()}),
        )
        .await?;
    relations.extend(
        store
            .pages(
                "QArchiveInverseRelations",
                "/issue/inverseRelations",
                json!({"id":work.id()}),
            )
            .await?,
    );
    let files = store
        .linear
        .attachments(work.id())
        .await?
        .into_iter()
        .filter(|n| n["id"] != child_id(work.id(), "state"))
        .collect();
    Ok(ArchiveItem {
        kind: work.managed()?.kind,
        native,
        workflow: work.managed()?.clone(),
        comments,
        documents,
        files: ordered(files)?,
        relations: ordered(relations)?,
        history: ordered(
            store
                .pages("QArchiveHistory", "/issue/history", json!({"id":work.id()}))
                .await?,
        )?,
    })
}
/// Discover the closure from native child pages plus recorded children and collect every source.
/// `graph` must be the caller's complete fresh same-Project graph. Detached/foreign/unmanaged
/// children and failed structural/review/pending guards become explicit blockers. Read failures
/// propagate with their typed cause; this function has no side effects.
pub async fn collect(store: &Store, epic: &Work, graph: &[Work]) -> Result<ArchiveSet> {
    require(
        epic.managed()?.kind == Kind::Epic,
        "INVALID_INPUT",
        "Archive root must be a managed Epic",
    )?;
    let mut full = graph.to_vec();
    let mut queue = VecDeque::from([epic.id().to_owned()]);
    let mut discovered = BTreeSet::new();
    let mut blockers = vec![];
    while let Some(id) = queue.pop_front() {
        if !discovered.insert(id.clone()) {
            continue;
        }
        require(
            discovered.len() <= 20_000,
            "INCOMPLETE_DATA",
            "Archive closure exceeds 20,000 items",
        )?;
        let fresh = match store.work(&id).await {
            Ok(w) => w,
            Err(f) if f.code == "RECORD_MISSING" => {
                block(
                    &mut blockers,
                    "MISSING_CHILD",
                    &id,
                    "Recorded child is absent; deletion cannot be inferred",
                );
                continue;
            }
            Err(f) => return Err(f),
        };
        let children = store
            .pages("QArchiveChildren", "/issue/children", json!({"id":id}))
            .await?;
        for child in children {
            queue.push_back(identity(&child)?.to_owned());
        }
        if let Some(m) = &fresh.meta {
            queue.extend(m.children.clone());
        }
        if let Some(w) = full.iter_mut().find(|w| w.id() == id) {
            *w = fresh;
        } else {
            full.push(fresh);
        }
    }
    let mut sources = vec![];
    for id in &discovered {
        let Some(work) = rules::find(&full, id) else {
            continue;
        };
        let Some(meta) = &work.meta else {
            block(
                &mut blockers,
                "UNMANAGED_ITEM",
                id,
                "Native descendant has no managed ownership or concrete kind",
            );
            continue;
        };
        if work.native["project"]["id"] != epic.managed()?.project_id {
            block(
                &mut blockers,
                "FOREIGN_ITEM",
                id,
                "Native descendant belongs to another Project",
            );
        }
        if id != epic.id()
            && (meta.kind == Kind::Epic
                || !rules::parent(work).is_some_and(|p| discovered.contains(p)))
        {
            block(
                &mut blockers,
                "MOVED_CHILD",
                id,
                "Recorded descendant no longer has a parent in the Epic closure",
            );
        }
        if !work.status()?.terminal() {
            block(
                &mut blockers,
                "NONTERMINAL_ITEM",
                id,
                "Archive source is unfinished",
            );
        }
        for detail in rules::discrepancies(work, &full) {
            block(&mut blockers, "NATIVE_DRIFT", id, detail);
        }
        if matches!(meta.kind, Kind::Module | Kind::Atomic)
            && meta.status == Status::Done
            && !meta
                .review
                .as_ref()
                .is_some_and(|r| r.accepted && r.round == meta.round && r.revision == meta.revision)
        {
            block(
                &mut blockers,
                "REVIEW_INVALID",
                id,
                "Terminal reviewed work lacks current accepted review",
            );
        }
        sources.push(item(store, work).await?);
    }
    for outside in &full {
        if discovered.contains(outside.id()) || outside.status().is_ok_and(Status::terminal) {
            continue;
        }
        if outside.meta.is_some() {
            let refs = rules::module_ids(&outside.fields);
            let duplicate = outside.fields["duplicate_of"].as_str();
            if refs.iter().any(|id| discovered.contains(id))
                || duplicate.is_some_and(|r| {
                    sources
                        .iter()
                        .any(|s| s.native["id"] == r || s.native["url"] == r)
                })
            {
                block(
                    &mut blockers,
                    "ACTIVE_REFERENCE",
                    outside.id(),
                    "Active same-Project work refers to an archived descendant",
                );
            }
        }
    }
    for source in &sources {
        for relation in &source.relations {
            if !relation["archivedAt"].is_null() {
                continue;
            }
            for endpoint in ["issue", "relatedIssue"] {
                let Some(id) = relation[endpoint]["id"].as_str() else {
                    return Err(Fault::new(
                        "INCOMPLETE_DATA",
                        "Relation endpoint identity missing",
                    ));
                };
                if discovered.contains(id) {
                    continue;
                }
                let external = match rules::find(&full, id) {
                    Some(w) => w.clone(),
                    None => store.work(id).await?,
                };
                if !external.status()?.terminal() {
                    block(
                        &mut blockers,
                        "ACTIVE_REFERENCE",
                        source.native["id"].as_str().unwrap_or(epic.id()),
                        format!("Active native relation to {id}"),
                    );
                }
            }
        }
    }
    sources.sort_by(|a, b| {
        a.native["identifier"]
            .as_str()
            .cmp(&b.native["identifier"].as_str())
            .then_with(|| a.native["id"].as_str().cmp(&b.native["id"].as_str()))
    });
    let root = sources
        .iter()
        .position(|s| s.native["id"] == epic.id())
        .ok_or_else(|| Fault::new("UNMANAGED_ITEM", "Epic source unavailable"))?;
    let epic_item = sources.remove(root);
    let mut set = ArchiveSet {
        epic: epic_item,
        items: sources,
        assets: vec![],
        blockers,
    };
    collect_assets(store, &mut set).await?;
    set.blockers
        .sort_by(|a, b| (&a.item, &a.code, &a.detail).cmp(&(&b.item, &b.code, &b.detail)));
    set.blockers.dedup();
    Ok(set)
}
/// Extract actual Markdown link/image destinations plus canonical upload URLs, including raw body links.
fn asset_urls(body: &str) -> BTreeSet<String> {
    let mut urls = BTreeSet::new();
    for event in Parser::new(body) {
        if let Event::Start(Tag::Link { dest_url, .. } | Tag::Image { dest_url, .. }) = event
            && dest_url.contains("uploads.linear.app")
        {
            urls.insert(dest_url.to_string());
        }
    }
    for (offset, _) in body.match_indices("https://uploads.linear.app/") {
        let tail = &body[offset..];
        let end = tail
            .find(|c: char| c.is_whitespace() || matches!(c, ')' | '>' | '"' | '\'' | '\x60'))
            .unwrap_or(tail.len());
        urls.insert(tail[..end].into());
    }
    urls
}
/// Recursively collect links from all source string values, retaining complete text elsewhere.
fn links(value: &Value, urls: &mut BTreeSet<String>) {
    match value {
        Value::String(s) => urls.extend(asset_urls(s)),
        Value::Array(xs) => {
            for x in xs {
                links(x, urls);
            }
        }
        Value::Object(xs) => {
            for x in xs.values() {
                links(x, urls);
            }
        }
        _ => {}
    }
}
/// Read each referenced asset once to prove readability, size and digest; retain bytes only transiently.
async fn collect_assets(store: &Store, set: &mut ArchiveSet) -> Result<()> {
    for source in std::iter::once(&set.epic).chain(&set.items) {
        let id = identity(&source.native)?;
        let mut urls = BTreeSet::new();
        links(&source.native["description"], &mut urls);
        links(&json!(source.comments), &mut urls);
        links(&json!(source.documents), &mut urls);
        for file in &source.files {
            if file["metadata"]["artifact"].is_object()
                || file["url"]
                    .as_str()
                    .is_some_and(|s| s.contains("uploads.linear.app"))
            {
                let url = file["url"]
                    .as_str()
                    .ok_or_else(|| Fault::new("INCOMPLETE_DATA", "Asset attachment URL missing"))?;
                urls.insert(url.into());
            }
        }
        for url in urls {
            match store.linear.get_asset(&url).await {
                Ok((bytes, content_type)) => {
                    let digest = format!("{:x}", Sha256::digest(&bytes));
                    let size = bytes.len() as u64;
                    let file = source.files.iter().find(|f| f["url"] == url);
                    let artifact=file.map(|f|f["metadata"]["artifact"].clone()).filter(Value::is_object)
                        .unwrap_or_else(||json!({"filename":"preserved-upload","content_type":content_type,"size_bytes":size,"sha256":digest,"title":null,"note":null}));
                    let filename = artifact["filename"]
                        .as_str()
                        .unwrap_or("preserved-upload")
                        .to_owned();
                    if artifact["sha256"].as_str().is_some_and(|d| d != digest)
                        || artifact["size_bytes"].as_u64().is_some_and(|n| n != size)
                    {
                        block(
                            &mut set.blockers,
                            "ASSET_CHANGED",
                            id,
                            "Artifact bytes differ from recorded intent",
                        );
                    }
                    set.assets.push(ArchiveAsset {
                        issue_id: id.into(),
                        identifier: source.native["identifier"].as_str().unwrap_or(id).into(),
                        origin: file.and_then(|f| f["id"].as_str()).unwrap_or(&url).into(),
                        url,
                        size,
                        digest,
                        filename,
                        content_type,
                        artifact,
                    });
                }
                Err(f) => block(
                    &mut set.blockers,
                    &f.code,
                    id,
                    format!("Unreadable/foreign/oversize asset {url}: {}", f.message),
                ),
            }
        }
    }
    set.assets
        .sort_by(|a, b| (&a.issue_id, &a.origin).cmp(&(&b.issue_id, &b.origin)));
    Ok(())
}
