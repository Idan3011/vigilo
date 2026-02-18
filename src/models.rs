use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Serialize, Deserialize)]
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    // ── Token usage (Claude Code: from transcript; Cursor: not exposed in hooks) ──
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input_tokens: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output_tokens: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cache_read_tokens: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cache_write_tokens: Option<u64>,
    // ── Per-request metadata ────────────────────────────────────────────────────
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stop_reason: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub service_tier: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub permission_mode: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_use_id: Option<String>,
    // ── Client-specific metadata ────────────────────────────────────────────────
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cursor_version: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub generation_id: Option<String>,
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

#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum Risk {
    Read,
    Write,
    Exec,
    Unknown,
}

impl Risk {
    /// Classify a tool name into its risk level.
    ///
    /// Covers vigilo native tools, Claude Code canonical names, and
    /// Cursor normalized names — one authoritative mapping for the whole codebase.
    pub fn classify(tool: &str) -> Self {
        // Strip MCP proxy prefix (e.g. "MCP:list_directory" → "list_directory")
        let tool = tool.strip_prefix("MCP:").unwrap_or(tool);

        match tool {
            "Bash" | "Shell" | "run_command" => Risk::Exec,

            "Read" | "Glob" | "Grep" | "WebFetch" | "WebSearch"
            | "read_file" | "list_directory" | "search_files" | "get_file_info"
            | "git_status" | "git_diff" | "git_log"
            // Claude Code internal tools (task management, hooks, agents, planning)
            | "Task" | "TaskCreate" | "TaskUpdate" | "TaskGet" | "TaskList" | "TaskOutput"
            | "EnterPlanMode" | "ExitPlanMode" | "AskUserQuestion"
            | "PostToolUse" | "postToolUse" => Risk::Read,

            "Write" | "Edit" | "MultiEdit" | "NotebookEdit"
            | "write_file" | "create_directory" | "delete_file" | "move_file"
            | "patch_file" | "git_commit" => Risk::Write,

            _ => Risk::Unknown,
        }
    }
}

/// Compute a unified diff between two strings using the `similar` crate.
/// Returns `None` when the texts are identical. Truncates at 10,000 chars.
pub fn compute_unified_diff(old: &str, new: &str) -> Option<String> {
    use similar::{ChangeTag, TextDiff};

    let diff = TextDiff::from_lines(old, new);
    let mut out = String::new();
    for group in diff.grouped_ops(3) {
        for op in &group {
            for change in diff.iter_changes(op) {
                let prefix = match change.tag() {
                    ChangeTag::Delete => "-",
                    ChangeTag::Insert => "+",
                    ChangeTag::Equal => " ",
                };
                out.push_str(prefix);
                out.push_str(change.value());
            }
        }
        out.push('\n');
    }
    if out.len() > 10_000 {
        out.truncate(10_000);
        out.push_str("... (truncated)\n");
    }
    if out.trim().is_empty() {
        None
    } else {
        Some(out)
    }
}

/// Format a microsecond duration for human-readable display.
pub fn fmt_duration(us: u64) -> String {
    match us {
        us if us < 1_000 => format!("{us}µs"),
        us if us < 1_000_000 => format!("{:.1}ms", us as f64 / 1_000.0),
        us => format!("{:.1}s", us as f64 / 1_000_000.0),
    }
}
