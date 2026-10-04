//! Explicitly opted-in native archive gates; ordinary test runs never access credentials.
use agent_tasks::{
    linear::Linear,
    model::{Fault, Result},
    records::{Store, child_id, markdown_equivalent},
};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{path::PathBuf, time::Instant};
use uuid::Uuid;

/// Exact fixture inventory and write-ahead observations; no credentials are journaled.
struct Probe {
    /// Authenticated native provider, created only after the explicit live opt-in.
    store: Store,
    /// Explicit report file, required to be new for a fresh fixture.
    path: PathBuf,
    /// Non-secret fixture identities, intents, native responses, and gate outcomes.
    report: Value,
}
impl Probe {
    /// Open the protected credential provider only when the operator explicitly opts in.
    fn new() -> Self {
        assert_eq!(
            std::env::var("ATL_ARCHIVE_LIVE").as_deref(),
            Ok("disposable-fixture")
        );
        let team = std::env::var("ATL_LIVE_TEAM_ID").expect("ATL_LIVE_TEAM_ID required");
        Uuid::parse_str(&team).expect("team must be a UUID");
        let path =
            PathBuf::from(std::env::var("ATL_LIVE_REPORT").expect("ATL_LIVE_REPORT required"));
        assert!(
            path.is_absolute() && !path.exists(),
            "fresh absolute report path required"
        );
        let key = std::env::var("LINEAR_API_KEY")
            .ok()
            .or_else(|| {
                std::env::var("LINEAR_API_KEY_FILE").ok().map(|p| {
                    std::fs::read_to_string(p)
                        .expect("protected credential provider")
                        .trim()
                        .to_owned()
                })
            })
            .expect("protected credential provider required");
        let run = Uuid::new_v4().to_string();
        let report = json!({"version":1,"run":run,"team":team,"fixture_project":child_id(&run,"project"),"sentinel_project":child_id(&run,"sentinel-project"),"issues":[],"steps":{},"gates":{}});
        let p = Self {
            store: Store {
                linear: Linear::new(Some(key), false).expect("native client"),
            },
            path,
            report,
        };
        p.save();
        p
    }

    /// Open only an explicitly opted-in existing fixture report; do not create a new run or replay old steps.
    fn resume() -> Self {
        assert_eq!(
            std::env::var("ATL_ARCHIVE_LIVE").as_deref(),
            Ok("disposable-fixture")
        );
        assert_eq!(
            std::env::var("ATL_ARCHIVE_FOLLOWUP").as_deref(),
            Ok("renderer-and-resources")
        );
        let path =
            PathBuf::from(std::env::var("ATL_LIVE_REPORT").expect("existing report required"));
        assert!(
            path.is_absolute()
                && std::fs::metadata(&path).expect("existing report").len() <= 32 * 1024 * 1024
        );
        let report: Value = serde_json::from_slice(&std::fs::read(&path).expect("read report"))
            .expect("report JSON");
        validate_existing_report(
            &report,
            &std::env::var("ATL_LIVE_TEAM_ID").expect("same team required"),
        )
        .expect("exact existing fixture inventory");
        let key = std::env::var("LINEAR_API_KEY")
            .ok()
            .or_else(|| {
                std::env::var("LINEAR_API_KEY_FILE").ok().map(|p| {
                    std::fs::read_to_string(p)
                        .expect("protected provider")
                        .trim()
                        .to_owned()
                })
            })
            .expect("protected provider required");
        Self {
            store: Store {
                linear: Linear::new(Some(key), false).expect("native client"),
            },
            path,
            report,
        }
    }

    /// Derive a fixture-owned identity from the recorded run and a stable purpose.
    fn id(&self, purpose: &str) -> String {
        child_id(self.report["run"].as_str().expect("run"), purpose)
    }
    /// Flush the evidence after each intent and response; failure stops before another write.
    fn save(&self) {
        std::fs::write(
            &self.path,
            serde_json::to_vec_pretty(&self.report).expect("report encoding"),
        )
        .expect("report persistence");
    }
    /// Record exact non-secret variables before a mutation and never blindly retry an unknown effect.
    async fn call(&mut self, step: &str, op: &str, args: Value) -> Result<Value> {
        assert!(
            self.report["steps"][step].is_null(),
            "step already attempted"
        );
        self.report["steps"][step] = json!({"operation":op,"variables":args,"state":"pending"});
        self.save();
        let started = Instant::now();
        let result = self.store.linear.call(op, args).await;
        match &result {
            Ok(v) => {
                let safe = evidence_response(op, v);
                self.report["steps"][step] = json!({"operation":op,"state":"confirmed","result":safe,"elapsed_ms":started.elapsed().as_millis()})
            }
            Err(f) => {
                self.report["steps"][step]["fault"] =
                    json!({"code":f.code,"message":f.message,"uncertain":f.uncertain})
            }
        }
        self.save();
        if result.as_ref().is_err_and(|f| f.uncertain) {
            panic!("unknown native effect at {step}; inspect saved intent before continuing");
        }
        result
    }
    /// Save a gate result independently: absent/blocked measurements never become passing defaults.
    fn gate(&mut self, name: &str, result: Result<Value>) {
        self.report["gates"][name] = match result {
            Ok(v) => json!({"state":"observed","evidence":v}),
            Err(f) => json!({"state":"blocked","code":f.code,"message":f.message}),
        };
        self.save();
    }
    /// Create one native fixture Issue after recording it in the exact cleanup inventory.
    async fn issue(
        &mut self,
        purpose: &str,
        parent: Option<&str>,
        sentinel: bool,
    ) -> Result<String> {
        let id = self.id(purpose);
        let project = self.report[if sentinel {
            "sentinel_project"
        } else {
            "fixture_project"
        }]
        .clone();
        self.report["issues"]
            .as_array_mut()
            .expect("inventory")
            .push(json!({"id":id,"purpose":purpose,"sentinel":sentinel,"project":project}));
        self.save();
        let mut input = json!({"id":id,"title":format!("Archive gate fixture {purpose}"),"description":format!("Archive fixture {}",self.report["run"].as_str().expect("run")),"teamId":self.report["team"],"projectId":project});
        if let Some(parent) = parent {
            input["parentId"] = json!(parent);
        }
        self.call(
            &format!("create-{purpose}"),
            "MCreateIssue",
            json!({"input":input}),
        )
        .await?;
        Ok(id)
    }
    /// Read the sentinel by exact ID and require native project, title and untrashed status.
    async fn sentinel(&self, id: &str) -> Result<Value> {
        let v = self
            .store
            .linear
            .object("QArchiveIssue", "issue", id)
            .await?;
        validate_sentinel(&v, &self.report["sentinel_project"])?;
        Ok(v)
    }
}

