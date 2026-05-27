# plugin-mcp-yeti

Reference Yeti MCP tool-source plugin. Compiles to a `wasm32-wasip2`
component that ships in `{rootDirectory}/applications/plugin-mcp-yeti/`,
loads alongside any other yeti app, and contributes a typed tool
catalog to the deployment's MCP endpoint.

Built against the `yeti:mcp/mcp-tool-source` WIT interface
(ADR-012 §yeti:mcp). The host's `yeti:mcp/mcp-client` impl is a pure
router that fans `list-tools` / `call-tool` / `list-resources` etc.
out to every registered source by name — this plugin registers under
`yeti-builtin` and currently owns the `yeti_hello` tool end-to-end.

Use this as a copy-paste starting point for your own MCP tool sources
— domain knowledge bases, third-party service connectors, organization-
specific workflows, anything an MCP client should be able to discover
and invoke without you forking the platform.

## What it shows

The plugin implements `yeti_sdk::mcp::McpToolSource` on a unit struct.
The yeti-compiler auto-detects the impl
(`detect_provider_kind` → `discover_mcp_tool_source_in_file`) and:

1. Switches the component's `wit_bindgen!` world from `customer-app`
   to `tool-source-export`.
2. Emits a `Guest` impl for `yeti:mcp/mcp-tool-source` that delegates
   verb-for-verb into the customer struct (`list-tools`, `call-tool`,
   `list-resources`, `read-resource`, `list-prompts`, `get-prompt`,
   `describe`).

No `register_hook` calls, no FFI bridges — typed exports across the
WIT component boundary.

## Configuration

Lives in `Cargo.toml`:

```toml
[package.metadata.app]
plugin = true                           # required — loads before user apps
customer_id = "yeti"                    # arbitrary org tag
resources = { path = "resources/*.rs" } # required for wasm component compile
```

`plugin = true` flags this as a tool-source-style plugin-app: it loads
before regular user apps so its catalog is registered before the first
MCP request hits the router. `resources = ...` tells the compiler
scaffolder to scan the resource sources for `impl McpToolSource for X`
and switch the WIT world accordingly.

## Wiring it up

The integration surface is one trait impl:

```rust
use yeti_sdk::mcp::{
    McpDescriptor, McpToolSource, ToolInfo, ToolResult,
};

#[derive(Default)]
pub struct Source;

impl McpToolSource for Source {
    fn list_tools(&self) -> Result<Vec<ToolInfo>, String> {
        Ok(vec![ToolInfo {
            name: "my_tool".to_owned(),
            description: "What it does".to_owned(),
            input_schema: br#"{"type":"object"}"#.to_vec(),
            source: String::new(), // filled in by host
        }])
    }

    fn call_tool(&self, name: &str, args: &[u8]) -> Result<ToolResult, String> {
        match name {
            "my_tool" => Ok(ToolResult {
                content: b"hello".to_vec(),
                is_error: false,
            }),
            other => Err(format!("tool `{other}` not found")),
        }
    }

    fn describe(&self) -> McpDescriptor {
        McpDescriptor {
            name: "my-source".to_owned(),
            has_resources: false,
            has_prompts: false,
        }
    }
}
```

That's the entire integration. `list_resources` / `read_resource` /
`list_prompts` / `get_prompt` have empty/NotFound defaults — only
override when you actually expose those surfaces.

## Verifying it loaded

After `yeti start`, exercise the MCP endpoint:

```bash
# List tools — yeti_hello should appear alongside the platform tools
curl -sk -X POST https://localhost:9996/yeti-mcp/mcp \
  -H 'Content-Type: application/json' \
  -d '{"jsonrpc":"2.0","method":"tools/list","id":1}' | jq '.result.tools[].name'

# Call yeti_hello — returns the greeting
curl -sk -X POST https://localhost:9996/yeti-mcp/mcp \
  -H 'Content-Type: application/json' \
  -d '{"jsonrpc":"2.0","method":"tools/call","id":2,
       "params":{"name":"yeti_hello","arguments":{}}}'
```

Adjust the port if your `yeti-config.yaml` `port:` is something other
than 9996.

## Extending it

### Add more tools

Extend the `list_tools` vec and add a matching arm in `call_tool`:

```rust
fn list_tools(&self) -> Result<Vec<ToolInfo>, String> {
    Ok(vec![
        ToolInfo { name: "tool_a".to_owned(), /* ... */ },
        ToolInfo { name: "tool_b".to_owned(), /* ... */ },
    ])
}

fn call_tool(&self, name: &str, args: &[u8]) -> Result<ToolResult, String> {
    match name {
        "tool_a" => handle_a(args),
        "tool_b" => handle_b(args),
        other => Err(format!("tool `{other}` not found")),
    }
}
```

### Expose resources / prompts

Override the default-empty `list_resources` / `read_resource` (and/or
`list_prompts` / `get_prompt`) and flip the corresponding flag in
`describe()`:

```rust
fn describe(&self) -> McpDescriptor {
    McpDescriptor {
        name: "my-source".to_owned(),
        has_resources: true,  // ← flip
        has_prompts: false,
    }
}
```

### Ship a vector-search knowledge base

The most common extension pattern is "vectorize this corpus, expose a
`docs_search` tool over it." Steps:

1. Write a `build.rs` that walks your corpus, chunks it, runs each
   chunk through an embedding model, and serializes the resulting
   index into a `knowledge.bin` file embedded via `include_bytes!`.
2. In `call_tool`, match `"docs_search"`, embed the query argument,
   run cosine similarity, and return the top-K chunks.

This is exactly what yeti-mcp's built-in `docs_search` does for the
SDK + prelude macros + AGENTS.md + CLI reference — your plugin
extends the same pattern to your own corpus.

## See also

- [ADR-012 §yeti:mcp](https://github.com/yetirocks/yeti-core/blob/main/docs/adr/012-wit-component-model.md)
  — the `mcp-client` / `mcp-tool-source` interface split
- [yeti:mcp WIT](https://github.com/yetirocks/yeti-core/blob/main/wit/deps/yeti-mcp.wit)
  — the wire spec
- [`plugin-auth-okta`](https://github.com/YetiRocks/plugin-auth-okta) —
  the same pattern applied to auth (`OidcProvider` → `auth-provider-export`)
- [Yeti documentation](https://github.com/YetiRocks/yeti-documentation)
  — full plugin author guide
