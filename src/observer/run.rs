//! Bounded blocking Agent Run socket calls and execution-pinned analysis answers.

use std::collections::HashSet;
use std::io::{Cursor, Read, Write};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use super::error::{Error, Result};

/// Maximum encoded request or response frame, including its terminating newline.
const FRAME_BYTES: usize = 1024 * 1024;
/// Absolute wall-clock allowance for one ordinary socket call.
const CALL_BUDGET: Duration = Duration::from_secs(5);
/// Shared allowance for pagination, transcripts, or both answer-fencing reads.
const OBSERVATION_BUDGET: Duration = Duration::from_secs(15);
/// Socket pagination size, independent of the smaller MCP discovery limit.
const PAGE_SIZE: usize = 64;
/// Maximum pages inspected in one bounded observation; excess remains unknown.
const PAGE_LIMIT: usize = 16;
/// Maximum relevant workers whose transcript is included in one input bundle.
const DEVELOPER_LIMIT: usize = 16;
/// Maximum retained UTF-8 bytes from one worker's transcript messages.
const TRANSCRIPT_BYTES: usize = 8 * 1024;
/// Caps per-worker metadata even when upstream messages contain empty content.
const TRANSCRIPT_EVENTS: usize = 32;
/// Maximum retained descriptive task summary; identifiers are never shortened.
const SUMMARY_BYTES: usize = 2048;
/// Bounds connecting threads even when a full Unix backlog stalls the OS connect.
static CONNECTING: AtomicBool = AtomicBool::new(false);

/// Local socket location; calls own separate connections and never resume agents.
pub struct Client {
    /// Existing Agent Run API socket; construction performs no I/O.
    socket: PathBuf,
}

/// Stable accepted execution identity, persisted together with the original request.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Receipt {
    /// Exact Agent Run identity; it is not a task summary or display label.
    pub agent_id: String,
    /// Positive admission sequence, required again before and after reading an answer.
    pub sequence: u32,
}

/// Required public execution facts; additions to the upstream DTO are tolerated.
#[derive(Debug, Deserialize)]
struct Agent {
    /// Exact stable run identity.
    agent_id: String,
    /// Immutable upstream creation time; absent historical fixtures retain deterministic ordering.
    #[serde(default)]
    created_at: Option<f64>,
    /// Latest execution start, distinct from the stable agent's original creation time.
    #[serde(default)]
    started_at: Option<f64>,
    /// Current execution sequence, which may differ after an explicit resume.
    sequence: u32,
    /// Actual workdir, absent when upstream has no repository association.
    workdir: Option<PathBuf>,
    /// Abbreviated native task description, never treated as the original task.
    task_summary: String,
    /// Runtime profile, used to exclude dedicated summary runs.
    profile: String,
    /// Closed execution status interpreted by `execution_state`.
    status: String,
    /// Current native execution phase.
    phase: String,
    /// Optional health facts, filtered before entering semantic evidence.
    health: Option<Value>,
    /// Verified latest answer digest, absent before a verified answer exists.
    answer_sha256: Option<String>,
}

/// One required list page; observation clocks and delivery counters are discarded.
#[derive(Deserialize)]
struct Page {
    /// Public run views in server order.
    items: Vec<Agent>,
    /// Full upstream inventory size at this revision.
    total: usize,
    /// Offset of the first unseen row, or null at the end.
    next_offset: Option<usize>,
    /// Whether this page reaches the end of the matching inventory.
    complete: bool,
    /// Inventory revision used for complete observations, not unrelated point-read fencing.
    revision: Value,
}

/// Borrowed wire request; serialization cannot clone an unbounded parameter tree.
#[derive(Serialize)]
struct Request<'a> {
    /// Fixed protocol version accepted by the local socket API.
    jsonrpc: &'static str,
    /// Request ID scoped to this fresh single-request connection.
    id: u8,
    /// Exact dispatcher method, with no MCP tool-name translation.
    method: &'a str,
    /// Original caller parameters, borrowed without modification.
    params: &'a Value,
}

impl Client {
    /// Event-wait on a separate socket for this unbound API execution, without a job timeout
    /// or status-poll loop. The response is only a wakeup; result() still verifies all fences.
    /// EOF/broker restart leaves the saved analysis untouched for an explicit recovery request.
    pub fn wait(&self, receipt: &Receipt) -> Result<()> {
        use std::io::{BufRead, BufReader};
        validate_receipt(receipt)?;
        let deadline = Instant::now() + CALL_BUDGET;
        let mut socket = connect(&self.socket, deadline)?;
        let mut frame = serde_json::to_vec(
            &json!({"jsonrpc":"2.0","id":1,"method":"wait","params":{"agent_id":receipt.agent_id}}),
        )?;
        frame.push(b'\n');
        write_frame(&mut socket, &frame, deadline)?;
        socket.set_nonblocking(false).map_err(io_error)?;
        let mut response = Vec::new();
        BufReader::new(socket)
            .take(FRAME_BYTES as u64 + 1)
            .read_until(b'\n', &mut response)
            .map_err(io_error)?;
        if response.len() > FRAME_BYTES || !response.ends_with(b"\n") {
            return Err(Error::new(
                "run_protocol",
                "Invalid completion wakeup frame.",
            ));
        }
        let value: Value = serde_json::from_slice(&response)?;
        if value["jsonrpc"] != "2.0"
            || value["id"] != 1
            || value.get("error").is_some()
            || !value["result"].is_object()
            || value["result"]["timed_out"] == true
            || value["result"]["agent_id"] != receipt.agent_id
            || !matches!(
                value["result"]["status"].as_str(),
                Some("succeeded" | "failed" | "timed_out" | "cancelled" | "lost")
            )
        {
            return Err(Error::new(
                "run_protocol",
                "Completion wakeup was not confirmed.",
            ));
        }
        Ok(())
    }
    /// Retains an existing socket path without opening it or changing configuration.
    pub fn new(socket: PathBuf) -> Self {
        Self { socket }
    }

    /// Sends one object-parameter JSON-RPC 2 request and validates its exact ID.
    /// Both frames are bounded to 1 MiB and the entire call to five seconds.
    /// Admission may have happened after a transport error; callers must recover
    /// `start` by replaying their exact persisted request, never a replacement key.
    /// Diagnostics contain stable codes and fixed prose, never upstream bodies.
    pub fn call(&self, method: &str, params: &Value) -> Result<Value> {
        self.call_until(method, params, Instant::now() + CALL_BUDGET)
    }

    /// Submits the caller's exact durable start parameters once, without defaults,
    /// namespace changes, retry, or resume. Requires a nonempty ID and positive
    /// u32 sequence; other acceptance fields remain owned by the caller/API.
    pub fn submit(&self, params: &Value) -> Result<Receipt> {
        let value = self.call("start", params)?;
        let receipt: Receipt = decode(value)?;
        validate_receipt(&receipt)?;
        Ok(receipt)
    }

