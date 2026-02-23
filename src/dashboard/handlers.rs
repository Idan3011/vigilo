use axum::extract::{Query, State};
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::Json;
use std::collections::HashMap;
use std::convert::Infallible;

use super::types::*;
use super::AppState;
use crate::crypto::{self, EncryptionKey};
use crate::models::{McpEvent, Outcome, Risk};
use crate::view::counts::{collect_active_projects, EventCounts};
use crate::view::data::{cursor_session_tokens, load_sessions, LoadFilter};
use crate::view::fmt::{event_cost_usd, fmt_arg, normalize_model, risk_label, session_cost_usd};

#[derive(serde::Deserialize, Default)]
pub struct DateRangeParams {
    pub since: Option<String>,
    pub until: Option<String>,
    pub session: Option<String>,
    pub last: Option<usize>,
}

#[derive(serde::Deserialize, Default)]
pub struct EventFilterParams {
    pub since: Option<String>,
    pub until: Option<String>,
    pub session: Option<String>,
    pub tool: Option<String>,
    pub risk: Option<String>,
    pub last: Option<usize>,
}

fn event_to_item(e: &McpEvent, key: Option<&EncryptionKey>) -> EventItem {
    let is_error = matches!(e.outcome, Outcome::Err { .. });
    let project_root = e.project.root.as_deref();
    let arg_display = fmt_arg(e, key, project_root);
    let error_message = match &e.outcome {
        Outcome::Err { message, .. } => Some(message.clone()),
        _ => None,
    };

    EventItem {
        id: e.id.to_string(),
        timestamp: e.timestamp.clone(),
        session_id: e.session_id.to_string(),
        server: e.server.clone(),
        tool: e.tool.clone(),
        risk: e.risk,
        duration_us: e.duration_us,
        is_error,
        project: e.project.name.clone(),
        branch: e.project.branch.clone(),
        arg_display,
        input_tokens: e.input_tokens(),
        output_tokens: e.output_tokens(),
        cache_read_tokens: e.cache_read_tokens(),
        cache_write_tokens: e.cache_write_tokens(),
        model: e.model().map(|m| normalize_model(m).to_string()),
        error_message,
    }
}

pub(super) struct TodaySummary {
    pub sessions: Vec<(String, Vec<McpEvent>)>,
    pub counts: EventCounts,
    pub projects: Vec<String>,
}

pub(super) fn today_summary(ledger_path: &std::path::Path) -> TodaySummary {
    let today = chrono::Local::now()
        .date_naive()
        .format("%Y-%m-%d")
        .to_string();
    let filter = LoadFilter {
        since: Some(&today),
        until: Some(&today),
        ..Default::default()
    };
    let sessions = load_sessions(ledger_path, &filter).unwrap_or_default();
    let all_events: Vec<&McpEvent> = sessions.iter().flat_map(|(_, e)| e).collect();
    let mut counts = EventCounts::from_events(&all_events);
    counts.add_cursor_tokens(&sessions);
    let projects = collect_active_projects(&sessions);
    TodaySummary {
        sessions,
        counts,
        projects,
    }
}

pub async fn summary(State(state): State<AppState>) -> Json<SummaryResponse> {
    let ts = today_summary(&state.ledger_path);
    Json(SummaryResponse {
        sessions: ts.sessions.len(),
        total_calls: ts.counts.total,
        errors: ts.counts.errors,
        reads: ts.counts.reads,
        writes: ts.counts.writes,
        execs: ts.counts.execs,
        input_tokens: ts.counts.total_in,
        output_tokens: ts.counts.total_out,
        cache_read_tokens: ts.counts.total_cr,
        cost_usd: ts.counts.total_cost,
        total_duration_us: ts.counts.total_us,
        projects: ts.projects,
    })
}

pub async fn sessions(
    State(state): State<AppState>,
    Query(params): Query<DateRangeParams>,
) -> Json<Vec<SessionListItem>> {
    let filter = LoadFilter {
        since: params.since.as_deref(),
        until: params.until.as_deref(),
        last: params.last,
        ..Default::default()
    };

    let sessions = load_sessions(&*state.ledger_path, &filter).unwrap_or_default();
    let items = build_merged_session_list(&sessions);
    Json(items)
}

