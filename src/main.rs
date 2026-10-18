//! Rust MCP application; protocol, presentation and deployment have separate boundaries.
mod observer;
mod response;
mod tools;
use clap::{Parser, Subcommand};
use mcp_presentation::Renderer;
use rmcp::{
    ErrorData as McpError, RoleServer, ServerHandler, ServiceExt, model::*, service::RequestContext,
};
use std::{path::PathBuf, process::ExitCode, sync::Arc};

/// Private catalog cache lifetime in milliseconds for modern MCP requests.
const TOOLS_LIST_TTL_MS: u64 = 60_000;
/// Fixed admission bound for synchronous Git/socket work; identity remains responsive.
const MAX_BLOCKING_REQUESTS: usize = 4;

#[derive(Parser)]
#[command(version, about = concat!(env!("CARGO_PKG_NAME"), " MCP server"))]
/// CLI dispatch; config selection is lazy and never blocks discovery.
struct Cli {
    /// Absolute config location; precedence over AGENT_TASKS_CONFIG and HOME default.
    #[arg(long, global = true)]
    config: Option<PathBuf>,
    #[command(subcommand)]
    command: Commands,
}
#[derive(Subcommand)]
enum Commands {
    /// Internal one-shot continuation; stdin is the parent's transferred dispatch lease.
    #[command(hide = true)]
    DeliverStatus {
        /// Original observer storage root, not a newly resolved configuration.
        #[arg(long)]
        home: PathBuf,
        /// Original common-Git-directory digest.
        #[arg(long)]
        key: String,
    },
    /// Run the protocol-only stdio endpoint.
    Mcp,
    /// Check local resources, without credentials, network or mutation.
    Doctor {
        #[arg(long)]
        json: bool,
    },
    /// Export the actual registry or inspect unimplemented tool stubs.
    Contract {
        #[command(subcommand)]
        command: ContractCommand,
    },
    /// Install a verified single-binary bundle. Does not restart services.
    SelfInstall {
        #[arg(long)]
        bundle: PathBuf,
        #[arg(long)]
        home: PathBuf,
        #[arg(long)]
        bin_dir: PathBuf,
    },
    /// Select a retained compatible installation.
    Releases {
        #[command(subcommand)]
        command: ReleaseCommand,
    },
}
#[derive(Subcommand)]
enum ContractCommand {
    Export,
    Readiness,
}
#[derive(Subcommand)]
enum ReleaseCommand {
    Use {
        version: String,
        #[arg(long)]
        home: PathBuf,
        #[arg(long)]
        bin_dir: PathBuf,
    },
}
#[derive(Clone)]
/// Configuration-independent MCP catalog with a lazily resolved file store.
struct Handler {
    /// Captured settings location, resolved only on an owner business request.
    config: observer::Config,
    /// Immutable family identity presenter.
    identity: Arc<Renderer>,
    /// Static serde-derived discovery, independent of filesystem configuration.
    catalog: Vec<Tool>,
    /// Bounds queued/active blocking work, retained until its actual closure exits.
    admission: Arc<tokio::sync::Semaphore>,
}
impl Handler {
    /// Parse trusted layouts/catalog, capturing config location without reading it.
    fn new(config: observer::Config) -> Result<Self, &'static str> {
        let identity = Renderer::new().map_err(|_| "presentation_setup_failed")?;
        let catalog = serde_json::from_value(serde_json::Value::Array(tools::definitions()))
            .map_err(|_| "catalog_invalid")?;
        Ok(Self {
            config,
            identity: Arc::new(identity),
            catalog,
            admission: Arc::new(tokio::sync::Semaphore::new(MAX_BLOCKING_REQUESTS)),
        })
    }
}
impl ServerHandler for Handler {
    /// Advertise the flat planning/status contract without accessing project files.
    fn get_info(&self) -> ServerConfig {
        ServerConfig::new(ServerCapabilities::builder().enable_tools().build())
            .with_server_info(Implementation::new(env!("CARGO_PKG_NAME"), env!("CARGO_PKG_VERSION")))
            .with_instructions("Use plan to save or read an owner-agreed flat plan; original host metadata captures its creator's contact address automatically. Use status only on owner request. Developers never maintain progress records. With Bridge configured, one status call delivers its final compact report as a message; no follow-up retrieval is required. Without Bridge, after the completion notice read the exact returned analysis_id once. Never poll or resume analyzers. No legacy project registration or task hierarchy exists.")
    }
    /// Return the static catalog; modern requests cache it privately for 60 seconds.
    /// Pagination is unused; legacy sessions retain their original wire fields.
    async fn list_tools(
        &self,
        _: Option<PaginatedRequestParams>,
        context: RequestContext<RoleServer>,
    ) -> Result<ListToolsResult, McpError> {
        let mut result = ListToolsResult::with_all_items(self.catalog.clone());
        if context
            .protocol_version()
            .is_some_and(|version| version.as_str() >= ProtocolVersion::V_2026_07_28.as_str())
        {
            result = result
                .with_ttl_ms(TOOLS_LIST_TTL_MS)
                .with_cache_scope(CacheScope::Private);
        }
        Ok(result)
    }
    /// Route known calls to bounded text/domain errors; unknown tools retain METHOD_NOT_FOUND.
    /// Business calls may read/write the configured root; identity remains configuration-independent.
    async fn call_tool(
        &self,
        request: CallToolRequestParams,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResponse, McpError> {
        // rmcp moves protocol _meta out of params into this per-request context.
        let metadata = serde_json::to_value(&context.meta).ok();
        let mut args = serde_json::Value::Object(request.arguments.unwrap_or_default());
        let name = request.name.to_string();
        let origin = metadata
            .as_ref()
            .and_then(|m| m.get("threadId"))
            .and_then(serde_json::Value::as_str)
            .map(|thread| observer::Caller {
                transport: "codex_queue".into(),
                external_session_id: thread.to_owned(),
                external_turn_id: None,
            });
        if name == "status"
            && let Some(thread) = metadata
                .as_ref()
                .and_then(|m| m.get("threadId"))
                .and_then(serde_json::Value::as_str)
        {
            // Original host metadata supplies a return destination only, never caller authority.
            args["caller_context"] =
                serde_json::json!({"transport":"codex_queue","external_session_id":thread});
        }
        let identity = self.identity.clone();
        let config = self.config.clone();
        if name == "get_status" {
            let reply = tools::call(&name, args, &identity, &config, origin.as_ref())
                .ok_or_else(|| McpError::new(ErrorCode::METHOD_NOT_FOUND, "Unknown tool", None))?;
            return Ok(reply.into());
        }
        let permit = match self.admission.clone().try_acquire_owned() {
            Ok(permit) => permit,
            Err(_) => {
                let mut reply = CallToolResult::success(vec![ContentBlock::text(
                    "ERROR busy: Observer request capacity is occupied; no work was submitted.",
                )]);
                reply.is_error = Some(true);
                return Ok(reply.into());
            }
        };
        let reply = tokio::task::spawn_blocking(move || {
            let _permit = permit;
            tools::call(&name, args, &identity, &config, origin.as_ref())
        })
        .await
        .map_err(|_| McpError::new(ErrorCode::INTERNAL_ERROR, "Observer worker failed", None))?
        .ok_or_else(|| McpError::new(ErrorCode::METHOD_NOT_FOUND, "Unknown tool", None))?;
        Ok(reply.into())
    }
}
#[allow(clippy::print_stdout, reason = "Explicit CLI branch, never MCP output")]
fn print_json(value: impl serde::Serialize) -> ExitCode {
    match serde_json::to_string_pretty(&value) {
        Ok(text) => {
            println!("{text}");
            ExitCode::SUCCESS
        }
        Err(_) => {
            eprintln!("CLI serialization failed");
            ExitCode::FAILURE
        }
    }
}
/// Release qualification recorded in the embedded family manifest, so
/// `doctor` and `get_status` report the same value the release tooling reads.
/// An unreadable manifest fails closed to `not_verified`.
fn qualification() -> &'static str {
    static QUALIFICATION: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    QUALIFICATION.get_or_init(|| {
        toml::from_str::<toml::Value>(include_str!("../family.toml"))
            .ok()
            .and_then(|family| {
                family
                    .get("qualification")
                    .and_then(toml::Value::as_str)
                    .map(str::to_owned)
            })
            .unwrap_or_else(|| "not_verified".to_owned())
    })
}

