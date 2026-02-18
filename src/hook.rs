use crate::{
    git, ledger, models,
    models::{McpEvent, Outcome, ProjectContext, Risk},
    server,
};
use anyhow::Result;
use chrono::Utc;
use uuid::Uuid;

/// Which AI client sent this hook payload.
/// Add a new variant here when supporting a new client — one place, one decision.
#[derive(Debug)]
enum HookClient {
    Cursor,
    ClaudeCode,
}

/// Identify the client from the raw payload.
///
/// Rules (in priority order):
///   1. `conversation_id` present  → Cursor.
///      Both Cursor lifecycle hooks and Cursor PostToolUse always carry this field.
///      Claude Code never sends it.
///   2. Everything else            → Claude Code PostToolUse.
fn detect_client(payload: &serde_json::Value) -> HookClient {
    if payload.get("conversation_id").is_some() {
        return HookClient::Cursor;
    }
    HookClient::ClaudeCode
}

pub async fn run(ledger_path: &str) -> Result<()> {
    use std::io::Read;
    let mut input = String::new();
    std::io::stdin().read_to_string(&mut input)?;

    let Ok(payload) = serde_json::from_str::<serde_json::Value>(&input) else {
        return Ok(());
    };

    match detect_client(&payload) {
        HookClient::Cursor => handle_cursor_hook(&payload, ledger_path).await,
        HookClient::ClaudeCode => handle_claude_hook(&payload, ledger_path).await,
    }
}

// ── Claude Code PostToolUse ───────────────────────────────────────────────────

async fn handle_claude_hook(payload: &serde_json::Value, ledger_path: &str) -> Result<()> {
    let tool_name = payload["tool_name"]
        .as_str()
        .unwrap_or("unknown")
        .to_string();

    // Skip vigilo's own tools — already logged by the MCP server
    if tool_name.starts_with("mcp__vigilo__") {
        return Ok(());
    }

    let mut arguments = payload["tool_input"].clone();
    // Strip full file content from Write/write_file — file_path is sufficient.
    // Content can be tens of KB and adds no observability value.
    if matches!(tool_name.as_str(), "Write" | "write_file") {
        if let Some(obj) = arguments.as_object_mut() {
            obj.remove("content");
        }
    }
    let response = payload["tool_response"].clone();
    let cwd = payload["cwd"].as_str().unwrap_or(".");

    let is_error = response
        .get("is_error")
        .and_then(|v| v.as_bool())
        .unwrap_or(false)
        || response
            .get("success")
            .and_then(|v| v.as_bool())
            .map(|s| !s)
            .unwrap_or(false);

    let store_response = hook_store_response();
    let outcome = if is_error {
        Outcome::Err {
            code: -1,
            message: extract_error_message(&response),
        }
    } else if store_response {
        Outcome::Ok { result: response }
    } else {
        // Default: drop response content — can be huge (base64 images, full file reads).
        // Enable with HOOK_STORE_RESPONSE=true in ~/.vigilo/config or env.
        Outcome::Ok {
            result: serde_json::Value::Null,
        }
    };

    let risk = Risk::classify(&tool_name);

    // transcript_path is the same file for the entire conversation — most stable grouping key.
    // Fall back to session_id string (hashed), then random.
    let session_id = payload["transcript_path"]
        .as_str()
        .or_else(|| payload["session_id"].as_str())
        .map(stable_uuid)
        .unwrap_or_else(Uuid::new_v4);

    // Edit/MultiEdit carry old_string + new_string — we can diff them directly
    let diff = compute_edit_diff(&tool_name, &arguments);

    let git_dir = resolve_git_dir(&tool_name, &arguments, cwd);
    let project = build_project(&git_dir).await;
    let tag = std::env::var("VIGILO_TAG")
        .ok()
        .or_else(|| project.branch.clone());

    let tool_use_id_str = payload["tool_use_id"].as_str();
    let tmeta = payload["transcript_path"]
        .as_str()
        .map(|p| read_transcript_meta(p, tool_use_id_str))
        .unwrap_or_default();

    let event = McpEvent {
        id: Uuid::new_v4(),
        timestamp: Utc::now().to_rfc3339(),
        session_id,
        server: "claude-code".to_string(),
        tool: tool_name,
        arguments,
        outcome,
        duration_us: tmeta.duration_us.unwrap_or(0),
        risk,
        project,
        tag,
        diff,
        timed_out: false,
        model: tmeta.model,
        input_tokens: tmeta.input_tokens,
        output_tokens: tmeta.output_tokens,
        cache_read_tokens: tmeta.cache_read_tokens,
        cache_write_tokens: tmeta.cache_write_tokens,
        stop_reason: tmeta.stop_reason,
        service_tier: tmeta.service_tier,
        permission_mode: payload["permission_mode"].as_str().map(|s| s.to_string()),
        tool_use_id: tool_use_id_str.map(|s| s.to_string()),
        cursor_version: None,
        generation_id: None,
    };

    if let Err(e) = ledger::append_event(&event, ledger_path) {
        eprintln!("[vigilo hook] ledger error: {e}");
    }
    Ok(())
}