    /// Observes workers in the canonical Git common directory, excluding supplied
    /// analyzer IDs and summary profiles. Returns stable allowlisted facts plus
    /// bounded transcript/check messages and sequence/hash-fenced sealed answers. Historical
    /// failures outside this repository's active episode are explicitly omitted. Retained answer
    /// bytes do not independently verify their assertions. Uses `complete=false`
    /// for missing workdirs/logs, inaccessible Git state, or collection ceilings.
    /// Matching workdir alone does not prove relevance to a particular plan.
    /// Never changes source, starts an agent, or includes observation/delivery clocks.
    pub fn developers(&self, common_dir: &Path, excluded: &[String]) -> Result<Value> {
        let deadline = Instant::now() + OBSERVATION_BUDGET;
        let common_dir = common_dir.canonicalize().map_err(io_error)?;
        let (mut agents, mut complete) = self.agents(deadline, None)?;
        agents.sort_by(|a, b| {
            worker_priority(&a.status)
                .cmp(&worker_priority(&b.status))
                .then_with(|| {
                    b.created_at
                        .partial_cmp(&a.created_at)
                        .unwrap_or(std::cmp::Ordering::Equal)
                })
                .then_with(|| b.agent_id.cmp(&a.agent_id))
        });
        let mut omissions = Vec::new();
        if !complete {
            omissions.push(json!({"reason": "agent_inventory_incomplete"}));
        }
        let active_since = agents
            .iter()
            .filter(|agent| {
                worker_priority(&agent.status) == 0
                    && !excluded.contains(&agent.agent_id)
                    && !matches!(agent.profile.as_str(), "summary" | "progress-summary")
                    && !agent
                        .task_summary
                        .starts_with("You are a read-only progress summarizer.")
            })
            .filter_map(|agent| {
                let identity =
                    super::git::identify(agent.workdir.as_deref()?, Some("HEAD")).ok()?;
                (identity.common_dir == common_dir)
                    .then_some(agent.started_at.or(agent.created_at))
                    .flatten()
            })
            .min_by(f64::total_cmp);
        let mut older_failures = 0;
        let mut items = Vec::new();
        for agent in agents {
            if Instant::now() >= deadline {
                complete = false;
                omissions.push(json!({"reason": "developer_observation_deadline"}));
                break;
            }
            if excluded.contains(&agent.agent_id)
                || matches!(agent.profile.as_str(), "summary" | "progress-summary")
                || agent
                    .task_summary
                    .starts_with("You are a read-only progress summarizer.")
            {
                continue;
            }
            let Some(workdir) = agent.workdir.as_deref() else {
                continue;
            };
            let identity = match super::git::identify(workdir, Some("HEAD")) {
                Ok(identity) => identity,
                Err(_) => continue,
            };
            if identity.common_dir != common_dir {
                continue;
            }
            if worker_priority(&agent.status) == 2
                && active_since.is_some_and(|since| {
                    agent
                        .started_at
                        .or(agent.created_at)
                        .is_some_and(|created| created < since)
                })
            {
                older_failures += 1;
                continue;
            }
            if items.len() == DEVELOPER_LIMIT || Instant::now() >= deadline {
                complete = false;
                omissions.push(json!({"agent_id": agent.agent_id, "reason": "developer_budget"}));
                continue;
            }
            let (summary, omitted) = bounded_text(&agent.task_summary, SUMMARY_BYTES);
            let mut row = json!({
                "agent_id": agent.agent_id,
                "sequence": agent.sequence,
                "workdir": workdir,
                "task_summary": summary,
                "task_summary_omitted_bytes": omitted,
                "profile": agent.profile,
                "status": agent.status,
                "phase": agent.phase,
                "health": stable_health(agent.health.as_ref()),
                "answer_sha256": agent.answer_sha256,
            });
            if omitted > 0 {
                complete = false;
            }
            match self.call_until(
                "transcript",
                &json!({"agent_id": agent.agent_id, "view": "blocks", "tail_blocks": 32}),
                deadline.min(Instant::now() + CALL_BUDGET),
            ) {
                Ok(transcript) => match transcript_evidence(transcript, &agent.agent_id) {
                    Ok((events, covered)) => {
                        row["transcript"] = events;
                        complete &= covered;
                    }
                    Err(_) => {
                        complete = false;
                        row["transcript"] = Value::Null;
                        omissions.push(
                            json!({"agent_id": agent.agent_id, "reason": "transcript_invalid"}),
                        );
                    }
                },
                Err(_) => {
                    complete = false;
                    row["transcript"] = Value::Null;
                    omissions.push(
                        json!({"agent_id": agent.agent_id, "reason": "transcript_unavailable"}),
                    );
                }
            }
            if agent.status == "succeeded" && agent.answer_sha256.is_some() {
                match self.answer_content(&agent, deadline) {
                    Ok(content) => {
                        let (excerpt, omitted) = bounded_text(&content, TRANSCRIPT_BYTES);
                        row["sealed_answer"] = json!({"content":excerpt,"source_sha256":agent.answer_sha256,"sequence":agent.sequence,"omitted_bytes":omitted,"meaning":"Retained answer bytes verified; assertions about tests/source still require matching evidence."});
                        complete &= omitted == 0;
                    }
                    Err(error) => {
                        complete = false;
                        omissions.push(json!({"agent_id":agent.agent_id,"reason":"worker_answer_unavailable","code":error.code}));
                    }
                }
            }
            items.push(row);
        }
        if older_failures > 0 {
            omissions.push(json!({"reason":"older_terminal_failures_outside_current_episode","count":older_failures}));
            complete = false;
        }
        items.sort_by(|left, right| left["agent_id"].as_str().cmp(&right["agent_id"].as_str()));
        omissions.sort_by_key(Value::to_string);
        Ok(json!({"items": items, "complete": complete, "omissions": omissions}))
    }

    /// Observes one original execution once; active statuses return `None`.
    /// Terminal failures return `analysis_failed`; terminal success without a
    /// complete verified inline answer returns `answer_unavailable`, never pending.
    /// Requires identical original sequence/hash before and after `answer`, checks
    /// the actual content digest, and accepts only JSON with the exact input ID and
    /// plan revision. Replaced executions or changed identities are `answer_mismatch`.
    /// No completion loop, resume, agent launch, or current-snapshot substitution occurs.
    pub fn result(
        &self,
        receipt: &Receipt,
        input_id: &str,
        plan_revision: u64,
    ) -> Result<Option<Value>> {
        validate_receipt(receipt)?;
        let deadline = Instant::now() + OBSERVATION_BUDGET;
        let before = self.execution(receipt, deadline)?;
        if !execution_state(&before.status)? {
            return Ok(None);
        }
        let content = self.answer_content(&before, deadline)?;
        let output: Value = serde_json::from_str(&content)
            .map_err(|_| Error::new("analysis_invalid", "Analysis answer is not valid JSON."))?;
        if output.get("analysis_input_id").and_then(Value::as_str) != Some(input_id)
            || output.get("plan_revision").and_then(Value::as_u64) != Some(plan_revision)
        {
            return Err(answer_mismatch());
        }
        Ok(Some(output))
    }

    /// Read one successful execution's sealed text with exact sequence/hash fences before and after.
    /// The returned bytes may contain untrusted developer assertions, never acceptance authority.
    fn answer_content(&self, before: &Agent, deadline: Instant) -> Result<String> {
        let hash = before
            .answer_sha256
            .as_deref()
            .ok_or_else(answer_unavailable)?;
        let answer = self.call_until(
            "answer",
            &json!({"agent_id": before.agent_id}),
            deadline.min(Instant::now() + CALL_BUDGET),
        )?;
        if answer.get("agent_id").and_then(Value::as_str) != Some(before.agent_id.as_str()) {
            return Err(answer_mismatch());
        }
        if answer.get("available").and_then(Value::as_bool) != Some(true)
            || answer.get("inline_complete").and_then(Value::as_bool) != Some(true)
            || answer.get("status").and_then(Value::as_str) != Some("succeeded")
        {
            return Err(answer_unavailable());
        }
        if answer.get("sha256").and_then(Value::as_str) != Some(hash) {
            return Err(answer_mismatch());
        }
        let content = answer
            .get("content")
            .and_then(Value::as_str)
            .ok_or_else(answer_unavailable)?;
        if format!("{:x}", Sha256::digest(content.as_bytes())) != hash {
            return Err(answer_mismatch());
        }
        let after = self.execution(
            &Receipt {
                agent_id: before.agent_id.clone(),
                sequence: before.sequence,
            },
            deadline,
        )?;
        if after.status != "succeeded" || after.answer_sha256.as_deref() != Some(hash) {
            return Err(answer_mismatch());
        }
        Ok(content.to_owned())
    }