/// Parse an RFC 3339 timestamp to epoch seconds.
fn ts_to_epoch(ts: &str) -> Option<i64> {
    chrono::DateTime::parse_from_rfc3339(ts)
        .or_else(|_| chrono::DateTime::parse_from_rfc3339(&format!("{ts}Z")))
        .ok()
        .map(|dt| dt.timestamp())
}

const MERGE_GAP_SECS: u64 = 7200; // 2 hours

/// Build a session list, merging consecutive sessions that belong to the same
/// conversation (same server + same project + time gap < 2 hours).
pub(super) fn build_merged_session_list(
    sessions: &[(String, Vec<McpEvent>)],
) -> Vec<SessionListItem> {
    if sessions.is_empty() {
        return Vec::new();
    }

    // Build individual session metadata
    struct SessionMeta {
        id: String,
        id_prefix: String,
        server: String,
        project: Option<String>,
        branch: Option<String>,
        date: String,
        call_count: usize,
        duration_us: u64,
        cost_usd: f64,
        error_count: usize,
        first_epoch: i64,
        last_epoch: i64,
    }

    let metas: Vec<SessionMeta> = sessions
        .iter()
        .filter_map(|(sid, events)| {
            let first = events.first()?;
            let last = events.last()?;
            let mut cost = session_cost_usd(events);
            if let Some(ct) = cursor_session_tokens(events) {
                cost += ct.cost_usd;
            }
            let error_count = events
                .iter()
                .filter(|e| matches!(e.outcome, Outcome::Err { .. }))
                .count();

            Some(SessionMeta {
                id: sid.clone(),
                id_prefix: sid[..8.min(sid.len())].to_string(),
                server: first.server.clone(),
                project: first.project.name.clone(),
                branch: first.project.branch.clone(),
                date: first
                    .timestamp
                    .get(..10)
                    .unwrap_or(&first.timestamp)
                    .to_string(),
                call_count: events.len(),
                duration_us: events.iter().map(|e| e.duration_us).sum(),
                cost_usd: cost,
                error_count,
                first_epoch: ts_to_epoch(&first.timestamp).unwrap_or(0),
                last_epoch: ts_to_epoch(&last.timestamp).unwrap_or(0),
            })
        })
        .collect();

    // Merge sessions with same server + project + gap < threshold.
    // HashMap lookup for candidate groups by (server, project) key.
    let mut groups: Vec<(SessionListItem, i64)> = Vec::new();
    let mut group_index: HashMap<(String, Option<String>), Vec<usize>> = HashMap::new();

    for meta in &metas {
        let key = (meta.server.clone(), meta.project.clone());
        let mut merged = false;

        if let Some(indices) = group_index.get(&key) {
            for &idx in indices.iter().rev() {
                let (_, last_epoch) = &groups[idx];
                if meta.first_epoch.abs_diff(*last_epoch) < MERGE_GAP_SECS {
                    let (group, last_epoch) = &mut groups[idx];
                    group.call_count += meta.call_count;
                    group.duration_us += meta.duration_us;
                    group.cost_usd += meta.cost_usd;
                    group.error_count += meta.error_count;
                    group.session_ids.push(meta.id_prefix.clone());
                    if meta.last_epoch > *last_epoch {
                        *last_epoch = meta.last_epoch;
                    }
                    merged = true;
                    break;
                }
            }
        }

        if !merged {
            let idx = groups.len();
            groups.push((
                SessionListItem {
                    id: meta.id.clone(),
                    server: meta.server.clone(),
                    date: meta.date.clone(),
                    project: meta.project.clone(),
                    branch: meta.branch.clone(),
                    call_count: meta.call_count,
                    duration_us: meta.duration_us,
                    cost_usd: meta.cost_usd,
                    error_count: meta.error_count,
                    session_ids: vec![meta.id_prefix.clone()],
                },
                meta.last_epoch,
            ));
            group_index.entry(key).or_default().push(idx);
        }
    }

    groups.sort_by_key(|(_, epoch)| *epoch);
    groups.into_iter().map(|(item, _)| item).collect()
}