// ── Cursor agent lifecycle hooks ──────────────────────────────────────────────
//
// Two paths arrive here:
// A. Cursor lifecycle hooks (hooks.json): hook_event_name = beforeShellExecution / afterFileEdit …
// B. Cursor PostToolUse (via ~/.claude/settings.json): no hook_event_name, native tool names
//    (Shell, Write, Read, MCP:list_directory, …)

async fn handle_cursor_hook(payload: &serde_json::Value, ledger_path: &str) -> Result<()> {
    let hook_event = payload["hook_event_name"].as_str().unwrap_or("PostToolUse");

    // stop / beforeSubmitPrompt carry no actionable tool data — skip
    if matches!(hook_event, "stop" | "beforeSubmitPrompt") {
        return Ok(());
    }

    // conversation_id is the stable session key for Cursor
    let session_id = payload["conversation_id"]
        .as_str()
        .map(stable_uuid)
        .unwrap_or_else(Uuid::new_v4);

    let cwd = payload["cwd"]
        .as_str()
        .or_else(|| {
            payload["workspace_roots"]
                .as_array()
                .and_then(|a| a.first())
                .and_then(|v| v.as_str())
        })
        .unwrap_or(".");

    let (tool_name, arguments, risk, diff) = match hook_event {
        "beforeShellExecution" => {
            let cmd = payload["command"].as_str().unwrap_or("").to_string();
            let args = serde_json::json!({ "command": cmd });
            (String::from("Bash"), args, Risk::Exec, None)
        }

        "afterFileEdit" => {
            let file_path = payload["file_path"].as_str().unwrap_or("");
            let diff = payload["edits"].as_array().and_then(|edits| {
                let mut out = String::new();
                for edit in edits {
                    let old = edit
                        .get("old_string")
                        .and_then(|v| v.as_str())
                        .unwrap_or("");
                    let new = edit
                        .get("new_string")
                        .and_then(|v| v.as_str())
                        .unwrap_or("");
                    if let Some(d) = models::compute_unified_diff(old, new) {
                        out.push_str(&d);
                    }
                }
                if out.is_empty() {
                    None
                } else {
                    Some(out)
                }
            });
            let args = serde_json::json!({ "file_path": file_path });
            (String::from("Edit"), args, Risk::Write, diff)
        }

        "beforeReadFile" => {
            let file_path = payload["file_path"].as_str().unwrap_or("");
            let args = serde_json::json!({ "file_path": file_path });
            (String::from("Read"), args, Risk::Read, None)
        }

        "beforeMCPExecution" => {
            let tool = payload["tool_name"]
                .as_str()
                .unwrap_or("unknown")
                .to_string();
            let args = payload["tool_input"].clone();
            let r = Risk::classify(&tool);
            (tool, args, r, None)
        }

        // Cursor PostToolUse — received when ~/.claude/settings.json has PostToolUse hooks.
        // Cursor sends "postToolUse" (lowercase); keep accepting "PostToolUse" for older versions.
        // Cursor 2.4+ enriches this payload with "model", "duration", "tool_output", etc.
        "PostToolUse" | "postToolUse" => {
            let raw_tool = payload["tool_name"].as_str().unwrap_or("unknown");
            // Strip MCP: prefix (e.g. "MCP:list_directory" → "list_directory")
            let tool = raw_tool.strip_prefix("MCP:").unwrap_or(raw_tool);
            // Normalise Cursor native names to canonical ones
            let canonical = match tool {
                "Shell" => "Bash",
                "Write" => "Edit",
                other => other,
            };
            let mut args = payload["tool_input"].clone();
            if args.is_null() {
                args = payload["arguments"].clone();
            }
            // Cursor Write carries full file content — drop it to keep the ledger lean
            if let Some(obj) = args.as_object_mut() {
                if obj.contains_key("content") {
                    obj.remove("content");
                }
            }
            let r = Risk::classify(canonical);
            (canonical.to_string(), args, r, None)
        }

        other => {
            // Unknown future Cursor hook — log as-is
            let args = payload.clone();
            (other.to_string(), args, Risk::Unknown, None)
        }
    };

    let git_dir = resolve_git_dir(&tool_name, &arguments, cwd);
    let project = build_project(&git_dir).await;
    let tag = std::env::var("VIGILO_TAG")
        .ok()
        .or_else(|| project.branch.clone());

    // Duration: Cursor 2.4+ postToolUse includes "duration" in ms — convert to µs.
    let duration_us = payload["duration"]
        .as_f64()
        .map(|ms| (ms * 1000.0) as u64)
        .unwrap_or(0);

    // Model priority:
    //   1. payload["model"] — present in Cursor 2.4+ postToolUse (exact, per-request)
    //   2. lastUsedModel from conversation's SQLite store (per-conversation)
    //   3. cli-config.json global default (coarse fallback)
    //
    // "default" / "auto" are Cursor's name for Auto mode. We normalize them
    // to "Auto" for consistent display and cost tracking.
    let raw_conv_id = payload["conversation_id"].as_str().unwrap_or("");
    let model = payload["model"]
        .as_str()
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string())
        .or_else(|| read_cursor_model_from_db(raw_conv_id))
        .or_else(read_cursor_model_fallback)
        .map(|s| normalize_cursor_model(&s));

    let event = McpEvent {
        id: Uuid::new_v4(),
        timestamp: Utc::now().to_rfc3339(),
        session_id,
        server: "cursor".to_string(),
        tool: tool_name,
        arguments,
        outcome: Outcome::Ok {
            result: serde_json::Value::Null,
        },
        duration_us,
        risk,
        project,
        tag,
        diff,
        timed_out: false,
        model,
        // Token counts not exposed in Cursor hook payloads
        input_tokens: None,
        output_tokens: None,
        cache_read_tokens: None,
        cache_write_tokens: None,
        stop_reason: None,
        service_tier: None,
        permission_mode: None,
        tool_use_id: payload["tool_use_id"].as_str().map(|s| s.to_string()),
        cursor_version: payload["cursor_version"].as_str().map(|s| s.to_string()),
        generation_id: payload["generation_id"].as_str().map(|s| s.to_string()),
    };

    if let Err(e) = ledger::append_event(&event, ledger_path) {
        eprintln!("[vigilo hook] ledger error: {e}");
    }
    Ok(())
}

