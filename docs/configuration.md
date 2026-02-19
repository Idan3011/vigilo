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
| `VIGILO_ENCRYPTION_KEY` | _(auto-generated)_ | Base64 AES-256-GCM key; overrides the key file at `~/.vigilo/encryption.key` |
| `VIGILO_TAG` | _(git branch)_ | Session label; overrides auto-derived branch name |
| `VIGILO_TIMEOUT_SECS` | `30` | Max seconds per tool call before timeout |
| `CURSOR_DATA_DIR` | _(auto-discovered)_ | Override Cursor database directory for `cursor-usage` |
| `NO_COLOR` | _(unset)_ | Disable colored output (also `--no-color` flag) |

## Encryption

Arguments and results are encrypted at rest with AES-256-GCM. Metadata (tool name, risk, timing, git context) is always plaintext — the shape of what happened is never hidden, only the content.

**Automatic (default):** On first MCP server run, vigilo auto-generates a key and saves it to `~/.vigilo/encryption.key` (mode 600). No setup required.

**Manual override:** Set the `VIGILO_ENCRYPTION_KEY` env var with a base64-encoded 32-byte key. This takes priority over the key file.

```bash
vigilo generate-key                        # print a new key
export VIGILO_ENCRYPTION_KEY=<output>      # override the key file
```

`vigilo view`, `query`, and `diff` decrypt automatically when the key is present (from file or env var).
