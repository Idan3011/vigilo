use crate::{
    cursor,
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
    pub last: Option<usize>,
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

/// Returns (path, rotation_timestamp_ms) for rotated files, sorted oldest first,
/// with the active ledger file appended last (timestamp = u128::MAX).
fn all_ledger_files_with_ts(ledger_path: &str) -> Vec<(std::path::PathBuf, u128)> {
    let path = std::path::Path::new(ledger_path);
    let parent = path.parent().unwrap_or(std::path::Path::new("."));
    let stem = crate::ledger::ledger_stem(path);
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
    files.push((path.to_path_buf(), u128::MAX));
    files
}

pub(super) fn all_ledger_files(ledger_path: &str) -> Vec<std::path::PathBuf> {
    all_ledger_files_with_ts(ledger_path)
        .into_iter()
        .map(|(p, _)| p)
        .collect()
}

/// Convert "YYYY-MM-DD" to epoch milliseconds (start of day UTC).
fn date_to_epoch_ms(date: &str) -> Option<u128> {
    let dt = chrono::NaiveDate::parse_from_str(date, "%Y-%m-%d").ok()?;
    let ts = dt.and_hms_opt(0, 0, 0)?.and_utc().timestamp_millis();
    Some(ts as u128)
}

pub(super) fn load_sessions(
    ledger_path: &str,
    filter: &LoadFilter,
) -> Result<Vec<(String, Vec<McpEvent>)>> {
    let files = all_ledger_files_with_ts(ledger_path);
    let any_exists = files.iter().any(|(f, _)| f.exists());
    if !any_exists {
        return Ok(Vec::new());
    }

    let since_ms = filter.since.and_then(date_to_epoch_ms);

    let mut map: HashMap<String, Vec<McpEvent>> = HashMap::new();

    // When `last` is set, read newest files first so we can stop early
    let file_order: Vec<&(std::path::PathBuf, u128)> = if filter.last.is_some() {
        files.iter().rev().collect()
    } else {
        files.iter().collect()
    };

    for (file_path, rotation_ts) in file_order {
        // Skip rotated files entirely before --since (all events predate the filter)
        if let Some(since) = since_ms {
            if *rotation_ts < since {
                continue;
            }
        }
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
        // Stop reading older files once we have more sessions than needed
        if let Some(n) = filter.last {
            if map.len() > n {
                break;
            }
        }
    }

    let mut sessions: Vec<(String, Vec<McpEvent>)> = map.into_iter().collect();
    sessions.sort_by(|a, b| {
        let last_a = a.1.last().map(|e| e.timestamp.as_str()).unwrap_or("");
        let last_b = b.1.last().map(|e| e.timestamp.as_str()).unwrap_or("");
        last_a.cmp(last_b)
    });

    if let Some(n) = filter.last {
        let skip = sessions.len().saturating_sub(n);
        sessions.drain(..skip);
    }

    Ok(sessions)
}

/// Load the last `n` events from the ledger without grouping by session.
/// Reads from newest files first and stops early.
pub(super) fn load_tail_events(ledger_path: &str, n: usize) -> Result<Vec<McpEvent>> {
    let files = all_ledger_files(ledger_path);
    if !files.iter().any(|f| f.exists()) {
        return Ok(Vec::new());
    }

    let mut events: Vec<McpEvent> = Vec::new();

    for file_path in files.iter().rev() {
        let Ok(file) = File::open(file_path) else {
            continue;
        };
        let mut batch: Vec<McpEvent> = BufReader::new(file)
            .lines()
            .map_while(Result::ok)
            .filter(|l| !l.trim().is_empty())
            .filter_map(|l| {
                let mut e: McpEvent = serde_json::from_str(&l).ok()?;
                if e.risk == Risk::Unknown {
                    e.risk = Risk::classify(&e.tool);
                }
                Some(e)
            })
            .collect();

        batch.append(&mut events);
        events = batch;

        if events.len() >= n {
            break;
        }
    }

    events.sort_by(|a, b| a.timestamp.cmp(&b.timestamp));
    let skip = events.len().saturating_sub(n);
    events.drain(..skip);
    Ok(events)
}

pub(super) fn cursor_session_tokens(
    events: &[McpEvent],
) -> Option<cursor::CachedSessionTokens> {
    let first = events.first()?;
    if first.server != "cursor" {
        return None;
    }
    if events.iter().any(|e| e.input_tokens().is_some()) {
        return None;
    }

    let (start_ms, end_ms) = session_time_range_ms(events)?;
    let cached = cursor::load_cached_tokens_for_range(start_ms, end_ms);
    cursor::aggregate_cached_tokens(&cached)
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{McpEvent, Outcome, ProjectContext};
    use std::io::Write;
    use uuid::Uuid;

    fn make_event(session_id: Uuid, tool: &str, ts: &str) -> McpEvent {
        McpEvent {
            id: Uuid::new_v4(),
            timestamp: ts.to_string(),
            session_id,
            server: "vigilo".to_string(),
            tool: tool.to_string(),
            arguments: serde_json::json!({"path": "test.rs"}),
            outcome: Outcome::Ok {
                result: serde_json::Value::Null,
            },
            duration_us: 100,
            risk: Risk::Read,
            project: ProjectContext::default(),
            ..Default::default()
        }
    }

    fn write_events(path: &str, events: &[McpEvent]) {
        let mut file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
            .unwrap();
        for e in events {
            let mut line = serde_json::to_string(e).unwrap();
            line.push('\n');
            file.write_all(line.as_bytes()).unwrap();
        }
    }

    #[test]
    fn load_tail_events_returns_last_n() {
        let dir = tempfile::tempdir().unwrap();
        let ledger = dir.path().join("events.jsonl");
        let path = ledger.to_str().unwrap();

        let sid = Uuid::new_v4();
        let events: Vec<McpEvent> = (0..10)
            .map(|i| make_event(sid, "read_file", &format!("2026-02-19T10:{i:02}:00Z")))
            .collect();
        write_events(path, &events);

        let tail = load_tail_events(path, 3).unwrap();
        assert_eq!(tail.len(), 3);
        assert!(tail[0].timestamp.contains("10:07"));
        assert!(tail[2].timestamp.contains("10:09"));
    }

    #[test]
    fn load_tail_events_returns_all_when_fewer_than_n() {
        let dir = tempfile::tempdir().unwrap();
        let ledger = dir.path().join("events.jsonl");
        let path = ledger.to_str().unwrap();

        let sid = Uuid::new_v4();
        write_events(
            path,
            &[make_event(sid, "read_file", "2026-02-19T10:00:00Z")],
        );

        let tail = load_tail_events(path, 50).unwrap();
        assert_eq!(tail.len(), 1);
    }

    #[test]
    fn load_sessions_last_limits_sessions() {
        let dir = tempfile::tempdir().unwrap();
        let ledger = dir.path().join("events.jsonl");
        let path = ledger.to_str().unwrap();

        let s1 = Uuid::new_v4();
        let s2 = Uuid::new_v4();
        let s3 = Uuid::new_v4();
        write_events(
            path,
            &[
                make_event(s1, "read_file", "2026-02-19T10:00:00Z"),
                make_event(s2, "read_file", "2026-02-19T11:00:00Z"),
                make_event(s3, "read_file", "2026-02-19T12:00:00Z"),
            ],
        );

        let filter = LoadFilter {
            last: Some(2),
            ..Default::default()
        };
        let sessions = load_sessions(path, &filter).unwrap();
        assert_eq!(sessions.len(), 2);
        // Should be the two most recent sessions
        let sids: Vec<Uuid> = sessions.iter().map(|(_, e)| e[0].session_id).collect();
        assert!(sids.contains(&s2));
        assert!(sids.contains(&s3));
    }

    #[test]
    fn load_sessions_date_filter() {
        let dir = tempfile::tempdir().unwrap();
        let ledger = dir.path().join("events.jsonl");
        let path = ledger.to_str().unwrap();

        let sid = Uuid::new_v4();
        write_events(
            path,
            &[
                make_event(sid, "read_file", "2026-02-18T10:00:00Z"),
                make_event(sid, "read_file", "2026-02-19T10:00:00Z"),
            ],
        );

        let filter = LoadFilter {
            since: Some("2026-02-19"),
            ..Default::default()
        };
        let sessions = load_sessions(path, &filter).unwrap();
        let total_events: usize = sessions.iter().map(|(_, e)| e.len()).sum();
        assert_eq!(total_events, 1);
    }

    #[test]
    fn all_ledger_files_finds_rotated_files() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("events.jsonl"), "").unwrap();
        std::fs::write(dir.path().join("events.100.jsonl"), "").unwrap();
        std::fs::write(dir.path().join("events.200.jsonl"), "").unwrap();

        let files = all_ledger_files(dir.path().join("events.jsonl").to_str().unwrap());
        assert_eq!(files.len(), 3);
        // Active file should be last
        assert!(files.last().unwrap().ends_with("events.jsonl"));
    }
}
