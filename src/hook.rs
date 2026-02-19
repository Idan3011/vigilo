use crate::{
    hook_helpers::{
        build_project, compute_edit_diff, extract_error_message, read_transcript_meta,
        resolve_git_dir, stable_uuid, write_hook_event,
    },
    models::{McpEvent, Outcome, Risk},
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
        token_usage: crate::models::TokenUsage {
            model: tmeta.model.clone(),
            input_tokens: tmeta.input_tokens,
            output_tokens: tmeta.output_tokens,
            cache_read_tokens: tmeta.cache_read_tokens,
            cache_write_tokens: tmeta.cache_write_tokens,
            stop_reason: tmeta.stop_reason.clone(),
            service_tier: tmeta.service_tier.clone(),
        },
        hook_context: crate::models::HookContext {
            permission_mode: payload["permission_mode"].as_str().map(|s| s.to_string()),
            tool_use_id: tool_use_id_str.map(|s| s.to_string()),
        },
        ..Default::default()
    };

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
    if let Some(id) = read_mcp_session_id() {
        return id;
    }
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

async fn handle_cursor_hook(payload: &serde_json::Value, ledger_path: &str) -> Result<()> {
    let hook_event = payload["hook_event_name"].as_str().unwrap_or("PostToolUse");
    if matches!(hook_event, "stop" | "beforeSubmitPrompt") {
        return Ok(());
    }

    let session_id = read_mcp_session_id().unwrap_or_else(|| {
        payload["conversation_id"]
            .as_str()
            .map(stable_uuid)
            .unwrap_or_else(Uuid::new_v4)
    });
    let cwd = cursor_cwd(payload);
    let (tool_name, arguments, risk, diff) = parse_cursor_event(payload, hook_event);

    if crate::models::is_vigilo_mcp_tool(&tool_name) {
        return Ok(());
    }

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

    let event = McpEvent {
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
        token_usage: crate::models::TokenUsage {
            model,
            ..Default::default()
        },
        hook_context: crate::models::HookContext {
            tool_use_id: payload["tool_use_id"].as_str().map(|s| s.to_string()),
            ..Default::default()
        },
        cursor_meta: crate::models::CursorMeta {
            cursor_version: payload["cursor_version"].as_str().map(|s| s.to_string()),
            generation_id: payload["generation_id"].as_str().map(|s| s.to_string()),
        },
        ..Default::default()
    };

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
            if let Some(d) = crate::hook_helpers::compute_unified_diff(old, new) {
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

fn read_mcp_session_id() -> Option<Uuid> {
    let content = std::fs::read_to_string(crate::models::mcp_session_path()).ok()?;
    let mut lines = content.lines();
    let uuid_str = lines.next()?;
    let pid: u32 = lines.next()?.parse().ok()?;
    if !is_process_alive(pid) {
        return None;
    }
    Uuid::parse_str(uuid_str).ok()
}

fn is_process_alive(pid: u32) -> bool {
    if std::path::Path::new(&format!("/proc/{pid}")).exists() {
        return true;
    }
    std::process::Command::new("kill")
        .args(["-0", &pid.to_string()])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

fn hook_store_response() -> bool {
    let config = crate::models::load_config();
    let val = std::env::var("VIGILO_HOOK_STORE_RESPONSE")
        .ok()
        .or_else(|| config.get("HOOK_STORE_RESPONSE").cloned())
        .unwrap_or_default();
    matches!(val.to_lowercase().as_str(), "true" | "1" | "yes")
}

fn read_cursor_model_from_db(conversation_id: &str) -> Option<String> {
    let home = crate::models::home();
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
    let home = crate::models::home();
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detect_client_cursor_has_conversation_id() {
        let payload = serde_json::json!({ "conversation_id": "abc-123" });
        assert!(matches!(detect_client(&payload), HookClient::Cursor));
    }

    #[test]
    fn detect_client_claude_code_without_conversation_id() {
        let payload = serde_json::json!({ "tool_name": "Read" });
        assert!(matches!(detect_client(&payload), HookClient::ClaudeCode));
    }

    #[test]
    fn parse_claude_tool_extracts_name_and_args() {
        let payload = serde_json::json!({
            "tool_name": "Read",
            "tool_input": { "file_path": "src/foo.rs" }
        });
        let (name, args) = parse_claude_tool(&payload);
        assert_eq!(name, "Read");
        assert_eq!(args["file_path"], "src/foo.rs");
    }

    #[test]
    fn parse_claude_tool_strips_content_from_write() {
        let payload = serde_json::json!({
            "tool_name": "Write",
            "tool_input": { "file_path": "src/foo.rs", "content": "big blob" }
        });
        let (name, args) = parse_claude_tool(&payload);
        assert_eq!(name, "Write");
        assert!(args.get("content").is_none());
        assert_eq!(args["file_path"], "src/foo.rs");
    }

    #[test]
    fn parse_claude_tool_strips_content_from_write_file() {
        let payload = serde_json::json!({
            "tool_name": "write_file",
            "tool_input": { "file_path": "src/foo.rs", "content": "big blob" }
        });
        let (_, args) = parse_claude_tool(&payload);
        assert!(args.get("content").is_none());
    }

    #[test]
    fn claude_session_id_from_transcript_path() {
        let payload = serde_json::json!({ "transcript_path": "transcripts/session.jsonl" });
        let id1 = claude_session_id(&payload);
        let id2 = claude_session_id(&payload);
        assert_eq!(id1, id2);
    }

    #[test]
    fn claude_session_id_from_session_id_field() {
        let payload = serde_json::json!({ "session_id": "my-session" });
        let id = claude_session_id(&payload);
        assert_ne!(id, Uuid::nil());
    }

    #[test]
    fn build_claude_outcome_ok() {
        let response = serde_json::json!({ "content": [{ "text": "hello" }] });
        let outcome = build_claude_outcome(&response);
        assert!(matches!(outcome, Outcome::Ok { .. }));
    }

    #[test]
    fn build_claude_outcome_error_via_is_error() {
        let response = serde_json::json!({ "is_error": true, "content": [{ "text": "fail" }] });
        let outcome = build_claude_outcome(&response);
        assert!(matches!(outcome, Outcome::Err { .. }));
    }

    #[test]
    fn build_claude_outcome_error_via_success_false() {
        let response = serde_json::json!({ "success": false, "error": "bad" });
        let outcome = build_claude_outcome(&response);
        assert!(matches!(outcome, Outcome::Err { .. }));
    }

    #[test]
    fn cursor_cwd_from_cwd_field() {
        let payload = serde_json::json!({ "cwd": "workspace/my-project" });
        assert_eq!(cursor_cwd(&payload), "workspace/my-project");
    }

    #[test]
    fn cursor_cwd_from_workspace_roots() {
        let payload = serde_json::json!({ "workspace_roots": ["workspace/other"] });
        assert_eq!(cursor_cwd(&payload), "workspace/other");
    }

    #[test]
    fn cursor_cwd_fallback_to_dot() {
        let payload = serde_json::json!({});
        assert_eq!(cursor_cwd(&payload), ".");
    }

    #[test]
    fn parse_cursor_event_shell_execution() {
        let payload = serde_json::json!({ "command": "ls -la" });
        let (tool, args, risk, diff) = parse_cursor_event(&payload, "beforeShellExecution");
        assert_eq!(tool, "Bash");
        assert_eq!(args["command"], "ls -la");
        assert_eq!(risk, Risk::Exec);
        assert!(diff.is_none());
    }

    #[test]
    fn parse_cursor_event_read_file() {
        let payload = serde_json::json!({ "file_path": "src/main.rs" });
        let (tool, args, risk, _) = parse_cursor_event(&payload, "beforeReadFile");
        assert_eq!(tool, "Read");
        assert_eq!(args["file_path"], "src/main.rs");
        assert_eq!(risk, Risk::Read);
    }

    #[test]
    fn parse_cursor_event_mcp_execution() {
        let payload = serde_json::json!({
            "tool_name": "git_status",
            "tool_input": { "path": "my-repo" }
        });
        let (tool, args, risk, _) = parse_cursor_event(&payload, "beforeMCPExecution");
        assert_eq!(tool, "git_status");
        assert_eq!(args["path"], "my-repo");
        assert_eq!(risk, Risk::Read);
    }

    #[test]
    fn parse_cursor_post_tool_use_strips_mcp_prefix() {
        let payload = serde_json::json!({
            "tool_name": "MCP:git_status",
            "tool_input": { "path": "my-repo" }
        });
        let (tool, args, risk, _) = parse_cursor_post_tool_use(&payload);
        assert_eq!(tool, "git_status");
        assert_eq!(args["path"], "my-repo");
        assert_eq!(risk, Risk::Read);
    }

    #[test]
    fn parse_cursor_post_tool_use_canonicalizes_shell() {
        let payload = serde_json::json!({
            "tool_name": "Shell",
            "tool_input": { "command": "echo hi" }
        });
        let (tool, _, risk, _) = parse_cursor_post_tool_use(&payload);
        assert_eq!(tool, "Bash");
        assert_eq!(risk, Risk::Exec);
    }

    #[test]
    fn parse_cursor_post_tool_use_strips_content() {
        let payload = serde_json::json!({
            "tool_name": "Write",
            "tool_input": { "file_path": "src/lib.rs", "content": "big" }
        });
        let (tool, args, _, _) = parse_cursor_post_tool_use(&payload);
        assert_eq!(tool, "Edit");
        assert!(args.get("content").is_none());
    }

    #[test]
    fn parse_cursor_post_tool_use_falls_back_to_arguments() {
        let payload = serde_json::json!({
            "tool_name": "Read",
            "arguments": { "file_path": "src/lib.rs" }
        });
        let (_, args, _, _) = parse_cursor_post_tool_use(&payload);
        assert_eq!(args["file_path"], "src/lib.rs");
    }

    #[test]
    fn parse_cursor_file_edit_with_diff() {
        let payload = serde_json::json!({
            "file_path": "/src/lib.rs",
            "edits": [
                { "old_string": "hello", "new_string": "world" }
            ]
        });
        let (tool, args, risk, diff) = parse_cursor_file_edit(&payload);
        assert_eq!(tool, "Edit");
        assert_eq!(args["file_path"], "/src/lib.rs");
        assert_eq!(risk, Risk::Write);
        assert!(diff.is_some());
        let d = diff.unwrap();
        assert!(d.contains("-hello"));
        assert!(d.contains("+world"));
    }

    #[test]
    fn parse_cursor_event_unknown_passthrough() {
        let payload = serde_json::json!({ "foo": "bar" });
        let (tool, _, risk, _) = parse_cursor_event(&payload, "customEvent");
        assert_eq!(tool, "customEvent");
        assert_eq!(risk, Risk::Unknown);
    }

    #[test]
    fn normalize_cursor_model_default_becomes_auto() {
        assert_eq!(normalize_cursor_model("default"), "Auto");
        assert_eq!(normalize_cursor_model("auto"), "Auto");
    }

    #[test]
    fn normalize_cursor_model_passes_through() {
        assert_eq!(
            normalize_cursor_model("claude-3.5-sonnet"),
            "claude-3.5-sonnet"
        );
    }

    #[test]
    fn parse_cursor_file_edit_no_edits() {
        let payload = serde_json::json!({ "file_path": "/src/lib.rs" });
        let (tool, args, risk, diff) = parse_cursor_file_edit(&payload);
        assert_eq!(tool, "Edit");
        assert_eq!(args["file_path"], "/src/lib.rs");
        assert_eq!(risk, Risk::Write);
        assert!(diff.is_none());
    }

    #[test]
    fn parse_cursor_file_edit_empty_edits_array() {
        let payload = serde_json::json!({
            "file_path": "/src/lib.rs",
            "edits": []
        });
        let (_, _, _, diff) = parse_cursor_file_edit(&payload);
        assert!(diff.is_none());
    }

    #[test]
    fn parse_cursor_file_edit_multiple_edits() {
        let payload = serde_json::json!({
            "file_path": "/src/lib.rs",
            "edits": [
                { "old_string": "foo", "new_string": "bar" },
                { "old_string": "hello", "new_string": "world" }
            ]
        });
        let (_, _, _, diff) = parse_cursor_file_edit(&payload);
        let d = diff.unwrap();
        assert!(d.contains("-foo"));
        assert!(d.contains("+bar"));
        assert!(d.contains("-hello"));
        assert!(d.contains("+world"));
    }

    #[test]
    fn is_process_alive_for_current_process() {
        let pid = std::process::id();
        assert!(is_process_alive(pid));
    }

    #[test]
    fn is_process_alive_bogus_pid() {
        // PID 0 is kernel, large PIDs shouldn't exist
        assert!(!is_process_alive(4_000_000));
    }

    #[test]
    fn read_mcp_session_id_from_tempfile() {
        let dir = tempfile::tempdir().unwrap();
        let session_file = dir.path().join("mcp-session");
        let uuid = uuid::Uuid::new_v4();
        let pid = std::process::id();
        std::fs::write(&session_file, format!("{uuid}\n{pid}")).unwrap();

        // read_mcp_session_id uses a fixed path, so we test the parsing logic directly
        let content = std::fs::read_to_string(&session_file).unwrap();
        let mut lines = content.lines();
        let uuid_str = lines.next().unwrap();
        let parsed_pid: u32 = lines.next().unwrap().parse().unwrap();
        assert!(is_process_alive(parsed_pid));
        let parsed = Uuid::parse_str(uuid_str).unwrap();
        assert_eq!(parsed, uuid);
    }

    #[test]
    fn read_mcp_session_id_stale_pid() {
        // Simulate a stale session file with a dead PID
        let dir = tempfile::tempdir().unwrap();
        let session_file = dir.path().join("mcp-session");
        let uuid = uuid::Uuid::new_v4();
        std::fs::write(&session_file, format!("{uuid}\n4000000")).unwrap();

        let content = std::fs::read_to_string(&session_file).unwrap();
        let mut lines = content.lines();
        let _uuid_str = lines.next().unwrap();
        let pid: u32 = lines.next().unwrap().parse().unwrap();
        // Dead PID should not be alive
        assert!(!is_process_alive(pid));
    }

    #[test]
    fn build_claude_outcome_ok_stores_null_by_default() {
        // hook_store_response() defaults to false, so result should be Null
        let response = serde_json::json!({ "content": [{ "text": "hello" }] });
        let outcome = build_claude_outcome(&response);
        match outcome {
            Outcome::Ok { result } => assert!(result.is_null()),
            _ => panic!("expected Ok"),
        }
    }

    #[test]
    fn build_claude_outcome_error_extracts_message() {
        let response = serde_json::json!({
            "is_error": true,
            "content": [{ "text": "something broke" }]
        });
        let outcome = build_claude_outcome(&response);
        match outcome {
            Outcome::Err { code, message } => {
                assert_eq!(code, -1);
                assert!(message.contains("something broke"));
            }
            _ => panic!("expected Err"),
        }
    }

    #[test]
    fn parse_cursor_post_tool_use_write_becomes_edit() {
        let payload = serde_json::json!({
            "tool_name": "Write",
            "tool_input": { "file_path": "a.rs" }
        });
        let (tool, _, risk, _) = parse_cursor_post_tool_use(&payload);
        assert_eq!(tool, "Edit");
        assert_eq!(risk, Risk::Write);
    }
}