/// Check every report identity before credential access; the followup cannot adopt an unrelated run or invent targets.
fn validate_existing_report(report: &Value, team: &str) -> Result<()> {
    let run = report["run"]
        .as_str()
        .ok_or_else(|| Fault::new("STATE_INVALID", "Existing run missing"))?;
    Uuid::parse_str(run).map_err(|_| Fault::new("STATE_INVALID", "Invalid existing run"))?;
    observed(
        report["version"] == 1 && report["team"] == team,
        "Existing report belongs to another team/version",
    )?;
    observed(
        report["fixture_project"] == child_id(run, "project")
            && report["sentinel_project"] == child_id(run, "sentinel-project"),
        "Existing Project identity mismatch",
    )?;
    let items = report["issues"]
        .as_array()
        .ok_or_else(|| Fault::new("STATE_INVALID", "Existing inventory missing"))?;
    for item in items {
        let purpose = item["purpose"]
            .as_str()
            .ok_or_else(|| Fault::new("STATE_INVALID", "Inventory purpose missing"))?;
        observed(
            item["id"] == child_id(run, purpose),
            "Inventory identity mismatch",
        )?;
        let expected = if item["sentinel"] == true {
            &report["sentinel_project"]
        } else {
            &report["fixture_project"]
        };
        observed(item["project"] == *expected, "Inventory Project mismatch")?;
    }
    for purpose in ["subject", "survivor", "sentinel"] {
        observed(
            items.iter().any(|item| item["purpose"] == purpose),
            "Required existing owned fixture missing",
        )?;
    }
    Ok(())
}
/// Verify one exact owned existing Issue before a followup write; missing/trash/foreign data refuses.
async fn owned_followup_issue(p: &Probe, purpose: &str) -> Result<Value> {
    let id = p.id(purpose);
    let native = p.store.linear.object("QArchiveIssue", "issue", &id).await?;
    observed(
        native["id"] == id
            && native["team"]["id"] == p.report["team"]
            && native["project"]["id"] == p.report["fixture_project"]
            && !agent_tasks::archive::is_trashed(&native)?
            && native["description"]
                .as_str()
                .is_some_and(|s| s.contains(p.report["run"].as_str().expect("run"))),
        "Existing followup Issue ownership/state mismatch",
    )?;
    Ok(native)
}
/// Journal one real preservation action before invoking the production helper; never replay an existing attempt.
async fn preservation_step(
    p: &mut Probe,
    key: &str,
    action: &agent_tasks::archive::PreservationAction,
) -> Result<agent_tasks::archive::PreservationEffect> {
    observed(
        p.report["steps"][key].is_null(),
        "Preservation step already attempted; reconcile explicitly",
    )?;
    p.report["steps"][key] =
        json!({"operation":"execute_preservation","action":action,"state":"pending"});
    p.save();
    let result = agent_tasks::archive::execute_preservation(&p.store, action).await;
    match &result {
        Ok(effect) => {
            p.report["steps"][key] =
                json!({"operation":"execute_preservation","state":"confirmed","effect":effect})
        }
        Err(f) => {
            p.report["steps"][key]["fault"] =
                json!({"code":f.code,"message":f.message,"uncertain":f.uncertain})
        }
    }
    p.save();
    if result.as_ref().is_err_and(|f| f.uncertain) {
        panic!("unknown preservation effect; inspect the saved exact action");
    }
    result
}

/// Verify the expected sentinel's native project/title and nullable untrashed flag.
/// Missing/invalid flags remain incomplete; only a present true flag means trashed.
fn validate_sentinel(native: &Value, project: &Value) -> Result<()> {
    agent_tasks::model::require(
        native["project"]["id"] == *project
            && !agent_tasks::archive::is_trashed(native)?
            && native["title"] == "Archive gate fixture sentinel",
        "SENTINEL_CHANGED",
        "Unrelated sentinel changed",
    )
}

/// Copy a native response for durable evidence, removing ephemeral signed upload credentials.
fn evidence_response(operation: &str, native: &Value) -> Value {
    let mut safe = native.clone();
    if operation == "MFileUpload"
        && let Some(slot) = safe["fileUpload"]["uploadFile"].as_object_mut()
    {
        slot.remove("uploadUrl");
        slot.remove("headers");
    }
    safe
}
/// Prove native pagination using two records per page and a small quota-aware fixture.
/// Missing or repeated cursors refuse; at most 200 pages are read and no writes occur.
async fn small_pages(store: &Store, query: &str, pointer: &str, id: &str) -> Result<Vec<Value>> {
    let mut nodes = vec![];
    let mut after = Value::Null;
    for _ in 0..200 {
        let response = store
            .linear
            .call(query, json!({"id":id,"first":2,"after":after}))
            .await?;
        let page = response
            .pointer(pointer)
            .ok_or_else(|| Fault::new("INCOMPLETE_DATA", "probe page missing"))?;
        nodes.extend(
            page["nodes"]
                .as_array()
                .ok_or_else(|| Fault::new("INCOMPLETE_DATA", "probe nodes missing"))?
                .iter()
                .cloned(),
        );
        if page["pageInfo"]["hasNextPage"] == false {
            return Ok(nodes);
        }
        let next = page["pageInfo"]["endCursor"].clone();
        observed(
            next.is_string() && next != after,
            "probe cursor did not advance",
        )?;
        after = next;
    }
    Err(Fault::new("INCOMPLETE_DATA", "probe page budget exceeded"))
}

