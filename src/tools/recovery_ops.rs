//! Producer for the exceptional `git_recovery` tool: explicit recovery of pending local Git work.
//!
//! The normal commit path is automatic settlement after each mutation. This module only decodes the
//! closed payload, validates it, calls the persistence recovery engine under the dispatcher's lock and
//! returns the shared acknowledgement. It never locks, never replays a business mutation and never
//! pushes. Registration, routing and rendering belong to the shared dispatcher.
#![allow(
    dead_code,
    reason = "The shared dispatcher routes to this producer when the registered tool surface lands"
)]
use super::{input, work};
use crate::{
    model,
    persist::{
        self, EventClass, GitReceipt,
        recover::{Action, PreserveItem, Report, recover},
    },
    store::{self, Error, LockGuard, Result, Store},
};
use schemars::JsonSchema;
use serde::Deserialize;

/// Presentation label of the recovery acknowledgement; it is never a canonical reference.
pub const TARGET: &str = "Git recovery";
/// Most intents one request may name.
const MAX_INTENTS: usize = 16;
/// Most files one preservation may name.
const MAX_PATHS: usize = 32;
/// Longest note line.
const NOTE_BYTES: usize = 200;
/// Most notes in one acknowledgement.
const MAX_NOTES: usize = 8;
/// Most references in one acknowledgement.
const MAX_REFS: usize = 16;
/// Most pending references named in an error.
const ERROR_REFS: usize = 4;

/// One file the caller authorizes to be committed with exactly these bytes.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PreservePathIn {
    /// Owned relative path of the file.
    pub relative: String,
    /// Lowercase SHA-256 of the authorized bytes.
    pub sha256: String,
    /// Length of the authorized bytes.
    pub len: u64,
}

/// Closed wire payload; `version` of the common fields carries the pending snapshot version.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(tag = "op", rename_all = "snake_case", deny_unknown_fields)]
pub enum RecoveryOp {
    /// Resolve unknown and committing intents from HEAD, trailers and the files; read-only for Git.
    Reconcile {
        /// Up to sixteen pending intent identities (`PG-` plus 24 hex characters).
        intents: Vec<String>,
    },
    /// Commit the named published or held intents together without a policy decision.
    Retry {
        /// Up to sixteen pending intent identities.
        intents: Vec<String>,
    },
    /// Treat the named unknown intents with matching bytes as published, after explicit authorization.
    Adopt {
        /// Up to sixteen pending intent identities.
        intents: Vec<String>,
    },
    /// Stop tracking the named intents without committing; files are left untouched.
    Release {
        /// Up to sixteen pending intent identities.
        intents: Vec<String>,
    },
    /// Commit exactly the authorized bytes of the named files.
    Preserve {
        /// Up to thirty-two files with their exact byte identity.
        paths: Vec<PreservePathIn>,
    },
}

/// Tool description for the registry owner.
pub const DESCRIPTION: &str = "Purpose: Exceptional explicit recovery of pending local Git work after a deferred, held, unknown or drifted automatic commit. The normal commit path is automatic after each successful mutation; never call this as a checkpoint.\nInput: common project and version, where version is the pending snapshot version shown by context or status (stale otherwise, nothing changes). op is one of: reconcile and retry, adopt, release with intents (1-16 identities PG-<24 hex>); preserve with paths (1-32 {relative, sha256, len}). reconcile resolves unknown or committing intents from HEAD, commit trailers and files. retry commits the named published or held intents together; for a held intent this is the explicit authorization to commit exactly the recorded bytes. adopt treats an unknown intent whose file bytes match as published, never a drifted one. release stops tracking without committing and leaves files untouched. preserve commits exactly the authorized byte identities.\nEffects: local Git commits only through a temporary index with user hooks and signing; foreign staging is preserved; no push, fetch or business replay. Failures are stale, invalid_arguments, not_locked, recovery_blocked or preserve_blocked with nothing committed.\nOutput: acknowledgement with the recovery receipt, at most two change lines and the remaining pending references.";

/// Event class the dispatcher uses for this producer; recovery settles itself and policy skips it.
pub fn event_class() -> EventClass {
    EventClass::Recovery
}

