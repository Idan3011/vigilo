use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;
use uuid::Uuid;

/// Returns the user's home directory as a `PathBuf`.
pub fn home_dir() -> PathBuf {
    PathBuf::from(std::env::var("HOME").unwrap_or_else(|_| ".".into()))
}

/// Returns `~/.vigilo`.
pub fn vigilo_dir() -> PathBuf {
    home_dir().join(".vigilo")
}

/// Returns `~/.vigilo/<subpath>`.
pub fn vigilo_path(subpath: &str) -> PathBuf {
    vigilo_dir().join(subpath)
}

pub fn mcp_session_path() -> PathBuf {
    vigilo_path("mcp-session")
}

pub fn shorten_home(path: &str) -> String {
    let h = home_dir();
    let h_str = h.to_string_lossy();
    if !h_str.is_empty() && path.starts_with(h_str.as_ref()) {
        format!("~{}", &path[h_str.len()..])
    } else {
        path.to_string()
    }
}

pub fn load_config() -> HashMap<String, String> {
    let path = vigilo_path("config");
    let Ok(content) = std::fs::read_to_string(&path) else {
        return HashMap::new();
    };
    content
        .lines()
        .filter(|l| !l.trim_start().starts_with('#') && !l.trim().is_empty())
        .filter_map(|l| {
            let (k, v) = l.split_once('=')?;
            Some((k.trim().to_string(), v.trim().to_string()))
        })
        .collect()
}

#[derive(Serialize, Deserialize, Default)]
pub struct McpEvent {
    pub id: Uuid,
    pub timestamp: String,
    pub session_id: Uuid,
    pub server: String,
    pub tool: String,
    pub arguments: serde_json::Value,
    pub outcome: Outcome,
    #[serde(alias = "duration_ms")]
    pub duration_us: u64,
    pub risk: Risk,
    #[serde(default)]
    pub project: ProjectContext,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tag: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub diff: Option<String>,
    #[serde(default)]
    pub timed_out: bool,

    // Token/model metadata (flattened for backward-compatible JSONL)
    #[serde(default, flatten)]
    pub token_usage: TokenUsage,

    // Claude Code hook context (flattened)
    #[serde(default, flatten)]
    pub hook_context: HookContext,

    // Cursor-specific metadata (flattened)
    #[serde(default, flatten)]
    pub cursor_meta: CursorMeta,
}

/// Token and model metadata — populated by Claude Code hooks (via transcript)
/// and Cursor hooks. Flattened into McpEvent for backward-compatible JSONL.
#[derive(Serialize, Deserialize, Default, Clone)]
pub struct TokenUsage {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input_tokens: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output_tokens: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cache_read_tokens: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cache_write_tokens: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stop_reason: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub service_tier: Option<String>,
}

/// Claude Code hook-specific fields.
#[derive(Serialize, Deserialize, Default, Clone)]
pub struct HookContext {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub permission_mode: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_use_id: Option<String>,
}

/// Cursor-specific metadata.
#[derive(Serialize, Deserialize, Default, Clone)]
pub struct CursorMeta {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cursor_version: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub generation_id: Option<String>,
}

// Convenience accessors so view code can still use e.model, e.input_tokens, etc.
impl McpEvent {
    pub fn model(&self) -> Option<&str> {
        self.token_usage.model.as_deref()
    }
    pub fn input_tokens(&self) -> Option<u64> {
        self.token_usage.input_tokens
    }
    pub fn output_tokens(&self) -> Option<u64> {
        self.token_usage.output_tokens
    }
    pub fn cache_read_tokens(&self) -> Option<u64> {
        self.token_usage.cache_read_tokens
    }
    pub fn cache_write_tokens(&self) -> Option<u64> {
        self.token_usage.cache_write_tokens
    }
}