/// Convert a measurement mismatch into an explicit blocked gate without fabricating a limit.
fn observed(condition: bool, message: &str) -> Result<()> {
    agent_tasks::model::require(condition, "GATE_BLOCKED", message)
}

/// Fixed three-size probe budget, independent of environment overrides: less than2MiB source total.
fn document_sizes() -> [usize; 3] {
    [64 * 1024, 256 * 1024, 1024 * 1024]
}
/// Allocate a fresh cleanup attempt only after an exact native read confirms the deterministic
/// fixture identity remains untrashed. Existing failed/unknown intents remain unchanged.
fn cleanup_step(report: &Value, purpose: &str, native: &Value) -> Result<String> {
    let run = report["run"]
        .as_str()
        .ok_or_else(|| Fault::new("STATE_INVALID", "Fixture run missing"))?;
    observed(
        native["id"] == child_id(run, purpose) && !agent_tasks::archive::is_trashed(native)?,
        "Fresh exact untrashed fixture read required",
    )?;
    let steps = report["steps"]
        .as_object()
        .ok_or_else(|| Fault::new("STATE_INVALID", "Fixture steps missing"))?;
    for attempt in 1..=steps.len().saturating_add(1) {
        let key = format!("cleanup-{purpose}-attempt-{attempt}");
        if !steps.contains_key(&key) {
            return Ok(key);
        }
    }
    Err(Fault::new(
        "STATE_INVALID",
        "Cleanup attempt budget exhausted",
    ))
}

