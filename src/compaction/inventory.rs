//! The closed nested `compactions/` inventory (provider side of the `cp-inventory` boundary) and the
//! full record scan used by reads.
//!
//! [`inventory()`] reads names only, through a bounded non-recursive listing, and is the one function the
//! knowledge allocation observation calls for the compaction home: it never calls the allocator,
//! never decodes a record and never writes. [`scan`] additionally decodes every record for reads and
//! summaries.
use super::record::{self, CpRecord, HOME, MAX_REVISIONS};
use crate::store::{
    self, DirListing as Listing, EntryKind, Inventory, MODULE_CAP, Result, Snapshot, Store,
};
use std::collections::BTreeMap;

/// Maximum entries listed under one `CP-NNN` directory or one revision directory.
pub const NESTED_CAP: usize = 512;

/// The leaf name a store publication leftover `.<name>.tmp-<pid>-<seq>` or `.<name>.rm-<pid>-<seq>`
/// belongs to; `None` for every other name, so foreign dotfiles stay foreign.
///
/// Recognition is decided by `store::own_temp_name`; the stem is then split from the same grammar because
/// the storage owner exposes only the boolean.
pub fn own_temp_leaf(name: &str) -> Option<&str> {
    if !store::own_temp_name(name) {
        return None;
    }
    let rest = name.strip_prefix('.')?;
    for marker in [".tmp-", ".rm-"] {
        if let Some((leaf, suffix)) = rest.rsplit_once(marker) {
            let ok = suffix
                .split_once('-')
                .is_some_and(|(pid, seq)| pid.parse::<u32>().is_ok() && seq.parse::<u64>().is_ok());
            if ok && !leaf.is_empty() && !leaf.contains(['/', '\0']) {
                return Some(leaf);
            }
        }
    }
    None
}

/// Parse `CP-<canonical digits>` and return its number.
fn cp_number(stem: &str) -> Option<u64> {
    record::parse_cp_id(stem).ok()
}

/// Parse a revision directory name `rN` with N in 1 to 8, canonical decimal.
fn revision_number(name: &str) -> Option<u32> {
    let n = name.strip_prefix('r')?;
    if n.starts_with('0') || n.len() > 1 {
        return None;
    }
    n.parse::<u32>()
        .ok()
        .filter(|v| (1..=MAX_REVISIONS).contains(v))
}

/// Parse a blob name `A-NN.md` with NN in 01 to 32.
fn blob_name(name: &str) -> bool {
    name.strip_suffix(".md")
        .is_some_and(record::parse_action_id)
}

