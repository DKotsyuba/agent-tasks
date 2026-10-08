//! Reviewed documentation compaction: bounded revision-bound proposals, independent review and
//! guarded, interruption-safe application of Markdown document changes.
//!
//! The domain owns the `compactions/CP-NNN.yaml` record (every full prior revision and every review
//! kept append-only), the closed nested `compactions/` inventory that the knowledge allocator composes,
//! proposal validation, the persistent independent reviewer rules and the apply engine. It imports
//! nothing from `tools`: the producer `tools::compaction_ops` decodes the wire payload and converts the
//! result into the shared acknowledgement.
//!
//! Provider facts (documents, references, publication provenance, allocation) enter only through the
//! private adapter [`env::Env`]; the module reimplements no Store algorithm, YAML validator, metadata
//! serializer or effect journal. See `docs/contracts/compaction.md` for the agreed boundaries.
// Wiring into the shared registry belongs to the registry owner; until it lands, public domain items are
// legitimately unused by the binary crate.
#![allow(dead_code)]

/// Guarded, interruption-safe application of an accepted proposal.
pub mod apply;
/// The thin adapter between the domain and the provider modules.
pub mod env;
/// Pure apply decisions: post images, oracle classification and the removal gate.
pub mod gate;
/// The closed nested compactions inventory and the record scan.
pub mod inventory;
/// The live adapter over the real document, reference, knowledge, store and persistence providers.
pub mod live;
/// Proposal lifecycle operations: propose, revise, review, reviewer recovery and withdraw.
pub mod ops;
/// Read-only views of compaction records for context and status.
pub mod read;
/// The closed record model, capacities, deterministic names and revision hash.
pub mod record;
/// Independent review, the persistent reviewer and loss-only replacement.
pub mod review;
/// Proposal validation, ledger accounting, incoming coverage and acceptance digests.
pub mod validate;

/// Module controls against the faithful provider substitute.
#[cfg(test)]
mod composed_tests;
/// Faithful test substitute of the provider facts behind the adapter; module evidence only.
#[cfg(test)]
pub(crate) mod fake;
#[cfg(test)]
mod tests;

#[allow(
    unused_imports,
    reason = "Public domain API consumed once the registry and allocator integrate it"
)]
pub use {
    inventory::{CpScan, inventory, scan},
    read::{
        CpSummaries, CpSummaryRow, StagedCandidate, read_cp, read_staged, retained_revision,
        summaries,
    },
    record::*,
};