/// Build nested-fence/table/Unicode fixture text without slicing a native document.
fn document_body(bytes: usize) -> String {
    let seed = "Юникод 😀 | table\n\n| a | b |\n|---|---|\n| 1 | 2 |\n\n````text\n```nested\n## literal\n```\n````\n\n";
    format!(
        "## Archive\n### Complete section\n{}",
        seed.repeat(bytes.div_ceil(seed.len()))
    )
}
/// Exercise native trash/read/list/restore on one owned Issue and explicitly leave quota observation to the owner.
async fn gate_delete(p: &mut Probe, issue: &str) -> Result<Value> {
    let deleted = p
        .call("g1-delete", "MArchiveProbeDelete", json!({"id":issue}))
        .await?;
    let exact = p
        .store
        .linear
        .object("QArchiveIssue", "issue", issue)
        .await?;
    observed(
        agent_tasks::archive::is_trashed(&exact)?,
        "exact-id trash read is unavailable",
    )?;
    let listed=p.store.pages("QIssues","issues",json!({"filter":{"project":{"id":{"eq":p.report["fixture_project"]}}},"includeArchived":true})).await?;
    let restored = p
        .call("g1-restore", "MArchiveProbeRestore", json!({"id":issue}))
        .await?;
    let readback = p
        .store
        .linear
        .object("QArchiveIssue", "issue", issue)
        .await?;
    observed(
        !agent_tasks::archive::is_trashed(&readback)?,
        "issueUnarchive did not restore trash",
    )?;
    Ok(
        json!({"delete":deleted,"exact_trashed":exact["trashed"],"listed_while_trashed":listed.iter().any(|n| n["id"]==issue),"restore":restored,"quota_relief":"UNKNOWN: owner must inspect Free quota before/after"}),
    )
}
/// Measure increasing document sizes with full exact readback, section presentation, and honest search pages.
async fn gate_sizes(p: &mut Probe, issue: &str) -> Result<Value> {
    let id = p.id("size-document");
    let sizes = document_sizes();
    let mut evidence = vec![];
    for (index, size) in sizes.into_iter().enumerate() {
        let body = document_body(size);
        let op = if index == 0 {
            "MCreateDocument"
        } else {
            "MUpdateDocument"
        };
        let args = if index == 0 {
            json!({"input":{"id":id,"issueId":issue,"title":"Archive size fixture","content":body}})
        } else {
            json!({"id":id,"input":{"content":body}})
        };
        p.call(&format!("g2-size-{index}"), op, args).await?;
        let read = p.store.linear.object("QDocument", "document", &id).await?;
        observed(
            markdown_equivalent(&body, read["content"].as_str().unwrap_or("")),
            "document readback differs",
        )?;
        let section = agent_tasks::sections::find_section(
            read["content"].as_str().unwrap_or(""),
            "Complete section",
        )?;
        let mut selected = read.clone();
        selected["content"] = json!(section.body);
        selected["section"] =
            json!({"heading":section.heading,"index":section.index,"count":section.count});
        let rendered = agent_tasks::render::render_outcome(
            "get_context",
            &json!({"type":"document","id":id,"section":"Complete section"}),
            &agent_tasks::model::Outcome::ok(selected),
        );
        let presentation_complete = rendered.contains(section.body);
        let search = p
            .store
            .linear
            .call(
                "QSearchDocuments",
                json!({"term":"Archive size fixture","first":1,"includeArchived":true}),
            )
            .await?;
        evidence.push(json!({"document_bytes":body.len(),"section_bytes":section.body.len(),"rendered_bytes":rendered.len(),"protocol_text_json_bytes":serde_json::to_vec(&rendered).expect("text JSON").len(),"section_complete":presentation_complete,"search_page_info":search["searchDocuments"]["pageInfo"],"search_has_fixture":search["searchDocuments"]["nodes"].as_array().is_some_and(|ns|ns.iter().any(|n|n["id"]==id))}));
        observed(
            presentation_complete,
            "section exceeds rendered text budget",
        )?;
    }
    Ok(
        json!({"measurements":evidence,"limits":"largest observed successful sizes only; never extrapolate"}),
    )
}
/// Stress native connections with >100 comments, >50 replies, and bounded history edits; counts remain empirical.
async fn gate_paging(p: &mut Probe, issue: &str, peer: &str) -> Result<Value> {
    let root = p.id("comment-0");
    for index in 0..101 {
        p.call(&format!("g4-comment-{index}"),"MCreateComment",json!({"input":{"id":p.id(&format!("comment-{index}")),"issueId":issue,"body":format!("fixture comment {index}")}})).await?;
    }
    for index in 0..51 {
        p.call(&format!("g4-reply-{index}"),"MCreateComment",json!({"input":{"id":p.id(&format!("reply-{index}")),"issueId":issue,"parentId":root,"body":format!("fixture reply {index}")}})).await?;
    }
    p.call(
        "g4-reaction",
        "MArchiveProbeReaction",
        json!({"input":{"id":p.id("reaction"),"commentId":root,"emoji":"👍"}}),
    )
    .await?;
    for index in 0..101 {
        p.call(
            &format!("g4-history-{index}"),
            "MUpdateIssue",
            json!({"id":issue,"input":{"title":format!("Archive fixture history {index}")}}),
        )
        .await?;
    }
    p.call("g4-relation","MCreateIssueRelation",json!({"input":{"id":p.id("relation"),"issueId":issue,"relatedIssueId":peer,"type":"related"}})).await?;
    for index in 0..3 {
        let target = p
            .issue(&format!("relation-peer-{index}"), None, false)
            .await?;
        p.call(&format!("g4-extra-relation-{index}"),"MCreateIssueRelation",json!({"input":{"id":p.id(&format!("extra-relation-{index}")),"issueId":issue,"relatedIssueId":target,"type":"related"}})).await?;
        p.call(&format!("g4-extra-inverse-{index}"),"MCreateIssueRelation",json!({"input":{"id":p.id(&format!("extra-inverse-{index}")),"issueId":target,"relatedIssueId":peer,"type":"related"}})).await?;
    }
    let comments = p
        .store
        .pages("QArchiveComments", "/issue/comments", json!({"id":issue}))
        .await?;
    let replies = p
        .store
        .pages("QArchiveReplies", "/comment/children", json!({"id":root}))
        .await?;
    let history = p
        .store
        .pages("QArchiveHistory", "/issue/history", json!({"id":issue}))
        .await?;
    let relations = small_pages(&p.store, "QIssueRelations", "/issue/relations", issue).await?;
    let inverse = small_pages(
        &p.store,
        "QArchiveInverseRelations",
        "/issue/inverseRelations",
        peer,
    )
    .await?;
    observed(
        comments.len() >= 101 && replies.len() >= 51,
        "comment/reply paging missing records",
    )?;
    Ok(
        json!({"comments":comments.len(),"replies":replies.len(),"history":history.len(),"history_over_100":history.len()>100,"history_grouping":"native may group edits; false means gate still incomplete","relations":relations,"inverse_relations":inverse,"reaction_observed":comments.iter().any(|c|c["reactions"].as_array().is_some_and(|r|!r.is_empty())),"relations_native_page_size":2,"relations_multiple_pages":relations.len()>2&&inverse.len()>2,"document_comments_over_100":"UNKNOWN: follow-up gate below"}),
    )
}
/// Observe whether each native comment/reaction/document edit changes the owning Issue timestamp.
async fn gate_drift(p: &mut Probe, issue: &str) -> Result<Value> {
    let mut evidence = vec![];
    let comment = p.id("comment-0");
    let document = p.id("size-document");
    for (index, op, args) in [
        (
            "comment",
            "MArchiveProbeCommentEdit",
            json!({"id":comment,"input":{"body":"edited fixture comment"}}),
        ),
        (
            "reaction",
            "MArchiveProbeReaction",
            json!({"input":{"id":p.id("issue-reaction"),"issueId":issue,"emoji":"👍"}}),
        ),
        (
            "document",
            "MUpdateDocument",
            json!({"id":document,"input":{"title":"Edited archive fixture"}}),
        ),
    ] {
        let before = p
            .store
            .linear
            .object("QArchiveIssue", "issue", issue)
            .await?;
        p.call(&format!("g5-{index}"), op, args).await?;
        let after = p
            .store
            .linear
            .object("QArchiveIssue", "issue", issue)
            .await?;
        evidence.push(json!({"effect":index,"before":before["updatedAt"],"after":after["updatedAt"],"changed":before["updatedAt"]!=after["updatedAt"]}));
    }
    Ok(
        json!({"measurements":evidence,"apply_source_change_abort":"UNKNOWN until compact integration probe"}),
    )
}
/// Measure attachment metadata size by deterministic overwrite and exact metadata readback.
async fn gate_metadata(p: &mut Probe, issue: &str) -> Result<Value> {
    let id = p.id("metadata");
    let mut evidence = vec![];
    for (index, size) in [16 * 1024, 64 * 1024, 256 * 1024].into_iter().enumerate() {
        let metadata = json!({"probe": "x".repeat(size)});
        let args = if index == 0 {
            json!({"input":{"id":id,"issueId":issue,"title":"Archive metadata fixture","url":"https://linear.app","metadata":metadata}})
        } else {
            json!({"id":id,"input":{"metadata":metadata}})
        };
        p.call(
            &format!("g6-{index}"),
            if index == 0 {
                "MCreateArtifact"
            } else {
                "MUpdateAttachment"
            },
            args,
        )
        .await?;
        let read = p
            .store
            .linear
            .object("QArtifact", "attachment", &id)
            .await?;
        observed(read["metadata"] == metadata, "metadata readback changed")?;
        evidence
            .push(json!({"metadata_bytes":serde_json::to_vec(&metadata).expect("metadata").len()}));
    }
    Ok(json!({"measurements":evidence,"limit":"largest successful measurement only"}))
}
/// Preserve a reparented document and copied embedded upload, then observe cascade retention by exact IDs.
async fn gate_retention(p: &mut Probe, issue: &str, survivor: &str) -> Result<Value> {
    let bytes = b"archive gate binary\0\xfffixture".to_vec();
    let digest = format!("{:x}", Sha256::digest(&bytes));
    let slot = p.call("g3-original-reservation", "MFileUpload", json!({"contentType":"application/octet-stream","filename":"fixture.bin","size":bytes.len(),"makePublic":false})).await?["fileUpload"]["uploadFile"].clone();
    p.report["upload_intent"] =
        json!({"kind":"original","asset_url":slot["assetUrl"],"digest":digest});
    p.save();
    p.store
        .linear
        .put_upload(&slot, bytes.clone())
        .await
        .unwrap_or_else(|_| panic!("unknown original upload; inspect saved reservation"));
    let original = slot["assetUrl"].as_str().expect("asset URL");
    p.call("g3-original-file","MCreateArtifact",json!({"input":{"id":p.id("original-file"),"issueId":issue,"title":"Original fixture file","url":original,"metadata":{"fixture":true}}})).await?;
    let copy = p.call("g3-copy-reservation", "MFileUpload", json!({"contentType":"application/octet-stream","filename":"fixture.bin","size":bytes.len(),"makePublic":false})).await?["fileUpload"]["uploadFile"].clone();
    p.report["upload_intent"] = json!({"kind":"copy","asset_url":copy["assetUrl"],"digest":digest});
    p.save();
    p.store
        .linear
        .put_upload(&copy, bytes)
        .await
        .unwrap_or_else(|_| panic!("unknown copied upload; inspect saved reservation"));
    let copied = copy["assetUrl"].as_str().expect("asset URL");
    p.call("g3-copied-file","MCreateArtifact",json!({"input":{"id":p.id("copied-file"),"issueId":survivor,"title":"Copied fixture file","url":copied,"metadata":{"fixture":true}}})).await?;
    let doc = p.id("retention-document");
    p.call("g3-document","MCreateDocument",json!({"input":{"id":doc,"issueId":issue,"title":"Retention fixture","content":format!("## Retained\n[embedded]({original})")}})).await?;
    let native = p.store.linear.object("QDocument", "document", &doc).await?;
    let docs = p
        .store
        .pages("QArchiveDocuments", "/issue/documents", json!({"id":issue}))
        .await?;
    let content_id = docs
        .iter()
        .find(|d| d["id"] == doc)
        .and_then(|d| d["documentContentId"].as_str())
        .ok_or_else(|| Fault::new("GATE_BLOCKED", "documentContentId missing"))?;
    for index in 0..101 {
        p.call(&format!("g3-document-comment-{index}"),"MCreateComment",json!({"input":{"id":p.id(&format!("document-comment-{index}")),"documentContentId":content_id,"body":format!("fixture document comment {index}")}})).await?;
    }
    let doc_comments = p
        .store
        .pages(
            "QArchiveDocumentComments",
            "/document/comments",
            json!({"id":doc}),
        )
        .await?;
    observed(doc_comments.len() >= 101, "document comments incomplete")?;
    p.call(
        "g3-reparent",
        "MUpdateDocument",
        json!({"id":doc,"input":{"issueId":survivor,"projectId":null}}),
    )
    .await?;
    p.call("g3-link","MCreateArtifact",json!({"input":{"id":p.id("link"),"issueId":issue,"title":"Historical link","url":"https://linear.app","metadata":{"fixture":true}}})).await?;
    p.call("g3-delete", "MArchiveProbeDelete", json!({"id":issue}))
        .await?;
    let retained = p.store.linear.object("QDocument", "document", &doc).await?;
    observed(
        retained["issue"]["id"] == survivor
            && markdown_equivalent(
                native["content"].as_str().unwrap_or(""),
                retained["content"].as_str().unwrap_or(""),
            ),
        "reparented document not retained",
    )?;
    let (read, _) = p.store.linear.get_asset(copied).await?;
    observed(
        format!("{:x}", Sha256::digest(&read)) == digest,
        "copied bytes changed",
    )?;
    let original_asset = p
        .store
        .linear
        .get_asset(original)
        .await
        .map(|(b, _)| json!({"digest":format!("{:x}",Sha256::digest(&b))}))
        .unwrap_or_else(|f| json!({"code":f.code}));
    let comments = p
        .store
        .pages("QArchiveComments", "/issue/comments", json!({"id":issue}))
        .await;
    let links = p.store.linear.attachments(issue).await;
    Ok(
        json!({"reparented_document":retained,"document_comments_before":doc_comments.len(),"copied_digest":digest,"original_asset_after":original_asset,"comments_after":comments.map(|v|json!(v)).unwrap_or_else(|f|json!({"code":f.code})),"attachments_after":links.map(|v|json!(v)).unwrap_or_else(|f|json!({"code":f.code}))}),
    )
}
/// Run all controlled gates on a fresh disposable Project; ordinary cargo test leaves this ignored.
#[tokio::test]
#[ignore = "native fixture writes require root-owned empirical acceptance and protected credentials"]
async fn archive_native_gates() {
    let mut p = Probe::new();
    for (purpose, key) in [
        ("project", "fixture_project"),
        ("sentinel-project", "sentinel_project"),
    ] {
        let args = json!({"input":{"id":p.report[key],"name":if purpose == "project" { "Archive capability fixture" } else { "Archive sentinel fixture" },"teamIds":[p.report["team"]],"description":"Disposable archive capability probe"}});
        p.call(&format!("create-{purpose}"), "MCreateProject", args)
            .await
            .expect("create isolated fixture Project");
    }
    let sentinel = p.issue("sentinel", None, true).await.expect("sentinel");
    let survivor = p.issue("survivor", None, false).await.expect("survivor");
    let issue = p.issue("subject", None, false).await.expect("subject");
    let child = p
        .issue("child", Some(&issue), false)
        .await
        .expect("live child");
    p.report["sentinel_before"] = p.sentinel(&sentinel).await.expect("initial sentinel");
    p.save();
    let r = gate_delete(&mut p, &child).await;
    p.gate("G1", r);
    let r = gate_sizes(&mut p, &issue).await;
    p.gate("G2", r);
    let r = gate_paging(&mut p, &issue, &survivor).await;
    p.gate("G4", r);
    let r = gate_drift(&mut p, &issue).await;
    p.gate("G5", r);
    let r = gate_metadata(&mut p, &survivor).await;
    p.gate("G6", r);
    let r = gate_retention(&mut p, &issue, &survivor).await;
    p.gate("G3", r);
    p.gate("G7",Ok(json!({"native_step_latencies":"steps.*.elapsed_ms","compact_per_item_budget":"UNKNOWN until compact preview/apply plus queued-call measurements"})));
    p.report["sentinel_after"] = p
        .sentinel(&sentinel)
        .await
        .expect("sentinel remains untouched");
    p.report["cleanup"] = json!(
        "Run archive_native_cleanup only after reviewing exact issue inventory; fixture Projects retained for evidence."
    );
    p.save();
}

