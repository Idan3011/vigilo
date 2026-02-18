use crate::{
    hook_helpers::{
        build_project, compute_edit_diff, extract_error_message, read_transcript_meta,
        resolve_git_dir, stable_uuid, write_hook_event,
    },
    models::{McpEvent, Outcome, Risk},
    server,
};
use anyhow::Result;
use chrono::Utc;
use uuid::Uuid;

#[derive(Debug)]
enum HookClient {
    Cursor,
    ClaudeCode,
}

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

async fn handle_claude_hook(payload: &serde_json::Value, ledger_path: &str) -> Result<()> {
    let (tool_name, arguments) = parse_claude_tool(payload);
    if tool_name.starts_with("mcp__vigilo__") {
        return Ok(());
    }

    let outcome = build_claude_outcome(&payload["tool_response"]);
    let risk = Risk::classify(&tool_name);
    let session_id = claude_session_id(payload);
    let diff = compute_edit_diff(&tool_name, &arguments);

    let cwd = payload["cwd"].as_str().unwrap_or(".");
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

    let event = build_claude_event(
        session_id,
        tool_name,
        arguments,
        outcome,
        risk,
        project,
        tag,
        diff,
        &tmeta,
        payload,
        tool_use_id_str,
    );

    write_hook_event(&event, ledger_path);
    Ok(())
}

fn parse_claude_tool(payload: &serde_json::Value) -> (String, serde_json::Value) {
    let tool_name = payload["tool_name"]
        .as_str()
        .unwrap_or("unknown")
        .to_string();
    let mut arguments = payload["tool_input"].clone();
    if matches!(tool_name.as_str(), "Write" | "write_file") {
        if let Some(obj) = arguments.as_object_mut() {
            obj.remove("content");
        }
    }
    (tool_name, arguments)
}

fn claude_session_id(payload: &serde_json::Value) -> Uuid {
    payload["transcript_path"]
        .as_str()
        .or_else(|| payload["session_id"].as_str())
        .map(stable_uuid)
        .unwrap_or_else(Uuid::new_v4)
}