/// Select a CLI operation and lazy config location. MCP stdout is protocol-only;
/// discovery/doctor do not initialize roots or require installed credentials/config.
#[tokio::main]
async fn main() -> ExitCode {
    let cli = Cli::parse();
    let config = observer::Config::new(cli.config);
    match cli.command {
        Commands::DeliverStatus { home, key } => match observer::bridge::worker(&home, &key) {
            Ok(()) => ExitCode::SUCCESS,
            Err(_) => ExitCode::FAILURE,
        },
        Commands::SelfInstall {
            bundle,
            home,
            bin_dir,
        } => match family_delivery::install(&bundle, &home, &bin_dir) {
            Ok(m) => print_json(m),
            Err(e) => {
                eprintln!("install: {e}");
                ExitCode::FAILURE
            }
        },
        Commands::Releases {
            command:
                ReleaseCommand::Use {
                    version,
                    home,
                    bin_dir,
                },
        } => match family_delivery::use_version(&home, &bin_dir, &version) {
            Ok(m) => print_json(m),
            Err(e) => {
                eprintln!("activate: {e}");
                ExitCode::FAILURE
            }
        },
        Commands::Doctor { json: _ } => {
            let ready = Handler::new(config.clone()).is_ok();
            let output = print_json(
                serde_json::json!({"product":env!("CARGO_PKG_NAME"),"version":env!("CARGO_PKG_VERSION"),
                "local_ready":ready,"release_qualification":qualification(),"incomplete_tools":tools::incomplete()}),
            );
            if ready { output } else { ExitCode::from(2) }
        }
        Commands::Contract {
            command: ContractCommand::Export,
        } => match Handler::new(config.clone()) {
            Ok(h) => print_json(h.catalog),
            Err(e) => {
                eprintln!("contract: {e}");
                ExitCode::FAILURE
            }
        },
        Commands::Contract {
            command: ContractCommand::Readiness,
        } => print_json(tools::incomplete()),
        Commands::Mcp => {
            let h = match Handler::new(config.clone()) {
                Ok(h) => h,
                Err(e) => {
                    eprintln!("startup: {e}");
                    return ExitCode::FAILURE;
                }
            };
            match h.serve(rmcp::transport::stdio()).await {
                Ok(service) => match service.waiting().await {
                    Ok(_) => ExitCode::SUCCESS,
                    Err(_) => ExitCode::FAILURE,
                },
                Err(_) => {
                    eprintln!("MCP transport failed");
                    ExitCode::FAILURE
                }
            }
        }
    }
}