/// Continue only the same owned report with bounded actual renderer/preservation probes.
/// Creates one managed Epic target, one source file, one copy and one readable proof Document;
/// reparents the existing size Document. It never deletes an Issue, repeats comment/history
/// volume, retries g6-1 or overwrites its pending journal. Root review/opt-in is required.
#[tokio::test]
#[ignore = "root-owned targeted existing fixture followup; no deletion"]
async fn archive_native_followup() {
    let mut p = Probe::resume();
    let sentinel = p.id("sentinel");
    p.sentinel(&sentinel)
        .await
        .expect("initial existing sentinel");
    owned_followup_issue(&p, "subject")
        .await
        .expect("owned subject");
    owned_followup_issue(&p, "survivor")
        .await
        .expect("owned survivor");
    let pending = p.report["steps"]["g6-1"].clone();
    observed(
        pending["operation"] == "MUpdateAttachment"
            && pending["state"] == "pending"
            && pending["variables"]["id"] == p.id("metadata"),
        "Expected pending metadata probe missing",
    )
    .expect("same run2 pending probe");
    let existing = p
        .store
        .linear
        .object("QArtifact", "attachment", &p.id("metadata"))
        .await
        .expect("exact metadata readback");
    observed(existing["issue"]["id"]==p.id("survivor")&&existing["metadata"]==p.report["steps"]["g6-0"]["result"]["attachmentCreate"]["attachment"]["metadata"],"Metadata no longer equals prior confirmed intent").expect("pending probe observation");
    p.report["followup_observations"]["g6-1"] = json!({"read_only":true,"current_json_bytes":serde_json::to_vec(&existing["metadata"]).unwrap().len(),"equals_previous_confirmed":true,"equals_unknown_proposal":existing["metadata"]==pending["variables"]["input"]["metadata"],"configured_total_metadata_budget":agent_tasks::archive::METADATA_BUDGET_BYTES});
    p.save();
    let epic = p.id("followup-epic");
    let input = json!({"request_id":epic,"actor":"codex:archive-fixture","project_id":p.report["fixture_project"],"team_id":p.report["team"],"title":"Archive retention fixture","fields":{"description":format!("Archive fixture {}",p.report["run"].as_str().unwrap()),"business_requirements":"Controlled native renderer/preservation measurement only","acceptance_criteria":"Exact payload and byte readback; no deletion"}});
    observed(
        p.report["steps"]["followup-create-epic"].is_null(),
        "Followup already attempted; do not recreate",
    )
    .unwrap();
    let fixture_project = p.report["fixture_project"].clone();
    p.report["issues"].as_array_mut().unwrap().push(
        json!({"id":epic,"purpose":"followup-epic","sentinel":false,"project":fixture_project}),
    );
    p.report["steps"]["followup-create-epic"] =
        json!({"operation":"create_epic","arguments":input,"state":"pending"});
    p.save();
    let gateway =
        agent_tasks::gateway::Gateway::new(p.store.linear.clone()).expect("fixture gateway");
    let created = gateway.call("create_epic", input).await;
    p.report["steps"]["followup-create-epic"]["outcome"] = serde_json::to_value(&created).unwrap();
    p.save();
    assert_eq!(
        created.status, "ok",
        "managed target creation failed; reconcile exact saved intent"
    );
    p.report["steps"]["followup-create-epic"]["state"] = json!("confirmed");
    p.save();
    let bytes = b"actual preservation fixture\0binary".to_vec();
    let digest = format!("{:x}", Sha256::digest(&bytes));
    let slot=p.call("followup-reserve-source","MFileUpload",json!({"contentType":"application/octet-stream","filename":"followup-resource.bin","size":bytes.len(),"makePublic":false})).await.expect("source reservation")["fileUpload"]["uploadFile"].clone();
    p.report["steps"]["followup-put-source"] =
        json!({"state":"pending","asset_url":slot["assetUrl"],"sha256":digest});
    p.save();
    p.store
        .linear
        .put_upload(&slot, bytes.clone())
        .await
        .expect("source PUT; unknown attempt stays recorded");
    p.report["steps"]["followup-put-source"]["state"] = json!("confirmed");
    p.save();
    let source = p.id("followup-source-file");
    let artifact = json!({"filename":"followup-resource.bin","content_type":"application/octet-stream","size_bytes":bytes.len(),"sha256":digest,"title":"Preservation source","note":null});
    p.call("followup-source-file","MCreateArtifact",json!({"input":{"id":source,"issueId":p.id("subject"),"title":"Preservation source","url":slot["assetUrl"],"metadata":{"artifact":artifact}}})).await.expect("source attachment");
    let origin = p
        .store
        .work(&p.id("subject"))
        .await
        .expect("source identity");
    let source_item = agent_tasks::archive::collect_item(&p.store, &origin)
        .await
        .expect("actual source snapshot");
    let root = p.store.work(&epic).await.expect("managed target");
    let mut set = agent_tasks::archive::ArchiveSet {
        epic: agent_tasks::archive::collect_item(&p.store, &root)
            .await
            .unwrap(),
        items: vec![],
        assets: vec![],
        blockers: vec![],
    };
    // Capture through the production collector after binding this managed source is intentionally avoided:
    // the primitive receives the exact owned native attachment and verified source bytes.
    let asset = agent_tasks::archive::ArchiveAsset {
        issue_id: p.id("subject"),
        identifier: origin.native["identifier"].as_str().unwrap().into(),
        origin: source.clone(),
        url: slot["assetUrl"].as_str().unwrap().into(),
        size: bytes.len() as u64,
        digest: digest.clone(),
        filename: "followup-resource.bin".into(),
        content_type: artifact["content_type"].as_str().unwrap().into(),
        artifact,
    };
    let action = agent_tasks::archive::PreservationAction::PrepareCopy {
        target_issue: epic.clone(),
        attachment_id: child_id(
            &epic,
            &format!("compact-file:{}:{}", asset.issue_id, asset.origin),
        ),
        asset,
    };
    let prepared = preservation_step(&mut p, "followup-prepare-copy", &action)
        .await
        .expect("actual prepare helper");
    let attach = prepared.next.expect("canonical publish action");
    preservation_step(&mut p, "followup-attach-copy", &attach)
        .await
        .expect("actual attach helper");
    agent_tasks::archive::preservation_readback(&p.store, &attach)
        .await
        .expect("actual copied byte/readback proof");
    let source_document = source_item
        .documents
        .iter()
        .find(|d| d["id"] == p.id("size-document"))
        .expect("existing size Document")
        .clone();
    let move_doc = agent_tasks::archive::PreservationAction::ReparentDocument {
        target_issue: epic.clone(),
        document: agent_tasks::archive::document_fingerprint(&source_document)
            .expect("compact exact witness"),
    };
    preservation_step(&mut p, "followup-reparent-document", &move_doc)
        .await
        .expect("actual reparent helper");
    agent_tasks::archive::preservation_readback(&p.store, &move_doc)
        .await
        .expect("actual reparent readback");
    set.epic = agent_tasks::archive::collect_item(&p.store, &p.store.work(&epic).await.unwrap())
        .await
        .unwrap();
    let expected = agent_tasks::archive::render(&set, None).expect("actual production renderer");
    assert!(
        expected.len() < 2 * 1024 * 1024,
        "fixed existing fixture exceeded followup document budget"
    );
    let doc = p.id("followup-renderer-document");
    p.call("followup-renderer-document","MCreateDocument",json!({"input":{"id":doc,"issueId":epic,"title":"Readable archive format proof","content":expected}})).await.expect("readable renderer Document");
    let native = p
        .store
        .linear
        .object("QDocument", "document", &doc)
        .await
        .expect("exact canonical renderer readback");
    agent_tasks::archive::verify_readback(&expected, native["content"].as_str().unwrap())
        .expect("exact source payload/structure proof; do not approve on hash alone");
    let blockers = agent_tasks::archive::validate_readback(
        &set,
        &expected,
        &native,
        agent_tasks::archive::ArchiveLimits {
            archive_max_bytes: Some(native["content"].as_str().unwrap().len()),
            section_max_bytes: Some(agent_tasks::render::TEXT_BUDGET_BYTES - 64 * 1024),
        },
    )
    .expect("canonical complete section/wire checks");
    assert!(blockers.is_empty(), "canonical readback bounds failed");
    p.report["followup_observations"]["renderer"] = json!({"document_id":doc,"request_bytes":expected.len(),"canonical_bytes":native["content"].as_str().unwrap().len(),"literal_payload_proof":true,"complete_section_replies":true,"canonical_content":native["content"],"no_deletion":true,"quota_relief":"UNPROVEN"});
    assert_eq!(
        p.report["steps"]["g6-1"], pending,
        "old pending journal must remain unchanged"
    );
    p.sentinel(&sentinel)
        .await
        .expect("unchanged existing sentinel");
    p.save();
}