fn build_claude_outcome(response: &serde_json::Value) -> Outcome {
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
    if is_error {
        Outcome::Err {
            code: -1,
            message: extract_error_message(response),
        }
    } else if store_response {
        Outcome::Ok {
            result: response.clone(),
        }
    } else {
        Outcome::Ok {
            result: serde_json::Value::Null,
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn build_claude_event(
    session_id: Uuid,
    tool_name: String,
    arguments: serde_json::Value,
    outcome: Outcome,
    risk: Risk,
    project: crate::models::ProjectContext,
    tag: Option<String>,
    diff: Option<String>,
    tmeta: &crate::hook_helpers::TranscriptMeta,
    payload: &serde_json::Value,
    tool_use_id_str: Option<&str>,
) -> McpEvent {
    McpEvent {
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
        model: tmeta.model.clone(),
        input_tokens: tmeta.input_tokens,
        output_tokens: tmeta.output_tokens,
        cache_read_tokens: tmeta.cache_read_tokens,
        cache_write_tokens: tmeta.cache_write_tokens,
        stop_reason: tmeta.stop_reason.clone(),
        service_tier: tmeta.service_tier.clone(),
        permission_mode: payload["permission_mode"].as_str().map(|s| s.to_string()),
        tool_use_id: tool_use_id_str.map(|s| s.to_string()),
        ..Default::default()
    }
}

async fn handle_cursor_hook(payload: &serde_json::Value, ledger_path: &str) -> Result<()> {
    let hook_event = payload["hook_event_name"].as_str().unwrap_or("PostToolUse");
    if matches!(hook_event, "stop" | "beforeSubmitPrompt") {
        return Ok(());
    }

    let session_id = payload["conversation_id"]
        .as_str()
        .map(stable_uuid)
        .unwrap_or_else(Uuid::new_v4);
    let cwd = cursor_cwd(payload);
    let (tool_name, arguments, risk, diff) = parse_cursor_event(payload, hook_event);

    let git_dir = resolve_git_dir(&tool_name, &arguments, &cwd);
    let project = build_project(&git_dir).await;
    let tag = std::env::var("VIGILO_TAG")
        .ok()
        .or_else(|| project.branch.clone());

    let duration_us = payload["duration"]
        .as_f64()
        .map(|ms| (ms * 1000.0) as u64)
        .unwrap_or(0);
    let model = resolve_cursor_model(payload, payload["conversation_id"].as_str().unwrap_or(""));

    let event = build_cursor_event(
        session_id,
        tool_name,
        arguments,
        risk,
        project,
        tag,
        diff,
        duration_us,
        model,
        payload,
    );

    write_hook_event(&event, ledger_path);
    Ok(())
}

fn cursor_cwd(payload: &serde_json::Value) -> String {
    payload["cwd"]
        .as_str()
        .or_else(|| {
            payload["workspace_roots"]
                .as_array()
                .and_then(|a| a.first())
                .and_then(|v| v.as_str())
        })
        .unwrap_or(".")
        .to_string()
}

fn parse_cursor_event(
    payload: &serde_json::Value,
    hook_event: &str,
) -> (String, serde_json::Value, Risk, Option<String>) {
    match hook_event {
        "beforeShellExecution" => {
            let cmd = payload["command"].as_str().unwrap_or("").to_string();
            (
                String::from("Bash"),
                serde_json::json!({ "command": cmd }),
                Risk::Exec,
                None,
            )
        }
        "afterFileEdit" => parse_cursor_file_edit(payload),
        "beforeReadFile" => {
            let file_path = payload["file_path"].as_str().unwrap_or("");
            (
                String::from("Read"),
                serde_json::json!({ "file_path": file_path }),
                Risk::Read,
                None,
            )
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
        "PostToolUse" | "postToolUse" => parse_cursor_post_tool_use(payload),
        other => (other.to_string(), payload.clone(), Risk::Unknown, None),
    }
}

fn parse_cursor_file_edit(
    payload: &serde_json::Value,
) -> (String, serde_json::Value, Risk, Option<String>) {
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
            if let Some(d) = crate::models::compute_unified_diff(old, new) {
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

fn parse_cursor_post_tool_use(
    payload: &serde_json::Value,
) -> (String, serde_json::Value, Risk, Option<String>) {
    let raw_tool = payload["tool_name"].as_str().unwrap_or("unknown");
    let tool = raw_tool.strip_prefix("MCP:").unwrap_or(raw_tool);
    let canonical = match tool {
        "Shell" => "Bash",
        "Write" => "Edit",
        other => other,
    };
    let mut args = payload["tool_input"].clone();
    if args.is_null() {
        args = payload["arguments"].clone();
    }
    if let Some(obj) = args.as_object_mut() {
        if obj.contains_key("content") {
            obj.remove("content");
        }
    }
    let r = Risk::classify(canonical);
    (canonical.to_string(), args, r, None)
}

fn resolve_cursor_model(payload: &serde_json::Value, raw_conv_id: &str) -> Option<String> {
    payload["model"]
        .as_str()
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string())
        .or_else(|| read_cursor_model_from_db(raw_conv_id))
        .or_else(read_cursor_model_fallback)
        .map(|s| normalize_cursor_model(&s))
}

#[allow(clippy::too_many_arguments)]
fn build_cursor_event(
    session_id: Uuid,
    tool_name: String,
    arguments: serde_json::Value,
    risk: Risk,
    project: crate::models::ProjectContext,
    tag: Option<String>,
    diff: Option<String>,
    duration_us: u64,
    model: Option<String>,
    payload: &serde_json::Value,
) -> McpEvent {
    McpEvent {
        id: Uuid::new_v4(),
        timestamp: Utc::now().to_rfc3339(),
        session_id,
        server: "cursor".to_string(),
        tool: tool_name,
        arguments,
        duration_us,
        risk,
        project,
        tag,
        diff,
        model,
        tool_use_id: payload["tool_use_id"].as_str().map(|s| s.to_string()),
        cursor_version: payload["cursor_version"].as_str().map(|s| s.to_string()),
        generation_id: payload["generation_id"].as_str().map(|s| s.to_string()),
        ..Default::default()
    }
}

fn hook_store_response() -> bool {
    let config = server::load_config();
    let val = std::env::var("VIGILO_HOOK_STORE_RESPONSE")
        .ok()
        .or_else(|| config.get("HOOK_STORE_RESPONSE").cloned())
        .unwrap_or_default();
    matches!(val.to_lowercase().as_str(), "true" | "1" | "yes")
}

fn read_cursor_model_from_db(conversation_id: &str) -> Option<String> {
    let home = std::env::var("HOME").ok()?;
    let chats = std::path::Path::new(&home).join(".cursor/chats");

    for entry in std::fs::read_dir(&chats).ok()?.flatten() {
        let db = entry.path().join(conversation_id).join("store.db");
        if db.exists() {
            return extract_last_used_model_from_db(&db);
        }
    }
    None
}

const LAST_USED_MODEL_NEEDLE: &[u8] = b"226c617374557365644d6f64656c223a22";

fn extract_last_used_model_from_db(db_path: &std::path::Path) -> Option<String> {
    let data = std::fs::read(db_path).ok()?;

    let pos = data
        .windows(LAST_USED_MODEL_NEEDLE.len())
        .position(|w| w == LAST_USED_MODEL_NEEDLE)?;

    let after = &data[pos + LAST_USED_MODEL_NEEDLE.len()..];
    let end = after.windows(2).position(|w| w == b"22")?;
    let model_hex = &after[..end];

    if model_hex.len() % 2 != 0 {
        return None;
    }

    let model_bytes: Vec<u8> = model_hex
        .chunks(2)
        .filter_map(|c| u8::from_str_radix(std::str::from_utf8(c).ok()?, 16).ok())
        .collect();

    String::from_utf8(model_bytes)
        .ok()
        .filter(|s| !s.is_empty())
}

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

fn normalize_cursor_model(model: &str) -> String {
    match model {
        "default" | "auto" => "Auto".to_string(),
        _ => model.to_string(),
    }
}
