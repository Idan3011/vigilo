use crate::{
    git, ledger,
    models::{McpEvent, ProjectContext},
};
use uuid::Uuid;

const SESSION_NAMESPACE: Uuid = Uuid::from_bytes([
    0xa1, 0xb2, 0xc3, 0xd4, 0xe5, 0xf6, 0x47, 0x08, 0x89, 0xab, 0xcd, 0xef, 0x01, 0x23, 0x45, 0x67,
]);

pub fn stable_uuid(s: &str) -> Uuid {
    Uuid::new_v5(&SESSION_NAMESPACE, s.as_bytes())
}

pub fn resolve_git_dir(tool: &str, args: &serde_json::Value, cwd: &str) -> String {
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

const MAX_DIFF_BYTES: usize = 10_000;
const TRANSCRIPT_USAGE_TAIL: u64 = 64 * 1024;
const TRANSCRIPT_DURATION_TAIL: u64 = 512 * 1024;

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
    if out.len() > MAX_DIFF_BYTES {
        out.truncate(MAX_DIFF_BYTES);
        out.push_str("... (truncated)\n");
    }
    if out.trim().is_empty() {
        None
    } else {
        Some(out)
    }
}

pub fn compute_edit_diff(tool: &str, args: &serde_json::Value) -> Option<String> {
    if tool != "Edit" && tool != "MultiEdit" {
        return None;
    }
    let old = args.get("old_string").and_then(|v| v.as_str())?;
    let new = args.get("new_string").and_then(|v| v.as_str())?;
    compute_unified_diff(old, new)
}

