//! One requested analysis continuation and immutable delivery-only Bridge publication.
use super::{
    Caller,
    error::{Error, Result},
    run,
    state::{self, Locked},
};
use bridge_sdk::{Client, ClientError, TransportOutcome, bridge_protocol::*};
use serde::{Deserialize, Serialize};
use std::{
    fs::{File, OpenOptions},
    io::Read,
    os::{
        fd::AsFd,
        unix::{
            fs::{MetadataExt, OpenOptionsExt},
            process::CommandExt,
        },
    },
    path::{Path, PathBuf},
    process::{Command, Stdio},
    time::Duration,
};

/// Explicit dedicated producer deployment. It grants message delivery, never caller authority.
#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Settings {
    /// Existing Bridge capability endpoint, in a protected service directory.
    pub registration_socket: PathBuf,
    /// Existing Bridge custody endpoint; recovery never substitutes another deployment.
    pub core_socket: PathBuf,
    /// Owned private regular credential file; contents never enter state, argv or diagnostics.
    pub token_file: PathBuf,
    /// Expected authenticated producer in custody receipts; defaults to agent-tasks.
    #[serde(default = "producer")]
    pub producer: String,
    /// Trusted deployment host mapping; defaults to local.
    #[serde(default = "host")]
    pub host_id: String,
    /// Trusted Codex store mapping; defaults to codex-main.
    #[serde(default = "store")]
    pub codex_store: String,
}
/// Default dedicated publisher name, independent of native caller identity.
fn producer() -> String {
    "agent-tasks".into()
}
/// Default local host namespace.
fn host() -> String {
    "local".into()
}
/// Default Codex store namespace, never inferred from a persistent server's environment.
fn store() -> String {
    "codex-main".into()
}

/// Immutable publication lifecycle; a possible write permits status reconciliation only.
#[derive(Clone, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case", deny_unknown_fields)]
enum Phase {
    /// Analysis is still pending; no report body is invented.
    Waiting,
    /// Final body/hash are frozen; attempted means a write may already have occurred.
    Ready {
        body: String,
        hash: String,
        attempted: bool,
    },
    /// Bridge owns delivery; custody is not proof that the owner read the message.
    Accepted { receipt: EventReceipt },
}
/// One frozen return destination/outcome, retained independently of report eviction.
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Delivery {
    /// Original deployment and credential-file association.
    settings: Settings,
    /// Original private credential fingerprint; never the credential itself.
    credential_sha256: String,
    /// Exact grant-authorized return destination; not native caller attestation.
    address: String,
    /// Stable event identity, including repository/input/generation/destination.
    event_id: String,
    /// Automatic delivery remains required after pending or any nonterminal result-observation error.
    announced: bool,
    /// Frozen domain/publication state.
    phase: Phase,
    /// Safe last error class, retained when automatic work cannot continue.
    last_error: Option<String>,
}

/// Own the inherited file description. Explicit unlock also releases the stdin duplicate
/// while the repository transaction is still held, preventing a lost worker handoff.
struct ContinuationLease(File);
impl Drop for ContinuationLease {
    /// Release all duplicate-descriptor locks without closing or reading protocol streams.
    fn drop(&mut self) {
        let _ = fs2::FileExt::unlock(&self.0);
    }
}

/// Validate explicit deployment and freeze the requested return destination before admission.
/// Only Codex is supported; absent/invalid destination refuses without calling the model.
pub fn prepare(
    settings: Settings,
    caller: Option<&Caller>,
    input: &str,
    generation: u64,
) -> Result<Delivery> {
    let caller = caller
        .filter(|c| c.transport == "codex_queue" && !c.external_session_id.is_empty())
        .ok_or_else(|| {
            Error::new(
                "bridge_destination",
                "Automatic status delivery requires the originating Codex chat metadata.",
            )
        })?;
    let (_, credential_sha256) = client(&settings)?;
    let address = return_address(Some(&settings), caller)?;
    if settings.producer.is_empty()
        || settings.producer.len() > 128
        || settings.producer.chars().any(char::is_control)
    {
        return Err(Error::new(
            "bridge_configuration",
            "Invalid dedicated Bridge producer name.",
        ));
    }
    let key = state::digest(&(input, generation, &address))?;
    Ok(Delivery {
        settings,
        credential_sha256,
        address,
        event_id: format!("at-status-{key}"),
        announced: false,
        phase: Phase::Waiting,
        last_error: None,
    })
}