/// Whether `id` is an intent identity: `PG-` plus 24 lowercase hexadecimal characters.
fn intent_id(id: &str) -> bool {
    id.strip_prefix("PG-").is_some_and(|hex| {
        hex.len() == 24 && hex.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
    })
}

/// Validate and copy a bounded list of intent identities.
fn intents(name: &str, ids: Vec<String>) -> Result<Vec<String>> {
    let checked = if ids.is_empty() || ids.len() > MAX_INTENTS {
        Err(format!(
            "Name between one and {MAX_INTENTS} pending intents."
        ))
    } else if ids.iter().any(|id| !intent_id(id)) {
        Err("Each intent must be PG- followed by 24 lowercase hexadecimal characters.".to_owned())
    } else {
        Ok(())
    };
    input::field(name, checked)?;
    Ok(ids)
}

/// Validate the authorized byte identities of a preservation.
fn preserve(paths: Vec<PreservePathIn>) -> Result<Vec<PreserveItem>> {
    let checked = if paths.is_empty() || paths.len() > MAX_PATHS {
        Err(format!(
            "Name between one and {MAX_PATHS} files to preserve."
        ))
    } else {
        paths.iter().try_for_each(|p| {
            model::text(&p.relative, 256)?;
            if p.sha256.len() == 64
                && p.sha256
                    .bytes()
                    .all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
            {
                Ok(())
            } else {
                Err("sha256 must be 64 lowercase hexadecimal characters.".to_owned())
            }
        })
    };
    input::field("paths", checked)?;
    Ok(paths
        .into_iter()
        .map(|p| PreserveItem {
            relative: p.relative,
            sha256: p.sha256,
            len: p.len,
        })
        .collect())
}

/// Convert the wire payload to the engine action after bounded validation.
fn action(op: RecoveryOp) -> Result<Action> {
    Ok(match op {
        RecoveryOp::Reconcile { intents: ids } => Action::Reconcile(intents("intents", ids)?),
        RecoveryOp::Retry { intents: ids } => Action::Retry(intents("intents", ids)?),
        RecoveryOp::Adopt { intents: ids } => Action::Adopt(intents("intents", ids)?),
        RecoveryOp::Release { intents: ids } => Action::Release(intents("intents", ids)?),
        RecoveryOp::Preserve { paths } => Action::Preserve(preserve(paths)?),
    })
}

/// Cut plain text to `cap` bytes on a character boundary, without quoting.
fn cut(text: &str, cap: usize) -> String {
    let mut end = text.len().min(cap);
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    text[..end].to_owned()
}

/// Cut a note to its byte limit.
fn note(line: &str) -> String {
    cut(line, NOTE_BYTES)
}

/// Receipt lines first, then at most two plain change lines, eight notes in all.
fn notes(receipt: &GitReceipt, changes: &[String]) -> Vec<String> {
    receipt
        .lines()
        .iter()
        .take(6)
        .chain(changes.iter().take(2))
        .take(MAX_NOTES)
        .map(|l| note(l))
        .collect()
}

/// Pending references as `intent (phase)` lines, the first `limit`.
fn pending_lines(store: &Store, limit: usize) -> Vec<String> {
    persist::pending(store)
        .refs
        .iter()
        .take(limit)
        .map(|p| format!("{} ({:?}, {} path(s))", p.intent, p.phase, p.paths))
        .collect()
}

/// Keep unknown and held facts visible on a refusal: the message names up to four, the ledger the rest.
fn refuse(store: &Store, error: Error, effects: &mut Vec<String>) -> Error {
    let all = pending_lines(store, MAX_REFS);
    if all.is_empty() || error.code == "stale" {
        return error;
    }
    effects.extend(
        all.iter()
            .skip(ERROR_REFS)
            .map(|l| format!("Still pending: {l}.")),
    );
    let named: Vec<_> = all.iter().take(ERROR_REFS).cloned().collect();
    Error::new(
        error.code,
        store::safe(
            &format!("{} Pending: {}.", error.message, named.join("; ")),
            400,
        ),
    )
}

