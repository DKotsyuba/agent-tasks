//! Small authoritative registry: one planning operation, one observer and product identity.
use crate::observer::{self, Config, PlanArgs, StatusArgs};
use mcp_presentation::Renderer;
use rmcp::model::{CallToolResult, ContentBlock};
use serde_json::{Value, json};

/// Export the closed runtime catalog without consulting settings or project registration.
pub fn definitions() -> Vec<Value> {
    vec![
        definition(
            "get_status",
            "Read product identity and declared host qualification. This is not a progress query.",
            json!({"type":"object","properties":{},"additionalProperties":false}),
            true,
        ),
        definition(
            "plan",
            "Read or save the owner-agreed goal and flat outcomes. Save only during owner-authorized planning; expected_revision is required after first save. Original protected MCP metadata automatically records the creating agent's Bridge contact address; ordinary revisions preserve it, new_plan captures a new creator and baseline. No registration, developer reports or manual completion.",
            serde_json::to_value(schemars::schema_for!(PlanArgs)).unwrap_or(Value::Null),
            false,
        ),
        definition(
            "status",
            "On owner request, estimate progress from Git and Agent Run. Compact localized bullet report (Russian by default, configurable in [summary]) with program-computed progress and unique observed commit count; details only on request. Service messages are English. With Bridge configured, one call delivers the final report as a message to the original host-metadata destination; PENDING requires no follow-up. Otherwise, after the completion notice read the exact analysis_id once. Never poll or launch replacement work. Explicit handle reads never start inference. API caller_context is a return destination only, never the persistent server environment.",
            serde_json::to_value(schemars::schema_for!(StatusArgs)).unwrap_or(Value::Null),
            false,
        ),
    ]
}

/// Declare the actual effects; status stores snapshots and may submit one analysis job.
fn definition(name: &str, description: &str, input: Value, read: bool) -> Value {
    json!({"name":name,"description":description,"inputSchema":input,"annotations":{"readOnlyHint":read,"destructiveHint":false,"idempotentHint":read,"openWorldHint":!read}})
}

/// No generated or partially implemented business tool is advertised.
pub fn incomplete() -> Vec<&'static str> {
    Vec::new()
}

/// Dispatch one synchronous bounded business operation; the transport runs it off its executor.
pub fn call(
    name: &str,
    args: Value,
    identity: &Renderer,
    config: &Config,
    origin: Option<&observer::Caller>,
) -> Option<CallToolResult> {
    let result = match name {
        "get_status" => {
            if args.as_object().is_some_and(|o| o.is_empty()) {
                let reply = identity.identity(
                    "agent-tasks",
                    env!("CARGO_PKG_VERSION"),
                    crate::qualification(),
                );
                return Some(CallToolResult::success(vec![ContentBlock::text(
                    reply.text().to_owned(),
                )]));
            }
            Err(observer::error::Error::new(
                "invalid_arguments",
                "get_status accepts an empty object.",
            ))
        }
        "plan" => serde_json::from_value(args)
            .map_err(observer::error::Error::from)
            .and_then(|a| observer::plan(config, a, origin)),
        "status" => serde_json::from_value(args)
            .map_err(observer::error::Error::from)
            .and_then(|a| observer::status(config, a)),
        _ => return None,
    };
    Some(match result {
        Ok(text) => CallToolResult::success(vec![ContentBlock::text(text)]),
        Err(error) => {
            let mut reply = CallToolResult::success(vec![ContentBlock::text(format!(
                "ERROR {}: {}\n",
                error.code, error.message
            ))]);
            reply.is_error = Some(true);
            reply
        }
    })
}
