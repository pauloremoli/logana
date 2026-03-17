# MCP Server

logana includes an embedded [Model Context Protocol (MCP)](https://modelcontextprotocol.io/) server. When enabled, it exposes marked lines and annotations as MCP resources and provides tools so AI assistants can interact with your log analysis session in real time.

## Starting the Server

### On launch

```sh
logana app.log --mcp           # default port 9876
logana app.log --mcp 8080      # custom port
```

### From inside the TUI

```
:enable-mcp                    # default port 9876
:enable-mcp --port 8080        # custom port
:disable-mcp                   # stop the server
```

The server listens at `http://localhost:<port>/mcp` using the Streamable HTTP transport.

## Default Port in Config

Set a persistent default port in `~/.config/logana/config.json`:

```json
{
  "mcp_port": 9876
}
```

When both the config and a `--port` flag are present, the config value takes precedence.

## Resources

| URI | Description |
|---|---|
| `logana://marks` | All marked lines — one entry per line formatted as `<line_number>: <text>` |
| `logana://annotations` | All annotations — each block shows the 1-based line numbers and the comment text |

Resources are updated every render frame so the MCP client always sees the current state of the active tab.

## Tools

| Tool | Parameters | Description |
|---|---|---|
| `toggle_mark` | `line_index` (1-based) | Mark or unmark a log line |
| `add_annotation` | `text`, `line_indices` (1-based list) | Attach a comment to one or more lines |
| `remove_annotation` | `index` (0-based) | Remove an annotation by its position in the list |

Tool calls are applied to the active tab and immediately reflected in the TUI.

## Connecting an AI Assistant

Point your MCP client at the server endpoint. For example, to use it with Claude Desktop, add an entry to your `claude_desktop_config.json`:

```json
{
  "mcpServers": {
    "logana": {
      "url": "http://localhost:9876/mcp"
    }
  }
}
```

Once connected, the assistant can read your marked lines and annotations and call tools to mark or annotate lines on your behalf.