// ── Shared helpers ────────────────────────────────────────────────────────────

async fn build_project(git_dir: &str) -> ProjectContext {
    ProjectContext {
        root: git::root_in(git_dir).await,
        name: git::name_in(Some(git_dir)).await,
        branch: git::branch_in(git_dir).await,
        commit: git::commit_in(git_dir).await,
        dirty: git::dirty_in(git_dir).await,
    }
}

fn resolve_git_dir(tool: &str, args: &serde_json::Value, cwd: &str) -> String {
    let path_str = match tool {
        "Read" | "Edit" | "Write" | "MultiEdit" | "NotebookEdit" => {
            args.get("file_path").and_then(|v| v.as_str())
        }
        "Glob" | "Grep" => args.get("path").and_then(|v| v.as_str()),
        _ => args
            .get("file_path")
            .or_else(|| args.get("path"))
            .and_then(|v| v.as_str()),
    };

    match path_str {
        Some(p) => {
            let path = std::path::Path::new(p);
            if path.is_dir() {
                p.to_string()
            } else {
                path.parent()
                    .and_then(|d| d.to_str())
                    .unwrap_or(cwd)
                    .to_string()
            }
        }
        None => cwd.to_string(),
    }
}

fn compute_edit_diff(tool: &str, args: &serde_json::Value) -> Option<String> {
    if tool != "Edit" && tool != "MultiEdit" {
        return None;
    }
    let old = args.get("old_string").and_then(|v| v.as_str())?;
    let new = args.get("new_string").and_then(|v| v.as_str())?;
    models::compute_unified_diff(old, new)
}