    /// Performs one connection/frame exchange under a caller's absolute deadline.
    /// A fresh connection permits constant request ID 1 without multiplexing races.
    fn call_until(&self, method: &str, params: &Value, deadline: Instant) -> Result<Value> {
        if method.is_empty() || !params.is_object() {
            return Err(Error::new(
                "run_request_invalid",
                "Agent Run requires a method and object parameters.",
            ));
        }
        let mut frame = vec![0; FRAME_BYTES];
        let length = {
            let mut output = Cursor::new(frame.as_mut_slice());
            serde_json::to_writer(
                &mut output,
                &Request {
                    jsonrpc: "2.0",
                    id: 1,
                    method,
                    params,
                },
            )
            .map_err(|_| {
                Error::new(
                    "run_request_too_large",
                    "Agent Run request exceeds its 1 MiB frame limit.",
                )
            })?;
            usize::try_from(output.position()).map_err(|_| {
                Error::new(
                    "run_request_too_large",
                    "Agent Run request exceeds its frame limit.",
                )
            })?
        };
        if length >= FRAME_BYTES {
            return Err(Error::new(
                "run_request_too_large",
                "Agent Run request exceeds its 1 MiB frame limit.",
            ));
        }
        frame.truncate(length);
        frame.push(b'\n');
        let mut socket = connect(&self.socket, deadline)?;
        write_frame(&mut socket, &frame, deadline)?;
        let mut response = Vec::new();
        let mut chunk = [0; 8192];
        loop {
            remaining(deadline)?;
            let count = match socket.read(&mut chunk) {
                Ok(count) => count,
                Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    wait_io(deadline)?;
                    continue;
                }
                Err(error)
                    if matches!(
                        error.kind(),
                        std::io::ErrorKind::UnexpectedEof
                            | std::io::ErrorKind::ConnectionReset
                            | std::io::ErrorKind::ConnectionAborted
                    ) =>
                {
                    return Err(Error::new(
                        "run_protocol",
                        "Agent Run closed an incomplete response frame.",
                    ));
                }
                Err(error) => return Err(io_error(error)),
            };
            if count == 0 {
                return Err(Error::new(
                    "run_protocol",
                    "Agent Run response has no complete newline frame.",
                ));
            }
            if response.len() + count > FRAME_BYTES {
                return Err(Error::new(
                    "run_response_too_large",
                    "Agent Run response exceeds its 1 MiB frame limit.",
                ));
            }
            response.extend_from_slice(&chunk[..count]);
            if let Some(end) = chunk[..count].iter().position(|byte| *byte == b'\n') {
                if end + 1 != count {
                    return Err(Error::new(
                        "run_protocol",
                        "Agent Run returned trailing response bytes.",
                    ));
                }
                break;
            }
            if response.len() == FRAME_BYTES {
                return Err(Error::new(
                    "run_response_too_large",
                    "Agent Run response exceeds its 1 MiB frame limit.",
                ));
            }
        }
        let mut value: Value = serde_json::from_slice(&response).map_err(|_| {
            Error::new(
                "run_protocol",
                "Agent Run response is not valid UTF-8 JSON.",
            )
        })?;
        if value.get("jsonrpc").and_then(Value::as_str) != Some("2.0")
            || value.get("id").and_then(Value::as_u64) != Some(1)
            || value.get("result").is_some() == value.get("error").is_some()
        {
            return Err(Error::new(
                "run_protocol",
                "Agent Run response envelope or request ID is invalid.",
            ));
        }
        if let Some(error) = value.get("error") {
            if matches!(
                error.get("code").and_then(Value::as_i64),
                Some(-32700 | -32600 | -32601 | -32602)
            ) {
                return Err(Error::new(
                    "run_request_invalid",
                    "Agent Run rejected the request before dispatch.",
                ));
            }
            let domain_code = error
                .pointer("/data/code")
                .or_else(|| error.get("code"))
                .and_then(Value::as_str);
            let code = domain_code
                .filter(|code| {
                    !code.is_empty()
                        && code.len() <= 64
                        && code.bytes().all(|byte| {
                            byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b'.')
                        })
                })
                .unwrap_or("run_rpc_error");
            return Err(Error::new(code, "Agent Run rejected the API request."));
        }
        Ok(value["result"].take())
    }

    /// Collect bounded inventory pages; moving inventories retain validated partial views.
    /// a point read returns immediately on the exact sought ID and is fenced by that
    /// execution's own sequence/hash, not unrelated worker or delivery changes.
    /// A moving point scan that misses its target remains incomplete, never proof of loss.
    fn agents(&self, deadline: Instant, sought: Option<&str>) -> Result<(Vec<Agent>, bool)> {
        let mut agents = Vec::new();
        let mut seen = HashSet::new();
        let mut offset = 0;
        let mut revision = None;
        let mut changed = false;
        for _ in 0..PAGE_LIMIT {
            let page: Page = decode(self.call_until(
                "list_agents",
                &json!({"limit": PAGE_SIZE, "offset": offset}),
                deadline.min(Instant::now() + CALL_BUDGET),
            )?)?;
            let moved = revision.as_ref().is_some_and(|old| old != &page.revision);
            if page.revision.is_null() {
                return Err(Error::new(
                    "run_changed",
                    "Agent Run inventory changed during pagination.",
                ));
            }
            changed |= moved;
            revision = Some(page.revision);
            if page.items.len() > PAGE_SIZE {
                return Err(protocol_data());
            }
            if page.total < offset + page.items.len() {
                return if changed {
                    Ok((agents, false))
                } else {
                    Err(protocol_data())
                };
            }
            let next = offset + page.items.len();
            for agent in page.items {
                if !valid_identifier(&agent.agent_id)
                    || agent.sequence == 0
                    || !valid_identifier(&agent.profile)
                    || !valid_identifier(&agent.status)
                    || !valid_identifier(&agent.phase)
                {
                    return Err(protocol_data());
                }
                if sought == Some(agent.agent_id.as_str()) {
                    return Ok((vec![agent], false));
                }
                if !seen.insert(agent.agent_id.clone()) {
                    if sought.is_none() && !changed {
                        return Err(protocol_data());
                    }
                    if let Some(known) = agents
                        .iter_mut()
                        .find(|known| known.agent_id == agent.agent_id)
                        && agent.sequence >= known.sequence
                    {
                        *known = agent;
                    }
                    changed = true;
                    continue;
                }
                agents.push(agent);
            }
            match page.next_offset {
                Some(cursor)
                    if !page.complete
                        && cursor == next
                        && cursor > offset
                        && cursor < page.total =>
                {
                    offset = cursor
                }
                None if next == page.total && page.complete => return Ok((agents, !changed)),
                _ if changed => return Ok((agents, false)),
                _ => return Err(protocol_data()),
            }
        }
        Ok((agents, false))
    }

    /// Finds the original admitted sequence; never falls through to a newer run.
    fn execution(&self, receipt: &Receipt, deadline: Instant) -> Result<Agent> {
        let (agents, complete) = self.agents(deadline, Some(&receipt.agent_id))?;
        match agents
            .into_iter()
            .find(|agent| agent.agent_id == receipt.agent_id)
        {
            Some(agent) if agent.sequence == receipt.sequence => Ok(agent),
            Some(_) => Err(answer_mismatch()),
            None if !complete => Err(Error::new(
                "run_unavailable",
                "Agent Run inventory is incomplete; original execution is unknown.",
            )),
            None => Err(Error::new(
                "answer_unavailable",
                "Original analysis execution is no longer available.",
            )),
        }
    }
}