pub async fn stats(
    State(state): State<AppState>,
    Query(params): Query<DateRangeParams>,
) -> Json<StatsResponse> {
    let filter = LoadFilter {
        since: params.since.as_deref(),
        until: params.until.as_deref(),
        session: params.session.as_deref(),
        ..Default::default()
    };

    let sessions = load_sessions(&*state.ledger_path, &filter).unwrap_or_default();
    let all_events: Vec<&McpEvent> = sessions.iter().flat_map(|(_, e)| e).collect();
    let mut c = EventCounts::from_events(&all_events);
    c.add_cursor_tokens(&sessions);

    // Named accumulator structs for readability
    #[derive(Default)]
    struct ModelAccum {
        calls: usize,
        input_tokens: u64,
        output_tokens: u64,
        cache_read_tokens: u64,
        cost_usd: f64,
    }
    #[derive(Default)]
    struct ToolAccum {
        count: usize,
        error_count: usize,
    }
    #[derive(Default)]
    struct ProjectAccum {
        count: usize,
        reads: usize,
        writes: usize,
        execs: usize,
    }

    // Model breakdown
    let mut model_map: HashMap<String, ModelAccum> = HashMap::new();
    for e in &all_events {
        if let Some(m) = e.model() {
            let entry = model_map.entry(normalize_model(m).to_string()).or_default();
            entry.calls += 1;
            entry.input_tokens += e.input_tokens().unwrap_or(0);
            entry.output_tokens += e.output_tokens().unwrap_or(0);
            entry.cache_read_tokens += e.cache_read_tokens().unwrap_or(0);
            if let Some(cost) = event_cost_usd(e) {
                entry.cost_usd += cost;
            }
        }
    }
    for (_, events) in &sessions {
        if let Some(ct) = cursor_session_tokens(events) {
            let entry = model_map.entry(ct.model.clone()).or_default();
            entry.input_tokens += ct.input_tokens;
            entry.output_tokens += ct.output_tokens;
            entry.cache_read_tokens += ct.cache_read_tokens;
            entry.cost_usd += ct.cost_usd;
        }
    }
    let mut models: Vec<ModelStatsJson> = model_map
        .into_iter()
        .map(|(model, a)| ModelStatsJson {
            model,
            calls: a.calls,
            input_tokens: a.input_tokens,
            output_tokens: a.output_tokens,
            cache_read_tokens: a.cache_read_tokens,
            cost_usd: a.cost_usd,
        })
        .collect();
    models.sort_by(|a, b| b.calls.cmp(&a.calls));

    // Tool breakdown
    let mut tool_map: HashMap<&str, ToolAccum> = HashMap::new();
    for e in &all_events {
        let entry = tool_map.entry(&e.tool).or_default();
        entry.count += 1;
        if matches!(e.outcome, Outcome::Err { .. }) {
            entry.error_count += 1;
        }
    }
    let mut tools: Vec<ToolCount> = tool_map
        .into_iter()
        .map(|(tool, a)| ToolCount {
            tool: tool.to_string(),
            count: a.count,
            error_count: a.error_count,
        })
        .collect();
    tools.sort_by(|a, b| b.count.cmp(&a.count));

    // File breakdown
    let mut file_map: HashMap<String, usize> = HashMap::new();
    for e in &all_events {
        if let Some(path) = e
            .arguments
            .get("file_path")
            .or_else(|| e.arguments.get("path"))
            .and_then(|v| v.as_str())
        {
            if !crypto::is_encrypted(path) {
                let parts: Vec<&str> = path.rsplit('/').take(2).collect();
                let display = if parts.len() >= 2 {
                    format!("{}/{}", parts[1], parts[0])
                } else {
                    parts[0].to_string()
                };
                *file_map.entry(display).or_default() += 1;
            }
        }
    }
    let mut files: Vec<FileCount> = file_map
        .into_iter()
        .map(|(file, count)| FileCount { file, count })
        .collect();
    files.sort_by(|a, b| b.count.cmp(&a.count));

    // Project breakdown
    let mut proj_map: HashMap<String, ProjectAccum> = HashMap::new();
    for e in &all_events {
        let name = e
            .project
            .name
            .as_deref()
            .or(e.project.root.as_deref())
            .unwrap_or("unknown")
            .to_string();
        let entry = proj_map.entry(name).or_default();
        entry.count += 1;
        match e.risk {
            Risk::Read => entry.reads += 1,
            Risk::Write => entry.writes += 1,
            Risk::Exec => entry.execs += 1,
            Risk::Unknown => {}
        }
    }
    let mut projects: Vec<ProjectStatsJson> = proj_map
        .into_iter()
        .map(|(name, a)| ProjectStatsJson {
            name,
            count: a.count,
            reads: a.reads,
            writes: a.writes,
            execs: a.execs,
        })
        .collect();
    projects.sort_by(|a, b| b.count.cmp(&a.count));

    // Timeline (group by date)
    let mut day_map: HashMap<String, TimelineDay> = HashMap::new();
    for e in &all_events {
        let date = e.timestamp.get(..10).unwrap_or("unknown").to_string();
        let entry = day_map
            .entry(date.clone())
            .or_insert_with(|| TimelineDay::new(date));
        if let Some(cost) = event_cost_usd(e) {
            entry.cost_usd += cost;
        }
        entry.input_tokens += e.input_tokens().unwrap_or(0);
        entry.output_tokens += e.output_tokens().unwrap_or(0);
        match e.risk {
            Risk::Read => entry.reads += 1,
            Risk::Write => entry.writes += 1,
            Risk::Exec => entry.execs += 1,
            Risk::Unknown => {}
        }
        if matches!(e.outcome, Outcome::Err { .. }) {
            entry.errors += 1;
        }
    }
    // Merge Cursor cached token data into timeline
    for (_, events) in &sessions {
        if let Some(ct) = cursor_session_tokens(events) {
            if let Some(first) = events.first() {
                let date = first.timestamp.get(..10).unwrap_or("unknown").to_string();
                let entry = day_map.entry(date.clone()).or_insert_with(|| TimelineDay {
                    date,
                    cost_usd: 0.0,
                    input_tokens: 0,
                    output_tokens: 0,
                    reads: 0,
                    writes: 0,
                    execs: 0,
                    errors: 0,
                });
                entry.cost_usd += ct.cost_usd;
                entry.input_tokens += ct.input_tokens;
                entry.output_tokens += ct.output_tokens;
            }
        }
    }
    let mut timeline: Vec<TimelineDay> = day_map.into_values().collect();
    timeline.sort_by(|a, b| a.date.cmp(&b.date));

    Json(StatsResponse {
        counts: CountsJson {
            total: c.total,
            reads: c.reads,
            writes: c.writes,
            execs: c.execs,
            errors: c.errors,
            input_tokens: c.total_in,
            output_tokens: c.total_out,
            cache_read_tokens: c.total_cr,
            cost_usd: c.total_cost,
            total_duration_us: c.total_us,
        },
        models,
        tools,
        files,
        projects,
        timeline,
    })
}