/// Canonical return address from original host metadata and trusted deployment mapping.
/// Without explicit Bridge settings use the documented local/codex-main mapping; this is
/// a contact address, never evidence of current reachability, native attestation or authority.
pub fn return_address(settings: Option<&Settings>, caller: &Caller) -> Result<String> {
    if caller.transport != "codex_queue" {
        return Err(Error::new(
            "bridge_destination",
            "Only Codex return addresses are supported.",
        ));
    }
    let encode = |s: &str| s.replace('%', "%25").replace('/', "%2F");
    NativeAddress::parse(&format!(
        "{}/codex/{}/{}",
        encode(settings.map_or("local", |s| s.host_id.as_str())),
        encode(settings.map_or("codex-main", |s| s.codex_store.as_str())),
        encode(&caller.external_session_id)
    ))
    .map(|a| a.canonical())
    .map_err(|_| Error::new("bridge_destination", "Invalid configured return address."))
}

/// Read one bounded owner-only credential without following the file or its immediate parent.
/// Paths and secret bytes never appear in returned errors; SDK reconnects are disabled.
fn client(settings: &Settings) -> Result<(Client, String)> {
    let load = || -> std::io::Result<String> {
        let parent = settings
            .token_file
            .parent()
            .ok_or(std::io::ErrorKind::InvalidInput)?;
        if !settings.token_file.is_absolute()
            || !settings.registration_socket.is_absolute()
            || !settings.core_socket.is_absolute()
        {
            return Err(std::io::ErrorKind::InvalidInput.into());
        }
        let directory = OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NOFOLLOW | libc::O_DIRECTORY)
            .open(parent)?;
        let mut file = OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
            .open(&settings.token_file)?;
        let d = directory.metadata()?;
        let m = file.metadata()?;
        // SAFETY: geteuid has no arguments, pointers, mutation or lifetime preconditions.
        let uid = unsafe { libc::geteuid() };
        if !m.is_file()
            || m.uid() != uid
            || d.uid() != uid
            || m.mode() & 0o077 != 0
            || d.mode() & 0o077 != 0
            || m.len() > 16384
        {
            return Err(std::io::ErrorKind::PermissionDenied.into());
        }
        let mut bytes = Vec::new();
        file.by_ref().take(16385).read_to_end(&mut bytes)?;
        if bytes.len() > 16384 {
            return Err(std::io::ErrorKind::InvalidData.into());
        }
        String::from_utf8(bytes)
            .map(|s| s.trim_end_matches(['\r', '\n']).to_owned())
            .map_err(|_| std::io::ErrorKind::InvalidData.into())
    };
    let credential = load().map_err(|_| {
        Error::new(
            "bridge_configuration",
            "Dedicated protected Bridge credential is unavailable.",
        )
    })?;
    let fingerprint = state::digest(&credential)?;
    let mut config = bridge_sdk::Config::new(
        &settings.registration_socket,
        &settings.core_socket,
        Secret::new(credential),
        Duration::from_secs(5),
    )
    .map_err(|_| {
        Error::new(
            "bridge_configuration",
            "Invalid Bridge deployment settings.",
        )
    })?;
    config.reconnect_once = false;
    Ok((
        Client::new(config).map_err(|_| {
            Error::new(
                "bridge_configuration",
                "Invalid Bridge deployment settings.",
            )
        })?,
        fingerprint,
    ))
}

impl Delivery {
    /// True only after exact authenticated custody has been verified.
    pub fn accepted(&self) -> bool {
        matches!(self.phase, Phase::Accepted { .. })
    }
    /// Freeze the final validated report or a classified terminal failure exactly once.
    pub fn ready(&mut self, body: String) -> Result<()> {
        if matches!(self.phase, Phase::Waiting) {
            let hash = publication_intent_fingerprint(&self.params(&body)?).map_err(|_| {
                Error::new("bridge_invalid", "Status message exceeds Bridge limits.")
            })?;
            self.phase = Phase::Ready {
                body,
                hash,
                attempted: false,
            };
        }
        Ok(())
    }
    /// Build the sole supported granted publication: message/keep_all without native registration.
    fn params(&self, body: &str) -> Result<PublishParams> {
        Ok(PublishParams {
            destination: Destination::Address {
                address: self.address.clone(),
            },
            event_id: self.event_id.clone(),
            kind: "agent-tasks.status".into(),
            payload: Payload::new(body)
                .map_err(|_| Error::new("bridge_invalid", "Invalid compact status message."))?,
            mode: Mode::Message,
            policy: Policy::KeepAll,
            metadata: Metadata::default(),
            expires_at_ms: None,
            delivery_scope_key: None,
        })
    }
}

/// Freeze a result body in the same atomic transition as the report and cleared admission.
pub fn completed(
    locked: &mut Locked,
    revision: u64,
    observed_at: &str,
    compact: &str,
) -> Result<()> {
    if let Some(delivery) = &mut locked.state.delivery {
        delivery.ready(format!(
            "Agent Tasks · plan {revision} · snapshot {observed_at}\n{compact}"
        ))?;
    }
    Ok(())
}

