# Configuration

## Config file

`~/.vigilo/config` — created by `vigilo setup`, applies to every session:

```ini
LEDGER=/home/user/.vigilo/events.jsonl
CURSOR_DB=/home/user/.cursor-server/data/state.vscdb
```

## Environment variables

Override the config file when set.

| Variable | Default | Description |
|---|---|---|
| `VIGILO_LEDGER` | `~/.vigilo/events.jsonl` | Ledger file path |
| `VIGILO_ENCRYPTION_KEY` | _(unset)_ | Base64 AES-256-GCM key; encrypts arguments and results at rest |
| `VIGILO_TAG` | _(git branch)_ | Session label; overrides auto-derived branch name |
| `VIGILO_TIMEOUT_SECS` | `30` | Max seconds per tool call before timeout |
| `CURSOR_DATA_DIR` | _(auto-discovered)_ | Override Cursor database directory for `cursor-usage` |
| `NO_COLOR` | _(unset)_ | Disable colored output (also `--no-color` flag) |

## Encryption

Arguments and results can be encrypted at rest with AES-256-GCM. Metadata (tool name, risk, timing, git context) is always plaintext — the shape of what happened is never hidden, only the content.

```bash
vigilo generate-key
export VIGILO_ENCRYPTION_KEY=<output>
```

Add the export line to your shell profile (`~/.bashrc` or `~/.zshrc`) to persist it.

`vigilo view`, `query`, and `diff` decrypt automatically when the key is present.