fn extract_error_message(response: &serde_json::Value) -> String {
    response
        .get("content")
        .and_then(|c| c.as_array())
        .and_then(|a| a.first())
        .and_then(|v| v.get("text"))
        .and_then(|v| v.as_str())
        .or_else(|| response.get("error").and_then(|v| v.as_str()))
        .unwrap_or("error")
        .to_string()
}

/// Fixed namespace UUID for deriving stable session IDs via UUID v5 (SHA-1).
/// Changing this value would break session grouping for new ledger entries.
const SESSION_NAMESPACE: Uuid = Uuid::from_bytes([
    0xa1, 0xb2, 0xc3, 0xd4, 0xe5, 0xf6, 0x47, 0x08, 0x89, 0xab, 0xcd, 0xef, 0x01, 0x23, 0x45, 0x67,
]);

/// Derive a deterministic UUID from any string (transcript path, conversation_id,
/// etc.) using UUID v5 (SHA-1). Stable across Rust versions and platforms.
fn stable_uuid(s: &str) -> Uuid {
    Uuid::new_v5(&SESSION_NAMESPACE, s.as_bytes())
}

/// Whether to store full tool response content in hook events.
/// Default: false — responses can be huge (base64 images, full file reads).
/// Set HOOK_STORE_RESPONSE=true in ~/.vigilo/config or env to enable.
fn hook_store_response() -> bool {
    let config = server::load_config();
    let val = std::env::var("VIGILO_HOOK_STORE_RESPONSE")
        .ok()
        .or_else(|| config.get("HOOK_STORE_RESPONSE").cloned())
        .unwrap_or_default();
    matches!(val.to_lowercase().as_str(), "true" | "1" | "yes")
}

/// All metadata extractable from the Claude Code transcript for the current tool call.
#[derive(Default)]
struct TranscriptMeta {
    model: Option<String>,
    input_tokens: Option<u64>,
    output_tokens: Option<u64>,
    cache_read_tokens: Option<u64>,
    cache_write_tokens: Option<u64>,
    stop_reason: Option<String>,
    service_tier: Option<String>,
    /// Wall-clock duration of this tool call, computed from transcript timestamps.
    /// = tool_result message timestamp − tool_use message timestamp.
    /// None when the tool_use_id is not found in the tailed region.
    duration_us: Option<u64>,
}

