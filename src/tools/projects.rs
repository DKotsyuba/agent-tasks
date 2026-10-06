//! Project registration and discovery; paths stay inside the registry, not normal agent output.
use super::{
    input::{ProjectListArgs, RegisterArgs},
    read::{continuation, page, render_page},
    work::initialize,
};
use crate::{
    model::{self, Project, SCHEMA},
    response::Templates,
    store::{self, Config, Error, Result, Store},
};
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::{
    fs,
    path::Path,
    process::{Command, Stdio},
    time::{Duration, Instant},
};

/// Compact registration receipt with an alias and confirmed effects; no filesystem location.
#[derive(Serialize)]
struct Registered<'a> {
    /// Stable alias used in every subsequent business call.
    project: &'a str,
    /// Display-safe project title.
    name: String,
    /// Existing manifest's whole-file version.
    version: String,
    /// Whether registration changed any owned files or binding.
    changed: bool,
    /// Independently captured effects, retained if presentation fails.
    effects: &'a [String],
}

/// Bootstrap ignore rules retain the allocator, excluding only locks, backups and publication temps.
const IGNORE: &[u8] = b".agent-tasks/write.lock\n.agent-tasks/backups/\n**/.*.tmp-*\n";

/// Run local Git with bounded time and no provider output in tool responses.
/// Caller-supplied paths are argv values; inherited Git directory/index overrides are removed.
/// Nonzero exit returns false. Spawn/wait/timeout failures refuse without claiming rollback.
/// Unit fixtures ignore machine Git configuration; shipping calls preserve signing and hooks.
fn git(root: &Path, args: &[&str]) -> Result<bool> {
    let mut command = Command::new("git");
    command
        .current_dir(root)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    for name in [
        "GIT_DIR",
        "GIT_WORK_TREE",
        "GIT_INDEX_FILE",
        "GIT_COMMON_DIR",
    ] {
        command.env_remove(name);
    }
    #[cfg(test)]
    command
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_NOSYSTEM", "1");
    let mut child = command.spawn().map_err(|_| {
        Error::new(
            "git",
            "Cannot start Git; install Git and inspect registration before retrying.",
        )
    })?;
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        if let Some(status) = child.try_wait().map_err(|_| {
            Error::new(
                "git",
                "Cannot observe Git outcome; inspect the documentation repository.",
            )
        })? {
            return Ok(status.success());
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            return Err(Error::new(
                "git_outcome_unknown",
                "Git exceeded 30 seconds; existing files/staging remain. Inspect the documentation repository before retrying.",
            ));
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}

/// Generate a short Markdown entrypoint; human fields are literal content, never executable templates.
fn readme(alias: &str, project: &Project) -> Vec<u8> {
    format!("# {}\n\n{}\n\n## Agent workflow\n\nUse project=\"{}\" with get_context, project_status and search.\nPlan Modules and Tasks with plan_work; record outcomes with record_work.\nReview the whole Module with review_module.\n\n## Storage\n\nproject.yaml owns project intent. modules/ contains structured work.\n.agent-tasks/state.yaml keeps generated numbering. Markdown holds free-form documents.\nSource code and docstrings describe implemented behavior.\n", project.title, project.purpose, alias).into_bytes()
}

/// Create one missing bootstrap document without overwriting a differing existing file.
fn bootstrap(store: &Store, relative: &str, bytes: &[u8], effects: &mut Vec<String>) -> Result<()> {
    match store.bytes(relative)? {
        Some(existing) if existing == bytes => Ok(()),
        Some(_) => Err(Error::new(
            "conflict",
            format!("{relative} already contains different content; nothing is overwritten."),
        )),
        None => store.publish(relative, bytes, None, effects),
    }
}

/// Initialize/finish a local documentation repository and commit only the four bootstrap records.
/// An existing HEAD is preserved. docs_remote is separate from the source-code manifest remote.
/// No push, fetch, agent launch, history rewrite or ongoing auto-commit occurs.
fn bootstrap_git(
    store: &Store,
    docs_remote: Option<&str>,
    effects: &mut Vec<String>,
) -> Result<()> {
    let path = store.path(".git")?;
    if path.exists() && !path.is_dir() {
        return Err(Error::new(
            "conflict",
            "A Git worktree file is not an independent documentation repository.",
        ));
    }
    if !path.exists() {
        if !git(&store.root, &["init", "--quiet", "--initial-branch=main"])? {
            return Err(Error::new(
                "git",
                "Git initialization failed; inspect the documentation root.",
            ));
        }
        effects.push("Initialized documentation Git repository.".into());
    }
    store
        .bytes(".git/config")?
        .ok_or_else(|| Error::new("git", "Git configuration is missing."))?;
    if let Some(remote) = docs_remote {
        if git(
            &store.root,
            &["config", "--local", "--get", "remote.origin.url"],
        )? {
            if !git(
                &store.root,
                &[
                    "config",
                    "--local",
                    "--fixed-value",
                    "--get",
                    "remote.origin.url",
                    remote,
                ],
            )? {
                return Err(Error::new(
                    "conflict",
                    "Documentation origin already differs; it is not replaced.",
                ));
            }
        } else {
            if !git(&store.root, &["remote", "add", "origin", remote])? {
                return Err(Error::new(
                    "git",
                    "Cannot configure documentation origin; inspect the repository.",
                ));
            }
            effects.push("Configured documentation origin; no network operation.".into());
        }
    }
    if git(&store.root, &["rev-parse", "--verify", "--quiet", "HEAD"])? {
        return Ok(());
    }
    let files = [
        "project.yaml",
        ".agent-tasks/state.yaml",
        "README.md",
        ".gitignore",
    ];
    let mut add = vec!["add", "--"];
    add.extend(files);
    if !git(&store.root, &add)? {
        return Err(Error::new(
            "git",
            "Cannot stage bootstrap files; inspect the repository.",
        ));
    }
    let mut commit = vec![
        "-c",
        "user.name=agent-tasks",
        "-c",
        "user.email=agent-tasks@localhost",
        "commit",
        "--quiet",
        "--only",
        "-m",
        "docs: initialize project documentation",
        "--",
    ];
    commit.extend(files);
    if !git(&store.root, &commit)? {
        return Err(Error::new(
            "git",
            "Initial commit failed; files/staging remain. Inspect Git configuration and hooks before retrying.",
        ));
    }
    effects.push("Committed initial documentation.".into());
    Ok(())
}

/// Register one explicit absolute documentation root under registry then root locks.
/// Identical completed bindings are no-ops; conflicts never retarget aliases or replace files.
/// Partial local effects remain visible; the alias is published only after bootstrap Git succeeds.
pub(super) fn register(
    config: &Config,
    args: RegisterArgs,
    templates: &Templates,
    effects: &mut Vec<String>,
) -> Result<String> {
    model::text(&args.project, 128).map_err(store::invalid)?;
    if !args
        .project
        .bytes()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, b'-' | b'_' | b'.'))
    {
        return Err(Error::new(
            "invalid_arguments",
            "Project aliases use ASCII letters, digits, dash, underscore or dot.",
        ));
    }
    let docs_remote = args.docs_remote.filter(|r| !r.is_empty());
    model::optional(&docs_remote, 256).map_err(store::invalid)?;
    if docs_remote.as_ref().is_some_and(|r| r.starts_with('-')) {
        return Err(Error::new(
            "invalid_arguments",
            "Documentation remote cannot start with an option marker.",
        ));
    }
    let at = store::now();
    let desired = Project {
        schema_version: SCHEMA,
        title: args.name,
        purpose: args.description,
        remote: args.remote.filter(|r| !r.is_empty()),
        created_at: at.clone(),
        updated_at: at,
    };
    desired.validate().map_err(store::invalid)?;
    let store = Store::from_root(&args.doc_dir)?;
    if store.root.parent().is_none() {
        return Err(Error::new(
            "path",
            "A filesystem root cannot be a documentation repository.",
        ));
    }
    let registry_store = config.prepare_registry(effects)?;
    let _registry_lock = registry_store.file_lock("projects.lock", true, effects)?;
    let (mut registry, observed) = config.aliases()?;
    if let Some(existing) = registry.aliases.get(&args.project) {
        if Store::from_root(existing)?.root != store.root {
            return Err(Error::new(
                "conflict",
                "Alias already points to another root; it is never retargeted implicitly.",
            ));
        }
    } else if registry
        .aliases
        .values()
        .any(|path| Store::from_root(path).is_ok_and(|s| s.root == store.root))
    {
        return Err(Error::new(
            "conflict",
            "Documentation root already has another alias; use get_project_list.",
        ));
    }
    if registry.aliases.contains_key(&args.project) {
        let _root_lock = store.lock(false, &mut Vec::new())?;
        if let Some(snapshot) = store.project()? {
            if snapshot.value.title != desired.title
                || snapshot.value.purpose != desired.purpose
                || snapshot.value.remote != desired.remote
            {
                return Err(Error::new(
                    "conflict",
                    "Project metadata differs; use edit_project rather than replaying registration.",
                ));
            }
            if let Some(remote) = docs_remote.as_deref() {
                store.bytes(".git/config")?.ok_or_else(|| {
                    Error::new("conflict", "Documentation Git configuration is missing.")
                })?;
                if !git(
                    &store.root,
                    &[
                        "config",
                        "--local",
                        "--fixed-value",
                        "--get",
                        "remote.origin.url",
                        remote,
                    ],
                )? {
                    return Err(Error::new(
                        "conflict",
                        "Documentation origin differs; registration never changes an existing origin.",
                    ));
                }
            }
            return render_registration(
                &args.project,
                &snapshot.value,
                snapshot.version,
                false,
                effects,
                templates,
            );
        }
    } else if registry.aliases.len() >= 256 {
        return Err(Error::new(
            "capacity",
            "Registry already contains 256 aliases.",
        ));
    }
    // Refuse foreign content before preparing a new root; matching partial bootstrap can resume.
    if store.root.exists() {
        for entry in fs::read_dir(&store.root)
            .map_err(|_| Error::new("io", "Cannot inspect documentation root."))?
        {
            let name = entry
                .map_err(|_| Error::new("io", "Cannot inspect documentation entry."))?
                .file_name();
            if ![
                "project.yaml",
                "modules",
                "epics",
                "atomics",
                ".agent-tasks",
                "README.md",
                ".gitignore",
                ".git",
            ]
            .iter()
            .any(|allowed| name == *allowed)
            {
                return Err(Error::new(
                    "conflict",
                    "Nonempty documentation root contains foreign files; they are not overwritten.",
                ));
            }
        }
    }
    store.prepare(effects)?;
    let _root_lock = store.lock(true, effects)?;
    let snapshot = match store.project()? {
        Some(snapshot) => {
            if snapshot.value.title != desired.title
                || snapshot.value.purpose != desired.purpose
                || snapshot.value.remote != desired.remote
            {
                return Err(Error::new(
                    "conflict",
                    "Project metadata differs; use edit_project rather than replaying registration.",
                ));
            }
            snapshot
        }
        None => {
            initialize(&store, desired.clone(), effects)?;
            store.project()?.ok_or_else(|| {
                Error::new("partial_init", "Manifest publication is not observable.")
            })?
        }
    };
    bootstrap(
        &store,
        "README.md",
        &readme(&args.project, &snapshot.value),
        effects,
    )?;
    bootstrap(&store, ".gitignore", IGNORE, effects)?;
    bootstrap_git(&store, docs_remote.as_deref(), effects)?;
    registry
        .aliases
        .insert(args.project.clone(), store.root.clone());
    let bytes = toml::to_string(&registry)
        .map_err(|_| Error::new("config", "Cannot encode project registry."))?
        .into_bytes();
    if bytes.len() > 64 * 1024 {
        return Err(Error::new(
            "capacity",
            "Registry would exceed 64 KiB; binding was not published.",
        ));
    }
    registry_store.publish("projects.toml", &bytes, observed.as_deref(), effects)?;
    render_registration(
        &args.project,
        &snapshot.value,
        snapshot.version,
        true,
        effects,
        templates,
    )
}

