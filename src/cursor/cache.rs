use anyhow::Result;
use std::collections::HashMap;
use std::path::Path;

use crate::view::fmt::normalize_model;

const CACHE_STALE_SECS: u64 = 3600;

fn cache_path() -> String {
    crate::models::vigilo_path("cursor-tokens.jsonl")
        .to_string_lossy()
        .into_owned()
}

#[derive(serde::Serialize, serde::Deserialize, Debug)]
pub struct CachedTokenEvent {
    pub timestamp_ms: i64,
    pub model: String,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cache_read_tokens: u64,
    pub cache_write_tokens: u64,
    pub cost_cents: f64,
}

impl CachedTokenEvent {
    pub(super) fn from_api(ev: &serde_json::Value) -> Option<Self> {
        let ts_str = ev["timestamp"].as_str()?;
        let timestamp_ms = ts_str.parse::<i64>().ok()?;
        let tok = &ev["tokenUsage"];
        Some(Self {
            timestamp_ms,
            model: normalize_model(ev["model"].as_str().unwrap_or("unknown")).to_string(),
            input_tokens: tok["inputTokens"].as_u64().unwrap_or(0),
            output_tokens: tok["outputTokens"].as_u64().unwrap_or(0),
            cache_read_tokens: tok["cacheReadTokens"].as_u64().unwrap_or(0),
            cache_write_tokens: tok["cacheWriteTokens"].as_u64().unwrap_or(0),
            cost_cents: tok["totalCents"].as_f64().unwrap_or(0.0),
        })
    }
}

pub struct CachedSessionTokens {
    pub model: String,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cache_read_tokens: u64,
    pub cost_usd: f64,
    pub request_count: usize,
}

pub(super) fn write_cache(events: &[serde_json::Value]) -> Result<()> {
    let path = cache_path();
    if let Some(parent) = Path::new(&path).parent() {
        std::fs::create_dir_all(parent)?;
    }
    let mut lines = Vec::new();
    for ev in events {
        if let Some(cached) = CachedTokenEvent::from_api(ev) {
            lines.push(serde_json::to_string(&cached)?);
        }
    }
    std::fs::write(&path, lines.join("\n") + "\n")?;
    Ok(())
}

pub fn load_cached_tokens_for_range(start_ms: i64, end_ms: i64) -> Vec<CachedTokenEvent> {
    let Ok(content) = std::fs::read_to_string(cache_path()) else {
        return Vec::new();
    };
    content
        .lines()
        .filter(|l| !l.trim().is_empty())
        .filter_map(|l| serde_json::from_str::<CachedTokenEvent>(l).ok())
        .filter(|e| e.timestamp_ms >= start_ms && e.timestamp_ms <= end_ms)
        .collect()
}

pub fn aggregate_cached_tokens(events: &[CachedTokenEvent]) -> Option<CachedSessionTokens> {
    if events.is_empty() {
        return None;
    }

    let mut input = 0u64;
    let mut output = 0u64;
    let mut cache_read = 0u64;
    let mut cost_cents = 0.0f64;
    let mut model_counts: HashMap<String, usize> = HashMap::new();

    for e in events {
        input += e.input_tokens;
        output += e.output_tokens;
        cache_read += e.cache_read_tokens;
        cost_cents += e.cost_cents;
        *model_counts.entry(e.model.clone()).or_default() += 1;
    }

    let model = model_counts
        .into_iter()
        .max_by_key(|(_, c)| *c)
        .map(|(m, _)| m)
        .unwrap_or_else(|| "unknown".to_string());

    Some(CachedSessionTokens {
        model,
        input_tokens: input,
        output_tokens: output,
        cache_read_tokens: cache_read,
        cost_usd: cost_cents / 100.0,
        request_count: events.len(),
    })
}

pub fn is_cache_stale() -> bool {
    let path = cache_path();
    match std::fs::metadata(&path) {
        Ok(meta) => {
            let age = meta
                .modified()
                .ok()
                .and_then(|t| t.elapsed().ok())
                .map(|d| d.as_secs())
                .unwrap_or(u64::MAX);
            age > CACHE_STALE_SECS
        }
        Err(_) => true,
    }
}