/// Soft-clean only validated deterministic fixture Issue IDs; preserve the separate sentinel and Projects.
#[tokio::test]
#[ignore = "explicit cleanup of the saved fixture inventory only"]
async fn archive_native_cleanup() {
    assert_eq!(
        std::env::var("ATL_ARCHIVE_LIVE").as_deref(),
        Ok("disposable-fixture")
    );
    let path = PathBuf::from(std::env::var("ATL_LIVE_REPORT").expect("report"));
    let report: Value =
        serde_json::from_slice(&std::fs::read(&path).expect("read report")).expect("report");
    let team = std::env::var("ATL_LIVE_TEAM_ID").expect("team");
    assert_eq!(report["team"], team);
    let key = std::env::var("LINEAR_API_KEY")
        .ok()
        .or_else(|| {
            std::env::var("LINEAR_API_KEY_FILE").ok().map(|p| {
                std::fs::read_to_string(p)
                    .expect("protected provider")
                    .trim()
                    .to_owned()
            })
        })
        .expect("credential provider");
    let mut p = Probe {
        store: Store {
            linear: Linear::new(Some(key), false).expect("client"),
        },
        path,
        report,
    };
    assert_eq!(p.report["fixture_project"], p.id("project"));
    assert_eq!(p.report["sentinel_project"], p.id("sentinel-project"));
    let sentinel = p.id("sentinel");
    p.sentinel(&sentinel)
        .await
        .expect("sentinel before cleanup");
    let inventory = p.report["issues"].as_array().expect("inventory").clone();
    for item in inventory {
        if item["sentinel"] == true {
            continue;
        }
        let purpose = item["purpose"].as_str().expect("purpose");
        let id = item["id"].as_str().expect("id");
        assert_eq!(id, p.id(purpose));
        assert_eq!(item["project"], p.report["fixture_project"]);
        let native = p
            .store
            .linear
            .object("QArchiveIssue", "issue", id)
            .await
            .expect("exact cleanup read; missing is not confirmation");
        assert_eq!(native["project"]["id"], p.report["fixture_project"]);
        assert_eq!(native["team"]["id"], p.report["team"]);
        assert!(
            native["description"]
                .as_str()
                .is_some_and(|s| s.contains(p.report["run"].as_str().expect("run")))
        );
        if agent_tasks::archive::is_trashed(&native).expect("present native trash flag") {
            continue;
        }
        let step = cleanup_step(&p.report, purpose, &native)
            .expect("fresh untrashed identity is required for a new cleanup attempt");
        p.call(&step, "MArchiveProbeDelete", json!({"id":id}))
            .await
            .expect("soft cleanup");
        assert_eq!(
            p.store
                .linear
                .object("QArchiveIssue", "issue", id)
                .await
                .expect("exact trash confirmation")["trashed"],
            true
        );
    }
    p.report["sentinel_after_cleanup"] =
        p.sentinel(&sentinel).await.expect("sentinel after cleanup");
    p.save();
}
/// Non-live safety check: fixture IDs and bodies are deterministic and remain isolated.
#[test]
fn archive_fixture_identity_and_payload_are_bounded() {
    let run = "00000000-0000-4000-8000-000000000001";
    assert_ne!(child_id(run, "project"), child_id(run, "sentinel-project"));
    assert_ne!(child_id(run, "subject"), child_id(run, "sentinel"));
    let body = document_body(1024);
    assert!(body.len() >= 1024 && body.len() < 1200);
    assert!(agent_tasks::sections::find_section(&body, "literal").is_err());
    assert_eq!(child_id(run, "subject"), child_id(run, "subject"));
}

