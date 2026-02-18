# Architecture

## Source tree

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
│   ├── search.rs      Query, diff, watch, CSV/JSON export
│   ├── data.rs        Ledger loading and event filtering
│   └── fmt.rs         Shared formatting (colors, duration, tokens)
├── doctor.rs          Health check subcommand (vigilo doctor)
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

## Development

```bash
git clone https://github.com/Idan3011/vigilo.git && cd vigilo
cargo run -- doctor          # run from source, no install step
make dev                     # test + clippy + fmt + install to ~/.cargo/bin
```