pub async fn events(
    State(state): State<AppState>,
    Query(params): Query<EventFilterParams>,
) -> Json<Vec<EventItem>> {
    let key = state.encryption_key.as_deref();

    let filter = LoadFilter {
        since: params.since.as_deref(),
        until: params.until.as_deref(),
        session: params.session.as_deref(),
        last: params.last,
    };

    let sessions = load_sessions(&*state.ledger_path, &filter).unwrap_or_default();
    let items: Vec<EventItem> = sessions
        .iter()
        .flat_map(|(_, evts)| evts)
        .filter(|e| params.tool.as_ref().is_none_or(|t| &e.tool == t))
        .filter(|e| params.risk.as_ref().is_none_or(|r| risk_label(e.risk) == r))
        .map(|e| event_to_item(e, key))
        .collect();

    Json(items)
}

#[derive(serde::Deserialize, Default)]
pub struct ErrorFilterParams {
    pub since: Option<String>,
    pub until: Option<String>,
    pub limit: Option<usize>,
}

pub async fn errors(
    State(state): State<AppState>,
    Query(params): Query<ErrorFilterParams>,
) -> Json<ErrorsResponse> {
    let key = state.encryption_key.as_deref();
    let limit = params.limit.unwrap_or(50).min(500);

    let filter = LoadFilter {
        since: params.since.as_deref(),
        until: params.until.as_deref(),
        ..Default::default()
    };

    let sessions = load_sessions(&*state.ledger_path, &filter).unwrap_or_default();
    let all_events: Vec<&McpEvent> = sessions.iter().flat_map(|(_, e)| e).collect();

    let err_events: Vec<&McpEvent> = all_events
        .iter()
        .filter(|e| matches!(e.outcome, Outcome::Err { .. }))
        .copied()
        .collect();
    let truncated = err_events.len() > limit;

    let mut by_tool_map: HashMap<&str, usize> = HashMap::new();
    for e in &err_events {
        *by_tool_map.entry(&e.tool).or_default() += 1;
    }
    let mut by_tool: Vec<ToolErrorCount> = by_tool_map
        .into_iter()
        .map(|(tool, count)| ToolErrorCount {
            tool: tool.to_string(),
            count,
        })
        .collect();
    by_tool.sort_by(|a, b| b.count.cmp(&a.count));

    let recent_errors: Vec<EventItem> = err_events
        .iter()
        .rev()
        .take(limit)
        .map(|e| event_to_item(e, key))
        .collect();

    Json(ErrorsResponse {
        total_calls: all_events.len(),
        error_count: err_events.len(),
        by_tool,
        recent_errors,
        truncated,
    })
}

