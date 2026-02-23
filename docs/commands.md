# Command Reference

## Today at a glance

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

## Session list

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

## Last N events

```bash
vigilo tail                               # last 20 events (flat, chronological)
vigilo tail -n 50                         # last 50 events
```

```
  02-16 17:27:01  ○ READ   Read     hook.rs              ░ CLAUDE ░  df66fc59
  02-16 17:27:02  ◆ WRITE  Edit     hook.rs    +12 -3    ░ CLAUDE ░  df66fc59
```

## Viewing events

```bash
vigilo view                               # all sessions (first 5 + last 5 events each)
vigilo view --last 5                      # last 5 sessions
vigilo view --last 3 --expand             # last 3 sessions, all events shown
vigilo view --risk exec                   # filter by risk: read | write | exec
vigilo view --tool Bash                   # filter by tool name
vigilo view --since 7d                    # last 7 days
vigilo view --since 2026-02-01 --until yesterday
```

Long sessions auto-collapse to the first 5 + last 5 events. Use `--expand` to see everything.

## Live tail

```bash
vigilo watch                              # see events as they happen
```

## Aggregate stats

```bash
vigilo stats                              # all-time stats
vigilo stats --since 1m                   # last month
vigilo stats --since 2026-02-01 --until 2026-02-15
```

Shows session count, tool calls, risk breakdown, token usage, estimated cost, tool/file frequency, model breakdown, and active projects.

## Errors

```bash
vigilo errors                             # errors grouped by tool, with recent list
vigilo errors --since 1w                  # errors from the last week
```

## File diffs

```bash
vigilo diff --last 1                      # what files changed in the last session
vigilo diff --since today                 # all diffs from today
```

## Filtered search

```bash
vigilo query --tool delete_file --since 1w   # what did the AI delete this week?
vigilo query --risk exec --since 2d          # all shell commands, last 2 days
vigilo query --session cd9b                  # events from a specific session
```

## Cursor token usage

```bash
vigilo cursor-usage                       # real token usage from cursor.com (30 days)
vigilo cursor-usage --since-days 7        # last 7 days
vigilo cursor-usage --sync                # fetch and cache without printing
```

Reads credentials from Cursor's local database. Auto-discovers the database path on macOS, Linux, Windows, and WSL. Cached token data enriches `vigilo view` for Cursor sessions.

## Export

```bash
vigilo export                             # save to ~/.vigilo/export.csv
vigilo export --format json               # save as JSON
vigilo export --output ~/report.csv       # custom output path
vigilo export --since today               # export only today's events
vigilo export --last 3 --format json      # last 3 sessions as JSON
```

## Prune old ledger files

```bash
vigilo prune                             # delete rotated files older than 30 days
vigilo prune --older-than 7              # delete rotated files older than 7 days
```

Only affects rotated ledger files (e.g. `events.1234567890.jsonl`). The active ledger file is never deleted.

## Health check

```bash
vigilo doctor                             # check configuration and dependencies
```

Validates ledger path, encryption key, config file, and Cursor database. Shows pass/fail/info for each check.

## Dashboard

```bash
vigilo dashboard                          # launch web dashboard on port 7847
vigilo dashboard --port 9000              # custom port
```

Opens a real-time web dashboard with session timeline, token breakdown, risk charts, model usage, and a live event feed. Sessions from the same conversation (e.g. after context compression) are automatically merged.

If the default port is in use, vigilo will prompt to use an available port instead.

## Other

```bash
vigilo generate-key                       # generate a base64 AES-256 encryption key
vigilo setup                              # interactive setup wizard
```

## MCP tools

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

## Date expressions

`today`, `yesterday`, `7d`, `2w`, `1m`, or `YYYY-MM-DD`.

## Global flags

`--no-color` disables colored output (also respects `NO_COLOR` env).