/// Write one bounded request under an absolute deadline, preserving partial progress.
/// Uses nonblocking I/O rather than platform socket timeout setters on closing Unix peers.
fn write_frame(socket: &mut UnixStream, frame: &[u8], deadline: Instant) -> Result<()> {
    socket.set_nonblocking(true).map_err(io_error)?;
    let mut written = 0;
    while written < frame.len() {
        remaining(deadline)?;
        let count = match socket.write(&frame[written..]) {
            Ok(count) => count,
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                wait_io(deadline)?;
                continue;
            }
            Err(error) => return Err(io_error(error)),
        };
        if count == 0 {
            return Err(Error::new(
                "run_unavailable",
                "Agent Run closed the request connection.",
            ));
        }
        written += count;
    }
    Ok(())
}

/// Connects with a bounded wait, retaining at most one stalled OS connector.
/// The worker owns its stream and drops it if the receiver has timed out.
fn connect(path: &Path, deadline: Instant) -> Result<UnixStream> {
    remaining(deadline)?;
    if CONNECTING
        .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
        .is_err()
    {
        return Err(Error::new(
            "run_busy",
            "An Agent Run socket connection is already pending.",
        ));
    }
    let (send, receive) = mpsc::sync_channel(1);
    let path = path.to_owned();
    // ponytail: one global connector bounds backlog-stalled threads; use a native
    // nonblocking socket dependency only if concurrent admission becomes necessary.
    if std::thread::Builder::new()
        .name("observer-run-connect".into())
        .spawn(move || {
            let result = UnixStream::connect(path);
            CONNECTING.store(false, Ordering::Release);
            let _ = send.send(result);
        })
        .is_err()
    {
        CONNECTING.store(false, Ordering::Release);
        return Err(Error::new(
            "run_unavailable",
            "Cannot start the Agent Run socket connector.",
        ));
    }
    receive
        .recv_timeout(remaining(deadline)?)
        .map_err(|error| match error {
            mpsc::RecvTimeoutError::Timeout => timeout(),
            mpsc::RecvTimeoutError::Disconnected => {
                Error::new("run_unavailable", "Agent Run socket connector stopped.")
            }
        })?
        .map_err(io_error)
}

/// Returns strictly positive time left, preventing slow frames from resetting a deadline.
fn remaining(deadline: Instant) -> Result<Duration> {
    deadline
        .checked_duration_since(Instant::now())
        .filter(|left| !left.is_zero())
        .ok_or_else(timeout)
}

/// Maps socket failures without including paths, server bodies, or private OS prose.
fn io_error(error: std::io::Error) -> Error {
    match error.kind() {
        std::io::ErrorKind::TimedOut | std::io::ErrorKind::WouldBlock => timeout(),
        _ => Error::new("run_unavailable", "Agent Run socket I/O is unavailable."),
    }
}

/// Stable timeout error; a timed-out start can still have been admitted upstream.
fn timeout() -> Error {
    Error::new(
        "run_timeout",
        "Agent Run socket observation exceeded its deadline; admission may be uncertain.",
    )
}

/// Decodes only required external fields, keeping malformed payloads out of diagnostics.
fn decode<T: serde::de::DeserializeOwned>(value: Value) -> Result<T> {
    serde_json::from_value(value).map_err(|_| protocol_data())
}

/// Safe schema error independent of arbitrary upstream values and deserializer excerpts.
fn protocol_data() -> Error {
    Error::new(
        "run_protocol",
        "Agent Run response data is missing or invalid.",
    )
}

/// Validates passive receipt fields again after deserialization or persisted reload.
fn validate_receipt(receipt: &Receipt) -> Result<()> {
    if !valid_identifier(&receipt.agent_id) || receipt.sequence == 0 {
        Err(protocol_data())
    } else {
        Ok(())
    }
}

/// Accepts exact bounded upstream identifiers without truncation or control bytes.
fn valid_identifier(value: &str) -> bool {
    !value.is_empty() && value.len() <= 256 && !value.chars().any(char::is_control)
}

/// Distinguishes actual active statuses from success and all documented failures.
fn execution_state(status: &str) -> Result<bool> {
    match status {
        "created" | "starting" | "running" | "cancelling" => Ok(false),
        "succeeded" => Ok(true),
        "failed" | "timed_out" | "cancelled" | "lost" => Err(Error::new(
            "analysis_failed",
            "Analysis execution terminated without success.",
        )),
        _ => Err(protocol_data()),
    }
}

/// Explicitly unavailable terminal output, distinct from a still-running analysis.
fn answer_unavailable() -> Error {
    Error::new(
        "answer_unavailable",
        "Successful analysis has no complete verified inline answer.",
    )
}

/// Rejects replaced execution, hash drift, content corruption, or wrong input identity.
fn answer_mismatch() -> Error {
    Error::new(
        "answer_mismatch",
        "Analysis execution, answer hash, or input identity does not match the saved request.",
    )
}

/// Active execution evidence wins observation capacity; successful answers precede old failures.
fn worker_priority(status: &str) -> u8 {
    match status {
        "created" | "starting" | "running" | "cancelling" => 0,
        "succeeded" => 1,
        _ => 2,
    }
}

/// Prefer concrete check results and final summaries; this selects evidence, never certifies tests.
fn transcript_priority(message: &Value) -> u8 {
    let content = message.get("content").and_then(Value::as_str).unwrap_or("");
    if message.get("role").and_then(Value::as_str) == Some("tool_result")
        && [
            "test result:",
            " passed",
            " failed",
            "Exit code:",
            "exit_code",
            "returncode",
        ]
        .iter()
        .any(|needle| content.contains(needle))
    {
        0
    } else if message.get("role").and_then(Value::as_str) == Some("assistant")
        && ["CHECKS", "Checks:", "UNFINISHED", "Remaining"]
            .iter()
            .any(|needle| content.contains(needle))
    {
        1
    } else {
        2
    }
}

/// Keeps stable health state only; silence durations and observation clocks are omitted.
fn stable_health(health: Option<&Value>) -> Value {
    match health {
        Some(Value::String(state)) if valid_identifier(state) => json!(state),
        Some(Value::Object(fields)) => {
            let selected = ["state", "status", "kind", "reason_code"]
                .into_iter()
                .filter_map(|key| {
                    fields
                        .get(key)
                        .and_then(Value::as_str)
                        .filter(|state| valid_identifier(state))
                        .map(|state| (key.to_owned(), json!(state)))
                })
                .collect::<serde_json::Map<_, _>>();
            Value::Object(selected)
        }
        _ => Value::Null,
    }
}

/// Returns a UTF-8-safe descriptive prefix and exact omitted byte count.
/// Callers must expose the omission; identifiers never pass through this helper.
fn bounded_text(text: &str, maximum: usize) -> (&str, usize) {
    let mut end = text.len().min(maximum);
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    (&text[..end], text.len() - end)
}

