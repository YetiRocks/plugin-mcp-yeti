# plugin-mcp-yeti

Reference Yeti MCP plugin demonstrating the two cross-dylib hook surfaces
yeti-mcp exposes for runtime extension. Compiles to a dylib that ships in
`{rootDirectory}/applications/plugin-mcp-yeti/`; loads alongside any other
yeti plugin-app; extends the live `/yeti-mcp/agent` MCP endpoint with
custom tools without recompiling the yeti host binary.

Built against the YTC-367 cross-dylib hook-registration bridge
([ADR-009](https://github.com/yetirocks/yeti-core/blob/main/docs/adr/009-cross-dylib-hook-registration.md)).
Use this as a copy-paste starting point for your own MCP plugins —
custom-domain tool servers, organization knowledge bases, third-party
service connectors, anything an MCP client should be able to discover and
invoke without you forking the platform.

## What it shows

The plugin registers two `tower::Service` instances into yeti-mcp's hook
chains:

- **`Service<ListToolsRequest>`** — runs on every `tools/list` MCP
  request. Receives yeti-mcp's auto-inventoried platform tools as the
  accumulator seed, appends one extra `yeti_hello` tool, and returns the
  merged list. Real plugins push N domain-specific tools instead of one
  greeting.
- **`Service<CallToolRequest>`** — runs on every `tools/call` MCP request
  *before* yeti-mcp's built-in `{table}_{op}` CRUD dispatcher. Returns
  `Some(value)` for tools this plugin owns, `None` otherwise — yeti-mcp
  then falls through to the auto-inventory dispatch. Multi-plugin
  deployments iterate registered services in registration order; the
  first to return `Some` wins.

Both services round-trip through `BoxCloneSyncService<Bytes, Bytes,
BridgeError>` on the FFI boundary, with typed `Service<R>` wrappers on
both ends bincode-serializing the request/response. The SDK helper
`yeti_sdk::service_bridge::register_hook` does all the type erasure for
you — plugin authors only ever see the typed `Service`.

## Configuration

Lives in the plugin's `Cargo.toml`:

```toml
[package.metadata.app]
plugin = true                          # required — CI lint enforces
customer_id = "yeti"                   # arbitrary org tag
resources = { path = "resources/*.rs" } # required for dylib compile
```

`plugin = true` flags this as a static-service-style plugin-app: it loads
before regular user apps so its hook installations are in place before
the first request hits the dispatcher. `resources = ...` is what tells
the compiler scaffolder to scan for `impl Plugin for ...` declarations
and emit the `__yeti_export_plugin!` macro — without it the dylib won't
build with the right entry points.

The scaffolder reads `[package.metadata.mcp.plugins.{flavor}]` blocks
for plugin-specific configuration. This stub plugin doesn't take any
config (it only knows about one hardcoded tool), but a real plugin would
declare its config schema in this block and read it via the SDK's
app-metadata accessor.

## Wiring it up

The yeti app loader calls `Plugin::register_hooks(&self)` between
`resources()` and `on_ready()` (YTC-367 / ADR-009). Inside, you build
your typed `Service`s and call `yeti_sdk::service_bridge::register_hook`
with the versioned chain name:

```rust
use yeti_sdk::plugins::Plugin;
use yeti_sdk::prelude::extension::mcp::{
    CALL_TOOL_HOOK_CHAIN_NAME, CallToolRequest, CallToolResponse, CallToolService,
    LIST_TOOLS_HOOK_CHAIN_NAME, ListToolsRequest, ListToolsResponse, ListToolsService,
};

impl Plugin for YetiMcpPlugin {
    fn id(&self) -> &'static str { "plugin-mcp-yeti" }
    fn is_plugin(&self) -> bool { true }

    fn register_hooks(&self) -> yeti_sdk::error::Result<()> {
        yeti_sdk::service_bridge::register_hook::<
            ListToolsService, ListToolsRequest, ListToolsResponse, YetiError,
        >(LIST_TOOLS_HOOK_CHAIN_NAME, build_list_tools_service())?;

        yeti_sdk::service_bridge::register_hook::<
            CallToolService, CallToolRequest, CallToolResponse, YetiError,
        >(CALL_TOOL_HOOK_CHAIN_NAME, build_call_tool_service())?;
        Ok(())
    }
}
```

That's the entire integration surface. The hook chain names are versioned
(`v1`) — when yeti-mcp evolves the wire shape it ships a `v2` alongside,
hosts iterate both chains, and v1-shipped plugins keep working until they
migrate.

## Verifying it loaded

After `yeti start`, look in `{rootDirectory}/logs/yeti.log` for:

```
[plugin-mcp-yeti] register_hooks called; building services
[plugin-mcp-yeti] Registered list_tools + call_tool hooks via yeti.mcp.list_tools.v1 / yeti.mcp.call_tool.v1
✓ Started plugin-mcp-yeti elapsed=...
```

Then exercise the MCP endpoint:

```bash
# List tools — yeti_hello should appear alongside the platform tools
curl -sk -X POST https://localhost:9996/yeti-mcp/agent \
  -H 'Content-Type: application/json' \
  -d '{"jsonrpc":"2.0","method":"tools/list","id":1}' | jq '.result.tools[].name'

# Call yeti_hello — plugin returns the greeting
curl -sk -X POST https://localhost:9996/yeti-mcp/agent \
  -H 'Content-Type: application/json' \
  -d '{"jsonrpc":"2.0","method":"tools/call","id":2,
       "params":{"name":"yeti_hello","arguments":{}}}'
```

`yeti_hello` should appear in the list output and the call output should
return the plugin's greeting JSON. Adjust the port if your
`yeti-config.yaml` `port:` is something other than 9996.

## Extending it

Use this as a starting point for your own MCP plugin. The same shape
works for any tool surface — domain knowledge bases, third-party API
connectors, organization-specific workflows, etc.

### Add more tools

The accumulator pattern means you can register multiple
`ListToolsService`s (each adding its own tools) and yeti-mcp threads them
all. For tool implementations, extend the `match` block in
`build_call_tool_service`:

```rust
async move {
    match req.tool_name.as_str() {
        "my_tool_a" => Ok(CallToolResponse::some(json!({"result": "..."}))),
        "my_tool_b" => Ok(CallToolResponse::some(handle_b(req.arguments).await?)),
        _ => Ok(CallToolResponse::none()), // fall through to next plugin / auto-inventory
    }
}
```

### Replace yeti-mcp's defaults entirely

Clear the accumulator instead of appending:

```rust
service_fn(move |mut req: ListToolsRequest| async move {
    req.tools.clear();           // suppress auto-inventory
    req.tools.push(my_tool_a()); // emit only your tools
    req.tools.push(my_tool_b());
    Ok(req.tools)
})
```

Combine with a `call_tool` service that never returns `None` and you've
fully replaced the deployment's MCP server with your own. Useful for
locked-down deployments that want to expose only a curated tool surface.

### Ship a vector-search knowledge base

The most common extension pattern is "vectorize this corpus, expose a
`docs_search` tool over it." Steps:

1. Write a `build.rs` that walks your corpus (README, SDK docs, CLI
   reference, internal wiki, etc.), chunks it, runs each chunk through
   an embedding model, and serializes the resulting index into a
   `knowledge.bin` file embedded in your dylib via `include_bytes!`.
2. Implement a `docs_search` `Service<CallToolRequest>` that loads the
   index at first call, takes a `query` argument, embeds it, runs cosine
   similarity, and returns the top-K chunks.
3. Register it via `register_hook` against `CALL_TOOL_HOOK_CHAIN_NAME`.

This is exactly what yeti-mcp itself does for its built-in `docs_search`
tool (vectorized SDK + prelude macros + AGENTS.md + CLI reference) —
your plugin extends the same pattern to your own corpus.

## What it isn't

This is reference code, not a production-ready MCP server. A real
plugin would also:

- Read its configuration from `[package.metadata.mcp.plugins.{flavor}]`
  via the SDK's runtime app-metadata accessor (the accessor itself is a
  follow-up to YTC-367 — current stub returns hardcoded defaults).
- Carry MCP behavior hints (`readOnlyHint`, `destructiveHint`,
  `idempotentHint`) on every emitted `ToolDefinition`. The Anthropic
  Connector Directory rejects ~30% of submissions for missing these.
- Validate `req.arguments` against each tool's declared `input_schema`
  before invoking the handler.
- Surface metrics / audit events on every call so operators can see what
  agents are invoking and why.
- Cache vector-search results / embedding lookups to avoid recomputing
  on hot paths.

The McpHooks surface (`list_tools` accumulator + `call_tool` dispatcher)
covers the MVP. Future hook chains will land for `resources/list`,
`resources/read`, `prompts/list`, `prompts/get` — same pattern, same
wire shape, same `Plugin::register_hooks` lifecycle.

## See also

- [YTC-367 / ADR-009](https://github.com/yetirocks/yeti-core/blob/main/docs/adr/009-cross-dylib-hook-registration.md)
  — the cross-dylib hook-registration bridge that makes this plugin
  possible
- [`plugin-auth-okta`](https://github.com/YetiRocks/plugin-auth-okta) —
  the same pattern applied to auth (OAuth claim mapping + JWT mint
  extension)
- [Yeti documentation](https://github.com/YetiRocks/yeti-documentation)
  — full guide to writing Yeti plugins, including the broader plugin
  author contract from [ADR-008](https://github.com/yetirocks/yeti-core/blob/main/docs/adr/008-plugin-author-contract.md)