/// Own a continuation after pending or a nonterminal admission/result-observation error. Ready replies
/// do not also send a message; previously promised deliveries remain tied to the original chat.
pub fn after_status(
    locked: &mut Locked,
    home: &Path,
    key: &str,
    result: &Result<String>,
) -> Result<()> {
    let Some(delivery) = &mut locked.state.delivery else {
        return Ok(());
    };
    if delivery.accepted() {
        return Ok(());
    }
    if result.as_ref().is_ok_and(|s| s.starts_with("PENDING ")) {
        delivery.announced = true;
    }
    if result.is_err()
        && locked
            .state
            .pending
            .as_ref()
            .is_some_and(|p| p.terminal.is_none())
    {
        delivery.announced = true;
    }
    if !delivery.announced {
        // An uncertain start must retain its original destination even before a PENDING
        // acknowledgement; a later caller recovers that same request, not its own chat.
        if !locked
            .state
            .pending
            .as_ref()
            .is_some_and(|p| p.terminal.is_none())
        {
            locked.state.delivery = None;
        }
        return locked.save();
    }
    terminal_failure(locked)?;
    locked.save()?;
    spawn(locked, home, key)
}

/// Convert only a confirmed terminal analysis failure into a bounded owner message.
fn terminal_failure(locked: &mut Locked) -> Result<()> {
    if let Some(code) = locked
        .state
        .pending
        .as_ref()
        .and_then(|p| p.terminal.as_deref())
        && let Some(delivery) = &mut locked.state.delivery
    {
        delivery.ready(format!("Agent Tasks: the report could not be retrieved ({code}).\nThe previous estimate is retained; no new progress is claimed."))?;
    }
    Ok(())
}

/// Transfer the already-held file lease to the worker's stdin, never MCP pipes.
/// The worker has its own process group and remains alive after normal frontend EOF.
fn spawn(locked: &Locked, home: &Path, key: &str) -> Result<()> {
    let Some(lease) = locked.completion_lease()? else {
        return Ok(());
    };
    let mut command = Command::new(std::env::current_exe()?);
    command
        .arg("deliver-status")
        .arg("--home")
        .arg(home)
        .arg("--key")
        .arg(key)
        .env_clear()
        .stdin(Stdio::from(lease))
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .process_group(0);
    for name in ["TMPDIR", "TMP", "TEMP"] {
        if let Some(value) = std::env::var_os(name) {
            command.env(name, value);
        }
    }
    let mut child = command.spawn().map_err(|_| Error::new("delivery_worker_unavailable", "Analysis is saved, but its message continuation could not start; a later status recovers it."))?;
    // The operating system adopts the child if this frontend exits; while alive we reap its exit.
    std::thread::spawn(move || {
        let _ = child.wait();
    });
    Ok(())
}

/// Own the inherited dispatch lease and wait for saved analyses, never sampling Git or launching
/// models. A lost broker connection leaves durable state for the next explicit status request.
pub fn worker(home: &Path, key: &str) -> Result<()> {
    let mut locked = Locked::wait(home, key)?;
    // Reverse declaration drop order releases the lease before a live transaction,
    // including early error returns from publication and persistence.
    let lease = ContinuationLease(File::from(std::io::stdin().as_fd().try_clone_to_owned()?));
    if !locked.matches_completion_lease(&lease.0)? {
        return Err(Error::new(
            "delivery_worker_invalid",
            "Missing owned completion lease.",
        ));
    }
    loop {
        let active = locked
            .state
            .delivery
            .as_ref()
            .is_some_and(|d| d.announced && !d.accepted());
        if !active {
            drop(lease);
            return Ok(());
        }
        terminal_failure(&mut locked)?;
        if locked
            .state
            .delivery
            .as_ref()
            .is_some_and(|d| matches!(d.phase, Phase::Ready { .. }))
        {
            publish(&mut locked)?;
            drop(lease);
            return Ok(());
        } else {
            let Some(pending) = locked.state.pending.clone() else {
                drop(lease);
                return Ok(());
            };
            let Some(receipt) = pending.receipt else {
                // Recover only the original saved admission key/namespace. A lost start ACK
                // must not lose the original chat merely because completion was already fast.
                let recovery = super::settle(&mut locked, &super::View::Compact);
                terminal_failure(&mut locked)?;
                if let Err(error) = recovery {
                    if let Some(delivery) = &mut locked.state.delivery {
                        delivery.last_error = Some(error.code);
                    }
                    locked.save()?;
                    if locked
                        .state
                        .pending
                        .as_ref()
                        .is_some_and(|p| p.terminal.is_none())
                    {
                        drop(lease);
                        return Ok(());
                    }
                }
                continue;
            };
            drop(locked);
            let client = run::Client::new(pending.socket);
            let wakeup = client.wait(&receipt);
            locked = Locked::wait(home, key)?;
            if let Err(error) = wakeup {
                if let Some(delivery) = &mut locked.state.delivery {
                    delivery.last_error = Some(error.code.clone());
                }
                locked.save()?;
                drop(lease);
                return Err(error);
            }
            if !locked.state.pending.as_ref().is_some_and(|p| {
                p.generation == pending.generation && p.bundle.input_id == pending.bundle.input_id
            }) {
                continue;
            }
            let result = super::settle(&mut locked, &super::View::Compact);
            terminal_failure(&mut locked)?;
            if let Err(error) = result {
                if let Some(delivery) = &mut locked.state.delivery {
                    delivery.last_error = Some(error.code);
                }
                locked.save()?;
                if !locked
                    .state
                    .delivery
                    .as_ref()
                    .is_some_and(|d| matches!(d.phase, Phase::Ready { .. }))
                {
                    drop(lease);
                    return Ok(());
                }
            }
        }
        if locked
            .state
            .delivery
            .as_ref()
            .is_some_and(|d| !d.accepted() && matches!(d.phase, Phase::Ready { .. }))
        {
            publish(&mut locked)?;
        }
        // Release the worker lease while still holding the repository lock: a concurrent
        // explicit request cannot promise work between our final check and lease release.
        drop(lease);
        return Ok(());
    }
}

