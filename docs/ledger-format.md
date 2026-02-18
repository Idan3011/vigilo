# Ledger Format

Append-only JSONL at `~/.vigilo/events.jsonl`. One event per line. Rotates at 10 MB, keeping up to 5 archived files.

## Example event

```json
{
  "id": "550e8400-e29b-41d4-a716-446655440000",
  "timestamp": "2026-02-16T14:32:01.123Z",
  "session_id": "6ba7b810-9dad-11d1-80b4-00c04fd430c8",
  "server": "vigilo",
  "tool": "write_file",
  "arguments": { "path": "/src/auth.rs", "content": "..." },
  "outcome": { "status": "ok", "result": "wrote 2048 bytes" },
  "duration_us": 1200,
  "risk": "write",
  "project": {
    "root": "/projects/my-app",
    "name": "my-app",
    "branch": "feature/auth",
    "commit": "23dbb5e",
    "dirty": true
  },
  "model": "claude-opus-4-6",
  "input_tokens": 65000,
  "output_tokens": 270,
  "cache_read_tokens": 162000,
  "diff": "+fn authenticate(token: &str) -> bool {\n-fn auth(t: &str) -> bool {"
}
```

## Field reference

| Field | Always present | Description |
|---|---|---|
| `id` | yes | Unique event UUID |
| `timestamp` | yes | RFC 3339 timestamp |
| `session_id` | yes | Groups all events from one session |
| `server` | yes | `vigilo`, `claude-code`, or `cursor` |
| `tool` | yes | Tool name |
| `arguments` | yes | Tool input (encrypted if key is set) |
| `outcome` | yes | `{status, result}` or `{status, code, message}` |
| `duration_us` | yes | Execution time in microseconds |
| `risk` | yes | `read` / `write` / `exec` / `unknown` |
| `project` | yes | Git context (root, name, branch, commit, dirty) |
| `model` | no | Model that made this call |
| `input_tokens` | no | Cumulative input tokens at this point in the session |
| `output_tokens` | no | Cumulative output tokens |
| `cache_read_tokens` | no | Cache read tokens |
| `diff` | no | Unified diff for write operations |
| `tag` | no | Session label (auto-derived from branch) |

## Rotation

When the active ledger exceeds 10 MB, it is renamed with a timestamp suffix (e.g., `events.1708100000000.jsonl`) and a fresh empty file is created. Up to 5 rotated files are kept; older ones are deleted.

All vigilo commands that read events (`view`, `stats`, `query`, `export`, etc.) scan both the active ledger and all rotated files automatically.