/// Retains bounded native transcript events with their actual coverage metadata,
/// excluding timestamps, delivery state, raw log paths, and observation cursors.
/// Missing or malformed logs are errors, never evidence that no work happened.
fn transcript_evidence(value: Value, agent_id: &str) -> Result<(Value, bool)> {
    if value.get("agent_id").and_then(Value::as_str) != Some(agent_id) {
        return Err(protocol_data());
    }
    let messages = value
        .get("messages")
        .and_then(Value::as_array)
        .ok_or_else(protocol_data)?;
    let mut complete = value
        .get("complete")
        .and_then(Value::as_bool)
        .ok_or_else(protocol_data)?;
    let mut bytes_left = TRANSCRIPT_BYTES;
    let mut events = Vec::new();
    let mut selected: Vec<_> = messages.iter().collect();
    selected.sort_by_key(|message| {
        (
            transcript_priority(message),
            std::cmp::Reverse(message.get("seq").and_then(Value::as_u64).unwrap_or(0)),
        )
    });
    for message in selected.into_iter().take(TRANSCRIPT_EVENTS) {
        let content = message
            .get("content")
            .and_then(Value::as_str)
            .ok_or_else(protocol_data)?;
        let role = message
            .get("role")
            .and_then(Value::as_str)
            .ok_or_else(protocol_data)?;
        if !valid_identifier(role) {
            return Err(protocol_data());
        }
        let seq = message
            .get("seq")
            .and_then(Value::as_u64)
            .ok_or_else(protocol_data)?;
        if bytes_left == 0 {
            complete = false;
            continue;
        }
        let (content, omitted) = bounded_text(content, bytes_left);
        bytes_left -= content.len();
        let mut event = json!({"seq": seq, "role": role, "content": content, "observer_omitted_bytes": omitted});
        for field in ["name", "error_source"] {
            if let Some(value) = message.get(field) {
                if !value.is_null() && !value.as_str().is_some_and(valid_identifier) {
                    return Err(protocol_data());
                }
                event[field] = value.clone();
            }
        }
        for field in ["first_seq", "last_seq", "omitted_bytes"] {
            if let Some(value) = message.get(field) {
                if !value.is_null() && value.as_u64().is_none() {
                    return Err(protocol_data());
                }
                event[field] = value.clone();
            }
        }
        for field in [
            "error",
            "partial_before",
            "partial_after",
            "content_complete",
            "starts_block",
        ] {
            if let Some(value) = message.get(field) {
                if !value.is_null() && value.as_bool().is_none() {
                    return Err(protocol_data());
                }
                event[field] = value.clone();
            }
        }
        if omitted > 0
            || message.get("content_complete").and_then(Value::as_bool) == Some(false)
            || message.get("partial_before").and_then(Value::as_bool) == Some(true)
            || message.get("partial_after").and_then(Value::as_bool) == Some(true)
            || message
                .get("omitted_bytes")
                .and_then(Value::as_u64)
                .is_some_and(|bytes| bytes > 0)
        {
            complete = false;
        }
        events.push(event);
    }
    events.sort_by_key(|message| message["seq"].as_u64());
    let omitted_messages = messages.len() - events.len();
    complete &= omitted_messages == 0;
    Ok((
        json!({"messages": events, "complete": complete, "omitted_messages": omitted_messages}),
        complete,
    ))
}

/// Disposable Unix-socket regressions; tests do not contact a running Agent Run.
/// Wait briefly for nonblocking local socket readiness without resetting an absolute deadline.
/// No model/status polling occurs: this only completes the current bounded wire exchange.
fn wait_io(deadline: Instant) -> Result<()> {
    // ponytail: 1ms I/O retry; use Tokio socket readiness if concurrent volume needs it.
    std::thread::park_timeout(remaining(deadline)?.min(Duration::from_millis(1)));
    Ok(())
}

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "Assertions in bounded disposable socket fixtures"
)]
mod tests {
    use super::*;
    use std::io::{BufRead, BufReader};
    use std::os::unix::net::UnixListener;
    use std::process::Command;
    use std::sync::Mutex;
    use std::thread::{self, JoinHandle};

    /// Serializes fixture connections to exercise the intentional single-connector cap.
    static SOCKETS: Mutex<()> = Mutex::new(());
    /// Root-provided native temporary directory; task and CI own its placement.
    fn scratch() -> std::path::PathBuf {
        std::env::temp_dir()
            .canonicalize()
            .expect("native temporary directory exists")
    }

    /// Owns a disposable listener path and a bounded scripted server thread.
    struct Fake {
        /// Temporary root retained until the server has been joined.
        directory: tempfile::TempDir,
        /// Production client directed only at this fixture's private socket.
        client: Client,
        /// Worker that owns the listener and checks every expected request.
        worker: JoinHandle<()>,
    }

    impl Fake {
        /// Serves method/result pairs, wrapping results in matching JSON-RPC frames.
        /// A scripted `jsonrpc` field supplies a deliberately malformed raw envelope.
        fn new(script: Vec<(&'static str, Value)>) -> Self {
            let directory = tempfile::Builder::new()
                .prefix("run-")
                .tempdir_in(scratch())
                .unwrap();
            let socket = directory.path().join("api.sock");
            let listener = UnixListener::bind(&socket).unwrap();
            listener.set_nonblocking(true).unwrap();
            let worker = thread::spawn(move || {
                for (method, result) in script {
                    let deadline = Instant::now() + Duration::from_secs(5);
                    let mut stream = loop {
                        match listener.accept() {
                            Ok((stream, _)) => break stream,
                            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                                assert!(
                                    Instant::now() < deadline,
                                    "expected fixture request did not arrive"
                                );
                                thread::park_timeout(Duration::from_millis(1));
                            }
                            Err(error) => panic!("fixture listener failed: {:?}", error.kind()),
                        }
                    };
                    stream.set_nonblocking(false).unwrap();
                    stream
                        .set_read_timeout(Some(Duration::from_secs(5)))
                        .unwrap();
                    stream
                        .set_write_timeout(Some(Duration::from_secs(5)))
                        .unwrap();
                    let mut line = String::new();
                    BufReader::new(&mut stream)
                        .take(FRAME_BYTES as u64 + 1)
                        .read_line(&mut line)
                        .unwrap();
                    assert!(line.ends_with('\n'));
                    let request: Value = serde_json::from_str(&line).unwrap();
                    assert_eq!(request["jsonrpc"], "2.0");
                    assert_eq!(request["method"], method);
                    assert!(request["params"].is_object());
                    if method == "start" {
                        assert_eq!(request["params"]["request_id"], "persisted-key");
                    }
                    let envelope = if result.get("jsonrpc").is_some() {
                        result
                    } else {
                        json!({"jsonrpc": "2.0", "id": request["id"], "result": result})
                    };
                    let mut response = serde_json::to_vec(&envelope).unwrap();
                    response.push(b'\n');
                    // Split the frame so the client must collect, rather than assume,
                    // one read equals one JSON message.
                    let split = response.len() / 2;
                    stream.write_all(&response[..split]).unwrap();
                    stream.write_all(&response[split..]).unwrap();
                }
            });
            Self {
                directory,
                client: Client::new(socket),
                worker,
            }
        }

        /// Observes server completion before releasing its disposable socket directory.
        fn finish(self) {
            self.worker.join().unwrap();
            drop(self.directory);
        }
    }

    /// Constructs a complete public execution view with intentionally volatile fields.
    fn agent(id: &str, sequence: u32, status: &str, hash: Option<&str>) -> Value {
        json!({
            "agent_id": id, "sequence": sequence, "workdir": null,
            "task_summary": "Implement the storage boundary", "profile": "developer",
            "status": status, "phase": "executing", "answer_sha256": hash,
            "health": {"state": "healthy", "elapsed_seconds": 99},
            "last_progress_at": "volatile", "delivery": {"pending": true}
        })
    }

    /// Builds one coherent inventory page, retaining real pagination metadata.
    fn page(items: Vec<Value>, offset: usize, total: usize, next: Option<usize>) -> Value {
        assert!(offset + items.len() <= total);
        json!({"items": items, "total": total, "next_offset": next, "complete": next.is_none(),
            "revision": 1, "message_revision": 19})
    }

    /// Builds an available verified answer for the exact serialized content bytes.
    fn answer(content: &str) -> Value {
        json!({"agent_id": "analyzer", "status": "succeeded", "available": true,
            "sha256": format!("{:x}", Sha256::digest(content.as_bytes())),
            "content": content, "inline_complete": true})
    }

    /// Runs fixture-only Git with external overrides, hooks, and signing disabled.
    /// Commands may write only inside the caller's disposable scratch repository.
    fn fixture_git(root: &Path, args: &[&str]) {
        let mut command = Command::new("git");
        for (name, _) in std::env::vars_os() {
            if name.to_str().is_some_and(|name| name.starts_with("GIT_")) {
                command.env_remove(name);
            }
        }
        let output = command
            .current_dir(root)
            .args([
                "-c",
                "user.name=Socket Fixture",
                "-c",
                "user.email=socket@example.invalid",
                "-c",
                "commit.gpgsign=false",
                "-c",
                "core.hooksPath=/dev/null",
            ])
            .args(args)
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .output()
            .unwrap();
        assert!(output.status.success(), "isolated fixture Git failed");
    }

