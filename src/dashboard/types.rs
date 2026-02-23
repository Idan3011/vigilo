use serde::Serialize;

#[derive(Serialize)]
pub struct SummaryResponse {
    pub sessions: usize,
    pub total_calls: usize,
    pub errors: usize,
    pub reads: usize,
    pub writes: usize,
    pub execs: usize,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cache_read_tokens: u64,
    pub cost_usd: f64,
    pub total_duration_us: u64,
    pub projects: Vec<String>,
}

#[derive(Serialize)]
pub struct SessionListItem {
    pub id: String,
    pub server: String,
    pub date: String,
    pub project: Option<String>,
    pub branch: Option<String>,
    pub call_count: usize,
    pub duration_us: u64,
    pub cost_usd: f64,
    pub error_count: usize,
    /// All constituent session ID prefixes (for merged/compacted sessions)
    pub session_ids: Vec<String>,
}

#[derive(Serialize)]
pub struct StatsResponse {
    pub counts: CountsJson,
    pub models: Vec<ModelStatsJson>,
    pub tools: Vec<ToolCount>,
    pub files: Vec<FileCount>,
    pub projects: Vec<ProjectStatsJson>,
    pub timeline: Vec<TimelineDay>,
}

#[derive(Serialize)]
pub struct CountsJson {
    pub total: usize,
    pub reads: usize,
    pub writes: usize,
    pub execs: usize,
    pub errors: usize,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cache_read_tokens: u64,
    pub cost_usd: f64,
    pub total_duration_us: u64,
}

#[derive(Serialize)]
pub struct ModelStatsJson {
    pub model: String,
    pub calls: usize,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cache_read_tokens: u64,
    pub cost_usd: f64,
}

#[derive(Serialize)]
pub struct ToolCount {
    pub tool: String,
    pub count: usize,
    pub error_count: usize,
}

#[derive(Serialize)]
pub struct FileCount {
    pub file: String,
    pub count: usize,
}

#[derive(Serialize)]
pub struct ProjectStatsJson {
    pub name: String,
    pub count: usize,
    pub reads: usize,
    pub writes: usize,
    pub execs: usize,
}

#[derive(Serialize, Default)]
pub struct TimelineDay {
    pub date: String,
    pub cost_usd: f64,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub reads: usize,
    pub writes: usize,
    pub execs: usize,
    pub errors: usize,
}

impl TimelineDay {
    pub fn new(date: String) -> Self {
        Self {
            date,
            ..Default::default()
        }
    }
}

#[derive(Serialize)]
pub struct EventItem {
    pub id: String,
    pub timestamp: String,
    pub session_id: String,
    pub server: String,
    pub tool: String,
    pub risk: crate::models::Risk,
    pub duration_us: u64,
    pub is_error: bool,
    pub project: Option<String>,
    pub branch: Option<String>,
    pub arg_display: String,
    pub input_tokens: Option<u64>,
    pub output_tokens: Option<u64>,
    pub cache_read_tokens: Option<u64>,
    pub cache_write_tokens: Option<u64>,
    pub model: Option<String>,
    pub error_message: Option<String>,
}

#[derive(Serialize)]
pub struct ErrorsResponse {
    pub total_calls: usize,
    pub error_count: usize,
    pub by_tool: Vec<ToolErrorCount>,
    pub recent_errors: Vec<EventItem>,
    pub truncated: bool,
}

#[derive(Serialize)]
pub struct ToolErrorCount {
    pub tool: String,
    pub count: usize,
}