pub fn extract_error_message(response: &serde_json::Value) -> String {
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

pub async fn build_project(git_dir: &str) -> ProjectContext {
    let (root, name, branch, commit, dirty) = tokio::join!(
        git::root_in(git_dir),
        git::name_in(Some(git_dir)),
        git::branch_in(git_dir),
        git::commit_in(git_dir),
        git::dirty_in(git_dir),
    );
    ProjectContext {
        root,
        name,
        branch,
        commit,
        dirty,
    }
}

pub fn write_hook_event(event: &McpEvent, ledger_path: &str) {
    if let Err(e) = ledger::append_event(event, ledger_path) {
        eprintln!("[vigilo hook] ledger error: {e}");
    }
}

#[derive(Default)]
pub struct TranscriptMeta {
    pub model: Option<String>,
    pub input_tokens: Option<u64>,
    pub output_tokens: Option<u64>,
    pub cache_read_tokens: Option<u64>,
    pub cache_write_tokens: Option<u64>,
    pub stop_reason: Option<String>,
    pub service_tier: Option<String>,
    pub duration_us: Option<u64>,
}

pub fn read_transcript_meta(transcript_path: &str, tool_use_id: Option<&str>) -> TranscriptMeta {
    let Ok(mut file) = std::fs::File::open(transcript_path) else {
        return TranscriptMeta::default();
    };
    let size = file.metadata().map(|m| m.len()).unwrap_or(0);

    let meta = scan_transcript_usage(&mut file, size);

    if let Some(id) = tool_use_id {
        let duration = compute_tool_duration(&mut file, size, id);
        return TranscriptMeta {
            duration_us: duration,
            ..meta
        };
    }

    meta
}

/// Check that the first parseable line has the expected transcript structure.
/// Returns false (with a stderr warning) if the format looks foreign.
fn check_transcript_format(file: &mut std::fs::File, size: u64) -> bool {
    use std::io::{BufRead, Seek, SeekFrom};

    let _ = file.seek(SeekFrom::Start(0));
    let reader = std::io::BufReader::new(&mut *file);

    for line in reader.lines().take(10).map_while(Result::ok) {
        let Ok(v) = serde_json::from_str::<serde_json::Value>(&line) else {
            continue;
        };
        // A valid Claude Code transcript line has "type" and "message" fields.
        if v.get("type").and_then(|t| t.as_str()).is_some() && v.get("message").is_some() {
            return true;
        }
        // First parseable JSON line doesn't match expected shape.
        eprintln!(
            "[vigilo] warning: transcript format may have changed (size={size}, \
             keys: {:?}) — token/duration data may be missing",
            v.as_object().map(|o| o.keys().collect::<Vec<_>>())
        );
        return false;
    }
    // No parseable JSON lines at all — could be empty or binary.
    false
}

fn scan_transcript_usage(file: &mut std::fs::File, size: u64) -> TranscriptMeta {
    use std::io::{BufRead, Seek, SeekFrom};

    if !check_transcript_format(file, size) {
        return TranscriptMeta::default();
    }

    let tail_start = size.saturating_sub(TRANSCRIPT_USAGE_TAIL);
    let _ = file.seek(SeekFrom::Start(tail_start));
    let mut reader = std::io::BufReader::new(&mut *file);
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
    meta
}

fn compute_tool_duration(file: &mut std::fs::File, size: u64, id: &str) -> Option<u64> {
    use std::io::{BufRead, Seek, SeekFrom};

    let id_bytes = id.as_bytes();
    let read_from = size.saturating_sub(TRANSCRIPT_DURATION_TAIL);
    file.seek(SeekFrom::Start(read_from)).ok()?;
    let mut reader = std::io::BufReader::new(&mut *file);
    if read_from > 0 {
        let mut skip = String::new();
        let _ = reader.read_line(&mut skip);
    }

    let mut invoke_ts: Option<i64> = None;
    let mut result_ts: Option<i64> = None;

    for line in reader.lines().map_while(Result::ok) {
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
        let ts = parse_timestamp_micros(&v)?;
        match v["type"].as_str() {
            Some("assistant") => {
                if has_tool_use_id(&v["message"]["content"], id) {
                    invoke_ts = Some(ts);
                }
            }
            Some("user") => {
                if has_tool_result_id(&v["message"]["content"], id) {
                    result_ts = Some(ts);
                }
            }
            _ => {}
        }
    }

    let diff_us = result_ts? - invoke_ts?;
    if diff_us > 0 {
        Some(diff_us as u64)
    } else {
        None
    }
}

fn parse_timestamp_micros(v: &serde_json::Value) -> Option<i64> {
    v["timestamp"]
        .as_str()
        .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
        .map(|dt| dt.timestamp_micros())
}

fn has_tool_use_id(content: &serde_json::Value, id: &str) -> bool {
    content
        .as_array()
        .map(|arr| {
            arr.iter()
                .any(|item| item["type"] == "tool_use" && item["id"].as_str() == Some(id))
        })
        .unwrap_or(false)
}

fn has_tool_result_id(content: &serde_json::Value, id: &str) -> bool {
    content
        .as_array()
        .map(|arr| {
            arr.iter().any(|item| {
                item["type"] == "tool_result" && item["tool_use_id"].as_str() == Some(id)
            })
        })
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stable_uuid_is_deterministic() {
        let a = stable_uuid("same-input");
        let b = stable_uuid("same-input");
        assert_eq!(a, b);
    }

    #[test]
    fn stable_uuid_differs_for_different_input() {
        let a = stable_uuid("input-a");
        let b = stable_uuid("input-b");
        assert_ne!(a, b);
    }

    #[test]
    fn resolve_git_dir_from_file_path_for_read() {
        let tmp = tempfile::tempdir().unwrap();
        let file = tmp.path().join("main.rs");
        std::fs::write(&file, "").unwrap();
        let file_str = file.to_str().unwrap();
        let args = serde_json::json!({ "file_path": file_str });
        let dir = resolve_git_dir("Read", &args, "/fallback");
        assert_eq!(dir, tmp.path().to_str().unwrap());
    }

    #[test]
    fn resolve_git_dir_from_path_for_grep() {
        let tmp = tempfile::tempdir().unwrap();
        let dir_path = tmp.path().to_str().unwrap();
        let args = serde_json::json!({ "path": dir_path });
        let dir = resolve_git_dir("Grep", &args, "/fallback");
        assert_eq!(dir, dir_path);
    }

    #[test]
    fn resolve_git_dir_falls_back_to_cwd() {
        let tmp = tempfile::tempdir().unwrap();
        let cwd = tmp.path().to_str().unwrap();
        let args = serde_json::json!({});
        let dir = resolve_git_dir("Bash", &args, cwd);
        assert_eq!(dir, cwd);
    }

    #[test]
    fn resolve_git_dir_generic_tool_checks_file_path_first() {
        let tmp = tempfile::tempdir().unwrap();
        let sub = tmp.path().join("sub");
        std::fs::create_dir(&sub).unwrap();
        let file = sub.join("c.txt");
        std::fs::write(&file, "").unwrap();
        let file_str = file.to_str().unwrap();
        let args = serde_json::json!({ "file_path": file_str, "path": "/x/y" });
        let dir = resolve_git_dir("SomeTool", &args, "/fallback");
        assert_eq!(dir, sub.to_str().unwrap());
    }

    #[test]
    fn compute_edit_diff_returns_none_for_non_edit() {
        let args = serde_json::json!({ "old_string": "a", "new_string": "b" });
        assert!(compute_edit_diff("Read", &args).is_none());
        assert!(compute_edit_diff("Bash", &args).is_none());
    }

    #[test]
    fn compute_edit_diff_returns_diff_for_edit() {
        let args = serde_json::json!({ "old_string": "hello\n", "new_string": "world\n" });
        let diff = compute_edit_diff("Edit", &args);
        assert!(diff.is_some());
        let d = diff.unwrap();
        assert!(d.contains("-hello"));
        assert!(d.contains("+world"));
    }

    #[test]
    fn compute_edit_diff_works_for_multi_edit() {
        let args = serde_json::json!({ "old_string": "a\n", "new_string": "b\n" });
        assert!(compute_edit_diff("MultiEdit", &args).is_some());
    }

    #[test]
    fn extract_error_message_from_content_text() {
        let response = serde_json::json!({
            "content": [{ "text": "file not found" }]
        });
        assert_eq!(extract_error_message(&response), "file not found");
    }

    #[test]
    fn extract_error_message_from_error_field() {
        let response = serde_json::json!({ "error": "permission denied" });
        assert_eq!(extract_error_message(&response), "permission denied");
    }

    #[test]
    fn extract_error_message_fallback() {
        let response = serde_json::json!({});
        assert_eq!(extract_error_message(&response), "error");
    }

    #[test]
    fn has_tool_use_id_finds_match() {
        let content = serde_json::json!([
            { "type": "tool_use", "id": "tu_123", "name": "Read" }
        ]);
        assert!(has_tool_use_id(&content, "tu_123"));
    }

    #[test]
    fn has_tool_use_id_no_match() {
        let content = serde_json::json!([
            { "type": "tool_use", "id": "tu_999", "name": "Read" }
        ]);
        assert!(!has_tool_use_id(&content, "tu_123"));
    }

    #[test]
    fn has_tool_use_id_non_array_returns_false() {
        let content = serde_json::json!("not an array");
        assert!(!has_tool_use_id(&content, "tu_123"));
    }

    #[test]
    fn has_tool_result_id_finds_match() {
        let content = serde_json::json!([
            { "type": "tool_result", "tool_use_id": "tu_123", "content": "ok" }
        ]);
        assert!(has_tool_result_id(&content, "tu_123"));
    }

    #[test]
    fn has_tool_result_id_no_match() {
        let content = serde_json::json!([
            { "type": "tool_result", "tool_use_id": "tu_999" }
        ]);
        assert!(!has_tool_result_id(&content, "tu_123"));
    }

    #[test]
    fn parse_timestamp_micros_valid() {
        let v = serde_json::json!({ "timestamp": "2026-02-18T12:00:00Z" });
        let us = parse_timestamp_micros(&v);
        assert!(us.is_some());
        assert!(us.unwrap() > 0);
    }

    #[test]
    fn parse_timestamp_micros_invalid() {
        let v = serde_json::json!({ "timestamp": "not-a-date" });
        assert!(parse_timestamp_micros(&v).is_none());
    }

    #[test]
    fn parse_timestamp_micros_missing() {
        let v = serde_json::json!({});
        assert!(parse_timestamp_micros(&v).is_none());
    }

    #[test]
    fn check_transcript_format_accepts_valid() {
        use std::io::Write;
        let mut tmp = tempfile::NamedTempFile::new().unwrap();
        let line = serde_json::json!({
            "type": "assistant",
            "message": { "model": "test" }
        });
        writeln!(tmp, "{}", serde_json::to_string(&line).unwrap()).unwrap();
        tmp.flush().unwrap();

        let mut file = std::fs::File::open(tmp.path()).unwrap();
        let size = file.metadata().unwrap().len();
        assert!(check_transcript_format(&mut file, size));
    }

    #[test]
    fn check_transcript_format_rejects_foreign() {
        use std::io::Write;
        let mut tmp = tempfile::NamedTempFile::new().unwrap();
        writeln!(tmp, r#"{{"event":"something","data":"foreign"}}"#).unwrap();
        tmp.flush().unwrap();

        let mut file = std::fs::File::open(tmp.path()).unwrap();
        let size = file.metadata().unwrap().len();
        assert!(!check_transcript_format(&mut file, size));
    }

    #[test]
    fn check_transcript_format_rejects_empty() {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        let mut file = std::fs::File::open(tmp.path()).unwrap();
        assert!(!check_transcript_format(&mut file, 0));
    }

    #[test]
    fn scan_transcript_usage_extracts_model_and_tokens() {
        use std::io::Write;
        let mut tmp = tempfile::NamedTempFile::new().unwrap();
        let line = serde_json::json!({
            "type": "assistant",
            "message": {
                "model": "claude-sonnet-4-20250514",
                "stop_reason": "end_turn",
                "usage": {
                    "input_tokens": 1000,
                    "output_tokens": 500,
                    "cache_read_input_tokens": 200,
                    "cache_creation_input_tokens": 50
                }
            }
        });
        writeln!(tmp, "{}", serde_json::to_string(&line).unwrap()).unwrap();
        tmp.flush().unwrap();

        let mut file = std::fs::File::open(tmp.path()).unwrap();
        let size = file.metadata().unwrap().len();
        let meta = scan_transcript_usage(&mut file, size);

        assert_eq!(meta.model.as_deref(), Some("claude-sonnet-4-20250514"));
        assert_eq!(meta.input_tokens, Some(1000));
        assert_eq!(meta.output_tokens, Some(500));
        assert_eq!(meta.cache_read_tokens, Some(200));
        assert_eq!(meta.cache_write_tokens, Some(50));
        assert_eq!(meta.stop_reason.as_deref(), Some("end_turn"));
    }

    #[test]
    fn scan_transcript_usage_skips_non_assistant() {
        use std::io::Write;
        let mut tmp = tempfile::NamedTempFile::new().unwrap();
        let line = serde_json::json!({
            "type": "user",
            "message": { "model": "should-be-ignored" }
        });
        writeln!(tmp, "{}", serde_json::to_string(&line).unwrap()).unwrap();
        tmp.flush().unwrap();

        let mut file = std::fs::File::open(tmp.path()).unwrap();
        let size = file.metadata().unwrap().len();
        let meta = scan_transcript_usage(&mut file, size);

        assert!(meta.model.is_none());
    }

    #[test]
    fn read_transcript_meta_returns_default_for_missing_file() {
        let meta = read_transcript_meta("/nonexistent/path/transcript.jsonl", None);
        assert!(meta.model.is_none());
        assert!(meta.duration_us.is_none());
    }

    #[test]
    fn compute_tool_duration_from_transcript() {
        use std::io::Write;
        let mut tmp = tempfile::NamedTempFile::new().unwrap();

        let invoke = serde_json::json!({
            "type": "assistant",
            "timestamp": "2026-02-18T12:00:00.000000Z",
            "message": {
                "content": [
                    { "type": "tool_use", "id": "tu_abc", "name": "Read" }
                ]
            }
        });
        let result = serde_json::json!({
            "type": "user",
            "timestamp": "2026-02-18T12:00:01.500000Z",
            "message": {
                "content": [
                    { "type": "tool_result", "tool_use_id": "tu_abc", "content": "ok" }
                ]
            }
        });
        writeln!(tmp, "{}", serde_json::to_string(&invoke).unwrap()).unwrap();
        writeln!(tmp, "{}", serde_json::to_string(&result).unwrap()).unwrap();
        tmp.flush().unwrap();

        let mut file = std::fs::File::open(tmp.path()).unwrap();
        let size = file.metadata().unwrap().len();
        let duration = compute_tool_duration(&mut file, size, "tu_abc");

        assert!(duration.is_some());
        assert_eq!(duration.unwrap(), 1_500_000);
    }

    #[test]
    fn compute_unified_diff_returns_diff_for_changes() {
        let diff = compute_unified_diff("hello\n", "world\n");
        assert!(diff.is_some());
        let d = diff.unwrap();
        assert!(d.contains("-hello"));
        assert!(d.contains("+world"));
    }

    #[test]
    fn compute_unified_diff_returns_none_for_identical() {
        assert!(compute_unified_diff("same\n", "same\n").is_none());
    }

    #[test]
    fn compute_unified_diff_handles_empty_strings() {
        let diff = compute_unified_diff("", "new content\n");
        assert!(diff.is_some());
        assert!(diff.unwrap().contains("+new content"));
    }
}