    /// Exercises split framing, exact RPC IDs, safe domain errors, and start receipts.
    #[test]
    fn framing_errors_and_start_receipt() {
        let _guard = SOCKETS.lock().unwrap();
        let fake = Fake::new(vec![
            (
                "start",
                json!({"agent_id": "analyzer", "sequence": 7, "created": true, "agent": {}}),
            ),
            ("start", json!({"agent_id": "analyzer", "sequence": 0})),
            ("probe", json!({"jsonrpc": "2.0", "id": 2, "result": {}})),
            (
                "probe",
                json!({"jsonrpc": "2.0", "id": 1, "error": {
                "code": -32000, "message": "sensitive response body", "data": {"code": "api_unavailable"}}}),
            ),
        ]);
        let receipt = fake
            .client
            .submit(&json!({"request_id": "persisted-key"}))
            .unwrap();
        assert_eq!(receipt.agent_id, "analyzer");
        assert_eq!(receipt.sequence, 7);
        assert_eq!(
            fake.client
                .submit(&json!({"request_id": "persisted-key"}))
                .unwrap_err()
                .code,
            "run_protocol"
        );
        assert_eq!(
            fake.client.call("probe", &json!({})).unwrap_err().code,
            "run_protocol"
        );
        let error = fake.client.call("probe", &json!({})).unwrap_err();
        assert_eq!(error.code, "api_unavailable");
        assert!(!error.message.contains("sensitive"));
        fake.finish();
    }

    /// Rejects oversize requests without connecting and bounds slow/absent response data.
    #[test]
    fn size_deadline_and_malformed_frames() {
        let _guard = SOCKETS.lock().unwrap();
        let directory = tempfile::Builder::new()
            .prefix("frame-")
            .tempdir_in(scratch())
            .unwrap();
        let client = Client::new(directory.path().join("missing.sock"));
        assert_eq!(
            client
                .call("probe", &json!({"large": "x".repeat(FRAME_BYTES)}))
                .unwrap_err()
                .code,
            "run_request_too_large"
        );
        assert_eq!(
            client.call("probe", &Value::Null).unwrap_err().code,
            "run_request_invalid"
        );
        assert_eq!(remaining(Instant::now()).unwrap_err().code, "run_timeout");
        for bytes in [
            b"{}".to_vec(),
            b"not-json\n".to_vec(),
            b"{\"jsonrpc\":\"2.0\",\"id\":1}\n".to_vec(),
            b"\xff\n".to_vec(),
            b"{}\n{}\n".to_vec(),
            vec![b'x'; FRAME_BYTES + 1],
        ] {
            let case = format!(
                "{} bytes, prefix {:?}",
                bytes.len(),
                &bytes[..bytes.len().min(16)]
            );
            let path = directory.path().join("wire.sock");
            let listener = UnixListener::bind(&path).unwrap();
            let expected = if bytes.len() > FRAME_BYTES {
                "run_response_too_large"
            } else {
                "run_protocol"
            };
            let server = thread::spawn(move || {
                let (mut stream, _) = listener.accept().unwrap();
                stream
                    .set_read_timeout(Some(Duration::from_secs(5)))
                    .unwrap();
                stream
                    .set_write_timeout(Some(Duration::from_secs(5)))
                    .unwrap();
                let mut request = String::new();
                BufReader::new(&mut stream).read_line(&mut request).unwrap();
                // Oversize rejection may close the client before this write finishes.
                let _ = stream.write_all(&bytes);
            });
            assert_eq!(
                Client::new(path.clone())
                    .call("probe", &json!({}))
                    .unwrap_err()
                    .code,
                expected,
                "{case}"
            );
            server.join().unwrap();
            std::fs::remove_file(path).unwrap();
        }
    }

