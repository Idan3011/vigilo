# Manual Setup

`vigilo setup` handles all of this automatically. Use this guide only if you prefer manual configuration.

## Claude Code

### 1. MCP server

Add to `~/.claude.json`:

```json
{
  "mcpServers": {
    "vigilo": {
      "command": "vigilo",
      "type": "stdio"
    }
  }
}
```

### 2. Hook for built-in tools

Claude Code's built-in tools (Read, Write, Bash, Edit, etc.) don't go through MCP. A PostToolUse hook captures them too.

Add to `~/.claude/settings.json`:

```json
{
  "hooks": {
    "PostToolUse": [{
      "matcher": ".*",
      "hooks": [{ "type": "command", "command": "vigilo hook" }]
    }]
  }
}
```

## Cursor

### 1. MCP server

Add to `~/.cursor/mcp.json`:

```json
{
  "mcpServers": {
    "vigilo": {
      "command": "vigilo"
    }
  }
}
```

### 2. Hooks for built-in tools

Add to `~/.cursor/hooks.json`:

```json
{
  "version": 1,
  "hooks": {
    "beforeShellExecution": [{ "command": "vigilo hook" }],
    "afterFileEdit":        [{ "command": "vigilo hook" }],
    "beforeReadFile":       [{ "command": "vigilo hook" }],
    "beforeMCPExecution":   [{ "command": "vigilo hook" }]
  }
}
```
