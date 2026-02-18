# vigilo

[![CI](https://github.com/Idan3011/vigilo/actions/workflows/ci.yml/badge.svg)](https://github.com/Idan3011/vigilo/actions/workflows/ci.yml)

> Every tool call your AI agent makes — logged, timed, diff'd. Nothing sent anywhere.

AI coding agents read files, write code, run commands, and commit to git on your behalf. **vigilo** gives you a complete, queryable record of everything they do — without changing how they work or what they can do.

Works with **Claude Code** and **Cursor**. Runs as a standard [MCP](https://modelcontextprotocol.io/) server plus lightweight hooks for built-in tools. Every call is recorded to a local append-only ledger with risk level, timing, git context, model, token usage, and — for file writes — a unified diff of what changed.

No network calls. No accounts. No SaaS. Everything stays on your machine.

---

## What it looks like

### `vigilo view`

```
 ░ CLAUDE ░  df66fc59  16 Feb 17:26
 │  my-app · feature/auth@23dbb5e
 │  claude-opus-4-6
 │  17:26:39  ● EXEC   Bash     cargo test && cargo ins…   6.3s
 │  17:26:45  ○ READ   Read     src/auth.rs
 │  17:27:01  ○ READ   Grep     src/app/**/*.ts
 │  17:27:02  ◆ WRITE  Edit     auth.rs                    +12 -3
 │  17:27:03  ✖ ERR    Bash     npm install                2.1s
 │  ··· 42 more calls ···
 │  17:35:01  ○ READ   Read     auth.rs
 │  17:35:06  ◆ WRITE  Edit     auth.rs                    +3 -1
 │  17:35:12  ● EXEC   Bash     cargo test                 4.2s
 └─ 52 calls · r:22 w:11 e:19 · 1 err · 48.3s
    tokens: 65K in · 270 out · cache: 162K read · ~$0.03 (list pricing)

 ░ CURSOR ░  a3b8c91e  16 Feb 18:02
 │  Auto
 │  18:02:14  ○ READ   Read     navbar.html                141ms
 │  18:02:19  ◆ WRITE  Edit     navbar.html                +8 -3
 └─ 2 calls · r:1 w:1 e:0 · 0.3s
    tokens: 71K in · 5K out · cache: 905K read · $0.35 (2 reqs)
```

Long sessions auto-collapse to the first 5 + last 5 events. Use `--expand` to see everything.

Sessions are sorted by last activity. Claude costs show `~` (estimated from public API list pricing, not your actual bill). Cursor costs come directly from cursor.com billing and are exact.

### `vigilo stats`

```
── vigilo stats ────────────────────────────────

  76 sessions · 1215 calls · 1 error (0%) · 407s total
  risk: 487 read · 329 write · 328 exec

  tools                    files
  ─────                    ─────
  303× Bash                84× hook.rs
  270× Read                64× data.service.ts
  240× Edit                62× view.rs
   86× Write               30× main.rs

  models
  ──────
  310× claude-opus-4-6     996 in · 33K out · ~$66.07
  168× Auto
  144× claude-sonnet-4-5   102 in · 23K out · ~$3.09

  total: 1K in · 56K out · ~$69.16 (list pricing)

  projects
  ────────
  611× my-app          r:265 w:160 e:163
  320× unknown         r:101 w:13  e:165
```

### `vigilo cursor-usage`

```
 ░ CURSOR ░  you@example.com  (pro_plus)
  billing: 02-05 → 03-05  (user)
  plan: 7000/7000 requests  (0 remaining)  26%

── token usage (30d) ────────────────────────────

  213 requests
  1.2M input · 257K output · 38.9M cache read · 2.5M cache write
  $18.89 total cost

  by model
  ────────
  153× Auto                921K in · 174K out · cache:28.5M · $10.41
   40× claude-4.5-sonnet   176K in · 56K out  · cache:4.7M  · $6.26
   20× composer-1.5        135K in · 26K out  · cache:5.6M  · $2.21
```

Fetches real per-request token usage directly from cursor.com. Cached locally so `vigilo view` can enrich Cursor sessions with model, tokens, and cost.

---

## Installation

```bash
curl -fsSL https://raw.githubusercontent.com/Idan3011/vigilo/main/install.sh | bash
```

Downloads a pre-built binary for your platform (Linux x86_64, macOS x86_64/arm64). Falls back to building from source if no binary is available.

**From source** (requires [Rust](https://rustup.rs/)):

```bash
cargo install --git https://github.com/Idan3011/vigilo.git
```

---

## Setup

### Interactive setup (recommended)

```bash
vigilo setup
```

Auto-detects Claude Code and Cursor, configures MCP servers and hooks, optionally generates an encryption key, and saves everything to `~/.vigilo/config`. Zero manual editing required.

### Manual — Claude Code

**1. MCP server** — add to `~/.claude.json`:

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

**2. Hook for built-in tools** — add to `~/.claude/settings.json`:

Claude Code's built-in tools (Read, Write, Bash, Edit, etc.) don't go through MCP. A PostToolUse hook captures them too:

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

### Manual — Cursor

**1. MCP server** — add to `~/.cursor/mcp.json`:

```json
{
  "mcpServers": {
    "vigilo": {
      "command": "vigilo"
    }
  }
}
```

**2. Hooks for built-in tools** — add to `~/.cursor/hooks.json`:

```json
{
  "version": 1,
  "hooks": {
    "beforeShellExecution": [{ "command": "vigilo hook" }],
    "afterFileEdit":        [{ "command": "vigilo hook" }]
  }
}
```

---

## Subcommands

### Today at a glance

```bash
vigilo summary                            # sessions, calls, errors, tokens, cost — today only
```

```
── today ───────────────────────────────────────

  3 sessions · 127 calls · 0 errors · 42.1s
  risk: 48 read · 39 write · 40 exec
  tokens: 12K in · 3K out · cache: 89K read · ~$1.23
  active: ai-observability · feature/render-deploy
```

### Session list

```bash
vigilo sessions                           # one line per session
vigilo sessions --last 5                  # last 5 sessions
vigilo sessions --since 1w               # sessions from the last week
```

```
── 12 sessions ─────────────────────────────────

  ░ CLAUDE ░  df66fc59  02-16 17:26  ai-observability     127 calls  42.1s
  ░ CURSOR ░  a3b7e012  02-16 14:10  my-frontend           83 calls  18.3s
```

### Last N events

```bash
vigilo tail                               # last 20 events (flat, chronological)
vigilo tail -n 50                         # last 50 events
```

```
  02-16 17:27:01  ○ READ   Read     hook.rs              ░ CLAUDE ░  df66fc59
  02-16 17:27:02  ◆ WRITE  Edit     hook.rs    +12 -3    ░ CLAUDE ░  df66fc59
```

### Viewing events

```bash
vigilo view                               # all sessions (first 5 + last 5 events each)
vigilo view --last 5                      # last 5 sessions
vigilo view --last 3 --expand             # last 3 sessions, all events shown
vigilo view --risk exec                   # filter by risk: read | write | exec
vigilo view --tool Bash                   # filter by tool name
vigilo view --since 7d                    # last 7 days
vigilo view --since 2026-02-01 --until yesterday
```

### Live tail

```bash
vigilo watch                              # see events as they happen
```

### Aggregate stats

```bash
vigilo stats                              # all-time stats
vigilo stats --since 1m                   # last month
vigilo stats --since 2026-02-01 --until 2026-02-15
```

### Errors

```bash
vigilo errors                             # errors grouped by tool, with recent list
vigilo errors --since 1w                  # errors from the last week
```

### File diffs

```bash
vigilo diff --last 1                      # what files changed in the last session
vigilo diff --since today                 # all diffs from today
```

### Filtered search

```bash
vigilo query --tool delete_file --since 1w   # what did the AI delete this week?
vigilo query --risk exec --since 2d          # all shell commands, last 2 days
vigilo query --session cd9b                  # events from a specific session
```

### Cursor token usage

```bash
vigilo cursor-usage                       # real token usage from cursor.com (30 days)
vigilo cursor-usage --since-days 7        # last 7 days
vigilo cursor-usage --sync                # fetch and cache without printing
```

Reads credentials from Cursor's local database. Auto-discovers the database path on macOS, Linux, Windows, and WSL. Cached token data enriches `vigilo view` for Cursor sessions.

### Export

```bash
vigilo export                             # CSV to stdout
vigilo export --format json               # full JSON array
```

### Other

```bash
vigilo generate-key                       # generate a base64 AES-256 encryption key
vigilo setup                              # interactive setup wizard
```

**Date expressions** — `today`, `yesterday`, `7d`, `2w`, `1m`, or `YYYY-MM-DD`.

---

## Tools

When running as an MCP server, vigilo exposes these tools to the AI agent:

| Tool | Risk | Description |
|---|---|---|
| `read_file` | read | Read a file; supports `start_line` / `end_line` for large files |
| `write_file` | write | Write content to a file; creates parent directories |
| `list_directory` | read | List directory entries, sorted |
| `create_directory` | write | Create a directory and any missing parents |
| `delete_file` | write | Delete a file |
| `move_file` | write | Move or rename a file or directory |
| `search_files` | read | Recursive pattern search; supports `regex: true` |
| `run_command` | exec | Run a shell command; returns stdout and stderr |
| `get_file_info` | read | File/directory metadata (size, type, modified time) |
| `patch_file` | write | Apply a unified diff patch to a file |
| `git_status` | read | Working tree status |
| `git_diff` | read | Unstaged (or `staged: true`) diff |
| `git_log` | read | Recent commits, one-line format |
| `git_commit` | write | Stage all changes and create a commit |

---

## Configuration

### Config file

`~/.vigilo/config` — created by `vigilo setup`, applies to every session:

```ini
LEDGER=/home/user/.vigilo/events.jsonl
CURSOR_DB=/home/user/.cursor-server/data/state.vscdb
```

### Environment variables

Override the config file when set.

| Variable | Default | Description |
|---|---|---|
| `VIGILO_LEDGER` | `~/.vigilo/events.jsonl` | Ledger file path |
| `VIGILO_ENCRYPTION_KEY` | _(unset)_ | Base64 AES-256-GCM key; encrypts arguments and results at rest |
| `VIGILO_TAG` | _(git branch)_ | Session label; overrides auto-derived branch name |
| `VIGILO_TIMEOUT_SECS` | `30` | Max seconds per tool call before timeout |
| `CURSOR_DATA_DIR` | _(auto-discovered)_ | Override Cursor database directory for `cursor-usage` |

### Encryption

Arguments and results can be encrypted at rest with AES-256-GCM. Metadata (tool name, risk, timing, git context) is always plaintext — the shape of what happened is never hidden, only the content.

```bash
vigilo generate-key
export VIGILO_ENCRYPTION_KEY=<output>
```

`vigilo view`, `query`, and `diff` decrypt automatically when the key is present.

---

## Ledger format

Append-only JSONL at `~/.vigilo/events.jsonl`. One event per line. Rotates at 10 MB, keeping up to 5 archived files.

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

---

## Architecture

```
src/
├── main.rs            CLI entry: dispatches subcommands
├── server/
│   ├── mod.rs         MCP JSON-RPC server over stdio
│   ├── execute.rs     Tool dispatch, ledger logging, encryption
│   ├── tools.rs       14 tool implementations (fs, git, shell)
│   └── schema.rs      Tool JSON schemas for tools/list
├── view/
│   ├── mod.rs         View entry point and shared helpers
│   ├── stats.rs       Stats, errors, summary subcommands
│   ├── counts.rs      Event aggregation and section printers
│   ├── session.rs     Session list, detail, and tail views
│   ├── search.rs      Query, diff, watch, CSV export
│   ├── data.rs        Ledger loading and event filtering
│   └── fmt.rs         Shared formatting (colors, duration, tokens)
├── hook.rs            Claude Code PostToolUse + Cursor hook processing
├── hook_helpers.rs    Shared hook utilities (events, transcripts, diffs)
├── models.rs          McpEvent, Outcome, Risk, ProjectContext
├── ledger.rs          Append-only JSONL writer with 10MB rotation
├── cursor_usage.rs    Cursor token usage via local DB + cursor.com API
├── setup.rs           Interactive setup wizard
├── git.rs             Async git helpers (root, name, branch, commit, dirty)
└── crypto.rs          AES-256-GCM encryption/decryption
```

## Design principles

- **Local only** — no network calls in the MCP server path; `cursor-usage` is opt-in
- **Non-blocking** — ledger failures log to stderr; tool responses are never delayed
- **Witness, not judge** — records what happened; enforces no policies, blocks nothing
- **Shape is transparent, content is private** — timing, risk, and git context are always plaintext; file contents are optionally encrypted