#[derive(Serialize, Deserialize, Clone, Default)]
pub struct ProjectContext {
    pub root: Option<String>,
    pub name: Option<String>,
    pub branch: Option<String>,
    pub commit: Option<String>,
    pub dirty: bool,
}

#[derive(Serialize, Deserialize, Clone)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum Outcome {
    Ok { result: serde_json::Value },
    Err { code: i32, message: String },
}

impl Default for Outcome {
    fn default() -> Self {
        Outcome::Ok {
            result: serde_json::Value::Null,
        }
    }
}

#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Default)]
#[serde(rename_all = "snake_case")]
pub enum Risk {
    Read,
    Write,
    Exec,
    #[default]
    Unknown,
}

/// Single source of truth: vigilo MCP tool name → risk level.
/// Both `is_vigilo_mcp_tool` and the vigilo branch of `Risk::classify` derive from this.
pub const VIGILO_TOOLS: &[(&str, Risk)] = &[
    ("read_file", Risk::Read),
    ("write_file", Risk::Write),
    ("list_directory", Risk::Read),
    ("create_directory", Risk::Write),
    ("delete_file", Risk::Write),
    ("move_file", Risk::Write),
    ("search_files", Risk::Read),
    ("run_command", Risk::Exec),
    ("get_file_info", Risk::Read),
    ("patch_file", Risk::Write),
    ("git_status", Risk::Read),
    ("git_diff", Risk::Read),
    ("git_log", Risk::Read),
    ("git_commit", Risk::Write),
];

impl Risk {
    pub fn classify(tool: &str) -> Self {
        let tool = tool.strip_prefix("MCP:").unwrap_or(tool);

        // Check vigilo MCP tools first (from single source of truth)
        if let Some((_, risk)) = VIGILO_TOOLS.iter().find(|(name, _)| *name == tool) {
            return *risk;
        }

        // Non-vigilo tools (Claude Code / Cursor builtins)
        match tool {
            "Bash" | "Shell" => Risk::Exec,

            "Read" | "Glob" | "Grep" | "WebFetch" | "WebSearch" | "Task" | "TaskCreate"
            | "TaskUpdate" | "TaskGet" | "TaskList" | "TaskOutput" | "EnterPlanMode"
            | "ExitPlanMode" | "AskUserQuestion" | "PostToolUse" | "postToolUse" => Risk::Read,

            "Write" | "Edit" | "MultiEdit" | "NotebookEdit" => Risk::Write,

            _ => Risk::Unknown,
        }
    }
}