/// Signed PUT URLs and upload headers never reach a durable fixture report.
#[test]
fn archive_report_removes_signed_upload_credentials() {
    let raw = json!({"fileUpload":{"uploadFile":{"assetUrl":"https://uploads.linear.app/canonical","uploadUrl":"SIGNED_CANARY","headers":[{"value":"HEADER_CANARY"}]}}});
    let safe = evidence_response("MFileUpload", &raw).to_string();
    assert!(!safe.contains("SIGNED_CANARY") && !safe.contains("HEADER_CANARY"));
    assert!(safe.contains("canonical"));
    assert_eq!(
        raw["fileUpload"]["uploadFile"]["uploadUrl"],
        "SIGNED_CANARY"
    );
}

/// Probe workload cannot be expanded through an arbitrary size-count or aggregate environment override.
#[test]
fn archive_document_probe_has_fixed_count_and_aggregate_budget() {
    let sizes = document_sizes();
    assert_eq!(sizes.len(), 3);
    let bytes = sizes
        .into_iter()
        .map(|n| document_body(n).len())
        .try_fold(0usize, usize::checked_add)
        .expect("bounded aggregate");
    assert!(bytes < 2 * 1024 * 1024);
    assert!(sizes.into_iter().all(|n| n <= 1024 * 1024));
}
/// Failed cleanup intents can receive a new attempt only after a known fresh exact untrashed read.
#[test]
fn archive_cleanup_retries_only_after_exact_native_confirmation() {
    let run = "00000000-0000-4000-8000-000000000001";
    let report = json!({"run":run,"steps":{"cleanup-subject":{"state":"pending","fault":{"uncertain":true}},"cleanup-subject-attempt-1":{"state":"pending","fault":{"uncertain":false}}}});
    let native = json!({"id":child_id(run,"subject"),"trashed":false});
    assert_eq!(
        cleanup_step(&report, "subject", &native).unwrap(),
        "cleanup-subject-attempt-2"
    );
    assert!(
        cleanup_step(
            &report,
            "subject",
            &json!({"id":child_id(run,"subject"),"trashed":true})
        )
        .is_err()
    );
    assert!(
        cleanup_step(
            &report,
            "subject",
            &json!({"id":child_id(run,"sentinel"),"trashed":false})
        )
        .is_err()
    );
    assert!(cleanup_step(&report, "subject", &json!({"id":child_id(run,"subject")})).is_err());
    assert_eq!(
        report["steps"]["cleanup-subject"]["fault"]["uncertain"],
        true
    );
}

