//! Pure commit policy and the event schema a settlement is asked about.
//!
//! The owner-approved production policy is [`CommitAfterSuccess`]: commit after each successful
//! actual mutation of work, knowledge, documents or compaction. It is a pure function over typed
//! facts: no I/O, no clock, no configuration, no thresholds, no timers and no shipping switch.
use crate::store::OperationId;

/// Closed set of mutation families. Producers pick one; the set grows only by a new contract revision.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum EventClass {
    /// Existing work, Epic, Atomic, Module and review mutations.
    Work,
    /// Typed knowledge records and checklists.
    Knowledge,
    /// Managed Markdown documents and their metadata.
    Document,
    /// Creating a compaction proposal.
    CompactionPropose,
    /// Revising a compaction proposal.
    CompactionRevise,
    /// Reviewing a compaction proposal.
    CompactionReview,
    /// Applying an accepted compaction proposal.
    CompactionApply,
    /// Withdrawing a compaction proposal.
    CompactionWithdraw,
    /// Explicit Git recovery; commits only what it was asked to commit.
    Recovery,
}

impl EventClass {
    /// Stable lowercase name used in the journal and in commit subjects.
    pub fn name(self) -> &'static str {
        match self {
            Self::Work => "work",
            Self::Knowledge => "knowledge",
            Self::Document => "document",
            Self::CompactionPropose => "compaction-propose",
            Self::CompactionRevise => "compaction-revise",
            Self::CompactionReview => "compaction-review",
            Self::CompactionApply => "compaction-apply",
            Self::CompactionWithdraw => "compaction-withdraw",
            Self::Recovery => "recovery",
        }
    }
}

/// How the business call ended, as judged by the dispatcher.
///
/// `Success` means the handler returned Ok. `Partial` means it returned an error after publishing at
/// least one effect. `Failed` means an error with no publication at all: nothing was written, no
/// intent exists and settlement is not called. Settlement refines `Success` to `Partial` when any
/// event of the call has `SyncUnknown`, `UnknownAfterPublication` or is untracked.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EventOutcome {
    /// The handler returned Ok.
    Success,
    /// The handler failed after publishing at least one effect.
    Partial,
    /// The handler failed before publishing anything.
    Failed,
}

/// What a settlement is asked about; built by the dispatcher, never by producers.
#[derive(Debug, Clone)]
pub struct Event {
    /// Mutation family.
    pub class: EventClass,
    /// Advisory canonical references of affected records or documents: up to 16, each up to 256 bytes.
    pub refs: Vec<String>,
    /// Caller operation identity, when the call has one.
    pub operation: Option<OperationId>,
    /// Dispatcher-judged outcome.
    pub outcome: EventOutcome,
}

/// Read-only pending facts a policy may use; no file content and no Git output.
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize)]
pub struct PendingFacts {
    /// Open intents that are not committed.
    pub intents: usize,
    /// Path entries across those intents.
    pub paths: usize,
    /// Creation time of the oldest open intent, in Unix seconds.
    pub oldest_unix_secs: Option<u64>,
    /// Intents whose provenance is unknown.
    pub unknown: usize,
    /// Intents with native drift.
    pub drifted: usize,
}

/// Everything the pure policy sees about one settlement.
#[derive(Debug, Clone)]
pub struct PolicyInput<'a> {
    /// The event.
    pub event: &'a Event,
    /// Pending facts at settlement time.
    pub pending: &'a PendingFacts,
    /// Tracked eligible-file publications this call produced; 0 for a true no-op or when only
    /// not-applicable effects occurred.
    pub tracked: usize,
    /// Some event of this call, directories included, is `SyncUnknown` or `UnknownAfterPublication`.
    pub sync_uncertain: bool,
    /// Eligible-file publications of this call without a journal entry; not-applicable effects never count.
    pub untracked: usize,
}

/// A policy's answer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Decision {
    /// Run the Git engine over the eligible intents.
    Commit,
    /// Do not commit now.
    Defer(DeferReason),
    /// Nothing is due.
    Skip,
}

/// Why a policy deferred; other reasons are engine states, never policy output.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeferReason {
    /// A test policy that never commits.
    PolicyDeferred,
    /// The call was partial, failed, sync-uncertain or had untracked eligible files.
    IncompleteOutcome,
}

