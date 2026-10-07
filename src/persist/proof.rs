//! Committed-bytes proof: exact bytes that live in a commit reachable from the attached HEAD.
//!
//! A pending, deferred, ignored or working-tree file is never proof. All Git is read-only, bounded and
//! never fetches. This file grows with locators and per-item proofs; the first piece is reachability,
//! which the journal uses before it drops a committed intent.
use crate::{persist::git, store::Store};
use std::time::Instant;

/// Whether `commit` is a full object id that is an ancestor of (or equal to) the current HEAD.
pub fn reachable(store: &Store, commit: &str) -> bool {
    if !(commit.len() == 40 || commit.len() == 64)
        || !commit
            .bytes()
            .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
    {
        return false;
    }
    let deadline = Instant::now() + git::COMMAND_TIME;
    git::run(
        &store.root,
        &["merge-base", "--is-ancestor", commit, "HEAD"],
        None,
        deadline,
    )
    .is_ok_and(|o| o.success())
}