/// Persist possible-write state before SDK I/O. Unknown/rotated outcomes only read the original
/// event; absent history never authorizes a second publication or changed destination.
fn publish(locked: &mut Locked) -> Result<()> {
    let Some(mut delivery) = locked.state.delivery.clone() else {
        return Ok(());
    };
    let Phase::Ready {
        body,
        hash,
        attempted,
    } = delivery.phase.clone()
    else {
        return Ok(());
    };
    let (client, fingerprint) = client(&delivery.settings)?;
    let params = delivery.params(&body)?;
    if publication_intent_fingerprint(&params)
        .map_err(|_| Error::new("bridge_invalid", "Invalid frozen message."))?
        != hash
    {
        return Err(Error::new(
            "bridge_invalid",
            "Frozen message identity mismatch.",
        ));
    }
    delivery.phase = Phase::Ready {
        body: body.clone(),
        hash: hash.clone(),
        attempted: true,
    };
    locked.state.delivery = Some(delivery.clone());
    locked.save()?;
    let result = if attempted || fingerprint != delivery.credential_sha256 {
        reconcile(&client, &delivery)
    } else {
        client
            .publish(&delivery.event_id, params)
            .map(|r| r.receipt)
    };
    let publication_unknown = !attempted
        && fingerprint == delivery.credential_sha256
        && (matches!(
            &result,
            Err(ClientError::Transport {
                outcome: TransportOutcome::OutcomeUnknown,
                ..
            })
        ) || matches!(&result, Err(ClientError::Service(e)) if e.effect != Some(MutationEffect::NotCommitted)));
    let result = if publication_unknown {
        reconcile(&client, &delivery)
    } else {
        result
    };
    match result {
        Ok(receipt)
            if receipt.producer == delivery.settings.producer
                && receipt.event_id == delivery.event_id
                && receipt.intent_hash == hash
                && receipt.destination.address == delivery.address
                && receipt.destination.route_key.is_none()
                && receipt.destination.caller_scope.is_none()
                && receipt.destination.destination_scope.is_none() =>
        {
            delivery.phase = Phase::Accepted { receipt };
            delivery.last_error = None;
        }
        Err(error) => {
            let definitely_unsent = matches!(
                error,
                ClientError::InvalidConfig(_)
                    | ClientError::InvalidRequest(_)
                    | ClientError::Transport {
                        outcome: TransportOutcome::NotSent,
                        ..
                    }
            ) || matches!(&error, ClientError::Service(e) if e.effect == Some(MutationEffect::NotCommitted));
            if definitely_unsent
                && !attempted
                && !publication_unknown
                && fingerprint == delivery.credential_sha256
            {
                delivery.phase = Phase::Ready {
                    body,
                    hash,
                    attempted: false,
                };
            }
            delivery.last_error = Some("bridge_delivery_unconfirmed".into());
        }
        Ok(_) => {
            delivery.last_error = Some("bridge_custody_mismatch".into());
        }
    }
    locked.state.delivery = Some(delivery);
    locked.save()
}

/// One read-only reconciliation of the original event after possible custody; never republishes.
fn reconcile(
    client: &Client,
    delivery: &Delivery,
) -> std::result::Result<EventReceipt, ClientError> {
    client
        .status(
            format!("{}-status", delivery.event_id),
            StatusSelector::Event {
                event_id: delivery.event_id.clone(),
            },
        )
        .and_then(|status| match status {
            StatusResult::Event { event: Some(event) } => Ok(event.receipt),
            _ => Err(ClientError::Transport {
                outcome: TransportOutcome::OutcomeUnknown,
                reconciliation: None,
                reason: "original custody not visible",
            }),
        })
}