/// Read metadata from a Claude Code transcript (JSONL).
///
/// Model / usage / stop_reason: tails the last 64 KB (these are always in the
/// most recent assistant message, right at the end of the file).
///
/// Duration: the tool_use and tool_result messages can be arbitrarily far from
/// the end of a long session. We search backwards in 64 KB chunks up to 512 KB
/// until both are found, then compute result_ts − invoke_ts.
fn read_transcript_meta(transcript_path: &str, tool_use_id: Option<&str>) -> TranscriptMeta {
    use std::io::{BufRead, Seek, SeekFrom};

    let Ok(mut file) = std::fs::File::open(transcript_path) else {
        return TranscriptMeta::default();
    };
    let size = file.metadata().map(|m| m.len()).unwrap_or(0);

    // ── Part 1: model / usage from the last 64 KB ────────────────────────────
    let tail_start = size.saturating_sub(64 * 1024);
    let _ = file.seek(SeekFrom::Start(tail_start));
    let mut reader = std::io::BufReader::new(&mut file);
    if tail_start > 0 {
        let mut skip = String::new();
        let _ = reader.read_line(&mut skip);
    }

    let mut meta = TranscriptMeta::default();
    for line in reader.lines().map_while(Result::ok) {
        let Ok(v) = serde_json::from_str::<serde_json::Value>(&line) else {
            continue;
        };
        if v["type"].as_str() != Some("assistant") {
            continue;
        }
        let msg = &v["message"];
        if let Some(m) = msg["model"].as_str() {
            meta.model = Some(m.to_string());
        }
        if let Some(r) = msg["stop_reason"].as_str() {
            meta.stop_reason = Some(r.to_string());
        }
        let usage = &msg["usage"];
        meta.input_tokens = usage["input_tokens"].as_u64().or(meta.input_tokens);
        meta.output_tokens = usage["output_tokens"].as_u64().or(meta.output_tokens);
        meta.cache_read_tokens = usage["cache_read_input_tokens"]
            .as_u64()
            .or(meta.cache_read_tokens);
        meta.cache_write_tokens = usage["cache_creation_input_tokens"]
            .as_u64()
            .or(meta.cache_write_tokens);
        if let Some(t) = usage["service_tier"].as_str() {
            meta.service_tier = Some(t.to_string());
        }
    }

    // ── Part 2: duration via backwards scan for tool_use_id ──────────────────
    // The tool_use and tool_result are consecutive messages written just before
    // the hook fires.  In a long session they can fall just outside a fixed tail.
    // Scan backwards in 64 KB chunks, giving up after 512 KB total (8 chunks).
    if let Some(id) = tool_use_id {
        let id_bytes = id.as_bytes();

        // Read the scan region in one go (≤ 512 KB — fast enough).
        let read_from = size.saturating_sub(512 * 1024);
        if file.seek(SeekFrom::Start(read_from)).is_ok() {
            let mut reader2 = std::io::BufReader::new(&mut file);
            if read_from > 0 {
                let mut skip = String::new();
                let _ = reader2.read_line(&mut skip);
            }

            let mut invoke_ts: Option<i64> = None;
            let mut result_ts: Option<i64> = None;

            for line in reader2.lines().map_while(Result::ok) {
                // Fast pre-filter: skip lines that don't mention the tool_use_id.
                if !line
                    .as_bytes()
                    .windows(id_bytes.len())
                    .any(|w| w == id_bytes)
                {
                    continue;
                }
                let Ok(v) = serde_json::from_str::<serde_json::Value>(&line) else {
                    continue;
                };
                let msg_us = v["timestamp"]
                    .as_str()
                    .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
                    .map(|dt| dt.timestamp_micros());
                let Some(ts) = msg_us else { continue };

                match v["type"].as_str() {
                    Some("assistant") => {
                        if let Some(content) = v["message"]["content"].as_array() {
                            if content.iter().any(|item| {
                                item["type"] == "tool_use" && item["id"].as_str() == Some(id)
                            }) {
                                invoke_ts = Some(ts);
                            }
                        }
                    }
                    Some("user") => {
                        if let Some(content) = v["message"]["content"].as_array() {
                            if content.iter().any(|item| {
                                item["type"] == "tool_result"
                                    && item["tool_use_id"].as_str() == Some(id)
                            }) {
                                result_ts = Some(ts);
                            }
                        }
                    }
                    _ => {}
                }
            }

            if let (Some(start), Some(end)) = (invoke_ts, result_ts) {
                let diff_us = end - start;
                if diff_us > 0 {
                    meta.duration_us = Some(diff_us as u64);
                }
            }
        }
    }

    meta
}