/// The compaction inventory over an abstract bounded lister; pure grammar, no other I/O.
///
/// Grammar: `compactions/` absent is complete and empty. A regular `CP-<canonical>.yaml` counts its
/// identifier. A `CP-<canonical>` directory counts the same identifier even without a record, with the
/// warning `CP-NNN: staged directory without a record`. Below a directory only `rN` (1 to 8) revision
/// directories are allowed, and below those only regular `A-NN.md` (01 to 32) blobs. Only regular own
/// publication leftover files are warnings; the same names as a link, directory or other nonregular
/// entry are foreign. Every other entry, link, nonregular file, noncanonical name, depth three entry or
/// capped listing is named and makes the inventory incomplete. Never recurses beyond depth two.
pub fn inventory_with(list: &dyn Fn(&str, usize) -> Result<Listing>) -> Result<Inventory> {
    let top = list(HOME, MODULE_CAP)?;
    let mut complete = top.complete;
    let mut warnings = Vec::new();
    let mut records: BTreeMap<u64, bool> = BTreeMap::new();
    let mut dirs: BTreeMap<u64, bool> = BTreeMap::new();
    for entry in &top.entries {
        let name = entry.name.as_str();
        if entry.kind == EntryKind::File
            && let Some(leaf) = own_temp_leaf(name)
            && leaf.strip_suffix(".yaml").and_then(cp_number).is_some()
        {
            warnings.push(format!("{name}: publication leftover ignored."));
            continue;
        }
        match (entry.kind, name.strip_suffix(".yaml")) {
            (EntryKind::File, Some(stem)) => {
                if let Some(n) = cp_number(stem) {
                    records.insert(n, true);
                    continue;
                }
            }
            (EntryKind::Directory, None) => {
                if let Some(n) = cp_number(name) {
                    dirs.insert(n, true);
                    continue;
                }
            }
            _ => {}
        }
        complete = false;
        warnings.push(format!("{name}: unrecognized entry in {HOME}/."));
    }
    for n in dirs.keys() {
        if !records.contains_key(n) {
            warnings.push(format!("CP-{n:03}: staged directory without a record."));
        }
        let id = format!("CP-{n:03}");
        let home = format!("{HOME}/{id}");
        let children = list(&home, NESTED_CAP)?;
        complete &= children.complete;
        for child in &children.entries {
            let rel = format!("{home}/{}", child.name);
            let Some(rev) = (child.kind == EntryKind::Directory)
                .then(|| revision_number(&child.name))
                .flatten()
            else {
                complete = false;
                warnings.push(format!("{rel}: unrecognized entry in {home}/."));
                continue;
            };
            let blobs = list(&format!("{home}/r{rev}"), NESTED_CAP)?;
            complete &= blobs.complete;
            for blob in &blobs.entries {
                if blob.kind == EntryKind::File
                    && let Some(leaf) = own_temp_leaf(&blob.name)
                    && blob_name(leaf)
                {
                    warnings.push(format!(
                        "{rel}/{}: publication leftover ignored.",
                        blob.name
                    ));
                    continue;
                }
                if blob.kind == EntryKind::File && blob_name(&blob.name) {
                    continue;
                }
                complete = false;
                warnings.push(format!(
                    "{rel}/{}: unrecognized entry in a revision directory.",
                    blob.name
                ));
            }
        }
    }
    let ids = records
        .keys()
        .chain(dirs.keys())
        .copied()
        .collect::<std::collections::BTreeSet<_>>()
        .into_iter()
        .map(|n| format!("CP-{n:03}"))
        .collect();
    Ok(Inventory {
        ids,
        warnings,
        complete,
    })
}

/// Names of the compaction home as read from the store: the one function the knowledge allocation
/// observation composes. Reads names only; never decodes, writes, creates the home or allocates.
pub fn inventory(store: &Store) -> Result<Inventory> {
    inventory_with(&|relative, cap| store.list_dir(relative, cap))
}

/// Everything read from the compaction home, with unreadable records named rather than dropped.
pub struct CpScan {
    /// Healthy records in numeric order.
    pub records: Vec<Snapshot<CpRecord>>,
    /// Identifiers with a staged directory but no record.
    pub orphans: Vec<String>,
    /// Named unreadable, oversized, symlinked or misnamed records.
    pub unreadable: Vec<String>,
    /// Inventory warnings.
    pub warnings: Vec<String>,
    /// True only when the inventory is complete and every record decoded.
    pub complete: bool,
    /// Scope bound digest over the scanned record versions.
    pub version: String,
}

/// Scan the compaction home and decode every record.
///
/// A corrupt record becomes a named `unreadable` row and clears `complete`; one bad sibling is never an
/// error and never a silent drop.
pub fn scan(store: &Store) -> Result<CpScan> {
    let inv = inventory(store)?;
    let mut records = Vec::new();
    let mut orphans = Vec::new();
    let mut unreadable = Vec::new();
    let mut complete = inv.complete;
    let mut versions = Vec::new();
    for id in &inv.ids {
        let relative = record::record_path(id);
        match store.bytes(&relative) {
            Ok(Some(bytes)) => match record::decode_record(&bytes, id) {
                Ok(value) => {
                    let version = store.version(&relative, Some(&bytes));
                    versions.push(format!("{id}:{version}"));
                    records.push(Snapshot {
                        value,
                        bytes,
                        version,
                    });
                }
                Err(_) => {
                    complete = false;
                    unreadable.push(format!("{id}: record cannot be decoded."));
                }
            },
            Ok(None) => orphans.push(id.clone()),
            Err(e) => {
                complete = false;
                unreadable.push(format!("{id}: {}", store::safe(&e.message, 120)));
            }
        }
    }
    let version = record::digest_parts(
        "agent-tasks/cp-scan/v1",
        &versions.iter().map(|v| v.as_bytes()).collect::<Vec<_>>(),
    );
    Ok(CpScan {
        records,
        orphans,
        unreadable,
        warnings: inv.warnings,
        complete,
        version,
    })
}
