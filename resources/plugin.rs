//! Reference plugin: extends yeti-mcp's tool registry via the
//! YTC-367 cross-dylib hook-registration bridge (ADR-009).
//!
//! Two services are registered:
//!
//!  1. `Service<ListToolsRequest>` — runs on every `tools/list` MCP
//!     request. Receives the auto-generated platform inventory as the
//!     accumulator seed, appends one extra `yeti_hello` tool, and
//!     returns the merged list.
//!
//!  2. `Service<CallToolRequest>` — runs on every `tools/call` MCP
//!     request *before* yeti-mcp's built-in `{table}_{op}` CRUD
//!     dispatcher. Returns `Some(value)` for tools this plugin owns
//!     (`yeti_hello`), `None` otherwise — yeti-mcp then falls through
//!     to the auto-inventory dispatch.
//!
//! Registration is automatic. The Yeti app loader calls
//! [`YetiMcpPlugin::register_hooks`] (YTC-367 / ADR-009) between
//! `resources()` and `on_ready()`, which:
//!
//!   1. Builds the `list_tools` and `call_tool` services from
//!      `service_fn` closures.
//!   2. Calls `yeti_sdk::service_bridge::register_hook` with the
//!      versioned chain names (`yeti.mcp.list_tools.v1` /
//!      `yeti.mcp.call_tool.v1`) to bridge them across the cdylib
//!      boundary into yeti-mcp's hook chain.
//!
//! Extending this stub into a real knowledge-base / vector-search /
//! domain-specific tool set is straightforward — add additional
//! `ToolDefinition` entries in `build_list_tools_service` and match
//! their names in `build_call_tool_service`. See the README for the
//! recommended layout.

use serde_json::{Value, json};
use yeti_sdk::error::{Result, YetiError};
use yeti_sdk::plugins::Plugin;
use yeti_sdk::prelude::extension::mcp::{
    CALL_TOOL_HOOK_CHAIN_NAME, CallToolRequest, CallToolResponse, CallToolService,
    LIST_TOOLS_HOOK_CHAIN_NAME, ListToolsRequest, ListToolsResponse, ListToolsService,
    ToolAnnotations, ToolDefinition,
};
use yeti_sdk::prelude::{BoxCloneSyncService, service_fn};

/// Tool name this plugin owns end-to-end. Extracted as a const so the
/// `list_tools` accumulator and the `call_tool` dispatcher can't drift.
const HELLO_TOOL: &str = "yeti_hello";

/// Default-constructible Plugin shell. The compiler scaffolder
/// detects `impl Plugin for YetiMcpPlugin` in the generated `lib.rs`
/// and emits `__yeti_export_plugin!(YetiMcpPlugin)` automatically — no
/// manual macro invocation required.
#[derive(Default)]
pub struct YetiMcpPlugin;

impl Plugin for YetiMcpPlugin {
    fn id(&self) -> &'static str {
        "plugin-mcp-yeti"
    }

    fn name(&self) -> &'static str {
        "Yeti MCP tool extender (reference)"
    }

    fn config_toml(&self) -> Option<&'static str> {
        Some(include_str!("../Cargo.toml"))
    }

    fn is_plugin(&self) -> bool {
        true
    }

    /// YTC-367 / ADR-009 — install the list_tools and call_tool
    /// services into yeti-mcp's hook chain across the cdylib boundary.
    ///
    /// Logs a single info line on success so end-to-end verification
    /// has a deterministic signal to grep for in `yeti.log`. Errors
    /// (FFI registration) log at warn so a registration failure
    /// doesn't take down the host.
    fn register_hooks(&self) -> Result<()> {
        log::info!("[plugin-mcp-yeti] register_hooks called; building services");

        let list_svc = build_list_tools_service();
        if let Err(e) = yeti_sdk::service_bridge::register_hook::<
            ListToolsService,
            ListToolsRequest,
            ListToolsResponse,
            YetiError,
        >(LIST_TOOLS_HOOK_CHAIN_NAME, list_svc)
        {
            log::warn!(
                "[plugin-mcp-yeti] register_hooks: register_list_tools_hook failed: {}",
                e
            );
        }

        let call_svc = build_call_tool_service();
        if let Err(e) = yeti_sdk::service_bridge::register_hook::<
            CallToolService,
            CallToolRequest,
            CallToolResponse,
            YetiError,
        >(CALL_TOOL_HOOK_CHAIN_NAME, call_svc)
        {
            log::warn!(
                "[plugin-mcp-yeti] register_hooks: register_call_tool_hook failed: {}",
                e
            );
        }

        log::info!(
            "[plugin-mcp-yeti] Registered list_tools + call_tool hooks via yeti.mcp.list_tools.v1 / yeti.mcp.call_tool.v1"
        );
        Ok(())
    }
}