/// Fresh native null flags are valid for sentinel/cleanup; missing flags and positive trash remain distinct.
#[test]
fn archive_nullable_sentinel_and_cleanup_flags_are_explicit() {
    let run = "00000000-0000-4000-8000-000000000001";
    let report = json!({"run":run,"steps":{}});
    let project = json!("fixture-project");
    let sentinel = json!({"id":child_id(run,"sentinel"),"project":{"id":project},"title":"Archive gate fixture sentinel","trashed":null});
    validate_sentinel(&sentinel, &project).unwrap();
    let native = json!({"id":child_id(run,"subject"),"trashed":null});
    assert_eq!(
        cleanup_step(&report, "subject", &native).unwrap(),
        "cleanup-subject-attempt-1"
    );
    assert!(
        !agent_tasks::archive::is_trashed(&native).unwrap(),
        "null never confirms deletion"
    );
    let mut missing = sentinel.clone();
    missing.as_object_mut().unwrap().remove("trashed");
    assert_eq!(
        validate_sentinel(&missing, &project).unwrap_err().code,
        "INCOMPLETE_DATA"
    );
    let mut trashed = sentinel;
    trashed["trashed"] = json!(true);
    assert_eq!(
        validate_sentinel(&trashed, &project).unwrap_err().code,
        "SENTINEL_CHANGED"
    );
    assert!(agent_tasks::archive::is_trashed(&trashed).unwrap());
    let mut missing = native;
    missing.as_object_mut().unwrap().remove("trashed");
    assert_eq!(
        cleanup_step(&report, "subject", &missing).unwrap_err().code,
        "INCOMPLETE_DATA"
    );
}

/// Existing-report followups reject another run/team or changed owned identity without clearing old pending evidence.
#[test]
fn archive_followup_inventory_is_exact_and_pending_is_preserved() {
    let run = "00000000-0000-4000-8000-000000000001";
    let team = "00000000-0000-4000-8000-000000000002";
    let project = child_id(run, "project");
    let sentinel_project = child_id(run, "sentinel-project");
    let mut report = json!({"version":1,"run":run,"team":team,"fixture_project":project,"sentinel_project":sentinel_project,"issues":[],
        "steps":{"g6-1":{"state":"pending","fault":{"uncertain":true}}}});
    for purpose in ["subject", "survivor", "sentinel"] {
        let is_sentinel = purpose == "sentinel";
        report["issues"].as_array_mut().unwrap().push(json!({"purpose":purpose,"id":child_id(run,purpose),"sentinel":is_sentinel,"project":if is_sentinel {&sentinel_project}else{&project}}));
    }
    let pending = report["steps"]["g6-1"].clone();
    validate_existing_report(&report, team).unwrap();
    assert_eq!(report["steps"]["g6-1"], pending);
    assert!(validate_existing_report(&report, "another team").is_err());
    report["issues"][0]["id"] = json!(child_id(run, "sentinel"));
    assert!(validate_existing_report(&report, team).is_err());
}
