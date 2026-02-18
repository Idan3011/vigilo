use crate::{
    git, ledger,
    models::{self, McpEvent, ProjectContext},
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

pub fn compute_edit_diff(tool: &str, args: &serde_json::Value) -> Option<String> {
    if tool != "Edit" && tool != "MultiEdit" {
        return None;
    }
    let old = args.get("old_string").and_then(|v| v.as_str())?;
    let new = args.get("new_string").and_then(|v| v.as_str())?;
    models::compute_unified_diff(old, new)
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
    ProjectContext {
        root: git::root_in(git_dir).await,
        name: git::name_in(Some(git_dir)).await,
        branch: git::branch_in(git_dir).await,
        commit: git::commit_in(git_dir).await,
        dirty: git::dirty_in(git_dir).await,
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

fn scan_transcript_usage(file: &mut std::fs::File, size: u64) -> TranscriptMeta {
    use std::io::{BufRead, Seek, SeekFrom};

    let tail_start = size.saturating_sub(64 * 1024);
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
    let read_from = size.saturating_sub(512 * 1024);
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