/// Render a confirmed registration once; template failure preserves alias/version and effect facts.
fn render_registration(
    alias: &str,
    project: &Project,
    version: String,
    changed: bool,
    effects: &[String],
    templates: &Templates,
) -> Result<String> {
    let view = Registered {
        project: alias,
        name: store::safe(&project.title, 256),
        version,
        changed,
        effects,
    };
    Ok(templates.render("project_registered", &view).unwrap_or_else(|_| format!("{} {}. Version: {}. Presentation degraded.\n{}\nInspect get_project_list before another registration.\n", if changed {"REGISTERED"} else {"UNCHANGED"}, store::safe(alias,128), view.version, effects.join("\n"))))
}

/// Return manifest-derived names/descriptions by alias, with unavailable entries retained.
/// Read-only and path-free; bounded pagination binds both registry bytes and each displayed manifest.
pub(super) fn list(
    config: &Config,
    args: ProjectListArgs,
    templates: &Templates,
) -> Result<String> {
    let (registry, bytes) = config.aliases()?;
    let mut digest = Sha256::new();
    digest.update(bytes.as_deref().unwrap_or(b"missing"));
    let mut value = page("Projects".into(), String::new());
    value
        .lines
        .push("Use project=\"<alias>\" with get_context, project_status and search.".into());
    for (alias, root) in registry.aliases {
        let record = (|| {
            let store = Store::from_root(&root)?;
            let _lock = store.lock(false, &mut Vec::new())?;
            store
                .project()?
                .ok_or_else(|| Error::new("not_initialized", "Project is not initialized."))
        })();
        match record {
            Ok(snapshot) => {
                digest.update(snapshot.version);
                if snapshot.value.purpose.len() > 320 {
                    value.detail_coverage = "PARTIAL".into();
                }
                value.rows.push(format!(
                    "{} — {}\n{}",
                    store::safe(&alias, 128),
                    store::safe(&snapshot.value.title, 256),
                    store::safe(&snapshot.value.purpose, 320)
                ));
            }
            Err(e) => {
                digest.update(e.code.as_bytes());
                value.coverage = "PARTIAL".into();
                value.rows.push(format!(
                    "{} — unavailable ({})",
                    store::safe(&alias, 128),
                    e.code
                ));
            }
        }
    }
    if value.rows.is_empty() {
        value
            .lines
            .push("No projects registered. Use register_project to create documentation.".into());
    }
    value.version = format!("{:x}", digest.finalize());
    value.snapshot_version = value.version.clone();
    continuation(
        args.start,
        args.limit,
        args.version.as_deref(),
        &value.version,
    )?;
    render_page(value, args.start, args.limit, true, templates)
}