/// Read the actual model used for a Cursor conversation from its SQLite store.
///
/// Cursor stores conversation metadata in:
///   ~/.cursor/chats/<workspace-hash>/<conversation-id>/store.db
///
/// The `meta` table has one row (key="0") whose value is a hex-encoded JSON
/// object. That object contains `"lastUsedModel"` — the exact model string
/// Cursor used for this conversation (e.g. "claude-4.5-sonnet-thinking",
/// "gpt-5", "claude-4-sonnet").
///
/// We read the raw SQLite file and search for the known byte pattern rather
/// than linking against libsqlite3. The hex-encoded JSON appears as literal
/// ASCII in the file, so a simple byte search is both correct and fast.
fn read_cursor_model_from_db(conversation_id: &str) -> Option<String> {
    let home = std::env::var("HOME").ok()?;
    let chats = std::path::Path::new(&home).join(".cursor/chats");

    // Walk workspace-hash directories to find <conversation_id>/store.db
    for entry in std::fs::read_dir(&chats).ok()?.flatten() {
        let db = entry.path().join(conversation_id).join("store.db");
        if db.exists() {
            return extract_last_used_model_from_db(&db);
        }
    }
    None
}

/// Hex encoding of the JSON key sequence `"lastUsedModel":"`.
/// When Node.js does `Buffer.from(JSON.stringify(obj)).toString('hex')`,
/// each ASCII byte maps to exactly two lowercase hex digits in the output.
const LAST_USED_MODEL_NEEDLE: &[u8] = b"226c617374557365644d6f64656c223a22";

fn extract_last_used_model_from_db(db_path: &std::path::Path) -> Option<String> {
    let data = std::fs::read(db_path).ok()?;

    let pos = data
        .windows(LAST_USED_MODEL_NEEDLE.len())
        .position(|w| w == LAST_USED_MODEL_NEEDLE)?;

    // After the needle, read hex chars until the closing `"` (encoded as "22").
    let after = &data[pos + LAST_USED_MODEL_NEEDLE.len()..];
    let end = after.windows(2).position(|w| w == b"22")?;
    let model_hex = &after[..end];

    if model_hex.len() % 2 != 0 {
        return None;
    }

    // Decode two-hex-char pairs back to bytes, then to UTF-8.
    let model_bytes: Vec<u8> = model_hex
        .chunks(2)
        .filter_map(|c| u8::from_str_radix(std::str::from_utf8(c).ok()?, 16).ok())
        .collect();

    String::from_utf8(model_bytes)
        .ok()
        .filter(|s| !s.is_empty())
}

/// Fallback: read the global default model from Cursor's cli-config.json.
fn read_cursor_model_fallback() -> Option<String> {
    let home = std::env::var("HOME").ok()?;
    let path = std::path::Path::new(&home).join(".cursor/cli-config.json");
    let content = std::fs::read_to_string(path).ok()?;
    let v: serde_json::Value = serde_json::from_str(&content).ok()?;
    v["model"]["displayName"]
        .as_str()
        .or_else(|| v["model"]["displayModelId"].as_str())
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string())
}

/// Normalize Cursor model placeholders to a canonical name.
/// "default" and "auto" both mean Cursor's Auto routing mode.
fn normalize_cursor_model(model: &str) -> String {
    match model {
        "default" | "auto" => "Auto".to_string(),
        _ => model.to_string(),
    }
}