/// Build the `tools/list` accumulator service. Receives the host's
/// auto-inventory in `req.tools`, appends the `yeti_hello` tool, and
/// returns the merged list.
fn build_list_tools_service() -> ListToolsService {
    BoxCloneSyncService::new(service_fn(|mut req: ListToolsRequest| async move {
        req.tools.push(ToolDefinition {
            name: HELLO_TOOL.to_owned(),
            description: "Returns a greeting from the plugin-mcp-yeti template."
                .to_owned(),
            input_schema: json!({
                "type": "object",
                "properties": {},
                "additionalProperties": false,
            }),
            annotations: Some(ToolAnnotations {
                read_only_hint: Some(true),
                destructive_hint: Some(false),
                idempotent_hint: Some(true),
            }),
        });
        Ok::<ListToolsResponse, YetiError>(req.tools)
    }))
}

/// Build the `tools/call` dispatcher service. Handles `yeti_hello`
/// explicitly; returns `None` for anything else so yeti-mcp falls
/// through to its built-in auto-inventory dispatcher.
fn build_call_tool_service() -> CallToolService {
    BoxCloneSyncService::new(service_fn(|req: CallToolRequest| async move {
        if req.tool_name == HELLO_TOOL {
            return Ok::<CallToolResponse, YetiError>(CallToolResponse::some(&json!({
                "greeting": "Hello from plugin-mcp-yeti template! \
                    This response was generated by the plugin's call_tool hook, \
                    demonstrating the McpHooks bridge.",
                "tool": HELLO_TOOL,
                "arguments": req.arguments,
            })));
        }
        Ok(CallToolResponse::none())
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use yeti_sdk::prelude::{Service, ServiceExt};

    #[tokio::test]
    async fn list_tools_appends_hello() {
        let mut svc = build_list_tools_service();
        let req = ListToolsRequest {
            deployment_hash: "local".to_owned(),
            tools: vec![],
        };
        let tools = svc.ready().await.unwrap().call(req).await.unwrap();
        assert_eq!(tools.len(), 1);
        assert_eq!(tools[0].name, HELLO_TOOL);
        assert!(tools[0]
            .annotations
            .as_ref()
            .and_then(|a| a.read_only_hint)
            .unwrap_or(false));
    }

    #[tokio::test]
    async fn call_tool_handles_yeti_hello() {
        let mut svc = build_call_tool_service();
        let req = CallToolRequest {
            deployment_hash: "local".to_owned(),
            tool_name: HELLO_TOOL.to_owned(),
            arguments: json!({}),
        };
        let resp = svc.ready().await.unwrap().call(req).await.unwrap();
        assert!(resp.is_some(), "yeti_hello should be handled");
        let value = resp.value().unwrap();
        assert!(value.get("greeting").is_some());
        assert_eq!(value.get("tool").and_then(Value::as_str), Some(HELLO_TOOL));
    }

    #[tokio::test]
    async fn call_tool_falls_through_for_unknown() {
        let mut svc = build_call_tool_service();
        let req = CallToolRequest {
            deployment_hash: "local".to_owned(),
            tool_name: "widgets_list".to_owned(),
            arguments: json!({"limit": 10}),
        };
        let resp = svc.ready().await.unwrap().call(req).await.unwrap();
        assert!(resp.is_none(), "unknown tool should fall through");
    }
}