    /// A server that received the request but withholds its response cannot reset
    /// the absolute read deadline; a channel releases and joins it after rejection.
    #[test]
    fn socket_read_deadline() {
        let _guard = SOCKETS.lock().unwrap();
        let directory = tempfile::Builder::new()
            .prefix("timeout-")
            .tempdir_in(scratch())
            .unwrap();
        let path = directory.path().join("api.sock");
        let listener = UnixListener::bind(&path).unwrap();
        let (ready_send, ready_receive) = mpsc::sync_channel(1);
        let (release_send, release_receive) = mpsc::sync_channel(1);
        let server = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(5)))
                .unwrap();
            let mut request = String::new();
            BufReader::new(&mut stream).read_line(&mut request).unwrap();
            ready_send.send(()).unwrap();
            release_receive
                .recv_timeout(Duration::from_secs(5))
                .unwrap();
        });
        let result = Client::new(path).call_until(
            "probe",
            &json!({}),
            Instant::now() + Duration::from_secs(1),
        );
        ready_receive.recv_timeout(Duration::from_secs(5)).unwrap();
        release_send.send(()).unwrap();
        server.join().unwrap();
        assert_eq!(result.unwrap_err().code, "run_timeout");
    }

    /// Accepts only exact input/revision/hash identity, with a second inventory fence.
    #[test]
    fn exact_answer_identity_and_sequence_race() {
        let _guard = SOCKETS.lock().unwrap();
        let content = r#"{"analysis_input_id":"input-1","plan_revision":3,"rows":[]}"#;
        let output = answer(content);
        let hash = output["sha256"].as_str().unwrap();
        let receipt = Receipt {
            agent_id: "analyzer".into(),
            sequence: 4,
        };
        for (after_sequence, expected_input, expected_revision, error) in [
            (4, "input-1", 3, None),
            (5, "input-1", 3, Some("answer_mismatch")),
            (4, "different-input", 3, Some("answer_mismatch")),
            (4, "input-1", 9, Some("answer_mismatch")),
        ] {
            let fake = Fake::new(vec![
                (
                    "list_agents",
                    page(
                        vec![agent("analyzer", 4, "succeeded", Some(hash))],
                        0,
                        1,
                        None,
                    ),
                ),
                ("answer", output.clone()),
                (
                    "list_agents",
                    page(
                        vec![agent("analyzer", after_sequence, "succeeded", Some(hash))],
                        0,
                        1,
                        None,
                    ),
                ),
            ]);
            let result = fake
                .client
                .result(&receipt, expected_input, expected_revision);
            match error {
                Some(code) => assert_eq!(result.unwrap_err().code, code),
                None => assert_eq!(result.unwrap().unwrap()["analysis_input_id"], "input-1"),
            }
            fake.finish();
        }
    }

    /// Terminal missing/truncated content is unavailable; active is the only pending state.
    #[test]
    fn incomplete_unavailable_failed_and_pending_answers() {
        let _guard = SOCKETS.lock().unwrap();
        let receipt = Receipt {
            agent_id: "analyzer".into(),
            sequence: 4,
        };
        for status in [
            "created",
            "starting",
            "running",
            "cancelling",
            "failed",
            "timed_out",
            "cancelled",
            "lost",
        ] {
            let fake = Fake::new(vec![(
                "list_agents",
                page(vec![agent("analyzer", 4, status, None)], 0, 1, None),
            )]);
            let result = fake.client.result(&receipt, "input-1", 3);
            if matches!(status, "created" | "starting" | "running" | "cancelling") {
                assert!(result.unwrap().is_none());
            } else {
                assert_eq!(result.unwrap_err().code, "analysis_failed");
            }
            fake.finish();
        }
        for (available, inline_complete) in [(false, false), (true, false)] {
            let fake = Fake::new(vec![
                (
                    "list_agents",
                    page(
                        vec![agent("analyzer", 4, "succeeded", Some("digest"))],
                        0,
                        1,
                        None,
                    ),
                ),
                (
                    "answer",
                    json!({"agent_id": "analyzer", "status": "succeeded", "available": available, "inline_complete": inline_complete}),
                ),
            ]);
            assert_eq!(
                fake.client.result(&receipt, "input-1", 3).unwrap_err().code,
                "answer_unavailable"
            );
            fake.finish();
        }
    }

    /// Rejects hash drift, forged content, old receipt lookup, and invalid JSON.
    #[test]
    fn changed_hash_forged_content_and_invalid_json() {
        let _guard = SOCKETS.lock().unwrap();
        let content = r#"{"analysis_input_id":"input-1","plan_revision":3}"#;
        let original = answer(content);
        let hash = original["sha256"].as_str().unwrap();
        let receipt = Receipt {
            agent_id: "analyzer".into(),
            sequence: 4,
        };
        let fake = Fake::new(vec![(
            "list_agents",
            page(
                vec![agent("analyzer", 5, "succeeded", Some(hash))],
                0,
                1,
                None,
            ),
        )]);
        assert_eq!(
            fake.client.result(&receipt, "input-1", 3).unwrap_err().code,
            "answer_mismatch"
        );
        fake.finish();
        let fake = Fake::new(vec![
            (
                "list_agents",
                page(
                    vec![agent("analyzer", 4, "succeeded", Some(hash))],
                    0,
                    1,
                    None,
                ),
            ),
            ("answer", original.clone()),
            (
                "list_agents",
                page(
                    vec![agent("analyzer", 4, "succeeded", Some("changed"))],
                    0,
                    1,
                    None,
                ),
            ),
        ]);
        assert_eq!(
            fake.client.result(&receipt, "input-1", 3).unwrap_err().code,
            "answer_mismatch"
        );
        fake.finish();
        let mut forged = original.clone();
        forged["content"] = json!("forged-content");
        let fake = Fake::new(vec![
            (
                "list_agents",
                page(
                    vec![agent("analyzer", 4, "succeeded", Some(hash))],
                    0,
                    1,
                    None,
                ),
            ),
            ("answer", forged),
        ]);
        assert_eq!(
            fake.client.result(&receipt, "input-1", 3).unwrap_err().code,
            "answer_mismatch"
        );
        fake.finish();
        let invalid = answer("invalid JSON");
        let invalid_hash = invalid["sha256"].as_str().unwrap();
        let fake = Fake::new(vec![
            (
                "list_agents",
                page(
                    vec![agent("analyzer", 4, "succeeded", Some(invalid_hash))],
                    0,
                    1,
                    None,
                ),
            ),
            ("answer", invalid.clone()),
            (
                "list_agents",
                page(
                    vec![agent("analyzer", 4, "succeeded", Some(invalid_hash))],
                    0,
                    1,
                    None,
                ),
            ),
        ]);
        assert_eq!(
            fake.client.result(&receipt, "input-1", 3).unwrap_err().code,
            "analysis_invalid"
        );
        fake.finish();
    }

    /// Exact result reads tolerate unrelated pagination changes while preserving result fences.
    #[test]
    fn point_result_uses_execution_identity_in_a_moving_inventory() {
        let _guard = SOCKETS.lock().unwrap();
        let content = r#"{"analysis_input_id":"input-1","plan_revision":3}"#;
        let output = answer(content);
        let hash = output["sha256"].as_str().unwrap();
        let first = page(vec![agent("other", 1, "running", None)], 0, 2, Some(1));
        let mut target = page(
            vec![agent("analyzer", 4, "succeeded", Some(hash))],
            1,
            2,
            None,
        );
        target["revision"] = json!(2);
        let mut later = first.clone();
        later["revision"] = json!(3);
        let mut final_target = target.clone();
        final_target["revision"] = json!(4);
        let fake = Fake::new(vec![
            ("list_agents", first),
            ("list_agents", target),
            ("answer", output),
            ("list_agents", later),
            ("list_agents", final_target),
        ]);
        assert_eq!(
            fake.client
                .result(
                    &Receipt {
                        agent_id: "analyzer".into(),
                        sequence: 4
                    },
                    "input-1",
                    3
                )
                .unwrap()
                .unwrap()["analysis_input_id"],
            "input-1"
        );
        fake.finish();
    }

    /// A missing target after moving pages is unknown rather than an authoritative disappearance.
    #[test]
    fn moving_point_scan_preserves_unknown_absence() {
        let _guard = SOCKETS.lock().unwrap();
        let first = page(vec![agent("first", 1, "running", None)], 0, 2, Some(1));
        let mut last = page(vec![agent("second", 1, "running", None)], 1, 2, None);
        last["revision"] = json!(2);
        let fake = Fake::new(vec![("list_agents", first), ("list_agents", last)]);
        assert_eq!(
            fake.client
                .result(
                    &Receipt {
                        agent_id: "analyzer".into(),
                        sequence: 4
                    },
                    "input-1",
                    3
                )
                .unwrap_err()
                .code,
            "run_unavailable"
        );
        fake.finish();
    }

    /// Moving revisions retain explicitly partial views; malformed stable cursors still fail.
    #[test]
    fn pagination_revision_and_cursor_failures() {
        let _guard = SOCKETS.lock().unwrap();
        let mut changed = page(vec![agent("second", 1, "running", None)], 1, 2, None);
        changed["revision"] = json!(2);
        let fake = Fake::new(vec![
            (
                "list_agents",
                page(vec![agent("first", 1, "running", None)], 0, 2, Some(1)),
            ),
            ("list_agents", changed),
        ]);
        let (retained, complete) = fake
            .client
            .agents(Instant::now() + CALL_BUDGET, None)
            .unwrap();
        assert_eq!(
            retained.len(),
            2,
            "unrelated inventory movement must not erase known worker views"
        );
        assert!(!complete, "moving inventory cannot claim full coverage");
        fake.finish();
        let fake = Fake::new(vec![(
            "list_agents",
            page(vec![agent("first", 1, "running", None)], 0, 2, Some(0)),
        )]);
        assert_eq!(
            fake.client
                .agents(Instant::now() + CALL_BUDGET, None)
                .unwrap_err()
                .code,
            "run_protocol"
        );
        fake.finish();
    }

    /// Changed overlapping pages keep the latest observed execution rather than dropping all workers.
    #[test]
    fn moving_inventory_preserves_latest_duplicate_sequence() {
        let _guard = SOCKETS.lock().unwrap();
        let first = page(vec![agent("worker", 1, "running", None)], 0, 2, Some(1));
        let mut last = page(vec![agent("worker", 2, "running", None)], 1, 2, None);
        last["revision"] = json!(2);
        let fake = Fake::new(vec![("list_agents", first), ("list_agents", last)]);
        let (items, complete) = fake
            .client
            .agents(Instant::now() + CALL_BUDGET, None)
            .unwrap();
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].sequence, 2);
        assert!(!complete);
        fake.finish();
    }

    /// Active work wins bounded transcript capacity over older failed executions.
    #[test]
    fn developer_priority_preserves_current_work() {
        let _guard = SOCKETS.lock().unwrap();
        let repository = tempfile::tempdir_in(scratch()).unwrap();
        fixture_git(repository.path(), &["init", "--quiet", "-b", "main"]);
        fixture_git(
            repository.path(),
            &["commit", "--quiet", "--allow-empty", "-m", "Baseline"],
        );
        let mut rows: Vec<Value> = (0..17)
            .map(|i| {
                let mut row = agent(&format!("old-{i:02}"), 1, "failed", None);
                row["workdir"] = json!(repository.path());
                row["created_at"] = json!(i);
                row
            })
            .collect();
        let mut active = agent("current", 1, "running", None);
        active["workdir"] = json!(repository.path());
        active["created_at"] = json!(99);
        rows.push(active);
        let mut script = vec![("list_agents", page(rows, 0, 18, None))];
        let ids = std::iter::once("current".to_owned());
        for id in ids {
            script.push(("transcript",json!({"agent_id":id,"messages":[{"seq":1,"role":"assistant","content":"Current implementation and remaining work","content_complete":true}],"complete":true})));
        }
        let fake = Fake::new(script);
        let observed = fake
            .client
            .developers(&repository.path().join(".git"), &[])
            .unwrap();
        assert!(
            observed["items"]
                .as_array()
                .unwrap()
                .iter()
                .any(|row| row["agent_id"] == "current"),
            "active work must not be displaced by old failures"
        );
        fake.finish();
    }

    /// Worker handoff text is included only after its sealed bytes and execution identity verify.
    #[test]
    fn developer_answer_preserves_sealed_check_context() {
        let _guard = SOCKETS.lock().unwrap();
        let repository = tempfile::tempdir_in(scratch()).unwrap();
        fixture_git(repository.path(), &["init", "--quiet", "-b", "main"]);
        fixture_git(
            repository.path(),
            &["commit", "--quiet", "--allow-empty", "-m", "Baseline"],
        );
        let content =
            "CHECKS: source tree abc; test result: ok. 64 passed. Remaining: Rust consumers.";
        let hash = format!("{:x}", Sha256::digest(content.as_bytes()));
        let mut worker = agent("worker", 1, "succeeded", Some(&hash));
        worker["workdir"] = json!(repository.path());
        let mut result = answer(content);
        result["agent_id"] = json!("worker");
        let fake = Fake::new(vec![
            ("list_agents", page(vec![worker.clone()], 0, 1, None)),
            (
                "transcript",
                json!({"agent_id":"worker","messages":[],"complete":true}),
            ),
            ("answer", result),
            ("list_agents", page(vec![worker], 0, 1, None)),
        ]);
        let observed = fake
            .client
            .developers(&repository.path().join(".git"), &[])
            .unwrap();
        assert_eq!(observed["items"][0]["sealed_answer"]["content"], content);
        assert_eq!(observed["items"][0]["sealed_answer"]["source_sha256"], hash);
        assert!(
            observed["items"][0]["sealed_answer"]["meaning"]
                .as_str()
                .unwrap()
                .contains("assertions")
        );
        fake.finish();
    }

    /// A plausible developer handoff with forged bytes is omitted instead of becoming check evidence.
    #[test]
    fn developer_answer_rejects_changed_content() {
        let _guard = SOCKETS.lock().unwrap();
        let repository = tempfile::tempdir_in(scratch()).unwrap();
        fixture_git(repository.path(), &["init", "--quiet", "-b", "main"]);
        fixture_git(
            repository.path(),
            &["commit", "--quiet", "--allow-empty", "-m", "Baseline"],
        );
        let content = "CHECKS: 64 passed";
        let hash = format!("{:x}", Sha256::digest(content.as_bytes()));
        let mut worker = agent("worker", 1, "succeeded", Some(&hash));
        worker["workdir"] = json!(repository.path());
        let mut result = answer(content);
        result["agent_id"] = json!("worker");
        result["content"] = json!("Forged success claim");
        let fake = Fake::new(vec![
            ("list_agents", page(vec![worker], 0, 1, None)),
            (
                "transcript",
                json!({"agent_id":"worker","messages":[],"complete":true}),
            ),
            ("answer", result),
        ]);
        let observed = fake
            .client
            .developers(&repository.path().join(".git"), &[])
            .unwrap();
        assert!(observed["items"][0].get("sealed_answer").is_none());
        assert_eq!(observed["complete"], false);
        assert!(
            observed["omissions"]
                .as_array()
                .unwrap()
                .iter()
                .any(|row| row["code"] == "answer_mismatch")
        );
        fake.finish();
    }

    /// Large source-reading output cannot consume the budget before a concrete recent test result.
    #[test]
    fn transcript_prioritizes_check_results_with_explicit_omissions() {
        let (view,complete)=transcript_evidence(json!({"agent_id":"worker","complete":true,"messages":[
            {"seq":1,"role":"tool_result","name":"Read","content":"source line\n".repeat(TRANSCRIPT_BYTES),"content_complete":true},
            {"seq":2,"role":"tool_result","name":"command","content":"Exit code: 0\ntest result: ok. 64 passed; 0 failed","content_complete":true}
        ]}),"worker").unwrap();
        assert!(
            view["messages"]
                .as_array()
                .unwrap()
                .iter()
                .any(|row| row["content"].as_str().unwrap().contains("64 passed"))
        );
        assert!(
            !complete,
            "discarded source-reading history must remain disclosed"
        );
    }

    /// Paginates native workers, excludes analyzer/profile runs, and removes telemetry.
    #[test]
    fn developer_pagination_exclusion_and_transcript_coverage() {
        let _guard = SOCKETS.lock().unwrap();
        let repository = tempfile::Builder::new()
            .prefix("git-")
            .tempdir_in(scratch())
            .unwrap();
        fixture_git(repository.path(), &["init", "--quiet", "-b", "integration"]);
        fixture_git(
            repository.path(),
            &[
                "commit",
                "--quiet",
                "--allow-empty",
                "-m",
                "Fixture baseline",
            ],
        );
        let mut excluded = agent("excluded", 1, "running", None);
        excluded["workdir"] = json!(repository.path());
        let mut summary = agent("summary", 1, "running", None);
        summary["profile"] = json!("progress-summary");
        summary["workdir"] = json!(repository.path());
        let mut developer = agent("developer", 1, "running", None);
        developer["workdir"] = json!(repository.path());
        let mut old_analyzer = agent("old-analyzer", 1, "succeeded", None);
        old_analyzer["workdir"] = json!(repository.path());
        old_analyzer["profile"] = json!("approved-read-only");
        old_analyzer["task_summary"] =
            json!("You are a read-only progress summarizer. Interpret this snapshot.");
        let fake = Fake::new(vec![
            (
                "list_agents",
                page(vec![excluded, summary, old_analyzer], 0, 4, Some(3)),
            ),
            ("list_agents", page(vec![developer], 3, 4, None)),
            (
                "transcript",
                json!({"agent_id": "developer", "messages": [
                {"seq": 9, "at": "changing-clock", "role": "assistant", "content": "Implementing storage", "content_complete": true}
            ], "complete": true, "cursor": "clock-independent", "next_cursor": null}),
            ),
        ]);
        let observed = fake
            .client
            .developers(&repository.path().join(".git"), &["excluded".into()])
            .unwrap();
        assert_eq!(observed["items"].as_array().unwrap().len(), 1);
        assert_eq!(observed["items"][0]["agent_id"], "developer");
        assert_eq!(observed["items"][0]["transcript"]["messages"][0]["seq"], 9);
        assert_eq!(observed["complete"], true);
        let serialized = observed.to_string();
        for omitted in [
            "changing-clock",
            "elapsed_seconds",
            "delivery",
            "last_progress_at",
            "message_revision",
        ] {
            assert!(!serialized.contains(omitted));
        }
        fake.finish();
        let (truncated, complete) = transcript_evidence(json!({"agent_id": "developer", "messages": [
            {"seq": 1, "role": "assistant", "content": "é".repeat(TRANSCRIPT_BYTES), "content_complete": false, "partial_before": true}
        ], "complete": true}), "developer").unwrap();
        assert!(!complete);
        assert_eq!(
            truncated["messages"][0]["observer_omitted_bytes"],
            TRANSCRIPT_BYTES
        );
        assert!(
            transcript_evidence(
                json!({"agent_id": "developer", "complete": true}),
                "developer"
            )
            .is_err()
        );
    }
}
