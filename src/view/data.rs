use crate::{
    cursor_usage,
    models::{McpEvent, Risk},
};
use anyhow::Result;
use std::collections::HashMap;
use std::fs::File;
use std::io::{BufRead, BufReader};

#[derive(Default)]
pub(super) struct LoadFilter<'a> {
    pub since: Option<&'a str>,
    pub until: Option<&'a str>,
    pub session: Option<&'a str>,
}

impl LoadFilter<'_> {
    pub(super) fn matches_date(&self, timestamp: &str) -> bool {
        let date = timestamp.get(..10).unwrap_or("");
        if let Some(since) = self.since {
            if date < since {
                return false;
            }
        }
        if let Some(until) = self.until {
            if date > until {
                return false;
            }
        }
        true
    }

    fn matches_session(&self, session_id: &str) -> bool {
        self.session.is_none_or(|pfx| session_id.starts_with(pfx))
    }
}

pub(super) fn all_ledger_files(ledger_path: &str) -> Vec<std::path::PathBuf> {
    let path = std::path::Path::new(ledger_path);
    let parent = path.parent().unwrap_or(std::path::Path::new("."));
    let stem = path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("events");
    let active_name = path.file_name().and_then(|s| s.to_str()).unwrap_or("");

    let mut files: Vec<(std::path::PathBuf, u128)> = std::fs::read_dir(parent)
        .into_iter()
        .flatten()
        .filter_map(|e| e.ok())
        .filter_map(|e| {
            let name = e.file_name().into_string().ok()?;
            if name == active_name {
                return None;
            }
            if name.starts_with(stem) && name.ends_with(".jsonl") {
                let ts: u128 = name
                    .strip_prefix(&format!("{stem}."))?
                    .strip_suffix(".jsonl")?
                    .parse()
                    .ok()?;
                Some((e.path(), ts))
            } else {
                None
            }
        })
        .collect();

    files.sort_by_key(|(_, ts)| *ts);
    let mut result: Vec<std::path::PathBuf> = files.into_iter().map(|(p, _)| p).collect();
    result.push(path.to_path_buf());
    result
}

pub(super) fn load_sessions(
    ledger_path: &str,
    filter: &LoadFilter,
) -> Result<Vec<(String, Vec<McpEvent>)>> {
    let files = all_ledger_files(ledger_path);
    let any_exists = files.iter().any(|f| f.exists());
    if !any_exists {
        return Err(anyhow::anyhow!(
            "no ledger found at {ledger_path}\nRun vigilo first to generate events."
        ));
    }

    let mut map: HashMap<String, Vec<McpEvent>> = HashMap::new();
    for file_path in &files {
        let Ok(file) = File::open(file_path) else {
            continue;
        };
        for line in BufReader::new(file).lines() {
            let Ok(line) = line else { continue };
            if line.trim().is_empty() {
                continue;
            }
            if let Ok(mut event) = serde_json::from_str::<McpEvent>(&line) {
                if event.risk == Risk::Unknown {
                    event.risk = Risk::classify(&event.tool);
                }
                let sid = event.session_id.to_string();
                if !filter.matches_session(&sid) {
                    continue;
                }
                if !filter.matches_date(&event.timestamp) {
                    continue;
                }
                map.entry(sid).or_default().push(event);
            }
        }
    }

    let mut sessions: Vec<(String, Vec<McpEvent>)> = map.into_iter().collect();
    sessions.sort_by(|a, b| {
        let last_a = a.1.last().map(|e| e.timestamp.as_str()).unwrap_or("");
        let last_b = b.1.last().map(|e| e.timestamp.as_str()).unwrap_or("");
        last_a.cmp(last_b)
    });
    Ok(sessions)
}

pub(super) fn cursor_session_tokens(
    events: &[McpEvent],
) -> Option<cursor_usage::CachedSessionTokens> {
    let first = events.first()?;
    if first.server != "cursor" {
        return None;
    }
    if events.iter().any(|e| e.input_tokens.is_some()) {
        return None;
    }

    let (start_ms, end_ms) = session_time_range_ms(events)?;
    let cached = cursor_usage::load_cached_tokens_for_range(start_ms, end_ms);
    cursor_usage::aggregate_cached_tokens(&cached)
}

fn session_time_range_ms(events: &[McpEvent]) -> Option<(i64, i64)> {
    let parse_ts = |ts: &str| -> Option<i64> {
        chrono::DateTime::parse_from_rfc3339(ts)
            .or_else(|_| chrono::DateTime::parse_from_rfc3339(&format!("{ts}Z")))
            .ok()
            .map(|dt| dt.timestamp_millis())
    };

    let first_ts = events.first().and_then(|e| parse_ts(&e.timestamp))?;
    let last_ts = events.last().and_then(|e| parse_ts(&e.timestamp))?;

    Some((first_ts - 60_000, last_ts + 60_000))
}