/// Run one recovery under the dispatcher's lock; never locks and never replays a business write.
///
/// `common.version` must equal the pending snapshot version observed under this same guard, otherwise
/// the call fails with `stale` before any effect.
///
/// # Errors
/// `stale`, `invalid_arguments`, `not_locked`, `recovery_blocked` or `preserve_blocked`; a refusal
/// commits nothing and names up to four pending references.
pub fn execute_locked(
    store: &Store,
    guard: &LockGuard,
    common: &input::Common,
    op: RecoveryOp,
    effects: &mut Vec<String>,
) -> Result<work::Ack> {
    let action = action(op)?;
    let Report {
        changed,
        lines,
        receipt,
        version,
    } = recover(store, guard, &common.version, action).map_err(|e| refuse(store, e, effects))?;
    let pending = persist::pending(store);
    let mut ack = work::ack(
        TARGET,
        version,
        if pending.refs.is_empty() {
            "clear"
        } else {
            "pending"
        },
        changed,
    );
    ack.notes = notes(&receipt, &lines);
    ack.refs = pending
        .refs
        .iter()
        .take(MAX_REFS)
        .map(|p| cut(&p.intent, 256))
        .collect();
    Ok(ack)
}

/// Real-repository regressions for the recovery producer.
#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "Explicit isolated fixture assertions"
)]
mod tests {
    use super::*;
    use crate::persist::{
        Event, EventOutcome,
        policy::DeferAll,
        production_policy, settle,
        testing::{GitFixture, with_policy},
    };
    use crate::store::{Attest, Publish, RECORD_CAP};
    use serde_json::json;

    /// Common fields carrying a pending snapshot version.
    fn common(version: String) -> input::Common {
        input::Common {
            project: "docs".into(),
            version,
            actor: None,
            reference: None,
        }
    }

    /// A fresh request: store plus root write lock.
    fn call(f: &GitFixture) -> (Store, LockGuard) {
        let store = Store::from_root(f.dir.path()).unwrap();
        let guard = store.lock(true, &mut Vec::new()).unwrap().unwrap();
        (store, guard)
    }

    /// Publish one new file.
    fn create(store: &Store, relative: &str, bytes: &[u8]) {
        store
            .publish_with(
                Publish {
                    relative,
                    bytes,
                    observed: None,
                    cap: RECORD_CAP,
                    operation: None,
                    attest: Attest::Optional,
                },
                &mut Vec::new(),
            )
            .unwrap();
    }

    /// Settle a work event with the given outcome under the given policy.
    fn settle_with(
        store: &Store,
        guard: &LockGuard,
        outcome: EventOutcome,
        defer: bool,
    ) -> GitReceipt {
        let event = Event {
            class: EventClass::Work,
            refs: Vec::new(),
            operation: None,
            outcome,
        };
        if defer {
            with_policy(&DeferAll, || {
                settle(store, guard, &event, production_policy())
            })
        } else {
            settle(store, guard, &event, production_policy())
        }
    }

    /// The payload is closed: unknown operations and fields are rejected.
    #[test]
    fn the_wire_payload_is_closed() {
        assert!(
            serde_json::from_value::<RecoveryOp>(json!({"op": "release", "intents": []})).is_ok()
        );
        assert!(
            serde_json::from_value::<RecoveryOp>(json!({"op": "push", "intents": []})).is_err()
        );
        assert!(
            serde_json::from_value::<RecoveryOp>(
                json!({"op": "retry", "intents": [], "force": true})
            )
            .is_err()
        );
        assert!(serde_json::from_value::<RecoveryOp>(
            json!({"op": "preserve", "paths": [{"relative": "a", "sha256": "0", "len": 1, "mode": 1}]})
        )
        .is_err());
        assert_eq!(event_class(), EventClass::Recovery);
        assert!(DESCRIPTION.len() < 4096);
    }

    /// Bounds and shapes are refused before any recovery runs.
    #[test]
    fn invalid_payloads_name_their_field() {
        let f = GitFixture::new();
        let (store, guard) = call(&f);
        let version = persist::pending_version(&store);
        for op in [
            RecoveryOp::Release {
                intents: Vec::new(),
            },
            RecoveryOp::Retry {
                intents: vec!["PG-NOTHEX".into()],
            },
            RecoveryOp::Adopt {
                intents: vec!["PG-000000000000000000000000".into(); 17],
            },
            RecoveryOp::Preserve { paths: Vec::new() },
        ] {
            let error = execute_locked(
                &store,
                &guard,
                &common(version.clone()),
                op,
                &mut Vec::new(),
            )
            .err()
            .unwrap();
            assert_eq!(error.code, "invalid_arguments");
            assert!(
                error.message.starts_with("intents:") || error.message.starts_with("paths:"),
                "{}",
                error.message
            );
        }
    }