/// Pure commit policy: no I/O, no clock reads, no global state, no configuration.
pub trait Policy {
    /// Decide what to do about one settlement.
    fn decide(&self, input: &PolicyInput<'_>) -> Decision;
}

/// The production policy: commit after each successful actual mutation.
///
/// Recovery events skip (recovery commits only what it was explicitly asked to); a call with no
/// tracked eligible file skips; a partial, failed, sync-uncertain call or one with an untracked
/// eligible file is deferred as incomplete and is never committed as a tracked subset; every other
/// call commits.
pub struct CommitAfterSuccess;

impl Policy for CommitAfterSuccess {
    /// See the type documentation for the exact rules.
    fn decide(&self, input: &PolicyInput<'_>) -> Decision {
        if input.event.class == EventClass::Recovery || input.tracked == 0 {
            Decision::Skip
        } else if input.event.outcome != EventOutcome::Success
            || input.sync_uncertain
            || input.untracked > 0
        {
            Decision::Defer(DeferReason::IncompleteOutcome)
        } else {
            Decision::Commit
        }
    }
}

/// Test-only policy that never commits, for deferral and pending scenarios.
#[cfg(test)]
pub struct DeferAll;

#[cfg(test)]
impl Policy for DeferAll {
    /// Always defers with [`DeferReason::PolicyDeferred`].
    fn decide(&self, _: &PolicyInput<'_>) -> Decision {
        Decision::Defer(DeferReason::PolicyDeferred)
    }
}

/// The policy a production dispatcher passes: always [`CommitAfterSuccess`]. In tests an in-process
/// override installed by `testing::with_policy` applies to the calling thread only.
pub fn production_policy() -> &'static dyn Policy {
    #[cfg(test)]
    if let Some(policy) = super::testing::overridden_policy() {
        return policy;
    }
    &CommitAfterSuccess
}

/// Pure policy regressions.
#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "Explicit isolated assertions"
)]
mod tests {
    use super::*;

    /// Build an event of one class and outcome.
    fn event(class: EventClass, outcome: EventOutcome) -> Event {
        Event {
            class,
            refs: Vec::new(),
            operation: None,
            outcome,
        }
    }

    /// Ask the production policy about one set of facts.
    fn ask(
        class: EventClass,
        outcome: EventOutcome,
        tracked: usize,
        uncertain: bool,
        untracked: usize,
    ) -> Decision {
        let e = event(class, outcome);
        let facts = PendingFacts::default();
        CommitAfterSuccess.decide(&PolicyInput {
            event: &e,
            pending: &facts,
            tracked,
            sync_uncertain: uncertain,
            untracked,
        })
    }

    /// Each mutation family commits after a successful tracked call and nothing else does.
    #[test]
    fn commits_every_family_after_success_and_nothing_else() {
        for class in [
            EventClass::Work,
            EventClass::Knowledge,
            EventClass::Document,
            EventClass::CompactionPropose,
            EventClass::CompactionRevise,
            EventClass::CompactionReview,
            EventClass::CompactionApply,
            EventClass::CompactionWithdraw,
        ] {
            assert_eq!(
                ask(class, EventOutcome::Success, 2, false, 0),
                Decision::Commit
            );
            assert_eq!(
                ask(class, EventOutcome::Success, 0, false, 0),
                Decision::Skip,
                "true no-op"
            );
            assert_eq!(
                ask(class, EventOutcome::Partial, 2, false, 0),
                Decision::Defer(DeferReason::IncompleteOutcome)
            );
            assert_eq!(
                ask(class, EventOutcome::Success, 2, true, 0),
                Decision::Defer(DeferReason::IncompleteOutcome),
                "uncertain sync"
            );
            assert_eq!(
                ask(class, EventOutcome::Success, 2, false, 1),
                Decision::Defer(DeferReason::IncompleteOutcome),
                "untracked eligible sibling"
            );
        }
        assert_eq!(
            ask(EventClass::Recovery, EventOutcome::Success, 3, false, 0),
            Decision::Skip
        );
    }

    /// The production policy is the real policy unless a test installs a thread-local override.
    #[test]
    fn production_policy_is_commit_after_success() {
        let e = event(EventClass::Work, EventOutcome::Success);
        let facts = PendingFacts::default();
        let input = PolicyInput {
            event: &e,
            pending: &facts,
            tracked: 1,
            sync_uncertain: false,
            untracked: 0,
        };
        assert_eq!(production_policy().decide(&input), Decision::Commit);
        crate::persist::testing::with_policy(&DeferAll, || {
            assert_eq!(
                production_policy().decide(&input),
                Decision::Defer(DeferReason::PolicyDeferred)
            );
        });
        assert_eq!(production_policy().decide(&input), Decision::Commit);
    }
}
