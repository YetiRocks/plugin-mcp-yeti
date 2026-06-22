//! Reference MCP tool source for Yeti.
//!
//! Implements [`yeti_sdk::mcp::McpProvider`] on a unit struct. The
//! yeti-compiler (`detect_provider_kind` →
//! `discover_mcp_tool_source_in_file`) sees `impl McpProvider for X`,
//! switches the component's `wit_bindgen!` world from `customer-app`
//! to `tool-source-export`, and auto-emits a `Guest` impl for
//! `yeti:mcp/mcp-tool-source` that delegates verb-for-verb into this
//! struct. No manual export macros, no hook bridge.
//!
//! Per ADR-012 §yeti:mcp, the host's `mcp-client.list-tools` /
//! `call-tool` etc. fan out across every registered source by name —
//! this source registers under the `name` returned from `describe()`.
//!
//! Only `yeti_hello` is in this source's catalog. Extend the
//! `list_tools` and `call_tool` match arms together to add more.

use yeti_sdk::mcp::{
    McpDescriptor, McpProvider, PromptInfo, PromptResult, ResourceContent, ResourceInfo,
    ToolInfo, ToolResult,
};

/// Source name registered with the host. Becomes the routing key for
/// `mcp-client.list-tools(source: Some("yeti-builtin"))`.
const SOURCE_NAME: &str = "yeti-builtin";

/// Tool name owned end-to-end by this source. Extracted as a const so
/// the catalog (`list_tools`) and dispatcher (`call_tool`) can't
/// drift.
const HELLO_TOOL: &str = "yeti_hello";

/// JSON Schema for `yeti_hello`: takes no arguments.
const HELLO_INPUT_SCHEMA: &[u8] = br#"{"type":"object","properties":{},"additionalProperties":false}"#;

/// Default-constructible source. The compiler detects
/// `impl McpProvider for YetiMcp` and scaffolds the WIT export
/// wiring; no `__yeti_export_*` macro invocation required.
///
/// Note: the struct name (`YetiMcp`) intentionally does **not** match
/// the PascalCase form of the filename (`source.rs` → `Source`) —
/// matching would trigger the resource auto-discovery's Phase-2
/// `pub struct` fallback and the compiler would try to register this
/// type as an HTTP resource on top of the provider export. The e2e
/// `plugin-mcp-noop` test follows the same convention
/// (`noop.rs` + `pub struct NoopMcp`).
#[derive(Default)]
pub struct YetiMcp;

impl McpProvider for YetiMcp {
    fn list_tools(&self) -> Result<Vec<ToolInfo>, String> {
        Ok(vec![ToolInfo {
            name: HELLO_TOOL.to_owned(),
            description: "Returns a greeting from the plugin-mcp-yeti reference source."
                .to_owned(),
            input_schema: HELLO_INPUT_SCHEMA.to_vec(),
            // Source field is filled in by the host from `describe()`;
            // leave empty here.
            source: String::new(),
        }])
    }

    fn call_tool(&self, name: &str, args: &[u8]) -> Result<ToolResult, String> {
        match name {
            HELLO_TOOL => {
                // Echo back the args verbatim alongside a fixed
                // greeting so callers can verify round-trip works.
                let args_str = std::str::from_utf8(args).unwrap_or("<non-utf8>");
                let payload = format!(
                    r#"{{"greeting":"Hello from plugin-mcp-yeti reference source.","tool":"{HELLO_TOOL}","arguments":{args_str}}}"#,
                    args_str = if args_str.is_empty() { "{}" } else { args_str },
                );
                Ok(ToolResult {
                    content: payload.into_bytes(),
                    is_error: false,
                })
            },
            other => Err(format!("tool `{other}` not found in `{SOURCE_NAME}`")),
        }
    }

    // No resources, no prompts — defaults return empty / NotFound.

    fn describe(&self) -> McpDescriptor {
        McpDescriptor {
            name: SOURCE_NAME.to_owned(),
            has_resources: false,
            has_prompts: false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lists_yeti_hello() {
        let src = YetiMcp;
        let tools = src.list_tools().unwrap();
        assert_eq!(tools.len(), 1);
        assert_eq!(tools[0].name, HELLO_TOOL);
        assert!(!tools[0].description.is_empty());
        assert!(!tools[0].input_schema.is_empty());
    }

    #[test]
    fn calls_yeti_hello() {
        let src = YetiMcp;
        let result = src.call_tool(HELLO_TOOL, b"{}").unwrap();
        assert!(!result.is_error);
        let body = String::from_utf8(result.content).unwrap();
        assert!(body.contains("Hello from plugin-mcp-yeti"));
        assert!(body.contains(HELLO_TOOL));
    }

    #[test]
    fn call_tool_unknown_errors() {
        let src = YetiMcp;
        let err = src.call_tool("widgets_list", b"{}").unwrap_err();
        assert!(err.contains("widgets_list"));
        assert!(err.contains(SOURCE_NAME));
    }

    #[test]
    fn describe_advertises_source_name() {
        let src = YetiMcp;
        let desc = src.describe();
        assert_eq!(desc.name, SOURCE_NAME);
        assert!(!desc.has_resources);
        assert!(!desc.has_prompts);
    }

    #[test]
    fn no_resources_by_default() {
        let src = YetiMcp;
        assert!(src.list_resources().unwrap().is_empty());
    }

    #[test]
    fn no_prompts_by_default() {
        let src = YetiMcp;
        assert!(src.list_prompts().unwrap().is_empty());
    }
}