pub async fn event_stream(
    State(state): State<AppState>,
) -> Sse<impl tokio_stream::Stream<Item = Result<Event, Infallible>>> {
    let ledger_path = state.ledger_path.clone();
    let key = state.encryption_key.clone();

    let stream = async_stream::stream! {
        use notify::{RecursiveMode, Watcher, EventKind};
        use std::io::{BufRead, BufReader, Seek, SeekFrom};

        let path: std::path::PathBuf = (*ledger_path).clone();

        // Start at end of file
        let mut pos = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);

        let (tx, mut rx) = tokio::sync::mpsc::channel(32);
        let watch_path = path.clone();
        let _watcher = {
            let tx = tx.clone();
            let mut w = notify::recommended_watcher(move |res: Result<notify::Event, _>| {
                if let Ok(evt) = res {
                    if matches!(evt.kind, EventKind::Modify(_) | EventKind::Create(_)) {
                        let _ = tx.blocking_send(());
                    }
                }
            }).ok();
            if let Some(ref mut w) = w {
                let _ = w.watch(
                    watch_path.parent().unwrap_or(&watch_path),
                    RecursiveMode::NonRecursive,
                );
            }
            w
        };

        loop {
            let _ = tokio::time::timeout(
                std::time::Duration::from_secs(30),
                rx.recv()
            ).await;

            let read_path = path.clone();
            let read_pos = pos;
            let read_key = key.clone();

            let result = tokio::task::spawn_blocking(move || {
                let Ok(file) = std::fs::File::open(&read_path) else {
                    return (read_pos, Vec::new());
                };
                let meta_len = file.metadata().map(|m| m.len()).unwrap_or(0);
                let mut current_pos = read_pos;
                if meta_len <= current_pos {
                    if meta_len < current_pos {
                        current_pos = 0;
                    }
                    return (current_pos, Vec::new());
                }

                let mut reader = BufReader::new(file);
                if reader.seek(SeekFrom::Start(current_pos)).is_err() {
                    return (current_pos, Vec::new());
                }

                let mut items = Vec::new();
                let mut line = String::new();
                while reader.read_line(&mut line).unwrap_or(0) > 0 {
                    let trimmed = line.trim();
                    if !trimmed.is_empty() {
                        if let Ok(event) = serde_json::from_str::<McpEvent>(trimmed) {
                            let item = event_to_item(&event, read_key.as_deref());
                            if let Ok(json) = serde_json::to_string(&item) {
                                items.push(json);
                            }
                        }
                    }
                    line.clear();
                }
                (meta_len, items)
            }).await;

            if let Ok((new_pos, items)) = result {
                pos = new_pos;
                for json in items {
                    yield Ok(Event::default().data(json));
                }
            }
        }
    };

    Sse::new(stream).keep_alive(KeepAlive::default())
}