pub fn is_vigilo_mcp_tool(name: &str) -> bool {
    VIGILO_TOOLS.iter().any(|(tool, _)| *tool == name)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn is_vigilo_mcp_tool_matches_all_14_tools() {
        let tools = [
            "read_file",
            "write_file",
            "list_directory",
            "create_directory",
            "delete_file",
            "move_file",
            "search_files",
            "run_command",
            "get_file_info",
            "patch_file",
            "git_status",
            "git_diff",
            "git_log",
            "git_commit",
        ];
        for tool in tools {
            assert!(is_vigilo_mcp_tool(tool), "{tool} should match");
        }
    }

    #[test]
    fn is_vigilo_mcp_tool_rejects_non_mcp_tools() {
        assert!(!is_vigilo_mcp_tool("Read"));
        assert!(!is_vigilo_mcp_tool("Bash"));
        assert!(!is_vigilo_mcp_tool("Edit"));
        assert!(!is_vigilo_mcp_tool("unknown"));
        assert!(!is_vigilo_mcp_tool(""));
    }

    #[test]
    fn shorten_home_replaces_prefix() {
        let h = home_dir();
        let path = format!("{}/projects/vigilo", h.display());
        let short = shorten_home(&path);
        assert!(short.starts_with("~/"));
        assert!(short.ends_with("/projects/vigilo"));
    }

    #[test]
    fn shorten_home_leaves_unrelated_paths() {
        assert_eq!(shorten_home("/tmp/foo"), "/tmp/foo");
    }

    #[test]
    fn risk_classify_strips_mcp_prefix() {
        assert_eq!(Risk::classify("MCP:git_status"), Risk::Read);
        assert_eq!(Risk::classify("MCP:run_command"), Risk::Exec);
        assert_eq!(Risk::classify("MCP:write_file"), Risk::Write);
    }

    #[test]
    fn risk_classify_unknown_tool() {
        assert_eq!(Risk::classify("nonexistent_tool"), Risk::Unknown);
    }

    #[test]
    fn mcp_session_path_contains_vigilo() {
        let path = mcp_session_path();
        let path_str = path.to_string_lossy();
        assert!(path_str.contains(".vigilo"));
        assert!(path_str.ends_with("mcp-session"));
    }

    #[test]
    fn load_config_returns_empty_for_missing_file() {
        // HOME is set to something real, but .vigilo/config may not exist in test env
        // This tests that the function doesn't panic
        let _config = load_config();
    }

    #[test]
    fn mcp_event_default_has_zero_values() {
        let e = McpEvent::default();
        assert_eq!(e.duration_us, 0);
        assert_eq!(e.risk, Risk::Unknown);
        assert!(e.tag.is_none());
        assert!(e.model().is_none());
    }

    #[test]
    fn mcp_event_serializes_and_deserializes() {
        let e = McpEvent {
            tool: "read_file".to_string(),
            risk: Risk::Read,
            ..Default::default()
        };
        let json = serde_json::to_string(&e).unwrap();
        let parsed: McpEvent = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.tool, "read_file");
        assert_eq!(parsed.risk, Risk::Read);
    }

    #[test]
    fn mcp_event_flatten_backward_compatible() {
        // Simulate reading a legacy flat JSON event (pre-refactor format)
        let legacy_json = serde_json::json!({
            "id": "00000000-0000-0000-0000-000000000000",
            "timestamp": "2026-02-18T12:00:00Z",
            "session_id": "00000000-0000-0000-0000-000000000000",
            "server": "claude-code",
            "tool": "Read",
            "arguments": {},
            "outcome": { "status": "ok", "result": null },
            "duration_us": 100,
            "risk": "read",
            "project": { "dirty": false },
            "model": "claude-opus-4-6",
            "input_tokens": 1000,
            "output_tokens": 500,
            "cache_read_tokens": 200,
            "permission_mode": "auto",
            "tool_use_id": "tu_123",
            "cursor_version": "0.45.0",
            "generation_id": "gen_abc"
        });
        let parsed: McpEvent = serde_json::from_value(legacy_json).unwrap();
        assert_eq!(parsed.model(), Some("claude-opus-4-6"));
        assert_eq!(parsed.input_tokens(), Some(1000));
        assert_eq!(parsed.output_tokens(), Some(500));
        assert_eq!(parsed.cache_read_tokens(), Some(200));
        assert_eq!(parsed.hook_context.permission_mode.as_deref(), Some("auto"));
        assert_eq!(parsed.hook_context.tool_use_id.as_deref(), Some("tu_123"));
        assert_eq!(parsed.cursor_meta.cursor_version.as_deref(), Some("0.45.0"));
        assert_eq!(parsed.cursor_meta.generation_id.as_deref(), Some("gen_abc"));

        // Verify re-serialization produces the same flat format
        let reserialized = serde_json::to_value(&parsed).unwrap();
        assert_eq!(reserialized["model"], "claude-opus-4-6");
        assert_eq!(reserialized["input_tokens"], 1000);
        assert_eq!(reserialized["permission_mode"], "auto");
        assert_eq!(reserialized["cursor_version"], "0.45.0");
        // Must NOT produce nested objects
        assert!(reserialized.get("token_usage").is_none());
        assert!(reserialized.get("hook_context").is_none());
        assert!(reserialized.get("cursor_meta").is_none());
    }
}
