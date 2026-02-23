<picture>
  <source media="(prefers-color-scheme: dark)" srcset="docs/vigilo-wordmark-dark.svg">
  <img src="docs/vigilo-wordmark-light.svg" alt="vigilo" width="260">
</picture>

[![CI](https://github.com/Idan3011/vigilo/actions/workflows/ci.yml/badge.svg)](https://github.com/Idan3011/vigilo/actions/workflows/ci.yml)

> Every tool call your AI agent makes — logged, timed, diff'd. Nothing sent anywhere.

AI coding agents read files, write code, run commands, and commit to git on your behalf. **vigilo** gives you a complete, queryable record of everything they do — without changing how they work or what they can do. Works with **Claude Code** and **Cursor**.

---

## What it looks like

![vigilo demo](docs/demo.gif)

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

### `vigilo dashboard`

Launch a real-time web dashboard to visualize all agent activity.

![vigilo dashboard](docs/dashboard.gif)

```bash
vigilo dashboard              # default port 7847
vigilo dashboard --port 9000  # custom port
```

- **Session timeline** — calls, cost, and errors over time with interactive charts
- **Live event feed** — every tool call appears in real-time via SSE, with filtering by server/risk/tool and sortable columns
- **Session merging** — context-compressed conversation fragments are auto-merged into unified sessions
- **Token breakdown** — input, output, cache read/write by model with cost estimates
- **Cross-dimensional stats** — tools, files, models, and projects in one view
- Binds to `127.0.0.1` only — never exposed to the network. Includes CORS, CSP, and host validation headers. If the port is in use, vigilo prompts for a random available port.

---

## How it works

vigilo runs as a local [MCP](https://modelcontextprotocol.io/) server over **stdio**. There is no network listener, no open port, no daemon. The AI agent (Claude Code or Cursor) spawns vigilo as a child process — the same way it would spawn any MCP server.

When the agent makes a tool call through MCP, vigilo logs the event to `~/.vigilo/events.jsonl` (tool name, arguments, result, timing, risk level, git context, diffs) and then executes it locally. The agent sees no difference.

Some tools are built into the agent and bypass MCP entirely (Claude Code's Read, Write, Bash, Edit, etc.). For these, a lightweight **PostToolUse hook** pipes the event to `vigilo hook` via stdin after the tool completes. Same logging, zero overhead on execution.

**Nothing leaves your machine.** No telemetry, no phoning home, no accounts, no SaaS. The only exception is `vigilo cursor-usage`, which makes opt-in HTTPS requests to cursor.com to fetch your token usage — using your existing local Cursor credentials.

**Encryption at rest is automatic.** On first run, vigilo generates an AES-256-GCM key at `~/.vigilo/encryption.key` and encrypts content (arguments, results) transparently. The key is zeroized from memory on drop. Metadata (tool name, risk level, timing, git context) is always plaintext so you can always see the shape of what happened. You can also provide your own key via `VIGILO_ENCRYPTION_KEY` env var.

---

## Install

```bash
curl -fsSL https://raw.githubusercontent.com/Idan3011/vigilo/main/install.sh | bash
```

Or from source: `cargo install --git https://github.com/Idan3011/vigilo.git`

Then run the interactive setup:

```bash
vigilo setup
```

Auto-detects Claude Code and Cursor, configures MCP servers and hooks, optionally generates an encryption key, and saves everything to `~/.vigilo/config`.

For manual setup, see [docs/manual-setup.md](docs/manual-setup.md).

---

## Commands

```bash
vigilo                        # MCP server mode (default, reads stdio)
vigilo dashboard              # web dashboard on port 7847
vigilo view                   # ledger grouped by session
vigilo sessions               # one-line session list
vigilo stats                  # aggregate stats across all sessions
vigilo errors                 # errors grouped by tool
vigilo tail                   # last 20 events (flat, chronological)
vigilo diff --last 1          # file diffs from last session
vigilo watch                  # live tail of incoming events
vigilo cursor-usage           # Cursor token usage (last 30 days)
vigilo export                 # dump all events as CSV
vigilo summary                # today at a glance
vigilo doctor                 # check configuration health
vigilo setup                  # interactive setup wizard
vigilo prune                  # delete old rotated ledger files
vigilo generate-key           # generate AES-256 encryption key
```

Most commands accept `--since`, `--until`, `--last`, `--session`, `--risk`, `--tool`, and `--expand` flags. See [docs/commands.md](docs/commands.md) for full details.

---

## Documentation

- [Command reference](docs/commands.md) — all subcommands, flags, and examples
- [Configuration](docs/configuration.md) — config file, environment variables, encryption
- [Ledger format](docs/ledger-format.md) — JSONL schema, field reference, rotation
- [Architecture](docs/architecture.md) — source tree, design principles, development
- [Manual setup](docs/manual-setup.md) — Claude Code and Cursor JSON snippets

## License

[MIT](LICENSE)
