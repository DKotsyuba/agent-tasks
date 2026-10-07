//! Automatic local Git persistence: private provenance journal, bounded Git runner, committed-bytes
//! proof, pure policy, receipts, settlement and recovery. Store primitives in `store` call into the
//! journal; nothing here ever takes the root write lock, and reads never commit.
#![allow(
    dead_code,
    unused_imports,
    reason = "Early primitives handoff: consumers in other Modules land later and this allowance is removed with them"
)]
pub mod engine;
pub mod git;
pub mod journal;
pub mod locator;
pub mod policy;
pub mod proof;
pub mod receipt;
pub mod recover;
pub mod status;
#[cfg(test)]
mod tests;

pub use engine::{settle, settled};
pub use policy::{
    CommitAfterSuccess, Decision, DeferReason, Event, EventClass, EventOutcome, PendingFacts,
    Policy, PolicyInput, production_policy,
};
pub use receipt::{Attention, EarlierCommit, GitOutcome, GitReceipt, PendingRef, Phase, Reason};
pub use status::{
    CallIntents, EffectGit, EffectReceipt, EffectStatus, ExpectedEffect, ForeignReason, IntentView,
    PathEffect, PendingSummary, UnknownReason, call_intents, effect_status, intent_view, pending,
    pending_version, receipt,
};

/// Disposable real-repository fixtures for this Module's tests and for other Modules' in-crate tests.
#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "Explicit isolated fixture construction"
)]
pub mod testing {
    use super::{git, policy::Policy};
    use crate::store::{LockGuard, Store};
    use std::{
        cell::Cell,
        time::{Duration, Instant},
    };

    std::thread_local! {
        /// In-process policy override for the calling thread only; never a shipping option.
        static OVERRIDE: Cell<Option<&'static dyn Policy>> = const { Cell::new(None) };
    }

    /// The policy installed by [`with_policy`] on this thread, if any.
    pub(super) fn overridden_policy() -> Option<&'static dyn Policy> {
        OVERRIDE.with(Cell::get)
    }

    /// Run `body` with `policy` visible to `production_policy()` on this thread only.
    pub fn with_policy<R>(policy: &'static dyn Policy, body: impl FnOnce() -> R) -> R {
        let previous = OVERRIDE.with(|slot| slot.replace(Some(policy)));
        let result = body();
        OVERRIDE.with(|slot| slot.set(previous));
        result
    }

    /// A temporary documentation root that is an independent Git repository with one commit.
    ///
    /// Git commands run with isolated configuration, a fixed identity and no signing, so a test never
    /// reads or changes the machine's own Git setup.
    pub struct GitFixture {
        /// Keeps the temporary directory alive for the test.
        pub dir: tempfile::TempDir,
        /// Store resolved on the fixture root.
        pub store: Store,
    }

    /// Install the isolated Git environment on the calling thread.
    pub fn isolate_git() {
        git::set_test_environment(
            [
                ("GIT_CONFIG_GLOBAL", "/dev/null"),
                ("GIT_CONFIG_NOSYSTEM", "1"),
                ("GIT_AUTHOR_NAME", "Fixture Author"),
                ("GIT_AUTHOR_EMAIL", "author@example.invalid"),
                ("GIT_COMMITTER_NAME", "Fixture Committer"),
                ("GIT_COMMITTER_EMAIL", "committer@example.invalid"),
            ]
            .map(|(k, v)| (k.to_owned(), v.to_owned()))
            .to_vec(),
        );
    }

    impl GitFixture {
        /// Create the fixture: `git init`, an initial commit and the owned `.agent-tasks/` directory.
        pub fn new() -> Self {
            isolate_git();
            let dir = tempfile::tempdir().unwrap();
            let store = Store::from_root(dir.path()).unwrap();
            let fixture = Self { dir, store };
            fixture.git(&["init", "--quiet", "--initial-branch=main"]);
            fixture.git(&["config", "commit.gpgsign", "false"]);
            std::fs::write(
                fixture.dir.path().join(".gitignore"),
                ".agent-tasks/write.lock\n.agent-tasks/backups/\n**/.*.tmp-*\n**/.*.rm-*\n",
            )
            .unwrap();
            fixture.git(&["add", "--", ".gitignore"]);
            fixture.git(&["commit", "--quiet", "-m", "fixture: initial"]);
            fixture.store.prepare(&mut Vec::new()).unwrap();
            fixture
        }

        /// Run Git in the fixture and return trimmed stdout; panics when Git fails.
        pub fn git(&self, args: &[&str]) -> String {
            let out = git::run(
                self.dir.path(),
                args,
                None,
                Instant::now() + Duration::from_secs(20),
            )
            .unwrap();
            assert!(
                out.success(),
                "git {args:?} failed: {}",
                String::from_utf8_lossy(&out.stderr)
            );
            out.text()
        }

        /// Acquire the cooperative root write lock through the fixture store.
        pub fn lock(&self) -> LockGuard {
            self.store
                .lock(true, &mut Vec::new())
                .unwrap()
                .expect("write lock")
        }
    }
}