    /// A stale version fails before any effect.
    #[test]
    fn a_stale_version_changes_nothing() {
        let f = GitFixture::new();
        let (store, guard) = call(&f);
        create(&store, "doc.md", b"mine");
        let receipt = settle_with(&store, &guard, EventOutcome::Success, true);
        let intent = receipt.pending[0].intent.clone();
        let error = execute_locked(
            &store,
            &guard,
            &common("not-the-version".into()),
            RecoveryOp::Release {
                intents: vec![intent],
            },
            &mut Vec::new(),
        )
        .err()
        .unwrap();
        assert_eq!(error.code, "stale");
        assert_eq!(persist::pending(&store).refs.len(), 1);
    }

    /// Release stops tracking without touching files and reports a clear acknowledgement.
    #[test]
    fn release_acknowledges_a_clear_pending_state() {
        let f = GitFixture::new();
        let (store, guard) = call(&f);
        create(&store, "doc.md", b"mine");
        let intent = settle_with(&store, &guard, EventOutcome::Success, true).pending[0]
            .intent
            .clone();
        let version = persist::pending_version(&store);
        let ack = execute_locked(
            &store,
            &guard,
            &common(version),
            RecoveryOp::Release {
                intents: vec![intent],
            },
            &mut Vec::new(),
        )
        .unwrap();
        assert_eq!(
            (ack.target.as_str(), ack.phase.as_str(), ack.changed),
            (TARGET, "clear", true)
        );
        assert!(ack.refs.is_empty() && ack.notes.len() <= MAX_NOTES);
        assert!(ack.notes.iter().all(|n| n.len() <= NOTE_BYTES));
        assert!(f.dir.path().join("doc.md").exists());
    }

    /// Retry commits a held intent and the acknowledgement carries the receipt first.
    #[test]
    fn retry_commits_a_held_intent_with_a_receipt_note() {
        let f = GitFixture::new();
        let (store, guard) = call(&f);
        create(&store, "doc.md", b"mine");
        let intent = settle_with(&store, &guard, EventOutcome::Partial, false).pending[0]
            .intent
            .clone();
        drop(guard);
        let (next, guard) = call(&f);
        let version = persist::pending_version(&next);
        let ack = execute_locked(
            &next,
            &guard,
            &common(version),
            RecoveryOp::Retry {
                intents: vec![intent],
            },
            &mut Vec::new(),
        )
        .unwrap();
        assert!(ack.changed, "{:?}", ack.notes);
        assert_eq!(
            (ack.phase.as_str(), ack.refs.len()),
            ("clear", 0),
            "{:?}",
            ack.refs
        );
        assert!(
            ack.notes[0].starts_with("Git: committed"),
            "{:?}",
            ack.notes
        );
        assert!(ack.notes.len() <= MAX_NOTES);
    }

    /// A refusal names pending facts and commits nothing.
    #[test]
    fn a_refusal_names_the_remaining_pending_work() {
        let f = GitFixture::new();
        let (store, guard) = call(&f);
        create(&store, "doc.md", b"mine");
        settle_with(&store, &guard, EventOutcome::Success, true);
        let version = persist::pending_version(&store);
        let before = f.git(&["rev-list", "--count", "HEAD"]);
        let error = execute_locked(
            &store,
            &guard,
            &common(version),
            RecoveryOp::Retry {
                intents: vec!["PG-ffffffffffffffffffffffff".into()],
            },
            &mut Vec::new(),
        )
        .err()
        .unwrap();
        assert_eq!(error.code, "recovery_blocked");
        assert!(error.message.contains("Pending: PG-"), "{}", error.message);
        assert_eq!(f.git(&["rev-list", "--count", "HEAD"]), before);
    }
}
